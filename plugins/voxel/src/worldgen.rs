//! Infinite procedural worlds: editable noise, biomes and features.
//!
//! Generation is canonical: every cell asks only a hash of its own
//! coordinates plus the config, so paging order never changes the world.
//! Chunks are 32 cells (`SECTION`); render distances are in blocks and must
//! be multiples of 32.

/// One noise layer: value-noise fbm over x/z, scaled by amplitude.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseLayer {
    pub name: String,
    pub scale: f64,
    pub octaves: u32,
    pub amplitude: f64,
    pub seed: i64,
}

impl NoiseLayer {
    pub fn sample(&self, base_seed: u32, x: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut freq = 1.0;
        let mut norm = 0.0;
        for o in 0..self.octaves {
            let s = base_seed
                .wrapping_add(self.seed as u32)
                .wrapping_add(o.wrapping_mul(101));
            sum += amp * noise3(s, x / self.scale * freq, 0.5, z / self.scale * freq);
            norm += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        (sum / norm.max(1e-6)) * self.amplitude
    }
}

/// One biome: a temperature/humidity rectangle plus surface and features.
#[derive(Debug, Clone, PartialEq)]
pub struct Biome {
    pub name: String,
    pub temp: (f64, f64),
    pub humidity: (f64, f64),
    pub surface: String,
    pub subsurface: String,
    pub hill_mult: f64,
    pub tree_density: u32,
    pub rock_density: u32,
    pub pond_density: u32,
}

/// One modular feature: trees, rocks, ponds (more kinds later).
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub kind: String,
    pub biome: String,
    pub density: u32,
    pub size: i32,
    pub material: String,
}

/// The whole infinite config, parsed from text lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Worldgen {
    pub layers: Vec<NoiseLayer>,
    pub biomes: Vec<Biome>,
    pub features: Vec<Feature>,
    pub base_height: i32,
    pub sea_level: i32,
}

impl Default for Worldgen {
    fn default() -> Worldgen {
        Worldgen {
            layers: vec![
                NoiseLayer {
                    name: "continent".into(),
                    scale: 220.0,
                    octaves: 4,
                    amplitude: 22.0,
                    seed: 11,
                },
                NoiseLayer {
                    name: "hills".into(),
                    scale: 48.0,
                    octaves: 3,
                    amplitude: 7.0,
                    seed: 77,
                },
                NoiseLayer {
                    name: "detail".into(),
                    scale: 13.0,
                    octaves: 2,
                    amplitude: 2.0,
                    seed: 129,
                },
            ],
            biomes: vec![
                Biome {
                    name: "rocky".into(),
                    temp: (0.0, 0.35),
                    humidity: (0.0, 0.35),
                    surface: "stone".into(),
                    subsurface: "stone".into(),
                    hill_mult: 1.8,
                    tree_density: 0,
                    rock_density: 90,
                    pond_density: 800,
                },
                Biome {
                    name: "desert".into(),
                    temp: (0.55, 1.0),
                    humidity: (0.0, 0.45),
                    surface: "sand".into(),
                    subsurface: "sand".into(),
                    hill_mult: 0.6,
                    tree_density: 0,
                    rock_density: 200,
                    pond_density: 900,
                },
                Biome {
                    name: "forest".into(),
                    temp: (0.0, 0.55),
                    humidity: (0.5, 1.0),
                    surface: "grass".into(),
                    subsurface: "dirt".into(),
                    hill_mult: 1.2,
                    tree_density: 60,
                    rock_density: 400,
                    pond_density: 350,
                },
                Biome {
                    name: "plains".into(),
                    temp: (0.0, 1.0),
                    humidity: (0.0, 1.0),
                    surface: "grass".into(),
                    subsurface: "dirt".into(),
                    hill_mult: 1.0,
                    tree_density: 170,
                    rock_density: 320,
                    pond_density: 500,
                },
            ],
            features: Vec::new(),
            base_height: 20,
            sea_level: 8,
        }
    }
}

