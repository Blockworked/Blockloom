// Realtime path tracing: every pixel sends fresh paths from its G-buffer
// surface each frame, through Solari's traced world. Each vertex samples one
// light (the sun or a glowing triangle) and the BRDF, weighted against each
// other by multiple importance sampling; a path that escapes sees the sky.
// No cache and no reuse, so it is unbiased and noisy; the denoiser follows.

import bevy_core_pipeline::tonemapping::tonemapping_luminance as luminance;
import bevy_pbr::render::utils::rand_f;
import bevy_render::view::View;
import bevy_solari::realtime::gbuffer_utils::gpixel_resolve;
import bevy_solari::scene::brdf::{brdf_pdf, evaluate_and_sample_brdf, evaluate_brdf, F_AB};
import bevy_solari::scene::sampling::{power_heuristic, random_emissive_light_pdf, sample_environment_map_light, sample_random_light};
import bevy_solari::scene::bindings::{MIRROR_ROUGHNESS_THRESHOLD, RAY_T_MAX, RAY_T_MIN, ResolvedMaterial, resolve_ray_hit_full, trace_ray};

enable wgpu_ray_query;

// `PathUniforms`, field for field.
struct PathUniforms {
    paths: u32,
    bounces: u32,
    frame: u32,
    _padding: u32,
}

@group(1) @binding(0) var view_output: texture_storage_2d<rgba16float, write>;
@group(1) @binding(1) var gbuffer: texture_2d<u32>;
@group(1) @binding(2) var depth_buffer: texture_depth_2d;
@group(1) @binding(3) var<uniform> view: View;
@group(1) @binding(4) var<uniform> settings: PathUniforms;

@compute @workgroup_size(8, 8, 1)
fn trace_paths(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if any(global_id.xy >= vec2<u32>(view.main_pass_viewport.zw)) { return; }
    let depth = textureLoad(depth_buffer, global_id.xy, 0);
    if depth == 0.0 { return; }

    let pixel_index = global_id.x + global_id.y * u32(view.main_pass_viewport.z);
    var rng = pixel_index + settings.frame * 5782582u;

    let surface = gpixel_resolve(textureLoad(gbuffer, global_id.xy, 0), depth, global_id.xy, view.main_pass_viewport.zw, view.world_from_clip);
    let wo = normalize(view.world_position - surface.world_position);

    var radiance = vec3(0.0);
    for (var i = 0u; i < settings.paths; i++) {
        let path = trace_path(surface.world_position, surface.world_normal, wo, surface.material, &rng);
        // A degenerate sample is dropped rather than poisoning the frame.
        let exponent = bitcast<vec3<u32>>(path) & vec3(0x7f800000u);
        if all(exponent != vec3(0x7f800000u)) { radiance += path; }
    }
    radiance = radiance / f32(max(settings.paths, 1u)) + surface.material.emissive;
    textureStore(view_output, global_id.xy, vec4(radiance * view.exposure, 1.0));
}

// Light arriving at the camera from one path that starts at a surface seen
// through the pixel, the surface's own glow left out.
fn trace_path(position: vec3<f32>, normal: vec3<f32>, wo: vec3<f32>, material: ResolvedMaterial, rng: ptr<function, u32>) -> vec3<f32> {
    var radiance = vec3(0.0);
    var throughput = vec3(1.0);
    var origin = position + normal * RAY_T_MIN;
    var n = normal;
    var v = wo;
    var m = material;
    for (var bounce = 0u; bounce < settings.bounces; bounce++) {
        let n_dot_v = max(dot(n, v), 0.0001);
        let f_ab = F_AB(m.perceptual_roughness, n_dot_v);

        // One light, weighted against the BRDF finding it.
        let mirror = m.roughness <= MIRROR_ROUGHNESS_THRESHOLD && m.metallic > 0.9999;
        if !mirror {
            let light = sample_random_light(origin, n, rng);
            var weight = 1.0;
            if light.brdf_rays_can_hit {
                let brdf_pdf_at_light = brdf_pdf(v, light.wi, n, m, f_ab);
                weight = power_heuristic(1.0 / light.inverse_solid_angle_pdf, brdf_pdf_at_light);
            }
            let brdf = evaluate_brdf(v, light.wi, n, m, f_ab);
            radiance += weight * throughput * light.radiance * light.inverse_pdf * brdf;
        }

        let next = evaluate_and_sample_brdf(v, n, m, f_ab, rng);
        if next.pdf == 0.0 { break; }
        throughput *= next.throughput;

        let ray = trace_ray(origin, next.wi, RAY_T_MIN, RAY_T_MAX, RAY_FLAG_NONE);
        if ray.kind == RAY_QUERY_INTERSECTION_NONE {
            radiance += throughput * sample_environment_map_light(next.wi);
            break;
        }
        let hit = resolve_ray_hit_full(ray);
        let hit_v = -next.wi;
        let facing = dot(hit.world_normal, hit_v);
        if facing > 0.0 && any(hit.material.emissive > vec3(0.0)) {
            let light_pdf = random_emissive_light_pdf(hit, ray.t, max(facing, 0.0001));
            radiance += power_heuristic(next.pdf, light_pdf) * throughput * hit.material.emissive;
        }

        origin = hit.world_position + hit.geometric_world_normal * RAY_T_MIN;
        n = hit.world_normal;
        v = hit_v;
        m = hit.material;

        // Russian roulette past the first bounce.
        if bounce > 0u {
            let survive = saturate(luminance(throughput));
            if rand_f(rng) >= survive { break; }
            throughput /= survive;
        }
    }
    return radiance;
}
