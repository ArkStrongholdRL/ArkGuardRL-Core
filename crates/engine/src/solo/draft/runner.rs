// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::solo::{map::DeployMap, Phase, RoundKind, Runner};

pub(in crate::solo) struct Flow {
    pub stage: String,
    pub rng: Rng,
    pub pending: Option<Value>,
}
impl Runner {
    /// Opt into SP decisions before the first battle. Unported picked effects
    /// fault the rollout explicitly; they are never removed from the draft pool.
    pub fn enable_drafts(&mut self, stage_id: &str) -> Result<(), String> {
        self.healthy()?;
        if self.index != 0 || self.phase() != Phase::Prep || self.draft.is_some() {
            return Err("Drafts must be enabled once before the first battle".into());
        }
        let prep = self
            .prep
            .as_ref()
            .filter(|p| p.has_effects())
            .ok_or("Drafts require owned preparation effects")?;
        if self.data.table("stages").and_then(|t| t.get(stage_id)) != Some(&self.stage) {
            return Err("Draft stage does not match the runner stage".into());
        }
        let mode = &self.data.table("config").unwrap()["modes"][prep.draft_mode()];
        if list(&mode["spRounds"]).contains(&json!(1)) {
            return Err("First-round drafts must precede preparation initialization".into());
        }
        self.draft = Some(Flow {
            stage: stage_id.into(),
            rng: Rng::new(crate::deterministic::rng::derive_seed(self.seed, "draft")),
            pending: None,
        });
        Ok(())
    }
    /// Select an offered index, once, then run the deferred preparation-start hooks.
    pub fn pick_draft(&mut self, index: usize) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Draft {
            return Err("WRONG_PHASE".into());
        }
        let card = self
            .draft
            .as_ref()
            .and_then(|d| d.pending.as_ref())
            .and_then(|d| d["cards"].get(index))
            .ok_or("BAD_TARGET")?
            .clone();
        let mut prep = self.prep.as_ref().unwrap().clone();
        let mut player = self.player.clone();
        let mut map = self.map.clone();
        let apply = (|| {
            prep.apply_draft(&card, &mut map, &mut player)
                .map_err(|e| e.to_string())?;
            map = map.clone().resolved(&self.data)?;
            let boss = !matches!(
                self.plans[self.index].as_ref().unwrap().kind,
                RoundKind::Normal
            );
            prep.change_map(DeployMap::new(&self.stage, &map, boss)?, &mut player)
                .map_err(|e| e.to_string())?;
            prep.enter_prep(&mut player).map_err(|e| e.to_string())
        })();
        if let Err(error) = apply {
            return self.fail(error);
        }
        self.prep = Some(prep);
        self.player = player;
        self.map = map;
        self.draft.as_mut().unwrap().pending = None;
        Ok(())
    }
}
