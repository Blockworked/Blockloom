//! Depth-pyramid traversal and bounded, asynchronous mesh upload requests.
use crate::plugins::LodBridge;
use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::{
    mip_generation::experimental::depth::{ViewDepthPyramid, early_downsample_depth},
    schedule::Core3d,
};
use bevy::prelude::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::view::ExtractedView;
use bevy::render::{Extract, ExtractSchedule, RenderApp};
use blockloom_plugin_api::lod::Feedback;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

const MAX_INSTANCES: usize = 2048;
const MAX_REQUESTS: usize = 512;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    Mesh(AssetId<Mesh>),
    Tile {
        plugin: String,
        set: String,
        generation: u64,
        revision: u64,
        id: String,
    },
}
#[derive(Clone, PartialEq)]
struct Candidate {
    target: Target,
    lo: Vec3,
    hi: Vec3,
}
#[derive(Clone, Default)]
struct Input {
    candidates: Vec<Candidate>,
    targets: Vec<Target>,
    epoch: u64,
}
#[derive(Default)]
struct Shared {
    input: Input,
    clip: Option<Mat4>,
    ready: Option<(u64, Mat4, u64, HashSet<Target>)>,
    frame: u64,
    in_flight: usize,
}
#[derive(Resource, Clone, Default)]
pub struct DepthRequests(Arc<Mutex<Shared>>);
impl DepthRequests {
    pub fn allows(&self, asset: &AssetId<Mesh>) -> bool {
        let state = self.0.lock().unwrap();
        state
            .ready
            .as_ref()
            .is_none_or(|(epoch, clip, frame, ready)| {
                *epoch != state.input.epoch
                    || Some(*clip) != state.clip
                    || state.frame.saturating_sub(*frame) > 3
                    || ready.contains(&Target::Mesh(*asset))
            })
    }
}

pub fn register(app: &mut App) {
    let shared = DepthRequests::default();
    app.insert_resource(shared.clone());
    app.init_resource::<LodBridge>();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .insert_resource(shared)
            .add_systems(ExtractSchedule, extract)
            .add_systems(
                Core3d,
                traverse
                    .after(early_downsample_depth)
                    .before(bevy::core_pipeline::Core3dSystems::MainPass),
            );
    }
}
fn extract(
    shared: Res<DepthRequests>,
    lod: Extract<Res<LodBridge>>,
    link: Res<crate::plugin_compute::ComputeLink>,
    cameras: Extract<
        Query<
            (
                &GlobalTransform,
                &Projection,
                &Camera,
                Option<&bevy::render::occlusion_culling::OcclusionCulling>,
            ),
            With<Camera3d>,
        >,
    >,
    meshes: Extract<
        Query<(
            &Mesh3d,
            &Aabb,
            &GlobalTransform,
            &ViewVisibility,
            &InheritedVisibility,
        )>,
    >,
) {
    let bound = link.bound_assets();
    let mut candidates = Vec::new();
    for (mesh, aabb, pose, view, inherited) in &meshes {
        if !bound.contains(&mesh.id()) || !view.get() || !inherited.get() {
            continue;
        }
        let transform = pose.affine();
        let center = transform.transform_point3a(aabb.center);
        let half = transform.matrix3.abs() * aabb.half_extents;
        candidates.push(Candidate {
            target: Target::Mesh(mesh.id()),
            lo: (center - half).into(),
            hi: (center + half).into(),
        });
    }
    let mut lod = lod.0.lock().unwrap();
    for ((plugin, name), set) in &lod.sets {
        for tile in &set.tiles {
            candidates.push(Candidate {
                target: Target::Tile {
                    plugin: plugin.clone(),
                    set: name.clone(),
                    generation: set.generation,
                    revision: set.revision,
                    id: tile.id.clone(),
                },
                lo: Vec3::from_array(tile.min),
                hi: Vec3::from_array(tile.max),
            });
        }
    }
    candidates.sort_by_key(|c| {
        (
            format!("{:?}", c.target),
            c.lo.to_array().map(f32::to_bits),
            c.hi.to_array().map(f32::to_bits),
        )
    });
    let mut state = shared.0.lock().unwrap();
    state.frame += 1;
    if candidates != state.input.candidates {
        state.input.epoch += 1;
        state.ready = None;
        let mut seen = HashSet::new();
        state.input.targets = candidates
            .iter()
            .filter_map(|c| seen.insert(c.target.clone()).then_some(c.target.clone()))
            .collect();
        state.input.candidates = candidates;
    }
    let mut cameras = cameras.iter().filter(|(_, _, camera, _)| camera.is_active);
    state.clip = cameras.next().and_then(|(pose, projection, _, occlusion)| {
        occlusion.map(|_| projection.get_clip_from_view() * pose.to_matrix().inverse())
    });
    if cameras.next().is_some() || state.clip.is_none() {
        state.clip = None;
        state.ready = None;
    }
    lod.clip = state.clip;
    lod.feedback = state.feedback(&lod.sets);
}

