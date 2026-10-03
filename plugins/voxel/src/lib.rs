//! Cube and smooth voxel worlds with live edits, player checkpoints and paging.
//!
//! Cells and sparse edits are authoritative; meshes are disposable. Streamed
//! worlds publish bounded sections near invokers, with optional GPU vertex output.
//! Explicit fracture detaches unsupported regions into editable moving bodies.
//! Project edit lines seed each run; persistent worlds restore player saves.

mod fracture;
mod gpu_mesh;
mod grid;
mod lod;
mod lod_mesh;
mod mesher;
mod palette;
mod ray;
mod shape;
mod smooth;
mod terrain;

use blockloom_plugin_api::mesh::{ColliderKind, MeshData};
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};
use grid::{Grid, SECTION, Shape};

const MESH_TILE: i32 = 16;
use palette::Palette;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The most cells a world may hold: generation and meshing both cost this.
const MAX_CELLS: i64 = 4_194_304;

/// The `world` resource, with what an absent field reads as.
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
struct Settings {
    seed: i64,
    size: [f64; 3],
    voxel_size: f64,
    origin: [f64; 3],
    preset: String,
    solid: bool,
    surface: Surface,
    palette_colors: Vec<String>,
    palette_emission: Vec<f64>,
    /// Edits laid over the generated terrain, oldest first (see `apply_edit`).
    edits: Vec<String>,
    persistent: bool,
    save_slot: String,
    streamed: bool,
    stream_radius: i32,
    vertical_radius: i32,
    max_resident_bytes: usize,
    max_pages: usize,
    pages_per_tick: usize,
    stream_center: [f64; 3],
    gpu_meshing: bool,
    max_fragments: usize,
    max_fragment_cells: usize,
    anchor_y: i32,
    palette_density: Vec<f64>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            seed: 1337,
            size: [64.0, 32.0, 64.0],
            voxel_size: 1.0,
            origin: [0.0; 3],
            preset: "island".to_string(),
            solid: true,
            surface: Surface::Cubes,
            palette_colors: Vec::new(),
            palette_emission: Vec::new(),
            edits: Vec::new(),
            persistent: false,
            save_slot: "world".into(),
            streamed: false,
            stream_radius: 2,
            vertical_radius: 2,
            max_resident_bytes: 16 * 1024 * 1024,
            max_pages: 64,
            pages_per_tick: 2,
            stream_center: [0.0; 3],
            gpu_meshing: false,
            max_fragments: 16,
            max_fragment_cells: 4096,
            anchor_y: 0,
            palette_density: Vec::new(),
        }
    }
}

impl Settings {
    fn identity(&self) -> Value {
        json!({"seed":self.seed,"size":self.size,"voxel_size":self.voxel_size,"origin":self.origin,
            "preset":self.preset,"surface":self.surface,"edits":self.edits,"streamed":self.streamed})
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Surface {
    #[default]
    Cubes,
    Smooth,
}

struct World {
    grid: Grid,
    lod_meshes: lod_mesh::Cache,
    lod_generation: u64,
    palette: Palette,
    voxel: f32,
    origin: [f32; 3],
    solid: bool,
    surface: Surface,
    seed: i64,
    settings: Settings,
    preview: bool,
    visible: BTreeSet<[i32; 3]>,
    pending: BTreeSet<[i32; 3]>,
    pending_fragments: BTreeMap<u64, (BTreeSet<[i32; 3]>, Value)>,
    centre: [i32; 3],
    serial: u64,
    fragments: BTreeMap<u64, fracture::Fragment>,
    next_fragment: u64,
    gpu_buffers: BTreeMap<[i32; 3], Vec<(String, u32)>>,
    /// The mesh names each chunk has drawn, so a remesh can retire the ones
    /// it no longer makes.
    published: BTreeMap<[i32; 3], BTreeSet<String>>,
}

fn check_edit_bounds(lo: [i32; 3], hi: [i32; 3]) -> Result<(), String> {
    let cells: i64 = (0..3)
        .map(|i| i64::from((hi[i] - lo[i] + 1).max(0)))
        .product();
    if cells > 262144 {
        return Err("one voxel brush may visit at most 262144 cells; split the region".into());
    }
    Ok(())
}

fn distance(a: [i32; 3], b: [i32; 3]) -> i64 {
    (0..3)
        .map(|i| {
            let d = i64::from(a[i]) - i64::from(b[i]);
            d * d
        })
        .sum()
}

fn validate_save_slot(slot: &str) -> Result<(), String> {
    if slot.is_empty()
        || slot.len() > 64
        || !slot
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(
            "a voxel save slot needs 1 to 64 letters, digits, underscores or hyphens".into(),
        );
    }
    Ok(())
}

fn save_key(slot: &str) -> Result<String, Error> {
    validate_save_slot(slot).map_err(Error::bad_argument)?;
    Ok(format!("voxel/{slot}.json"))
}

fn save_service(host: &Host, op: &str, args: &Value) -> Result<Value, Error> {
    let answer = host.call_json(op, args)?;
    if let Some(message) = answer["error"].as_str() {
        return Err(Error::new(message));
    }
    Ok(answer)
}

impl World {
    fn replace_fragments(
        &mut self,
        fragments: BTreeMap<u64, fracture::Fragment>,
    ) -> Result<Vec<Value>, Error> {
        let mut effects: Vec<_> = self
            .fragments
            .keys()
            .map(|id| json!({"effect":"remove_mesh","name":fracture::name(*id)}))
            .collect();
        for fragment in fragments.values() {
            effects.push(fragment.mesh(self)?);
        }
        self.next_fragment = fragments.keys().next_back().copied().unwrap_or(0) + 1;
        self.pending_fragments.clear();
        self.fragments = fragments;
        Ok(effects)
    }

    fn checkpoint(&self) -> Value {
        json!({"version":2,"settings":self.settings,"active_seed":self.seed,"grid":self.grid.snapshot(),"fragments":self.fragments.values().collect::<Vec<_>>()})
    }

