//! Level building on top of [`Tilemap`]: autotile rules, brushes, tile
//! regions, Tiled tileset import, parallax layers and rooms.
//!
//! - [`AutotileSet`] picks a sheet cell from a painted cell's neighbours,
//!   either the 16 edge cases (N/E/S/W) or the 47 blob cases (edges plus
//!   the corners between two filled edges).
//! - [`TileBrush`] is every edit a paint stroke makes: paint, erase, fill,
//!   line, rect, scatter. [`Tilemap::apply_brush`] runs one and re-resolves
//!   autotiles around what changed.
//! - [`TileRegion`] marks sheet cells as spawn, checkpoint, kill, ladder or
//!   water, which the runtime acts on and the editor overlays.
//! - [`ParallaxSpec`] and [`RoomSpec`] are the Parallax and Room components.
//! - [`LevelSense`] is the level half of the sensor snapshot: live maps and
//!   room bounds, which `tile at` and `room containing` read.
//!
//! Everything here is 2D: a tilemap is a flat layer in a 2D world.

use crate::material::{TileAnimation, TileRect, Tilemap};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ─── Autotiles ─────────────────────────────────────────────────────────────

/// How an autotile set reads a cell's neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AutotileMode {
    /// Four edges, 16 cases: bits N=1, E=2, S=4, W=8.
    #[default]
    Edge,
    /// Edges plus corners, 47 cases: bits N=1, NE=2, E=4, SE=8, S=16,
    /// SW=32, W=64, NW=128, a corner only counting between two filled edges.
    Blob,
}

/// One case of an autotile set: the neighbour mask and the cell it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutotileRule {
    pub mask: u8,
    pub tile: i32,
}

/// A named terrain whose painted cells pick their sheet cell by neighbours.
/// A cell belongs to the set when it shows any of the set's tiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutotileSet {
    pub name: String,
    #[serde(default)]
    pub mode: AutotileMode,
    #[serde(default)]
    pub rules: Vec<AutotileRule>,
}

const N: u8 = 1;
const NE: u8 = 2;
const E: u8 = 4;
const SE: u8 = 8;
const S: u8 = 16;
const SW: u8 = 32;
const W: u8 = 64;
const NW: u8 = 128;

/// A blob mask with every corner dropped that doesn't sit between two
/// filled edges, which is what makes 256 raw masks 47 cases.
pub fn reduce_blob(mask: u8) -> u8 {
    let mut out = mask & (N | E | S | W);
    for (corner, a, b) in [(NE, N, E), (SE, S, E), (SW, S, W), (NW, N, W)] {
        if mask & corner != 0 && mask & a != 0 && mask & b != 0 {
            out |= corner;
        }
    }
    out
}

/// The 47 blob cases in ascending mask order: the order a blob strip lays
/// its tiles out in.
pub fn blob_masks() -> Vec<u8> {
    let mut masks: Vec<u8> = (0..=255u8).map(reduce_blob).collect();
    masks.sort_unstable();
    masks.dedup();
    masks
}

impl AutotileSet {
    /// An edge set over 16 consecutive sheet cells from `first`, cell
    /// `first + mask` for each mask.
    pub fn edge_strip(name: &str, first: i32) -> Self {
        Self {
            name: name.to_string(),
            mode: AutotileMode::Edge,
            rules: (0..16)
                .map(|mask| AutotileRule {
                    mask,
                    tile: first + mask as i32,
                })
                .collect(),
        }
    }

    /// A blob set over 47 consecutive sheet cells from `first`, in
    /// [`blob_masks`] order.
    pub fn blob_strip(name: &str, first: i32) -> Self {
        Self {
            name: name.to_string(),
            mode: AutotileMode::Blob,
            rules: blob_masks()
                .into_iter()
                .enumerate()
                .map(|(index, mask)| AutotileRule {
                    mask,
                    tile: first + index as i32,
                })
                .collect(),
        }
    }

    pub fn contains(&self, tile: i32) -> bool {
        tile >= 0 && self.rules.iter().any(|rule| rule.tile == tile)
    }

    /// The tile a freshly painted cell starts as, before its neighbours
    /// resolve it: the fully surrounded case, or the first rule.
    pub fn fill_tile(&self) -> Option<i32> {
        let full = match self.mode {
            AutotileMode::Edge => N | E | S | W,
            AutotileMode::Blob => 255,
        };
        self.rules
            .iter()
            .find(|rule| rule.mask == full)
            .or(self.rules.first())
            .map(|rule| rule.tile)
    }

    /// The tile for a neighbour mask: the exact case, else the rule sharing
    /// the most bits with it (the lowest tile wins a tie), so a partial set still
    /// paints something sensible.
    pub fn pick(&self, mask: u8) -> Option<i32> {
        if let Some(rule) = self.rules.iter().find(|rule| rule.mask == mask) {
            return Some(rule.tile);
        }
        let width = match self.mode {
            AutotileMode::Edge => 4,
            AutotileMode::Blob => 8,
        };
        let keep = if width == 8 { 0xFF } else { 0x0F };
        self.rules
            .iter()
            .max_by_key(|rule| {
                let agree = (!(rule.mask ^ mask)) & keep;
                // max_by_key keeps the last max, so the lowest tile wins.
                (agree.count_ones(), std::cmp::Reverse(rule.tile))
            })
            .map(|rule| rule.tile)
    }
}

// ─── Regions ───────────────────────────────────────────────────────────────

/// What a region tile means to the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RegionKind {
    /// Where a body respawns before it has touched any checkpoint.
    #[default]
    Spawn,
    /// Touching it moves the body's respawn point here.
    Checkpoint,
    /// Touching it sends the body back to its respawn point.
    Kill,
    /// Gravity lets go inside it, so a body climbs by its own velocity.
    Ladder,
    /// Gravity weakens and motion drags inside it.
    Water,
}

impl RegionKind {
    pub const ALL: [RegionKind; 5] = [
        RegionKind::Spawn,
        RegionKind::Checkpoint,
        RegionKind::Kill,
        RegionKind::Ladder,
        RegionKind::Water,
    ];

