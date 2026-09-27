//! Edits to Bevy's own PBR shaders for what Bevy has no switch for: true
//! disk lights, shadows for area lights and a sun shadow fade. 3D only.
//!
//! Each shader is patched as it loads, by exact text edits against the
//! pinned Bevy. An edit that no longer finds its text leaves that shader
//! alone and reports it, so a Bevy bump degrades to the old behaviour.
//!
//! - A disk is a `RectLight` with both extents negative (the side of the
//!   equal-area square), which unpatched Bevy still draws as that square.
//!   The patch integrates a 12-gon of the disk's area instead.
//! - An area light's shadow comes from a black, shadow-casting point light
//!   at exactly its position, its "twin" (`lights.rs`). The patch looks the
//!   twin up in the fragment's cluster and reads its cube shadow; black
//!   point lights are skipped in the point loop, since they light nothing.
//! - The sun's shadow fades to nothing over the last `fade` of its distance.
//! - In a browser, Bevy's material shaders stop binding textures and
//!   samplers to `let`s, which naga takes but WGSL (and Chrome) refuses.
//! - SSAO's visibility is raised to the project's AO intensity, and depth
//!   of field leaves the near field sharp when its far limit is negative
//!   (`post.rs` sets both).

use bevy::prelude::*;
use bevy::shader::Source;
use blockloom_protocol::RuntimeMessage;
use std::collections::HashMap;

/// Appended to every patched source, so a shader is never patched twice.
const MARKER: &str = "\n// blockloom: patched\n";

/// What the patched shaders are built with. `shadows.rs` writes the fade.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct PbrPatches {
    /// Fraction of the shadow distance the sun's shadows fade over.
    pub shadow_fade: f32,
    /// The power on SSAO's visibility: 1 leaves Bevy's, 0 turns it off.
    pub ao_intensity: f32,
}

impl Default for PbrPatches {
    fn default() -> Self {
        Self {
            shadow_fade: 0.1,
            ao_intensity: 1.0,
        }
    }
}

impl PbrPatches {
    /// The AO power as the shader bakes it, in twentieths so a slider
    /// doesn't recompile SSAO on every pixel it moves.
    pub fn ao_power(&self) -> f32 {
        (self.ao_intensity.clamp(0.0, 4.0) * 20.0).round() / 20.0
    }
}

/// Which patches have taken, for tests and diagnostics.
#[derive(Resource, Default, Debug)]
pub struct PatchedShaders {
    pub applied: Vec<&'static str>,
    pub failed: Vec<&'static str>,
}

struct Patch {
    /// The end of the shader's asset path.
    module: &'static str,
    edit: fn(&str, &PbrPatches) -> Result<String, &'static str>,
    /// What of the config the edit bakes in; a change re-patches only the
    /// shaders whose key moved.
    key: fn(&PbrPatches) -> u32,
}

fn no_key(_: &PbrPatches) -> u32 {
    0
}

fn fade_key(config: &PbrPatches) -> u32 {
    config.shadow_fade.to_bits()
}

fn ao_key(config: &PbrPatches) -> u32 {
    config.ao_power().to_bits()
}

const PATCHES: &[Patch] = &[
    Patch {
        module: "bevy_pbr/decal/clustered.wesl",
        edit: crate::decals::patch_shader,
        key: no_key,
    },
    Patch {
        module: "bevy_pbr/render/pbr_lighting.wesl",
        edit: patch_lighting,
        key: no_key,
    },
    Patch {
        module: "bevy_pbr/render/pbr_functions.wesl",
        edit: patch_functions,
        key: no_key,
    },
    Patch {
        module: "bevy_pbr/render/shadows.wesl",
        edit: patch_shadows,
        key: fade_key,
    },
    Patch {
        module: "bevy_pbr/ssao/ssao.wesl",
        edit: patch_ssao,
        key: ao_key,
    },
    Patch {
        module: "bevy_post_process/dof/dof.wesl",
        edit: patch_depth_of_field,
        key: no_key,
    },
];

/// Only the browser needs these: its WGSL compiler holds Bevy's shaders to
/// the spec. Bindless is never on there, so only the plain bindings matter.
#[cfg(target_arch = "wasm32")]
const WEB_PATCHES: &[Patch] = &[
    Patch {
        module: "bevy_pbr/render/pbr_fragment.wesl",
        edit: patch_handle_lets,
        key: no_key,
    },
    Patch {
        module: "bevy_pbr/render/pbr_prepass.wesl",
        edit: patch_handle_lets,
        key: no_key,
    },
    Patch {
        module: "bevy_pbr/render/pbr_prepass_functions.wesl",
        edit: patch_handle_lets,
        key: no_key,
    },
];
#[cfg(not(target_arch = "wasm32"))]
const WEB_PATCHES: &[Patch] = &[];

