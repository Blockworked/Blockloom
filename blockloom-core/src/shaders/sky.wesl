// The sky's radiance in any direction, in nits: a physical atmosphere, a
// gradient, and the sun and moon disks. The runtime's cube pass writes the
// sky's light with it and the background pass draws with it, so the two
// never disagree. `SkyParams` is `sky::SkyParams` in the runtime, field for
// field. Directions are world space, Y up.

import blockloom::scattering::{rayleigh_phase, cornette_shanks};

const PI: f32 = 3.14159265358979;

// Cubes store radiance over this, so a bright sky stays inside FP16.
const SKY_UNIT: f32 = 1000.0;

const KIND_PHYSICAL: u32 = 1u;
const KIND_GRADIENT: u32 = 2u;
const KIND_HDRI: u32 = 3u;

const FLAG_DISKS: u32 = 1u;
const FLAG_MOON: u32 = 2u;
const FLAG_DITHER: u32 = 4u;

// Mie extinction over scattering; `PhysicalSky::MIE_EXTINCTION`.
const MIE_EXTINCTION: f32 = 1.1111111;

struct SkyParams {
    // x kind, y flags, z the light cube's face size, w the HDRI's mip count.
    info: vec4<u32>,
    // rgb: sunlight above the air, lux per channel.
    sun: vec4<f32>,
    // xyz: towards the sun. w: sine of the disk's angular radius.
    sun_dir: vec4<f32>,
    // rgb: disk luminance at its centre through the air, nits. w: limb darkening.
    sun_disk: vec4<f32>,
    // xyz: towards the moon. w: sine of the disk's angular radius.
    moon_dir: vec4<f32>,
    // x nits, y phase, z halo fraction, w halo power.
    moon: vec4<f32>,
    // rgb 1/Mm, w scale height km.
    rayleigh: vec4<f32>,
    // x scattering 1/Mm, y g, z scale height km, w horizon curve.
    mie: vec4<f32>,
    // rgb 1/Mm.
    ozone: vec4<f32>,
    // x planet radius, y atmosphere top radius, z eye height (km), w horizon sine.
    planet: vec4<f32>,
    // rgb linear ground albedo.
    ground: vec4<f32>,
    // rgb nits, already ramped on the sun.
    night: vec4<f32>,
    // Gradient stops, rgb nits.
    top: vec4<f32>,
    middle: vec4<f32>,
    bottom: vec4<f32>,
    // rgb nits, w how much of it shows (warmth ramped on the sun).
    warm: vec4<f32>,
    // x horizon offset, y softness.
    horizon: vec4<f32>,
    // World to image rotation for the HDRI, as columns.
    hdri_x: vec4<f32>,
    hdri_y: vec4<f32>,
    hdri_z: vec4<f32>,
    // rgb tint times brightness, w background blur as a mip level.
    hdri: vec4<f32>,
    // x 2^exposure bias, y ambient dimmer, z background blur on, w unused.
    scale: vec4<f32>,
}

// The world direction a cube texel looks along: the standard face mapping,
// then z negated into Bevy's world, as `HdrCube::direction` does.
fn face_direction(face: u32, texel: vec2<u32>, size: u32) -> vec3<f32> {
    let st = 2.0 * (vec2<f32>(texel) + 0.5) / f32(size) - 1.0;
    let s = st.x;
    let t = st.y;
    var d: vec3<f32>;
    switch face {
        case 0u: { d = vec3<f32>(1.0, -t, -s); }
        case 1u: { d = vec3<f32>(-1.0, -t, s); }
        case 2u: { d = vec3<f32>(s, 1.0, t); }
        case 3u: { d = vec3<f32>(s, -1.0, -t); }
        case 4u: { d = vec3<f32>(s, -t, 1.0); }
        default: { d = vec3<f32>(-s, -t, -1.0); }
    }
    return normalize(d) * vec3<f32>(1.0, 1.0, -1.0);
}

// What to sample a cube with to read `dir`.
fn cube_lookup(dir: vec3<f32>) -> vec3<f32> {
    return dir * vec3<f32>(1.0, 1.0, -1.0);
}

fn hdri_lookup(dir: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    let rotation = mat3x3<f32>(sky.hdri_x.xyz, sky.hdri_y.xyz, sky.hdri_z.xyz);
    return cube_lookup(rotation * dir);
}

