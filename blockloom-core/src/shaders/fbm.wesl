// Fractal sums of gradient noise. `octaves` is capped at 16; each octave
// scales frequency by `lacunarity` and amplitude by `gain` (2 and 0.5 are
// the classic values).

import blockloom::noise::{gradient2, gradient3};

const MAX_OCTAVES: u32 = 16u;

// Rotating each octave keeps lattice artifacts from lining up.
const ROTATE2: mat2x2<f32> = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
const ROTATE3: mat3x3<f32> = mat3x3<f32>(
    0.00, 0.80, 0.60,
    -0.80, 0.36, -0.48,
    -0.60, -0.48, 0.64,
);

// Roughly in [-1, 1].
fn fbm2(p: vec2<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        sum += gradient2(q) * amplitude;
        total += amplitude;
        q = ROTATE2 * q * lacunarity + vec2<f32>(17.0, 31.0);
        amplitude *= gain;
    }
    return sum / max(total, 1e-6);
}

fn fbm3(p: vec3<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        sum += gradient3(q) * amplitude;
        total += amplitude;
        q = ROTATE3 * q * lacunarity + vec3<f32>(17.0, 31.0, 47.0);
        amplitude *= gain;
    }
    return sum / max(total, 1e-6);
}

// Sum of |noise|, in [0, 1]: billowy, for clouds and smoke.
fn turbulence2(p: vec2<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        sum += abs(gradient2(q)) * amplitude;
        total += amplitude;
        q = ROTATE2 * q * lacunarity + vec2<f32>(17.0, 31.0);
        amplitude *= gain;
    }
    return clamp(sum / max(total, 1e-6), 0.0, 1.0);
}

fn turbulence3(p: vec3<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        sum += abs(gradient3(q)) * amplitude;
        total += amplitude;
        q = ROTATE3 * q * lacunarity + vec3<f32>(17.0, 31.0, 47.0);
        amplitude *= gain;
    }
    return clamp(sum / max(total, 1e-6), 0.0, 1.0);
}

// Inverted |noise|, squared, in [0, 1]: sharp crests, for mountains and veins.
fn ridged2(p: vec2<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        let ridge = 1.0 - abs(gradient2(q));
        sum += ridge * ridge * amplitude;
        total += amplitude;
        q = ROTATE2 * q * lacunarity + vec2<f32>(17.0, 31.0);
        amplitude *= gain;
    }
    return clamp(sum / max(total, 1e-6), 0.0, 1.0);
}

fn ridged3(p: vec3<f32>, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    var q = p;
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0u; i < min(octaves, MAX_OCTAVES); i++) {
        let ridge = 1.0 - abs(gradient3(q));
        sum += ridge * ridge * amplitude;
        total += amplitude;
        q = ROTATE3 * q * lacunarity + vec3<f32>(17.0, 31.0, 47.0);
        amplitude *= gain;
    }
    return clamp(sum / max(total, 1e-6), 0.0, 1.0);
}
