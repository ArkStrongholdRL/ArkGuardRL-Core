// SPDX-License-Identifier: GPL-3.0-or-later
//! Equipment that owns death, skill-window or stealth-growth state.
use super::*;
use crate::battle::{
    damage::DamageType,
    equipment::{Effect, Instance},
    skills, targeting,
};

impl Host {
    pub(super) fn chivalry_skill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
        index: usize,
        instance: &Instance,
        event: skills::Event,
        context: &skills::Context,
        skill: &skills::Runtime,
    ) -> Result<(), String> {
        let Effect::Chivalry {
            duration,
            attack,
            speed,
            movement,
            next,
            mut until,
            mut combo,
            mut doomed,
        } = instance.effect
        else {
            return Ok(());
        };
        let key = format!("{}:combo", instance.tag);
        if event == skills::Event::SkillStart && self.equipment_member(unit, "kazimierzShip") {
            until = w.time + duration;
            if self.equipment_partner(unit, instance) && skill.bank.kind.timed() {
                combo = true;
                doomed = false;
                self.component_buff(
                    w,
                    r,
                    unit,
                    &key,
                    [("atkPct".into(), attack)].into(),
                    f64::INFINITY,
                    unit,
                )?;
            }
        }
        if event == skills::Event::SkillEnd && combo {
            combo = false;
            r.remove(w, self, unit, &key)?;
            if doomed && context.reason != "death" {
                doomed = false;
                self.tac_timer(
                    w,
                    unit,
                    w.time,
                    0.,
                    1,
                    tactics::Task::Callback(|c| c.retreat()),
                )?;
            }
        }
        self.equipment.units.get_mut(&unit).unwrap().effects[index].effect = Effect::Chivalry {
            duration,
            attack,
            speed,
            movement,
            next,
            until,
            combo,
            doomed,
        };
        Ok(())
    }
    pub(super) fn chivalry_fatal(&mut self, event: &mut health::Event) -> Result<(), String> {
        if event.prevented {
            return Ok(());
        }
        let Some(loadout) = self.equipment.units.get_mut(&event.target) else {
            return Ok(());
        };
        for instance in &mut loadout.effects {
            if let Effect::Chivalry {
                combo: true,
                doomed,
                ..
            } = &mut instance.effect
            {
                *doomed = true;
                event.prevented = true;
                break;
            }
        }
        Ok(())
    }
    pub(super) fn equipment_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
    ) -> Result<(), String> {
        for (index, instance) in self.equipment.effects(unit) {
            if let Effect::Crest {
                per,
                cap,
                scale,
                next,
                deployment,
                ..
            } = instance.effect
            {
                self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                    Effect::Crest {
                        per,
                        cap,
                        scale,
                        next,
                        deployment,
                        bonus: 0.,
                    };
            }
            if let Effect::Cocoon { cap, used } = instance.effect {
                let a = w.unit(unit)?;
                if used >= cap
                    || a.stats.alive
                    || a.removed
                    || a.deployment.remove_reason.as_deref() != Some("killed")
                {
                    continue;
                }
                let revived = kits::dispatch::Context {
                    host: self,
                    world: w,
                    buffs: r,
                    unit,
                }
                .redeploy_free(unit)?;
                if revived {
                    self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                        Effect::Cocoon {
                            cap,
                            used: used + 1,
                        };
                }
            }
        }
        Ok(())
    }
    pub(super) fn equipment_special_periodic(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
        index: usize,
        instance: &Instance,
    ) -> Result<(), String> {
        match instance.effect {
            Effect::Chivalry {
                duration,
                attack,
                speed,
                movement,
                next,
                until,
                combo,
                doomed,
            } => {
                self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                    Effect::Chivalry {
                        duration,
                        attack,
                        speed,
                        movement,
                        next: next + 0.5,
                        until,
                        combo,
                        doomed,
                    };
                if w.time >= until || !Self::tac_up(w, unit) {
                    return Ok(());
                }
                for enemy in self.equipment_targets(
                    w,
                    unit,
                    targeting::Profile {
                        can_hit_fly: true,
                        ..Default::default()
                    },
                )? {
                    let aspd = w.unit(enemy)?.stats.base_number("aspd");
                    self.component_buff(
                        w,
                        r,
                        enemy,
                        &format!("{}:aura", instance.tag),
                        [
                            ("aspd".into(), -(1. - speed) * aspd),
                            ("moveMul".into(), movement),
                        ]
                        .into(),
                        0.6,
                        unit,
                    )?;
                }
            }
            Effect::Crest {
                per,
                cap,
                scale,
                next,
                mut bonus,
                mut deployment,
            } => {
                let current = w.unit(unit)?.deployment.deploy_seq;
                if current != deployment {
                    bonus = 0.;
                    deployment = current;
                }
                if Self::tac_up(w, unit)
                    && self.equipment_member(unit, "siracusaShip")
                    && Self::tac_stat_flag(w, unit, "stealth")?
                    && bonus < cap
                {
                    bonus = (bonus + per * 0.25).min(cap);
                    self.component_buff(
                        w,
                        r,
                        unit,
                        &format!("{}:stealth", instance.tag),
                        [("atkPct".into(), bonus)].into(),
                        f64::INFINITY,
                        unit,
                    )?;
                }
                self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                    Effect::Crest {
                        per,
                        cap,
                        scale,
                        next: next + 0.25,
                        bonus,
                        deployment,
                    };
            }
            _ => (),
        }
        Ok(())
    }
    fn tac_stat_flag(w: &mut World, unit: Id, flag: &str) -> Result<bool, String> {
        Ok(w.unit_mut(unit)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains(flag))
    }
    pub(super) fn equipment_crest_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: &health::Event,
        q: &hit::Request,
    ) -> Result<(), String> {
        let Some(source) = event.source else {
            return Ok(());
        };
        if event.amount <= 0.
            || q.tags.iter().any(|tag| tag == "item")
            || !Self::tac_up(w, source)
            || Self::tac_stat_flag(w, source, "stealth")?
        {
            return Ok(());
        }
        for (index, instance) in self.equipment.effects(source) {
            if let Effect::Crest {
                per,
                cap,
                scale,
                next,
                bonus,
                deployment,
            } = instance.effect
            {
                if bonus <= 0. || deployment != w.unit(source)?.deployment.deploy_seq {
                    continue;
                }
                self.equipment.units.get_mut(&source).unwrap().effects[index].effect =
                    Effect::Crest {
                        per,
                        cap,
                        scale,
                        next,
                        bonus: 0.,
                        deployment,
                    };
                let target = w.unit(event.target)?;
                if self.equipment_partner(source, &instance)
                    && target.stats.alive
                    && target.side == health::Side::Enemy
                {
                    self.equipment_damage(
                        w,
                        r,
                        source,
                        event.target,
                        scale,
                        DamageType::True,
                        "crest",
                        false,
                    )?;
                }
                r.remove(w, self, source, &format!("{}:stealth", instance.tag))?;
            }
        }
        Ok(())
    }
}
