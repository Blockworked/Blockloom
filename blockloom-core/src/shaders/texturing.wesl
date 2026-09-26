// World-space texturing: triplanar and stochastic (hex-tiled) sampling,
// macro and micro variation, and the mask stack. Box-projected surfaces and
// terrain both draw through these, so a dial means the same thing on each.
// Everything samples with explicit gradients, so it is safe in branches.

import blockloom::hash::hash_cell2;
import blockloom::noise::value2;

// What every projected surface reads each frame, from one shared buffer.
struct SurfaceGlobals {
    // Snow cover 0-1, wetness 0-1, debug view (1 blends), seconds.
    weather: vec4<f32>,
    // Wind heading (x, z), speed in m/s, gust 0-1.
    wind: vec4<f32>,
}

// A material's mask stack. A rule with strength 0 is off.
struct MaskUniforms {
    // start, blend, strength, roughness
    slope: vec4<f32>,
    // linear rgb, invert
    slope_color: vec4<f32>,
    height: vec4<f32>,
    height_color: vec4<f32>,
    cavity: vec4<f32>,
    cavity_color: vec4<f32>,
    // amount (0 off), max slope, min height, blend
    snow: vec4<f32>,
    // linear rgb, roughness
    snow_color: vec4<f32>,
    // amount (0 off), darken, roughness, puddles
    wetness: vec4<f32>,
}

// The detail pass's dials.
struct DetailUniforms {
    // stochastic (0/1), contrast, macro strength, 1 / macro size
    tiling: vec4<f32>,
    // detail strength, 1 / detail size, detail fade distance, macro map (0/1)
    detail: vec4<f32>,
}

// What the mask stack made of a surface, and how much of each mask landed.
struct Masked {
    albedo: vec3<f32>,
    roughness: f32,
    // rules, snow, wetness, unused
    coverage: vec4<f32>,
}

// Three hex-tile lookups and their blend weights.
struct Tiles {
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    weights: vec3<f32>,
}

// Plane weights from a normal, sharpened so mostly one plane shows.
fn triplanar_weights(n: vec3<f32>, sharpness: f32) -> vec3<f32> {
    let w = pow(abs(n), vec3<f32>(sharpness));
    return w / max(dot(w, vec3<f32>(1.0)), 1e-4);
}

// Degrees from flat for a world normal.
fn slope_degrees(n: vec3<f32>) -> f32 {
    return degrees(acos(clamp(n.y, -1.0, 1.0)));
}

fn smooth01(t: f32) -> f32 {
    let c = clamp(t, 0.0, 1.0);
    return c * c * (3.0 - 2.0 * c);
}

// The triangle of hex tiles `uv` falls in (Heitz & Neyret, Deliot & Heitz
// 2019), each tile's lookup shifted by a hash of its corner. `contrast`
// sharpens the blend from soft (0) to nearly hard seams (1).
fn hex_tiles(uv: vec2<f32>, contrast: f32) -> Tiles {
    let grid = uv * 3.4641016;
    let skewed = vec2<f32>(grid.x, -0.57735027 * grid.x + 1.15470054 * grid.y);
    let base = vec2<i32>(floor(skewed));
    let f = fract(skewed);
    let z = 1.0 - f.x - f.y;
    var w: vec3<f32>;
    var v0: vec2<i32>;
    var v1: vec2<i32>;
    var v2: vec2<i32>;
    if (z > 0.0) {
        w = vec3<f32>(z, f.y, f.x);
        v0 = base;
        v1 = base + vec2<i32>(0, 1);
        v2 = base + vec2<i32>(1, 0);
    } else {
        w = vec3<f32>(-z, 1.0 - f.y, 1.0 - f.x);
        v0 = base + vec2<i32>(1, 1);
        v1 = base + vec2<i32>(1, 0);
        v2 = base + vec2<i32>(0, 1);
    }
    let sharp = pow(w, vec3<f32>(1.0 + contrast * 7.0));
    var tiles: Tiles;
    tiles.weights = sharp / max(dot(sharp, vec3<f32>(1.0)), 1e-5);
    tiles.uv0 = uv + hash_cell2(v0) * 7.31;
    tiles.uv1 = uv + hash_cell2(v1) * 7.31;
    tiles.uv2 = uv + hash_cell2(v2) * 7.31;
    return tiles;
}

