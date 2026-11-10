// SPDX-License-Identifier: GPL-3.0-or-later
//! Decision-time geometry derived from public map data, templates and previews.
use super::*;
use crate::battle::{
    grid::{Grid, Rect, BLOCK, COLS, CRATE, N},
    waves::{has_patrol, normalize_route},
};
use crate::deterministic::math::round_half_even;

const MAX_ROUTES: usize = 64;
const MAX_CHECKPOINTS: usize = 256;
const MAX_PATH_CELLS: usize = 65_536;
const MAX_BYTES: usize = 8 << 20;

fn position(value: &Value) -> Result<[f64; 2], String> {
    let coordinates = value.as_array().ok_or("Invalid public spatial position")?;
    if coordinates.len() != 2 {
        return Err("Invalid public spatial position".into());
    }
    let row = coordinates[0]
        .as_f64()
        .ok_or("Invalid public spatial row")?;
    let col = coordinates[1]
        .as_f64()
        .ok_or("Invalid public spatial column")?;
    if !row.is_finite() || !col.is_finite() {
        return Err("Non-finite public spatial position".into());
    }
    Ok([row, col])
}

fn cell(point: [f64; 2]) -> Result<[i32; 2], String> {
    let point = point.map(round_half_even);
    if point[0] < 0.
        || point[0] >= crate::battle::grid::ROWS as f64
        || point[1] < 0.
        || point[1] >= COLS as f64
    {
        return Err("Public spatial position outside the stage".into());
    }
    Ok(point.map(|n| n as i32))
}

fn segment(
    grid: &mut Grid,
    from: [f64; 2],
    to: [f64; 2],
    checkpoint: Option<usize>,
    flying: bool,
    remaining: &mut usize,
) -> Result<Value, String> {
    let start = cell(from)?;
    let end = cell(to)?;
    let path = if flying {
        None
    } else {
        grid.find_path(start, end, false)
    };
    let size = path.as_ref().map_or(0, Vec::len);
    *remaining = remaining
        .checked_sub(size)
        .ok_or("Public spatial path capacity exceeded")?;
    Ok(json!({"from":from,"to":to,"checkpoint":checkpoint,"groundCells":path}))
}

fn route(raw: &Value, grid: &mut Grid, remaining: &mut usize) -> Result<Value, String> {
    let mut route = normalize_route(raw, MAX_CHECKPOINTS)?;
    // E_NUM entries are reserved template slots with no concrete route yet.
    if raw["motion"] == "E_NUM" {
        route["motion"] = json!("E_NUM");
        route["segments"] = json!([]);
        route["repeatFrom"] = Value::Null;
        return Ok(route);
    }
    let flying = route["motion"] == "FLY";
    let mut cursor = position(&route["start"])?;
    cell(cursor)?;
    let mut segments = Vec::new();
    let patrol = has_patrol(raw);
    for (index, checkpoint) in route["checkpoints"].as_array().unwrap().iter().enumerate() {
        match checkpoint["type"].as_str() {
            Some("MOVE") => {
                let target = position(&checkpoint["pos"])?;
                segments.push(segment(
                    grid,
                    cursor,
                    target,
                    Some(index),
                    flying,
                    remaining,
                )?);
                cursor = target;
            }
            Some("APPEAR") => {
                cursor = position(&checkpoint["pos"])?;
                cell(cursor)?;
            }
            Some("WAIT" | "DISAPPEAR") => {}
            _ => return Err("Unsupported public route checkpoint".into()),
        }
    }
    let (checkpoint, end) = if patrol {
        let checkpoints = route["checkpoints"].as_array().unwrap();
        let index = checkpoints
            .iter()
            .position(|cp| cp["type"] == "MOVE")
            .ok_or("Patrol route has no movement checkpoint")?;
        for checkpoint in &checkpoints[..index] {
            if checkpoint["type"] == "APPEAR" {
                cursor = position(&checkpoint["pos"])?;
            }
        }
        (Some(index), position(&checkpoints[index]["pos"])?)
    } else {
        (None, position(&route["end"])?)
    };
    segments.push(segment(grid, cursor, end, checkpoint, flying, remaining)?);
    route["segments"] = json!(segments);
    route["repeatFrom"] = json!(if patrol { Some(0) } else { None::<usize> });
    Ok(route)
}

