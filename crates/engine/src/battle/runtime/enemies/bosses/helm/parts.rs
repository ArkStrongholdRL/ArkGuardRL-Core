// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::content::enemies::bosses::Blade;
impl Host {
    pub(super) fn blade_dive(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Memory::Blade { home, .. } = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        let mut targets = vec![];
        for t in self.roster.as_ref().unwrap().ids().to_vec() {
            let u = w.unit(t)?;
            if u.stats.alive
                && u.deployed
                && u.deployment.kind == crate::battle::deploy::Kind::Operator
            {
                targets.push(t);
            }
        }
        targeting::sort_allies(w, id, &mut targets, Default::default())?;
        let mut keyed = vec![];
        for t in targets {
            keyed.push((t, Self::tac_stat(w, t, "atk")?));
        }
        keyed.sort_by(|a, b| a.1.total_cmp(&b.1));
        let Some(&(target, _)) = keyed.first() else {
            return Ok(());
        };
        let tile = w.unit(target)?.deployment.tile;
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Blade {
                phase: BladePhase::Dive { tile, hits: 0 },
                home,
            },
        );
        r.remove(w, self, id, "boss:hover")?;
        let mut b = buff("boss:sortie", &[]);
        b.runtime.persist = true;
        b.flags.insert("disarm".into());
        self.enemy_buff(w, r, id, b)?;
        let hand = match self.boss_config(id) {
            Some(Boss::Blade(Blade { hammer: true, .. })) => "right_hand_blink",
            _ => "left_hand_blink",
        };
        if let Some(route) = self.spawns.as_ref().unwrap().branch_route(hand) {
            if let Some(p) = route["start"].as_array().filter(|p| p.len() == 2) {
                w.unit_mut(id)?.position = [
                    p[0].as_f64().ok_or("Invalid blink row")?,
                    p[1].as_f64().ok_or("Invalid blink column")?,
                ];
            }
        }
        Ok(())
    }
    pub(super) fn blade_down(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Blade,
        home: [f64; 2],
    ) -> Result<(), String> {
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Blade {
                phase: BladePhase::Down,
                home,
            },
        );
        w.unit_mut(id)?.motion_fly = false;
        w.unit_mut(id)?.ground = true;
        self.apply_status(
            w,
            r,
            id,
            "stun",
            status::Options {
                duration: config.stun,
                force: true,
                ..Default::default()
            },
        )?;
        let mut b = buff("boss:downed", &[("dmgTakenMul", config.scale)]);
        b.runtime.duration = config.down.min(config.stun);
        b.runtime.visible = true;
        b.runtime.on_expire = Some(BLADE_RETURN);
        self.enemy_buff(w, r, id, b)
    }
    pub(super) fn blade_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Blade,
        dt: f64,
    ) -> Result<(), String> {
        let Memory::Blade {
            phase: BladePhase::Dive { tile, .. },
            ..
        } = self.content.as_ref().unwrap().boss(id)
        else {
            return Ok(());
        };
        if self.enemy_can_cast(w, id, false)? && self.helm_step(w, id, tile, dt)? {
            let leader = self.helm_leader(w, id)?.unwrap_or(id);
            self.helm_blast(
                w,
                r,
                leader,
                id,
                tile,
                config.stun,
                config.dot,
                config.duration,
                true,
            )?;
            self.blade_replace(w, r, id)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blade_return(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if w.unit(id)?.stats.alive
            && matches!(
                self.content.as_ref().unwrap().boss(id),
                Memory::Blade {
                    phase: BladePhase::Down,
                    ..
                }
            )
        {
            self.blade_replace(w, r, id)?;
        }
        Ok(())
    }
    fn blade_replace(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Memory::Blade { home, .. } = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Blade {
                phase: BladePhase::Gone,
                home,
            },
        );
        w.unit_mut(id)?.motion_fly = true;
        w.unit_mut(id)?.ground = false;
        let hand = match self.boss_config(id) {
            Some(Boss::Blade(Blade { hammer: true, .. })) => "right_hand_origin",
            _ => "left_hand_origin",
        };
        let route = self.spawns.as_ref().unwrap().branch_route(hand);
        let ledger = &self.spawns.as_ref().unwrap().ledger.alive()[&id];
        let mut options = serde_json::json!({"route":route.clone().unwrap_or_else(||serde_json::json!({"motion":"FLY","start":home,"end":home,"steps":[{"t":"WAIT","s":99999}]})),"tag":ledger.tag,"countInTotal":false,"mods":ledger.mods});
        if route.is_none() {
            options["pos"] = serde_json::json!(home);
        }
        let key = self.spawns.as_ref().unwrap().definition(id)?.key.clone();
        let hp = w.unit(id)?.stats.hp;
        let replacement = self.with_spawns(w, r, |spawns, context, world| {
            spawns.spawn_runtime(world, context, &key, &options)
        })?;
        w.unit_mut(replacement)?.stats.hp = hp.min(Self::tac_stat(w, replacement, "maxHp")?);
        self.kill_enemy(id, None, r, w)?;
        self.rebuild_index(w)
    }
    pub(super) fn helm_blast(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        by: Id,
        tile: [i32; 2],
        stun: f64,
        dot: f64,
        duration: f64,
        blade: bool,
    ) -> Result<(), String> {
        for t in self.enemy_tile_allies(w, tile, 1)? {
            if !targeting::area_selectable(w, Some(by), t)? {
                continue;
            }
            if stun > 0. {
                self.apply_status(
                    w,
                    r,
                    t,
                    "stun",
                    status::Options {
                        duration: stun,
                        source: Some(source),
                        ignore_select: true,
                        ..Default::default()
                    },
                )?;
            }
            if dot > 0. && duration > 0. {
                let mut b = buff(
                    if blade {
                        "boss:bladeDiveDot"
                    } else {
                        "boss:helmShellDot"
                    },
                    &[],
                );
                b.runtime.duration = duration;
                b.runtime.interval = 1.;
                b.runtime.visible = true;
                b.runtime.source = Some(source);
                b.runtime.tick_payload = dot;
                b.runtime.on_tick = Some(PART_DOT);
                r.add(
                    w,
                    self,
                    t,
                    buffs::Spec {
                        buff: b,
                        ..Default::default()
                    }
                    .retaining_unit(source),
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn helm_dot(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let source = b.runtime.source.ok_or("Missing part damage source")?;
        let amount = b.runtime.tick_payload;
        let mut q = ability_damage(Some(source), c.unit, amount, DamageType::Physical);
        q.is_skill = false;
        q.ignore_select = true;
        q.tags.push(
            if b.key.as_deref() == Some("boss:bladeDiveDot") {
                "bladeDive"
            } else {
                "helmShell"
            }
            .into(),
        );
        self.enemy_damage(w, r, &mut q)
    }
}
