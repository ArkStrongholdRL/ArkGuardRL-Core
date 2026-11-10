// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::deterministic::math::hypot;
impl Host {
    pub(in crate::battle::runtime) fn yan_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "yanShip" && p.tier >= 2)
            .cloned()
        else {
            return Ok(());
        };
        let ops = self.bond_ops(w);
        let mut atk = 0.;
        let mut hp = 0.;
        let mut owner = None;
        for id in ops {
            if !Self::tac_up(w, id)
                || !self
                    .equipment
                    .units
                    .get(&id)
                    .is_some_and(|l| l.bonds.iter().any(|b| b == "yanShip"))
            {
                continue;
            }
            owner.get_or_insert(id);
            let stats = w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
            atk += stats.n("atk");
            hp += stats.n("maxHp");
        }
        let Some(owner) = owner else { return Ok(()) };
        if hp <= 0. {
            return Ok(());
        }
        let share = 0.3;
        for index in 0..if p.tier >= 3 { 2 } else { 1 } {
            let mut best = None;
            let mut score = (3u8, f64::INFINITY, usize::MAX);
            let rect = self.rect;
            for row in rect.r0..=rect.r1 {
                for col in rect.c0..=rect.c1 {
                    let tile = [row, col];
                    if self
                        .bonds
                        .reserved_map_chars
                        .contains(&(row as usize * COLS + col as usize))
                    {
                        continue;
                    }
                    if self
                        .roster
                        .as_ref()
                        .unwrap()
                        .deployment()
                        .reserved(w, tile)?
                    {
                        continue;
                    }
                    let grid = self.grid.as_ref().unwrap();
                    let cell = &grid.tiles()[row as usize * COLS + col as usize];
                    let class = if cell.build == "NONE" {
                        if cell.pass != "ALL" {
                            0
                        } else {
                            1
                        }
                    } else {
                        2
                    };
                    let distance = hypot(
                        row as f64 - (rect.r0 + rect.r1) as f64 / 2.,
                        col as f64 - (rect.c0 + rect.c1) as f64 / 2.,
                    );
                    let key = row as usize * COLS + col as usize;
                    if class < score.0
                        || class == score.0
                            && (distance < score.1 - 1e-9
                                || (distance - score.1).abs() <= 1e-9 && key < score.2)
                    {
                        best = Some(tile);
                        score = (class, distance, key);
                    }
                }
            }
            let Some(tile) = best else { break };
            let id = (kits::dispatch::Context {
                host: self,
                world: w,
                buffs: r,
                unit: owner,
            })
            .summon("enemy_9012_acloon", tile, 0.)?;
            let Some(id) = id else { continue };
            let u = w.unit_mut(id)?;
            u.summoner = None;
            u.stats.edit_base(|base| {
                *base.0.entry("atk".into()).or_default() += atk * share;
                *base.0.entry("maxHp".into()).or_default() += hp * share;
            });
            u.operator.tactical.yanyou_mut()?.hover = if index == 0 { 0. } else { 0.8 };
            if p.tier >= 3 {
                self.equipment_stat(
                    w,
                    r,
                    id,
                    "bond:yanyou".into(),
                    [
                        ("atkMul".into(), p.value("atk", 1.)),
                        (
                            "dmgTakenMul".into(),
                            (1. - p.n("damage_resistance")).max(0.),
                        ),
                    ]
                    .into(),
                )?;
            }
            let u = w.unit_mut(id)?;
            u.stats.hp = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        }
        Ok(())
    }
}
