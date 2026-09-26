// Spatiotemporal denoiser for ray-traced lighting, in the manner of SVGF.
// The traced frame is divided by the G-buffer's albedo, accumulated along the
// motion vectors, blurred by an edge-aware a-trous wavelet whose luminance
// tolerance follows the variance, and multiplied back. Smooth surfaces keep a
// short history and little spatial blur, since what they reflect doesn't move
// with them.

import bevy_render::view::View;
import bevy_render::utils::{octahedral_decode, octahedral_encode};
import bevy_pbr::render::rgb9e5::rgb9e5_to_vec3_;

// `PreviousViewData`, field for field.
struct PreviousView {
    view_from_world: mat4x4<f32>,
    clip_from_world: mat4x4<f32>,
    unjittered_clip_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
}

// `DenoiseUniforms`, field for field.
struct DenoiseUniforms {
    // Nonzero throws the history away.
    reset: u32,
    // Frames a rough surface's history holds at most.
    max_history: f32,
    // Frames a mirror's history holds at most.
    min_history: f32,
    // How strongly the luminance edge stops the blur (SVGF's phi colour).
    color_sigma: f32,
}

// Brightest demodulated luminance kept, in exposed units; its square has to
// fit a half float.
const MAX_SIGNAL: f32 = 200.0;

@group(0) @binding(0) var output: texture_storage_2d<rgba16float, read_write>;
@group(0) @binding(1) var gbuffer: texture_2d<u32>;
@group(0) @binding(2) var depth_buffer: texture_depth_2d;
@group(0) @binding(3) var motion_vectors: texture_2d<f32>;
@group(0) @binding(4) var<uniform> view: View;
@group(0) @binding(5) var<uniform> previous_view: PreviousView;
@group(0) @binding(6) var<uniform> settings: DenoiseUniforms;
// Last frame's accumulated signal, moments (luminance, its square, history
// length) and guide (octahedral normal, view depth, log2 exposure).
@group(0) @binding(7) var history_color: texture_2d<f32>;
@group(0) @binding(8) var history_moments: texture_2d<f32>;
@group(0) @binding(9) var history_guide: texture_2d<f32>;
@group(0) @binding(10) var color_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var moments_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var guide_out: texture_storage_2d<rgba16float, write>;
// The wavelet's ping-pong: signal and its variance.
@group(0) @binding(13) var filter_in: texture_2d<f32>;
@group(0) @binding(14) var filter_out: texture_storage_2d<rgba16float, write>;

// No NaN or infinity in any channel. Checked on the bits, since a driver may
// compile float compares as if NaN can't happen.
fn finite(v: vec3<f32>) -> bool {
    let exponent = bitcast<vec3<u32>>(v) & vec3(0x7f800000u);
    return all(exponent != vec3(0x7f800000u));
}

struct Surface {
    position: vec3<f32>,
    normal: vec3<f32>,
    base_color: vec3<f32>,
    emissive: vec3<f32>,
    perceptual_roughness: f32,
    metallic: f32,
    reflectance: f32,
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3(0.2126, 0.7152, 0.0722));
}

fn view_size() -> vec2<u32> {
    return vec2<u32>(view.main_pass_viewport.zw);
}

fn world_position(pixel: vec2<u32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(view_size());
    let ndc = (uv - vec2(0.5)) * vec2(2.0, -2.0);
    let world = view.world_from_clip * vec4(ndc, depth, 1.0);
    return world.xyz / world.w;
}

fn gbuffer_normal(packed: u32) -> vec3<f32> {
    let packed_normal = vec2(f32(packed & 0xFFFu), f32((packed >> 12u) & 0xFFFu)) / 4095.0;
    return octahedral_decode(packed_normal);
}

// The G-buffer as Solari reads it.
fn load_surface(pixel: vec2<u32>, depth: f32) -> Surface {
    let g = textureLoad(gbuffer, pixel, 0);
    let base_rough = unpack4x8unorm(g.r);
    let props = unpack4x8unorm(g.b);
    var s: Surface;
    s.position = world_position(pixel, depth);
    s.normal = gbuffer_normal(g.a);
    s.base_color = pow(base_rough.rgb, vec3(2.2));
    s.perceptual_roughness = base_rough.a;
    s.emissive = rgb9e5_to_vec3_(g.g);
    s.reflectance = props.r;
    s.metallic = props.g;
    return s;
}