// Distance along `dir` from `origin` to where it leaves a sphere of
// `radius` about the planet's centre, or to where it enters one, or -1.
fn sphere_exit(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> f32 {
    let b = dot(origin, dir);
    let c = dot(origin, origin) - radius * radius;
    let d = b * b - c;
    if d < 0.0 {
        return -1.0;
    }
    return -b + sqrt(d);
}

fn sphere_entry(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> f32 {
    let b = dot(origin, dir);
    let c = dot(origin, origin) - radius * radius;
    let d = b * b - c;
    if d < 0.0 || c < 0.0 {
        return -1.0;
    }
    let t = -b - sqrt(d);
    return select(-1.0, t, t > 0.0);
}

fn ozone_density(height: f32) -> f32 {
    return max(1.0 - abs(height - 25.0) / 15.0, 0.0);
}

// Rayleigh, Mie and ozone density at a point, relative to sea level.
fn densities(p: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    let h = max(length(p) - sky.planet.x, 0.0);
    return vec3<f32>(
        exp(-h / sky.rayleigh.w),
        exp(-h / sky.mie.z),
        ozone_density(h),
    );
}

// Optical depth (in 1/Mm times km) to extinction per channel.
fn extinction(depth: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    return (sky.rayleigh.rgb * depth.x + vec3<f32>(sky.mie.x * MIE_EXTINCTION * depth.y)
        + sky.ozone.rgb * depth.z) * 1.0e-3;
}

// Densities integrated from `p` towards the sun, or -1 in x where the
// planet is in the way.
fn depth_to_sun(p: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    let sun = sky.sun_dir.xyz;
    if sphere_entry(p, sun, sky.planet.x) > 0.0 {
        return vec3<f32>(-1.0);
    }
    let span = max(sphere_exit(p, sun, sky.planet.y), 0.0);
    let steps = 6;
    let ds = span / f32(steps);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        depth += densities(p + sun * ((f32(i) + 0.5) * ds), sky) * ds;
    }
    return depth;
}

// Bends a direction for the horizon curve: below 1 lifts the horizon's
// color up the sky, above 1 pulls it down.
fn curved(dir: vec3<f32>, curve: f32) -> vec3<f32> {
    if dir.y <= 0.0 || curve == 1.0 {
        return dir;
    }
    let y = pow(dir.y, curve);
    let flat = dir.xz / max(length(dir.xz), 1e-6);
    return vec3<f32>(flat.x * sqrt(1.0 - y * y), y, flat.y * sqrt(1.0 - y * y));
}

// Single scattering of sunlight, plus the ground under the horizon and
// the night sky. Nits, before the sky's exposure bias.
fn physical_radiance(view_dir: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    let dir = curved(view_dir, sky.mie.w);
    let origin = vec3<f32>(0.0, sky.planet.x + sky.planet.z, 0.0);
    let ground = sphere_entry(origin, dir, sky.planet.x);
    let span = select(max(sphere_exit(origin, dir, sky.planet.y), 0.0), ground, ground > 0.0);
    let steps = 16;
    let ds = span / f32(steps);
    var view_depth = vec3<f32>(0.0);
    var rayleigh = vec3<f32>(0.0);
    var mie = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let p = origin + dir * ((f32(i) + 0.5) * ds);
        let density = densities(p, sky);
        view_depth += density * ds * 0.5;
        let to_sun = depth_to_sun(p, sky);
        if to_sun.x >= 0.0 {
            let through = exp(-extinction(view_depth + to_sun, sky));
            rayleigh += density.x * through * ds;
            mie += density.y * through * ds;
        }
        view_depth += density * ds * 0.5;
    }
    let mu = dot(dir, sky.sun_dir.xyz);
    var radiance = sky.sun.rgb * 1.0e-3 * (sky.rayleigh.rgb * rayleigh * rayleigh_phase(mu)
        + sky.mie.x * mie * cornette_shanks(mu, sky.mie.y));
    if ground > 0.0 {
        // Lambertian ground lit by the sun through the air, seen through it.
        let hit = origin + dir * ground;
        let up = normalize(hit);
        let to_sun = depth_to_sun(hit, sky);
        if to_sun.x >= 0.0 {
            let lit = sky.sun.rgb * exp(-extinction(to_sun, sky)) * max(dot(up, sky.sun_dir.xyz), 0.0);
            radiance += exp(-extinction(view_depth, sky)) * sky.ground.rgb / PI * lit;
        }
    }
    // Night glows from the whole upper sky, faintly under the horizon.
    let night_shape = select(0.25, 1.0, view_dir.y > sky.planet.w);
    return radiance + sky.night.rgb * night_shape;
}

