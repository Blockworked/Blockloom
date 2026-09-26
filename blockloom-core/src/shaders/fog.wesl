// The air: analytic height fog and aerial haze along a view ray, and the
// froxel grid's slicing. `FogUniforms` is `fog::FogUniforms` in the runtime,
// field for field. Directions are world space, Y up; distances metres.

const PI: f32 = 3.14159265358979;

const FLAG_HEIGHT: u32 = 1u;
const FLAG_VOLUMETRIC: u32 = 2u;
const FLAG_AERIAL: u32 = 4u;
const FLAG_SKY: u32 = 8u;
const FLAG_HISTORY: u32 = 16u;
const FLAG_SUN_VOLUMETRIC: u32 = 32u;

// Local fog volumes and lights the froxels read, at most.
const MAX_LOCAL_FOG: u32 = 16u;
const MAX_FOG_LIGHTS: u32 = 16u;

// A fog volume: a box or a ball that adds density inside, fading across
// its blend distance.
struct LocalFogVolume {
    // xyz centre, w shape: 0 box, 1 sphere.
    centre: vec4<f32>,
    // World to the volume's own frame, a quaternion.
    rotation: vec4<f32>,
    // Half size (x the radius for a sphere), w the blend distance.
    extent: vec4<f32>,
    // rgb albedo, w density per metre.
    albedo: vec4<f32>,
    // rgb emitted nits per unit density.
    emissive: vec4<f32>,
}

// A point or spot light the fog scatters, unshadowed.
struct FogLight {
    // xyz position, w range.
    position: vec4<f32>,
    // rgb candela (lumens over 4 pi times color).
    color: vec4<f32>,
    // xyz where a spot points, w cos of the outer angle (-2 for a point).
    direction: vec4<f32>,
    // x cos of the inner angle, y beam near fade, z beam far fade (metres).
    cone: vec4<f32>,
    // x beam density per metre (0 for none), y g, z falloff curve.
    beam: vec4<f32>,
}

struct FogUniforms {
    world_from_clip: mat4x4<f32>,
    // Last frame's, for reprojecting the froxels' history.
    previous_clip_from_world: mat4x4<f32>,
    // xyz where the camera stood last frame.
    previous_camera: vec4<f32>,
    // xyz camera position, w what the camera multiplies light by.
    camera: vec4<f32>,
    // xyz the way the camera faces, w frame count.
    forward: vec4<f32>,
    // xy target size in pixels, zw one pixel in uv.
    screen_size: vec4<f32>,
    // xyz froxels across, down and deep, w flags.
    grid: vec4<u32>,
    // x local volumes, y lights, z sky cube mip count, w lights with beams.
    counts: vec4<u32>,

    // Height fog: x density per metre, y falloff, z base height, w start.
    height: vec4<f32>,
    // rgb fog color already lit by the ambient, nits.
    height_ambient: vec4<f32>,
    // rgb fog color lit by the sun (lux), w sun boost.
    height_sun: vec4<f32>,
    // rgb fog color lit by the moon (lux), w boost g.
    height_moon: vec4<f32>,
    // rgb the fog's own color, times the sky's horizon light.
    height_albedo: vec4<f32>,
    // xyz towards the sun, w unused.
    sun_dir: vec4<f32>,
    // xyz towards the moon.
    moon_dir: vec4<f32>,

    // Aerial haze: x extinction per metre, y height scale, z desaturation,
    // w blue shift.
    aerial: vec4<f32>,
    // rgb the haze's color with no sky to take it from, nits; w unused.
    aerial_color: vec4<f32>,
    // rgb tint on the sky's color.
    aerial_tint: vec4<f32>,

    // Volumetric: x density per metre, y falloff, z base height, w g.
    volumetric: vec4<f32>,
    // rgb albedo, w ambient multiplier.
    volumetric_albedo: vec4<f32>,
    // rgb emitted nits per unit density, w range in metres.
    volumetric_emissive: vec4<f32>,
    // xyz noise offset (scrolled by the wind), w noise amount.
    noise: vec4<f32>,
    // x 1 / noise scale, y history blend, z depth jitter, w unused.
    noise_scale: vec4<f32>,
    // rgb ambient radiance, nits.
    ambient: vec4<f32>,

    locals: array<LocalFogVolume, MAX_LOCAL_FOG>,
    lights: array<FogLight, MAX_FOG_LIGHTS>,
}

// Optical depth of exponential height fog from `origin_y` along a ray
// rising `dir_y` per metre, over `length`. `fog::height_fog_depth` on the
// CPU.
fn height_fog_depth(density: f32, falloff: f32, base: f32, origin_y: f32, dir_y: f32, length: f32) -> f32 {
    let at_origin = density * exp(clamp(-falloff * (origin_y - base), -80.0, 80.0));
    let k = falloff * dir_y;
    if abs(k) < 1e-5 {
        return at_origin * length;
    }
    return at_origin * (1.0 - exp(clamp(-k * length, -80.0, 80.0))) / k;
}

