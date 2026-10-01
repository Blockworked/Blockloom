//! Edits to Bevy Solari's shaders, 3D only: the sky lights what a traced
//! ray escapes to. Since Bevy 0.20.0-rc.2 Solari samples the environment map
//! on a miss itself, so these edits are no-ops there and only apply to older
//! shaders that bind the map but never read it.
//!
//! Same rules as `pbr_patch`: exact text against the pinned Bevy, patched as
//! each shader loads, and an edit that no longer finds its text leaves that
//! shader alone and says so.
//!
//! - A realtime path adds the sky where one of its bounces escapes.
//! - A world cache GI ray does too, once a second ray past its reach has
//!   escaped as well, so a cell deep inside a big hall isn't lit by a sky it
//!   can't see.
//! - The reference path tracer adds it on every bounce but the camera's own
//!   ray, whose miss is left to the raster background (sun disk, stars).

use crate::pbr_patch::replace_once;
use bevy::prelude::*;
use bevy::shader::Source;
use blockloom_protocol::RuntimeMessage;
use std::collections::HashSet;

/// Appended to every patched source, so a shader is never patched twice.
const MARKER: &str = "\n// blockloom: solari patched\n";

struct Patch {
    /// The end of the shader's asset path.
    module: &'static str,
    edit: fn(&str) -> Result<String, &'static str>,
}

const PATCHES: &[Patch] = &[
    Patch {
        module: "bevy_solari/realtime/initial_path.wesl",
        edit: patch_initial_path,
    },
    Patch {
        module: "bevy_solari/realtime/world_cache_update.wesl",
        edit: patch_world_cache,
    },
    Patch {
        module: "bevy_solari/pathtracer/pathtracer.wesl",
        edit: patch_pathtracer,
    },
];

/// Which patches have taken, for tests and diagnostics.
#[derive(Resource, Default, Debug)]
pub struct SolariPatches {
    pub applied: Vec<&'static str>,
    pub failed: Vec<&'static str>,
}

pub fn register(app: &mut App) {
    app.init_resource::<SolariPatches>()
        .add_systems(PostUpdate, patch_solari_shaders);
}

fn patch_solari_shaders(
    mut events: MessageReader<AssetEvent<Shader>>,
    mut shaders: ResMut<Assets<Shader>>,
    mut state: ResMut<SolariPatches>,
    mut seen: Local<HashSet<AssetId<Shader>>>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id }
        | AssetEvent::LoadedWithDependencies { id }
        | AssetEvent::Modified { id }) = event
        else {
            continue;
        };
        let Some(shader) = shaders.get(*id) else {
            continue;
        };
        let Some(patch) = PATCHES.iter().find(|p| shader.path.ends_with(p.module)) else {
            continue;
        };
        let source = shader.source.as_str();
        if source.contains(MARKER) || !seen.insert(*id) {
            continue;
        }
        match (patch.edit)(source) {
            Ok(mut patched) => {
                patched.push_str(MARKER);
                if let Some(mut shader) = shaders.get_mut(*id) {
                    shader.source = Source::Wesl(patched.into());
                }
                if !state.applied.contains(&patch.module) {
                    state.applied.push(patch.module);
                }
            }
            Err(missing) => {
                if !state.failed.contains(&patch.module) {
                    state.failed.push(patch.module);
                    crate::bridge::send(&RuntimeMessage::Error {
                        actor: "Blockloom".into(),
                        message: format!(
                            "Couldn't patch {} ({missing}); traced light won't see the sky",
                            patch.module
                        ),
                    });
                }
            }
        }
    }
}

/// A bounce that escapes picks up the sky, weighted by everything the path
/// carried to it. With ReSTIR the first hit's BRDF is kept out of the
/// throughput until shading, so it goes back in here; the first bounce hasn't
/// had it taken out yet.
fn patch_initial_path(source: &str) -> Result<String, &'static str> {
    // Since rc.2 Bevy samples the map on a miss itself, with proper ReSTIR
    // handling; nothing left to add.
    if source.contains("sample_environment_map_light") {
        return Ok(source.to_string());
    }
    let source = replace_once(
        source,
        "import package::scene::sampling::{calculate_resolved_light_contribution, isinf,",
        "import package::scene::sampling::{sample_environment_map_light, calculate_resolved_light_contribution, isinf,",
        "the sampling import",
    )?;
    replace_once(
        &source,
        "            break;
        }

        let ray_hit = resolve_ray_hit_full(ray);
        let p_brdf = next_bounce.pdf;
