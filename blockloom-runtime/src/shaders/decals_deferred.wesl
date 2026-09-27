import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_pbr::{
    render::mesh_view_bindings::deferred_prepass_texture,
    prepass::utils::prepass_depth,
    deferred::functions::{pbr_input_from_deferred_gbuffer, deferred_gbuffer_from_pbr_input},
    decal::clustered::apply_decals,
};

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<u32> {
    let data = textureLoad(deferred_prepass_texture, vec2<i32>(in.position.xy), 0);
    let depth = prepass_depth(in.position, 0u);
    if (depth == 0.0) { return data; }
    var pbr = pbr_input_from_deferred_gbuffer(vec4(in.position.xy, depth, 1.0), data);
    let original = pbr;
    apply_decals(&pbr);
    if (all(pbr.material.base_color == original.material.base_color)
        && all(pbr.material.emissive == original.material.emissive)
        && all(pbr.N == original.N)
        && pbr.material.perceptual_roughness == original.material.perceptual_roughness) {
        return data;
    }
    return deferred_gbuffer_from_pbr_input(pbr);
}
