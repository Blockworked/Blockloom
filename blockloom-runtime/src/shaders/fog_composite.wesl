// Lays the air over the frame, before tonemapping: aerial haze on far
// surfaces, height fog along every ray (the sky's too, so the horizon melts
// into it rather than hard-edging), then the froxel grid's volumetric fog
// nearest the camera. Each pixel's distance comes from the depth prepass.

import blockloom::fog::{
    FogUniforms, FLAG_HEIGHT, FLAG_VOLUMETRIC, FLAG_AERIAL, FLAG_SKY, PI,
    height_fog_depth, distance_slice, haze_weights,
};
import blockloom::sky::{SKY_UNIT, cube_lookup};
import blockloom::scattering::henyey_greenstein;
import blockloom::frame::luminance;

@group(0) @binding(0) var<uniform> fog: FogUniforms;
@group(0) @binding(1) var screen: texture_2d<f32>;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var froxels: texture_3d<f32>;
@group(0) @binding(4) var linear_sampler: sampler;
@group(0) @binding(5) var sky_light: texture_cube<f32>;

// How far the sky is, for fog that has no surface to stop at.
const SKY_DISTANCE: f32 = 1.0e6;

// The sky's light towards the horizon under `dir`, nits: what haze and fog
// glow with when there is a sky.
fn horizon_light(dir: vec3<f32>) -> vec3<f32> {
    let flat = vec3<f32>(dir.x, 0.08, dir.z);
    let level = max(f32(fog.counts.z) - 1.0, 0.0);
    return textureSampleLevel(sky_light, linear_sampler, cube_lookup(normalize(flat)), level * 0.5).rgb * SKY_UNIT;
}

// Isotropic scattering plus a boost towards a light, for height fog.
fn fog_phase(mu: f32, boost: f32, g: f32) -> f32 {
    return 1.0 / (4.0 * PI) + boost * henyey_greenstein(mu, g);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let scene = textureLoad(screen, pixel, 0);
    var color = scene.rgb;
    let z = textureLoad(depth, pixel, 0);
    let sky = z <= 0.0;
    let uv = position.xy * fog.screen_size.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = fog.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let hit = fog.world_from_clip * vec4<f32>(ndc, select(z, 0.5, sky), 1.0);
    let camera = fog.camera.xyz;
    let dir = normalize(hit.xyz / hit.w - near.xyz / near.w);
    let dist = select(length(hit.xyz / hit.w - camera), SKY_DISTANCE, sky);
    let exposure = fog.camera.w;
    let flags = fog.grid.w;
    let has_sky = (flags & FLAG_SKY) != 0u;

    if (flags & FLAG_AERIAL) != 0u && !sky {
        let a = fog.aerial;
        let depth_along = height_fog_depth(a.x, 1.0 / a.y, 0.0, camera.y, dir.y, dist);
        let through = exp(-depth_along * haze_weights(a.w));
        let kept = dot(through, vec3<f32>(1.0 / 3.0));
        var haze = fog.aerial_color.rgb;
        if has_sky {
            haze = horizon_light(dir) * fog.aerial_tint.rgb;
        }
        let gray = mix(color, vec3<f32>(luminance(color)), a.z * (1.0 - kept));
        color = gray * through + haze * exposure * (1.0 - through);
    }

    if (flags & FLAG_HEIGHT) != 0u {
        let h = fog.height;
        let start = min(h.w, dist);
        let origin_y = camera.y + dir.y * start;
        let optical = height_fog_depth(h.x, h.y, h.z, origin_y, dir.y, dist - start);
        let through = exp(-optical);
        var inscatter = fog.height_ambient.rgb
            + fog.height_sun.rgb * fog_phase(dot(dir, fog.sun_dir.xyz), fog.height_sun.w, fog.height_moon.w)
            + fog.height_moon.rgb * fog_phase(dot(dir, fog.moon_dir.xyz), fog.height_sun.w, fog.height_moon.w);
        if has_sky {
            inscatter += horizon_light(dir) * fog.height_albedo.rgb;
        }
        color = color * through + inscatter * exposure * (1.0 - through);
    }

    if (flags & FLAG_VOLUMETRIC) != 0u {
        let depth_slices = f32(fog.grid.z);
        let w = distance_slice(dist, fog.volumetric_emissive.w);
        // Slice k holds everything up to its far edge, at w = (k + 1) / depth.
        var v = textureSampleLevel(froxels, linear_sampler, vec3<f32>(uv, w - 0.5 / depth_slices), 0.0);
        let first = clamp(w * depth_slices, 0.0, 1.0);
        v = mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), v, first);
        color = color * v.a + v.rgb;
    }

    return vec4<f32>(min(color, vec3<f32>(60000.0)), scene.a);
}
