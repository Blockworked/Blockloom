// The Game view's exposure debug views, drawn over the HDR scene before it
// is tonemapped. The input is already exposed, so 1.0 is paper white.

struct HdrDebug {
    // 1 false color, 2 clipping zebra.
    mode: u32,
    // Exposed luminance that counts as clipped.
    paper_white: f32,
    pad: vec2<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: HdrDebug;

const MIDDLE_GREY: f32 = 0.18;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Bands of stops around middle grey: purple crushed, blue and teal shadow,
// green middle grey, yellow near white, red past it, white far past it.
fn false_color(exposed: f32) -> vec3<f32> {
    let stops = log2(max(exposed, 1e-6) / MIDDLE_GREY);
    let white = log2(max(settings.paper_white, 1e-6) / MIDDLE_GREY);
    if stops >= white + 2.0 {
        return vec3<f32>(1.0, 1.0, 1.0);
    }
    if stops >= white {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    if stops >= white - 1.0 {
        return vec3<f32>(1.0, 0.8, 0.0);
    }
    if stops >= 0.5 {
        return vec3<f32>(0.45, 0.45, 0.45);
    }
    if stops >= -0.5 {
        return vec3<f32>(0.1, 0.8, 0.1);
    }
    if stops >= -2.0 {
        return vec3<f32>(0.12, 0.12, 0.12);
    }
    if stops >= -4.0 {
        return vec3<f32>(0.0, 0.35, 0.45);
    }
    if stops >= -6.0 {
        return vec3<f32>(0.0, 0.05, 0.6);
    }
    return vec3<f32>(0.2, 0.0, 0.3);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureLoad(screen, vec2<i32>(position.xy), 0);
    let exposed = luminance(color.rgb);
    if settings.mode == 1u {
        return vec4<f32>(false_color(exposed), color.a);
    }
    // Diagonal black stripes stay black under any tonemapper.
    let stripe = ((i32(position.x + position.y) / 6) & 1) == 0;
    if settings.mode == 2u && exposed > settings.paper_white && stripe {
        return vec4<f32>(0.0, 0.0, 0.0, color.a);
    }
    return color;
}
