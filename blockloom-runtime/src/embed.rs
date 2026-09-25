//! The world embedded in the editor: no window and no pipes.
//!
//! The editor calls [`run`] on a thread of its own. Messages travel over
//! channels. Every world camera draws straight into a small ring of dma-bufs
//! the editor's Game view imports through EGL, laid out in a tiled DRM format
//! modifier the viewer said it can sample. With no such modifier, frames are
//! copied into linear system-memory images instead - the CPU never touches a
//! pixel either way.

use crate::bridge;
use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::app::{PluginsState, TerminalCtrlCHandlerPlugin};
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::{ManualTextureViewHandle, RenderTarget, ScalingMode};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::TextureView;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::{ManualTextureView, ManualTextureViews, OutputColorAttachment};
use bevy::render::view::{ViewTargetAttachments, clear_view_attachments, prepare_view_attachments};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::ui::{IsDefaultUiCamera, UiScale};
use bevy::window::ExitCondition;
use blockloom_core::scene::Mode;
use blockloom_protocol::{EditorMessage, GAME_SIZE, RuntimeMessage};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How many shared images the ring holds: one the viewer shows, one ready
/// behind it, one being copied into.
const SLOTS: usize = 3;
/// The camera target every world camera points at.
const VIEW: ManualTextureViewHandle = ManualTextureViewHandle(0xB10C);
/// What the world draws at until the view says how big it is.
const SIZE: UVec2 = UVec2::new(GAME_SIZE.0, GAME_SIZE.1);
/// Bounds on a requested size, so a collapsed or huge view can't ask for
/// an image nothing can allocate.
const MIN_SIZE: u32 = 16;
const MAX_SIZE: u32 = 8192;
/// The longest a world waits for the display: a hidden view presents
/// nothing, and the world still has to keep hearing the editor.
const PACE_TIMEOUT: Duration = Duration::from_millis(50);
/// `DRM_FORMAT_XBGR8888`: RGBA bytes in memory, alpha ignored.
const FOURCC_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
/// `DRM_FORMAT_MOD_LINEAR`.
pub const MODIFIER_LINEAR: u64 = 0;
/// What the cameras draw in. The bytes are sRGB-encoded, which the viewer
/// samples as plain RGBA and shows as is.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

static NEXT_WORLD: AtomicU64 = AtomicU64::new(1);

/// Everything an embedded world is started with.
pub struct Embedded {
    pub mode: Mode,
    pub incoming: Receiver<EditorMessage>,
    pub outgoing: Sender<RuntimeMessage>,
    pub frames: Arc<FrameExchange>,
}

/// Runs a world on the calling thread until the editor shuts it down or drops
/// its sender. `blockloom_core::init` must already have run.
pub fn run(embedded: Embedded) {
    let Embedded {
        mode,
        incoming,
        outgoing,
        frames,
    } = embedded;
    let world = NEXT_WORLD.fetch_add(1, Ordering::Relaxed);
    bridge::attach(world, outgoing);
    // Runs on unwind too, so a panicking world still tells the editor it's gone.
    let _detach = Detach(world, frames.clone());

    let mut app = App::new();
    app.insert_resource(dmabuf::vulkan_settings());
    // No winit: the editor owns the display. No log plugin or Ctrl-C
    // handler either, since both are process-wide and the editor has its own.
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..default()
            })
            .set(crate::asset_plugin())
            // Async compiles are tasks on Bevy's process-wide pools, which
            // outlive this world and would drop its GPU device on a pool
            // thread - racing the editor's own exit.
            .set(bevy::render::RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .disable::<TerminalCtrlCHandlerPlugin>(),
    );
    crate::add_world(&mut app, mode, Engine::new(incoming, mode));
    add_surface(&mut app, frames.clone());
    app.set_runner(move |app| paced(app, &frames));
    app.run();
}

/// Bevy's loop, one update per frame the view presents rather than on a timer
/// of its own, so a 144 Hz screen gets 144 evenly spaced frames.
fn paced(mut app: App, frames: &FrameExchange) -> AppExit {
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    let mut seen = frames.presented_count();
    loop {
        app.update();
        if let Some(exit) = app.should_exit() {
            return exit;
        }
        seen = frames.wait_presented(seen, PACE_TIMEOUT);
    }
}

struct Detach(u64, Arc<FrameExchange>);

impl Drop for Detach {
    fn drop(&mut self) {
        bridge::detach(self.0);
        self.1.clear();
    }
}

// ─── The exchange ───────────────────────────────────────────────────────────

/// One shared image, as the viewer imports it.
pub struct SharedImage {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// The ring as it stands. A new generation replaces every image at once.
/// Linear rings may only be samplable as external textures; tiled ones use
/// a modifier the viewer offered as a plain texture.
pub struct SlotSet {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub images: Vec<SharedImage>,
}

/// The hand-off between the world, which draws into slots, and the viewer,
/// which shows the newest finished one and says which it is still reading.
/// Outlives any one world, so a restart keeps the same viewer.
pub struct FrameExchange {
    ring: Mutex<Ring>,
    wake: Box<dyn Fn() + Send + Sync>,
    /// How many frames the view has put on screen, and its waiter.
    presented: Mutex<u64>,
    shown: Condvar,
    offer: Mutex<Offer>,
    wanted: Mutex<Viewport>,
}

/// What the view shows the game at: physical pixels, and how many of them
/// make one logical pixel, so a HiDPI view looks the size a player's window
/// would rather than half of it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub size: UVec2,
    pub scale: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            size: SIZE,
            scale: 1.0,
        }
    }
}

