//! Stage a complete visual cut before retiring its previous covering meshes.

use crate::{
    World,
    lod_mesh::{self, Cache, Key},
    lod_seam::{self, Caps, Geometry, Join},
};
use blockloom_plugin_api::mesh::{ColliderKind, MeshData};
use blockloom_plugin_sdk::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CUT_WORDS: usize = 524288;

struct Build {
    keys: Vec<Key>,
    generation: u64,
    revision: u64,
    // None reuses an installed tile without staging its render geometry.
    groups: BTreeMap<Key, Option<Geometry>>,
    words: usize,
    seam_ready: BTreeSet<Key>,
}

#[derive(Clone)]
struct PublishedTile {
    names: BTreeSet<String>,
    words: usize,
    valid: bool,
    caps: Caps,
    joins: Vec<Join>,
}

#[derive(Default)]
pub struct Publisher {
    pub names: BTreeSet<String>,
    pub active: bool,
    pub error: Option<String>,
    cache: Cache,
    build: Option<Build>,
    failed: Option<(u64, u64, Vec<Key>)>,
    pub tiles: usize,
    pub revision: u64,
    pub generation: u64,
    active_keys: Vec<Key>,
    installed: BTreeMap<Key, PublishedTile>,
    observed_revision: Option<u64>,
}

impl Publisher {
    pub fn reset_jobs(&mut self) {
        self.cache = Cache::default();
        self.build = None;
        self.failed = None;
        self.error = None;
        self.observed_revision = None;
        for tile in self.installed.values_mut() {
            tile.valid = false;
        }
    }

    pub fn invalidate(
        &mut self,
        bounds: Option<([i32; 3], [i32; 3])>,
        size: [i32; 3],
        revision: u64,
    ) {
        let Some(bounds) = bounds else { return };
        self.cache.invalidate(Some(bounds));
        if let Some(build) = &mut self.build {
            build.groups.retain(|key, groups| {
                let neighbors = lod_seam::joins(*key, &build.keys, size);
                if !key.affected(size, bounds)
                    && !neighbors.iter().any(|j| j.other.affected(size, bounds))
                {
                    return true;
                }
                build.seam_ready.remove(key);
                build.words -= groups
                    .as_ref()
                    .map_or_else(|| self.installed[key].words, Geometry::words);
                false
            });
            build.revision = revision;
        }
        for (key, tile) in &mut self.installed {
            if key.affected(size, bounds)
                || tile.joins.iter().any(|j| j.other.affected(size, bounds))
            {
                tile.valid = false;
            }
        }
        self.failed = None;
        self.error = None;
        self.observed_revision = Some(revision);
    }

    pub fn pending(&self) -> usize {
        self.build
            .as_ref()
            .map_or(0, |b| b.keys.len() - b.seam_ready.len())
    }

    pub fn fallback(&mut self, world: &World) -> Vec<Value> {
        let mut effects: Vec<_> = self
            .names
            .iter()
            .map(|name| json!({"effect":"remove_mesh","name":name}))
            .collect();
        if self.active {
            effects.extend(
                world
                    .published
                    .values()
                    .flatten()
                    .map(|name| json!({"effect":"mesh_visibility","name":name,"visible":true})),
            );
        }
        *self = Self::default();
        effects
    }

