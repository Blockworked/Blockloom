//! The asset pipeline: what an imported file *is* and what the game
//! needs from it, decided once at import time rather than every Play.
//!
//! Four jobs in one place:
//!
//! - Model rigs: [`inspect_model`] reads a glTF/GLB/OBJ/FBX file well enough
//!   to say how many meshes, materials, nodes and animations it holds, and
//!   whether it is skinned. The runtime draws a `Visual::Model` as its file
//!   (see `dim3`), so a broken rig is caught here, at import, not mid-game.
//! - Atlases: [`pack_atlas`] lays small sprites into one sheet with a shelf
//!   packer, and [`bake_atlas`] draws that sheet, so a game with dozens of
//!   sprites pays one texture bind. A build bakes its Image looks into
//!   [`BAKED_ATLAS_IMAGE`] and the player draws them out of it.
//! - Compression: [`decide_texture`] and [`decide_audio`] turn an inspected
//!   file into a concrete recommendation (downscale, transcode, resample)
//!   with the reason attached, so the tray can show it and the build can
//!   apply it without guessing twice.
//! - Reimport tracking: [`PipelineManifest`] remembers the fingerprint each
//!   asset imported with, so [`scan_project`] can say which files changed on
//!   disk since and need a reimport. The manifest lives at
//!   `.blockloom/pipeline.json`, beside the script build cache, never inside
//!   `assets/` itself.
//! - Import roles: an [`ImportRole`] says what a file is imported *as* - HDR
//!   sky, 3D volume, heightmap, IES profile, light cookie. Most follow from
//!   the extension; a PNG heightmap or cookie is an override the manifest
//!   keeps. Each role's decoder lives in its own submodule and the `load_*`
//!   functions are what a pass reads a file through.

pub mod bc6h;
pub mod hdr;
pub mod height;
pub mod ies;
pub mod volume;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where reimport fingerprints live: beside the script cache, not an asset.
pub const PIPELINE_FILE: &str = ".blockloom/pipeline.json";
/// Manifest schema, bumped when [`ManifestEntry`] changes shape.
pub const PIPELINE_VERSION: u32 = 1;

// ─── Models ────────────────────────────────────────────────────────────────

/// A rigged-mesh format the pipeline understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModelFormat {
    Gltf,
    Glb,
    Fbx,
    Obj,
    #[default]
    Unknown,
}

impl ModelFormat {
    /// From a file extension alone, before reading any bytes.
    pub fn from_extension(name: &str) -> Self {
        match name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "gltf" => ModelFormat::Gltf,
            "glb" => ModelFormat::Glb,
            "fbx" => ModelFormat::Fbx,
            "obj" => ModelFormat::Obj,
            _ => ModelFormat::Unknown,
        }
    }

    pub fn is_3d_model(self) -> bool {
        !matches!(self, ModelFormat::Unknown)
    }
}

/// What a model file holds, as far as a game cares: counts plus whether
/// anything is skinned or animated. Warnings are non-fatal notes ("no
/// materials, renders untextured").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub format: ModelFormat,
    pub meshes: usize,
    pub materials: usize,
    pub nodes: usize,
    pub animations: usize,
    pub skins: usize,
    pub warnings: Vec<String>,
}

impl ModelInfo {
    pub fn has_skin(&self) -> bool {
        self.skins > 0
    }

    pub fn has_animations(&self) -> bool {
        self.animations > 0
    }

    /// One line for the tray: "glb: 3 meshes, 2 materials, 1 animation".
    pub fn summary(&self) -> String {
        let kind = match self.format {
            ModelFormat::Gltf => "gltf",
            ModelFormat::Glb => "glb",
            ModelFormat::Fbx => "fbx",
            ModelFormat::Obj => "obj",
            ModelFormat::Unknown => "model",
        };
        let mut parts = vec![
            format!(
                "{} mesh{}",
                self.meshes,
                if self.meshes == 1 { "" } else { "es" }
            ),
            format!(
                "{} material{}",
                self.materials,
                if self.materials == 1 { "" } else { "s" }
            ),
        ];
        if self.animations > 0 {
            parts.push(format!(
                "{} animation{}",
                self.animations,
                if self.animations == 1 { "" } else { "s" }
            ));
        }
        if self.skins > 0 {
            parts.push("skinned".to_string());
        }
        format!("{kind}: {}", parts.join(", "))
    }
}

/// Sniff the format from extension plus magic bytes. GLB starts `glTF`,
/// binary FBX starts `Kaydara FBX Binary`, text FBX mentions `FBXHeader`,
/// OBJ is text with `v`/`f` lines, glTF JSON starts with `{`.
pub fn detect_format(name: &str, bytes: &[u8]) -> ModelFormat {
    if bytes.len() >= 4 && &bytes[..4] == b"glTF" {
        return ModelFormat::Glb;
    }
    if bytes.starts_with(b"Kaydara FBX Binary") {
        return ModelFormat::Fbx;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    if head.contains("FBXHeader") || head.contains("FbxHeader") {
        return ModelFormat::Fbx;
    }
    let trimmed: Vec<&str> = head
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(8)
        .collect();
    if !trimmed.is_empty()
        && trimmed.iter().all(|line| {
            line.starts_with('{')
                || line.starts_with('[')
                || line.starts_with('"')
                || line == &"}"
                || line == &"]"
        })
    {
        return ModelFormat::Gltf;
    }
    if trimmed.first().is_some_and(|line| line.starts_with('{')) {
        return ModelFormat::Gltf;
    }
    if head.contains("\nmtllib ") || head.contains("\nusemtl ") {
        return ModelFormat::Obj;
    }
    // Bare OBJ without material refs: vertex/face lines lead.
    let mut verts = 0;
    let mut faces = 0;
    for line in head.lines().take(64) {
        let line = line.trim();
        if line.starts_with("v ") || line.starts_with("v\t") {
            verts += 1;
        } else if line.starts_with("f ") || line.starts_with("f\t") {
            faces += 1;
        }
    }
    if verts > 0 && faces > 0 {
        return ModelFormat::Obj;
    }
    ModelFormat::from_extension(name)
}

/// Inspect raw model bytes: parse glTF/GLB structure, count OBJ lines, or
/// sniff FBX magic. Never touches the GPU; a broken rig errors here.
pub fn inspect_model(name: &str, bytes: &[u8]) -> Result<ModelInfo, String> {
    if bytes.is_empty() {
        return Err(format!("{name} is empty"));
    }
    match detect_format(name, bytes) {
        ModelFormat::Glb => inspect_glb(name, bytes),
        ModelFormat::Gltf => inspect_gltf_bytes(name, bytes),
        ModelFormat::Obj => Ok(inspect_obj(bytes)),
        ModelFormat::Fbx => Ok(inspect_fbx(name, bytes)),
        ModelFormat::Unknown => Err(format!(
            "{name} isn't a model Blockloom loads (.gltf, .glb, .obj, .fbx)"
        )),
    }
}

/// Inspect one model file on disk.
pub fn inspect_model_file(path: &Path) -> Result<ModelInfo, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    inspect_model(&name, &bytes)
}

fn inspect_gltf_bytes(name: &str, bytes: &[u8]) -> Result<ModelInfo, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{name} isn't UTF-8 glTF JSON"))?;
    inspect_gltf_value(
        name,
        &serde_json::from_str(text).map_err(|e| format!("{name}: bad glTF JSON: {e}"))?,
    )
}

