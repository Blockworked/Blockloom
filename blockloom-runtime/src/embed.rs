//! The world embedded in the editor: no window and no pipes.
//!
//! The editor calls [`run`] on a thread of its own. Messages travel over
//! channels. Every world camera renders into one offscreen target, and each
//! frame is copied on the GPU into a small ring of linear dma-bufs that the
//! editor's Game view imports through EGL - the CPU never touches a pixel.

use crate::bridge;
use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::app::{PluginsState, TerminalCtrlCHandlerPlugin};
use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::ui::IsDefaultUiCamera;
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
/// What a player's window would show; the editor scales it to fit.
const SIZE: UVec2 = UVec2::new(GAME_SIZE.0, GAME_SIZE.1);
/// The longest a world waits for the display: a hidden view presents
/// nothing, and the world still has to keep hearing the editor.
const PACE_TIMEOUT: Duration = Duration::from_millis(50);
/// `DRM_FORMAT_XBGR8888`: RGBA bytes in memory, alpha ignored.
const FOURCC_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
/// `DRM_FORMAT_MOD_LINEAR`.
const MODIFIER_LINEAR: u64 = 0;

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
        })
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

    fn install(&self, width: u32, height: u32, images: Vec<SharedImage>) -> u64 {
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
                modifier: MODIFIER_LINEAR,
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
    built: bool,
}

impl GameSurface {
    /// What pointer coordinates are measured against.
    pub fn size(&self) -> Vec2 {
        SIZE.as_vec2()
    }
}

/// What the render world copies each finished frame between: the cameras'
/// own target, which stays optimally tiled for drawing, and the linear ring
/// the viewer reads. Some GPUs can't draw into a linear image at all.
#[derive(Resource, Clone, ExtractResource)]
struct FrameCopy {
    exchange: Arc<FrameExchange>,
    generation: u64,
    source: Option<wgpu::Texture>,
    ring: Vec<wgpu::Texture>,
}

