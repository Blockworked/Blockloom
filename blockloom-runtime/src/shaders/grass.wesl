// Grass blades: the mesh carries each vertex's height up its blade and the
// blade's height (uv) and its thinning rank (color alpha). Blades bend with
// the wind and flutter in gusts, then shrink and thin out with distance,
// collapsing to nothing past the cull distance.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions::{get_world_from_local, mesh_normal_local_to_world},
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
};
import blockloom::texturing::SurfaceGlobals;
import blockloom::noise::value2;

// Fade start, cull distance, stiffness, wind multiplier.
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> grass: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> surface: SurfaceGlobals;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = get_world_from_local(vertex_in.instance_index);
    var world = (world_from_local * vec4<f32>(v.position, 1.0)).xyz;
    var t = 0.0;
    var blade = 0.0;
    @if(VERTEX_UVS_A) {
        t = v.uv.x;
        blade = v.uv.y;
    }
    var rank = 0.0;
    @if(VERTEX_COLORS) {
        rank = v.color.a;
    }
    let root = world - vec3<f32>(0.0, t * blade, 0.0);

    // Thin by rank and shrink towards the cull distance.
    let dist = distance(view.world_position.xyz, root);
    let fade = clamp((dist - grass.x) / max(grass.y - grass.x, 0.01), 0.0, 1.0);
    let alive = select(0.0, 1.0, rank < 1.0 - fade && dist < grass.y);
    world = root + (world - root) * alive * mix(1.0, 0.4, fade);

    // Lean with the wind, harder in gusts that roll across with it, plus a
    // quick flutter.
    let wind = surface.wind.xy * grass.w;
    let speed = length(wind);
    let heading = select(vec2<f32>(0.0), wind / max(speed, 1e-4), speed > 1e-4);
    let gust = value2(root.xz * 0.06 - wind * globals.time * 0.15);
    let give = 1.0 - clamp(grass.z, 0.0, 1.0);
    let push = clamp(speed * 0.06 * (0.5 + gust), 0.0, 1.2) * give;
    let rate = 4.0 + 3.0 * rank + max(surface.wind.w, 0.0);
    let flutter = sin(globals.time * rate + root.x * 1.3 + root.z * 0.7) * (0.03 + 0.1 * push) * give;
    let bend = t * t * blade * alive;
    world += vec3<f32>(heading.x, 0.0, heading.y) * bend * (push + flutter);
    world.y -= bend * push * push * 0.35;

    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    @if(VERTEX_NORMALS) {
        out.world_normal = mesh_normal_local_to_world(v.normal, vertex_in.instance_index);
    }
    @if(VERTEX_UVS_A) {
        out.uv = v.uv;
    }
    @if(VERTEX_COLORS) {
        out.color = vec4<f32>(v.color.rgb, 1.0);
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex_in.instance_index;
    }
    return out;
}
