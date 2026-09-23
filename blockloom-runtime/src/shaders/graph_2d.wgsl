#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}

#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
#ifdef SRGB_OUTPUT
#import bevy_render::color_operations::linear_to_srgb
#endif
#ifdef OKLAB_OUTPUT
#import bevy_render::color_operations::linear_rgb_to_oklab
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> secondary: vec4<f32>;
// mode, speed, strength, time.
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> params: vec4<f32>;
// use_texture, rounded, 0, 0.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> flags: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var texture_sampler: sampler;

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
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var output_color = tint;
    if (flags.x > 0.5) {
        output_color = output_color * textureSample(texture, texture_sampler, mesh.uv);
    }
    // A circle look renders its quad round, the way the sprite disc does.
    if (flags.y > 0.5 && length((mesh.uv - 0.5) * 2.0) > 1.0) {
        discard;
    }
    output_color = apply_effect(mesh.uv, output_color);

#ifdef TONEMAP_IN_SHADER
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
#endif
#ifdef SRGB_OUTPUT
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
#endif
#ifdef OKLAB_OUTPUT
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
#endif
    return output_color;
}