fn add_surface(app: &mut App, exchange: Arc<FrameExchange>) {
    app.insert_resource(GameSurface {
        exchange: exchange.clone(),
        built: false,
    })
    .insert_resource(FrameCopy {
        exchange,
        generation: 0,
        source: None,
        ring: Vec::new(),
    })
    .add_plugins(ExtractResourcePlugin::<FrameCopy>::default())
    .add_systems(Update, target_cameras)
    .add_systems(Last, build_surface);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(Render, copy_frame.in_set(RenderSystems::Cleanup));
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

/// Allocates the target and the ring once the render device exists. If
/// sharing fails the world still runs, unseen, rather than retrying.
fn build_surface(
    mut surface: ResMut<GameSurface>,
    mut copy: ResMut<FrameCopy>,
    device: Res<RenderDevice>,
    mut views: ResMut<ManualTextureViews>,
) {
    if surface.built {
        return;
    }
    surface.built = true;
    let size = SIZE;
    let device = device.wgpu_device();
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("game view target"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    views.insert(
        VIEW,
        ManualTextureView {
            texture_view: source
                .create_view(&wgpu::TextureViewDescriptor::default())
                .into(),
            size,
            view_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        },
    );
    copy.source = Some(source);

    let mut ring = Vec::with_capacity(SLOTS);
    let mut images = Vec::with_capacity(SLOTS);
    for _ in 0..SLOTS {
        match dmabuf::allocate(device, size) {
            Ok((texture, image)) => {
                ring.push(texture);
                images.push(image);
            }
            Err(error) => {
                tracing::error!("game view: {error}");
                bridge::send(&RuntimeMessage::Error {
                    actor: "Game view".to_string(),
                    message: format!("Couldn't share frames with the editor: {error}"),
                });
                return;
            }
        }
    }
    copy.ring = ring;
    copy.generation = surface.exchange.install(size.x, size.y, images);
}

/// After the cameras' submission: copy the frame into a free slot, and tell
/// the viewer once the GPU has finished. With no slot free the frame is
/// simply not shown - the viewer is still on an older one.
fn copy_frame(copy: Option<Res<FrameCopy>>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    if let Some(copy) = copy
        && let Some(source) = &copy.source
        && !copy.ring.is_empty()
        && let Some(index) = copy.exchange.claim(copy.generation)
    {
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
        let (exchange, generation) = (copy.exchange.clone(), copy.generation);
        queue.on_submitted_work_done(move || exchange.finished(generation, index));
    }
    // Callbacks only fire when the device is polled.
    let _ = device.poll(wgpu::PollType::Poll);
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
    use super::{SharedImage, extent};
    use ash::vk;
    use bevy::math::UVec2;
    use std::os::fd::{FromRawFd, OwnedFd};
    use wgpu::hal::api::Vulkan;

    /// Byte-for-byte what the sRGB target holds; the viewer shows it as is.
    const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
    const HANDLE: vk::ExternalMemoryHandleTypeFlags =
        vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;

    /// A linear, exportable RGBA image on wgpu's own Vulkan device, wrapped
    /// as a wgpu texture a frame can be copied into, plus the fd another API
    /// imports it by. Linear and in system memory, so the viewer needs no
    /// modifier support and can sit on a different GPU.
    pub fn allocate(
        device: &wgpu::Device,
        size: UVec2,
    ) -> Result<(wgpu::Texture, SharedImage), String> {
        // SAFETY: every Vulkan object is made on wgpu's device and handed
        // straight back to wgpu, which owns and frees the image and memory.
        unsafe {
            let hal = device
                .as_hal::<Vulkan>()
                .ok_or("the renderer isn't using Vulkan")?;
            let raw = hal.raw_device();
            let instance = hal.shared_instance().raw_instance();
            let physical = hal.raw_physical_device();

            let features = instance
                .get_physical_device_format_properties(physical, FORMAT)
                .linear_tiling_features;
            if !features.contains(vk::FormatFeatureFlags::TRANSFER_DST) {
                return Err("this GPU can't copy into a linear RGBA image".to_string());
            }

            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(HANDLE);
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(FORMAT)
                .extent(vk::Extent3D {
                    width: size.x,
                    height: size.y,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::LINEAR)
                .usage(vk::ImageUsageFlags::TRANSFER_DST)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external);
            let image = raw
                .create_image(&info, None)
                .map_err(|e| format!("creating a shared image: {e}"))?;

            let needs = raw.get_image_memory_requirements(image);
            let properties = instance.get_physical_device_memory_properties(physical);
            let allowed = |i: u32| needs.memory_type_bits & (1 << i) != 0;
            let flags = |i: u32| properties.memory_types[i as usize].property_flags;
            // System memory first, which any GPU can read; on one GPU with
            // shared memory the first allowed type is the same thing.
            let Some(memory_type) = (0..properties.memory_type_count)
                .find(|&i| allowed(i) && !flags(i).contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
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
                ash::khr::external_memory_fd::Device::new(instance, raw).get_memory_fd(
                    &vk::MemoryGetFdInfoKHR::default()
                        .memory(memory)
                        .handle_type(HANDLE),
                )
            });
            let fd = match exported {
                Ok(fd) => OwnedFd::from_raw_fd(fd),
                Err(e) => {
                    raw.destroy_image(image, None);
                    raw.free_memory(memory, None);
                    return Err(format!("exporting a shared image: {e}"));
                }
            };
            let layout = raw.get_image_subresource_layout(
                image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    array_layer: 0,
                },
            );

            let hal_texture = hal.texture_from_raw(
                image,
                &wgpu::hal::TextureDescriptor {
                    label: Some("game view"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUses::COPY_DST,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                },
                None,
                wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
            );
            drop(hal);
            let texture = device.create_texture_from_hal::<Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("game view"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            );
            Ok((
                texture,
                SharedImage {
                    fd,
                    offset: layout.offset as u32,
                    stride: layout.row_pitch as u32,
                },
            ))
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
        let generation = exchange.install(4, 4, images(3));
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
        let old = exchange.install(4, 4, images(3));
        let slot = exchange.claim(old).unwrap();
        let new = exchange.install(8, 8, images(3));
        exchange.finished(old, slot);
        assert_eq!(exchange.latest(), None);
        assert_eq!(exchange.claim(old), None);
        assert!(exchange.claim(new).is_some());
        exchange.clear();
        assert!(exchange.slots().is_none());
    }

    // Needs a Vulkan GPU with dma-buf export: `cargo test -p blockloom-runtime -- --ignored embedded`.
    #[test]
    #[ignore = "needs a GPU"]
    fn an_embedded_world_draws_into_shared_images() {
        blockloom_core::init();
        let (to_world, incoming) = std::sync::mpsc::channel();
        let (outgoing, reports) = std::sync::mpsc::channel();
        let exchange = FrameExchange::new(|| {});
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

        // The first frames come before the world has a camera; wait for one
        // that shows the background.
        let started = std::time::Instant::now();
        let mut seen = None;
        while started.elapsed() < Duration::from_secs(20) {
            if let (Some(set), Some((generation, index))) = (exchange.slots(), exchange.latest())
                && set.width == SIZE.x
                && set.generation == generation
            {
                // Keep the world off this slot while it is read.
                exchange.hold(generation, index);
                seen = Some((
                    set.images.len(),
                    middle_pixel(&set.images[index], SIZE.x as usize, SIZE.y as usize),
                ));
                if seen.is_some_and(|(_, [r, g, b])| r > 150 && g < 80 && b < 80) {
                    break;
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
        let (slots, pixel) = seen.unwrap_or_else(|| panic!("no frame arrived: {errors:?}"));
        assert_eq!(slots, SLOTS);
        // Red first in memory, the way XBGR8888 says.
        assert!(
            pixel[0] > 150 && pixel[1] < 80 && pixel[2] < 80,
            "expected red, read {pixel:?}"
        );
        // The world's end is the viewer's cue to show nothing.
        assert!(exchange.slots().is_none());
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
