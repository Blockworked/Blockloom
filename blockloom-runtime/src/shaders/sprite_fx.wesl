import bevy_sprite_render::mesh2d::{
    vertex_output::VertexOutput,
    view_bindings::view,
};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

// The sprite's tint.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tint: vec4<f32>;
// The frame's rectangle on its texture, in UV: min xy, max zw.
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> uv_rect: vec4<f32>;
// Sprite width and height in world units, outline width, flags
// (1 flip x, 2 flip y, 4 palette).
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> shape: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> outline_color: vec4<f32>;
// Palette row, then unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> palette_params: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var sprite_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var sprite_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var palette_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var palette_sampler: sampler;

fn has_flag(flag: u32) -> bool {
    return (u32(shape.w + 0.5) & flag) != 0u;
}

// Samples the sprite at `local` (0-1 over the sprite, y down); outside it
// is transparent.
fn sprite_at(local: vec2<f32>) -> vec4<f32> {
    if (any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    var p = local;
    if (has_flag(1u)) {
        p.x = 1.0 - p.x;
    }
    if (has_flag(2u)) {
        p.y = 1.0 - p.y;
    }
    let uv = mix(uv_rect.xy, uv_rect.zw, p);
    return textureSampleLevel(sprite_texture, sprite_sampler, uv, 0.0);
}

// The sprite's red channel picks a column of the palette, the row is the
// palette index; alpha stays the sprite's.
fn swap_palette(color: vec4<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(palette_texture));
    let column = round(color.r * (size.x - 1.0));
    let row = clamp(palette_params.x, 0.0, size.y - 1.0);
    let uv = (vec2<f32>(column, row) + 0.5) / size;
    let swapped = textureSampleLevel(palette_texture, palette_sampler, uv, 0.0);
    return vec4<f32>(swapped.rgb, color.a);
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let size = max(shape.xy, vec2<f32>(1.0));
    let width = shape.z;
    // The quad is grown by the outline on every side.
    let point = mesh.uv * (size + 2.0 * width) - width;
    let local = point / size;
    var color = sprite_at(local);
    if (has_flag(4u) && color.a > 0.0) {
        color = swap_palette(color);
    }
    var output_color = color * tint;
    if (width > 0.0 && color.a < 0.5) {
        var reach = 0.0;
        for (var i = 0u; i < 8u; i = i + 1u) {
            let angle = f32(i) * 0.7853982;
            let offset = vec2<f32>(cos(angle), sin(angle)) * width / size;
            reach = max(reach, sprite_at(local + offset).a);
        }
        if (reach >= 0.5) {
            output_color = vec4<f32>(outline_color.rgb, outline_color.a * tint.a);
        }
    }
    if (output_color.a <= 0.001) {
        discard;
    }

    @if(TONEMAP_IN_SHADER)
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
    @if(SRGB_OUTPUT)
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
    @if(OKLAB_OUTPUT)
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
    return output_color;
}
