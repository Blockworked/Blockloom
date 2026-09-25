// Joint bilateral upsample: a half-res scratch texture onto a full-res
// target. The four nearest scratch texels are weighted bilinearly and, with
// a depth guide, by how close their depth is to this pixel's, so a half-res
// effect doesn't bleed across silhouettes.

import blockloom::frame::{FrameUniforms, full_texel};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@if(DEPTH_GUIDE && MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_multisampled_2d;
@if(DEPTH_GUIDE && !MULTISAMPLED)
@group(0) @binding(2) var depth: texture_depth_2d;

@if(DEPTH_GUIDE)
fn depth_at(pixel: vec2<i32>) -> f32 {
    let edge = vec2<i32>(frame.target_size) - 1;
    // Mip level or sample index, 0 either way.
    return textureLoad(depth, clamp(pixel, vec2<i32>(0), edge), 0);
}

// No guide: every tap agrees, which leaves plain bilinear.
@if(!DEPTH_GUIDE)
fn depth_at(pixel: vec2<i32>) -> f32 {
    return 1.0;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let here = depth_at(vec2<i32>(position.xy));
    // This pixel's centre in scratch texels, measured between texel centres.
    let at = position.xy * 0.5 - 0.5;
    let base = vec2<i32>(floor(at));
    let f = fract(at);
    let edge = vec2<i32>(frame.scratch_size) - 1;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(i & 1, i >> 1);
        let texel = clamp(base + o, vec2<i32>(0), edge);
        let bilinear = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        let there = depth_at(full_texel(texel));
        // Reverse-Z depth goes as 1/distance, so this is the relative gap in
        // distance, whatever the near plane.
        let gap = abs(here - there) / max(max(here, there), 1e-7);
        let w = (bilinear + 1e-3) / (1e-3 + gap);
        sum += textureLoad(source, texel, 0) * w;
        total += w;
    }
    return sum / total;
}