",
        "            // Blockloom: the sky lights what escapes.
            var escaped = path.throughput_past_first_hit * next_bounce.throughput * sample_environment_map_light(next_bounce.wi);
            @if(RESTIR) {
                if bounce > 0u { escaped *= path.x1_brdf; }
            }
            path.radiance += escaped;

            break;
        }

        let ray_hit = resolve_ray_hit_full(ray);
        let p_brdf = next_bounce.pdf;
",
        "the escaped bounce",
    )
}

/// A cell's GI ray that escapes its reach, and then the world, sees the sky.
/// The cache holds irradiance, so a cosine-sampled sky counts pi times.
fn patch_world_cache(source: &str) -> Result<String, &'static str> {
    // Since rc.2 Bevy adds pi times the map on a miss itself.
    if source.contains("sample_environment_map_light") {
        return Ok(source.to_string());
    }
    let source = replace_once(
        source,
        "import package::scene::sampling::{calculate_resolved_light_contribution, trace_visibility};",
        "import package::scene::sampling::{calculate_resolved_light_contribution, sample_environment_map_light, trace_visibility};",
        "the sampling import",
    )?;
    let source = replace_once(
        &source,
        "import package::scene::bindings::{trace_ray, resolve_ray_hit_full, RAY_T_MIN};",
        "import package::scene::bindings::{trace_ray, resolve_ray_hit_full, RAY_T_MIN, RAY_T_MAX};",
        "the bindings import",
    )?;
    replace_once(
        &source,
        "        world_cache.active_cells_new_radiance[active_cell_id.x] += ray_hit.material.base_color * radiance;
    }
}
",
        "        world_cache.active_cells_new_radiance[active_cell_id.x] += ray_hit.material.base_color * radiance;
    } else {
        // Blockloom: past the GI reach, the sky if nothing else is out there.
        let origin = geometry_data.world_position + (geometry_data.world_normal * RAY_T_MIN);
        let beyond = trace_ray(origin, ray_direction, constants.world_cache_max_gi_ray_distance, RAY_T_MAX, RAY_FLAG_TERMINATE_ON_FIRST_HIT);
        if beyond.kind == RAY_QUERY_INTERSECTION_NONE {
            world_cache.active_cells_new_radiance[active_cell_id.x] += 3.14159265 * sample_environment_map_light(ray_direction);
        }
    }
}
",
        "the GI ray",
    )
}

/// Bounces that escape see the sky; a camera ray that does is left out of
/// the image, so the view keeps the background the raster passes drew.
fn patch_pathtracer(source: &str) -> Result<String, &'static str> {
    // Since rc.2 Bevy adds the map on every miss, camera ray included; the
    // raster background no longer shows through the reference tracer.
    if source.contains("sample_environment_map_light") {
        return Ok(source.to_string());
    }
    let source = replace_once(
        source,
        "import package::scene::sampling::{sample_random_light, random_emissive_light_pdf, power_heuristic};",
        "import package::scene::sampling::{sample_random_light, random_emissive_light_pdf, power_heuristic, sample_environment_map_light};",
        "the sampling import",
    )?;
    replace_once(
        &source,
        "        } else { break; }
    }
",
        "        } else {
            // Blockloom: the sky, or the raster background for a camera ray.
            if p_bounce == 0.0 { return; }
            radiance += throughput * sample_environment_map_light(ray_direction);
            break;
        }
    }
",
        "the escaped ray",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edit_names_what_it_could_not_find() {
        for patch in PATCHES {
            assert!((patch.edit)("").is_err(), "{}", patch.module);
        }
    }

    #[test]
    fn a_patched_path_adds_the_sky_before_it_gives_up() {
        let source = "import package::scene::sampling::{calculate_resolved_light_contribution, isinf, LightSample};
        if ray.kind == RAY_QUERY_INTERSECTION_NONE {
            break;
        }

        let ray_hit = resolve_ray_hit_full(ray);
        let p_brdf = next_bounce.pdf;
";
        let patched = patch_initial_path(source).unwrap();
        let sky = patched
            .find("sample_environment_map_light(next_bounce.wi)")
            .unwrap();
        assert!(sky < patched.find("break;").unwrap());
    }
}
