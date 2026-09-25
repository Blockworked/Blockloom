import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> scale_offset: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<uniform> options: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var albedo_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var albedo_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var normal_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var roughness_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var roughness_sampler: sampler;

fn projected_uv(uv: vec2<f32>) -> vec2<f32> {
    let scaled = uv * scale_offset.xy;
    let c = cos(options.x);
    let s = sin(options.x);
    return vec2<f32>(scaled.x * c - scaled.y * s,
                     scaled.x * s + scaled.y * c) + scale_offset.zw;
}

fn weights(normal: vec3<f32>) -> vec3<f32> {
    let w = pow(abs(normal), vec3<f32>(4.0));
    return w / max(dot(w, vec3<f32>(1.0)), 0.0001);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let pos = in.world_position.xyz * options.y;
    let w = weights(normalize(pbr.world_normal));
    let uv_x = projected_uv(pos.yz);
    let uv_y = projected_uv(pos.xz);
    let uv_z = projected_uv(pos.xy);

    if (options.z > 0.5) {
        let color = textureSample(albedo_texture, albedo_sampler, uv_x) * w.x
                  + textureSample(albedo_texture, albedo_sampler, uv_y) * w.y
                  + textureSample(albedo_texture, albedo_sampler, uv_z) * w.z;
        pbr.material.base_color *= color;
    }
    if (options.w >= 2.0) {
        if (options.y > 0.0) {
            let rough = textureSample(roughness_texture, roughness_sampler, uv_x).g * w.x
                      + textureSample(roughness_texture, roughness_sampler, uv_y).g * w.y
                      + textureSample(roughness_texture, roughness_sampler, uv_z).g * w.z;
            pbr.material.perceptual_roughness *= rough;
        } else {
            @if(VERTEX_UVS_A)
            pbr.material.perceptual_roughness *= textureSample(roughness_texture, roughness_sampler, projected_uv(in.uv)).g;
        }
    }
    if (options.w == 1.0 || options.w == 3.0) {
        let nx = textureSample(normal_texture, normal_sampler, uv_x).xyz * 2.0 - 1.0;
        let ny = textureSample(normal_texture, normal_sampler, uv_y).xyz * 2.0 - 1.0;
        let nz = textureSample(normal_texture, normal_sampler, uv_z).xyz * 2.0 - 1.0;
        let sign_n = sign(pbr.world_normal);
        pbr.N = normalize(vec3<f32>(nx.z * sign_n.x, nx.x, nx.y) * w.x
                          + vec3<f32>(ny.x, ny.z * sign_n.y, ny.y) * w.y
                          + vec3<f32>(nz.x, nz.y, nz.z * sign_n.z) * w.z);
    }
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    return out;
}