#[derive(Default)]
pub(super) struct Cache {
    map: Option<Box<MapGeometry>>,
    routes: Option<RouteGeometry>,
    key: String,
}

#[derive(PartialEq)]
struct MapKey {
    stage_id: Value,
    boss: bool,
    overrides: std::collections::BTreeMap<String, bool>,
}

struct MapGeometry {
    key: MapKey,
    grid: Grid,
    fields: String,
}

struct RouteHint {
    base: bool,
    flying: bool,
    gate: Value,
    start: Value,
}

struct RouteGeometry {
    templates: Vec<String>,
    fields: String,
    hints: Vec<RouteHint>,
}

impl MapGeometry {
    fn new(stage: &Value, key: MapKey) -> Result<Self, String> {
        let boss = key.boss;
        let bounds = if boss { Rect::BOSS } else { Rect::NORMAL };
        let mut stage = stage.clone();
        if stage["legend"].is_null() {
            stage["legend"] = stage["tiles"].clone();
        }
        if stage["rows"].is_null() {
            stage["rows"] = stage["grid"].clone();
        }
        let mut grid = Grid::new(&stage, bounds).map_err(str::to_string)?;
        let raw_devices = stage["devices"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if raw_devices.len() > N {
            return Err("Public spatial device capacity exceeded".into());
        }
        let mut devices = Vec::new();
        for device in raw_devices {
            let alias = device["alias"].as_str().or(device["raw"]["alias"].as_str());
            let active = alias
                .and_then(|alias| key.overrides.get(alias))
                .copied()
                .unwrap_or_else(|| {
                    device["active"]
                        .as_bool()
                        .unwrap_or(device["hidden"] != true)
                });
            if !active && device["hidden"] == true {
                continue;
            }
            let at = if device["pos"].is_array() {
                position(&device["pos"])?
            } else {
                position(&json!([device["row"], device["col"]]))?
            };
            let at = cell(at)?;
            let bit = match device["role"].as_str() {
                Some("crate") => Some(CRATE),
                Some("platform" | "mound") => Some(BLOCK),
                _ => None,
            };
            if let Some(bit) = bit.filter(|_| active) {
                grid.set_obstacle(at, true, bit)?;
            }
            let mut visible = json!({"key":device["key"],"alias":alias,"role":device["role"],
                "position":at,"direction":device["dir"],"active":active});
            for key in ["rangeGrid", "rangeTiles"] {
                if let Some(value) = device.get(key) {
                    if value.as_array().is_some_and(|a| a.len() > N) {
                        return Err("Public spatial device range capacity exceeded".into());
                    }
                    visible[key] = value.clone();
                }
            }
            devices.push(visible);
        }
        let mut tiles = Vec::with_capacity(N);
        for (index, tile) in grid.tiles().iter().enumerate() {
            let at = [(index / COLS) as i32, (index % COLS) as i32];
            tiles.push(json!({"position":at,"key":tile.key,"height":tile.height,
                "build":tile.build,"pass":tile.pass,"terrain":tile.terrain,"special":tile.special,
                "obstacle":grid.obstacle(at),"elevated":grid.obstacle(at) & BLOCK != 0,"groundPassable":grid.ground_passable(at,false),
                "flyPassable":grid.fly_passable(at)}));
        }
        let fields = json!({"stageId":&key.stage_id,
            "field":if boss {"bossL"} else {"normal"},"coordinates":"stage",
            "boardOffset":if boss {[7,0]} else {[0,0]},
            "fieldBounds":[bounds.r0,bounds.c0,bounds.r1,bounds.c1],
            "tiles":tiles,"devices":devices})
        .to_string();
        if fields.len() > MAX_BYTES {
            return Err("Public spatial byte capacity exceeded".into());
        }
        Ok(Self { key, grid, fields })
    }
}

impl RouteGeometry {
    fn new(
        templates: &[String],
        waves: &Value,
        boss: bool,
        grid: &mut Grid,
    ) -> Result<Self, String> {
        let mut routes = Vec::new();
        let mut remaining = MAX_PATH_CELLS;
        for template in templates {
            for (set, key) in [("base", "routes"), ("extra", "extraRoutes")] {
                let raw = waves[template][key]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if raw.len() > MAX_ROUTES.saturating_sub(routes.len()) {
                    return Err("Public spatial route capacity exceeded".into());
                }
                for (index, raw) in raw.iter().enumerate() {
                    let mut geometry = route(raw, grid, &mut remaining)?;
                    geometry["templateId"] = json!(template);
                    geometry["routeSet"] = json!(set);
                    geometry["templateRouteIndex"] = json!(index);
                    let row = geometry["start"][0]
                        .as_f64()
                        .ok_or("Missing public route start")?;
                    geometry["gate"] = json!(if row + if boss { 13. } else { 6. } >= 18. {
                        "upper"
                    } else {
                        "lower"
                    });
                    routes.push(geometry);
                }
            }
        }
        let hints = routes
            .iter()
            .map(|route| RouteHint {
                base: route["routeSet"] == "base",
                flying: route["motion"] == "FLY",
                gate: route["gate"].clone(),
                start: route["start"].clone(),
            })
            .collect();
        let fields = json!({"templateIds":templates,"routes":routes}).to_string();
        Ok(Self {
            templates: templates.to_vec(),
            fields,
            hints,
        })
    }
}

impl Session {
    /// Owned compatibility projection. The Python transport uses the serialized
    /// entry point to avoid rebuilding and reserializing the static geometry.
    pub fn public_spatial(&self) -> Result<Value, String> {
        serde_json::from_str(&self.public_spatial_json()?).map_err(|error| error.to_string())
    }