fn inspect_glb(name: &str, bytes: &[u8]) -> Result<ModelInfo, String> {
    if bytes.len() < 20 {
        return Err(format!("{name} is too short to be a .glb"));
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_start: usize = 20;
    let json_end = json_start.saturating_add(json_len).min(bytes.len());
    let text = std::str::from_utf8(&bytes[json_start..json_end])
        .map_err(|_| format!("{name}: bad glTF JSON chunk"))?;
    inspect_gltf_value(
        name,
        &serde_json::from_str(text).map_err(|e| format!("{name}: bad glTF JSON: {e}"))?,
    )
}

fn count(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn inspect_gltf_value(name: &str, value: &serde_json::Value) -> Result<ModelInfo, String> {
    if value.get("asset").is_none() {
        return Err(format!("{name} has no glTF `asset` section"));
    }
    let mut warnings = Vec::new();
    let meshes = count(value, "meshes");
    let materials = count(value, "materials");
    let nodes = count(value, "nodes");
    let animations = count(value, "animations");
    let skins = count(value, "skins");
    if meshes == 0 {
        warnings.push("no meshes: nothing to draw".to_string());
    }
    if materials == 0 {
        warnings.push("no materials: renders untextured".to_string());
    }
    Ok(ModelInfo {
        format: ModelFormat::Gltf,
        meshes,
        materials,
        nodes,
        animations,
        skins,
        warnings,
    })
}

fn inspect_obj(bytes: &[u8]) -> ModelInfo {
    let text = String::from_utf8_lossy(bytes);
    let mut verts = 0usize;
    let mut faces = 0usize;
    let mut materials = 0usize;
    let mut objects = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("v ") || line.starts_with("v\t") {
            verts += 1;
        } else if line.starts_with("f ") || line.starts_with("f\t") {
            faces += 1;
        } else if line.starts_with("usemtl ") {
            materials += 1;
        } else if line.starts_with("o ") || line.starts_with("g ") {
            objects += 1;
        }
    }
    let mut warnings = Vec::new();
    if faces == 0 {
        warnings.push("no faces: nothing to draw".to_string());
    }
    if verts == 0 {
        warnings.push("no vertices".to_string());
    }
    // OBJ has no rig: one mesh per object block, at least one when faces exist.
    let meshes = objects.max(usize::from(faces > 0));
    ModelInfo {
        format: ModelFormat::Obj,
        meshes,
        materials,
        nodes: objects,
        animations: 0,
        skins: 0,
        warnings,
    }
}

fn inspect_fbx(name: &str, bytes: &[u8]) -> ModelInfo {
    let mut warnings = Vec::new();
    let binary = bytes.starts_with(b"Kaydara FBX Binary");
    let text = if binary {
        String::new()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    // Cheap heuristics: count the node types a game cares about. Exact
    // counts need the full FBX tree; import-time only needs the shape.
    let (meshes, materials, animations, skins) = if binary {
        let mut meshes = 0;
        let mut materials = 0;
        for window in bytes.windows(8) {
            if window == b"Geometry" {
                meshes += 1;
            } else if window == b"Material" {
                materials += 1;
            }
        }
        (
            meshes.min(4096),
            materials.min(4096),
            0,
            usize::from(bytes.windows(8).any(|w| w == b"Deformer")),
        )
    } else {
        (
            text.matches("Geometry:").count(),
            text.matches("Material:").count(),
            text.matches("AnimationStack:").count(),
            text.matches("Deformer:").count(),
        )
    };
    if !binary && !text.contains("FBXHeader") && !text.contains("FbxHeader") {
        warnings.push(format!(
            "{name}: no FBX header found, counts are approximate"
        ));
    }
    if meshes == 0 {
        warnings.push("no geometry found: nothing to draw".to_string());
    }
    warnings.push("fbx imports as a static mesh: rigs play back only from glTF".to_string());
    ModelInfo {
        format: ModelFormat::Fbx,
        meshes,
        materials,
        nodes: meshes,
        animations,
        skins,
        warnings,
    }
}

// ─── Textures ────────────────────────────────────────────────────────────

/// What an image file decodes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureInfo {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub bytes: u64,
}

impl TextureInfo {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Uncompressed RGBA VRAM, the number that matters on weak GPUs.
    pub fn vram_bytes(&self) -> u64 {
        self.pixels() * 4
    }
}

/// Decode just enough to learn the dimensions. Uses the `image` crate the
/// workspace already has; exotic files error rather than guess.
pub fn inspect_texture(name: &str, bytes: &[u8]) -> Result<TextureInfo, String> {
    let format = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let image = image::load_from_memory(bytes)
        .map_err(|e| format!("{name} doesn't decode as an image: {e}"))?;
    Ok(TextureInfo {
        width: image.width(),
        height: image.height(),
        format,
        bytes: bytes.len() as u64,
    })
}

/// Where a texture should end up after the pipeline runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureTarget {
    /// Keep the file as-is: small UI, pixel art, already compressed.
    AsIs,
    /// Downscale on import, keep the container.
    Downscale,
    /// Transcode to a GPU-compressed container (KTX2/Basis) at build time.
    Transcode,
    /// HDR: BC6H in KTX2 at build time, which keeps values above 1.
    Bc6h,
}

/// One texture's import plan, with the reason attached for the tray.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TexturePlan {
    pub target: TextureTarget,
    pub target_width: u32,
    pub target_height: u32,
    pub reason: String,
}

/// Decide what import does with a texture: huge photos get downscaled and
/// transcoded, small sprites stay exactly as authored.
pub fn decide_texture(info: &TextureInfo, max_size: u32) -> TexturePlan {
    let max_side = info.width.max(info.height);
    let downscale = max_side > max_size.max(64);
    let (target_width, target_height) = if downscale {
        let scale = max_size as f64 / max_side as f64;
        (
            ((info.width as f64 * scale).round() as u32).max(1),
            ((info.height as f64 * scale).round() as u32).max(1),
        )
    } else {
        (info.width, info.height)
    };
    let big = info.pixels() > 256 * 256;
    let flat_source = matches!(info.format.as_str(), "png" | "bmp" | "tga");
    if downscale && big && flat_source {
        TexturePlan {
            target: TextureTarget::Transcode,
            target_width,
            target_height,
            reason: format!(
                "{}x{} exceeds the {max_size}px import cap: downscale and transcode to KTX2",
                info.width, info.height
            ),
        }
    } else if downscale {
        TexturePlan {
            target: TextureTarget::Downscale,
            target_width,
            target_height,
            reason: format!(
                "{}x{} exceeds the {max_size}px import cap",
                info.width, info.height
            ),
        }
    } else if big && flat_source {
        TexturePlan {
            target: TextureTarget::Transcode,
            target_width,
            target_height,
            reason: "large lossless source: transcode to KTX2 at build time".to_string(),
        }
    } else {
        TexturePlan {
            target: TextureTarget::AsIs,
            target_width,
            target_height,
            reason: "small enough to ship as authored".to_string(),
        }
    }
}

// ─── Audio ───────────────────────────────────────────────────────────────

/// What a sound file holds, as far as the mixer cares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub format: String,
    pub channels: u8,
    pub sample_rate: u32,
    pub duration_secs: f32,
    pub bytes: u64,
}

