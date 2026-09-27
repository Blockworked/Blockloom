// Scene luminance meter: one workgroup samples a 64x32 grid of the exposed
// image and writes its log-average and peak luminance, plus the
// log-averages auto-exposure's center-weighted and spot meters read.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> result: array<f32, 8>;

const GRID_X: u32 = 64u;
const GRID_Y: u32 = 32u;
const THREADS: u32 = 256u;
// Gaussian widths over the frame's height, for the two weighted meters.
const CENTER_SIGMA: f32 = 0.35;
const SPOT_SIGMA: f32 = 0.1;

var<workgroup> log_sums: array<f32, 256>;
var<workgroup> peaks: array<f32, 256>;
var<workgroup> center_sums: array<f32, 256>;
var<workgroup> center_weights: array<f32, 256>;
var<workgroup> spot_sums: array<f32, 256>;
var<workgroup> spot_weights: array<f32, 256>;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn gaussian(offset: vec2<f32>, sigma: f32) -> f32 {
    return exp(-dot(offset, offset) / (2.0 * sigma * sigma));
}

@compute @workgroup_size(256, 1, 1)
fn meter(@builtin(local_invocation_index) index: u32) {
    let size = textureDimensions(source);
    let aspect = f32(size.x) / max(f32(size.y), 1.0);
    var log_sum = 0.0;
    var peak = 0.0;
    var center_sum = 0.0;
    var center_weight = 0.0;
    var spot_sum = 0.0;
    var spot_weight = 0.0;
    for (var i = index; i < GRID_X * GRID_Y; i += THREADS) {
        let cell = vec2<f32>(f32(i % GRID_X) + 0.5, f32(i / GRID_X) + 0.5);
        let uv = cell / vec2<f32>(f32(GRID_X), f32(GRID_Y));
        let texel = vec2<u32>(uv * vec2<f32>(size));
        let lum = max(luminance(textureLoad(source, min(texel, size - 1u), 0).rgb), 0.0);
        // Floor keeps pure black from sending the log average to zero.
        let log_lum = log2(max(lum, 1.0e-4));
        log_sum += log_lum;
        peak = max(peak, lum);
        let offset = (uv - 0.5) * vec2<f32>(aspect, 1.0);
        let wc = gaussian(offset, CENTER_SIGMA);
        center_sum += log_lum * wc;
        center_weight += wc;
        let ws = gaussian(offset, SPOT_SIGMA);
        spot_sum += log_lum * ws;
        spot_weight += ws;
    }
    log_sums[index] = log_sum;
    peaks[index] = peak;
    center_sums[index] = center_sum;
    center_weights[index] = center_weight;
    spot_sums[index] = spot_sum;
    spot_weights[index] = spot_weight;
    workgroupBarrier();

    for (var stride = THREADS / 2u; stride > 0u; stride /= 2u) {
        if index < stride {
            log_sums[index] += log_sums[index + stride];
            peaks[index] = max(peaks[index], peaks[index + stride]);
            center_sums[index] += center_sums[index + stride];
            center_weights[index] += center_weights[index + stride];
            spot_sums[index] += spot_sums[index + stride];
            spot_weights[index] += spot_weights[index + stride];
        }
        workgroupBarrier();
    }

    if index == 0u {
        let average = log_sums[0] / f32(GRID_X * GRID_Y);
        result[0] = exp2(average);
        result[1] = peaks[0];
        result[2] = 1.0;
        result[3] = exp2(select(average, center_sums[0] / center_weights[0], center_weights[0] > 0.0));
        result[4] = exp2(select(average, spot_sums[0] / spot_weights[0], spot_weights[0] > 0.0));
        result[5] = 0.0;
        result[6] = 0.0;
        result[7] = 0.0;
    }
}
