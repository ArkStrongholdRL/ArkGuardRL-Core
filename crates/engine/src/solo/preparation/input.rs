// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Action, Destination};
use crate::battle::direction::Direction;
use serde_json::Value;

pub(super) fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("Expected preparation input object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported preparation input: {key}"));
    }
    Ok(())
}
impl Action {
    /// Parse the original g.* preparation intent shape at the external boundary.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let kind = value["t"]
            .as_str()
            .ok_or("Expected preparation intent type")?;
        keys(
            value,
            match kind {
                "g.choice" => &["t", "idx", "choiceId"],
                "g.art" => &["t", "itemUid", "row", "col", "dir"],
                "g.buy" => &["t", "slot"],
                "g.reward" => &["t", "idx"],
                "g.unequip" => &["t", "itemUid"],
                "g.equip" => &["t", "itemUid", "targetUid", "replaceUid"],
                "g.sell" | "g.destroy" => &["t", "uid"],
                "g.move" => &["t", "uid", "to", "dir"],
                "g.refresh" | "g.freeze" | "g.levelUp" => &["t"],
                _ => return Err(format!("Unsupported preparation intent: {kind}")),
            },
        )?;
        let index = |key| {
            value[key]
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("Expected preparation slot index")
        };
        let uid = || value["uid"].as_u64().ok_or("Expected preparation uid");
        Ok(match kind {
            "g.choice" if value.get("choiceId").is_none() => Self::Draft(index("idx")?),
            "g.choice" => Self::Choice {
                index: index("idx")?,
                id: value["choiceId"]
                    .as_str()
                    .ok_or("Expected choice id")?
                    .into(),
            },
            "g.art" => Self::Art {
                item: value["itemUid"].as_u64().ok_or("Expected art uid")?,
                tile: {
                    let coordinate = |key: &str| {
                        value[key]
                            .as_i64()
                            .and_then(|v| i32::try_from(v).ok())
                            .ok_or("Expected art coordinate")
                    };
                    [coordinate("row")?, coordinate("col")?]
                },
                direction: match value
                    .get("dir")
                    .filter(|v| !v.is_null())
                    .map(Value::as_str)
                    .unwrap_or(Some("RIGHT"))
                {
                    Some("RIGHT") => Direction::Right,
                    Some("UP") => Direction::Up,
                    Some("DOWN") => Direction::Down,
                    Some("LEFT") => Direction::Left,
                    _ => return Err("Invalid art facing".into()),
                },
            },
            "g.buy" => Self::Buy(index("slot")?),
            "g.reward" => Self::PickReward(index("idx")?),
            "g.equip" => Self::Equip {
                item: value["itemUid"].as_u64().ok_or("Expected equipment uid")?,
                target: value["targetUid"].as_u64().ok_or("Expected carrier uid")?,
                replace: value
                    .get("replaceUid")
                    .filter(|v| !v.is_null())
                    .map(|v| v.as_u64().ok_or("Expected replaced equipment uid"))
                    .transpose()?,
            },
            "g.unequip" => {
                Self::Unequip(value["itemUid"].as_u64().ok_or("Expected equipment uid")?)
            }
            "g.sell" => Self::Sell(uid()?),
            "g.destroy" => Self::Destroy(uid()?),
            "g.refresh" => Self::Refresh,
            "g.freeze" => Self::Freeze,
            "g.levelUp" => Self::LevelUp,
            "g.move" => {
                let to = &value["to"];
                let destination = match to["area"]
                    .as_str()
                    .ok_or("Expected preparation destination")?
                {
                    "hand" => {
                        keys(to, &["area", "idx"])?;
                        Destination::Hand(
                            to["idx"]
                                .as_u64()
                                .and_then(|v| usize::try_from(v).ok())
                                .ok_or("Expected hand index")?,
                        )
                    }
                    "board" => {
                        keys(to, &["area", "row", "col", "dir"])?;
                        let dir = value
                            .get("dir")
                            .or_else(|| to.get("dir"))
                            .filter(|v| !v.is_null());
                        let direction = match dir
                            .map(|v| v.as_str().ok_or("Expected facing string"))
                            .transpose()?
                            .unwrap_or("RIGHT")
                        {
                            "RIGHT" => Direction::Right,
                            "UP" => Direction::Up,
                            "DOWN" => Direction::Down,
                            "LEFT" => Direction::Left,
                            _ => return Err("Invalid preparation facing".into()),
                        };
                        let coordinate = |key| {
                            to[key]
                                .as_i64()
                                .and_then(|v| i32::try_from(v).ok())
                                .ok_or("Expected preparation board coordinate")
                        };
                        Destination::Board {
                            home: [coordinate("row")?, coordinate("col")?],
                            direction,
                        }
                    }
                    _ => return Err("Invalid preparation destination".into()),
                };
                Self::Move {
                    uid: uid()?,
                    to: destination,
                }
            }
            _ => unreachable!(),
        })
    }
}
