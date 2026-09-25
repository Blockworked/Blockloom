// Every fullscreen HDR pass: the Game view's exposure debug views, the HDR
// tone curve and the output encode. The input is already exposed, so 1.0
// is paper white.

struct HdrPass {
    // 1 false color, 2 clipping, 3 histogram, 4 waveform, 5 calibration,
    // 6 HDR preview, 10 HDR tone curve, 11 scRGB encode, 12 PQ encode.
    mode: u32,
    // Exposed luminance that counts as clipped; paper white nits to encode.
    white: f32,
    // Peak over paper white: how far the tone curve may reach.
    headroom: f32,
    // 10000 nits over paper white, the brightest a PQ signal says.
    level: f32,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: HdrPass;

const MIDDLE_GREY: f32 = 0.18;
// Stops either side of middle grey that the scopes show.
const SCOPE_STOPS: f32 = 8.0;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn stops_of(exposed: f32) -> f32 {
    return log2(max(exposed, 1e-6) / MIDDLE_GREY);
}

// Bands of stops around middle grey: purple crushed, blue and teal shadow,
// green middle grey, yellow near white, red past it, white far past it.
fn false_color(exposed: f32) -> vec3<f32> {
    let stops = stops_of(exposed);
    let white = stops_of(settings.white);
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

// Linear to 0.8, then an exponential shoulder towards the headroom, applied
// to the brightest channel so hues hold.
fn tone(color: vec3<f32>, headroom: f32) -> vec3<f32> {
    let knee = 0.8;
    let peak = max(color.r, max(color.g, color.b));
    if peak <= knee || headroom <= knee {
        return min(color, vec3<f32>(max(headroom, knee)));
    }
    let span = headroom - knee;
    let toned = knee + span * (1.0 - exp(-(peak - knee) / span));
    return color * (toned / peak);
}

fn pq(nits: vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let y = pow(clamp(nits / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vec3<f32>(m2));
}

fn rec709_to_rec2020(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(color, vec3<f32>(0.6274, 0.3293, 0.0433)),
        dot(color, vec3<f32>(0.0691, 0.9195, 0.0114)),
        dot(color, vec3<f32>(0.0164, 0.0880, 0.8956)),
    );
}

fn in_panel(pixel: vec2<f32>, origin: vec2<f32>, size: vec2<f32>) -> bool {
    return all(pixel >= origin) && all(pixel < origin + size);
}

// Where a scope row sits for a level, 0 at the bottom.
fn scope_row(exposed: f32) -> f32 {
    return clamp((stops_of(exposed) + SCOPE_STOPS) / (2.0 * SCOPE_STOPS), 0.0, 1.0);
}

// A luminance histogram over the lower left, counted from a grid of samples.
fn histogram(pixel: vec2<f32>, size: vec2<f32>, color: vec4<f32>) -> vec4<f32> {
    let panel = vec2<f32>(floor(size.x * 0.32), floor(size.y * 0.2));
    let origin = vec2<f32>(12.0, size.y - panel.y - 12.0);
    if !in_panel(pixel, origin, panel) {
        return color;
    }
    let local = (pixel - origin) / panel;
    let bins = 64.0;
    let bin = floor(local.x * bins);
    let cols = 48u;
    let rows = 27u;
    var count = 0.0;
    for (var j = 0u; j < rows; j++) {
        for (var i = 0u; i < cols; i++) {
            let at = (vec2<f32>(f32(i), f32(j)) + 0.5) / vec2<f32>(f32(cols), f32(rows)) * size;
            let exposed = luminance(textureLoad(screen, vec2<i32>(at), 0).rgb);
            if floor(scope_row(exposed) * (bins - 0.001)) == bin {
                count += 1.0;
            }
        }
    }
    // A bar a quarter of the samples tall fills the panel.
    let height = count / f32(cols * rows) * 4.0;
    let up = 1.0 - local.y;
    var out = color.rgb * 0.2;
    if up < height {
        out = vec3<f32>(0.85);
    }
    let x = local.x;
    if abs(x - scope_row(MIDDLE_GREY)) < 0.004 {
        out = vec3<f32>(0.1, 0.8, 0.1);
    }
    if abs(x - scope_row(settings.white)) < 0.004 {
        out = vec3<f32>(1.0, 0.0, 0.0);
    }
    return vec4<f32>(out, color.a);
}

// A luminance waveform along the bottom: each column's samples plotted by
// level, so a hot sky or a crushed floor shows where it sits.
fn waveform(pixel: vec2<f32>, size: vec2<f32>, color: vec4<f32>) -> vec4<f32> {
    let panel = vec2<f32>(size.x, floor(size.y * 0.35));
    let origin = vec2<f32>(0.0, size.y - panel.y);
    if !in_panel(pixel, origin, panel) {
        return color;
    }
    let local = (pixel - origin) / panel;
    let level = 1.0 - local.y;
    let band = 1.5 / panel.y;
    let rows = 64u;
    var hits = 0.0;
    for (var j = 0u; j < rows; j++) {
        let at = vec2<f32>(pixel.x, (f32(j) + 0.5) / f32(rows) * size.y);
        let exposed = luminance(textureLoad(screen, vec2<i32>(at), 0).rgb);
        if abs(scope_row(exposed) - level) < band {
            hits += 1.0;
        }
    }
    var out = color.rgb * 0.15 + vec3<f32>(0.1, 1.0, 0.3) * min(hits / 4.0, 1.0);
    if abs(level - scope_row(MIDDLE_GREY)) < 0.5 / panel.y {
        out = vec3<f32>(0.1, 0.5, 0.1);
    }
    if abs(level - scope_row(settings.white)) < 0.5 / panel.y {
        out = vec3<f32>(0.8, 0.0, 0.0);
    }
    return vec4<f32>(out, color.a);
}

// Test patches: a black-level ramp along the top, a paper-white ramp along
// the bottom, and the peak test in the middle - a square at the peak setting
// on a field at the brightest a PQ signal says. Raise the peak until the
// square just disappears.
fn calibration(pixel: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let uv = pixel / size;
    if uv.y < 0.15 {
        let step = floor(uv.x * 16.0);
        return vec4<f32>(vec3<f32>(step * step * 0.0004), 1.0);
    }
    if uv.y > 0.85 {
        let step = floor(uv.x * 16.0);
        return vec4<f32>(vec3<f32>(step / 15.0), 1.0);
    }
    let centre = abs(uv - vec2<f32>(0.5)) * vec2<f32>(size.x / size.y, 1.0);
    if max(centre.x, centre.y) < 0.12 {
        return vec4<f32>(vec3<f32>(settings.headroom), 1.0);
    }
    if max(centre.x, centre.y) < 0.3 {
        return vec4<f32>(vec3<f32>(settings.level), 1.0);
    }
    return vec4<f32>(vec3<f32>(0.0), 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureLoad(screen, vec2<i32>(position.xy), 0);
    let size = vec2<f32>(textureDimensions(screen));
    let exposed = luminance(color.rgb);
    switch settings.mode {
        case 1u: {
            return vec4<f32>(false_color(exposed), color.a);
        }
        case 2u: {
            // Diagonal black stripes stay black under any tonemapper.
            let stripe = ((i32(position.x + position.y) / 6) & 1) == 0;
            if exposed > settings.white && stripe {
                return vec4<f32>(0.0, 0.0, 0.0, color.a);
            }
            return color;
        }
        case 3u: {
            return histogram(position.xy, size, color);
        }
        case 4u: {
            return waveform(position.xy, size, color);
        }
        case 5u: {
            return calibration(position.xy, size);
        }
        case 6u: {
            // What an HDR display shows below paper white; brighter clips.
            return vec4<f32>(min(tone(color.rgb, settings.headroom), vec3<f32>(1.0)), color.a);
        }
        case 10u: {
            return vec4<f32>(tone(max(color.rgb, vec3<f32>(0.0)), settings.headroom), color.a);
        }
        case 11u: {
            // scRGB: linear Rec.709, 1.0 at 80 nits.
            return vec4<f32>(max(color.rgb, vec3<f32>(0.0)) * settings.white / 80.0, color.a);
        }
        case 12u: {
            let wide = max(rec709_to_rec2020(max(color.rgb, vec3<f32>(0.0))), vec3<f32>(0.0));
            return vec4<f32>(pq(wide * settings.white), color.a);
        }
        default: {
            return color;
        }
    }
}
