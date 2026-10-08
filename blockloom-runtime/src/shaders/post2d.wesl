// 2D post: pixelation, outline, ordered dither with palette or level
// quantize, and a CRT preset, over the finished frame. The maths matches
// `blockloom_core::post2d`; change the two together. The texture holds linear
// values, the look works on display (sRGB) ones.

struct Post2d {
    // Pixel block size, levels, dither amount, split fraction.
    a: vec4<f32>,
    // Outline strength, threshold.
    b: vec4<f32>,
    // Outline colour (display values).
    c: vec4<f32>,
    // CRT scanlines, curvature, vignette, mask.
    d: vec4<f32>,
    // Palette size.
    e: vec4<f32>,
    palette: array<vec4<f32>, 32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: Post2d;

fn to_display(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn bayer4(x: u32, y: u32) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(y % 4u) * 4u + (x % 4u)] + 0.5) / 16.0 - 0.5;
}

fn load(p: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let q = clamp(p, vec2<f32>(0.0), size - vec2<f32>(1.0));
    return to_display(textureLoad(screen, vec2<i32>(q), 0).rgb);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(screen));
    let plain = textureLoad(screen, vec2<i32>(position.xy), 0);
    let split = settings.a.w;
    if split > 0.0 && split < 1.0 && position.x / size.x > split {
        return plain;
    }
    let block = max(settings.a.x, 1.0);

    // CRT glass: bow the picture and cut what falls outside it.
    var uv = position.xy / size;
    let bow = settings.d.y;
    if bow > 0.0 {
        let c = uv * 2.0 - 1.0;
        let k = bow * 0.25;
        let w = vec2<f32>(c.x * (1.0 + k * c.y * c.y), c.y * (1.0 + k * c.x * c.x));
        if abs(w.x) > 1.0 || abs(w.y) > 1.0 {
            return vec4<f32>(0.0, 0.0, 0.0, plain.a);
        }
        uv = w * 0.5 + 0.5;
    }

    let cell = floor(uv * size / block);
    let p = (cell + 0.5) * block;
    var color = load(p, size);

    // Edges: a luma step to any of the four neighbouring blocks.
    if settings.b.x > 0.0 {
        let here = luma(color);
        var edge = 0.0;
        edge = max(edge, abs(here - luma(load(p + vec2<f32>(block, 0.0), size))));
        edge = max(edge, abs(here - luma(load(p - vec2<f32>(block, 0.0), size))));
        edge = max(edge, abs(here - luma(load(p + vec2<f32>(0.0, block), size))));
        edge = max(edge, abs(here - luma(load(p - vec2<f32>(0.0, block), size))));
        if edge > settings.b.y {
            color = mix(color, settings.c.rgb, settings.b.x);
        }
    }

    // Dither, then a fixed palette or per-channel levels.
    let count = u32(settings.e.x + 0.5);
    let dither = settings.a.z;
    let nudge = bayer4(u32(cell.x), u32(cell.y)) * dither;
    if count > 0u {
        let probe = color + vec3<f32>(nudge * 0.25);
        var best = color;
        var best_d = 1e9;
        for (var i = 0u; i < count; i++) {
            let d = dot(settings.palette[i].rgb - probe, settings.palette[i].rgb - probe);
            if d < best_d {
                best_d = d;
                best = settings.palette[i].rgb;
            }
        }
        color = best;
    } else if settings.a.y > 1.0 {
        let steps = settings.a.y - 1.0;
        color = round(clamp(color + vec3<f32>(nudge / steps), vec3<f32>(0.0), vec3<f32>(1.0)) * steps) / steps;
    }

    // Scanlines, phosphor mask and vignette.
    let row = u32(floor(position.y / block));
    if settings.d.x > 0.0 && row % 2u == 1u {
        color *= 1.0 - settings.d.x * 0.6;
    }
    if settings.d.w > 0.0 {
        let phase = u32(position.x) % 3u;
        var mask = vec3<f32>(1.0 - settings.d.w * 0.5);
        mask[phase] = 1.0;
        color *= mask;
    }
    if settings.d.z > 0.0 {
        let c = uv * 2.0 - 1.0;
        color *= 1.0 - settings.d.z * clamp(dot(c, c) * 0.5, 0.0, 1.0);
    }
    return vec4<f32>(to_linear(color), plain.a);
}
