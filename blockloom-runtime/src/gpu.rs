//! GPU measurement: a whole-frame timestamp span beside Bevy's per-pass ones,
//! and the bytes the render targets and the allocator hold.

use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic};
use bevy::ecs::system::SystemParam;
use bevy::pbr::ViewShadowBindings;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, MapMode, PollType,
    WgpuFeatures,
};
use bevy::render::renderer::{
    PendingCommandBuffers, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderSystems};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// Whole-frame GPU time, first command buffer to last.
pub const FRAME_GPU: DiagnosticPath = DiagnosticPath::const_new("gpu/frame");

/// Frames in flight the timer keeps readbacks for; a busy slot skips a frame.
const SLOTS: usize = 3;
/// Render frames between two memory samples.
const MEMORY_EVERY: u32 = 30;

pub fn register(app: &mut App) {
    let shared = GpuShared::default();
    app.register_diagnostic(Diagnostic::new(FRAME_GPU).with_suffix("ms"))
        .insert_resource(shared.clone())
        .init_resource::<GpuMemory>()
        .add_systems(PreUpdate, sync);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .insert_resource(shared)
            .init_resource::<FrameClock>()
            .init_resource::<MemoryClock>()
            .add_systems(
                RenderGraph,
                (
                    begin_frame.in_set(RenderGraphSystems::Begin),
                    end_frame
                        .after(RenderGraphSystems::Render)
                        .before(RenderGraphSystems::Submit),
                ),
            )
            .add_systems(
                Render,
                sample_memory.in_set(RenderSystems::PrepareBindGroups),
            );
    }
}

/// Handed between the render world, which measures, and the main world.
#[derive(Resource, Clone, Default)]
struct GpuShared(Arc<Mutex<Pending>>);

#[derive(Default)]
struct Pending {
    /// `None` until the render device is up.
    timestamps: Option<bool>,
    frames: Vec<f64>,
    memory: Option<MemorySample>,
}

/// What a GPU allocation is for, as far as its label says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Color,
    Depth,
    Prepass,
    Shadow,
    /// Bloom, SSAO, TAA history and the other screen-space passes.
    Post,
    /// The editor's own scratch target.
    GameView,
    /// Unlabeled textures, which is what image assets are.
    Images,
    Other,
}

impl Kind {
    const ALL: [Kind; 8] = [
        Kind::Color,
        Kind::Depth,
        Kind::Prepass,
        Kind::Shadow,
        Kind::Post,
        Kind::GameView,
        Kind::Images,
        Kind::Other,
    ];

    fn is_target(self) -> bool {
        !matches!(self, Kind::Images | Kind::Other)
    }

    fn metric(self) -> &'static str {
        match self {
            Kind::Color => "memory/targets/color",
            Kind::Depth => "memory/targets/depth",
            Kind::Prepass => "memory/targets/prepass",
            Kind::Shadow => "memory/targets/shadow",
            Kind::Post => "memory/targets/post",
            Kind::GameView => "memory/targets/game_view",
            Kind::Images => "memory/images",
            Kind::Other => "memory/other",
        }
    }

    /// Sorts an allocation by the label Bevy (or we) gave it.
    pub fn of(label: &str) -> Kind {
        const POST: [&str; 14] = [
            "bloom",
            "ssao",
            "ssr",
            "taa_history",
            "depth of field",
            "motion_blur",
            "auto_exposure",
            "SMAA",
            "view_transmission",
            "volumetric",
            "oit_",
            "deferred_lighting",
            "screenshot-capture",
            "working_",
        ];
        if label.starts_with("main_texture") {
            Kind::Color
        } else if label == "view_depth_texture" || label.contains("depth pyramid") {
            Kind::Depth
        } else if label.starts_with("prepass_") || label.starts_with("deferred_lighting_id") {
            Kind::Prepass
        } else if label.contains("shadow_map") {
            Kind::Shadow
        } else if POST.iter().any(|prefix| label.starts_with(prefix)) {
            Kind::Post
        } else if label.starts_with("game view") {
            Kind::GameView
        } else if label == "Unlabeled texture" {
            Kind::Images
        } else {
            Kind::Other
        }
    }
}

