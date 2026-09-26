// Under the water: every pixel's light is absorbed and replaced by the
// water's own lit color over the part of its ray that is under the surface,
// and whatever it lands on under the surface is dappled with caustics.

import blockloom::water::caustics;

struct Underwater {
    world_from_clip: mat4x4<f32>,
    // Camera position, surface level.
    camera: vec4<f32>,
    // Lit water color (exposed), fog distance.
    fog: vec4<f32>,
    // Per-channel extinction, caustics strength.
    tint: vec4<f32>,
    // Caustics scale, water time, absorption distance, 0.
    caustics: vec4<f32>,
    // Viewport size.
    size: vec4<f32>,
}

@group(0) @binding(0) var<uniform> water: Underwater;
@group(0) @binding(1) var screen: texture_2d<f32>;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_2d;

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let scene = textureLoad(screen, pixel, 0);
    // Reverse-Z: 0 is the far plane.
    let z = textureLoad(depth, pixel, 0);
    let uv = position.xy / water.size.xy;
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, max(z, 1e-7), 1.0);
    let world = water.world_from_clip * clip;
    let point = world.xyz / world.w;
    let eye = water.camera.xyz;
    let level = water.camera.w;
    let ray = point - eye;
    var length_under = length(ray);
    let dir = ray / max(length_under, 1e-5);
    if (z <= 0.0) {
        length_under = water.fog.w * 20.0;
    }
    // The ray leaves the water where it crosses the surface.
    if (dir.y > 1e-5) {
        length_under = min(length_under, (level - eye.y) / dir.y);
    }
    var color = scene.rgb;
    if (z > 0.0 && point.y < level && water.tint.w > 0.0) {
        let column = level - point.y;
        let light = exp(-column / max(water.caustics.z, 1e-3)) * water.tint.w;
        color *= 1.0 + caustics(point.xz / max(water.caustics.x, 1e-3), water.caustics.y) * light;
    }
    let transmit = exp(-water.tint.xyz * max(length_under, 0.0) / max(water.fog.w, 1e-3));
    return vec4<f32>(color * transmit + water.fog.rgb * (vec3<f32>(1.0) - transmit), scene.a);
}