fn patch_at(index: usize) -> &'static Patch {
    PATCHES
        .iter()
        .chain(WEB_PATCHES)
        .nth(index)
        .expect("a known patch")
}

pub fn register(app: &mut App) {
    app.init_resource::<PbrPatches>()
        .init_resource::<PatchedShaders>()
        .add_systems(PostUpdate, patch_shaders);
}

/// Originals of every shader a patch applies to, and what they were last
/// patched with.
#[derive(Default)]
pub struct Originals {
    /// The patch, the original source and the key it was last built with.
    sources: HashMap<AssetId<Shader>, (usize, String, Option<u32>)>,
}

pub fn patch_shaders(
    mut events: MessageReader<AssetEvent<Shader>>,
    mut shaders: ResMut<Assets<Shader>>,
    config: Res<PbrPatches>,
    mut state: ResMut<PatchedShaders>,
    mut originals: Local<Originals>,
) {
    let mut dirty = Vec::new();
    for event in events.read() {
        let (AssetEvent::Added { id }
        | AssetEvent::LoadedWithDependencies { id }
        | AssetEvent::Modified { id }) = event
        else {
            continue;
        };
        if originals.sources.contains_key(id) {
            continue;
        }
        let Some(shader) = shaders.get(*id) else {
            continue;
        };
        let Some(index) = PATCHES
            .iter()
            .chain(WEB_PATCHES)
            .position(|p| shader.path.ends_with(p.module))
        else {
            continue;
        };
        let source = shader.source.as_str();
        if source.contains(MARKER) {
            continue;
        }
        originals
            .sources
            .insert(*id, (index, source.to_string(), None));
    }
    for (id, (index, _, built)) in &mut originals.sources {
        let key = (patch_at(*index).key)(&config);
        if *built != Some(key) {
            *built = Some(key);
            dirty.push(*id);
        }
    }
    for id in dirty {
        let (index, original, _) = &originals.sources[&id];
        let patch = patch_at(*index);
        match (patch.edit)(original, &config) {
            Ok(mut patched) => {
                patched.push_str(MARKER);
                if let Some(mut shader) = shaders.get_mut(id) {
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
                            "Couldn't patch {} ({missing}); its PBR extensions fall back to Bevy's own shader",
                            patch.module
                        ),
                    });
                }
            }
        }
    }
}

/// `let texture = pbr_bindings::x;` (and `sampler_`) becomes the phony
/// `_ = pbr_bindings::x;`, still a statement for the `@else` in front of
/// it, and the bare `texture`/`sampler_` arguments after it name the
/// binding itself. The bindless `let`s are left as they are: that branch
/// never survives in a browser.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn patch_handle_lets(source: &str, _: &PbrPatches) -> Result<String, &'static str> {
    const NAMES: [&str; 2] = ["texture", "sampler_"];
    let mut out = String::with_capacity(source.len());
    let mut bound: [Option<String>; 2] = [None, None];
    let mut rest = source;
    let mut edits = 0;
    while !rest.is_empty() {
        // A plain `let` of one of the two names.
        if let Some((slot, binding, len)) = NAMES.iter().enumerate().find_map(|(slot, name)| {
            let head = format!("let {name} = pbr_bindings::");
            let tail = rest.strip_prefix(&head)?;
            let end = tail.find(';')?;
            let binding = &tail[..end];
            binding
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                .then(|| {
                    (
                        slot,
                        format!("pbr_bindings::{binding}"),
                        head.len() + end + 1,
                    )
                })
        }) {
            out.push_str(&format!("_ = {binding};"));
            bound[slot] = Some(binding);
            rest = &rest[len..];
            edits += 1;
            continue;
        }
        let c = rest.chars().next().unwrap_or_default();
        let word_start =
            !out.ends_with(|p: char| p.is_ascii_alphanumeric() || p == '_' || p == '.' || p == ':');
        // A bindless `let` keeps its own name.
        let after_let = out.ends_with("let ");
        if word_start && !after_let {
            let used = NAMES.iter().enumerate().find_map(|(slot, name)| {
                let tail = rest.strip_prefix(name)?;
                let whole = !tail.starts_with(|n: char| n.is_ascii_alphanumeric() || n == '_');
                (whole && bound[slot].is_some()).then_some((slot, name.len()))
            });
            if let Some((slot, len)) = used {
                out.push_str(bound[slot].as_deref().unwrap_or_default());
                rest = &rest[len..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if edits == 0 {
        return Err("its texture lets");
    }
    Ok(out)
}

/// `source` with `from` replaced by `to`, exactly once.
pub(crate) fn replace_once(
    source: &str,
    from: &str,
    to: &str,
    what: &'static str,
) -> Result<String, &'static str> {
    if source.matches(from).count() != 1 {
        return Err(what);
    }
    Ok(source.replacen(from, to, 1))
}

/// SSAO's visibility to the power of the project's AO intensity, before
/// Bevy's floor so fully open sky stays open.
fn patch_ssao(source: &str, config: &PbrPatches) -> Result<String, &'static str> {
    let power = config.ao_power();
    replace_once(
        source,
        "    visibility = clamp(visibility, 0.03, 1.0);",
        &format!("    visibility = clamp(pow(visibility, {power:?}), 0.03, 1.0);"),
        "the visibility clamp",
    )
}

/// A negative far limit means the near field stays sharp: the limit is its
/// size, and anything nearer than the focus has no blur.
fn patch_depth_of_field(source: &str, _: &PbrPatches) -> Result<String, &'static str> {
    let source = replace_once(
        source,
        "    let depth = min(-depth_ndc_to_view_z(raw_depth), dof_params.max_depth);",
        "    let depth = min(-depth_ndc_to_view_z(raw_depth), abs(dof_params.max_depth));
             if dof_params.max_depth < 0.0 && depth < focus {
        return 0.0;
    }",
        "the depth clamp",
    )?;
    Ok(source)
}

