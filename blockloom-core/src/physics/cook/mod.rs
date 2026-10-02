//! Collision cooking: a model file turned into what a backend builds a collider
//! from, and the cache and lookup that carry it from the editor into a build.
//!
//! The pipeline is load ([`load`]), clean ([`RawMesh::clean`]: weld, drop
//! degenerate triangles, validate), then either a convex hull ([`hull`], capped
//! at a vertex limit with the leftover error reported), the mesh as it is, or an
//! approximate convex decomposition ([`decompose`]). Output is deterministic, so a
//! cooked file is reproducible from its key: the source bytes, the settings, the
//! cooker version and the target format.

pub mod decompose;
pub mod hull;
pub mod load;
pub mod lookup;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use decompose::Decompose;
pub use lookup::{
    CollisionLookup, CookReport, CookedEntry, FolderCollision, NoCollisionData, Source,
    cook_project, request_id,
};

/// Bump when cooking output would change for the same input.
pub const COOKER_VERSION: u32 = 1;
/// The layout cooked data is stored in, part of every cache key.
pub const TARGET_FORMAT: &str = "f32-v1";
/// The most triangles a collision mesh may keep.
pub const MAX_TRIANGLES: usize = 2_000_000;
/// The most points one convex hull may keep (PhysX's limit, and Unity's).
pub const MAX_HULL_VERTICES: usize = 255;

/// What a collider wants from a mesh file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MeshKind {
    /// One convex hull of every vertex.
    Hull,
    /// The triangles as they are (scenery).
    Triangles,
    /// Several convex hulls approximating a concave mesh (dynamic bodies).
    Decomposed,
}

impl MeshKind {
    pub fn name(self) -> &'static str {
        match self {
            MeshKind::Hull => "hull",
            MeshKind::Triangles => "triangles",
            MeshKind::Decomposed => "decomposed",
        }
    }
}

/// Project-wide cooking choices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CookSettings {
    /// Vertices closer than this (in the model's own units) become one.
    #[serde(default = "default_weld")]
    pub weld: f32,
    /// The most points a single hull keeps.
    #[serde(default = "default_hull_vertices")]
    pub max_hull_vertices: u16,
    /// Mesh paths that cook to several hulls instead of one, with their limits.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub decompose: std::collections::BTreeMap<String, Decompose>,
}

fn default_weld() -> f32 {
    1e-5
}

fn default_hull_vertices() -> u16 {
    MAX_HULL_VERTICES as u16
}

impl Default for CookSettings {
    fn default() -> Self {
        Self {
            weld: default_weld(),
            max_hull_vertices: default_hull_vertices(),
            decompose: Default::default(),
        }
    }
}

impl CookSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The decomposition a mesh asks for, if any.
    pub fn decomposition_of(&self, mesh: &str) -> Option<&Decompose> {
        self.decompose.get(mesh)
    }

    /// Why these settings can't cook, if they can't.
    pub fn check(&self) -> Result<(), String> {
        if !self.weld.is_finite() || self.weld < 0.0 {
            return Err("The weld distance must be zero or more".into());
        }
        if !(4..=MAX_HULL_VERTICES as u16).contains(&self.max_hull_vertices) {
            return Err(format!(
                "A hull keeps between 4 and {MAX_HULL_VERTICES} points"
            ));
        }
        for (mesh, d) in &self.decompose {
            d.check().map_err(|why| format!("{mesh}: {why}"))?;
        }
        Ok(())
    }
}

/// Triangles, as a backend wants to be handed them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawMesh {
    pub positions: Vec<[f32; 3]>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
}

/// What cooking did, for the inspector and the cook report.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CookStats {
    pub input_vertices: usize,
    pub input_triangles: usize,
    pub vertices: usize,
    pub triangles: usize,
    /// Vertices merged into another.
    pub welded: usize,
    /// Triangles dropped for having no area or repeating a corner.
    pub degenerate: usize,
    pub hulls: usize,
    /// The worst distance (model units) a source point sits outside the cooked
    /// shape. Zero for the exact triangle mesh.
    pub error: f32,
    /// What the cooked data weighs in memory.
    pub bytes: usize,
}

