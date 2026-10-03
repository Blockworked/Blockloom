//! GPU readback is used only here as a correctness oracle.
use blockloom_plugin_api::{compute::LoadoutKernel, manifest::Capability, schema::Contributions};
use blockloom_plugin_gpu::engine::{ComputeEngine, Report};
use blockloom_plugin_host::{
    native::{NativeModule, default_services},
    world::Effect,
};
use serde_json::json;
use std::collections::BTreeSet;
fn engine() -> ComputeEngine {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    ComputeEngine::new(device, queue)
}
fn run(surface: &str, size: [i32; 3], centre: [i32; 3]) {
    let native = unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::from([Capability::GpuCompute]),
            default_services("0.0.1".into()),
        )
    }
    .unwrap();
    let args = json!({"resources":[{"type_id":"world","payload":{"surface":surface,"preset":"empty","size":size,"gpu_meshing":true}}]});
    native.call_json("world.start", &args).unwrap();
    let answer = native
        .call_json(
            "sphere",
            &json!({"x":centre[0],"y":centre[1],"z":centre[2],"radius":3.2,"material":"stone"}),
        )
        .unwrap();
    let contributions: Contributions =
        serde_json::from_str(include_str!("../package/schemas/voxel.json")).unwrap();
    let mut engine = engine();
    assert!(
        engine
            .set_kernels(&[LoadoutKernel {
                plugin: "p".into(),
                schema: contributions.kernels[0].clone(),
                source: include_str!("../package/kernels/mesh.wgsl").into()
            }])
            .is_empty()
    );
    let mut meshes = Vec::new();
    for value in answer["effects"].as_array().unwrap() {
        let effect: Effect = serde_json::from_value(value.clone()).unwrap();
        if let Effect::Mesh(data) = &effect {
            data.check().unwrap();
            meshes.push(data.clone());
        }
        if let Some(command) = effect.gpu_command() {
            engine.submit("p", command.unwrap()).unwrap();
        }
    }
    assert!(!meshes.is_empty());
    for mesh in meshes {
        let gpu = mesh.gpu.as_ref().unwrap();
        engine
            .submit(
                "p",
                blockloom_plugin_api::compute::GpuCommand::Read {
                    buffer: gpu.buffer.clone(),
                    offset: 0,
                    words: gpu.vertices * 10,
                    tag: "oracle".into(),
                    as_type: blockloom_plugin_api::compute::ReadAs::F32,
                },
            )
            .unwrap();
        let mut reports = engine.run();
        for _ in 0..1000 {
            reports.extend(engine.collect());
            if reports.iter().any(|r| matches!(r, Report::Read { .. })) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut output = None;
        for report in reports {
            match report {
                Report::Read { words, .. } => {
                    output = Some(words.into_iter().map(f32::from_bits).collect::<Vec<_>>())
                }
                Report::Error { message, .. } => panic!("{message}"),
            }
        }
        check_geometry(surface, &mesh, output.expect("GPU read finished"));
    }
}

fn check_geometry(surface: &str, mesh: &blockloom_plugin_api::mesh::MeshData, output: Vec<f32>) {
    let mut actual = Vec::new();
    let mut area = 0.0;
    for tri in output.as_chunks::<30>().0 {
        let points = [
            [tri[0], tri[1], tri[2]],
            [tri[10], tri[11], tri[12]],
            [tri[20], tri[21], tri[22]],
        ];
        let (a, b) = (sub(points[1], points[0]), sub(points[2], points[0]));
        let cross = cross(a, b);
        let length = dot(cross, cross).sqrt();
        if length < 1e-8 {
            continue;
        }
        area += length / 2.0;
        for v in tri.as_chunks::<10>().0 {
            assert!(dot(cross, [v[3], v[4], v[5]]) > 0.0);
            assert_eq!(v[9], 1.0);
        }
        actual.push(points);
    }
    let mut expected = Vec::new();
    let mut reference_area = 0.0;
    for indices in mesh.indices.as_chunks::<3>().0 {
        let points = indices.map(|i| {
            let start = i as usize * 3;
            [
                mesh.positions[start],
                mesh.positions[start + 1],
                mesh.positions[start + 2],
            ]
        });
        let n = cross(sub(points[1], points[0]), sub(points[2], points[0]));
        reference_area += dot(n, n).sqrt() / 2.0;
        expected.push(points);
    }
    assert!(
        (reference_area - area).abs() < 0.01,
        "{surface}: {reference_area} vs {area}"
    );
    if surface == "smooth" {
        assert_eq!(actual.len(), expected.len());
        for (i, triangle) in expected.iter().enumerate() {
            let matched = actual.iter().position(|candidate| {
                triangle.iter().all(|p| {
                    candidate
                        .iter()
                        .any(|q| (0..3).all(|a| (p[a] - q[a]).abs() < 2e-5))
                })
            });
            assert!(matched.is_some(), "missing CPU triangle {i}: {triangle:?}");
            actual.swap_remove(matched.unwrap());
        }
    }
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] - b[i])
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
#[test]
#[ignore = "needs a GPU or lavapipe"]
fn gpu_cube_and_smooth_surfaces_match_cpu_geometry() {
    for surface in ["cubes", "smooth"] {
        run(surface, [16; 3], [7; 3]);
        run(surface, [33; 3], [31; 3]);
    }
}