impl Worldgen {
    /// Parse `noise`, `biome` and `feature` lines. Empty means defaults.
    pub fn parse(
        noise: &[String],
        biomes: &[String],
        features: &[String],
    ) -> Result<Worldgen, String> {
        let mut wg = Worldgen::default();
        if !noise.is_empty() {
            wg.layers.clear();
            for (n, line) in noise.iter().enumerate() {
                wg.layers
                    .push(parse_noise(line).map_err(|why| format!("noise {}: {why}", n + 1))?);
            }
        }
        if !biomes.is_empty() {
            wg.biomes.clear();
            for (n, line) in biomes.iter().enumerate() {
                wg.biomes
                    .push(parse_biome(line).map_err(|why| format!("biome {}: {why}", n + 1))?);
            }
        }
        if !features.is_empty() {
            wg.features.clear();
            for (n, line) in features.iter().enumerate() {
                wg.features
                    .push(parse_feature(line).map_err(|why| format!("feature {}: {why}", n + 1))?);
            }
        }
        if wg.layers.is_empty() || wg.layers.len() > 8 {
            return Err("a world needs 1 to 8 noise layers".into());
        }
        if wg.biomes.is_empty() || wg.biomes.len() > 16 {
            return Err("a world needs 1 to 16 biomes".into());
        }
        if wg.features.len() > 64 {
            return Err("a world holds at most 64 features".into());
        }
        Ok(wg)
    }

    pub fn biome_at(&self, seed: u32, x: i32, z: i32) -> &Biome {
        let temp = fbm(seed ^ 0x70f1, x as f64 / 180.0, z as f64 / 180.0, 3);
        let hum = fbm(seed ^ 0x9017, x as f64 / 160.0, z as f64 / 160.0, 3);
        for biome in &self.biomes {
            if temp >= biome.temp.0
                && temp <= biome.temp.1
                && hum >= biome.humidity.0
                && hum <= biome.humidity.1
            {
                return biome;
            }
        }
        &self.biomes[0]
    }

    /// Surface height for a column, before trees and features.
    pub fn column_height(&self, seed: u32, x: i32, z: i32) -> i32 {
        let biome = self.biome_at(seed, x, z);
        self.height_for(biome, seed, x, z)
    }

    /// The same height with the biome already looked up.
    pub fn height_for(&self, biome: &Biome, seed: u32, x: i32, z: i32) -> i32 {
        let mut height = self.base_height as f64;
        for layer in &self.layers {
            height += layer.sample(seed, x as f64, z as f64) * biome.hill_mult;
        }
        height.round().clamp(2.0, 120.0) as i32
    }

    /// Exact tree trunk for a column (its own biome's density).
    /// Sampling inlines the same gate with the asking column's density,
    /// so borders may thin; this stays the canonical single-column query.
    #[allow(dead_code)]
    pub fn tree_at(&self, seed: u32, x: i32, z: i32) -> Option<(i32, i32)> {
        let biome = self.biome_at(seed, x, z);
        let density = self.tree_density_for(&biome.name);
        if density == 0 || !hash(seed ^ 0x77, x, 0, z).is_multiple_of(density) {
            return None;
        }
        let top = self.height_for(biome, seed, x, z);
        if top <= self.sea_level + 1 {
            return None;
        }
        let trunk = 4 + (hash(seed, x, 1, z) % 2) as i32;
        Some((top, top + trunk))
    }

    #[allow(dead_code)]
    fn pond_center(&self, seed: u32, x: i32, z: i32) -> Option<(i32, i32, i32)> {
        let biome = self.biome_at(seed, x, z);
        let density = self.pond_density_for(&biome.name);
        if density == 0 || !hash(seed ^ 0x90d, x, 0, z).is_multiple_of(density) {
            return None;
        }
        let radius = 2 + (hash(seed ^ 0x9a1, x, 3, z) % 3) as i32;
        Some((x, z, radius))
    }

    /// Feature density for one biome: a `feature` line naming it wins,
    /// else the biome's own dial.
    fn feature_density(&self, kind: &str, biome: &str, fallback: u32) -> u32 {
        self.features
            .iter()
            .find(|f| f.kind == kind && (f.biome == "any" || f.biome == biome))
            .map(|f| f.density)
            .unwrap_or(fallback)
    }

    pub fn tree_density_for(&self, biome: &str) -> u32 {
        let fallback = self
            .biomes
            .iter()
            .find(|b| b.name == biome)
            .map(|b| b.tree_density)
            .unwrap_or(170);
        self.feature_density("tree", biome, fallback)
    }