    /// Which kind wins when a body touches several: a kill beats a
    /// checkpoint beats a spawn beats water beats a ladder.
    pub fn rank(self) -> u8 {
        match self {
            RegionKind::Ladder => 0,
            RegionKind::Water => 1,
            RegionKind::Spawn => 2,
            RegionKind::Checkpoint => 3,
            RegionKind::Kill => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RegionKind::Spawn => "Spawn",
            RegionKind::Checkpoint => "Checkpoint",
            RegionKind::Kill => "Kill",
            RegionKind::Ladder => "Ladder",
            RegionKind::Water => "Water",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(name))
    }
}

/// Every cell painted `tile` is part of a `kind` region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileRegion {
    pub tile: i32,
    pub kind: RegionKind,
}

// ─── Brushes ───────────────────────────────────────────────────────────────

/// What a stroke does to the cells it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushTool {
    /// A square of `size` cells stamped along the drag.
    #[default]
    Paint,
    Erase,
    /// Flood fill from the end cell over its connected same-tile cells.
    Fill,
    /// One cell wide, start to end.
    Line,
    /// The rectangle between start and end, filled.
    Rect,
    /// Paint's stamp, but each cell only takes paint with `density` odds.
    Scatter,
    /// Reads the tile under the pointer and changes nothing.
    Pick,
}

/// One stroke's settings. A stroke runs from one cell to another; a click
/// is a stroke that starts and ends on the same cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TileBrush {
    pub tool: BrushTool,
    /// Sheet cells to paint with. More than one makes variants, picked per
    /// cell as `jitter` allows.
    pub tiles: Vec<i32>,
    /// An autotile set's name. Paint marks cells as that terrain and lets
    /// the rules pick the cell; erase re-resolves the neighbours.
    pub autotile: String,
    /// Stamp side in cells, 1-16.
    pub size: u32,
    /// Scatter's odds a covered cell takes paint, 0-1.
    pub density: f32,
    /// Odds a painted cell takes a random variant rather than the first
    /// tile, 0-1.
    pub jitter: f32,
    /// Seeds scatter and variants, so a replayed stroke paints the same.
    pub seed: u64,
}

impl Default for TileBrush {
    fn default() -> Self {
        Self {
            tool: BrushTool::Paint,
            tiles: vec![0],
            autotile: String::new(),
            size: 1,
            density: 0.3,
            jitter: 0.0,
            seed: 0,
        }
    }
}

