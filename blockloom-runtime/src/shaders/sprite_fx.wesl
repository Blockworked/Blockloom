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
// (1 flip x, 2 flip y, 4 palette, 8 silhouette: outline color wherever the
// grown sprite covers, drawn behind a rig's or stack's pieces).
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> shape: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> outline_color: vec4<f32>;
// Palette row, then unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> palette_params: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var sprite_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var sprite_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var palette_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var palette_sampler: sampler;
// 9-slice, as Bevy's sprite shader has it: scale (x 0 is off), then the
// min and max insets over the frame, then side and center tile values.
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var<uniform> slice_scale: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var<uniform> slice_insets: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var<uniform> slice_tiles: vec4<f32>;

fn has_flag(flag: u32) -> bool {
    return (u32(shape.w + 0.5) & flag) != 0u;
}

// Maps p from [a, b] onto [c, d], tiling every `tile` of it when nonzero.
fn tile_or_stretch(p: f32, a: f32, b: f32, c: f32, d: f32, tile: f32) -> f32 {
    var value = (p - a) / (b - a);
    if (tile != 0.0) {
        value = (value - tile * floor(value / tile)) / tile;
    }
    return value * (d - c) + c;
}

// One axis of the slice: a corner keeps its scale, the middle stretches or
// tiles.
fn slice_axis(p: f32, scale: f32, min_inset: f32, max_inset: f32, tile: f32) -> f32 {
    let min_scaled = min_inset / scale;
    let max_scaled = max_inset / scale;
    if (p < min_scaled) {
        return p * scale;
    }
    if (p > 1.0 - max_scaled) {
        return 1.0 - (1.0 - p) * scale;
    }
    return tile_or_stretch(p, min_scaled, 1.0 - max_scaled, min_inset, 1.0 - max_inset, tile);
}

// Where `p` (0-1 over the drawn sprite) lands on the frame once sliced.
fn slice(p: vec2<f32>) -> vec2<f32> {
    let scale = slice_scale.xy;
    if (scale.x == 0.0) {
        return p;
    }
    let min_inset = slice_insets.xy;
    let max_inset = slice_insets.zw;
    let edge = (p < min_inset / scale) | (p > vec2<f32>(1.0) - max_inset / scale);
    // A middle beside a border tiles as a side, else as the center.
    let tile = select(slice_tiles.zw, slice_tiles.xy, edge.yx);
    return vec2<f32>(
        slice_axis(p.x, scale.x, min_inset.x, max_inset.x, tile.x),
        slice_axis(p.y, scale.y, min_inset.y, max_inset.y, tile.y),
    );
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
    let uv = mix(uv_rect.xy, uv_rect.zw, slice(p));
    return textureSampleLevel(sprite_texture, sprite_sampler, uv, 0.0);
}

// The most any point within `width` of `local` covers.
fn reach(local: vec2<f32>, size: vec2<f32>, width: f32) -> f32 {
    var most = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        let angle = f32(i) * 0.7853982;
        let offset = vec2<f32>(cos(angle), sin(angle)) * width / size;
        most = max(most, sprite_at(local + offset).a);
    }
    return most;
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
    let outline = vec4<f32>(outline_color.rgb, outline_color.a * tint.a);
    var output_color = vec4<f32>(0.0);
    if (has_flag(8u)) {
        var covered = color.a;
        if (width > 0.0 && covered < 0.5) {
            covered = reach(local, size, width);
        }
        if (covered >= 0.5) {
            output_color = outline;
        }
    } else {
        if (has_flag(4u) && color.a > 0.0) {
            color = swap_palette(color);
        }
        output_color = color * tint;
        if (width > 0.0 && color.a < 0.5 && reach(local, size, width) >= 0.5) {
            output_color = outline;
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
