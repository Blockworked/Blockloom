import bevy_pbr::decal::clustered::apply_decals;
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};
import bevy_pbr::render::mesh_view_bindings::view;
import blockloom::texturing::{
    SurfaceGlobals, MaskUniforms, DetailUniforms, sample_uv, triplanar, triplanar_normal,
    triplanar_weights, slope_degrees, macro_noise, macro_tint, blend_detail, apply_masks_with_state, lasting_state,
    wetness_at, blend_debug,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> scale_offset: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<uniform> options: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var albedo_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var albedo_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var normal_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var roughness_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var roughness_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var<uniform> masks: MaskUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(109) var<uniform> detail: DetailUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(110) var macro_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var macro_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(112) var detail_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(113) var detail_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(114) var<storage, read> globals: SurfaceGlobals;

fn rotate(v: vec2<f32>) -> vec2<f32> {
    let c = cos(options.x);
    let s = sin(options.x);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

fn projected_uv(uv: vec2<f32>) -> vec2<f32> {
    return rotate(uv * scale_offset.xy) + scale_offset.zw;
}

// A derivative carried through `projected_uv` (no offset).
fn projected_grad(g: vec2<f32>) -> vec2<f32> {
    return rotate(g * scale_offset.xy);
}

// One map box-projected on the three planes, each lookup optionally hex-tiled.
fn box_sample(
    t: texture_2d<f32>,
    s: sampler,
    pos: vec3<f32>,
    gx: vec3<f32>,
    gy: vec3<f32>,
    w: vec3<f32>,
) -> vec4<f32> {
    let stochastic = detail.tiling.x > 0.5;
    let contrast = detail.tiling.y;
    return sample_uv(t, s, projected_uv(pos.yz), projected_grad(gx.yz), projected_grad(gy.yz), stochastic, contrast) * w.x
        + sample_uv(t, s, projected_uv(pos.xz), projected_grad(gx.xz), projected_grad(gy.xz), stochastic, contrast) * w.y
        + sample_uv(t, s, projected_uv(pos.xy), projected_grad(gx.xy), projected_grad(gy.xy), stochastic, contrast) * w.z;
}

@group(constants::MATERIAL_BIND_GROUP) @binding(116) var lasting_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(117) var wet_texture: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let world = in.world_position.xyz;
    let n = normalize(pbr.world_normal);
    let pos = world * options.y;
    let gx = dpdx(pos);
    let gy = dpdy(pos);
    let wgx = dpdx(world);
    let wgy = dpdy(world);
    let w = triplanar_weights(n, 4.0);
    let projected = options.y > 0.0;
    var uv = vec2<f32>(0.0);
    var ugx = vec2<f32>(0.0);
    var ugy = vec2<f32>(0.0);
    @if(VERTEX_UVS_A) {
        uv = projected_uv(in.uv);
        ugx = dpdx(uv);
        ugy = dpdy(uv);
    }

    if (options.z > 0.5) {
        var color: vec4<f32>;
        if (projected) {
            color = box_sample(albedo_texture, albedo_sampler, pos, gx, gy, w);
        } else {
            color = sample_uv(albedo_texture, albedo_sampler, uv, ugx, ugy, detail.tiling.x > 0.5, detail.tiling.y);
        }
        pbr.material.base_color *= color;
    }
    if (options.w >= 2.0) {
        if (projected) {
            pbr.material.perceptual_roughness *= box_sample(roughness_texture, roughness_sampler, pos, gx, gy, w).g;
        } else {
            pbr.material.perceptual_roughness *= sample_uv(roughness_texture, roughness_sampler, uv, ugx, ugy, detail.tiling.x > 0.5, detail.tiling.y).g;
        }
    }
    if (options.w == 1.0 || options.w == 3.0) {
        let stochastic = detail.tiling.x > 0.5;
        let contrast = detail.tiling.y;
        let nx = sample_uv(normal_texture, normal_sampler, projected_uv(pos.yz), projected_grad(gx.yz), projected_grad(gy.yz), stochastic, contrast).xyz * 2.0 - 1.0;
        let ny = sample_uv(normal_texture, normal_sampler, projected_uv(pos.xz), projected_grad(gx.xz), projected_grad(gy.xz), stochastic, contrast).xyz * 2.0 - 1.0;
        let nz = sample_uv(normal_texture, normal_sampler, projected_uv(pos.xy), projected_grad(gx.xy), projected_grad(gy.xy), stochastic, contrast).xyz * 2.0 - 1.0;
        let sign_n = sign(pbr.world_normal);
        pbr.N = normalize(vec3<f32>(nx.z * sign_n.x, nx.x, nx.y) * w.x
                          + vec3<f32>(ny.x, ny.z * sign_n.y, ny.y) * w.y
                          + vec3<f32>(nz.x, nz.y, nz.z * sign_n.z) * w.z);
    }

    // Macro variation breaks up repeats at a scale the tiles can't.
    if (detail.tiling.z > 0.0) {
        var v: f32;
        if (detail.detail.w > 0.5) {
            v = triplanar(macro_texture, macro_sampler, world, wgx, wgy, w, detail.tiling.w, false, 0.0).r;
        } else {
            v = macro_noise(world.xz + world.y * vec2<f32>(0.37, 0.71), detail.tiling.w);
        }
        pbr.material.base_color = vec4<f32>(pbr.material.base_color.rgb * macro_tint(v, detail.tiling.z), pbr.material.base_color.a);
    }
    // Micro detail normals up close.
    if (detail.detail.x > 0.0) {
        let d = triplanar_normal(detail_texture, detail_sampler, world, wgx, wgy, n, w, detail.detail.y, false, 0.0);
        let distance = length(view.world_position.xyz - world);
        pbr.N = blend_detail(pbr.N, d, n, detail.detail.x, distance, detail.detail.z);
    }

    let masked = apply_masks_with_state(
        pbr.material.base_color.rgb,
        pbr.material.perceptual_roughness,
        slope_degrees(n),
        world.y,
        0.0,
        masks,
        globals,
        lasting_state(world,globals.surface_frame,lasting_texture),
        wetness_at(world,globals.wet_frame,wet_texture),
    );
    pbr.material.base_color = vec4<f32>(masked.albedo, pbr.material.base_color.a);
    pbr.material.perceptual_roughness = masked.roughness;

    var out: FragmentOutput;
    if (globals.weather.z > 0.5) {
        out.color = vec4<f32>(vec3<f32>(0.2) + blend_debug(vec4<f32>(0.0), masked.coverage), 1.0);
        return out;
    }
    apply_decals(&pbr);
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    return out;
}