/// Disk lights: `rect_light` integrates through `blockloom_area_integral`,
/// which falls through to the quad for a rect.
fn patch_lighting(source: &str, _: &PbrPatches) -> Result<String, &'static str> {
    let start = source.find("fn rect_light(").ok_or("rect_light")?;
    let (head, body) = source.split_at(start);
    if body.matches("ltc_integrate_quad(").count() == 0 {
        return Err("rect_light's integrals");
    }
    let body = body.replace("ltc_integrate_quad(", "blockloom_area_integral(light, ");
    let head = replace_once(
        head,
        "fn ltc_integrate_quad(",
        &format!("{DISK_INTEGRAL}\nfn ltc_integrate_quad("),
        "ltc_integrate_quad",
    )?;
    Ok(head + &body)
}

const DISK_INTEGRAL: &str = r#"
// Blockloom: a rect light with both extents negative is a disk, given as
// the side of the square of its area. It integrates as a 12-gon of the same
// area, wound the way the quad's corners are.
fn blockloom_area_integral(
    light: ptr<function, RectLight>,
    N: vec3<f32>,
    V: vec3<f32>,
    P: vec3<f32>,
    Minv: mat3x3<f32>,
    points: array<vec3<f32>, 4>
) -> f32 {
    if ((*light).width >= 0.0 || (*light).height >= 0.0) {
        return ltc_integrate_quad(N, V, P, Minv, points);
    }
    let T1 = normalize(V - N * dot(V, N));
    let T2 = -cross(N, T1);
    let Minv_local = Minv * transpose(mat3x3<f32>(T1, T2, N));
    // side / sqrt(pi) is the disk's radius; sqrt(pi / 3) more gives the 12-gon.
    let r = abs((*light).width) * 0.57735027;
    let step = 0.52359878;

    var clipped: array<vec3<f32>, 13>;
    var n_clipped: i32 = 0;
    for (var i = 0i; i < 12i; i++) {
        let a0 = f32(i) * step;
        let a1 = f32(i + 1) * step;
        let a = Minv_local * ((*light).position + r * (cos(a0) * (*light).right - sin(a0) * (*light).up) - P);
        let b = Minv_local * ((*light).position + r * (cos(a1) * (*light).right - sin(a1) * (*light).up) - P);
        if (a.z >= 0.0) {
            clipped[n_clipped] = a;
            n_clipped++;
        }
        if ((a.z >= 0.0) != (b.z >= 0.0)) {
            clipped[n_clipped] = mix(a, b, a.z / (a.z - b.z));
            n_clipped++;
        }
    }
    if (n_clipped == 0) {
        return 0.0;
    }
    for (var i = 0i; i < n_clipped; i++) {
        clipped[i] = normalize(clipped[i]);
    }
    var sum = 0.0;
    for (var i = 0i; i < n_clipped; i++) {
        sum += ltc_integrate_edge(clipped[i], clipped[(i + 1) % n_clipped]);
    }
    return sum;
}
"#;

