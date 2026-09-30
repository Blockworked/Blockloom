// A water surface: the Gerstner waves of `blockloom::water` displace the
// mesh, and the fragment works out what is seen through it (refracted,
// absorbed, with caustics on the bed), what it reflects (a planar mirror, or
// screen-space, then the body's probe, then the sky color) and where it
// foams. The sun's glint and the lit color of the water itself come from
// Bevy's own lighting. Simulated ripples ride on top of the waves.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, lights},
    view_transformations::{position_world_to_clip, position_world_to_ndc, ndc_to_uv, uv_to_ndc, position_ndc_to_world},
    pbr_types::pbr_input_new,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};
import bevy_pbr::render::mesh_view_bindings as view_bindings;
import blockloom::water::{WaterWaves, displace, frame, detail_slope, ripple, fresnel, caustics, worley_edge};
import blockloom::noise::value2;

struct WaterLook {
    // Linear shallow color, clarity.
    shallow: vec4<f32>,
    // Linear deep color, absorption distance.
    deep: vec4<f32>,
    // Linear foam color, foam amount.
    foam_color: vec4<f32>,
    // Shore distance, crest threshold, foam scale, foam drift.
    foam: vec4<f32>,
    // Refraction, roughness, glint, reflection strength.
    optics: vec4<f32>,
    // Screen-space reflections on, probe ready, caustics strength, caustics scale.
    modes: vec4<f32>,
    // Probe centre, 1 with a caustics texture.
    probe: vec4<f32>,
    // Sky color for reflections with nothing better, detail fade distance.
    sky: vec4<f32>,
    // Underwater fog color, fog distance.
    under: vec4<f32>,
    // 1 with a planar mirror, its blur radius in uv, 0, 0.
    mirror: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> waves: WaterWaves;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<uniform> look: WaterLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var face_px: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var face_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var face_nx: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var face_py: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var face_ny: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var face_pz: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var face_nz: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(109) var caustics_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(110) var caustics_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var ripple_field: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(112) var mirror_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(113) var mirror_sampler: sampler;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = get_world_from_local(vertex_in.instance_index);
    let rest = (world_from_local * vec4<f32>(v.position, 1.0)).xyz;
    let q = rest.xz;
    // Far out the waves are finer than the mesh: let them go flat.
    let far = distance(q, view.world_position.xz);
    let calm = 1.0 - smoothstep(look.sky.w * 4.0, look.sky.w * 12.0, far);
    let moved = displace(waves, q) * calm;
    let over = q + moved.xz;
    let rings = ripple(ripple_field, waves, over);
    let world = vec3<f32>(over.x, waves.flow.z + moved.y + rings.y, over.y);

    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    @if(VERTEX_NORMALS) {
        out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    @if(VERTEX_UVS_A) {
        // The rest point: the fragment rebuilds the frame from it.
        out.uv = q;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = vec2<f32>(calm, 0.0);
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex_in.instance_index;
    }
    return out;
}

// What the opaque pass left at a screen uv, and how deep it is.
@if(VIEW_TRANSMISSION_TEXTURE)
fn scene_color(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(view_bindings::view_transmission_texture, view_bindings::view_transmission_sampler, uv, 0.0).rgb;
}

@if(!VIEW_TRANSMISSION_TEXTURE)
fn scene_color(uv: vec2<f32>) -> vec3<f32> {
    return look.deep.rgb * 0.2;
}

@if(DEPTH_PREPASS && MULTISAMPLED)
fn scene_depth(uv: vec2<f32>) -> f32 {
    // Scalar casts only, like the particle soft fade: vector casts crash
    // Mali's shader compiler on some drivers.
    let dims = textureDimensions(view_bindings::depth_prepass_texture);
    let size = vec2<f32>(f32(dims.x), f32(dims.y));
    let scaled = clamp(uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * size;
    let texel = vec2<i32>(i32(scaled.x), i32(scaled.y));
    return textureLoad(view_bindings::depth_prepass_texture, texel, 0);
}

@if(DEPTH_PREPASS && !MULTISAMPLED)
fn scene_depth(uv: vec2<f32>) -> f32 {
    let dims = textureDimensions(view_bindings::depth_prepass_texture);
    let size = vec2<f32>(f32(dims.x), f32(dims.y));
    let scaled = clamp(uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * size;
    let texel = vec2<i32>(i32(scaled.x), i32(scaled.y));
    return textureLoad(view_bindings::depth_prepass_texture, texel, 0);
}

@if(!DEPTH_PREPASS)
fn scene_depth(uv: vec2<f32>) -> f32 {
    return 0.0;
}

fn has_depth() -> bool {
    @if(DEPTH_PREPASS) {
        return true;
    }
    return false;
}

// The world point behind a screen uv, or far along the view ray when the
// depth buffer says sky.
fn scene_point(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    return position_ndc_to_world(vec3<f32>(uv_to_ndc(uv), max(depth, 1e-7)));
}

// One face of the body's probe, looked at along `d`.
fn probe_face(face: u32, d: vec3<f32>) -> vec3<f32> {
    var forward = vec3<f32>(1.0, 0.0, 0.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    switch face {
        case 1u: { forward = vec3<f32>(-1.0, 0.0, 0.0); }
        case 2u: { forward = vec3<f32>(0.0, 1.0, 0.0); up = vec3<f32>(0.0, 0.0, 1.0); }
        case 3u: { forward = vec3<f32>(0.0, -1.0, 0.0); up = vec3<f32>(0.0, 0.0, -1.0); }
        case 4u: { forward = vec3<f32>(0.0, 0.0, -1.0); }
        case 5u: { forward = vec3<f32>(0.0, 0.0, 1.0); }
        default: {}
    }
    let right = cross(forward, up);
    let along = max(dot(d, forward), 1e-4);
    let uv = vec2<f32>(0.5 + 0.5 * dot(d, right) / along, 0.5 - 0.5 * dot(d, up) / along);
    switch face {
        case 1u: { return textureSampleLevel(face_nx, face_sampler, uv, 0.0).rgb; }
        case 2u: { return textureSampleLevel(face_py, face_sampler, uv, 0.0).rgb; }
        case 3u: { return textureSampleLevel(face_ny, face_sampler, uv, 0.0).rgb; }
        case 4u: { return textureSampleLevel(face_nz, face_sampler, uv, 0.0).rgb; }
        case 5u: { return textureSampleLevel(face_pz, face_sampler, uv, 0.0).rgb; }
        default: { return textureSampleLevel(face_px, face_sampler, uv, 0.0).rgb; }
    }
}

fn probe(d: vec3<f32>) -> vec3<f32> {
    let a = abs(d);
    if (a.x >= a.y && a.x >= a.z) {
        return probe_face(select(1u, 0u, d.x > 0.0), d);
    }
    if (a.y >= a.z) {
        return probe_face(select(3u, 2u, d.y > 0.0), d);
    }
    return probe_face(select(4u, 5u, d.z > 0.0), d);
}

// Marches the reflected ray through the depth buffer. xyz is the color, w
// how much to trust it.
fn screen_reflection(p: vec3<f32>, r: vec3<f32>) -> vec4<f32> {
    if (!has_depth() || look.modes.x < 0.5) {
        return vec4<f32>(0.0);
    }
    let reach = max(distance(p, view.world_position) * 1.5, 4.0);
    var t = reach / 64.0;
    var last = 0.0;
    for (var i = 0; i < 24; i++) {
        let at = p + r * t;
        let ndc = position_world_to_ndc(at);
        if (any(abs(ndc.xy) > vec2<f32>(1.0)) || ndc.z <= 0.0) {
            return vec4<f32>(0.0);
        }
        let uv = ndc_to_uv(ndc.xy);
        let depth = scene_depth(uv);
        // Reverse z: something in front of the ray point is a nearer depth.
        if (depth > ndc.z) {
            let hit = scene_point(uv, depth);
            // Too far behind the ray is a gap, not a hit.
            let gap = distance(hit, at);
            if (gap < (t - last) * 2.0 + 0.5) {
                let edge = min(1.0 - abs(ndc.x), 1.0 - abs(ndc.y));
                let fade = smoothstep(0.0, 0.15, edge) * (1.0 - f32(i) / 24.0);
                return vec4<f32>(scene_color(uv), fade);
            }
            return vec4<f32>(0.0);
        }
        last = t;
        t *= 1.35;
    }
    return vec4<f32>(0.0);
}

// The planar mirror at a screen uv, bent by the surface and blurred by its
// roughness. The mirror camera is upside down, so v flips.
fn mirror(screen: vec2<f32>, n: vec3<f32>, dist: f32) -> vec3<f32> {
    let bend = n.xz * look.optics.x * 0.12 / (1.0 + dist * 0.05);
    let uv = vec2<f32>(screen.x, 1.0 - screen.y) + vec2<f32>(bend.x, -bend.y);
    let radius = look.mirror.y;
    if (radius <= 1e-4) {
        return textureSampleLevel(mirror_texture, mirror_sampler, uv, 0.0).rgb;
    }
    // A golden-angle disc of taps.
    var sum = vec3<f32>(0.0);
    let taps = 12;
    for (var i = 0; i < taps; i++) {
        let f = f32(i) + 0.5;
        let a = f * 2.39996323;
        let r = sqrt(f / f32(taps)) * radius;
        sum += textureSampleLevel(mirror_texture, mirror_sampler, uv + vec2<f32>(cos(a), sin(a)) * r, 0.0).rgb;
    }
    return sum / f32(taps);
}

fn reflection(p: vec3<f32>, r: vec3<f32>, screen: vec2<f32>, n: vec3<f32>, dist: f32) -> vec3<f32> {
    if (look.mirror.x > 0.5) {
        return mirror(screen, n, dist);
    }
    // Below the horizon a flat reflection sees the water's own depths.
    let upward = vec3<f32>(r.x, max(r.y, 0.02), r.z);
    var env = look.sky.rgb;
    if (look.modes.y > 0.5) {
        env = probe(normalize(upward));
    }
    let hit = screen_reflection(p, r);
    return mix(env, hit.rgb, hit.w);
}

fn caustic_light(p: vec2<f32>, t: f32) -> f32 {
    let scaled = p / max(look.modes.w, 1e-3);
    if (look.probe.w > 0.5) {
        let a = textureSampleLevel(caustics_texture, caustics_sampler, scaled + vec2<f32>(t * 0.02, t * 0.013), 0.0).r;
        let b = textureSampleLevel(caustics_texture, caustics_sampler, scaled * 1.3 - vec2<f32>(t * 0.017, -t * 0.011), 0.0).r;
        return min(a, b) * 2.0;
    }
    return caustics(scaled, t);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let t = waves.counts.z;
    var q = in.world_position.xz;
    @if(VERTEX_UVS_A) {
        q = in.uv;
    }
    var calm = 1.0;
    @if(VERTEX_UVS_B) {
        calm = in.uv_b.x;
    }
    let p = in.world_position.xyz;
    let to_eye = view.world_position - p;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);

    // The Gerstner frame, then finer slopes that fade before they alias.
    let f = frame(waves, q);
    let fine = 1.0 / (1.0 + dist / max(look.sky.w, 1e-3));
    let slopes = detail_slope(waves, q) * fine + ripple(ripple_field, waves, in.world_position.xz).xz;
    var n = normalize(mix(vec3<f32>(0.0, 1.0, 0.0), f.normal, calm) - vec3<f32>(slopes.x, 0.0, slopes.y));
    let below = !is_front || (view.world_position.y < waves.flow.z - 0.05 && dot(v, n) < 0.0);
    if (below) {
        n = -n;
    }
    let cos_v = clamp(dot(n, v), 0.0, 1.0);

    // What is seen through the surface, bent by the slope.
    let screen = in.position.xy / view.viewport.zw;
    let water_ndc = position_world_to_ndc(p);
    var uv = screen + n.xz * look.optics.x * 0.08 / (1.0 + dist * 0.05);
    var bed_depth = scene_depth(uv);
    // Whatever stands in front of the water can't be refracted.
    if (bed_depth > water_ndc.z) {
        uv = screen;
        bed_depth = scene_depth(uv);
    }
    let behind = scene_color(uv);
    var through = dist * 4.0;
    var column = look.deep.w * 4.0;
    var bed = p - v * through;
    if (has_depth() && bed_depth > 0.0) {
        bed = scene_point(uv, bed_depth);
        through = distance(bed, p);
        column = max(waves.flow.z - bed.y, 0.0);
    }

    let absorption = max(look.deep.w, 1e-3);
    let extinction = (vec3<f32>(1.0) - look.shallow.rgb * 0.75) * (2.0 - look.shallow.w);
    let transmit = exp(-extinction * through / absorption);
    var seen = behind;
    if (!below && look.modes.z > 0.0) {
        let light = exp(-column / absorption) * look.modes.z;
        seen *= 1.0 + caustic_light(bed.xz, t) * light;
    }
    let depth_tint = mix(look.shallow.rgb, look.deep.rgb, 1.0 - exp(-through / absorption));
    let refracted = seen * transmit;
    let scatter = depth_tint * (vec3<f32>(1.0) - transmit);

    // Foam: along the shore, on pinched crests, broken up by noise.
    let drift = waves.flow.xy * t + vec2<f32>(look.foam.w * t);
    let shore = select(0.0, 1.0 - clamp(column / max(look.foam.x, 1e-3), 0.0, 1.0), look.foam.x > 0.0 && !below);
    let crest = clamp((look.foam.y + 0.3 - f.jacobian) / 0.3, 0.0, 1.0) * calm;
    let pattern_uv = (q - drift) / max(look.foam.z, 1e-3);
    let pattern = 0.6 * worley_edge(pattern_uv) + 0.4 * value2(pattern_uv * 3.1 + vec2<f32>(t * 0.1));
    let cover = clamp(max(shore, crest) * look.foam_color.w, 0.0, 1.0);
    let foam = clamp(cover * 1.6 - (1.0 - pattern), 0.0, 1.0);

    var out: FragmentOutput;
    if (below) {
        // From underneath: Snell's window, total reflection outside it.
        let window = smoothstep(0.62, 0.7, cos_v);
        let fog = look.under.rgb;
        let color = mix(fog, behind, window * exp(-dist / max(look.under.w, 1e-3)));
        out.color = vec4<f32>(mix(color, look.foam_color.rgb * fog * 2.0, foam), 1.0);
        return out;
    }

    // Bevy lights the water's own color and adds the sun's glint.
    var pbr = pbr_input_new();
    pbr.material.base_color = vec4<f32>(mix(scatter, look.foam_color.rgb, foam), 1.0);
    pbr.material.perceptual_roughness = mix(look.optics.y, 0.8, foam);
    pbr.material.metallic = 0.0;
    pbr.material.reflectance = vec3<f32>(0.354 * sqrt(look.optics.z) * (1.0 - foam));
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = n;
    pbr.N = n;
    pbr.V = v;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    let lit = apply_pbr_lighting(pbr).rgb;

    let r = reflect(-v, n);
    let fr = fresnel(cos_v) * look.optics.w * (1.0 - foam);
    let color = lit + refracted * (1.0 - fr) * (1.0 - foam) + reflection(p, r, screen, n, dist) * fr;
    out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(color, 1.0));
    return out;
}
