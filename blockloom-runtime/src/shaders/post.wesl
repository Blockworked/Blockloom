// The post stack's own passes. Bloom is a five-level chain: a thresholded
// prefilter into the first level, 13-tap downsamples, then tent upsamples
// that scatter each level into the one above. `composite` lays the bloom
// and its lens dirt over the exposed scene, white balances, grades and
// shapes the tone, all before the tonemapper; `finish` runs after it and
// sharpening for the LUT and film grain. Each entry point binds only its
// own slots, so every binding number is unique.

struct PostUniforms {
    // threshold, knee (0-1 of the threshold), scatter, intensity
    bloom: vec4<f32>,
    // dirt intensity, bloom on, debug mode, grading on
    flags: vec4<f32>,
    // toe, shoulder, saturation, contrast
    tone: vec4<f32>,
    // The white balance matrix, column by column.
    balance_x: vec4<f32>,
    balance_y: vec4<f32>,
    balance_z: vec4<f32>,
    // w: AO texture bound
    lift: vec4<f32>,
    // w: depth bound
    gamma: vec4<f32>,
    // w: tone shape on
    gain: vec4<f32>,
    // contribution, texels a side, LUT on, HDR output
    lut: vec4<f32>,
    // domain min, atlas page
    lut_min: vec4<f32>,
    // domain max, atlas pages
    lut_max: vec4<f32>,
    // intensity, size in pixels, response, seed
    grain: vec4<f32>,
    // focus distance, CoC scale, focal length, widest CoC in pixels
    dof: vec4<f32>,
    // far limit (negative: near field kept sharp), near plane, 0, 0
    dof_view: vec4<f32>,
}

@group(0) @binding(0) var<uniform> post: PostUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;
// The level below, for an upsample.
@group(0) @binding(3) var low: texture_2d<f32>;
@group(0) @binding(4) var bloom: texture_2d<f32>;
@group(0) @binding(5) var dirt: texture_2d<f32>;
@group(0) @binding(6) var ambient_occlusion: texture_2d<f32>;
@if(DEPTH && MULTISAMPLED)
@group(0) @binding(7) var depth: texture_depth_multisampled_2d;
@if(DEPTH && !MULTISAMPLED)
@group(0) @binding(7) var depth: texture_depth_2d;
@group(0) @binding(8) var lut: texture_3d<f32>;

const MIDDLE_GREY: f32 = 0.18;

// Debug modes, `post::PostDebug`.
const DEBUG_BLOOM_MIP: f32 = 1.0;
const DEBUG_COC: f32 = 2.0;
const DEBUG_AO: f32 = 3.0;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ─── Bloom ───────────────────────────────────────────────────────────────

// Quadratic soft threshold: a knee either side of the threshold.
fn soft_threshold(c: vec3<f32>) -> vec3<f32> {
    let threshold = post.bloom.x;
    let knee = threshold * post.bloom.y + 1e-5;
    let brightness = max(c.r, max(c.g, c.b));
    var soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    let weight = max(soft, brightness - threshold) / max(brightness, 1e-5);
    return c * weight;
}

