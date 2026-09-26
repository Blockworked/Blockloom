//! Light probes: boxes that remember the light around where they stand.
//!
//! A reflection probe bakes one cubemap that surfaces inside its box reflect,
//! box-projected so a reflection lines up with the room rather than sitting
//! at infinity. An irradiance probe bakes a grid of bricks, one ambient cube
//! each (six colors, one per axis direction), that dynamic actors inside are
//! lit by. Both bake from the runtime's own capture service and land under
//! `.blockloom/probes/`, stamped with what the scene looked like, so the
//! editor knows when a move has left one stale.

use crate::pipeline::hdr::HdrCube;
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProbeKind {
    /// A box-projected cubemap: sharp reflections for what is inside.
    #[default]
    Reflection,
    /// A brick grid of ambient cubes: bounced diffuse light.
    Irradiance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeSpec {
    #[serde(default)]
    pub kind: ProbeKind,
    /// The box in metres, turned with the actor but not scaled by it.
    #[serde(default = "default_size")]
    pub size: [f32; 3],
    /// How much of the box, from its faces inwards, fades the probe out:
    /// 0 is a hard edge, 1 fades all the way to the centre. Overlapping
    /// probes blend through this.
    #[serde(default = "default_falloff")]
    pub falloff: f32,
    /// Texels per cube face, a power of two. Reflection probes only.
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    /// Bricks along each axis. Irradiance probes only.
    #[serde(default = "default_grid")]
    pub grid: [u32; 3],
    /// Scales what the probe gives back.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Reflections line up with the box's walls rather than sitting at
    /// infinity. Right for rooms, wrong for an open sky.
    #[serde(default = "default_true")]
    pub box_projection: bool,
    /// Rebakes by itself in the scene view once the probe or the scene
    /// around it has changed since the last bake.
    #[serde(default = "default_true")]
    pub auto_bake: bool,
}

fn default_size() -> [f32; 3] {
    [10.0, 5.0, 10.0]
}

fn default_falloff() -> f32 {
    0.2
}

fn default_resolution() -> u32 {
    256
}

fn default_grid() -> [u32; 3] {
    [4, 3, 4]
}

fn default_intensity() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

impl Default for ProbeSpec {
    fn default() -> Self {
        Self {
            kind: ProbeKind::Reflection,
            size: default_size(),
            falloff: default_falloff(),
            resolution: default_resolution(),
            grid: default_grid(),
            intensity: default_intensity(),
            box_projection: true,
            auto_bake: true,
        }
    }
}

/// Most bricks along one axis; a bake captures a cube per brick.
pub const MAX_GRID: u32 = 16;

impl ProbeSpec {
    /// The face size a reflection bake renders at: a power of two, which the
    /// GPU's environment filter insists on.
    pub fn face_resolution(&self) -> u32 {
        self.resolution.clamp(32, 1024).next_power_of_two()
    }

    pub fn bricks(&self) -> [u32; 3] {
        self.grid.map(|n| n.clamp(1, MAX_GRID))
    }

    pub fn half_extents(&self) -> [f32; 3] {
        self.size.map(|side| side.abs().max(0.01) / 2.0)
    }

    /// Where brick `(x, y, z)` samples, in the box's own frame (-0.5..0.5 on
    /// each axis): at the centre of its cell.
    pub fn brick_point(&self, x: u32, y: u32, z: u32) -> [f32; 3] {
        let [nx, ny, nz] = self.bricks();
        let at = |i: u32, n: u32| (i as f32 + 0.5) / n as f32 - 0.5;
        [at(x, nx), at(y, ny), at(z, nz)]
    }
}

// ─── Bakes on disk ─────────────────────────────────────────────────────────

pub const PROBE_DIR: &str = ".blockloom/probes";

/// What a bake was made from, beside its data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BakeInfo {
    pub kind: ProbeKind,
    pub stamp: u64,
}

pub fn info_path(dir: &Path, actor: &str) -> PathBuf {
    dir.join(PROBE_DIR)
        .join(format!("{}.json", file_stem(actor)))
}

/// A reflection bake: a BC6H cube, the format the sky ships in.
pub fn cube_path(dir: &Path, actor: &str) -> PathBuf {
    dir.join(PROBE_DIR)
        .join(format!("{}.dds", file_stem(actor)))
}

pub fn grid_path(dir: &Path, actor: &str) -> PathBuf {
    dir.join(PROBE_DIR)
        .join(format!("{}.irr", file_stem(actor)))
}

