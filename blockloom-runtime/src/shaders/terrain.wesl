// A terrain's ground: up to four layers blended by baked weights, each
// triplanar-projected from a texture array, then the same macro, detail and
// mask passes as any projected surface.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};
import bevy_pbr::mesh_view_bindings::view;
import blockloom::texturing::{
    SurfaceGlobals, MaskUniforms, DetailUniforms, triplanar, triplanar_array,
    triplanar_normal, triplanar_normal_array, triplanar_weights, slope_degrees, macro_noise,
    macro_tint, blend_detail, apply_masks, blend_debug,
};

struct TerrainUniforms {
    // Per layer: linear rgb tint, roughness.
    colors: array<vec4<f32>, 4>,
    // Per layer texture repeats per metre.
    density: vec4<f32>,
    // Layer count, then albedo, normal and roughness map bits.
    maps: vec4<f32>,
    // The terrain frame's rotation, for its baked normals.
    rotation: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> terrain: TerrainUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var weight_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var weight_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var surface_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var surface_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var albedo_texture: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var layer_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var normal_texture: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var roughness_texture: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(109) var<uniform> masks: MaskUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(110) var<uniform> detail: DetailUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var macro_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(112) var macro_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(113) var detail_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(114) var detail_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(115) var<storage, read> globals: SurfaceGlobals;

fn rotate_by(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Terrain UV (0-1 corner to corner of the sample grid) to the texel centres
// of a per-sample map.
fn grid_uv(uv: vec2<f32>, side: vec2<f32>) -> vec2<f32> {
    return (uv * (side - 1.0) + 0.5) / side;
}

fn has(bits: f32, layer: i32) -> bool {
    return ((u32(bits + 0.5) >> u32(layer)) & 1u) == 1u;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let world = in.world_position.xyz;
    let gx = dpdx(world);
    let gy = dpdy(world);
    var uv = vec2<f32>(0.0);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    let raw = textureSample(weight_texture, weight_sampler, grid_uv(uv, vec2<f32>(textureDimensions(weight_texture))));
    let weights = raw / max(dot(raw, vec4<f32>(1.0)), 1e-4);
    let baked = textureSample(surface_texture, surface_sampler, grid_uv(uv, vec2<f32>(textureDimensions(surface_texture))));
    let n = normalize(rotate_by(terrain.rotation, baked.xyz * 2.0 - 1.0));
    let cavity = baked.w;
    let w = triplanar_weights(n, 4.0);
    let stochastic = detail.tiling.x > 0.5;
    let contrast = detail.tiling.y;
    let count = i32(terrain.maps.x + 0.5);

    var albedo = vec3<f32>(0.0);
    var roughness = 0.0;
    var normal_sum = vec3<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let a = weights[k];
        if (k >= count || a < 0.004) {
            continue;
        }
        let scale = terrain.density[k];
        var color = terrain.colors[k].rgb;
        var r = terrain.colors[k].w;
        if (has(terrain.maps.y, k)) {
            color *= triplanar_array(albedo_texture, layer_sampler, k, world, gx, gy, w, scale, stochastic, contrast).rgb;
        }
        if (has(terrain.maps.w, k)) {
            r *= triplanar_array(roughness_texture, layer_sampler, k, world, gx, gy, w, scale, stochastic, contrast).g;
        }
        var layer_n = n;
        if (has(terrain.maps.z, k)) {
            layer_n = triplanar_normal_array(normal_texture, layer_sampler, k, world, gx, gy, n, w, scale, stochastic, contrast);
        }
        albedo += color * a;
        roughness += r * a;
        normal_sum += layer_n * a;
    }
    var shading_n = n;
    if (dot(normal_sum, normal_sum) > 1e-6) {
        shading_n = normalize(normal_sum);
    }

    if (detail.tiling.z > 0.0) {
        var v: f32;
        if (detail.detail.w > 0.5) {
            v = triplanar(macro_texture, macro_sampler, world, gx, gy, w, detail.tiling.w, false, 0.0).r;
        } else {
            v = macro_noise(world.xz, detail.tiling.w);
        }
        albedo *= macro_tint(v, detail.tiling.z);
    }
    if (detail.detail.x > 0.0) {
        let d = triplanar_normal(detail_texture, detail_sampler, world, gx, gy, n, w, detail.detail.y, false, 0.0);
        let distance = length(view.world_position.xyz - world);
        shading_n = blend_detail(shading_n, d, n, detail.detail.x, distance, detail.detail.z);
    }

    let masked = apply_masks(albedo, roughness, slope_degrees(n), world.y, cavity, masks, globals);
    pbr.material.base_color = vec4<f32>(masked.albedo, 1.0);
    pbr.material.perceptual_roughness = clamp(masked.roughness, 0.045, 1.0);
    pbr.world_normal = n;
    pbr.N = shading_n;

    var out: FragmentOutput;
    if (globals.weather.z > 0.5) {
        out.color = vec4<f32>(blend_debug(weights, masked.coverage), 1.0);
        return out;
    }
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    return out;
}
