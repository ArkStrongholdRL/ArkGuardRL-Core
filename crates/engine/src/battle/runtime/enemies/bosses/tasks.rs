// SPDX-License-Identifier: GPL-3.0-or-later
use super::ensemble::AreaStrike;
use super::*;
use crate::battle::element::Element;
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum BossTask {
    Pipe { echo: Id, radius: f64, element: f64 },
    Rockfall { target: Id, scale: f64 },
    Species { loss: f64 },
}
impl BossTask {
    pub(in crate::battle::runtime::enemies) fn target(self) -> Option<Id> {
        match self {
            Self::Pipe { echo, .. } => Some(echo),
            Self::Rockfall { target, .. } => Some(target),
            Self::Species { .. } => None,
        }
    }
}
impl Host {
    pub(in crate::battle::runtime::enemies) fn boss_task(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        task: BossTask,
    ) -> Result<bool, String> {
        if !w.unit(id)?.stats.alive {
            return Ok(false);
        }
        match task {
            BossTask::Pipe {
                echo,
                radius,
                element,
            } => {
                if !w.unit(echo)?.stats.alive {
                    return Ok(false);
                }
                self.boss_arts_area(
                    w,
                    r,
                    id,
                    AreaStrike {
                        position: w.unit(echo)?.position,
                        radius,
                        scale: 1.,
                        element: Element::Apoptosis,
                        ratio: element,
                        attack: None,
                    },
                )?;
                self.echo_hit(w, r, echo)?;
            }
            BossTask::Rockfall { target, scale } => {
                if w.unit(target)?.stats.alive {
                    let mut q = ability_damage(
                        Some(id),
                        target,
                        Self::tac_stat(w, id, "atk")? * scale,
                        DamageType::Physical,
                    );
                    self.enemy_damage(w, r, &mut q)?;
                }
            }
            BossTask::Species { loss } => self.lose_lp(loss)?,
        }
        Ok(true)
    }
}