/// GPU memory by what it is for, plus the allocator's own totals.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct MemorySample {
    bytes: [u64; Kind::ALL.len()],
    /// Sizes came from the allocator, padding included. Otherwise they were
    /// worked out from the views' textures, and only targets are known.
    pub exact: bool,
    /// Bytes the allocator has handed out, where the backend reports it.
    pub allocated: Option<u64>,
    /// Bytes it has reserved from the driver, free space included.
    pub reserved: Option<u64>,
    /// FP16 color targets, a share of `Color`: what rendering HDR costs.
    pub hdr: u64,
}

impl MemorySample {
    pub fn get(&self, kind: Kind) -> u64 {
        self.bytes[kind as usize]
    }

    fn add(&mut self, kind: Kind, bytes: u64) {
        self.bytes[kind as usize] += bytes;
    }

    pub fn targets(&self) -> u64 {
        Kind::ALL
            .into_iter()
            .filter(|kind| kind.is_target())
            .map(|kind| self.get(kind))
            .sum()
    }
}

/// The latest memory sample, main world.
#[derive(Resource, Default)]
pub struct GpuMemory {
    timestamps: Option<bool>,
    sample: Option<MemorySample>,
}

fn sync(shared: Res<GpuShared>, mut memory: ResMut<GpuMemory>, mut diagnostics: Diagnostics) {
    let mut pending = shared.0.lock().unwrap();
    for ms in pending.frames.drain(..) {
        diagnostics.add_measurement(&FRAME_GPU, || ms);
    }
    if pending.timestamps.is_some() {
        memory.timestamps = pending.timestamps;
    }
    if let Some(sample) = pending.memory.take() {
        memory.sample = Some(sample);
    }
}

// ─── Frame timer ───────────────────────────────────────────────────────────

/// Render world. `None` inside until the device is up, and for good on an
/// adapter without timestamps inside encoders.
#[derive(Resource, Default)]
struct FrameClock {
    started: bool,
    timer: Option<FrameTimer>,
}

/// A timestamp pair per slot, resolved and read back a few frames later.
struct FrameTimer {
    set: wgpu::QuerySet,
    resolve: Buffer,
    slots: [Slot; SLOTS],
    /// The slot this frame opened, if one was free.
    open: Option<usize>,
    period_ns: f64,
}

const WAITING: u8 = 0;
const LANDED: u8 = 1;
const FAILED: u8 = 2;

struct Slot {
    readback: Buffer,
    busy: bool,
    state: Arc<AtomicU8>,
}

