//! The render-world half of plugin GPU compute.
//!
//! Plugin modules run on the main thread and the device lives in the render
//! world, so the two meet at a [`ComputeLink`]: the main world files the
//! commands plugins asked for, the render world runs them once a frame on its
//! own device through [`ComputeEngine`], and what comes back (errors, finished
//! reads) waits there for the main world to hand to the plugins.

use crate::bridge;
use crate::engine::Engine;
use bevy::prelude::*;
use bevy::render::mesh::{RenderMesh, allocator::MeshAllocator};
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use blockloom_plugin_api::compute::{GpuCommand, LoadoutKernel};
use blockloom_plugin_gpu::engine::{ComputeEngine, Report};
use blockloom_protocol::RuntimeMessage;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const COPY_BYTES_PER_FRAME: u64 = blockloom_plugin_api::mesh::MAX_VERTICES as u64 * 40;
const COPY_MESHES_PER_FRAME: usize = 64;

struct MeshBinding {
    plugin: String,
    buffer: String,
    vertices: u32,
    order: u64,
}

#[derive(Resource, Default)]
struct VisibleMeshes(HashSet<AssetId<Mesh>>);

#[derive(Default)]
struct Link {
    /// A new kernel set for the engine to build.
    kernels: Option<Vec<LoadoutKernel>>,
    commands: Vec<(String, GpuCommand)>,
    /// The run ended: buffers and reads go, kernels stay.
    clear: bool,
    reports: Vec<Report>,
    /// Whether a device is there to compute on, once known.
    available: Option<bool>,
    meshes: HashMap<AssetId<Mesh>, MeshBinding>,
    mesh_order: u64,
    completed_meshes: Vec<(String, u32)>,
}

/// What the two worlds share.
#[derive(Resource, Clone, Default)]
pub struct ComputeLink(Arc<Mutex<Link>>);

impl ComputeLink {
    pub fn set_kernels(&self, kernels: Vec<LoadoutKernel>) {
        self.0.lock().unwrap().kernels = Some(kernels);
    }

    pub fn push(&self, commands: Vec<(String, GpuCommand)>) {
        self.0.lock().unwrap().commands.extend(commands);
    }

    pub fn clear(&self) {
        let mut link = self.0.lock().unwrap();
        link.clear = true;
        link.commands.clear();
        link.meshes.clear();
        link.completed_meshes.clear();
    }

    pub fn bind_mesh(&self, asset: AssetId<Mesh>, plugin: &str, buffer: &str, vertices: u32) {
        let mut link = self.0.lock().unwrap();
        let order = link.mesh_order;
        link.mesh_order += 1;
        link.meshes.insert(
            asset,
            MeshBinding {
                plugin: plugin.into(),
                buffer: buffer.into(),
                vertices,
                order,
            },
        );
    }

    pub(crate) fn bound_assets(&self) -> HashSet<AssetId<Mesh>> {
        self.0.lock().unwrap().meshes.keys().copied().collect()
    }
    pub fn unbind_mesh(&self, asset: AssetId<Mesh>) {
        self.0.lock().unwrap().meshes.remove(&asset);
    }

    pub fn take_reports(&self) -> Vec<Report> {
        std::mem::take(&mut self.0.lock().unwrap().reports)
    }

    /// `Some(false)` when there is no device to compute on.
    pub fn available(&self) -> Option<bool> {
        self.0.lock().unwrap().available
    }

    fn set_available(&self, available: bool) {
        self.0.lock().unwrap().available = Some(available);
    }
}

pub fn register(app: &mut App) {
    let link = ComputeLink::default();
    app.insert_resource(link.clone()).add_systems(Update, feed);
    match app.get_sub_app_mut(RenderApp) {
        Some(render) => {
            render
                .insert_resource(link)
                .init_resource::<VisibleMeshes>()
                .add_systems(ExtractSchedule, extract_visible_meshes)
                .add_systems(Render, run.in_set(RenderSystems::PrepareBindGroups));
        }
        // A world without a renderer has nothing to compute on.
        None => link.set_available(false),
    }
}