/// Sniff WAV/OGG/MP3/FLAC headers. WAV parses exactly; compressed formats
/// estimate from file size since full decode belongs at play time.
pub fn inspect_audio(name: &str, bytes: &[u8]) -> Result<AudioInfo, String> {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if bytes.starts_with(b"RIFF") && bytes.len() >= 44 && &bytes[8..12] == b"WAVE" {
        return inspect_wav(&ext, bytes);
    }
    if bytes.starts_with(b"OggS") {
        // ~128kbps Vorbis ballpark for the estimate the tray shows.
        let duration = bytes.len() as f32 / 16_000.0;
        return Ok(AudioInfo {
            format: "ogg".to_string(),
            channels: 2,
            sample_rate: 44_100,
            duration_secs: duration,
            bytes: bytes.len() as u64,
        });
    }
    if bytes.starts_with(b"fLaC") {
        let duration = bytes.len() as f32 / 60_000.0;
        return Ok(AudioInfo {
            format: "flac".to_string(),
            channels: 2,
            sample_rate: 44_100,
            duration_secs: duration,
            bytes: bytes.len() as u64,
        });
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        let duration = bytes.len() as f32 / 16_000.0;
        return Ok(AudioInfo {
            format: "mp3".to_string(),
            channels: 2,
            sample_rate: 44_100,
            duration_secs: duration,
            bytes: bytes.len() as u64,
        });
    }
    if ext == "wav" {
        return Err(format!("{name} isn't a WAV Blockloom can read"));
    }
    Err(format!(
        "{name} isn't audio Blockloom loads (.wav, .ogg, .mp3, .flac)"
    ))
}

fn u16le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn inspect_wav(ext: &str, bytes: &[u8]) -> Result<AudioInfo, String> {
    if bytes.len() < 44 {
        return Err("WAV is truncated".to_string());
    }
    let channels = u16le(bytes, 22).min(8) as u8;
    let sample_rate = u32le(bytes, 24);
    let bits = u16le(bytes, 34);
    if channels == 0 || sample_rate == 0 || bits == 0 {
        return Err("WAV header has zero channels, rate or depth".to_string());
    }
    // Find the data chunk rather than assuming it follows fmt.
    let mut data_len = bytes.len().saturating_sub(44) as u32;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let chunk = &bytes[at..at + 4];
        let len = u32le(bytes, at + 4) as usize;
        if chunk == b"data" {
            data_len = len.min(bytes.len().saturating_sub(at + 8)) as u32;
            break;
        }
        at += 8 + len + (len & 1);
    }
    let bytes_per_frame = channels as u32 * (bits as u32 / 8).max(1);
    let frames = data_len / bytes_per_frame.max(1);
    Ok(AudioInfo {
        format: if ext.is_empty() {
            "wav".to_string()
        } else {
            ext.to_string()
        },
        channels,
        sample_rate,
        duration_secs: frames as f32 / sample_rate as f32,
        bytes: bytes.len() as u64,
    })
}

/// One sound's import plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlan {
    /// Transcode to Ogg Vorbis at build time.
    pub transcode: bool,
    /// Resample to this rate on import (the source rate when unchanged).
    pub target_rate: u32,
    pub reason: String,
}

/// WAVs ship as Ogg; already-compressed files ship as authored unless they
/// carry a wasteful rate. SFX above 48kHz resample down: nobody hears the
/// difference through a game speaker.
pub fn decide_audio(info: &AudioInfo) -> AudioPlan {
    let target_rate = info.sample_rate.min(48_000);
    if info.format == "wav" {
        AudioPlan {
            transcode: true,
            target_rate,
            reason: format!(
                "WAV {:.1}s ships as Ogg Vorbis{}",
                info.duration_secs,
                if target_rate != info.sample_rate {
                    format!(" resampled to {target_rate}Hz")
                } else {
                    String::new()
                }
            ),
        }
    } else if info.sample_rate > 48_000 {
        AudioPlan {
            transcode: false,
            target_rate,
            reason: format!("resample {0}Hz to {target_rate}Hz", info.sample_rate),
        }
    } else {
        AudioPlan {
            transcode: false,
            target_rate,
            reason: "ships as authored".to_string(),
        }
    }
}

// ─── Atlases ─────────────────────────────────────────────────────────────

/// One sprite going into an atlas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasInput {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Where one sprite landed in the sheet, with normalized UVs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasEntry {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub uv: [f32; 4],
}

/// A packed sheet: one texture bind for many small sprites.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasLayout {
    pub width: u32,
    pub height: u32,
    pub entries: Vec<AtlasEntry>,
}

