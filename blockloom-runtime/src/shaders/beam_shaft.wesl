// A spot's beam as cheap geometry: an open cone drawn additively, bright
// where it is seen side-on and fading at its silhouette, carved by noise
// scrolling down the beam. Stands in for the froxels when they are off.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
};
import blockloom::fog::{PI, beam_fade};
import blockloom::noise::value3;

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;

// rgb light per metre of distance from the lamp, w fresnel power.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> color: vec4<f32>;
// x range, y near fade, z far fade, w falloff curve.
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> beam: vec4<f32>;
// x noise amount, y scroll m/s, z 1 / noise scale.
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> noise: vec4<f32>;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    // The mesh's v runs from the lamp (0) to the range (1).
    let along = mesh.uv.y * beam.x;
    let to_eye = view.world_position - mesh.world_position.xyz;
    let edge = pow(abs(dot(normalize(mesh.world_normal), normalize(to_eye))), color.w);
    let angle = mesh.uv.x * 2.0 * PI;
    let q = vec3<f32>(cos(angle) * 1.5, sin(angle) * 1.5, (along - globals.time * noise.y) * noise.z);
    let carve = max(mix(1.0, value3(q) * 2.0, noise.x), 0.0);
    let fade = beam_fade(along, beam.x, beam.y, beam.z, beam.w);
    // Thin out rather than fill the screen when the camera stands in it.
    let near_eye = smoothstep(0.2, 1.5, length(to_eye));
    var light = color.rgb * edge * carve * fade * near_eye / max(along, 0.25) * view.exposure;
    var out = vec4<f32>(light, 0.0);
    @if(TONEMAP_IN_SHADER)
    out = tonemapping::tone_mapping(out, view.color_grading);
    return out;
}
