// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{body, direction::Placement, targeting};
impl Host {
    fn raid_tile(&mut self, w: &mut World, id: Id, enemy: Id) -> Result<Option<[i32; 2]>, String> {
        let roster = self.roster.as_ref().unwrap();
        let entry = roster.entry(id)?;
        let direction = entry.direction;
        let ranged = !w.unit(id)?.targeting.melee_operator;
        let range = entry.definition.range_grid.clone();
        let extend = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("rangeExtend")
            .max(0.) as u32;
        let block = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("blockCnt")
            > 0.;
        let body = body::keys(w.unit(enemy)?);
        let at = w
            .unit(enemy)?
            .position
            .map(|x| crate::deterministic::math::round_half_even(x) as i32);
        let paths = self
            .spawns
            .as_mut()
            .unwrap()
            .path_tiles(self.grid.as_mut().unwrap());
        let placement = Placement {
            boss_like: self.rect.r0 == 0,
            right_side: false,
            field_coords: false,
            row_offset: None,
            col_offset: 0,
        };
        let mut best = None;
        let mut score = (2u8, f64::INFINITY, [i32::MAX; 2]);
        for row in 0..crate::battle::grid::ROWS as i32 {
            for col in 0..COLS as i32 {
                let tile = [row, col];
                let grid = self.grid.as_ref().unwrap();
                if !placement.on_own_board(tile)
                    || !grid.in_rect(tile)
                    || !grid.can_stand(tile, ranged)
                    || self
                        .roster
                        .as_ref()
                        .unwrap()
                        .deployment()
                        .reserved(w, tile)?
                {
                    continue;
                }
                if !body.iter().any(|k| {
                    (row - (*k / COLS) as i32)
                        .abs()
                        .max((col - (*k % COLS) as i32).abs())
                        <= 2
                }) {
                    continue;
                }
                let reach = targeting::absolute_range(&range, tile, direction, extend);
                if !body.iter().any(|k| reach.contains(k)) {
                    continue;
                }
                let key = row as usize * COLS + col as usize;
                let priority = if !ranged
                    && block
                    && grid.ground_passable(tile, false)
                    && (paths.contains(&key) || body.contains(&key))
                {
                    0
                } else {
                    1
                };
                let delta = [row - at[0], col - at[1]];
                let distance = delta[0].abs().max(delta[1].abs()) as f64
                    + 0.01 * (delta[0].abs() + delta[1].abs()) as f64;
                let local = direction.local(delta);
                if priority < score.0
                    || priority == score.0
                        && (distance < score.1 - 1e-9
                            || (distance - score.1).abs() <= 1e-9 && local < score.2)
                {
                    best = Some(tile);
                    score = (priority, distance, local);
                }
            }
        }
        Ok(best)
    }
    pub(super) fn raid_buff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Bond,
    ) -> Result<(), String> {
        let l = self.bond_layers(&p.id);
        let mut mods = Numbers::new();
        mods.insert("atkPct".into(), p.n("base_atk") + p.n("atk_per_stack") * l);
        mods.insert(
            "hpPct".into(),
            p.n("base_max_hp") + p.n("max_hp_per_stack") * l,
        );
        let aspd = p.n("base_attack_speed") + p.n("attack_speed_per_stack") * l;
        if aspd != 0. {
            mods.insert("aspd".into(), aspd);
        }
        r.add(
            w,
            self,
            id,
            buffs::Spec {
                buff: Buff {
                    key: Some("bond:raidShip".into()),
                    mods,
                    ..Default::default()
                },
                mods_present: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    pub(super) fn raid_poll(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "raidShip")
            .cloned()
        else {
            return Ok(());
        };
        for id in self.bond_ops(w) {
            if !self.equipment_member(id, &p.id) || !Self::tac_up(w, id) || !skills::can_act(w, id)?
            {
                continue;
            }
            let revived =
                self.bonds.raid_revived.remove(&id) == Some(w.unit(id)?.deployment.deploy_seq);
            if revived {
                let tile = w.unit(id)?.deployment.tile;
                for enemy in self.spawns.as_ref().unwrap().live().to_vec() {
                    let at = w
                        .unit(enemy)?
                        .position
                        .map(|x| crate::deterministic::math::round_half_even(x) as i32);
                    if (at[0] - tile[0]).abs() > 1 || (at[1] - tile[1]).abs() > 1 {
                        continue;
                    }
                    if w.unit(enemy)?.stats.alive
                        && !w.unit(enemy)?.hidden
                        && w.unit(enemy)?.blocked_by().is_none()
                    {
                        let mut grid = self.grid.take().unwrap();
                        let result = self.with_kit_buffs(w, r, |h, w| {
                            crate::battle::motion::Hooks::check_block(h, enemy, w, &mut grid)
                        });
                        self.grid = Some(grid);
                        result?;
                    }
                }
                if !w.unit(id)?.blocking().is_empty() {
                    continue;
                }
            }
            let skill = self.roster.as_ref().unwrap().entry(id)?.skill()?;
            let ready = !skill.bank.no_skill
                && ((skill.bank.ready() && !(skill.bank.active && skill.bank.kind.timed()))
                    || skill.bank.active
                        && (skill.bank.kind == crate::battle::sp::Kind::Passive
                            || skill.spec.sp_type == skills::SpType::None));
            let u = w.unit(id)?;
            if !revived
                && !ready
                && w.time - u.last_attack_at.max(u.deployment.deployed_at)
                    < p.value("no_attack_duration", 10.) - 1e-9
            {
                continue;
            }
            let range = self.roster.as_ref().unwrap().range(w, id)?;
            let mut targets = Vec::new();
            let mut occupied = false;
            for enemy in self.spawns.as_ref().unwrap().live().to_vec() {
                let profile = self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(id)?
                    .definition
                    .combat
                    .profile
                    .selection;
                let visible = targeting::can_target_enemy(w, self, Some(id), enemy, profile)?;
                let ground = !targeting::flying(w, enemy)?
                    && targeting::can_target_enemy(
                        w,
                        self,
                        Some(id),
                        enemy,
                        targeting::Profile {
                            ignore_stealth: true,
                            ground_only: true,
                            ..Default::default()
                        },
                    )?;
                if !revived && (visible || ground) && body::in_range(w.unit(enemy)?, &range) {
                    occupied = true;
                    break;
                }
                if ground {
                    let distance = targeting::Queries::remaining_distance(self, enemy, w)?;
                    let taunt = if revived {
                        w.unit_mut(enemy)?
                            .stats
                            .stats()
                            .map_err(str::to_string)?
                            .n("taunt")
                    } else {
                        0.
                    };
                    targets.push((distance, enemy, taunt));
                }
            }
            if occupied {
                continue;
            }
            targets.sort_by(|a, b| {
                b.2.total_cmp(&a.2)
                    .then(a.0.total_cmp(&b.0))
                    .then(a.1.cmp(&b.1))
            });
            for (_, enemy, _) in targets.into_iter().take(8) {
                let Some(tile) = self.raid_tile(w, id, enemy)? else {
                    continue;
                };
                kits::dispatch::Context {
                    host: self,
                    world: w,
                    buffs: r,
                    unit: id,
                }
                .retreat_with_reason("raid")?;
                if w.unit(id)?.stats.alive {
                    break;
                }
                let tiles = Arc::clone(&self.tiles);
                let mut roster = self.roster.take().unwrap();
                let result = roster.redeploy_at(w, r, self, &tiles, id, Some(tile), true);
                self.roster = Some(roster);
                let landed = result?;
                if !landed {
                    let mut roster = self.roster.take().unwrap();
                    let result = roster.redeploy_at(w, r, self, &tiles, id, None, true);
                    self.roster = Some(roster);
                    result?;
                }
                self.rebuild_index(w)?;
                self.flush_kit_events(w, r)?;
                if landed {
                    self.raid_buff(w, r, id, &p)?;
                }
                break;
            }
        }
        Ok(())
    }
}