impl AtlasLayout {
    /// The entry for `name`, if it was packed.
    pub fn entry(&self, name: &str) -> Option<&AtlasEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// Lay sprites into one sheet with a deterministic shelf packer: tallest
/// first, rows left to right, sheet grows down. `padding` keeps bleeding
/// sprites from sampling their neighbour; `max_size` caps the sheet (2048
/// is safe on weak GPUs). Errors when one sprite alone exceeds the cap.
pub fn pack_atlas(
    inputs: &[AtlasInput],
    max_size: u32,
    padding: u32,
) -> Result<AtlasLayout, String> {
    if inputs.is_empty() {
        return Err("nothing to pack".to_string());
    }
    let max_size = max_size.max(64);
    let mut sorted = inputs.to_vec();
    sorted.sort_by(|a, b| {
        b.height
            .cmp(&a.height)
            .then_with(|| b.width.cmp(&a.width))
            .then_with(|| a.name.cmp(&b.name))
    });
    for input in &sorted {
        if input.width == 0 || input.height == 0 {
            return Err(format!("{} has no pixels", input.name));
        }
        if input.width + padding * 2 > max_size || input.height + padding * 2 > max_size {
            return Err(format!(
                "{} ({}x{}) doesn't fit a {max_size}px atlas",
                input.name, input.width, input.height
            ));
        }
    }
    // Shelves: each row is as tall as its first (tallest) sprite.
    let mut shelves: Vec<(u32, u32)> = Vec::new(); // (y, height)
    let mut placements: Vec<(usize, u32, u32)> = Vec::new(); // (index, x, y)
    let mut seen_names = std::collections::HashSet::new();
    for (index, input) in sorted.iter().enumerate() {
        if !seen_names.insert(input.name.clone()) {
            return Err(format!("{} is listed twice", input.name));
        }
        let w = input.width + padding * 2;
        let h = input.height + padding * 2;
        let mut placed = false;
        // Rows are tried top to bottom; x is wherever the row has run to.
        let mut row_end: HashMap<usize, u32> = HashMap::new();
        for (placed_index, x, y) in &placements {
            let shelf = shelves.iter().position(|(sy, _)| *sy == *y).unwrap_or(0);
            row_end.insert(
                shelf,
                row_end
                    .get(&shelf)
                    .copied()
                    .unwrap_or(0)
                    .max(x + sorted[*placed_index].width + padding * 2),
            );
        }
        for (shelf, (y, height)) in shelves.iter().enumerate() {
            if h > *height {
                continue;
            }
            let x = row_end.get(&shelf).copied().unwrap_or(0);
            if x + w <= max_size {
                placements.push((index, x, *y));
                placed = true;
                break;
            }
        }
        if !placed {
            let y: u32 = shelves.iter().map(|(y, h)| y + h).max().unwrap_or(0);
            if y + h > max_size {
                return Err(format!(
                    "atlas overflows {max_size}px with {} ({}x{})",
                    input.name, input.width, input.height
                ));
            }
            shelves.push((y, h));
            placements.push((index, 0, y));
        }
    }
    let height: u32 = shelves.iter().map(|(y, h)| y + h).max().unwrap_or(0);
    // Width is the widest row, rounded up to keep UV math exact.
    let mut width = 0u32;
    for (index, x, _) in &placements {
        width = width.max(x + sorted[*index].width + padding * 2);
    }
    let mut entries: Vec<AtlasEntry> = placements
        .iter()
        .map(|(index, x, y)| {
            let input = &sorted[*index];
            AtlasEntry {
                name: input.name.clone(),
                x: x + padding,
                y: y + padding,
                width: input.width,
                height: input.height,
                uv: [
                    (x + padding) as f32 / width as f32,
                    (y + padding) as f32 / height as f32,
                    (x + padding + input.width) as f32 / width as f32,
                    (y + padding + input.height) as f32 / height as f32,
                ],
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(AtlasLayout {
        width,
        height,
        entries,
    })
}

/// Where a build keeps its baked sprite sheet and layout, inside `game/`.
/// The player draws an Image look out of the sheet when its path is listed.
pub const BAKED_ATLAS_IMAGE: &str = ".blockloom/atlas.png";
pub const BAKED_ATLAS_LAYOUT: &str = ".blockloom/atlas.json";
/// A sprite bigger than this on either side keeps its own texture: an atlas
/// is for the many small ones, and one backdrop would crowd them all out.
pub const ATLAS_SPRITE_MAX: u32 = 512;

/// A drawn sheet and where each sprite landed in it.
#[derive(Debug, Clone)]
pub struct BakedAtlas {
    pub image: image::RgbaImage,
    pub layout: AtlasLayout,
}

/// Pack and draw project images into one sheet. Entry names are the
/// project-relative paths. Each sprite's edge pixels are smeared into its
/// padding, so a filtered sample at the border reads the sprite, never its
/// neighbour.
pub fn bake_atlas(
    project_dir: &Path,
    paths: &[String],
    max_size: u32,
    padding: u32,
) -> Result<BakedAtlas, String> {
    let mut sprites = HashMap::new();
    let mut inputs = Vec::new();
    for path in paths {
        crate::build_control::check()?;
        let relative = crate::assets::normalize(path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        let full = crate::assets::resolve(project_dir, &relative)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        let bytes = crate::vfs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        let sprite = image::load_from_memory(&bytes)
            .map_err(|e| format!("{relative} doesn't decode as an image: {e}"))?
            .to_rgba8();
        inputs.push(AtlasInput {
            name: relative.clone(),
            width: sprite.width(),
            height: sprite.height(),
        });
        sprites.insert(relative, sprite);
    }
    let layout = pack_atlas(&inputs, max_size, padding)?;
    let mut sheet = image::RgbaImage::new(layout.width.max(1), layout.height.max(1));
    let pad = padding as i64;
    for entry in &layout.entries {
        let sprite = &sprites[&entry.name];
        let (w, h) = (entry.width as i64, entry.height as i64);
        for dy in -pad..h + pad {
            crate::build_control::check()?;
            for dx in -pad..w + pad {
                let pixel = *sprite.get_pixel(dx.clamp(0, w - 1) as u32, dy.clamp(0, h - 1) as u32);
                let (x, y) = (entry.x as i64 + dx, entry.y as i64 + dy);
                if x >= 0 && y >= 0 && (x as u32) < sheet.width() && (y as u32) < sheet.height() {
                    sheet.put_pixel(x as u32, y as u32, pixel);
                }
            }
        }
    }
    Ok(BakedAtlas {
        image: sheet,
        layout,
    })
}

/// Write a baked sheet as a PNG and its layout as JSON beside it.
pub fn write_atlas(
    atlas: &BakedAtlas,
    image_path: &Path,
    layout_path: &Path,
) -> Result<(), String> {
    for path in [image_path, layout_path] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
    }
    atlas
        .image
        .save_with_format(image_path, image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", image_path.display()))?;
    let json = serde_json::to_string_pretty(&atlas.layout).map_err(|e| e.to_string())?;
    std::fs::write(layout_path, json).map_err(|e| format!("{}: {e}", layout_path.display()))
}

/// The layout a build baked into `game_dir`, if it baked one.
pub fn read_baked_layout(game_dir: &Path) -> Option<AtlasLayout> {
    let text = crate::vfs::read_to_string(&game_dir.join(BAKED_ATLAS_LAYOUT)).ok()?;
    serde_json::from_str(&text).ok()
}

// ─── Import roles ────────────────────────────────────────────────────────

/// What a file is imported as. The same PNG can be a sprite, a heightmap or a
/// light cookie; only the author knows, so the extension's guess can be
/// overridden per asset (see [`set_role`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRole {
    /// An ordinary colour texture: the downscale/transcode plan.
    Texture,
    /// High dynamic range: skies, IBL sources, emissive masks. Ships as BC6H.
    Hdr,
    /// A 3D texture: a `.cube` grading LUT or an image strip of slices.
    Volume,
    /// Terrain heights, 16-bit where the source has it.
    Heightmap,
    /// A photometric light profile.
    Ies,
    /// A grayscale mask a light projects.
    Cookie,
}

impl ImportRole {
    pub const ALL: [ImportRole; 6] = [
        ImportRole::Texture,
        ImportRole::Hdr,
        ImportRole::Volume,
        ImportRole::Heightmap,
        ImportRole::Ies,
        ImportRole::Cookie,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ImportRole::Texture => "texture",
            ImportRole::Hdr => "hdr",
            ImportRole::Volume => "volume",
            ImportRole::Heightmap => "heightmap",
            ImportRole::Ies => "ies",
            ImportRole::Cookie => "cookie",
        }
    }

    /// By name, case ignored. `auto` and empty are `None`: follow the extension.
    pub fn parse(name: &str) -> Result<Option<ImportRole>, String> {
        let name = name.trim().to_lowercase();
        if name.is_empty() || name == "auto" {
            return Ok(None);
        }
        Self::ALL
            .into_iter()
            .find(|role| role.name() == name)
            .map(Some)
            .ok_or_else(|| format!("\"{name}\" isn't an import role"))
    }

    /// The role a file's extension implies, or `None` for kinds with no role
    /// (models, audio, scripts...).
    pub fn detect(relative: &str) -> Option<ImportRole> {
        use crate::assets::AssetKind;
        match crate::assets::kind_of(relative) {
            AssetKind::Image => Some(ImportRole::Texture),
            AssetKind::Hdr => Some(ImportRole::Hdr),
            AssetKind::Volume => Some(ImportRole::Volume),
            AssetKind::Height => Some(ImportRole::Heightmap),
            AssetKind::Light => Some(ImportRole::Ies),
            _ => None,
        }
    }
}

/// Where a build puts an HDR sky baked to a BC6H cube, relative to its
/// game folder.
pub fn baked_sky_path(relative: &str) -> String {
    format!(".blockloom/sky/{}.dds", relative.replace('/', "__"))
}

/// An HDR's plan: BC6H always (the only block format that keeps values past
/// 1), downscaled past `hdr_max`.
pub fn decide_hdr(info: &hdr::HdrInfo, settings: &ImportSettings) -> TexturePlan {
    let max = settings.hdr_max.max(64);
    let side = info.width.max(info.height);
    let (width, height) = if side > max {
        let scale = max as f64 / side as f64;
        (
            ((info.width as f64 * scale).round() as u32).max(1),
            ((info.height as f64 * scale).round() as u32).max(1),
        )
    } else {
        (info.width, info.height)
    };
    let shape = if info.is_equirect() {
        "equirectangular sky, bakes to a cubemap"
    } else {
        "HDR texture"
    };
    let size = hdr::bc6h_bytes(width, height, true);
    let reason = if side > max {
        format!(
            "{shape}: downscale to {width}x{height} and ship as BC6H ({})",
            human_bytes(size)
        )
    } else {
        format!("{shape}: ships as BC6H ({})", human_bytes(size))
    };
    TexturePlan {
        target: TextureTarget::Bc6h,
        target_width: width,
        target_height: height,
        reason,
    }
}

fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MiB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KiB", b as f64 / (1 << 10) as f64),
        b => format!("{b} B"),
    }
}

/// The summary and warnings for a file imported as `role`.
fn describe_role(
    relative: &str,
    role: ImportRole,
    settings: &ImportSettings,
    bytes: &[u8],
) -> Result<(String, Vec<String>), String> {
    let mut warnings = Vec::new();
    let summary = match role {
        ImportRole::Texture => unreachable!("textures take the plain image path"),
        ImportRole::Hdr => {
            let info = hdr::inspect_hdr(relative, bytes)?;
            if !info.channels.iter().any(|c| c == "R" || c.ends_with(".R")) {
                warnings.push("no R channel: reads as black".to_string());
            }
            let plan = decide_hdr(&info, settings);
            format!(
                "{} {}x{} {}-bit: {}",
                info.format, info.width, info.height, info.bits, plan.reason
            )
        }
        ImportRole::Volume => {
            let volume = decode_volume(relative, bytes)?;
            let [x, y, z] = volume.info.size;
            format!(
                "{x}x{y}x{z} volume from a {}: 3D texture, {}",
                volume.info.source,
                human_bytes(volume.info.texels() * 8)
            )
        }
        ImportRole::Heightmap => {
            let map = height::decode_heightmap(relative, bytes)?;
            let info = &map.info;
            if info.bits < 16 {
                warnings.push("8-bit heights terrace: export 16-bit".to_string());
            }
            if info.width != info.height || !height::is_terrain_side(info.width) {
                warnings.push(format!(
                    "{}x{} isn't 2^n+1 square (513, 1025...): terrain resamples it",
                    info.width, info.height
                ));
            }
            if info.max <= info.min {
                warnings.push("every sample is the same height".to_string());
            }
            format!(
                "{}x{} {}-bit heightmap, range {:.3}-{:.3}",
                info.width, info.height, info.bits, info.min, info.max
            )
        }
        ImportRole::Ies => {
            let text = std::str::from_utf8(bytes).map_err(|_| format!("{relative} isn't text"))?;
            let info = ies::parse_ies(relative, text)?.info();
            format!(
                "IES {}x{} angles ({}), peak {:.0} cd",
                info.vertical_angles, info.horizontal_angles, info.symmetry, info.max_candela
            )
        }
        ImportRole::Cookie => {
            let info = inspect_texture(relative, bytes)?;
            if info.width != info.height {
                warnings.push("not square: the cookie stretches over the cone".to_string());
            }
            if !info.width.is_power_of_two() || !info.height.is_power_of_two() {
                warnings.push("not a power of two: mips come out uneven".to_string());
            }
            let side = info
                .width
                .max(info.height)
                .min(settings.texture_max.max(64));
            format!(
                "{}x{} light cookie: one channel, {side}px R8",
                info.width, info.height
            )
        }
    };
    Ok((summary, warnings))
}

/// A volume from its bytes: a `.cube` LUT as text, anything else as a strip.
pub fn decode_volume(relative: &str, bytes: &[u8]) -> Result<volume::Volume, String> {
    if relative.to_lowercase().ends_with(".cube") {
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{relative} isn't text"))?;
        return volume::parse_cube(relative, text);
    }
    let image = image::load_from_memory(bytes)
        .map_err(|e| format!("{relative} doesn't decode as an image: {e}"))?
        .to_rgba8();
    volume::volume_from_strip(relative, &image)
}

fn read_asset(project_dir: &Path, relative: &str) -> Result<(String, Vec<u8>), String> {
    let relative = crate::assets::normalize(relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let full = crate::assets::resolve(project_dir, &relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let bytes = crate::vfs::read(&full).map_err(|e| format!("{relative}: {e}"))?;
    Ok((relative, bytes))
}

/// A project's volume, ready to upload as a 3D texture.
pub fn load_volume(project_dir: &Path, relative: &str) -> Result<volume::Volume, String> {
    let (relative, bytes) = read_asset(project_dir, relative)?;
    decode_volume(&relative, &bytes)
}

/// A project's heightmap, samples 0-1.
pub fn load_heightmap(project_dir: &Path, relative: &str) -> Result<height::Heightmap, String> {
    let (relative, bytes) = read_asset(project_dir, relative)?;
    height::decode_heightmap(&relative, &bytes)
}

/// A project's IES profile.
pub fn load_ies(project_dir: &Path, relative: &str) -> Result<ies::IesProfile, String> {
    let (relative, bytes) = read_asset(project_dir, relative)?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{relative} isn't text"))?;
    ies::parse_ies(&relative, &text)
}

// ─── Reimport tracking ───────────────────────────────────────────────────

/// Import-time knobs, snapshotted per asset so a settings change dirties
/// exactly the assets it affects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSettings {
    #[serde(default = "default_texture_max")]
    pub texture_max: u32,
    #[serde(default = "default_audio_quality")]
    pub audio_quality: u8,
    #[serde(default = "default_atlas_max")]
    pub atlas_max: u32,
    /// Longest side an HDR keeps: skies want more than sprites do.
    #[serde(default = "default_hdr_max")]
    pub hdr_max: u32,
}

fn default_hdr_max() -> u32 {
    4096
}

fn default_texture_max() -> u32 {
    2048
}

fn default_audio_quality() -> u8 {
    5
}

fn default_atlas_max() -> u32 {
    2048
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            texture_max: default_texture_max(),
            audio_quality: default_audio_quality(),
            atlas_max: default_atlas_max(),
            hdr_max: default_hdr_max(),
        }
    }
}

