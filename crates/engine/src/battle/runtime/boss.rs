// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
const POOL: Id = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossKind {
    FinalAssault,
    HiddenCore,
}
pub struct BossConfig {
    pub kind: BossKind,
    pub pool: health::Pool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossState {
    pub kind: BossKind,
    pub hp: f64,
    pub max_hp: f64,
    pub damage: f64,
}
impl BossConfig {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.pool.validate_shared()
    }
}
impl Field {
    pub(crate) fn inherit_boss_damage(&mut self, damage: f64) -> Result<(), String> {
        self.healthy()?;
        if self.core.boss.is_none()
            || self.clock.started()
            || self.finished()
            || !damage.is_finite()
            || damage < 0.
        {
            return Err("Boss damage carry requires an unstarted boss field".into());
        }
        self.core.world.player_mut(1).unwrap().match_boss_damage = damage;
        Ok(())
    }
    pub(crate) fn match_boss_damage(&self) -> Result<f64, String> {
        self.healthy()?;
        Ok(self.core.world.player(1).unwrap().match_boss_damage)
    }
    pub fn new_boss(
        data: Arc<Catalog>,
        stage: &Value,
        template: Template,
        seed: u32,
        time_limit: f64,
        limits: Limits,
        boss: BossConfig,
    ) -> Result<Self, String> {
        Self::build(data, stage, template, seed, time_limit, limits, Some(boss))
    }
    pub fn boss_state(&self) -> Result<Option<BossState>, String> {
        self.healthy()?;
        Ok(self.core.boss.map(|kind| {
            let pool = self.core.world.pool(POOL).unwrap();
            BossState {
                kind,
                hp: pool.hp,
                max_hp: pool.max_hp,
                damage: self.core.world.player(1).unwrap().boss_damage,
            }
        }))
    }
}
impl Host {
    pub(super) fn attach_boss_pool(&mut self, w: &mut World, id: Id) -> Result<(), String> {
        if w.unit(id)?.boss && w.pool(POOL).is_some() {
            w.unit_mut(id)?.pool = Some(POOL);
            Self::sync_boss_hp(w, id)?;
        }
        Ok(())
    }
    fn sync_boss_hp(w: &mut World, id: Id) -> Result<(), String> {
        let pool = w
            .pool(w.unit(id)?.pool.ok_or("Boss unit has no pool")?)
            .ok_or("Missing boss pool")?;
        let hp = pool.hp;
        let max = pool.max_hp;
        let unit = w.unit_mut(id)?;
        if unit.stats.base_number("maxHp") != max {
            unit.stats.edit_base(|b| {
                b.0.insert("maxHp".into(), max);
            });
        }
        unit.stats.hp = unit.stats.stats().map_err(str::to_string)?.n("maxHp") * (hp / max).min(1.);
        Ok(())
    }
    pub(super) fn sync_bosses(&mut self, w: &mut World) -> Result<(), String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        let mut effects = self.buffs.take().unwrap();
        let result = (|| {
            for id in ids {
                if !w.unit(id)?.stats.alive || w.unit(id)?.pool.is_none() {
                    continue;
                }
                Self::sync_boss_hp(w, id)?;
                if w.unit(id)?.stats.hp <= 0. {
                    self.kill_enemy(id, None, &mut effects, w)?;
                }
            }
            Ok(())
        })();
        self.buffs = Some(effects);
        result
    }
}
