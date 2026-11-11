// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::deploy;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "bard",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_skadi2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "SP_FULL",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_skadi2_3" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    s.combat.profile.no_attack = true;
    s.combat.projectile = None;
    s.extra.tactics = Tactics::Skadi2(Skadi2 {
        regen: n(tb, "attack@atk_to_hp_recovery_ratio", 0.1),
        inspire: n(bb, "atk", 0.),
        damage: n(bb, "atk_scale", 0.),
        loss: n(bb, "hp_ratio", 0.),
        nearby_atk: n(
            t,
            "skadi2_e_003_t_2[atk][1].atk",
            n(t, "skadi2_t_2[atk][1].atk", 0.),
        ),
        abyssal_atk: n(
            t,
            "skadi2_e_003_t_2[atk][2].atk",
            n(t, "skadi2_t_2[atk][2].atk", 0.),
        ),
        defence: n(t, "skadi2_e_003_t_2[def].def", 0.),
        module_count: n(tm, "cnt", f64::INFINITY),
        module_atk: n(tm, "atk", 0.),
    });
    Ok(())
}
pub(crate) struct Skadi2 {
    regen: f64,
    inspire: f64,
    damage: f64,
    loss: f64,
    nearby_atk: f64,
    abyssal_atk: f64,
    defence: f64,
    module_count: f64,
    module_atk: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pulses: u64,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            pulses: previous.map_or(0, |s| s.pulses),
        }
    }
}
impl Skadi2 {
    fn inspire(k: &mut KitContext<'_>, target: Id, value: f64) -> Result<(), String> {
        if value <= 0.
            || matches!(
                k.host.tac_def(target)?.extra.tactics,
                Tactics::Skadi2(_) | Tactics::Cetsyr(_)
            )
        {
            return Ok(());
        }
        let key = "inspire";
        if k.world.unit(target)?.stats.buffs().iter().any(|b| {
            b.key.as_deref() == Some(key)
                && b.runtime.source != Some(k.unit)
                && b.runtime.tick_payload > value
                && b.runtime.time_left > 0.1
        }) {
            return Ok(());
        }
        let modifiers = crate::battle::stats::Modifiers::aggregate_except(
            k.world.unit(target)?.stats.buffs(),
            key,
        )
        .map_err(str::to_string)?;
        let factor = (1. + modifiers.add.get("atkPct").copied().unwrap_or(0.)).max(0.)
            * modifiers.mul.get("atkMul").copied().unwrap_or(1.);
        let mut buff = Buff {
            key: Some(key.into()),
            mods: [(
                "atkFlat".into(),
                if factor > 1e-6 { value / factor } else { value },
            )]
            .into(),
            ..Default::default()
        };
        buff.runtime.duration = 0.75;
        buff.runtime.tick_payload = value;
        k.add(target, buff)?;
        Ok(())
    }
}
impl Behavior for Skadi2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        if let KitEvent::Scheduled(due) = event {
            if !due[3] || !k.up() {
                return Ok(());
            }
            let id = k.unit;
            let remove: Vec<_> = k
                .world
                .unit(id)?
                .stats
                .buffs()
                .iter()
                .filter(|b| {
                    b.key.as_ref().is_some_and(|key| {
                        key == "inspire" || key.starts_with("inspire:") || key.ends_with(":inspire")
                    })
                })
                .map(|b| b.runtime.id())
                .collect();
            for buff in remove {
                k.buffs.remove_id(k.world, k.host, id, buff)?;
            }
            let keys = k.keys()?;
            let allies = k.allies(&keys)?;
            let mut others = Vec::new();
            let mut abyssal = false;
            for &ally in &allies {
                if ally == id || k.world.unit(ally)?.deployment.kind != deploy::Kind::Operator {
                    continue;
                }
                others.push(ally);
                let def = k.host.tac_def(ally)?;
                abyssal |= [
                    "char_143_ghost",
                    "char_263_skadi",
                    "char_474_glady",
                    "char_4145_ulpia",
                    "char_1023_ghost2",
                    "char_218_cuttle",
                ]
                .contains(&def.char_id.as_str());
            }
            let attack = if abyssal {
                self.abyssal_atk
            } else if !others.is_empty() {
                self.nearby_atk
            } else {
                0.
            } + if others.len() as f64 >= self.module_count {
                self.module_atk
            } else {
                0.
            };
            let defence = if others.is_empty() { 0. } else { self.defence };
            k.world.unit_mut(id)?.operator.tactical.skadi2_mut()?.pulses += 1;
            let active = k.active()?;
            let atk = k.stat(id, "atk")?;
            if active {
                for &ally in &allies {
                    if ally != id {
                        Self::inspire(k, ally, atk * self.inspire)?;
                    }
                }
                if k.world.unit(id)?.operator.tactical.skadi2()?.pulses % 2 == 0 {
                    if self.damage > 0. {
                        for target in k.enemies(None, true)? {
                            k.damage(target, atk * self.damage, DamageType::True, true, "tide")?;
                        }
                    }
                    let loss = k.stat(id, "maxHp")? * self.loss;
                    if loss > 0. {
                        let hp = k.world.unit(id)?.stats.hp;
                        if hp - loss >= 1. {
                            k.lose_hp(loss)?;
                        } else {
                            k.world.unit_mut(id)?.stats.hp = hp.min(1.);
                        }
                    }
                }
            } else {
                for ally in allies {
                    if self.regen > 0. {
                        k.buff(
                            ally,
                            &format!("trait:bard:{id}"),
                            [("hpRegen".into(), atk * self.regen)].into(),
                            0.75,
                        )?;
                    }
                }
            }
            if attack > 0. || defence > 0. {
                k.buff(
                    id,
                    "skadi2:predator",
                    [("atkPct".into(), attack), ("defPct".into(), defence)].into(),
                    0.75,
                )?;
            }
        }
        Ok(())
    }
}