// Traced by Solari or the path tracer: something in the G-buffer at that
// depth. A forward surface leaves the G-buffer empty; the sky has no depth.
fn traced(pixel: vec2<u32>, depth: f32) -> bool {
    return depth != 0.0 && textureLoad(gbuffer, pixel, 0).a != 0u;
}

fn view_depth(view_from_world: mat4x4<f32>, position: vec3<f32>) -> f32 {
    return -(view_from_world * vec4(position, 1.0)).z;
}

// Karis' fit of the split-sum specular reflectance.
fn specular_albedo(f0: vec3<f32>, perceptual_roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// What the traced light is divided by, so the filter blurs light rather
// than texture.
fn albedo(s: Surface) -> vec3<f32> {
    let n_dot_v = max(dot(s.normal, normalize(view.world_position - s.position)), 1.0e-4);
    let f0 = mix(vec3(0.16 * s.reflectance * s.reflectance), s.base_color, s.metallic);
    let diffuse = s.base_color * (1.0 - s.metallic);
    return max(diffuse + specular_albedo(f0, s.perceptual_roughness, n_dot_v), vec3(0.02));
}

// 0 for a rough surface, 1 for a mirror-smooth metal.
fn gloss(s: Surface) -> f32 {
    return (1.0 - smoothstep(0.1, 0.45, s.perceptual_roughness)) * mix(0.4, 1.0, s.metallic);
}

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = view_size();
    if any(id.xy >= size) { return; }
    let pixel = id.xy;
    let depth = textureLoad(depth_buffer, pixel, 0);
    if !traced(pixel, depth) {
        textureStore(color_out, pixel, vec4(0.0));
        textureStore(moments_out, pixel, vec4(0.0));
        textureStore(guide_out, pixel, vec4(0.0));
        textureStore(filter_out, pixel, vec4(0.0));
        return;
    }
    let s = load_surface(pixel, depth);
    let traced = textureLoad(output, pixel).rgb;
    var signal = max(traced - s.emissive * view.exposure, vec3(0.0)) / albedo(s);
    // NaNs and infinities go; fireflies are clipped, which also keeps the
    // squared moments inside half floats.
    if !finite(signal) { signal = vec3(0.0); }
    signal *= min(1.0, MAX_SIGNAL / max(luminance(signal), 1e-6));
    let log_exposure = log2(view.exposure);

    // Where this point stood last frame, and how far from that camera.
    let motion = textureLoad(motion_vectors, pixel, 0).xy;
    let previous = vec2<f32>(pixel) - motion * vec2<f32>(size);
    let expected = view_depth(previous_view.view_from_world, s.position);

    var history = vec3(0.0);
    var moments = vec3(0.0);
    var history_exposure = 0.0;
    var weight = 0.0;
    if settings.reset == 0u {
        let base = floor(previous);
        let f = previous - base;
        for (var i = 0u; i < 4u; i++) {
            let offset = vec2(f32(i & 1u), f32(i >> 1u));
            let tap = base + offset;
            if any(tap < vec2(0.0)) || any(tap >= vec2<f32>(size)) { continue; }
            let texel = vec2<u32>(tap);
            let guide = textureLoad(history_guide, texel, 0);
            // A different surface was there: disoccluded.
            if guide.z <= 0.0 || abs(guide.z - expected) > 0.03 * expected + 0.01 { continue; }
            if dot(octahedral_decode(guide.xy), s.normal) < 0.9 { continue; }
            let bilinear = mix(1.0 - f, f, offset);
            let w = bilinear.x * bilinear.y;
            history += w * textureLoad(history_color, texel, 0).rgb;
            moments += w * textureLoad(history_moments, texel, 0).xyz;
            history_exposure += w * guide.w;
            weight += w;
        }
    }

    var accumulated = signal;
    let first = luminance(signal);
    var m = vec2(first, first * first);
    var length = 1.0;
    if weight > 0.01 {
        history /= weight;
        moments /= weight;
        // Exposure moved since: the history is in last frame's units.
        let scale = exp2(log_exposure - history_exposure / weight);
        history *= scale;
        let longest = mix(settings.max_history, settings.min_history, gloss(s));
        length = min(moments.z + 1.0, longest);
        let alpha = 1.0 / length;
        accumulated = mix(history, signal, alpha);
        m = mix(moments.xy * vec2(scale, scale * scale), m, alpha);
    }
    var variance = max(m.y - m.x * m.x, 0.0);
    // A fresh history knows nothing of its noise yet: blur it freely.
    if length < 4.0 {
        let l = luminance(accumulated);
        variance = max(variance, l * l * (4.0 - length));
    }

    textureStore(color_out, pixel, vec4(accumulated, 0.0));
    textureStore(moments_out, pixel, vec4(m, length, 0.0));
    textureStore(guide_out, pixel, vec4(octahedral_encode(s.normal), view_depth(view.view_from_world, s.position), log_exposure));
    textureStore(filter_out, pixel, vec4(accumulated, min(variance, MAX_SIGNAL * MAX_SIGNAL)));
}

