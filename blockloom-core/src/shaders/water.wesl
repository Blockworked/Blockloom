// Gerstner waves, Phillips detail and simulated ripples: the GPU half of
// `blockloom_core::water`. `displace` and `frame` sum exactly what
// `WaterBody::displace` and `WaterBody::sample` sum, so a floating body sits
// on the surface that is drawn. Change the two together.

const MAX_WAVES: u32 = 12u;
const DETAIL_WAVES: u32 = 16u;
const TAU: f32 = 6.28318530718;

struct WaterWaves {
    // Per wave: direction xz, wavenumber, angular speed.
    shape: array<vec4<f32>, 12>,
    // Per wave: phase, vertical amplitude, horizontal amplitude, 0.
    size: array<vec4<f32>, 12>,
    // Per detail wave: direction xz, wavenumber, angular speed.
    detail_shape: array<vec4<f32>, 16>,
    // Per detail wave: phase, slope, 0, 0.
    detail_size: array<vec4<f32>, 16>,
    // Wave count, 1 with a ripple field, water time, 1 in 2D.
    counts: vec4<f32>,
    // Current xz, surface level at rest, 0.
    flow: vec4<f32>,
    // The ripple field: world xz of its first cell, world units a cell,
    // how far between its last two ticks to draw.
    field: vec4<f32>,
}

fn is_flat(w: WaterWaves) -> bool {
    return w.counts.w > 0.5;
}

fn planar(w: WaterWaves, v: vec2<f32>) -> vec2<f32> {
    return select(v, vec2<f32>(v.x, 0.0), is_flat(w));
}

fn theta(w: WaterWaves, i: u32, q: vec2<f32>, t: f32) -> f32 {
    let s = w.shape[i];
    let drift = w.flow.xy * t;
    return s.z * dot(s.xy, q - drift) - s.w * t + w.size[i].x;
}

// Where the rest point `q` moves to: horizontal offset in xz and height
// above the level, packed as (dx, height, dz).
fn displace(w: WaterWaves, q_in: vec2<f32>) -> vec3<f32> {
    let q = planar(w, q_in);
    let t = w.counts.z;
    var offset = vec2<f32>(0.0);
    var height = 0.0;
    let count = min(u32(w.counts.x + 0.5), MAX_WAVES);
    for (var i = 0u; i < count; i++) {
        let th = theta(w, i, q, t);
        offset += planar(w, w.shape[i].xy) * w.size[i].z * cos(th);
        height += w.size[i].y * sin(th);
    }
    return vec3<f32>(offset.x, height, offset.y);
}

struct WaterFrame {
    normal: vec3<f32>,
    // Below 1 where a crest is being pinched: where foam starts.
    jacobian: f32,
}

// The Gerstner surface's normal and pinch at the rest point `q`.
fn frame(w: WaterWaves, q_in: vec2<f32>) -> WaterFrame {
    let q = planar(w, q_in);
    let t = w.counts.z;
    var xx = 0.0;
    var zz = 0.0;
    var xz = 0.0;
    var sx = 0.0;
    var sz = 0.0;
    let count = min(u32(w.counts.x + 0.5), MAX_WAVES);
    for (var i = 0u; i < count; i++) {
        let d = planar(w, w.shape[i].xy);
        let th = theta(w, i, q, t);
        let s = sin(th);
        let c = cos(th);
        let wa = w.shape[i].z * w.size[i].y;
        let wh = w.shape[i].z * w.size[i].z;
        sx += d.x * wa * c;
        sz += d.y * wa * c;
        xx += d.x * d.x * wh * s;
        zz += d.y * d.y * wh * s;
        xz += d.x * d.y * wh * s;
    }
    var out: WaterFrame;
    if (is_flat(w)) {
        out.normal = normalize(vec3<f32>(-sx, 1.0 - xx, 0.0));
        out.jacobian = 1.0 - xx;
        return out;
    }
    let tx = vec3<f32>(1.0 - xx, sx, -xz);
    let tz = vec3<f32>(-xz, sz, 1.0 - zz);
    out.normal = normalize(cross(tz, tx));
    out.jacobian = (1.0 - xx) * (1.0 - zz) - xz * xz;
    return out;
}