fn extract_visible_meshes(
    mut visible: ResMut<VisibleMeshes>,
    meshes: Extract<Query<(&Mesh3d, &ViewVisibility, &InheritedVisibility)>>,
) {
    visible.0.clear();
    for (mesh, view, inherited) in &meshes {
        if inherited.get() && view.get() {
            visible.0.insert(mesh.id());
        }
    }
}

#[derive(Default)]
struct CopyBudget {
    bytes: u64,
    meshes: usize,
}

impl CopyBudget {
    fn admit(&mut self, vertices: u32) -> bool {
        let bytes = u64::from(vertices) * 40;
        if self.meshes >= COPY_MESHES_PER_FRAME || self.bytes + bytes > COPY_BYTES_PER_FRAME {
            return false;
        }
        self.bytes += bytes;
        self.meshes += 1;
        true
    }
}

fn run(
    link: Res<ComputeLink>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut engine: Local<Option<ComputeEngine>>,
    allocator: Res<MeshAllocator>,
    meshes: Res<RenderAssets<RenderMesh>>,
    visible: Res<VisibleMeshes>,
    depth: Option<Res<crate::plugin_lod::DepthRequests>>,
    mut copied: Local<HashSet<AssetId<Mesh>>>,
    mut failed: Local<HashSet<String>>,
) {
    let (kernels, commands, clear) = {
        let mut shared = link.0.lock().unwrap();
        (
            shared.kernels.take(),
            std::mem::take(&mut shared.commands),
            std::mem::take(&mut shared.clear),
        )
    };
    if engine.is_none() {
        // Nothing has asked yet: do not build a pipeline cache for no one.
        if kernels.as_ref().is_none_or(Vec::is_empty) && commands.is_empty() {
            return;
        }
        *engine = Some(ComputeEngine::new(
            device.wgpu_device().clone(),
            (**queue).clone(),
        ));
        link.set_available(true);
    }
    let engine = engine.as_mut().unwrap();
    let mut reports = Vec::new();
    if let Some(kernels) = kernels {
        for message in engine.set_kernels(&kernels) {
            reports.push(Report::Error {
                plugin: String::new(),
                message,
            });
        }
    }
    if clear {
        engine.clear();
        copied.clear();
        failed.clear();
    }
    for (plugin, command) in commands {
        if let Err(message) = engine.submit(&plugin, command) {
            reports.push(Report::Error { plugin, message });
        }
    }
    reports.extend(engine.run());
    for report in &reports {
        if let Report::Error { plugin, .. } = report {
            failed.insert(plugin.clone());
        }
    }
    // Prepared Bevy meshes use position, normal, color in a 40-byte vertex.
    let shared = link.0.lock().unwrap();
    copied.retain(|id| shared.meshes.contains_key(id));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("plugin GPU meshes"),
    });
    let mut any = false;
    let mut completed = Vec::new();
    let mut pending: Vec<_> = shared
        .meshes
        .iter()
        .filter(|(id, binding)| {
            visible.0.contains(id)
                && depth.as_ref().is_none_or(|depth| depth.allows(id))
                && !copied.contains(id)
                && !failed.contains(&binding.plugin)
                && !failed.contains("")
        })
        .collect();
    pending.sort_unstable_by_key(|(_, binding)| binding.order);
    let mut budget = CopyBudget::default();
    for (id, binding) in pending {
        let MeshBinding {
            plugin,
            buffer,
            vertices,
            ..
        } = binding;
        if budget.meshes == COPY_MESHES_PER_FRAME {
            break;
        }
        let Some(mesh) = meshes.get(*id) else {
            continue;
        };
        let layout = mesh.layout.0.layout();
        if layout.array_stride != 40 {
            continue;
        }
        let Some(source) = engine.mesh_buffer(plugin, buffer, vertices * 10) else {
            continue;
        };
        let Some(target) = allocator.mesh_vertex_slice(id) else {
            continue;
        };
        if target.range.end - target.range.start < *vertices {
            continue;
        }
        if !budget.admit(*vertices) {
            // Keep FIFO order among ready meshes, including a large oldest mesh.
            break;
        }
        encoder.copy_buffer_to_buffer(
            source,
            0,
            target.buffer,
            u64::from(target.range.start) * 40,
            u64::from(*vertices) * 40,
        );
        copied.insert(*id);
        completed.push((plugin.clone(), *vertices));
        any = true;
    }
    drop(shared);
    if any {
        queue.submit([encoder.finish()]);
        link.0.lock().unwrap().completed_meshes.extend(completed);
    }
    if !reports.is_empty() {
        link.0.lock().unwrap().reports.extend(reports);
    }
}