impl FrameTimer {
    fn new(device: &RenderDevice, queue: &RenderQueue) -> Self {
        let set = device
            .wgpu_device()
            .create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu_frame_timer"),
                ty: wgpu::QueryType::Timestamp,
                count: (SLOTS * 2) as u32,
            });
        // Resolve offsets must be multiples of
        // `QUERY_RESOLVE_BUFFER_ALIGNMENT` (256), not of the 16 bytes two
        // timestamps write.
        let resolve = device.create_buffer(&BufferDescriptor {
            label: Some("gpu_frame_timer_resolve"),
            size: SLOTS as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = std::array::from_fn(|_| Slot {
            readback: device.create_buffer(&BufferDescriptor {
                label: Some("gpu_frame_timer_readback"),
                size: 16,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            busy: false,
            state: Arc::new(AtomicU8::new(WAITING)),
        });
        Self {
            set,
            resolve,
            slots,
            open: None,
            period_ns: queue.get_timestamp_period() as f64,
        }
    }

    /// Reads every slot whose results have landed, freeing it.
    fn harvest(&mut self, into: &mut Vec<f64>) {
        for slot in &mut self.slots {
            if !slot.busy {
                continue;
            }
            match slot.state.swap(WAITING, Ordering::Acquire) {
                LANDED => {}
                FAILED => {
                    slot.busy = false;
                    continue;
                }
                _ => continue,
            }
            let pair = slot.readback.slice(..).get_mapped_range().ok().map(|data| {
                [0, 8].map(|at| u64::from_le_bytes(data[at..at + 8].try_into().unwrap()))
            });
            slot.readback.unmap();
            slot.busy = false;
            // A zero or backwards pair is a query the driver never wrote.
            if let Some(ms) = pair.and_then(|[begin, end]| frame_ms(begin, end, self.period_ns)) {
                into.push(ms);
            }
        }
    }
}

fn frame_ms(begin: u64, end: u64, period_ns: f64) -> Option<f64> {
    (begin != 0 && end > begin).then(|| (end - begin) as f64 * period_ns / 1e6)
}

fn timestamps_supported(features: WgpuFeatures) -> bool {
    features.contains(WgpuFeatures::TIMESTAMP_QUERY | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS)
}

/// Opens the frame span in a command buffer of its own, ahead of every pass.
fn begin_frame(
    mut clock: ResMut<FrameClock>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    shared: Res<GpuShared>,
    mut pending_buffers: ResMut<PendingCommandBuffers>,
) {
    if !clock.started {
        clock.started = true;
        let supported = timestamps_supported(device.features());
        shared.0.lock().unwrap().timestamps = Some(supported);
        clock.timer = supported.then(|| FrameTimer::new(&device, &queue));
    }
    let Some(timer) = clock.timer.as_mut() else {
        return;
    };
    // Map callbacks only fire when the device is polled.
    let _ = device.poll(PollType::Poll);
    let mut frames = Vec::new();
    timer.harvest(&mut frames);
    if !frames.is_empty() {
        shared.0.lock().unwrap().frames.extend(frames);
    }
    timer.open = timer.slots.iter().position(|slot| !slot.busy);
    let Some(index) = timer.open else {
        return;
    };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("gpu_frame_timer_begin"),
    });
    encoder.write_timestamp(&timer.set, (index * 2) as u32);
    pending_buffers.push_encoder(encoder, "gpu_frame_timer_begin");
}

/// Closes the span after every pass, and queues its readback.
fn end_frame(
    mut clock: ResMut<FrameClock>,
    device: Res<RenderDevice>,
    mut pending_buffers: ResMut<PendingCommandBuffers>,
) {
    let Some(timer) = clock.timer.as_mut() else {
        return;
    };
    let Some(index) = timer.open.take() else {
        return;
    };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("gpu_frame_timer_end"),
    });
    let first = (index * 2) as u32;
    let offset = index as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
    encoder.write_timestamp(&timer.set, first + 1);
    encoder.resolve_query_set(&timer.set, first..first + 2, &timer.resolve, offset);
    let slot = &mut timer.slots[index];
    encoder.copy_buffer_to_buffer(&timer.resolve, offset, &slot.readback, 0, 16);
    let state = slot.state.clone();
    encoder.map_buffer_on_submit(&slot.readback, MapMode::Read, .., move |result| {
        state.store(
            if result.is_ok() { LANDED } else { FAILED },
            Ordering::Release,
        );
    });
    slot.busy = true;
    pending_buffers.push_encoder(encoder, "gpu_frame_timer_end");
}

// ─── Memory ────────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct MemoryClock(u32);

/// Sorts the allocator's report by label where the backend has one
/// (Vulkan, DX12), and otherwise works the targets out from the views.
fn sample_memory(
    mut clock: ResMut<MemoryClock>,
    device: Res<RenderDevice>,
    shared: Res<GpuShared>,
    views: ViewTextures,
) {
    clock.0 = clock.0.wrapping_add(1);
    if clock.0 % MEMORY_EVERY != 1 {
        return;
    }
    let sample = match device.wgpu_device().generate_allocator_report() {
        Some(report) => {
            let mut sample = MemorySample {
                exact: true,
                allocated: Some(report.total_allocated_bytes),
                reserved: Some(report.total_reserved_bytes),
                ..default()
            };
            for allocation in &report.allocations {
                sample.add(Kind::of(&allocation.name), allocation.size);
            }
            sample
        }
        None => views.sample(),
    };
    let sample = MemorySample {
        hdr: views.hdr_bytes(),
        ..sample
    };
    shared.0.lock().unwrap().memory = Some(sample);
}