    pub fn advance(&mut self, world: &mut World, keys: Vec<Key>) -> Result<Vec<Value>, String> {
        let identity = (world.lod_generation, world.grid.revision());
        let mut changes = world.grid.take_lod_changes();
        if changes.is_none() && self.observed_revision != Some(identity.1) {
            changes = Some(([0; 3], world.grid.size().map(|s| s - 1)));
        }
        world.lod_meshes.invalidate(changes);
        self.invalidate(changes, world.grid.size(), identity.1);
        if self.active && self.active_keys == keys && (self.generation, self.revision) == identity {
            return Ok(Vec::new());
        }
        if self
            .failed
            .as_ref()
            .is_some_and(|(g, r, k)| (*g, *r) == identity && *k == keys)
        {
            return Ok(Vec::new());
        }
        let roots = |keys: &[Key]| {
            keys.iter()
                .map(|k| k.tile.map(|c| c >> (4 - k.level)))
                .collect::<BTreeSet<_>>()
        };
        let restart = self.build.as_ref().is_none_or(|b| {
            (b.generation, b.revision) != identity || roots(&b.keys).is_disjoint(&roots(&keys))
        });
        if restart {
            self.cache = Cache::default();
            self.failed = None;
            self.error = None;
            let groups: BTreeMap<_, _> = keys
                .iter()
                .filter(|key| {
                    self.generation == identity.0
                        && self.installed.get(key).is_some_and(|t| {
                            t.valid && t.joins == lod_seam::joins(**key, &keys, world.grid.size())
                        })
                })
                .map(|&key| (key, None))
                .collect();
            let words = groups.keys().map(|key| self.installed[key].words).sum();
            let seam_ready = groups.keys().copied().collect();
            self.build = Some(Build {
                keys,
                generation: identity.0,
                revision: identity.1,
                groups,
                words,
                seam_ready,
            });
        }
        let build = self.build.as_mut().unwrap();
        if let Some(&key) = build.keys.iter().find(|k| !build.groups.contains_key(k)) {
            let joins = lod_seam::joins(key, &build.keys, world.grid.size());
            let faces = std::array::from_fn(|face| joins.iter().any(|j| j.face == face));
            let result = self.cache.poll(
                key,
                &mut world.grid,
                &world.palette,
                world.surface,
                world.voxel,
            );
            let groups = match result {
                Ok(tile) => {
                    if tile.groups.is_some() {
                        tile.visual(
                            &world.grid,
                            &world.palette,
                            world.surface,
                            world.voxel,
                            faces,
                        )
                        .map(Some)
                    } else {
                        Ok(None)
                    }
                }
                Err(why) => Err(why),
            };
            let groups = match groups {
                Ok(groups) => groups,
                Err(why) => {
                    self.failed = Some((build.generation, build.revision, build.keys.clone()));
                    self.build = None;
                    return Err(why);
                }
            };
            let Some(groups) = groups else {
                return Ok(Vec::new());
            };
            let words = groups.words();
            if build.words + words > MAX_CUT_WORDS {
                self.failed = Some((build.generation, build.revision, build.keys.clone()));
                self.build = None;
                return Err("visual LOD cut exceeds its 2 MiB geometry budget".into());
            }
            build.words += words;
            build.groups.insert(key, Some(groups));
        }
        if build.groups.len() != build.keys.len() {
            return Ok(Vec::new());
        }
        if let Some(&key) = build.keys.iter().find(|k| !build.seam_ready.contains(k)) {
            let mut geometry = build.groups.remove(&key).unwrap().unwrap();
            let joins = lod_seam::joins(key, &build.keys, world.grid.size());
            let neighbors: Vec<_> = joins
                .into_iter()
                .map(|j| {
                    let caps = build.groups[&j.other]
                        .as_ref()
                        .map_or_else(|| &self.installed[&j.other].caps, |g| &g.caps);
                    (j, caps)
                })
                .collect();
            let old_words = geometry.words();
            let result = lod_seam::stitch(
                key,
                std::mem::take(&mut geometry.groups),
                &geometry.caps,
                &neighbors,
                world.grid.size(),
                world.voxel,
                &world.palette,
            );
            let groups = match result {
                Ok(groups) => groups,
                Err(why) => {
                    self.failed = Some((build.generation, build.revision, build.keys.clone()));
                    self.build = None;
                    return Err(why);
                }
            };
            geometry.groups = groups;
            if geometry.words() > lod_mesh::MAX_OUTPUT_WORDS {
                self.failed = Some((build.generation, build.revision, build.keys.clone()));
                self.build = None;
                return Err("visual LOD seam exceeds the tile output budget".into());
            }
            build.words = build.words - old_words + geometry.words();
            if build.words > MAX_CUT_WORDS {
                self.failed = Some((build.generation, build.revision, build.keys.clone()));
                self.build = None;
                return Err("visual LOD seams exceed the 2 MiB cut budget".into());
            }
            build.groups.insert(key, Some(geometry));
            build.seam_ready.insert(key);
        }
        if build.seam_ready.len() != build.keys.len() {
            return Ok(Vec::new());
        }
        if self.active
            && world
                .pending_fragments
                .values()
                .any(|(pages, _)| pages.iter().any(|p| world.pending.contains(p)))
        {
            return Ok(Vec::new());
        }
        let build = self.build.take().unwrap();
        if (build.generation, build.revision) != (world.lod_generation, world.grid.revision()) {
            return Ok(Vec::new());
        }
        let mut names = BTreeSet::new();
        let mut effects = Vec::new();
        let mut installed = BTreeMap::new();
        for (key, groups) in build.groups {
            let Some(groups) = groups else {
                let tile = self.installed[&key].clone();
                names.extend(tile.names.iter().cloned());
                installed.insert(key, tile);
                continue;
            };
            let words = groups.words();
            let caps = groups.caps;
            let joins = lod_seam::joins(key, &build.keys, world.grid.size());
            let mut tile_names = BTreeSet::new();
            let [x, y, z] = key.tile;
            let base = key.tile.map(|c| c * (lod_mesh::TILE << key.level));
            for (glow, group) in groups.groups {
                let name = format!(
                    "visual/{}/{x}/{y}/{z}{}",
                    key.level,
                    glow.map_or(String::new(), |m| format!("/glow{m}"))
                );
                let gpu = if world.settings.gpu_meshing && world.surface == crate::Surface::Cubes {
                    crate::gpu_mesh::visual(&group, world.voxel)
                } else {
                    None
                };
                let mesh = MeshData {
                    name: name.clone(),
                    positions: group.positions,
                    normals: group.normals,
                    colors: group.colors,
                    indices: group.indices,
                    origin: [0, 1, 2].map(|a| world.origin[a] + base[a] as f32 * world.voxel),
                    emission: glow
                        .and_then(|m| world.palette.get(m))
                        .map(|m| m.color.map(|c| c * m.emission)),
                    roughness: 0.9,
                    transition_ms: 150,
                    collider: false,
                    collider_kind: ColliderKind::Trimesh,
                    gpu,
                    body: None,
                };
                mesh.check()?;
                let mut effect = crate::serde_value(&mesh);
                effect["effect"] = json!("mesh");
                effects.push(effect);
                tile_names.insert(name);
            }
            names.extend(tile_names.iter().cloned());
            installed.insert(
                key,
                PublishedTile {
                    names: tile_names,
                    words,
                    valid: true,
                    caps,
                    joins,
                },
            );
        }
        effects.extend(
            self.names
                .difference(&names)
                .map(|name| json!({"effect":"remove_mesh","name":name})),
        );
        if !self.active {
            effects.extend(
                world
                    .published
                    .values()
                    .flatten()
                    .map(|name| json!({"effect":"mesh_visibility","name":name,"visible":false})),
            );
        }
        self.installed = installed;
        self.names = names;
        self.active = true;
        self.tiles = build.keys.len();
        self.active_keys = build.keys;
        self.generation = build.generation;
        self.revision = build.revision;
        self.cache = Cache::default();
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Settings, Surface};
    fn world() -> World {
        let mut world = World::new(&Settings {
            size: [256.0; 3],
            streamed: true,
            preset: "empty".into(),
            ..Settings::default()
        })
        .unwrap();
        world.generate("empty", 0).unwrap();
        world.grid.set([4, 4, 4], 1);
        world.grid.set([40, 4, 4], 7);
        world
    }
    fn finish(publisher: &mut Publisher, world: &mut World, keys: &[Key]) -> Vec<Value> {
        for _ in 0..200 {
            let effects = publisher.advance(world, keys.to_vec()).unwrap();
            if publisher.active
                && publisher.active_keys == keys
                && publisher.revision == world.grid.revision()
            {
                return effects;
            }
            assert!(effects.is_empty());
        }
        panic!("visual cut did not finish");
    }
    #[test]
    fn cube_lod_cuts_submit_persistent_records_and_bounded_crossfades() {
        let mut world = world();
        world.settings.gpu_meshing = true;
        world.surface = Surface::Cubes;
        let mut publisher = Publisher::default();
        let effects = finish(
            &mut publisher,
            &mut world,
            &[Key {
                level: 3,
                tile: [0; 3],
            }],
        );
        let meshes: Vec<_> = effects
            .iter()
            .filter(|effect| effect["effect"] == "mesh")
            .map(|effect| serde_json::from_value::<MeshData>(serde_json::to_value(effect).unwrap()))
            .collect();
        assert!(!meshes.is_empty());
        for mesh in meshes {
            let mesh = mesh.unwrap();
            mesh.check().unwrap();
            assert!(!mesh.collider);
            assert_eq!(mesh.transition_ms, 150);
            assert!(!mesh.gpu.unwrap().quads.unwrap().records.is_empty());
        }
        assert!(
            effects
                .iter()
                .all(|effect| effect["effect"] != "gpu_dispatch")
        );
    }

