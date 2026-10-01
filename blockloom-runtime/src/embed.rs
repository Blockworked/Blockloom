//! The world embedded in the editor: no window and no pipes.
//!
//! The editor calls [`run`] on a thread of its own. Messages travel over
//! channels. Every world camera draws straight into a small ring of dma-bufs
//! the editor's Game view imports through EGL, laid out in a tiled DRM format
//! modifier the viewer said it can sample. With no such modifier, frames are
//! copied into linear system-memory images instead - the CPU never touches a
//! pixel either way.
//!
//! The ring is 8-bit, so an HDR frame can't travel through it. On Wayland the
//! viewer also offers a surface of its own, under its window, and a frame the
//! project wants HDR goes there instead: a swapchain in scRGB or HDR10,
//! presented straight to the compositor while the view shows through to it.

use crate::bridge;
use crate::display::{self, Formats};
use crate::engine::Engine;
use crate::hdr::{DisplayOffers, HdrFrame, HdrMetadata};
use crate::world::WorldCamera;
use bevy::app::{PluginsState, TerminalCtrlCHandlerPlugin};
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::{ManualTextureViewHandle, RenderTarget, ScalingMode};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::TextureView;
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderInstance, RenderQueue};
use bevy::render::texture::{ManualTextureView, ManualTextureViews, OutputColorAttachment};
use bevy::render::view::{ViewTargetAttachments, clear_view_attachments, prepare_view_attachments};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::ui::UiScale;
use bevy::window::ExitCondition;
use blockloom_core::scene::{Mode, OutputSpace};
use blockloom_protocol::{EditorMessage, GAME_SIZE, RuntimeMessage};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    // Same DLSS project id as the process runtime; must land before
    // `DefaultPlugins` holds `DlssInitPlugin` under the `dlss` feature.
    #[cfg(all(feature = "dlss", not(target_arch = "wasm32")))]
    app.insert_resource(bevy::anti_alias::dlss::DlssProjectId(
        bevy::asset::uuid::uuid!("259f6fa9-7a86-42c3-a74a-91643c0b9c7b"),
    ));
    let mut vulkan = dmabuf::vulkan_settings();
    display::add_vulkan_extensions(&mut vulkan);
    app.insert_resource(vulkan);
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
        let update_start = std::time::Instant::now();
        app.update();
        let update_ms = update_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(exit) = app.should_exit() {
            return exit;
        }
        let wait_start = std::time::Instant::now();
        seen = frames.wait_presented(seen, PACE_TIMEOUT);
        let wait_ms = wait_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(mut pace) = app
            .world_mut()
            .get_resource_mut::<crate::performance::LoopPace>()
        {
            pace.push(update_ms, wait_ms);
        }
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
    /// The viewer's Wayland surface for HDR frames, as the addresses of its
    /// `wl_display` and `wl_surface`. It outlives every world.
    hdr_target: Mutex<Option<(usize, usize)>>,
    /// Whether a world is presenting there rather than into the ring.
    hdr_live: AtomicBool,
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
            hdr_target: Mutex::new(None),
            hdr_live: AtomicBool::new(false),
        })
    }

    /// The viewer made a Wayland surface under its window that a world may
    /// present HDR frames to. It must stay alive as long as the exchange.
    pub fn offer_hdr_surface(&self, display: usize, surface: usize) {
        if let Ok(mut target) = self.hdr_target.lock() {
            *target = Some((display, surface));
        }
    }

    fn hdr_target(&self) -> Option<(usize, usize)> {
        self.hdr_target.lock().ok().and_then(|target| *target)
    }

    /// Whether the world's frames are going to the HDR surface, so the
    /// view should show through to it rather than show the ring.
    pub fn hdr_live(&self) -> bool {
        self.hdr_live.load(Ordering::Acquire)
    }

    fn set_hdr_live(&self, live: bool) {
        if self.hdr_live.swap(live, Ordering::AcqRel) != live {
            (self.wake)();
        }
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
        self.hdr_live.store(false, Ordering::Release);
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
#[extract_app(RenderApp)]
struct FrameCopy {
    exchange: Arc<FrameExchange>,
    generation: u64,
    direct: bool,
    modifier: u64,
    /// Where the cameras draw when no slot is free, and the copy's source.
    scratch: Option<wgpu::Texture>,
    ring: Vec<wgpu::Texture>,
    views: Vec<TextureView>,
    /// The format the cameras draw in on the HDR surface this frame, or
    /// none while frames go through the ring.
    hdr: Option<wgpu::TextureFormat>,
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
        hdr: None,
    })
    .add_plugins(ExtractResourcePlugin::<FrameCopy>::default())
    .add_systems(Update, (target_cameras, fit_cameras))
    .add_systems(Last, build_surface);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<Drawing>()
            .init_resource::<HdrPlane>()
            .add_systems(
                Render,
                (
                    (aim_plane, aim_cameras)
                        .in_set(RenderSystems::PrepareViews)
                        .after(clear_view_attachments)
                        .before(prepare_view_attachments),
                    finish_frame.in_set(RenderSystems::Cleanup),
                ),
            );
    }
}

