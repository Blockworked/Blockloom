// A standard surface whose base color also reads one layer of a texture
// array: uv_a tiles, uv_b.x names the layer.
import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};
@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::{io::{VertexOutput, FragmentOutput}, bindings as prepass_bindings},
    deferred::functions::deferred_output,
    render::mesh_view_bindings::view,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var layers_sampler: sampler;

fn texel(in: VertexOutput) -> vec4<f32> {
    var color = vec4<f32>(1.0);
    @if(VERTEX_UVS_A && VERTEX_UVS_B) {
        color = textureSample(layers, layers_sampler, in.uv, i32(round(in.uv_b.x)));
    }
    return color;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // The depth and normal prepass only needs the cutout's holes.
    @if(PREPASS_PIPELINE && !DEFERRED_PREPASS) {
        var alpha = texel(in).a;
        @if(VERTEX_COLORS) {
            alpha *= in.color.a;
        }
        @if(MAY_DISCARD) {
            if alpha < 0.5 {
                discard;
            }
        }
        var out: FragmentOutput;
        @if(NORMAL_PREPASS) {
            out.normal = vec4(in.world_normal * 0.5 + vec3(0.5), 1.0);
        }
        @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
            out.frag_depth = in.unclipped_depth;
        }
        @if(MOTION_VECTOR_PREPASS) {
            let now = view.unjittered_clip_from_world * in.world_position;
            let then = prepass_bindings::previous_view_uniforms.unjittered_clip_from_world
                * in.previous_world_position;
            out.motion_vector = (now.xy / now.w - then.xy / then.w) * vec2(0.5, -0.5);
        }
        return out;
    }
    @if(!PREPASS_PIPELINE || DEFERRED_PREPASS) {
        var pbr = pbr_input_from_standard_material(in, is_front);
        pbr.material.base_color *= texel(in);
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
}
