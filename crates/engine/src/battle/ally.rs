// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol ai.js updateAlly. The loop owns attack/control/retarget
// ordering; the content host dispatches the concrete attack payload.
use super::{
    blocking, cadence,
    health::{Id, World},
    search::{self, AttackProfile, Commitments, Selection},
    skills::{self, Runtime},
};
pub trait Host: search::Host + skills::Hooks + blocking::Hooks {
    fn effective_profile(
        &mut self,
        id: Id,
        skill: Option<&Runtime>,
        w: &mut World,
    ) -> Result<Option<AttackProfile>, String>;
    fn range_changed(&mut self, id: Id, w: &mut World) -> Result<bool, String>;
    fn refresh_range(&mut self, id: Id, w: &mut World) -> Result<(), String>;
    fn store_energy(&mut self, id: Id, p: AttackProfile, w: &mut World) -> Result<bool, String>;
    fn has_store_energy(&self, id: Id, p: AttackProfile) -> bool;
    /// Executes payloads and then attack hooks, skill.after_attack and afterAttack
    /// in that order. Mandatory: no default that silently skips an attack.
    fn perform(
        &mut self,
        id: Id,
        p: AttackProfile,
        selection: Selection,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String>;
}
fn interval(w: &mut World, id: Id) -> Result<f64, String> {
    w.unit_mut(id)?
        .stats
        .stats()
        .map(|s| s.n("interval"))
        .map_err(str::to_string)
}
fn store<H: Host>(w: &mut World, h: &mut H, id: Id, p: AttackProfile) -> Result<(), String> {
    if h.has_store_energy(id, p) && h.store_energy(id, p, w)? {
        let duration = interval(w, id)?;
        w.unit_mut(id)?.cadence.complete_attack(duration);
    }
    Ok(())
}
fn acquire<H: Host>(
    w: &mut World,
    h: &mut H,
    id: Id,
    p: AttackProfile,
    limit: usize,
) -> Result<Selection, String> {
    let keys = search::Host::range(h, id, w)?;
    search::acquire(w, h, id, &keys, p, limit)
}
pub fn update<H: Host>(
    w: &mut World,
    h: &mut H,
    cache: &Commitments,
    id: Id,
    mut skill: Option<&mut Runtime>,
    dt: f64,
    limit: usize,
) -> Result<(), String> {
    cadence::tick_unit(w, id, dt)?;
    if !w.unit(id)?.blocking().is_empty() {
        blocking::enforce_capacity(w, h, id)?;
    }
    if !skills::can_act(w, id)? {
        return Ok(());
    }
    let Some(_) = h.effective_profile(id, skill.as_deref(), w)? else {
        return Ok(());
    };
    if h.range_changed(id, w)? {
        h.refresh_range(id, w)?;
    }
    let Some(mut p) = h.effective_profile(id, skill.as_deref(), w)? else {
        return Ok(());
    };
    if p.no_attack
        || p.only_while_skill && !skill.as_deref().map(|s| s.bank.active).unwrap_or(false)
    {
        return Ok(());
    }
    if w.unit_mut(id)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("disarm")
        || w.unit(id)?.cadence.cooldown > 0.0
    {
        return Ok(());
    }
    if !search::Host::can_attack(h, id, p, w)? {
        if h.has_store_energy(id, p) {
            w.unit_mut(id)?.cadence.had_target = Some(false);
            store(w, h, id, p)?;
        }
        return Ok(());
    }
    let mut selected = acquire(w, h, id, p, limit)?;
    let trigger_allies = skill
        .as_deref()
        .map(|s| s.spec.trigger_allies)
        .unwrap_or(false);
    if selected.targets.is_empty()
        && trigger_allies
        && if let Some(s) = skill.as_deref_mut() {
            s.about_to_attack(w, h)?
        } else {
            false
        }
    {
        let Some(next) = h.effective_profile(id, skill.as_deref(), w)? else {
            return Err("Skill cast removed the attack profile".into());
        };
        p = next;
        if p.no_attack || !w.unit(id)?.stats.alive {
            return Ok(());
        }
        selected = acquire(w, h, id, p, limit)?;
    } else if selected.targets.is_empty() {
        let committed = cache.targets(w, h, id, p)?;
        if !committed.is_empty() {
            selected.targets = committed;
        }
    }
    if selected.targets.is_empty() {
        w.unit_mut(id)?.cadence.had_target = Some(false);
        store(w, h, id, p)?;
        return Ok(());
    }
    w.unit_mut(id)?.cadence.had_target = Some(true);
    if if let Some(s) = skill.as_deref_mut() {
        s.about_to_attack(w, h)?
    } else {
        false
    } {
        let Some(next) = h.effective_profile(id, skill.as_deref(), w)? else {
            return Err("Skill cast removed the attack profile".into());
        };
        p = next;
        if p.no_attack || !w.unit(id)?.stats.alive {
            return Ok(());
        }
        selected = acquire(w, h, id, p, limit)?;
        if selected.targets.is_empty() {
            store(w, h, id, p)?;
            return Ok(());
        }
    }
    h.perform(id, p, selected, skill, w)?;
    let duration = interval(w, id)?;
    w.unit_mut(id)?.cadence.complete_attack(duration);
    Ok(())
}