// The three stops around a horizon line, warming towards a low sun. Nits.
fn gradient_radiance(dir: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    let h = dir.y - sky.horizon.x;
    let soft = max(sky.horizon.y, 1e-3);
    let up = mix(sky.middle.rgb, sky.top.rgb, smoothstep(0.0, soft, h));
    let down = mix(sky.middle.rgb, sky.bottom.rgb, smoothstep(0.0, soft, -h));
    var color = select(down, up, h >= 0.0);
    if sky.warm.w > 0.0 {
        let band = exp(-abs(h) / (soft * 0.35));
        let flat_dir = dir.xz / max(length(dir.xz), 1e-6);
        let flat_sun = sky.sun_dir.xz / max(length(sky.sun_dir.xz), 1e-6);
        let facing = 0.35 + 0.65 * (0.5 + 0.5 * dot(flat_dir, flat_sun));
        color = mix(color, sky.warm.rgb, clamp(band * facing * sky.warm.w, 0.0, 1.0));
    }
    return color;
}

// Where `dir` falls on a disk of angular radius asin(`radius_sine`) about
// `centre`: the offset in disk radii, or a length over 1 outside it.
fn disk_offset(dir: vec3<f32>, centre: vec3<f32>, radius_sine: f32) -> vec2<f32> {
    var side = cross(centre, vec3<f32>(0.0, 1.0, 0.0));
    if dot(side, side) < 1e-8 {
        side = vec3<f32>(1.0, 0.0, 0.0);
    }
    side = normalize(side);
    let rise = cross(side, centre);
    if dot(dir, centre) <= 0.0 {
        return vec2<f32>(2.0);
    }
    return vec2<f32>(dot(dir, side), dot(dir, rise)) / radius_sine;
}

// Soft edge a pixel wide, from how fast `r` changes across the screen.
fn disk_edge(r: f32, width: f32) -> f32 {
    return 1.0 - smoothstep(1.0 - width, 1.0 + width, r);
}

// The sun's and moon's disks and the moon's halo, nits, nothing below the
// horizon. `pixel` is roughly how many disk radii one pixel spans.
fn disks(dir: vec3<f32>, sky: SkyParams, sun_pixel: f32, moon_pixel: f32) -> vec3<f32> {
    if dir.y < sky.planet.w {
        return vec3<f32>(0.0);
    }
    var out = vec3<f32>(0.0);
    let q = disk_offset(dir, sky.sun_dir.xyz, sky.sun_dir.w);
    let r = length(q);
    if r < 1.0 + sun_pixel {
        let mu = sqrt(max(1.0 - min(r * r, 1.0), 0.0));
        let limb = 1.0 - sky.sun_disk.w * (1.0 - mu);
        out += sky.sun_disk.rgb * limb * disk_edge(r, sun_pixel);
    }
    if (sky.info.y & FLAG_MOON) != 0u {
        let centre = sky.moon_dir.xyz;
        let m = disk_offset(dir, centre, sky.moon_dir.w);
        let rm = length(m);
        let phase = sky.moon.y * 2.0 * PI;
        // Lit from the sun's side of the screen, by as much as the phase says.
        let side = normalize(cross(centre, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-6, 0.0, 0.0));
        let rise = cross(side, centre);
        var towards = vec2<f32>(dot(sky.sun_dir.xyz, side), dot(sky.sun_dir.xyz, rise));
        towards = select(vec2<f32>(1.0, 0.0), normalize(towards), dot(towards, towards) > 1e-8);
        let light = vec3<f32>(towards * sin(phase), -cos(phase));
        let lit_fraction = 0.5 - 0.5 * cos(phase);
        let edge = disk_edge(rm, moon_pixel);
        if rm < 1.0 + moon_pixel {
            let normal = vec3<f32>(m, sqrt(max(1.0 - min(rm * rm, 1.0), 0.0)));
            let lit = smoothstep(-0.05, 0.1, dot(normal, light));
            out += vec3<f32>(sky.moon.x * (lit + 0.01)) * edge;
        }
        if sky.moon.z > 0.0 {
            // The glow is behind the disk, so it shows through its soft rim.
            let halo = pow(max(dot(dir, centre), 0.0), sky.moon.w);
            out += vec3<f32>(sky.moon.x * sky.moon.z * lit_fraction * halo) * (1.0 - edge);
        }
    }
    return out;
}

// The sky's own radiance, nits times its exposure bias, without disks:
// what lights the world. The HDRI kind reads its image instead.
fn sky_radiance(dir: vec3<f32>, sky: SkyParams) -> vec3<f32> {
    switch sky.info.x {
        case KIND_PHYSICAL: { return physical_radiance(dir, sky) * sky.scale.x; }
        case KIND_GRADIENT: { return gradient_radiance(dir, sky) * sky.scale.x; }
        default: { return vec3<f32>(0.0); }
    }
}
