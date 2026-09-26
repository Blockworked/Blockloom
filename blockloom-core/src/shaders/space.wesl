// What hangs over any sky but a flat one: a procedural star field and
// curtains of aurora, in nits. The sky's background pass adds them behind
// the world. `SpaceParams` is `space::SpaceParams` in the runtime, field
// for field. Directions are world space, Y up, -Z north.

import blockloom::hash::{pcg3, unit};
import blockloom::noise::{gradient2, value2};
import blockloom::fbm::fbm2;

const TAU: f32 = 6.28318530718;

struct SpaceParams {
    // x density 0-1, y nits of the brightest star, z magnitude slope,
    // w color variation.
    stars: vec4<f32>,
    // x twinkle 0-1, y twinkles a second, z sine of the horizon fade,
    // w how visible the stars are (the sun's ramp).
    twinkle: vec4<f32>,
    // x Milky Way nits (0 for none), y its turn, z its tilt, both radians.
    milky_way: vec4<f32>,
    // x KP over 9, y foot altitude km, z height km, w fold width km.
    aurora: vec4<f32>,
    // x ray scale km, y nits, z flow speed, w horizon glow.
    aurora_look: vec4<f32>,
    // rgb the curtains' foot color, w layers.
    aurora_bottom: vec4<f32>,
    // rgb the top color, w how visible the aurora is (night).
    aurora_top: vec4<f32>,
    // x seconds, y pole azimuth radians, z unused, w lightning flash 0-1.
    time: vec4<f32>,
    // rgb the flash's nits over the whole sky.
    flash: vec4<f32>,
}

// Two layers of cells; a pixel checks the one cell it falls in per layer.
fn star_layer(dir: vec3<f32>, scale: f32, salt: u32, space: SpaceParams, pixel: f32) -> vec3<f32> {
    let p = dir * scale;
    let cell = floor(p);
    let h = pcg3(bitcast<vec3<u32>>(vec3<i32>(cell)) + vec3<u32>(salt, salt * 7u, salt * 13u));
    if unit(h.x) > space.stars.x {
        return vec3<f32>(0.0);
    }
    // Where in the cell the star sits, kept off the walls so its glow isn't
    // cut by the next cell.
    let offset = vec3<f32>(unit(h.y), unit(h.z), unit(h.x ^ h.y)) * 0.6 + 0.2;
    let star = normalize(cell + offset);
    let distance = length(dir - star);
    // One pixel of Gaussian, whatever the resolution.
    let sigma = max(pixel, 1e-5);
    let glow = exp(-0.5 * distance * distance / (sigma * sigma));
    if glow < 1e-3 {
        return vec3<f32>(0.0);
    }
    let magnitude = pow(unit(h.y ^ h.z), space.stars.z);
    let heat = (unit(h.z ^ 0x9e3779b9u) - 0.5) * space.stars.w;
    let color = normalize(vec3<f32>(1.0 + heat * 0.6, 1.0, 1.0 - heat * 0.8)) * 1.7320508;
    let speed = space.twinkle.y;
    let flicker = value2(vec2<f32>(space.time.x * speed, f32(h.x & 1023u)));
    // Twinkles harder low in the sky, through more air.
    let amount = space.twinkle.x * (1.0 - 0.5 * clamp(dir.y, 0.0, 1.0));
    let twinkle = 1.0 - amount + amount * 2.0 * flicker;
    return color * magnitude * twinkle * glow;
}

fn stars(dir: vec3<f32>, space: SpaceParams, pixel: f32) -> vec3<f32> {
    let visible = space.twinkle.w * smoothstep(0.0, max(space.twinkle.z, 1e-4), dir.y);
    if visible <= 0.0 || space.stars.x <= 0.0 {
        return vec3<f32>(0.0);
    }
    let field = star_layer(dir, 180.0, 1u, space, pixel)
        + star_layer(dir, 420.0, 2u, space, pixel) * 0.5;
    return field * space.stars.y * visible;
}