impl ImportSettings {
    fn fingerprint(&self) -> u32 {
        let mut text = format!(
            "{}:{}:{}",
            self.texture_max, self.audio_quality, self.atlas_max
        );
        // Newer knobs join only when moved, so adding one dirties nothing.
        if self.hdr_max != default_hdr_max() {
            text += &format!(":hdr{}", self.hdr_max);
        }
        crc32(text.as_bytes())
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

/// One asset's last-import fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// CRC32 of the source bytes at import.
    pub hash: u32,
    pub size: u64,
    pub mtime: u64,
    /// [`ImportSettings`] fingerprint at import.
    pub settings: u32,
    /// What import produced, relative to the project folder.
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub detail: String,
    /// The role it imported as. `None` in manifests written before roles,
    /// which read as the role its extension gives.
    #[serde(default)]
    pub role: Option<ImportRole>,
}

/// The whole folder's fingerprints, keyed by project-relative path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineManifest {
    #[serde(default = "pipeline_version")]
    pub version: u32,
    #[serde(default)]
    pub settings: ImportSettings,
    #[serde(default)]
    pub entries: HashMap<String, ManifestEntry>,
    /// Roles the author chose over what the extension says, by path.
    #[serde(default)]
    pub roles: HashMap<String, ImportRole>,
    /// Per-texture exposure bias in stops, by path. HDR files only.
    #[serde(default)]
    pub exposure_bias: HashMap<String, f32>,
}

