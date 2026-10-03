//! CPU samples and packed offsets feed GPU meshing; collision keeps the CPU reference.
use crate::{Surface, World};
use blockloom_plugin_api::mesh::GpuVertices;
use blockloom_plugin_sdk::{Value, gpu};

pub fn build(
    world: &mut World,
    chunk: [i32; 3],
    base: [i32; 3],
    extent: [i32; 3],
    glow: Option<u8>,
    effects: &mut Vec<Value>,
) -> Option<GpuVertices> {
    if !world.settings.gpu_meshing || !world.grid.shaped_in(chunk).is_empty() {
        return None;
    }
    let lo = base.map(|v| {
        if world.surface == Surface::Smooth && v == 0 {
            -1
        } else {
            0
        }
    });
    let anchors = [0, 1, 2].map(|a| extent[a] - lo[a]);
    let n = anchors.map(|v| v + 3);
    let mut samples = Vec::with_capacity(n.iter().product::<i32>() as usize * 2);
    for z in 0..n[2] {
        for y in 0..n[1] {
            for x in 0..n[0] {
                let cell = [
                    base[0] + lo[0] - 1 + x,
                    base[1] + lo[1] - 1 + y,
                    base[2] + lo[2] - 1 + z,
                ];
                samples.push(i32::from(world.grid.density(cell)));
                samples.push(
                    if world.surface == Surface::Cubes && !world.grid.is_full(cell) {
                        0
                    } else {
                        i32::from(world.grid.get(cell))
                    },
                );
            }
        }
    }
    let offsets = packed_offsets(&samples, n, anchors, &world.palette, glow, world.surface);
    let vertices = *offsets.last().unwrap();
    if vertices == 0 || vertices as usize > blockloom_plugin_api::mesh::MAX_VERTICES {
        return None;
    }
    let words = vertices * 10;
    if world
        .gpu_buffers
        .values()
        .flatten()
        .map(|(_, w)| u64::from(*w))
        .sum::<u64>()
        + u64::from(words)
        > 16 * 1024 * 1024
    {
        return None;
    }
    world.serial += 1;
    let output = format!("vertices{}", world.serial);
    let input = format!("samples{}", world.serial);
    let params = format!("params{}", world.serial);
    let palette = format!("palette{}", world.serial);
    let offsets_buffer = format!("offsets{}", world.serial);
    let mut colors = vec![0.0; 256 * 4];
    for id in 1..=world.palette.len() {
        let material = world.palette.get(id).unwrap();
        let i = id as usize * 4;
        colors[i..i + 3].copy_from_slice(&material.color.map(|c| crate::round(c) as f32));
        colors[i + 3] = if material.emission > 0.0 {
            id as f32
        } else {
            0.0
        };
    }
    let parameters = [
        n[0] as f32,
        n[1] as f32,
        n[2] as f32,
        world.voxel,
        anchors[0] as f32,
        anchors[1] as f32,
        anchors[2] as f32,
        glow.unwrap_or(0) as f32,
        lo[0] as f32,
        lo[1] as f32,
        lo[2] as f32,
        if world.surface == Surface::Smooth {
            1.0
        } else {
            0.0
        },
    ];
    effects.extend([
        gpu::buffer(&input, samples.len() as u32),
        gpu::write_i32(&input, 0, &samples),
        gpu::buffer(&palette, colors.len() as u32),
        gpu::write_f32(&palette, 0, &colors),
        gpu::buffer(&params, 12),
        gpu::write_f32(&params, 0, &parameters),
        gpu::buffer(&offsets_buffer, offsets.len() as u32),
        gpu::write_u32(&offsets_buffer, 0, &offsets),
        gpu::buffer(&output, words),
        gpu::dispatch(
            "mesh",
            &[
                ("samples", &input),
                ("palette", &palette),
                ("params", &params),
                ("vertices", &output),
                ("offsets", &offsets_buffer),
            ],
            [(anchors.iter().product::<i32>() as u32).div_ceil(64), 1, 1],
        ),
        gpu::free(&input),
        gpu::free(&palette),
        gpu::free(&params),
        gpu::free(&offsets_buffer),
    ]);
    world
        .gpu_buffers
        .entry(chunk)
        .or_default()
        .push((output.clone(), words));
    Some(GpuVertices {
        buffer: output,
        vertices,
    })
}

// Count topology only. Interpolation and normals remain GPU work.
fn packed_offsets(
    samples: &[i32],
    n: [i32; 3],
    anchors: [i32; 3],
    palette: &crate::palette::Palette,
    glow: Option<u8>,
    surface: Surface,
) -> Vec<u32> {
    let sample = |p: [i32; 3]| {
        let i = ((p[2] * n[1] + p[1]) * n[0] + p[0]) as usize * 2;
        (samples[i], samples[i + 1] as u8)
    };
    let selected = |m| {
        palette
            .get(m)
            .is_some_and(|look| (look.emission > 0.0).then_some(m) == glow)
    };
    let mut offsets = Vec::with_capacity(anchors.iter().product::<i32>() as usize + 1);
    offsets.push(0);
    let mut total = 0;
    for z in 1..=anchors[2] {
        for y in 1..=anchors[1] {
            for x in 1..=anchors[0] {
                let at = [x, y, z];
                if surface == Surface::Cubes {
                    let (_, m) = sample(at);
                    if m != 0 && selected(m) {
                        for axis in 0..3 {
                            for sign in [-1, 1] {
                                let mut next = at;
                                next[axis] += sign;
                                if sample(next).1 == 0 {
                                    total += 6;
                                }
                            }
                        }
                    }
                } else {
                    let corners =
                        crate::smooth::CORNERS.map(|c| sample([0, 1, 2].map(|a| at[a] + c[a])));
                    for tet in crate::smooth::TETS {
                        let mut count = 0;
                        let mut material = None;
                        for c in tet {
                            if corners[c].0 < 0 {
                                count += 1;
                                material.get_or_insert(corners[c].1);
                            }
                        }
                        if (1..=3).contains(&count) && material.is_some_and(selected) {
                            total += if count == 2 { 6 } else { 3 };
                        }
                    }
                }
                offsets.push(total);
            }
        }
    }
    offsets
}
