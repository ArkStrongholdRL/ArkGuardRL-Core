// SPDX-License-Identifier: GPL-3.0-or-later
//! Tile-targeted preparation actions. The caller commits only a successful draft.
use super::*;
impl Preparation {
    pub(super) fn use_art(
        &mut self,
        uid: u64,
        tile: [i32; 2],
        direction: Direction,
        player: &mut Player,
    ) -> Result<(), Error> {
        let loc = self.find(uid).ok_or(Error::BadTarget)?;
        let item = self.piece(loc).clone();
        let raw = &self.rules.data.table("items").unwrap()[&item.key];
        if item.kind != Kind::Item || raw["itemType"] != "MAGIC" {
            return Err(Error::BadTarget);
        }
        if !(9..=12).contains(&tile[0]) || !(2..=10).contains(&tile[1]) {
            return Err(Error::BadTile);
        }
        let limit = self.rules.data.table("config").unwrap()["economy"]["maxArtsPerRound"]
            .as_u64()
            .filter(|n| *n > 0)
            .unwrap_or(2);
        if self.round.arts >= limit {
            return Err(Error::BadTarget);
        }
        if self.meta.is_none() {
            return Err(Error::Content("Arts require preparation effects".into()));
        }
        let buffs = raw["buffs"]
            .as_array()
            .ok_or_else(|| Error::Content("Missing art effects".into()))?;
        if buffs.iter().any(|b| b["key"] == "trap_create_self_choice")
            && buffs.iter().all(|b| {
                matches!(
                    b["key"].as_str(),
                    Some("trap_create_self_choice" | "trap_disney_special")
                )
            })
        {
            self.bounty_art(&item.key)?;
        } else if buffs.len() == 1 && buffs[0]["key"] == "trap_copy_front_char" {
            let default = vec![json!([0, 0])];
            let range = raw["rangeGrid"]
                .as_array()
                .filter(|a| !a.is_empty())
                .unwrap_or(&default);
            if range.len() > self.capacity {
                return Err(Error::Capacity);
            }
            let mut target = None;
            for offset in range {
                let r = offset[0]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or(Error::BadTarget)?;
                let c = offset[1]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or(Error::BadTarget)?;
                if r == i32::MIN || c == i32::MIN {
                    return Err(Error::BadTarget);
                }
                let [dr, dc] = direction.rotate([r, c]);
                let at = [
                    tile[0].checked_add(dr).ok_or(Error::BadTarget)?,
                    tile[1].checked_add(dc).ok_or(Error::BadTarget)?,
                ];
                if let Some(p) = self.board.get(&at).filter(|p| p.kind == Kind::Chess) {
                    target = Some(p.clone());
                    break;
                }
            }
            let target = target.ok_or(Error::BadTarget)?;
            // Snapshot before acquisition: merging can remove the original carrier and its items.
            if self
                .gain(Kind::Chess, &target.key, GainSource::Grant, player)?
                .is_none()
            {
                return Err(Error::HandFull);
            }
            for equipped in target.items {
                self.gain(Kind::Item, &equipped.key, GainSource::Grant, player)?;
            }
        } else {
            return Err(Error::Content(format!("Unsupported art: {}", item.key)));
        }
        if let Some(loc) = self.find(uid) {
            self.take(loc);
        }
        self.release_item(&item);
        self.round.arts = add(self.round.arts, 1).map_err(|_| Error::Capacity)?;
        self.refill();
        Ok(())
    }
}
