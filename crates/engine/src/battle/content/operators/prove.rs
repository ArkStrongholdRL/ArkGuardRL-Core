// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "closerange",
    can_hit_fly: true,
    ranged: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            kind: Kind::Passive,
            sp_type: SpType::None,
            ..Skill::duration("skchr_prove_1")
        },
        Skill::duration("skchr_prove_2"),
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.modifiers = {
        s.hit = HitHandler::Prove(Hit {
            drop: number(&bb["hp_ratio_drop"], 0.),
            up: number(&bb["atk_scale_up"], 0.),
            prob: number(&t["prob"], 0.),
            front_prob: number(&t["prob2"], 0.),
            crit_scale: number(&t["atk_scale"], 1.),
        });
        if default_skill {
            Numbers::new()
        } else {
            [("atkPct".into(), number(&bb["atk"], 0.))].into()
        }
    };
    if !default_skill {
        s.attack_filter = Some(huntable);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub drop: f64,
    pub up: f64,
    pub prob: f64,
    pub front_prob: f64,
    pub crit_scale: f64,
}
impl Hit {
    pub fn apply(
        self,
        w: &mut World,
        source: Id,
        dir: Direction,
        q: &mut hit::Request,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let Self {
            drop,
            up,
            prob,
            front_prob,
            crit_scale,
        } = self;

        if drop > 0. && up > 0. {
            let u = w.unit_mut(q.hit.target)?;
            let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
            let ratio = if max > 0. { u.stats.hp / max } else { 0. };
            let lost = (1. - ratio).min(1.).max(0.);
            if lost > 0. {
                q.damage.amount *= 1. + (lost / drop) * up;
            }
        }
        if q.is_splash || q.tags.iter().any(|t| t == "chain") {
            return Ok(());
        }
        let home = w.unit(source)?.deployment.tile;
        let v = dir.vector();
        let front = [home[0] + v[0], home[1] + v[1]];
        let in_front = Grid::in_bounds(front)
            && body::in_range(
                w.unit(q.hit.target)?,
                &[front[0] as usize * COLS + front[1] as usize],
            );
        let p = if in_front { front_prob } else { prob };
        if p > 0. && rng.chance(p) {
            q.damage.amount *= crit_scale;
        }
        Ok(())
    }
}
fn huntable(w: &mut World, id: Id) -> Result<bool, String> {
    let u = w.unit_mut(id)?;
    let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
    Ok((if max > 0. { u.stats.hp / max } else { 0. }) <= 0.8 + 1e-9)
}
