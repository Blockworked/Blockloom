//! The terrain store: grids kept as content-addressed tiles under
//! `terrain/` in the project folder.
//!
//! A grid (heights, painted weights, holes, a density map) is cut into
//! [`TILE`]-square tiles, each deflated and named by its own hash, and a
//! manifest listing them is named by its hash too. The document only ever
//! holds a manifest name, so undo is the document's own snapshot history: an
//! older name still points at older tiles. A stroke writes only the tiles it
//! changed. [`prune`] drops whatever no manifest in use still names.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The store's folder inside a project.
pub const DIR: &str = "terrain";
/// Samples per tile side.
pub const TILE: u32 = 64;

/// What one sample of a grid holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GridKind {
    /// A height, 0-1 as a little-endian u16.
    Height16,
    /// Four layer weights.
    Rgba8,
    /// One byte: holes, densities.
    Mask8,
}

impl GridKind {
    pub fn stride(self) -> usize {
        match self {
            GridKind::Height16 => 2,
            GridKind::Rgba8 => 4,
            GridKind::Mask8 => 1,
        }
    }
}

/// A whole grid in memory, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub kind: GridKind,
    pub side: u32,
    pub bytes: Vec<u8>,
}

impl Grid {
    pub fn new(kind: GridKind, side: u32) -> Self {
        Self {
            kind,
            side,
            bytes: vec![0; side as usize * side as usize * kind.stride()],
        }
    }

    pub fn from_heights(field: &super::Heightfield) -> Self {
        let mut bytes = Vec::with_capacity(field.samples.len() * 2);
        for sample in &field.samples {
            let value = (sample.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Self {
            kind: GridKind::Height16,
            side: field.side,
            bytes,
        }
    }

    pub fn to_heights(&self) -> Option<super::Heightfield> {
        (self.kind == GridKind::Height16).then(|| super::Heightfield {
            side: self.side,
            samples: self
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair) as f32 / u16::MAX as f32)
                .collect(),
        })
    }

    pub fn from_weights(side: u32, weights: &[[u8; 4]]) -> Self {
        Self {
            kind: GridKind::Rgba8,
            side,
            bytes: weights.iter().flatten().copied().collect(),
        }
    }

    pub fn to_weights(&self) -> Option<Vec<[u8; 4]>> {
        (self.kind == GridKind::Rgba8).then(|| self.bytes.as_chunks::<4>().0.to_vec())
    }

    /// One byte per sample; only for [`GridKind::Mask8`].
    pub fn mask(&self) -> Option<&[u8]> {
        (self.kind == GridKind::Mask8).then_some(self.bytes.as_slice())
    }

    pub fn mask_mut(&mut self) -> Option<&mut [u8]> {
        (self.kind == GridKind::Mask8).then_some(self.bytes.as_mut_slice())
    }

    /// The same grid at another side, nearest-sample for masks and weights.
    pub fn resample(&self, side: u32) -> Grid {
        if side == self.side {
            return self.clone();
        }
        if let Some(field) = self.to_heights() {
            return Grid::from_heights(&field.resample(side));
        }
        let stride = self.kind.stride();
        let scale = (self.side - 1) as f32 / (side - 1).max(1) as f32;
        let mut out = Grid::new(self.kind, side);
        for j in 0..side {
            for i in 0..side {
                let si = ((i as f32 * scale).round() as u32).min(self.side - 1);
                let sj = ((j as f32 * scale).round() as u32).min(self.side - 1);
                let from = (sj * self.side + si) as usize * stride;
                let to = (j * side + i) as usize * stride;
                out.bytes[to..to + stride].copy_from_slice(&self.bytes[from..from + stride]);
            }
        }
        out
    }

    fn tiles_per_side(&self) -> u32 {
        self.side.div_ceil(TILE)
    }

    /// The raw bytes of tile `(tx, tz)`, rows cut at the grid's edge.
    fn tile_bytes(&self, tx: u32, tz: u32) -> Vec<u8> {
        let stride = self.kind.stride();
        let (x0, z0) = (tx * TILE, tz * TILE);
        let (x1, z1) = ((x0 + TILE).min(self.side), (z0 + TILE).min(self.side));
        let mut out = Vec::with_capacity(((x1 - x0) * (z1 - z0)) as usize * stride);
        for z in z0..z1 {
            let from = (z * self.side + x0) as usize * stride;
            let to = (z * self.side + x1) as usize * stride;
            out.extend_from_slice(&self.bytes[from..to]);
        }
        out
    }

