//! CPU samples feed bounded GPU meshing; collision keeps the CPU reference.
use crate::{Surface, World, grid::CHUNK};
use blockloom_plugin_api::mesh::GpuVertices;
use blockloom_plugin_sdk::{Value, gpu};

pub fn build(
    world: &mut World,
    chunk: [i32; 3],
    glow: Option<u8>,
    effects: &mut Vec<Value>,
) -> Option<GpuVertices> {
    if !world.settings.gpu_meshing || !world.grid.shaped_in(chunk).is_empty() {
        return None;
    }
    let base = chunk.map(|v| v * CHUNK);
    let lo = base.map(|v| {
        if world.surface == Surface::Smooth && v == 0 {
            -1
        } else {
            0
        }
    });
    let anchors = lo.map(|v| CHUNK - v);
    let n = anchors.map(|v| v + 3);
    let vertices = (anchors.iter().product::<i32>() * 36) as u32;
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
    let mut samples = Vec::new();
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
        gpu::buffer(&output, words),
        gpu::dispatch(
            "mesh",
            &[
                ("samples", &input),
                ("palette", &palette),
                ("params", &params),
                ("vertices", &output),
            ],
            [vertices.div_ceil(36 * 64), 1, 1],
        ),
        gpu::free(&input),
        gpu::free(&palette),
        gpu::free(&params),
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
