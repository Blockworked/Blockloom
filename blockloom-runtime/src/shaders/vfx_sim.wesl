// The GPU half of the VFX graph: one emitter's particles, stepped in place.
// `begin` opens the step (spawn budget in, counters zeroed) and `simulate`
// runs the update stack on every live slot, then hands empty slots out of
// the budget. `vfx::Pool` in core is the CPU twin: change them together.

import blockloom::hash::pcg;
import blockloom::vfx::{Particle, alive, rand, curl, noise3, ball, cone, lut_at};

struct SimParams {
    // World from emitter: its turn and position.
    emitter: mat4x4<f32>,
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // Gravity times the emitter's scale, dt.
    gravity: vec4<f32>,
    // The air at the emitter times its `wind`, the pool's clock.
    wind: vec4<f32>,
    // Speed, lifetime, half the spread in radians, size_random.
    launch: vec4<f32>,
    // The shape's numbers, the shape's kind.
    shape: vec4<f32>,
    // Spin, random rotation, flipbook frames, random start frame.
    look: vec4<f32>,
    // Viewport width, height, depth ready, collision thickness.
    screen: vec4<f32>,
    // Size start, size end, projection y scale, aspect.
    size: vec4<f32>,
    // Capacity, particles to start, seed, flags.
    counts: vec4<u32>,
    // Ribbon step, ribbon points, triangles, the step's sequence number.
    ribbon: vec4<f32>,
    // The size curve's LUT, four samples a lane.
    size_lut: array<vec4<f32>, 8>,
    // Three lanes a module: [kind, a, b, c], [d, e, f, g], [target, has target].
    modules: array<vec4<f32>, 24>,
}

// What one step reports, read back on the CPU.
struct EmitterState {
    budget: atomic<i32>,
    alive: atomic<u32>,
    spawned: atomic<u32>,
    died: atomic<u32>,
    collided: atomic<u32>,
    // Screens of particle area, times 10000.
    overdraw: atomic<u32>,
    // Which step these counts are from, so a readback of a frame that
    // didn't step isn't counted twice.
    sequence: u32,
    pad: u32,
    spawn_at: vec4<f32>,
    die_at: vec4<f32>,
    collide_at: vec4<f32>,
}

const FLAG_FLAT: u32 = 1u;
const FLAG_OUTWARD: u32 = 2u;
const FLAG_RIBBONS: u32 = 4u;
const MAX_MODULES: u32 = 8u;
const COVERAGE: f32 = 10000.0;

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read_write> trail: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> state: EmitterState;
// Four lanes a triangle, in the emitter's frame: a and the running area, b, c, normal.
@group(0) @binding(4) var<storage, read> surface: array<vec4<f32>>;
@if(MULTISAMPLED)
@group(0) @binding(5) var depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(5) var depth: texture_depth_2d;

fn flag(bit: u32) -> bool {
    return (params.counts.w & bit) != 0u;
}

fn size_at(t: f32) -> f32 {
    let at = lut_at(t);
    let i = u32(at.x);
    let j = min(i + 1u, 31u);
    let a = params.size_lut[i / 4u][i % 4u];
    let b = params.size_lut[j / 4u][j % 4u];
    return mix(params.size.x, params.size.y, t) * mix(a, b, at.y);
}

@compute @workgroup_size(1)
fn begin() {
    atomicStore(&state.budget, i32(params.counts.y));
    atomicStore(&state.alive, 0u);
    atomicStore(&state.spawned, 0u);
    atomicStore(&state.died, 0u);
    atomicStore(&state.collided, 0u);
    atomicStore(&state.overdraw, 0u);
    state.sequence = u32(params.ribbon.w);
}

// A point and its normal on the actor's mesh, area weighted.
fn on_surface(u: vec3<f32>) -> array<vec3<f32>, 2> {
    let count = u32(params.ribbon.z);
    let total = surface[(count - 1u) * 4u].w;
    let want = u.x * total;
    var lo = 0u;
    var hi = count - 1u;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (surface[mid * 4u].w < want) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    let a = surface[lo * 4u].xyz;
    let b = surface[lo * 4u + 1u].xyz;
    let c = surface[lo * 4u + 2u].xyz;
    var s = u.y;
    var r = u.z;
    if (s + r > 1.0) {
        s = 1.0 - s;
        r = 1.0 - r;
    }
    return array<vec3<f32>, 2>(a + (b - a) * s + (c - a) * r, surface[lo * 4u + 3u].xyz);
}