    fn put_tile(&mut self, tx: u32, tz: u32, bytes: &[u8]) -> Result<(), String> {
        let stride = self.kind.stride();
        let (x0, z0) = (tx * TILE, tz * TILE);
        let (x1, z1) = ((x0 + TILE).min(self.side), (z0 + TILE).min(self.side));
        let row = (x1 - x0) as usize * stride;
        if bytes.len() != row * (z1 - z0) as usize {
            return Err(format!("terrain tile {tx},{tz} is the wrong size"));
        }
        for (k, z) in (z0..z1).enumerate() {
            let to = (z * self.side + x0) as usize * stride;
            self.bytes[to..to + row].copy_from_slice(&bytes[k * row..(k + 1) * row]);
        }
        Ok(())
    }
}

/// A grid's table of tiles. Named by its own hash in the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub kind: GridKind,
    pub side: u32,
    pub tile: u32,
    /// Tile names, row-major.
    pub tiles: Vec<String>,
}

/// A stable 96-bit name for some bytes: FNV-1a and CRC32 side by side.
pub fn hash_name(bytes: &[u8]) -> String {
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        fnv ^= *byte as u64;
        fnv = fnv.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(bytes);
    format!("{fnv:016x}{:08x}", crc.finalize())
}

pub fn dir(project: &Path) -> PathBuf {
    project.join(DIR)
}

fn file(project: &Path, name: &str, extension: &str) -> Result<PathBuf, String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("\"{name}\" isn't a terrain store name"));
    }
    Ok(dir(project).join(format!("{name}.{extension}")))
}

