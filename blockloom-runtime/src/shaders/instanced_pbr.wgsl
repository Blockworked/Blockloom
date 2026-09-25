#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_tag,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

// One record per actor, indexed by its MeshTag. Slot 0 is the identity that
// merged batches use, since their tint and UVs are baked into the vertices.
struct Instance {
    tint: vec4<f32>,
    // UV scale X/Y, offset X/Y.
    uv: vec4<f32>,
    // UV rotation in radians; the rest is reserved.
    options: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> instances: array<Instance>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let data = instances[get_tag(in.instance_index)];
    var local = in;
#ifdef VERTEX_UVS_A
    let scaled = in.uv * data.uv.xy;
    let c = cos(data.options.x);
    let s = sin(data.options.x);
    local.uv = vec2<f32>(scaled.x * c - scaled.y * s, scaled.x * s + scaled.y * c) + data.uv.zw;
#endif
    var pbr = pbr_input_from_standard_material(local, is_front);
    pbr.material.base_color *= data.tint;
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    return out;
}
