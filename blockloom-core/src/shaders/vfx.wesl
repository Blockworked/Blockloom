// The VFX graph's shared half: the particle slot both sims fill and the
// renderer draws, the random streams they spawn with and the noise fields
// the update modules push by. `vfx::Particle` and `vfx::curl` in core are the
// CPU twins; change them together.

import blockloom::hash::{pcg, unit};
import blockloom::noise::gradient3;

const FLAG_PINNED: u32 = 1u;
const LUT: u32 = 32u;
const TAU: f32 = 6.28318530717959;

// One slot. `life <= 0` is empty.
struct Particle {
    position: vec3<f32>,
    age: f32,
    velocity: vec3<f32>,
    life: f32,
    // Degrees, degrees a second.
    rotation: f32,
    spin: f32,
    size: f32,
    seed: u32,
    // Newest ribbon point in the low 16 bits, points held in the high 16.
    ribbon: u32,
    flags: u32,
    hits: u32,
    frame: f32,
}

fn alive(p: Particle) -> bool {
    return p.life > 0.0;
}

fn pinned(p: Particle) -> bool {
    return (p.flags & FLAG_PINNED) != 0u;
}

// Life fraction, 0 at birth and 1 at death.
fn life_t(p: Particle) -> f32 {
    return clamp(p.age / max(p.life, 1e-4), 0.0, 1.0);
}

// The LUT slot a life fraction reads, with the blend to the next.
fn lut_at(t: f32) -> vec2<f32> {
    let x = clamp(t, 0.0, 1.0) * f32(LUT - 1u);
    return vec2<f32>(floor(x), fract(x));
}

// Next float in [0, 1) off a stream.
fn rand(state: ptr<function, u32>) -> f32 {
    *state = pcg(*state);
    return unit(*state);
}

// Three decorrelated noise channels.
fn noise3(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        gradient3(p),
        gradient3(p + vec3<f32>(31.4, -17.2, 9.7)),
        gradient3(p + vec3<f32>(-8.3, 23.9, 41.1)),
    );
}

// The curl of a noise potential: a swirl with no sources or sinks. Flat
// worlds take the curl of one channel, which stays in the plane.
fn curl(p: vec3<f32>, flat: bool) -> vec3<f32> {
    let e = 0.05;
    let x = vec3<f32>(e, 0.0, 0.0);
    let y = vec3<f32>(0.0, e, 0.0);
    let z = vec3<f32>(0.0, 0.0, e);
    if (flat) {
        let dx = (gradient3(p + x) - gradient3(p - x)) / (2.0 * e);
        let dy = (gradient3(p + y) - gradient3(p - y)) / (2.0 * e);
        return vec3<f32>(dy, -dx, 0.0);
    }
    let dx = (noise3(p + x) - noise3(p - x)) / (2.0 * e);
    let dy = (noise3(p + y) - noise3(p - y)) / (2.0 * e);
    let dz = (noise3(p + z) - noise3(p - z)) / (2.0 * e);
    return vec3<f32>(dy.z - dz.y, dz.x - dx.z, dx.y - dy.x);
}

// A direction on the unit sphere (or circle, flat) off two uniforms.
fn ball(u: vec2<f32>, flat: bool) -> vec3<f32> {
    if (flat) {
        let angle = u.x * TAU;
        return vec3<f32>(cos(angle), sin(angle), 0.0);
    }
    let z = u.x * 2.0 - 1.0;
    let angle = u.y * TAU;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(angle), z, r * sin(angle));
}

// A launch direction within `half` radians of the facing: +x flat, -z in 3D.
fn cone(u: vec2<f32>, half: f32, flat: bool) -> vec3<f32> {
    if (flat) {
        let jitter = (u.x * 2.0 - 1.0) * half;
        return vec3<f32>(cos(jitter), sin(jitter), 0.0);
    }
    let cos_t = 1.0 - u.x * (1.0 - cos(half));
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let phi = u.y * TAU;
    return vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), -cos_t);
}

// Flipbook frame `index` of a `grid` sheet, as a uv offset and scale.
fn frame_rect(index: f32, grid: vec2<f32>) -> vec4<f32> {
    let frames = max(grid.x * grid.y, 1.0);
    let i = index - frames * floor(index / frames);
    let column = i - grid.x * floor(i / grid.x);
    let row = floor(i / grid.x);
    let scale = 1.0 / grid;
    return vec4<f32>(vec2<f32>(column, row) * scale, scale);
}