// The Milky Way panorama's uv for a direction, turned and tilted.
fn milky_way_uv(dir: vec3<f32>, space: SpaceParams) -> vec2<f32> {
    let turn = space.milky_way.y;
    let tilt = space.milky_way.z;
    let a = vec3<f32>(dir.x * cos(turn) - dir.z * sin(turn), dir.y, dir.x * sin(turn) + dir.z * cos(turn));
    let b = vec3<f32>(a.x, a.y * cos(tilt) - a.z * sin(tilt), a.y * sin(tilt) + a.z * cos(tilt));
    return vec2<f32>(atan2(b.x, -b.z) / TAU + 0.5, acos(clamp(b.y, -1.0, 1.0)) / 3.14159265);
}

// How far poleward a point `flat` km from the camera is, against how far
// the KP pushes the oval towards the camera: 0 outside, 1 well inside.
fn oval(flat: vec2<f32>, space: SpaceParams) -> f32 {
    let pole = vec2<f32>(sin(space.time.y), -cos(space.time.y));
    let poleward = dot(flat, pole);
    // KP 9 reaches 1500 km past the camera; KP 1 stays 1200 km poleward.
    let edge = mix(1200.0, -1500.0, space.aurora.x);
    return smoothstep(edge, edge + 400.0, poleward);
}

// Curtains of light between the foot and top altitudes, marched through
// each layer. Flat ground: the camera is at 0 km.
fn aurora(dir: vec3<f32>, space: SpaceParams) -> vec3<f32> {
    let visible = space.aurora_top.w;
    if visible <= 0.0 || space.aurora.x <= 0.0 || dir.y <= 0.01 {
        return vec3<f32>(0.0);
    }
    let foot = space.aurora.y;
    let height = space.aurora.z;
    let width = space.aurora.w;
    let flow = space.time.x * space.aurora_look.z * 0.02;
    let layers = u32(space.aurora_bottom.w);
    var light = vec3<f32>(0.0);
    const STEPS: u32 = 12u;
    for (var layer = 0u; layer < layers; layer++) {
        let base = foot + f32(layer) * height * 0.12;
        let t0 = base / dir.y;
        let t1 = (base + height) / dir.y;
        let dt = (t1 - t0) / f32(STEPS);
        let shift = vec2<f32>(f32(layer) * 37.0, f32(layer) * 19.0);
        for (var i = 0u; i < STEPS; i++) {
            let t = t0 + (f32(i) + 0.5) * dt;
            let p = dir * t;
            let flat = p.xz;
            let cover = oval(flat, space);
            if cover <= 0.0 {
                continue;
            }
            // A folded line: where a slow warp crosses zero.
            let q = flat / width + shift;
            let warp = vec2<f32>(fbm2(q * 0.35 + flow, 3u, 2.0, 0.5), fbm2(q * 0.35 - flow + 5.2, 3u, 2.0, 0.5));
            let line = gradient2(q + warp * 1.5 + vec2<f32>(flow * 3.0, 0.0));
            let sheet = exp(-line * line * 60.0);
            // Fine vertical rays along the sheet.
            let rays = 0.4 + 0.6 * value2(flat / space.aurora_look.x + vec2<f32>(flow * 20.0, 0.0));
            let h = (p.y - base) / height;
            let profile = smoothstep(0.0, 0.06, h) * exp(-h * 3.0);
            let color = mix(space.aurora_bottom.rgb, space.aurora_top.rgb, smoothstep(0.15, 0.8, h));
            light += color * sheet * rays * profile * cover * (dt / height);
        }
    }
    light *= space.aurora_look.y * 4.0;
    // The diffuse glow along the poleward horizon.
    let pole = vec2<f32>(sin(space.time.y), -cos(space.time.y));
    let facing = max(dot(normalize(dir.xz + vec2<f32>(1e-5)), pole), 0.0);
    let glow = space.aurora_look.w * facing * exp(-dir.y * 10.0) * space.aurora.x;
    light += space.aurora_bottom.rgb * glow * space.aurora_look.y;
    return light * visible;
}
