//! Material assets: a surface saved as a `.blockmaterial` resource that any
//! number of actors follow. An actor's `Material` component carries the link
//! and its own copy, rebuilt from the file plus the paths it overrides
//! (`roughness`, `shader.source`), so the runtime only ever sees the copy.

use crate::components::ActorComponent;
use crate::linked::AssetLink;
use crate::material::SurfaceMaterial;
use crate::project::Actor;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const EXTENSION: &str = "blockmaterial";
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct MaterialFile {
    #[serde(default)]
    format: u32,
    #[serde(rename = "type", default)]
    kind: String,
    material: SurfaceMaterial,
}

pub fn is_material(path: &str) -> bool {
    path.to_lowercase().ends_with(&format!(".{EXTENSION}"))
}

pub fn read(dir: &Path, relative: &str) -> Result<SurfaceMaterial, String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let text = crate::vfs::read_to_string(&full).map_err(|e| format!("{relative}: {e}"))?;
    let file: MaterialFile = serde_json::from_str(&text).map_err(|e| format!("{relative}: {e}"))?;
    if file.format > FORMAT {
        return Err(format!(
            "{relative} was saved by a newer Blockloom (material format {})",
            file.format
        ));
    }
    let mut material = file.material;
    material.normalize();
    Ok(material)
}

pub fn write(dir: &Path, relative: &str, material: &SurfaceMaterial) -> Result<(), String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut material = material.clone();
    material.normalize();
    let file = MaterialFile {
        format: FORMAT,
        kind: "material".into(),
        material,
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    text.push('\n');
    crate::files::write_atomic(&full, text)
}

/// The link on `actor`'s material, if it follows a file.
pub fn link(actor: &Actor) -> Option<&AssetLink> {
    actor.components.iter().find_map(|c| match c {
        ActorComponent::Material { source, .. } => source.as_ref(),
        _ => None,
    })
}

/// The actor's material and its link, if it follows a file.
pub fn linked_mut(actor: &mut Actor) -> Option<(&mut SurfaceMaterial, &mut AssetLink)> {
    actor.components.iter_mut().find_map(|c| match c {
        ActorComponent::Material {
            material,
            source: Some(link),
        } => Some((material, link)),
        _ => None,
    })
}

/// `own` as the file says now, keeping every overridden path.
pub fn rebuild(own: &SurfaceMaterial, file: &SurfaceMaterial, link: &AssetLink) -> SurfaceMaterial {
    crate::linked::merged(own, file, &link.overrides)
}

/// Records which paths an edit from `before` to `now` overrode.
pub fn mark(
    before: &SurfaceMaterial,
    now: &SurfaceMaterial,
    file: &SurfaceMaterial,
    link: &mut AssetLink,
) {
    // Compared as the inspector would write them, so clamping isn't an edit.
    let json = |m: &SurfaceMaterial| {
        let mut m = m.clone();
        m.normalize();
        serde_json::to_value(m).unwrap_or_default()
    };
    crate::linked::mark(&json(before), &json(now), &json(file), &mut link.overrides);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instance_keeps_its_roughness_when_the_file_changes() {
        let file = SurfaceMaterial::default();
        let mut link = AssetLink::default();
        let mut own = file.clone();
        own.roughness = 0.1;
        mark(&file, &own, &file, &mut link);
        assert_eq!(link.overrides, ["roughness"]);
        let mut next = file.clone();
        next.metallic = 1.0;
        next.roughness = 0.9;
        let out = rebuild(&own, &next, &link);
        assert_eq!(out.metallic, 1.0);
        assert_eq!(out.roughness, 0.1);
    }
}
