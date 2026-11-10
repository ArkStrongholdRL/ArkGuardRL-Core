// SPDX-License-Identifier: GPL-3.0-or-later
//! Stage fixtures use the shared unit, occupancy, blocking and damage lifecycles.
use super::*;
use crate::battle::deploy;
#[derive(Clone, Debug, Default)]
pub struct MapConfig {
    pub effects: Vec<String>,
    pub device_overrides: std::collections::BTreeMap<String, bool>,
    pub top_layers: f64,
    pub cold_wind: Option<ColdWind>,
}
#[derive(Clone, Debug)]
pub struct ColdWind {
    pub interval: f64,
    pub duration: f64,
    pub first: Option<f64>,
}
impl MapConfig {
    pub fn parse(v: &Value) -> Result<Self, String> {
        if v.is_null() {
            return Ok(Self::default());
        }
        let object = v.as_object().ok_or("Expected map configuration object")?;
        if object
            .keys()
            .any(|k| !["deviceOverrides", "topLayers", "coldWind", "effects"].contains(&k.as_str()))
        {
            return Err("Unknown map configuration field".into());
        }
        let mut out = Self::default();
        if let Some(effects) = v.get("effects") {
            let effects = effects.as_array().ok_or("Expected map effect list")?;
            if effects.len() > super::super::grid::N {
                return Err("Map effect capacity exceeded".into());
            }
            out.effects = effects
                .iter()
                .map(|e| {
                    e.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "Expected map effect ID".to_string())
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(overrides) = v.get("deviceOverrides") {
            for (key, value) in overrides
                .as_object()
                .ok_or("Expected device override object")?
            {
                out.device_overrides.insert(
                    key.clone(),
                    value.as_bool().ok_or("Device switch must be boolean")?,
                );
            }
        }
        if let Some(layers) = v.get("topLayers") {
            out.top_layers = layers.as_f64().ok_or("Expected device layer count")?;
        }
        if let Some(c) = v.get("coldWind").filter(|c| !c.is_null()) {
            if c.as_object()
                .ok_or("Expected cold wind configuration")?
                .keys()
                .any(|k| !["interval", "duration", "first"].contains(&k.as_str()))
            {
                return Err("Unknown cold wind field".into());
            }
            out.cold_wind = Some(ColdWind {
                interval: c["interval"].as_f64().ok_or("Expected wind interval")?,
                duration: c["duration"].as_f64().ok_or("Expected wind duration")?,
                first: c
                    .get("first")
                    .map(|x| x.as_f64().ok_or("Expected first wind time"))
                    .transpose()?,
            });
        }
        out.validate()?;
        Ok(out)
    }

    pub(crate) fn resolved(mut self, data: &Catalog) -> Result<Self, String> {
        self.validate()?;
        let mut overrides = std::collections::BTreeMap::new();
        for id in &self.effects {
            let record = data
                .table("effects")
                .and_then(|t| t.get(id))
                .ok_or_else(|| format!("Unknown map effect: {id}"))?;
            let buffs = record["buffs"]
                .as_array()
                .ok_or("Map effect has no buffs")?;
            if buffs.is_empty() {
                return Err("Map effect has no buffs".into());
            }
            for buff in buffs {
                if buff["key"] != "auto_chess_change_map" {
                    return Err(format!("Not a map-switch effect: {id}"));
                }
                for (alias, value) in buff["bb"]
                    .as_object()
                    .ok_or("Map effect has no parameters")?
                {
                    if !alias.starts_with("trap_") || !alias.contains('#') {
                        return Err(format!("Unsupported map effect parameter: {alias}"));
                    }
                    let on = match value {
                        Value::Bool(v) => *v,
                        Value::Number(v) => v.as_f64().unwrap_or(0.) != 0.,
                        Value::String(v) => !v.is_empty() && v != "0" && v != "false",
                        _ => return Err("Invalid map effect switch".into()),
                    };
                    overrides.insert(alias.clone(), on);
                }
            }
        }
        overrides.extend(self.device_overrides);
        self.device_overrides = overrides;
        self.validate()?;
        Ok(self)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.effects.len() > super::super::grid::N
            || self.device_overrides.len() > super::super::grid::N
            || !self.top_layers.is_finite()
            || self.top_layers < 0.
            || self.cold_wind.as_ref().is_some_and(|c| {
                !c.interval.is_finite()
                    || c.interval <= 0.
                    || !c.duration.is_finite()
                    || c.duration < 0.
                    || c.first.is_some_and(|f| !f.is_finite() || f < 0.)
            })
        {
            return Err("Invalid map device configuration".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Devices {
    pub(super) stage: Value,
    pub(super) config: MapConfig,
    pub(super) cold_due: Option<f64>,
    crates: Vec<([i32; 2], f64)>,
    removed: std::collections::BTreeSet<[i32; 2]>,
    pub(super) added: Vec<([i32; 2], f64)>,
    pub(super) turrets: Vec<super::turrets::Turret>,
    pub(super) ids: Vec<Id>,
    breaks: Vec<(f64, Id, Id)>,
    cleared: Vec<[i32; 2]>,
    elevated: std::collections::BTreeSet<[i32; 2]>,
}
impl Devices {
    pub(super) fn movement_quiet(&self) -> bool {
        self.crates.is_empty()
            && self.added.is_empty()
            && self.breaks.is_empty()
            && self.cleared.is_empty()
            && self.turrets.is_empty()
            && self.ids.is_empty()
            && self.cold_due.is_none()
            && self.config.cold_wind.is_none()
    }
    pub(super) fn compile(stage: &Value, grid: &mut Grid) -> Result<Self, String> {
        if stage["devices"]
            .as_array()
            .is_some_and(|d| d.len() > super::super::grid::N)
        {
            return Err("Stage device capacity exceeded".into());
        }
        let mut out = Self {
            stage: stage.clone(),
            ..Default::default()
        };
        for d in stage["devices"].as_array().into_iter().flatten() {
            if !d["active"].as_bool().unwrap_or(d["hidden"] != true) {
                continue;
            }
            let at = [
                d["row"]
                    .as_i64()
                    .or_else(|| d["pos"][0].as_i64())
                    .unwrap_or(-1) as i32,
                d["col"]
                    .as_i64()
                    .or_else(|| d["pos"][1].as_i64())
                    .unwrap_or(-1) as i32,
            ];
            match d["role"].as_str() {
                Some("platform" | "mound") if inside(grid.rect(), at) => {
                    grid.set_obstacle(at, true, super::super::grid::BLOCK)?;
                    out.elevated.insert(at);
                }
                Some("crate") if inside(grid.rect(), at) => {
                    if out.crates.iter().any(|(tile, _)| *tile == at) {
                        continue;
                    }
                    let hp = if d["key"] == "trap_1105_accrate" {
                        100.
                    } else {
                        d["stats"]["maxHp"]
                            .as_f64()
                            .filter(|v| v.is_finite() && *v > 0.)
                            .unwrap_or(100.)
                    };
                    out.crates.push((at, hp));
                    grid.set_obstacle(at, true, super::super::grid::CRATE)?;
                }
                // Controllers supply terrain parameters; cold wind is triggered by effects,
                // not by the presence of its controller in the stage data.
                Some("blower" | "mireController" | "tideController" | "coldWind" | "turret") => {}
                _ if inside(grid.rect(), at) => {
                    return Err(format!("Stage device not implemented: {}", d["key"]))
                }
                _ => {}
            }
        }
        Ok(out)
    }
    pub(super) fn flush(&mut self, grid: &mut Grid) -> Result<(), String> {
        for tile in self.cleared.drain(..) {
            grid.set_obstacle(tile, false, super::super::grid::CRATE)?;
        }
        Ok(())
    }
    pub(super) fn elevated(&self, at: [i32; 2]) -> bool {
        self.elevated.contains(&at)
    }
}
pub(super) struct Hooks<'a> {
    pub(super) host: &'a mut Host,
    pub(super) buffs: &'a mut buffs::Runtime,
}
impl blocking::Hooks for Hooks<'_> {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        blocking::restore_stealth(w, self.buffs, self.host, id)
    }
}
impl deploy::Hooks for Hooks<'_> {
    fn initialize(
        &mut self,
        _: Id,
        _: deploy::Options,
        _: &mut World,
        _: &mut deploy::Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn event(
        &mut self,
        _: deploy::Event,
        _: Id,
        _: Option<Id>,
        _: &mut World,
        _: &mut deploy::Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn remove_temporary_effects(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        self.buffs.discard_temporary(w, self.host, id)
    }
}
impl Core {
    pub(super) fn install_devices(&mut self) -> Result<(), String> {
        for (tile, hp) in self.host.devices.crates.clone() {
            self.spawn_map_unit(tile, &Value::Null, Some(hp))?;
        }
        let mut buffs = self.host.buffs.take().unwrap();
        let result = (|| {
            for id in self.host.devices.ids.clone() {
                if self
                    .host
                    .devices
                    .removed
                    .contains(&self.world.unit(id)?.deployment.tile)
                {
                    self.host.remove_device(&mut self.world, &mut buffs, id)?;
                }
            }
            Ok(())
        })();
        self.host.buffs = Some(buffs);
        result
    }
    pub(super) fn device_scheduled(&mut self) -> Result<(), String> {
        let now = self.world.time;
        let due: Vec<_> = self
            .host
            .devices
            .breaks
            .iter()
            .copied()
            .filter(|(at, _, _)| *at <= now)
            .collect();
        self.host.devices.breaks.retain(|(at, _, _)| *at > now);
        let mut buffs = self.host.buffs.take().unwrap();
        let result = (|| {
            for (_, device, enemy) in due {
                if self.world.unit(device)?.stats.alive
                    && self
                        .world
                        .unit(enemy)
                        .is_ok_and(|e| e.stats.alive && e.blocked_by() == Some(device))
                {
                    self.host
                        .kill_device(&mut self.world, &mut buffs, device, Some(enemy))?;
                }
            }
            Ok(())
        })();
        self.host.buffs = Some(buffs);
        result
    }
}
impl Host {
    pub(super) fn device_blocked(
        &mut self,
        w: &mut World,
        blocker: Id,
        enemy: Id,
    ) -> Result<(), String> {
        if !self.devices.ids.contains(&blocker) {
            return Ok(());
        }
        if self.devices.breaks.len() >= self.tactical_limit {
            return Err("Device timer capacity exceeded".into());
        }
        let delay = w
            .unit_mut(enemy)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("interval")
            .clamp(0.1, 10.);
        self.devices.breaks.push((w.time + delay, blocker, enemy));
        Ok(())
    }
    pub(super) fn kill_device(
        &mut self,
        w: &mut World,
        buffs: &mut buffs::Runtime,
        id: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        let tile = w.unit(id)?.deployment.tile;
        let mut roster = self
            .roster
            .take()
            .ok_or("Missing device deployment context")?;
        let result = roster
            .deployment_mut()
            .kill(w, &mut Hooks { host: self, buffs }, id, credit);
        self.roster = Some(roster);
        result?;
        if let Some(grid) = self.grid.as_mut() {
            grid.set_obstacle(tile, false, super::super::grid::CRATE)?;
        } else {
            self.devices.cleared.push(tile);
        }
        self.devices.breaks.retain(|(_, device, _)| *device != id);
        Ok(())
    }
}

impl Field {
    /// Install map-card switches and device effects before adding the lineup.
    pub fn configure_map(&mut self, config: MapConfig) -> Result<(), String> {
        self.healthy()?;
        config.validate()?;
        if self.clock.started() || self.ticks() != 0 || !self.allies().is_empty() {
            return Err("Map configuration requires an unstarted field without a lineup".into());
        }
        let config = config.resolved(&self.core.host.data)?;
        let previous = &self.core.host.devices.config;
        if config.effects.is_empty()
            && config.device_overrides.is_empty()
            && config.cold_wind.is_none()
            && previous.effects.is_empty()
            && previous.device_overrides.is_empty()
            && previous.cold_wind.is_none()
            && self.core.world.unit_ids().next().is_none()
        {
            // No switch to apply: the field was built from this same stage, so
            // rebuilding the grid, devices and terrain would only repeat work.
            self.core.host.devices.config = config;
            return Ok(());
        }
        let stage = self.core.host.devices.stage.clone();
        let mut grid = Grid::new(&stage, self.core.host.rect)?;
        let mut devices = Devices::compile(&stage, &mut grid)?;
        let mut terrain_stage = stage.clone();
        for (i, d) in stage["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let alias = d["alias"].as_str().or(d["raw"]["alias"].as_str());
            let Some(on) = alias.and_then(|a| config.device_overrides.get(a)).copied() else {
                continue;
            };
            let at = device_position(d);
            let active = d["active"].as_bool().unwrap_or(d["hidden"] != true);
            terrain_stage["devices"][i]["active"] = json!(on);
            if !grid.in_rect(at) {
                continue;
            }
            match d["role"].as_str() {
                Some("crate") if active && !on => {
                    devices.removed.insert(at);
                    grid.set_obstacle(at, false, super::super::grid::CRATE)?;
                }
                Some("crate") if !active && on => {
                    devices.added.push((at, num(&d["stats"]["maxHp"], 100.)))
                }
                Some("platform" | "mound") => {
                    grid.set_obstacle(at, on, super::super::grid::BLOCK)?;
                    if on {
                        devices.elevated.insert(at);
                    } else {
                        devices.elevated.remove(&at);
                    }
                }
                _ => {}
            }
        }
        let terrain = super::super::terrain::Config::compile(&terrain_stage, &grid)?;
        devices.cold_due = config
            .cold_wind
            .as_ref()
            .map(|c| c.first.unwrap_or(c.interval));
        devices.config = config;
        self.core.host.devices = devices;
        self.core.host.terrain = Arc::new(terrain);
        self.core.host.grid = Some(Box::new(grid));
        Ok(())
    }
    /// Update a live covenant's wind duration without restarting its pulse timer.
    pub fn update_cold_wind_duration(&mut self, duration: f64) -> Result<(), String> {
        self.healthy()?;
        if !duration.is_finite() || duration < 0. {
            return Err("Invalid cold wind duration".into());
        }
        self.core
            .host
            .devices
            .config
            .cold_wind
            .as_mut()
            .ok_or("Cold wind is not configured")?
            .duration = duration;
        Ok(())
    }
    /// Live highest bond layers, used when a turret starts its next attack.
    pub fn update_map_layers(&mut self, layers: f64) -> Result<(), String> {
        self.healthy()?;
        if !layers.is_finite() || layers < 0. {
            return Err("Invalid device bond layers".into());
        }
        self.core.host.devices.config.top_layers = layers;
        Ok(())
    }
}
pub(super) fn num(v: &Value, fallback: f64) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| v.is_finite())
        .unwrap_or(fallback)
}
pub(super) fn device_position(d: &Value) -> [i32; 2] {
    [
        d["row"]
            .as_i64()
            .or(d["pos"][0].as_i64())
            .or(d["pos"]["row"].as_i64())
            .unwrap_or(-1) as i32,
        d["col"]
            .as_i64()
            .or(d["pos"][1].as_i64())
            .or(d["pos"]["col"].as_i64())
            .unwrap_or(-1) as i32,
    ]
}
impl Core {
    pub(super) fn cold_wind_scheduled(&mut self) -> Result<(), String> {
        let Some(due) = self.host.devices.cold_due else {
            return Ok(());
        };
        if self.world.time < due {
            return Ok(());
        }
        let cold = self.host.devices.config.cold_wind.as_ref().unwrap().clone();
        // Original repeating timers schedule the next pulse from the current tick.
        self.host.devices.cold_due = Some(self.world.time + cold.interval);
        let ids = self.host.spawns.as_ref().unwrap().live().to_vec();
        let mut buffs = self.host.buffs.take().unwrap();
        let result = (|| {
            if cold.duration <= 0. {
                return Ok(());
            }
            for id in ids {
                if !self.world.unit(id)?.hidden && self.world.unit(id)?.stats.alive {
                    self.host.apply_status(
                        &mut self.world,
                        &mut buffs,
                        id,
                        "cold",
                        super::super::status::Options {
                            duration: cold.duration,
                            ..Default::default()
                        },
                    )?;
                }
            }
            Ok(())
        })();
        self.host.buffs = Some(buffs);
        result
    }
}
impl Host {
    fn remove_device(
        &mut self,
        w: &mut World,
        buffs: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let mut roster = self
            .roster
            .take()
            .ok_or("Missing device deployment context")?;
        let tile = w.unit(id)?.deployment.tile;
        let result = roster.deployment_mut().retreat(
            w,
            &mut Hooks { host: self, buffs },
            id,
            "removed",
            true,
        );
        self.roster = Some(roster);
        result?;
        self.grid
            .as_mut()
            .ok_or("Missing device grid")?
            .set_obstacle(tile, false, super::super::grid::CRATE)?;
        Ok(())
    }
}