// One map at `uv`, plainly or hex-tiled.
fn sample_uv(
    t: texture_2d<f32>,
    s: sampler,
    uv: vec2<f32>,
    gx: vec2<f32>,
    gy: vec2<f32>,
    stochastic: bool,
    contrast: f32,
) -> vec4<f32> {
    if (!stochastic) {
        return textureSampleGrad(t, s, uv, gx, gy);
    }
    let tiles = hex_tiles(uv, contrast);
    return textureSampleGrad(t, s, tiles.uv0, gx, gy) * tiles.weights.x
        + textureSampleGrad(t, s, tiles.uv1, gx, gy) * tiles.weights.y
        + textureSampleGrad(t, s, tiles.uv2, gx, gy) * tiles.weights.z;
}

// A layer of an array at `uv`, plainly or hex-tiled.
fn sample_uv_array(
    t: texture_2d_array<f32>,
    s: sampler,
    layer: i32,
    uv: vec2<f32>,
    gx: vec2<f32>,
    gy: vec2<f32>,
    stochastic: bool,
    contrast: f32,
) -> vec4<f32> {
    if (!stochastic) {
        return textureSampleGrad(t, s, uv, layer, gx, gy);
    }
    let tiles = hex_tiles(uv, contrast);
    return textureSampleGrad(t, s, tiles.uv0, layer, gx, gy) * tiles.weights.x
        + textureSampleGrad(t, s, tiles.uv1, layer, gx, gy) * tiles.weights.y
        + textureSampleGrad(t, s, tiles.uv2, layer, gx, gy) * tiles.weights.z;
}

// A map projected along each world axis at `scale` tiles per metre and
// blended by `w`. `gx`/`gy` are the screen derivatives of `pos`.
fn triplanar(
    t: texture_2d<f32>,
    s: sampler,
    pos: vec3<f32>,
    gx: vec3<f32>,
    gy: vec3<f32>,
    w: vec3<f32>,
    scale: f32,
    stochastic: bool,
    contrast: f32,
) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    if (w.x > 0.01) {
        sum += sample_uv(t, s, pos.zy * scale, gx.zy * scale, gy.zy * scale, stochastic, contrast) * w.x;
    }
    if (w.y > 0.01) {
        sum += sample_uv(t, s, pos.xz * scale, gx.xz * scale, gy.xz * scale, stochastic, contrast) * w.y;
    }
    if (w.z > 0.01) {
        sum += sample_uv(t, s, pos.xy * scale, gx.xy * scale, gy.xy * scale, stochastic, contrast) * w.z;
    }
    return sum / max(dot(select(vec3<f32>(0.0), w, w > vec3<f32>(0.01)), vec3<f32>(1.0)), 1e-4);
}

fn triplanar_array(
    t: texture_2d_array<f32>,
    s: sampler,
    layer: i32,
    pos: vec3<f32>,
    gx: vec3<f32>,
    gy: vec3<f32>,
    w: vec3<f32>,
    scale: f32,
    stochastic: bool,
    contrast: f32,
) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    if (w.x > 0.01) {
        sum += sample_uv_array(t, s, layer, pos.zy * scale, gx.zy * scale, gy.zy * scale, stochastic, contrast) * w.x;
    }
    if (w.y > 0.01) {
        sum += sample_uv_array(t, s, layer, pos.xz * scale, gx.xz * scale, gy.xz * scale, stochastic, contrast) * w.y;
    }
    if (w.z > 0.01) {
        sum += sample_uv_array(t, s, layer, pos.xy * scale, gx.xy * scale, gy.xy * scale, stochastic, contrast) * w.z;
    }
    return sum / max(dot(select(vec3<f32>(0.0), w, w > vec3<f32>(0.01)), vec3<f32>(1.0)), 1e-4);
}