#[derive(SystemParam)]
struct ViewTextures<'w, 's> {
    targets: Query<'w, 's, &'static ViewTarget>,
    depths: Query<'w, 's, &'static ViewDepthStencilTexture>,
    prepasses: Query<'w, 's, &'static ViewPrepassTextures>,
    shadows: Query<'w, 's, &'static ViewShadowBindings>,
}

impl ViewTextures<'_, '_> {
    /// The views' FP16 color targets, each counted once. Labels don't say a
    /// format, so this reads the textures even when the allocator reports.
    fn hdr_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        let mut bytes = 0;
        let mut count = |texture: &wgpu::Texture| {
            if texture.format() == wgpu::TextureFormat::Rgba16Float && seen.insert(texture.clone())
            {
                bytes += texture_bytes(texture);
            }
        };
        for target in &self.targets {
            count(target.main_texture());
            count(target.main_texture_other());
            if let Some(sampled) = target.sampled_main_texture() {
                count(sampled);
            }
        }
        bytes
    }

    /// Every view's targets, each texture counted once however many views
    /// share it.
    fn sample(&self) -> MemorySample {
        let mut seen = HashSet::new();
        let mut sample = MemorySample::default();
        let mut count = |kind: Kind, texture: &wgpu::Texture| {
            if seen.insert(texture.clone()) {
                sample.add(kind, texture_bytes(texture));
            }
        };
        for target in &self.targets {
            count(Kind::Color, target.main_texture());
            count(Kind::Color, target.main_texture_other());
            if let Some(sampled) = target.sampled_main_texture() {
                count(Kind::Color, sampled);
            }
        }
        for depth in &self.depths {
            count(Kind::Depth, &depth.attachment.texture.texture);
        }
        for prepass in &self.prepasses {
            if let Some(depth) = &prepass.depth {
                count(Kind::Prepass, &depth.texture.texture);
                if let Some(previous) = &depth.previous_frame_texture {
                    count(Kind::Prepass, &previous.texture);
                }
            }
            for attachment in [
                &prepass.normal,
                &prepass.motion_vectors,
                &prepass.deferred,
                &prepass.deferred_lighting_pass_id,
            ]
            .into_iter()
            .flatten()
            {
                count(Kind::Prepass, &attachment.texture.texture);
                for extra in [
                    &attachment.resolve_target,
                    &attachment.previous_frame_texture,
                ]
                .into_iter()
                .flatten()
                {
                    count(Kind::Prepass, &extra.texture);
                }
            }
        }
        for shadow in &self.shadows {
            count(Kind::Shadow, &shadow.point_light_depth_texture);
            count(Kind::Shadow, &shadow.directional_light_depth_texture);
        }
        sample
    }
}

/// What a texture's pixels take, every mip, layer and sample included.
/// Drivers pad beyond this, so it is a floor rather than the exact figure.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let format = texture.format();
    let size = texture.size();
    let dimension = texture.dimension();
    let samples = u64::from(texture.sample_count());
    (0..texture.mip_level_count())
        .map(|level| {
            let mip = size.mip_level_size(level, dimension);
            let layers = match dimension {
                wgpu::TextureDimension::D3 => mip.depth_or_array_layers,
                _ => size.depth_or_array_layers,
            };
            let plane = wgpu::Extent3d {
                depth_or_array_layers: 1,
                ..mip
            };
            format.theoretical_memory_footprint(plane) * u64::from(layers)
        })
        .sum::<u64>()
        * samples
}

// ─── Report ────────────────────────────────────────────────────────────────

