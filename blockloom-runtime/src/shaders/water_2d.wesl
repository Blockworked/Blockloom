// 2D water: a strip from the surface down to the body's depth. The top edge
// rides the same Gerstner waves the floating bodies do; below it the water
// darkens with depth, glows with drifting caustics and foams along the line.

import bevy_sprite_render::mesh2d::{
    functions as mesh_functions,
    vertex_output::VertexOutput,
    view_bindings::view,
    vertex_input::{Vertex, decompress_vertex},
};
import blockloom::water::{WaterWaves, displace, frame, ripple, caustics, worley_edge};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

struct FlatLook {
    // Linear shallow color, clarity.
    shallow: vec4<f32>,
    // Linear deep color, absorption distance.
    deep: vec4<f32>,
    // Linear foam color, foam amount.
    foam_color: vec4<f32>,
    // Foam line width, crest threshold, foam scale, foam drift.
    foam: vec4<f32>,
    // Caustics strength, caustics scale, 0, 0.
    light: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> waves: WaterWaves;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> look: FlatLook;

// The surface's height over x, at the rest point under it.
fn surface_at(x: f32) -> f32 {
    return waves.flow.z + displace(waves, vec2<f32>(x, 0.0)).y + ripple(waves, vec2<f32>(x, 0.0)).y;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex, vertex.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world = mesh_functions::mesh2d_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    var uv = vec2<f32>(0.0);
    @if(VERTEX_UVS) {
        uv = v.uv;
    }
    // uv.y is 0 along the top edge: that row rides the waves.
    if (uv.y < 0.5) {
        let q = vec2<f32>(world.x, 0.0);
        let moved = displace(waves, q);
        world = vec4<f32>(world.x + moved.x, waves.flow.z + moved.y + ripple(waves, q).y, world.z, 1.0);
    }
    var out: VertexOutput;
    out.world_position = world;
    out.position = mesh_functions::mesh2d_position_world_to_clip(world);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = uv;
    out.instance_index = vertex.instance_index;
    return out;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let t = waves.counts.z;
    let p = mesh.world_position.xy;
    let depth = max(surface_at(p.x) - p.y, 0.0);
    let absorption = max(look.deep.w, 1e-3);
    let fade = 1.0 - exp(-depth / absorption);
    var color = mix(look.shallow.rgb, look.deep.rgb, fade);
    var alpha = mix(1.0 - look.shallow.w, 1.0, fade);

    // Light through the waves, fading with depth.
    if (look.light.x > 0.0) {
        let drift = waves.flow.xy * t;
        let glow = caustics((p - drift) / max(look.light.y, 1e-3), t);
        color += look.shallow.rgb * glow * look.light.x * 0.35 * (1.0 - fade);
    }

    // A foam line along the top, thicker where crests pinch.
    let f = frame(waves, vec2<f32>(p.x, 0.0));
    let crest = clamp((look.foam.y + 0.3 - f.jacobian) / 0.3, 0.0, 1.0);
    let width = look.foam.x * (1.0 + 2.0 * crest);
    let q = vec2<f32>(p.x - waves.flow.x * t - look.foam.w * t, depth) / max(look.foam.z, 1e-3);
    let pattern = worley_edge(q);
    let line = (1.0 - smoothstep(0.0, max(width, 1e-3), depth)) * look.foam_color.w;
    let foam = clamp(line * (0.6 + 0.8 * pattern), 0.0, 1.0);
    color = mix(color, look.foam_color.rgb, foam);
    alpha = max(alpha, foam);

    var out = vec4<f32>(color, alpha);
    @if(TONEMAP_IN_SHADER)
    out = tonemapping::tone_mapping(out, view.color_grading);
    @if(SRGB_OUTPUT)
    out = vec4(linear_to_srgb(out.rgb), out.a);
    @if(OKLAB_OUTPUT)
    out = vec4(linear_rgb_to_oklab(out.rgb), out.a);
    return out;
}