/// splitmix64 over a seed and a cell, as a 0-1 float.
fn cell_noise(seed: u64, x: i32, y: i32, salt: u64) -> f32 {
    let mut z = seed
        ^ (x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ salt.wrapping_mul(0x1656_67B1_9E37_79F9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// The cells a Bresenham line from `a` to `b` crosses, both ends included.
pub fn line_cells(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let mut err = dx + dy;
    let mut out = vec![(x, y)];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        out.push((x, y));
    }
    out
}

/// What a map is made of, for the editor's stats line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TileStats {
    pub tiles: usize,
    /// Draw calls: the whole map is one mesh.
    pub draw_batches: usize,
    pub colliding_rects: usize,
    pub animated: usize,
    pub region_tiles: usize,
}

impl Tilemap {
    fn at(&self, x: i32, y: i32) -> Option<i32> {
        if x < 0 || y < 0 {
            return None;
        }
        self.tile_at(x as u32, y as u32)
    }

    fn put(&mut self, x: i32, y: i32, tile: i32) -> bool {
        match self.at(x, y) {
            Some(old) if old != tile => {
                self.set_tile(x as u32, y as u32, tile);
                true
            }
            _ => false,
        }
    }

    /// The animation a painted tile belongs to: its own, or the one it is a
    /// frame of. Painting a frame paints the animation's tile, so every
    /// cell of it cycles in step.
    pub fn animation_base(&self, tile: i32) -> i32 {
        if self.animations.iter().any(|a| a.tile == tile) {
            return tile;
        }
        self.animations
            .iter()
            .find(|animation| animation.frames.contains(&tile))
            .map(|animation| animation.tile)
            .unwrap_or(tile)
    }

    pub fn autotile(&self, name: &str) -> Option<&AutotileSet> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.autotiles
            .iter()
            .find(|set| set.name.eq_ignore_ascii_case(name))
    }

    /// The neighbour mask of `(x, y)` for `set`. Off the map counts as
    /// filled, so a terrain meeting the border shows no edge there.
    pub fn neighbour_mask(&self, set: &AutotileSet, x: i32, y: i32) -> u8 {
        let filled = |dx: i32, dy: i32| match self.at(x + dx, y + dy) {
            Some(tile) => set.contains(tile),
            None => true,
        };
        // Grid y grows down, so north is y - 1.
        match set.mode {
            AutotileMode::Edge => {
                let mut mask = 0;
                for (bit, dx, dy) in [(1, 0, -1), (2, 1, 0), (4, 0, 1), (8, -1, 0)] {
                    if filled(dx, dy) {
                        mask |= bit;
                    }
                }
                mask
            }
            AutotileMode::Blob => {
                let mut mask = 0;
                for (bit, dx, dy) in [
                    (N, 0, -1),
                    (NE, 1, -1),
                    (E, 1, 0),
                    (SE, 1, 1),
                    (S, 0, 1),
                    (SW, -1, 1),
                    (W, -1, 0),
                    (NW, -1, -1),
                ] {
                    if filled(dx, dy) {
                        mask |= bit;
                    }
                }
                reduce_blob(mask)
            }
        }
    }

    /// Re-picks every cell of every autotile set within one cell of
    /// `cells`. Returns how many changed.
    pub fn retile(&mut self, cells: &[(i32, i32)]) -> usize {
        let sets = self.autotiles.clone();
        let mut near = std::collections::BTreeSet::new();
        for &(x, y) in cells {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    near.insert((y + dy, x + dx));
                }
            }
        }
        let mut changed = 0;
        for (y, x) in near {
            let Some(tile) = self.at(x, y) else {
                continue;
            };
            let Some(set) = sets.iter().find(|set| set.contains(tile)) else {
                continue;
            };
            let mask = self.neighbour_mask(set, x, y);
            if let Some(next) = set.pick(mask)
                && self.put(x, y, next)
            {
                changed += 1;
            }
        }
        changed
    }

    /// The tile a brush lays at `(x, y)`: its autotile terrain, else one of
    /// its tiles, a variant with `jitter` odds.
    fn brush_tile(&self, brush: &TileBrush, x: i32, y: i32) -> Option<i32> {
        if let Some(set) = self.autotile(&brush.autotile) {
            return set.fill_tile();
        }
        let first = *brush.tiles.first()?;
        let tile = if brush.tiles.len() > 1 && cell_noise(brush.seed, x, y, 1) < brush.jitter {
            let pick = (cell_noise(brush.seed, x, y, 2) * brush.tiles.len() as f32) as usize;
            brush.tiles[pick.min(brush.tiles.len() - 1)]
        } else {
            first
        };
        Some(self.animation_base(tile))
    }

    /// Runs one stroke from cell `from` to cell `to` (grid cells, y down,
    /// either may lie off the map). Returns the cells it changed, autotile
    /// fix-ups included.
    pub fn apply_brush(
        &mut self,
        brush: &TileBrush,
        from: (i32, i32),
        to: (i32, i32),
    ) -> Vec<(i32, i32)> {
        let erase = brush.tool == BrushTool::Erase;
        let mut touched: Vec<(i32, i32)> = Vec::new();
        let size = brush.size.clamp(1, 16) as i32;
        let stamp = |center: (i32, i32)| {
            let lo = -(size - 1) / 2;
            let mut cells = Vec::new();
            for dy in lo..lo + size {
                for dx in lo..lo + size {
                    cells.push((center.0 + dx, center.1 + dy));
                }
            }
            cells
        };
        let cells: Vec<(i32, i32)> = match brush.tool {
            BrushTool::Pick => return Vec::new(),
            BrushTool::Paint | BrushTool::Erase | BrushTool::Scatter => {
                let mut cells: Vec<(i32, i32)> =
                    line_cells(from, to).into_iter().flat_map(stamp).collect();
                cells.sort_unstable();
                cells.dedup();
                if brush.tool == BrushTool::Scatter {
                    let density = brush.density.clamp(0.0, 1.0);
                    cells.retain(|&(x, y)| cell_noise(brush.seed, x, y, 0) < density);
                }
                cells
            }
            BrushTool::Line => line_cells(from, to),
            BrushTool::Rect => {
                let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                let mut cells = Vec::new();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        cells.push((x, y));
                    }
                }
                cells
            }
            BrushTool::Fill => self.flood(to),
        };
        for (x, y) in cells {
            let tile = if erase {
                -1
            } else {
                match self.brush_tile(brush, x, y) {
                    Some(tile) => tile,
                    None => continue,
                }
            };
            if self.put(x, y, tile) {
                touched.push((x, y));
            }
        }
        if !touched.is_empty() && !self.autotiles.is_empty() {
            let before = self.tiles.clone();
            self.retile(&touched);
            for (index, (old, new)) in before.iter().zip(&self.tiles).enumerate() {
                let cell = (
                    (index as u32 % self.width) as i32,
                    (index as u32 / self.width) as i32,
                );
                if old != new && !touched.contains(&cell) {
                    touched.push(cell);
                }
            }
        }
        touched
    }

    /// The cells connected to `start` (4-way) showing the same tile.
    fn flood(&self, start: (i32, i32)) -> Vec<(i32, i32)> {
        let Some(want) = self.at(start.0, start.1) else {
            return Vec::new();
        };
        let mut seen = vec![false; self.tiles.len()];
        let mut stack = vec![start];
        let mut out = Vec::new();
        while let Some((x, y)) = stack.pop() {
            if self.at(x, y) != Some(want) {
                continue;
            }
            let index = (y as u32 * self.width + x as u32) as usize;
            if std::mem::replace(&mut seen[index], true) {
                continue;
            }
            out.push((x, y));
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
        out
    }

    /// The cell under a point in the actor's own frame (centered, y up), or
    /// `None` off the map.
    pub fn cell_at_local(&self, point: [f32; 2]) -> Option<(u32, u32)> {
        let (x, y) = self.cell_at_local_unclamped(point);
        (x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height)
            .then_some((x as u32, y as u32))
    }

    /// [`Tilemap::cell_at_local`] without the bounds check, for a stroke
    /// that wanders off the edge.
    pub fn cell_at_local_unclamped(&self, point: [f32; 2]) -> (i32, i32) {
        let [w, h] = self.size();
        let [tw, th] = self.tile_size;
        (
            ((point[0] + w / 2.0) / tw.max(1e-3)).floor() as i32,
            ((h / 2.0 - point[1]) / th.max(1e-3)).floor() as i32,
        )
    }

    /// The cells a box in the actor's own frame covers, clipped to the map.
    pub fn cells_in_local(&self, min: [f32; 2], max: [f32; 2]) -> Vec<(u32, u32)> {
        let (x0, y0) = self.cell_at_local_unclamped([min[0], max[1]]);
        let (x1, y1) = self.cell_at_local_unclamped([max[0], min[1]]);
        let (w, h) = (self.width as i32, self.height as i32);
        let (x0, x1) = (x0.max(0), x1.min(w - 1));
        let (y0, y1) = (y0.max(0), y1.min(h - 1));
        let mut cells = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                cells.push((x as u32, y as u32));
            }
        }
        cells
    }

    /// A cell's centre in the actor's own frame.
    pub fn cell_center_local(&self, x: u32, y: u32) -> [f32; 2] {
        let [w, h] = self.size();
        let [tw, th] = self.tile_size;
        [
            x as f32 * tw + tw / 2.0 - w / 2.0,
            h / 2.0 - y as f32 * th - th / 2.0,
        ]
    }

    pub fn region_of(&self, tile: i32) -> Option<RegionKind> {
        (tile >= 0)
            .then(|| self.regions.iter().find(|region| region.tile == tile))
            .flatten()
            .map(|region| region.kind)
    }

    /// The region under a point in the actor's own frame.
    pub fn region_at_local(&self, point: [f32; 2]) -> Option<RegionKind> {
        let (x, y) = self.cell_at_local(point)?;
        self.region_of(self.tile_at(x, y)?)
    }

    /// One region kind's cells, merged into rectangles.
    pub fn region_rects(&self, kind: RegionKind) -> Vec<TileRect> {
        self.merged_rects(|tile| self.region_of(tile) == Some(kind))
    }

    /// The first cell of a region kind, row-major: where a map's spawn is.
    pub fn first_region_cell(&self, kind: RegionKind) -> Option<(u32, u32)> {
        let index = self
            .tiles
            .iter()
            .position(|&tile| self.region_of(tile) == Some(kind))?;
        Some((index as u32 % self.width, index as u32 / self.width))
    }

    pub fn stats(&self) -> TileStats {
        let tiles = self.tiles.iter().filter(|&&tile| tile >= 0).count();
        TileStats {
            tiles,
            draw_batches: usize::from(tiles > 0),
            colliding_rects: self.solid_rects().len(),
            animated: self
                .tiles
                .iter()
                .filter(|&&tile| self.animations.iter().any(|a| a.tile == tile))
                .count(),
            region_tiles: self
                .tiles
                .iter()
                .filter(|&&tile| self.region_of(tile).is_some())
                .count(),
        }
    }

    /// Takes a tileset's sheet, flags, animations and autotiles, keeping
    /// every painted cell that still lands in the sheet.
    pub fn apply_import(&mut self, import: &TilesetImport) {
        self.tileset = import.image.clone();
        self.tile_size = import.tile_size;
        self.sheet_columns = import.columns.max(1);
        self.sheet_rows = import.rows.max(1);
        self.solid = import.solid;
        self.passable = import.passable.clone();
        self.animations = import.animations.clone();
        if !import.autotiles.is_empty() {
            self.autotiles = import.autotiles.clone();
        }
        if !import.regions.is_empty() {
            self.regions = import.regions.clone();
        }
        self.normalize();
    }
}

