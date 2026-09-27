import blockloom::texturing::{SurfaceGlobals, lasting_state, apply_lasting};

import bevy_pbr::decal::clustered::apply_decals;
import bevy_pbr::render::{
    mesh_functions::get_tag,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};

// The forward pass lights the surface itself; the deferred one (while ray
// tracing lights the G-buffer) only writes it.
@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

// One record per actor, indexed by its MeshTag. Slot 0 is the identity that
// merged batches use, since their tint and UVs are baked into the vertices.
struct Instance {
    tint: vec4<f32>,
    // UV scale X/Y, offset X/Y.
    uv: vec4<f32>,
    // UV rotation in radians; the rest is reserved.
    options: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage, read> instances: array<Instance>;

@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage,read> globals: SurfaceGlobals;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var lasting_texture: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let data = instances[get_tag(in.instance_index)];
    var local = in;
    @if(VERTEX_UVS_A) {
        let scaled = in.uv * data.uv.xy;
        let c = cos(data.options.x);
        let s = sin(data.options.x);
        local.uv = vec2<f32>(scaled.x * c - scaled.y * s, scaled.x * s + scaled.y * c) + data.uv.zw;
    }
    var pbr = pbr_input_from_standard_material(local, is_front);
    pbr.material.base_color *= data.tint;
    let stain = apply_lasting(pbr.material.base_color.rgb,pbr.material.perceptual_roughness,lasting_state(in.world_position.xyz,globals.surface_frame,lasting_texture));
    pbr.material.base_color = vec4<f32>(stain.albedo,pbr.material.base_color.a);
    pbr.material.perceptual_roughness = stain.roughness;
    @if(!PREPASS_PIPELINE) {
        apply_decals(&pbr);
    }
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    @if(PREPASS_PIPELINE) {
        return deferred_output(in, pbr);
    }
    @if(!PREPASS_PIPELINE) {
        var out: FragmentOutput;
        out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
        return out;
    }
}
