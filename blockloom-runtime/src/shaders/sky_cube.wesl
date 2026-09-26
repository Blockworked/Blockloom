// Writes the sky's light into two cubes whenever it changes: the full one
// the reflections and a blurred background are filtered from, and a small
// one scaled by the ambient dimmer that only the diffuse light is filtered
// from. Radiance over `SKY_UNIT`, disks left out: the sun is a light of its
// own.

import blockloom::sky::{SkyParams, SKY_UNIT, KIND_HDRI, face_direction, hdri_lookup, sky_radiance};

@group(0) @binding(0) var<uniform> sky: SkyParams;
@group(0) @binding(1) var source: texture_cube<f32>;
@group(0) @binding(2) var source_sampler: sampler;
@group(0) @binding(3) var light_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(4) var diffuse_out: texture_storage_2d_array<rgba16float, write>;

// The diffuse cube is this many times smaller a side.
const DIFFUSE_RATIO: u32 = 8u;

fn radiance(dir: vec3<f32>, size: u32) -> vec3<f32> {
    if sky.info.x == KIND_HDRI {
        // The image's level whose texels match this face's, so a small sun
        // in the image is averaged rather than hit or missed.
        let source_size = f32(textureDimensions(source).x);
        let lod = clamp(log2(source_size / f32(size)), 0.0, f32(sky.info.w) - 1.0);
        return textureSampleLevel(source, source_sampler, hdri_lookup(dir, sky), lod).rgb
            * sky.hdri.rgb * sky.scale.x;
    }
    return sky_radiance(dir, sky);
}

@compute @workgroup_size(8, 8, 1)
fn write_sky(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = sky.info.z;
    if id.x >= size || id.y >= size || id.z >= 6u {
        return;
    }
    let dir = face_direction(id.z, id.xy, size);
    let light = min(radiance(dir, size) / SKY_UNIT, vec3<f32>(60000.0));
    textureStore(light_out, id.xy, id.z, vec4<f32>(light, 1.0));
    if all(id.xy % DIFFUSE_RATIO == vec2<u32>(0u)) {
        let small = size / DIFFUSE_RATIO;
        let texel = id.xy / DIFFUSE_RATIO;
        let ambient = radiance(face_direction(id.z, texel, small), small) * sky.scale.y / SKY_UNIT;
        textureStore(diffuse_out, texel, id.z, vec4<f32>(min(ambient, vec3<f32>(60000.0)), 1.0));
    }
}