    pub fn pond_density_for(&self, biome: &str) -> u32 {
        let fallback = self
            .biomes
            .iter()
            .find(|b| b.name == biome)
            .map(|b| b.pond_density)
            .unwrap_or(500);
        self.feature_density("pond", biome, fallback)
    }

    pub fn rock_density_for(&self, biome: &str) -> u32 {
        let fallback = self
            .biomes
            .iter()
            .find(|b| b.name == biome)
            .map(|b| b.rock_density)
            .unwrap_or(320);
        self.feature_density("rock", biome, fallback)
    }
}

/// Seed helper: the same i64 seed always maps to one u32.
pub fn seed32(seed: i64) -> u32 {
    (seed as u64 ^ (seed as u64 >> 32)) as u32
}

/// A built-in material id. Custom blocks resolve through the registry map.
pub fn builtin_id(name: &str) -> u8 {
    match name {
        "stone" => 1,
        "dirt" => 2,
        "grass" => 3,
        "sand" => 4,
        "wood" => 5,
        "leaves" => 6,
        "glow" => 7,
        _ => 1,
    }
}

/// Surface height including ponds (depressed) for infinite worlds. Uses
/// the same density gating as sampling, so the bound always covers it.
pub fn height_bound_infinite(wg: &Worldgen, seed: i64, x: i32, z: i32) -> i32 {
    let seed = seed32(seed);
    let biome = wg.biome_at(seed, x, z);
    let mut top = wg.height_for(biome, seed, x, z);
    let tree_density = wg.tree_density_for(&biome.name);
    if tree_density != 0 {
        for dx in -2..=2 {
            for dz in -2..=2 {
                let (nx, nz) = (x + dx, z + dz);
                if !hash(seed ^ 0x77, nx, 0, nz).is_multiple_of(tree_density) {
                    continue;
                }
                let crown = wg.column_height(seed, nx, nz) + 4 + (hash(seed, nx, 1, nz) % 2) as i32;
                top = top.max(crown + 2);
            }
        }
    }
    top
}

/// One canonical infinite cell. `id_of` maps material names to ids.
#[cfg(test)]
pub fn sample_infinite(wg: &Worldgen, seed: i64, cell: [i32; 3], id_of: &dyn Fn(&str) -> u8) -> u8 {
    let seed = seed32(seed);
    let [x, y, z] = cell;
    if !(0..=128).contains(&y) {
        return 0;
    }
    let biome = wg.biome_at(seed, x, z);
    let top = wg.height_for(biome, seed, x, z);
    // Ponds: a sandy dish one deep where a pond center lands.
    let mut pond = false;
    let pond_density = wg.pond_density_for(&biome.name);
    if pond_density != 0 {
        for dx in -5..=5 {
            for dz in -5..=5 {
                let (nx, nz) = (x + dx, z + dz);
                if !hash(seed ^ 0x90d, nx, 0, nz).is_multiple_of(pond_density) {
                    continue;
                }
                // The gate uses this column's density, so border dishes may
                // shrink to pits; the dish itself is always canonical.
                let r = 2 + (hash(seed ^ 0x9a1, nx, 3, nz) % 3) as i32;
                if dx * dx + dz * dz <= r * r {
                    pond = true;
                }
            }
        }
    }
    let ground = if pond { top - 1 } else { top };
    if y <= ground {
        let depth = ground - y;
        if depth == 0 {
            return id_of(if pond { "sand" } else { &biome.surface });
        } else if depth <= 3 {
            return id_of(&biome.subsurface);
        }
        return id_of("stone");
    }
    // Rocks: a one-high stone knob on the surface.
    {
        let density = wg.rock_density_for(&biome.name);
        if y == ground + 1 && density != 0 && hash(seed ^ 0x20cc, x, 7, z).is_multiple_of(density) {
            return id_of("stone");
        }
    }
    // Trees: trunks over their column, leaves over neighbours. The gate
    // uses this column's density, so crowns at biome borders may thin;
    // every query still answers the same.
    let tree_density = wg.tree_density_for(&biome.name);
    if tree_density != 0 {
        if hash(seed ^ 0x77, x, 0, z).is_multiple_of(tree_density) {
            let trunk = 4 + (hash(seed, x, 1, z) % 2) as i32;
            if top > wg.sea_level + 1 && y > top && y <= top + trunk {
                return id_of("wood");
            }
        }
        for dx in -2..=2 {
            for dz in -2..=2 {
                let (nx, nz) = (x + dx, z + dz);
                if !hash(seed ^ 0x77, nx, 0, nz).is_multiple_of(tree_density) {
                    continue;
                }
                let crown = wg.column_height(seed, nx, nz) + 4 + (hash(seed, nx, 1, nz) % 2) as i32;
                let dy = y - crown;
                if (-1..=2).contains(&dy) && dx * dx + dz * dz + dy * dy * 2 <= 6 {
                    return id_of("leaves");
                }
            }
        }
    }
    0
}

