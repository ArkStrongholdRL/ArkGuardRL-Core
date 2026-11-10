// SPDX-License-Identifier: GPL-3.0-or-later
//! Healing, area statuses and damage deferral assembled by individual Kits.
use super::*;
pub(crate) const HP_DRAIN: u64 = 1201;
pub(crate) const PROTEGE_COUNTER: u64 = 1202;
pub(crate) const PLASMA_DRAIN: u64 = 1203;
pub(crate) enum SpChange {
    Gain(f64),
    Bond(f64),
    Clear,
    Damaged,
}
#[derive(Default)]
pub(crate) struct State {
    pub protege: Option<Id>,
    pub debt: f64,
    pub fragile_scale: f64,
    pub next_aura: Vec<f64>,
    pub next_lavender: f64,
    pub next_sp_aura: f64,
    pub next_support_aura: f64,
    pub next_phalanx: f64,
    pub next_forge: f64,
    pub next_block_stats: f64,
    pub next_reaper: f64,
    pub reaper_window: f64,
    pub reaper_limit: usize,
    pub reaper_accepted: usize,
    pub reaper_queue: std::collections::VecDeque<f64>,
    pub quiet_until: f64,
    pub bandage: Option<Id>,
    pub sp_changes: Vec<SpChange>,
}
#[derive(Default)]
pub(crate) struct Support {
    pub auto_heal: bool,
    pub sp_period: Option<f64>,
    pub sp_duration: Option<f64>,
    pub sp_profession: Option<&'static str>,
    pub radial_push: Option<f64>,
    pub reveal: bool,
    pub effect_push: bool,
    pub pull_unblocked: bool,
    pub heal_bonus: Option<HealBonus>,
    pub received_heal: Option<ReceivedHeal>,
    pub top_heal: f64,
    pub incantation: f64,
    pub protege: Option<Protege>,
    pub reaper: f64,
    pub overflow: f64,
    pub skill_heal: f64,
    pub skill_drain_heal: f64,
    pub peaks: Vec<[f64; 2]>,
    pub cook: Option<Cook>,
    pub lavender: f64,
    pub sp_aura: f64,
    pub support_atk: f64,
    pub sp_near: f64,
    pub zone: Option<Zone>,
    pub quiet: Option<Quiet>,
    pub sluggish: Option<[f64; 2]>,
    pub weaken: Option<[f64; 2]>,
    pub enemy_auras: Vec<EnemyAura>,
    pub phalanx: Option<Phalanx>,
    pub push: Option<PushCast>,
    pub inward: Option<f64>,
    pub heavy_penetration: Option<[f64; 2]>,
    pub refund: f64,
    pub death_sp: Option<[f64; 2]>,
    pub bandage: Option<f64>,
    pub plasma: Option<Plasma>,
    pub deferred: Option<Deferred>,
    pub block_stats: Option<[f64; 4]>,
    pub forge: Option<Numbers>,
    pub elite_scale: Option<f64>,
}
#[derive(Clone, Copy)]
pub(crate) struct HealBonus {
    pub below: f64,
    pub scale: f64,
    pub inclusive: bool,
}
pub(crate) struct ReceivedHeal {
    pub below: f64,
    pub normal: f64,
    pub low: f64,
}
pub(crate) struct Protege {
    pub taunt: f64,
    pub scale: f64,
}
pub(crate) struct Cook {
    pub duration: f64,
    pub defence: f64,
    pub attack: f64,
}
pub(crate) struct Zone {
    pub duration: f64,
    pub scale: f64,
}
pub(crate) struct Quiet {
    pub duration: f64,
    pub count: usize,
}
pub(crate) struct Plasma {
    pub duration: f64,
    pub interval: f64,
    pub ratio: f64,
    pub attack: f64,
}
pub(crate) struct Deferred {
    pub resistance: f64,
    pub duration: f64,
    pub interval: f64,
}
pub(crate) struct Phalanx {
    pub guard: Numbers,
    pub keep: Numbers,
    pub taunt: f64,
    pub aura: StatAura,
    pub burst: f64,
}
pub(crate) struct PushCast {
    pub grid: Vec<[i32; 2]>,
    pub force: f64,
    pub direct: f64,
    pub wall: f64,
    pub collision: f64,
}
pub(crate) enum AuraFilter {
    All,
    Normal,
    LowHp(f64),
}
pub(crate) struct EnemyAura {
    pub key: &'static str,
    pub mods: Numbers,
    pub filter: AuraFilter,
    pub period: f64,
    pub active_only: bool,
    pub growth: Option<[f64; 2]>,
}
pub(crate) fn healing(s: &mut Spec, count: f64) {
    s.combat.heal = true;
    s.combat.projectile = None;
    s.combat.profile.heal = Some(crate::battle::search::Heal {
        multi: count > 1.,
        count,
        hp_at_most: 1.,
        element_heal: false,
    });
    s.combat.profile.heal_attack = true;
}
pub(crate) fn skill_healing(s: &mut Spec, count: f64, scale: f64) {
    s.combat.skill_heal = true;
    s.combat.skill_heal_scale = scale;
    let mut p = s.combat.profile;
    p.heal = Some(crate::battle::search::Heal {
        multi: count > 1.,
        count,
        hp_at_most: 1.,
        element_heal: false,
    });
    p.heal_attack = true;
    s.combat.skill_profile = Some(p);
}
pub(super) fn slow_sp(raw: &Value, s: &mut Spec) {
    s.extra.support.sp_near = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["hidden"] == true)
        .map(|t| number(&t["bb"]["sp_recovery_per_sec"], 0.))
        .sum();
}
pub(super) fn debuffer(raw: &Value, s: &mut Spec) {
    let tb = &raw["trait"]["bb"];
    if !tb["atk"].is_null() && !tb["duration"].is_null() {
        s.extra.support.weaken = Some([number(&tb["atk"], 0.).abs(), number(&tb["duration"], 2.)]);
    }
}
