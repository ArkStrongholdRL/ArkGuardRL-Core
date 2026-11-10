// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::bosses::{
        gun::{self, SPRING_SHIELD},
        Spring,
    },
    injury,
};
impl Host {
    fn spring_drop(&mut self, w: &mut World, r: &mut buffs::Runtime, id: Id) -> Result<(), String> {
        let Memory::Spring(mut s) = self.content.as_ref().unwrap().boss(id) else {
            return Ok(());
        };
        s.up = false;
        s.down_at = Some(w.time);
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Spring(s));
        r.remove(w, self, id, "boss:springShield")?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_injury(
        event: &mut injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        let id = c.request.hit.target;
        let Some(Boss::Spring(config)) = c.host.boss_config(id) else {
            return Ok(());
        };
        if !matches!(c.host.content.as_ref().unwrap().boss(id),Memory::Spring(s) if s.up) {
            return Ok(());
        }
        let kind = c.request.hit.damage_type;
        if (config.kind == SpringKind::Arts
            && kind == DamageType::Physical
            && c.request.attack_type == hit::AttackType::Normal)
            || (config.kind == SpringKind::Element
                && matches!(kind, DamageType::Physical | DamageType::Arts))
        {
            event.amount *= config.scale;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_observe(
        token: u64,
        event: injury::Observation,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        let id = c.request.hit.target;
        if token != SPRING_SHIELD {
            return c.host.spring_transfer(
                c.world,
                c.runtime,
                id,
                c.request.hit.source,
                c.request.hit.credit,
                event.amount,
                event.cancelled,
            );
        }
        let Some(Boss::Spring(config)) = c.host.boss_config(id) else {
            return Ok(());
        };
        if !matches!(c.host.content.as_ref().unwrap().boss(id),Memory::Spring(s) if s.up) {
            return Ok(());
        }
        let b = c.runtime.get(c.world, id, c.buff)?;
        if (config.kind == SpringKind::Arts && b.shield <= 1e-6)
            || (config.kind == SpringKind::Hits && b.runtime.shield_hits == 0)
        {
            return c.host.spring_drop(c.world, c.runtime, id);
        }
        if config.kind != SpringKind::Arts
            || c.request.hit.damage_type != DamageType::Physical
            || c.request.attack_type != hit::AttackType::Normal
        {
            return Ok(());
        }
        let Some(source) = c
            .request
            .hit
            .source
            .filter(|&s| c.world.unit(s).is_ok_and(|u| u.stats.alive))
        else {
            return Ok(());
        };
        let mut q = ability_damage(
            None,
            source,
            Self::tac_stat(c.world, id, "atk")? * config.counter,
            DamageType::Physical,
        );
        q.hit.credit = Some(id);
        q.hit.on_applied = true;
        q.attack_type = hit::AttackType::Addition;
        q.ignore_select = true;
        q.tags = vec!["springCounter".into()];
        c.resolve(&mut q)?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_transfer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        source: Option<Id>,
        credit: Option<Id>,
        amount: f64,
        cancelled: bool,
    ) -> Result<(), String> {
        if !(amount > 0.) || cancelled {
            return Ok(());
        }
        let credit = source
            .or(credit)
            .filter(|&t| w.unit(t).is_ok_and(|u| u.side == health::Side::Ally));
        let mut guns = self
            .gun_companions(w, id, false)
            .into_iter()
            .filter(|&t| matches!(self.boss_config(t), Some(Boss::Gun(_))))
            .collect::<Vec<_>>();
        targeting::sort_allies(
            w,
            id,
            &mut guns,
            targeting::Profile {
                block_free: true,
                ..Default::default()
            },
        )?;
        let mut targets = vec![];
        if let Some(&g) = guns.first() {
            targets.push((g, amount));
        }
        let others = self.gun_companions(w, id, true);
        for &t in &others {
            targets.push((t, amount / others.len() as f64));
        }
        for (target, amount) in targets {
            self.lose_hp_with_credit(
                w,
                r,
                health::Hit {
                    source: None,
                    credit,
                    target,
                    damage_type: DamageType::True,
                    no_sp: true,
                    silent: false,
                    hp_loss: true,
                    on_applied: false,
                },
                amount,
                false,
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_modifier(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        source: Option<Id>,
        credit: Option<Id>,
        amount: f64,
    ) -> Result<(), String> {
        if matches!(
            self.boss_config(id),
            Some(Boss::Spring(Spring {
                kind: SpringKind::Arts,
                ..
            }))
        ) {
            self.spring_transfer(w, r, id, source, credit, amount, false)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_element_burst(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if matches!(
            self.boss_config(id),
            Some(Boss::Spring(Spring {
                kind: SpringKind::Element,
                ..
            }))
        ) && matches!(self.content.as_ref().unwrap().boss(id),Memory::Spring(s) if s.up)
        {
            self.spring_drop(w, r, id)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn spring_counter_applied(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
    ) -> Result<(), String> {
        if q.tags.iter().any(|s| s == "springCounter") {
            if let Some(id) = q.hit.credit {
                if let Some(Boss::Spring(config)) = self.boss_config(id) {
                    let mut effect = crate::battle::element::Request::new(
                        Some(id),
                        Some(id),
                        q.hit.target,
                        Element::Erosion,
                        Self::tac_stat(w, id, "atk")? * config.erosion,
                    );
                    effect.damage.ignore_select = true;
                    effect.damage.tags = vec!["springCounter".into()];
                    crate::battle::element::apply(w, self, r, &mut effect)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies::bosses) fn spring_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Spring,
        dt: f64,
    ) -> Result<(), String> {
        let Memory::Spring(mut s) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        if !s.up && s.down_at.is_some_and(|at| w.time - at >= config.regen) {
            s.up = true;
            let b = gun::spring_shield(w, id, config)?;
            self.enemy_buff(w, r, id, b)?;
        }
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Spring(s.clone()));
        if w.time >= w.unit(id)?.motion.unbalance_until - 1e-9 {
            if let Some(mut dash) = s.dash.take() {
                let gun = if w.unit(dash.gun).is_ok_and(|u| u.stats.alive) {
                    Some(dash.gun)
                } else {
                    let p = w.unit(id)?.position;
                    let mut guns = self
                        .gun_companions(w, id, false)
                        .into_iter()
                        .filter(|&t| matches!(self.boss_config(t), Some(Boss::Gun(_))))
                        .collect::<Vec<_>>();
                    guns.sort_by(|&a, &b| {
                        let a = w.unit(a).unwrap().position;
                        let b = w.unit(b).unwrap().position;
                        hypot(a[1] - p[1], a[0] - p[0]).total_cmp(&hypot(b[1] - p[1], b[0] - p[0]))
                    });
                    guns.first().copied()
                };
                if let Some(g) = gun {
                    let speed = Self::tac_stat(w, id, "moveSpeed")? * dash.mul * 0.5 * dt;
                    Self::gun_step(w, id, w.unit(g)?.position, speed)?;
                }
                self.gun_trample(w, r, id, &mut dash.hit, 1.)?;
                if w.time >= dash.until {
                    r.remove(w, self, id, "boss:dash")?;
                    if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                        route.points = None;
                    }
                } else {
                    s.dash = Some(dash)
                }
            }
        }
        let Memory::Spring(mut current) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        current.dash = s.dash;
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Spring(current));
        self.rebuild_index(w)
    }
    pub(super) fn spring_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &mut enemy::Context,
        event: enemy::Event,
        config: Spring,
    ) -> Result<(), String> {
        if event != enemy::Event::Attack {
            return Ok(());
        }
        let Some(every) = config.every else {
            return Ok(());
        };
        let Memory::Spring(mut s) = self.content.as_ref().unwrap().boss(c.enemy) else {
            unreachable!()
        };
        if !s.up {
            return Ok(());
        }
        s.attacks = (s.attacks + 1) % every;
        let fire = s.attacks == 0;
        self.content
            .as_mut()
            .unwrap()
            .set_boss(c.enemy, Memory::Spring(s));
        if !fire {
            return Ok(());
        }
        let Some(&first) = c
            .targets
            .iter()
            .find(|&&t| w.unit(t).is_ok_and(|u| u.stats.alive))
        else {
            return Ok(());
        };
        let atk = Self::tac_stat(w, c.enemy, "atk")?;
        if config.kind == SpringKind::Hits {
            for _ in 0..10 {
                if !w.unit(first)?.stats.alive {
                    break;
                }
                let mut q = ability_damage(
                    Some(c.enemy),
                    first,
                    atk * config.shot,
                    DamageType::Physical,
                );
                self.enemy_damage(w, r, &mut q)?;
            }
        } else {
            let mut target = Some(first);
            let mut hit = vec![];
            for k in 0..config.times.min(w.unit_capacity() as u32) {
                let Some(t) = target else { break };
                hit.push(t);
                self.enemy_element(
                    w,
                    r,
                    c.enemy,
                    t,
                    Element::Erosion,
                    atk * config.shot
                        * crate::deterministic::math::powi(0.85, k as i32)
                            .map_err(str::to_string)?,
                )?;
                let origin = w.unit(t)?.position;
                let mut next = self.boss_area(w, c.enemy)?;
                next.retain(|&a| {
                    !hit.contains(&a) && {
                        let p = w.unit(a).unwrap().position;
                        hypot(p[1] - origin[1], p[0] - origin[0]) <= 2.
                    }
                });
                targeting::sort_allies(w, c.enemy, &mut next, Default::default())?;
                next.sort_by(|&a, &b| {
                    let a = w.unit(a).unwrap().position;
                    let b = w.unit(b).unwrap().position;
                    hypot(a[1] - origin[1], a[0] - origin[0])
                        .total_cmp(&hypot(b[1] - origin[1], b[0] - origin[0]))
                });
                target = next.first().copied();
            }
        }
        Ok(())
    }
}
