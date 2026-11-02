// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/hooks.js. Typed payloads; no Python objects.
// Technical callback failures fault the runtime instead of becoming timeout samples.
use std::collections::BTreeMap;
pub type Handle = u64;
#[derive(Clone, Debug)]
pub struct Hook<K> {
    pub key: K,
    pub token: u64,
    pub priority: f64,
    pub owner: Option<u64>,
    pub once: bool,
    removed: bool,
}
#[derive(Clone, Debug)]
pub struct Timer {
    pub token: u64,
    pub due: f64,
    pub owner: Option<u64>,
    pub interval: f64,
    pub count: Option<u64>,
    pub holds_battle: bool,
    pub cancelled: bool,
}
#[derive(Debug)]
pub struct Context<T> {
    pub value: T,
    pub stop_propagation: bool,
}
impl<T> Context<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            stop_propagation: false,
        }
    }
}
pub trait Host<K: Ord + Clone, T> {
    fn hook(
        &mut self,
        handle: Handle,
        token: u64,
        c: &mut Context<T>,
        runtime: &mut Runtime<K>,
    ) -> Result<(), String>;
    fn timer(
        &mut self,
        _handle: Handle,
        _token: u64,
        _runtime: &mut Runtime<K>,
    ) -> Result<(), String> {
        Err("Battle timer callback handler not installed".into())
    }
}
pub struct Runtime<K> {
    hooks: BTreeMap<Handle, Hook<K>>,
    lists: BTreeMap<K, Vec<Handle>>,
    timers: BTreeMap<Handle, Timer>,
    hook_sequence: u64,
    timer_sequence: u64,
    time: f64,
    depth: usize,
    delivered: usize,
    max_hooks: usize,
    max_timers: usize,
    max_depth: usize,
    max_callbacks: usize,
    faulted: bool,
}
impl<K: Ord + Clone> Runtime<K> {
    pub fn new(
        max_hooks: usize,
        max_timers: usize,
        max_depth: usize,
        max_callbacks: usize,
    ) -> Result<Self, String> {
        if [max_hooks, max_timers, max_depth, max_callbacks].contains(&0) {
            return Err("Battle callback limits must be positive".into());
        }
        Ok(Self {
            hooks: BTreeMap::new(),
            lists: BTreeMap::new(),
            timers: BTreeMap::new(),
            hook_sequence: 0,
            timer_sequence: 0,
            time: 0.0,
            depth: 0,
            delivered: 0,
            max_hooks,
            max_timers,
            max_depth,
            max_callbacks,
            faulted: false,
        })
    }
    pub fn time(&self) -> f64 {
        self.time
    }
    pub fn set_time(&mut self, time: f64) -> Result<(), String> {
        if !time.is_finite() || time < 0.0 {
            return Err("Invalid battle callback time".into());
        }
        self.time = time;
        Ok(())
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    /// Skill, buff and projectile content callbacks can share this recursion and
    /// delivery guard with emitted hooks, matching Battle._safe's role.
    pub fn callback<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.enter()?;
        let result = self.delivery().and_then(|_| f(self));
        self.leave(result)
    }
    fn ready(&self) -> Result<(), String> {
        if self.faulted {
            Err("Battle callbacks are faulted".into())
        } else {
            Ok(())
        }
    }
    fn enter(&mut self) -> Result<(), String> {
        self.ready()?;
        if self.depth >= self.max_depth {
            self.faulted = true;
            return Err("Battle callback depth exhausted".into());
        }
        if self.depth == 0 {
            self.delivered = 0;
        }
        self.depth += 1;
        Ok(())
    }
    fn leave<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        self.depth -= 1;
        if result.is_err() {
            self.faulted = true;
        }
        if self.depth == 0 {
            self.hooks.retain(|_, h| !h.removed);
            self.timers.retain(|_, t| !t.cancelled);
        }
        match result {
            Ok(_) if self.faulted => Err("Battle callback suppressed a technical failure".into()),
            other => other,
        }
    }
    fn delivery(&mut self) -> Result<(), String> {
        if self.delivered >= self.max_callbacks {
            self.faulted = true;
            return Err("Battle callback delivery budget exhausted".into());
        }
        self.delivered += 1;
        Ok(())
    }
    pub fn on(
        &mut self,
        key: K,
        token: u64,
        priority: f64,
        owner: Option<u64>,
        once: bool,
    ) -> Result<Handle, String> {
        self.ready()?;
        if self.hooks.len() >= self.max_hooks {
            self.faulted = true;
            return Err("Battle hook capacity exceeded".into());
        }
        let id = self
            .hook_sequence
            .checked_add(1)
            .ok_or("Battle hook identity exhausted")?;
        self.hook_sequence = id;
        let priority = if priority.is_nan() || priority == 0.0 {
            0.0
        } else {
            priority
        };
        let list = self.lists.entry(key.clone()).or_default();
        let mut index = list.len();
        while index > 0 && self.hooks[&list[index - 1]].priority < priority {
            index -= 1;
        }
        list.insert(index, id);
        self.hooks.insert(
            id,
            Hook {
                key,
                token,
                priority,
                owner,
                once,
                removed: false,
            },
        );
        Ok(id)
    }
    pub fn has_hook(&self, key: &K) -> bool {
        self.lists.contains_key(key)
    }
    pub fn handles(&self, key: &K) -> &[Handle] {
        self.lists.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn get_hook(&self, id: Handle) -> Option<&Hook<K>> {
        self.hooks.get(&id)
    }
    pub fn off(&mut self, id: Handle) {
        if let Some(h) = self.hooks.get_mut(&id) {
            h.removed = true;
            let key = h.key.clone();
            if let Some(list) = self.lists.get_mut(&key) {
                list.retain(|&other| other != id);
                if list.is_empty() {
                    self.lists.remove(&key);
                }
            }
        }
        if self.depth == 0 {
            self.hooks.retain(|_, h| !h.removed);
        }
    }
    pub fn off_key(&mut self, key: &K, token: Option<u64>) {
        let ids = self.handles(key).to_vec();
        for id in ids.into_iter().rev() {
            if token.map(|t| self.hooks[&id].token == t).unwrap_or(true) {
                self.off(id);
            }
        }
    }
    pub fn off_owner(&mut self, owner: Option<u64>) {
        let ids: Vec<_> = self
            .hooks
            .iter()
            .filter(|(_, h)| h.owner == owner)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            self.off(id);
        }
        for t in self.timers.values_mut() {
            if t.owner == owner {
                t.cancelled = true;
            }
        }
    }
    pub fn emit<T, H: Host<K, T>>(
        &mut self,
        key: &K,
        c: &mut Context<T>,
        h: &mut H,
    ) -> Result<(), String> {
        self.ready()?;
        if !self.has_hook(key) {
            return Ok(());
        }
        self.enter()?;
        let snapshot = self.handles(key).to_vec();
        let result = (|| {
            for id in snapshot {
                let record = self.hooks.get(&id).ok_or("Missing retained hook")?;
                if record.removed {
                    continue;
                }
                let token = record.token;
                let once = record.once;
                self.delivery()?;
                let result = h.hook(id, token, c, self);
                if once {
                    self.off(id);
                }
                result?;
                if c.stop_propagation {
                    break;
                }
            }
            Ok(())
        })();
        self.leave(result)
    }
    fn schedule(
        &mut self,
        token: u64,
        due: f64,
        interval: f64,
        owner: Option<u64>,
        holds_battle: bool,
    ) -> Result<Handle, String> {
        self.ready()?;
        if self.timers.len() >= self.max_timers {
            self.faulted = true;
            return Err("Battle timer capacity exceeded".into());
        }
        let id = self
            .timer_sequence
            .checked_add(1)
            .ok_or("Battle timer identity exhausted")?;
        self.timer_sequence = id;
        self.timers.insert(
            id,
            Timer {
                token,
                due,
                interval,
                owner,
                holds_battle,
                count: if interval > 0.0 { Some(0) } else { None },
                cancelled: false,
            },
        );
        Ok(id)
    }
    pub fn after(
        &mut self,
        seconds: f64,
        token: u64,
        owner: Option<u64>,
        holds_battle: bool,
    ) -> Result<Handle, String> {
        let delay = if seconds.is_nan() || seconds <= 0.0 {
            0.0
        } else {
            seconds
        };
        self.schedule(token, self.time + delay, 0.0, owner, holds_battle)
    }
    pub fn every(
        &mut self,
        seconds: f64,
        token: u64,
        owner: Option<u64>,
        immediate: bool,
    ) -> Result<Handle, String> {
        let interval = if seconds.is_nan() || seconds < 1.0 / 30.0 {
            1.0 / 30.0
        } else {
            seconds
        };
        self.schedule(
            token,
            self.time + if immediate { 0.0 } else { interval },
            interval,
            owner,
            false,
        )
    }
    pub fn cancel(&mut self, id: Handle) {
        if let Some(t) = self.timers.get_mut(&id) {
            t.cancelled = true;
        }
    }
    pub fn timer(&self, id: Handle) -> Option<&Timer> {
        self.timers.get(&id)
    }
    pub fn timer_mut(&mut self, id: Handle) -> Option<&mut Timer> {
        self.timers.get_mut(&id)
    }
    pub fn pending(&self) -> usize {
        self.timers.values().filter(|t| !t.cancelled).count()
    }
    pub fn pending_holds(&self) -> bool {
        self.timers.values().any(|t| !t.cancelled && t.holds_battle)
    }
    pub fn run_scheduled<T, H: Host<K, T>>(&mut self, h: &mut H) -> Result<(), String> {
        self.enter()?;
        let now = self.time + 1e-9;
        let mut due: Vec<_> = self
            .timers
            .iter()
            .filter(|(_, t)| !t.cancelled && t.due <= now)
            .map(|(&id, t)| (id, t.due))
            .collect();
        due.sort_by(|a, b| {
            let d = a.1 - b.1;
            if d < 0.0 {
                std::cmp::Ordering::Less
            } else if d > 0.0 {
                std::cmp::Ordering::Greater
            } else {
                a.0.cmp(&b.0)
            }
        });
        let result = (|| {
            for (id, _) in due {
                if self
                    .timers
                    .get(&id)
                    .ok_or("Missing retained timer")?
                    .cancelled
                {
                    continue;
                }
                if self.timers[&id].interval > 0.0 {
                    for _ in 0..8 {
                        let t = &self.timers[&id];
                        if !(t.due <= now) || t.cancelled {
                            break;
                        }
                        let token = t.token;
                        let count = t
                            .count
                            .ok_or("Missing periodic timer count")?
                            .checked_add(1)
                            .ok_or("Battle timer count exhausted")?;
                        self.timers.get_mut(&id).unwrap().count = Some(count);
                        self.delivery()?;
                        h.timer(id, token, self)?;
                        let t = self.timers.get_mut(&id).ok_or("Missing retained timer")?;
                        t.due += t.interval;
                    }
                    if self.timers[&id].due <= now {
                        let t = self.timers.get_mut(&id).unwrap();
                        t.due = self.time + t.interval;
                    }
                } else {
                    let token = self.timers[&id].token;
                    self.timers.get_mut(&id).unwrap().cancelled = true;
                    self.delivery()?;
                    h.timer(id, token, self)?;
                }
            }
            Ok(())
        })();
        self.leave(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        calls: usize,
        reenter: bool,
        fail: bool,
    }
    impl super::Host<u8, ()> for Host {
        fn hook(
            &mut self,
            _: Handle,
            _: u64,
            c: &mut Context<()>,
            r: &mut Runtime<u8>,
        ) -> Result<(), String> {
            self.calls += 1;
            if self.reenter && self.calls < 2 {
                r.emit(&0, c, self)?;
            }
            if self.fail {
                return Err("content callback failed".into());
            }
            Ok(())
        }
    }
    #[test]
    fn once_removes_after_callback_so_nested_emit_can_enter_it_again() {
        let mut r = Runtime::new(4, 4, 4, 16).unwrap();
        r.on(0, 1, 0.0, None, true).unwrap();
        let mut h = Host {
            calls: 0,
            reenter: true,
            fail: false,
        };
        r.emit(&0, &mut Context::new(()), &mut h).unwrap();
        assert_eq!(h.calls, 2);
        assert!(!r.has_hook(&0));
        assert_eq!(r.depth, 0);
        assert!(r.hooks.is_empty());
    }
    #[test]
    fn callback_error_cleans_frames_and_once_registration_then_faults() {
        let mut r = Runtime::new(4, 4, 4, 16).unwrap();
        r.on(0, 1, 0.0, None, true).unwrap();
        let mut h = Host {
            calls: 0,
            reenter: false,
            fail: true,
        };
        assert_eq!(
            r.emit(&0, &mut Context::new(()), &mut h).unwrap_err(),
            "content callback failed"
        );
        assert!(!r.has_hook(&0));
        assert!(r.faulted());
        assert_eq!(r.depth, 0);
        assert!(r.hooks.is_empty());
        assert!(r.run_scheduled(&mut h).is_err());
    }
    #[test]
    fn delivery_and_shared_callback_depth_fail_without_a_valid_continuation() {
        let mut r = Runtime::new(4, 4, 4, 1).unwrap();
        for _ in 0..2 {
            r.on(0, 1, 0.0, None, false).unwrap();
        }
        let mut h = Host {
            calls: 0,
            reenter: false,
            fail: false,
        };
        assert!(r.emit(&0, &mut Context::new(()), &mut h).is_err());
        assert_eq!(h.calls, 1);
        assert!(r.faulted());
        assert_eq!(r.depth, 0);
        let mut r = Runtime::<u8>::new(4, 4, 1, 16).unwrap();
        assert!(r.callback(|r| r.callback(|_| Ok(()))).is_err());
        assert!(r.faulted());
        assert_eq!(r.depth, 0);
    }
    #[test]
    fn capacity_and_owner_removal_preserve_hold_semantics() {
        let mut r = Runtime::<u8>::new(1, 1, 4, 8).unwrap();
        r.after(1.0, 1, Some(9), true).unwrap();
        assert!(r.pending_holds());
        r.off_owner(Some(9));
        assert_eq!(r.pending(), 0);
        assert!(!r.pending_holds());
        let mut h = Host {
            calls: 0,
            reenter: false,
            fail: false,
        };
        r.run_scheduled(&mut h).unwrap();
        assert!(r.timers.is_empty());
        assert!(r.after(1.0, 2, None, true).is_ok());
        assert!(r.after(1.0, 3, None, false).is_err());
        assert!(r.faulted());
        assert_eq!(r.pending(), 1);
    }
}