    fn save(&self, host: &Host, slot: &str) -> Result<(), Error> {
        if self.preview {
            return Err(Error::new("player saves are unavailable in preview"));
        }
        save_service(
            host,
            "save.write",
            &json!({"key":save_key(slot)?, "text":self.checkpoint().to_string()}),
        )?;
        Ok(())
    }
}

type Checkpoint = (Grid, BTreeMap<u64, fracture::Fragment>, i64);

fn decode_checkpoint(world: &World, value: Value) -> Result<Checkpoint, Error> {
    let version = value["version"].as_u64().unwrap_or(0);
    if ![1, 2].contains(&version) {
        return Err(Error::new("unsupported voxel checkpoint version"));
    }
    let settings: Settings = serde_json_from(value["settings"].clone()).map_err(Error::new)?;
    if settings.identity() != world.settings.identity() {
        return Err(Error::new(
            "voxel checkpoint belongs to different world settings",
        ));
    }
    let snapshot: grid::Snapshot = serde_json_from(value["grid"].clone()).map_err(Error::new)?;
    let expected = if version == 1 {
        world.settings.size.map(|s| (s as i32 + 15) / 16 * 16)
    } else {
        world.grid.size()
    };
    if snapshot.size != expected
        || snapshot.page_size != if version == 1 { 16 } else { SECTION }
        || snapshot.generator.is_some() != world.settings.streamed
    {
        return Err(Error::new("voxel checkpoint bounds or storage mode differ"));
    }
    let mut grid = Grid::restore(snapshot, world.palette.len()).map_err(Error::new)?;
    if version == 1 {
        grid.clip_legacy(world.grid.size());
    }
    let fragments: Vec<fracture::Fragment> =
        serde_json_from(value.get("fragments").cloned().unwrap_or(json!([])))
            .map_err(Error::new)?;
    if fragments.len() > world.settings.max_fragments {
        return Err(Error::new("voxel checkpoint exceeds fragment budget"));
    }
    let mut table = BTreeMap::new();
    for mut fragment in fragments {
        if fragment.grid.generator.is_some()
            || fragment.id == 0
            || fragment.id >= 1000000000
            || table.contains_key(&fragment.id)
            || fragment.grid.size.iter().any(|s| *s > 128)
        {
            return Err(Error::new("invalid checkpoint fragment"));
        }
        let fragment_grid =
            Grid::restore(fragment.grid.clone(), world.palette.len()).map_err(Error::new)?;
        if fragment_grid.solid_count() > world.settings.max_fragment_cells as u64 {
            return Err(Error::new("checkpoint fragment cell budget exceeded"));
        }
        fragment.grid = fragment_grid.snapshot();
        fragment.mesh(world)?;
        table.insert(fragment.id, fragment);
    }
    Ok((
        grid,
        table,
        value
            .get("active_seed")
            .and_then(Value::as_i64)
            .unwrap_or(settings.seed),
    ))
}

fn read_checkpoint(host: &Host, world: &World, slot: &str) -> Result<Option<Checkpoint>, Error> {
    let answer = save_service(
        host,
        "save.read",
        &json!({"key":save_key(slot)?, "as":"text"}),
    )?;
    if answer["found"] != true {
        return Ok(None);
    }
    let value = blockloom_plugin_sdk::serde_json::from_str(
        answer["text"]
            .as_str()
            .ok_or_else(|| Error::new("invalid voxel save text"))?,
    )
    .map_err(|e| Error::new(e.to_string()))?;
    decode_checkpoint(world, value).map(Some)
}

fn round(v: f32) -> f64 {
    (f64::from(v) * 1e5).round() / 1e5
}

fn floats(values: &[f32]) -> Vec<f64> {
    values.iter().map(|&v| round(v)).collect()
}

fn chunk_name(chunk: [i32; 3], glow: Option<u8>) -> String {
    let [x, y, z] = chunk;
    match glow {
        None => format!("chunk/{x}/{y}/{z}"),
        Some(material) => format!("chunk/{x}/{y}/{z}/glow{material}"),
    }
}

impl World {
    fn new(settings: &Settings) -> Result<World, String> {
        let mut size = [0i32; 3];
        for (axis, cells) in settings.size.iter().enumerate() {
            let limit = if settings.streamed { 1048576.0 } else { 1024.0 };
            if !(1.0..=limit).contains(cells) {
                return Err(format!(
                    "the world's size must be 1 to 1024 cells, not {cells}"
                ));
            }
            size[axis] = *cells as i32;
        }
        let grid = Grid::new(size);
        let total: i64 = grid.size().iter().map(|&s| i64::from(s)).product();
        if !settings.streamed && total > MAX_CELLS {
            return Err(format!(
                "a {}x{}x{} world holds {total} cells, at most {MAX_CELLS}",
                grid.size()[0],
                grid.size()[1],
                grid.size()[2]
            ));
        }
        if !(0.05..=64.0).contains(&settings.voxel_size) {
            return Err("the voxel size must be 0.05 to 64".to_string());
        }
        if !(0..=8).contains(&settings.stream_radius)
            || !(0..=8).contains(&settings.vertical_radius)
            || !(grid::SECTION_CELLS..=16777216).contains(&settings.max_resident_bytes)
            || !(1..=512).contains(&settings.max_pages)
            || !(1..=16).contains(&settings.pages_per_tick)
            || settings
                .origin
                .iter()
                .chain(&settings.stream_center)
                .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err("invalid voxel residency settings".into());
        }
        if settings.max_fragments > 128
            || !(1..=4096).contains(&settings.max_fragment_cells)
            || settings.palette_density.len() > 255
            || settings
                .palette_density
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 100000.0)
        {
            return Err("invalid voxel fracture budgets or density".into());
        }
        validate_save_slot(&settings.save_slot)?;
        Ok(World {
            grid,
            lod_meshes: lod_mesh::Cache::default(),
            lod_generation: 0,
            palette: Palette::new(&settings.palette_colors, &settings.palette_emission)?,
            voxel: settings.voxel_size as f32,
            origin: settings.origin.map(|v| v as f32),
            solid: settings.solid,
            surface: settings.surface,
            seed: settings.seed,
            published: BTreeMap::new(),
            settings: settings.clone(),
            preview: false,
            serial: 0,
            fragments: BTreeMap::new(),
            next_fragment: 1,
            gpu_buffers: BTreeMap::new(),
            visible: BTreeSet::new(),
            pending: BTreeSet::new(),
            pending_fragments: BTreeMap::new(),
            centre: settings
                .stream_center
                .map(|v| (v as i32).div_euclid(SECTION)),
        })
    }

    fn reset_lod_meshes(&mut self) {
        self.lod_meshes = lod_mesh::Cache::default();
        self.lod_generation = self.lod_generation.wrapping_add(1);
    }

    fn generate(&mut self, preset: &str, seed: i64) -> Result<(), String> {
        if !terrain::PRESETS.contains(&preset) {
            return Err(format!(
                "no terrain preset called {preset} (try {})",
                terrain::PRESETS.join(", ")
            ));
        }
        self.reset_lod_meshes();
        if self.settings.streamed {
            self.grid.stream(preset, seed);
            self.pending.extend(&self.visible);
        } else {
            terrain::generate(&mut self.grid, preset, seed)?;
        }
        self.seed = seed;
        if !self.settings.streamed {
            self.grid.mark_all_dirty();
        }
        Ok(())
    }

    fn invoke(&mut self, centres: &[[i32; 3]]) -> Vec<Value> {
        if !self.settings.streamed {
            return Vec::new();
        }
        let n = self.grid.section_counts();
        let radius = self.settings.stream_radius;
        let mut candidates = BTreeSet::new();
        for &centre in centres {
            for z in -radius..=radius {
                for y in -self.settings.vertical_radius..=self.settings.vertical_radius {
                    for x in -radius..=radius {
                        let page = [centre[0] + x, centre[1] + y, centre[2] + z];
                        if (0..3).all(|a| page[a] >= 0 && page[a] < n[a]) {
                            candidates.insert(page);
                        }
                    }
                }
            }
        }
        let mut candidates: Vec<_> = candidates.into_iter().collect();
        candidates.sort_by_key(|c| {
            (
                centres
                    .iter()
                    .map(|at| {
                        let dx = i64::from(c[0] - at[0]);
                        let dz = i64::from(c[2] - at[2]);
                        dx * dx + dz * dz
                    })
                    .min()
                    .unwrap_or(0),
                centres
                    .iter()
                    .map(|at| (c[1] - at[1]).abs())
                    .min()
                    .unwrap_or(0),
                *c,
            )
        });
        candidates.truncate(
            self.settings
                .max_pages
                .min(self.settings.max_resident_bytes / grid::SECTION_CELLS),
        );
        let next: BTreeSet<_> = candidates.into_iter().collect();
        self.pending.extend(next.difference(&self.visible));
        let mut effects = Vec::new();
        for gone in self.visible.difference(&next) {
            for (buffer, _) in self.gpu_buffers.remove(gone).unwrap_or_default() {
                effects.push(blockloom_plugin_sdk::gpu::free(&buffer));
            }
            for name in self.published.remove(gone).unwrap_or_default() {
                effects.push(json!({"effect":"remove_mesh","name":name}));
            }
        }
        self.visible = next;
        self.grid.retain_pages(&self.visible);
        if let Some(&at) = centres.first() {
            self.centre = at;
        }
        effects
    }

    /// Fills the box between two corners (any order), clipped to the world.
    fn fill_box(&mut self, a: [i32; 3], b: [i32; 3], material: u8) -> Result<u64, String> {
        let size = self.grid.size();
        let lo = [0, 1, 2].map(|i| a[i].min(b[i]).max(0));
        let hi = [0, 1, 2].map(|i| a[i].max(b[i]).min(size[i] - 1));
        check_edit_bounds(lo, hi)?;
        let mut changed = 0;
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    changed += u64::from(self.grid.set([x, y, z], material));
                }
            }
        }
        Ok(changed)
    }

    /// Fills a ball in cell units; smooth worlds combine signed sphere density.
    fn fill_sphere(&mut self, centre: [i32; 3], radius: f64, material: u8) -> Result<u64, String> {
        let size = self.grid.size();
        let reach = radius.ceil() as i32 + i32::from(self.surface == Surface::Smooth);
        let lo = [0, 1, 2].map(|i| (centre[i] - reach).max(0));
        let hi = [0, 1, 2].map(|i| (centre[i] + reach).min(size[i] - 1));
        check_edit_bounds(lo, hi)?;
        let mut changed = 0;
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let d = [x - centre[0], y - centre[1], z - centre[2]]
                        .map(|d| f64::from(d) * f64::from(d));
                    if self.surface == Surface::Smooth {
                        let sphere = ((d.iter().sum::<f64>().sqrt() - radius) * 256.0)
                            .round()
                            .clamp(-256.0, 256.0) as i16;
                        if sphere == 256 {
                            continue;
                        }
                        let cell = [x, y, z];
                        let before = self.grid.get(cell);
                        let old = if self.grid.shape_at(cell) == Shape::Cube {
                            self.grid.density(cell)
                        } else {
                            -256
                        };
                        let density = if material == 0 {
                            old.max(-sphere)
                        } else {
                            old.min(sphere)
                        };
                        let paint = if material != 0 && sphere < 0 {
                            material
                        } else {
                            before
                        };
                        if density == old && paint == before {
                            continue;
                        }
                        changed += u64::from(self.grid.set_density([x, y, z], density, paint));
                    } else if d[0] + d[1] + d[2] <= radius * radius {
                        changed += u64::from(self.grid.set([x, y, z], material));
                    }
                }
            }
        }
        Ok(changed)
    }

    /// Applies one saved edit line: `set X Y Z material`, `fill X1 Y1 Z1 X2
    /// Y2 Z2 material`, `sphere X Y Z radius material` or `shape X Y Z
    /// shape` (which only reshapes a solid cell).
    fn apply_edit(&mut self, line: &str) -> Result<u64, String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (verb, rest) = words.split_first().ok_or("an empty edit")?;
        let cells = |from: usize, count: usize| -> Result<Vec<i32>, String> {
            (from..from + count)
                .map(|i| {
                    rest.get(i)
                        .and_then(|w| w.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && v.abs() < 1e9)
                        .map(|v| v.floor() as i32)
                        .ok_or_else(|| format!("expected a number at word {}", i + 2))
                })
                .collect()
        };
        let material = |at: usize| -> Result<u8, String> {
            let word = rest.get(at).ok_or("the material is missing")?;
            let value = word
                .parse::<i64>()
                .map_or_else(|_| json!(word), |n| json!(n));
            self.palette.id_of(&value)
        };
        match *verb {
            "set" => {
                let c = cells(0, 3)?;
                let m = material(3)?;
                Ok(u64::from(self.grid.set([c[0], c[1], c[2]], m)))
            }
            "fill" => {
                let c = cells(0, 6)?;
                let m = material(6)?;
                self.fill_box([c[0], c[1], c[2]], [c[3], c[4], c[5]], m)
            }
            "sphere" => {
                let c = cells(0, 3)?;
                let radius = rest
                    .get(3)
                    .and_then(|w| w.parse::<f64>().ok())
                    .filter(|r| (0.0..=512.0).contains(r))
                    .ok_or("the radius must be 0 to 512")?;
                let m = material(4)?;
                self.fill_sphere([c[0], c[1], c[2]], radius, m)
            }
            "shape" => {
                let c = cells(0, 3)?;
                let shape = Shape::from_name(&rest.get(3..).unwrap_or_default().join(" "))?;
                Ok(u64::from(self.grid.reshape([c[0], c[1], c[2]], shape)))
            }
            other => Err(format!("unknown edit \"{other}\"")),
        }
    }

    /// Meshes every dirty section and says what the game should now draw.
    fn flush(&mut self) -> Vec<Value> {
        self.lod_meshes.invalidate(self.grid.take_lod_changes());
        let mut effects = Vec::new();
        self.pending.extend(self.grid.take_dirty());
        if self.settings.streamed {
            self.pending.retain(|c| self.visible.contains(c));
        }
        let mut work: Vec<_> = self.pending.iter().copied().collect();
        if self.settings.streamed {
            work.sort_by_key(|c| (distance(*c, self.centre), *c));
            work.truncate(self.settings.pages_per_tick);
        }
        for chunk in work {
            for (buffer, _) in self.gpu_buffers.remove(&chunk).unwrap_or_default() {
                effects.push(blockloom_plugin_sdk::gpu::free(&buffer));
            }
            self.pending.remove(&chunk);
            if self.settings.streamed {
                self.grid.load_page(chunk);
            }
            let mut names = BTreeSet::new();
            for z in 0..SECTION / MESH_TILE {
                for y in 0..SECTION / MESH_TILE {
                    for x in 0..SECTION / MESH_TILE {
                        let tile = [x, y, z];
                        let base = [0, 1, 2].map(|a| chunk[a] * SECTION + tile[a] * MESH_TILE);
                        let extent =
                            [0, 1, 2].map(|a| (self.grid.size()[a] - base[a]).clamp(0, MESH_TILE));
                        if extent.contains(&0) {
                            continue;
                        }
                        let groups = match self.surface {
                            Surface::Cubes => mesher::mesh_region(
                                &self.grid,
                                &self.palette,
                                base,
                                extent,
                                self.voxel,
                            ),
                            Surface::Smooth => smooth::mesh_region(
                                &self.grid,
                                &self.palette,
                                base,
                                extent,
                                self.voxel,
                            ),
                        };
                        let origin =
                            [0, 1, 2].map(|a| self.origin[a] + base[a] as f32 * self.voxel);
                        for (glow, group) in groups {
                            // Keep mesh names stable while storage uses larger sections.
                            let name = chunk_name(base.map(|c| c / MESH_TILE), glow);
                            let emission = glow
                                .and_then(|m| self.palette.get(m))
                                .map(|m| m.color.map(|c| c * m.emission));
                            let gpu =
                                gpu_mesh::build(self, chunk, base, extent, glow, &mut effects);
                            let mesh = MeshData {
                                name: name.clone(),
                                positions: group.positions,
                                normals: group.normals,
                                colors: group.colors,
                                indices: group.indices,
                                origin,
                                emission,
                                roughness: 0.9,
                                collider: self.solid,
                                collider_kind: ColliderKind::Trimesh,
                                gpu,
                                body: None,
                            };
                            let mut effect = serde_value(&mesh);
                            effect["effect"] = json!("mesh");
                            effects.push(effect);
                            names.insert(name);
                        }
                    }
                }
            }
            let before = self.published.remove(&chunk).unwrap_or_default();
            for gone in before.difference(&names) {
                effects.push(json!({"effect": "remove_mesh", "name": gone}));
            }
            if names.is_empty() {
                self.grid.prune(chunk);
            } else {
                self.published.insert(chunk, names);
            }
        }
        let ready: Vec<_> = self
            .pending_fragments
            .iter_mut()
            .filter_map(|(id, (pages, _))| {
                pages.retain(|c| self.pending.contains(c));
                pages.is_empty().then_some(*id)
            })
            .collect();
        for id in ready {
            effects.push(self.pending_fragments.remove(&id).unwrap().1);
        }
        effects
    }
}

