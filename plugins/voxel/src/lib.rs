//! Finite cube or smooth voxel worlds with generated terrain and live edits.
//!
//! The world's settings are the plugin's `world` resource. When a running
//! game hosts the module it hears `world.start`, builds the world from that
//! resource and answers with a `mesh` effect per chunk; every edit after that
//! answers with the meshes it changed (and `remove_mesh` for ones that went
//! away), so the game only ever redraws touched chunks. A chunk's cells are
//! the authority, its meshes are disposable.
//!
//! Ops: `world.start`, `world.stop`, `set`, `fill`, `sphere`, `shape`, `generate`,
//! `get`, `height`, `count`, `cast`, `break`, `place`. Cell coordinates are
//! whole numbers from one corner of the world; a material is its id or its
//! name (`air` or 0 clears). A solid cell may be a slab, top slab or post
//! instead of a whole cube (`shape`). Cube-world rays use occupied cells;
//! smooth-world rays intersect the surface. A ray (`cast`, `break`, `place`) is given in
//! world units, like the cubes are drawn.
//!
//! Edits made by blocks last as long as the run: stopping the game starts the
//! next from the generated world again. Edits the project saves are the
//! `world` resource's `edits` lines, applied in order on top of the terrain.

mod grid;
mod mesher;
mod palette;
mod ray;
mod shape;
mod smooth;
mod terrain;

use blockloom_plugin_api::mesh::{ColliderKind, MeshData};
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};
use grid::{CHUNK, Grid, Shape};
use palette::Palette;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// The most cells a world may hold: generation and meshing both cost this.
const MAX_CELLS: i64 = 4_194_304;

/// The `world` resource, with what an absent field reads as.
#[derive(Deserialize, Clone)]
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
        }
    }
}

#[derive(Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Surface {
    #[default]
    Cubes,
    Smooth,
}

struct World {
    grid: Grid,
    palette: Palette,
    voxel: f32,
    origin: [f32; 3],
    solid: bool,
    surface: Surface,
    seed: i64,
    /// The mesh names each chunk has drawn, so a remesh can retire the ones
    /// it no longer makes.
    published: BTreeMap<[i32; 3], BTreeSet<String>>,
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
            if !(1.0..=1024.0).contains(cells) {
                return Err(format!(
                    "the world's size must be 1 to 1024 cells, not {cells}"
                ));
            }
            size[axis] = *cells as i32;
        }
        let grid = Grid::new(size);
        let total: i64 = grid.size().iter().map(|&s| i64::from(s)).product();
        if total > MAX_CELLS {
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
        Ok(World {
            grid,
            palette: Palette::new(&settings.palette_colors, &settings.palette_emission)?,
            voxel: settings.voxel_size as f32,
            origin: settings.origin.map(|v| v as f32),
            solid: settings.solid,
            surface: settings.surface,
            seed: settings.seed,
            published: BTreeMap::new(),
        })
    }

    fn generate(&mut self, preset: &str, seed: i64) -> Result<(), String> {
        terrain::generate(&mut self.grid, preset, seed)?;
        self.seed = seed;
        self.grid.mark_all_dirty();
        Ok(())
    }

    /// Fills the box between two corners (any order), clipped to the world.
    fn fill_box(&mut self, a: [i32; 3], b: [i32; 3], material: u8) -> u64 {
        let size = self.grid.size();
        let lo = [0, 1, 2].map(|i| a[i].min(b[i]).max(0));
        let hi = [0, 1, 2].map(|i| a[i].max(b[i]).min(size[i] - 1));
        let mut changed = 0;
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    changed += u64::from(self.grid.set([x, y, z], material));
                }
            }
        }
        changed
    }

    /// Fills a ball in cell units; smooth worlds combine signed sphere density.
    fn fill_sphere(&mut self, centre: [i32; 3], radius: f64, material: u8) -> u64 {
        let size = self.grid.size();
        let reach = radius.ceil() as i32 + i32::from(self.surface == Surface::Smooth);
        let lo = [0, 1, 2].map(|i| (centre[i] - reach).max(0));
        let hi = [0, 1, 2].map(|i| (centre[i] + reach).min(size[i] - 1));
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
        changed
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
                Ok(self.fill_box([c[0], c[1], c[2]], [c[3], c[4], c[5]], m))
            }
            "sphere" => {
                let c = cells(0, 3)?;
                let radius = rest
                    .get(3)
                    .and_then(|w| w.parse::<f64>().ok())
                    .filter(|r| (0.0..=512.0).contains(r))
                    .ok_or("the radius must be 0 to 512")?;
                let m = material(4)?;
                Ok(self.fill_sphere([c[0], c[1], c[2]], radius, m))
            }
            "shape" => {
                let c = cells(0, 3)?;
                let shape = Shape::from_name(&rest.get(3..).unwrap_or_default().join(" "))?;
                Ok(u64::from(self.grid.reshape([c[0], c[1], c[2]], shape)))
            }
            other => Err(format!("unknown edit \"{other}\"")),
        }
    }

    /// Meshes every dirty chunk and says what the game should now draw.
    fn flush(&mut self) -> Vec<Value> {
        let mut effects = Vec::new();
        for chunk in self.grid.take_dirty() {
            let groups = match self.surface {
                Surface::Cubes => mesher::mesh_chunk(&self.grid, &self.palette, chunk, self.voxel),
                Surface::Smooth => smooth::mesh_chunk(&self.grid, &self.palette, chunk, self.voxel),
            };
            let origin = [0, 1, 2].map(|a| self.origin[a] + (chunk[a] * CHUNK) as f32 * self.voxel);
            let mut names = BTreeSet::new();
            for (glow, group) in groups {
                let name = chunk_name(chunk, glow);
                let emission = glow.and_then(|m| self.palette.get(m)).map(|m| {
                    [
                        m.color[0] * m.emission,
                        m.color[1] * m.emission,
                        m.color[2] * m.emission,
                    ]
                });
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
                };
                let mut effect = serde_value(&mesh);
                effect["effect"] = json!("mesh");
                effects.push(effect);
                names.insert(name);
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
    })
}

#[derive(Default)]
struct Voxel {
    world: Option<World>,
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

    fn start(&mut self, args: &Value) -> Result<Value, Error> {
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
        let mut effects = world.flush();
        effects.extend(problems);
        let line = format!(
            "voxel world {}x{}x{}, {} chunks drawn",
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
        json!({"changed": changed, "effects": world.flush()})
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
        let changed = world.fill_box(a, b, material);
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
        let changed = world.fill_sphere(centre, radius, material);
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
        Ok(Voxel::edited(world, 0))
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

    fn call_json(&mut self, _host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "world.start" => self.start(&args),
            "world.stop" => {
                self.world = None;
                Ok(Value::Null)
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
                    "chunks": world.published.len(),
                    "size": world.grid.size(),
                }))
            }
            _ => Err(Error::unsupported(op)),
        }
    }
}

export_plugin!(Voxel);