/// Heights store as row deltas, which deflate far better than levels.
fn encode(kind: GridKind, raw: &[u8]) -> Vec<u8> {
    let mut data = raw.to_vec();
    if kind == GridKind::Height16 {
        let mut previous = 0u16;
        for pair in data.as_chunks_mut::<2>().0 {
            let value = u16::from_le_bytes(*pair);
            *pair = value.wrapping_sub(previous).to_le_bytes();
            previous = value;
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = encoder.write_all(&data);
    encoder.finish().unwrap_or_default()
}

fn decode(kind: GridKind, packed: &[u8]) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(packed)
        .read_to_end(&mut data)
        .map_err(|e| format!("terrain tile won't inflate: {e}"))?;
    if kind == GridKind::Height16 {
        let mut previous = 0u16;
        for pair in data.as_chunks_mut::<2>().0 {
            let value = u16::from_le_bytes(*pair).wrapping_add(previous);
            *pair = value.to_le_bytes();
            previous = value;
        }
    }
    Ok(data)
}

/// Writes a grid into the project's store, skipping tiles already there,
/// and answers the manifest's name.
pub fn save(project: &Path, grid: &Grid) -> Result<String, String> {
    let folder = dir(project);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let per_side = grid.tiles_per_side();
    let mut tiles = Vec::with_capacity((per_side * per_side) as usize);
    for tz in 0..per_side {
        for tx in 0..per_side {
            let raw = grid.tile_bytes(tx, tz);
            // The kind is part of the name: equal bytes of different kinds
            // encode differently.
            let mut keyed = vec![grid.kind.stride() as u8];
            keyed.extend_from_slice(&raw);
            let name = hash_name(&keyed);
            let path = file(project, &name, "tile")?;
            if !path.exists() {
                write_atomic(&path, &encode(grid.kind, &raw))?;
            }
            tiles.push(name);
        }
    }
    let manifest = Manifest {
        kind: grid.kind,
        side: grid.side,
        tile: TILE,
        tiles,
    };
    let json = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    let name = hash_name(&json);
    let path = file(project, &name, "json")?;
    if !path.exists() {
        write_atomic(&path, &json)?;
    }
    Ok(name)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("part");
    std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_manifest(project: &Path, name: &str) -> Result<Manifest, String> {
    let path = file(project, name, "json")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads a grid back out of the store.
pub fn load(project: &Path, name: &str) -> Result<Grid, String> {
    let manifest = read_manifest(project, name)?;
    if manifest.tile != TILE {
        return Err(format!("terrain grid {name} uses {} tiles", manifest.tile));
    }
    let mut grid = Grid::new(manifest.kind, manifest.side);
    let per_side = grid.tiles_per_side();
    if manifest.tiles.len() != (per_side * per_side) as usize {
        return Err(format!("terrain grid {name} is missing tiles"));
    }
    for tz in 0..per_side {
        for tx in 0..per_side {
            let tile = &manifest.tiles[(tz * per_side + tx) as usize];
            let path = file(project, tile, "tile")?;
            let packed = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            grid.put_tile(tx, tz, &decode(manifest.kind, &packed)?)?;
        }
    }
    Ok(grid)
}

/// Deletes every tile and manifest no name in `keep` reaches. `keep` should
/// hold every manifest the open document and its undo history name.
pub fn prune(project: &Path, keep: &HashSet<String>) -> Result<usize, String> {
    let folder = dir(project);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(0);
    };
    let mut live: HashSet<String> = HashSet::new();
    for name in keep {
        if let Ok(manifest) = read_manifest(project, name) {
            live.insert(format!("{name}.json"));
            live.extend(manifest.tiles.iter().map(|tile| format!("{tile}.tile")));
        }
    }
    let mut removed = 0;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let ours = file_name.ends_with(".tile")
            || file_name.ends_with(".json")
            || file_name.ends_with(".part");
        if ours && !live.contains(&file_name) && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Every store name a terrain spec points at.
pub fn names(spec: &super::TerrainSpec) -> impl Iterator<Item = &str> {
    [spec.heights.as_str(), spec.splat.as_str(), spec.holes.as_str()]
        .into_iter()
        .chain(spec.grass.iter().map(|g| g.density_map.as_str()))
        .chain(spec.scatter.iter().map(|s| s.density_map.as_str()))
        .filter(|name| !name.is_empty())
}

/// The store names every terrain in a project points at.
pub fn project_names(project: &crate::project::Project) -> HashSet<String> {
    project
        .actors
        .iter()
        .filter_map(|actor| actor.components.terrain())
        .flat_map(|spec| names(spec).map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// A terrain's heights at its own resolution: the stored grid resampled if
/// the resolution moved since, or flat.
pub fn heights_for(project: Option<&Path>, spec: &super::TerrainSpec) -> super::Heightfield {
    let side = spec.resolution;
    project
        .filter(|_| !spec.heights.is_empty())
        .and_then(|dir| {
            load(dir, &spec.heights)
                .map_err(|e| tracing::warn!("terrain heights: {e}"))
                .ok()
        })
        .and_then(|grid| grid.to_heights())
        .map(|field| field.resample(side))
        .unwrap_or_else(|| super::Heightfield::flat(side, 0.0))
}

/// An optional grid at a terrain's resolution, if it names one that loads.
pub fn grid_for(project: Option<&Path>, name: &str, side: u32) -> Option<Grid> {
    if name.is_empty() {
        return None;
    }
    let dir = project?;
    load(dir, name)
        .map_err(|e| tracing::warn!("terrain grid {name}: {e}"))
        .ok()
        .map(|grid| grid.resample(side))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::Heightfield;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blockloom-terrain-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_grid_round_trips_through_the_store() {
        let project = temp();
        let mut field = Heightfield::flat(129, 0.25);
        for (k, sample) in field.samples.iter_mut().enumerate() {
            *sample = (k % 97) as f32 / 96.0;
        }
        let grid = Grid::from_heights(&field);
        let name = save(&project, &grid).unwrap();
        let back = load(&project, &name).unwrap();
        assert_eq!(back, grid);
        let heights = back.to_heights().unwrap();
        assert!((heights.at(5, 0) - field.at(5, 0)).abs() < 1e-4);
        // Saving the same grid again writes nothing new.
        assert_eq!(save(&project, &grid).unwrap(), name);
        std::fs::remove_dir_all(project).ok();
    }

    #[test]
    fn a_small_edit_writes_only_its_tiles_and_prune_keeps_what_is_named() {
        let project = temp();
        let mut grid = Grid::new(GridKind::Mask8, 257);
        let first = save(&project, &grid).unwrap();
        let count = || std::fs::read_dir(dir(&project)).unwrap().count();
        // Every tile of an empty grid is the same, bar the short edge ones.
        let before = count();
        grid.mask_mut().unwrap()[10] = 200;
        let second = save(&project, &grid).unwrap();
        assert_ne!(first, second);
        assert_eq!(count(), before + 2, "one new tile and one manifest");
        let keep: HashSet<String> = [second.clone()].into();
        assert!(prune(&project, &keep).unwrap() >= 1);
        assert!(load(&project, &first).is_err());
        assert_eq!(load(&project, &second).unwrap(), grid);
        std::fs::remove_dir_all(project).ok();
    }

    #[test]
    fn grids_resample_by_kind() {
        let mut grid = Grid::new(GridKind::Rgba8, 129);
        let at = (64 * 129 + 64) * 4;
        grid.bytes[at..at + 4].copy_from_slice(&[1, 2, 3, 4]);
        let big = grid.resample(257);
        let at = (128 * 257 + 128) * 4;
        assert_eq!(&big.bytes[at..at + 4], &[1, 2, 3, 4]);
    }

    #[test]
    fn names_must_be_hex() {
        assert!(load(Path::new("/tmp"), "../etc/passwd").is_err());
        assert!(load(Path::new("/tmp"), "").is_err());
    }
}
