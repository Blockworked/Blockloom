//! Environment assets: a scene's look (sky, sun, fog, clouds, wind, post and
//! the 2D look) saved as a `.blockenv` that any number of scenes follow. A
//! scene keeps its own copy in its `World`, rebuilt from the file plus the
//! paths it overrides (`fog.height`, `sky.kind`). `.blocklighting` assets
//! still name just the lighting.

use crate::linked::{self, AssetLink};
use crate::scene::World;
use serde_json::{Map, Value, json};
use std::path::Path;

pub const EXTENSION: &str = "blockenv";
const FORMAT: u32 = 1;

/// The `World` fields an environment carries.
pub const KEYS: &[&str] = &[
    "background",
    "lighting",
    "post",
    "sky",
    "fog",
    "clouds",
    "cloud_layers",
    "lightning",
    "wind",
    "director",
    "surface",
    "lighting2d",
    "post2d",
];

pub fn is_environment(path: &str) -> bool {
    path.to_lowercase().ends_with(&format!(".{EXTENSION}"))
}

/// The environment half of `world`, as one JSON object.
pub fn extract(world: &World) -> Value {
    let Ok(Value::Object(all)) = serde_json::to_value(world) else {
        return Value::Object(Map::new());
    };
    let mut out = Map::new();
    for key in KEYS {
        if let Some(value) = all.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    // A lighting asset link belongs to the scene, not the environment.
    if let Some(Value::Object(lighting)) = out.get_mut("lighting") {
        lighting.remove("asset");
    }
    Value::Object(out)
}

/// `world` with its environment fields taken from `env`. Fields `env`
/// doesn't name stay as they are.
pub fn apply(world: &World, env: &Value) -> Result<World, String> {
    let mut all = serde_json::to_value(world).map_err(|e| e.to_string())?;
    let asset = world.lighting.asset.clone();
    if let (Value::Object(all), Value::Object(env)) = (&mut all, env) {
        for key in KEYS {
            if let Some(value) = env.get(*key) {
                all.insert((*key).to_string(), value.clone());
            }
        }
    }
    let mut next: World = serde_json::from_value(all).map_err(|e| e.to_string())?;
    // Legacy fields are read-only, so they don't survive the round trip.
    next.legacy_quality = world.legacy_quality.clone();
    next.legacy_sound = world.legacy_sound;
    next.legacy_input = world.legacy_input.clone();
    next.legacy_display = world.legacy_display;
    next.lighting.asset = asset;
    Ok(next)
}

pub fn read(dir: &Path, relative: &str) -> Result<Value, String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let text = crate::vfs::read_to_string(&full).map_err(|e| format!("{relative}: {e}"))?;
    let mut file: Value = serde_json::from_str(&text).map_err(|e| format!("{relative}: {e}"))?;
    let format = file.get("format").and_then(Value::as_u64).unwrap_or(0);
    if format > FORMAT as u64 {
        return Err(format!(
            "{relative} was saved by a newer Blockloom (environment format {format})"
        ));
    }
    let env = file
        .get_mut("environment")
        .map(Value::take)
        .ok_or_else(|| format!("{relative} isn't an environment"))?;
    // Read through a world so a file that doesn't fit is refused here.
    let world = apply(&World::default(), &env).map_err(|e| format!("{relative}: {e}"))?;
    Ok(extract(&world))
}

pub fn write(dir: &Path, relative: &str, env: &Value) -> Result<(), String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = json!({"format": FORMAT, "type": "environment", "environment": env});
    let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    text.push('\n');
    crate::files::write_atomic(&full, text)
}

/// `world` as the file says now, keeping every overridden path.
pub fn rebuild(world: &World, file: &Value, link: &AssetLink) -> World {
    let merged = linked::merge(&extract(world), file, &link.overrides);
    apply(world, &merged).unwrap_or_else(|_| world.clone())
}

/// Records which paths an edit from `before` to `now` overrode.
pub fn mark(before: &World, now: &World, file: &Value, link: &mut AssetLink) {
    linked::mark(&extract(before), &extract(now), file, &mut link.overrides);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_keeps_its_fog_when_the_file_changes() {
        let base = World::default();
        let file = extract(&base);
        let mut link = AssetLink::default();
        let mut own = base.clone();
        own.background = "#112233".into();
        mark(&base, &own, &file, &mut link);
        assert_eq!(link.overrides, ["background"]);
        let mut other = base.clone();
        other.background = "#FFFFFF".into();
        other.fixed_rate = 30.0;
        other.wind.speed = 9.0;
        let next = extract(&other);
        let out = rebuild(&own, &next, &link);
        assert_eq!(out.background, "#112233");
        assert_eq!(out.wind.speed, 9.0);
        assert_eq!(out.fixed_rate, base.fixed_rate, "not an environment field");
    }
}