/// A mesh as JSON with its numbers short enough to keep a call's answer small.
fn serde_value(mesh: &MeshData) -> Value {
    json!({
        "name": mesh.name,
        "positions": floats(&mesh.positions),
        "normals": floats(&mesh.normals),
        "colors": floats(&mesh.colors),
        "indices": mesh.indices,
        "origin": mesh.origin.map(round),
        "emission": mesh.emission.map(|e| e.map(round)),
        "roughness": mesh.roughness,
        "collider": mesh.collider,
        "collider_kind": mesh.collider_kind,
        "gpu": mesh.gpu,
        "body": mesh.body,
    })
}

#[derive(Default)]
struct Voxel {
    world: Option<World>,
    invokers: Vec<String>,
    lod_generation: u64,
}

fn int(args: &Value, key: &str) -> Result<i32, Error> {
    args[key]
        .as_f64()
        .filter(|v| v.is_finite() && v.abs() < 1e9)
        .map(|v| v.floor() as i32)
        .ok_or_else(|| Error::bad_argument(format!("{key} must be a number")))
}

fn cell(args: &Value) -> Result<[i32; 3], Error> {
    Ok([int(args, "x")?, int(args, "y")?, int(args, "z")?])
}

fn fragment_id(args: &Value) -> Result<u64, Error> {
    let id = number(args, "id")?;
    if !(1.0..1000000000.0).contains(&id) || id.fract() != 0.0 {
        return Err(Error::bad_argument(
            "fragment id must be a positive integer",
        ));
    }
    Ok(id as u64)
}