/// Main world: files the plugins' commands and the kernel set with the
/// render world, and hands back what it reports.
fn feed(
    mut engine: NonSendMut<Engine>,
    link: Res<ComputeLink>,
    mut seen: Local<u64>,
    mut warned: Local<bool>,
) {
    if engine.plugins.kernels_serial != *seen {
        *seen = engine.plugins.kernels_serial;
        link.set_kernels(engine.plugins.loadout.kernels.clone());
    }
    if std::mem::take(&mut engine.plugins.gpu_clear) {
        link.clear();
    }
    let commands = std::mem::take(&mut engine.plugins.gpu);
    if !commands.is_empty() {
        if link.available() == Some(false) {
            if !std::mem::replace(&mut *warned, true) {
                bridge::send(&RuntimeMessage::Error {
                    actor: String::new(),
                    message: "plugin GPU compute needs a rendering world, and this one has none"
                        .to_string(),
                });
            }
        } else {
            link.push(commands);
        }
    }
    for (plugin, vertices) in std::mem::take(&mut link.0.lock().unwrap().completed_meshes) {
        let _ = engine.plugins.diagnostics.count(&plugin, "gpu_meshes", 1);
        let _ = engine
            .plugins
            .diagnostics
            .count(&plugin, "gpu_mesh_vertices", i64::from(vertices));
    }
    let reports = link.take_reports();
    if !reports.is_empty() {
        crate::plugins::gpu_reports(&mut engine, reports);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::MeshOp;
    use blockloom_core::scene::Mode;
    use blockloom_plugin_api::mesh::{ColliderKind, GpuVertices, MeshData};

    #[test]
    fn mesh_copy_budget_bounds_bytes_and_submissions_without_starving_large_meshes() {
        let max = blockloom_plugin_api::mesh::MAX_VERTICES as u32;
        let mut budget = CopyBudget::default();
        assert!(budget.admit(max));
        assert!(!budget.admit(3));
        assert_eq!(budget.bytes, COPY_BYTES_PER_FRAME);
        let mut budget = CopyBudget::default();
        for _ in 0..COPY_MESHES_PER_FRAME {
            assert!(budget.admit(3));
        }
        assert!(!budget.admit(3));
        assert!(CopyBudget::default().admit(max));
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn gpu_mesh_buffers_copy_into_prepared_bevy_meshes() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
                .disable::<bevy::log::LogPlugin>()
                .disable::<bevy::audio::AudioPlugin>(),
        );
        let began = std::time::Instant::now();
        while app.plugins_state() == bevy::app::PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
            assert!(
                began.elapsed() < std::time::Duration::from_secs(30),
                "renderer initialization timed out"
            );
        }
        app.finish();
        app.cleanup();
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        let words = vec![
            2.0f32, 0., 0., 0., 0., 1., 1., 0., 0., 1., 3., 0., 0., 0., 0., 1., 1., 0., 0., 1., 2.,
            1., 0., 0., 0., 1., 1., 0., 0., 1.,
        ];
        engine.plugins.gpu = vec![
            (
                "p".into(),
                GpuCommand::Buffer {
                    name: "out".into(),
                    words: 30,
                },
            ),
            (
                "p".into(),
                GpuCommand::Write {
                    buffer: "out".into(),
                    offset: 0,
                    data: words.iter().map(|v| v.to_bits()).collect(),
                },
            ),
        ];
        engine.plugins.meshes.push(MeshOp::Put {
            plugin: "p".into(),
            mesh: MeshData {
                name: "triangle".into(),
                positions: vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
                normals: [0., 0., 1.].repeat(3),
                colors: vec![1.; 12],
                indices: vec![0, 1, 2],
                origin: [0.; 3],
                emission: None,
                roughness: 0.9,
                transition_ms: 0,
                collider: false,
                collider_kind: ColliderKind::Trimesh,
                uvs: Vec::new(),
                texture: None,
                gpu: Some(GpuVertices {
                    buffer: "out".into(),
                    vertices: 3,
                    quads: None,
                }),
                body: None,
            },
        });
        app.insert_non_send(engine)
            .init_resource::<crate::plugin_meshes::PluginMeshes>()
            .add_systems(Update, crate::plugin_meshes::sync);
        register(&mut app);
        crate::plugin_lod::register(&mut app);
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                32,
                32,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                None,
            ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                bevy::core_pipeline::prepass::DepthPrepass,
                bevy::render::occlusion_culling::OcclusionCulling,
                Msaa::Off,
                bevy::camera::RenderTarget::from(image),
                Transform::from_xyz(1.5, 0.5, 5.0).looking_at(Vec3::new(1.5, 0.5, 0.0), Vec3::Y),
            ))
            .id();
        app.world_mut()
            .non_send_mut::<Engine>()
            .plugins
            .meshes
            .push(MeshOp::Visibility {
                plugin: "p".into(),
                name: "triangle".into(),
                visible: false,
            });
        for _ in 0..8 {
            app.update();
        }
        assert!(
            !app.world()
                .non_send::<Engine>()
                .plugins
                .diagnostics
                .metrics()
                .iter()
                .any(|(name, _)| name == "plugins/p/gpu_meshes")
        );
        assert_eq!(
            app.world()
                .resource::<ComputeLink>()
                .0
                .lock()
                .unwrap()
                .meshes
                .len(),
            1
        );
        app.world_mut()
            .non_send_mut::<Engine>()
            .plugins
            .meshes
            .push(MeshOp::Visibility {
                plugin: "p".into(),
                name: "triangle".into(),
                visible: true,
            });
        *app.world_mut().get_mut::<Transform>(camera).unwrap() =
            Transform::from_xyz(100.0, 0.5, 5.0).looking_at(Vec3::new(100.0, 0.5, 0.0), Vec3::Y);
        for _ in 0..8 {
            app.update();
        }
        assert!(
            !app.world()
                .non_send::<Engine>()
                .plugins
                .diagnostics
                .metrics()
                .iter()
                .any(|(name, _)| name == "plugins/p/gpu_meshes")
        );
        app.world_mut()
            .non_send_mut::<Engine>()
            .plugins
            .meshes
            .push(MeshOp::Instances {
                plugin: "p".into(),
                set: blockloom_plugin_api::rendering::InstanceData {
                    name: "visible-copy".into(),
                    mesh: "triangle".into(),
                    positions: vec![100.0, 0.0, 0.0],
                    yaw: vec![],
                    scales: vec![],
                },
            });
        for _ in 0..8 {
            app.update();
        }
        assert!(
            app.world()
                .non_send::<Engine>()
                .plugins
                .diagnostics
                .metrics()
                .iter()
                .any(|(name, value)| name == "plugins/p/gpu_meshes" && *value == 1.0)
        );
        let id = *app
            .world()
            .resource::<ComputeLink>()
            .0
            .lock()
            .unwrap()
            .meshes
            .keys()
            .next()
            .unwrap();
        let render = app.get_sub_app_mut(RenderApp).unwrap();
        let target = {
            let allocator = render.world().resource::<MeshAllocator>();
            let slice = allocator.mesh_vertex_slice(&id).unwrap();
            (slice.buffer.clone(), u64::from(slice.range.start) * 40)
        };
        let device = render.world().resource::<RenderDevice>();
        let queue = render.world().resource::<RenderQueue>();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 120,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(&target.0, target.1, &staging, 0, 120);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = staging.slice(..).get_mapped_range().unwrap();
        assert_eq!(bytemuck::cast_slice::<u8, f32>(&data), words.as_slice());
    }
}