/// Fill one 32-cell section of infinite terrain. Matches `sample_infinite`
/// cell for cell: strata from the column, then pond dishes, rock knobs,
/// trunks and leaves. Columns evaluate once per page; only the top layers
/// run the feature gates, so deep stone stays a strata read.
#[cfg(test)]
pub fn fill_infinite_page(
    wg: &Worldgen,
    id_of: &dyn Fn(&str) -> u8,
    seed: i64,
    chunk: [i32; 3],
    cells: &mut [u8; 32768],
) {
    fill_infinite_page_cached(
        wg,
        id_of,
        seed,
        chunk,
        cells,
        &mut std::collections::BTreeMap::new(),
    );
}

pub fn fill_infinite_page_cached(
    wg: &Worldgen,
    id_of: &dyn Fn(&str) -> u8,
    seed: i64,
    chunk: [i32; 3],
    cells: &mut [u8; 32768],
    columns: &mut std::collections::BTreeMap<[i32; 2], Column>,
) {
    let seed = seed32(seed);
    let [cx, cy, cz] = chunk;
    let tops = std::cell::RefCell::new([None; 36 * 36]);
    for lz in 0..32 {
        for lx in 0..32 {
            let (x, z) = (cx * 32 + lx, cz * 32 + lz);
            let address = [x, z];
            let column = if let Some(column) = columns.get(&address) {
                *column
            } else {
                let column = column(wg, seed, x, z, id_of, &|x, z| {
                    let index = ((z - cz * 32 + 2) * 36 + x - cx * 32 + 2) as usize;
                    *tops.borrow_mut()[index].get_or_insert_with(|| wg.column_height(seed, x, z))
                });
                if columns.len() == 4096 {
                    columns.pop_first();
                }
                columns.insert(address, column);
                column
            };
            for ly in 0..32 {
                cells[((lz * 32 + ly) * 32 + lx) as usize] = column.sample(cy * 32 + ly);
            }
        }
    }
}