fn number(args: &Value, key: &str) -> Result<f64, Error> {
    args[key]
        .as_f64()
        .filter(|v| v.is_finite() && v.abs() < 1e9)
        .ok_or_else(|| Error::bad_argument(format!("{key} must be a number")))
}

fn triple(args: &Value, keys: [&str; 3]) -> Result<[f64; 3], Error> {
    Ok([
        number(args, keys[0])?,
        number(args, keys[1])?,
        number(args, keys[2])?,
    ])
}

impl Voxel {
    fn world(&mut self) -> Result<&mut World, Error> {
        self.world
            .as_mut()
            .ok_or_else(|| Error::new("the voxel world starts with the game; press Play first"))
    }

    fn start(&mut self, host: &Host, args: &Value) -> Result<Value, Error> {
        let payload = args["resources"]
            .as_array()
            .and_then(|all| all.iter().find(|r| r["type_id"] == "world"))
            .map(|r| r["payload"].clone())
            .unwrap_or(Value::Null);
        let settings: Settings = if payload.is_null() {
            Settings::default()
        } else {
            serde_json_from(payload).map_err(|e| Error::new(format!("the world resource: {e}")))?
        };
        let mut world = World::new(&settings).map_err(Error::new)?;
        world.lod_generation = self.lod_generation;
        world
            .generate(&settings.preset, settings.seed)
            .map_err(Error::new)?;
        // A bad saved edit is reported and skipped; the rest still apply.
        let mut problems = Vec::new();
        for (n, line) in settings.edits.iter().enumerate() {
            if let Err(why) = world.apply_edit(line) {
                problems.push(json!({
                    "effect": "error",
                    "message": format!("voxel edit {} ({line}): {why}", n + 1),
                }));
            }
        }
        world.preview = args["preview"].as_bool().unwrap_or(false);
        self.invokers = args["records"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter(|r| r["type_id"] == "invoker")
                    .filter_map(|r| r["actor"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if settings.persistent && !world.preview {
            match read_checkpoint(host, &world, &settings.save_slot) {
                Ok(Some((grid, fragments, seed))) => {
                    world.seed = seed;
                    world.grid = grid;
                    world.reset_lod_meshes();
                    world.fragments = fragments;
                    world.next_fragment =
                        world.fragments.keys().next_back().copied().unwrap_or(0) + 1;
                }
                Ok(None) => {}
                Err(why) => {
                    problems.push(json!({"effect":"error", "message":format!("voxel save: {why}")}))
                }
            }
        }
        let mut effects = world.invoke(&[world.centre]);
        effects.extend(world.flush());
        for fragment in world.fragments.values() {
            effects.push(fragment.mesh(&world)?);
        }
        effects.extend(problems);
        let line = format!(
            "voxel world {}x{}x{}, {} sections drawn",
            world.grid.size()[0],
            world.grid.size()[1],
            world.grid.size()[2],
            world.published.len()
        );
        self.world = Some(world);
        effects.push(json!({"effect": "say", "text": line}));
        Ok(json!({ "effects": effects }))
    }

    fn edited(world: &mut World, changed: u64) -> Value {
        let mut effects = world.flush();
        if changed > 0 {
            effects.push(json!({"effect":"nav_dirty"}));
        }
        json!({"changed": changed, "effects":effects})
    }

    fn set(&mut self, args: &Value) -> Result<Value, Error> {
        let at = cell(args)?;
        let world = self.world()?;
        let material = world
            .palette
            .id_of(&args["material"])
            .map_err(Error::bad_argument)?;
        let changed = u64::from(world.grid.set(at, material));
        Ok(Voxel::edited(world, changed))
    }

    fn fill(&mut self, args: &Value) -> Result<Value, Error> {
        let (a, b) = (
            [int(args, "x1")?, int(args, "y1")?, int(args, "z1")?],
            [int(args, "x2")?, int(args, "y2")?, int(args, "z2")?],
        );
        let world = self.world()?;
        let material = world
            .palette
            .id_of(&args["material"])
            .map_err(Error::bad_argument)?;
        let changed = world
            .fill_box(a, b, material)
            .map_err(Error::bad_argument)?;
        Ok(Voxel::edited(world, changed))
    }

    fn shape(&mut self, args: &Value) -> Result<Value, Error> {
        let at = cell(args)?;
        let shape = Shape::from_name(args["shape"].as_str().unwrap_or("cube"))
            .map_err(Error::bad_argument)?;
        let world = self.world()?;
        let changed = u64::from(world.grid.reshape(at, shape));
        Ok(Voxel::edited(world, changed))
    }

    fn sphere(&mut self, args: &Value) -> Result<Value, Error> {
        let centre = cell(args)?;
        let radius = args["radius"]
            .as_f64()
            .filter(|r| (0.0..=512.0).contains(r))
            .ok_or_else(|| Error::bad_argument("radius must be 0 to 512"))?;
        let world = self.world()?;
        let material = world
            .palette
            .id_of(&args["material"])
            .map_err(Error::bad_argument)?;
        let changed = world
            .fill_sphere(centre, radius, material)
            .map_err(Error::bad_argument)?;
        Ok(Voxel::edited(world, changed))
    }

    /// Casts the ray an op's arguments describe, in world units.
    fn cast_ray(&mut self, args: &Value) -> Result<(Option<ray::Hit>, f32), Error> {
        let origin = triple(args, ["x", "y", "z"])?;
        let dir = triple(args, ["dx", "dy", "dz"])?;
        let reach = args
            .get("reach")
            .filter(|r| !r.is_null())
            .map_or(Ok(32.0), |_| number(args, "reach"))?;
        let world = self.world()?;
        let voxel = f64::from(world.voxel);
        let cells = [0, 1, 2].map(|a| ((origin[a] - f64::from(world.origin[a])) / voxel) as f32);
        let cast = if world.surface == Surface::Smooth {
            ray::cast_smooth
        } else {
            ray::cast
        };
        let hit = cast(
            &world.grid,
            cells,
            dir.map(|d| d as f32),
            (reach / voxel) as f32,
        );
        Ok((hit, world.voxel))
    }

    fn cast(&mut self, args: &Value) -> Result<Value, Error> {
        let (hit, voxel) = self.cast_ray(args)?;
        let world = self.world()?;
        Ok(match hit {
            None => json!({"hit": false, "distance": -1.0, "value": -1.0}),
            Some(hit) => {
                let distance = round(hit.distance * voxel);
                // World-space boxes, which a scene tool may outline.
                let boxed = |cell: [i32; 3]| {
                    let min = [0, 1, 2].map(|a| round(world.origin[a] + cell[a] as f32 * voxel));
                    let max =
                        [0, 1, 2].map(|a| round(world.origin[a] + (cell[a] + 1) as f32 * voxel));
                    [min[0], min[1], min[2], max[0], max[1], max[2]]
                };
                json!({
                    "hit": true,
                    "cell": hit.cell,
                    "before": hit.before(),
                    "cell_box": boxed(hit.cell),
                    "before_box": boxed(hit.before()),
                    "normal": hit.normal,
                    "material": world.grid.get(hit.cell),
                    "distance": distance,
                    "value": distance,
                })
            }
        })
    }

    /// Breaks the first solid cell along a ray, or builds on the empty one
    /// in front of it.
    fn dig(&mut self, args: &Value, place: bool) -> Result<Value, Error> {
        let material = if place {
            let world = self.world()?;
            Some(
                world
                    .palette
                    .id_of(&args["material"])
                    .map_err(Error::bad_argument)?,
            )
        } else {
            None
        };
        let (hit, _) = self.cast_ray(args)?;
        let world = self.world()?;
        let target = hit.and_then(|hit| match material {
            None => Some(hit.cell),
            // A ray that starts inside a cell has no face to build against.
            Some(_) => {
                (hit.normal != [0; 3] && world.grid.get(hit.before()) == 0).then(|| hit.before())
            }
        });
        let changed = target.map_or(0, |at| u64::from(world.grid.set(at, material.unwrap_or(0))));
        let mut answer = Voxel::edited(world, changed);
        answer["hit"] = json!(hit.is_some());
        answer["cell"] = json!(target);
        Ok(answer)
    }

    fn regenerate(&mut self, args: &Value) -> Result<Value, Error> {
        let world = self.world()?;
        let preset = args["preset"].as_str().unwrap_or("island");
        let seed = args["seed"].as_i64().unwrap_or(world.seed);
        world.generate(preset, seed).map_err(Error::bad_argument)?;
        let mut answer = Voxel::edited(world, 0);
        answer["effects"]
            .as_array_mut()
            .unwrap()
            .push(json!({"effect":"nav_dirty"}));
        Ok(answer)
    }
}

fn serde_json_from<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    // The SDK re-exports `serde_json` only as a module path, not its `from_value`.
    blockloom_plugin_sdk::serde_json::from_value(value).map_err(|e| e.to_string())
}

impl Plugin for Voxel {
    fn start(host: &Host) -> Result<Self, Error> {
        host.info("voxel started");
        Ok(Voxel::default())
    }

    fn call_json(&mut self, host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        if matches!(
            op,
            "save"
                | "world.save"
                | "world.stop"
                | "fracture"
                | "fragment_set"
                | "fragment_shape"
                | "fragment_sphere"
                | "fragment_cast"
                | "hook.poses"
                | "fragments"
        ) && let Some(world) = &mut self.world
        {
            fracture::poses(world, host);
        }
        let mut answer = match op {
            "fracture" => {
                let a = [int(&args, "x1")?, int(&args, "y1")?, int(&args, "z1")?];
                let b = [int(&args, "x2")?, int(&args, "y2")?, int(&args, "z2")?];
                fracture::detach(self.world()?, a, b)
            }
            "fragment_set" => {
                let id = fragment_id(&args)?;
                let cell = cell(&args)?;
                let world = self.world()?;
                let material = world
                    .palette
                    .id_of(&args["material"])
                    .map_err(Error::bad_argument)?;
                fracture::edit(world, id, |local| {
                    Ok(u64::from(local.grid.set(cell, material)))
                })
            }
            "fragment_shape" => {
                let id = fragment_id(&args)?;
                let cell = cell(&args)?;
                let shape = Shape::from_name(args["shape"].as_str().unwrap_or("cube"))
                    .map_err(Error::bad_argument)?;
                fracture::edit(self.world()?, id, |local| {
                    Ok(u64::from(local.grid.reshape(cell, shape)))
                })
            }
            "fragment_sphere" => {
                let id = fragment_id(&args)?;
                let centre = cell(&args)?;
                let radius = number(&args, "radius")?;
                if !(0.0..=512.0).contains(&radius) {
                    return Err(Error::bad_argument("radius must be 0 to 512"));
                }
                let world = self.world()?;
                let material = world
                    .palette
                    .id_of(&args["material"])
                    .map_err(Error::bad_argument)?;
                fracture::edit(world, id, |local| {
                    local
                        .fill_sphere(centre, radius, material)
                        .map_err(Error::bad_argument)
                })
            }
            "fragment_cast" => {
                let id = fragment_id(&args)?;
                let origin = triple(&args, ["x", "y", "z"])?;
                let direction = triple(&args, ["dx", "dy", "dz"])?;
                let reach = number(&args, "reach")?;
                fracture::cast(self.world()?, id, origin, direction, reach)
            }
            "hook.poses" => {
                let world = self.world()?;
                if world.settings.persistent
                    && !world.preview
                    && !world.fragments.is_empty()
                    && args["tick"].as_u64().is_some_and(|tick| tick % 300 == 0)
                {
                    world.save(host, &world.settings.save_slot)?;
                }
                Ok(Value::Null)
            }
            "fragments" => {
                let world = self.world()?;
                Ok(
                    json!({"fragments":world.fragments.values().map(|f| json!({"id":f.id,"origin":f.origin,"rotation":f.rotation,"velocity":f.velocity,"size":f.grid.size,"cells":Grid::restore(f.grid.clone(),world.palette.len()).map_or(0,|g|g.solid_count())})).collect::<Vec<_>>()}),
                )
            }
            "world.preview" => {
                let world = self.world()?;
                if !world.preview {
                    return Ok(Value::Null);
                }
                Ok(json!({"effects":world.flush()}))
            }
            "world.start" => self.start(host, &args),
            "world.save" => Ok(json!({"state":self.world()?.checkpoint()})),
            "world.restore" => {
                let world = self.world()?;
                let (grid, fragments, seed) = decode_checkpoint(world, args["state"].clone())?;
                let mut effects = world.replace_fragments(fragments)?;
                world.grid = grid;
                world.reset_lod_meshes();
                world.seed = seed;
                world.pending.extend(&world.visible);
                if !world.settings.streamed {
                    world.grid.mark_all_dirty();
                }
                effects.extend(world.flush());
                Ok(json!({"effects":effects}))
            }
            "save" => {
                let world = self.world()?;
                let slot = args["slot"].as_str().unwrap_or(&world.settings.save_slot);
                world.save(host, slot)?;
                Ok(json!({"saved":true}))
            }
            "load" => {
                let world = self.world()?;
                let slot = args["slot"].as_str().unwrap_or(&world.settings.save_slot);
                if world.preview {
                    return Err(Error::new("player saves are unavailable in preview"));
                }
                let (grid, fragments, seed) = read_checkpoint(host, world, slot)?
                    .ok_or_else(|| Error::new("no voxel save in this slot"))?;
                let mut effects = world.replace_fragments(fragments)?;
                world.grid = grid;
                world.reset_lod_meshes();
                world.seed = seed;
                world.pending.extend(&world.visible);
                if !world.settings.streamed {
                    world.grid.mark_all_dirty();
                }
                effects.extend(world.flush());
                Ok(json!({"loaded":true,"effects":effects}))
            }
            "clear_save" => {
                let world = self.world()?;
                if world.preview {
                    return Err(Error::new("player saves are unavailable in preview"));
                }
                let slot = args["slot"].as_str().unwrap_or(&world.settings.save_slot);
                save_service(host, "save.delete", &json!({"key":save_key(slot)?}))?;
                Ok(json!({"cleared":true}))
            }
            "stream" => {
                let world = self.world()?;
                let point = triple(&args, ["x", "y", "z"])?;
                let centre = [0, 1, 2].map(|a| {
                    (((point[a] - f64::from(world.origin[a])) / f64::from(world.voxel)) as i32)
                        .div_euclid(SECTION)
                });
                let mut effects = world.invoke(&[centre]);
                effects.extend(world.flush());
                Ok(
                    json!({"effects":effects,"pending":world.pending.len(),"resident":world.grid.resident_pages()}),
                )
            }
            "hook.stream" => {
                let mut points = Vec::new();
                for actor in &self.invokers {
                    if let Ok(value) = host.call_json("world.actor", &json!({"actor":actor}))
                        && let Ok(point) = serde_json_from::<[f64; 3]>(value["position"].clone())
                    {
                        points.push(point);
                    }
                }
                let world = self.world()?;
                let centres: Vec<_> = points
                    .iter()
                    .map(|point| {
                        [0, 1, 2].map(|a| {
                            (((point[a] - f64::from(world.origin[a])) / f64::from(world.voxel))
                                as i32)
                                .div_euclid(SECTION)
                        })
                    })
                    .collect();
                let mut effects = if centres.is_empty() {
                    Vec::new()
                } else {
                    world.invoke(&centres)
                };
                effects.extend(world.flush());
                Ok(json!({"effects":effects}))
            }
            "world.stop" => {
                let mut effects = Vec::new();
                if let Some(world) = self.world.take() {
                    if world.settings.persistent
                        && !world.preview
                        && let Err(why) = world.save(host, &world.settings.save_slot)
                    {
                        effects.push(json!({"effect":"error","message":format!("voxel stop save failed: {why}")}));
                    }
                    for name in world.published.values().flatten() {
                        effects.push(json!({"effect":"remove_mesh","name":name}));
                    }
                    for id in world.fragments.keys() {
                        effects.push(json!({"effect":"remove_mesh","name":fracture::name(*id)}));
                    }
                    for (buffer, _) in world.gpu_buffers.values().flatten() {
                        effects.push(blockloom_plugin_sdk::gpu::free(buffer));
                    }
                }
                self.invokers.clear();
                Ok(json!({"effects":effects}))
            }
            "set" => self.set(&args),
            "fill" => self.fill(&args),
            "sphere" => self.sphere(&args),
            "shape" => self.shape(&args),
            "generate" => self.regenerate(&args),
            "get" => {
                let at = cell(&args)?;
                let grid = &self.world()?.grid;
                let material = grid.get(at);
                let shape = grid.shape_at(at).name();
                Ok(
                    json!({"material": material, "shape": shape, "density": f64::from(grid.density(at))/256.0, "value": material}),
                )
            }
            "lod_sample" => {
                let at = cell(&args)?;
                let level = int(&args, "level")?;
                if !(0..=i32::from(lod::MAX_LEVEL)).contains(&level) {
                    return Err(Error::new("voxel LOD level must be between 0 and 4"));
                }
                let grid = &mut self.world()?.grid;
                let sample = grid.lod_sample(level as u8, at).map_err(Error::new)?;
                Ok(json!({"level":level,"cell":at,"sample_width":1 << level,
                    "material":sample.material,"opacity":sample.opacity,
                    "density":f64::from(sample.density)/256.0,
                    "density_material":sample.density_material,"children":sample.children,
                    "revision":grid.revision(),"reduction_version":lod::REDUCTION_VERSION}))
            }
            "lod_mesh" => {
                let tile = cell(&args)?;
                let level = int(&args, "level")?;
                if !(1..=i32::from(lod::MAX_LEVEL)).contains(&level) {
                    return Err(Error::bad_argument(
                        "coarse mesh level must be between 1 and 4",
                    ));
                }
                let world = self.world()?;
                let result = world
                    .lod_meshes
                    .poll(
                        lod_mesh::Key {
                            level: level as u8,
                            tile,
                        },
                        &mut world.grid,
                        &world.palette,
                        world.surface,
                        world.voxel,
                    )
                    .map_err(Error::new)?;
                let mut meshes = Vec::new();
                for (glow, group) in result.groups.iter().flatten() {
                    let [x, y, z] = tile;
                    let name = format!(
                        "lod/{level}/{x}/{y}/{z}{}",
                        glow.map_or(String::new(), |m| format!("/glow{m}"))
                    );
                    let mesh = MeshData {
                        name,
                        positions: group.positions.clone(),
                        normals: group.normals.clone(),
                        colors: group.colors.clone(),
                        indices: group.indices.clone(),
                        origin: [0, 1, 2].map(|a| {
                            world.origin[a]
                                + result.base[a] as f32 * (1 << level) as f32 * world.voxel
                        }),
                        emission: glow
                            .and_then(|m| world.palette.get(m))
                            .map(|m| m.color.map(|c| c * m.emission)),
                        roughness: 0.9,
                        collider: false,
                        collider_kind: ColliderKind::Trimesh,
                        gpu: None,
                        body: None,
                    };
                    mesh.check().map_err(Error::new)?;
                    meshes.push(serde_value(&mesh));
                }
                let (dependency_lo, dependency_hi) = result.dependencies();
                Ok(
                    json!({"ready":result.groups.is_some(),"meshes":meshes,"level":level,"tile":tile,
                    "tile_width":lod_mesh::TILE,"base":result.base,"extent":result.extent,
                    "sample_width":1 << level,"samples_ready":result.progress(),"samples_total":result.total(),
                    "base_visit_budget":lod_mesh::BASE_VISITS_PER_POLL,"base_visit_bound":result.visits,
                    "dependency_lo":dependency_lo,"dependency_hi":dependency_hi,
                    "built_revision":result.revision,"revision":world.grid.revision(),
                    "generation":world.lod_generation,"reduction_version":lod::REDUCTION_VERSION}),
                )
            }
            "height" => {
                let (x, z) = (int(&args, "x")?, int(&args, "z")?);
                let top = self.world()?.grid.height_at(x, z).map_or(-1, i64::from);
                Ok(json!({"height": top, "value": top}))
            }
            "cast" => self.cast(&args),
            "break" => self.dig(&args, false),
            "place" => self.dig(&args, true),
            "count" => {
                let world = self.world()?;
                Ok(json!({
                    "solid": world.grid.solid_count(),
                    "chunks": world.published.keys().map(|c| [c[0], c[2]]).collect::<BTreeSet<_>>().len(),
                    "column_counts": world.grid.column_counts(),
                    "resident_columns": world.grid.resident_columns(),
                    "drawn_sections": world.published.len(),
                    "resident_sections": world.grid.resident_pages(),
                    "allocated_bytes": world.grid.allocated_bytes(),
                    "gpu_allocated_bytes": world.gpu_buffers.values().flatten().map(|(_, words)| u64::from(*words) * 4).sum::<u64>(),
                    "lod_nodes": world.grid.lod_nodes(),
                    "lod_samples": world.grid.lod_samples(),
                    "lod_sample_limit": lod::MAX_SAMPLES,
                    "lod_mesh_tiles": world.lod_meshes.len(),
                    "lod_mesh_tile_limit": lod_mesh::MAX_TILES,
                    "lod_generation": world.lod_generation,
                    "voxel_revision": world.grid.revision(),
                    "size": world.grid.size(),
                    "resident": world.grid.resident_pages(),
                    "pending": world.pending.len(),
                }))
            }
            _ => Err(Error::unsupported(op)),
        }?;
        self.lod_generation = self
            .world
            .as_ref()
            .map_or(self.lod_generation, |w| w.lod_generation);
        if matches!(
            op,
            "set"
                | "fill"
                | "sphere"
                | "shape"
                | "generate"
                | "break"
                | "place"
                | "fracture"
                | "fragment_set"
                | "fragment_shape"
                | "fragment_sphere"
        ) && let Some(world) = &self.world
            && world.settings.persistent
            && !world.preview
            && let Err(why) = world.save(host, &world.settings.save_slot)
        {
            answer["effects"]
                .as_array_mut()
                .unwrap()
                .push(json!({"effect":"error","message":format!("voxel autosave failed: {why}")}));
        }
        Ok(answer)
    }
}

export_plugin!(Voxel);