/// The tiled modifiers the viewer can sample as a plain texture. Each change
/// bumps `version`, which is the world's cue to reconsider its ring.
#[derive(Clone, Default, PartialEq, Debug)]
struct Offer {
    version: u64,
    modifiers: Vec<u64>,
}

#[derive(Default)]
struct Ring {
    set: Option<Arc<SlotSet>>,
    busy: Vec<bool>,
    ready: Option<usize>,
    held: Option<usize>,
    last: usize,
    generations: u64,
}

impl FrameExchange {
    /// `wake` runs, on whatever thread, whenever there is something new to
    /// show - a finished frame, a new ring, or no world at all.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            ring: Mutex::new(Ring::default()),
            wake: Box::new(wake),
            presented: Mutex::new(0),
            shown: Condvar::new(),
            offer: Mutex::new(Offer::default()),
            wanted: Mutex::new(Viewport::default()),
        })
    }

    /// The view is `width` x `height` physical pixels at `scale` per logical
    /// one. The world draws at that size from its next frame. Any thread.
    pub fn resize(&self, width: u32, height: u32, scale: f32) {
        let size = UVec2::new(width, height).clamp(UVec2::splat(MIN_SIZE), UVec2::splat(MAX_SIZE));
        let scale = if scale.is_finite() {
            scale.clamp(0.25, 8.0)
        } else {
            1.0
        };
        if let Ok(mut wanted) = self.wanted.lock() {
            *wanted = Viewport { size, scale };
        }
    }

    fn wanted(&self) -> Viewport {
        self.wanted
            .lock()
            .map_or_else(|_| Viewport::default(), |wanted| *wanted)
    }

    /// The viewer can sample these `XBGR8888` modifiers without an external
    /// texture. Linear is left out: that path is the fallback anyway.
    pub fn accept(&self, modifiers: impl IntoIterator<Item = u64>) {
        let mut modifiers: Vec<u64> = modifiers
            .into_iter()
            .filter(|&modifier| modifier != MODIFIER_LINEAR)
            .collect();
        modifiers.sort_unstable();
        modifiers.dedup();
        if let Ok(mut offer) = self.offer.lock()
            && offer.modifiers != modifiers
        {
            offer.version += 1;
            offer.modifiers = modifiers;
        }
    }

    /// The viewer couldn't import ring `generation` after all: withdraw its
    /// modifier, so the world falls back to something else.
    pub fn refuse(&self, generation: u64) {
        let Some(set) = self.slots() else { return };
        if set.generation != generation || set.modifier == MODIFIER_LINEAR {
            return;
        }
        if let Ok(mut offer) = self.offer.lock() {
            offer.modifiers.retain(|&modifier| modifier != set.modifier);
            offer.version += 1;
        }
    }

    fn offer(&self) -> Offer {
        self.offer
            .lock()
            .map_or_else(|_| Offer::default(), |offer| offer.clone())
    }

    /// The view just put a frame on screen: time for the world's next one.
    /// Any thread.
    pub fn presented(&self) {
        if let Ok(mut count) = self.presented.lock() {
            *count += 1;
        }
        self.shown.notify_all();
    }

    fn presented_count(&self) -> u64 {
        self.presented.lock().map_or(0, |count| *count)
    }

    /// Blocks until a frame past `seen` is presented, or `timeout` passes.
    /// Answers the count now, so a present during the update isn't waited on.
    fn wait_presented(&self, seen: u64, timeout: Duration) -> u64 {
        let Ok(count) = self.presented.lock() else {
            return seen;
        };
        match self
            .shown
            .wait_timeout_while(count, timeout, |count| *count == seen)
        {
            Ok((count, _)) => *count,
            Err(_) => seen,
        }
    }

    /// The current ring, or none while no world is drawing.
    pub fn slots(&self) -> Option<Arc<SlotSet>> {
        self.ring.lock().ok()?.set.clone()
    }

    /// The newest finished frame, as (generation, slot).
    pub fn latest(&self) -> Option<(u64, usize)> {
        let ring = self.ring.lock().ok()?;
        Some((ring.set.as_ref()?.generation, ring.ready?))
    }

    /// The viewer is now reading `index`; the one it read before is free.
    pub fn hold(&self, generation: u64, index: usize) {
        if let Ok(mut ring) = self.ring.lock()
            && ring
                .set
                .as_ref()
                .is_some_and(|set| set.generation == generation)
        {
            ring.held = Some(index);
        }
    }

    fn install(&self, width: u32, height: u32, modifier: u64, images: Vec<SharedImage>) -> u64 {
        let generation = {
            let Ok(mut ring) = self.ring.lock() else {
                return 0;
            };
            ring.generations += 1;
            let generation = ring.generations;
            ring.busy = vec![false; images.len()];
            ring.ready = None;
            ring.held = None;
            ring.set = Some(Arc::new(SlotSet {
                generation,
                width,
                height,
                fourcc: FOURCC_XBGR8888,
                modifier,
                images,
            }));
            generation
        };
        (self.wake)();
        generation
    }

    /// A slot nobody is reading or drawing, taken round-robin so the one
    /// just let go gets the most time to finish being read.
    fn claim(&self, generation: u64) -> Option<usize> {
        let mut ring = self.ring.lock().ok()?;
        if ring.set.as_ref()?.generation != generation {
            return None;
        }
        let count = ring.busy.len();
        let index = (1..=count)
            .map(|step| (ring.last + step) % count)
            .find(|&i| !ring.busy[i] && ring.ready != Some(i) && ring.held != Some(i))?;
        ring.busy[index] = true;
        ring.last = index;
        Some(index)
    }

    fn finished(&self, generation: u64, index: usize) {
        {
            let Ok(mut ring) = self.ring.lock() else {
                return;
            };
            if ring
                .set
                .as_ref()
                .is_none_or(|set| set.generation != generation)
            {
                return;
            }
            ring.busy[index] = false;
            ring.ready = Some(index);
        }
        (self.wake)();
    }

    fn clear(&self) {
        if let Ok(mut ring) = self.ring.lock() {
            ring.set = None;
            ring.busy.clear();
            ring.ready = None;
            ring.held = None;
        }
        (self.wake)();
    }
}

