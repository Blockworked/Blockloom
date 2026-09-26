// Volumetric fog in a froxel grid over the camera's first `range` metres.
// `inject` works out each froxel's medium and the light it scatters towards
// the camera - shadowed sun and moon, the lights that ask for it, the
// ambient light and the fog's own glow - blended with last frame's grid
// reprojected. `integrate` then marches each column front to back, so the
// composite reads what lies between the camera and any depth in one fetch.

import bevy_pbr::render::mesh_view_types::{Lights, DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT};
import blockloom::fog::{
    FogUniforms, FLAG_HISTORY, FLAG_SUN_VOLUMETRIC, MAX_LOCAL_FOG, MAX_FOG_LIGHTS,
    slice_distance, distance_slice, local_coverage, beam_fade, cone_window,
};
import blockloom::scattering::henyey_greenstein;
import blockloom::fbm::fbm3;
import blockloom::hash::{pcg3, unit};

@group(0) @binding(0) var<uniform> fog: FogUniforms;
@group(0) @binding(1) var<uniform> lights: Lights;
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var history: texture_3d<f32>;
@group(0) @binding(5) var history_sampler: sampler;
@group(0) @binding(6) var injected: texture_storage_3d<rgba16float, write>;

@group(0) @binding(7) var medium: texture_3d<f32>;
@group(0) @binding(8) var integrated: texture_storage_3d<rgba16float, write>;

// The world direction through a uv on screen.
fn ray(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // Reverse Z: 1 is the near plane; 0.5 stays finite with an infinite far.
    let near = fog.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let far = fog.world_from_clip * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(far.xyz / far.w - near.xyz / near.w);
}

// How much light reaches `p` from directional light `index`, 0-1.
fn sun_shadow(index: u32, p: vec3<f32>) -> f32 {
    let light = &lights.directional_lights[index];
    let depth_along = dot(p - fog.camera.xyz, fog.forward.xyz);
    var cascade = (*light).num_cascades;
    for (var i = 0u; i < (*light).num_cascades; i++) {
        if depth_along < (*light).cascades[i].far_bound {
            cascade = i;
            break;
        }
    }
    if cascade >= (*light).num_cascades {
        return 1.0;
    }
    let clip = (*light).cascades[cascade].clip_from_world * vec4<f32>(p, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    if any(ndc.xy < vec2<f32>(-1.0)) || ndc.z < 0.0 || any(ndc > vec3<f32>(1.0)) {
        return 1.0;
    }
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let layer = i32((*light).depth_texture_base_index + cascade);
    return textureSampleCompareLevel(shadow_maps, shadow_sampler, uv, layer, ndc.z);
}

struct Medium {
    scattering: vec3<f32>,
    extinction: f32,
    emissive: vec3<f32>,
}

fn medium_at(p: vec3<f32>) -> Medium {
    var out: Medium;
    let v = fog.volumetric;
    var global = v.x * exp(clamp(-v.y * (p.y - v.z), -80.0, 8.0));
    let amount = fog.noise.w;
    if amount > 0.0 && global > 0.0 {
        let n = fbm3((p + fog.noise.xyz) * fog.noise_scale.x, 3u, 2.0, 0.5) * 0.5 + 0.5;
        global *= max(mix(1.0, n * 2.0, amount), 0.0);
    }
    out.extinction = global;
    out.scattering = global * fog.volumetric_albedo.rgb;
    out.emissive = global * fog.volumetric_emissive.rgb;
    for (var i = 0u; i < min(fog.counts.x, MAX_LOCAL_FOG); i++) {
        let local = fog.locals[i];
        let density = local.albedo.w * local_coverage(local, p);
        out.extinction += density;
        out.scattering += density * local.albedo.rgb;
        out.emissive += density * local.emissive.rgb;
    }
    return out;
}

struct Lit {
    // Light per unit of the fog's own scattering.
    fog: vec3<f32>,
    // Light the beams scatter themselves, and their extinction.
    beams: vec3<f32>,
    beam_extinction: f32,
}

// Light arriving at `p` that scatters along `-view`, plus what the lights'
// own beams scatter there.
fn light_at(p: vec3<f32>, view: vec3<f32>) -> Lit {
    var out: Lit;
    let g = fog.volumetric.w;
    out.fog = fog.ambient.rgb * fog.volumetric_albedo.w;
    if (fog.grid.w & FLAG_SUN_VOLUMETRIC) != 0u {
        for (var i = 0u; i < lights.n_directional_lights; i++) {
            let sun = &lights.directional_lights[i];
            if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT) == 0u {
                continue;
            }
            let towards = (*sun).direction_to_light;
            let phase = henyey_greenstein(dot(view, towards), g);
            out.fog += (*sun).color.rgb * phase * sun_shadow(i, p);
        }
    }
    for (var i = 0u; i < min(fog.counts.y, MAX_FOG_LIGHTS); i++) {
        let lamp = fog.lights[i];
        let offset = lamp.position.xyz - p;
        let d2 = max(dot(offset, offset), 1e-4);
        let range = lamp.position.w;
        // Bevy's own range window, so the fog fades where the light does.
        let factor = d2 / (range * range);
        let window = clamp(1.0 - factor * factor, 0.0, 1.0);
        let towards = offset * inverseSqrt(d2);
        let cone = cone_window(dot(-towards, lamp.direction.xyz), lamp.direction.w, lamp.cone.x);
        let arriving = lamp.color.rgb * window * window / d2 * cone
            * henyey_greenstein(dot(view, towards), lamp.beam.y);
        out.fog += arriving;
        if lamp.beam.x > 0.0 && cone > 0.0 {
            let density = lamp.beam.x
                * beam_fade(sqrt(d2), range, lamp.cone.y, lamp.cone.z, lamp.beam.z);
            out.beams += arriving * density;
            out.beam_extinction += density;
        }
    }
    return out;
}