// Froxel slices are spaced by the square of depth, so near slices are
// thin: slice coordinate w in [0, 1] to metres and back.
fn slice_distance(w: f32, range: f32) -> f32 {
    return range * w * w;
}

fn distance_slice(distance: f32, range: f32) -> f32 {
    return sqrt(clamp(distance / range, 0.0, 1.0));
}

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// How much of a local volume covers `p`: 1 inside, easing to 0 across the
// blend distance, as `VolumeSpec::coverage` measures it.
fn local_coverage(volume: LocalFogVolume, p: vec3<f32>) -> f32 {
    let offset = p - volume.centre.xyz;
    var outside: f32;
    if volume.centre.w > 0.5 {
        outside = max(length(offset) - volume.extent.x, 0.0);
    } else {
        let local = quat_rotate(volume.rotation, offset);
        outside = length(max(abs(local) - volume.extent.xyz, vec3<f32>(0.0)));
    }
    if outside <= 0.0 {
        return 1.0;
    }
    if volume.extent.w <= 0.0 {
        return 0.0;
    }
    return clamp(1.0 - outside / volume.extent.w, 0.0, 1.0);
}

// How much more each channel of haze extinction there is than green's:
// gray at 0, Rayleigh's blue-heavy spread at 1.
fn haze_weights(blue_shift: f32) -> vec3<f32> {
    return mix(vec3<f32>(1.0), vec3<f32>(0.43, 1.0, 2.44), blue_shift);
}

// How much of a beam is left `distance` metres from its light.
// `fog::beam_fade` on the CPU.
fn beam_fade(distance: f32, range: f32, near: f32, far: f32, curve: f32) -> f32 {
    let r = max(range, 0.01);
    let t = clamp(distance / r, 0.0, 1.0);
    var fade = pow(max(1.0 - t, 1e-6), curve);
    if near > 0.0 {
        fade *= smoothstep(0.0, near, distance);
    }
    if far > 0.0 {
        fade *= 1.0 - smoothstep(r - far, r, distance);
    }
    return fade;
}

// A spot's soft cone edge, 1 for a point light (`cos_outer` -2).
fn cone_window(cos_angle: f32, cos_outer: f32, cos_inner: f32) -> f32 {
    if cos_outer < -1.5 {
        return 1.0;
    }
    return smoothstep(cos_outer, cos_inner, cos_angle);
}

// Where mote `seed` (each 0-1) sits in a beam's frame at `time`: a cone
// down -Z of `range` metres and `tan_outer` spread, or a ball for a point
// (`tan_outer` < 0). Wanders `drift` metres per second and wraps along
// the beam so the field never empties.
fn mote_in_beam(seed: vec3<f32>, time: f32, range: f32, tan_outer: f32, near: f32, drift: f32) -> vec3<f32> {
    let wander = vec3<f32>(
        sin(time * 0.37 + seed.y * 40.0),
        sin(time * 0.29 + seed.z * 40.0) - 0.4,
        sin(time * 0.23 + seed.x * 40.0),
    ) * drift;
    let angle = seed.y * 2.0 * PI;
    let spread = sqrt(seed.z);
    if tan_outer < 0.0 {
        // Uniform in the ball's volume: radius goes as the cube root.
        let r = range * pow(fract(seed.x + time * drift * 0.02), 0.333);
        let up = spread * 2.0 - 1.0;
        let flat = sqrt(max(1.0 - up * up, 0.0));
        return vec3<f32>(cos(angle) * flat, up, sin(angle) * flat) * r + wander;
    }
    let near_end = min(near, range * 0.5);
    let along = near_end + (range - near_end) * pow(fract(seed.x + time * drift / max(range, 0.01)), 0.333);
    let radius = tan_outer * along * spread;
    return vec3<f32>(cos(angle) * radius, sin(angle) * radius, -along) + wander;
}

// Where mote `seed` sits in a `size` metre box around `camera`, wrapped
// so the field follows the camera without popping.
fn mote_in_box(seed: vec3<f32>, time: f32, camera: vec3<f32>, size: f32, drift: f32) -> vec3<f32> {
    let wander = vec3<f32>(
        sin(time * 0.31 + seed.y * 40.0) + time * 0.2,
        sin(time * 0.27 + seed.z * 40.0) - time * 0.05,
        sin(time * 0.19 + seed.x * 40.0),
    ) * drift;
    let p = seed * size + wander;
    return camera + (fract((p - camera) / size) - 0.5) * size;
}

// A mote's flicker, 1 - twinkle to 1.
fn mote_twinkle(seed: vec3<f32>, time: f32, twinkle: f32) -> f32 {
    let wave = sin(time * (2.0 + seed.z * 5.0) + seed.x * 60.0) * 0.5 + 0.5;
    return 1.0 - twinkle * wave * wave;
}