/// Procedural strata and features for one column, reused at every height.
#[derive(Clone, Copy)]
pub struct Column {
    top: i32,
    ground: i32,
    trunk_end: i32,
    surface: u8,
    subsurface: u8,
    stone: u8,
    wood: u8,
    leaves: u8,
    rock: bool,
    crowns: [u64; 3],
}
impl Column {
    pub fn sample(&self, y: i32) -> u8 {
        if !(0..=128).contains(&y) {
            return 0;
        }
        if y <= self.ground {
            return match self.ground - y {
                0 => self.surface,
                1..=3 => self.subsurface,
                _ => self.stone,
            };
        }
        if y == self.ground + 1 && self.rock {
            return self.stone;
        }
        if y > self.top && y <= self.trunk_end {
            return self.wood;
        }
        if self.crowns[y as usize / 64] & (1 << (y as usize % 64)) != 0 {
            return self.leaves;
        }
        0
    }
}
pub fn column(
    wg: &Worldgen,
    seed: u32,
    x: i32,
    z: i32,
    id_of: &dyn Fn(&str) -> u8,
    height: &dyn Fn(i32, i32) -> i32,
) -> Column {
    let biome = wg.biome_at(seed, x, z);
    let top = height(x, z);
    let pond_density = wg.pond_density_for(&biome.name);
    let mut pond = false;
    if pond_density != 0 {
        for dx in -5..=5 {
            for dz in -5..=5 {
                let (nx, nz) = (x + dx, z + dz);
                if hash(seed ^ 0x90d, nx, 0, nz).is_multiple_of(pond_density) {
                    let r = 2 + (hash(seed ^ 0x9a1, nx, 3, nz) % 3) as i32;
                    pond |= dx * dx + dz * dz <= r * r;
                }
            }
        }
    }
    let rock_density = wg.rock_density_for(&biome.name);
    let tree_density = wg.tree_density_for(&biome.name);
    let mut result = Column {
        top,
        ground: top - i32::from(pond),
        trunk_end: top,
        surface: id_of(if pond { "sand" } else { &biome.surface }),
        subsurface: id_of(&biome.subsurface),
        stone: id_of("stone"),
        wood: id_of("wood"),
        leaves: id_of("leaves"),
        rock: rock_density != 0 && hash(seed ^ 0x20cc, x, 7, z).is_multiple_of(rock_density),
        crowns: [0; 3],
    };
    if tree_density != 0 {
        if top > wg.sea_level + 1 && hash(seed ^ 0x77, x, 0, z).is_multiple_of(tree_density) {
            result.trunk_end = top + 4 + (hash(seed, x, 1, z) % 2) as i32;
        }
        for dx in -2..=2 {
            for dz in -2..=2 {
                let (nx, nz) = (x + dx, z + dz);
                if !hash(seed ^ 0x77, nx, 0, nz).is_multiple_of(tree_density) {
                    continue;
                }
                let crown = height(nx, nz) + 4 + (hash(seed, nx, 1, nz) % 2) as i32;
                for dy in -1..=2 {
                    let y = crown + dy;
                    if (0..=128).contains(&y) && dx * dx + dz * dz + dy * dy * 2 <= 6 {
                        result.crowns[y as usize / 64] |= 1 << (y as usize % 64);
                    }
                }
            }
        }
    }
    result
}
/// Render distance in blocks must be a multiple of the 32-cell chunk.
pub fn stream_radius_for(render_distance_blocks: i32) -> Result<i32, String> {
    if !(32..=2048).contains(&render_distance_blocks) {
        return Err("render distance is 32 to 2048 blocks".into());
    }
    if render_distance_blocks % 32 != 0 {
        return Err("render distance steps by 32 blocks, one chunk".into());
    }
    Ok(render_distance_blocks / 32)
}

/// Distant LOD reach in blocks: 128 chunks is 4096 blocks.
pub fn lod_distance_for(distant_chunks: i32) -> Result<f64, String> {
    if !(0..=256).contains(&distant_chunks) {
        return Err("distant chunks is 0 to 256".into());
    }
    Ok(distant_chunks as f64 * 32.0)
}

fn parse_noise(line: &str) -> Result<NoiseLayer, String> {
    // `noise continent scale 220 octaves 4 amplitude 22 seed 11`
    let w: Vec<&str> = line.split_whitespace().collect();
    if w.first() != Some(&"noise") || w.len() < 2 {
        return Err("starts with `noise <name>`".into());
    }
    let name = w[1].to_string();
    let mut scale = 64.0;
    let mut octaves = 4u32;
    let mut amplitude = 8.0;
    let mut seed = 0i64;
    let mut i = 2;
    while i < w.len() {
        match w[i] {
            "scale" => {
                scale = num(&w, &mut i, 4.0, 4096.0)?;
            }
            "octaves" => {
                octaves = num(&w, &mut i, 1.0, 8.0)? as u32;
            }
            "amplitude" => {
                amplitude = num(&w, &mut i, 0.0, 128.0)?;
            }
            "seed" => {
                seed = num(&w, &mut i, -2147483648.0, 2147483647.0)? as i64;
            }
            other => return Err(format!("unknown word {other}")),
        }
    }
    Ok(NoiseLayer {
        name,
        scale,
        octaves,
        amplitude,
        seed,
    })
}