// ─── The surface ────────────────────────────────────────────────────────────

/// The images the world draws into, in the main world.
#[derive(Resource)]
pub struct GameSurface {
    exchange: Arc<FrameExchange>,
    /// The offer version the ring was last reconsidered against.
    seen: Option<u64>,
    /// What the ring is allocated at.
    viewport: Viewport,
    /// Sharing failed outright; the world runs unseen rather than retrying.
    failed: bool,
}

impl GameSurface {
    /// What pointer coordinates are measured against.
    pub fn size(&self) -> Vec2 {
        self.viewport.size.as_vec2()
    }
}

/// The shared ring, as the render world sees it. `direct` rings are tiled
/// images the cameras draw into themselves; otherwise each frame lands in
/// `scratch` and is copied into a linear slot.
#[derive(Resource, Clone, ExtractResource)]
struct FrameCopy {
    exchange: Arc<FrameExchange>,
    generation: u64,
    direct: bool,
    modifier: u64,
    /// Where the cameras draw when no slot is free, and the copy's source.
    scratch: Option<wgpu::Texture>,
    ring: Vec<wgpu::Texture>,
    views: Vec<TextureView>,
}

/// The slot this frame's cameras are drawing into, render world only.
#[derive(Resource, Default)]
struct Drawing(Option<usize>);

fn add_surface(app: &mut App, exchange: Arc<FrameExchange>) {
    app.insert_resource(GameSurface {
        exchange: exchange.clone(),
        seen: None,
        viewport: Viewport::default(),
        failed: false,
    })
    .insert_resource(FrameCopy {
        exchange,
        generation: 0,
        direct: false,
        modifier: MODIFIER_LINEAR,
        scratch: None,
        ring: Vec::new(),
        views: Vec::new(),
    })
    .add_plugins(ExtractResourcePlugin::<FrameCopy>::default())
    .add_systems(Update, (target_cameras, fit_cameras))
    .add_systems(Last, build_surface);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<Drawing>().add_systems(
            Render,
            (
                aim_cameras
                    .in_set(RenderSystems::PrepareViews)
                    .after(clear_view_attachments)
                    .before(prepare_view_attachments),
                finish_frame.in_set(RenderSystems::Cleanup),
            ),
        );
    }
}

/// Points every world camera at the shared view, and makes it the one the
/// interface draws over: with no window there is no default to fall back on.
fn target_cameras(
    mut commands: Commands,
    cameras: Query<(Entity, &RenderTarget, Has<IsDefaultUiCamera>), With<WorldCamera>>,
) {
    for (entity, target, ui) in &cameras {
        let aimed = matches!(target, RenderTarget::TextureView(handle) if *handle == VIEW);
        if !aimed || !ui {
            commands
                .entity(entity)
                .insert((RenderTarget::TextureView(VIEW), IsDefaultUiCamera));
        }
    }
}

/// Shows the 2D world and the interface at the view's logical size, the way
/// a player's window at that size would, whatever its pixel density.
fn fit_cameras(
    surface: Res<GameSurface>,
    mut ui_scale: ResMut<UiScale>,
    mut cameras: Query<&mut Projection, With<WorldCamera>>,
) {
    let Viewport { size, scale } = surface.viewport;
    if ui_scale.0 != scale {
        ui_scale.0 = scale;
    }
    let height = size.y as f32 / scale;
    for mut projection in &mut cameras {
        let fitted = matches!(&*projection, Projection::Orthographic(ortho)
            if matches!(ortho.scaling_mode, ScalingMode::FixedVertical { viewport_height } if viewport_height == height));
        if !fitted && let Projection::Orthographic(ortho) = projection.as_mut() {
            ortho.scaling_mode = ScalingMode::FixedVertical {
                viewport_height: height,
            };
        }
    }
}

