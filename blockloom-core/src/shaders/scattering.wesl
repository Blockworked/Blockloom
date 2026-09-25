// Participating media: phase functions, extinction and the per-step
// integral a ray march accumulates. `cos_theta` is the cosine between the
// view ray and the direction towards the light.

const PI: f32 = 3.14159265358979;

fn rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_theta * cos_theta);
}

// `g` in (-1, 1): positive scatters forward, negative back.
fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4);
    return (1.0 - g2) / (4.0 * PI * denom * sqrt(denom));
}

// Henyey-Greenstein with Rayleigh's shape at small `g`, for aerosols.
fn cornette_shanks(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4);
    return 3.0 / (8.0 * PI) * (1.0 - g2) * (1.0 + cos_theta * cos_theta)
        / ((2.0 + g2) * denom * sqrt(denom));
}

// Schlick's cheap stand-in for Henyey-Greenstein.
fn schlick_phase(cos_theta: f32, g: f32) -> f32 {
    let k = 1.55 * g - 0.55 * g * g * g;
    let denom = 1.0 - k * cos_theta;
    return (1.0 - k * k) / (4.0 * PI * max(denom * denom, 1e-4));
}

// A forward and a back lobe, `blend` of the way to the back one: clouds.
fn dual_lobe_phase(cos_theta: f32, g_forward: f32, g_back: f32, blend: f32) -> f32 {
    return mix(henyey_greenstein(cos_theta, g_forward), henyey_greenstein(cos_theta, g_back), blend);
}

// Beer-Lambert: how much light survives `optical_depth`.
fn transmittance(optical_depth: vec3<f32>) -> vec3<f32> {
    return exp(-optical_depth);
}

fn beer(extinction: f32, distance: f32) -> f32 {
    return exp(-extinction * distance);
}

// Darkens thin edges facing the light (Schneider, Horizon Zero Dawn).
fn powder(extinction: f32, distance: f32) -> f32 {
    return 1.0 - exp(-2.0 * extinction * distance);
}

fn beer_powder(extinction: f32, distance: f32) -> f32 {
    return beer(extinction, distance) * powder(extinction, distance) * 2.0;
}

// Light scattered towards the eye over one march step of length `step`,
// integrated analytically so it stays energy-conserving at any step size
// (Hillaire 2015). `scattered` is scattering times incoming light.
fn scatter_step(scattered: vec3<f32>, extinction: vec3<f32>, step: f32) -> vec3<f32> {
    let e = max(extinction, vec3<f32>(1e-6));
    return (scattered - scattered * exp(-e * step)) / e;
}