// ─── Tiled import ──────────────────────────────────────────────────────────

/// A tileset as read from a Tiled JSON tileset (`.tsj`/`.json`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TilesetImport {
    /// The image, as the file spells it: relative to the tileset file.
    pub image: String,
    pub tile_size: [f32; 2],
    pub columns: u32,
    pub rows: u32,
    /// True when any tile carries a collision shape.
    pub solid: bool,
    /// Tiles a solid set lets through: those without a collision shape, or
    /// marked `passable` / `solid: false`.
    pub passable: Vec<i32>,
    pub animations: Vec<TileAnimation>,
    /// Edge and mixed wang sets, as edge and blob autotiles.
    pub autotiles: Vec<AutotileSet>,
    /// Tiles with a `region` property naming a [`RegionKind`].
    pub regions: Vec<TileRegion>,
    /// What didn't come across, for the editor to say.
    pub skipped: Vec<String>,
}

/// Reads a Tiled JSON tileset. Collision shapes, `passable`/`solid`
/// properties, animations, `region` properties and edge or mixed wang sets
/// come across; corner wang sets are listed in `skipped`.
pub fn import_tiled_tileset(json: &str) -> Result<TilesetImport, String> {
    let doc: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("not a Tiled JSON tileset: {error}"))?;
    let image = doc["image"]
        .as_str()
        .ok_or("the tileset has no single image (image collections aren't supported)")?
        .to_string();
    let tile_w = doc["tilewidth"]
        .as_f64()
        .ok_or("the tileset has no tilewidth")? as f32;
    let tile_h = doc["tileheight"]
        .as_f64()
        .ok_or("the tileset has no tileheight")? as f32;
    let columns = doc["columns"]
        .as_u64()
        .filter(|&c| c > 0)
        .ok_or("the tileset has no columns")? as u32;
    let count = doc["tilecount"].as_u64().unwrap_or(columns as u64) as u32;
    let rows = count.div_ceil(columns).max(1);
    let mut import = TilesetImport {
        image,
        tile_size: [tile_w.max(1.0), tile_h.max(1.0)],
        columns,
        rows,
        ..TilesetImport::default()
    };
    let mut colliding = std::collections::HashSet::new();
    let mut open = Vec::new();
    for tile in doc["tiles"].as_array().into_iter().flatten() {
        let Some(id) = tile["id"].as_i64() else {
            continue;
        };
        let id = id as i32;
        let has_shape = tile["objectgroup"]["objects"]
            .as_array()
            .is_some_and(|objects| !objects.is_empty());
        if has_shape {
            colliding.insert(id);
        }
        for property in tile["properties"].as_array().into_iter().flatten() {
            let name = property["name"].as_str().unwrap_or_default().to_lowercase();
            let value = &property["value"];
            match name.as_str() {
                "passable" if value.as_bool() == Some(true) => open.push(id),
                "solid" if value.as_bool() == Some(false) => open.push(id),
                "solid" if value.as_bool() == Some(true) => {
                    colliding.insert(id);
                }
                "region" => match value.as_str().and_then(RegionKind::parse) {
                    Some(kind) => import.regions.push(TileRegion { tile: id, kind }),
                    None => import
                        .skipped
                        .push(format!("tile {id}: unknown region {value}")),
                },
                _ => {}
            }
        }
        if let Some(frames) = tile["animation"].as_array()
            && !frames.is_empty()
        {
            let ids: Vec<i32> = frames
                .iter()
                .filter_map(|frame| frame["tileid"].as_i64().map(|t| t as i32))
                .collect();
            let total: f64 = frames
                .iter()
                .map(|frame| frame["duration"].as_f64().unwrap_or(100.0))
                .sum();
            let mean = (total / frames.len() as f64).max(1.0);
            import.animations.push(TileAnimation {
                tile: id,
                frames: ids,
                fps: (1000.0 / mean) as f32,
            });
        }
    }
    import.solid = !colliding.is_empty();
    if import.solid {
        let cells = (columns * rows) as i32;
        import.passable = (0..cells)
            .filter(|id| !colliding.contains(id) || open.contains(id))
            .collect();
    }
    for set in doc["wangsets"].as_array().into_iter().flatten() {
        let name = set["name"].as_str().unwrap_or("terrain").to_string();
        let kind = set["type"].as_str().unwrap_or("corner");
        let mode = match kind {
            "edge" => AutotileMode::Edge,
            "mixed" => AutotileMode::Blob,
            _ => {
                import
                    .skipped
                    .push(format!("wang set \"{name}\": {kind} sets aren't supported"));
                continue;
            }
        };
        let mut rules = Vec::new();
        for tile in set["wangtiles"].as_array().into_iter().flatten() {
            let (Some(id), Some(wang)) = (tile["tileid"].as_i64(), tile["wangid"].as_array())
            else {
                continue;
            };
            // Tiled's wangid runs top, top-right, right, ... top-left: the
            // same order as the blob bits.
            let on = |index: usize| wang.get(index).and_then(|v| v.as_u64()).unwrap_or(0) != 0;
            let mask = match mode {
                AutotileMode::Edge => {
                    u8::from(on(0))
                        | u8::from(on(2)) << 1
                        | u8::from(on(4)) << 2
                        | u8::from(on(6)) << 3
                }
                AutotileMode::Blob => {
                    reduce_blob((0..8).fold(0u8, |mask, bit| mask | u8::from(on(bit)) << bit))
                }
            };
            if !rules.iter().any(|rule: &AutotileRule| rule.mask == mask) {
                rules.push(AutotileRule {
                    mask,
                    tile: id as i32,
                });
            }
        }
        if !rules.is_empty() {
            import.autotiles.push(AutotileSet { name, mode, rules });
        }
    }
    Ok(import)
}

