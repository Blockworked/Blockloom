// The volume heat map: each pixel's surface is put back in the world (from
// depth in 3D, on the screen plane in 2D), measured against every volume the
// way `VolumeSpec::coverage` measures the camera, and tinted by the sum.

struct HeatVolume {
    // xyz the centre, w the shape: 0 box, 1 sphere.
    centre: vec4<f32>,
    // World to the volume's own frame, as a quaternion.
    rotation: vec4<f32>,
    // Half size in world units (x the radius for a sphere), w the blend.
    extent: vec4<f32>,
    // x the volume's weight.
    weight: vec4<f32>,
}

const MAX_VOLUMES: u32 = 32u;

struct HeatUniforms {
    world_from_clip: mat4x4<f32>,
    size: vec2<f32>,
    count: u32,
    // 1 in 2D, where depth only sorts.
    flat: u32,
    volumes: array<HeatVolume, MAX_VOLUMES>,
}

@group(0) @binding(0) var<uniform> heat: HeatUniforms;
@group(0) @binding(1) var screen: texture_2d<f32>;
@if(DEPTH && MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_multisampled_2d;
@if(DEPTH && !MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_2d;

// Reverse-Z: 0 is the far plane, so the sky reads as nothing there.
@if(DEPTH)
fn depth_at(pixel: vec2<i32>) -> f32 {
    return textureLoad(depth, pixel, 0);
}

@if(!DEPTH)
fn depth_at(pixel: vec2<i32>) -> f32 {
    return 0.5;
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn outside(volume: HeatVolume, point: vec3<f32>) -> f32 {
    var d = point - volume.centre.xyz;
    if volume.centre.w > 0.5 {
        if heat.flat != 0u {
            d.z = 0.0;
        }
        return max(length(d) - volume.extent.x, 0.0);
    }
    var q = max(abs(rotate(volume.rotation, d)) - volume.extent.xyz, vec3<f32>(0.0));
    if heat.flat != 0u {
        q.z = 0.0;
    }
    return length(q);
}

fn coverage(volume: HeatVolume, point: vec3<f32>) -> f32 {
    let o = outside(volume, point);
    if o <= 0.0 {
        return 1.0;
    }
    let blend = volume.extent.w;
    if blend <= 0.0 {
        return 0.0;
    }
    return clamp(1.0 - o / blend, 0.0, 1.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// The scene view's heat colours: blue for a trace, red for full cover.
fn heat_color(t: f32) -> vec4<f32> {
    let hue = 240.0 * (1.0 - t) / 60.0;
    let chroma = (1.0 - abs(2.0 * 0.55 - 1.0)) * 0.9;
    let x = chroma * (1.0 - abs(hue % 2.0 - 1.0));
    var rgb = vec3<f32>(x, 0.0, chroma);
    if hue < 1.0 {
        rgb = vec3<f32>(chroma, x, 0.0);
    } else if hue < 2.0 {
        rgb = vec3<f32>(x, chroma, 0.0);
    } else if hue < 3.0 {
        rgb = vec3<f32>(0.0, chroma, x);
    } else if hue < 4.0 {
        rgb = vec3<f32>(0.0, x, chroma);
    }
    rgb += 0.55 - chroma / 2.0;
    return vec4<f32>(to_linear(rgb), 0.3 + 0.5 * t);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let scene = textureLoad(screen, pixel, 0);
    let z = depth_at(pixel);
    if z <= 0.0 {
        return scene;
    }
    let uv = position.xy / heat.size;
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0);
    let world = heat.world_from_clip * clip;
    let point = world.xyz / world.w;
    var total = 0.0;
    for (var i = 0u; i < min(heat.count, MAX_VOLUMES); i++) {
        let volume = heat.volumes[i];
        total += coverage(volume, point) * volume.weight.x;
    }
    if total <= 0.0 {
        return scene;
    }
    let tint = heat_color(clamp(total, 0.0, 1.0));
    return vec4<f32>(mix(scene.rgb, tint.rgb, tint.a), scene.a);
}