// The B3 spline, 1/16 4/16 6/16 4/16 1/16, from the centre out.
fn kernel(offset: i32) -> f32 {
    return select(select(0.0625, 0.25, abs(offset) == 1), 0.375, offset == 0);
}

@compute @workgroup_size(8, 8, 1)
fn atrous(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(view_size());
    if any(vec2<i32>(id.xy) >= size) { return; }
    let pixel = vec2<i32>(id.xy);
    let center = textureLoad(filter_in, pixel, 0);
    let depth = textureLoad(depth_buffer, pixel, 0);
    if !traced(vec2<u32>(pixel), depth) {
        textureStore(filter_out, pixel, center);
        return;
    }
    let s = load_surface(vec2<u32>(pixel), depth);
    let step = i32(constants::STEP);
    let tolerance = 0.01 * view_depth(view.view_from_world, s.position) + 0.001;
    // Mirrors mostly keep their own pixel.
    let spread = 1.0 - 0.9 * gloss(s);

    // The variance, blurred a little, sets the luminance tolerance.
    var variance = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(pixel + vec2(x, y), vec2(0), size - 1);
            let k = select(0.125, 0.25, x == 0) * select(0.5, 1.0, y == 0);
            variance += k * textureLoad(filter_in, q, 0).a;
        }
    }
    let sigma = settings.color_sigma * sqrt(max(variance, 0.0)) + 1.0e-4;
    let center_luminance = luminance(center.rgb);

    let own = kernel(0) * kernel(0);
    var sum = own * center.rgb;
    var weights = own;
    var variances = own * own * center.a;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            if x == 0 && y == 0 { continue; }
            let q = pixel + vec2(x, y) * step;
            if any(q < vec2(0)) || any(q >= size) { continue; }
            let q_depth = textureLoad(depth_buffer, q, 0);
            let packed = textureLoad(gbuffer, q, 0).a;
            if q_depth == 0.0 || packed == 0u { continue; }
            let sample = textureLoad(filter_in, q, 0);
            let normal = gbuffer_normal(packed);
            let plane = abs(dot(s.normal, world_position(vec2<u32>(q), q_depth) - s.position));
            let w = kernel(x) * kernel(y) * spread
                * pow(max(dot(s.normal, normal), 0.0), 64.0)
                * exp(-plane / tolerance)
                * exp(-abs(luminance(sample.rgb) - center_luminance) / sigma);
            sum += w * sample.rgb;
            weights += w;
            variances += w * w * sample.a;
        }
    }
    let filtered = vec4(sum / weights, variances / (weights * weights));
    textureStore(filter_out, pixel, filtered);
    // The first pass's result is next frame's history, as in SVGF.
    @if(FEEDBACK) {
        textureStore(color_out, pixel, vec4(filtered.rgb, 0.0));
    }
}

@compute @workgroup_size(8, 8, 1)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= view_size()) { return; }
    let pixel = id.xy;
    let depth = textureLoad(depth_buffer, pixel, 0);
    if !traced(pixel, depth) { return; }
    let s = load_surface(pixel, depth);
    let light = textureLoad(filter_in, pixel, 0).rgb;
    textureStore(output, pixel, vec4(light * albedo(s) + s.emissive * view.exposure, 1.0));
}
