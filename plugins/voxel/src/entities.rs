//! Block entities: data that lives in one cell, such as a chest's slots.
//!
//! An entity is made the first time its cell is asked about and goes when
//! the block under it changes, handing back what it held.

use crate::grid::Block;
use crate::registry::{EntityDef, Registry};
use blockloom_plugin_sdk::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Entities a world may hold.
pub const MAX_ENTITIES: usize = 16384;
/// Bytes of free-form data one entity may hold, as JSON.
const MAX_DATA: usize = 4096;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stack {
    pub item: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockEntity {
    pub kind: String,
    #[serde(default)]
    pub data: BTreeMap<String, Value>,
    #[serde(default)]
    pub items: Vec<Option<Stack>>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Entities {
    #[serde(with = "cells")]
    by_cell: BTreeMap<[i32; 3], BlockEntity>,
    #[serde(skip)]
    defs: BTreeMap<Block, EntityDef>,
}

/// JSON maps need text keys, so cells save as `[[x, y, z], entity]` pairs.
mod cells {
    use super::BlockEntity;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<[i32; 3], BlockEntity>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<[i32; 3], BlockEntity>, D::Error> {
        Ok(Vec::<([i32; 3], BlockEntity)>::deserialize(d)?
            .into_iter()
            .collect())
    }
}

fn item_name(item: &str) -> Result<(), String> {
    if item.is_empty() || item.len() > 64 {
        return Err("an item name needs 1 to 64 characters".into());
    }
    Ok(())
}

impl Entities {
    pub fn attach(&mut self, registry: &Registry) {
        self.defs = registry
            .defs()
            .filter_map(|d| d.entity.clone().map(|e| (d.id, e)))
            .collect();
    }

    pub fn len(&self) -> usize {
        self.by_cell.len()
    }

    /// The entity at a cell, made fresh when the block there carries one.
    pub fn at(&mut self, cell: [i32; 3], block: Block) -> Result<&mut BlockEntity, String> {
        let def = self
            .defs
            .get(&block)
            .ok_or("no block entity lives in that cell")?;
        if self.by_cell.get(&cell).is_some_and(|e| e.kind != def.kind) {
            self.by_cell.remove(&cell);
        }
        if !self.by_cell.contains_key(&cell) && self.by_cell.len() >= MAX_ENTITIES {
            return Err(format!(
                "a world holds at most {MAX_ENTITIES} block entities"
            ));
        }
        Ok(self.by_cell.entry(cell).or_insert_with(|| BlockEntity {
            kind: def.kind.clone(),
            data: BTreeMap::new(),
            items: vec![None; def.slots],
        }))
    }

    pub fn stack_of(&self, block: Block) -> u32 {
        self.defs.get(&block).map_or(64, |d| d.stack)
    }

    /// Drops entities in the box whose block no longer carries their kind.
    pub fn reconcile(
        &mut self,
        lo: [i32; 3],
        hi: [i32; 3],
        block_at: impl Fn([i32; 3]) -> Block,
    ) -> Vec<([i32; 3], BlockEntity)> {
        let gone: Vec<_> = self
            .by_cell
            .iter()
            .filter(|(c, e)| {
                (0..3).all(|a| (lo[a]..=hi[a]).contains(&c[a]))
                    && self
                        .defs
                        .get(&block_at(**c))
                        .is_none_or(|d| d.kind != e.kind)
            })
            .map(|(c, _)| *c)
            .collect();
        gone.into_iter()
            .map(|c| (c, self.by_cell.remove(&c).unwrap()))
            .collect()
    }

    pub fn clear(&mut self) {
        self.by_cell.clear();
    }
}

impl BlockEntity {
    pub fn set_data(&mut self, key: &str, value: Value) -> Result<(), String> {
        if key.is_empty() || key.len() > 64 {
            return Err("a data key needs 1 to 64 characters".into());
        }
        let old = self.data.insert(key.to_string(), value);
        if blockloom_plugin_sdk::json!(self.data).to_string().len() > MAX_DATA {
            match old {
                Some(old) => self.data.insert(key.to_string(), old),
                None => self.data.remove(key),
            };
            return Err(format!(
                "a block entity holds at most {MAX_DATA} bytes of data"
            ));
        }
        Ok(())
    }

    /// Adds into matching stacks, then empty slots; answers what didn't fit.
    pub fn add(&mut self, item: &str, mut count: u32, stack: u32) -> Result<u32, String> {
        item_name(item)?;
        for pass in 0..2 {
            for slot in &mut self.items {
                if count == 0 {
                    return Ok(0);
                }
                match slot {
                    Some(s) if pass == 0 && s.item == item && s.count < stack => {
                        let n = count.min(stack - s.count);
                        s.count += n;
                        count -= n;
                    }
                    None if pass == 1 => {
                        let n = count.min(stack);
                        *slot = Some(Stack {
                            item: item.to_string(),
                            count: n,
                        });
                        count -= n;
                    }
                    _ => {}
                }
            }
        }
        Ok(count)
    }

    /// Takes up to `count`, last slots first; answers how many it took.
    pub fn take(&mut self, item: &str, mut count: u32) -> u32 {
        let wanted = count;
        for slot in self.items.iter_mut().rev() {
            if let Some(s) = slot
                && s.item == item
            {
                let n = count.min(s.count);
                s.count -= n;
                count -= n;
                if s.count == 0 {
                    *slot = None;
                }
            }
        }
        wanted - count
    }

    pub fn count(&self, item: &str) -> u32 {
        self.items
            .iter()
            .flatten()
            .filter(|s| item.is_empty() || s.item == item)
            .map(|s| s.count)
            .sum()
    }

    /// What it held, as `item xN` pairs.
    pub fn contents(&self) -> String {
        self.items
            .iter()
            .flatten()
            .map(|s| format!("{} x{}", s.item, s.count))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chest() -> (Registry, Entities) {
        let reg = Registry::parse(&[
            r#"{"name":"chest","id":40,"entity":{"kind":"chest","slots":3,"stack":10}}"#
                .to_string(),
        ])
        .unwrap();
        let mut e = Entities::default();
        e.attach(&reg);
        (reg, e)
    }

    #[test]
    fn a_chest_fills_its_stacks_then_its_slots_and_gives_back_what_it_held() {
        let (_, mut entities) = chest();
        assert!(entities.at([1, 1, 1], 1).is_err(), "stone carries nothing");
        let stack = entities.stack_of(40);
        let chest = entities.at([1, 1, 1], 40).unwrap();
        assert_eq!(chest.add("apple", 15, stack).unwrap(), 0);
        assert_eq!(
            chest.add("stick", 20, stack).unwrap(),
            10,
            "one slot of ten left"
        );
        assert_eq!(chest.count("apple"), 15);
        assert_eq!(chest.take("apple", 12), 12);
        assert_eq!(chest.count(""), 13);
        chest.set_data("owner", "ada".into()).unwrap();
        assert!(chest.set_data("big", "x".repeat(5000).into()).is_err());
        assert_eq!(entities.reconcile([0; 3], [2; 3], |_| 40).len(), 0);
        let gone = entities.reconcile([0; 3], [2; 3], |_| 0);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].1.contents(), "apple x3, stick x10");
        assert_eq!(gone[0].1.data["owner"], "ada");
        let saved = blockloom_plugin_sdk::json!({"e": entities.at([0, 0, 0], 40).map(|_| ()).ok()});
        assert!(saved["e"].is_null());
        let text = blockloom_plugin_sdk::serde_json::to_string(&entities).unwrap();
        let back: Entities = blockloom_plugin_sdk::serde_json::from_str(&text).unwrap();
        assert_eq!(back.len(), 1);
    }
}