/// Cooked collision data in the model's own units.
#[derive(Debug, Clone, PartialEq)]
pub enum Cooked {
    Hull {
        points: Vec<[f32; 3]>,
        volume: f32,
        stats: CookStats,
    },
    Triangles {
        vertices: Vec<[f32; 3]>,
        indices: Vec<u32>,
        stats: CookStats,
    },
    Decomposed {
        hulls: Vec<Vec<[f32; 3]>>,
        volume: f32,
        stats: CookStats,
    },
}

impl Cooked {
    pub fn kind(&self) -> MeshKind {
        match self {
            Cooked::Hull { .. } => MeshKind::Hull,
            Cooked::Triangles { .. } => MeshKind::Triangles,
            Cooked::Decomposed { .. } => MeshKind::Decomposed,
        }
    }

    pub fn stats(&self) -> &CookStats {
        match self {
            Cooked::Hull { stats, .. }
            | Cooked::Triangles { stats, .. }
            | Cooked::Decomposed { stats, .. } => stats,
        }
    }

    /// The enclosed volume at scale 1; zero for a triangle mesh.
    pub fn volume(&self) -> f32 {
        match self {
            Cooked::Hull { volume, .. } | Cooked::Decomposed { volume, .. } => *volume,
            Cooked::Triangles { .. } => 0.0,
        }
    }

    /// Every point the data holds, for bounds and fitting.
    pub fn points(&self) -> Box<dyn Iterator<Item = &[f32; 3]> + '_> {
        match self {
            Cooked::Hull { points, .. } => Box::new(points.iter()),
            Cooked::Triangles { vertices, .. } => Box::new(vertices.iter()),
            Cooked::Decomposed { hulls, .. } => Box::new(hulls.iter().flatten()),
        }
    }

    /// The smallest box holding it: `(min, max)`.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in self.points() {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        (lo, hi)
    }
}

/// Why cooking didn't produce anything.
#[derive(Debug, Clone, PartialEq)]
pub enum CookError {
    /// The user stopped it.
    Cancelled,
    Message(String),
}

impl std::fmt::Display for CookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookError::Cancelled => f.write_str("Cooking was cancelled"),
            CookError::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CookError {}

impl From<String> for CookError {
    fn from(message: String) -> Self {
        CookError::Message(message)
    }
}

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, CookError> {
    Err(CookError::Message(message.into()))
}

/// Cancellation and progress for a long cook.
#[derive(Clone, Default)]
pub struct CookControl {
    cancel: Arc<AtomicBool>,
    #[allow(clippy::type_complexity)]
    progress: Option<Arc<dyn Fn(&str, f32) + Send + Sync>>,
}

