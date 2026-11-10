// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol enemies/{archetypes,reflection,invisible}.js.
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Prisoner {
    pub attacks: u32,
    pub free_all: bool,
    pub aspd: f64,
    pub def: f64,
    pub atk: f64,
    pub penetration: f64,
    pub res: f64,
    pub regen: f64,
}
#[derive(Clone, Copy, Debug)]
pub enum Family {
    Prisoner(Prisoner),
    Phalanx { res: f64, radius: f64, def: f64 },
    HolyGuard { res: f64, radius: f64, aspd: f64 },
    ShadowBlade { bat: f64 },
    ShadowKiller { scale: f64 },
    Tear { restore: f64, stun: f64 },
}
#[derive(Clone, Copy, Default)]
pub(super) enum Memory {
    #[default]
    None,
    Blades {
        used: u32,
        clear_cd: bool,
    },
    Veil {
        left: f64,
    },
    Prisoner {
        freed: bool,
    },
    Shadow {
        concealed: bool,
        power: bool,
    },
}
impl Family {
    pub(super) fn compile(
        key: &str,
        raw: &Value,
        ov: &Value,
        original: &str,
        tag: Option<&str>,
    ) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let value = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        Ok(Some(match key {
            "enemy_1116_liprr"
            | "enemy_1116_liprr_2"
            | "enemy_1118_lidbox_2"
            | "enemy_1119_vofsd"
            | "enemy_1121_lifbos"
            | "enemy_1121_lifbos_2" => {
                let attacks = (value("confinement.times", 4.) + 0.5).floor().max(1.);
                if attacks > i32::MAX as f64 {
                    return Err("Prisoner attack count outside positive i32 domain".into());
                }
                Self::Prisoner(Prisoner {
                    attacks: attacks as u32,
                    free_all: matches!(key, "enemy_1121_lifbos" | "enemy_1121_lifbos_2"),
                    aspd: t("confinement.attack_speed"),
                    def: t("confinement.def"),
                    atk: t("liberty.atk"),
                    penetration: t("liberty.def_penetrate"),
                    res: t("liberty.magic_resistance"),
                    regen: t("liberty.hp_recovery_per_sec"),
                })
            }
            "enemy_1169_duphlx" | "enemy_1169_duphlx_2" => Self::Phalanx {
                res: talent(
                    raw,
                    ov,
                    &["refracting.magic_resistance", "Refracting.magic_resistance"],
                ),
                radius: raw["stats"]["rawRangeRadius"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n != 0.)
                    .unwrap_or(1.5),
                def: t("auraDefup.def"),
            },
            "enemy_1174_duholy" | "enemy_1174_duholy_2" => Self::HolyGuard {
                res: talent(
                    raw,
                    ov,
                    &["refracting.magic_resistance", "Refracting.magic_resistance"],
                ),
                radius: value("traitAbility.range_radius", 1.1),
                aspd: t("traitAbility.attack_speed"),
            },
            "enemy_1175_dushdo_2" => Self::ShadowBlade {
                bat: t("traitAbility.base_attack_time"),
            },
            "enemy_1299_ymkilr" | "enemy_1299_ymkilr_2" => Self::ShadowKiller {
                scale: abilities::skill(raw, ov, "InvisibleCombat")["bb"]["atk_scale"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n != 0.)
                    .unwrap_or(1.),
            },
            "enemy_2034_sythef" | "enemy_2034_sythef_2" => Self::Tear {
                restore: if matches!(original, "enemy_2034_sythef" | "enemy_2034_sythef_2") {
                    if tag == Some("duck") {
                        1.
                    } else {
                        0.
                    }
                } else {
                    3.
                },
                stun: t("Combat.attack@stun"),
            },
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Prisoner(_) => Memory::Prisoner { freed: false },
            Self::ShadowKiller { .. } => Memory::Shadow {
                concealed: true,
                power: true,
            },
            _ => Memory::None,
        }
    }
    pub(super) fn spawn<H: buffs::Hooks>(
        self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let b = match self {
            Self::Prisoner(p) => {
                let mut b = Buff::default();
                b.key = Some("ab:confined".into());
                b.runtime.persist = true;
                b.runtime.visible = true;
                b.mods.insert("aspd".into(), p.aspd);
                b.mods.insert("defFlat".into(), p.def);
                b
            }
            Self::ShadowKiller { .. } | Self::ShadowBlade { .. } | Self::Tear { .. } => {
                let mut b = flag_buff("ab:stealth", "stealth");
                b.runtime.stealth_restore = Some(match self {
                    Self::Tear { restore, .. } => restore,
                    _ => 3.,
                });
                b
            }
            _ => return Ok(()),
        };
        r.add(
            w,
            h,
            id,
            Spec {
                mods_present: !b.mods.is_empty(),
                flags_present: !b.flags.is_empty(),
                buff: b,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
impl Kit {
    pub(super) fn refraction(&self) -> Option<(f64, f64)> {
        match *self {
            Self::Refraction { res, hp_pct } => Some((res, hp_pct)),
            Self::Form(Form::Stone(s)) => Some((s.res, 0.)),
            Self::Family(Family::Phalanx { res, .. } | Family::HolyGuard { res, .. }) => {
                Some((res, 0.))
            }
            _ => None,
        }
    }
}
impl Runtime {
    pub(crate) fn prisoner_attack(&mut self, id: Id) -> bool {
        let s = self.units.get_mut(&id).unwrap();
        if let (Kit::Family(Family::Prisoner(p)), Memory::Prisoner { freed: false }) =
            (s.definition.kit, s.memory)
        {
            s.count += 1;
            return s.count >= p.attacks;
        }
        false
    }
    pub(crate) fn free_prisoner(&mut self, id: Id) -> Option<Prisoner> {
        let s = self.units.get_mut(&id)?;
        if let (Kit::Family(Family::Prisoner(p)), Memory::Prisoner { freed: false }) =
            (s.definition.kit, s.memory)
        {
            s.memory = Memory::Prisoner { freed: true };
            return Some(p);
        }
        None
    }
    pub(crate) fn free_all_once(&mut self) -> bool {
        let first = !self.freed_all;
        self.freed_all = true;
        first
    }
    pub(crate) fn shadow_tick(&mut self, id: Id, on: bool) {
        if let Memory::Shadow { concealed, power } = &mut self.units.get_mut(&id).unwrap().memory {
            if on && !*concealed {
                *power = true;
            }
            *concealed = on;
        }
    }
    pub(crate) fn shadow_power(&self, id: Id) -> bool {
        self.units
            .get(&id)
            .is_some_and(|s| matches!(s.memory, Memory::Shadow { power: true, .. }))
    }
    pub(crate) fn shadow_spent(&mut self, id: Id) {
        if let Memory::Shadow { power, .. } = &mut self.units.get_mut(&id).unwrap().memory {
            *power = false;
        }
    }
}