fn parse_biome(line: &str) -> Result<Biome, String> {
    // `biome plains temp 0.25 0.75 hum 0.25 0.75 surface grass sub dirt hills 1 trees 170 rocks 320 ponds 500`
    let w: Vec<&str> = line.split_whitespace().collect();
    if w.first() != Some(&"biome") || w.len() < 2 {
        return Err("starts with `biome <name>`".into());
    }
    let name = w[1].to_string();
    let mut temp = (0.0, 1.0);
    let mut hum = (0.0, 1.0);
    let mut surface = "grass".to_string();
    let mut subsurface = "dirt".to_string();
    let mut hill_mult = 1.0;
    let mut tree_density = 170u32;
    let mut rock_density = 320u32;
    let mut pond_density = 500u32;
    let mut i = 2;
    while i < w.len() {
        match w[i] {
            "temp" => {
                i += 1;
                let a = pair_num(&w, &mut i, 0.0, 1.0)?;
                let b = pair_num(&w, &mut i, 0.0, 1.0)?;
                temp = (a.min(b), a.max(b));
            }
            "hum" => {
                i += 1;
                let a = pair_num(&w, &mut i, 0.0, 1.0)?;
                let b = pair_num(&w, &mut i, 0.0, 1.0)?;
                hum = (a.min(b), a.max(b));
            }
            "surface" => {
                i += 1;
                surface = word(&w, i)?;
                i += 1;
            }
            "sub" => {
                i += 1;
                subsurface = word(&w, i)?;
                i += 1;
            }
            "hills" => {
                hill_mult = num(&w, &mut i, 0.0, 4.0)?;
            }
            "trees" => {
                tree_density = num(&w, &mut i, 0.0, 100000.0)? as u32;
            }
            "rocks" => {
                rock_density = num(&w, &mut i, 0.0, 100000.0)? as u32;
            }
            "ponds" => {
                pond_density = num(&w, &mut i, 0.0, 100000.0)? as u32;
            }
            other => return Err(format!("unknown word {other}")),
        }
    }
    Ok(Biome {
        name,
        temp,
        humidity: hum,
        surface,
        subsurface,
        hill_mult,
        tree_density,
        rock_density,
        pond_density,
    })
}

fn parse_feature(line: &str) -> Result<Feature, String> {
    // `feature tree biome forest density 60 size 5 material wood`
    let w: Vec<&str> = line.split_whitespace().collect();
    if w.first() != Some(&"feature") || w.len() < 2 {
        return Err("starts with `feature <tree|rock|pond>`".into());
    }
    let kind = w[1].to_string();
    if !["tree", "rock", "pond"].contains(&kind.as_str()) {
        return Err("feature kind is tree, rock or pond".into());
    }
    let mut biome = "any".to_string();
    let mut density = 170u32;
    let mut size = 4i32;
    let mut material = "wood".to_string();
    let mut i = 2;
    while i < w.len() {
        match w[i] {
            "biome" => {
                i += 1;
                biome = word(&w, i)?;
                i += 1;
            }
            "density" => {
                density = num(&w, &mut i, 0.0, 100000.0)? as u32;
            }
            "size" => {
                size = num(&w, &mut i, 1.0, 16.0)? as i32;
            }
            "material" => {
                i += 1;
                material = word(&w, i)?;
                i += 1;
            }
            other => return Err(format!("unknown word {other}")),
        }
    }
    Ok(Feature {
        kind,
        biome,
        density,
        size,
        material,
    })
}

fn word(w: &[&str], i: usize) -> Result<String, String> {
    w.get(i)
        .map(|s| s.to_string())
        .ok_or("a word is missing".to_string())
}

fn num(w: &[&str], i: &mut usize, lo: f64, hi: f64) -> Result<f64, String> {
    *i += 1;
    let v = pair_num(w, i, lo, hi)?;
    Ok(v)
}

fn pair_num(w: &[&str], i: &mut usize, lo: f64, hi: f64) -> Result<f64, String> {
    let v = w
        .get(*i)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or("a number is missing")?;
    *i += 1;
    if !(lo..=hi).contains(&v) {
        return Err(format!("a number stays {lo} to {hi}"));
    }
    Ok(v)
}

fn hash(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ (z as u32).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn noise3(seed: u32, x: f64, y: f64, z: f64) -> f64 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (tx, ty, tz) = (
        smooth(fx as f32 - x as f32 + (x - fx.floor()) as f32),
        0.0,
        0.0,
    );
    let _ = (tx, ty, tz);
    let (tx, ty, tz) = (
        smooth((x - fx) as f32),
        smooth((y - fy) as f32),
        smooth((z - fz) as f32),
    );
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let corner = |dx, dy, dz| unit(hash(seed, ix + dx, iy + dy, iz + dz)) as f64;
    let lerp = |a: f64, b: f64, t: f32| a + (b - a) * t as f64;
    let lower = lerp(
        lerp(corner(0, 0, 0), corner(1, 0, 0), tx),
        lerp(corner(0, 1, 0), corner(1, 1, 0), tx),
        ty,
    );
    let upper = lerp(
        lerp(corner(0, 0, 1), corner(1, 0, 1), tx),
        lerp(corner(0, 1, 1), corner(1, 1, 1), tx),
        ty,
    );
    lerp(lower, upper, tz)
}

