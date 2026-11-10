//! Batch event-free movement ticks while keeping the original movement arithmetic.
use super::*;
use crate::battle::clock::Host as _;

struct Segment {
    id: Id,
    target: [f64; 2],
    position: [f64; 2],
    next: [f64; 2],
    stride: f64,
    terrain: super::super::terrain::State,
    radius_squared: f64,
}

impl Field {
    /// Diagnostic/reference switch. Switching execution strategy never resets a battle.
    pub fn set_movement_batching(&mut self, enabled: bool) -> Result<(), String> {
        self.healthy()?;
        self.core.movement_batching = enabled;
        Ok(())
    }

    pub fn movement_batch_ticks(&self) -> u64 {
        self.clock.movement_ticks()
    }
    pub fn event_window_stats(&self) -> [u64; 2] {
        [self.clock.movement_ticks(), self.core.event_windows]
    }
}

impl Core {
    fn movement_segments(&mut self, clock: &Clock) -> Result<Option<Vec<Segment>>, String> {
        let h = &self.host;
        if !self.movement_batching
            || self.boss.is_some()
            || h.boss_round.is_some()
            || !h.flights.as_ref().unwrap().list().is_empty()
            || h.flights.as_ref().unwrap().faulted()
            || h.buffs.as_ref().unwrap().faulted()
            || h.spawns.as_ref().unwrap().faulted()
            || h.content.as_ref().unwrap().faulted()
            || !h.devices.movement_quiet()
            || !h.movement_bonds_quiet()
            || !h.movement_growth_quiet()
            || !h.zones.is_empty()
            || !h.tactical_timers.is_empty()
            || h.tactical_slots != 0
            || !h.pending_skill_starts.is_empty()
            || !h.deferred_kit_events.is_empty()
            || !h.equipment.clock_quiet()
        {
            return Ok(None);
        }
        let spawns = h.spawns.as_ref().unwrap();
        let ids = spawns.live();
        if (ids.is_empty() && spawns.queue.pending().is_empty())
            || self.world.unit_ids().count() != ids.len() + h.roster.as_ref().unwrap().ids().len()
            || spawns.retained() != ids.len()
            || ids
                .iter()
                .any(|&id| !event_window::quiet_enemy(h.content.as_ref().unwrap().kit_ref(id)))
        {
            return Ok(None);
        }
        let grid = h.grid.as_ref().unwrap();
        let rect = grid.rect();
        let mut segments = Vec::with_capacity(ids.len());
        for &id in ids {
            let profile = spawns.definition(id)?.profile;
            if matches!(profile.mode, enemy::Mode::Heal) {
                return Ok(None);
            }
            let u = self.world.unit_mut(id)?;
            if !u.stats.alive
                || !u.stats.cached_clean()
                || !u.deployed
                || u.hidden
                || u.removed
                || u.pool.is_some()
                || u.hit_area.is_some()
                || u.blocked_by().is_some()
                || u.enemy.carrier.is_some()
                || u.enemy.swing
                || u.enemy.targets().is_some()
                || u.enemy.pause_until > clock.time()
                || u.motion.attack_stand_until > clock.time()
                || u.motion.unbalance_until > clock.time()
                || u.motion.fear.is_some()
            {
                return Ok(None);
            }
            let (flags_empty, stride) = {
                let stats = u.stats.stats().map_err(str::to_string)?;
                (
                    stats.flags.is_empty(),
                    stats.get(super::super::stats::Stat::MoveSpeed) * 0.5 * DT,
                )
            };
            if !flags_empty || !h.terrain.movement_ground_clear(u) {
                return Ok(None);
            }
            let Some(route) = u.motion.route.as_ref() else {
                return Ok(None);
            };
            let Some(target) = route
                .points
                .as_ref()
                .and_then(|points| points.get(route.point))
                .copied()
            else {
                return Ok(None);
            };
            if route.version != grid.version()
                || route.wait_left.is_some()
                || route.patrol
                || !matches!(route.legs().get(route.leg), Some(motion::Leg::Move { .. }))
                || !stride.is_finite()
                || stride <= 1e-9
                || !u
                    .position
                    .iter()
                    .chain(target.iter())
                    .all(|n| n.is_finite())
                || u.position[0] < rect.r0 as f64
                || u.position[0] > rect.r1 as f64
                || u.position[1] < rect.c0 as f64
                || u.position[1] > rect.c1 as f64
                || target[0] < rect.r0 as f64
                || target[0] > rect.r1 as f64
                || target[1] < rect.c0 as f64
                || target[1] > rect.c1 as f64
            {
                return Ok(None);
            }
            segments.push(Segment {
                id,
                target,
                position: u.position,
                next: u.position,
                stride,
                terrain: u.terrain,
                radius_squared: if profile.melee {
                    1e-9
                } else {
                    let radius = u.stats.base_number("rangeRadius") + 0.25;
                    radius * radius + 1e-9
                },
            });
        }
        Ok(Some(segments))
    }