/// Area light shadows through their twins, and black point lights skipped.
fn patch_functions(source: &str, _: &PbrPatches) -> Result<String, &'static str> {
    let point_loop =
        "            i < clusterable_object_index_ranges.first_spot_light_index_offset;
            i = i + 1u) {
        let light_id = clustering::get_clusterable_object_id(i);
";
    let source = replace_once(
        source,
        point_loop,
        &format!(
            "{point_loop}        // Blockloom: a black point light is an area light's shadow twin.
        if all(view_bindings::clustered_lights.data[light_id].color_inverse_square_range.xyz == vec3<f32>(0.0)) {{
            continue;
        }}
"
        ),
        "the point light loop",
    )?;
    replace_once(
        &source,
        "        let light_contrib = lighting::rect_light(&light, &lighting_input, enable_diffuse);
        direct_light += light_contrib;
",
        "        // Blockloom: the shadow of the black point light standing exactly where this one does.
        var area_shadow: f32 = 1.0;
        if ((in.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u) {
            for (var j: u32 = clusterable_object_index_ranges.first_point_light_index_offset;
                    j < clusterable_object_index_ranges.first_spot_light_index_offset;
                    j = j + 1u) {
                let twin = clustering::get_clusterable_object_id(j);
                let data = view_bindings::clustered_lights.data[twin];
                if (all(data.color_inverse_square_range.xyz == vec3<f32>(0.0))
                        && all(data.position_radius.xyz == light.position)
                        && (data.flags & mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
                    area_shadow = shadows::fetch_point_shadow(twin, in.world_position, in.world_normal, in.frag_coord.xy);
                    break;
                }
            }
        }
        let light_contrib = lighting::rect_light(&light, &lighting_input, enable_diffuse);
        direct_light += light_contrib * area_shadow;
",
        "the rect light loop",
    )
}

/// The sun's shadow fades out over the last `fade` of the last cascade.
fn patch_shadows(source: &str, config: &PbrPatches) -> Result<String, &'static str> {
    let fade = config.shadow_fade.clamp(0.0, 0.5);
    let source = replace_once(
        source,
        "fn fetch_directional_shadow(",
        &format!(
            "// Blockloom: how much of the shadow distance the sun's shadows fade over.\n\
             const BLOCKLOOM_SHADOW_FADE: f32 = {fade:?};\n\nfn fetch_directional_shadow("
        ),
        "fetch_directional_shadow",
    )?;
    replace_once(
        &source,
        "            shadow = mix(shadow, next_shadow, (-view_z - next_near_bound) / (this_far_bound - next_near_bound));
        }
    }
    return shadow;
",
        "            shadow = mix(shadow, next_shadow, (-view_z - next_near_bound) / (this_far_bound - next_near_bound));
        }
    }
    if (BLOCKLOOM_SHADOW_FADE > 0.0) {
        let far = (*light).cascades[(*light).num_cascades - 1u].far_bound;
        shadow = mix(shadow, 1.0, smoothstep(far * (1.0 - BLOCKLOOM_SHADOW_FADE), far, -view_z));
    }
    return shadow;
",
        "the cascade blend",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texture_let_becomes_the_binding_it_named() {
        let source = "\
@if(BINDLESS)
        let texture = bindless_textures_2d[material_indices[slot].normal_map_texture];
@else
        let texture = pbr_bindings::normal_map_texture;
@if(BINDLESS)
        let sampler_ = bindless_samplers_filtering[material_indices[slot].normal_map_sampler];
@else
        let sampler_ = pbr_bindings::normal_map_sampler;
        let Nt = textureSampleBias(
                texture,
                sampler_,
                uv, texture_2d_size, view.texture).rgb;
";
        let patched = patch_handle_lets(source, &PbrPatches::default()).unwrap();
        assert!(patched.contains("@else\n        _ = pbr_bindings::normal_map_texture;"));
        assert!(patched.contains("@else\n        _ = pbr_bindings::normal_map_sampler;"));
        // The bindless branch keeps its own lets.
        assert!(patched.contains("let texture = bindless_textures_2d"));
        assert!(patched.contains("let sampler_ = bindless_samplers_filtering"));
        assert!(patched.contains(
            "textureSampleBias(\n                pbr_bindings::normal_map_texture,\n                pbr_bindings::normal_map_sampler,"
        ));
        // Longer names and fields that only start the same are untouched.
        assert!(patched.contains("uv, texture_2d_size, view.texture).rgb;"));
        assert!(patch_handle_lets("fn f() {}", &PbrPatches::default()).is_err());
    }
}