fn born(index: u32) -> Particle {
    var seed = pcg(params.counts.z ^ pcg(index + 0x9E3779B9u));
    let flat = flag(FLAG_FLAT);
    let u = vec4<f32>(rand(&seed), rand(&seed), rand(&seed), rand(&seed));
    let kind = u32(params.shape.w);
    var local = vec3<f32>(0.0);
    var outward = ball(u.xy, flat);
    switch kind {
        case 1u: {
            let reach = select(select(pow(u.z, 1.0 / 3.0), sqrt(u.z), flat), 1.0, params.shape.y > 0.5);
            local = outward * params.shape.x * reach;
        }
        case 2u: {
            local = (u.xyz - vec3<f32>(0.5)) * params.shape.xyz;
            if (flat) {
                local.z = 0.0;
            }
            outward = select(vec3<f32>(0.0, 1.0, 0.0), normalize(local), length(local) > 1e-6);
        }
        case 3u: {
            let angle = u.x * 6.28318530717959;
            let r = sqrt(u.y) * params.shape.x;
            if (flat) {
                local = vec3<f32>(0.0, (u.x * 2.0 - 1.0) * params.shape.x, 0.0);
            } else {
                local = vec3<f32>(cos(angle) * r, sin(angle) * r, 0.0);
            }
            outward = select(vec3<f32>(0.0, 1.0, 0.0), normalize(local), length(local) > 1e-6);
        }
        case 4u: {
            if (params.ribbon.z >= 1.0) {
                let hit = on_surface(u.xyz);
                local = hit[0];
                outward = hit[1];
            }
        }
        default: {}
    }
    let v = vec2<f32>(rand(&seed), rand(&seed));
    var dir = cone(v, params.launch.z, flat);
    if (flag(FLAG_OUTWARD)) {
        dir = outward;
    }
    let turn = mat3x3<f32>(params.emitter[0].xyz, params.emitter[1].xyz, params.emitter[2].xyz);
    var p: Particle;
    p.position = (params.emitter * vec4<f32>(local, 1.0)).xyz;
    p.velocity = normalize(turn * dir) * params.launch.x * (0.5 + rand(&seed));
    p.life = max(params.launch.y * (0.6 + 0.8 * rand(&seed)), 1e-3);
    p.age = 0.0;
    p.rotation = select(0.0, rand(&seed) * 360.0, params.look.y > 0.5);
    p.spin = params.look.x * select(1.0, -1.0, rand(&seed) < 0.5);
    p.size = 1.0 - params.launch.w * rand(&seed);
    p.seed = pcg(seed);
    p.ribbon = 1u << 16u;
    p.flags = 0u;
    p.hits = 0u;
    p.frame = select(0.0, floor(rand(&seed) * params.look.z), params.look.w > 0.5);
    return p;
}

// The scene's surface under a world point, from the depth prepass: its
// world position and normal, or w = 0 where there is none to hit.
fn scene_at(world: vec3<f32>) -> array<vec4<f32>, 2> {
    var none = array<vec4<f32>, 2>(vec4<f32>(0.0), vec4<f32>(0.0));
    if (params.screen.z < 0.5) {
        return none;
    }
    let clip = params.clip_from_world * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-4) {
        return none;
    }
    let ndc = clip.xyz / clip.w;
    if (any(abs(ndc.xy) > vec2<f32>(0.999))) {
        return none;
    }
    let size = params.screen.xy;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    let texel = vec2<i32>(uv * size);
    let d = textureLoad(depth, texel, 0);
    // Reverse z: nearer is larger, and 0 is the sky.
    if (d <= 0.0 || ndc.z >= d) {
        return none;
    }
    let s = unproject(ndc.xy, d);
    let right = unproject(ndc.xy + vec2<f32>(2.0 / size.x, 0.0), textureLoad(depth, texel + vec2<i32>(1, 0), 0));
    let down = unproject(ndc.xy - vec2<f32>(0.0, 2.0 / size.y), textureLoad(depth, texel + vec2<i32>(0, 1), 0));
    var n = cross(right - s, down - s);
    if (length(n) < 1e-8) {
        return none;
    }
    n = normalize(n);
    return array<vec4<f32>, 2>(vec4<f32>(s, 1.0), vec4<f32>(n, 0.0));
}

fn unproject(ndc: vec2<f32>, d: f32) -> vec3<f32> {
    let w = params.world_from_clip * vec4<f32>(ndc, d, 1.0);
    return w.xyz / w.w;
}

fn lay_point(i: u32, p: ptr<function, Particle>) {
    let points = u32(params.ribbon.y);
    let base = i * points;
    let head = (*p).ribbon & 0xFFFFu;
    let held = max((*p).ribbon >> 16u, 1u);
    let pos = (*p).position;
    let before = trail[base + (head + points - 1u) % points];
    let moved = held > 1u && distance(pos, before.xyz) >= params.ribbon.x;
    if (held == 1u || moved) {
        let next = (head + 1u) % points;
        trail[base + next] = vec4<f32>(pos, params.wind.w);
        (*p).ribbon = next | (min(held + 1u, points) << 16u);
    } else {
        trail[base + head] = vec4<f32>(pos, params.wind.w);
    }
}

