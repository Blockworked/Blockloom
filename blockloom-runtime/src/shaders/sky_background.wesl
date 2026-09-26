// Draws the sky behind the world, in Bevy's opaque pass where its skybox
// would go: a fullscreen triangle at the far plane, depth-tested so only
// what nothing covers is drawn. Gradients are worked out per pixel, a
// physical sky reads its light cube and adds its sun and moon, an HDRI
// reads its image sharp or its filtered light blurred. The main view adds
// the stars, the Milky Way, aurora and any lightning flash on top.

import bevy_render::view::View;
import bevy_pbr::render::utils::coords_to_viewport_uv;
import blockloom::sky::{
    SkyParams, SKY_UNIT, KIND_PHYSICAL, KIND_GRADIENT, KIND_HDRI, FLAG_DISKS, FLAG_DITHER,
    cube_lookup, hdri_lookup, gradient_radiance, disks,
};
import blockloom::hash::hash21;
import blockloom::space::{SpaceParams, stars, aurora, milky_way_uv};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> sky: SkyParams;
@group(0) @binding(2) var light_cube: texture_cube<f32>;
@group(0) @binding(3) var source: texture_cube<f32>;
@group(0) @binding(4) var blurred: texture_cube<f32>;
@group(0) @binding(5) var cube_sampler: sampler;
@group(0) @binding(6) var<uniform> space: SpaceParams;
@group(0) @binding(7) var milky_way: texture_2d<f32>;

fn ray_direction(position: vec2<f32>) -> vec3<f32> {
    let clip = view.view_from_clip * vec4<f32>(
        coords_to_viewport_uv(position, view.main_pass_viewport) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0),
        1.0,
        1.0,
    );
    return normalize((view.world_from_view * vec4<f32>(clip.xyz / clip.w, 0.0)).xyz);
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let clip = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u)) * 4.0 - vec2<f32>(1.0);
    return VertexOutput(vec4<f32>(clip, 0.0, 1.0));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = ray_direction(in.position.xy);
    var color = vec3<f32>(0.0);
    switch sky.info.x {
        case KIND_GRADIENT: {
            color = gradient_radiance(dir, sky) * sky.scale.x;
        }
        case KIND_HDRI: {
            if sky.scale.z > 0.0 {
                color = textureSampleLevel(blurred, cube_sampler, cube_lookup(dir), sky.hdri.w).rgb * SKY_UNIT;
            } else {
                color = textureSampleLevel(source, cube_sampler, hdri_lookup(dir, sky), 0.0).rgb
                    * sky.hdri.rgb * sky.scale.x;
            }
        }
        case KIND_PHYSICAL: {
            color = textureSampleLevel(light_cube, cube_sampler, cube_lookup(dir), 0.0).rgb * SKY_UNIT;
            if (sky.info.y & FLAG_DISKS) != 0u {
                // How many disk radii a pixel spans, for a pixel-wide soft edge.
                let pixel = length(fwidth(dir));
                color += disks(dir, sky, pixel / sky.sun_dir.w, pixel / sky.moon_dir.w) * sky.scale.x;
            }
        }
        default: {}
    }
    // Space belongs to the view itself, not to what probes capture.
    if (sky.info.y & FLAG_DISKS) != 0u {
        let pixel = length(fwidth(dir));
        var night = stars(dir, space, pixel) + aurora(dir, space);
        if space.milky_way.x > 0.0 {
            let fade = space.twinkle.w * smoothstep(0.0, max(space.twinkle.z, 1e-4), dir.y);
            let band = textureSampleLevel(milky_way, cube_sampler, milky_way_uv(dir, space), 0.0).rgb;
            night += band * space.milky_way.x * fade;
        }
        color += night * sky.scale.x + space.flash.rgb * space.time.w;
    }
    color *= view.exposure;
    if (sky.info.y & FLAG_DITHER) != 0u {
        // About one 8-bit step of noise, in the color's own scale.
        let noise = hash21(in.position.xy) + hash21(in.position.yx + 17.0) - 1.0;
        color *= 1.0 + noise * (2.0 / 255.0);
    }
    return vec4<f32>(min(color, vec3<f32>(60000.0)), 1.0);
}
