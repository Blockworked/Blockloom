import bevy_pbr::render::{
    mesh_functions::{get_world_from_local, get_previous_world_from_local, mesh_normal_local_to_world, mesh_position_local_to_world, get_visibility_range_dither_level},
    view_transformations::position_world_to_clip,
};
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::VertexOutput;
@else
import bevy_pbr::render::forward_io::VertexOutput;

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage, read> records: array<vec2<u32>>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> palette: array<vec4<f32>>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var<uniform> scale: vec4<f32>;

struct QuadVertex {
    @builtin(instance_index) instance: u32,
    @location(0) index: u32,
}
@vertex
fn vertex(input: QuadVertex) -> VertexOutput {
    let data = records[input.index / 4u];
    let r = data.x;
    let face = (r >> 24u) & 7u;
    let axis = face / 2u;
    let u = (axis + 1u) % 3u;
    let v = (axis + 2u) % 3u;
    let corner = input.index % 4u;
    var p = vec3<f32>(0.0);
    p[axis] = f32(r & 255u);
    p[u] = f32((r >> 8u) & 255u) + select(0.0, f32(data.y & 255u), corner == 1u || corner == 2u);
    p[v] = f32((r >> 16u) & 255u) + select(0.0, f32((data.y >> 8u) & 255u), corner >= 2u);
    p *= scale.x;
    var normal = vec3<f32>(0.0);
    normal[axis] = select(-1.0, 1.0, face % 2u == 1u);
    let model = get_world_from_local(input.instance);
    var out: VertexOutput;
    out.world_position = mesh_position_local_to_world(model, vec4<f32>(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        out.world_normal = mesh_normal_local_to_world(normal, input.instance);
    }
    @if(VERTEX_COLORS) { out.color = vec4<f32>(palette[(data.y >> 16u) & 255u].rgb, 1.0); }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) { out.instance_index = input.instance; }
    @if(PREPASS_PIPELINE && MOTION_VECTOR_PREPASS) {
        out.previous_world_position = mesh_position_local_to_world(get_previous_world_from_local(input.instance), vec4<f32>(p, 1.0));
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(VISIBILITY_RANGE_DITHER) {
        out.visibility_range_dither = get_visibility_range_dither_level(input.instance, model[3]);
    }
    return out;
}
