
// Appended to Bevy's clustered decal module. The tag packs cell and opacity.
@if(CLUSTERED_DECALS_ARE_USABLE)
fn blockloom_decal(
    iterator: ClusteredDecalIterator,
    world_normal: vec3<f32>,
    color: ptr<function, vec4<f32>>,
    normal: ptr<function, vec3<f32>>,
    roughness: ptr<function, f32>,
    emission: ptr<function, vec4<f32>>,
) {
    let id = clustered_forward::get_clusterable_object_id(u32(iterator.decal_index_offset));
    let local_from_world = mesh_view_bindings::clustered_decals.decals[id].local_from_world;
    let local = (local_from_world * vec4<f32>(iterator.world_position, 1.0)).xyz;
    let axes = transpose(local_from_world);
    let n = normalize(axes[2].xyz);
    let angle = smoothstep(0.17, 0.64, dot(normalize(world_normal), n));
    let depth = 1.0 - smoothstep(0.35, 0.5, abs(local.z));
    let fade = f32((iterator.tag >> 8u) & 65535u) / 65535.0;
    let cell = iterator.tag & 3u;
    let uv = (clamp(iterator.uv, vec2<f32>(0.00390625), vec2<f32>(0.99609375))
        + vec2<f32>(f32(cell & 1u), f32(cell >> 1u))) * 0.5;
    if (iterator.base_color_texture_index < 0) { return; }
    let albedo = textureSampleLevel(mesh_view_bindings::clustered_decal_textures[iterator.base_color_texture_index],
        mesh_view_bindings::clustered_decal_sampler, uv, 0.0);
    let alpha = albedo.a * angle * depth * fade;
    (*color) = vec4<f32>(mix((*color).rgb, albedo.rgb, alpha), (*color).a);
    if (iterator.metallic_roughness_texture_index >= 0) {
        let surface = textureSampleLevel(mesh_view_bindings::clustered_decal_textures[iterator.metallic_roughness_texture_index],
            mesh_view_bindings::clustered_decal_sampler, uv, 0.0);
        (*roughness) = mix(*roughness, surface.g, alpha);
    }
    if (iterator.normal_map_texture_index >= 0) {
        let bump = textureSampleLevel(mesh_view_bindings::clustered_decal_textures[iterator.normal_map_texture_index],
            mesh_view_bindings::clustered_decal_sampler, uv, 0.0).xyz * 2.0 - 1.0;
        let t = normalize(axes[0].xyz);
        let b = normalize(axes[1].xyz);
        let perturbed = normalize(*normal + t * bump.x + b * bump.y);
        (*normal) = normalize(mix(*normal, perturbed, alpha));
    }
    if (iterator.emissive_texture_index >= 0) {
        let glow = textureSampleLevel(mesh_view_bindings::clustered_decal_textures[iterator.emissive_texture_index],
            mesh_view_bindings::clustered_decal_sampler, uv, 0.0).rgb;
        (*emission) += vec4<f32>(glow * alpha * 4.0, 0.0);
    }
}
