// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "splashcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_sntlla_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skcom_quickattack[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let mode = tactical_mode(raw)?;

    s.combat.area.splash = Some(Splash {
        radius: 1.,
        scale: 1.,
        hits_fly: true,
        fortress_melee: false,
    });
    s.combat.skill_area = s.combat.area;
    skill_range(raw, s)?;
    if mode == Mode::First {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("aspd".into(), n(bb, "attack_speed", 0.)),
        ]
        .into();
    } else {
        s.modifiers = [(
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        )]
        .into();
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Sntlla(Sntlla {
        mode,
        scale: n(bb, "attack@atk_scale", 1.),
        cold: n(bb, "attack@cold", 0.),
        delay: n(t, "interval", 20.),
        attack: n(t, "atk", 0.),
        resist: (-n(t, "one_minus_status_resistance", 0.)).clamp(0., 1.),
    });
    Ok(())
}

pub(crate) struct Sntlla {
    pub mode: Mode,
    pub scale: f64,
    pub cold: f64,
    pub delay: f64,
    pub attack: f64,
    pub resist: f64,
}

pub(crate) struct State {
    pub ready: f64,
    pub row: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ready: f64::NEG_INFINITY,
            row: Default::default(),
        }
    }
}

impl Host {
    pub(in crate::battle::runtime) fn sntlla_start(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.sntlla_mut()?.ready = 0.;
        w.unit_mut(id)?.operator.tactical.sntlla_mut()?.row = 0;
        Ok(())
    }
    pub(in crate::battle::runtime) fn sntlla_skill_tick(
        &mut self,
        w: &mut World,
        id: Id,
        dt: f64,
        p: &Sntlla,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            let ready = w.unit(id)?.operator.tactical.sntlla()?.ready;
            let next = if ready <= 0. {
                ready
            } else if ready <= dt + 1e-9 {
                0.
            } else {
                ready - dt
            };
            w.unit_mut(id)?.operator.tactical.sntlla_mut()?.ready = next;
            if next <= 0.
                && skills::can_act(w, id)?
                && !Self::tac_has(w, id, "disarm")
                && !self.tac_enemies(w, id, None, true)?.is_empty()
            {
                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                let tile = w.unit(id)?.deployment.tile;
                let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
                let row = w.unit(id)?.operator.tactical.sntlla()?.row;
                let mut picked = None;
                for step in 0..3 {
                    let index = (row + step) % 3;
                    let lat = [1, -1, 0][index];
                    let candidates: Vec<_> = keys
                        .iter()
                        .copied()
                        .filter(|&k| {
                            dir.local([(k / COLS) as i32 - tile[0], (k % COLS) as i32 - tile[1]])[0]
                                == lat
                        })
                        .collect();
                    if !candidates.is_empty() {
                        let index_pick =
                            (self.rng.next_f64() * candidates.len() as f64).floor() as usize;
                        picked = Some(candidates[index_pick]);
                        w.unit_mut(id)?.operator.tactical.sntlla_mut()?.row = (index + 1) % 3;
                        break;
                    }
                }
                if let Some(k) = picked {
                    let interval = Self::tac_stat(w, id, "interval")?;
                    w.unit_mut(id)?.operator.tactical.sntlla_mut()?.ready = interval;
                    w.unit_mut(id)?.attacks += 1;
                    w.unit_mut(id)?.last_attack_at = w.time;
                    self.tac_timer(
                        w,
                        id,
                        w.time + 0.3,
                        0.,
                        1,
                        Task::Icicle {
                            position: [(k / COLS) as f64, (k % COLS) as f64],
                            scale: p.scale,
                            cold: p.cold,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn sntlla_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Sntlla,
    ) -> Result<(), String> {
        if due[3]
            && w.time - w.unit(id)?.deployment.deployed_at >= p.delay - 1e-9
            && !Self::tac_has(w, id, "sntlla:born")
        {
            self.component_buff(
                w,
                r,
                id,
                "sntlla:born",
                [("atkPct".into(), p.attack)].into(),
                f64::INFINITY,
                id,
            )?;
            if p.resist > 0. {
                self.advanced_status(w, r, id, id, "resist", f64::INFINITY, Some(p.resist))?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn sntlla_icicle_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let id = t.source.ok_or("Operator timer requires a source")?;
        let Task::Icicle {
            position,
            scale,
            cold,
        } = &mut t.kind
        else {
            return Err("Invalid icicle timer".into());
        };

        let targets = self.tac_radius(w, *position, 1.5)?;
        for a in targets {
            let amount = Self::tac_stat(w, id, "atk")? * *scale;
            self.tac_damage(w, r, id, a, amount, DamageType::Arts, true, "iceSpike")?;
            if Self::tac_up(w, a) && *cold > 0. {
                self.advanced_status(w, r, id, a, "cold", *cold, None)?;
            }
        }

        Ok(true)
    }
}

impl Behavior for Sntlla {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_))
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                if k.up() {
                    k.host.sntlla_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.sntlla_start(k.world, id)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                k.host.sntlla_skill_tick(k.world, id, context.dt, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
