// Integer hashes (PCG, Jarzynski & Olano 2020) and floats in [0, 1) made
// from them. Bit-exact on every GPU, unlike the `fract(sin(...))` trick.

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn pcg2(v: vec2<u32>) -> vec2<u32> {
    var h = v * 1664525u + 1013904223u;
    h.x += h.y * 1664525u;
    h.y += h.x * 1664525u;
    h = h ^ (h >> vec2<u32>(16u));
    h.x += h.y * 1664525u;
    h.y += h.x * 1664525u;
    return h ^ (h >> vec2<u32>(16u));
}

fn pcg3(v: vec3<u32>) -> vec3<u32> {
    var h = v * 1664525u + 1013904223u;
    h.x += h.y * h.z;
    h.y += h.z * h.x;
    h.z += h.x * h.y;
    h = h ^ (h >> vec3<u32>(16u));
    h.x += h.y * h.z;
    h.y += h.z * h.x;
    h.z += h.x * h.y;
    return h;
}

// The top 24 bits as a float in [0, 1).
fn unit(x: u32) -> f32 {
    return f32(x >> 8u) * (1.0 / 16777216.0);
}

// Hashes of a lattice cell, for noise.
fn hash_cell2(cell: vec2<i32>) -> vec2<f32> {
    let h = pcg2(bitcast<vec2<u32>>(cell));
    return vec2<f32>(unit(h.x), unit(h.y));
}

fn hash_cell3(cell: vec3<i32>) -> vec3<f32> {
    let h = pcg3(bitcast<vec3<u32>>(cell));
    return vec3<f32>(unit(h.x), unit(h.y), unit(h.z));
}

// White noise over continuous inputs: the bits of the float are the key.
fn hash11(p: f32) -> f32 {
    return unit(pcg(bitcast<u32>(p)));
}

fn hash21(p: vec2<f32>) -> f32 {
    return unit(pcg2(bitcast<vec2<u32>>(p)).x);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let h = pcg2(bitcast<vec2<u32>>(p));
    return vec2<f32>(unit(h.x), unit(h.y));
}

fn hash31(p: vec3<f32>) -> f32 {
    return unit(pcg3(bitcast<vec3<u32>>(p)).x);
}

fn hash33(p: vec3<f32>) -> vec3<f32> {
    let h = pcg3(bitcast<vec3<u32>>(p));
    return vec3<f32>(unit(h.x), unit(h.y), unit(h.z));
}