// ─── Parallax layers ───────────────────────────────────────────────────────

/// A layer that scrolls at its own rate against the camera. It sorts behind
/// or in front of actors by its Render layer, like any other actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParallaxSpec {
    /// How far the layer moves per unit the camera moves, 0-2 per axis: 0
    /// hangs on the camera like a sky, 1 is the actors' own plane, above 1
    /// sweeps past as foreground.
    pub scroll: [f32; 2],
    /// Repeats along x and y, so the layer never runs out.
    pub wrap: [bool; 2],
    /// 0-1: how far a distant layer fades towards the background color,
    /// scaled by its distance from the actors' plane (|1 - scroll|).
    pub dim: f32,
}

impl Default for ParallaxSpec {
    fn default() -> Self {
        Self {
            scroll: [0.5, 0.5],
            wrap: [false, false],
            dim: 0.0,
        }
    }
}

impl ParallaxSpec {
    pub fn normalize(&mut self) {
        for axis in &mut self.scroll {
            *axis = if axis.is_finite() {
                axis.clamp(0.0, 2.0)
            } else {
                1.0
            };
        }
        self.dim = self.dim.clamp(0.0, 1.0);
    }

    /// Where the layer draws given where it stands, how far the camera has
    /// moved from where the world started it, the camera now and the size
    /// of one repeat.
    pub fn drawn_at(
        &self,
        authored: [f32; 2],
        camera_moved: [f32; 2],
        camera: [f32; 2],
        size: [f32; 2],
    ) -> [f32; 2] {
        let mut out = [0.0; 2];
        for axis in 0..2 {
            let mut at = authored[axis] + camera_moved[axis] * (1.0 - self.scroll[axis]);
            if self.wrap[axis] && size[axis] > 1e-3 {
                let d = at - camera[axis];
                at -= size[axis] * (d / size[axis]).round();
            }
            out[axis] = at;
        }
        out
    }

    /// How far to fade towards the background, 0-1.
    pub fn dimming(&self) -> f32 {
        let far = ((1.0 - self.scroll[0]).abs() + (1.0 - self.scroll[1]).abs()) / 2.0;
        (self.dim * far.min(1.0)).clamp(0.0, 1.0)
    }
}

/// Which scroll factor `set parallax` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ParallaxAxis {
    #[default]
    Both,
    X,
    Y,
}

impl ParallaxAxis {
    pub fn name(self) -> &'static str {
        match self {
            ParallaxAxis::Both => "Both",
            ParallaxAxis::X => "X",
            ParallaxAxis::Y => "Y",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "both" | "" => Some(ParallaxAxis::Both),
            "x" => Some(ParallaxAxis::X),
            "y" => Some(ParallaxAxis::Y),
            _ => None,
        }
    }

    pub fn apply(self, scroll: &mut [f32; 2], value: f32) {
        let value = if value.is_finite() {
            value.clamp(0.0, 2.0)
        } else {
            1.0
        };
        match self {
            ParallaxAxis::Both => *scroll = [value; 2],
            ParallaxAxis::X => scroll[0] = value,
            ParallaxAxis::Y => scroll[1] = value,
        }
    }
}

// ─── Rooms ─────────────────────────────────────────────────────────────────

/// A room: a rectangle of the level centred on the actor. The camera stays
/// inside the room its target stands in, and entering one fires `when
/// actor enters room`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoomSpec {
    /// World units across and up, before the actor's scale.
    pub size: [f32; 2],
    /// Keeps the camera inside the room while its target is in it.
    pub camera: bool,
    /// Seconds the camera takes to slide into a newly entered room.
    pub blend: f32,
    /// Builds the tilemaps inside it only while it is near the camera,
    /// through the streaming cells.
    pub stream: bool,
}

impl Default for RoomSpec {
    fn default() -> Self {
        Self {
            size: [1280.0, 720.0],
            camera: true,
            blend: 0.4,
            stream: false,
        }
    }
}

impl RoomSpec {
    pub fn normalize(&mut self) {
        for side in &mut self.size {
            *side = if side.is_finite() { side.max(1.0) } else { 1.0 };
        }
        self.blend = if self.blend.is_finite() {
            self.blend.clamp(0.0, 5.0)
        } else {
            0.0
        };
    }

