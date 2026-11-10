// SPDX-License-Identifier: GPL-3.0-or-later
//! Parameters for the next set of independent operator Kits.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    First,
    Second,
    Third,
}
#[derive(Default)]
pub(crate) enum Advanced {
    #[default]
    None,
    Harold(Harold),
    Mostma(Mostma),
    Mizuki(Mizuki),
    Aroma(Aroma),
    Billro(Billro),
    Mudrok(Mudrok),
    Flamtl(Flamtl),
    Fartth(Fartth),
    Lisa(Lisa),
    Mountain(Mountain),
    Surtr(Surtr),
    Aglina(Aglina),
    Reckpr(Reckpr),
    Glady(Glady),
    Gnosis(Gnosis),
}
pub(crate) struct Harold {
    pub severe: bool,
    pub element_ratio: f64,
    pub extra_scale: f64,
    pub resistance: f64,
}
pub(crate) struct Mostma {
    pub lock: bool,
    pub ripple: bool,
    pub slow: f64,
    pub slow_scale: f64,
    pub force: f64,
    pub lock_scale: f64,
}
pub(crate) struct Mizuki {
    pub mode: Mode,
    pub scale: f64,
    pub count: usize,
    pub extra: usize,
    pub skill_scale: f64,
    pub status: f64,
    pub loss: f64,
    pub threshold: f64,
    pub attack: f64,
    pub slow: f64,
}
pub(crate) struct Aroma {
    pub landing: bool,
    pub first_scale: f64,
    pub levitate: f64,
    pub land_scale: f64,
    pub fly_scale: f64,
}
pub(crate) struct Billro {
    pub mode: Mode,
    pub heal: f64,
    pub charged_heal: f64,
    pub recovery: f64,
    pub charged_atk: f64,
    pub ramp_atk: f64,
    pub mark: f64,
    pub guard: Numbers,
    pub keep: Numbers,
    pub sluggish: f64,
    pub bind: f64,
    pub per_enemy: f64,
    pub enemy_cap: f64,
}
pub(crate) struct Mudrok {
    pub mode: Mode,
    pub grid: Vec<[i32; 2]>,
    pub sleep: f64,
    pub slow: f64,
    pub stun: f64,
    pub awake: Numbers,
    pub layer_period: f64,
    pub layer_add: u32,
    pub layer_cap: u32,
    pub layer_heal: f64,
    pub layer_sp: f64,
    pub sarkaz_resistance: f64,
    pub module: Numbers,
}
pub(crate) struct Flamtl {
    pub mode: Mode,
    pub total: u32,
    pub cost: f64,
    pub ally_dodge: f64,
    pub ally_duration: f64,
    pub faction_dodge: f64,
    pub grid: Vec<[i32; 2]>,
    pub module: Numbers,
}
pub(crate) struct Fartth {
    pub mode: Mode,
    pub focus: f64,
    pub delay: f64,
    pub far_scale: f64,
    pub module_sp: f64,
    pub base_grid: Vec<[i32; 2]>,
}
pub(crate) struct Lisa {
    pub fox: bool,
    pub fragile: f64,
    pub boost: f64,
    pub regen: f64,
    pub module_sp: f64,
}
pub(crate) struct Mountain {
    pub mode: Mode,
    pub probability: f64,
    pub active_probability: f64,
    pub scale: f64,
    pub weaken: f64,
    pub duration: f64,
    pub module_aspd: f64,
}
pub(crate) struct Surtr {
    pub mode: Mode,
    pub withdraw: f64,
    pub drain_interval: f64,
    pub drain_peak: f64,
    pub drain_ramp: f64,
    pub solo: f64,
    pub module_aspd: f64,
    pub module_fragile: f64,
}
pub(crate) struct Aglina {
    pub weightless: bool,
    pub aspd: f64,
    pub regen: f64,
    pub module_sp: f64,
}
pub(crate) struct Reckpr {
    pub guard: bool,
    pub duration: f64,
    pub heal: f64,
    pub probability: f64,
    pub sp: f64,
    pub aspd: f64,
    pub learn_duration: f64,
}
pub(crate) struct Glady {
    pub mode: Mode,
    pub force: f64,
    pub interval: f64,
    pub scale: f64,
    pub slow: f64,
    pub regen: f64,
    pub resistance: f64,
    pub max_weight: f64,
    pub light_scale: f64,
    pub drag: f64,
    pub drag_distance: f64,
    pub far_radius: f64,
    pub far_force: f64,
}
pub(crate) struct Gnosis {
    pub mode: Mode,
    pub cold: f64,
    pub burst_cold: f64,
    pub burst_scale: f64,
    pub fragile_cold: f64,
    pub fragile_frozen: f64,
    pub module_sp: f64,
    pub resist_delay: f64,
    pub resist: f64,
}
#[derive(Default)]
pub(crate) struct AdvancedState {
    pub elapsed: f64,
    pub skill_left: f64,
    pub accumulator: f64,
    pub aura_accumulator: f64,
    pub next_fast: f64,
    pub next_aura: f64,
    pub charged: bool,
    pub floating: std::collections::BTreeSet<Id>,
    pub locked: std::collections::BTreeSet<Id>,
    pub tornado: Option<[f64; 2]>,
    pub layers: u32,
    pub next_layer: f64,
    pub riposte: bool,
    pub riposte_now: bool,
    pub critical: bool,
    pub solo: bool,
    pub last_hurt: f64,
    pub ember: bool,
    pub dp_given: u32,
    pub extra_range: Vec<usize>,
    pub harold_half: std::collections::BTreeSet<Id>,
}