// Three tangent-space normals, one per projection, as one world normal
// around the geometric normal `n` (Golus' whiteout triplanar blend).
fn triplanar_normals(nx: vec3<f32>, ny: vec3<f32>, nz: vec3<f32>, n: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let x = vec3<f32>(nx.xy + n.zy, abs(nx.z) * n.x);
    let y = vec3<f32>(ny.xy + n.xz, abs(ny.z) * n.y);
    let z = vec3<f32>(nz.xy + n.xy, abs(nz.z) * n.z);
    return normalize(x.zyx * w.x + y.xzy * w.y + z.xyz * w.z);
}

// A normal map projected along each axis, as a world normal around `n`.
fn triplanar_normal(
    t: texture_2d<f32>,
    s: sampler,
    pos: vec3<f32>,
    gx: vec3<f32>,
    gy: vec3<f32>,
    n: vec3<f32>,
    w: vec3<f32>,
    scale: f32,
    stochastic: bool,
    contrast: f32,
) -> vec3<f32> {
    let flat = vec3<f32>(0.0, 0.0, 1.0);
    var tx = flat;
    var ty = flat;
    var tz = flat;
    if (w.x > 0.01) {
        tx = sample_uv(t, s, pos.zy * scale, gx.zy * scale, gy.zy * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    if (w.y > 0.01) {
        ty = sample_uv(t, s, pos.xz * scale, gx.xz * scale, gy.xz * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    if (w.z > 0.01) {
        tz = sample_uv(t, s, pos.xy * scale, gx.xy * scale, gy.xy * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    return triplanar_normals(tx, ty, tz, n, w);
}

fn triplanar_normal_array(
    t: texture_2d_array<f32>,
    s: sampler,
    layer: i32,
    pos: vec3<f32>,
    gx: vec3<f32>,
    gy: vec3<f32>,
    n: vec3<f32>,
    w: vec3<f32>,
    scale: f32,
    stochastic: bool,
    contrast: f32,
) -> vec3<f32> {
    let flat = vec3<f32>(0.0, 0.0, 1.0);
    var tx = flat;
    var ty = flat;
    var tz = flat;
    if (w.x > 0.01) {
        tx = sample_uv_array(t, s, layer, pos.zy * scale, gx.zy * scale, gy.zy * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    if (w.y > 0.01) {
        ty = sample_uv_array(t, s, layer, pos.xz * scale, gx.xz * scale, gy.xz * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    if (w.z > 0.01) {
        tz = sample_uv_array(t, s, layer, pos.xy * scale, gx.xy * scale, gy.xy * scale, stochastic, contrast).xyz * 2.0 - 1.0;
    }
    return triplanar_normals(tx, ty, tz, n, w);
}

// Procedural macro variation: two octaves of value noise, 0-1.
fn macro_noise(xz: vec2<f32>, inverse_size: f32) -> f32 {
    let p = xz * inverse_size;
    return value2(p) * 0.65 + value2(p * 2.7 + vec2<f32>(17.3, 5.1)) * 0.35;
}

// The multiplier macro variation lays over albedo for a variation value
// `v` (0-1) at `strength`.
fn macro_tint(v: f32, strength: f32) -> f32 {
    return mix(1.0, 0.55 + v * 0.9, strength);
}

// A detail normal `d` (world space, around the geometric normal `n`) laid
// over the surface normal `base`, faded out past `fade` metres.
fn blend_detail(base: vec3<f32>, d: vec3<f32>, n: vec3<f32>, strength: f32, distance: f32, fade: f32) -> vec3<f32> {
    let keep = strength * (1.0 - smooth01(distance / max(fade, 1e-3)));
    return normalize(base + (d - n) * keep);
}

// How much a rule mask covers a surface whose rule reads `value`.
fn rule_weight(value: f32, rule: vec4<f32>, invert: f32) -> f32 {
    if (rule.z <= 0.0) {
        return 0.0;
    }
    var t = select(step(rule.x, value), clamp((value - rule.x) / rule.y, 0.0, 1.0), rule.y > 0.0);
    if (invert > 0.5) {
        t = 1.0 - t;
    }
    return smooth01(t) * rule.z;
}

// Snow depth 0-1: `SnowMask::weight` on the CPU.
fn snow_weight(slope: f32, height: f32, cover: f32, snow: vec4<f32>) -> f32 {
    if (snow.x <= 0.0) {
        return 0.0;
    }
    let blend = snow.w;
    var flat = 1.0 - step(snow.y, slope);
    var high = step(snow.z, height);
    if (blend > 0.0) {
        flat = 1.0 - clamp((slope - snow.y) / blend + 0.5, 0.0, 1.0);
        high = clamp((height - snow.z) / blend + 0.5, 0.0, 1.0);
    }
    let depth = clamp(cover * snow.x, 0.0, 1.0);
    return smooth01(flat * high * 2.0 * depth);
}

// The mask stack in order: slope, height and cavity tint and roughen, snow
// settles, wetness darkens and glosses. `cavity` is 0-1, hollows high.
fn apply_masks(
    albedo: vec3<f32>,
    roughness: f32,
    slope: f32,
    height: f32,
    cavity: f32,
    masks: MaskUniforms,
    globals: SurfaceGlobals,
) -> Masked {
    var out: Masked;
    out.albedo = albedo;
    out.roughness = roughness;
    let a = rule_weight(slope, masks.slope, masks.slope_color.w);
    out.albedo = mix(out.albedo, masks.slope_color.rgb, a);
    out.roughness = mix(out.roughness, masks.slope.w, a);
    let b = rule_weight(height, masks.height, masks.height_color.w);
    out.albedo = mix(out.albedo, masks.height_color.rgb, b);
    out.roughness = mix(out.roughness, masks.height.w, b);
    let c = rule_weight(cavity, masks.cavity, masks.cavity_color.w);
    out.albedo = mix(out.albedo, masks.cavity_color.rgb, c);
    out.roughness = mix(out.roughness, masks.cavity.w, c);
    let snow = snow_weight(slope, height, globals.weather.x, masks.snow);
    out.albedo = mix(out.albedo, masks.snow_color.rgb, snow);
    out.roughness = mix(out.roughness, masks.snow_color.w, snow);
    var wet = 0.0;
    if (masks.wetness.x > 0.0) {
        let level = clamp(globals.weather.y * masks.wetness.x, 0.0, 1.0);
        // Hollows fill first; with puddles at 0 everything wets evenly.
        let pooled = clamp(level * 2.0 - 1.0 + cavity * masks.wetness.w * 2.0, 0.0, 1.0);
        wet = mix(level, pooled, masks.wetness.w) * (1.0 - snow);
        out.albedo *= 1.0 - masks.wetness.y * wet;
        out.roughness = mix(out.roughness, masks.wetness.z, wet);
    }
    out.coverage = vec4<f32>(max(max(a, b), c), snow, wet, 0.0);
    return out;
}

// The blend debug view: paint layers as red, green, blue and yellow, rule
// masks as magenta, snow as white and wetness as cyan over them.
fn blend_debug(layers: vec4<f32>, coverage: vec4<f32>) -> vec3<f32> {
    var color = layers.x * vec3<f32>(0.8, 0.15, 0.1)
        + layers.y * vec3<f32>(0.15, 0.7, 0.15)
        + layers.z * vec3<f32>(0.15, 0.25, 0.85)
        + layers.w * vec3<f32>(0.85, 0.75, 0.1);
    color = mix(color, vec3<f32>(0.85, 0.1, 0.85), coverage.x * 0.8);
    color = mix(color, vec3<f32>(1.0), coverage.y);
    color = mix(color, vec3<f32>(0.1, 0.8, 0.9), coverage.z * 0.8);
    return color;
}
