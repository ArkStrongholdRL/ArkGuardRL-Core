// SPDX-License-Identifier: GPL-3.0-or-later
//! Preparation placement classes; battle pathing keeps its own occupancy grid.
use crate::battle::{
    grid::{Grid, Rect, COLS, N},
    runtime::MapConfig,
};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct DeployMap([u8; 36]);
impl DeployMap {
    pub(in crate::solo) fn public_cells(&self) -> Vec<Value> {
        self.0.iter().enumerate().filter_map(|(i, c)| {
            (*c != 0).then(|| serde_json::json!({"row":9+i/9,"col":2+i%9,"kind":if *c==1{"melee"}else{"ranged"}}))
        }).collect()
    }

    pub fn new(stage: &Value, map: &MapConfig, boss: bool) -> Result<Self, String> {
        let grid = Grid::new(stage, if boss { Rect::BOSS } else { Rect::NORMAL })?;
        let mut out = Self([0; 36]);
        for r in 9..=12 {
            for c in 2..=10 {
                let row = if boss { r - 7 } else { r };
                let tile = &grid.tiles()[row * COLS + c];
                out.0[(r - 9) * 9 + c - 2] = match (tile.height.as_str(), tile.build.as_str()) {
                    ("LOW", "ALL" | "MELEE") => 1,
                    ("HIGH", "ALL" | "RANGED") | ("LOW", "RANGED") => 2,
                    _ => 0,
                };
            }
        }
        let devices = stage["devices"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if devices.len() > N {
            return Err("Stage device capacity exceeded".into());
        }
        for d in devices {
            let row = d["row"].as_i64().or(d["pos"][0].as_i64()).unwrap_or(-100);
            let col = d["col"].as_i64().or(d["pos"][1].as_i64()).unwrap_or(-100);
            let row = row.saturating_add(if boss { 7 } else { 0 });
            if !(9..=12).contains(&row) || !(2..=10).contains(&col) {
                continue;
            }
            let alias = d["alias"].as_str().or(d["raw"]["alias"].as_str());
            let active = alias
                .and_then(|a| map.device_overrides.get(a))
                .copied()
                .unwrap_or_else(|| d["active"].as_bool().unwrap_or(d["hidden"] != true));
            if !active {
                continue;
            }
            let cls = &mut out.0[(row as usize - 9) * 9 + col as usize - 2];
            match d["role"].as_str() {
                Some("crate" | "mound") => *cls = 0,
                Some("platform") => *cls = 2,
                Some("waterPlatform") => {
                    return Err("Water platform is outside current map support".into())
                }
                _ => (),
            }
        }
        Ok(out)
    }
    pub fn can_place(&self, record: &Value, [row, col]: [i32; 2]) -> bool {
        if !(9..=12).contains(&row) || !(2..=10).contains(&col) {
            return false;
        }
        let cls = self.0[(row as usize - 9) * 9 + col as usize - 2];
        let trait_text = record
            .get("traitBase")
            .filter(|v| !v.is_null())
            .unwrap_or(&record["trait"])["desc"]
            .as_str()
            .unwrap_or("");
        let ranged = record["position"] != "MELEE"
            || record["placeOnRanged"] == true
            || trait_text.contains("可以放置于远程位");
        cls == 1 || cls == 2 && ranged
    }
}
