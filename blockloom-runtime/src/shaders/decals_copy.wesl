import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var source: texture_2d<u32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<u32> {
    return textureLoad(source, vec2<i32>(in.position.xy), 0);
}
