// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded access to battle operations from an operator callback.
use super::*;

pub(crate) struct Context<'a> {
    pub(in crate::battle::runtime) host: &'a mut Host,
    pub(in crate::battle::runtime) world: &'a mut World,
    pub(in crate::battle::runtime) buffs: &'a mut buffs::Runtime,
    pub(in crate::battle::runtime) unit: Id,
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn up(&self) -> bool {
        Host::tac_up(self.world, self.unit)
    }
    pub(in crate::battle::runtime) fn active(&self) -> Result<bool, String> {
        Ok(self.world.unit(self.unit)?.operator.active_skill)
    }
    pub(in crate::battle::runtime) fn stat(&mut self, unit: Id, key: &str) -> Result<f64, String> {
        Host::tac_stat(self.world, unit, key)
    }
    pub(in crate::battle::runtime) fn flag(&mut self, unit: Id, key: &str) -> Result<bool, String> {
        Ok(self
            .world
            .unit_mut(unit)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains(key))
    }
    pub(in crate::battle::runtime) fn has(&self, unit: Id, key: &str) -> bool {
        Host::tac_has(self.world, unit, key)
    }
    pub(in crate::battle::runtime) fn keys(&self) -> Result<Vec<usize>, String> {
        self.host
            .roster
            .as_ref()
            .unwrap()
            .range(self.world, self.unit)
    }
    pub(in crate::battle::runtime) fn range(
        &self,
        grid: &[[i32; 2]],
    ) -> Result<Vec<usize>, String> {
        self.host.tac_keys(self.world, self.unit, grid)
    }
    pub(in crate::battle::runtime) fn enemies(
        &mut self,
        keys: Option<&[usize]>,
        fly: bool,
    ) -> Result<Vec<Id>, String> {
        self.host.tac_enemies(self.world, self.unit, keys, fly)
    }
    pub(in crate::battle::runtime) fn radius(
        &mut self,
        at: [f64; 2],
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        self.host.tac_radius(self.world, at, radius)
    }
    pub(in crate::battle::runtime) fn allies(&mut self, keys: &[usize]) -> Result<Vec<Id>, String> {
        let mut out = Vec::new();
        for id in self.host.roster.as_ref().unwrap().ids().to_vec() {
            if crate::battle::targeting::friendly_selectable(self.world, self.unit, id)?
                && body::in_range(self.world.unit(id)?, keys)
            {
                out.push(id);
            }
        }
        Ok(out)
    }
    pub(in crate::battle::runtime) fn add(
        &mut self,
        unit: Id,
        mut buff: Buff,
    ) -> Result<Option<u64>, String> {
        buff.runtime.source = Some(self.unit);
        self.buffs.add(
            self.world,
            self.host,
            unit,
            buffs::Spec {
                buff,
                ..Default::default()
            },
        )
    }
    pub(in crate::battle::runtime) fn buff(
        &mut self,
        unit: Id,
        key: &str,
        mods: Numbers,
        duration: f64,
    ) -> Result<(), String> {
        self.host
            .component_buff(self.world, self.buffs, unit, key, mods, duration, self.unit)
    }
    pub(in crate::battle::runtime) fn remove(&mut self, unit: Id, key: &str) -> Result<(), String> {
        self.buffs
            .remove(self.world, self.host, unit, key)
            .map(|_| ())
    }
    pub(in crate::battle::runtime) fn status(
        &mut self,
        target: Id,
        key: &str,
        duration: f64,
        value: Option<f64>,
    ) -> Result<(), String> {
        self.host.advanced_status(
            self.world, self.buffs, self.unit, target, key, duration, value,
        )
    }
    pub(in crate::battle::runtime) fn heal(
        &mut self,
        target: Id,
        amount: f64,
        options: health::HealOptions,
    ) -> Result<f64, String> {
        self.host.support_heal(
            self.world,
            self.buffs,
            Some(self.unit),
            target,
            amount,
            options,
        )
    }
    pub(in crate::battle::runtime) fn deal(
        &mut self,
        request: &mut hit::Request,
    ) -> Result<f64, String> {
        self.host.resolve_hit(self.world, self.buffs, request)
    }
    pub(in crate::battle::runtime) fn damage(
        &mut self,
        target: Id,
        amount: f64,
        kind: DamageType,
        skill: bool,
        tag: &str,
    ) -> Result<f64, String> {
        let mut request = hit::Request::new(
            health::Hit {
                source: Some(self.unit),
                credit: Some(self.unit),
                target,
                damage_type: kind,
                no_sp: false,
                silent: false,
                hp_loss: false,
                on_applied: false,
            },
            Damage {
                amount,
                mul: 1.,
                resolved: false,
                penetration: Default::default(),
            },
        );
        request.is_skill = skill;
        request.tags.push(tag.into());
        if matches!(tag, "addition" | "funnel" | "vulpisHunt") {
            request.calculation = hit::Calculation::Precomputed;
            request.attack_type = hit::AttackType::Addition;
            request.tags.push("addition".into());
        }
        self.deal(&mut request)
    }
    pub(in crate::battle::runtime) fn elemental(
        &mut self,
        target: Id,
        amount: f64,
        element: element::Element,
    ) -> Result<f64, String> {
        let mut request =
            element::Request::new(Some(self.unit), Some(self.unit), target, element, amount);
        element::apply(self.world, self.host, self.buffs, &mut request)
    }
    pub(in crate::battle::runtime) fn element_damage(
        &mut self,
        target: Id,
        amount: f64,
        element: element::Element,
        tag: &str,
    ) -> Result<f64, String> {
        let mut request = hit::Request::new(
            health::Hit {
                source: Some(self.unit),
                credit: Some(self.unit),
                target,
                damage_type: DamageType::Elemental,
                no_sp: false,
                silent: false,
                hp_loss: false,
                on_applied: false,
            },
            Damage {
                amount,
                mul: 1.,
                resolved: false,
                penetration: Default::default(),
            },
        );
        request.element = Some(element);
        request.dodge_mode = injury::DodgeMode::Ignore;
        request.tags.push(tag.into());
        self.deal(&mut request)
    }
    pub(in crate::battle::runtime) fn lose_hp(&mut self, amount: f64) -> Result<(), String> {
        self.host.support_lose_hp(
            self.world,
            self.buffs,
            self.unit,
            amount,
            Some(self.unit),
            false,
        )
    }
    pub(in crate::battle::runtime) fn dp(&mut self, amount: f64) -> Result<(), String> {
        self.host.roster.as_mut().unwrap().add_dp(amount)
    }
    pub(in crate::battle::runtime) fn skill<T>(
        &mut self,
        operation: impl FnOnce(&mut skills::Runtime, &mut Host, &mut World) -> Result<T, String>,
    ) -> Result<T, String> {
        let id = self.unit;
        self.host
            .with_kit_buffs(self.world, self.buffs, |host, world| {
                let mut skill = host.roster.as_mut().unwrap().take_skill(id)?;
                let result = operation(&mut skill, host, world);
                let restored = host.roster.as_mut().unwrap().restore_skill(id, skill);
                match result {
                    Ok(value) => {
                        restored?;
                        Ok(value)
                    }
                    Err(error) => {
                        restored?;
                        Err(error)
                    }
                }
            })
    }
    pub(in crate::battle::runtime) fn later(
        &mut self,
        delay: f64,
        callback: fn(&mut Context<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        self.host.tac_timer(
            self.world,
            self.unit,
            self.world.time + delay,
            0.,
            1,
            Task::Callback(callback),
        )
    }
    pub(in crate::battle::runtime) fn later_at(
        &mut self,
        delay: f64,
        position: [f64; 2],
        attack: f64,
        survives_retreat: bool,
        callback: fn(&mut Context<'_>, [f64; 2], f64) -> Result<(), String>,
    ) -> Result<(), String> {
        self.host.tac_timer(
            self.world,
            self.unit,
            self.world.time + delay,
            0.,
            1,
            Task::AreaCallback {
                position,
                attack,
                survives_retreat,
                callback,
            },
        )
    }
    pub(in crate::battle::runtime) fn later_key(
        &mut self,
        delay: f64,
        key: u64,
        callback: fn(&mut Context<'_>, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        self.host.tac_timer(
            self.world,
            self.unit,
            self.world.time + delay,
            0.,
            1,
            Task::KeyedCallback { key, callback },
        )
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn faction(
        &self,
        unit: Id,
        bond: &str,
        nations: &[&str],
    ) -> Result<bool, String> {
        if self.world.unit(unit)?.side != health::Side::Ally {
            return Ok(false);
        }
        let def = self.host.tac_def(unit)?;
        let raw = &self
            .host
            .data
            .table("chess")
            .ok_or("Missing operator data")?[&def.key];
        Ok(raw["bonds"]
            .as_array()
            .is_some_and(|a| a.iter().any(|b| b.as_str() == Some(bond)))
            || raw["nationId"]
                .as_str()
                .is_some_and(|n| nations.contains(&n)))
    }

    pub(in crate::battle::runtime) fn retreat(&mut self) -> Result<(), String> {
        self.retreat_with_reason("retreat")
    }
    pub(in crate::battle::runtime) fn retreat_with_reason(
        &mut self,
        reason: &str,
    ) -> Result<(), String> {
        if !self.up() {
            return Ok(());
        }
        let mut roster = self.host.roster.take().ok_or("Missing retreat roster")?;
        let result =
            if self.world.unit(self.unit)?.deployment.kind == crate::battle::deploy::Kind::Token {
                roster.expire_token(self.world, self.buffs, self.host, self.unit)
            } else {
                roster.retreat(self.world, self.buffs, self.host, self.unit)
            };
        self.host.roster = Some(roster);
        result?;
        if self.world.unit(self.unit)?.deployment.kind == crate::battle::deploy::Kind::Operator {
            self.world.unit_mut(self.unit)?.deployment.remove_reason = Some(reason.into());
        }
        self.host.kit_event(
            self.world,
            self.buffs,
            &mut Event::Death {
                unit: self.unit,
                credit: None,
            },
        )?;
        self.host.flush_kit_events(self.world, self.buffs)
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn pull(
        &mut self,
        target: Id,
        force: f64,
        to: Option<[f64; 2]>,
    ) -> Result<(), String> {
        let grid = self.host.grid.take().ok_or("Missing pull grid")?;
        let result = if let Some(to) = to {
            displacement::pull(
                self.world,
                &grid,
                &mut allies::Blocking {
                    host: self.host,
                    buffs: self.buffs,
                },
                target,
                force,
                displacement::Pull {
                    to,
                    center: None,
                    stop: 0.05,
                },
            )
        } else {
            let dir = self
                .host
                .roster
                .as_ref()
                .unwrap()
                .entry(self.unit)?
                .direction;
            displacement::pull_to_front(
                self.world,
                &grid,
                &mut allies::Blocking {
                    host: self.host,
                    buffs: self.buffs,
                },
                target,
                self.unit,
                dir,
                force,
            )
        };
        self.host.grid = Some(grid);
        result?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn release(&mut self) -> Result<(), String> {
        blocking::release(
            self.world,
            &mut allies::Blocking {
                host: self.host,
                buffs: self.buffs,
            },
            self.unit,
        )
    }
    pub(in crate::battle::runtime) fn end_skill(&mut self, reason: &str) -> Result<bool, String> {
        if self
            .host
            .roster
            .as_ref()
            .unwrap()
            .entry(self.unit)?
            .skill()
            .is_err()
        {
            return Ok(false);
        }
        self.skill(|skill, host, world| skill.end(world, host, reason))?;
        Ok(true)
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn summon(
        &mut self,
        key: &str,
        tile: [i32; 2],
        duration: f64,
    ) -> Result<Option<Id>, String> {
        let prepared = self
            .host
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .find(|&id| {
                self.world.unit(id).is_ok_and(|u| {
                    u.deployment.kind == crate::battle::deploy::Kind::Token
                        && u.deployment.uid.is_some()
                        && !u.removed
                        && u.summoner == Some(self.unit)
                }) && self.host.tac_def(id).is_ok_and(|def| def.key == key)
            });
        if let Some(token) = prepared {
            if !Host::tac_up(self.world, token) && !self.redeploy_free(token)? {
                return Ok(None);
            }
            return Ok(Some(token));
        }
        let definition =
            Definition::token(&self.host.data, key, &self.host.tac_def(self.unit)?.key)?;
        let direction = self
            .host
            .roster
            .as_ref()
            .unwrap()
            .entry(self.unit)?
            .direction;
        let grid = self.host.grid.take().ok_or("Missing summon grid")?;
        let mut roster = self.host.roster.take().ok_or("Missing summon roster")?;
        let result = roster.spawn_token(
            self.world, self.buffs, self.host, &grid, definition, self.unit, tile, direction,
        );
        self.host.roster = Some(roster);
        self.host.grid = Some(grid);
        let token = result?;
        self.host.rebuild_index(self.world)?;
        self.host.flush_kit_events(self.world, self.buffs)?;
        if let Some(token) = token.filter(|_| duration > 0.) {
            let owner = self.unit;
            self.unit = token;
            let result = self.later(duration, |k| k.retreat());
            self.unit = owner;
            result?;
        }
        Ok(token)
    }
    pub(in crate::battle::runtime) fn free_adjacent(&self) -> Result<Option<[i32; 2]>, String> {
        let tile = self.world.unit(self.unit)?.deployment.tile;
        let dir = self
            .host
            .roster
            .as_ref()
            .unwrap()
            .entry(self.unit)?
            .direction;
        let grid = self.host.grid.as_ref().ok_or("Missing summon grid")?;
        for offset in [
            [0, 1],
            [1, 1],
            [-1, 1],
            [1, 0],
            [-1, 0],
            [0, -1],
            [1, -1],
            [-1, -1],
        ] {
            let offset = dir.rotate(offset);
            let position = [tile[0] + offset[0], tile[1] + offset[1]];
            if grid.in_rect(position)
                && grid.can_stand(position, false)
                && !self
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .deployment()
                    .reserved(self.world, position)?
            {
                return Ok(Some(position));
            }
        }
        Ok(None)
    }
    pub(in crate::battle::runtime) fn relocate(
        &mut self,
        unit: Id,
        tile: [i32; 2],
    ) -> Result<bool, String> {
        let grid = self.host.grid.take().ok_or("Missing relocation grid")?;
        let mut roster = self.host.roster.take().ok_or("Missing relocation roster")?;
        let result = roster.relocate(self.world, self.buffs, self.host, &grid, unit, tile);
        self.host.roster = Some(roster);
        self.host.grid = Some(grid);
        result
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn summon_tiles(
        &self,
        grid: &[[i32; 2]],
        ground: bool,
    ) -> Result<Vec<[i32; 2]>, String> {
        let map = self.host.grid.as_ref().ok_or("Missing summon grid")?;
        let own = self.world.unit(self.unit)?.deployment.tile;
        let mut tiles = Vec::new();
        for key in self.range(grid)? {
            let tile = [(key / COLS) as i32, (key % COLS) as i32];
            if tile != own
                && map.in_rect(tile)
                && map.can_stand(tile, false)
                && (!ground || map.ground_passable(tile, true))
                && !self
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .deployment()
                    .reserved(self.world, tile)?
            {
                tiles.push(tile);
            }
        }
        Ok(tiles)
    }
    pub(in crate::battle::runtime) fn best_summon_tile(
        &self,
        tiles: &[[i32; 2]],
    ) -> Option<[i32; 2]> {
        let mut best = tiles.first().copied();
        let mut distance = f64::INFINITY;
        for &tile in tiles {
            let mut nearest = f64::INFINITY;
            for id in self.world.unit_ids() {
                let u = self.world.unit(id).ok()?;
                if u.side == health::Side::Enemy && u.stats.alive && !u.hidden {
                    nearest = nearest.min(crate::deterministic::math::hypot(
                        u.position[0] - tile[0] as f64,
                        u.position[1] - tile[1] as f64,
                    ));
                }
            }
            if nearest < distance - 1e-9 {
                distance = nearest;
                best = Some(tile);
            }
        }
        best
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn move_redeploy(
        &mut self,
        tile: [i32; 2],
        skill: &mut skills::Runtime,
        clear_sp: bool,
    ) -> Result<bool, String> {
        if !self.relocate(self.unit, tile)? {
            return Ok(false);
        }
        let sequence = self.world.next_deployment_sequence()?;
        let now = self.world.time;
        let unit = self.world.unit_mut(self.unit)?;
        unit.deployment.deploy_seq = sequence;
        unit.deployment.aggro_seq = sequence;
        unit.deployment.deployed_at = now;
        if clear_sp
            && !skill.bank.no_skill
            && skill.bank.kind != crate::battle::sp::Kind::Passive
            && !(skill.bank.active && skill.bank.kind.timed())
        {
            skill.bank.sp = 0.;
            skill.bank.charges = if skill.bank.cost(self.world)? <= 0. {
                skill.bank.max_charges() as f64
            } else {
                0.
            };
        }
        let definition = self.host.tac_def(self.unit)?;
        let direction = self
            .host
            .roster
            .as_ref()
            .unwrap()
            .entry(self.unit)?
            .direction;
        definition.skill_event(
            self.world,
            self.buffs,
            self.host,
            skills::Event::Deploy,
            direction,
            &mut skills::Context {
                reason: "move".into(),
                dt: 0.,
                amount: 0.,
                no_ammo: false,
                targets: Vec::new(),
            },
            skill,
        )?;
        Ok(true)
    }
}

impl Context<'_> {
    pub(in crate::battle::runtime) fn tactical_point(
        &mut self,
    ) -> Result<Option<[i32; 2]>, String> {
        // A preplaced tactical summon already owns its tile, including on a full board.
        for &id in self.host.roster.as_ref().unwrap().ids() {
            let unit = self.world.unit(id)?;
            if unit.deployment.kind == crate::battle::deploy::Kind::Token
                && unit.deployment.uid.is_some()
                && unit.summoner == Some(self.unit)
            {
                return Ok(Some(unit.deployment.home));
            }
        }
        let grid = self
            .host
            .grid
            .as_mut()
            .ok_or("Missing tactical point grid")?;
        let paths = self
            .host
            .spawns
            .as_mut()
            .ok_or("Missing tactical point routes")?
            .path_tiles(grid);
        let roster = self.host.roster.as_ref().unwrap();
        let origin = self.world.unit(self.unit)?.deployment.tile;
        let direction = roster.entry(self.unit)?.direction;
        let mut best = None;
        let mut score = (2u8, f64::INFINITY, [i32::MAX; 2]);
        for key in roster.base_range(self.world, self.unit)? {
            let tile = [(key / COLS) as i32, (key % COLS) as i32];
            let grid = self.host.grid.as_ref().unwrap();
            if !grid.ground_passable(tile, false)
                || !grid.can_stand(tile, false)
                || roster.deployment().reserved(self.world, tile)?
            {
                continue;
            }
            let delta = [tile[0] - origin[0], tile[1] - origin[1]];
            let local = direction.local(delta);
            let distance = delta[0].abs().max(delta[1].abs()) as f64 + 0.01 * local[0].abs() as f64;
            if distance <= 0. {
                continue;
            }
            let path = if paths.contains(&key) { 0 } else { 1 };
            if path < score.0
                || path == score.0
                    && (distance < score.1 - 1e-9
                        || (distance - score.1).abs() <= 1e-9 && local < score.2)
            {
                score = (path, distance, local);
                best = Some(tile);
            }
        }
        Ok(best)
    }
    pub(in crate::battle::runtime) fn redeploy_free(&mut self, token: Id) -> Result<bool, String> {
        let tiles = Arc::clone(&self.host.tiles);
        let mut roster = self.host.roster.take().ok_or("Missing revival roster")?;
        let result = roster.redeploy_free(self.world, self.buffs, self.host, &tiles, token);
        self.host.roster = Some(roster);
        let revived = result?;
        self.host.rebuild_index(self.world)?;
        self.host.flush_kit_events(self.world, self.buffs)?;
        Ok(revived)
    }
}
