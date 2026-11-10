// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Context as KitContext, Event as KitEvent};
use super::*;
#[derive(Default)]
pub(crate) struct State {
    pub doll: bool,
    pub pending_end: bool,
    pub switching: bool,
}
const SWITCH: u64 = 1520;
const SUBSTITUTE: u64 = 1521;
pub(crate) fn switch(k: &mut KitContext<'_>) -> Result<(), String> {
    k.host.growth_doll_swap(k.world, k.unit)?;
    if !k.end_skill("substitute")? {
        k.world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .dollkeeper_mut()?
            .pending_end = true;
    }
    let statuses: Vec<_> = k
        .world
        .unit(k.unit)?
        .stats
        .buffs()
        .iter()
        .filter(|b| b.runtime.status.is_some())
        .map(|b| b.runtime.id())
        .collect();
    for buff in statuses {
        k.buffs.remove_id(k.world, k.host, k.unit, buff)?;
    }
    k.world
        .unit_mut(k.unit)?
        .operator
        .tactical
        .dollkeeper_mut()?
        .switching = true;
    let mut buff = Buff {
        key: Some("trait:dollSwitching".into()),
        flags: [
            "invulnerable",
            "noSp",
            "noHeal",
            "healFree",
            "isolated",
            "disarm",
        ]
        .map(str::to_string)
        .into(),
        ..Default::default()
    };
    buff.runtime.duration = 1.;
    buff.runtime.on_expire = Some(SWITCH);
    buff.runtime.on_remove = Some(SWITCH);
    k.add(k.unit, buff)?;
    let maximum = k.stat(k.unit, "maxHp")?;
    k.world.unit_mut(k.unit)?.stats.hp = maximum;
    Ok(())
}
pub(crate) fn enter(
    k: &mut KitContext<'_>,
    duration: f64,
    hp: f64,
    no_attack: bool,
) -> Result<bool, String> {
    if !k.up() || k.world.unit(k.unit)?.operator.tactical.dollkeeper()?.doll {
        return Ok(false);
    }
    k.world
        .unit_mut(k.unit)?
        .operator
        .tactical
        .dollkeeper_mut()?
        .doll = true;
    switch(k)?;
    let mut buff = Buff {
        key: Some("trait:substitute".into()),
        mods: [("blockCnt".into(), -99.), ("hpMul".into(), hp)].into(),
        flags: if no_attack {
            ["noSp", "disarm", "silence"].map(str::to_string).into()
        } else {
            ["noSp".to_string()].into()
        },
        ..Default::default()
    };
    buff.runtime.duration = 1. + duration;
    buff.runtime.on_expire = Some(SUBSTITUTE);
    buff.runtime.on_remove = Some(SUBSTITUTE);
    k.add(k.unit, buff)?;
    k.release()?;
    let maximum = k.stat(k.unit, "maxHp")?;
    k.world.unit_mut(k.unit)?.stats.hp = maximum;
    k.host.bond_doll_swap(k.world, k.unit)?;
    Ok(true)
}
pub(crate) fn resume(k: &mut KitContext<'_>, skill: &mut skills::Runtime) -> Result<(), String> {
    if std::mem::take(
        &mut k
            .world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .dollkeeper_mut()?
            .pending_end,
    ) {
        k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
            skill.end(world, host, "substitute")
        })?;
    }
    Ok(())
}
pub(crate) fn handle(k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
    let id = k.unit;
    match event {
        KitEvent::Buff {
            context,
            event,
            token,
        } if *token == SWITCH || *token == SUBSTITUTE => {
            if k.buffs
                .get(k.world, context.unit, context.buff)?
                .runtime
                .source
                != Some(id)
            {
                return Ok(());
            }
            if *token == SWITCH {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .dollkeeper_mut()?
                    .switching = false;
                if *event == buffs::Event::Expire && k.up() {
                    let maximum = k.stat(id, "maxHp")?;
                    k.world.unit_mut(id)?.stats.hp = maximum;
                }
            } else if k.world.unit(id)?.operator.tactical.dollkeeper()?.doll {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .dollkeeper_mut()?
                    .doll = false;
                if k.up() {
                    switch(k)?;
                    k.host.bond_doll_swap(k.world, id)?;
                }
            }
        }
        KitEvent::Death { unit, .. } if *unit == id => {
            *k.world.unit_mut(id)?.operator.tactical.dollkeeper_mut()? = State::default();
        }
        _ => {}
    }
    Ok(())
}
