// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::sync::{Arc, Mutex};
pub(in crate::battle::runtime) type SharedVolley = Arc<Volley>;
use crate::battle::{
    content::enemies::{specials::Memory, Special},
    targeting,
};
#[derive(Debug)]
pub(in crate::battle::runtime) struct Volley {
    pub target: Option<Id>,
    position: Mutex<[f64; 2]>,
    radius: f64,
    bound: f64,
    in_play: bool,
}
impl Host {
    pub(super) fn enemy_catapult_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        let Kit::Special(Special::Catapult {
            times,
            interval,
            radius,
            bound,
        }) = self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        if event == enemy::Event::BeforeAttack {
            if let Some(&target) = c.targets.first() {
                self.content.as_mut().unwrap().set_special(
                    id,
                    Memory::Catapult {
                        target: Some(target),
                        position: Some(w.unit(target)?.position),
                    },
                );
            }
        }
        if event != enemy::Event::Attack {
            return Ok(());
        }
        let Memory::Catapult { target, position } = self.content.as_ref().unwrap().special(id)
        else {
            unreachable!()
        };
        let in_play = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains("inPlay")
            || w.unit(id)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some("inPlay"));
        let volley = Arc::new(Volley {
            target,
            position: Mutex::new(position.unwrap_or(w.unit(id)?.position)),
            radius: if in_play { radius } else { 1.5 },
            bound,
            in_play,
        });
        self.enemy_catapult_stone(w, r, id, &volley)?;
        if interval > 0. && times > 1 {
            for stone in 1..times {
                self.enemy_schedule(
                    w,
                    Some(id),
                    interval * f64::from(stone),
                    0.,
                    1,
                    EnemyTask::Catapult(Arc::clone(&volley)),
                )?;
            }
        } else {
            for _ in 1..times {
                self.enemy_catapult_stone(w, r, id, &volley)?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_catapult_stone(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        volley: &Volley,
    ) -> Result<(), String> {
        let mut position = {
            let mut aim = volley.position.lock().unwrap();
            if let Some(t) = volley.target {
                if w.unit(t)?.stats.alive {
                    *aim = w.unit(t)?.position;
                }
            }
            *aim
        };
        if volley.in_play {
            // The source rolls x before y. World coordinates are row, column.
            position[1] += (self.rng.next_f64() * 2. - 1.) * volley.bound.max(0.);
            position[0] += (self.rng.next_f64() * 2. - 1.) * volley.bound.max(0.);
        }
        let mut primary = None;
        if !volley.in_play {
            if let Some(t) = volley.target {
                if w.unit(t)?.stats.alive
                    && targeting::can_target_ally(
                        w,
                        id,
                        t,
                        true,
                        targeting::Profile {
                            block_free: true,
                            ..Default::default()
                        },
                    )?
                {
                    primary = Some(t);
                    self.catapult_damage(w, r, id, t, false)?;
                }
            }
        }
        if volley.radius > 0. {
            for t in spatial::allies_in_radius(
                w,
                self.roster.as_ref().unwrap().ids(),
                position,
                volley.radius,
                None,
                false,
                w.unit_capacity(),
            )? {
                if Some(t) != primary && targeting::area_selectable(w, Some(id), t)? {
                    self.catapult_damage(w, r, id, t, true)?;
                }
            }
        }
        Ok(())
    }
    fn catapult_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        splash: bool,
    ) -> Result<(), String> {
        let amount = Self::tac_stat(w, id, "atk")?;
        let mut q = ability_damage(Some(id), target, amount, DamageType::Physical);
        q.is_attack = true;
        q.is_skill = false;
        q.is_splash = splash;
        q.dodge_mode = crate::battle::injury::DodgeMode::Normal;
        q.tags = vec!["catapult:stone".into()];
        self.enemy_damage(w, r, &mut q)
    }
}