/// Allocates the scratch target and the ring once the render device exists,
/// again whenever the view changes size, and the ring alone whenever the
/// viewer's offer changes what it should be: tiled in a modifier the viewer
/// samples directly if there is one, linear otherwise.
fn build_surface(
    mut surface: ResMut<GameSurface>,
    mut copy: ResMut<FrameCopy>,
    device: Res<RenderDevice>,
    mut views: ResMut<ManualTextureViews>,
) {
    let offer = surface.exchange.offer();
    let wanted = surface.exchange.wanted();
    let resized = wanted.size != surface.viewport.size;
    surface.viewport.scale = wanted.scale;
    if surface.failed || (surface.seen == Some(offer.version) && !resized && copy.scratch.is_some())
    {
        return;
    }
    surface.seen = Some(offer.version);
    surface.viewport.size = wanted.size;
    let size = wanted.size;
    let device = device.wgpu_device();
    if copy.scratch.is_none() || resized {
        let scratch = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game view target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        views.insert(
            VIEW,
            ManualTextureView {
                texture_view: scratch
                    .create_view(&wgpu::TextureViewDescriptor::default())
                    .into(),
                size,
                view_format: TARGET_FORMAT,
            },
        );
        copy.scratch = Some(scratch);
    }

    // A tiled ring the offer still covers stays put.
    let built = !copy.ring.is_empty() && !resized;
    if built && copy.direct && offer.modifiers.contains(&copy.modifier) {
        return;
    }
    let tiled = match dmabuf::allocate_tiled(device, size, &offer.modifiers, SLOTS) {
        Ok(ring) => ring,
        Err(reason) => {
            if !offer.modifiers.is_empty() {
                tracing::info!("game view: sharing linear frames: {reason}");
            }
            None
        }
    };
    let (direct, ring) = match tiled {
        Some(ring) => (true, ring),
        None if built && !copy.direct => return,
        None => match (0..SLOTS)
            .map(|_| dmabuf::allocate_linear(device, size))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(ring) => (false, ring),
            Err(error) => {
                surface.failed = true;
                tracing::error!("game view: {error}");
                bridge::send(&RuntimeMessage::Error {
                    actor: "Game view".to_string(),
                    message: format!("Couldn't share frames with the editor: {error}"),
                });
                return;
            }
        },
    };
    let modifier = ring.first().map_or(MODIFIER_LINEAR, |slot| slot.modifier);
    let (mut textures, mut images) = (Vec::new(), Vec::new());
    for slot in ring {
        textures.push(slot.texture);
        images.push(slot.image);
    }
    copy.views = textures
        .iter()
        .map(|texture| {
            texture
                .create_view(&wgpu::TextureViewDescriptor::default())
                .into()
        })
        .collect();
    copy.ring = textures;
    copy.direct = direct;
    copy.modifier = modifier;
    copy.generation = surface.exchange.install(size.x, size.y, modifier, images);
}

/// Before the cameras' views are prepared: point their output at a free slot
/// of a direct ring. With none free they draw into the scratch target, and
/// the viewer stays on an older frame.
fn aim_cameras(
    copy: Option<Res<FrameCopy>>,
    cameras: Query<&ExtractedCamera>,
    mut attachments: ResMut<ViewTargetAttachments>,
    mut drawing: ResMut<Drawing>,
) {
    drawing.0 = None;
    let Some(copy) = copy.filter(|copy| copy.direct) else {
        return;
    };
    let target = NormalizedRenderTarget::TextureView(VIEW);
    // Before the world has a camera a slot would go out undrawn.
    if !cameras
        .iter()
        .any(|camera| camera.target.as_ref() == Some(&target))
    {
        return;
    }
    if let Some(index) = copy.exchange.claim(copy.generation) {
        attachments.insert(
            target,
            OutputColorAttachment::new(copy.views[index].clone(), TARGET_FORMAT),
        );
        drawing.0 = Some(index);
    }
}

/// After the cameras' submission: tell the viewer once the GPU has finished
/// the frame. A linear ring first gets the frame copied into a free slot.
fn finish_frame(
    copy: Option<Res<FrameCopy>>,
    mut drawing: ResMut<Drawing>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let drawn = drawing.0.take();
    if let Some(copy) = copy {
        let index = if copy.direct {
            drawn
        } else {
            copy_frame(&copy, &device, &queue)
        };
        if let Some(index) = index {
            let (exchange, generation) = (copy.exchange.clone(), copy.generation);
            queue.on_submitted_work_done(move || exchange.finished(generation, index));
        }
    }
    // Callbacks only fire when the device is polled.
    let _ = device.poll(wgpu::PollType::Poll);
}

/// Copies the scratch target into a free linear slot, and answers which.
fn copy_frame(copy: &FrameCopy, device: &RenderDevice, queue: &RenderQueue) -> Option<usize> {
    let source = copy.scratch.as_ref()?;
    if copy.ring.is_empty() {
        return None;
    }
    let index = copy.exchange.claim(copy.generation)?;
    let mut encoder =
        device
            .wgpu_device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("game view copy"),
            });
    encoder.copy_texture_to_texture(
        source.as_image_copy(),
        copy.ring[index].as_image_copy(),
        source.size(),
    );
    queue.submit([encoder.finish()]);
    Some(index)
}

fn extent(size: UVec2) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    }
}

// ─── dma-buf images ─────────────────────────────────────────────────────────

mod dmabuf {
    use super::{SharedImage, TARGET_FORMAT, extent};
    use ash::{ext, khr, vk};
    use bevy::math::UVec2;
    use bevy::render::renderer::raw_vulkan_init::RawVulkanInitSettings;
    use std::os::fd::{FromRawFd, OwnedFd};
    use wgpu::hal::api::Vulkan;