// The detail waves' slope (dh/dx, dh/dz) at `q`. Normals only.
fn detail_slope(w: WaterWaves, q_in: vec2<f32>) -> vec2<f32> {
    let q = planar(w, q_in);
    let t = w.counts.z;
    let drift = w.flow.xy * t;
    var slope = vec2<f32>(0.0);
    for (var i = 0u; i < DETAIL_WAVES; i++) {
        let s = w.detail_shape[i];
        let th = s.z * dot(s.xy, q - drift) - s.w * t + w.detail_size[i].x;
        slope += planar(w, s.xy) * w.detail_size[i].y * cos(th);
    }
    return slope;
}

// One cell of the ripple field, lerped between its last two ticks. Outside
// the field the water is still.
fn ripple_cell(field: texture_2d<f32>, w: WaterWaves, i: i32, j: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(field));
    if (i < 0 || j < 0 || i >= size.x || j >= size.y) {
        return 0.0;
    }
    let h = textureLoad(field, vec2<i32>(i, j), 0).rg;
    return mix(h.g, h.r, w.field.w);
}

fn ripple_height(field: texture_2d<f32>, w: WaterWaves, c: vec2<f32>) -> f32 {
    let base = floor(c);
    let f = c - base;
    let i = i32(base.x);
    let j = i32(base.y);
    let a = mix(ripple_cell(field, w, i, j), ripple_cell(field, w, i + 1, j), f.x);
    let b = mix(ripple_cell(field, w, i, j + 1), ripple_cell(field, w, i + 1, j + 1), f.x);
    return mix(a, b, f.y);
}

// The simulated ripples over world `p`, as (dh/dx, height, dh/dz). The same
// bilinear lookup `RippleField::sample` makes.
fn ripple(field: texture_2d<f32>, w: WaterWaves, p_in: vec2<f32>) -> vec3<f32> {
    if (w.counts.y < 0.5) {
        return vec3<f32>(0.0);
    }
    let p = planar(w, p_in);
    let cell = max(w.field.z, 1e-4);
    var c = (p - w.field.xy) / cell;
    if (is_flat(w)) {
        c.y = 0.0;
    }
    let h = ripple_height(field, w, c);
    let dx = (ripple_height(field, w, c + vec2<f32>(0.5, 0.0)) - ripple_height(field, w, c - vec2<f32>(0.5, 0.0))) / cell;
    var dz = 0.0;
    if (!is_flat(w)) {
        dz = (ripple_height(field, w, c + vec2<f32>(0.0, 0.5)) - ripple_height(field, w, c - vec2<f32>(0.0, 0.5))) / cell;
    }
    return vec3<f32>(dx, h, dz);
}

// Schlick's Fresnel for water (F0 0.02).
fn fresnel(cos_theta: f32) -> f32 {
    let m = clamp(1.0 - cos_theta, 0.0, 1.0);
    let m2 = m * m;
    return 0.02 + 0.98 * m2 * m2 * m;
}

// A sharp, drifting web of caustic light, about 0 to 2: two layers of
// Worley edges sliding past each other.
fn caustics(p: vec2<f32>, t: f32) -> f32 {
    let a = worley_edge(p + vec2<f32>(t * 0.13, t * 0.07));
    let b = worley_edge(p * 1.37 - vec2<f32>(t * 0.09, -t * 0.11));
    return pow(min(a, b), 3.0) * 2.0;
}

fn caustic_hash(cell: vec2<f32>) -> vec2<f32> {
    let h = vec2<f32>(dot(cell, vec2<f32>(127.1, 311.7)), dot(cell, vec2<f32>(269.5, 183.3)));
    return fract(sin(h) * 43758.5453);
}

// 1 on the edges between Worley cells, falling to 0 inside them.
fn worley_edge(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let local = fract(p);
    var first = 8.0;
    var second = 8.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y));
            let d = length(offset + caustic_hash(cell + offset) - local);
            if (d < first) {
                second = first;
                first = d;
            } else if (d < second) {
                second = d;
            }
        }
    }
    return 1.0 - clamp((second - first) * 2.5, 0.0, 1.0);
}
