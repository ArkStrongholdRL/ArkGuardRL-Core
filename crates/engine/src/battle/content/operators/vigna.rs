// SPDX-License-Identifier: GPL-3.0-or-later
//! Vigna's one-attack ATK proc and low-HP module multiplier.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "charger",
    can_hit_fly: false,
    ranged: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill::duration("skchr_vigna_2"),
        Skill::duration("skchr_vigna_1"),
    ],
    build,
};
pub(super) fn build(raw: &Value, _default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    let tb = &raw["trait"]["bb"];
    let bat = number(&raw["stats"]["bat"], 1.);
    let bat = if bat == 0. { 1. } else { bat };
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        (
            "batPct".into(),
            (number(&bb["base_attack_time"], 0.) / bat).max(-0.9),
        ),
    ]
    .into();
    s.attack_proc = Some(AttackProc {
        key: "vigna:proc",
        normal: number(&t["prob1"], 0.),
        active: number(&t["prob2"], 0.),
        atk: number(&t["atk"], 0.),
    });
    s.dp_on_kill = number(&tb["cost"], 1.);
    if !tb["hp_ratio"].is_null() {
        s.hit = HitHandler::Vigna(Hit {
            below: number(&tb["hp_ratio"], 0.),
            scale: number(&tb["atk_scale"], 1.),
        });
    }
    Ok(())
}
#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub below: f64,
    pub scale: f64,
}
impl Hit {
    pub fn apply(self, w: &mut World, q: &mut hit::Request) -> Result<(), String> {
        let u = w.unit_mut(q.hit.target)?;
        let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        if (if max > 0. { u.stats.hp / max } else { 0. }) < self.below {
            q.damage.amount *= self.scale;
        }
        Ok(())
    }
}