    /// Byte-for-byte what the sRGB target holds; the viewer shows it as is.
    const LINEAR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
    /// Tiled slots are drawn into directly, so they take the target's format.
    const TILED_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
    const HANDLE: vk::ExternalMemoryHandleTypeFlags =
        vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;

    /// One slot of the ring: the wgpu texture the world writes, and what
    /// the viewer imports it by.
    pub struct Slot {
        pub texture: wgpu::Texture,
        pub image: SharedImage,
        pub modifier: u64,
    }

    /// Turns on `VK_EXT_image_drm_format_modifier` where the GPU has it;
    /// wgpu doesn't on its own.
    pub fn vulkan_settings() -> RawVulkanInitSettings {
        let mut settings = RawVulkanInitSettings::default();
        // SAFETY: only adds extensions the adapter reports supporting.
        unsafe {
            settings.add_create_device_callback(|args, adapter, _| {
                let wanted = [
                    ext::image_drm_format_modifier::NAME,
                    // Core since 1.2, which the modifier extension needs.
                    khr::image_format_list::NAME,
                ];
                if wanted.iter().all(|name| {
                    adapter
                        .physical_device_capabilities()
                        .supports_extension(name)
                }) {
                    for name in wanted {
                        if !args.extensions.contains(&name) {
                            args.extensions.push(name);
                        }
                    }
                }
            });
        }
        settings
    }