// The 13-tap downsample from Call of Duty: Advanced Warfare.
fn sample13(tex: texture_2d<f32>, uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(tex));
    let a = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-2.0, -2.0), 0.0).rgb;
    let b = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(0.0, -2.0), 0.0).rgb;
    let c = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(2.0, -2.0), 0.0).rgb;
    let d = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-2.0, 0.0), 0.0).rgb;
    let e = textureSampleLevel(tex, linear_clamp, uv, 0.0).rgb;
    let f = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(2.0, 0.0), 0.0).rgb;
    let g = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-2.0, 2.0), 0.0).rgb;
    let h = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(0.0, 2.0), 0.0).rgb;
    let i = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(2.0, 2.0), 0.0).rgb;
    let j = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    let k = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb;
    let l = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    let m = textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb;
    if karis {
        // Each group weighed down by its brightness, so a single hot pixel
        // can't flicker the whole chain.
        let g0 = (a + b + d + e) * 0.25;
        let g1 = (b + c + e + f) * 0.25;
        let g2 = (d + e + g + h) * 0.25;
        let g3 = (e + f + h + i) * 0.25;
        let g4 = (j + k + l + m) * 0.25;
        let w0 = 0.125 / (1.0 + luminance(g0));
        let w1 = 0.125 / (1.0 + luminance(g1));
        let w2 = 0.125 / (1.0 + luminance(g2));
        let w3 = 0.125 / (1.0 + luminance(g3));
        let w4 = 0.5 / (1.0 + luminance(g4));
        return (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
    }
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

// A 3x3 tent over the level below.
fn tent(tex: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(tex));
    var sum = textureSampleLevel(tex, linear_clamp, uv, 0.0).rgb * 4.0;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-1.0, 0.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(1.0, 0.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(0.0, -1.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(0.0, 1.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    sum += textureSampleLevel(tex, linear_clamp, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb;
    return sum / 16.0;
}

@fragment
fn prefilter(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let c = sample13(source, uv, true);
    // Keeps NaN and inf out of the chain.
    let safe = select(vec3<f32>(0.0), min(c, vec3<f32>(65000.0)), c == c);
    return vec4<f32>(soft_threshold(safe), 1.0);
}

@fragment
fn downsample(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(sample13(source, uv, false), 1.0);
}

// This level, with the upsampled level below scattered into it.
@fragment
fn upsample(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let here = textureSampleLevel(source, linear_clamp, uv, 0.0).rgb;
    let below = tent(low, uv);
    return vec4<f32>(mix(here, below, post.bloom.z), 1.0);
}

// ─── Composite: bloom, white balance, grading, tone shape ────────────────

fn tone_shape(x: f32) -> f32 {
    let v = max(x, 0.0);
    if v < MIDDLE_GREY {
        let t = v / MIDDLE_GREY;
        let weight = 1.0 - t * t * (3.0 - 2.0 * t);
        return MIDDLE_GREY * pow(t, 1.0 + post.tone.x * weight);
    }
    let over = v - MIDDLE_GREY;
    return MIDDLE_GREY + over / (1.0 + post.tone.y * over);
}

fn grade(input: vec3<f32>) -> vec3<f32> {
    let balance = mat3x3<f32>(post.balance_x.xyz, post.balance_y.xyz, post.balance_z.xyz);
    var c = max(balance * input, vec3<f32>(0.0));
    // Contrast in log space, pivoting on middle grey.
    c = MIDDLE_GREY * pow(c / MIDDLE_GREY, vec3<f32>(post.tone.w));
    // Lift raises the shadows and leaves white where it was; gain scales.
    c = c * post.gain.xyz + post.lift.xyz * (vec3<f32>(1.0) - clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    c = pow(max(c, vec3<f32>(0.0)), 1.0 / post.gamma.xyz);
    let l = luminance(c);
    return max(vec3<f32>(l) + (c - vec3<f32>(l)) * post.tone.z, vec3<f32>(0.0));
}

// Reverse-Z infinite perspective, as Bevy's `depth_ndc_to_view_z`.
fn view_depth(ndc: f32) -> f32 {
    return post.dof_view.y / max(ndc, 1e-7);
}

@if(DEPTH)
fn depth_at(pixel: vec2<i32>) -> f32 {
    return textureLoad(depth, pixel, 0);
}

@if(!DEPTH)
fn depth_at(pixel: vec2<i32>) -> f32 {
    return 0.0;
}

// Bevy's circle of confusion, signed: negative in front of the focus.
fn circle_of_confusion(pixel: vec2<i32>) -> f32 {
    let focus = post.dof.x;
    let limit = abs(post.dof_view.x);
    let d = min(view_depth(depth_at(pixel)), limit);
    let size = post.dof.y * abs(d - focus) / (d * max(focus - post.dof.z, 1e-5));
    let height = f32(textureDimensions(source).y);
    let coc = clamp(size * height, 0.0, post.dof.w);
    if d < focus {
        if post.dof_view.x < 0.0 {
            return 0.0;
        }
        return -coc;
    }
    return coc;
}

fn debug_view(mode: f32, uv: vec2<f32>, pixel: vec2<i32>) -> vec3<f32> {
    if mode == DEBUG_BLOOM_MIP {
        return textureSampleLevel(bloom, linear_clamp, uv, 0.0).rgb;
    }
    if mode == DEBUG_COC {
        if post.gamma.w < 0.5 {
            return vec3<f32>(0.0);
        }
        let coc = circle_of_confusion(pixel) / max(post.dof.w, 1.0);
        if coc < 0.0 {
            return vec3<f32>(1.0, 0.45, 0.1) * -coc;
        }
        return vec3<f32>(0.15, 0.45, 1.0) * coc;
    }
    if post.lift.w < 0.5 {
        return vec3<f32>(1.0);
    }
    let size = vec2<f32>(textureDimensions(ambient_occlusion));
    let texel = vec2<i32>(uv * size);
    return vec3<f32>(textureLoad(ambient_occlusion, texel, 0).r);
}

@fragment
fn composite(@builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let mode = post.flags.z;
    if mode > 0.5 {
        return vec4<f32>(debug_view(mode, uv, pixel), 1.0);
    }
    let scene = textureLoad(source, pixel, 0);
    var c = scene.rgb;
    if post.flags.y > 0.5 {
        var glow = textureSampleLevel(bloom, linear_clamp, uv, 0.0).rgb;
        let lens = textureSampleLevel(dirt, linear_clamp, uv, 0.0).rgb;
        glow *= vec3<f32>(1.0) + lens * post.flags.x;
        c += glow * post.bloom.w;
    }
    if post.flags.w > 0.5 {
        c = grade(c);
    }
    if post.gain.w > 0.5 {
        c = vec3<f32>(tone_shape(c.r), tone_shape(c.g), tone_shape(c.b));
    }
    return vec4<f32>(c, scene.a);
}

// ─── Finish: LUT and grain, on display values ────────────────────────────

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let v = max(c, vec3<f32>(0.0));
    return select(pow((v + 0.055) / 1.055, vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
}

fn hash(p: vec2<u32>, seed: u32) -> f32 {
    var h = p.x * 1664525u + p.y * 1013904223u + seed * 747796405u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967295.0;
}

// Value noise over grain-sized cells, so a bigger grain is softer too.
fn grain_noise(position: vec2<f32>) -> f32 {
    let size = max(post.grain.y, 1.0);
    let p = position / size;
    let cell = floor(p);
    let f = p - cell;
    let seed = u32(post.grain.w);
    let base = vec2<u32>(vec2<i32>(cell) + vec2<i32>(4096));
    let a = hash(base, seed);
    let b = hash(base + vec2<u32>(1u, 0u), seed);
    let c = hash(base + vec2<u32>(0u, 1u), seed);
    let d = hash(base + vec2<u32>(1u, 1u), seed);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

@fragment
fn finish(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let scene = textureLoad(source, vec2<i32>(position.xy), 0);
    var c = scene.rgb;
    // A LUT maps display values, which HDR output hasn't got yet.
    if post.lut.z > 0.5 && post.lut.w < 0.5 {
        let span = max(post.lut_max.xyz - post.lut_min.xyz, vec3<f32>(1e-5));
        let coord = clamp((to_srgb(c) - post.lut_min.xyz) / span, vec3<f32>(0.0), vec3<f32>(1.0));
        let n = post.lut.y;
        let texel = coord * (n - 1.0) / n + 0.5 / n;
        // The atlas stacks LUT pages in depth: this page's slab.
        let slab = (post.lut_min.w + texel.z) / max(post.lut_max.w, 1.0);
        let graded = textureSampleLevel(lut, linear_clamp, vec3<f32>(texel.xy, slab), 0.0).rgb;
        c = mix(c, to_linear(graded), post.lut.x);
    }
    if post.grain.x > 0.0 {
        let e = to_srgb(c);
        let n = grain_noise(position.xy) - 0.5;
        // The response fades grain out of the highlights, as film does.
        let amount = post.grain.x * 0.25 * mix(1.0, 1.0 - luminance(e), post.grain.z);
        let grained = e + vec3<f32>(n * amount);
        if post.lut.w > 0.5 {
            c = max(c + vec3<f32>(n * amount) * max(luminance(c), 0.05), vec3<f32>(0.0));
        } else {
            c = to_linear(grained);
        }
    }
    return vec4<f32>(c, scene.a);
}