@compute @workgroup_size(64)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.counts.x) {
        return;
    }
    let dt = params.gravity.w;
    let time = params.wind.w;
    let flat = flag(FLAG_FLAT);
    let ribbons = flag(FLAG_RIBBONS);
    let emitter_at = params.emitter[3].xyz;
    var p = particles[i];
    if (alive(p)) {
        p.age += dt;
        var dead = p.age >= p.life;
        var pos = p.position;
        var vel = p.velocity;
        if (!dead) {
            vel += params.gravity.xyz * dt;
            var collide = -1;
            for (var m = 0u; m < MAX_MODULES; m++) {
                let a = params.modules[m * 3u];
                let b = params.modules[m * 3u + 1u];
                let c = params.modules[m * 3u + 2u];
                switch u32(a.x) {
                    case 1u: {
                        vel += a.yzw * dt;
                    }
                    case 2u: {
                        vel *= exp(-a.y * dt);
                    }
                    case 3u: {
                        let q = pos * a.z + vec3<f32>(time * a.w);
                        vel += curl(q, flat) * a.y * dt;
                    }
                    case 4u: {
                        let q = pos * a.z + vec3<f32>(p.age * 1.7) + vec3<f32>(f32(p.seed & 1023u));
                        var push = noise3(q);
                        if (flat) {
                            push.z = 0.0;
                        }
                        vel += push * a.y * dt;
                    }
                    case 5u: {
                        let goal = select(emitter_at, c.xyz, c.w > 0.5);
                        let to = goal - pos;
                        let d = length(to);
                        if (d < a.w) {
                            dead = true;
                        } else if (a.z <= 0.0 || d < a.z) {
                            vel += select(vec3<f32>(0.0), to / d, d > 1e-6) * a.y * dt;
                        }
                    }
                    case 6u: {
                        collide = i32(m);
                    }
                    default: {}
                }
            }
            // The air eases in over about a quarter second.
            let next = pos + (vel + params.wind.xyz * min(p.age / 0.25, 1.0)) * dt;
            var moved = true;
            if (collide >= 0 && !flat) {
                let hit = scene_at(next);
                if (hit[0].w > 0.5 && distance(next, hit[0].xyz) < params.screen.w) {
                    let a = params.modules[u32(collide) * 3u];
                    let b = params.modules[u32(collide) * 3u + 1u];
                    var n = hit[1].xyz;
                    if (dot(n, pos - hit[0].xyz) < 0.0) {
                        n = -n;
                    }
                    let vn = n * dot(vel, n);
                    let vt = vel - vn;
                    vel = vt * (1.0 - a.z) - vn * a.y;
                    pos = hit[0].xyz + n * 0.01;
                    p.age += p.life * a.w;
                    p.hits += 1u;
                    atomicAdd(&state.collided, 1u);
                    state.collide_at = vec4<f32>(pos, 1.0);
                    moved = false;
                    if (b.x > 0.5 || p.age >= p.life) {
                        dead = true;
                    }
                }
            }
            if (moved) {
                pos = next;
            }
            for (var m = 0u; m < MAX_MODULES; m++) {
                let a = params.modules[m * 3u];
                let b = params.modules[m * 3u + 1u];
                if (u32(a.x) == 7u && dot(pos - b.xyz, a.yzw) < 0.0) {
                    dead = true;
                }
            }
        }
        if (dead) {
            atomicAdd(&state.died, 1u);
            state.die_at = vec4<f32>(pos, 1.0);
            p.life = 0.0;
        } else {
            p.position = pos;
            p.velocity = vel;
            p.rotation += p.spin * dt;
            if (ribbons) {
                lay_point(i, &p);
            }
        }
    } else if (atomicSub(&state.budget, 1) > 0) {
        p = born(i);
        let points = u32(params.ribbon.y);
        trail[i * points] = vec4<f32>(p.position, time);
        p.ribbon = 1u << 16u;
        atomicAdd(&state.spawned, 1u);
        state.spawn_at = vec4<f32>(p.position, 1.0);
    }
    if (alive(p)) {
        atomicAdd(&state.alive, 1u);
        // The share of the screen this one covers, for the overdraw meter.
        let clip = params.clip_from_world * vec4<f32>(p.position, 1.0);
        if (clip.w > 1e-4) {
            let side = size_at(p.age / p.life) * p.size * params.size.z / (2.0 * clip.w);
            let area = min(side * side / max(params.size.w, 1e-3), 16.0);
            atomicAdd(&state.overdraw, u32(area * COVERAGE));
        }
    }
    particles[i] = p;
}
