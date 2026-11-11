// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::deterministic::math::hypot;
impl Host {
    pub(in crate::battle::runtime::enemies::bosses) fn boss_spawn_branch(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        branch: &str,
        fallback_key: &str,
        mods: Value,
        fallback_motion: &str,
    ) -> Result<Vec<Id>, String> {
        let phase = self.spawns.as_ref().unwrap().branch_phase(branch);
        let mut specs = vec![];
        if let Some(phase) = phase {
            for s in phase {
                if s["action"]
                    .as_str()
                    .is_some_and(|a| !a.eq_ignore_ascii_case("SPAWN"))
                {
                    continue;
                }
                let Some(key) = s["key"]
                    .as_str()
                    .or_else(|| s["enemyKey"].as_str())
                    .filter(|k| k.starts_with("enemy_"))
                else {
                    continue;
                };
                let count = s["count"]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .unwrap_or(1.)
                    .max(1.)
                    .ceil();
                if count > w.unit_capacity() as f64 || count > usize::MAX as f64 {
                    return Err("Branch spawn count exceeds unit capacity".into());
                }
                let route = s["routeIndex"]
                    .as_u64()
                    .and_then(|index| {
                        self.spawns
                            .as_ref()
                            .unwrap()
                            .extra_routes()
                            .get(index as usize)
                    })
                    .cloned();
                let route = route
                    .unwrap_or_else(|| self.boss_goal_route(w.unit(id).unwrap().position, "WALK"));
                let mut options = serde_json::json!({"route":route,"tag":s["tag"],"mods":mods});
                if options["route"]["start"].is_null() {
                    options["pos"] = serde_json::json!(w.unit(id)?.position);
                }
                if s["unharmful"] == true {
                    options["countInTotal"] = Value::Bool(false);
                }
                specs.push((key.to_owned(), count as usize, options));
            }
        } else {
            let position = w.unit(id)?.position;
            let options = serde_json::json!({"pos":position,"route":self.boss_goal_route(position,fallback_motion),"mods":mods});
            specs.push((fallback_key.to_owned(), 1, options));
        }
        let mut spawned = vec![];
        for (key, count, options) in specs {
            if spawned
                .len()
                .checked_add(count)
                .is_none_or(|n| n > w.unit_capacity())
            {
                return Err("Branch spawn count exceeds unit capacity".into());
            }
            for _ in 0..count {
                spawned.push(self.with_spawns(w, r, |spawns, context, world| {
                    spawns.spawn_runtime(world, context, &key, &options)
                })?);
            }
        }
        self.rebuild_index(w)?;
        Ok(spawned)
    }
    fn boss_goal_route(&self, position: [f64; 2], motion: &str) -> Value {
        let grid = self.grid.as_ref().unwrap();
        let mut best = position.map(|v| (v + 0.5).floor() as i32);
        let mut distance = f64::INFINITY;
        for (key, tile) in grid.tiles().iter().enumerate() {
            if tile.special.as_deref() != Some("end") {
                continue;
            }
            let p = [
                key / crate::battle::grid::COLS,
                key % crate::battle::grid::COLS,
            ]
            .map(|v| v as i32);
            let d = hypot(p[1] as f64 - position[1], p[0] as f64 - position[0]);
            if d < distance {
                distance = d;
                best = p;
            }
        }
        serde_json::json!({"motion":motion,"start":position,"end":best,"checkpoints":[]})
    }
}