    /// A linear, exportable RGBA image a frame can be copied into. Linear
    /// and in system memory, so the viewer needs no modifier support and can
    /// sit on a different GPU.
    pub fn allocate_linear(device: &wgpu::Device, size: UVec2) -> Result<Slot, String> {
        // SAFETY: every Vulkan object is made on wgpu's device and handed
        // straight back to wgpu, which owns and frees the image and memory.
        unsafe {
            let hal = device
                .as_hal::<Vulkan>()
                .ok_or("the renderer isn't using Vulkan")?;
            let instance = hal.shared_instance().raw_instance();
            let features = instance
                .get_physical_device_format_properties(hal.raw_physical_device(), LINEAR_FORMAT)
                .linear_tiling_features;
            if !features.contains(vk::FormatFeatureFlags::TRANSFER_DST) {
                return Err("this GPU can't copy into a linear RGBA image".to_string());
            }
            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(HANDLE);
            let info = image_info(size, LINEAR_FORMAT, vk::ImageUsageFlags::TRANSFER_DST)
                .tiling(vk::ImageTiling::LINEAR)
                .push_next(&mut external);
            let (image, memory, fd) = create(&hal, &info, false)?;
            let layout = hal.raw_device().get_image_subresource_layout(
                image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    array_layer: 0,
                },
            );
            let texture = wrap(
                device,
                hal,
                image,
                memory,
                size,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::COPY_DST,
            );
            Ok(Slot {
                texture,
                image: SharedImage {
                    fd,
                    offset: layout.offset as u32,
                    stride: layout.row_pitch as u32,
                },
                modifier: super::MODIFIER_LINEAR,
            })
        }
    }

    /// `count` images the cameras draw into, all in one of the `offered`
    /// modifiers, which the driver picks. `None` when nothing was offered.
    pub fn allocate_tiled(
        device: &wgpu::Device,
        size: UVec2,
        offered: &[u64],
        count: usize,
    ) -> Result<Option<Vec<Slot>>, String> {
        if offered.is_empty() {
            return Ok(None);
        }
        let mut modifiers = usable_modifiers(device, size, offered)?;
        if modifiers.is_empty() {
            return Err("none of the viewer's modifiers can be drawn into".to_string());
        }
        let mut ring = Vec::with_capacity(count);
        for _ in 0..count {
            let slot = allocate_tiled_one(device, size, &modifiers)?;
            // The ring shares one layout, so the rest take the first's.
            modifiers = vec![slot.modifier];
            ring.push(slot);
        }
        Ok(Some(ring))
    }

    /// The offered modifiers this GPU can render into and export at `size`,
    /// in a single memory plane.
    fn usable_modifiers(
        device: &wgpu::Device,
        size: UVec2,
        offered: &[u64],
    ) -> Result<Vec<u64>, String> {
        // SAFETY: queries only, on wgpu's own physical device.
        unsafe {
            let hal = device
                .as_hal::<Vulkan>()
                .ok_or("the renderer isn't using Vulkan")?;
            if !hal
                .enabled_device_extensions()
                .contains(&ext::image_drm_format_modifier::NAME)
            {
                return Err("the GPU has no VK_EXT_image_drm_format_modifier".to_string());
            }
            let instance = hal.shared_instance().raw_instance();
            let physical = hal.raw_physical_device();
            let usable = drawable_modifiers(instance, physical)
                .into_iter()
                .filter(|modifier| offered.contains(modifier))
                .filter(|&modifier| {
                    let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
                        .drm_format_modifier(modifier)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE);
                    let mut external =
                        vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(HANDLE);
                    let info = vk::PhysicalDeviceImageFormatInfo2::default()
                        .format(TILED_FORMAT)
                        .ty(vk::ImageType::TYPE_2D)
                        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                        .push_next(&mut drm)
                        .push_next(&mut external);
                    let mut exported = vk::ExternalImageFormatProperties::default();
                    let mut answer = vk::ImageFormatProperties2::default().push_next(&mut exported);
                    instance
                        .get_physical_device_image_format_properties2(physical, &info, &mut answer)
                        .is_ok()
                        && answer.image_format_properties.max_extent.width >= size.x
                        && answer.image_format_properties.max_extent.height >= size.y
                        && exported
                            .external_memory_properties
                            .external_memory_features
                            .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
                })
                .collect();
            Ok(usable)
        }
    }

    /// The single-plane modifiers this GPU can render the target format into.
    pub unsafe fn drawable_modifiers(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
    ) -> Vec<u64> {
        // SAFETY: the caller's; a two-call query into a buffer it sizes.
        unsafe {
            let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
            let mut properties = vk::FormatProperties2::default().push_next(&mut list);
            instance.get_physical_device_format_properties2(
                physical,
                TILED_FORMAT,
                &mut properties,
            );
            let mut known = vec![
                vk::DrmFormatModifierPropertiesEXT::default();
                list.drm_format_modifier_count as usize
            ];
            let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
                .drm_format_modifier_properties(&mut known);
            let mut properties = vk::FormatProperties2::default().push_next(&mut list);
            instance.get_physical_device_format_properties2(
                physical,
                TILED_FORMAT,
                &mut properties,
            );
            let count = list.drm_format_modifier_count as usize;
            known.truncate(count);
            known
                .iter()
                .filter(|known| {
                    known.drm_format_modifier_plane_count == 1
                        && known
                            .drm_format_modifier_tiling_features
                            .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
                })
                .map(|known| known.drm_format_modifier)
                .collect()
        }
    }

    fn allocate_tiled_one(
        device: &wgpu::Device,
        size: UVec2,
        modifiers: &[u64],
    ) -> Result<Slot, String> {
        // SAFETY: as for `allocate_linear`.
        unsafe {
            let hal = device
                .as_hal::<Vulkan>()
                .ok_or("the renderer isn't using Vulkan")?;
            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(HANDLE);
            let mut list = vk::ImageDrmFormatModifierListCreateInfoEXT::default()
                .drm_format_modifiers(modifiers);
            let info = image_info(size, TILED_FORMAT, vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .push_next(&mut external)
                .push_next(&mut list);
            let (image, memory, fd) = create(&hal, &info, true)?;
            let raw = hal.raw_device();
            let instance = hal.shared_instance().raw_instance();
            let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
            let picked = ext::image_drm_format_modifier::Device::new(instance, raw)
                .get_image_drm_format_modifier_properties(image, &mut chosen);
            if let Err(e) = picked {
                raw.destroy_image(image, None);
                raw.free_memory(memory, None);
                return Err(format!("reading a shared image's modifier: {e}"));
            }
            let layout = raw.get_image_subresource_layout(
                image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                    mip_level: 0,
                    array_layer: 0,
                },
            );
            let texture = wrap(
                device,
                hal,
                image,
                memory,
                size,
                TARGET_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            Ok(Slot {
                texture,
                image: SharedImage {
                    fd,
                    offset: layout.offset as u32,
                    stride: layout.row_pitch as u32,
                },
                modifier: chosen.drm_format_modifier,
            })
        }
    }

    fn image_info(
        size: UVec2,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> vk::ImageCreateInfo<'static> {
        vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: size.x,
                height: size.y,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
    }

    /// Creates the image, gives it dedicated exportable memory and exports
    /// that as a dma-buf. `local` prefers video memory, which a tiled image
    /// the same GPU reads back wants; otherwise system memory, which any GPU
    /// can read.
    unsafe fn create(
        hal: &wgpu::hal::vulkan::Device,
        info: &vk::ImageCreateInfo,
        local: bool,
    ) -> Result<(vk::Image, vk::DeviceMemory, OwnedFd), String> {
        // SAFETY: the caller's; everything is made on wgpu's device.
        unsafe {
            let raw = hal.raw_device();
            let instance = hal.shared_instance().raw_instance();
            let image = raw
                .create_image(info, None)
                .map_err(|e| format!("creating a shared image: {e}"))?;

            let needs = raw.get_image_memory_requirements(image);
            let properties =
                instance.get_physical_device_memory_properties(hal.raw_physical_device());
            let allowed = |i: u32| needs.memory_type_bits & (1 << i) != 0;
            let local_type = |i: u32| {
                properties.memory_types[i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            };
            let Some(memory_type) = (0..properties.memory_type_count)
                .find(|&i| allowed(i) && local_type(i) == local)
                .or_else(|| (0..properties.memory_type_count).find(|&i| allowed(i)))
            else {
                raw.destroy_image(image, None);
                return Err("no memory can back a shared image".to_string());
            };
            let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(HANDLE);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let allocation = vk::MemoryAllocateInfo::default()
                .allocation_size(needs.size)
                .memory_type_index(memory_type)
                .push_next(&mut export)
                .push_next(&mut dedicated);
            let memory = match raw.allocate_memory(&allocation, None) {
                Ok(memory) => memory,
                Err(e) => {
                    raw.destroy_image(image, None);
                    return Err(format!("allocating a shared image: {e}"));
                }
            };
            let exported = raw.bind_image_memory(image, memory, 0).and_then(|()| {
                khr::external_memory_fd::Device::new(instance, raw).get_memory_fd(
                    &vk::MemoryGetFdInfoKHR::default()
                        .memory(memory)
                        .handle_type(HANDLE),
                )
            });
            match exported {
                Ok(fd) => Ok((image, memory, OwnedFd::from_raw_fd(fd))),
                Err(e) => {
                    raw.destroy_image(image, None);
                    raw.free_memory(memory, None);
                    Err(format!("exporting a shared image: {e}"))
                }
            }
        }
    }

    /// Hands `image` and its memory to wgpu, which frees both with the texture.
    unsafe fn wrap(
        device: &wgpu::Device,
        hal: impl std::ops::Deref<Target = wgpu::hal::vulkan::Device>,
        image: vk::Image,
        memory: vk::DeviceMemory,
        size: UVec2,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        let uses = if usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            wgpu::TextureUses::COLOR_TARGET
        } else {
            wgpu::TextureUses::COPY_DST
        };
        // SAFETY: the caller's; `image` was made on this device.
        unsafe {
            let hal_texture = hal.texture_from_raw(
                image,
                &wgpu::hal::TextureDescriptor {
                    label: Some("game view"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: uses,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                },
                None,
                wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
            );
            drop(hal);
            device.create_texture_from_hal::<Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("game view"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images(count: usize) -> Vec<SharedImage> {
        (0..count)
            .map(|_| SharedImage {
                fd: std::fs::File::open("/dev/null").unwrap().into(),
                offset: 0,
                stride: 0,
            })
            .collect()
    }

    #[test]
    fn a_slot_being_shown_or_waiting_is_never_drawn_into() {
        let exchange = FrameExchange::new(|| {});
        let generation = exchange.install(4, 4, MODIFIER_LINEAR, images(3));
        let first = exchange.claim(generation).unwrap();
        exchange.finished(generation, first);
        assert_eq!(exchange.latest(), Some((generation, first)));
        exchange.hold(generation, first);

        let second = exchange.claim(generation).unwrap();
        exchange.finished(generation, second);
        let third = exchange.claim(generation).unwrap();
        assert!(third != first && third != second);
        // Shown, ready and drawing: nothing left but the scratch target.
        assert_eq!(exchange.claim(generation), None);

        exchange.hold(generation, second);
        assert_eq!(exchange.claim(generation), Some(first));
    }

    #[test]
    fn a_world_waits_for_the_display_but_not_forever() {
        let exchange = FrameExchange::new(|| {});
        let seen = exchange.presented_count();
        // Nothing presented: the timeout lets it go.
        assert_eq!(
            exchange.wait_presented(seen, Duration::from_millis(5)),
            seen
        );
        // Presented during the update: no wait at all.
        exchange.presented();
        let started = std::time::Instant::now();
        assert_eq!(
            exchange.wait_presented(seen, Duration::from_secs(5)),
            seen + 1
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_new_ring_forgets_the_old_one() {
        let exchange = FrameExchange::new(|| {});
        let old = exchange.install(4, 4, MODIFIER_LINEAR, images(3));
        let slot = exchange.claim(old).unwrap();
        let new = exchange.install(8, 8, MODIFIER_LINEAR, images(3));
        exchange.finished(old, slot);
        assert_eq!(exchange.latest(), None);
        assert_eq!(exchange.claim(old), None);
        assert!(exchange.claim(new).is_some());
        exchange.clear();
        assert!(exchange.slots().is_none());
    }

    #[test]
    fn a_requested_size_stays_allocatable() {
        let exchange = FrameExchange::new(|| {});
        assert_eq!(exchange.wanted(), Viewport::default());
        exchange.resize(1920, 1080, 2.0);
        assert_eq!(exchange.wanted().size, UVec2::new(1920, 1080));
        exchange.resize(0, 100_000, f32::NAN);
        let wanted = exchange.wanted();
        assert_eq!(wanted.size, UVec2::new(MIN_SIZE, MAX_SIZE));
        assert_eq!(wanted.scale, 1.0);
    }

    #[test]
    fn a_refused_modifier_is_withdrawn_from_the_offer() {
        const TILED: u64 = 0x0300_0000_0060_6015;
        let exchange = FrameExchange::new(|| {});
        exchange.accept([MODIFIER_LINEAR, TILED, 7, TILED]);
        let offered = exchange.offer();
        // Linear is the fallback already, so never part of the offer.
        assert_eq!(offered.modifiers, vec![7, TILED]);
        // The same offer again changes nothing.
        exchange.accept([TILED, 7]);
        assert_eq!(exchange.offer().version, offered.version);

        let linear = exchange.install(4, 4, MODIFIER_LINEAR, images(3));
        exchange.refuse(linear);
        assert_eq!(exchange.offer(), offered);

        let tiled = exchange.install(4, 4, TILED, images(3));
        // A ring that isn't current any more can't withdraw anything.
        exchange.refuse(linear);
        assert_eq!(exchange.offer(), offered);
        exchange.refuse(tiled);
        let after = exchange.offer();
        assert_eq!(after.modifiers, vec![7]);
        assert!(after.version > offered.version);
    }

    // Need a Vulkan GPU with dma-buf export: `cargo test -p blockloom-runtime -- --ignored embedded`.
    #[test]
    #[ignore = "needs a GPU"]
    fn an_embedded_world_draws_into_shared_images() {
        let (set, index, errors) = run_red_world(|_| {});
        let (set, index) = (
            set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}")),
            index,
        );
        assert_eq!(set.images.len(), SLOTS);
        assert_eq!(set.modifier, MODIFIER_LINEAR);
        // Red first in memory, the way XBGR8888 says.
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(
            pixel[0] > 150 && pixel[1] < 80 && pixel[2] < 80,
            "expected red, read {pixel:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn an_embedded_world_draws_straight_into_a_tiled_ring() {
        // A real viewer offers what EGL samples; here, whatever the GPU draws.
        let modifiers = gpu_modifiers();
        assert!(!modifiers.is_empty(), "this GPU has no drawable modifiers");
        let (set, _, errors) = run_red_world(|exchange| exchange.accept(modifiers));
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_ne!(
            set.modifier, MODIFIER_LINEAR,
            "the world stayed on linear frames"
        );
        assert_eq!(set.images.len(), SLOTS);
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn an_embedded_world_draws_at_the_size_the_view_asks_for() {
        let (set, index, errors) = run_red_world(|exchange| exchange.resize(640, 360, 2.0));
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert_eq!((set.width, set.height), (640, 360));
        let pixel = middle_pixel(&set.images[index], 640, 360);
        assert!(
            pixel[0] > 150 && pixel[1] < 80,
            "expected red, read {pixel:?}"
        );
    }

    fn gpu_modifiers() -> Vec<u64> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
            .expect("a Vulkan adapter");
        // SAFETY: queries only, on the adapter's own physical device.
        unsafe {
            let hal = adapter.as_hal::<wgpu::hal::api::Vulkan>().unwrap();
            dmabuf::drawable_modifiers(
                hal.shared_instance().raw_instance(),
                hal.raw_physical_device(),
            )
        }
    }

    /// Runs a world with a red background until a frame shows it, answering
    /// the ring it arrived in, which slot, and any errors the world sent.
    fn run_red_world(
        offer: impl FnOnce(&FrameExchange),
    ) -> (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>) {
        blockloom_core::init();
        let (to_world, incoming) = std::sync::mpsc::channel();
        let (outgoing, reports) = std::sync::mpsc::channel();
        let exchange = FrameExchange::new(|| {});
        offer(&exchange);
        let size = exchange.wanted().size;
        let frames = exchange.clone();
        let world = std::thread::spawn(move || {
            run(Embedded {
                mode: Mode::TwoD,
                incoming,
                outgoing,
                frames,
            })
        });
        let mut red = blockloom_core::project::Project::starter("Embedded", Mode::TwoD);
        red.world.background = "#ff0000".to_string();
        to_world
            .send(EditorMessage::Load {
                project: Box::new(red),
                dir: None,
            })
            .unwrap();

        // The first frames come before the world has a camera; wait for
        // several, so one that shows the background has landed.
        let started = std::time::Instant::now();
        let mut seen = None;
        let mut frames_seen = 0;
        while started.elapsed() < Duration::from_secs(20) {
            if let (Some(set), Some((generation, index))) = (exchange.slots(), exchange.latest())
                && set.width == size.x
                && set.generation == generation
            {
                // Keep the world off this slot while it is read.
                exchange.hold(generation, index);
                exchange.presented();
                seen = Some((set.clone(), index));
                frames_seen += 1;
                if set.modifier != MODIFIER_LINEAR && frames_seen > 30 {
                    break;
                }
                if set.modifier == MODIFIER_LINEAR {
                    let [r, g, b] =
                        middle_pixel(&set.images[index], size.x as usize, size.y as usize);
                    if r > 150 && g < 80 && b < 80 {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        to_world.send(EditorMessage::Shutdown).unwrap();
        drop(to_world);
        world.join().unwrap();
        let errors: Vec<_> = reports
            .try_iter()
            .filter(|report| {
                matches!(
                    report,
                    RuntimeMessage::Error { .. } | RuntimeMessage::Fatal { .. }
                )
            })
            .collect();
        // The world's end is the viewer's cue to show nothing.
        assert!(exchange.slots().is_none());
        let (set, index) = seen.map_or((None, 0), |(set, index)| (Some(set), index));
        (set, index, errors)
    }

    /// Reads a pixel straight out of a linear dma-buf.
    fn middle_pixel(image: &SharedImage, width: usize, height: usize) -> [u8; 3] {
        use std::os::fd::AsRawFd;
        assert!(image.stride as usize >= width * 4);
        let length = image.offset as usize + image.stride as usize * height;
        // SAFETY: a read-only mapping of the buffer's own length, unmapped
        // before returning.
        unsafe {
            let mapped = libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ,
                libc::MAP_SHARED,
                image.fd.as_raw_fd(),
                0,
            );
            assert_ne!(mapped, libc::MAP_FAILED, "the dma-buf can't be mapped");
            let bytes = std::slice::from_raw_parts(mapped as *const u8, length);
            let at = image.offset as usize + image.stride as usize * (height / 2) + width / 2 * 4;
            let pixel = [bytes[at], bytes[at + 1], bytes[at + 2]];
            libc::munmap(mapped, length);
            pixel
        }
    }
}
