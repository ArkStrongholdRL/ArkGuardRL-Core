// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed components assembled by individual Kits. Mutable counters stay per unit.
use super::*;
use crate::battle::damage::DamageType;
pub(crate) const ARMOR_BLOCK: u64 = 1101;
pub(crate) const INJURY_OBSERVER: u64 = 1102;
pub(crate) const SKILL_COUNTER: u64 = 1103;
#[derive(Default)]
pub(crate) struct Memory {
    pub support: super::support::State,
    pub advanced: AdvancedState,
    pub tactical: TacticalState,
    pub hits: u64,
    pub active_skill: bool,
    pub skill_death_pending: bool,
    pub deployments: u64,
    pub uses: usize,
    pub revived: bool,
    pub boosted: bool,
    pub cost_cut: f64,
    pub ammo_granted: bool,
    pub ammo_grants: std::collections::BTreeMap<Id, (u64, f64)>,
    pub next_pulse: f64,
    pub blessing_pending: bool,
}
pub(crate) struct Mechanics {
    pub ammo: Option<AmmoSupport>,
    pub ammo_count: f64,
    pub permanent: Vec<Buff>,
    pub deploy_buff: Option<Buff>,
    pub condition: Option<Conditional>,
    pub observer: Option<InjuryObserver>,
    pub skill_counter: Option<Counter>,
    pub aura: Option<StatAura>,
    pub team: Option<TeamBuff>,
    pub random_bless: Option<Numbers>,
    pub deploy_sp: f64,
    pub respawn_delta: f64,
    pub cost_cut: Option<CostCut>,
    pub burst: Option<Burst>,
    pub skill_debuff: Option<Debuff>,
    pub skill_pull: Option<PullHit>,
    pub skill_push: Option<f64>,
    pub start_status: Option<(Vec<[i32; 2]>, OnHitStatus)>,
    pub secondary: Option<Secondary>,
    pub end_heal: f64,
    pub blocked_taken: f64,
    pub resist: f64,
    pub crit: Option<Critical>,
    pub hit_upgrade: Option<u64>,
    pub attack_boost: Option<AttackBoost>,
    pub growing_atk: Option<Vec<f64>>,
    pub enemy_atk: Option<EnemyAttack>,
    pub distance: Option<DistanceScale>,
    pub fly_scale: f64,
    pub lord_skill_scale: f64,
    pub skill_def_ignore: f64,
    pub force_attack: bool,
    pub revive: Option<Revive>,
    pub base_range: Option<Vec<[i32; 2]>>,
}
impl Default for Mechanics {
    fn default() -> Self {
        Self {
            ammo: None,
            ammo_count: 0.,
            permanent: vec![],
            deploy_buff: None,
            condition: None,
            observer: None,
            skill_counter: None,
            aura: None,
            team: None,
            random_bless: None,
            deploy_sp: 0.,
            respawn_delta: 0.,
            cost_cut: None,
            burst: None,
            skill_debuff: None,
            skill_pull: None,
            skill_push: None,
            start_status: None,
            secondary: None,
            end_heal: 0.,
            blocked_taken: 1.,
            resist: 0.,
            crit: None,
            hit_upgrade: None,
            attack_boost: None,
            growing_atk: None,
            enemy_atk: None,
            distance: None,
            fly_scale: 1.,
            lord_skill_scale: 1.,
            skill_def_ignore: 0.,
            force_attack: false,
            revive: None,
            base_range: None,
        }
    }
}
pub(crate) struct AmmoSupport {
    pub delay: f64,
    pub own: f64,
    pub ally: f64,
}
pub(crate) enum Predicate {
    Unblocked,
    AllyInRange,
}
pub(crate) struct Conditional {
    pub key: &'static str,
    pub when: Predicate,
    pub mods: Numbers,
}
#[derive(Clone, Copy)]
pub(crate) struct Counter {
    pub key: &'static str,
    pub stat: &'static str,
    pub scale: f64,
}
pub(crate) struct InjuryObserver {
    pub key: &'static str,
    pub counter: Option<Counter>,
    pub debuff: Option<Debuff>,
}
pub(crate) struct StatAura {
    pub key: &'static str,
    pub grid: Option<Vec<[i32; 2]>>,
    pub profession: Option<&'static str>,
    pub mods: Numbers,
}
pub(crate) struct TeamBuff {
    pub key: &'static str,
    pub characters: &'static [&'static str],
    pub mods: Numbers,
}
impl TeamBuff {
    pub fn apply<H: crate::battle::buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut crate::battle::buffs::Runtime,
        h: &mut H,
        source: Id,
        target: Id,
    ) -> Result<(), String> {
        let mut buff = persistent(self.key, self.mods.clone());
        buff.runtime.source = Some(source);
        r.add(
            w,
            h,
            target,
            crate::battle::buffs::Spec {
                buff,
                allow_dead: true,
                mods_present: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
pub(crate) struct CostCut {
    pub amount: f64,
    pub cap: f64,
    pub every: bool,
}
pub(crate) struct Burst {
    pub grid: Vec<[i32; 2]>,
    pub count: usize,
    pub scale: f64,
    pub damage: DamageType,
    pub status: Option<OnHitStatus>,
    pub debuff: Option<Debuff>,
    pub pull: Option<PullHit>,
    pub fly_scale: f64,
    pub ground: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct Debuff {
    pub key: &'static str,
    pub stat: &'static str,
    pub value: f64,
    pub duration: f64,
    pub strongest: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct PullHit {
    pub force: f64,
    pub travel_damage: f64,
    pub distance: f64,
}
pub(crate) enum Secondary {
    Sonic { radius: f64, scale: f64 },
    NearbyBlocked { scale: f64 },
}
pub(crate) struct Critical {
    pub chance: f64,
    pub attack_scale: f64,
    pub damage_scale: f64,
    pub stun: f64,
}
pub(crate) struct AttackBoost {
    pub chance: f64,
    pub scale: f64,
}
pub(crate) struct EnemyAttack {
    pub value: f64,
    pub cap: f64,
}
pub(crate) struct DistanceScale {
    pub minimum: f64,
    pub maximum: f64,
    pub attack_scale: f64,
    pub damage_scale: f64,
}
pub(crate) struct Revive {
    pub hp_mul: f64,
    pub aspd: f64,
    pub hp_ratio: f64,
}
pub(crate) fn talent_at(raw: &Value, index: i64) -> &Value {
    raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == index)
        .map(|t| &t["bb"])
        .unwrap_or(&Value::Null)
}
pub(crate) fn persistent(key: &str, mods: Numbers) -> Buff {
    let mut b = Buff {
        key: Some(key.into()),
        mods,
        ..Default::default()
    };
    b.runtime.persist = true;
    b
}
pub(crate) fn flat_bat(raw: &Value, value: f64) -> f64 {
    let bat = number(&raw["stats"]["bat"], 1.);
    (value / if bat == 0. { 1. } else { bat }).max(-0.95)
}
pub(crate) fn fastshot(raw: &Value, s: &mut Spec) {
    s.combat.profile.priority = crate::battle::targeting::Priority::Fly;
    s.extra.effects.fly_scale = number(&raw["trait"]["bb"]["atk_scale"], 1.);
}
pub(super) fn blocked_def(raw: &Value, s: &mut Spec, key: &'static str) {
    let def = number(&raw["trait"]["bb"]["def"], 0.);
    if def != 0. {
        s.while_blocked = Some(WhileBlocked {
            key,
            mods: [("defPct".into(), def)].into(),
        });
    }
}
pub(crate) fn skill_range(raw: &Value, s: &mut Spec) -> Result<(), String> {
    if !raw["skill"]["rangeGrid"].is_null() {
        let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?;
        s.trigger_range = Some(grid.clone());
        s.extra.skill_range = Some(grid);
    }
    Ok(())
}
pub(crate) fn module_range(raw: &Value, s: &mut Spec) -> Result<(), String> {
    if raw["module"]["active"] != true || raw["trait"]["rangeExpand"] != true {
        return Ok(());
    }
    let module = raw["module"]["id"].as_str().unwrap_or("");
    if let Some(record) = raw["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["uniEquipId"] == module)
    {
        if let Some(change) = record["talentChanges"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["rangeGrid"].is_array())
        {
            s.extra.effects.base_range =
                Some(crate::battle::operators::parse_range(&change["rangeGrid"])?);
        }
    }
    Ok(())
}
