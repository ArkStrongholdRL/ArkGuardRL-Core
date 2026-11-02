// SPDX-License-Identifier: GPL-3.0-or-later
// Port of Stronghold Protocol server/sim/grid.js, commit 41bbca57.
use crate::deterministic::math::hypot;
use serde_json::Value;
use std::{collections::VecDeque, sync::Arc};
pub const ROWS: usize = 19;
pub const COLS: usize = 21;
pub const N: usize = ROWS * COLS;
pub const BLOCK: u8 = 1;
pub const CRATE: u8 = 2;
const FOUR: [[i32; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub glyph: String,
    pub key: String,
    pub height: String,
    pub build: String,
    pub pass: String,
    pub terrain: Option<String>,
    pub special: Option<String>,
}
fn text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        s.to_string()
    } else {
        v.to_string()
    }
}
fn first<'a>(e: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|k| e.get(*k).filter(|v| !v.is_null()))
}
impl Tile {
    fn empty() -> Self {
        Self {
            glyph: "X".into(),
            key: "tile_forbidden".into(),
            height: "HIGH".into(),
            build: "NONE".into(),
            pass: "NONE".into(),
            terrain: None,
            special: None,
        }
    }
    pub fn normalize(glyph: &str, e: &Value) -> Self {
        let (h, b, p, k) = match glyph {
            "#" => ("HIGH", "NONE", "FLY", "tile_forbidden"),
            "X" => ("HIGH", "NONE", "NONE", "tile_forbidden"),
            "r" => ("LOW", "ALL", "ALL", "tile_road"),
            "R" => ("LOW", "NONE", "ALL", "tile_road"),
            "h" => ("HIGH", "RANGED", "FLY", "tile_wall"),
            "b" => ("LOW", "ALL", "FLY", "tile_fence_bound"),
            "a" | "A" => ("HIGH", "NONE", "FLY", "tile_achand"),
            "S" => ("LOW", "NONE", "ALL", "tile_start"),
            "E" => ("LOW", "NONE", "ALL", "tile_end"),
            "I" => ("LOW", "NONE", "ALL", "tile_telin"),
            "O" => ("LOW", "NONE", "ALL", "tile_telout"),
            "m" => ("LOW", "ALL", "ALL", "tile_mire"),
            "g" => ("LOW", "ALL", "ALL", "tile_smog"),
            "d" => ("LOW", "NONE", "ALL", "tile_deepsea"),
            "i" => ("LOW", "ALL", "ALL", "tile_infection"),
            _ => ("LOW", "NONE", "ALL", "tile_floor"),
        };
        let key = first(e, &["tileKey", "key", "tile"])
            .map(text)
            .unwrap_or(k.into());
        let height = first(e, &["height", "heightType"])
            .map(|v| {
                if text(v).to_uppercase().contains("HIGH") {
                    "HIGH"
                } else {
                    "LOW"
                }
            })
            .unwrap_or(h)
            .into();
        let build = if key == "tile_deepsea" {
            "NONE".into()
        } else {
            first(e, &["build", "buildable", "buildableType"])
                .map(|v| match v {
                    Value::Bool(true) => "ALL".into(),
                    Value::Bool(false) => "NONE".into(),
                    _ => text(v).to_uppercase(),
                })
                .unwrap_or(b.into())
        };
        let pass = if let Some(v) = first(e, &["pass", "passable", "passableMask"]) {
            let s = text(v).to_uppercase();
            if v == &Value::Bool(true) || matches!(s.as_str(), "ALL" | "GROUND" | "WALK") {
                "ALL"
            } else if s.contains("FLY") {
                "FLY"
            } else if v == &Value::Bool(false) || s == "NONE" || s.is_empty() {
                "NONE"
            } else {
                "ALL"
            }
        } else if let Some(v) = e.get("groundPassable").filter(|v| !v.is_null()) {
            if v == &Value::Bool(true) {
                "ALL"
            } else if e["flyPassable"] == false {
                "NONE"
            } else {
                "FLY"
            }
        } else {
            p
        }
        .into();
        let terrain = first(e, &["terrain"]).map(text).or_else(|| {
            match key.as_str() {
                "tile_mire" => Some("mire"),
                "tile_smog" => Some("smog"),
                "tile_deepsea" | "tile_deepwater" => Some("deepsea"),
                "tile_infection" => Some("infection"),
                _ => match glyph {
                    "m" => Some("mire"),
                    "g" => Some("smog"),
                    "d" => Some("deepsea"),
                    "i" => Some("infection"),
                    _ => None,
                },
            }
            .map(str::to_string)
        });
        let special = first(e, &["special"]).map(text).or_else(|| {
            match key.as_str() {
                "tile_start" => Some("start"),
                "tile_end" => Some("end"),
                "tile_telin" => Some("telin"),
                "tile_telout" => Some("telout"),
                _ => match glyph {
                    "S" => Some("start"),
                    "E" => Some("end"),
                    "I" => Some("telin"),
                    "O" => Some("telout"),
                    _ => None,
                },
            }
            .map(str::to_string)
        });
        Self {
            glyph: glyph.into(),
            key,
            height,
            build,
            pass,
            terrain,
            special,
        }
    }
    pub fn blockable(&self) -> bool {
        self.height == "LOW" && matches!(self.build.as_str(), "ALL" | "MELEE")
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub r0: i32,
    pub r1: i32,
    pub c0: i32,
    pub c1: i32,
}
impl Rect {
    // Simulation bounds include the goal columns outside the deployable board.
    pub const NORMAL: Self = Self {
        r0: 9,
        r1: 12,
        c0: 0,
        c1: 10,
    };
    pub const BOSS: Self = Self {
        r0: 0,
        r1: 5,
        c0: 0,
        c1: 20,
    };
    pub const FULL: Self = Self {
        r0: 0,
        r1: 18,
        c0: 0,
        c1: 20,
    };
}
#[derive(Clone, Debug)]
pub struct Flow {
    pub dest: i32,
    pub dist: [i32; N],
    pub parent: [i32; N],
    pub pen: [i32; N],
    pub next: [i32; N],
    pub official: [i32; N],
    pub cost: [i32; N],
    pub deep: [i32; N],
}
#[derive(Clone)]
pub struct Grid {
    tiles: Vec<Tile>,
    rect: Rect,
    obstacle: [u8; N],
    unblockable: [i32; N],
    water: [i32; N],
    version: u64,
    cache: VecDeque<((i32, i32, bool, bool), Arc<Flow>)>,
}
fn coord(k: usize) -> [i32; 2] {
    [(k / COLS) as i32, (k % COLS) as i32]
}
fn key([r, c]: [i32; 2]) -> usize {
    r as usize * COLS + c as usize
}
impl Grid {
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn new(stage: &Value, rect: Rect) -> Result<Self, &'static str> {
        if rect.r0 < 0
            || rect.c0 < 0
            || rect.r1 >= ROWS as i32
            || rect.c1 >= COLS as i32
            || rect.r0 > rect.r1
            || rect.c0 > rect.c1
        {
            return Err("Invalid simulation rectangle");
        }
        let mut tiles = Vec::with_capacity(N);
        for r in 0..ROWS {
            let line = &stage["rows"][r];
            let line = if let Some(s) = line.as_str() {
                s.to_string()
            } else if let Some(a) = line.as_array() {
                a.iter()
                    .map(|v| if v.is_null() { String::new() } else { text(v) })
                    .collect::<String>()
            } else {
                String::new()
            };
            let glyphs: Vec<char> = line.chars().collect();
            for c in 0..COLS {
                tiles.push(if let Some(g) = glyphs.get(c) {
                    let g = g.to_string();
                    Tile::normalize(&g, &stage["legend"][&g])
                } else {
                    Tile::empty()
                });
            }
        }
        let unblockable =
            std::array::from_fn(|k| i32::from(tiles[k].pass == "ALL" && !tiles[k].blockable()));
        let water = std::array::from_fn(|k| {
            i32::from(tiles[k].pass == "ALL" && tiles[k].terrain.as_deref() == Some("deepsea"))
        });
        Ok(Self {
            tiles,
            rect,
            obstacle: [0; N],
            unblockable,
            water,
            version: 0,
            cache: VecDeque::new(),
        })
    }
    pub fn in_bounds([r, c]: [i32; 2]) -> bool {
        r >= 0 && r < ROWS as i32 && c >= 0 && c < COLS as i32
    }
    fn in_rect(&self, [r, c]: [i32; 2]) -> bool {
        r >= self.rect.r0 && r <= self.rect.r1 && c >= self.rect.c0 && c <= self.rect.c1
    }
    pub fn walkable(&self, p: [i32; 2], ignore: bool) -> bool {
        self.in_rect(p)
            && self.tiles[key(p)].pass == "ALL"
            && (ignore || self.obstacle[key(p)] & BLOCK == 0)
    }
    pub fn ground_passable(&self, p: [i32; 2], ignore: bool) -> bool {
        self.in_rect(p)
            && self.tiles[key(p)].pass == "ALL"
            && (ignore || self.obstacle[key(p)] == 0)
    }
    pub fn fly_passable(&self, p: [i32; 2]) -> bool {
        self.in_rect(p) && self.tiles[key(p)].pass != "NONE"
    }
    pub fn can_stand(&self, p: [i32; 2], ranged: bool) -> bool {
        if !Self::in_bounds(p) {
            return false;
        }
        let t = &self.tiles[key(p)];
        if t.build == "NONE" {
            return false;
        }
        if ranged {
            return matches!(t.build.as_str(), "ALL" | "RANGED")
                || t.height == "LOW" && t.build == "MELEE";
        }
        self.obstacle[key(p)] & BLOCK == 0 && t.blockable()
    }
    pub fn obstacle(&self, p: [i32; 2]) -> u8 {
        if Self::in_bounds(p) {
            self.obstacle[key(p)]
        } else {
            0
        }
    }
    pub fn set_obstacle(&mut self, p: [i32; 2], on: bool, bit: u8) -> Result<(), &'static str> {
        if bit != BLOCK && bit != CRATE {
            return Err("Unknown obstacle kind");
        }
        if !Self::in_bounds(p) {
            return Ok(());
        }
        let k = key(p);
        let next = if on {
            self.obstacle[k] | bit
        } else {
            self.obstacle[k] & !bit
        };
        if next != self.obstacle[k] {
            self.obstacle[k] = next;
            self.version += 1;
            self.cache.clear();
        }
        Ok(())
    }
    fn spfa(&self, dest: usize, ignore: bool, penalty: bool) -> ([i32; N], [i32; N], [i32; N]) {
        let (mut dist, mut parent, mut pen) = ([-1; N], [-1; N], [0; N]);
        let mut queued = [false; N];
        let mut q = VecDeque::with_capacity(N);
        dist[dest] = 0;
        q.push_back(dest);
        queued[dest] = true;
        while let Some(cur) = q.pop_front() {
            queued[cur] = false;
            let [r, c] = coord(cur);
            for [dr, dc] in FOUR {
                let p = [r + dr, c + dc];
                if !self.walkable(p, ignore) {
                    continue;
                }
                let nb = key(p);
                let nd = dist[cur]
                    + if !ignore && self.obstacle[nb] & CRATE != 0 {
                        1000
                    } else {
                        1
                    };
                let np = if penalty {
                    pen[cur] + self.unblockable[nb]
                } else {
                    0
                };
                if dist[nb] < 0 || nd < dist[nb] || (nd == dist[nb] && np < pen[nb]) {
                    dist[nb] = nd;
                    pen[nb] = np;
                    parent[nb] = cur as i32;
                    if !queued[nb] {
                        q.push_back(nb);
                        queued[nb] = true;
                    }
                }
            }
        }
        (dist, parent, pen)
    }
    fn clear(&self, p: [i32; 2], ignore: bool) -> bool {
        self.walkable(p, ignore) && (ignore || self.obstacle[key(p)] & CRATE == 0)
    }
    fn build_field(&self, end: [i32; 2], diagonal: bool, ignore: bool) -> Flow {
        let mut f = Flow {
            dest: -1,
            dist: [-1; N],
            parent: [-1; N],
            pen: [0; N],
            next: [-1; N],
            official: [-1; N],
            cost: [0; N],
            deep: [0; N],
        };
        if !Self::in_bounds(end) {
            return f;
        }
        let dest = key(end);
        f.dest = dest as i32;
        let (_, official, _) = self.spfa(dest, ignore, false);
        let (dist, parent, pen) = self.spfa(dest, ignore, true);
        let next_o = smooth(&official, &dist, |a, b| {
            ray(coord(a), coord(b), diagonal, |p| self.clear(p, ignore))
        });
        // For preference rays, membership must refer to the original unsmoothed chain.
        let mut next_p = parent;
        for n in 0..N {
            if dist[n] < 0 || next_p[n] < 0 {
                continue;
            }
            let mut chain = [false; N];
            let mut x = n as i32;
            for _ in 0..N {
                if x < 0 {
                    break;
                }
                chain[x as usize] = true;
                x = parent[x as usize];
            }
            let mut b = next_p[n] as usize;
            while next_p[b] >= 0 {
                let to = next_p[b] as usize;
                if !ray(coord(n), coord(to), diagonal, |p| {
                    self.clear(p, ignore)
                        && (self.unblockable[key(p)] == 0
                            || chain[key(p)] && dist[key(p)] >= dist[to])
                }) {
                    break;
                }
                b = to;
            }
            next_p[n] = b as i32;
        }
        let mut order: Vec<_> = (0..N).filter(|&k| dist[k] >= 0).collect();
        order.sort_by_key(|&k| (dist[k], k));
        let through = |a: usize, b: usize, layer: &[i32; N]| {
            segment_tiles(coord(a), coord(b))
                .iter()
                .map(|&p| layer[key(p)])
                .sum::<i32>()
                - layer[a]
        };
        for k in order {
            let o = next_o[k];
            let p = next_p[k];
            if o < 0 {
                continue;
            }
            let o = o as usize;
            let mut use_ = o;
            let mut best = through(k, o, &self.unblockable) + f.cost[o];
            let mut wet = through(k, o, &self.water) + f.deep[o];
            if p >= 0 && p as usize != o {
                let p = p as usize;
                let cp = through(k, p, &self.unblockable) + f.cost[p];
                let wp = through(k, p, &self.water) + f.deep[p];
                if wp <= wet
                    && (cp < best
                        || (cp == best
                            && f.next[o] == p as i32
                            && on_segment(coord(k), coord(o), coord(p))))
                {
                    use_ = p;
                    best = cp;
                    wet = wp;
                }
            }
            f.next[k] = use_ as i32;
            f.cost[k] = best;
            f.deep[k] = wet;
        }
        f.dist = dist;
        f.parent = parent;
        f.pen = pen;
        f.official = next_o;
        f
    }
    pub fn flow_field(&mut self, end: [i32; 2], diagonal: bool, ignore: bool) -> Arc<Flow> {
        let ck = (end[0], end[1], diagonal, ignore);
        if let Some((_, f)) = self.cache.iter().find(|(k, _)| *k == ck) {
            return Arc::clone(f);
        }
        let f = Arc::new(self.build_field(end, diagonal, ignore));
        if self.cache.len() >= 64 {
            self.cache.pop_front();
        }
        self.cache.push_back((ck, Arc::clone(&f)));
        f
    }
    pub fn waypoints(
        &mut self,
        start: [i32; 2],
        end: [i32; 2],
        diagonal: bool,
        ignore: bool,
    ) -> Option<Vec<[i32; 2]>> {
        if !Self::in_bounds(start) || !Self::in_bounds(end) {
            return None;
        }
        if start == end {
            return Some(vec![start]);
        }
        let f = self.flow_field(end, diagonal, ignore);
        let mut k = key(start);
        if f.dist[k] < 0 {
            return None;
        }
        let mut out = vec![start];
        for _ in 0..N {
            let next = f.next[k];
            if next < 0 {
                return None;
            }
            k = next as usize;
            out.push(coord(k));
            if k == key(end) {
                return Some(out);
            }
        }
        None
    }
    pub fn find_path(
        &mut self,
        start: [i32; 2],
        end: [i32; 2],
        ignore: bool,
    ) -> Option<Vec<[i32; 2]>> {
        let wp = self.waypoints(start, end, true, ignore)?;
        let mut out = vec![start];
        for pair in wp.windows(2) {
            out.extend(bresenham_tiles(pair[0], pair[1]).into_iter().skip(1));
        }
        Some(out)
    }
    pub fn path_length(points: &[[i32; 2]]) -> f64 {
        points.windows(2).fold(0.0, |s, p| {
            s + hypot((p[1][0] - p[0][0]) as f64, (p[1][1] - p[0][1]) as f64)
        })
    }

    /// Smoothed geometric route length, preserving the upstream parent-chain sum order.
    pub fn field_length(field: &Flow, start: [i32; 2]) -> f64 {
        if !Self::in_bounds(start) || field.dist[key(start)] < 0 {
            return f64::INFINITY;
        }
        let mut stack = Vec::new();
        let mut k = key(start);
        while k as i32 != field.dest && field.next[k] >= 0 {
            let next = field.next[k] as usize;
            let a = coord(k);
            let b = coord(next);
            stack.push(hypot((b[0] - a[0]) as f64, (b[1] - a[1]) as f64));
            k = next;
            if stack.len() > N {
                return f64::INFINITY;
            }
        }
        stack.into_iter().rev().fold(0.0, |s, d| s + d)
    }

    /// Visibility for an off-centre enemy moving toward a tile centre. Start/end
    /// cells are exempt, but both side cells must be clear at a diagonal corner.
    pub fn straight_clear(&self, x: f64, y: f64, end: [i32; 2]) -> bool {
        if !x.is_finite() || !y.is_finite() || !Self::in_bounds(end) {
            return false;
        }
        // JS Math.round: ties toward +infinity, unlike the targeting round-half-even.
        let mut c = (x.floor() + f64::from(x - x.floor() >= 0.5)) as i32;
        let mut r = (y.floor() + f64::from(y - y.floor() >= 0.5)) as i32;
        let dx = end[1] as f64 - x;
        let dy = end[0] as f64 - y;
        let sc = if dx > 0.0 { 1 } else { -1 };
        let sr = if dy > 0.0 { 1 } else { -1 };
        let ddx = if dx != 0.0 {
            (1.0 / dx).abs()
        } else {
            f64::INFINITY
        };
        let ddy = if dy != 0.0 {
            (1.0 / dy).abs()
        } else {
            f64::INFINITY
        };
        let mut tx = if dx != 0.0 {
            (c as f64 + 0.5 * sc as f64 - x) / dx
        } else {
            f64::INFINITY
        };
        let mut ty = if dy != 0.0 {
            (r as f64 + 0.5 * sr as f64 - y) / dy
        } else {
            f64::INFINITY
        };
        let clear = |p| p == end || self.clear(p, false);
        for _ in 0..4 * COLS {
            if [r, c] == end || (tx >= 1.0 && ty >= 1.0) {
                break;
            }
            if (tx - ty).abs() < 1e-9 {
                if !clear([r, c + sc]) || !clear([r + sr, c]) {
                    return false;
                }
                c += sc;
                r += sr;
                tx += ddx;
                ty += ddy;
            } else if tx < ty {
                c += sc;
                tx += ddx;
            } else {
                r += sr;
                ty += ddy;
            }
            if !clear([r, c]) {
                return false;
            }
        }
        true
    }
}
fn smooth(parent: &[i32; N], dist: &[i32; N], los: impl Fn(usize, usize) -> bool) -> [i32; N] {
    let mut next = *parent;
    for n in 0..N {
        if dist[n] < 0 || next[n] < 0 {
            continue;
        }
        let mut b = next[n] as usize;
        while next[b] >= 0 && los(n, next[b] as usize) {
            b = next[b] as usize;
        }
        next[n] = b as i32;
    }
    next
}
fn on_segment([ar, ac]: [i32; 2], [br, bc]: [i32; 2], [cr, cc]: [i32; 2]) -> bool {
    (br - ar) * (cc - ac) == (bc - ac) * (cr - ar)
        && (br - ar) * (cr - br) + (bc - ac) * (cc - bc) > 0
}
fn ray(a: [i32; 2], b: [i32; 2], diagonal: bool, clear: impl Fn([i32; 2]) -> bool) -> bool {
    if !diagonal && a[0] != b[0] && a[1] != b[1] {
        return false;
    }
    let line = bresenham_tiles(a, b);
    if !clear(a) {
        return false;
    }
    for p in line.windows(2) {
        let [r, c] = p[0];
        let [nr, nc] = p[1];
        if nr != r && nc != c && (!clear([nr, c]) || !clear([r, nc])) {
            return false;
        }
        if !clear([nr, nc]) {
            return false;
        }
    }
    true
}
pub fn bresenham_tiles([mut r, mut c]: [i32; 2], [r1, c1]: [i32; 2]) -> Vec<[i32; 2]> {
    let dr = (r1 - r).abs();
    let dc = (c1 - c).abs();
    let sr = if r1 > r { 1 } else { -1 };
    let sc = if c1 > c { 1 } else { -1 };
    let mut err = dc - dr;
    let mut out = vec![[r, c]];
    for _ in 0..256 {
        if r == r1 && c == c1 {
            break;
        }
        let e = 2 * err;
        if e > -dr {
            err -= dr;
            c += sc;
        }
        if e < dc {
            err += dc;
            r += sr;
        }
        out.push([r, c]);
    }
    out
}
pub fn segment_tiles([mut r, mut c]: [i32; 2], [r1, c1]: [i32; 2]) -> Vec<[i32; 2]> {
    let dy = (r1 - r).abs();
    let dx = (c1 - c).abs();
    let sr = if r1 > r { 1 } else { -1 };
    let sc = if c1 > c { 1 } else { -1 };
    let (mut kx, mut ky) = (1, 1);
    let mut out = vec![[r, c]];
    while r != r1 || c != c1 {
        let tx = if kx <= dx {
            (2 * kx - 1) * dy
        } else {
            i32::MAX
        };
        let ty = if ky <= dy {
            (2 * ky - 1) * dx
        } else {
            i32::MAX
        };
        if tx == ty {
            c += sc;
            r += sr;
            kx += 1;
            ky += 1;
        } else if tx < ty {
            c += sc;
            kx += 1;
        } else {
            r += sr;
            ky += 1;
        }
        out.push([r, c]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open() -> Grid {
        Grid::new(
            &serde_json::json!({"rows": vec!["r".repeat(COLS); ROWS]}),
            Rect::FULL,
        )
        .unwrap()
    }
    #[test]
    fn cache_is_bounded_and_invalidated_only_on_changes() {
        let mut g = open();
        let before = g.flow_field([9, 2], true, false);
        assert!(Arc::ptr_eq(&before, &g.flow_field([9, 2], true, false)));
        g.set_obstacle([9, 8], false, CRATE).unwrap();
        assert!(Arc::ptr_eq(&before, &g.flow_field([9, 2], true, false)));
        g.set_obstacle([9, 8], true, CRATE).unwrap();
        let after = g.flow_field([9, 2], true, false);
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(before.dist[key([9, 8])], 6); // old output still owns its snapshot
        assert_eq!(after.dist[key([9, 8])], 1005);
        for k in 0..N {
            g.flow_field(coord(k), true, false);
        }
        assert_eq!(g.cache.len(), 64);
    }
    #[test]
    fn corner_clearance_and_obstacle_bits_are_independent() {
        let mut g = open();
        g.set_obstacle([9, 8], true, CRATE).unwrap();
        g.set_obstacle([9, 8], true, BLOCK).unwrap();
        g.set_obstacle([9, 8], false, BLOCK).unwrap();
        assert!(g.walkable([9, 8], false));
        assert!(!g.ground_passable([9, 8], false));
        assert!(!g.straight_clear(7.0, 9.0, [10, 8]));
        g.set_obstacle([9, 8], false, CRATE).unwrap();
        assert!(g.straight_clear(7.0, 9.0, [10, 8]));
        assert!(g.find_path([-1, 0], [9, 2], false).is_none());
        assert_eq!(g.waypoints([9, 2], [9, 2], true, false), Some(vec![[9, 2]]));
    }
}
