// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) struct Wolf {
    interval: f64,
    block: f64,
    guard: f64,
    pub taunt: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub count: u32,
    return_at: Option<f64>,
    next_grow: f64,
    returning: bool,
}
pub(crate) fn build(raw: &Value, s: &mut Spec) {
    let talents = raw["talents"].as_array().cloned().unwrap_or_default();
    let talent = talents.first().map(|t| &t["bb"]).unwrap_or(&Value::Null);
    let find = |key: &str, fallback: f64| {
        talents
            .iter()
            .find_map(|t| t["bb"][key].as_f64())
            .unwrap_or(fallback)
    };
    s.extra.tactics = Tactics::Wolf(Wolf {
        interval: n(
            talent,
            "vigil_wolf_t_1_enhance[trigger].interval",
            n(talent, "interval", 25.),
        ),
        block: n(
            talent,
            "vigil_wolf_t_1_enhance[trigger].block_cnt",
            n(talent, "block_cnt", 1.),
        ),
        guard: find("damage_scale", 1.),
        taunt: find("taunt_level", 0.),
    });
}
impl State {
    pub(crate) fn redeployed(previous: &Self) -> Self {
        Self {
            returning: previous.returning,
            ..Default::default()
        }
    }
}
impl Wolf {
    fn apply(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let count = k.world.unit(k.unit)?.operator.tactical.wolf()?.count;
        k.buff(
            k.unit,
            "token:wolves",
            [("blockCnt".into(), self.block * count as f64)].into(),
            f64::INFINITY,
        )?;
        Ok(())
    }
}
impl Behavior for Wolf {
    fn payload(
        &self,
        w: &World,
        id: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        payload.hits = w.unit(id)?.operator.tactical.wolf()?.count.max(1);
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                let now = k.world.time;
                let returning = k.world.unit(id)?.operator.tactical.wolf()?.returning;
                let state = k.world.unit_mut(id)?.operator.tactical.wolf_mut()?;
                state.count = if returning { 1 } else { 2 };
                state.returning = false;
                state.return_at = None;
                state.next_grow = now + self.interval;
                self.apply(k)?;
            }
            KitEvent::Fatal(context)
                if context.target == id
                    && !context.prevented
                    && k.world.unit(id)?.operator.tactical.wolf()?.count > 1 =>
            {
                k.world.unit_mut(id)?.operator.tactical.wolf_mut()?.count -= 1;
                self.apply(k)?;
                context.prevented = true;
                let maximum = k.stat(id, "maxHp")?;
                k.world.unit_mut(id)?.stats.hp = maximum;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.target == id
                && request.hit.source.is_some_and(|source| {
                    k.world
                        .unit(source)
                        .is_ok_and(|u| u.blocked_by() == Some(id))
                }) =>
            {
                request.damage.mul *= self.guard
            }
            KitEvent::Death { unit, .. } if *unit == id => {
                let reason = k.world.unit(id)?.deployment.remove_reason.as_deref();
                let owner = k.world.unit(id)?.summoner;
                if matches!(reason, Some("killed" | "retreat" | "forced"))
                    && owner.is_none_or(|owner| Host::tac_up(k.world, owner))
                {
                    k.world.unit_mut(id)?.removed = false;
                    let at = k.world.time + self.interval;
                    let state = k.world.unit_mut(id)?.operator.tactical.wolf_mut()?;
                    state.count = 0;
                    state.return_at = Some(at);
                    let mut buff = Buff {
                        key: Some("token:wolves".into()),
                        mods: [("blockCnt".into(), 0.)].into(),
                        ..Default::default()
                    };
                    buff.runtime.source = Some(id);
                    k.buffs.add(
                        k.world,
                        k.host,
                        id,
                        buffs::Spec {
                            buff,
                            allow_dead: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
            KitEvent::Death { unit, .. } if Some(*unit) == k.world.unit(id)?.summoner => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .wolf_mut()?
                    .return_at = None;
                if k.up() {
                    k.retreat()?;
                }
            }
            KitEvent::Scheduled(_) => {
                if k.up()
                    && k.world.time + 1e-9 >= k.world.unit(id)?.operator.tactical.wolf()?.next_grow
                {
                    let state = k.world.unit_mut(id)?.operator.tactical.wolf_mut()?;
                    state.next_grow += self.interval;
                    if state.count < 3 {
                        state.count += 1;
                        self.apply(k)?;
                    }
                }
                if k.world
                    .unit(id)?
                    .operator
                    .tactical
                    .wolf()?
                    .return_at
                    .is_some_and(|at| k.world.time + 1e-9 >= at)
                {
                    if k.world
                        .unit(id)?
                        .summoner
                        .is_some_and(|owner| !Host::tac_up(k.world, owner))
                    {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .wolf_mut()?
                            .return_at = None;
                    } else {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .wolf_mut()?
                            .returning = true;
                        if !k.redeploy_free(id)? {
                            let at = k.world.time + 0.25;
                            k.world
                                .unit_mut(id)?
                                .operator
                                .tactical
                                .wolf_mut()?
                                .return_at = Some(at);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