/// Actor ids are uuids already, but a clone's `~1` or anything odd a document
/// holds shouldn't reach the file system as is.
fn file_stem(actor: &str) -> String {
    actor
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn read_info(dir: &Path, actor: &str) -> Option<BakeInfo> {
    let text = std::fs::read_to_string(info_path(dir, actor)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a reflection bake: the cube in scene radiance, and its stamp.
pub fn write_cube(dir: &Path, actor: &str, cube: &HdrCube, stamp: u64) -> Result<(), String> {
    write_file(
        &cube_path(dir, actor),
        &crate::pipeline::bc6h::write_dds_cube(cube),
    )?;
    write_info(dir, actor, ProbeKind::Reflection, stamp)
}

/// Writes an irradiance bake: every brick's ambient cube, and its stamp.
pub fn write_grid(
    dir: &Path,
    actor: &str,
    grid: &IrradianceGrid,
    stamp: u64,
) -> Result<(), String> {
    write_file(&grid_path(dir, actor), &grid.to_bytes())?;
    write_info(dir, actor, ProbeKind::Irradiance, stamp)
}

fn write_info(dir: &Path, actor: &str, kind: ProbeKind, stamp: u64) -> Result<(), String> {
    let info = serde_json::to_string(&BakeInfo { kind, stamp }).map_err(|e| e.to_string())?;
    write_file(&info_path(dir, actor), info.as_bytes())
}

// ─── Ambient cubes ─────────────────────────────────────────────────────────

/// The six directions an ambient cube stores, in this order.
pub const CUBE_AXES: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

/// One brick: the light arriving from around each axis direction.
pub type AmbientCube = [[f32; 3]; 6];

/// Integrates a captured cube into an ambient cube: for each axis, the
/// cosine-weighted mean radiance over the hemisphere around it. That is the
/// irradiance over pi, which is what Bevy multiplies by a surface's diffuse
/// color, exactly as it does its environment maps' diffuse half.
pub fn ambient_cube(faces: &[Vec<[f32; 3]>; 6], size: u32) -> AmbientCube {
    let mut sums = [[0.0f64; 3]; 6];
    let mut weights = [0.0f64; 6];
    for (face, texels) in faces.iter().enumerate() {
        for y in 0..size {
            for x in 0..size {
                let dir = HdrCube::direction(face, x, y, size);
                let omega = texel_solid_angle(x, y, size);
                let radiance = texels[(y * size + x) as usize];
                for (axis, normal) in CUBE_AXES.iter().enumerate() {
                    let cos = dir[0] * normal[0] + dir[1] * normal[1] + dir[2] * normal[2];
                    if cos <= 0.0 {
                        continue;
                    }
                    let w = (cos * omega) as f64;
                    for c in 0..3 {
                        sums[axis][c] += radiance[c] as f64 * w;
                    }
                    weights[axis] += w;
                }
            }
        }
    }
    std::array::from_fn(|axis| {
        let w = weights[axis].max(f64::EPSILON);
        sums[axis].map(|sum| (sum / w) as f32)
    })
}

/// The solid angle one cube face texel covers (the standard area-element
/// integral over its corners).
fn texel_solid_angle(x: u32, y: u32, size: u32) -> f32 {
    let inv = 1.0 / size as f32;
    let u = 2.0 * (x as f32 + 0.5) * inv - 1.0;
    let v = 2.0 * (y as f32 + 0.5) * inv - 1.0;
    let (x0, y0, x1, y1) = (u - inv, v - inv, u + inv, v + inv);
    let area = |x: f32, y: f32| (x * y).atan2((x * x + y * y + 1.0).sqrt());
    area(x0, y0) - area(x0, y1) - area(x1, y0) + area(x1, y1)
}

/// A baked brick grid, x fastest, then y, then z.
#[derive(Debug, Clone, PartialEq)]
pub struct IrradianceGrid {
    pub bricks: [u32; 3],
    pub cubes: Vec<AmbientCube>,
}

const GRID_MAGIC: &[u8; 4] = b"BLIV";
const GRID_VERSION: u32 = 1;

impl IrradianceGrid {
    pub fn index(&self, x: u32, y: u32, z: u32) -> usize {
        let [nx, ny, _] = self.bricks;
        (x + nx * (y + ny * z)) as usize
    }

    /// Bevy's irradiance volume layout: a 3D texture `(Rx, 2Ry, 3Rz)` whose
    /// upper half along y holds the positive directions and lower half the
    /// negative ones, with x, y and z slabs stacked along depth. RGBA, one
    /// float per channel, texel after texel.
    pub fn atlas(&self) -> ([u32; 3], Vec<[f32; 4]>) {
        let [nx, ny, nz] = self.bricks;
        let size = [nx, ny * 2, nz * 3];
        let mut texels = vec![[0.0, 0.0, 0.0, 1.0]; (size[0] * size[1] * size[2]) as usize];
        for z in 0..nz {
            for y in 0..ny {
                for x in 0..nx {
                    let cube = &self.cubes[self.index(x, y, z)];
                    for (axis, rgb) in cube.iter().enumerate() {
                        // Axis pairs: +X, -X, +Y, -Y, +Z, -Z.
                        let slab = (axis / 2) as u32;
                        let negative = axis % 2 == 1;
                        let t = y + if negative { ny } else { 0 };
                        let p = z + slab * nz;
                        let at = (x + size[0] * (t + size[1] * p)) as usize;
                        texels[at] = [rgb[0], rgb[1], rgb[2], 1.0];
                    }
                }
            }
        }
        (size, texels)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.cubes.len() * 6 * 3 * 4);
        out.extend_from_slice(GRID_MAGIC);
        out.extend_from_slice(&GRID_VERSION.to_le_bytes());
        for n in self.bricks {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for value in self.cubes.iter().flatten().flatten() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<IrradianceGrid, String> {
        let word = |at: usize| -> Result<u32, String> {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| "the irradiance bake is truncated".to_string())
        };
        if bytes.get(..4) != Some(GRID_MAGIC) {
            return Err("not an irradiance bake".to_string());
        }
        if word(4)? != GRID_VERSION {
            return Err("the irradiance bake is from another version; bake it again".to_string());
        }
        let bricks = [word(8)?, word(12)?, word(16)?];
        if bricks.iter().any(|n| *n == 0 || *n > MAX_GRID) {
            return Err(format!("{bricks:?} isn't a brick grid"));
        }
        let count = (bricks[0] * bricks[1] * bricks[2]) as usize;
        let floats = bytes
            .get(20..20 + count * 18 * 4)
            .ok_or_else(|| "the irradiance bake is truncated".to_string())?;
        let values: Vec<f32> = floats
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let cubes = values
            .as_chunks::<18>()
            .0
            .iter()
            .map(|cube| std::array::from_fn(|axis| std::array::from_fn(|c| cube[axis * 3 + c])))
            .collect();
        Ok(IrradianceGrid { bricks, cubes })
    }

    pub fn read(dir: &Path, actor: &str) -> Result<IrradianceGrid, String> {
        let path = grid_path(dir, actor);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        IrradianceGrid::from_bytes(&bytes)
    }
}

// ─── Staleness ─────────────────────────────────────────────────────────────

/// The components that change what a probe sees. Blocks, scripts and the
/// like don't, and custom components carry maps with no stable order.
const LIT_BY: &[&str] = &["Place", "Look", "Render", "Material", "Light", "Parent"];

/// What a probe's bake depends on: its own placement and settings, plus
/// every lit thing around it and the world's light. A bake whose stamp no
/// longer matches is stale.
pub fn stamp(project: &Project, actor: &str) -> Option<u64> {
    let probe = project.actors.iter().find(|a| a.id == actor)?;
    let mut hash = Fnv::new();
    hash.json(&probe.placement());
    hash.json(probe.components.probe()?);
    hash.json(&probe.components.get("Parent"));
    for other in &project.actors {
        hash.write(other.id.as_bytes());
        for name in LIT_BY {
            hash.json(&other.components.get(name));
        }
    }
    hash.json(&project.world.lighting);
    hash.write(project.world.background.as_bytes());
    Some(hash.0)
}

/// Where a probe's bake stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeStatus {
    pub actor: String,
    pub name: String,
    pub kind: ProbeKind,
    /// A bake of the right kind is on disk.
    pub baked: bool,
    /// The probe, or the scene it sees, changed since that bake.
    pub dirty: bool,
}

pub fn statuses(project: &Project, dir: &Path) -> Vec<ProbeStatus> {
    project
        .actors
        .iter()
        .filter_map(|actor| {
            let spec = actor.components.probe()?;
            let info = read_info(dir, &actor.id).filter(|info| info.kind == spec.kind);
            let now = stamp(project, &actor.id);
            Some(ProbeStatus {
                actor: actor.id.clone(),
                name: actor.name.clone(),
                kind: spec.kind,
                baked: info.is_some(),
                dirty: info.is_none_or(|info| Some(info.stamp) != now),
            })
        })
        .collect()
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        // A separator, so ("ab", "c") and ("a", "bc") differ.
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3) ^ 0xff;
    }

    fn json<T: Serialize>(&mut self, value: &T) {
        self.write(serde_json::to_string(value).unwrap_or_default().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ActorComponent;
    use crate::scene::Mode;

    fn uniform_cube(size: u32, rgb: [f32; 3]) -> [Vec<[f32; 3]>; 6] {
        std::array::from_fn(|_| vec![rgb; (size * size) as usize])
    }

    #[test]
    fn uniform_light_gives_the_same_ambient_cube_every_way() {
        let cube = ambient_cube(&uniform_cube(8, [2.0, 1.0, 0.5]), 8);
        for side in cube {
            for (got, want) in side.iter().zip([2.0, 1.0, 0.5]) {
                assert!((got - want).abs() < 1e-4, "{side:?}");
            }
        }
    }

    #[test]
    fn a_bright_ceiling_lights_the_up_side_most() {
        let mut faces = uniform_cube(8, [0.0; 3]);
        // +Y is face 2.
        faces[2] = vec![[1.0; 3]; 64];
        let cube = ambient_cube(&faces, 8);
        let up = cube[2][0];
        let side = cube[0][0];
        assert!(up > 0.5, "{up}");
        assert!(side > 0.0 && side < up, "{side}");
        assert_eq!(cube[3][0], 0.0);
    }

    #[test]
    fn texel_solid_angles_cover_the_sphere() {
        let size = 16;
        let total: f32 = (0..size)
            .flat_map(|y| (0..size).map(move |x| texel_solid_angle(x, y, size)))
            .sum::<f32>()
            * 6.0;
        assert!((total - 4.0 * std::f32::consts::PI).abs() < 1e-3, "{total}");
    }

    #[test]
    fn the_atlas_puts_each_side_where_bevy_samples_it() {
        let grid = IrradianceGrid {
            bricks: [2, 1, 1],
            cubes: vec![
                std::array::from_fn(|axis| [axis as f32, 0.0, 0.0]),
                std::array::from_fn(|axis| [10.0 + axis as f32, 0.0, 0.0]),
            ],
        };
        let (size, texels) = grid.atlas();
        assert_eq!(size, [2, 2, 3]);
        let at = |x: u32, t: u32, p: u32| texels[(x + 2 * (t + 2 * p)) as usize][0];
        // +X upper half of slab 0, -X lower half.
        assert_eq!(at(0, 0, 0), 0.0);
        assert_eq!(at(0, 1, 0), 1.0);
        // -Z of the second brick: lower half of slab 2.
        assert_eq!(at(1, 1, 2), 15.0);
        assert_eq!(at(1, 0, 1), 12.0);
    }

    #[test]
    fn a_grid_survives_the_disk() {
        let grid = IrradianceGrid {
            bricks: [1, 2, 1],
            cubes: vec![[[0.25; 3]; 6], [[4.0, 5.0, 6.0]; 6]],
        };
        assert_eq!(IrradianceGrid::from_bytes(&grid.to_bytes()).unwrap(), grid);
        assert!(IrradianceGrid::from_bytes(b"nope").is_err());
    }

    #[test]
    fn bricks_sample_their_cell_centres() {
        let spec = ProbeSpec {
            grid: [2, 1, 4],
            ..ProbeSpec::default()
        };
        assert_eq!(spec.brick_point(0, 0, 0), [-0.25, 0.0, -0.375]);
        assert_eq!(spec.brick_point(1, 0, 3), [0.25, 0.0, 0.375]);
        assert_eq!(
            ProbeSpec {
                resolution: 200,
                ..spec
            }
            .face_resolution(),
            256
        );
    }

    fn project_with_probe() -> (Project, String) {
        let mut project = Project::starter("Probes", Mode::ThreeD);
        let actor = &mut project.actors[0];
        actor.components.insert(ActorComponent::Probe {
            probe: ProbeSpec::default(),
        });
        let id = actor.id.clone();
        (project, id)
    }

    #[test]
    fn moving_anything_lit_stales_the_stamp_and_blocks_dont() {
        let (mut project, id) = project_with_probe();
        let before = stamp(&project, &id).unwrap();
        project.actors[0].components.placement_mut().position[0] += 1.0;
        let moved = stamp(&project, &id).unwrap();
        assert_ne!(before, moved);
        project.world.lighting.illuminance *= 2.0;
        assert_ne!(stamp(&project, &id).unwrap(), moved);
        assert!(stamp(&project, "nobody").is_none());
    }

    #[test]
    fn status_reads_the_bake_on_disk() {
        let dir = std::env::temp_dir().join(format!("blockloom-probe-{}", std::process::id()));
        let (mut project, id) = project_with_probe();
        let status = statuses(&project, &dir);
        assert_eq!((status[0].baked, status[0].dirty), (false, true));

        let cube = HdrCube {
            size: 4,
            faces: std::array::from_fn(|_| vec![[1.0; 3]; 16]),
        };
        write_cube(&dir, &id, &cube, stamp(&project, &id).unwrap()).unwrap();
        let status = statuses(&project, &dir);
        assert_eq!((status[0].baked, status[0].dirty), (true, false));

        project.actors[0].components.placement_mut().position[1] += 1.0;
        assert!(statuses(&project, &dir)[0].dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
