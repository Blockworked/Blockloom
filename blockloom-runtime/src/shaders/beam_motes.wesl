// Dust motes: one camera-facing quad per mote, placed on the GPU from a
// seed so they drift and twinkle for free. In a beam they are lit by its
// lamp through the beam's cone and fades; as height dust they follow the
// camera in a wrapped box, lit by the sun and the ambient light.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
};
import blockloom::fog::{
    PI, beam_fade, cone_window, mote_in_beam, mote_in_box, mote_twinkle,
};
import blockloom::scattering::henyey_greenstein;

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;

// rgb radiance (a lamp's at one metre, or the sun's), w alpha.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> color: vec4<f32>;
// x size, y twinkle, z drift, w mode: 0 spot cone, 1 point ball, 2 dust box.
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> shape: vec4<f32>;
// x range (box size for dust), y tan of the outer angle, z near, w far fade.
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> beam: vec4<f32>;
// x cos outer (-2 for a point), y cos inner, z falloff curve, w g.
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> cone: vec4<f32>;
// Dust only: xyz towards the sun, w top height.
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var<uniform> sun: vec4<f32>;
// Dust only: rgb ambient radiance, w base height.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<uniform> ambient: vec4<f32>;

// HG normalized so isotropic is 1.
fn phase(cos_theta: f32) -> f32 {
    return henyey_greenstein(cos_theta, cone.w) * 4.0 * PI;
}

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    // The seed rides in the color; the position is only there for bounds.
    let seed = v.color.rgb;
    let t = globals.time;
    let mode = u32(shape.w + 0.5);
    let eye = view.world_position;
    var centre: vec3<f32>;
    var light: vec3<f32>;
    if mode == 2u {
        centre = mote_in_box(seed, t, eye, beam.x, shape.z);
        let view_dir = normalize(eye - centre);
        let top = sun.w;
        let height = max(top - ambient.w, 0.01);
        let thin = 1.0 - smoothstep(top - height * 0.4, top, centre.y);
        let edge = 1.0 - smoothstep(0.3 * beam.x, 0.5 * beam.x, length(centre - eye));
        light = (color.rgb * phase(dot(-view_dir, sun.xyz)) + ambient.rgb) * thin * edge;
    } else {
        let world_from_local = get_world_from_local(vertex_in.instance_index);
        let tan_outer = select(beam.y, -1.0, mode == 1u);
        let local = mote_in_beam(seed, t, beam.x, tan_outer, beam.z, shape.z);
        centre = (world_from_local * vec4<f32>(local, 1.0)).xyz;
        let lamp = world_from_local[3].xyz;
        let d = max(length(local), 0.05);
        let from_lamp = local / d;
        let window = cone_window(-from_lamp.z, cone.x, cone.y);
        let fade = beam_fade(d, beam.x, beam.z, beam.w, cone.z);
        let towards = normalize(lamp - centre);
        let view_dir = normalize(eye - centre);
        light = color.rgb * window * fade / (d * d) * phase(dot(-view_dir, towards));
    }
    light *= mote_twinkle(seed, t, shape.y) * color.a;

    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let corner = v.uv * 2.0 - 1.0;
    let world = centre + (right * corner.x + up * corner.y) * shape.x * 0.5;
    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(eye - centre);
    out.uv = v.uv;
    out.color = vec4<f32>(light * view.exposure, 1.0);
    return out;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let r = length(mesh.uv * 2.0 - 1.0);
    let disk = 1.0 - smoothstep(0.4, 1.0, r);
    var out = vec4<f32>(mesh.color.rgb * disk, 0.0);
    @if(TONEMAP_IN_SHADER)
    out = tonemapping::tone_mapping(out, view.color_grading);
    return out;
}
