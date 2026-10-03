//! Persistent cube quads or smooth topology spans feed GPU rendering.
use crate::{Surface, World};
use blockloom_plugin_api::mesh::{CompactQuads, GpuVertices};
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
    let (offsets, vertices, invocations) = if world.surface == Surface::Cubes {
        let mut quads = Vec::new();
        crate::mesher::cube_quads(
            &world.palette,
            [0; 3],
            extent,
            |p| {
                let p = p.map(|v| v + 1);
                samples[((p[2] * n[1] + p[1]) * n[0] + p[0]) as usize * 2 + 1] as u8
            },
            |q, m| {
                if (world.palette.get(m).unwrap().emission > 0.0).then_some(m) == glow {
                    // Byte coordinates cover both fine tiles and coarse LOD tiles.
                    let face = q.axis as u32 * 2 + u32::from(q.sign > 0);
                    quads.extend([
                        q.plane as u32
                            | (q.at[0] as u32) << 8
                            | (q.at[1] as u32) << 16
                            | face << 24,
                        q.size[0] as u32 | (q.size[1] as u32) << 8 | u32::from(m) << 16,
                    ]);
                }
            },
        );
        let count = quads.len() as u32 / 2;
        (quads, count * 6, count)
    } else {
        let offsets = packed_offsets(&samples, n, anchors, &world.palette, glow);
        let vertices = *offsets.last().unwrap();
        (offsets, vertices, anchors.iter().product::<i32>() as u32)
    };
    if vertices == 0 || vertices as usize > blockloom_plugin_api::mesh::MAX_VERTICES {
        return None;
    }
    let words = if world.surface == Surface::Cubes {
        offsets.len() as u32 * 6
            + 4 * offsets
                .as_chunks::<2>()
                .0
                .iter()
                .map(|r| (r[1] >> 16) & 255)
                .collect::<std::collections::BTreeSet<_>>()
                .len() as u32
    } else {
        vertices * 10
    };
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
    if world.surface == Surface::Cubes {
        let mut palette_ids = std::collections::BTreeMap::new();
        let mut compact_palette = Vec::new();
        let records = offsets
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[record, dimensions]| {
                let material = (dimensions >> 16) & 255;
                let next = palette_ids.len() as u32;
                let index = *palette_ids.entry(material).or_insert_with(|| {
                    let m = material as usize * 4;
                    compact_palette.push([colors[m], colors[m + 1], colors[m + 2], 1.0]);
                    next
                });
                [*record, (dimensions & 65535) | index << 16]
            })
            .collect();
        world
            .gpu_buffers
            .entry(chunk)
            .or_default()
            .push((None, words));
        return Some(GpuVertices {
            buffer: output,
            vertices,
            quads: Some(CompactQuads {
                records,
                palette: compact_palette,
                voxel: world.voxel,
            }),
        });
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
        invocations as f32,
    ];
    effects.extend([
        gpu::buffer(&input, samples.len() as u32),
        gpu::write_i32(&input, 0, &samples),
        gpu::buffer(&palette, colors.len() as u32),
        gpu::write_f32(&palette, 0, &colors),
        gpu::buffer(&params, 13),
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
            [invocations.div_ceil(64), 1, 1],
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
        .push((Some(output.clone()), words));
    Some(GpuVertices {
        buffer: output,
        vertices,
        quads: None,
    })
}

// Count topology only. Interpolation and normals remain GPU work.
fn packed_offsets(
    samples: &[i32],
    n: [i32; 3],
    anchors: [i32; 3],
    palette: &crate::palette::Palette,
    glow: Option<u8>,
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
                offsets.push(total);
            }
        }
    }
    offsets
}

