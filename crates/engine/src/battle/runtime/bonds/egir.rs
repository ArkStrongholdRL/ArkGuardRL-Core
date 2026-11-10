// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(Default)]
pub(super) struct Egir {
    opening: bool,
    seen: BTreeSet<Id>,
    batch: Vec<Id>,
    used: usize,
}
impl Host {
    pub(in crate::battle::runtime) fn egir_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "egirShip" && p.tier >= 2)
            .cloned()
        else {
            return Ok(());
        };
        let u = w.unit(id)?;
        if u.stats.alive
            || u.deployment.remove_reason.as_deref() != Some("killed")
            || !self.equipment_member(id, "egirShip")
            || !self.bonds.egir.seen.insert(id)
        {
            return Ok(());
        }
        if self.bonds.egir.opening {
            self.bonds.egir.batch.push(id);
            return Ok(());
        }
        self.egir_revive(w, r, id, &p)
    }
    fn egir_revive(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Bond,
    ) -> Result<(), String> {
        if self.bonds.egir.used >= p.n("max_free_respawn_cnt").max(0.) as usize
            || w.unit(id)?.stats.alive
            || w.unit(id)?.removed
        {
            return Ok(());
        }
        if (kits::dispatch::Context {
            host: self,
            world: w,
            buffs: r,
            unit: id,
        })
        .redeploy_free(id)?
        {
            self.bonds.egir.used += 1;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn egir_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "egirShip")
            .cloned()
        else {
            return Ok(());
        };
        let ops: Vec<_> = self
            .bond_ops(w)
            .into_iter()
            .filter(|&id| Self::tac_up(w, id))
            .collect();
        let mut members: Vec<_> = ops
            .iter()
            .copied()
            .filter(|&id| self.equipment_member(id, "egirShip"))
            .collect();
        members.sort_by_key(|&id| {
            let t = w.unit(id).unwrap().deployment.tile;
            (t[1], -t[0], id)
        });
        let mut marked: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
        let mut marks = Vec::new();
        for m in members {
            let mut seen = BTreeSet::from([m]);
            let mut queue = VecDeque::from([m]);
            let mut mine = Vec::new();
            while let Some(x) = queue.pop_front() {
                let tile = w.unit(x)?.deployment.tile;
                let d = self.roster.as_ref().unwrap().entry(x)?.direction.vector();
                let front = [tile[0] + d[0], tile[1] + d[1]];
                for &t in &ops {
                    if w.unit(t)?.deployment.tile != front
                        || seen.contains(&t)
                        || marked.get(&t).is_some_and(|v| v.contains(&m))
                    {
                        continue;
                    }
                    seen.insert(t);
                    mine.push(t);
                    if self.equipment_member(t, "egirShip") {
                        queue.push_back(t);
                    }
                }
            }
            let mut atk = 0.;
            let mut block = 0.;
            for &t in &mine {
                atk += w.unit(t)?.stats.base_number("atk");
                block += w.unit(t)?.stats.base_number("blockCnt");
                marks.push((m, t, w.unit(t)?.deployment.deploy_seq));
            }
            marked.insert(m, mine);
            let mut mods = Numbers::new();
            if atk > 0. {
                mods.insert("atkFinal".into(), atk);
            }
            if block > 0. {
                mods.insert("blockCnt".into(), block);
            }
            if !mods.is_empty() {
                self.equipment_stat(w, r, m, "bond:egir:devour".into(), mods)?;
            }
        }
        self.bonds.egir.opening = true;
        let result = (|| {
            let mut layered = BTreeSet::new();
            for (m, t, seq) in marks {
                if !w.unit(t)?.stats.alive || w.unit(t)?.deployment.deploy_seq != seq {
                    continue;
                }
                if p.n("damage_value") > 0. {
                    let mut q = deployment::damage(t, t, p.n("damage_value"), "bond:egir:devour");
                    q.hit.credit = Some(m);
                    q.hit.hp_loss = true;
                    q.hit.damage_type = crate::battle::damage::DamageType::Physical;
                    self.enemy_damage(w, r, &mut q)?;
                }
                if layered.insert(t) {
                    let key = &self.roster.as_ref().unwrap().entry(t)?.definition.key;
                    let tier = self
                        .data
                        .table("chess")
                        .and_then(|v| v.get(key))
                        .and_then(|v| v["tier"].as_f64())
                        .unwrap_or(0.);
                    self.growth_bond_gain(w, m, "egirShip", tier)?;
                }
            }
            Ok::<_, String>(())
        })();
        self.bonds.egir.opening = false;
        result?;
        let mut batch = std::mem::take(&mut self.bonds.egir.batch);
        batch.sort_by_key(|&id| {
            let t = w.unit(id).unwrap().deployment.tile;
            (t[1], t[0], id)
        });
        for id in batch {
            self.egir_revive(w, r, id, &p)?;
        }
        Ok(())
    }
}
