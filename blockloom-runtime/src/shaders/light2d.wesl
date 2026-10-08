// 2D lighting: one full-screen layer over the lit part of the world that
// multiplies what is under it by ambient plus every light. Each pixel is put
// back in the world through the view, then measured against the lights the
// same way `Light2dSense::color_at` measures a point on the CPU - change the
// two together.

import bevy_sprite_render::mesh2d::{
    functions as mesh_functions,
    vertex_output::VertexOutput,
    view_bindings::view,
    vertex_input::{Vertex, decompress_vertex},
};

const TAU: f32 = 6.28318530718;
const SHADOW_SAMPLES: u32 = 256u;
// Vec4s per light's shadow map.
const SHADOW_VEC4S: u32 = 64u;
const MAX_LIGHTS: u32 = 32u;
const MAX_SHADOWS: u32 = 16u;

struct Light {
    // World position, range, falloff exponent.
    pos_range: vec4<f32>,
    // Linear color times intensity, softness of the shadow edge.
    color_soft: vec4<f32>,
    // Facing, then the cosines of the cone's inner and outer edges. An outer
    // cosine below -1 is a point light.
    dir_cone: vec4<f32>,
    // x: the light's slot in `shadows`, or -1 for none.
    flags: vec4<f32>,
}

struct LightMap {
    // Linear ambient, then how many lights are live.
    ambient: vec4<f32>,
    lights: array<Light, 32>,
    shadows: array<vec4<f32>, 1024>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> map: LightMap;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex, vertex.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    // The mesh is a 2 by 2 quad: its corners are the screen's corners. Take
    // each back to the world on the plane, then give it the layer's depth so
    // it sorts with the sprites under it.
    let on_plane = view.world_from_clip * vec4<f32>(v.position.xy, 0.0, 1.0);
    let at = on_plane.xy / on_plane.w;
    let world = vec4<f32>(at, world_from_local[3].z, 1.0);
    var out: VertexOutput;
    out.world_position = world;
    out.position = mesh_functions::mesh2d_position_world_to_clip(world);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.instance_index = vertex.instance_index;
    return out;
}

fn shadow_at(slot: u32, index: u32) -> f32 {
    return map.shadows[slot * SHADOW_VEC4S + index / 4u][index % 4u];
}

fn light_at(l: Light, p: vec2<f32>) -> vec3<f32> {
    let range = l.pos_range.z;
    let d = p - l.pos_range.xy;
    let dist = length(d);
    if (dist >= range) {
        return vec3<f32>(0.0);
    }
    let r = dist / max(range, 0.001);
    var amount = pow(clamp(1.0 - r * r, 0.0, 1.0), l.pos_range.w);
    if (l.dir_cone.w > -1.0 && dist > 0.0001) {
        let cos_angle = dot(d, l.dir_cone.xy) / dist;
        amount *= smoothstep(l.dir_cone.w, l.dir_cone.z, cos_angle);
    }
    if (l.flags.x >= 0.0 && dist > 0.0001) {
        let slot = u32(l.flags.x + 0.5);
        let u = fract(atan2(d.y, d.x) / TAU) * f32(SHADOW_SAMPLES);
        let i0 = u32(floor(u)) % SHADOW_SAMPLES;
        let i1 = (i0 + 1u) % SHADOW_SAMPLES;
        // The nearer of the two, so a corner never leaks light.
        let blocked_at = min(shadow_at(slot, i0), shadow_at(slot, i1));
        let soft = max(l.color_soft.w, 0.001);
        amount *= clamp((blocked_at + soft - dist) / soft, 0.0, 1.0);
    }
    return l.color_soft.rgb * amount;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let p = mesh.world_position.xy;
    var light = map.ambient.rgb;
    let count = min(u32(map.ambient.w + 0.5), MAX_LIGHTS);
    for (var i = 0u; i < count; i++) {
        light += light_at(map.lights[i], p);
    }
    return vec4<f32>(light, 1.0);
}
