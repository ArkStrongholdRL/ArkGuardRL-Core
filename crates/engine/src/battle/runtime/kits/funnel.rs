// SPDX-License-Identifier: GPL-3.0-or-later
//! An attached drone keeps its launch-time profile and advances its ramp on impact.
use super::*;
#[derive(Clone, Copy)]
pub(crate) struct Parameters {
    pub initial: f64,
    pub delta: f64,
    pub cap: f64,
}
impl Parameters {
    pub fn read(raw: &Value) -> Self {
        let tb = &raw["trait"]["bb"];
        Self {
            initial: n(tb, "init_atk_scale", 0.2),
            delta: n(tb, "delta_atk_scale", 0.15),
            cap: n(tb, "max_atk_scale", 1.1),
        }
    }
    pub fn attachment(self, count: u32, per_target: bool) -> attacks::Attachment {
        attacks::Attachment {
            count,
            initial: self.initial,
            delta: self.delta,
            cap: self.cap,
            per_target,
            scale: 0.,
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct State {
    pub target: Option<Id>,
    pub scale: f64,
    pub map: BTreeMap<Id, (f64, f64)>,
}
impl State {
    pub fn step(&mut self, target: Id, time: f64, parameters: Parameters, per_target: bool) -> f64 {
        if per_target {
            let scale = match self.map.get(&target) {
                Some(&(scale, at)) if at == time => return scale,
                Some(&(scale, _)) => (scale + parameters.delta).min(parameters.cap),
                None => parameters.initial,
            };
            self.map.insert(target, (scale, time));
            scale
        } else {
            self.scale = if self.target == Some(target) {
                (self.scale + parameters.delta).min(parameters.cap)
            } else {
                parameters.initial
            };
            self.target = Some(target);
            self.scale
        }
    }
}
impl Host {
    pub(in crate::battle::runtime) fn funnel_prepare(
        &mut self,
        world: &mut World,
        source: Id,
        target: Option<Id>,
        payload: &mut attacks::Direct,
    ) -> Result<(), String> {
        let Some(mut attachment) = payload.attachment else {
            return Ok(());
        };
        let Some(target) = target.filter(|&id| world.unit(id).is_ok_and(|u| u.stats.alive)) else {
            return Ok(());
        };
        let def = self.tac_def(source)?;
        if let Tactics::Malist(p) = &def.extra.tactics {
            if self.rng.chance(p.prob) {
                payload.atk_scale *= p.crit;
            }
        }
        let time = world.time;
        let count = match &def.extra.tactics {
            Tactics::Malist(_) => world
                .unit(source)?
                .operator
                .tactical
                .malist()?
                .funnel
                .map
                .len(),
            Tactics::Kjera(_) => world
                .unit(source)?
                .operator
                .tactical
                .kjera()?
                .funnel
                .map
                .len(),
            _ => 0,
        };
        let valid = if count >= world.unit_capacity() {
            Some(world.unit_ids().collect::<BTreeSet<_>>())
        } else {
            None
        };
        let state = match &def.extra.tactics {
            Tactics::Whitw2(_) => {
                &mut world
                    .unit_mut(source)?
                    .operator
                    .tactical
                    .whitw2_mut()?
                    .funnel
            }
            Tactics::Rockr(_) => {
                &mut world
                    .unit_mut(source)?
                    .operator
                    .tactical
                    .rockr_mut()?
                    .funnel
            }
            Tactics::Malist(_) => {
                &mut world
                    .unit_mut(source)?
                    .operator
                    .tactical
                    .malist_mut()?
                    .funnel
            }
            Tactics::Kjera(_) => {
                &mut world
                    .unit_mut(source)?
                    .operator
                    .tactical
                    .kjera_mut()?
                    .funnel
            }
            _ => return Err("Attached drone has no funnel owner".into()),
        };
        if let Some(valid) = valid {
            state.map.retain(|id, _| valid.contains(id));
        }
        attachment.scale = state.step(
            target,
            time,
            Parameters {
                initial: attachment.initial,
                delta: attachment.delta,
                cap: attachment.cap,
            },
            attachment.per_target,
        );
        payload.attachment = Some(attachment);
        Ok(())
    }
    pub(in crate::battle::runtime) fn funnel_damage(
        &mut self,
        world: &mut World,
        source: Id,
        target: Id,
        attack_id: u64,
        payload: attacks::Direct,
    ) -> Result<f64, String> {
        let Some(attachment) = payload.attachment else {
            return Ok(0.);
        };
        if attachment.count > crate::battle::operators::Combat::MAX_HITS {
            return Err("Attached drone hit capacity exceeded".into());
        }
        let mut buffs = self.buffs.take().ok_or("Missing attached drone buffs")?;
        let result = (|| {
            let mut total = 0.;
            for _ in 0..attachment.count {
                if !world.unit(target)?.stats.alive {
                    break;
                }
                let amount = Self::tac_stat(world, source, "atk")?
                    * (payload.atk_scale * Self::tac_stat(world, source, "atkScaleMul")?)
                    * payload.skill_damage_mul
                    * payload.target_damage_mul
                    * attachment.scale;
                let mut request = hit::Request::new(
                    health::Hit {
                        source: Some(source),
                        credit: Some(source),
                        target,
                        damage_type: DamageType::Arts,
                        no_sp: false,
                        silent: false,
                        hp_loss: false,
                        on_applied: false,
                    },
                    Damage {
                        amount,
                        mul: 1.,
                        resolved: false,
                        penetration: Default::default(),
                    },
                );
                request.is_skill = payload.is_skill;
                request.attack_id = Some(attack_id);
                request.attack_type = hit::AttackType::Addition;
                request.tags = vec!["addition".into(), "funnel".into()];
                let dealt = self.resolve_hit(world, &mut buffs, &mut request);
                total += dealt?;
            }
            Ok(total)
        })();
        self.buffs = Some(buffs);
        result
    }
}