    pub(super) fn run_movement_batch(
        &mut self,
        clock: &mut Clock,
        budget: usize,
    ) -> Result<usize, String> {
        let Some(mut segments) = self.movement_segments(clock)? else {
            return Ok(0);
        };
        self.movement_paths.refresh(&self.host, &self.world)?;
        if segments
            .iter()
            .any(|s| !self.movement_paths.clear_at(s.position, s.radius_squared))
        {
            return Ok(0);
        }
        let Some(mut window) = event_window::Window::prepare(&mut self.host, &mut self.world)?
        else {
            return Ok(0);
        };
        if segments.iter().any(|s| !window.clear_at(s.position)) {
            return Ok(0);
        }
        let spawn_at = self
            .host
            .spawns
            .as_ref()
            .unwrap()
            .queue
            .pending()
            .front()
            .map(|p| p.time);
        let mut ticks = 0;
        // Plan in local coordinates. Effect changes and route transitions end the window.
        // Keep the original floating-point recurrence, then commit the final state once.
        while ticks < budget.min(64) {
            let tick = clock
                .ticks()
                .checked_add(ticks as u64)
                .ok_or("Battle tick count exhausted")?;
            let time = tick as f64 * DT;
            let next_time = tick.checked_add(1).ok_or("Battle tick count exhausted")? as f64 * DT;
            if next_time >= self.time_limit - 1e-9
                || next_time >= 3600.
                || spawn_at.is_some_and(|at| at <= time + 1e-9)
            {
                break;
            }
            let mut clear = true;
            for segment in &mut segments {
                let (next, _, reached, _) =
                    motion::step_position(segment.position, segment.target, segment.stride);
                if reached
                    || !self.movement_paths.clear_at(next, segment.radius_squared)
                    || !window.clear_at(next)
                    || !self
                        .host
                        .terrain
                        .movement_effect_stable(&segment.terrain, next, time)
                {
                    clear = false;
                    break;
                }
                segment.next = next;
            }
            if !clear || !window.project(&mut self.world, time)? {
                break;
            }
            window.accept();
            for segment in &mut segments {
                segment.position = segment.next;
                if self.host.terrain.active() {
                    segment.terrain.movement_arrive(segment.position);
                }
            }
            ticks += 1;
        }
        if ticks > 0 {
            self.event_windows = self.event_windows.saturating_add(1);
            window.commit(&mut self.host, &mut self.world)?;
            self.commitments.clear();
            for segment in segments {
                let unit = self.world.unit_mut(segment.id)?;
                unit.position = segment.position;
                unit.terrain = segment.terrain;
                unit.moving = true;
                for _ in 0..ticks {
                    unit.cadence.tick(DT, true);
                }
            }
            // DP and cooldown addition/subtraction retain their original rounding and caps.
            for _ in 0..ticks {
                self.phase(Phase::Dp, clock)?;
                clock.movement_tick()?;
            }
            self.world.time = clock.time();
            self.host.rebuild_index(&self.world)?;
        }
        Ok(ticks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::OnceLock;

    fn data() -> Arc<Catalog> {
        static DATA: OnceLock<Arc<Catalog>> = OnceLock::new();
        DATA.get_or_init(|| {
            let root = std::env::temp_dir().join(format!("arkguard-movement-{}-{}", std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            std::fs::create_dir(&root).unwrap();
            for name in ["config", "chess", "bonds", "garrisons", "items", "bands", "effects", "choices",
                "enemies", "factions", "waves", "stages", "bosses", "tokens", "backups"] {
                let table = if name == "enemies" {
                    json!({"enemy_1007_slime": {"stats": {"maxHp": 100, "atk": 10, "moveSpeed": 1.3}},
                        "enemy_1005_yokai": {"stats": {"motion": "FLY", "maxHp": 100, "atk": 10, "moveSpeed": 1.7, "rangeRadius": 3.}},
                        "enemy_10042_prtrop": {"stats": {"maxHp": 100, "atk": 10, "moveSpeed": 1.}},
                        "enemy_9006_actoxi": {"stats": {"maxHp": 1, "atk": 10, "moveSpeed": 0.},
                            "talents": {"bb": {"1.projectile_life_time": 2., "1.damage_atk_scale": 0.1}}},
                        "enemy_1267_nhpbr": {"stats": {"maxHp": 100, "atk": 10, "moveSpeed": 1.},
                            "talents": {"bb": {"PollutedDie.projectile_range": 2., "PollutedDie.projectile_life_time": 0.6,
                                "PollutedDie.polluted_damage_low": 3., "PollutedDie.polluted_damage_high": 2.}}}})
                } else if name == "chess" {
                    json!({"chess_char_1_05_a": {"position": "MELEE", "profession": "PIONEER",
                        "subProfessionId": "charger", "rangeGrid": [[0, 0], [0, 1]],
                        "stats": {"maxHp": 10000, "atk": 25, "cost": 0, "blockCnt": 1},
                        "skill": {"skillId": "skchr_vigna_2", "spType": "INCREASE_WITH_TIME",
                            "spCost": 10, "duration": 3, "trigger": {"rule": "DEFAULT"}}},
                        "chess_char_1_17_a": {"position": "RANGED", "profession": "CASTER",
                            "subProfessionId": "mystic", "rangeGrid": [[0, 0], [0, 1], [0, 2]],
                            "stats": {"maxHp": 10000, "atk": 25, "cost": 0, "blockCnt": 0},
                            "skill": {"skillId": "skchr_indigo_2", "spType": "INCREASE_WITH_TIME",
                                "spCost": 10, "duration": 3, "trigger": {"rule": "DEFAULT"}}}})
                } else if name == "effects" {
                    json!({"test_flawless": {"buffs": [{"key": "env_gbuff_new_with_verify",
                        "bbStr": {"key": "act1autochess_debuff_9"}, "bb": {"atk": 0.2, "def": 0.1}}]}})
                } else if name == "bonds" { json!({"yanShip": {}}) } else { json!({}) };
                std::fs::write(root.join(format!("{name}.json")), table.to_string()).unwrap();
            }
            let catalog = Arc::new(Catalog::load(&root, 1 << 20).unwrap());
            std::fs::remove_dir_all(root).unwrap();
            catalog
        }).clone()
    }

    fn field(input: &Value, time_limit: f64, enabled: bool) -> Field {
        let stage = json!({"rows": vec!["rrrrrrrrrrrrrrrrrrrrr"; 19], "legend": {}});
        field_on(&stage, input, time_limit, enabled)
    }

    fn field_on(stage: &Value, input: &Value, time_limit: f64, enabled: bool) -> Field {
        let template = Template::parse(input, &Value::Null, &Value::Null, 2048, 512).unwrap();
        let mut field =
            Field::new(data(), stage, template, 42, time_limit, Limits::default()).unwrap();
        field.set_movement_batching(enabled).unwrap();
        field
    }

    fn input() -> Value {
        json!({"routes": [{"motion": "WALK", "start": [10, 2], "end": [10, 18]}],
            "spawns": [{"enemyKey": "enemy_1007_slime", "time": 0., "count": 2, "interval": 0.4},
                {"enemyKey": "enemy_1007_slime", "time": 2. + 1e-10, "count": 1}]})
    }

    fn snapshot(field: &mut Field) -> Value {
        let states = field.enemy_states().unwrap();
        let mut units = Vec::new();
        for state in states {
            let unit = field.unit(state.id).unwrap();
            units.push(json!({"id": state.id, "position": state.position.map(f64::to_bits),
                "hp": state.hp.to_bits(), "cooldown": state.cooldown.to_bits(), "flags": state.flags,
                "moving": state.moving, "hidden": state.hidden, "leg": state.route_leg,
                "motion": format!("{:?}", unit.motion), "enemy": format!("{:?}", unit.enemy),
                "cadence": format!("{:?}", unit.cadence), "attacks": unit.attacks,
                "buffs": format!("{:?}", unit.stats.buffs()), "rateDirty": unit.stats.rate_dirty,
                "terrain": format!("{:?}", unit.terrain),
                "regen": unit.regen_acc.to_bits(), "deployed": unit.deployed, "removed": unit.removed,
                "pins": field.core.world.unit_pins(state.id).unwrap()}));
        }
        let index = field.core.host.index.as_ref().unwrap();
        let occupancy: Vec<_> = (0..super::super::super::grid::N)
            .map(|key| {
                (
                    index.any_enemy(&field.core.world, &[key], false).ok(),
                    index.any_enemy(&field.core.world, &[key], true).ok(),
                )
            })
            .collect();
        json!({"ticks": field.ticks(), "time": field.time().to_bits(), "worldTime": field.core.world.time.to_bits(),
            "rng": field.rng_state(), "report": field.report().unwrap(), "units": units, "index": occupancy,
            "dp": field.core.host.roster.as_ref().unwrap().dp(1).unwrap().to_bits(),
            "pending": field.pending_spawns(), "retained": field.retained_units()})
    }

    #[test]
    fn targeting_can_query_routes_while_movement_owns_grid() {
        let mut f = field(&input(), 40., false);
        f.run(1).unwrap();
        let id = f.enemies()[0];
        f.core
            .host
            .grid
            .as_mut()
            .unwrap()
            .set_obstacle([10, 4], true, super::super::super::grid::CRATE)
            .unwrap();
        let mut grid = f.core.host.take_grid();
        let cached = crate::battle::targeting::Queries::remaining_distance(
            &mut f.core.host,
            id,
            &mut f.core.world,
        )
        .unwrap();
        let authoritative = motion::remaining_distance(&mut f.core.world, &mut grid, id).unwrap();
        assert_eq!(cached.to_bits(), authoritative.to_bits());
        f.core.host.grid = Some(grid);
    }

    #[test]
    fn projectile_kill_can_launch_poison_death_projectile() {
        use crate::battle::{
            attacks, damage::DamageType, direction::Direction, operators::Placement, projectiles,
        };
        for tactical in [false, true] {
            let mut f = field(&input(), 40., false);
            let source = f
                .add_operator(
                    "chess_char_1_05_a",
                    Placement {
                        uid: 1,
                        home: [10, 9],
                        direction: Direction::Right,
                    },
                )
                .unwrap();
            f.run(1).unwrap();
            let target = f
                .spawn("enemy_9006_actoxi", &json!({"pos":[10, 11]}))
                .unwrap();
            let position = f.unit(target).unwrap().position;
            let deployment = f.unit(source).unwrap().deployment.deploy_seq;
            let attack_id = f.core.world.next_attack_id().unwrap();
            f.core
                .host
                .flights
                .as_mut()
                .unwrap()
                .add(
                    &mut f.core.world,
                    projectiles::Launch {
                        from: position,
                        source: Some(source),
                        target: Some(target),
                        to: None,
                        speed: 10.,
                        hit_dead: false,
                        max_age: 10.,
                        on_hit: true,
                        visual: "arrow".into(),
                        speed_changeable: None,
                        removable: None,
                        payload: if tactical {
                            allies::Shot::Tactical(super::super::tactics::Arrow {
                                source,
                                kind: super::super::tactics::ArrowKind::Snow,
                                scale: 1.,
                                hits: 1,
                                seen: [target, 0, 0, 0, 0],
                                count: 1,
                            })
                        } else {
                            allies::Shot::Ally {
                                source,
                                deployment,
                                attack_id,
                                payload: attacks::Direct {
                                    attachment: None,
                                    element_heal_ratio: 0.,
                                    mode: attacks::Mode::Damage(DamageType::Physical),
                                    hits: 1,
                                    atk_scale: 1.,
                                    skill_damage_mul: 1.,
                                    target_damage_mul: 1.,
                                    split_mul: None,
                                    heal_scale: 1.,
                                    is_skill: false,
                                },
                                area: attacks::Area::default(),
                            }
                        },
                    },
                )
                .unwrap();
            f.run(1).unwrap();
            assert!(!f.enemies().contains(&target));
            let flights = f.core.host.flights.as_ref().unwrap().list();
            assert!(flights
                .iter()
                .any(|p| matches!(p.payload, allies::Shot::Cloud(_))));
            assert!(f.core.world.unit_pins(source).unwrap() > 0);
        }
    }

    #[test]
    fn replay_frames_do_not_materialize_dirty_stats() {
        let mut observed = field(&input(), 40., true);
        let mut reference = field(&input(), 40., true);
        for _ in 0..60 {
            observed.run(1).unwrap();
            reference.run(1).unwrap();
            for field in [&mut observed, &mut reference] {
                let ids = field.enemies().to_vec();
                for id in ids {
                    field
                        .core
                        .world
                        .unit_mut(id)
                        .unwrap()
                        .stats
                        .edit_base(|base| {
                            *base.0.entry("maxHp".into()).or_default() += 1.;
                        });
                }
            }
            let frame = observed.replay_frame().unwrap();
            assert_eq!(frame["tick"], json!(observed.ticks()));
            assert_eq!(observed.replay_frame().unwrap(), frame);
        }
        assert_eq!(snapshot(&mut observed), snapshot(&mut reference));
        assert_eq!(observed.rng_state(), reference.rng_state());
    }
}
