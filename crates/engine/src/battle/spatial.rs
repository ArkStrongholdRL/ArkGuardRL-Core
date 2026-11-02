// SPDX-License-Identifier: GPL-3.0-or-later
// BattleQueries enemy tile/range indices. Rebuild after movement, before attacks.
use super::{
    body,
    grid::N,
    health::{Id, World},
    targeting::{self, Profile, Queries},
};
use std::collections::BTreeMap;
/// Content may permit a particular healer to bypass a target's noHeal flag.
/// The target profile's noHeal restriction still applies.
pub trait Healing {
    fn heal_through(&mut self, healer: Id, ally: Id, w: &mut World) -> Result<bool, String>;
}
#[derive(Clone, Copy, Debug)]
pub enum RadiusRule {
    Centre,
    Body,
    Collider,
}
/// Raw radius queries intentionally admit undeployed living enemies, as upstream.
/// `foes` adds untargetability/stealth filtering; sleep and flight stay the caller's choice.
pub fn enemies_in_radius<Q: Queries>(
    w: &mut World,
    q: &mut Q,
    enemies: &[Id],
    origin: [f64; 2],
    radius: f64,
    rule: RadiusRule,
    foes: bool,
    limit: usize,
) -> Result<Vec<Id>, String> {
    let expanded = if matches!(rule, RadiusRule::Collider) {
        radius + 0.1
    } else {
        radius
    };
    let r2 = expanded * expanded + 1e-9;
    let mut candidates = vec![];
    for &id in enemies {
        let e = w.unit(id)?;
        if !e.stats.alive || e.hidden {
            continue;
        }
        let inside = if e.hit_area.is_some() && !matches!(rule, RadiusRule::Centre) {
            body::in_radius(e, origin, expanded)
        } else {
            let dx = e.position[1] - origin[1];
            let dy = e.position[0] - origin[0];
            dx * dx + dy * dy <= r2
        };
        // Bound the complete intermediate domain too, never clip before filtering.
        if inside {
            if candidates.len() >= limit {
                return Err("Radius query target capacity exceeded".into());
            }
            candidates.push(id);
        }
    }
    let mut out = vec![];
    for id in candidates {
        if foes {
            let f = w
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .clone();
            if f.contains("untargetable") || f.contains("stealth") && q.enemy_stealthed(id, w)? {
                continue;
            }
        }
        let e = w.unit(id)?;
        if matches!(rule, RadiusRule::Collider)
            && e.hit_area.is_some()
            && !body::in_radius(e, origin, radius)
        {
            continue;
        }
        out.push(id);
    }
    Ok(out)
}
pub fn allies_in_radius(
    w: &World,
    allies: &[Id],
    origin: [f64; 2],
    radius: f64,
    owner: Option<Id>,
    include_devices: bool,
    limit: usize,
) -> Result<Vec<Id>, String> {
    let mut out = vec![];
    let r2 = radius * radius + 1e-9;
    for &id in allies {
        let a = w.unit(id)?;
        if !a.stats.alive
            || !a.deployed
            || a.hidden
            || !include_devices && a.deployment.kind == super::deploy::Kind::Device
            || owner.is_some() && a.owner != owner
        {
            continue;
        }
        let dx = a.position[1] - origin[1];
        let dy = a.position[0] - origin[0];
        if dx * dx + dy * dy <= r2 {
            if out.len() >= limit {
                return Err("Ally radius query target capacity exceeded".into());
            }
            out.push(id);
        }
    }
    Ok(out)
}
pub fn injured_allies<H: Healing>(
    w: &mut World,
    h: &mut H,
    allies: &[Id],
    keys: &[usize],
    healer: Option<Id>,
    include_element: bool,
    limit: usize,
) -> Result<Vec<Id>, String> {
    let mut out = vec![];
    for &id in allies {
        let a = w.unit(id)?;
        if !a.stats.alive
            || !a.deployed
            || a.hidden
            || a.deployment.kind == super::deploy::Kind::Device
        {
            continue;
        }
        let [r, c] = a.deployment.tile;
        // Match tileR * COLS + tileC without turning negative tiles into usize.
        let key = r as i64 * super::grid::COLS as i64 + c as i64;
        if key < 0 || !keys.contains(&(key as usize)) {
            continue;
        }
        if healer != Some(id) {
            let no_heal = w
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("noHeal");
            if no_heal
                && !match healer {
                    Some(healer) => h.heal_through(healer, id, w)?,
                    None => false,
                }
            {
                continue;
            }
            if w.unit(id)?.profile_no_heal {
                continue;
            }
        }
        let a = w.unit_mut(id)?;
        let max_hp = a.stats.stats().map_err(str::to_string)?.n("maxHp");
        let element = a.element_buildup.iter().fold(0.0, |sum, x| sum + x);
        if a.stats.hp < max_hp - 1e-6 || include_element && element > 0.0 {
            if out.len() >= limit {
                return Err("Healing query target capacity exceeded".into());
            }
            let ratio = if max_hp > 0.0 {
                a.stats.hp / max_hp
            } else {
                0.0
            };
            out.push((id, ratio, a.deployment.deploy_seq));
        }
    }
    out.sort_by(|a, b| {
        let delta = a.1 - b.1;
        if delta < 0.0 {
            std::cmp::Ordering::Less
        } else if delta > 0.0 {
            std::cmp::Ordering::Greater
        } else {
            a.2.cmp(&b.2)
        }
    });
    Ok(out.into_iter().map(|x| x.0).collect())
}
pub struct Index {
    range: Vec<Vec<Id>>,
    tiles: Vec<Vec<Id>>,
    multi: BTreeMap<Id, bool>,
    huge: BTreeMap<Id, bool>,
    max_enemies: usize,
    max_rows: usize,
    valid: bool,
}
impl Index {
    pub fn new(max_enemies: usize, max_rows: usize) -> Result<Self, String> {
        if max_enemies == 0 || max_rows == 0 {
            return Err("Spatial index limits must be positive".into());
        }
        Ok(Self {
            range: vec![vec![]; N],
            tiles: vec![vec![]; N],
            multi: BTreeMap::new(),
            huge: BTreeMap::new(),
            max_enemies,
            max_rows,
            valid: false,
        })
    }
    pub fn rebuild(&mut self, w: &World, enemies: &[Id]) -> Result<(), String> {
        self.valid = false;
        for bucket in self.range.iter_mut().chain(self.tiles.iter_mut()) {
            bucket.clear();
        }
        self.multi.clear();
        self.huge.clear();
        if enemies.len() > self.max_enemies {
            return Err("Spatial enemy capacity exceeded".into());
        }
        let mut rows = 0;
        for &id in enemies {
            let e = w.unit(id)?;
            if !e.stats.alive || e.hidden {
                continue;
            }
            let range = body::collider_keys(e);
            let tiles = body::keys(e);
            if self.multi.insert(id, range.len() > 1).is_some() {
                return Err("Duplicate indexed enemy".into());
            }
            self.huge.insert(id, e.hit_area.is_some());
            rows += range.len() + tiles.len();
            if rows > self.max_rows {
                return Err("Spatial row capacity exceeded; no memberships dropped".into());
            }
            for k in range {
                self.range[k].push(id);
            }
            for k in tiles {
                self.tiles[k].push(id);
            }
        }
        self.valid = true;
        Ok(())
    }
    pub fn enemies<Q: Queries>(
        &self,
        w: &mut World,
        q: &mut Q,
        keys: &[usize],
        attacker: Option<Id>,
        profile: Profile,
        tile_rule: bool,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        if !self.valid {
            return Err("Spatial index is invalid".into());
        }
        let mut out = vec![];
        let mut seen = std::collections::BTreeSet::new();
        let index = if tile_rule { &self.tiles } else { &self.range };
        for &key in keys {
            let Some(bucket) = index.get(key) else {
                continue;
            };
            for &id in bucket {
                let dedup = if tile_rule {
                    self.huge[&id]
                } else {
                    self.multi[&id]
                };
                if dedup && !seen.insert(id) {
                    continue;
                }
                if targeting::can_target_enemy(w, q, attacker, id, profile)? {
                    if out.len() >= limit {
                        return Err("Spatial query target capacity exceeded".into());
                    }
                    out.push(id);
                }
            }
        }
        Ok(out)
    }
    pub fn any_enemy(&self, w: &World, keys: &[usize], tile_rule: bool) -> Result<bool, String> {
        if !self.valid {
            return Err("Spatial index is invalid".into());
        }
        let index = if tile_rule { &self.tiles } else { &self.range };
        for &key in keys {
            if let Some(bucket) = index.get(key) {
                for &id in bucket {
                    let e = w.unit(id)?;
                    if e.stats.alive && e.deployed {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}
