// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 SkillRuntime lifecycle and trigger orchestration.
use super::{
    health::{Id, World},
    search::AttackProfile,
    sp::{Bank, Kind},
};
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpType {
    Time,
    Attack,
    Hurt,
    None,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Default,
    SpFull,
    Search,
    CustomRange,
    SkillRange,
    ActiveRange,
    Global,
    TakeDamage,
    Never,
    AutoStop,
}
impl Rule {
    pub fn parse(name: &str) -> Self {
        let name = name.to_uppercase();
        if name.starts_with("CUSTOM_RANGE") {
            return Self::CustomRange;
        }
        match name.as_str() {
            "ALWAYS" | "SP_FULL" => Self::SpFull,
            "SEARCH" => Self::Search,
            "SKILL_RANGE" => Self::SkillRange,
            "ACTIVE_RANGE" => Self::ActiveRange,
            "GDGLOW_SKILL_2" => Self::Global,
            "TAKE_DAMAGE" => Self::TakeDamage,
            "NEVER" | "MANUAL" => Self::Never,
            "AUTO_STOP" => Self::AutoStop,
            _ => Self::Default,
        }
    }
    fn per_tick(&self) -> bool {
        matches!(
            self,
            Self::SpFull
                | Self::Search
                | Self::CustomRange
                | Self::SkillRange
                | Self::ActiveRange
                | Self::Global
        )
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::SpFull => "SP_FULL",
            Self::Search => "SEARCH",
            Self::CustomRange => "CUSTOM_RANGE",
            Self::SkillRange => "SKILL_RANGE",
            Self::ActiveRange => "ACTIVE_RANGE",
            Self::Global => "GDGLOW_SKILL_2",
            Self::TakeDamage => "TAKE_DAMAGE",
            Self::Never => "NEVER",
            Self::AutoStop => "AUTO_STOP",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Spec {
    pub duration: f64,
    pub ammo: f64,
    pub init_sp: f64,
    pub sp_type: SpType,
    pub manual: bool,
    pub rule: Rule,
    pub attack: bool,
    pub targeting: bool,
    pub modifiers: bool,
    pub flags: bool,
    pub range_change: bool,
    pub trigger_allies: bool,
    pub heal_skill: bool,
    pub activate_on_deploy: bool,
    pub on_tick: bool,
    pub on_attack: bool,
    pub extra_ranges: bool,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            duration: 0.0,
            ammo: 0.0,
            init_sp: 0.0,
            sp_type: SpType::Time,
            manual: true,
            rule: Rule::Default,
            attack: false,
            targeting: false,
            modifiers: false,
            flags: false,
            range_change: false,
            trigger_allies: false,
            heal_skill: false,
            activate_on_deploy: false,
            on_tick: false,
            on_attack: false,
            extra_ranges: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Deploy,
    ApplyModifiers,
    RemoveModifiers,
    RefreshRange,
    Start,
    End,
    SkillStart,
    SkillEnd,
    DisplayOn,
    DisplayOff,
    PassiveWindow,
    Tick,
    Attack,
    AmmoUsed,
    SpGain,
}
#[derive(Clone, Debug)]
pub struct Context {
    pub reason: String,
    pub dt: f64,
    pub amount: f64,
    pub no_ammo: bool,
    pub targets: Vec<Id>,
}
impl Context {
    fn reason(reason: &str) -> Self {
        Self {
            reason: reason.into(),
            dt: 0.0,
            amount: 0.0,
            no_ammo: false,
            targets: vec![],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Default,
    Allies,
    TickRule,
    ExtraWithoutAttack,
    AnyCurrentEnemy,
}
/// Every content action is synchronous and can end/reset/reactivate this same skill.
/// Condition handlers only resolve geometric/content predicates; casting policy is
/// owned here. Errors fault the runtime and must not become valid rollout samples.
pub trait Hooks {
    fn can_activate(&mut self, _skill: &Runtime, _w: &mut World) -> Result<bool> {
        Ok(true)
    }
    fn event(
        &mut self,
        event: Event,
        ctx: &mut Context,
        skill: &mut Runtime,
        w: &mut World,
    ) -> Result<()>;
    fn condition(&mut self, which: Condition, skill: &Runtime, w: &mut World) -> Result<bool>;
    fn profile(&mut self, id: Id, w: &mut World) -> Result<Option<AttackProfile>>;
    fn has_sp_hook(&self) -> bool;
}
#[derive(Clone)]
pub struct Runtime {
    pub bank: Bank,
    pub spec: Spec,
    pub pending: bool,
    pub time_left: f64,
    pub ammo_left: f64,
    pub ammo_max: f64,
    pub activations: u64,
    pub last_start: f64,
    pub op_ready_at: f64,
    pub next_cast_at: f64,
    before_attack: bool,
    depth: usize,
    max_depth: usize,
    faulted: bool,
}
impl Runtime {
    pub fn new(bank: Bank, spec: Spec, max_depth: usize) -> Result<Self> {
        if max_depth == 0
            || !spec.duration.is_finite()
            || !spec.ammo.is_finite()
            || spec.ammo < 0.0
            || !spec.init_sp.is_finite()
            || spec.init_sp < 0.0
        {
            return Err("Invalid skill lifecycle limits".into());
        }
        Ok(Self {
            bank,
            spec,
            pending: false,
            time_left: 0.0,
            ammo_left: 0.0,
            ammo_max: 0.0,
            activations: 0,
            last_start: f64::NEG_INFINITY,
            op_ready_at: f64::NEG_INFINITY,
            next_cast_at: f64::NEG_INFINITY,
            before_attack: false,
            depth: 0,
            max_depth,
            faulted: false,
        })
    }
    /// Move the canonical state into a nested callback. The caller must put the
    /// returned state back before using this runtime again; the vacancy is faulted.
    pub(crate) fn take_for_callback(&mut self) -> Result<Self> {
        let mut vacant = Self::new(
            Bank::new(self.unit(), Kind::Passive, 0., 1, 1)?,
            Spec::default(),
            self.max_depth,
        )?;
        vacant.bank.no_skill = true;
        vacant.faulted = true;
        Ok(std::mem::replace(self, vacant))
    }
    pub fn unit(&self) -> Id {
        self.bank.unit
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    fn enter(&mut self) -> Result<()> {
        if self.faulted {
            return Err("Skill runtime is faulted".into());
        }
        if self.depth >= self.max_depth {
            self.faulted = true;
            return Err("Skill callback recursion limit exceeded".into());
        }
        self.depth += 1;
        Ok(())
    }
    fn leave<T>(&mut self, result: Result<T>) -> Result<T> {
        self.depth -= 1;
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    fn emit<H: Hooks>(
        &mut self,
        h: &mut H,
        w: &mut World,
        event: Event,
        ctx: &mut Context,
    ) -> Result<()> {
        h.event(event, ctx, self, w)
    }
    fn apply<H: Hooks>(&mut self, h: &mut H, w: &mut World) -> Result<()> {
        let mut c = Context::reason("");
        if self.spec.modifiers || self.spec.flags {
            self.emit(h, w, Event::ApplyModifiers, &mut c)?;
        }
        if self.spec.range_change {
            self.emit(h, w, Event::RefreshRange, &mut c)?;
        }
        Ok(())
    }
    fn remove<H: Hooks>(&mut self, h: &mut H, w: &mut World) -> Result<()> {
        let mut c = Context::reason("");
        self.emit(h, w, Event::RemoveModifiers, &mut c)?;
        if self.spec.range_change {
            self.emit(h, w, Event::RefreshRange, &mut c)?;
        }
        Ok(())
    }
    pub fn op_cooling(&self, time: f64) -> bool {
        self.spec.manual && time < self.op_ready_at - 1e-9
    }
    pub fn attack_override(&self) -> bool {
        self.bank.active
            && self.spec.attack
            && (self.bank.kind.timed() || self.bank.kind == Kind::Passive || self.pending)
    }
    pub fn targeting_override(&self) -> bool {
        self.bank.active
            && self.spec.targeting
            && (self.bank.kind.timed() || self.bank.kind == Kind::Passive || self.pending)
    }
    pub fn gain<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        amount: f64,
        reason: &str,
        silent: bool,
        ignore_lock: bool,
    ) -> Result<f64> {
        self.enter()?;
        let result = self.gain_inner(w, h, amount, reason, silent, ignore_lock);
        self.leave(result)
    }
    fn gain_inner<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        amount: f64,
        reason: &str,
        silent: bool,
        ignore_lock: bool,
    ) -> Result<f64> {
        if self.bank.no_skill || self.bank.kind == Kind::Passive || !(amount > 0.0) {
            return Ok(0.0);
        }
        if self.bank.active && self.bank.kind.timed() && reason != "init" && !ignore_lock {
            return Ok(0.0);
        }
        if reason != "init" && !ignore_lock && flag(w, self.unit(), "noSp")? {
            return Ok(0.0);
        }
        let mut cost = self.bank.cost(w)?;
        if self.bank.charges >= self.bank.max_charges() as f64 && self.bank.sp >= cost {
            return Ok(0.0);
        }
        let mut amt = amount;
        if !silent && h.has_sp_hook() {
            let mut ctx = Context::reason(reason);
            ctx.amount = amt;
            self.emit(h, w, Event::SpGain, &mut ctx)?;
            amt = if ctx.amount.is_finite() {
                ctx.amount
            } else {
                0.0
            };
            if !(amt > 0.0) || self.bank.no_skill || self.bank.kind == Kind::Passive {
                return Ok(0.0);
            }
            cost = self.bank.cost(w)?;
            if self.bank.charges >= self.bank.max_charges() as f64 && self.bank.sp >= cost {
                return Ok(0.0);
            }
        }
        if cost <= 0.0 {
            return Ok(0.0);
        }
        self.bank.apply_gain(cost, amt)?;
        Ok(amt)
    }
    pub fn reset<H: Hooks>(&mut self, w: &mut World, h: &mut H, carry: Option<f64>) -> Result<()> {
        self.enter()?;
        let result = self.reset_inner(w, h, carry);
        self.leave(result)
    }
    fn reset_inner<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        carry: Option<f64>,
    ) -> Result<()> {
        self.bank.active = false;
        self.pending = false;
        self.time_left = 0.0;
        self.ammo_left = 0.0;
        self.ammo_max = 0.0;
        self.bank.charges = 0.0;
        self.bank.sp = 0.0;
        self.op_ready_at = f64::NEG_INFINITY;
        self.next_cast_at = f64::NEG_INFINITY;
        if self.bank.no_skill {
            return Ok(());
        }
        if self.bank.kind == Kind::Passive {
            self.bank.active = true;
            self.apply(h, w)?;
            self.emit(h, w, Event::Start, &mut Context::reason("passive"))?;
            if self.spec.duration <= 0.0 {
                self.emit(h, w, Event::DisplayOn, &mut Context::reason("passive"))?;
                self.emit(h, w, Event::PassiveWindow, &mut Context::reason("passive"))?;
            }
            return Ok(());
        }
        let initial = carry.filter(|v| v.is_finite()).unwrap_or(self.spec.init_sp);
        self.gain(w, h, initial, "init", true, false)?;
        if self.bank.cost(w)? <= 0.0 {
            self.bank.charges = self.bank.max_charges() as f64;
        }
        if self.spec.activate_on_deploy {
            self.activate(w, h, "deploy", false)?;
        }
        Ok(())
    }
    pub fn activate<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        reason: &str,
        free: bool,
    ) -> Result<bool> {
        self.enter()?;
        let result = self.activate_inner(w, h, reason, free);
        self.leave(result)
    }
    fn activate_inner<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        reason: &str,
        free: bool,
    ) -> Result<bool> {
        if self.bank.no_skill
            || self.bank.kind == Kind::Passive
            || (!free && !self.bank.ready())
            || self.bank.active && self.bank.kind.timed()
        {
            return Ok(false);
        }
        let u = w.unit(self.unit())?;
        if !u.stats.alive || !u.deployed {
            return Ok(false);
        }
        if !h.can_activate(self, w)? {
            return Ok(false);
        }
        if !free {
            let full = self.bank.charges >= self.bank.max_charges() as f64;
            self.bank.charges -= 1.0;
            if self.bank.max_charges() == 1 || full {
                self.bank.sp = 0.0;
            }
        }
        self.activations = self
            .activations
            .checked_add(1)
            .ok_or("Skill activation count exhausted")?;
        self.last_start = w.time;
        if self.spec.manual {
            self.op_ready_at = w.time + 3.0;
        }
        self.bank.active = true;
        if self.bank.kind.timed() {
            self.time_left = if self.bank.kind == Kind::Duration {
                self.spec.duration.max(0.01)
            } else if self.bank.kind == Kind::Ammo && self.spec.duration > 0.0 {
                self.spec.duration
            } else {
                f64::INFINITY
            };
            self.ammo_left = if self.bank.kind == Kind::Ammo {
                self.spec.ammo.max(1.0)
            } else {
                0.0
            };
            self.ammo_max = self.ammo_left;
            self.apply(h, w)?;
        } else {
            self.pending = self.spec.attack;
            if self.spec.modifiers || self.spec.flags || self.spec.targeting {
                self.apply(h, w)?;
            }
        }
        let mut c = Context::reason(reason);
        self.emit(h, w, Event::DisplayOn, &mut c)?;
        self.emit(h, w, Event::Start, &mut c)?;
        self.emit(h, w, Event::SkillStart, &mut c)?;
        if self.bank.active && self.ammo_left > self.ammo_max {
            self.ammo_max = self.ammo_left;
        }
        if !self.bank.kind.timed() && !self.pending {
            self.end(w, h, "instant")?;
            if !self.bank.active
                && !self.spec.attack
                && !self.before_attack
                && w.unit(self.unit())?.stats.alive
            {
                let interval = number(w, self.unit(), "interval")?;
                if interval > 0.0 {
                    self.next_cast_at = w.time + interval;
                }
            }
        }
        Ok(true)
    }
    pub fn end<H: Hooks>(&mut self, w: &mut World, h: &mut H, reason: &str) -> Result<()> {
        self.enter()?;
        let result = self.end_inner(w, h, reason);
        self.leave(result)
    }
    fn end_inner<H: Hooks>(&mut self, w: &mut World, h: &mut H, reason: &str) -> Result<()> {
        if !self.bank.active || (self.bank.kind == Kind::Passive && reason != "death") {
            return Ok(());
        }
        self.bank.active = false;
        self.pending = false;
        self.time_left = 0.0;
        self.ammo_left = 0.0;
        self.ammo_max = 0.0;
        let n = self.activations;
        let mut c = Context::reason(reason);
        self.emit(h, w, Event::End, &mut c)?;
        if self.bank.active || self.activations != n {
            return Ok(());
        }
        self.remove(h, w)?;
        self.emit(h, w, Event::SkillEnd, &mut c)?;
        if self.bank.kind != Kind::Passive {
            self.emit(h, w, Event::DisplayOff, &mut c)?;
        }
        Ok(())
    }
    pub(crate) fn consume_ammo<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<()> {
        self.ammo_left -= 1.;
        self.emit(h, w, Event::AmmoUsed, &mut Context::reason(""))?;
        if self.bank.active && self.ammo_left <= 0. {
            self.end(w, h, "ammo")?;
        }
        Ok(())
    }
    pub fn add_ammo(&mut self, n: f64) {
        if self.bank.active && self.bank.kind == Kind::Ammo && n.is_finite() {
            self.ammo_left += n;
            if self.ammo_left > self.ammo_max {
                self.ammo_max = self.ammo_left;
            }
        }
    }
    pub fn extend(&mut self, seconds: f64) {
        if self.bank.active && self.time_left.is_finite() && seconds.is_finite() {
            self.time_left += seconds;
        }
    }
    pub fn after_attack<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        targets: &[Id],
        used_override: bool,
        no_ammo: bool,
    ) -> Result<()> {
        if targets.len() > w.unit_capacity() {
            self.faulted = true;
            return Err("Skill attack target capacity exceeded".into());
        }
        self.enter()?;
        let result = self.after_inner(w, h, targets, used_override, no_ammo);
        self.leave(result)
    }
    fn after_inner<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        targets: &[Id],
        used_override: bool,
        mut no_ammo: bool,
    ) -> Result<()> {
        if self.bank.no_skill {
            return Ok(());
        }
        let skill_attack =
            self.bank.active && (self.bank.kind.timed() || used_override && self.pending);
        if self.bank.active && self.spec.on_attack {
            let mut c = Context::reason("");
            c.targets = targets.to_vec();
            c.no_ammo = no_ammo;
            self.emit(h, w, Event::Attack, &mut c)?;
            no_ammo = c.no_ammo;
        }
        if self.bank.active && self.bank.kind == Kind::Ammo {
            if !no_ammo {
                self.consume_ammo(w, h)?;
            }
        } else if used_override && self.pending && !self.bank.kind.timed() {
            self.pending = false;
            self.end(w, h, "instant")?;
        }
        if self.spec.sp_type == SpType::Attack
            && !skill_attack
            && !(self.bank.active && self.bank.kind.timed())
        {
            self.gain(w, h, 1.0, "attack", false, false)?;
        }
        Ok(())
    }
    pub fn about_to_attack<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<bool> {
        self.enter()?;
        let result = self.about_inner(w, h);
        self.leave(result)
    }
    fn about_inner<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<bool> {
        if !self.bank.ready()
            || flag(w, self.unit(), "silence")?
            || self.bank.active && self.bank.kind.timed()
            || matches!(self.spec.rule, Rule::TakeDamage | Rule::Never)
            || self.spec.rule.per_tick()
            || self.pending
            || self.op_cooling(w.time)
        {
            return Ok(false);
        }
        let condition = if self.spec.trigger_allies {
            Condition::Allies
        } else {
            Condition::Default
        };
        if !h.condition(condition, self, w)? {
            return Ok(false);
        }
        self.before_attack = true;
        let result = self.activate(w, h, "DEFAULT", false);
        self.before_attack = false;
        result
    }
    pub fn on_damaged<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<()> {
        self.enter()?;
        let result = self.damaged_inner(w, h);
        self.leave(result)
    }
    fn damaged_inner<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<()> {
        if self.bank.no_skill {
            return Ok(());
        }
        if self.spec.sp_type == SpType::Hurt && !(self.bank.active && self.bank.kind.timed()) {
            self.gain(w, h, 1.0, "hurt", false, false)?;
        }
        if self.spec.rule == Rule::TakeDamage
            && self.bank.ready()
            && !self.pending
            && !(self.bank.active && self.bank.kind.timed())
            && can_act(w, self.unit())?
            && !flag(w, self.unit(), "silence")?
            && !self.op_cooling(w.time)
        {
            self.activate(w, h, "TAKE_DAMAGE", false)?;
        }
        Ok(())
    }
    pub fn tick<H: Hooks>(&mut self, w: &mut World, h: &mut H, dt: f64) -> Result<()> {
        self.enter()?;
        let result = self.tick_inner(w, h, dt);
        self.leave(result)
    }
    fn tick_inner<H: Hooks>(&mut self, w: &mut World, h: &mut H, dt: f64) -> Result<()> {
        if self.bank.no_skill {
            return Ok(());
        }
        if self.bank.kind != Kind::Passive && self.bank.sp > self.bank.cost(w)? {
            self.bank.normalize(w)?;
        }
        if self.bank.active {
            if self.spec.on_tick {
                let mut c = Context::reason("");
                c.dt = dt;
                self.emit(h, w, Event::Tick, &mut c)?;
            }
            if self.bank.kind != Kind::Passive
                && (self.bank.kind == Kind::Duration
                    || self.bank.kind == Kind::Ammo && self.time_left.is_finite())
            {
                self.time_left -= dt;
                if self.time_left <= 1e-9 {
                    self.end(w, h, "duration")?;
                }
            }
        }
        if self.spec.sp_type == SpType::Time
            && !(self.bank.active && self.bank.kind.timed())
            && !flag(w, self.unit(), "noSp")?
        {
            let u = w.unit(self.unit())?;
            if u.stats.alive && u.deployed && !u.hidden {
                let rate = number(w, self.unit(), "spRecovery")?;
                if rate > 0.0 {
                    self.gain(w, h, rate * dt, "time", false, false)?;
                }
            }
        }
        if !can_act(w, self.unit())? {
            return Ok(());
        }
        if self.spec.rule == Rule::AutoStop
            && self.bank.active
            && self.time_left < self.spec.duration / 2.0
            && !self.op_cooling(w.time)
            && !flag(w, self.unit(), "silence")?
            && h.condition(Condition::AnyCurrentEnemy, self, w)?
        {
            self.op_ready_at = w.time + 3.0;
            self.end(w, h, "manual")?;
        }
        if self.pending
            && self.spec.trigger_allies
            && self.spec.rule != Rule::SkillRange
            && !h.condition(Condition::Allies, self, w)?
        {
            self.end(w, h, "withdrawn")?;
            self.bank.add_charge(w, 1.0)?;
        }
        if !self.bank.ready()
            || flag(w, self.unit(), "silence")?
            || self.bank.active && self.bank.kind.timed()
            || self.pending
            || self.op_cooling(w.time)
        {
            return Ok(());
        }
        if !self.bank.kind.timed() && !self.spec.attack && w.time < self.next_cast_at - 1e-9 {
            return Ok(());
        }
        if self.spec.rule.per_tick() {
            let yes = if self.spec.rule == Rule::SpFull {
                true
            } else {
                h.condition(Condition::TickRule, self, w)?
            };
            if yes {
                let reason = self.spec.rule.name();
                self.activate(w, h, reason, false)?;
            }
        } else if !matches!(self.spec.rule, Rule::TakeDamage | Rule::Never) {
            let p = h.profile(self.unit(), w)?;
            if p.map(|p| p.no_attack || p.only_while_skill && !self.bank.active)
                .unwrap_or(false)
            {
                if h.condition(Condition::Default, self, w)? {
                    self.activate(w, h, "DEFAULT", false)?;
                }
            } else if self.spec.extra_ranges
                && !self.spec.heal_skill
                && h.condition(Condition::ExtraWithoutAttack, self, w)?
            {
                self.activate(w, h, "DEFAULT", false)?;
            }
        }
        Ok(())
    }
}
fn number(w: &mut World, id: Id, key: &str) -> Result<f64> {
    w.unit_mut(id)?
        .stats
        .stats()
        .map(|s| s.n(key))
        .map_err(str::to_string)
}
fn flag(w: &mut World, id: Id, key: &str) -> Result<bool> {
    w.unit_mut(id)?
        .stats
        .stats()
        .map(|s| s.flags.contains(key))
        .map_err(str::to_string)
}
pub fn can_act(w: &mut World, id: Id) -> Result<bool> {
    let u = w.unit_mut(id)?;
    Ok(u.stats.alive
        && u.deployed
        && !u.hidden
        && !u
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains("stun"))
}