    pub fn public_spatial_json(&self) -> Result<String, String> {
        self.project_spatial(|cache, header, effects, tail| {
            join_fields(&[
                header,
                &cache.map.as_ref().unwrap().fields,
                effects,
                &cache.routes.as_ref().unwrap().fields,
                tail,
            ])
        })
    }

    /// The key identifies public geometry within the catalog. Only resend its
    /// static fields when the consumer does not already hold this key.
    pub fn public_spatial_update(
        &self,
        known_geometry: Option<&str>,
    ) -> Result<(String, Option<String>, String), String> {
        self.project_spatial(|cache, header, effects, tail| {
            let geometry = if known_geometry == Some(cache.key.as_str()) {
                None
            } else {
                Some(join_fields(&[
                    &cache.map.as_ref().unwrap().fields,
                    &cache.routes.as_ref().unwrap().fields,
                ])?)
            };
            Ok((
                cache.key.clone(),
                geometry,
                join_fields(&[header, effects, tail])?,
            ))
        })
    }

    fn project_spatial<T>(
        &self,
        encode: impl FnOnce(&Cache, &str, &str, &str) -> Result<T, String>,
    ) -> Result<T, String> {
        self.healthy()?;
        let phase = match self.run.phase() {
            Phase::Prep => "PREP",
            Phase::Draft => "SP_DRAFT",
            _ => return Err("Public spatial projection requires a decision boundary".into()),
        };
        let run = &self.run;
        let round_number = run.snapshot()?.round;
        let preview = self.current_preview()?;
        let mode = &run
            .data
            .table("config")
            .ok_or("Missing public mode configuration")?["modes"]["mode_single_abyss"];
        let round = &mode["rounds"][round_number.to_string()];
        let boss = round["isBoss"] == true;
        let mut templates = Vec::new();
        if let Some(template) = round["template"].as_str() {
            templates.push(template.to_owned());
        } else {
            let bosses = run
                .data
                .table("bosses")
                .ok_or("Missing public boss definitions")?;
            for (id, template) in round["bossTemplates"].as_object().into_iter().flatten() {
                if preview.iter().any(|enemy| {
                    enemy["boss"] == true && enemy["enemyKey"] == bosses[id]["enemyKey"]
                }) {
                    templates.push(
                        template
                            .as_str()
                            .ok_or("Invalid public route template")?
                            .to_owned(),
                    );
                }
            }
        }
        if templates.is_empty() {
            return Err("Current route template is not publicly identifiable".into());
        }
        let effective_map = run.map.clone().resolved(&run.data)?;
        let key = MapKey {
            stage_id: self.public_setup["stageId"].clone(),
            boss,
            overrides: effective_map.device_overrides,
        };
        let mut cache = self
            .spatial
            .lock()
            .map_err(|_| "Public spatial cache is poisoned")?;
        if cache.map.as_ref().is_none_or(|map| map.key != key) {
            let map = MapGeometry::new(&run.stage, key)?;
            cache.map = Some(Box::new(map));
            cache.routes = None;
        }
        if cache
            .routes
            .as_ref()
            .is_none_or(|routes| routes.templates != templates)
        {
            let waves = run
                .data
                .table("waves")
                .ok_or("Missing public route templates")?;
            let map = cache.map.as_mut().unwrap();
            let routes = RouteGeometry::new(&templates, waves, boss, &mut map.grid)?;
            if map.fields.len() + routes.fields.len() > MAX_BYTES {
                return Err("Public spatial byte capacity exceeded".into());
            }
            let geometry_key = json!([
                &map.key.stage_id,
                map.key.boss,
                &map.key.overrides,
                &templates
            ])
            .to_string();
            if geometry_key.len() > MAX_BYTES {
                return Err("Public spatial key exceeds byte limit".into());
            }
            cache.key = geometry_key;
            cache.routes = Some(routes);
        }
        let map = cache.map.as_ref().unwrap();
        let routes = cache.routes.as_ref().unwrap();
        let associations: Vec<_> = preview
            .iter()
            .enumerate()
            .map(|(index, enemy)| {
                let candidates: Vec<_> = routes
                    .hints
                    .iter()
                    .enumerate()
                    .filter(|(_, route)| {
                        route.base
                            && route.flying == (enemy["fly"] == true)
                            && route.gate == enemy["gate"]
                            && (enemy["start"].is_null() || route.start == enemy["start"])
                    })
                    .map(|(index, _)| index)
                    .collect();
                json!({"previewIndex":index,"routeCandidates":candidates})
            })
            .collect();
        let header = json!({"schema":"arkguardrl/public-spatial/v1","revision":self.revision,
            "round":round_number,"phase":phase})
        .to_string();
        let effects = json!({"mapEffects":run.map.effects}).to_string();
        let tail =
            json!({"previewRoutes":associations,"pathBasis":"public-map-default"}).to_string();
        if header.len() + map.fields.len() + effects.len() + routes.fields.len() + tail.len() - 4
            > MAX_BYTES
        {
            return Err("Public spatial byte capacity exceeded".into());
        }
        encode(&cache, &header, &effects, &tail)
    }
}

fn join_fields(fields: &[&str]) -> Result<String, String> {
    // Each fragment is a nonempty JSON object serialized by serde_json.
    let capacity = fields.iter().map(|s| s.len()).sum::<usize>() - fields.len() + 1;
    if capacity > MAX_BYTES {
        return Err("Public spatial byte capacity exceeded".into());
    }
    let mut out = String::with_capacity(capacity);
    out.push('{');
    for (index, fields) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&fields[1..fields.len() - 1]);
    }
    out.push('}');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_geometry_preserves_patrol_and_teleport_boundaries() {
        let stage = json!({"rows":vec!["r".repeat(COLS); crate::battle::grid::ROWS]});
        let mut grid = Grid::new(&stage, Rect::BOSS).unwrap();
        let mut remaining = MAX_PATH_CELLS;
        let patrol = route(
            &json!({"start":[4,10],"end":[2,2],"steps":[
                {"t":"patrol","p":[3,9]}, {"t":"patrol","p":[2,3]},
                {"t":"patrol","p":[5,9]}
            ]}),
            &mut grid,
            &mut remaining,
        )
        .unwrap();
        assert_eq!(patrol["repeatFrom"], 0);
        let segments = patrol["segments"].as_array().unwrap();
        assert_eq!(segments.last().unwrap()["from"], json!([5., 9.]));
        assert_eq!(segments.last().unwrap()["to"], json!([3., 9.]));
        assert!(segments
            .iter()
            .all(|segment| segment["to"] != json!([2., 2.])));

        let portal = route(
            &json!({"start":[2,10],"end":[2,2],"checkpoints":[
                {"type":"DISAPPEAR","pos":[2,8]}, {"type":"MOVE","pos":[4,3]}
            ]}),
            &mut grid,
            &mut remaining,
        )
        .unwrap();
        assert!(portal["repeatFrom"].is_null());
        assert_eq!(portal["checkpoints"][1]["type"], "APPEAR");
        assert_eq!(portal["segments"][0]["from"], json!([4., 3.]));
        assert_eq!(portal["segments"][0]["to"], json!([4., 3.]));
    }
}