impl Shared {
    fn feedback(
        &self,
        sets: &std::collections::BTreeMap<(String, String), blockloom_plugin_api::lod::TileSet>,
    ) -> std::collections::BTreeMap<String, Vec<Feedback>> {
        let mut feedback = std::collections::BTreeMap::<String, Vec<Feedback>>::new();
        let Some((epoch, clip, frame, ready)) = &self.ready else {
            return feedback;
        };
        if self.input.candidates.len() > MAX_INSTANCES
            || *epoch != self.input.epoch
            || Some(*clip) != self.clip
            || self.frame.saturating_sub(*frame) > 3
        {
            return feedback;
        }
        for ((plugin, _), set) in sets {
            let visible = set
                .tiles
                .iter()
                .filter_map(|tile| {
                    ready
                        .contains(&Target::Tile {
                            plugin: plugin.clone(),
                            set: set.name.clone(),
                            generation: set.generation,
                            revision: set.revision,
                            id: tile.id.clone(),
                        })
                        .then_some(tile.id.clone())
                })
                .collect();
            feedback.entry(plugin.clone()).or_default().push(Feedback {
                name: set.name.clone(),
                generation: set.generation,
                revision: set.revision,
                visible,
            });
        }
        feedback
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Node {
    lo: [f32; 4],
    hi: [f32; 4],
    links: [u32; 4],
}
fn tree(candidates: &mut [Candidate], assets: &[Target], nodes: &mut Vec<Node>) -> u32 {
    let index = nodes.len() as u32;
    nodes.push(Node::default());
    let (lo, hi) = candidates.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), c| (lo.min(c.lo), hi.max(c.hi)),
    );
    let links = if candidates.len() == 1 {
        [
            0,
            0,
            assets
                .iter()
                .position(|id| *id == candidates[0].target)
                .unwrap() as u32,
            1,
        ]
    } else {
        let extent = hi - lo;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        candidates.sort_by(|a, b| (a.lo[axis] + a.hi[axis]).total_cmp(&(b.lo[axis] + b.hi[axis])));
        let mid = candidates.len() / 2;
        let (a, b) = candidates.split_at_mut(mid);
        [tree(a, assets, nodes), tree(b, assets, nodes), 0, 0]
    };
    nodes[index as usize] = Node {
        lo: lo.extend(0.0).to_array(),
        hi: hi.extend(0.0).to_array(),
        links,
    };
    index
}
struct Pipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}
fn pipeline(device: &wgpu::Device) -> Pipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("plugin LOD traversal"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/plugin_lod_traverse.wgsl").into()),
    });
    let entries = (0..4)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: if binding == 2 {
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                }
            } else {
                wgpu::BindingType::Buffer {
                    ty: if binding == 1 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding == 0,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }
            },
            count: None,
        })
        .collect::<Vec<_>>();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("plugin LOD traversal"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    Pipeline { layout, pipeline }
}
fn traverse(
    view: ViewQuery<(&ExtractedView, &ViewDepthPyramid)>,
    shared: Res<DepthRequests>,
    mut prepared: Local<Option<Pipeline>>,
    mut ctx: RenderContext,
) {
    let (view, pyramid) = view.into_inner();
    let clip = view
        .clip_from_world
        .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
    let (input, frame) = {
        let mut state = shared.0.lock().unwrap();
        if state.clip != Some(clip) {
            return;
        }
        if state.input.candidates.is_empty()
            || state.input.candidates.len() > MAX_INSTANCES
            || state.in_flight >= 2
        {
            return;
        }
        state.in_flight += 1;
        (state.input.clone(), state.frame)
    };
    let device = ctx.render_device().wgpu_device();
    let pipeline = prepared.get_or_insert_with(|| pipeline(device));
    let mut nodes = Vec::with_capacity(input.candidates.len() * 2 - 1);
    tree(&mut input.candidates.clone(), &input.targets, &mut nodes);
    let nodes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("plugin LOD nodes"),
        contents: bytemuck::cast_slice(&nodes),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let mut params = clip.to_cols_array().map(f32::to_bits).to_vec();
    params.extend([
        input.candidates.len() as u32 * 2 - 1,
        pyramid.mip_count,
        0,
        0,
    ]);
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let words = 2 + MAX_REQUESTS + input.targets.len();
    let output = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("plugin LOD request queue"),
        contents: bytemuck::cast_slice(&vec![0u32; words]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: nodes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&pyramid.all_mips),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("plugin LOD request readback"),
        size: ((2 + MAX_REQUESTS) * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("plugin LOD traversal"),
                timestamp_writes: None,
            });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    ctx.command_encoder()
        .copy_buffer_to_buffer(&output, 0, &staging, 0, staging.size());
    let state = shared.clone();
    let mapped = staging.clone();
    ctx.command_encoder()
        .map_buffer_on_submit(&staging, wgpu::MapMode::Read, .., move |result| {
            let mut state = state.0.lock().unwrap();
            state.in_flight -= 1;
            if result.is_ok() && state.input.epoch == input.epoch && state.clip == Some(clip) {
                let data = mapped.get_mapped_range(..).unwrap();
                let words: &[u32] = bytemuck::cast_slice(&data);
                if words[1] == 0 && words[0] as usize <= MAX_REQUESTS {
                    let ready = words[2..2 + words[0] as usize]
                        .iter()
                        .filter_map(|id| input.targets.get(*id as usize).cloned())
                        .collect();
                    state.ready = Some((input.epoch, clip, frame, ready));
                } else {
                    state.ready = None;
                }
            }
            mapped.unmap();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_feedback_is_scoped_versioned_and_expires() {
        use blockloom_plugin_api::lod::{Tile, TileSet};
        let lod = LodBridge::default();
        let set = TileSet {
            name: "visual".into(),
            generation: 1,
            revision: 2,
            tiles: vec![Tile {
                id: "tile".into(),
                min: [0.; 3],
                max: [1.; 3],
            }],
        };
        lod.submit("a".into(), set.clone());
        lod.submit("b".into(), set.clone());
        let target = Target::Tile {
            plugin: "a".into(),
            set: set.name.clone(),
            generation: 1,
            revision: 2,
            id: "tile".into(),
        };
        let mut state = Shared {
            clip: Some(Mat4::IDENTITY),
            ready: Some((0, Mat4::IDENTITY, 0, HashSet::from([target]))),
            ..default()
        };
        let sets = &lod.0.lock().unwrap().sets.clone();
        let feedback = state.feedback(sets);
        assert_eq!(feedback["a"][0].visible, ["tile"]);
        assert!(feedback["b"][0].visible.is_empty());
        state.frame = 4;
        assert!(state.feedback(sets).is_empty());
        state.frame = 0;
        state.input.epoch += 1;
        assert!(state.feedback(sets).is_empty());
        state.input.epoch = 0;
        state.clip = None;
        assert!(state.feedback(sets).is_empty());
        lod.0.lock().unwrap().feedback = feedback;
        lod.submit(
            "a".into(),
            TileSet {
                revision: 3,
                ..set.clone()
            },
        );
        assert!(lod.0.lock().unwrap().feedback.is_empty());
        lod.submit(
            "a".into(),
            TileSet {
                tiles: Vec::new(),
                ..set
            },
        );
        assert_eq!(lod.0.lock().unwrap().sets.len(), 1);
        lod.clear();
        assert!(lod.0.lock().unwrap().sets.is_empty());
    }

    #[test]
    fn traversal_shader_is_valid_and_feedback_expires() {
        let shader =
            naga::front::wgsl::parse_str(include_str!("shaders/plugin_lod_traverse.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&shader)
        .unwrap();
        let mut meshes = Assets::<Mesh>::default();
        let asset = meshes.add(Cuboid::default()).id();
        let requests = DepthRequests::default();
        {
            let mut state = requests.0.lock().unwrap();
            state.clip = Some(Mat4::IDENTITY);
            state.ready = Some((0, Mat4::IDENTITY, 0, HashSet::new()));
        }
        assert!(!requests.allows(&asset));
        requests.0.lock().unwrap().frame = 4;
        assert!(requests.allows(&asset));
        requests.0.lock().unwrap().frame = 0;
        requests.0.lock().unwrap().input.epoch = 1;
        assert!(requests.allows(&asset));
        requests.0.lock().unwrap().input.epoch = 0;
        requests.0.lock().unwrap().clip = None;
        assert!(requests.allows(&asset));
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn depth_queue_culls_deduplicates_and_reports_overflow() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
                .disable::<bevy::log::LogPlugin>()
                .disable::<bevy::audio::AudioPlugin>(),
        );
        while app.plugins_state() == bevy::app::PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
        let render = app.get_sub_app(RenderApp).unwrap();
        let device = render
            .world()
            .resource::<bevy::render::renderer::RenderDevice>()
            .wgpu_device();
        let queue = render
            .world()
            .resource::<bevy::render::renderer::RenderQueue>();
        let pipeline = pipeline(device);
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            depth.as_image_copy(),
            bytemuck::cast_slice(&[0.6f32; 16]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: None,
            },
            depth.size(),
        );
        let depth = depth.create_view(&default());
        let run = |mut candidates: Vec<Candidate>, targets: &[Target]| {
            let mut nodes = Vec::new();
            tree(&mut candidates, targets, &mut nodes);
            let nodes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&nodes),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let mut params = Mat4::IDENTITY.to_cols_array().map(f32::to_bits).to_vec();
            params.extend([candidates.len() as u32 * 2 - 1, 1, 0, 0]);
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let output = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&vec![0u32; 514 + targets.len()]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 514 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&default());
            {
                let mut pass = encoder.begin_compute_pass(&default());
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, staging.size());
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    tx.send(result).unwrap();
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let result =
                bytemuck::cast_slice::<u8, u32>(&staging.get_mapped_range(..).unwrap()).to_vec();
            staging.unmap();
            result
        };
        let mut meshes = Assets::<Mesh>::default();
        let assets: Vec<_> = (0..513)
            .map(|_| meshes.add(Cuboid::default()).id())
            .collect();
        let candidate = |asset, x: f32, z: f32| Candidate {
            target: Target::Mesh(asset),
            lo: Vec3::new(x - 0.1, -0.1, z),
            hi: Vec3::new(x + 0.1, 0.1, z + 0.01),
        };
        let result = run(
            vec![
                candidate(assets[0], 0.0, 0.8),
                candidate(assets[0], 0.2, 0.8),
                candidate(assets[1], 0.0, 0.2),
                candidate(assets[2], 3.0, 0.8),
                candidate(assets[3], 0.0, 1.1),
            ],
            &assets[..4]
                .iter()
                .copied()
                .map(Target::Mesh)
                .collect::<Vec<_>>(),
        );
        assert_eq!((result[0], result[1]), (2, 0));
        assert_eq!(
            result[2..4].iter().copied().collect::<HashSet<_>>(),
            HashSet::from([0, 3])
        );
        let result = run(
            assets.iter().map(|&id| candidate(id, 0.0, 0.8)).collect(),
            &assets.iter().copied().map(Target::Mesh).collect::<Vec<_>>(),
        );
        assert_eq!((result[0], result[1]), (513, 1));
        let targets: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|plugin| Target::Tile {
                plugin: plugin.into(),
                set: "visual".into(),
                generation: 1,
                revision: 2,
                id: "0/0/0/0".into(),
            })
            .collect();
        let result = run(
            vec![
                Candidate {
                    target: targets[0].clone(),
                    lo: Vec3::new(-0.1, -0.1, 0.8),
                    hi: Vec3::new(0.1, 0.1, 0.81),
                },
                Candidate {
                    target: targets[0].clone(),
                    lo: Vec3::new(-0.1, -0.1, 0.8),
                    hi: Vec3::new(0.1, 0.1, 0.81),
                },
                Candidate {
                    target: targets[1].clone(),
                    lo: Vec3::new(-0.1, -0.1, 0.2),
                    hi: Vec3::new(0.1, 0.1, 0.21),
                },
            ],
            &targets,
        );
        assert_eq!((result[0], result[1], result[2]), (1, 0, 0));
    }
}