fn fbm(seed: u32, x: f64, z: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise3(seed.wrapping_add(o * 101), x * freq, 0.5, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_distance_steps_by_chunk() {
        assert_eq!(stream_radius_for(256), Ok(8));
        assert_eq!(stream_radius_for(32), Ok(1));
        assert!(stream_radius_for(100).is_err());
        assert!(stream_radius_for(0).is_err());
        assert_eq!(lod_distance_for(128), Ok(4096.0));
    }

    #[test]
    fn lines_parse_and_defaults_fill_in() {
        let wg = Worldgen::parse(&[], &[], &[]).unwrap();
        assert_eq!(wg.layers.len(), 3);
        assert_eq!(wg.biomes.len(), 4);
        let wg = Worldgen::parse(
            &["noise continent scale 220 octaves 4 amplitude 22 seed 11".to_string()],
            &["biome plains temp 0.25 0.75 hum 0.25 0.75 surface grass sub dirt hills 1 trees 170 rocks 320 ponds 500".to_string()],
            &["feature tree biome forest density 60 size 5 material wood".to_string()],
        )
        .unwrap();
        assert_eq!(wg.layers[0].name, "continent");
        assert_eq!(wg.features[0].kind, "tree");
        assert!(Worldgen::parse(&["bogus".to_string()], &[], &[]).is_err());
    }

    #[test]
    fn the_same_seed_builds_the_same_column() {
        let wg = Worldgen::default();
        let a = wg.column_height(42, 10, -7);
        let b = wg.column_height(42, 10, -7);
        assert_eq!(a, b);
        assert!((2..=120).contains(&a));
    }

    #[test]
    fn vertical_pages_reuse_columns_without_changing_cells() {
        let wg = Worldgen::default();
        let mut columns = std::collections::BTreeMap::new();
        for y in 0..4 {
            let mut cached = [0; 32768];
            let mut reference = [0; 32768];
            fill_infinite_page_cached(&wg, &builtin_id, 42, [1, y, 1], &mut cached, &mut columns);
            fill_infinite_page(&wg, &builtin_id, 42, [1, y, 1], &mut reference);
            assert_eq!(cached, reference);
            assert_eq!(columns.len(), 1024);
        }
    }

    #[test]
    fn bulk_pages_match_canonical_samples_cell_for_cell() {
        let wg = Worldgen::default();
        let id_of = |name: &str| match name {
            "stone" => 1,
            "dirt" => 2,
            "grass" => 3,
            "sand" => 4,
            "wood" => 5,
            "leaves" => 6,
            _ => 1,
        };
        for chunk in [[0, 0, 0], [3, 1, -2], [-4, 0, 7]] {
            let mut cells = [0u8; 32768];
            fill_infinite_page(&wg, &id_of, 7, chunk, &mut cells);
            for z in 0..32 {
                for y in 0..32 {
                    for x in 0..32 {
                        let cell = [chunk[0] * 32 + x, chunk[1] * 32 + y, chunk[2] * 32 + z];
                        let at = ((z * 32 + y) * 32 + x) as usize;
                        assert_eq!(
                            cells[at],
                            sample_infinite(&wg, 7, cell, &id_of),
                            "chunk {chunk:?} cell {cell:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn infinite_cells_are_canonical_and_named() {
        let wg = Worldgen::default();
        let id_of = |name: &str| match name {
            "stone" => 1,
            "dirt" => 2,
            "grass" => 3,
            "sand" => 4,
            "wood" => 5,
            "leaves" => 6,
            _ => 1,
        };
        let a = sample_infinite(&wg, 7, [4, 10, -3], &id_of);
        let b = sample_infinite(&wg, 7, [4, 10, -3], &id_of);
        assert_eq!(a, b);
        assert_eq!(sample_infinite(&wg, 7, [0, 200, 0], &id_of), 0);
        // Deep stone under the surface.
        let top = wg.column_height(seed32(7), 4, -3);
        assert_eq!(sample_infinite(&wg, 7, [4, top - 5, -3], &id_of), 1);
    }
}