/// Points every world camera at the shared view. Quality scaling chooses
/// whether the interface draws here or through its native overlay camera.
fn target_cameras(
    mut commands: Commands,
    cameras: Query<(Entity, &RenderTarget), With<WorldCamera>>,
) {
    for (entity, target) in &cameras {
        let aimed = matches!(target, RenderTarget::TextureView(handle) if *handle == VIEW);
        if !aimed {
            commands
                .entity(entity)
                .insert(RenderTarget::TextureView(VIEW));
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
    mut target_bytes: ResMut<crate::performance::GameViewTargetBytes>,
    device: Res<RenderDevice>,
    frame: Res<HdrFrame>,
    mut views: ResMut<ManualTextureViews>,
) {
    let offer = surface.exchange.offer();
    let wanted = surface.exchange.wanted();
    let resized = wanted.size != surface.viewport.size;
    // An HDR frame only ever resolves once the HDR surface has offered it.
    let hdr = plane_format(frame.space);
    let reformatted = hdr != copy.hdr;
    surface.viewport.scale = wanted.scale;
    if surface.failed
        || (surface.seen == Some(offer.version)
            && !resized
            && !reformatted
            && copy.scratch.is_some())
    {
        return;
    }
    // A new format alone leaves the ring be; a new offer or size doesn't.
    let ring_stale = surface.seen != Some(offer.version) || resized || copy.ring.is_empty();
    surface.seen = Some(offer.version);
    surface.viewport.size = wanted.size;
    let size = wanted.size;
    let device = device.wgpu_device();
    if copy.scratch.is_none() || resized || reformatted {
        // The cameras' pipelines follow this view's format, so it is the HDR
        // surface's while frames go there.
        let format = hdr.unwrap_or(TARGET_FORMAT);
        let scratch = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game view target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        views.insert(
            VIEW,
            ManualTextureView {
                texture_view: scratch
                    .create_view(&wgpu::TextureViewDescriptor::default())
                    .into(),
                size,
                view_format: format,
            },
        );
        copy.scratch = Some(scratch);
        copy.hdr = hdr;
    }
    if !ring_stale {
        return;
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
    // A linear ring is only copied into, never drawn or sampled through
    // wgpu: a view on a COPY_DST-only image fails Vulkan validation
    // (VUID-VkImageViewCreateInfo-image-04441), so only a direct ring gets
    // views. `aim_cameras` only reads them when `direct` is set.
    copy.views = if direct {
        textures
            .iter()
            .map(|texture| {
                texture
                    .create_view(&wgpu::TextureViewDescriptor::default())
                    .into()
            })
            .collect()
    } else {
        Vec::new()
    };
    copy.ring = textures;
    target_bytes.0 = size.x as u64 * size.y as u64 * 4 * (copy.ring.len() as u64 + 1);
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
    let Some(copy) = copy.filter(|copy| copy.direct && copy.hdr.is_none()) else {
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
    mut plane: ResMut<HdrPlane>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let drawn = drawing.0.take();
    if let Some(texture) = plane.frame.take() {
        queue.present(texture);
        if let Some(copy) = &copy {
            copy.exchange.set_hdr_live(true);
        }
    } else if let Some(copy) = copy.as_ref().filter(|copy| copy.hdr.is_none())
        && plane.surface.is_some()
    {
        copy.exchange.set_hdr_live(false);
    }
    if let Some(copy) = copy.filter(|copy| copy.hdr.is_none()) {
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

/// The format a frame goes to the HDR surface in, or none for the ring.
/// Matches what `display::Formats::configure` picks for the space.
fn plane_format(space: OutputSpace) -> Option<wgpu::TextureFormat> {
    match space {
        OutputSpace::Sdr => None,
        OutputSpace::Scrgb => Some(wgpu::TextureFormat::Rgba16Float),
        OutputSpace::Hdr10 => Some(wgpu::TextureFormat::Rgb10a2Unorm),
    }
}

/// The swapchain on the viewer's HDR surface. Render world only.
#[derive(Resource, Default)]
struct HdrPlane {
    surface: Option<wgpu::Surface<'static>>,
    formats: Option<Formats>,
    config: Option<wgpu::SurfaceConfiguration>,
    metadata: Option<HdrMetadata>,
    /// This frame's texture, presented once the cameras are submitted.
    frame: Option<wgpu::SurfaceTexture>,
    /// Set once the surface was made or found wanting, so it is tried once.
    tried: bool,
    /// Set once an SDR configure installed the surface's error sink, so a
    /// refused HDR configure degrades instead of panicking the acquire.
    seeded: bool,
    /// Consecutive acquires with no texture: a surface that never yields
    /// one is dropped back to the 8-bit ring rather than retried forever.
    misses: u32,
}

/// Makes a swapchain on the viewer's HDR surface the first time there is
/// one, says what it offers, and on a frame going out HDR points the
/// cameras at its next texture.
#[allow(clippy::too_many_arguments)]
fn aim_plane(
    copy: Option<Res<FrameCopy>>,
    frame: Res<HdrFrame>,
    offers: Res<DisplayOffers>,
    instance: Res<RenderInstance>,
    adapter: Res<RenderAdapter>,
    device: Res<RenderDevice>,
    mut plane: ResMut<HdrPlane>,
    mut attachments: ResMut<ViewTargetAttachments>,
) {
    let Some(copy) = copy else { return };
    let plane = &mut *plane;
    if !plane.tried
        && let Some((display, surface)) = copy.exchange.hdr_target()
    {
        plane.tried = true;
        adopt_plane(plane, display, surface, &instance, &adapter, &offers);
    }
    let (Some(format), Some(surface), Some(formats), Some(scratch)) =
        (copy.hdr, &plane.surface, plane.formats, &copy.scratch)
    else {
        return;
    };
    let (configured, color_space) = formats.configure(frame.space);
    if configured != format {
        return;
    }
    let size = scratch.size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    let caps = surface.get_capabilities(&adapter);
    if !display::takes(&caps, format, color_space) {
        // Caps claimed a space the device refuses (seen on some Wayland
        // drivers); drop the plane and fall back to the 8-bit ring rather
        // than failing the run on the configure below.
        tracing::info!("game view: HDR surface refused {format:?} {color_space:?}");
        drop_plane(plane, &offers);
        return;
    }
    let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|mode| caps.present_modes.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo);
    // A surface that never configured has no error sink, so a refused
    // configure below would turn the acquire after it into a fatal panic.
    // Seed an SDR configure first: the 8-bit pair all but always takes,
    // and from then on a refusal is a logged error plus an early return.
    if !plane.seeded {
        plane.seeded = true;
        let (sdr, _) = formats.configure(OutputSpace::Sdr);
        let view = display::view_format(sdr);
        device.configure_surface(
            surface,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: sdr,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: size.width,
                height: size.height,
                present_mode,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: if view != sdr { vec![view] } else { vec![] },
            },
        );
    }
    let wanted = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space,
        width: size.width,
        height: size.height,
        // The view paces the world; the surface never makes it wait.
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
        view_formats: Vec::new(),
    };
    if plane.config.as_ref() != Some(&wanted) {
        device.configure_surface(surface, &wanted);
        // Wayland answers caps live, so the check above and the configure's
        // own check can disagree: re-check after, and drop the plane when
        // they do rather than acquiring a surface that never configured.
        let caps = surface.get_capabilities(&adapter);
        if !display::takes(&caps, format, color_space) {
            tracing::info!(
                "game view: HDR surface refused {format:?} {color_space:?} on configure"
            );
            drop_plane(plane, &offers);
            return;
        }
        plane.config = Some(wanted);
        plane.metadata = None;
    }
    let mut texture = surface.get_current_texture();
    if matches!(
        texture,
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
    ) {
        if let Some(config) = &plane.config {
            device.configure_surface(surface, config);
        }
        plane.metadata = None;
        texture = surface.get_current_texture();
    }
    let texture = match texture {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            plane.misses = 0;
            texture
        }
        // A surface that never yields a texture is dropped back to the
        // ring: transient timeouts recover on their own, a dead surface
        // does not, and the seed above keeps every attempt non-fatal.
        _ => {
            plane.misses += 1;
            if plane.misses >= 5 {
                tracing::info!("game view: HDR surface yielded no texture, falling back to SDR");
                drop_plane(plane, &offers);
            }
            return;
        }
    };
    let metadata = frame.metadata();
    if metadata != plane.metadata {
        if let Some(metadata) = &metadata {
            display::send_metadata(surface, device.wgpu_device(), metadata);
        }
        plane.metadata = metadata;
    }
    let view = texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    attachments.insert(
        NormalizedRenderTarget::TextureView(VIEW),
        OutputColorAttachment::new(view.into(), format),
    );
    plane.frame = Some(texture);
}

/// Drops the plane back to the 8-bit ring: the next frames rebuild without
/// it, and the offers drive the frame's space back to SDR with them.
fn drop_plane(plane: &mut HdrPlane, offers: &DisplayOffers) {
    plane.surface = None;
    plane.formats = None;
    plane.config = None;
    plane.metadata = None;
    plane.misses = 0;
    offers.set(vec![OutputSpace::Sdr]);
}

/// Makes the swapchain's surface on the viewer's `wl_surface`, and keeps
/// it only if the compositor takes an HDR space there.
fn adopt_plane(
    plane: &mut HdrPlane,
    display: usize,
    surface: usize,
    instance: &RenderInstance,
    adapter: &RenderAdapter,
    offers: &DisplayOffers,
) {
    use std::ptr::NonNull;
    use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
    let (Some(display), Some(surface)) = (
        NonNull::new(display as *mut std::ffi::c_void),
        NonNull::new(surface as *mut std::ffi::c_void),
    ) else {
        return;
    };
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        ))),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
    };
    // SAFETY: the viewer keeps the display and surface alive for as long as
    // the exchange, which outlives this world.
    let surface = match unsafe { instance.create_surface_unsafe(target) } {
        Ok(surface) => surface,
        Err(error) => {
            tracing::info!("game view: no HDR surface: {error}");
            offers.set(vec![OutputSpace::Sdr]);
            return;
        }
    };
    let caps = surface.get_capabilities(adapter);
    match Formats::of(&caps).filter(|formats| formats.offers().len() > 1) {
        Some(formats) => {
            tracing::info!("game view: HDR surface offers {:?}", formats.offers());
            offers.set(formats.offers());
            plane.formats = Some(formats);
            plane.surface = Some(surface);
        }
        None => offers.set(vec![OutputSpace::Sdr]),
    }
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
    const DMA_BUF: vk::ExternalMemoryHandleTypeFlags =
        vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;

    /// How slots are shared. A test on a software driver with no dma-bufs
    /// (lavapipe without udmabuf) asks for an opaque fd, which it backs with
    /// a memfd the test can map all the same.
    fn handle() -> vk::ExternalMemoryHandleTypeFlags {
        #[cfg(test)]
        if std::env::var_os("BLOCKLOOM_TEST_OPAQUE_FD").is_some() {
            return vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD;
        }
        DMA_BUF
    }

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
            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle());
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
                        vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(handle());
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
            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle());
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
            let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(handle());
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
                        .handle_type(handle()),
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
                wgpu::TextureUses::UNINITIALIZED,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_protocol::SceneView;

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
        let (set, index, errors) =
            run_world(red_world(Mode::TwoD), |_| {}, game_camera(), 0, is_red);
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
        let (set, _, errors) = run_world(
            red_world(Mode::TwoD),
            |exchange| exchange.accept(modifiers),
            game_camera(),
            0,
            is_red,
        );
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
        let (set, index, errors) = run_world(
            red_world(Mode::TwoD),
            |exchange| exchange.resize(640, 360, 2.0),
            game_camera(),
            0,
            is_red,
        );
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert_eq!((set.width, set.height), (640, 360));
        let pixel = middle_pixel(&set.images[index], 640, 360);
        assert!(
            pixel[0] > 150 && pixel[1] < 80,
            "expected red, read {pixel:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn quality_scaling_preserves_native_output() {
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let project = red_world(mode);
            let (native, native_index, errors) =
                run_world(project.clone(), |_| {}, game_camera(), 60, |_| false);
            assert!(errors.is_empty(), "{errors:?}");
            let native = native.expect("native view produced no frame");
            let reference = frame_pixels(
                &native.images[native_index],
                SIZE.x as usize,
                SIZE.y as usize,
            );
            let mut project = project;
            project.world.quality.resolution_scale = 0.5;
            project.world.quality.upscaler = blockloom_core::quality::Upscaler::Taa;
            let (set, index, errors) = run_world(project, |_| {}, game_camera(), 60, |_| false);
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.expect("scaled view produced no frame");
            assert_eq!((set.width, set.height), (SIZE.x, SIZE.y));
            let scaled = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            for (x, y) in [(0, 0), (SIZE.x / 2, SIZE.y / 2), (SIZE.x - 1, SIZE.y - 1)] {
                let i = (y * SIZE.x + x) as usize;
                assert!(
                    scaled[i]
                        .iter()
                        .zip(reference[i])
                        .all(|(a, b)| a.abs_diff(b) < 20),
                    "{mode:?} at ({x},{y}): native {:?}, scaled {:?}",
                    reference[i],
                    scaled[i]
                );
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn quality_scaling_keeps_ui_edges_at_native_resolution() {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::ui::{UiAnchor, UiProp};
        use blockloom_core::value::Value;

        for mode in [Mode::TwoD, Mode::ThreeD] {
            let mut project = red_world(mode);
            for actor in &mut project.actors {
                actor.components.remove("Look");
                actor.components.remove("Body");
            }
            project.world.quality.resolution_scale = 0.5;
            let mut blocks = vec![Instruction::new(K::WhenStarted)];
            for (id, x, width, color) in [
                ("stripe", 31.0, 1.0, "#ffffff"),
                ("tint", 80.0, 12.0, "#0000ff80"),
            ] {
                blocks.push(Instruction::new(K::ShowPanel {
                    element: Value::text(id),
                    title: Value::text(""),
                    modal: false,
                    anchor: UiAnchor::TopLeft,
                    x: Value::number(x),
                    y: Value::number(20.0),
                    width: Value::number(width),
                    height: Value::number(48.0),
                    parent: Value::text(""),
                }));
                for (prop, value) in [
                    (UiProp::Background, Value::text(color)),
                    (UiProp::Padding, Value::number(0.0)),
                    (UiProp::CornerRadius, Value::number(0.0)),
                ] {
                    blocks.push(Instruction::new(K::SetUiProp {
                        prop,
                        element: Value::text(id),
                        value,
                    }));
                }
            }
            project.actors[0]
                .graph
                .strands
                .push(Strand::with_instructions(0, 0, blocks));
            let (set, index, errors) = run_world_sending(
                project,
                |_| {},
                game_camera(),
                90,
                |_| false,
                vec![EditorMessage::Start],
            );
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.expect("scaled UI produced no frame");
            let pixels = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            let white = pixels[(40 * SIZE.x + 31) as usize];
            assert!(white.iter().all(|c| *c > 230), "{mode:?}: stripe {white:?}");
            let [r, g, b] = pixels[(40 * SIZE.x + 85) as usize];
            assert!(
                r > 100 && r < 220 && g < 30 && b > 170 && b < 210,
                "{mode:?}: translucent tint {:?}",
                [r, g, b]
            );
            for x in [30, 32] {
                let adjacent = pixels[(40 * SIZE.x + x) as usize];
                assert!(is_red(adjacent), "{mode:?}: neighbour {x}: {adjacent:?}");
            }
            assert!(is_red(pixels[(SIZE.y / 2 * SIZE.x + SIZE.x / 2) as usize]));
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn an_embedded_world_shows_false_color_over_its_hdr_frame() {
        // Red's luminance sits a quarter stop over middle grey: the green band.
        let view = SceneView {
            debug_view: blockloom_protocol::DebugView::FalseColor,
            ..game_camera()
        };
        let green = |[r, g, b]: [u8; 3]| g > 180 && r < 140 && b < 140;
        let (set, index, errors) = run_world(red_world(Mode::TwoD), |_| {}, view, 0, green);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(green(pixel), "expected the green band, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_3d_world_shows_false_color_too() {
        let view = SceneView {
            debug_view: blockloom_protocol::DebugView::FalseColor,
            ..game_camera()
        };
        let (set, index, errors) = run_world(red_world(Mode::ThreeD), |_| {}, view, 60, |_| false);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(
            is_false_color(pixel),
            "expected a false color band, read {pixel:?}"
        );
    }

    /// The red world with its post-process changed by `edit`.
    fn graded_red(
        mode: Mode,
        edit: impl FnOnce(&mut blockloom_core::scene::PostProcess),
    ) -> blockloom_core::project::Project {
        let mut red = red_world(mode);
        edit(&mut red.world.post);
        red
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn half_size_ssr_keeps_environment_reflections_under_scene_scaling() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::material::SurfaceMaterial;
        use blockloom_core::sky::SkyKind;
        for scale in [1.0, 0.53] {
            let mut room = dark_room(false);
            room.world.quality.resolution_scale = scale;
            room.world.post.ssr.enabled = true;
            room.world.sky.kind = SkyKind::Gradient;
            room.world.sky.lighting = false;
            room.world.sky.reflections = true;
            room.world.sky.gradient.top = "#FF0000".into();
            room.world.sky.gradient.middle = "#FF0000".into();
            room.world.sky.gradient.bottom = "#FF0000".into();
            room.actors[0].components.insert(ActorComponent::Material {
                material: SurfaceMaterial {
                    metallic: 1.0,
                    roughness: 0.2,
                    ..Default::default()
                },
            });
            let (set, index, reports) =
                run_world_reporting(room, |_| {}, game_camera(), 180, |_| false, Vec::new());
            assert!(
                reports.iter().all(|r| !matches!(
                    r,
                    RuntimeMessage::Error { .. } | RuntimeMessage::Fatal { .. }
                )),
                "{reports:?}"
            );
            assert!(
                reports.iter().any(|report| match report {
                    RuntimeMessage::Status(status) => status
                        .render_metrics
                        .iter()
                        .any(|metric| metric.name.contains("ssr_half")),
                    _ => false,
                }),
                "half-size SSR never ran at scale {scale}"
            );
            let set = set.expect("SSR produced no frame");
            let pixels = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            dump(&format!("ssr-half-{scale}"), &pixels);
            let pixel = pixels[(SIZE.y / 2 * SIZE.x + SIZE.x / 2) as usize];
            assert!(
                is_red(pixel),
                "environment reflection at scale {scale}: {pixel:?}"
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn grading_to_no_saturation_turns_red_grey() {
        let grey = |[r, g, b]: [u8; 3]| r > 20 && r.abs_diff(g) < 8 && r.abs_diff(b) < 8;
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let project = graded_red(mode, |post| post.grading.saturation = 0.0);
            let (set, index, errors) = run_world(project, |_| {}, game_camera(), 600, grey);
            let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
            assert!(errors.is_empty(), "{errors:?}");
            let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            assert!(grey(pixel), "{mode:?}: expected grey, read {pixel:?}");
        }
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn the_bloom_chain_shows_each_level() {
        let project = graded_red(Mode::TwoD, |post| {
            post.bloom_enabled = true;
            post.bloom_threshold = 0.0;
        });
        for mip in [0, 4] {
            let view = SceneView {
                debug_view: blockloom_protocol::DebugView::BloomMip,
                bloom_mip: mip,
                ..game_camera()
            };
            let (set, index, errors) = run_world(project.clone(), |_| {}, view, 600, is_red);
            let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
            assert!(errors.is_empty(), "{errors:?}");
            let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            assert!(
                is_red(pixel),
                "level {mip}: expected red bloom, read {pixel:?}"
            );
        }
    }

    /// Also shows the SSAO and depth of field shader patches apply: a patch
    /// that no longer matches Bevy reports an error.
    #[test]
    #[ignore = "needs a GPU"]
    fn ao_and_blur_size_debug_views_draw_in_3d() {
        let mut project = graded_red(Mode::ThreeD, |post| {
            post.depth_of_field.enabled = true;
            post.depth_of_field.focus_distance = 0.1;
            post.depth_of_field.f_stops = 0.5;
        });
        project.world.lighting.ao_enabled = true;
        let grey = |[r, g, b]: [u8; 3]| r > 60 && r.abs_diff(g) < 8 && r.abs_diff(b) < 8;
        let view = SceneView {
            debug_view: blockloom_protocol::DebugView::AmbientOcclusion,
            ..game_camera()
        };
        let (set, index, errors) = run_world(project.clone(), |_| {}, view, 600, grey);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(grey(pixel), "expected AO in grey, read {pixel:?}");

        // Everything past a 10 cm focus at f/0.5 is far: blue.
        let blue = |[r, g, b]: [u8; 3]| b > 150 && b > g && g > r;
        let view = SceneView {
            debug_view: blockloom_protocol::DebugView::CircleOfConfusion,
            ..game_camera()
        };
        let (set, index, errors) = run_world(project, |_| {}, view, 600, blue);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(blue(pixel), "expected far blur in blue, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn film_grain_breaks_up_a_flat_color() {
        let project = graded_red(Mode::TwoD, |post| post.grain.intensity = 1.0);
        let (set, index, errors) = run_world(project, |_| {}, game_camera(), 600, |_| false);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        let frame = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        let row = &frame[(SIZE.y as usize / 2) * SIZE.x as usize..][..SIZE.x as usize];
        let reds: std::collections::HashSet<u8> = row.iter().map(|p| p[0]).collect();
        assert!(
            reds.len() > 8,
            "a grained row should vary, read {} reds",
            reds.len()
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_light_component_lights_the_ground_after_batching() {
        let lit = run_world(dark_room(true), |_| {}, game_camera(), 90, |_| false);
        let dark = run_world(dark_room(false), |_| {}, game_camera(), 90, |_| false);
        let middle = |(set, index, errors): (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>)| {
            let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
            middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize)
        };
        let (lit, dark) = (middle(lit), middle(dark));
        assert!(
            lit.iter().all(|c| *c > 120),
            "expected a lit floor, read {lit:?}"
        );
        assert!(
            dark.iter().all(|c| *c < 30),
            "expected a dark floor, read {dark:?}"
        );
    }

    /// The dark room lit only by what ray tracing traces: the lamp stands in
    /// as a glowing sphere, and the floor is a deferred instanced surface
    /// Solari lights in place of the raster lights. Leaving the lamp out of
    /// the traced world leaves the floor dark, which shows the raster lamp
    /// isn't what lit it.
    #[test]
    #[ignore = "needs a GPU"]
    fn ray_tracing_lights_the_floor_from_a_traced_lamp() {
        use blockloom_core::components::ActorComponent;
        let traced = |lamp_traced: bool| {
            let mut room = dark_room(true);
            room.world.lighting.ray_tracing.enabled = true;
            for actor in &mut room.actors {
                if let Some(ActorComponent::Light { light }) = actor.components.get_mut("Light") {
                    light.ray_traced = lamp_traced;
                }
            }
            // Traced light is noisy for its first frames: a lit floor is
            // waited for, a dark one settles.
            let settle = if lamp_traced { 0 } else { 120 };
            let done = move |pixel| lamp_traced && lit_floor(pixel);
            floor_pixel(run_world(room, |_| {}, game_camera(), settle, done))
        };
        let (lit, dark) = (traced(true), traced(false));
        assert!(lit_floor(lit), "expected a traced, lit floor, read {lit:?}");
        assert!(
            dark.iter().all(|c| *c < 20),
            "expected the untraced lamp to leave the floor dark, read {dark:?}"
        );
    }

    /// `turn ray tracing` mid-run moves the floor between Solari and the
    /// raster lights both ways, re-preparing every surface in between. The
    /// lamp is left out of the traced world, so only its raster light shows.
    #[test]
    #[ignore = "needs a GPU"]
    fn turning_ray_tracing_mid_run_switches_between_the_rigs() {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::components::ActorComponent;
        let run = |start_traced: bool, switch: Option<bool>| {
            let mut room = dark_room(true);
            room.world.lighting.ray_tracing.enabled = start_traced;
            for actor in &mut room.actors {
                if let Some(ActorComponent::Light { light }) = actor.components.get_mut("Light") {
                    light.ray_traced = false;
                }
            }
            if let Some(enabled) = switch {
                room.actors[0].graph.strands.push(Strand::with_instructions(
                    0,
                    0,
                    vec![
                        Instruction::new(K::WhenStarted),
                        Instruction::new(K::SetRayTracing { enabled }),
                    ],
                ));
            }
            // Lit is waited for; dark settles.
            let lit = switch == Some(false);
            floor_pixel(run_world_sending(
                room,
                |_| {},
                game_camera(),
                if lit { 0 } else { 120 },
                move |pixel| lit && pixel.iter().all(|c| *c > 120),
                vec![EditorMessage::Start],
            ))
        };
        let traced = run(true, None);
        assert!(
            traced.iter().all(|c| *c < 20),
            "expected the untraced lamp to leave the floor dark, read {traced:?}"
        );
        let off = run(true, Some(false));
        assert!(
            off.iter().all(|c| *c > 120),
            "expected the raster lamp back once tracing is off, read {off:?}"
        );
        let on = run(false, Some(true));
        assert!(
            on.iter().all(|c| *c < 20),
            "expected tracing turned on to take the raster lamp away, read {on:?}"
        );
    }

    /// The reference path tracer draws the view from the same traced world:
    /// lit by the traced lamp, dark without it whatever the raster lamp does.
    #[test]
    #[ignore = "needs a GPU"]
    fn the_path_tracer_lights_the_floor_from_a_traced_lamp() {
        use blockloom_core::components::ActorComponent;
        let view = SceneView {
            path_tracer: blockloom_protocol::PathTracerView {
                enabled: true,
                ..Default::default()
            },
            ..game_camera()
        };
        let traced = |lamp_traced: bool| {
            let mut room = dark_room(true);
            for actor in &mut room.actors {
                if let Some(ActorComponent::Light { light }) = actor.components.get_mut("Light") {
                    light.ray_traced = lamp_traced;
                }
            }
            let settle = if lamp_traced { 0 } else { 120 };
            let done = move |pixel| lamp_traced && lit_floor(pixel);
            floor_pixel(run_world(room, |_| {}, view.clone(), settle, done))
        };
        let (lit, dark) = (traced(true), traced(false));
        assert!(
            lit_floor(lit),
            "expected a path traced, lit floor, read {lit:?}"
        );
        assert!(
            dark.iter().all(|c| *c < 20),
            "expected the untraced lamp to leave the floor dark, read {dark:?}"
        );
    }

    /// The dark room with ray tracing on in `mode`.
    fn traced_room(
        mode: blockloom_core::scene::TracingMode,
        lamp: bool,
    ) -> blockloom_core::project::Project {
        let mut room = dark_room(lamp);
        let tracing = &mut room.world.lighting.ray_tracing;
        tracing.enabled = true;
        tracing.mode = mode;
        room
    }

    const TRACING_MODES: [blockloom_core::scene::TracingMode; 2] = [
        blockloom_core::scene::TracingMode::Hybrid,
        blockloom_core::scene::TracingMode::PathTraced,
    ];

    /// A traced ray that escapes sees the sky: with no lamp, no sun and no
    /// ambient, a white gradient sky is all that lights the floor.
    #[test]
    #[ignore = "needs a GPU"]
    fn traced_light_sees_the_sky() {
        use blockloom_core::sky::SkyKind;
        let reference = SceneView {
            path_tracer: blockloom_protocol::PathTracerView {
                enabled: true,
                ..Default::default()
            },
            ..game_camera()
        };
        // Each tracing mode, then the reference path tracer.
        let runs = TRACING_MODES
            .map(|mode| (format!("{mode:?}"), mode, game_camera()))
            .into_iter()
            .chain([("Reference".to_string(), TRACING_MODES[0], reference)]);
        for (name, mode, view) in runs {
            let mut room = traced_room(mode, false);
            let sky = &mut room.world.sky;
            sky.kind = SkyKind::Gradient;
            sky.reflections = false;
            for stop in [
                &mut sky.gradient.top,
                &mut sky.gradient.middle,
                &mut sky.gradient.bottom,
            ] {
                *stop = "#FFFFFF".to_string();
            }
            let lit = floor_pixel(run_world(room, |_| {}, view, 0, lit_floor));
            assert!(
                lit_floor(lit),
                "{name}: expected the sky's light, read {lit:?}"
            );
        }
    }

    /// Under a flat sky the ambient is what a traced ray escapes to.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_flat_ambient_lights_a_traced_floor() {
        for mode in TRACING_MODES {
            let mut room = traced_room(mode, false);
            room.world.lighting.ambient_brightness = 1500.0;
            let lit = floor_pixel(run_world(room, |_| {}, game_camera(), 0, lit_floor));
            assert!(
                lit_floor(lit),
                "{mode:?}: expected the ambient, read {lit:?}"
            );
        }
    }

    /// Realtime path tracing lights the floor from the traced lamp alone.
    #[test]
    #[ignore = "needs a GPU"]
    fn realtime_path_tracing_lights_the_floor_from_a_traced_lamp() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::scene::TracingMode;
        let traced = |lamp_traced: bool| {
            let mut room = traced_room(TracingMode::PathTraced, true);
            for actor in &mut room.actors {
                if let Some(ActorComponent::Light { light }) = actor.components.get_mut("Light") {
                    light.ray_traced = lamp_traced;
                }
            }
            let settle = if lamp_traced { 0 } else { 120 };
            let done = move |pixel| lamp_traced && lit_floor(pixel);
            floor_pixel(run_world(room, |_| {}, game_camera(), settle, done))
        };
        let (lit, dark) = (traced(true), traced(false));
        assert!(lit_floor(lit), "expected a traced, lit floor, read {lit:?}");
        assert!(
            dark.iter().all(|c| *c < 20),
            "expected the untraced lamp to leave the floor dark, read {dark:?}"
        );
    }

    /// One path a pixel is speckled; the filter smooths it into the same
    /// light rather than a darker or brighter one.
    #[test]
    #[ignore = "needs a GPU"]
    fn the_denoiser_smooths_path_traced_light() {
        use blockloom_core::scene::{Denoiser, TracingMode};
        let patch = |denoiser: Denoiser| {
            let mut room = traced_room(TracingMode::PathTraced, true);
            // One path a pixel off a small lamp and a dim ambient is noisy.
            room.world.lighting.ambient_brightness = 300.0;
            room.world.lighting.ray_tracing.denoiser = denoiser;
            room.world.lighting.ray_tracing.paths = 1;
            // Once lit, a while longer for the history to fill.
            let lit = std::cell::Cell::new(0);
            let done = |pixel| {
                if lit_floor(pixel) {
                    lit.set(lit.get() + 1);
                }
                lit.get() > 60
            };
            let (set, index, errors) = run_world(room, |_| {}, game_camera(), 0, done);
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.unwrap_or_else(|| panic!("no frame arrived"));
            let frame = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
            dump(&format!("denoise-{denoiser:?}"), &frame);
            patch_stats(&frame, 24)
        };
        let (raw_mean, raw_spread) = patch(Denoiser::None);
        let (mean, spread) = patch(Denoiser::Filter);
        assert!(raw_mean > 20.0, "expected a lit floor, mean {raw_mean}");
        assert!(
            spread < raw_spread * 0.5,
            "expected the filter to smooth the grain: raw {raw_spread}, filtered {spread}"
        );
        assert!(
            (mean - raw_mean).abs() < raw_mean * 0.25,
            "expected the same light: raw {raw_mean}, filtered {mean}"
        );
    }

    /// A traced spot lights its cone and leaves the floor outside it dark,
    /// where a bare glowing disk would light all of it.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_traced_spot_stays_in_its_cone() {
        use blockloom_core::components::{LightKind, LightSpec};
        let mut room = room_with(LightSpec {
            kind: LightKind::Spot,
            intensity: 1_000_000.0,
            outer_angle: 30.0,
            inner_angle: 20.0,
            radius: 0.1,
            ..LightSpec::default()
        });
        room.world.lighting.ray_tracing.enabled = true;
        let (set, index, errors) = run_world(room, |_| {}, game_camera(), 120, |_| false);
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.unwrap_or_else(|| panic!("no frame arrived"));
        let (width, height) = (SIZE.x as usize, SIZE.y as usize);
        let frame = frame_pixels(&set.images[index], width, height);
        dump("traced-spot", &frame);
        let brightest = frame
            .iter()
            .copied()
            .max_by_key(|p| p.iter().map(|c| *c as u32).sum::<u32>());
        let brightest = brightest.unwrap_or_default();
        let lit = frame.iter().filter(|p| p.iter().any(|c| *c > 20)).count();
        assert!(
            lit_floor(brightest),
            "expected the cone lit, read {brightest:?}"
        );
        assert!(
            lit < frame.len() / 20,
            "expected the floor outside the cone dark, {lit} of {} pixels lit",
            frame.len()
        );
    }

    fn lit_floor(pixel: [u8; 3]) -> bool {
        pixel.iter().all(|c| *c > 60)
    }

    /// The mean luminance of the square `radius` around the middle of a
    /// frame, and its grain: the mean step between neighbouring pixels, which
    /// noise raises and a smooth gradient barely does.
    fn patch_stats(frame: &[[u8; 3]], radius: usize) -> (f32, f32) {
        let (width, height) = (SIZE.x as usize, SIZE.y as usize);
        let luminance = |x: usize, y: usize| {
            let [r, g, b] = frame[y * width + x];
            0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
        };
        let (mut sum, mut grain, mut count) = (0.0, 0.0, 0.0);
        for y in height / 2 - radius..height / 2 + radius {
            for x in width / 2 - radius..width / 2 + radius {
                sum += luminance(x, y);
                grain += (luminance(x + 1, y) - luminance(x, y)).abs();
                count += 1.0;
            }
        }
        (sum / count, grain / count)
    }

    /// Writes a frame to `BLOCKLOOM_TEST_DUMP/<name>.png` when that is set,
    /// for looking at what a test saw.
    fn dump(name: &str, frame: &[[u8; 3]]) {
        let Some(dir) = std::env::var_os("BLOCKLOOM_TEST_DUMP") else {
            return;
        };
        let bytes: Vec<u8> = frame.iter().flatten().copied().collect();
        if let Some(image) = image::RgbImage::from_raw(SIZE.x, SIZE.y, bytes) {
            let _ = image.save(std::path::Path::new(&dir).join(format!("{name}.png")));
        }
    }

    fn floor_pixel(result: (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>)) -> [u8; 3] {
        let (set, index, errors) = result;
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.unwrap_or_else(|| panic!("no frame arrived"));
        middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize)
    }

    /// The dark room with one light hung over the middle of the floor,
    /// pointing straight down.
    fn room_with(light: blockloom_core::components::LightSpec) -> blockloom_core::project::Project {
        use blockloom_core::components::ActorComponent;
        let mut room = dark_room(false);
        let mut lamp = blockloom_core::project::Actor::new(
            "Lamp",
            blockloom_core::scene::Visual::Sphere {
                color: "#FFFFFF".to_string(),
                radius: 0.05,
            },
        );
        lamp.components.remove("Look");
        let place = lamp.components.placement_mut();
        place.position = [0.0, 2.0, 0.0];
        // Forward (-Z) turned to face the floor.
        place.rotation = [-90.0, 0.0, 0.0];
        lamp.components.insert(ActorComponent::Light { light });
        room.actors.push(lamp);
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_rect_light_lights_the_floor_with_the_shadow_rig_on() {
        use blockloom_core::components::{LightKind, LightSpec};
        let mut room = room_with(LightSpec {
            kind: LightKind::Rect,
            width: 2.0,
            height: 2.0,
            intensity: 1_000_000.0,
            ..LightSpec::default()
        });
        // Every shadow option at once, to show none of them trips the GPU.
        let shadows = &mut room.world.lighting.shadows;
        shadows.filter = blockloom_core::scene::ShadowFilter::Temporal;
        shadows.sun_size = 0.5;
        shadows.contact = true;
        shadows.cascades = 2;
        let lit = floor_pixel(run_world(room, |_| {}, game_camera(), 90, |_| false));
        assert!(
            lit.iter().all(|c| *c > 120),
            "expected a lit floor, read {lit:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_shadowed_rect_light_is_blocked_by_what_hangs_under_it() {
        use blockloom_core::components::{LightKind, LightSpec};
        let room = |blocked: bool| {
            let mut room = room_with(LightSpec {
                kind: LightKind::Rect,
                width: 2.0,
                height: 2.0,
                intensity: 1_000_000.0,
                shadows: true,
                ..LightSpec::default()
            });
            if blocked {
                // Just under the light, off the camera's line to the floor.
                let mut shade = blockloom_core::project::Actor::new(
                    "Shade",
                    blockloom_core::scene::Visual::Cuboid {
                        color: "#FFFFFF".to_string(),
                        size: [0.6, 0.2, 0.6],
                    },
                );
                shade.components.placement_mut().position = [0.0, 1.6, 0.0];
                room.actors.push(shade);
            }
            floor_pixel(run_world(room, |_| {}, game_camera(), 90, |_| false))
        };
        let (lit, shaded) = (room(false), room(true));
        assert!(
            lit.iter().all(|c| *c > 120),
            "expected a lit floor, read {lit:?}"
        );
        assert!(
            shaded.iter().all(|c| *c < 30),
            "expected the shade's shadow, read {shaded:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_disk_light_lights_the_floor() {
        use blockloom_core::components::{LightKind, LightSpec};
        let room = |kind| {
            room_with(LightSpec {
                kind,
                width: 2.0,
                height: 2.0,
                intensity: 1_000_000.0,
                ..LightSpec::default()
            })
        };
        let disk = floor_pixel(run_world(
            room(LightKind::Disk),
            |_| {},
            game_camera(),
            90,
            |_| false,
        ));
        assert!(
            disk.iter().all(|c| *c > 100),
            "expected a lit floor, read {disk:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_spot_shines_through_its_cookie() {
        use blockloom_core::components::{LightKind, LightSpec};
        let dir =
            std::env::temp_dir().join(format!("blockloom-embed-cookie-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for (name, level) in [("black.png", 0u8), ("white.png", 255u8)] {
            image::GrayImage::from_pixel(8, 8, image::Luma([level]))
                .save(dir.join("assets").join(name))
                .unwrap();
        }
        let spot = |cookie: &str| {
            let project = room_with(LightSpec {
                kind: LightKind::Spot,
                intensity: 1_000_000.0,
                outer_angle: 60.0,
                cookie: format!("assets/{cookie}"),
                ..LightSpec::default()
            });
            let load = EditorMessage::Load {
                project: Box::new(project.clone()),
                dir: Some(dir.to_string_lossy().into_owned()),
            };
            floor_pixel(run_world_sending(
                project,
                |_| {},
                game_camera(),
                90,
                |_| false,
                vec![load],
            ))
        };
        let (lit, masked) = (spot("white.png"), spot("black.png"));
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            lit.iter().all(|c| *c > 120),
            "expected a lit floor, read {lit:?}"
        );
        assert!(
            masked.iter().all(|c| *c < 30),
            "expected the cookie's shade, read {masked:?}"
        );
    }

    /// A box-projected floor draws its texture, not the background.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_box_projected_floor_draws_its_texture() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::material::SurfaceMaterial;
        let dir =
            std::env::temp_dir().join(format!("blockloom-embed-boxed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([255, 40, 40]))
            .save(dir.join("assets").join("red.png"))
            .unwrap();
        let mut project = dark_room(true);
        project.actors[0]
            .components
            .insert(ActorComponent::Material {
                material: SurfaceMaterial {
                    albedo_texture: "assets/red.png".to_string(),
                    box_projection: true,
                    ..SurfaceMaterial::default()
                },
            });
        let load = EditorMessage::Load {
            project: Box::new(project.clone()),
            dir: Some(dir.to_string_lossy().into_owned()),
        };
        let pixel = floor_pixel(run_world_sending(
            project,
            |_| {},
            game_camera(),
            0,
            |pixel| pixel[0] > 120,
            vec![load],
        ));
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            pixel[0] > 120 && pixel[1] < pixel[0] / 2,
            "expected the red texture, read {pixel:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_baked_irradiance_probe_lights_the_floor_with_the_sky() {
        probe_lights_the_floor(blockloom_core::probe::ProbeKind::Irradiance);
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_baked_reflection_probe_lights_the_floor_with_the_sky() {
        probe_lights_the_floor(blockloom_core::probe::ProbeKind::Reflection);
    }

    fn probe_lights_the_floor(kind: blockloom_core::probe::ProbeKind) {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::probe::ProbeSpec;
        let room = |probe: bool| {
            let mut room = dark_room(false);
            // No sun and no ambient: only what the probe saw can light it.
            room.world.background = "#ffffff".to_string();
            if probe {
                let mut volume = blockloom_core::project::Actor::new(
                    "Probe",
                    blockloom_core::scene::Visual::Sphere {
                        color: "#FFFFFF".to_string(),
                        radius: 0.05,
                    },
                );
                volume.components.remove("Look");
                volume.components.placement_mut().position = [0.0, 1.0, 0.0];
                volume.components.insert(ActorComponent::Probe {
                    probe: ProbeSpec {
                        kind,
                        size: [30.0, 4.0, 30.0],
                        grid: [2, 1, 2],
                        ..ProbeSpec::default()
                    },
                });
                room.actors.push(volume);
            }
            room
        };
        let bake = EditorMessage::BakeProbes { actors: Vec::new() };
        // Bakes wait out warm-up, which is slow with other worlds running.
        let lit = floor_pixel(run_world_sending(
            room(true),
            |_| {},
            game_camera(),
            400,
            |pixel| pixel.iter().all(|c| *c > 90),
            vec![bake.clone()],
        ));
        let dark = floor_pixel(run_world_sending(
            room(false),
            |_| {},
            game_camera(),
            150,
            |_| false,
            vec![bake],
        ));
        assert!(
            dark.iter().all(|c| *c < 30),
            "expected a dark floor, read {dark:?}"
        );
        assert!(
            lit.iter().all(|c| *c > 90),
            "expected the sky's bounce, read {lit:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_bc6h_bake_on_disk_lights_the_floor() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::pipeline::hdr::HdrCube;
        use blockloom_core::probe::{self, ProbeKind, ProbeSpec};
        let dir = std::env::temp_dir().join(format!("blockloom-embed-bake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A black room: only the white cube written below can light it.
        let mut room = dark_room(false);
        room.world.background = "#000000".to_string();
        let mut volume = blockloom_core::project::Actor::new(
            "Probe",
            blockloom_core::scene::Visual::Sphere {
                color: "#FFFFFF".to_string(),
                radius: 0.05,
            },
        );
        volume.components.remove("Look");
        volume.components.placement_mut().position = [0.0, 1.0, 0.0];
        volume.components.insert(ActorComponent::Probe {
            probe: ProbeSpec {
                kind: ProbeKind::Reflection,
                size: [30.0, 4.0, 30.0],
                ..ProbeSpec::default()
            },
        });
        let id = volume.id.clone();
        room.actors.push(volume);
        // Radiance in nits, as a capture divides it back out of exposure.
        let cube = HdrCube {
            size: 16,
            faces: std::array::from_fn(|_| vec![[5000.0; 3]; 256]),
        };
        let stamp = probe::stamp(&room, &id).unwrap();
        probe::write_cube(&dir, &id, &cube, stamp).unwrap();
        let load = EditorMessage::Load {
            project: Box::new(room.clone()),
            dir: Some(dir.to_string_lossy().into_owned()),
        };
        let lit = floor_pixel(run_world_sending(
            room,
            |_| {},
            game_camera(),
            300,
            |pixel| pixel.iter().all(|c| *c > 90),
            vec![load],
        ));
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            lit.iter().all(|c| *c > 90),
            "expected the baked cube's light, read {lit:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_probe_bake_leaves_out_its_own_actor() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::probe::{ProbeKind, ProbeSpec};
        let mut room = dark_room(false);
        room.world.background = "#ffffff".to_string();
        // A black box drawn from inside too, around the probe: a bake that
        // saw it would be black.
        let mut probe = blockloom_core::project::Actor::new(
            "Probe",
            blockloom_core::scene::Visual::Cuboid {
                color: "#000000".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        probe.components.placement_mut().position = [0.0, 1.0, 0.0];
        probe.components.insert(ActorComponent::Material {
            material: blockloom_core::material::SurfaceMaterial {
                double_sided: true,
                ..Default::default()
            },
        });
        probe.components.insert(ActorComponent::Probe {
            probe: ProbeSpec {
                kind: ProbeKind::Reflection,
                size: [30.0, 4.0, 30.0],
                ..ProbeSpec::default()
            },
        });
        room.actors.push(probe);
        let lit = floor_pixel(run_world_sending(
            room,
            |_| {},
            game_camera(),
            400,
            |pixel| pixel.iter().all(|c| *c > 90),
            vec![EditorMessage::BakeProbes { actors: Vec::new() }],
        ));
        assert!(
            lit.iter().all(|c| *c > 90),
            "expected the sky's bounce, read {lit:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn an_exr_capture_keeps_the_frame_linear() {
        let dir = std::env::temp_dir().join(format!("blockloom-embed-exr-{}", std::process::id()));
        let path = dir.join("shot.exr");
        let capture = EditorMessage::CaptureExr {
            path: path.to_string_lossy().into_owned(),
        };
        let (_, _, errors) = run_world_sending(
            red_world(Mode::ThreeD),
            |_| {},
            game_camera(),
            120,
            |_| false,
            vec![capture],
        );
        assert!(errors.is_empty(), "{errors:?}");
        // The file is written off the main thread.
        let started = std::time::Instant::now();
        while !path.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let shot = image::open(&path)
            .unwrap_or_else(|error| panic!("no EXR at {}: {error}", path.display()))
            .into_rgba32f();
        // Above the starter's ground, the red background.
        let [r, g, b, _] = shot.get_pixel(shot.width() / 2, shot.height() / 10).0;
        assert!(
            r > 0.5 && g < 0.05 && b < 0.05,
            "expected linear red, read {r} {g} {b}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// Under the path tracer an EXR capture traces too, and waits for the
    /// sample budget: the dark room's floor comes out lit by the traced lamp.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_path_traced_exr_capture_waits_for_its_samples() {
        let dir =
            std::env::temp_dir().join(format!("blockloom-embed-pt-exr-{}", std::process::id()));
        let path = dir.join("traced.exr");
        let capture = EditorMessage::CaptureExr {
            path: path.to_string_lossy().into_owned(),
        };
        let view = SceneView {
            path_tracer: blockloom_protocol::PathTracerView {
                enabled: true,
                samples: 32,
                seconds: 0.0,
            },
            ..game_camera()
        };
        let (_, _, errors) =
            run_world_sending(dark_room(true), |_| {}, view, 200, |_| false, vec![capture]);
        assert!(errors.is_empty(), "{errors:?}");
        let started = std::time::Instant::now();
        while !path.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let shot = image::open(&path)
            .unwrap_or_else(|error| panic!("no EXR at {}: {error}", path.display()))
            .into_rgba32f();
        let [r, g, b, _] = shot.get_pixel(shot.width() / 2, shot.height() / 2).0;
        assert!(
            r > 0.05 && g > 0.05 && b > 0.05,
            "expected a lit floor, read {r} {g} {b}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_volume_over_the_camera_repaints_the_background() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::scene::Visual;
        use blockloom_core::volume::{Override, VolumeOverrides, VolumeSpec};
        let mut world = blockloom_core::project::Project::starter("Volumes", Mode::TwoD);
        world.world.background = "#0000ff".to_string();
        let mut cave = blockloom_core::project::Actor::new(
            "Cave",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0],
            },
        );
        cave.components.remove("Look");
        cave.components.insert(ActorComponent::Volume {
            volume: VolumeSpec {
                half_extents: [5000.0; 3],
                overrides: VolumeOverrides {
                    background: Override {
                        on: true,
                        value: "#ff0000".to_string(),
                    },
                    ..VolumeOverrides::default()
                },
                ..VolumeSpec::default()
            },
        });
        world.actors.push(cave);
        let (set, index, errors) = run_world(world, |_| {}, game_camera(), 0, is_red);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(is_red(pixel), "expected the volume's red, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn volumetric_clouds_preserve_terrain_and_cast_shadows() {
        let render = |enabled, shadows| {
            let mut room = dark_room(false);
            room.world.lighting.illuminance = 10000.0;
            room.world.lighting.light_direction = [0.0, 1.0, 0.0];
            room.world.clouds.enabled = enabled;
            room.world.clouds.shadows = shadows;
            room.world.clouds.coverage = 1.0;
            room.world.clouds.erosion = 0.0;
            room.world.clouds.density = 3.0;
            room.world.clouds.shadow_strength = 1.0;
            room.world.clouds.quality = blockloom_core::clouds::CloudQuality::Low;
            floor_pixel(run_world(room, |_| {}, game_camera(), 70, |_| false))
        };
        let clear = render(false, false);
        let unshadowed = render(true, false);
        let shadowed = render(true, true);
        assert!(clear[0] > 30, "unlit baseline {clear:?}");
        assert!(
            clear[0].abs_diff(unshadowed[0]) < 8,
            "terrain was lost: clear {clear:?}, clouds {unshadowed:?}"
        );
        assert!(
            shadowed[0] + 20 < unshadowed[0],
            "no cloud shadow: {shadowed:?}, clear {unshadowed:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn cloud_layers_cover_the_sky_without_volumetrics() {
        let render = |coverage: Option<(f32, bool)>| {
            let mut room = dark_room(false);
            room.world.camera.position = [0.0, 1.0, 0.0];
            room.world.camera.look_at = [0.0, 6.0, 10.0];
            room.world.background = "#002080".into();
            room.world.lighting.illuminance = 10000.0;
            if let Some((coverage, painted)) = coverage {
                room.world.cloud_layers = vec![blockloom_core::cloud_layers::CloudLayer {
                    coverage,
                    altitude: 2000.0,
                    horizon_fade: [1.0, 0.0],
                    ..Default::default()
                }];
                if painted {
                    // A missing file reports and falls back to the noise.
                    room.world.cloud_layers[0].coverage_texture = "assets/none.png".into();
                }
            }
            let (set, index, errors) = run_world(room, |_| {}, game_camera(), 40, |_| false);
            let set = set.expect("cloud layer frame");
            (
                middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize),
                errors,
            )
        };
        let (clear, errors) = render(None);
        assert!(errors.is_empty(), "{errors:?}");
        let (overcast, errors) = render(Some((1.0, false)));
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            overcast[2].saturating_add(40) < clear[2] || overcast[0] > clear[0].saturating_add(40),
            "the layer didn't cover the sky: clear {clear:?}, overcast {overcast:?}"
        );
        let (none, _) = render(Some((0.0, false)));
        assert_eq!(none, clear, "a clear layer changed the sky");
        let (_, errors) = render(Some((1.0, true)));
        assert!(!errors.is_empty(), "a missing coverage file said nothing");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn volumetric_clouds_cover_the_sky() {
        let render = |enabled| {
            let mut room = dark_room(false);
            room.world.camera.position = [0.0, 1.0, 0.0];
            room.world.camera.look_at = [0.0, 6.0, 10.0];
            room.world.background = "#002080".into();
            room.world.lighting.illuminance = 10000.0;
            room.world.clouds.enabled = enabled;
            room.world.clouds.coverage = 1.0;
            room.world.clouds.erosion = 0.0;
            room.world.clouds.density = 2.0;
            room.world.clouds.quality = blockloom_core::clouds::CloudQuality::Low;
            let (set, index, errors) = run_world(room, |_| {}, game_camera(), 70, |_| false);
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.expect("cloud frame");
            middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize)
        };
        let clear = render(false);
        let cloudy = render(true);
        // Dense undersides get little ambient, so they may be dark: only
        // check that the clouds hide the blue background.
        assert!(
            cloudy[2].saturating_add(40) < clear[2],
            "clouds didn't occlude the sky: clear {clear:?}, cloudy {cloudy:?}"
        );
    }

    /// How much a band of sky changes across `rows`: between neighbouring
    /// pixels, and from one frame to the next.
    fn grain(frames: &[Vec<[u8; 3]>], size: UVec2, rows: std::ops::Range<usize>) -> (f32, f32) {
        let luma = |frame: &[[u8; 3]], x: usize, y: usize| {
            let [r, g, b] = frame[y * size.x as usize + x];
            0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
        };
        let (mut spatial, mut temporal, mut count) = (0.0, 0.0, 0.0);
        for pair in frames.windows(2) {
            for y in rows.clone() {
                for x in 0..size.x as usize - 1 {
                    spatial += (luma(&pair[1], x + 1, y) - luma(&pair[1], x, y)).abs();
                    temporal += (luma(&pair[1], x, y) - luma(&pair[0], x, y)).abs();
                    count += 1.0;
                }
            }
        }
        (spatial / count, temporal / count)
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn distant_clouds_neither_crawl_nor_alias() {
        let size = UVec2::new(320, 180);
        let mut room = dark_room(false);
        room.world.camera.position = [0.0, 1.0, 0.0];
        room.world.camera.look_at = [0.0, 60.0, 1000.0];
        room.world.background = "#002080".into();
        room.world.lighting.illuminance = 10000.0;
        room.world.clouds.enabled = true;
        room.world.clouds.coverage = 0.6;
        room.world.clouds.density = 1.5;
        room.world.clouds.quality = blockloom_core::clouds::CloudQuality::Low;
        let frames = run_world_frames(room, size, game_camera(), 120, 8);
        for (index, frame) in frames.iter().enumerate() {
            let bytes: Vec<u8> = frame.iter().flatten().copied().collect();
            if let (Some(dir), Some(image)) = (
                std::env::var_os("BLOCKLOOM_TEST_DUMP"),
                image::RgbImage::from_raw(size.x, size.y, bytes),
            ) {
                let _ =
                    image.save(std::path::Path::new(&dir).join(format!("far-clouds-{index}.png")));
            }
        }
        // Rows from just above the horizon to a quarter of the way up.
        let (spatial, temporal) = grain(&frames, size, 30..110);
        // The old march read 2.3 on both here (lavapipe); filtered, about 1.3.
        assert!(
            spatial < 1.8 && temporal < 1.8,
            "far clouds speckle: spatial {spatial:.3}, temporal {temporal:.3}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn volumetric_clouds_read_an_authored_shape_volume() {
        // An empty shape volume leaves a full-coverage sky clear.
        let dir =
            std::env::temp_dir().join(format!("blockloom-cloud-noise-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 255]))
            .save(dir.join("assets/empty.png"))
            .unwrap();
        let render = |enabled, authored: bool| {
            let mut room = dark_room(false);
            room.world.camera.position = [0.0, 1.0, 0.0];
            room.world.camera.look_at = [0.0, 6.0, 10.0];
            room.world.background = "#002080".into();
            room.world.lighting.illuminance = 10000.0;
            room.world.clouds.enabled = enabled;
            room.world.clouds.coverage = 1.0;
            room.world.clouds.erosion = 0.0;
            room.world.clouds.density = 2.0;
            room.world.clouds.quality = blockloom_core::clouds::CloudQuality::Low;
            if authored {
                room.world.clouds.shape_volume = "assets/empty.png".into();
            }
            let reload = vec![EditorMessage::Load {
                project: Box::new(room.clone()),
                dir: Some(dir.to_string_lossy().into_owned()),
            }];
            let (set, index, errors) =
                run_world_sending(room, |_| {}, game_camera(), 70, |_| false, reload);
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.expect("cloud frame");
            middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize)
        };
        let clear = render(false, false);
        let empty = render(true, true);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            clear[2].abs_diff(empty[2]) < 12,
            "the empty volume still drew clouds: clear {clear:?}, authored {empty:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_physical_sky_paints_a_blue_day_behind_the_world() {
        use blockloom_core::sky::{SkyKind, SunMode};
        let mut room = dark_room(false);
        room.world.lighting.illuminance = 10_000.0;
        room.world.sky.kind = SkyKind::Physical;
        room.world.sky.sun.mode = SunMode::Manual;
        room.world.sky.sun.elevation = 50.0;
        // Up above the floor, looking at the sky away from the sun.
        room.world.camera.position = [0.0, 1.0, 0.0];
        room.world.camera.look_at = [0.0, 6.0, 10.0];
        let blue = |[r, _, b]: [u8; 3]| b > r.saturating_add(20);
        let (set, index, errors) = run_world(room, |_| {}, game_camera(), 5, blue);
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.unwrap_or_else(|| panic!("no frame arrived"));
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(blue(pixel), "expected a blue sky, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_gradient_sky_lights_the_floor_unless_told_not_to() {
        use blockloom_core::sky::SkyKind;
        let room = |lighting: bool| {
            // No sun and no ambient: only the sky can light the floor.
            let mut room = dark_room(false);
            let sky = &mut room.world.sky;
            sky.kind = SkyKind::Gradient;
            sky.lighting = lighting;
            sky.reflections = false;
            for stop in [
                &mut sky.gradient.top,
                &mut sky.gradient.middle,
                &mut sky.gradient.bottom,
            ] {
                *stop = "#FFFFFF".to_string();
            }
            room
        };
        let lit = floor_pixel(run_world(
            room(true),
            |_| {},
            game_camera(),
            0,
            |pixel| pixel.iter().all(|c| *c > 60),
        ));
        assert!(
            lit.iter().all(|c| *c > 60),
            "expected the sky's light, read {lit:?}"
        );
        let dark = floor_pixel(run_world(room(false), |_| {}, game_camera(), 30, |_| false));
        assert!(
            dark.iter().all(|c| *c < 30),
            "expected a dark floor, read {dark:?}"
        );
    }

    /// The dark room with the floor lit by a white ambient, looked at
    /// through air.
    fn foggy_room(
        fog: impl FnOnce(&mut blockloom_core::fog::Fog),
    ) -> blockloom_core::project::Project {
        let mut room = dark_room(false);
        room.world.lighting.ambient_brightness = 1500.0;
        fog(&mut room.world.fog);
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn height_fog_hides_the_floor_in_its_own_color() {
        let room = foggy_room(|fog| {
            fog.height.enabled = true;
            fog.height.distance = 1.0;
            fog.height.falloff = 0.0;
            fog.height.day_color = "#FF0000".to_string();
            fog.height.dusk_color = "#FF0000".to_string();
            fog.height.night_color = "#FF0000".to_string();
        });
        let pixel = floor_pixel(run_world(room, |_| {}, game_camera(), 60, is_red));
        assert!(is_red(pixel), "expected red fog, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_lake_tints_the_floor_under_it() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::water::{ReflectionMode, WaterSpec};
        let blue = |[r, _, b]: [u8; 3]| b > 60 && b > r.saturating_add(40);
        let mut room = dark_room(true);
        let mut spec = WaterSpec {
            depth: 3.0,
            ..WaterSpec::default()
        };
        spec.look.shallow = "#0040FF".to_string();
        spec.look.deep = "#0020C0".to_string();
        spec.look.absorption = 0.5;
        spec.foam.amount = 0.0;
        spec.underwater.caustics = 0.0;
        spec.reflections.mode = ReflectionMode::Sky;
        spec.waves.amplitude = 0.02;
        let mut lake = blockloom_core::project::Actor::new(
            "Lake",
            blockloom_core::scene::Visual::Sphere {
                color: "#FFFFFF".to_string(),
                radius: 0.05,
            },
        );
        lake.components.remove("Look");
        lake.components.placement_mut().position = [0.0, 1.0, 0.0];
        lake.components
            .insert(ActorComponent::Water { water: spec });
        room.actors.push(lake);
        let (set, index, errors) = run_world(room, |_| {}, game_camera(), 600, blue);
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.expect("no frame arrived");
        dump(
            "lake",
            &frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize),
        );
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(blue(pixel), "expected blue water, read {pixel:?}");
    }

    /// A still, clear lake at y 0.5 with a glowing red wall behind it, the
    /// wall's reflection landing mid-frame.
    fn mirror_lake(
        mode: blockloom_core::water::ReflectionMode,
    ) -> blockloom_core::project::Project {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::material::SurfaceMaterial;
        use blockloom_core::scene::Visual;
        use blockloom_core::water::WaterSpec;
        let mut room = blockloom_core::project::Project::starter("Mirror", Mode::ThreeD);
        room.world.background = "#000000".to_string();
        room.world.lighting.illuminance = 0.0;
        room.world.lighting.ambient_brightness = 0.0;
        room.actors.retain(|actor| actor.name == "Ground");
        let mut spec = WaterSpec {
            depth: 3.0,
            ..WaterSpec::default()
        };
        spec.waves.amplitude = 0.0;
        spec.detail.strength = 0.0;
        spec.foam.amount = 0.0;
        spec.underwater.caustics = 0.0;
        spec.look.shallow = "#000000".to_string();
        spec.look.deep = "#000000".to_string();
        spec.look.clarity = 0.0;
        spec.look.roughness = 0.02;
        spec.reflections.mode = mode;
        spec.reflections.planar_scale = 1.0;
        let mut lake = blockloom_core::project::Actor::new(
            "Lake",
            Visual::Sphere {
                color: "#FFFFFF".to_string(),
                radius: 0.05,
            },
        );
        lake.components.remove("Look");
        lake.components.placement_mut().position = [0.0, 0.5, 0.0];
        lake.components
            .insert(ActorComponent::Water { water: spec });
        room.actors.push(lake);
        let mut wall = blockloom_core::project::Actor::new(
            "Wall",
            Visual::Cuboid {
                color: "#000000".to_string(),
                size: [40.0, 12.0, 1.0],
            },
        );
        wall.components.placement_mut().position = [0.0, 6.0, -10.0];
        wall.components.insert(ActorComponent::Material {
            material: SurfaceMaterial {
                emissive: "#FF0000".to_string(),
                emissive_energy: 200.0,
                ..SurfaceMaterial::default()
            },
        });
        room.actors.push(wall);
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_planar_mirror_reflects_what_stands_over_the_water() {
        use blockloom_core::water::ReflectionMode;
        let red = |[r, _, _]: [u8; 3]| r > 60;
        let (set, index, errors) = run_world(
            mirror_lake(ReflectionMode::Planar),
            |_| {},
            game_camera(),
            600,
            red,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.expect("no frame arrived");
        dump(
            "mirror",
            &frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize),
        );
        let mirrored = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        let plain = floor_pixel(run_world(
            mirror_lake(ReflectionMode::Sky),
            |_| {},
            game_camera(),
            400,
            |_| false,
        ));
        assert!(
            red(mirrored) && mirrored[0] > plain[0].saturating_add(40),
            "expected the red wall in the water, read {mirrored:?} against {plain:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn volumetric_fog_glows_with_its_emissive() {
        let green =
            |[r, g, b]: [u8; 3]| g > 150 && g > r.saturating_add(60) && g > b.saturating_add(60);
        let room = foggy_room(|fog| {
            let v = &mut fog.volumetric;
            v.enabled = true;
            v.density = 1.0;
            v.falloff = 0.0;
            v.noise = 0.0;
            v.sun = false;
            v.ambient = 0.0;
            v.albedo = "#000000".to_string();
            v.emissive = "#00FF00".to_string();
            v.emissive_strength = 3000.0;
            v.range = 40.0;
        });
        let pixel = floor_pixel(run_world(room, |_| {}, game_camera(), 60, green));
        assert!(green(pixel), "expected green fog, read {pixel:?}");
    }

    /// A spot over the floor with a beam, drawn one way or the other.
    fn beam_room(
        density: f32,
        mode: blockloom_core::fog::BeamMode,
    ) -> blockloom_core::project::Project {
        use blockloom_core::components::{LightKind, LightSpec};
        let mut light = LightSpec {
            kind: LightKind::Spot,
            color: "#FF0000".to_string(),
            intensity: 100_000.0,
            range: 6.0,
            ..LightSpec::default()
        };
        light.beam.density = density;
        light.beam.mode = mode;
        light.beam.near_fade = 0.0;
        light.beam.far_fade = 0.0;
        light.beam.falloff = 0.0;
        let mut room = room_with(light);
        // A black floor, so only the beam can add light.
        room.actors[0]
            .components
            .set_visual(blockloom_core::scene::Visual::Plane {
                color: "#000000".to_string(),
                size: [20.0, 20.0],
            });
        room.world.fog.volumetric.enabled = true;
        room.world.fog.volumetric.density = 0.0;
        room.world.fog.volumetric.sun = false;
        room
    }

    fn red(pixel: [u8; 3]) -> u32 {
        pixel[0] as u32
    }

    fn brightness(pixel: [u8; 3]) -> u32 {
        pixel.iter().map(|c| *c as u32).sum()
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_froxel_beam_glows_between_the_lamp_and_the_floor() {
        use blockloom_core::fog::BeamMode;
        let bare = floor_pixel(run_world(
            beam_room(0.0, BeamMode::Volumetric),
            |_| {},
            game_camera(),
            60,
            |_| false,
        ));
        let beam = floor_pixel(run_world(
            beam_room(2.0, BeamMode::Volumetric),
            |_| {},
            game_camera(),
            60,
            |_| false,
        ));
        assert!(
            red(beam) > red(bare) + 10,
            "expected the beam to add light, read {bare:?} then {beam:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_shaft_cone_stands_in_for_the_froxels() {
        use blockloom_core::fog::BeamMode;
        let bare = floor_pixel(run_world(
            beam_room(0.0, BeamMode::Shaft),
            |_| {},
            game_camera(),
            60,
            |_| false,
        ));
        let shaft = floor_pixel(run_world(
            beam_room(2.0, BeamMode::Shaft),
            |_| {},
            game_camera(),
            60,
            |_| false,
        ));
        assert!(
            red(shaft) > red(bare) + 10,
            "expected the shaft to add light, read {bare:?} then {shaft:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn motes_glitter_in_a_beam_and_dust_in_the_air() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::fog::BeamMode;
        let dusty = |motes: bool| {
            let mut room = beam_room(0.0, BeamMode::Shaft);
            if let Some(ActorComponent::Light { light }) =
                room.actors[1].components.get_mut("Light")
            {
                let m = &mut light.beam.motes;
                m.enabled = motes;
                m.count = 4096;
                m.size = 0.3;
                m.alpha = 1.0;
            }
            room
        };
        let bare = floor_pixel(run_world(
            dusty(false),
            |_| {},
            game_camera(),
            60,
            |_| false,
        ));
        let motes = floor_pixel(run_world(dusty(true), |_| {}, game_camera(), 60, |_| false));
        assert!(
            red(motes) > red(bare) + 10,
            "expected motes in the beam, read {bare:?} then {motes:?}"
        );
        // Height dust in an ambient-lit room.
        let air = |dust: bool| {
            let mut room = foggy_room(|fog| {
                let d = &mut fog.volumetric.dust;
                d.enabled = dust;
                d.count = 4096;
                d.size = 0.3;
                d.alpha = 1.0;
                fog.volumetric.dust_height = 50.0;
            });
            room.actors[0]
                .components
                .set_visual(blockloom_core::scene::Visual::Plane {
                    color: "#000000".to_string(),
                    size: [20.0, 20.0],
                });
            room
        };
        let clear = floor_pixel(run_world(air(false), |_| {}, game_camera(), 60, |_| false));
        let dust = floor_pixel(run_world(air(true), |_| {}, game_camera(), 60, |_| false));
        assert!(
            brightness(dust) > brightness(clear) + 10,
            "expected dust, read {clear:?} then {dust:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_volume_fills_its_box_with_local_fog() {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::volume::VolumeSpec;
        let blue =
            |[r, g, b]: [u8; 3]| b > 150 && b > r.saturating_add(60) && b > g.saturating_add(60);
        let room = |inside: bool| {
            let mut room = foggy_room(|_| {});
            let mut zone = blockloom_core::project::Actor::new(
                "Smog",
                blockloom_core::scene::Visual::Sphere {
                    color: "#FFFFFF".to_string(),
                    radius: 1.0,
                },
            );
            zone.components.remove("Look");
            zone.components.placement_mut().position =
                if inside { [0.0; 3] } else { [0.0, 80.0, 0.0] };
            let mut volume = VolumeSpec {
                half_extents: [30.0, 30.0, 30.0],
                blend_distance: 0.0,
                ..VolumeSpec::default()
            };
            volume.fog.enabled = true;
            volume.fog.density = 2.0;
            volume.fog.albedo = "#000000".to_string();
            volume.fog.emissive = "#0000FF".to_string();
            volume.fog.emissive_strength = 3000.0;
            zone.components.insert(ActorComponent::Volume { volume });
            room.actors.push(zone);
            room
        };
        let inside = floor_pixel(run_world(room(true), |_| {}, game_camera(), 60, blue));
        assert!(blue(inside), "expected the box's blue fog, read {inside:?}");
        let outside = floor_pixel(run_world(room(false), |_| {}, game_camera(), 60, |_| false));
        assert!(!blue(outside), "expected a clear floor, read {outside:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn volumetric_fog_scatters_the_suns_light_when_asked() {
        let run = |sun: bool| {
            let mut room = dark_room(false);
            room.world.lighting.illuminance = 20_000.0;
            let v = &mut room.world.fog.volumetric;
            v.enabled = true;
            v.density = 0.8;
            v.falloff = 0.0;
            v.noise = 0.0;
            v.ambient = 0.0;
            v.anisotropy = 0.0;
            v.sun = sun;
            v.range = 40.0;
            let settle = if sun { 0 } else { 60 };
            floor_pixel(run_world(
                room,
                |_| {},
                game_camera(),
                settle,
                move |pixel| sun && pixel.iter().all(|c| *c > 90),
            ))
        };
        let lit = run(true);
        assert!(
            lit.iter().all(|c| *c > 90),
            "expected sunlit fog, read {lit:?}"
        );
        let dark = run(false);
        assert!(
            dark.iter().all(|c| *c < 40),
            "expected unlit fog to hide the floor, read {dark:?}"
        );
    }

    /// Light shafts come from the sun's own shadow maps: fog under a roof
    /// stays dark while the same fog in the open glows.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_roof_shadows_the_fog_under_it() {
        use blockloom_core::scene::Visual;
        let run = |roofed: bool| {
            let mut room = dark_room(false);
            room.world.lighting.illuminance = 20_000.0;
            room.world.lighting.light_direction = [0.1, 1.0, 0.05];
            let v = &mut room.world.fog.volumetric;
            v.enabled = true;
            v.density = 0.3;
            v.falloff = 0.0;
            v.noise = 0.0;
            v.ambient = 0.0;
            v.anisotropy = 0.0;
            v.range = 40.0;
            if roofed {
                let mut roof = blockloom_core::project::Actor::new(
                    "Roof",
                    Visual::Cuboid {
                        color: "#000000".to_string(),
                        size: [160.0, 1.0, 160.0],
                    },
                );
                roof.components.placement_mut().position = [0.0, 20.0, 0.0];
                room.actors.push(roof);
            }
            let settle = if roofed { 90 } else { 0 };
            floor_pixel(run_world(
                room,
                |_| {},
                game_camera(),
                settle,
                move |pixel| !roofed && pixel.iter().all(|c| *c > 90),
            ))
        };
        let open = run(false);
        assert!(
            open.iter().all(|c| *c > 90),
            "expected sunlit fog, read {open:?}"
        );
        let roofed = run(true);
        assert!(
            roofed.iter().all(|c| *c < 40),
            "expected the roof's shadow in the fog, read {roofed:?}"
        );
    }

    /// A physical night sky, looked at straight up.
    fn night_sky(
        space: impl FnOnce(&mut blockloom_core::sky::Sky),
    ) -> blockloom_core::project::Project {
        use blockloom_core::sky::{SkyKind, SunMode};
        let mut room = dark_room(false);
        room.world.post.exposure_ev = 0.0;
        let sky = &mut room.world.sky;
        sky.kind = SkyKind::Physical;
        sky.sun.mode = SunMode::Manual;
        sky.sun.elevation = -40.0;
        sky.physical.night_brightness = 0.0;
        space(sky);
        room.world.camera.position = [0.0, 1.0, 0.0];
        room.world.camera.look_at = [0.0, 20.0, 0.5];
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn an_aurora_hangs_green_over_the_night() {
        let green = |[r, g, b]: [u8; 3]| g > 40 && g > r.saturating_add(20) && g > b;
        let room = night_sky(|sky| {
            sky.aurora.enabled = true;
            sky.aurora.kp = 9.0;
            sky.aurora.layers = 3;
            sky.aurora.width = 5.0;
            sky.aurora.brightness = 5.0;
        });
        let (set, index, errors) = run_world(room, |_| {}, game_camera(), 90, |_| false);
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.unwrap_or_else(|| panic!("no frame arrived"));
        let pixel = brightest_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(
            green(pixel),
            "expected aurora green somewhere, brightest {pixel:?}"
        );
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn stars_come_out_at_night() {
        let bright = |[r, g, b]: [u8; 3]| r > 120 && g > 120 && b > 120;
        let run = |elevation: f32| {
            let room = night_sky(|sky| {
                sky.sun.elevation = elevation;
                sky.stars.enabled = true;
                sky.stars.density = 1.0;
                sky.stars.brightness = 5.0;
                sky.stars.magnitude_slope = 0.5;
                sky.stars.twinkle = 0.0;
            });
            let (set, index, errors) = run_world(room, |_| {}, game_camera(), 60, |_| false);
            assert!(errors.is_empty(), "{errors:?}");
            let set = set.unwrap_or_else(|| panic!("no frame arrived"));
            brightest_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize)
        };
        let night = run(-40.0);
        assert!(bright(night), "expected a star, brightest {night:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn a_lightning_block_flashes_the_dark_floor() {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::value::Value;
        let mut room = dark_room(false);
        room.world.lightning.flash_height = 5.0;
        room.world.lightning.decay = 2.0;
        room.world.lightning.thunder = false;
        let at = Value::number;
        room.actors[0].graph.strands.push(Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenStarted),
                Instruction::new(K::StrikeLightning {
                    x: at(0.0),
                    y: at(0.0),
                    z: at(0.0),
                }),
            ],
        ));
        let pixel = floor_pixel(run_world_sending(
            room,
            |_| {},
            game_camera(),
            0,
            |pixel| pixel.iter().all(|c| *c > 100),
            vec![EditorMessage::Start],
        ));
        assert!(
            pixel.iter().all(|c| *c > 100),
            "expected a flash, read {pixel:?}"
        );
    }

    /// The brightest pixel of a linear dma-buf, by the sum of its channels.
    fn brightest_pixel(image: &SharedImage, width: usize, height: usize) -> [u8; 3] {
        use std::os::fd::AsRawFd;
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
            let mut best = [0u8; 3];
            for y in 0..height {
                let row = image.offset as usize + image.stride as usize * y;
                for x in 0..width {
                    let at = row + x * 4;
                    let pixel = [bytes[at], bytes[at + 1], bytes[at + 2]];
                    let sum = |p: [u8; 3]| p.iter().map(|c| *c as u32).sum::<u32>();
                    if sum(pixel) > sum(best) {
                        best = pixel;
                    }
                }
            }
            libc::munmap(mapped, length);
            best
        }
    }

    /// A plain volume (no overrides) at `at`, and the heat map switched on.
    fn heat_world(mode: Mode, at: [f32; 3]) -> (blockloom_core::project::Project, SceneView) {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::scene::Visual;
        use blockloom_core::volume::VolumeSpec;
        let mut world = blockloom_core::project::Project::starter("Heat", mode);
        world.world.background = "#000000".to_string();
        if mode == Mode::ThreeD {
            world.actors.retain(|actor| actor.name == "Ground");
            world.actors[0].components.set_visual(Visual::Plane {
                color: "#FFFFFF".to_string(),
                size: [40.0, 40.0],
            });
        } else {
            world.actors.clear();
        }
        let mut volume = blockloom_core::project::Actor::new(
            "Zone",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0],
            },
        );
        volume.components.remove("Look");
        volume.components.placement_mut().position = at;
        volume.components.insert(ActorComponent::Volume {
            volume: VolumeSpec {
                half_extents: [30.0, 3.0, 30.0],
                blend_distance: 0.0,
                ..VolumeSpec::default()
            },
        });
        world.actors.push(volume);
        let mut view = game_camera();
        view.volumes.heatmap = true;
        (world, view)
    }

    /// The heat map's full-cover red, over whatever the scene is.
    fn is_heat([r, g, b]: [u8; 3]) -> bool {
        r > 150 && r > g.saturating_add(60) && r > b.saturating_add(60)
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn the_heat_map_tints_what_a_volume_covers_in_2d() {
        let (world, view) = heat_world(Mode::TwoD, [0.0; 3]);
        let (set, index, errors) = run_world(world, |_| {}, view, 60, is_heat);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(is_heat(pixel), "expected heat, read {pixel:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn the_heat_map_follows_the_surface_in_3d() {
        // Around the floor: the floor under it lights up.
        let (world, view) = heat_world(Mode::ThreeD, [0.0; 3]);
        let (set, index, errors) = run_world(world, |_| {}, view, 60, is_heat);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert!(errors.is_empty(), "{errors:?}");
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(is_heat(pixel), "expected heat on the floor, read {pixel:?}");

        // High above it, the same volume covers none of the floor.
        let (world, view) = heat_world(Mode::ThreeD, [0.0, 50.0, 0.0]);
        let (set, index, errors) = run_world(world, |_| {}, view, 60, |_| false);
        let set = set.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        let pixel = middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        assert!(!is_heat(pixel), "expected a plain floor, read {pixel:?}");
    }

    fn red_world(mode: Mode) -> blockloom_core::project::Project {
        let mut red = blockloom_core::project::Project::starter("Embedded", mode);
        red.world.background = "#ff0000".to_string();
        red
    }

    /// The dark room with a red emitter at the floor's middle, whose
    /// particles hang where they are born.
    fn sparks(sim: blockloom_core::vfx::SimMode) -> blockloom_core::project::Project {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::material::ParticleSpec;
        use blockloom_core::vfx::{Curve, ParticleBlend};
        let mut room = dark_room(false);
        let mut spec = ParticleSpec {
            rate: 200.0,
            lifetime: 30.0,
            speed: 0.0,
            gravity_scale: 0.0,
            size_start: 3.0,
            size_end: 3.0,
            color_start: "#FF0000".to_string(),
            color_end: "#FF0000".to_string(),
            // One particle at a time, so the red doesn't pile up into white.
            max: 1,
            wind: 0.0,
            sim,
            ..ParticleSpec::default()
        };
        spec.render.blend = ParticleBlend::Additive;
        spec.render.alpha = Curve::constant(1.0);
        let mut emitter = blockloom_core::project::Actor::new(
            "Sparks",
            blockloom_core::scene::Visual::Sphere {
                color: "#000000".to_string(),
                radius: 0.01,
            },
        );
        emitter.components.remove("Look");
        emitter.components.placement_mut().position = [0.0, 1.0, 0.0];
        emitter
            .components
            .insert(ActorComponent::Emitter { emitter: spec });
        room.actors.push(emitter);
        room
    }

    /// Red pixels in a frame of the sparks room, which is black but for
    /// the particles, and the mean row they sit on.
    fn red_sparks(room: blockloom_core::project::Project, name: &str) -> (usize, f32) {
        let (set, index, errors) = run_world_sending(
            room,
            |_| {},
            game_camera(),
            120,
            |_| false,
            vec![EditorMessage::Start],
        );
        assert!(errors.is_empty(), "{errors:?}");
        let set = set.expect("no frame arrived");
        let frame = frame_pixels(&set.images[index], SIZE.x as usize, SIZE.y as usize);
        dump(name, &frame);
        let rows: Vec<usize> = frame
            .into_iter()
            .enumerate()
            .filter(|&(_, pixel)| is_red(pixel))
            .map(|(i, _)| i / SIZE.x as usize)
            .collect();
        let mean = rows.iter().sum::<usize>() as f32 / rows.len().max(1) as f32;
        (rows.len(), mean)
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_particles_draw_where_they_are_born() {
        let (red, _) = red_sparks(sparks(blockloom_core::vfx::SimMode::Auto), "gpu_particles");
        assert!(red > 1000, "expected red sparks, found {red} red pixels");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn cpu_particles_draw_where_they_are_born() {
        let (red, _) = red_sparks(sparks(blockloom_core::vfx::SimMode::Cpu), "cpu_particles");
        assert!(red > 1000, "expected red sparks, found {red} red pixels");
    }

    /// Sparks born a metre over where they are drawn in `sparks`, falling,
    /// over no floor at all: only a hidden box can stop them.
    fn falling_sparks(box_under: bool) -> blockloom_core::project::Project {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::scene::{BodyKind, Physics};
        use blockloom_core::vfx::UpdateModule;
        let mut room = sparks(blockloom_core::vfx::SimMode::Auto);
        room.actors.retain(|actor| actor.name == "Sparks");
        let emitter = &mut room.actors[0];
        emitter.components.placement_mut().position = [0.0, 2.0, 0.0];
        if let Some(ActorComponent::Emitter { emitter: spec }) =
            emitter.components.get_mut("Emitter")
        {
            spec.gravity_scale = 1.0;
            spec.modules = vec![UpdateModule::Collide {
                bounce: 0.0,
                friction: 1.0,
                lifetime_loss: 0.0,
                kill: false,
            }];
        }
        if box_under {
            let mut shelf = blockloom_core::project::Actor::new(
                "Shelf",
                blockloom_core::scene::Visual::Cuboid {
                    color: "#000000".to_string(),
                    size: [4.0, 1.0, 4.0],
                },
            );
            shelf.components.placement_mut().position = [0.0, 0.5, 0.0];
            // Hidden, so the depth buffer never sees it.
            shelf.components.set_visible(false);
            shelf.components.set_physics(Physics {
                body: BodyKind::Static,
                ..Physics::default()
            });
            room.actors.push(shelf);
        }
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_particles_land_on_an_actor_the_camera_cant_see() {
        // Where `sparks` draws one hanging at the shelf's top.
        let (_, shelf) = red_sparks(sparks(blockloom_core::vfx::SimMode::Auto), "gpu_particles");
        let (red, resting) = red_sparks(falling_sparks(true), "gpu_particles_landed");
        assert!(
            red > 1000,
            "expected a spark on the shelf, found {red} red pixels"
        );
        assert!(
            (resting - shelf).abs() < 8.0,
            "expected the spark on the shelf at row {shelf}, found it at {resting}"
        );
        // With no shelf it falls straight past where the shelf would be.
        let (_, fallen) = red_sparks(falling_sparks(false), "gpu_particles_fell");
        assert!(
            fallen > shelf + 40.0,
            "expected the spark below row {shelf}, found it at {fallen}"
        );
    }

    /// Ribbons behind sparks that fly off sideways, with no heads drawn.
    fn ribbon_sparks() -> blockloom_core::project::Project {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::vfx::ParticleBlend;
        let mut room = sparks(blockloom_core::vfx::SimMode::Auto);
        let emitter = room.actors.last_mut().unwrap();
        if let Some(ActorComponent::Emitter { emitter: spec }) =
            emitter.components.get_mut("Emitter")
        {
            // Fast, since a software GPU only gets through a fraction of a
            // second of the run before the frame is read.
            spec.max = 16;
            spec.rate = 20.0;
            spec.lifetime = 5.0;
            spec.speed = 6.0;
            spec.size_start = 0.2;
            spec.size_end = 0.2;
            spec.render.blend = ParticleBlend::Alpha;
            spec.ribbon.enabled = true;
            spec.ribbon.heads = false;
            spec.ribbon.width = 0.3;
            spec.ribbon.width_curve = blockloom_core::vfx::Curve::constant(1.0);
            spec.ribbon.step = 0.02;
        }
        room
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_ribbons_draw_without_heads() {
        let (red, _) = red_sparks(ribbon_sparks(), "gpu_ribbons");
        assert!(red > 500, "expected red ribbons, found {red} red pixels");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn the_overdraw_meter_counts_the_sparks() {
        let (_, _, reports) = run_world_reporting(
            sparks(blockloom_core::vfx::SimMode::Auto),
            |_| {},
            game_camera(),
            120,
            |_| false,
            vec![EditorMessage::Start],
        );
        let overdraw = reports.iter().rev().find_map(|report| match report {
            RuntimeMessage::Status(status) => status
                .render_metrics
                .iter()
                .find(|metric| metric.name == "vfx/overdraw")
                .map(|metric| metric.value),
            _ => None,
        });
        // One spark covers a sliver of the view.
        let overdraw = overdraw.expect("no overdraw reported");
        assert!(
            overdraw > 0.001 && overdraw < 1.0,
            "expected a sliver of a screen, read {overdraw}"
        );
    }

    #[test]
    #[ignore = "needs a GPU with texture binding arrays"]
    fn decals_project_reject_and_fade_on_instanced_surfaces() {
        let mut diagnostics = App::new();
        diagnostics.add_plugins(bevy::log::LogPlugin::default());
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::decals::DecalPreset;
        use blockloom_core::value::Value;
        let run_mark = |normal_y: f64, height: f64, remove: bool, traced: bool| {
            let mut room = dark_room(true);
            room.world.lighting.ray_tracing.enabled = traced;
            let mut blocks = vec![
                Instruction::new(K::WhenStarted),
                Instruction::new(K::SpawnDecal {
                    preset: DecalPreset::Blood,
                    x: Value::number(0.0),
                    y: Value::number(height),
                    z: Value::number(0.0),
                    nx: Value::number(0.0),
                    ny: Value::number(normal_y),
                    nz: Value::number(0.0),
                    size: Value::number(12.0),
                    lifetime: Value::number(60.0),
                    fade: Value::number(2.0),
                }),
            ];
            if remove {
                blocks.push(Instruction::new(K::FadeDecals {
                    x: Value::number(0.0),
                    y: Value::number(0.0),
                    z: Value::number(0.0),
                    radius: Value::number(20.0),
                    seconds: Value::number(0.0),
                }));
            }
            room.actors[0]
                .graph
                .strands
                .push(Strand::with_instructions(0, 0, blocks));
            let (set, index, reports) = run_world_reporting(
                room,
                |_| {},
                game_camera(),
                90,
                |_| false,
                vec![EditorMessage::Start],
            );
            let active = reports.iter().rev().find_map(|report| match report {
                RuntimeMessage::Status(status) => status
                    .render_metrics
                    .iter()
                    .find(|metric| metric.name == "decals/active")
                    .map(|metric| metric.value),
                _ => None,
            });
            assert_eq!(
                active,
                Some(if remove { 0.0 } else { 1.0 }),
                "decal block did not run"
            );
            let tracing_available = reports
                .iter()
                .rev()
                .find_map(|report| match report {
                    RuntimeMessage::RayTracing(status) => Some(status.available && status.active),
                    _ => None,
                })
                .unwrap_or(false);
            let errors = reports
                .into_iter()
                .filter(|report| {
                    matches!(
                        report,
                        RuntimeMessage::Error { .. } | RuntimeMessage::Fatal { .. }
                    )
                })
                .collect();
            let result = (set, index, errors);
            if let (Some(set), index, _) = &result {
                dump(
                    &format!("decal-{normal_y}-{height}-{remove}-{traced}"),
                    &frame_pixels(&set.images[*index], SIZE.x as usize, SIZE.y as usize),
                );
            }
            (floor_pixel(result), tracing_available)
        };
        let marked = run_mark(1.0, 0.0, false, false).0;
        let backwards = run_mark(-1.0, 0.0, false, false).0;
        let detached = run_mark(1.0, 0.5, false, false).0;
        let removed = run_mark(1.0, 0.0, true, false).0;
        assert!(
            marked[0] > marked[1] + 10,
            "blood should tint red: {marked:?}"
        );
        for bare in [backwards, detached, removed] {
            assert!(
                bare[1] > marked[1] + 25,
                "expected an unmarked floor: {bare:?}, marked {marked:?}"
            );
        }
        #[cfg(feature = "ray_tracing")]
        {
            let (marked, available) = run_mark(1.0, 0.0, false, true);
            if !available {
                eprintln!("deferred decal check skipped: ray tracing unavailable");
                return;
            }
            let bare = run_mark(1.0, 0.0, true, true).0;
            assert!(
                marked[0] > marked[1] + 10 && bare[1] > marked[1] + 15,
                "deferred decals should tint the lit G-buffer: marked {marked:?}, bare {bare:?}"
            );
        }
    }

    /// The 3D starter's floor with no sun and no ambient light, plus a small
    /// still lamp above the middle of it, which batching merges.
    #[test]
    #[ignore = "needs a GPU"]
    fn destruction_and_fluids_draw_through_the_runtime() {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::components::ActorComponent;
        use blockloom_core::destruction::FractureSpec;
        use blockloom_core::value::Value;
        let mut room = dark_room(true);
        let mut cube = blockloom_core::project::Actor::new(
            "Crate",
            blockloom_core::scene::Visual::Cuboid {
                color: "#FFFFFF".into(),
                size: [2.0; 3],
            },
        );
        cube.components.placement_mut().position = [-2.0, 2.0, 0.0];
        cube.components.insert(ActorComponent::Fracture {
            fracture: FractureSpec {
                cells: 8,
                lifetime: 30.0,
                ..Default::default()
            },
        });
        room.actors.push(cube);
        room.actors[0].graph.strands.push(Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenStarted),
                Instruction::new(K::Fracture {
                    target: Value::text("Crate"),
                }),
                Instruction::new(K::Splash {
                    x: Value::number(0.0),
                    y: Value::number(0.0),
                    z: Value::number(0.0),
                    radius: Value::number(12.0),
                    strength: Value::number(0.2),
                }),
                Instruction::new(K::PuffSmoke {
                    x: Value::number(2.0),
                    y: Value::number(1.0),
                    z: Value::number(0.0),
                    radius: Value::number(1.0),
                    strength: Value::number(1.0),
                }),
            ],
        ));
        let (set, _, reports) = run_world_reporting(
            room,
            |_| {},
            game_camera(),
            180,
            |_| false,
            vec![EditorMessage::Start],
        );
        let errors: Vec<_> = reports
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    RuntimeMessage::Error { .. } | RuntimeMessage::Fatal { .. }
                )
            })
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(set.is_some(), "no rendered frame");
        for name in ["destruction/shards", "fluids/smoke_grids"] {
            assert!(
                reports.iter().any(|r| match r {
                    RuntimeMessage::Status(status) => status
                        .render_metrics
                        .iter()
                        .any(|m| m.name == name && m.value > 0.0),
                    _ => false,
                }),
                "{name} never became active"
            );
        }
    }

    fn dark_room(lamp: bool) -> blockloom_core::project::Project {
        use blockloom_core::components::{ActorComponent, LightSpec};
        use blockloom_core::scene::Visual;
        let mut room = blockloom_core::project::Project::starter("Room", Mode::ThreeD);
        room.world.background = "#000000".to_string();
        room.world.lighting.illuminance = 0.0;
        room.world.lighting.ambient_brightness = 0.0;
        room.actors.retain(|actor| actor.name == "Ground");
        room.actors[0].components.set_visual(Visual::Plane {
            color: "#FFFFFF".to_string(),
            size: [20.0, 20.0],
        });
        if lamp {
            let mut lamp = blockloom_core::project::Actor::new(
                "Lamp",
                Visual::Sphere {
                    color: "#FFFFFF".to_string(),
                    radius: 0.05,
                },
            );
            lamp.components.placement_mut().position = [0.0, 1.5, 3.0];
            lamp.components.insert(ActorComponent::Light {
                light: LightSpec {
                    intensity: 1_000_000.0,
                    ..LightSpec::default()
                },
            });
            room.actors.push(lamp);
        }
        room
    }

    /// Whether a pixel is one of the false color bands, as sRGB.
    fn is_false_color(pixel: [u8; 3]) -> bool {
        const BANDS: &[[f32; 3]] = &[
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.8, 0.0],
            [0.45, 0.45, 0.45],
            [0.1, 0.8, 0.1],
            [0.12, 0.12, 0.12],
            [0.0, 0.35, 0.45],
            [0.0, 0.05, 0.6],
            [0.2, 0.0, 0.3],
        ];
        let srgb =
            |linear: f32| (bevy::color::Srgba::gamma_function_inverse(linear) * 255.0).round();
        BANDS.iter().any(|band| {
            band.iter()
                .zip(pixel)
                .all(|(linear, got)| (srgb(*linear) - got as f32).abs() <= 6.0)
        })
    }

    /// Looking through the game's own camera, so the scene view's grid and
    /// selection stay out of the frame.
    fn game_camera() -> SceneView {
        SceneView {
            enabled: false,
            ..SceneView::default()
        }
    }

    fn is_red([r, g, b]: [u8; 3]) -> bool {
        r > 150 && g < 80 && b < 80
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

    /// Runs a project until a frame's middle pixel passes `done`, or for
    /// `settle` frames and then the next one, answering the ring it arrived
    /// in, which slot, and any errors the world sent.
    fn run_world(
        project: blockloom_core::project::Project,
        offer: impl FnOnce(&FrameExchange),
        view: SceneView,
        settle: usize,
        done: impl Fn([u8; 3]) -> bool,
    ) -> (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>) {
        run_world_sending(project, offer, view, settle, done, Vec::new())
    }

    /// `run_world`, with `extra` sent to the world after the project.
    fn run_world_sending(
        project: blockloom_core::project::Project,
        offer: impl FnOnce(&FrameExchange),
        view: SceneView,
        settle: usize,
        done: impl Fn([u8; 3]) -> bool,
        extra: Vec<EditorMessage>,
    ) -> (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>) {
        let (set, index, reports) = run_world_reporting(project, offer, view, settle, done, extra);
        let errors = reports
            .into_iter()
            .filter(|report| {
                matches!(
                    report,
                    RuntimeMessage::Error { .. } | RuntimeMessage::Fatal { .. }
                )
            })
            .collect();
        (set, index, errors)
    }

    /// As `run_world_sending`, but hands back everything the world reported.
    fn run_world_reporting(
        project: blockloom_core::project::Project,
        offer: impl FnOnce(&FrameExchange),
        view: SceneView,
        settle: usize,
        done: impl Fn([u8; 3]) -> bool,
        extra: Vec<EditorMessage>,
    ) -> (Option<Arc<SlotSet>>, usize, Vec<RuntimeMessage>) {
        blockloom_core::init();
        let (to_world, incoming) = std::sync::mpsc::channel();
        let (outgoing, reports) = std::sync::mpsc::channel();
        let exchange = FrameExchange::new(|| {});
        offer(&exchange);
        let size = exchange.wanted().size;
        let frames = exchange.clone();
        let project_mode = project.world.mode;
        // Worlds creating Vulkan instances at once crash in the loader's
        // driver negotiation, so they start one at a time.
        static STARTING: Mutex<()> = Mutex::new(());
        let mut starting = Some(STARTING.lock().unwrap_or_else(|e| e.into_inner()));
        let world = std::thread::spawn(move || {
            run(Embedded {
                mode: project_mode,
                incoming,
                outgoing,
                frames,
            })
        });
        to_world.send(EditorMessage::SceneView(view)).unwrap();
        to_world
            .send(EditorMessage::Load {
                project: Box::new(project),
                dir: None,
            })
            .unwrap();
        for message in extra {
            to_world.send(message).unwrap();
        }

        // The first frames come before the world has a camera; wait for
        // several, so one that shows the background has landed.
        let started = std::time::Instant::now();
        let mut seen = None;
        let mut frames_seen = 0;
        let mut last_frame = None;
        while started.elapsed() < Duration::from_secs(20) {
            if exchange.slots().is_some() {
                starting = None;
            }
            if let (Some(set), Some((generation, index))) = (exchange.slots(), exchange.latest())
                && set.width == size.x
                && set.generation == generation
                && last_frame != Some((generation, index))
            {
                last_frame = Some((generation, index));
                // Keep the world off this slot while it is read.
                exchange.hold(generation, index);
                exchange.presented();
                seen = Some((set.clone(), index));
                frames_seen += 1;
                if set.modifier != MODIFIER_LINEAR && frames_seen > 30 {
                    break;
                }
                if set.modifier == MODIFIER_LINEAR {
                    let pixel = middle_pixel(&set.images[index], size.x as usize, size.y as usize);
                    if done(pixel) || (settle > 0 && frames_seen > settle) {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        drop(starting);
        to_world.send(EditorMessage::Shutdown).unwrap();
        drop(to_world);
        world.join().unwrap();
        let reports: Vec<_> = reports.try_iter().collect();
        // The world's end is the viewer's cue to show nothing.
        assert!(exchange.slots().is_none());
        let (set, index) = seen.map_or((None, 0), |(set, index)| (Some(set), index));
        (set, index, reports)
    }

    /// Runs a project for `settle` frames, then keeps the next `keep`, each
    /// read out whole: what `run_world` can't give, a run of frames to
    /// compare.
    fn run_world_frames(
        project: blockloom_core::project::Project,
        size: UVec2,
        view: SceneView,
        settle: usize,
        keep: usize,
    ) -> Vec<Vec<[u8; 3]>> {
        blockloom_core::init();
        let (to_world, incoming) = std::sync::mpsc::channel();
        let (outgoing, _reports) = std::sync::mpsc::channel();
        let exchange = FrameExchange::new(|| {});
        exchange.resize(size.x, size.y, 1.0);
        let frames = exchange.clone();
        let project_mode = project.world.mode;
        static STARTING: Mutex<()> = Mutex::new(());
        let mut starting = Some(STARTING.lock().unwrap_or_else(|e| e.into_inner()));
        let world = std::thread::spawn(move || {
            run(Embedded {
                mode: project_mode,
                incoming,
                outgoing,
                frames,
            })
        });
        to_world.send(EditorMessage::SceneView(view)).unwrap();
        to_world
            .send(EditorMessage::Load {
                project: Box::new(project),
                dir: None,
            })
            .unwrap();
        let (mut last_frame, mut seen, mut kept) = (None, 0, Vec::new());
        let started = std::time::Instant::now();
        while kept.len() < keep && started.elapsed() < Duration::from_secs(1800) {
            if exchange.slots().is_some() {
                starting = None;
            }
            if let (Some(set), Some((generation, index))) = (exchange.slots(), exchange.latest())
                && set.width == size.x
                && set.generation == generation
                && last_frame != Some((generation, index))
            {
                last_frame = Some((generation, index));
                exchange.hold(generation, index);
                exchange.presented();
                seen += 1;
                assert_eq!(set.modifier, MODIFIER_LINEAR, "needs a linear ring");
                if seen > settle {
                    kept.push(frame_pixels(
                        &set.images[index],
                        size.x as usize,
                        size.y as usize,
                    ));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(starting);
        to_world.send(EditorMessage::Shutdown).unwrap();
        drop(to_world);
        world.join().unwrap();
        kept
    }

    /// Reads a whole frame out of a linear dma-buf, row by row.
    fn frame_pixels(image: &SharedImage, width: usize, height: usize) -> Vec<[u8; 3]> {
        use std::os::fd::AsRawFd;
        assert!(image.stride as usize >= width * 4);
        let length = image.offset as usize + image.stride as usize * height;
        // SAFETY: as `middle_pixel`.
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
            let frame = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let at = image.offset as usize + image.stride as usize * y + x * 4;
                    [bytes[at], bytes[at + 1], bytes[at + 2]]
                })
                .collect();
            libc::munmap(mapped, length);
            frame
        }
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
