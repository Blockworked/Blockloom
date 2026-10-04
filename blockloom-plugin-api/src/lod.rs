//! Bounded visual tile descriptors and asynchronous visibility feedback.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_TILES: usize = 512;
pub const MAX_NAME_BYTES: usize = 128;

/// A visual job's conservative bounds in world coordinates, independent of meshes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub id: String,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Replaces a plugin's named request set. An empty set removes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileSet {
    pub name: String,
    pub generation: u64,
    pub revision: u64,
    pub tiles: Vec<Tile>,
}
impl TileSet {
    pub fn check(&self) -> Result<(), String> {
        let valid_name = |s: &str| !s.is_empty() && s.len() <= MAX_NAME_BYTES;
        if !valid_name(&self.name) || self.tiles.len() > MAX_TILES {
            return Err("LOD tile set needs a bounded name and at most 512 tiles".into());
        }
        let mut seen = BTreeSet::new();
        for tile in &self.tiles {
            if !valid_name(&tile.id) || !seen.insert(&tile.id) {
                return Err("LOD tile ids must be bounded, nonempty and unique".into());
            }
            if !(0..3).all(|a| {
                tile.min[a].is_finite() && tile.max[a].is_finite() && tile.min[a] < tile.max[a]
            }) {
                return Err("LOD tile bounds must be finite with positive extent".into());
            }
        }
        Ok(())
    }
}

/// Advisory job priorities. Missing feedback always permits CPU scheduling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub name: String,
    pub generation: u64,
    pub revision: u64,
    pub visible: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_sets_validate_identity_bounds_duplicates_and_limits() {
        let mut set = TileSet {
            name: "terrain".into(),
            generation: 1,
            revision: 2,
            tiles: vec![Tile {
                id: "0/0/0/0".into(),
                min: [0.; 3],
                max: [1.; 3],
            }],
        };
        set.check().unwrap();
        assert_eq!(
            serde_json::from_value::<TileSet>(serde_json::to_value(&set).unwrap()).unwrap(),
            set
        );
        set.tiles.push(set.tiles[0].clone());
        assert!(set.check().is_err());
        set.tiles.pop();
        set.tiles[0].max[0] = f32::NAN;
        assert!(set.check().is_err());
        set.tiles[0].max[0] = 0.;
        assert!(set.check().is_err());
        set.tiles[0].max[0] = 1.;
        set.tiles[0].id = "x".repeat(MAX_NAME_BYTES + 1);
        assert!(set.check().is_err());
        set.tiles = (0..=MAX_TILES)
            .map(|i| Tile {
                id: i.to_string(),
                min: [0.; 3],
                max: [1.; 3],
            })
            .collect();
        assert!(set.check().is_err());
        set.tiles.clear();
        set.check().unwrap();
    }
}