impl CookControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports `(what, fraction)` as cooking goes.
    pub fn with_progress(mut self, report: impl Fn(&str, f32) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(report));
        self
    }

    /// A handle another thread flips to stop the cook.
    pub fn canceller(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn check(&self) -> Result<(), CookError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(CookError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn report(&self, what: &str, fraction: f32) {
        if let Some(report) = &self.progress {
            report(what, fraction.clamp(0.0, 1.0));
        }
    }
}

impl RawMesh {
    /// Welds close vertices, drops degenerate triangles and checks the result.
    /// Returns the cleaned mesh with the counts so far in the stats.
    pub fn clean(&self, weld: f32) -> Result<(RawMesh, CookStats), CookError> {
        let mut stats = CookStats {
            input_vertices: self.positions.len(),
            input_triangles: self.indices.len() / 3,
            ..Default::default()
        };
        if self.indices.len() % 3 != 0 {
            return fail("The index list is not a whole number of triangles");
        }
        if self.positions.is_empty() || self.indices.is_empty() {
            return fail("The mesh has no triangles");
        }
        if self.indices.len() / 3 > MAX_TRIANGLES {
            return fail(format!(
                "The mesh has {} triangles; a collider keeps at most {MAX_TRIANGLES}",
                self.indices.len() / 3
            ));
        }
        if self.positions.iter().flatten().any(|v| !v.is_finite()) {
            return fail("The mesh has a vertex that is not a finite number");
        }
        let count = self.positions.len() as u32;
        if self.indices.iter().any(|i| *i >= count) {
            return fail("A triangle names a vertex the mesh does not have");
        }

        // Weld on a grid of cell `weld`, comparing within the cell's neighbours
        // so a pair straddling a cell edge still merges.
        let mut remap: Vec<u32> = Vec::with_capacity(self.positions.len());
        let mut kept: Vec<[f32; 3]> = Vec::new();
        if weld > 0.0 {
            let mut grid: std::collections::HashMap<[i64; 3], Vec<u32>> = Default::default();
            let cell = |p: &[f32; 3]| p.map(|v| (v / weld).floor() as i64);
            for p in &self.positions {
                let c = cell(p);
                let mut found = None;
                'search: for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let key = [c[0] + dx, c[1] + dy, c[2] + dz];
                            if let Some(list) = grid.get(&key) {
                                for &k in list {
                                    let q = kept[k as usize];
                                    let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                                    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= weld * weld {
                                        found = Some(k);
                                        break 'search;
                                    }
                                }
                            }
                        }
                    }
                }
                match found {
                    Some(k) => {
                        stats.welded += 1;
                        remap.push(k);
                    }
                    None => {
                        let k = kept.len() as u32;
                        grid.entry(c).or_default().push(k);
                        kept.push(*p);
                        remap.push(k);
                    }
                }
            }
        } else {
            kept = self.positions.clone();
            remap = (0..count).collect();
        }

        let mut indices = Vec::with_capacity(self.indices.len());
        for tri in self.indices.chunks_exact(3) {
            let (a, b, c) = (
                remap[tri[0] as usize],
                remap[tri[1] as usize],
                remap[tri[2] as usize],
            );
            if a == b || b == c || a == c || triangle_area(&kept, a, b, c) <= 1e-14 {
                stats.degenerate += 1;
                continue;
            }
            indices.extend([a, b, c]);
        }
        if indices.is_empty() {
            return fail("Every triangle of the mesh is degenerate");
        }
        // Compact: drop vertices no triangle uses.
        let mut used = vec![u32::MAX; kept.len()];
        let mut positions = Vec::new();
        for i in &mut indices {
            if used[*i as usize] == u32::MAX {
                used[*i as usize] = positions.len() as u32;
                positions.push(kept[*i as usize]);
            }
            *i = used[*i as usize];
        }
        stats.vertices = positions.len();
        stats.triangles = indices.len() / 3;
        Ok((RawMesh { positions, indices }, stats))
    }

    /// Cooks this mesh into `kind`.
    pub fn cook(
        &self,
        kind: MeshKind,
        settings: &CookSettings,
        decompose: Option<&Decompose>,
        control: &CookControl,
    ) -> Result<Cooked, CookError> {
        settings.check()?;
        control.report("cleaning", 0.0);
        let (clean, mut stats) = self.clean(settings.weld)?;
        control.check()?;
        match kind {
            MeshKind::Triangles => {
                stats.bytes = clean.positions.len() * 12 + clean.indices.len() * 4;
                Ok(Cooked::Triangles {
                    vertices: clean.positions,
                    indices: clean.indices,
                    stats,
                })
            }
            MeshKind::Hull => {
                control.report("hull", 0.5);
                let hull = hull::quickhull(
                    &clean.positions,
                    settings.max_hull_vertices as usize,
                    control,
                )?;
                stats.hulls = 1;
                stats.error = hull.error;
                stats.vertices = hull.vertices.len();
                stats.triangles = hull.faces.len();
                stats.bytes = hull.vertices.len() * 12;
                Ok(Cooked::Hull {
                    volume: hull.volume(),
                    points: hull.vertices,
                    stats,
                })
            }
            MeshKind::Decomposed => {
                let settings_d = decompose.cloned().unwrap_or_default();
                let parts = decompose::decompose(&clean, &settings_d, control)?;
                stats.hulls = parts.hulls.len();
                stats.error = parts.error;
                stats.vertices = parts.hulls.iter().map(Vec::len).sum();
                stats.bytes = stats.vertices * 12;
                Ok(Cooked::Decomposed {
                    volume: parts.volume,
                    hulls: parts.hulls,
                    stats,
                })
            }
        }
    }
}

