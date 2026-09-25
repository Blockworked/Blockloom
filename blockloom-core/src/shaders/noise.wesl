// Lattice noise: value, gradient (Perlin) and cellular (Worley), in 2D and 3D.

import blockloom::hash::{hash_cell2, hash_cell3};

const TAU: f32 = 6.28318530717959;

// Quintic fade, so the second derivative is continuous across cells.
fn fade2(t: vec2<f32>) -> vec2<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn fade3(t: vec3<f32>) -> vec3<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// Value noise in [0, 1].
fn value2(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fade2(fract(p));
    let a = hash_cell2(cell).x;
    let b = hash_cell2(cell + vec2<i32>(1, 0)).x;
    let c = hash_cell2(cell + vec2<i32>(0, 1)).x;
    let d = hash_cell2(cell + vec2<i32>(1, 1)).x;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn value3(p: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(p));
    let f = fade3(fract(p));
    let a = hash_cell3(cell).x;
    let b = hash_cell3(cell + vec3<i32>(1, 0, 0)).x;
    let c = hash_cell3(cell + vec3<i32>(0, 1, 0)).x;
    let d = hash_cell3(cell + vec3<i32>(1, 1, 0)).x;
    let e = hash_cell3(cell + vec3<i32>(0, 0, 1)).x;
    let g = hash_cell3(cell + vec3<i32>(1, 0, 1)).x;
    let h = hash_cell3(cell + vec3<i32>(0, 1, 1)).x;
    let k = hash_cell3(cell + vec3<i32>(1, 1, 1)).x;
    let near = mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    let far = mix(mix(e, g, f.x), mix(h, k, f.x), f.y);
    return mix(near, far, f.z);
}

fn gradient_of2(cell: vec2<i32>) -> vec2<f32> {
    let angle = hash_cell2(cell).x * TAU;
    return vec2<f32>(cos(angle), sin(angle));
}

// A uniformly distributed unit vector.
fn gradient_of3(cell: vec3<i32>) -> vec3<f32> {
    let h = hash_cell3(cell);
    let z = h.x * 2.0 - 1.0;
    let angle = h.y * TAU;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(angle), r * sin(angle), z);
}

// Gradient noise, roughly in [-1, 1] and 0 at every lattice point.
fn gradient2(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let t = fract(p);
    let f = fade2(t);
    let a = dot(gradient_of2(cell), t);
    let b = dot(gradient_of2(cell + vec2<i32>(1, 0)), t - vec2<f32>(1.0, 0.0));
    let c = dot(gradient_of2(cell + vec2<i32>(0, 1)), t - vec2<f32>(0.0, 1.0));
    let d = dot(gradient_of2(cell + vec2<i32>(1, 1)), t - vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y) * 1.41421356;
}

fn gradient3(p: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(p));
    let t = fract(p);
    let f = fade3(t);
    var corners: array<f32, 8>;
    for (var i = 0u; i < 8u; i++) {
        let o = vec3<u32>(i & 1u, (i >> 1u) & 1u, (i >> 2u) & 1u);
        corners[i] = dot(gradient_of3(cell + vec3<i32>(o)), t - vec3<f32>(o));
    }
    let near = mix(mix(corners[0], corners[1], f.x), mix(corners[2], corners[3], f.x), f.y);
    let far = mix(mix(corners[4], corners[5], f.x), mix(corners[6], corners[7], f.x), f.y);
    return mix(near, far, f.z) * 1.15470054;
}

// Cellular noise: distances to the nearest (x) and second nearest (y)
// feature point, one point per unit cell.
fn worley2(p: vec2<f32>) -> vec2<f32> {
    let cell = vec2<i32>(floor(p));
    let t = fract(p);
    var nearest = vec2<f32>(8.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<i32>(x, y);
            let d = length(vec2<f32>(o) + hash_cell2(cell + o) - t);
            if (d < nearest.x) {
                nearest = vec2<f32>(d, nearest.x);
            } else if (d < nearest.y) {
                nearest.y = d;
            }
        }
    }
    return nearest;
}

fn worley3(p: vec3<f32>) -> vec2<f32> {
    let cell = vec3<i32>(floor(p));
    let t = fract(p);
    var nearest = vec2<f32>(8.0);
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                let d = length(vec3<f32>(o) + hash_cell3(cell + o) - t);
                if (d < nearest.x) {
                    nearest = vec2<f32>(d, nearest.x);
                } else if (d < nearest.y) {
                    nearest.y = d;
                }
            }
        }
    }
    return nearest;
}
