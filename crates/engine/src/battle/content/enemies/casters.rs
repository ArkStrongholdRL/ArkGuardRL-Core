// SPDX-License-Identifier: GPL-3.0-or-later
use super::movement::Cooldown;
use super::*;
mod champions;
pub(crate) use champions::BuffEffect;
#[derive(Clone, Copy, Debug)]
pub enum Spell {
    PipeStrike {
        gold: bool,
        radius: f64,
        element: f64,
    },
    Ensemble,
    Tide {
        stage: u8,
        scale: f64,
        element: f64,
    },
    Rockfall {
        stage: u8,
        scale: f64,
    },
    Tentacles {
        stage: u8,
    },
    Species {
        loss: f64,
    },
    GunCharge,
    SpringDash {
        duration: f64,
        mul: f64,
    },
    HelmShell,
    HelmDrones {
        ratio: f64,
        hp: f64,
    },
    BladeDive,
    Decree {
        scales: [f64; 2],
    },
    Repel {
        scale: f64,
        loss: f64,
    },
    Surge {
        max: usize,
        duration: f64,
        scale: f64,
    },
    XiShield {
        form2: bool,
        duration: f64,
        amount: f64,
        radius: f64,
        scale: f64,
    },
    XiCross {
        scale: f64,
    },
    MaceBlink {
        form2: bool,
        distance: f64,
        scale: f64,
    },
    MaceCircle {
        form2: bool,
        radius: f64,
        scale: f64,
    },
    Fear {
        max: usize,
        aspd: f64,
        peak: f64,
        ramp: f64,
        cut: f64,
        refund: f64,
        wait: f64,
    },
    LucienAoe {
        scale: f64,
        element: f64,
    },
    Phantom {
        distance: f64,
    },
    Charge {
        duration: f64,
        scale: f64,
    },
    Triple {
        radius: f64,
        delay: f64,
        scale: f64,
    },
    Cross {
        scale: f64,
        stun: f64,
    },
    DeathEye {
        duration: f64,
        scale: f64,
        element: f64,
    },
    Cannon {
        scale: f64,
    },
    Jazz {
        radius: f64,
        duration: f64,
        interval: f64,
        scale: f64,
        element: f64,
    },
    C4 {
        radius: f64,
        scale: f64,
    },
    Drift {
        damage: f64,
    },
    Sand {
        duration: f64,
        damage: f64,
        attack: f64,
    },
    Rock {
        radius: f64,
        scale: f64,
        stun: f64,
    },
    Chain {
        radius: f64,
        jump: f64,
        max: u32,
        duration: f64,
        speed: f64,
        aspd: f64,
    },
    Shield {
        amount: f64,
        hp: f64,
        aspd: f64,
    },
    TombShield {
        ratio: f64,
    },
    Blink {
        distance: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct Skill {
    pub spell: Spell,
    pub clock: Cooldown,
    pub priority: f64,
    pub clip: f64,
    pub turn: bool,
    pub own: bool,
    pub silence: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum Passive {
    Xi {
        duration: f64,
        invincible: f64,
        atk: f64,
    },
    Mace {
        resist: f64,
        duration: f64,
        invincible: f64,
        pen: [f64; 2],
        cut: f64,
        disarm: f64,
        radius: f64,
    },
    Wolf {
        duration: f64,
        invincible: f64,
        resistance: f64,
        atk: f64,
        bat: f64,
    },
    Lucien {
        penetration: f64,
        dodge: f64,
        element: f64,
    },
    Knight {
        ranged: bool,
        atk: f64,
        aspd: f64,
        speed: f64,
    },
    Pompeii {
        ratio: f64,
        aspd: f64,
        dot: f64,
        interval: f64,
        duration: f64,
        blast: f64,
        blast_interval: f64,
    },
    Plain,
    Stealth {
        jazz: bool,
    },
    DeathEye {
        element: f64,
    },
    Mouse {
        shield: f64,
        defense: f64,
        ratio: f64,
        scale: f64,
    },
    Mudrock {
        shield: f64,
        hp: f64,
        aspd: f64,
        atk: f64,
    },
    Tomb {
        scale: f64,
    },
    Snow {
        cold: f64,
        max: u32,
        fall: f64,
        jump: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct Caster {
    pub passive: Passive,
    pub skills: [Option<Skill>; 4],
}
pub(crate) const EMPTY: Caster = Caster {
    passive: Passive::Plain,
    skills: [None; 4],
};
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    pub left: [f64; 4],
    pub channel: bool,
    pub cannon: bool,
    pub enraged: bool,
    pub splash: bool,
    pub no_attack: bool,
    pub held: f64,
    pub phase: Option<lifecycle::RebornPhase>,
    pub next: f64,
    pub caught: usize,
    pub wait_until: f64,
    pub reborn_at: f64,
    pub evade: Option<bool>,
}
fn n(v: &Value, default: f64) -> f64 {
    v.as_f64().filter(|v| v.is_finite()).unwrap_or(default)
}
pub(super) fn ability(
    raw: &Value,
    ov: &Value,
    key: &str,
    spell: Spell,
    clip: f64,
    turn: bool,
    silence: bool,
) -> Option<Skill> {
    let clock = Cooldown::compile(raw, ov, key)?;
    let s = abilities::skill(raw, ov, key);
    Some(Skill {
        spell,
        clock,
        clip,
        turn,
        own: turn,
        silence,
        priority: n(&s["priority"], 0.),
    })
}
impl Caster {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        if let Some(config) = Self::champion(key, raw, ov)? {
            return Ok(Some(config));
        }
        let t = |key| talent(raw, ov, &[key]);
        let v = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        let skill = |key| abilities::skill(raw, ov, key);
        let radius = n(&raw["stats"]["rangeRadius"], 0.);
        let raw_radius = n(&raw["stats"]["rawRangeRadius"], 0.);
        let (passive, primary, secondary) = match key {
            "enemy_1050_lslime" => (
                Passive::Pompeii {
                    ratio: v("selfbuff.hp_ratio", 0.5),
                    aspd: t("selfbuff.attack_speed"),
                    dot: t("dot.damage"),
                    interval: v("dot.interval", 1.),
                    duration: t("dot.duration"),
                    blast: t("rangedamage.attack@damage"),
                    blast_interval: v("rangedamage.interval", 10.),
                },
                None,
                None,
            ),
            "enemy_1513_dekght" | "enemy_1513_dekght_2" => {
                let ranged = key.ends_with("_2");
                let spell = if ranged {
                    Spell::Triple {
                        radius: if radius != 0. { radius } else { 2.5 },
                        delay: n(&skill("TripleAttack")["bb"]["dekght_2[aoe].interval"], 0.),
                        scale: n(&skill("TripleAttack")["bb"]["dekght_2[aoe].atk_scale"], 1.),
                    }
                } else {
                    Spell::Charge {
                        duration: n(&skill("ChargeAttack")["bb"]["duration"], 0.),
                        scale: n(
                            &skill("ChargeAttack")["bb"]["dekght[aoe].atk_scale"],
                            n(&skill("ChargeAttack")["bb"]["atk_scale"], 1.),
                        ),
                    }
                };
                let mut cast = ability(
                    raw,
                    ov,
                    if ranged {
                        "TripleAttack"
                    } else {
                        "ChargeAttack"
                    },
                    spell,
                    if ranged { 1.667 } else { 0. },
                    true,
                    false,
                );
                if let Some(c) = &mut cast {
                    c.own = ranged;
                }
                (
                    Passive::Knight {
                        ranged,
                        atk: t("triggerrage.atk"),
                        aspd: t("triggerrage.attack_speed"),
                        speed: t("triggerrage.move_speed"),
                    },
                    cast,
                    None,
                )
            }

            "enemy_1404_msnip" => (
                Passive::Stealth { jazz: false },
                ability(
                    raw,
                    ov,
                    "CrossAttack",
                    Spell::Cross {
                        scale: n(&skill("CrossAttack")["bb"]["atk_scale"], 1.),
                        stun: n(&skill("CrossAttack")["bb"]["stun"], 0.),
                    },
                    0.,
                    true,
                    true,
                ),
                None,
            ),
            "enemy_1275_dwlock_2" => (
                Passive::DeathEye {
                    element: t("empty.attack@ep_damage_ratio"),
                },
                ability(
                    raw,
                    ov,
                    "DeathEye",
                    Spell::DeathEye {
                        duration: n(&skill("DeathEye")["bb"]["hit_duration"], 0.),
                        scale: n(&skill("DeathEye")["bb"]["atk_scale"], 0.),
                        element: n(&skill("DeathEye")["bb"]["ep_damage_ratio"], 0.),
                    },
                    0.,
                    true,
                    true,
                ),
                None,
            ),
            "enemy_1273_stmgun_2" => (
                Passive::Plain,
                ability(
                    raw,
                    ov,
                    "Cannon",
                    Spell::Cannon {
                        scale: n(&skill("Cannon")["bb"]["atk_scale"], 0.),
                    },
                    10.,
                    true,
                    false,
                ),
                None,
            ),
            "enemy_10034_cnvsax" => (
                Passive::Stealth { jazz: true },
                ability(
                    raw,
                    ov,
                    "fire",
                    Spell::Jazz {
                        radius: if raw_radius != 0. { raw_radius } else { 2.5 },
                        duration: n(&skill("fire")["bb"]["enemy_cnvsax[cd].duration"], 0.),
                        interval: n(&skill("fire")["bb"]["hit_interval"], 0.5),
                        scale: n(&skill("fire")["bb"]["atk_scale"], 0.),
                        element: n(&skill("fire")["bb"]["ep_damage_ratio"], 0.),
                    },
                    0.,
                    true,
                    true,
                ),
                None,
            ),
            "enemy_1504_cqbw" => (
                Passive::Plain,
                ability(
                    raw,
                    ov,
                    "C4",
                    Spell::C4 {
                        radius: n(&skill("C4")["bb"]["range_radius"], radius),
                        scale: n(&skill("C4")["bb"]["atk_scale"], 1.),
                    },
                    4.8,
                    true,
                    false,
                ),
                None,
            ),
            "enemy_1509_mousek" => (
                Passive::Mouse {
                    shield: t("shield.dynamic"),
                    defense: t("defup.def"),
                    ratio: v("enrage.hp_ratio", 0.5),
                    scale: v("enrage.damage_scale", 1.),
                },
                ability(
                    raw,
                    ov,
                    "DriftSand",
                    Spell::Drift {
                        damage: n(&skill("DriftSand")["bb"]["damage"], 0.),
                    },
                    2.333,
                    true,
                    false,
                ),
                ability(
                    raw,
                    ov,
                    "SandStorm",
                    Spell::Sand {
                        duration: n(&skill("SandStorm")["bb"]["duration"], 0.),
                        damage: n(&skill("SandStorm")["bb"]["damage"], 0.),
                        attack: n(&skill("SandStorm")["bb"]["atk"], 0.),
                    },
                    1.333,
                    true,
                    false,
                ),
            ),
            "enemy_2003_rockman" => (
                Passive::Plain,
                ability(
                    raw,
                    ov,
                    "StunAttack",
                    Spell::Rock {
                        radius: if raw_radius != 0. { raw_radius } else { 2.5 },
                        scale: n(&skill("StunAttack")["bb"]["atk_scale"], 1.),
                        stun: n(&skill("StunAttack")["bb"]["stun"], 0.),
                    },
                    2.333,
                    true,
                    false,
                ),
                None,
            ),
            "enemy_2050_smsha" => {
                let jump = v("Attack.attack@projectile_range", 1.6);
                (
                    Passive::Snow {
                        cold: t("Attack.attack@freeze"),
                        max: abilities::count(v("Attack.attack@chain.max_target", 1.))?,
                        fall: v("Attack.attack@chain.atk_scale", 1.),
                        jump,
                    },
                    ability(
                        raw,
                        ov,
                        "ChainBuff",
                        Spell::Chain {
                            radius: if radius != 0. { radius } else { 3.5 },
                            jump: n(&skill("ChainBuff")["bb"]["projectile_range"], jump),
                            max: abilities::count(n(
                                &skill("ChainBuff")["bb"]["chain.max_target"],
                                3.,
                            ))?,
                            duration: n(&skill("ChainBuff")["bb"]["duration"], 0.),
                            speed: n(&skill("ChainBuff")["bb"]["move_speed"], 0.),
                            aspd: n(&skill("ChainBuff")["bb"]["attack_speed"], 0.),
                        },
                        1.667,
                        true,
                        true,
                    ),
                    None,
                )
            }
            "enemy_1511_mdrock" => {
                let s = skill("RefreshShield");
                let amount = n(&s["bb"]["dynamic"], t("shield.dynamic"));
                let hp = n(&s["bb"]["max_hp"], t("shield.max_hp"));
                let aspd = n(&s["bb"]["attack_speed"], t("shield.attack_speed"));
                (
                    Passive::Mudrock {
                        shield: amount,
                        hp,
                        aspd,
                        atk: t("charge.attack@enemy_mdrock_s_1[charge].atk") / 6.,
                    },
                    ability(
                        raw,
                        ov,
                        "RefreshShield",
                        Spell::Shield { amount, hp, aspd },
                        0.,
                        false,
                        false,
                    ),
                    None,
                )
            }
            "enemy_2008_flking" => (
                Passive::Tomb {
                    scale: v("atkdown.atk_scale", 1.),
                },
                ability(
                    raw,
                    ov,
                    "refreshshield",
                    Spell::TombShield {
                        ratio: n(&skill("refreshshield")["bb"]["hp_ratio"], 0.),
                    },
                    0.,
                    false,
                    false,
                ),
                None,
            ),
            "enemy_1502_crowns" => (
                Passive::Plain,
                ability(
                    raw,
                    ov,
                    "blink",
                    Spell::Blink {
                        distance: n(&skill("blink")["bb"]["dist"], 1.5),
                    },
                    0.,
                    false,
                    true,
                ),
                None,
            ),
            _ => return Ok(None),
        };
        Ok(Some(Self {
            passive,
            skills: [primary, secondary, None, None],
        }))
    }
    pub(super) fn state(self) -> State {
        State {
            phase: self.rebirth().map(|_| lifecycle::RebornPhase::Fresh),
            next: 1.,
            wait_until: f64::NEG_INFINITY,
            no_attack: matches!(self.passive, Passive::Stealth { jazz: true }),
            left: self
                .skills
                .map(|s| s.map(|s| s.clock.initial).unwrap_or(0.)),
            ..Default::default()
        }
    }
    pub(super) fn spawn<H: buffs::Hooks>(
        self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let mut list = vec![];
        match self.passive {
            Passive::Wolf { resistance, .. } => {
                w.unit_mut(id)?.immune.insert("stun".into());
                let mut b = Buff::default();
                b.key = Some("ab:wolfGuard".into());
                b.runtime.persist = true;
                b.runtime.visible = true;
                b.mods.insert("dmgTakenMul".into(), 1. - resistance);
                list.push(b);
            }
            Passive::Lucien { penetration, .. } if penetration > 0. => {
                let mut b = Buff::default();
                b.key = Some("boss:pierce".into());
                b.runtime.persist = true;
                b.mods.insert("defIgnorePct".into(), penetration);
                list.push(b);
            }
            Passive::Stealth { jazz } => {
                let mut b = flag_buff("ab:stealth", "stealth");
                b.runtime.stealth_restore = Some(if jazz { 0. } else { 3. });
                list.push(b);
            }
            Passive::Mouse {
                shield, defense, ..
            } if shield > 0. => {
                list.push(Self::barrier("ab:sandDef", shield, &[("defFlat", defense)]))
            }
            Passive::Mudrock {
                shield, hp, aspd, ..
            } if shield > 0. => list.push(Self::barrier(
                "ab:rockPower",
                shield,
                &[("hpPct", hp), ("aspd", aspd)],
            )),
            _ => {}
        }
        for b in list {
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
        }
        Ok(())
    }
    pub(crate) fn barrier(key: &str, amount: f64, mods: &[(&str, f64)]) -> Buff {
        let mut b = Buff::default();
        b.key = Some(key.into());
        b.runtime.persist = true;
        b.runtime.visible = true;
        b.runtime.tick_payload = amount;
        b.runtime.injury.on_injury = Some(1703);
        b.mods = mods.iter().map(|&(k, v)| (k.into(), v)).collect();
        b
    }
}
impl Runtime {
    pub(crate) fn caster(&self, id: Id) -> State {
        self.units.get(&id).map(|s| s.caster).unwrap_or_default()
    }
    pub(crate) fn set_caster(&mut self, id: Id, state: State) {
        self.units.get_mut(&id).unwrap().caster = state;
    }
}
