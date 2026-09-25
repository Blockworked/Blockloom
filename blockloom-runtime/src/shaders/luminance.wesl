// Scene luminance meter: one workgroup samples a 64x32 grid of the exposed
// image and writes its log-average and peak luminance.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> result: array<f32, 4>;

const GRID_X: u32 = 64u;
const GRID_Y: u32 = 32u;
const THREADS: u32 = 256u;

var<workgroup> log_sums: array<f32, 256>;
var<workgroup> peaks: array<f32, 256>;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(256, 1, 1)
fn meter(@builtin(local_invocation_index) index: u32) {
    let size = textureDimensions(source);
    var log_sum = 0.0;
    var peak = 0.0;
    for (var i = index; i < GRID_X * GRID_Y; i += THREADS) {
        let cell = vec2<f32>(f32(i % GRID_X) + 0.5, f32(i / GRID_X) + 0.5);
        let texel = vec2<u32>(cell / vec2<f32>(f32(GRID_X), f32(GRID_Y)) * vec2<f32>(size));
        let lum = max(luminance(textureLoad(source, min(texel, size - 1u), 0).rgb), 0.0);
        // Floor keeps pure black from sending the log average to zero.
        log_sum += log2(max(lum, 1.0e-4));
        peak = max(peak, lum);
    }
    log_sums[index] = log_sum;
    peaks[index] = peak;
    workgroupBarrier();

    for (var stride = THREADS / 2u; stride > 0u; stride /= 2u) {
        if index < stride {
            log_sums[index] += log_sums[index + stride];
            peaks[index] = max(peaks[index], peaks[index + stride]);
        }
        workgroupBarrier();
    }

    if index == 0u {
        result[0] = exp2(log_sums[0] / f32(GRID_X * GRID_Y));
        result[1] = peaks[0];
        result[2] = 1.0;
        result[3] = 0.0;
    }
}