@compute @workgroup_size(4, 4, 4)
fn inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let grid = fog.grid.xyz;
    if any(id >= grid) {
        return;
    }
    let frame = u32(fog.forward.w);
    let jitter = (unit(pcg3(vec3<u32>(id.xy, id.z + frame * 131u)).x) - 0.5) * fog.noise_scale.z;
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(grid.xy);
    let w = (f32(id.z) + 0.5 + jitter) / f32(grid.z);
    let range = fog.volumetric_emissive.w;
    let view = ray(uv);
    let p = fog.camera.xyz + view * slice_distance(w, range);

    let m = medium_at(p);
    var value = vec4<f32>(0.0);
    // No medium and no beams: nothing to light.
    if m.extinction > 0.0 || fog.counts.w > 0u {
        let lit = light_at(p, view);
        let scattered = m.scattering * lit.fog + m.emissive + lit.beams;
        // Light is stored exposed, so bright fog stays inside FP16.
        value = vec4<f32>(scattered * fog.camera.w, m.extinction + lit.beam_extinction);
    }

    if (fog.grid.w & FLAG_HISTORY) != 0u {
        // Where this froxel's centre sat in last frame's grid.
        let centre = fog.camera.xyz + view * slice_distance((f32(id.z) + 0.5) / f32(grid.z), range);
        let clip = fog.previous_clip_from_world * vec4<f32>(centre, 1.0);
        if clip.w > 0.0 {
            let ndc = clip.xy / clip.w;
            let previous_uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            let previous_w = distance_slice(length(centre - fog.previous_camera.xyz), range);
            let coord = vec3<f32>(previous_uv, previous_w);
            if all(coord >= vec3<f32>(0.0)) && all(coord <= vec3<f32>(1.0)) {
                let old = textureSampleLevel(history, history_sampler, coord, 0.0);
                value = mix(old, value, fog.noise_scale.y);
            }
        }
    }
    textureStore(injected, id, value);
}

@compute @workgroup_size(8, 8, 1)
fn integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let grid = fog.grid.xyz;
    if any(id.xy >= grid.xy) {
        return;
    }
    let range = fog.volumetric_emissive.w;
    var scattered = vec3<f32>(0.0);
    var through = 1.0;
    for (var z = 0u; z < grid.z; z++) {
        let v = textureLoad(medium, vec3<u32>(id.xy, z), 0);
        let near = slice_distance(f32(z) / f32(grid.z), range);
        let far = slice_distance(f32(z + 1u) / f32(grid.z), range);
        let extinction = max(v.a, 1e-7);
        let lost = exp(-extinction * (far - near));
        // Energy-conserving over the slice (Hillaire 2015).
        scattered += through * (v.rgb - v.rgb * lost) / extinction;
        through *= lost;
        textureStore(integrated, vec3<u32>(id.xy, z), vec4<f32>(scattered, through));
    }
}
