// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/deploy.js and battle/tiles.js.
use super::{
    grid::{Grid, Rect, COLS, N},
    health::{Id, Side, World},
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Operator,
    Token,
    Enemy,
    Device,
}
#[derive(Clone, Debug)]
pub struct State {
    pub kind: Kind,
    pub uid: Option<u64>,
    pub home: [i32; 2],
    pub tile: [i32; 2],
    pub body: Option<[i32; 2]>,
    pub down_at_home: bool,
    pub deploy_seq: u64,
    pub aggro_seq: u64,
    pub deployed_at: f64,
    pub death_at: f64,
    pub respawn_at: f64,
    pub remove_reason: Option<String>,
    pub removing: bool,
    pub redeploy_mul: f64,
    pub counted: bool,
    pub in_total: bool,
    pub kills: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            kind: Kind::Enemy,
            uid: None,
            home: [0, 0],
            tile: [0, 0],
            body: None,
            down_at_home: false,
            deploy_seq: 0,
            aggro_seq: 0,
            deployed_at: f64::NEG_INFINITY,
            death_at: f64::NEG_INFINITY,
            respawn_at: f64::INFINITY,
            remove_reason: None,
            removing: false,
            redeploy_mul: 1.0,
            counted: false,
            in_total: false,
            kills: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub initial: bool,
    pub tile: Option<[i32; 2]>,
    pub aggro_seq: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Kill,
    EndSkill,
    ReleaseBlocked,
    Unblock,
    RefreshRange,
    Spawn,
    Deploy,
    Die,
    Death,
    Release,
}
/// Concrete skill state, effect removal, body animation and content hooks live in
/// the host. Required methods avoid silently dropping those steps during migration.
pub trait Hooks: super::blocking::Hooks {
    /// Reset attack/element/skill state and apply initial-deployment cooldown.
    /// Carry and retained SP require the future skill runtime and are not inferred here.
    fn initialize(
        &mut self,
        id: Id,
        options: Options,
        world: &mut World,
        deploy: &mut Deployment,
    ) -> Result<(), String>;
    fn event(
        &mut self,
        event: Event,
        id: Id,
        other: Option<Id>,
        world: &mut World,
        deploy: &mut Deployment,
    ) -> Result<(), String>;
    fn remove_temporary_effects(&mut self, id: Id, world: &mut World) -> Result<(), String>;
}
pub struct Deployment {
    rect: Rect,
    order: Vec<Id>,
    board_homes: Vec<(Id, [i32; 2])>,
    capacity: usize,
    occupants: [Option<Id>; N],
    sequence: u64,
    dp: BTreeMap<Id, f64>,
    player_capacity: usize,
    pub dp_max: f64,
    pub killed: u64,
    pub killed_in_total: u64,
}
fn key([r, c]: [i32; 2]) -> usize {
    r as usize * COLS + c as usize
}
impl Deployment {
    pub fn new(
        rect: Rect,
        capacity: usize,
        player_capacity: usize,
        dp_max: f64,
    ) -> Result<Self, String> {
        if !Grid::in_bounds([rect.r0, rect.c0])
            || !Grid::in_bounds([rect.r1, rect.c1])
            || rect.r0 > rect.r1
            || rect.c0 > rect.c1
            || capacity == 0
            || player_capacity == 0
            || !dp_max.is_finite()
            || dp_max < 0.0
        {
            return Err("Invalid deployment limits".into());
        }
        Ok(Self {
            rect,
            order: vec![],
            board_homes: vec![],
            capacity,
            occupants: [None; N],
            sequence: 0,
            dp: BTreeMap::new(),
            player_capacity,
            dp_max,
            killed: 0,
            killed_in_total: 0,
        })
    }
    pub fn register(&mut self, id: Id, w: &World) -> Result<(), String> {
        let u = w.unit(id)?;
        if self.order.len() >= self.capacity || self.order.contains(&id) {
            return Err("Duplicate deployment unit or capacity exceeded".into());
        }
        self.order.push(id);
        if u.side == Side::Ally
            && u.deployment.uid.is_some()
            && matches!(u.deployment.kind, Kind::Operator | Kind::Token)
        {
            if self.board_homes.len() >= self.capacity {
                self.order.pop();
                return Err("Board home capacity exceeded".into());
            }
            self.board_homes.push((id, u.deployment.home));
        }
        Ok(())
    }
    pub fn relocate<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        tile: [i32; 2],
    ) -> Result<bool, String> {
        let u = w.unit(id)?;
        if u.side != Side::Ally || !u.stats.alive || !u.deployed || !self.inside(tile) {
            return Ok(false);
        }
        if let Some(other) = self.occupants[key(tile)] {
            if other != id && w.unit(other)?.stats.alive {
                return Ok(false);
            }
        }
        if self.down_on(w, tile, None)?.is_some() {
            return Ok(false);
        }
        let old = u.deployment.tile;
        if Grid::in_bounds(old) && self.occupants[key(old)] == Some(id) {
            self.occupants[key(old)] = None;
        }
        super::blocking::release(w, h, id)?;
        h.event(Event::ReleaseBlocked, id, None, w, self)?;
        w.unit_mut(id)?.deployment.tile = tile;
        w.unit_mut(id)?.position = [tile[0] as f64, tile[1] as f64];
        self.occupants[key(tile)] = Some(id);
        h.event(Event::RefreshRange, id, None, w, self)?;
        Ok(true)
    }
    pub fn set_dp(&mut self, id: Id, dp: f64) -> Result<(), String> {
        if !dp.is_finite() || dp < 0.0 || dp > self.dp_max {
            return Err("Invalid deployment points".into());
        }
        if !self.dp.contains_key(&id) && self.dp.len() >= self.player_capacity {
            return Err("Deployment player capacity exceeded".into());
        }
        self.dp.insert(id, dp);
        Ok(())
    }
    pub fn occupant(&self, tile: [i32; 2]) -> Option<Id> {
        if Grid::in_bounds(tile) {
            self.occupants[key(tile)]
        } else {
            None
        }
    }
    pub fn dp(&self, id: Id) -> Option<f64> {
        self.dp.get(&id).copied()
    }
    fn inside(&self, [r, c]: [i32; 2]) -> bool {
        r >= self.rect.r0 && r <= self.rect.r1 && c >= self.rect.c0 && c <= self.rect.c1
    }
    pub fn is_down(w: &World, id: Id) -> Result<bool, String> {
        let u = w.unit(id)?;
        let d = &u.deployment;
        Ok(u.side == Side::Ally
            && d.kind == Kind::Operator
            && !u.stats.alive
            && !u.removed
            && d.remove_reason.as_deref() != Some("raid")
            && d.deploy_seq > 0
            && d.respawn_at.is_finite())
    }
    pub fn rest_tile(w: &World, id: Id) -> Result<[i32; 2], String> {
        let d = &w.unit(id)?.deployment;
        Ok(if Self::is_down(w, id)? {
            d.body.unwrap_or(d.tile)
        } else {
            d.home
        })
    }
    pub fn down_on(
        &self,
        w: &World,
        tile: [i32; 2],
        except: Option<Id>,
    ) -> Result<Option<Id>, String> {
        for &id in &self.order {
            if Some(id) == except || !Self::is_down(w, id)? {
                continue;
            }
            let d = &w.unit(id)?.deployment;
            if d.body.unwrap_or(d.tile) == tile {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    pub fn reserved(&self, w: &World, tile: [i32; 2]) -> Result<bool, String> {
        if !Grid::in_bounds(tile) {
            return Ok(true);
        }
        if let Some(id) = self.occupants[key(tile)] {
            if w.unit(id)?.stats.alive {
                return Ok(true);
            }
        }
        for &id in &self.order {
            let u = w.unit(id)?;
            if u.side != Side::Ally
                || u.stats.alive
                || u.removed
                || !matches!(u.deployment.kind, Kind::Operator | Kind::Token)
            {
                continue;
            }
            if Self::rest_tile(w, id)? == tile {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn lay_body(&self, w: &mut World, id: Id) -> Result<(), String> {
        let u = w.unit(id)?;
        let d = &u.deployment;
        let tile = d.tile;
        let home = d.home;
        let down_home = d.down_at_home;
        w.unit_mut(id)?.deployment.body = Some(tile);
        if tile == home {
            return Ok(());
        }
        // A removed board summon still owns its original home for this rule.
        let foreign_home = self
            .board_homes
            .iter()
            .any(|&(other, home)| other != id && home == tile);
        if (down_home || foreign_home) && self.inside(home) && !self.reserved(w, home)? {
            w.unit_mut(id)?.deployment.body = Some(home);
        }
        Ok(())
    }
    pub fn deploy<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        opts: Options,
    ) -> Result<bool, String> {
        if !self.order.contains(&id) {
            return Err("Unit is not registered for deployment".into());
        }
        let u = w.unit(id)?;
        if (u.stats.alive && u.deployed) || u.deployment.removing {
            return Ok(false);
        }
        let tile = opts.tile.unwrap_or(u.deployment.home);
        if !self.inside(tile) {
            if opts.tile.is_none() {
                w.unit_mut(id)?.removed = true;
            }
            return Ok(false);
        }
        if let Some(occ) = self.occupants[key(tile)] {
            let other = w.unit(occ)?;
            if occ != id && other.stats.alive && other.deployed {
                return Ok(false);
            }
        }
        if self.down_on(w, tile, Some(id))?.is_some() {
            return Ok(false);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Deployment sequence exhausted")?;
        let first = u.deployment.deploy_seq == 0;
        self.sequence = sequence;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.alive = true;
        u.deployed = true;
        u.removed = false;
        u.hidden = false;
        u.deployment.body = None;
        u.deployment.down_at_home = false;
        u.deployment.tile = tile;
        u.position = [tile[0] as f64, tile[1] as f64];
        u.deployment.deploy_seq = sequence;
        u.deployment.aggro_seq = opts.aggro_seq.unwrap_or(sequence);
        u.deployment.deployed_at = time;
        u.stats.mark_dirty();
        u.stats.hp = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        self.occupants[key(tile)] = Some(id);
        h.event(Event::RefreshRange, id, None, w, self)?;
        h.initialize(id, opts, w, self)?;
        if first {
            h.event(Event::Spawn, id, None, w, self)?;
        }
        h.event(Event::Deploy, id, None, w, self)?;
        Ok(true)
    }
    pub fn kill<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        killer: Option<Id>,
    ) -> Result<(), String> {
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        w.unit_mut(id)?.stats.hp = 0.0;
        h.event(Event::Kill, id, killer, w, self)?;
        let u = w.unit_mut(id)?;
        if !u.stats.alive {
            if u.pool.is_none() {
                u.stats.hp = 0.0;
            }
            return Ok(());
        }
        if u.stats.hp > 0.0 && u.pool.is_none() {
            u.stats.hp = u
                .stats
                .hp
                .min(u.stats.stats().map_err(str::to_string)?.n("maxHp"));
            return Ok(());
        }
        u.stats.hp = 0.0;
        self.remove(w, h, id, "killed", killer, false)
    }
    pub fn retreat<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        reason: &str,
        permanent: bool,
    ) -> Result<(), String> {
        let u = w.unit(id)?;
        if !u.stats.alive || u.side != Side::Ally {
            return Ok(());
        }
        self.remove(w, h, id, reason, None, permanent)
    }
    fn remove<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        reason: &str,
        killer: Option<Id>,
        permanent: bool,
    ) -> Result<(), String> {
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        u.deployment.death_at = time;
        u.deployment.remove_reason = Some(reason.into());
        u.deployment.removing = true;
        let result = h.event(Event::EndSkill, id, None, w, self);
        w.unit_mut(id)?.deployment.removing = false;
        result?;
        if w.unit(id)?.side == Side::Ally {
            super::blocking::release(w, h, id)?;
            h.event(Event::ReleaseBlocked, id, None, w, self)?;
            let tile = w.unit(id)?.deployment.tile;
            if Grid::in_bounds(tile) && self.occupants[key(tile)] == Some(id) {
                self.occupants[key(tile)] = None;
            }
            h.remove_temporary_effects(id, w)?;
            let u = w.unit_mut(id)?;
            u.stats.mark_dirty();
            if u.deployment.kind == Kind::Operator && !permanent {
                let mul = u.deployment.redeploy_mul
                    * u.stats.stats().map_err(str::to_string)?.n("redeployMul");
                u.deployment.respawn_at =
                    time + (u.stats.base_number("respawnTime") * mul).max(0.0);
                if Self::is_down(w, id)? {
                    self.lay_body(w, id)?;
                }
            } else {
                u.removed = true;
            }
        } else {
            w.unit_mut(id)?.removed = true;
            super::blocking::unblock(w, h, id)?;
            h.event(Event::Unblock, id, None, w, self)?;
            if reason == "killed" {
                let d = &w.unit(id)?.deployment;
                if d.counted {
                    self.killed += 1;
                }
                if d.in_total {
                    self.killed_in_total += 1;
                }
                if let Some(k) = killer {
                    w.unit_mut(k)?.deployment.kills += 1;
                }
            }
        }
        if reason != "leak" {
            h.event(Event::Die, id, killer, w, self)?;
        }
        h.event(Event::Death, id, killer, w, self)?;
        Ok(())
    }
    pub fn redeploy<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        free: bool,
        tile: Option<[i32; 2]>,
    ) -> Result<bool, String> {
        let u = w.unit(id)?;
        if u.side != Side::Ally || u.stats.alive || u.removed {
            return Ok(false);
        }
        let home = u.deployment.home;
        let owner = u.owner;
        let at = if let Some(t) = tile {
            if !self.inside(t) {
                return Ok(false);
            }
            Some(t)
        } else {
            let rest = Self::rest_tile(w, id)?;
            if rest == home {
                None
            } else {
                Some(rest)
            }
        };
        let pos = at.unwrap_or(home);
        if !Grid::in_bounds(pos) {
            return Ok(false);
        }
        if let Some(occ) = self.occupants[key(pos)] {
            if occ != id && w.unit(occ)?.stats.alive {
                return Ok(false);
            }
        }
        if at.is_some() && self.down_on(w, pos, Some(id))?.is_some() {
            return Ok(false);
        }
        let u = w.unit_mut(id)?;
        let cost =
            u.stats.base_number("cost") * u.stats.stats().map_err(str::to_string)?.n("costMul");
        let mut paid = 0.0;
        if !free {
            let Some(dp) = owner.and_then(|p| self.dp.get_mut(&p)) else {
                return Ok(false);
            };
            if *dp + 1e-9 < cost {
                return Ok(false);
            }
            paid = dp.min(cost);
            *dp = (*dp - cost).max(0.0);
        }
        if self.deploy(
            w,
            h,
            id,
            Options {
                tile: at,
                ..Options::default()
            },
        )? {
            return Ok(true);
        }
        if paid > 0.0 {
            if let Some(dp) = owner.and_then(|p| self.dp.get_mut(&p)) {
                *dp = (*dp + paid).min(self.dp_max);
            }
        }
        Ok(false)
    }
    pub fn check_redeploys<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<(), String> {
        let mut i = 0;
        while i < self.order.len() {
            let id = self.order[i];
            i += 1;
            let u = w.unit(id)?;
            if u.side != Side::Ally
                || u.stats.alive
                || u.removed
                || u.deployment.kind != Kind::Operator
                || w.time + 1e-9 < u.deployment.respawn_at
            {
                continue;
            }
            self.redeploy(w, h, id, false, None)?;
        }
        Ok(())
    }
    pub fn release_removed<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<(), String> {
        let release: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|&id| {
                w.unit(id)
                    .map(|u| u.removed && !u.stats.alive)
                    .unwrap_or(false)
            })
            .collect();
        for id in release {
            h.event(Event::Release, id, None, w, self)?;
            if w.unit(id)?.removed && !w.unit(id)?.stats.alive {
                self.order.retain(|&x| x != id);
                w.release_removed(id)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::{
        health::Unit,
        stats::{Base, UnitStats},
    };
    #[derive(Default)]
    struct Host {
        blocked_reentrant: bool,
        reenter: bool,
        releases: usize,
    }
    impl super::super::blocking::Hooks for Host {
        fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
        fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
    }
    impl Hooks for Host {
        fn initialize(
            &mut self,
            _: Id,
            _: Options,
            _: &mut World,
            _: &mut Deployment,
        ) -> Result<(), String> {
            Ok(())
        }
        fn event(
            &mut self,
            e: Event,
            id: Id,
            _: Option<Id>,
            w: &mut World,
            d: &mut Deployment,
        ) -> Result<(), String> {
            if e == Event::EndSkill && self.reenter {
                self.blocked_reentrant = !d.deploy(w, self, id, Options::default())?;
            }
            if e == Event::Release {
                self.releases += 1;
            }
            Ok(())
        }
        fn remove_temporary_effects(&mut self, id: Id, w: &mut World) -> Result<(), String> {
            w.unit_mut(id)?.stats.set_buffs(vec![]);
            Ok(())
        }
    }
    fn make(home: [i32; 2], kind: Kind) -> Unit {
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.0);
        base.0.insert("cost".into(), 7.0);
        base.0.insert("respawnTime".into(), 3.0);
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.stats.alive = false;
        u.owner = Some(1);
        u.deployment.kind = kind;
        u.deployment.home = home;
        u.deployment.tile = home;
        u.deployment.uid = Some(1);
        u
    }
    fn setup() -> (World, Deployment, Host) {
        let mut w = World::new(4, 1, 1).unwrap();
        let mut d = Deployment::new(
            Rect {
                r0: 9,
                r1: 12,
                c0: 2,
                c1: 10,
            },
            4,
            1,
            99.0,
        )
        .unwrap();
        let mut h = Host::default();
        w.insert_unit(1, make([9, 2], Kind::Operator)).unwrap();
        w.insert_unit(2, make([10, 4], Kind::Token)).unwrap();
        for id in [1, 2] {
            d.register(id, &w).unwrap();
            assert!(d.deploy(&mut w, &mut h, id, Options::default()).unwrap());
        }
        d.set_dp(1, 20.0).unwrap();
        (w, d, h)
    }
    #[test]
    fn death_cannot_redeploy_from_skill_end_and_removed_home_is_retained() {
        let (mut w, mut d, mut h) = setup();
        h.reenter = true;
        d.retreat(&mut w, &mut h, 2, "expired", true).unwrap();
        assert!(h.blocked_reentrant);
        d.release_removed(&mut w, &mut h).unwrap();
        assert!(w.unit(2).is_err());
        assert_eq!(h.releases, 1);
        d.release_removed(&mut w, &mut h).unwrap();
        assert_eq!(h.releases, 1);
        assert!(d.relocate(&mut w, &mut h, 1, [10, 4]).unwrap());
        d.kill(&mut w, &mut h, 1, None).unwrap();
        assert_eq!(w.unit(1).unwrap().deployment.body, Some([9, 2]));
    }
    #[test]
    fn body_prevents_deployment_and_failed_payment_is_refunded() {
        let (mut w, mut d, mut h) = setup();
        d.kill(&mut w, &mut h, 1, None).unwrap();
        assert!(d.reserved(&w, [9, 2]).unwrap());
        assert!(!d.relocate(&mut w, &mut h, 2, [9, 2]).unwrap());
        w.insert_unit(3, make([9, 2], Kind::Token)).unwrap();
        d.register(3, &w).unwrap();
        assert!(!d.redeploy(&mut w, &mut h, 3, false, None).unwrap());
        assert_eq!(d.dp(1), Some(20.0));
        d.set_dp(1, 6.0).unwrap();
        w.time = 10.0;
        d.check_redeploys(&mut w, &mut h).unwrap();
        assert!(!w.unit(1).unwrap().stats.alive);
        d.set_dp(1, 7.0).unwrap();
        d.check_redeploys(&mut w, &mut h).unwrap();
        assert!(w.unit(1).unwrap().stats.alive);
        assert_eq!(d.dp(1), Some(0.0));
    }
}
