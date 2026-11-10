// SPDX-License-Identifier: GPL-3.0-or-later
//! Static ground terrain and per-unit contact state. Active devices remain separate.
use super::{
    buffs::{self, Runtime, Spec},
    grid::{Grid, COLS, N},
    health::{Id, Side, Unit, World},
    stats::Buff,
    targeting,
};
use serde_json::Value;
pub(crate) const INFECTION: u64 = 1800;
pub(crate) const DEEPSEA: u64 = 1801;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    None,
    Mire,
    Smog,
    Deepsea,
    Infection,
}
impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Mire => "terrain:mire",
            Self::Smog => "terrain:smog",
            Self::Deepsea => "terrain:deepsea",
            Self::Infection => "terrain:infection",
        }
    }
}
#[derive(Clone, Copy, Default)]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct State {
    kind: Kind,
    entered: f64,
    triggers: f64,
    stacks: f64,
    pub(crate) direction: Option<super::direction::Direction>,
    air_atk: f64,
    air_move: Option<f64>,
    air_position: Option<[f64; 2]>,
}
impl State {
    pub(crate) fn movement_arrive(&mut self, position: [f64; 2]) {
        self.air_position = Some(position);
    }
}
#[derive(Clone)]
pub(crate) struct Config {
    tiles: [Kind; N],
    flow: Vec<Option<Flow>>,
    active: bool,
    mire_interval: f64,
    mire_aspd: f64,
    mire_move: f64,
    mire_max: f64,
    heavy: f64,
    deep_damage: f64,
    deep_aspd: f64,
    deep_move: f64,
    infection_damage: f64,
    infection_atk: f64,
    infection_aspd: f64,
    infection_duration: f64,
}
#[derive(Clone, Copy)]
struct Flow {
    direction: super::direction::Direction,
    atk: [f64; 3],
    speed: [f64; 2],
}
fn number(v: &Value, fallback: f64) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| v.is_finite())
        .unwrap_or(fallback)
}
fn aspd(v: f64) -> f64 {
    if v.abs() < 1. {
        v * 100.
    } else {
        v
    }
}
impl Config {
    pub(crate) fn compile(stage: &Value, grid: &Grid) -> Result<Self, String> {
        let mut tiles = [Kind::None; N];
        for (i, tile) in grid.tiles().iter().enumerate() {
            let at = [(i / COLS) as i32, (i % COLS) as i32];
            let rect = grid.rect();
            if at[0] < rect.r0 || at[0] > rect.r1 || at[1] < rect.c0 || at[1] > rect.c1 {
                continue;
            }
            tiles[i] = match tile.terrain.as_deref() {
                None => Kind::None,
                Some("mire") => Kind::Mire,
                Some("smog") => Kind::Smog,
                Some("deepsea") => Kind::Deepsea,
                Some("infection") => Kind::Infection,
                Some(name) => return Err(format!("Field terrain not implemented: {name}")),
            };
        }
        let controller = |role| {
            stage["devices"]
                .as_array()
                .and_then(|a| a.iter().find(|d| d["role"] == role))
                .map(|d| &d.get("raw").unwrap_or(d)["skill"]["bb"])
                .unwrap_or(&Value::Null)
        };
        let m = &stage["special"]["mire"];
        let md = controller("mireController");
        let deep = stage["special"]["deepsea"]["bb"]
            .as_object()
            .map(|_| &stage["special"]["deepsea"]["bb"])
            .unwrap_or_else(|| controller("tideController"));
        let inf = &stage["special"]["infection"]["bb"];
        let interval = number(&m["intervalSec"], 1.);
        let duration = number(&inf["duration"], 300.);
        let mut flow = vec![None; N];
        for d in stage["devices"].as_array().into_iter().flatten() {
            if d["role"] != "blower" || !d["active"].as_bool().unwrap_or(d["hidden"] != true) {
                continue;
            }
            use super::direction::Direction;
            let direction = match d["dir"]
                .as_str()
                .or(d["raw"]["dir"].as_str())
                .unwrap_or("UP")
                .to_uppercase()
                .as_str()
            {
                "UP" => Direction::Up,
                "DOWN" => Direction::Down,
                "LEFT" => Direction::Left,
                "RIGHT" => Direction::Right,
                _ => continue,
            };
            let raw = d.get("raw").unwrap_or(d);
            let bb = raw["skill"]
                .get("bb")
                .unwrap_or(&stage["special"]["blower"]["bb"]);
            let f = Flow {
                direction,
                atk: ["equal", "opposite", "vertical"]
                    .map(|k| number(&bb[format!("blower_s_character[{k}].atk")], 0.)),
                speed: ["equal", "opposite"]
                    .map(|k| number(&bb[format!("blower_s_enemy[{k}].move_speed")], 0.)),
            };
            for p in raw["rangeTiles"].as_array().into_iter().flatten() {
                let Some(at) = p[0]
                    .as_i64()
                    .zip(p[1].as_i64())
                    .map(|(r, c)| [r as i32, c as i32])
                else {
                    continue;
                };
                if grid.in_rect(at) {
                    flow[at[0] as usize * COLS + at[1] as usize] = Some(f);
                }
            }
        }
        Ok(Self {
            active: tiles.iter().any(|k| *k != Kind::None) || flow.iter().any(Option::is_some),
            flow,
            tiles,
            mire_interval: if interval > 0. { interval } else { 1. },
            mire_aspd: aspd(number(
                m.get("aspdPerStack").unwrap_or(&md["attack_speed"]),
                -0.05,
            )),
            mire_move: number(m.get("moveMulPerStack").unwrap_or(&md["move_speed"]), -0.05),
            mire_max: number(m.get("maxStacks").unwrap_or(&md["max_stack_cnt"]), 10.)
                .floor()
                .max(1.),
            heavy: number(m.get("heavyWeight").unwrap_or(&md["value"]), 3.),
            deep_damage: number(&deep["sea_drown[enemy].damage"], 40.),
            deep_aspd: aspd(number(&deep["sea_drown[enemy].attack_speed"], -0.6)),
            deep_move: number(&deep["sea_drown[enemy].move_speed"], 0.6),
            infection_damage: number(&inf["damage"], 70.),
            infection_atk: number(&inf["atk"], 0.2),
            infection_aspd: aspd(number(&inf["attack_speed"], 20.)),
            infection_duration: if duration > 0. { duration } else { 300. },
        })
    }
    pub(crate) fn active(&self) -> bool {
        self.active
    }
    pub(crate) fn at(&self, position: [f64; 2]) -> Kind {
        let [r, c] = position.map(|v| (v + 0.5).floor() as i32);
        if Grid::in_bounds([r, c]) {
            self.tiles[r as usize * COLS + c as usize]
        } else {
            Kind::None
        }
    }
    pub(crate) fn movement_ground_clear(&self, unit: &Unit) -> bool {
        let kind = self.at(unit.position);
        matches!(kind, Kind::None | Kind::Mire) && unit.terrain.kind == kind
    }
    pub(crate) fn stationary_ground_clear(&self, unit: &Unit) -> bool {
        self.at(unit.position) == Kind::None && unit.terrain.kind == Kind::None
    }
    pub(crate) fn movement_effect_stable(
        &self,
        state: &State,
        position: [f64; 2],
        time: f64,
    ) -> bool {
        if self.at(position) != state.kind {
            return false;
        }
        if state.kind == Kind::Mire
            && 1. + ((time - state.entered + 1e-9) / self.mire_interval).floor() > state.triggers
        {
            return false;
        }
        let old = state.air_move.unwrap_or(1.);
        self.enemy_airflow(position, state.air_position, false, old) == old
    }
    fn enemy_airflow(
        &self,
        position: [f64; 2],
        previous: Option<[f64; 2]>,
        hidden: bool,
        old: f64,
    ) -> f64 {
        let at = position.map(|v| (v + 0.5).floor() as i32);
        let flow = if Grid::in_bounds(at) {
            self.flow[at[0] as usize * COLS + at[1] as usize]
        } else {
            None
        };
        let mut mul = 1.;
        if !hidden {
            if let (Some(previous), Some(f)) = (previous, flow) {
                let delta = [position[0] - previous[0], position[1] - previous[1]];
                let speed = crate::deterministic::math::hypot(delta[0], delta[1]);
                if speed > 1e-6 {
                    let vector = f.direction.vector();
                    let dot = (delta[1] * vector[1] as f64 + delta[0] * vector[0] as f64) / speed;
                    if dot > 0.7 {
                        mul = 1. + f.speed[0];
                    } else if dot < -0.7 {
                        mul = 1. + f.speed[1];
                    }
                } else {
                    mul = old;
                }
            }
        }
        mul.max(0.)
    }
    pub(crate) fn refresh<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut Runtime,
        h: &mut H,
        id: Id,
        fresh: bool,
    ) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        if fresh {
            let direction = w.unit(id)?.terrain.direction;
            w.unit_mut(id)?.terrain = State {
                direction,
                ..Default::default()
            };
        }
        let u = w.unit(id)?;
        if !u.stats.alive || !u.deployed || u.deployment.kind == super::deploy::Kind::Device {
            return Ok(());
        }
        let enemy = u.side == Side::Enemy;
        let hidden = u.hidden;
        let position = if enemy {
            u.position
        } else {
            u.deployment.tile.map(f64::from)
        };
        let mut kind = self.at(position);
        if hidden
            || targeting::flying(w, id)?
            || kind == Kind::Smog && enemy
            || kind == Kind::Deepsea && !enemy
        {
            kind = Kind::None;
        }
        let state = w.unit(id)?.terrain;
        if state.kind != kind {
            if !matches!(state.kind, Kind::None | Kind::Infection) {
                r.remove(w, h, id, state.kind.key())?;
            }
            let now = w.time;
            w.unit_mut(id)?.terrain = State {
                kind,
                entered: now,
                triggers: 0.,
                stacks: 0.,
                ..state
            };
            let mut b = Buff::default();
            b.key = Some(kind.key().into());
            match kind {
                Kind::Smog => {
                    b.flags.insert("stealth".into());
                }
                Kind::Deepsea => {
                    b.mods = [
                        ("aspd".into(), self.deep_aspd),
                        ("moveMul".into(), self.deep_move),
                    ]
                    .into();
                    b.runtime.interval = 1.;
                    b.runtime.on_tick = Some(DEEPSEA);
                    b.runtime.tick_payload = self.deep_damage;
                }
                _ => {}
            }
            if matches!(kind, Kind::Smog | Kind::Deepsea) {
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        mods_present: !b.mods.is_empty(),
                        flags_present: !b.flags.is_empty(),
                        buff: b,
                        ..Default::default()
                    },
                )?;
            }
        }
        match kind {
            Kind::Infection => {
                let old = w
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some(kind.key()));
                if let Some(b) = old {
                    if b.runtime.time_left < self.infection_duration {
                        let bid = b.runtime.id();
                        r.edit(w, id, bid, |b| {
                            b.runtime.time_left = self.infection_duration
                        })?;
                    }
                } else {
                    let mut b = Buff::default();
                    b.key = Some(kind.key().into());
                    b.mods = [
                        ("atkPct".into(), self.infection_atk),
                        ("aspd".into(), self.infection_aspd),
                    ]
                    .into();
                    b.runtime.duration = self.infection_duration;
                    b.runtime.interval = 1.;
                    b.runtime.on_tick = Some(INFECTION);
                    b.runtime.tick_payload = self.infection_damage;
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: b,
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
            Kind::Mire => {
                let state = w.unit(id)?.terrain;
                let due = 1. + ((w.time - state.entered + 1e-9) / self.mire_interval).floor();
                if due > state.triggers {
                    let heavy = enemy
                        && w.unit_mut(id)?
                            .stats
                            .stats()
                            .map_err(str::to_string)?
                            .n("massLevel")
                            >= self.heavy;
                    let stacks = (state.stacks
                        + if heavy { 2. } else { 1. } * (due - state.triggers))
                        .min(self.mire_max);
                    w.unit_mut(id)?.terrain.triggers = due;
                    w.unit_mut(id)?.terrain.stacks = stacks;
                    if stacks != state.stacks {
                        let mut b = Buff::default();
                        b.key = Some(kind.key().into());
                        b.mods.insert("aspd".into(), self.mire_aspd * stacks);
                        if enemy {
                            b.mods
                                .insert("moveMul".into(), (1. + self.mire_move * stacks).max(0.));
                        }
                        r.add(
                            w,
                            h,
                            id,
                            Spec {
                                buff: b,
                                mods_present: true,
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
            _ => {}
        }
        self.airflow(w, r, h, id)
    }
    fn airflow<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let u = w.unit(id)?;
        let enemy = u.side == Side::Enemy;
        let position = if enemy {
            u.position
        } else {
            u.deployment.tile.map(f64::from)
        };
        let state = u.terrain;
        let (value, old, key, neutral) = if enemy {
            let mul = self.enemy_airflow(
                position,
                state.air_position,
                u.hidden,
                state.air_move.unwrap_or(1.),
            );
            w.unit_mut(id)?.terrain.air_position = Some(position);
            (mul, state.air_move.unwrap_or(1.), "moveMul", 1.)
        } else {
            let at = position.map(|v| (v + 0.5).floor() as i32);
            let flow = if Grid::in_bounds(at) {
                self.flow[at[0] as usize * COLS + at[1] as usize]
            } else {
                None
            };
            let atk = if !u.motion_fly {
                flow.map(|f| {
                    f.atk[match state.direction {
                        Some(d) if d == f.direction => 0,
                        Some(d) if d == f.direction.opposite() => 1,
                        _ => 2,
                    }]
                })
                .unwrap_or(0.)
            } else {
                0.
            };
            (atk, state.air_atk, "atkPct", 0.)
        };
        if value == old {
            return Ok(());
        }
        if enemy {
            w.unit_mut(id)?.terrain.air_move = Some(value);
        } else {
            w.unit_mut(id)?.terrain.air_atk = value;
        }
        if value == neutral {
            r.remove(w, h, id, "terrain:airflow")?;
        } else {
            r.add(
                w,
                h,
                id,
                Spec {
                    buff: Buff {
                        key: Some("terrain:airflow".into()),
                        mods: [(key.into(), value)].into(),
                        ..Default::default()
                    },
                    mods_present: true,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
}