/// What the profiler shows for this module, beside the `gpu/frame` timing
/// the diagnostics store already carries.
#[derive(SystemParam)]
pub struct GpuReport<'w> {
    memory: Option<Res<'w, GpuMemory>>,
}

impl GpuReport<'_> {
    /// `(name, value, unit)` rows.
    pub fn metrics(&self) -> Vec<(&'static str, f64, &'static str)> {
        let mut metrics = Vec::new();
        let Some(memory) = &self.memory else {
            return metrics;
        };
        if let Some(supported) = memory.timestamps {
            metrics.push(("gpu/timestamps", f64::from(u8::from(supported)), "flag"));
        }
        if let Some(sample) = memory.sample {
            metrics.push(("memory/exact", f64::from(u8::from(sample.exact)), "flag"));
            metrics.push(("memory/targets", sample.targets() as f64, "bytes"));
            metrics.push(("memory/targets/hdr", sample.hdr as f64, "bytes"));
            for kind in Kind::ALL {
                if sample.exact || kind.is_target() {
                    metrics.push((kind.metric(), sample.get(kind) as f64, "bytes"));
                }
            }
            if let Some(bytes) = sample.allocated {
                metrics.push(("memory/gpu_allocated", bytes as f64, "bytes"));
            }
            if let Some(bytes) = sample.reserved {
                metrics.push(("memory/gpu_reserved", bytes as f64, "bytes"));
            }
        }
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_time_follows_the_timestamp_period() {
        // 2.5 million ticks of 1ns is 2.5ms.
        assert_eq!(frame_ms(1_000, 2_501_000, 1.0), Some(2.5));
        assert_eq!(frame_ms(10, 20, 100.0), Some(0.001));
    }

    #[test]
    fn unwritten_or_backwards_queries_are_dropped() {
        assert_eq!(frame_ms(0, 500, 1.0), None);
        assert_eq!(frame_ms(500, 500, 1.0), None);
        assert_eq!(frame_ms(900, 500, 1.0), None);
    }

    #[test]
    fn timestamps_need_both_features() {
        assert!(!timestamps_supported(WgpuFeatures::TIMESTAMP_QUERY));
        assert!(timestamps_supported(
            WgpuFeatures::TIMESTAMP_QUERY | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS
        ));
    }

    #[test]
    fn bevy_labels_sort_into_kinds() {
        for (label, kind) in [
            ("main_texture_a", Kind::Color),
            ("main_texture_sampled", Kind::Color),
            ("view_depth_texture", Kind::Depth),
            ("view depth pyramid texture", Kind::Depth),
            ("prepass_depth_texture_1", Kind::Prepass),
            ("prepass_motion_vectors_textures", Kind::Prepass),
            ("deferred_lighting_id_depth_texture_a", Kind::Prepass),
            ("directional_light_shadow_map_texture", Kind::Shadow),
            ("point_light_shadow_map_texture", Kind::Shadow),
            ("bloom_texture", Kind::Post),
            ("ssao_noisy_texture", Kind::Post),
            ("taa_history_1_texture", Kind::Post),
            ("depth of field auxiliary texture", Kind::Post),
            ("working_scratch_a", Kind::Post),
            ("game view target", Kind::GameView),
            ("Unlabeled texture", Kind::Images),
            ("general mesh slab 0 (vertex buffer)", Kind::Other),
            ("Unlabeled buffer", Kind::Other),
        ] {
            assert_eq!(Kind::of(label), kind, "{label}");
        }
    }

    #[test]
    fn target_totals_leave_out_images_and_the_rest() {
        let mut sample = MemorySample::default();
        for (kind, bytes) in [
            (Kind::Color, 1),
            (Kind::Depth, 2),
            (Kind::Prepass, 4),
            (Kind::Shadow, 8),
            (Kind::Post, 16),
            (Kind::GameView, 32),
            (Kind::Images, 64),
            (Kind::Other, 128),
        ] {
            sample.add(kind, bytes);
        }
        assert_eq!(sample.targets(), 63);
    }
}