    /// The room's world rectangle for an actor at `center` scaled by
    /// `scale`. Rotation is ignored: rooms are axis-aligned.
    pub fn bounds(&self, center: [f32; 2], scale: [f32; 2]) -> RoomBounds {
        let half = [
            self.size[0] * scale[0].abs() / 2.0,
            self.size[1] * scale[1].abs() / 2.0,
        ];
        RoomBounds {
            min: [center[0] - half[0], center[1] - half[1]],
            max: [center[0] + half[0], center[1] + half[1]],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoomBounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl RoomBounds {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&point[0])
            && (self.min[1]..=self.max[1]).contains(&point[1])
    }

    pub fn area(&self) -> f32 {
        (self.max[0] - self.min[0]) * (self.max[1] - self.min[1])
    }

    pub fn overlaps(&self, min: [f32; 2], max: [f32; 2]) -> bool {
        self.min[0] <= max[0]
            && min[0] <= self.max[0]
            && self.min[1] <= max[1]
            && min[1] <= self.max[1]
    }

    /// A camera centre moved so a view of `half` half-extents stays inside
    /// the room, or centred on it along an axis the view is wider than.
    pub fn confine(&self, center: [f32; 2], half: [f32; 2]) -> [f32; 2] {
        let mut out = center;
        for axis in 0..2 {
            let (lo, hi) = (self.min[axis] + half[axis], self.max[axis] - half[axis]);
            out[axis] = if lo > hi {
                (self.min[axis] + self.max[axis]) / 2.0
            } else {
                center[axis].clamp(lo, hi)
            };
        }
        out
    }
}

// ─── What blocks read ──────────────────────────────────────────────────────

/// One live tilemap: where it stands and what it shows right now.
#[derive(Debug, Clone)]
pub struct TilemapSense {
    pub id: String,
    pub name: String,
    pub center: [f32; 2],
    pub scale: [f32; 2],
    pub map: Arc<Tilemap>,
}

impl TilemapSense {
    /// A world point in the map's own frame.
    pub fn local(&self, point: [f32; 2]) -> [f32; 2] {
        [
            (point[0] - self.center[0]) / self.scale[0].abs().max(1e-6),
            (point[1] - self.center[1]) / self.scale[1].abs().max(1e-6),
        ]
    }

    pub fn world(&self, local: [f32; 2]) -> [f32; 2] {
        [
            self.center[0] + local[0] * self.scale[0].abs(),
            self.center[1] + local[1] * self.scale[1].abs(),
        ]
    }

    pub fn cell_at(&self, point: [f32; 2]) -> Option<(u32, u32)> {
        self.map.cell_at_local(self.local(point))
    }

    pub fn answers_to(&self, wanted: &str) -> bool {
        self.id == wanted || self.name.eq_ignore_ascii_case(wanted)
    }

    /// The strongest region a world box touches (see [`RegionKind::rank`]),
    /// the cell nearest the box's centre among equals, with that cell's
    /// centre in world units.
    pub fn region_touching(&self, min: [f32; 2], max: [f32; 2]) -> Option<(RegionKind, [f32; 2])> {
        let (a, b) = (self.local(min), self.local(max));
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let mut best: Option<(RegionKind, f32, [f32; 2])> = None;
        for (x, y) in self.map.cells_in_local(lo, hi) {
            let Some(kind) = self.map.tile_at(x, y).and_then(|t| self.map.region_of(t)) else {
                continue;
            };
            let at = self.map.cell_center_local(x, y);
            let d = (at[0] - mid[0]).powi(2) + (at[1] - mid[1]).powi(2);
            let better = best.is_none_or(|(held, near, _)| {
                kind.rank() > held.rank() || (kind == held && d < near)
            });
            if better {
                best = Some((kind, d, at));
            }
        }
        best.map(|(kind, _, at)| (kind, self.world(at)))
    }
}

#[derive(Debug, Clone)]
pub struct RoomSense {
    pub id: String,
    pub name: String,
    pub bounds: RoomBounds,
}

/// The level as the blocks see it this frame.
#[derive(Debug, Clone, Default)]
pub struct LevelSense {
    pub tilemaps: Vec<TilemapSense>,
    pub rooms: Vec<RoomSense>,
    /// Actor id -> the room name it entered on the last fixed tick.
    pub entered: std::collections::HashMap<String, String>,
}

impl LevelSense {
    /// The tile at a world point: in the named map (empty for any, the first
    /// map with a tile there winning). `-1` for an empty cell or no map
    /// there; `Err` for a map name nothing answers to.
    pub fn tile_at(&self, point: [f32; 2], map: &str) -> Result<i32, String> {
        let map = map.trim();
        if !map.is_empty() {
            let found = self
                .tilemaps
                .iter()
                .find(|sense| sense.answers_to(map))
                .ok_or_else(|| format!("there's no tilemap named \"{map}\""))?;
            return Ok(found
                .cell_at(point)
                .and_then(|(x, y)| found.map.tile_at(x, y))
                .unwrap_or(-1));
        }
        Ok(self
            .tilemaps
            .iter()
            .filter_map(|sense| {
                let (x, y) = sense.cell_at(point)?;
                sense.map.tile_at(x, y).filter(|&tile| tile >= 0)
            })
            .next()
            .unwrap_or(-1))
    }

    /// The region under a world point, the first map with one winning.
    pub fn region_at(&self, point: [f32; 2]) -> Option<(RegionKind, &TilemapSense)> {
        self.tilemaps.iter().find_map(|sense| {
            let kind = sense.map.region_at_local(sense.local(point))?;
            Some((kind, sense))
        })
    }

    /// [`TilemapSense::region_touching`] over every map, the strongest winning
    /// (the first map among equals).
    pub fn region_touching(&self, min: [f32; 2], max: [f32; 2]) -> Option<(RegionKind, [f32; 2])> {
        region_touching(self.tilemaps.iter(), min, max)
    }

    /// The smallest room holding a point (the first of equals).
    pub fn room_at(&self, point: [f32; 2]) -> Option<&RoomSense> {
        smallest_room(&self.rooms, point)
    }
}

/// The strongest region a world box touches across `maps`.
pub fn region_touching<'a>(
    maps: impl Iterator<Item = &'a TilemapSense>,
    min: [f32; 2],
    max: [f32; 2],
) -> Option<(RegionKind, [f32; 2])> {
    maps.filter_map(|sense| sense.region_touching(min, max))
        .fold(
            None,
            |best: Option<(RegionKind, [f32; 2])>, found| match best {
                Some(best) if best.0.rank() >= found.0.rank() => Some(best),
                _ => Some(found),
            },
        )
}

