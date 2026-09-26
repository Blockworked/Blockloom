// Planar cloud layers over the scene, premultiplied, farthest first.
import blockloom::sky::{SKY_UNIT,cube_lookup};
import blockloom::scattering::henyey_greenstein;
struct Layer {
    shape: vec4<f32>,
    map: vec4<f32>,
    motion: vec4<f32>,
    pivot: vec4<f32>,
    tint: vec4<f32>,
    sun_tint: vec4<f32>,
    edge_tint: vec4<f32>,
}
struct LayerUniforms {
    world_from_clip: mat4x4<f32>,
    camera: vec4<f32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
    moon: vec4<f32>,
    moon_color: vec4<f32>,
    screen: vec4<f32>,
    range: vec4<u32>,
    layers: array<Layer,4>,
}
@group(0) @binding(0) var<uniform> u: LayerUniforms;
@if(!MULTISAMPLED) @group(0) @binding(1) var depth_tex: texture_depth_2d;
@if(MULTISAMPLED) @group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@group(0) @binding(2) var coverage: texture_2d_array<f32>;
@group(0) @binding(3) var flow: texture_2d_array<f32>;
@group(0) @binding(4) var repeat: sampler;
@group(0) @binding(5) var sky: texture_cube<f32>;
@group(0) @binding(6) var linear: sampler;
fn world(uv: vec2<f32>, d: f32) -> vec3<f32> {
    let p = u.world_from_clip * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), d, 1.0);
    return p.xyz / p.w;
}
// Mirrors `cloud_layers::remap` in core.
fn remap(n: f32, cover: f32, contrast: f32) -> f32 {
    let x = n + cover * 2.0 - 1.0;
    let t = clamp((x - 0.5) * max(contrast, 1.0) + 0.5, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
fn rotate(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a); let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}
// Tile coordinates of a point on the layer: spun round the pivot, then scrolled.
fn layer_uv(l: Layer, xz: vec2<f32>) -> vec2<f32> {
    let spun = rotate(xz - l.pivot.xy, -l.motion.z) + l.pivot.xy;
    return (spun - l.motion.xy) * l.map.x;
}
fn cloud(l: Layer, xz: vec2<f32>) -> f32 {
    let uv = layer_uv(l, xz);
    let slice = i32(l.sun_tint.w);
    var n = 0.0;
    if l.tint.w > 0.5 {
        // Two phases half a period apart, each faded out as it wraps.
        let push = (textureSampleLevel(flow, repeat, uv, slice, 0.0).rg * 2.0 - 1.0) * l.map.w;
        let p0 = fract(l.motion.w);
        let p1 = fract(l.motion.w + 0.5);
        let a = textureSampleLevel(coverage, repeat, uv - push * p0, slice, 0.0).r;
        let b = textureSampleLevel(coverage, repeat, uv - push * p1, slice, 0.0).r;
        n = mix(b, a, 1.0 - abs(2.0 * p0 - 1.0));
    } else {
        n = textureSampleLevel(coverage, repeat, uv, slice, 0.0).r;
    }
    return remap(n, l.shape.z, l.shape.w);
}
fn light(l: Layer, hit: vec3<f32>, ray: vec3<f32>, dir: vec4<f32>, color: vec3<f32>, alpha: f32) -> vec3<f32> {
    if max(max(color.x, color.y), color.z) <= 0.0 || dir.y <= 0.0 { return vec3<f32>(0.0); }
    let to = normalize(dir.xyz);
    // Coverage towards the light through the layer's thickness shades it.
    let shade = exp(-2.5 * cloud(l, hit.xz + to.xz / max(to.y, 0.1) * l.map.y));
    let cos_theta = dot(ray, to);
    let phase = mix(henyey_greenstein(cos_theta, 0.6), 0.0795775, 0.5);
    let body = l.tint.rgb * l.sun_tint.rgb * phase * mix(0.25, 1.0, shade);
    // Thin, backlit edges glow: the powder tint.
    let edge = l.edge_tint.rgb * henyey_greenstein(cos_theta, 0.8) * alpha * (1.0 - alpha) * 2.0;
    return color * (body + edge);
}
@fragment fn fragment(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let pos = vec2<i32>(pixel.xy);
    let uv = pixel.xy / u.screen.xy;
    let d = textureLoad(depth_tex, pos, 0);
    let eye = u.camera.xyz;
    let ray = normalize(world(uv, 0.00001) - eye);
    var scene = 1.0e9;
    if d > 0.0 { scene = length(world(uv, d) - eye); }
    let elevation = abs(ray.y);
    var acc = vec4<f32>(0.0);
    if elevation < 0.00001 { return acc; }
    let horizon = textureSampleLevel(sky, linear, cube_lookup(ray), 0.0).rgb * SKY_UNIT;
    let ambient = textureSampleLevel(sky, linear, cube_lookup(vec3<f32>(0.0, 1.0, 0.0)), 0.0).rgb * SKY_UNIT;
    for (var i = u.range.x; i < u.range.y; i++) {
        let l = u.layers[i];
        let t = (l.shape.x - eye.y) / ray.y;
        if t <= 0.0 || t >= scene { continue; }
        let fade = smoothstep(l.pivot.w, max(l.pivot.z, l.pivot.w + 0.0001), elevation);
        if fade <= 0.0 { continue; }
        let hit = eye + ray * t;
        // Parallax: the ray crosses the layer's thickness, so grazing views
        // see thick parts stretch along it.
        let through = hit.xz + ray.xz / elevation * l.map.y;
        let alpha = max(cloud(l, hit.xz), cloud(l, through)) * l.shape.y * fade;
        if alpha <= 0.0001 { continue; }
        var rgb = l.tint.rgb * ambient
            + light(l, hit, ray, u.sun, u.sun_color.rgb, alpha)
            + light(l, hit, ray, u.moon, u.moon_color.rgb, alpha);
        // Aerial haze: far layers sink into the horizon's own color.
        let haze = (1.0 - exp(-u.camera.w * t)) * l.map.z;
        rgb = mix(rgb, horizon, haze) * u.screen.z;
        acc = vec4<f32>(rgb * alpha, alpha) + acc * (1.0 - alpha);
    }
    return acc;
}