fn pipeline_version() -> u32 {
    PIPELINE_VERSION
}

impl Default for PipelineManifest {
    fn default() -> Self {
        Self {
            version: PIPELINE_VERSION,
            settings: ImportSettings::default(),
            entries: HashMap::new(),
            roles: HashMap::new(),
            exposure_bias: HashMap::new(),
        }
    }
}

impl PipelineManifest {
    /// What `relative` imports as: the author's override, else its extension's.
    pub fn role_of(&self, relative: &str) -> Option<ImportRole> {
        self.roles
            .get(relative)
            .copied()
            .or_else(|| ImportRole::detect(relative))
    }

    /// Stops an HDR file is scaled by when it is decoded. 0 when unset.
    pub fn bias_of(&self, relative: &str) -> f32 {
        self.exposure_bias.get(relative).copied().unwrap_or(0.0)
    }
}

fn manifest_path(project_dir: &Path) -> PathBuf {
    project_dir.join(PIPELINE_FILE)
}

/// Read the manifest, or an empty one when the project never imported
/// through the pipeline.
pub fn load_manifest(project_dir: &Path) -> PipelineManifest {
    let path = manifest_path(project_dir);
    let Ok(text) = crate::vfs::read_to_string(&path) else {
        return PipelineManifest::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Write the manifest, making `.blockloom/` first.
pub fn save_manifest(project_dir: &Path, manifest: &PipelineManifest) -> Result<(), String> {
    let path = manifest_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

fn file_fingerprint(path: &Path) -> Result<(u32, u64, u64), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0);
    Ok((crc32(&bytes), bytes.len() as u64, mtime))
}

/// Record one asset as freshly imported: fingerprint what is on disk now.
pub fn note_imported(project_dir: &Path, relative: &str, detail: &str) -> Result<(), String> {
    let relative = crate::assets::normalize(relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let full = crate::assets::resolve(project_dir, &relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    if !full.is_file() {
        return Ok(());
    }
    let (hash, size, mtime) = file_fingerprint(&full)?;
    let mut manifest = load_manifest(project_dir);
    let settings = manifest.settings.fingerprint();
    let role = manifest.role_of(&relative);
    manifest.entries.insert(
        relative,
        ManifestEntry {
            hash,
            size,
            mtime,
            settings,
            outputs: Vec::new(),
            detail: detail.to_string(),
            role,
        },
    );
    save_manifest(project_dir, &manifest)
}

/// Forget one asset: renames and deletes funnel through here so the manifest
/// never points at a file that left.
pub fn note_removed(project_dir: &Path, relative: &str) {
    let mut manifest = load_manifest(project_dir);
    let entry = manifest.entries.remove(relative);
    let role = manifest.roles.remove(relative);
    let bias = manifest.exposure_bias.remove(relative);
    if entry.is_some() || role.is_some() || bias.is_some() {
        let _ = save_manifest(project_dir, &manifest);
    }
}

/// Follow a rename through the manifest.
pub fn note_moved(project_dir: &Path, from: &str, to: &str) {
    let mut manifest = load_manifest(project_dir);
    let entry = manifest.entries.remove(from);
    let role = manifest.roles.remove(from);
    let bias = manifest.exposure_bias.remove(from);
    if entry.is_none() && role.is_none() && bias.is_none() {
        return;
    }
    if let Some(entry) = entry {
        manifest.entries.insert(to.to_string(), entry);
    }
    if let Some(role) = role {
        manifest.roles.insert(to.to_string(), role);
    }
    if let Some(bias) = bias {
        manifest.exposure_bias.insert(to.to_string(), bias);
    }
    let _ = save_manifest(project_dir, &manifest);
}

/// Imports `relative` as `role` from now on, or as its extension says when
/// `None`. The asset reads dirty until it is reimported under the new role.
pub fn set_role(
    project_dir: &Path,
    relative: &str,
    role: Option<ImportRole>,
) -> Result<PipelineReport, String> {
    let relative = crate::assets::normalize(relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let mut manifest = load_manifest(project_dir);
    match role {
        Some(role) if Some(role) != ImportRole::detect(&relative) => {
            manifest.roles.insert(relative.clone(), role);
        }
        _ => {
            manifest.roles.remove(&relative);
        }
    }
    save_manifest(project_dir, &manifest)?;
    inspect_asset(project_dir, &relative)
}

/// Scales an HDR file by `ev` stops wherever it is decoded, within +-16.
/// Its sky picks the change up on the next world build.
pub fn set_exposure_bias(
    project_dir: &Path,
    relative: &str,
    ev: f32,
) -> Result<PipelineReport, String> {
    let relative = crate::assets::normalize(relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let mut manifest = load_manifest(project_dir);
    if manifest.role_of(&relative) != Some(ImportRole::Hdr) {
        return Err(format!("{relative} isn't an HDR image"));
    }
    if !ev.is_finite() {
        return Err("Exposure bias must be a number".to_string());
    }
    let ev = ev.clamp(-16.0, 16.0);
    if ev == 0.0 {
        manifest.exposure_bias.remove(&relative);
    } else {
        manifest.exposure_bias.insert(relative.clone(), ev);
    }
    save_manifest(project_dir, &manifest)?;
    inspect_asset(project_dir, &relative)
}

/// What the tray shows per asset: what it is, what import would do, and
/// whether the file changed since import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineReport {
    pub path: String,
    pub kind: crate::assets::AssetKind,
    /// True when the file changed on disk (or the settings did) since the
    /// manifest entry was written. Untracked files are dirty: import is what
    /// learns their shape.
    pub dirty: bool,
    pub summary: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// What it imports as, when it is one of the roled kinds.
    #[serde(default)]
    pub role: Option<ImportRole>,
    /// Stops an HDR is scaled by when decoded.
    #[serde(default)]
    pub exposure_bias: f32,
}

/// Inspect one asset and say what the pipeline makes of it.
pub fn inspect_asset(project_dir: &Path, relative: &str) -> Result<PipelineReport, String> {
    let relative = crate::assets::normalize(relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let full = crate::assets::resolve(project_dir, &relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    if !full.is_file() {
        return Err(format!("{relative} isn't a file"));
    }
    let bytes = std::fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    let kind = crate::assets::kind_of(&relative);
    let manifest = load_manifest(project_dir);
    let role = manifest.role_of(&relative);
    let (summary, warnings) = describe(&relative, kind, role, &manifest.settings, &bytes);
    Ok(PipelineReport {
        dirty: is_dirty(&manifest, &relative, &full).unwrap_or(true),
        exposure_bias: manifest.bias_of(&relative),
        path: relative,
        kind,
        summary,
        warnings,
        role,
    })
}

fn describe(
    relative: &str,
    kind: crate::assets::AssetKind,
    role: Option<ImportRole>,
    settings: &ImportSettings,
    bytes: &[u8],
) -> (String, Vec<String>) {
    if let Some(role) = role
        && role != ImportRole::Texture
    {
        return match describe_role(relative, role, settings, bytes) {
            Ok(described) => described,
            Err(error) => (error.clone(), vec![error]),
        };
    }
    match kind {
        crate::assets::AssetKind::Model => match inspect_model(relative, bytes) {
            Ok(info) => (info.summary(), info.warnings.clone()),
            Err(error) => (error.clone(), vec![error]),
        },
        crate::assets::AssetKind::Image => match inspect_texture(relative, bytes) {
            Ok(info) => {
                let plan = decide_texture(&info, settings.texture_max);
                (
                    format!("{}x{} {}", info.width, info.height, plan.reason),
                    Vec::new(),
                )
            }
            Err(error) => (error.clone(), vec![error]),
        },
        crate::assets::AssetKind::Audio => match inspect_audio(relative, bytes) {
            Ok(info) => {
                let plan = decide_audio(&info);
                (
                    format!(
                        "{} {:.1}s {}Hz: {}",
                        info.format, info.duration_secs, info.sample_rate, plan.reason
                    ),
                    Vec::new(),
                )
            }
            Err(error) => (error.clone(), vec![error]),
        },
        crate::assets::AssetKind::Shader => (
            format!("{} bytes, custom shader source", bytes.len()),
            Vec::new(),
        ),
        _ => (
            format!("{} bytes, no pipeline step", bytes.len()),
            Vec::new(),
        ),
    }
}

fn is_dirty(manifest: &PipelineManifest, relative: &str, full: &Path) -> Result<bool, String> {
    let Some(entry) = manifest.entries.get(relative) else {
        return Ok(true);
    };
    if entry.settings != manifest.settings.fingerprint() {
        return Ok(true);
    }
    let imported_as = entry.role.or_else(|| ImportRole::detect(relative));
    if imported_as != manifest.role_of(relative) {
        return Ok(true);
    }
    let (hash, size, _) = file_fingerprint(full)?;
    Ok(hash != entry.hash || size != entry.size)
}

/// Every importable file under `assets/`, shallow-sorted, with its report.
/// Missing folders read as empty rather than an error: a fresh project has
/// no assets yet.
pub fn scan_project(project_dir: &Path) -> Vec<PipelineReport> {
    let manifest = load_manifest(project_dir);
    let mut reports = Vec::new();
    collect_reports(project_dir, &manifest, String::new(), &mut reports);
    reports.sort_by(|a, b| a.path.cmp(&b.path));
    reports
}

fn collect_reports(
    project_dir: &Path,
    manifest: &PipelineManifest,
    relative: String,
    reports: &mut Vec<PipelineReport>,
) {
    let Ok(entries) = crate::assets::list(project_dir, &relative) else {
        return;
    };
    for entry in entries {
        if entry.kind == crate::assets::AssetKind::Folder {
            collect_reports(project_dir, manifest, entry.path, reports);
            continue;
        }
        let full = match crate::assets::resolve(project_dir, &entry.path) {
            Some(full) => full,
            None => continue,
        };
        let bytes = std::fs::read(&full).unwrap_or_default();
        let role = manifest.role_of(&entry.path);
        let (summary, warnings) =
            describe(&entry.path, entry.kind, role, &manifest.settings, &bytes);
        let dirty = if full.is_file() {
            is_dirty(manifest, &entry.path, &full).unwrap_or(true)
        } else {
            true
        };
        reports.push(PipelineReport {
            exposure_bias: manifest.bias_of(&entry.path),
            path: entry.path,
            kind: entry.kind,
            dirty,
            summary,
            warnings,
            role,
        });
    }
}

/// Re-inspect `paths` (or everything dirty when empty) and refresh the
/// manifest. Returns the fresh reports.
pub fn reimport(project_dir: &Path, paths: &[String]) -> Result<Vec<PipelineReport>, String> {
    let wanted: Vec<String> = if paths.is_empty() {
        scan_project(project_dir)
            .into_iter()
            .filter(|report| report.dirty)
            .map(|report| report.path)
            .collect()
    } else {
        let mut clean = Vec::new();
        for path in paths {
            clean.push(
                crate::assets::normalize(path)
                    .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?,
            );
        }
        clean
    };
    let mut reports = Vec::new();
    for relative in wanted {
        let report = inspect_asset(project_dir, &relative)?;
        note_imported(project_dir, &relative, &report.summary)?;
        let mut fresh = inspect_asset(project_dir, &relative)?;
        // Just fingerprinted: clean by construction.
        fresh.dirty = false;
        reports.push(fresh);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gltf(meshes: usize, materials: usize, animations: usize, skins: usize) -> serde_json::Value {
        serde_json::json!({
            "asset": {"version": "2.0"},
            "meshes": vec![0; meshes],
            "materials": vec![0; materials],
            "nodes": vec![0; 3],
            "animations": vec![0; animations],
            "skins": vec![0; skins],
        })
    }

    #[test]
    fn gltf_counts_come_from_its_arrays() {
        let info = inspect_gltf_value("hero.gltf", &gltf(2, 1, 3, 1)).unwrap();
        assert_eq!(info.meshes, 2);
        assert_eq!(info.animations, 3);
        assert!(info.has_skin());
        assert!(info.has_animations());
        assert!(info.summary().contains("skinned"));
    }

    #[test]
    fn glb_reads_its_json_chunk() {
        let json = serde_json::to_string(&gltf(1, 1, 0, 0)).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(json.as_bytes());
        let info = inspect_model("hero.glb", &bytes).unwrap();
        assert_eq!(info.meshes, 1);
    }

    #[test]
    fn obj_counts_vertices_faces_and_materials() {
        let source = "o Cube\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl skin\nf 1 2 3\n";
        let info = inspect_model("cube.obj", source.as_bytes()).unwrap();
        assert_eq!(info.format, ModelFormat::Obj);
        assert_eq!(info.meshes, 1);
        assert_eq!(info.materials, 1);
        assert!(!info.has_skin());
    }

    #[test]
    fn fbx_is_sniffed_and_flagged_static() {
        let source = "Kaydara FBX Binary  \x00\x1a\x00GeometryGeometryMaterialDeformer";
        let info = inspect_model("rig.fbx", source.as_bytes()).unwrap();
        assert_eq!(info.format, ModelFormat::Fbx);
        assert_eq!(info.meshes, 2);
        assert!(info.has_skin());
        assert!(info.warnings.iter().any(|w| w.contains("static mesh")));
    }

    #[test]
    fn textures_downscale_and_transcode_by_size() {
        let small = TextureInfo {
            width: 64,
            height: 64,
            format: "png".to_string(),
            bytes: 1000,
        };
        assert_eq!(decide_texture(&small, 2048).target, TextureTarget::AsIs);
        let photo = TextureInfo {
            width: 4096,
            height: 3000,
            format: "png".to_string(),
            bytes: 1,
        };
        let plan = decide_texture(&photo, 2048);
        assert_eq!(plan.target, TextureTarget::Transcode);
        assert_eq!(plan.target_width, 2048);
    }

    #[test]
    fn wav_parses_and_plans_a_transcode() {
        // 44-byte header + 8 frames of mono 16-bit at 8000Hz.
        let mut bytes = vec![0u8; 44 + 16];
        bytes[0..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WAVE");
        bytes[22] = 1;
        bytes[24] = 0x40;
        bytes[25] = 0x1F;
        bytes[34] = 16;
        bytes[36..40].copy_from_slice(b"data");
        bytes[40] = 16;
        let info = inspect_audio("kick.wav", &bytes).unwrap();
        assert_eq!(info.channels, 1);
        assert_eq!(info.sample_rate, 8000);
        assert!(decide_audio(&info).transcode);
    }

    #[test]
    fn the_shelf_packer_lays_rows_without_overlap() {
        let inputs = vec![
            AtlasInput {
                name: "b".to_string(),
                width: 64,
                height: 64,
            },
            AtlasInput {
                name: "a".to_string(),
                width: 32,
                height: 32,
            },
            AtlasInput {
                name: "c".to_string(),
                width: 100,
                height: 16,
            },
        ];
        let layout = pack_atlas(&inputs, 256, 1).unwrap();
        assert_eq!(layout.entries.len(), 3);
        for (i, first) in layout.entries.iter().enumerate() {
            for second in &layout.entries[i + 1..] {
                let overlap = first.x < second.x + second.width
                    && second.x < first.x + first.width
                    && first.y < second.y + second.height
                    && second.y < first.y + first.height;
                assert!(!overlap, "{first:?} overlaps {second:?}");
            }
            // UVs match the pixel rect exactly.
            let [u0, v0, u1, v1] = first.uv;
            assert!((u0 * layout.width as f32 - first.x as f32).abs() < 0.01);
            assert!((v0 * layout.height as f32 - first.y as f32).abs() < 0.01);
            assert!((u1 * layout.width as f32 - (first.x + first.width) as f32).abs() < 0.01);
            assert!((v1 * layout.height as f32 - (first.y + first.height) as f32).abs() < 0.01);
        }
    }

    #[test]
    fn an_oversize_sprite_refuses_the_sheet() {
        let inputs = vec![AtlasInput {
            name: "bg".to_string(),
            width: 5000,
            height: 10,
        }];
        assert!(pack_atlas(&inputs, 2048, 0).is_err());
    }

    #[test]
    fn a_baked_sheet_holds_each_sprite_with_smeared_edges() {
        let dir = std::env::temp_dir().join(format!("blockloom-atlas-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let red = image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]));
        let blue = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]));
        red.save(dir.join("assets/red.png")).unwrap();
        blue.save(dir.join("assets/blue.png")).unwrap();

        let paths = ["assets/red.png".to_string(), "assets/blue.png".to_string()];
        let atlas = bake_atlas(&dir, &paths, 256, 1).unwrap();
        let red_at = atlas.layout.entry("assets/red.png").unwrap();
        let blue_at = atlas.layout.entry("assets/blue.png").unwrap();
        assert_eq!(
            *atlas.image.get_pixel(red_at.x, red_at.y),
            image::Rgba([255, 0, 0, 255])
        );
        assert_eq!(
            *atlas.image.get_pixel(blue_at.x + 1, blue_at.y + 1),
            image::Rgba([0, 0, 255, 255])
        );
        // The padding ring repeats the edge rather than staying clear.
        assert_eq!(
            *atlas.image.get_pixel(red_at.x - 1, red_at.y - 1),
            image::Rgba([255, 0, 0, 255])
        );

        let png = dir.join(BAKED_ATLAS_IMAGE);
        write_atlas(&atlas, &png, &dir.join(BAKED_ATLAS_LAYOUT)).unwrap();
        assert!(png.is_file());
        assert_eq!(read_baked_layout(&dir), Some(atlas.layout.clone()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn roles_follow_the_extension_unless_overridden() {
        let dir = std::env::temp_dir().join(format!("blockloom-role-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let hills = image::ImageBuffer::<image::Luma<u16>, _>::from_fn(5, 5, |x, y| {
            image::Luma([(x * y * 1000) as u16])
        });
        hills.save(dir.join("assets/hills.png")).unwrap();
        assert_eq!(
            ImportRole::detect("assets/hills.png"),
            Some(ImportRole::Texture)
        );
        assert_eq!(ImportRole::detect("assets/sky.exr"), Some(ImportRole::Hdr));
        assert_eq!(ImportRole::detect("assets/jump.wav"), None);

        reimport(&dir, &["assets/hills.png".to_string()]).unwrap();
        let report = set_role(&dir, "assets/hills.png", Some(ImportRole::Heightmap)).unwrap();
        assert_eq!(report.role, Some(ImportRole::Heightmap));
        assert!(
            report.summary.contains("16-bit heightmap"),
            "{}",
            report.summary
        );
        // A new role is a new import.
        assert!(report.dirty);
        reimport(&dir, &[]).unwrap();
        assert!(!inspect_asset(&dir, "assets/hills.png").unwrap().dirty);

        // Moving keeps the override; clearing it goes back to the extension.
        std::fs::rename(dir.join("assets/hills.png"), dir.join("assets/land.png")).unwrap();
        note_moved(&dir, "assets/hills.png", "assets/land.png");
        assert_eq!(
            load_manifest(&dir).role_of("assets/land.png"),
            Some(ImportRole::Heightmap)
        );
        let cleared =
            set_role(&dir, "assets/land.png", ImportRole::parse("auto").unwrap()).unwrap();
        assert_eq!(cleared.role, Some(ImportRole::Texture));
        assert!(ImportRole::parse("sprite").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn each_role_describes_its_file() {
        let settings = ImportSettings::default();
        let describe = |name: &str, bytes: &[u8]| {
            describe(
                name,
                crate::assets::kind_of(name),
                ImportRole::detect(name),
                &settings,
                bytes,
            )
        };
        let (summary, _) = describe("sky.hdr", b"#?RADIANCE\n\n-Y 4096 +X 8192\n");
        assert!(summary.contains("downscale to 4096x2048"), "{summary}");
        assert!(summary.contains("BC6H"), "{summary}");

        let mut cube = "LUT_3D_SIZE 2\n".to_string();
        for _ in 0..8 {
            cube += "0 0 0\n";
        }
        let (summary, warnings) = describe("grade.cube", cube.as_bytes());
        assert!(summary.starts_with("2x2x2 volume from a cube"), "{summary}");
        assert!(warnings.is_empty());

        let (summary, _) = describe(
            "down.ies",
            b"TILT=NONE\n1 1000 1 2 1 1 2 0 0 0\n1 1 10\n0 90\n0\n100 0\n",
        );
        assert!(summary.contains("peak 100 cd"), "{summary}");

        let (_, warnings) = describe("flat.r16", &[0u8; 8]);
        assert!(warnings.iter().any(|w| w.contains("same height")));
        assert!(warnings.iter().any(|w| w.contains("2^n+1")));

        let (summary, warnings) = describe("broken.ies", b"nothing here");
        assert_eq!(warnings, vec![summary]);
    }

    #[test]
    fn hdr_settings_only_dirty_when_moved() {
        let mut settings = ImportSettings::default();
        let before = settings.fingerprint();
        assert_eq!(before, crc32(b"2048:5:2048"));
        settings.hdr_max = 2048;
        assert_ne!(settings.fingerprint(), before);
    }

    #[test]
    fn the_manifest_tracks_dirt_and_reimport() {
        let dir = std::env::temp_dir().join(format!("blockloom-pipe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/a.obj"), "v 0 0 0\n").unwrap();
        // Untracked means dirty: import hasn't learned it yet.
        assert!(scan_project(&dir).iter().any(|r| r.dirty));
        reimport(&dir, &["assets/a.obj".to_string()]).unwrap();
        assert!(scan_project(&dir).iter().all(|r| !r.dirty));
        // Touching the file dirties it again.
        std::fs::write(dir.join("assets/a.obj"), "v 0 0 0\nv 1 1 1\nf 1 2 1\n").unwrap();
        assert!(scan_project(&dir).iter().any(|r| r.dirty));
        std::fs::remove_dir_all(&dir).ok();
    }
}