/// The smallest room holding `point`, so a nested room wins over its hall.
pub fn smallest_room(rooms: &[RoomSense], point: [f32; 2]) -> Option<&RoomSense> {
    rooms
        .iter()
        .filter(|room| room.bounds.contains(point))
        .fold(None, |best: Option<&RoomSense>, room| match best {
            Some(best) if best.bounds.area() <= room.bounds.area() => Some(best),
            _ => Some(room),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(width: u32, height: u32) -> Tilemap {
        let mut map = Tilemap {
            sheet_columns: 8,
            sheet_rows: 8,
            ..Tilemap::default()
        };
        map.resize(width, height);
        map
    }

    #[test]
    fn there_are_47_blob_cases() {
        let masks = blob_masks();
        assert_eq!(masks.len(), 47);
        assert_eq!(masks[0], 0);
        assert_eq!(*masks.last().unwrap(), 255);
        // A corner alone is not a case.
        assert_eq!(reduce_blob(NE), 0);
        assert_eq!(reduce_blob(N | E | NE), N | E | NE);
    }

    #[test]
    fn edge_autotile_picks_by_neighbours() {
        let mut m = map(5, 5);
        m.autotiles.push(AutotileSet::edge_strip("ground", 16));
        let brush = TileBrush {
            autotile: "ground".into(),
            ..TileBrush::default()
        };
        // A horizontal bar across the middle row, from wall to wall.
        m.apply_brush(
            &TileBrush {
                tool: BrushTool::Line,
                ..brush.clone()
            },
            (0, 2),
            (4, 2),
        );
        // The middle cell has east and west neighbours; north and south are
        // empty: mask E|W = 2|8 = 10.
        assert_eq!(m.tile_at(2, 2), Some(16 + 10));
        // The border counts as filled, so the ends read west (or east) too.
        assert_eq!(m.tile_at(0, 2), Some(16 + 10));
        // Erasing the middle re-resolves its neighbours.
        m.apply_brush(
            &TileBrush {
                tool: BrushTool::Erase,
                ..brush
            },
            (2, 2),
            (2, 2),
        );
        assert_eq!(m.tile_at(2, 2), Some(-1));
        assert_eq!(m.tile_at(1, 2), Some(16 + 8));
        assert_eq!(m.tile_at(3, 2), Some(16 + 2));
    }

    #[test]
    fn blob_autotile_counts_corners_between_edges() {
        let mut m = map(4, 4);
        m.autotiles.push(AutotileSet::blob_strip("grass", 0));
        let brush = TileBrush {
            tool: BrushTool::Rect,
            autotile: "grass".into(),
            ..TileBrush::default()
        };
        m.apply_brush(&brush, (0, 0), (3, 3));
        // A full map: every cell sees all eight neighbours (the edge counts).
        let full = blob_masks().iter().position(|&m| m == 255).unwrap() as i32;
        assert!(m.tiles.iter().all(|&tile| tile == full));
    }

    #[test]
    fn fill_flood_stops_at_other_tiles() {
        let mut m = map(4, 1);
        m.tiles = vec![1, 1, 2, 1];
        let brush = TileBrush {
            tool: BrushTool::Fill,
            tiles: vec![5],
            ..TileBrush::default()
        };
        let changed = m.apply_brush(&brush, (0, 0), (0, 0));
        assert_eq!(changed.len(), 2);
        assert_eq!(m.tiles, vec![5, 5, 2, 1]);
    }

    #[test]
    fn scatter_honours_density_and_replays() {
        let brush = TileBrush {
            tool: BrushTool::Scatter,
            tiles: vec![3, 4],
            size: 16,
            density: 0.25,
            jitter: 1.0,
            seed: 7,
            ..TileBrush::default()
        };
        let mut a = map(16, 16);
        let mut b = map(16, 16);
        a.apply_brush(&brush, (7, 7), (7, 7));
        b.apply_brush(&brush, (7, 7), (7, 7));
        assert_eq!(a.tiles, b.tiles);
        let filled = a.tiles.iter().filter(|&&t| t >= 0).count();
        assert!((30..100).contains(&filled), "{filled} of 256");
        assert!(a.tiles.contains(&3) && a.tiles.contains(&4));
    }

    #[test]
    fn painting_a_frame_paints_its_animation() {
        let mut m = map(2, 1);
        m.animations.push(TileAnimation {
            tile: 10,
            frames: vec![10, 11, 12],
            fps: 4.0,
        });
        let brush = TileBrush {
            tiles: vec![12],
            ..TileBrush::default()
        };
        m.apply_brush(&brush, (0, 0), (1, 0));
        assert_eq!(m.tiles, vec![10, 10]);
    }

    #[test]
    fn cells_and_regions_read_in_the_actors_frame() {
        let mut m = map(4, 2);
        m.tile_size = [10.0, 10.0];
        m.regions.push(TileRegion {
            tile: 7,
            kind: RegionKind::Kill,
        });
        m.set_tile(3, 1, 7);
        // The map spans x -20..20 and y -10..10; (3, 1) is bottom-right.
        assert_eq!(m.cell_at_local([15.0, -5.0]), Some((3, 1)));
        assert_eq!(m.cell_center_local(3, 1), [15.0, -5.0]);
        assert_eq!(m.region_at_local([15.0, -5.0]), Some(RegionKind::Kill));
        assert_eq!(m.cell_at_local([25.0, 0.0]), None);
        assert_eq!(m.region_rects(RegionKind::Kill).len(), 1);
        assert_eq!(m.first_region_cell(RegionKind::Kill), Some((3, 1)));
    }

    #[test]
    fn a_body_box_touches_the_strongest_region() {
        let mut m = map(4, 2);
        m.tile_size = [10.0, 10.0];
        m.regions.push(TileRegion {
            tile: 7,
            kind: RegionKind::Kill,
        });
        m.regions.push(TileRegion {
            tile: 5,
            kind: RegionKind::Water,
        });
        m.set_tile(3, 1, 7);
        m.set_tile(2, 1, 5);
        let sense = TilemapSense {
            id: "m".into(),
            name: "Map".into(),
            center: [100.0, 0.0],
            scale: [1.0, 1.0],
            map: Arc::new(m),
        };
        // Centre over the water cell, edge over the kill cell: the kill wins.
        assert_eq!(
            sense.region_touching([103.0, -8.0], [111.0, -2.0]),
            Some((RegionKind::Kill, [115.0, -5.0]))
        );
        // Wholly over water.
        assert_eq!(
            sense.region_touching([101.0, -8.0], [109.0, -2.0]),
            Some((RegionKind::Water, [105.0, -5.0]))
        );
        // Above both.
        assert_eq!(sense.region_touching([101.0, 2.0], [119.0, 8.0]), None);
        // Off the map altogether.
        assert_eq!(sense.region_touching([200.0, 0.0], [210.0, 5.0]), None);
    }

    #[test]
    fn stats_count_tiles_and_rects() {
        let mut m = map(4, 2);
        m.solid = true;
        m.tiles = vec![0, 0, -1, 0, 0, 0, -1, -1];
        let stats = m.stats();
        assert_eq!(stats.tiles, 5);
        assert_eq!(stats.draw_batches, 1);
        assert_eq!(stats.colliding_rects, 2);
    }

    #[test]
    fn a_tiled_tileset_brings_its_flags() {
        let json = r#"{
            "image": "tiles.png", "tilewidth": 16, "tileheight": 16,
            "columns": 4, "tilecount": 8,
            "tiles": [
                {"id": 0, "objectgroup": {"objects": [{"x":0}]}},
                {"id": 1, "objectgroup": {"objects": [{"x":0}]},
                 "properties": [{"name":"passable","type":"bool","value":true}]},
                {"id": 2, "animation": [{"tileid":2,"duration":100},{"tileid":3,"duration":100}]},
                {"id": 5, "properties": [{"name":"region","type":"string","value":"ladder"}]}
            ],
            "wangsets": [
                {"name":"dirt","type":"edge","wangtiles":[
                    {"tileid":4,"wangid":[1,0,1,0,1,0,1,0]},
                    {"tileid":6,"wangid":[0,0,1,0,0,0,1,0]}]},
                {"name":"hills","type":"corner","wangtiles":[]}
            ]
        }"#;
        let import = import_tiled_tileset(json).unwrap();
        assert_eq!(import.columns, 4);
        assert_eq!(import.rows, 2);
        assert!(import.solid);
        assert!(!import.passable.contains(&0));
        assert!(import.passable.contains(&1));
        assert!(import.passable.contains(&2));
        assert_eq!(import.animations[0].frames, vec![2, 3]);
        assert!((import.animations[0].fps - 10.0).abs() < 1e-4);
        assert_eq!(import.regions[0].kind, RegionKind::Ladder);
        assert_eq!(import.autotiles[0].rules.len(), 2);
        assert_eq!(import.autotiles[0].pick(15), Some(4));
        assert_eq!(import.autotiles[0].pick(2 | 8), Some(6));
        assert_eq!(import.skipped.len(), 1);

        let mut m = map(2, 2);
        m.set_tile(0, 0, 3);
        m.apply_import(&import);
        assert_eq!(m.tile_at(0, 0), Some(3));
        assert_eq!(m.tile_size, [16.0, 16.0]);
    }

    #[test]
    fn parallax_scrolls_and_wraps() {
        let far = ParallaxSpec {
            scroll: [0.0, 1.0],
            ..ParallaxSpec::default()
        };
        // Scroll 0 rides the camera; scroll 1 stays put.
        assert_eq!(
            far.drawn_at([0.0, 0.0], [100.0, 100.0], [100.0, 100.0], [0.0, 0.0]),
            [100.0, 0.0]
        );
        let wrapped = ParallaxSpec {
            scroll: [1.0, 1.0],
            wrap: [true, false],
            ..ParallaxSpec::default()
        };
        // A 200-wide layer the camera has left behind jumps to stay nearest.
        let at = wrapped.drawn_at([0.0, 0.0], [0.0, 0.0], [450.0, 0.0], [200.0, 0.0]);
        assert_eq!(at, [400.0, 0.0]);
        assert_eq!(ParallaxSpec::default().dimming(), 0.0);
        let dim = ParallaxSpec {
            scroll: [0.0, 0.0],
            dim: 0.5,
            ..ParallaxSpec::default()
        };
        assert!((dim.dimming() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rooms_confine_and_nest() {
        let hall = RoomSense {
            id: "a".into(),
            name: "Hall".into(),
            bounds: RoomSpec::default().bounds([0.0, 0.0], [1.0, 1.0]),
        };
        let closet = RoomSense {
            id: "b".into(),
            name: "Closet".into(),
            bounds: RoomSpec {
                size: [100.0, 100.0],
                ..RoomSpec::default()
            }
            .bounds([0.0, 0.0], [1.0, 1.0]),
        };
        let rooms = [hall.clone(), closet.clone()];
        assert_eq!(smallest_room(&rooms, [10.0, 10.0]).unwrap().name, "Closet");
        assert_eq!(smallest_room(&rooms, [500.0, 10.0]).unwrap().name, "Hall");
        assert!(smallest_room(&rooms, [5000.0, 10.0]).is_none());
        // The hall is 1280 wide; a 400-wide view can't see past its edge.
        assert_eq!(
            hall.bounds.confine([1000.0, 0.0], [200.0, 100.0]),
            [440.0, 0.0]
        );
        // The closet is narrower than the view: centre on it.
        assert_eq!(
            closet.bounds.confine([30.0, 0.0], [200.0, 100.0]),
            [0.0, 0.0]
        );
    }

    #[test]
    fn tile_at_reads_live_maps_by_world_point() {
        let mut m = map(2, 2);
        m.tile_size = [10.0, 10.0];
        m.set_tile(1, 0, 4);
        let level = LevelSense {
            tilemaps: vec![TilemapSense {
                id: "m1".into(),
                name: "Ground".into(),
                center: [100.0, 0.0],
                scale: [1.0, 1.0],
                map: Arc::new(m),
            }],
            ..Default::default()
        };
        assert_eq!(level.tile_at([105.0, 5.0], ""), Ok(4));
        assert_eq!(level.tile_at([95.0, 5.0], "ground"), Ok(-1));
        assert_eq!(level.tile_at([0.0, 0.0], ""), Ok(-1));
        assert!(level.tile_at([0.0, 0.0], "Sky").is_err());
    }
}