/// Pack axis-aligned rectangular triangle pairs, including clipped cube LOD seams.
pub fn visual(group: &crate::mesher::Group, voxel: f32) -> Option<GpuVertices> {
    if group.indices.is_empty() || group.indices.len() % 6 != 0 {
        return None;
    }
    let mut records = Vec::new();
    let mut palette = Vec::<[f32; 4]>::new();
    for pair in group.indices.chunks_exact(6) {
        let index = pair[0] as usize;
        let normal = &group.normals[index * 3..index * 3 + 3];
        let axis = (0..3).find(|&a| normal[a].abs() == 1.0)?;
        if (0..3).any(|a| a != axis && normal[a] != 0.0) {
            return None;
        }
        let color: [f32; 4] = group.colors[index * 4..index * 4 + 4].try_into().ok()?;
        let material = if let Some(index) = palette.iter().position(|p| *p == color) {
            index
        } else {
            if palette.len() == 256 {
                return None;
            }
            palette.push(color);
            palette.len() - 1
        };
        let mut points = Vec::new();
        for &i in pair {
            let i = i as usize;
            if group.normals[i * 3..i * 3 + 3] != *normal || group.colors[i * 4..i * 4 + 4] != color
            {
                return None;
            }
            let p: [f32; 3] = std::array::from_fn(|a| group.positions[i * 3 + a] / voxel);
            if p.iter().any(|p| {
                !p.is_finite() || (*p - p.round()).abs() > 1e-4 || !(0.0..=128.0).contains(p)
            }) {
                return None;
            }
            points.push(p.map(|p| p.round() as u32));
        }
        let triangle_a: std::collections::BTreeSet<_> = points[..3].iter().copied().collect();
        let triangle_b: std::collections::BTreeSet<_> = points[3..].iter().copied().collect();
        let shared: Vec<_> = triangle_a.intersection(&triangle_b).copied().collect();
        let unique: std::collections::BTreeSet<_> = points.into_iter().collect();
        if unique.len() != 4 || shared.len() != 2 || triangle_a.len() != 3 || triangle_b.len() != 3
        {
            return None;
        }
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let lo: [u32; 3] = std::array::from_fn(|a| unique.iter().map(|p| p[a]).min().unwrap());
        let hi: [u32; 3] = std::array::from_fn(|a| unique.iter().map(|p| p[a]).max().unwrap());
        if lo[axis] != hi[axis]
            || lo[u] == hi[u]
            || lo[v] == hi[v]
            || unique
                .iter()
                .any(|p| (p[u] != lo[u] && p[u] != hi[u]) || (p[v] != lo[v] && p[v] != hi[v]))
            || shared[0][u] == shared[1][u]
            || shared[0][v] == shared[1][v]
        {
            return None;
        }
        let face = axis as u32 * 2 + u32::from(normal[axis] > 0.0);
        records.push([
            lo[axis] | lo[u] << 8 | lo[v] << 16 | face << 24,
            (hi[u] - lo[u]) | (hi[v] - lo[v]) << 8 | (material as u32) << 16,
        ]);
    }
    Some(GpuVertices {
        buffer: "quads".into(),
        vertices: records.len() as u32 * 6,
        quads: Some(CompactQuads {
            records,
            palette,
            voxel,
        }),
    })
}

#[cfg(test)]
mod compact_tests {
    use super::*;
    #[test]
    fn lod_rectangles_preserve_exact_extents_and_fractional_shapes_fall_back() {
        let mut group = crate::mesher::Group::default();
        group.push_poly(
            &[
                [128.0, 0.0, 0.0],
                [128.0, 32.0, 0.0],
                [128.0, 32.0, 127.0],
                [128.0, 0.0, 127.0],
            ],
            [1.0, 0.0, 0.0],
            [0.5, 0.25, 0.125],
        );
        let quad = visual(&group, 1.0).unwrap().quads.unwrap();
        assert_eq!(quad.records, vec![[128 | 1 << 24, 32 | 127 << 8]]);
        assert_eq!(quad.palette, vec![[0.5, 0.25, 0.125, 1.0]]);
        group.positions[0] = 127.5;
        assert!(visual(&group, 1.0).is_none());
    }
}
