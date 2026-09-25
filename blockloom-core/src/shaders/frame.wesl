// The standard per-view uniform every Blockloom pass binds: the clock, the
// blended environment and the working target sizes. Mirrors
// `passes::FrameUniforms` on the Rust side; change both together.

struct FrameUniforms {
    // Seconds since start (wraps hourly), seconds since the last frame.
    time: f32,
    delta: f32,
    frame: u32,
    // EV100, resolved the way the camera's exposure is.
    exposure: f32,
    // Unit vector towards the sun, and its illuminance in lux.
    sun_direction: vec3<f32>,
    sun_illuminance: f32,
    // Linear colors. The ambient's alpha is its brightness.
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    background: vec4<f32>,
    // Full-res working target in pixels, and one texel in UV.
    target_size: vec2<f32>,
    target_texel: vec2<f32>,
    // The half-res scratch pair, likewise.
    scratch_size: vec2<f32>,
    scratch_texel: vec2<f32>,
}

// What the camera multiplies scene light by at `ev100`, as Bevy does.
fn exposure_scale(ev100: f32) -> f32 {
    return 1.0 / (1.2 * exp2(ev100));
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The full-res pixel a scratch texel stands for. A half-res pass reads depth
// here and the bilateral upsample compares against the same pixel.
fn full_texel(scratch: vec2<i32>) -> vec2<i32> {
    return scratch * 2;
}