fn triangle_area(p: &[[f32; 3]], a: u32, b: u32, c: u32) -> f32 {
    let (a, b, c) = (p[a as usize], p[b as usize], p[c as usize]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

// ─── Stored form ───────────────────────────────────────────────────────────

const MAGIC: &[u8; 4] = b"BLCK";

impl Cooked {
    /// The bytes written to the cache and shipped in a build.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&COOKER_VERSION.to_le_bytes());
        let kind = match self {
            Cooked::Hull { .. } => 0u8,
            Cooked::Triangles { .. } => 1,
            Cooked::Decomposed { .. } => 2,
        };
        out.push(kind);
        let stats = self.stats();
        for n in [
            stats.input_vertices,
            stats.input_triangles,
            stats.vertices,
            stats.triangles,
            stats.welded,
            stats.degenerate,
            stats.hulls,
            stats.bytes,
        ] {
            out.extend_from_slice(&(n as u64).to_le_bytes());
        }
        out.extend_from_slice(&stats.error.to_le_bytes());
        let points = |out: &mut Vec<u8>, list: &[[f32; 3]]| {
            out.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for p in list {
                for v in p {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        };
        match self {
            Cooked::Hull {
                points: list,
                volume,
                ..
            } => {
                out.extend_from_slice(&volume.to_le_bytes());
                points(&mut out, list);
            }
            Cooked::Triangles {
                vertices, indices, ..
            } => {
                points(&mut out, vertices);
                out.extend_from_slice(&(indices.len() as u32).to_le_bytes());
                for i in indices {
                    out.extend_from_slice(&i.to_le_bytes());
                }
            }
            Cooked::Decomposed { hulls, volume, .. } => {
                out.extend_from_slice(&volume.to_le_bytes());
                out.extend_from_slice(&(hulls.len() as u32).to_le_bytes());
                for hull in hulls {
                    points(&mut out, hull);
                }
            }
        }
        out
    }

    /// Reads what [`Cooked::to_bytes`] wrote; refuses anything else.
    pub fn from_bytes(bytes: &[u8]) -> Result<Cooked, String> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err("Not cooked collision data".into());
        }
        let version = r.u32()?;
        if version != COOKER_VERSION {
            return Err(format!(
                "Cooked with cooker {version}, this build reads {COOKER_VERSION}"
            ));
        }
        let kind = r.take(1)?[0];
        let mut stats = CookStats::default();
        let mut counts = [0usize; 8];
        for slot in &mut counts {
            *slot = r.u64()? as usize;
        }
        stats.input_vertices = counts[0];
        stats.input_triangles = counts[1];
        stats.vertices = counts[2];
        stats.triangles = counts[3];
        stats.welded = counts[4];
        stats.degenerate = counts[5];
        stats.hulls = counts[6];
        stats.bytes = counts[7];
        stats.error = r.f32()?;
        match kind {
            0 => {
                let volume = r.f32()?;
                let points = r.points()?;
                Ok(Cooked::Hull {
                    points,
                    volume,
                    stats,
                })
            }
            1 => {
                let vertices = r.points()?;
                let n = r.u32()? as usize;
                if n % 3 != 0 || n > MAX_TRIANGLES * 3 {
                    return Err("Corrupt cooked triangle list".into());
                }
                let mut indices = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = r.u32()?;
                    if i as usize >= vertices.len() {
                        return Err("Corrupt cooked triangle index".into());
                    }
                    indices.push(i);
                }
                Ok(Cooked::Triangles {
                    vertices,
                    indices,
                    stats,
                })
            }
            2 => {
                let volume = r.f32()?;
                let n = r.u32()? as usize;
                if n > 4096 {
                    return Err("Corrupt cooked hull count".into());
                }
                let mut hulls = Vec::with_capacity(n);
                for _ in 0..n {
                    hulls.push(r.points()?);
                }
                Ok(Cooked::Decomposed {
                    hulls,
                    volume,
                    stats,
                })
            }
            other => Err(format!("Unknown cooked kind {other}")),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("Cooked collision data ends early")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn points(&mut self) -> Result<Vec<[f32; 3]>, String> {
        let n = self.u32()? as usize;
        if n > MAX_TRIANGLES * 3 {
            return Err("Corrupt cooked point count".into());
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push([self.f32()?, self.f32()?, self.f32()?]);
        }
        Ok(out)
    }
}

/// A 128-bit content hash as hex: the cache key of cooked data. Not a security
/// boundary, only a way to name the same inputs the same way.
pub fn content_key(parts: &[&[u8]]) -> String {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for part in parts {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part.iter()) {
            a = (a ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            b = (b.rotate_left(5) ^ u64::from(*byte)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }
    format!("{a:016x}{b:016x}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn cube(size: f32) -> RawMesh {
        let h = size / 2.0;
        let p = |x: f32, y: f32, z: f32| [x * h, y * h, z * h];
        let positions = vec![
            p(-1., -1., -1.),
            p(1., -1., -1.),
            p(1., 1., -1.),
            p(-1., 1., -1.),
            p(-1., -1., 1.),
            p(1., -1., 1.),
            p(1., 1., 1.),
            p(-1., 1., 1.),
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ];
        RawMesh { positions, indices }
    }

    #[test]
    fn welding_merges_close_vertices_and_drops_degenerates() {
        let mut mesh = cube(2.0);
        // A duplicate of corner 0 nudged by less than the weld distance, and a
        // triangle that collapses once it is merged.
        mesh.positions.push([-1.0 + 1e-7, -1.0, -1.0]);
        mesh.indices.extend([0, 8, 1]);
        let (clean, stats) = mesh.clean(1e-5).unwrap();
        assert_eq!(stats.welded, 1);
        assert_eq!(stats.degenerate, 1);
        assert_eq!((clean.positions.len(), clean.indices.len() / 3), (8, 12));
        assert_eq!((stats.vertices, stats.triangles), (8, 12));
    }

    #[test]
    fn a_broken_mesh_is_refused_with_a_reason() {
        let nan = RawMesh {
            positions: vec![[f32::NAN, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        };
        assert!(nan.clean(0.0).unwrap_err().to_string().contains("finite"));
        let out_of_range = RawMesh {
            positions: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 7],
        };
        assert!(
            out_of_range
                .clean(0.0)
                .unwrap_err()
                .to_string()
                .contains("vertex the mesh does not have")
        );
        let flat = RawMesh {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            indices: vec![0, 1, 2],
        };
        assert!(
            flat.clean(0.0)
                .unwrap_err()
                .to_string()
                .contains("degenerate")
        );
        assert!(RawMesh::default().clean(0.0).is_err());
    }

    #[test]
    fn cooked_data_round_trips_and_rejects_damage() {
        let settings = CookSettings::default();
        for kind in [MeshKind::Hull, MeshKind::Triangles] {
            let cooked = cube(2.0)
                .cook(kind, &settings, None, &CookControl::new())
                .unwrap();
            let bytes = cooked.to_bytes();
            assert_eq!(Cooked::from_bytes(&bytes).unwrap(), cooked);
            assert!(Cooked::from_bytes(&bytes[..bytes.len() - 3]).is_err());
            let mut bad = bytes.clone();
            bad[0] = b'X';
            assert!(Cooked::from_bytes(&bad).is_err());
        }
    }

    #[test]
    fn the_same_inputs_cook_to_the_same_bytes() {
        let a = cube(3.0)
            .cook(
                MeshKind::Hull,
                &CookSettings::default(),
                None,
                &CookControl::new(),
            )
            .unwrap()
            .to_bytes();
        let b = cube(3.0)
            .cook(
                MeshKind::Hull,
                &CookSettings::default(),
                None,
                &CookControl::new(),
            )
            .unwrap()
            .to_bytes();
        assert_eq!(a, b);
        assert_ne!(content_key(&[b"a"]), content_key(&[b"b"]));
        assert_eq!(content_key(&[b"a", b"b"]), content_key(&[b"a", b"b"]));
        assert_ne!(content_key(&[b"ab"]), content_key(&[b"a", b"b"]));
    }

    #[test]
    fn a_cancelled_cook_stops() {
        let control = CookControl::new();
        control.cancel();
        assert_eq!(
            cube(1.0)
                .cook(MeshKind::Hull, &CookSettings::default(), None, &control)
                .unwrap_err(),
            CookError::Cancelled
        );
    }
}