    #[test]
    fn refinement_waits_for_all_children_and_revalidates_edits() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let parent = Key {
            level: 3,
            tile: [0; 3],
        };
        let first = finish(&mut publisher, &mut world, &[parent]);
        assert!(
            first
                .iter()
                .any(|v| v["effect"] == "mesh" && v["collider"] == false)
        );
        let names = publisher.names.clone();
        let children: Vec<_> = (0..8)
            .map(|i| Key {
                level: 2,
                tile: [i & 1, (i >> 1) & 1, (i >> 2) & 1],
            })
            .collect();
        assert!(
            publisher
                .advance(&mut world, children.clone())
                .unwrap()
                .is_empty()
        );
        world.grid.set([40, 4, 4], 0);
        assert!(
            publisher
                .advance(&mut world, children.clone())
                .unwrap()
                .is_empty()
        );
        assert_eq!(publisher.names, names);
        let final_effects = finish(&mut publisher, &mut world, &children);
        assert!(names.iter().all(|name| {
            final_effects
                .iter()
                .any(|e| e["effect"] == "remove_mesh" && e["name"] == *name)
        }));
        assert!(!publisher.names.iter().any(|name| name.ends_with("glow7")));
        assert_eq!(publisher.revision, world.grid.revision());
        assert_eq!(world.grid.allocated_bytes(), 0);
        world.reset_lod_meshes();
        publisher.reset_jobs();
        let final_effects = finish(&mut publisher, &mut world, &[parent]);
        assert!(final_effects.iter().any(|e| e["effect"] == "mesh"));
        assert_eq!(publisher.generation, world.lod_generation);
    }
    #[test]
    fn output_failure_retains_cover_and_teleports_cancel_old_work() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let parent = Key {
            level: 3,
            tile: [0; 3],
        };
        finish(&mut publisher, &mut world, &[parent]);
        let names = publisher.names.clone();
        let target = vec![Key {
            level: 1,
            tile: [0; 3],
        }];
        publisher.build = Some(Build {
            keys: target.clone(),
            generation: world.lod_generation,
            revision: world.grid.revision(),
            groups: BTreeMap::new(),
            words: MAX_CUT_WORDS,
            seam_ready: BTreeSet::new(),
        });
        assert!(publisher.advance(&mut world, target.clone()).is_err());
        assert_eq!(publisher.names, names);
        assert!(publisher.advance(&mut world, target).unwrap().is_empty());
        let old = vec![Key {
            level: 4,
            tile: [0; 3],
        }];
        assert!(publisher.advance(&mut world, old).unwrap().is_empty());
        let moved = vec![Key {
            level: 4,
            tile: [1; 3],
        }];
        assert!(
            publisher
                .advance(&mut world, moved.clone())
                .unwrap()
                .is_empty()
        );
        assert_eq!(publisher.build.as_ref().unwrap().keys, moved);
    }
    #[test]
    fn fracture_publication_waits_for_both_visual_and_collider_replacements() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let parent = Key {
            level: 3,
            tile: [0; 3],
        };
        finish(&mut publisher, &mut world, &[parent]);
        let old = publisher.names.clone();
        world.grid.set([4, 4, 4], 0);
        world.pending.insert([0; 3]);
        world.pending_fragments.insert(
            1,
            (
                BTreeSet::from([[0; 3]]),
                json!({"effect":"mesh","name":"fragment/1"}),
            ),
        );
        for _ in 0..30 {
            assert!(
                publisher
                    .advance(&mut world, vec![parent])
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(publisher.names, old);
        assert_eq!(publisher.build.as_ref().unwrap().groups.len(), 1);
        world.pending.clear();
        let mut effects = publisher.advance(&mut world, vec![parent]).unwrap();
        assert!(!effects.is_empty());
        world.lod_publisher = publisher;
        effects.extend(world.release_fragments());
        assert!(effects.iter().any(|e| e["name"] == "fragment/1"));
        assert!(world.pending_fragments.is_empty());
    }

    #[test]
    fn edits_reuse_installed_tiles_and_rebuild_boundary_dependencies() {
        for surface in [Surface::Cubes, Surface::Smooth] {
            let mut world = world();
            world.surface = surface;
            let keys = [0, 1, 4].map(|x| Key {
                level: 1,
                tile: [x, 0, 0],
            });
            world
                .grid
                .set_shaped([15, 4, 4], 7, crate::shape::Shape::Post);
            world
                .grid
                .set_shaped([16, 4, 4], 1, crate::shape::Shape::Post);
            world
                .grid
                .set_shaped([68, 4, 4], 1, crate::shape::Shape::Post);
            let mut publisher = Publisher::default();
            finish(&mut publisher, &mut world, &keys);
            let far_names = publisher.installed[&keys[2]].names.clone();
            assert!(!far_names.is_empty());
            world
                .grid
                .set_shaped([16, 4, 4], 7, crate::shape::Shape::Post);
            let effects = finish(&mut publisher, &mut world, &keys);
            assert!(
                effects
                    .iter()
                    .any(|e| e["effect"] == "mesh" && e["name"] == "visual/1/0/0/0/glow7")
            );
            assert!(
                effects
                    .iter()
                    .any(|e| e["effect"] == "mesh" && e["name"] == "visual/1/1/0/0/glow7")
            );
            assert!(
                effects
                    .iter()
                    .all(|e| !far_names.iter().any(|n| e["name"] == *n))
            );
            world.grid.set([15, 4, 4], 0);
            world.grid.set([16, 4, 4], 0);
            let effects = finish(&mut publisher, &mut world, &keys);
            assert!(
                effects
                    .iter()
                    .any(|e| e["effect"] == "remove_mesh" && e["name"] == "visual/1/0/0/0/glow7")
            );
            assert!(
                effects
                    .iter()
                    .all(|e| !far_names.iter().any(|n| e["name"] == *n))
            );
            world.grid.set([200, 200, 200], 1);
            assert!(finish(&mut publisher, &mut world, &keys).is_empty());
            assert_eq!(publisher.revision, world.grid.revision());
            assert_eq!(world.grid.allocated_bytes(), 0);
        }
    }

    #[test]
    fn edits_preserve_staged_geometry_and_partial_sampling_outside_the_halo() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let keys = [0, 1].map(|x| Key {
            level: 3,
            tile: [x, 0, 0],
        });
        // Stage the first tile, then begin sampling the second.
        while publisher.build.as_ref().is_none_or(|b| b.groups.is_empty()) {
            assert!(
                publisher
                    .advance(&mut world, keys.to_vec())
                    .unwrap()
                    .is_empty()
            );
        }
        publisher.advance(&mut world, keys.to_vec()).unwrap();
        let before = publisher
            .cache
            .poll(
                keys[1],
                &mut world.grid,
                &world.palette,
                world.surface,
                world.voxel,
            )
            .unwrap()
            .progress();
        world.grid.set([200, 200, 200], 1);
        assert!(
            publisher
                .advance(&mut world, keys.to_vec())
                .unwrap()
                .is_empty()
        );
        assert!(
            publisher
                .build
                .as_ref()
                .unwrap()
                .groups
                .contains_key(&keys[0])
        );
        let after = publisher
            .cache
            .poll(
                keys[1],
                &mut world.grid,
                &world.palette,
                world.surface,
                world.voxel,
            )
            .unwrap()
            .progress();
        assert!(after > before);
        // This edit invalidates staged tile zero but is outside tile one's halo.
        world.grid.set([4, 4, 4], 7);
        assert!(
            publisher
                .advance(&mut world, keys.to_vec())
                .unwrap()
                .is_empty()
        );
        assert!(
            !publisher
                .build
                .as_ref()
                .unwrap()
                .groups
                .contains_key(&keys[0])
        );
        let retained = publisher
            .cache
            .poll(
                keys[1],
                &mut world.grid,
                &world.palette,
                world.surface,
                world.voxel,
            )
            .unwrap()
            .progress();
        assert!(retained >= after);
        let effects = finish(&mut publisher, &mut world, &keys);
        assert!(effects.iter().any(|e| e["name"] == "visual/3/0/0/0/glow7"));
    }

    #[test]
    fn inspection_and_fine_flush_share_invalidation_with_visual_publication() {
        let mut world = world();
        let keys = [0, 4].map(|x| Key {
            level: 1,
            tile: [x, 0, 0],
        });
        world.grid.set([68, 4, 4], 1);
        let mut publisher = Publisher::default();
        finish(&mut publisher, &mut world, &keys);
        world.lod_publisher = publisher;
        world.grid.set([4, 4, 4], 7);
        world.invalidate_lod_changes();
        assert!(world.grid.take_lod_changes().is_none());
        assert!(!world.lod_publisher.installed[&keys[0]].valid);
        assert!(world.lod_publisher.installed[&keys[1]].valid);
        let mut publisher = std::mem::take(&mut world.lod_publisher);
        let effects = finish(&mut publisher, &mut world, &keys);
        assert!(effects.iter().any(|e| e["name"] == "visual/1/0/0/0/glow7"));
        assert!(effects.iter().all(|e| e["name"] != "visual/1/4/0/0"));
        // Losing the change box still forces a conservative complete rebuild.
        world.grid.set([4, 4, 4], 1);
        world.grid.take_lod_changes();
        let effects = finish(&mut publisher, &mut world, &keys);
        assert!(effects.iter().any(|e| e["name"] == "visual/1/4/0/0"));
    }

    #[test]
    fn camera_changes_reuse_matching_tiles_and_account_for_their_payload() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let keys = [0, 4, 5].map(|x| Key {
            level: 1,
            tile: [x, 0, 0],
        });
        world.grid.set([68, 4, 4], 1);
        world.grid.set([84, 4, 4], 1);
        finish(&mut publisher, &mut world, &keys[..2]);
        let reused_names = publisher.installed[&keys[0]].names.clone();
        let effects = finish(&mut publisher, &mut world, &[keys[0], keys[2]]);
        assert!(
            effects
                .iter()
                .all(|e| !reused_names.iter().any(|n| e["name"] == *n))
        );
        assert!(
            effects
                .iter()
                .any(|e| e["effect"] == "remove_mesh" && e["name"] == "visual/1/4/0/0")
        );
        assert!(
            effects
                .iter()
                .any(|e| e["effect"] == "mesh" && e["name"] == "visual/1/5/0/0")
        );
        let names = publisher.names.clone();
        // Exhaust the budget with retained geometry; a new tile must still be counted.
        publisher.installed.get_mut(&keys[0]).unwrap().words = MAX_CUT_WORDS;
        assert!(publisher.advance(&mut world, keys[..2].to_vec()).is_err());
        assert_eq!(publisher.names, names);
        assert!(publisher.active);
        assert_eq!(publisher.active_keys, [keys[0], keys[2]]);
    }

    #[test]
    fn seam_dependencies_and_layout_changes_rebuild_both_sides_atomically() {
        let mut world = world();
        let mut publisher = Publisher::default();
        let a = Key {
            level: 1,
            tile: [0; 3],
        };
        let b = Key {
            level: 0,
            tile: [2, 0, 0],
        };
        let c = Key {
            level: 0,
            tile: [5, 0, 0],
        };
        world.grid.set([16, 4, 4], 7);
        finish(&mut publisher, &mut world, &[a, b, c]);
        assert_eq!(publisher.installed[&a].joins.len(), 1);
        assert_eq!(publisher.installed[&b].joins.len(), 1);
        assert!(publisher.installed[&c].caps.iter().all(|p| p.is_empty()));
        let old = publisher.names.clone();
        world.grid.set([12, 4, 4], 7);
        assert!(!b.affected(world.grid.size(), ([12, 4, 4], [12, 4, 4])));
        assert!(
            publisher
                .advance(&mut world, vec![a, b, c])
                .unwrap()
                .is_empty()
        );
        assert!(!publisher.installed[&a].valid);
        assert!(!publisher.installed[&b].valid);
        assert!(publisher.installed[&c].valid);
        assert_eq!(publisher.names, old);
        let effects = finish(&mut publisher, &mut world, &[a, b, c]);
        assert!(effects.iter().any(|e| e["name"] == "visual/0/2/0/0/glow7"));
        assert!(effects.iter().all(|e| e["name"] != "visual/0/5/0/0/glow7"));
        // Removing a neighbor changes the face topology even without a voxel edit.
        let effects = finish(&mut publisher, &mut world, &[b, c]);
        assert!(
            effects
                .iter()
                .any(|e| e["name"] == "visual/0/2/0/0/glow7" && e["effect"] == "mesh")
        );
        assert!(publisher.installed[&b].joins.is_empty());
    }

    #[test]
    fn level_zero_keeps_exact_shapes_in_both_surface_modes() {
        for surface in [Surface::Cubes, Surface::Smooth] {
            let mut world = world();
            world.surface = surface;
            world.grid.reshape([4, 4, 4], crate::shape::Shape::Post);
            let key = Key {
                level: 0,
                tile: [0; 3],
            };
            let effects = finish(&mut Publisher::default(), &mut world, &[key]);
            let mesh = effects.iter().find(|e| e["effect"] == "mesh").unwrap();
            let positions = mesh["positions"].as_array().unwrap();
            assert!(positions.chunks_exact(3).any(|p| {
                let x = p[0].as_f64().unwrap();
                x > 4.0 && x < 5.0
            }));
        }
    }
}
