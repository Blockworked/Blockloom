import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tint: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> secondary: vec4<f32>;
// mode, speed, strength, time.
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> params: vec4<f32>;
// use_texture, unused, unused, unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> flags: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var texture_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var<uniform> uv_scale_offset: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> uv_options: vec4<f32>;

fn surface_uv(uv: vec2<f32>) -> vec2<f32> {
    let scaled = uv * uv_scale_offset.xy;
    let angle = uv_options.x;
    return vec2<f32>(scaled.x * cos(angle) - scaled.y * sin(angle),
                     scaled.x * sin(angle) + scaled.y * cos(angle)) + uv_scale_offset.zw;
}

fn surface_sample(mesh: VertexOutput) -> vec4<f32> {
    if (uv_options.y < 0.5) {
        return textureSample(texture, texture_sampler, surface_uv(mesh.uv));
    }
    let p = mesh.world_position.xyz * uv_options.z;
    let weights = pow(abs(normalize(mesh.world_normal)), vec3<f32>(4.0));
    let w = weights / max(dot(weights, vec3<f32>(1.0)), 0.0001);
    return textureSample(texture, texture_sampler, surface_uv(p.yz)) * w.x
         + textureSample(texture, texture_sampler, surface_uv(p.xz)) * w.y
         + textureSample(texture, texture_sampler, surface_uv(p.xy)) * w.z;
}

fn effect_uv(mesh: VertexOutput) -> vec2<f32> {
    if (uv_options.y < 0.5) { return mesh.uv; }
    let p = mesh.world_position.xyz * uv_options.z;
    let n = abs(mesh.world_normal);
    if (n.x >= n.y && n.x >= n.z) { return p.yz; }
    if (n.y >= n.z) { return p.xz; }
    return p.xy;
}

fn apply_effect(uv: vec2<f32>, base: vec4<f32>) -> vec4<f32> {
    let mode = u32(params.x + 0.5);
    let speed = params.y;
    let strength = params.z;
    let t = params.w;
    if (mode == 0u) {
        return base;
    }
    if (mode == 1u) {
        let bands = sin(uv.x * 8.0 + t * speed) * 0.5 + 0.5;
        return mix(base, secondary, bands * strength);
    }
    if (mode == 2u) {
        let v = sin(uv.x * 6.0 + t * speed) * sin(uv.y * 6.0 - t * speed * 1.3);
        let m = clamp(v * 0.5 + 0.5, 0.0, 1.0);
        return mix(base, secondary, m * strength);
    }
    if (mode == 3u) {
        let p = sin(t * speed) * 0.5 + 0.5;
        var color = mix(base, secondary, p * strength);
        color = vec4<f32>(color.rgb * (1.0 + p * strength), color.a);
        return color;
    }
    // Dissolve: hashed noise eats the surface against a pulsing threshold,
    // with a hot rim where it is being eaten.
    let n = fract(sin(dot(uv, vec2<f32>(12.9898, 78.233))) * 43758.55);
    let threshold = (sin(t * speed) * 0.5 + 0.5) * strength;
    if (n < threshold) {
        discard;
    }
    let rim = smoothstep(threshold, threshold + 0.15, n);
    return mix(secondary * 2.0, base, rim);
}

@fragment
fn fragment(mesh: VertexOutput) -> FragmentOutput {
    var output_color = tint;
    if (flags.x > 0.5) {
        output_color = output_color * surface_sample(mesh);
    }
    output_color = apply_effect(effect_uv(mesh), output_color);

    @if(TONEMAP_IN_SHADER)
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
    var out: FragmentOutput;
    out.color = output_color;
    return out;
}
