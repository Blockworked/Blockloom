//! True HDR output for a windowed runtime. Bevy only ever configures an 8-bit
//! sRGB swapchain, so where the display also offers an HDR color space this
//! module owns the window's surface instead: it takes the window's handle
//! before Bevy's `create_surfaces` sees it, configures the surface in the
//! space `HdrFrame` names (SDR, scRGB or HDR10), and hands Bevy each frame's
//! texture the way `prepare_windows` would. Bevy still renders and presents.
//! A display with no HDR space is left to Bevy entirely.

#[cfg(not(target_arch = "wasm32"))]
use crate::hdr::{DisplayOffers, HdrFrame, HdrMetadata};
#[cfg(not(target_arch = "wasm32"))]
use bevy::ecs::schedule::ApplyDeferred;
use bevy::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::render_resource::{SurfaceTexture, TextureView};
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderInstance};
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::view::window::{ExtractedWindow, SurfaceData, create_surfaces, prepare_windows};
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::view::{prepare_view_attachments, prepare_view_targets};
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::{Render, RenderApp, RenderSystems};
#[cfg(not(target_arch = "wasm32"))]
use bevy::window::{CompositeAlphaMode, PresentMode, RawHandleWrapper};
#[cfg(not(target_arch = "wasm32"))]
use blockloom_core::scene::OutputSpace;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::{SurfaceColorSpace, SurfaceColorSpaces, TextureFormat};

pub fn register(app: &mut App) {
    // Web builds stay on Bevy's SDR swapchain: there is no HDR takeover in
    // the browser, and web builds ship SDR-only.
    #[cfg(target_arch = "wasm32")]
    let _ = app;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render.add_systems(
            Render,
            (
                // In `PrepareViews` so the frame's extraction has landed before
                // this reads it (`HdrFrame` only arrives with the first frame),
                // still ahead of Bevy's own surface creation.
                (adopt_window, ApplyDeferred)
                    .chain()
                    .in_set(RenderSystems::PrepareViews)
                    .before(create_surfaces),
                acquire_frame
                    .in_set(RenderSystems::PrepareViews)
                    .after(prepare_windows)
                    .before(prepare_view_attachments)
                    .before(prepare_view_targets),
            ),
        );
    }
}

/// A window whose surface this module owns, in the space it last configured.
#[derive(Component)]
#[cfg(not(target_arch = "wasm32"))]
struct OwnSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    formats: Formats,
    /// What the swapchain was last told it carries, cleared by a reconfigure.
    metadata: Option<HdrMetadata>,
    // Keeps the native window alive as long as its surface.
    _handle: RawHandleWrapper,
}

/// A window whose display offers no HDR space: Bevy keeps it.
#[derive(Component)]
#[cfg(not(target_arch = "wasm32"))]
struct SurfaceDeclined;

/// The format for each space the surface was found to take.
#[derive(Clone, Copy, Debug)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Formats {
    sdr: TextureFormat,
    scrgb: bool,
    hdr10: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Formats {
    pub(crate) fn of(caps: &wgpu::SurfaceCapabilities) -> Option<Formats> {
        let sdr = caps
            .formats
            .iter()
            .copied()
            .find(|format| {
                matches!(
                    format,
                    TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb
                )
            })
            .or_else(|| caps.formats.first().copied())?;
        Some(Formats {
            sdr,
            scrgb: caps
                .color_spaces(TextureFormat::Rgba16Float)
                .contains(SurfaceColorSpaces::EXTENDED_SRGB_LINEAR),
            hdr10: caps
                .color_spaces(TextureFormat::Rgb10a2Unorm)
                .contains(SurfaceColorSpaces::BT2100_PQ),
        })
    }

    pub(crate) fn offers(&self) -> Vec<OutputSpace> {
        let mut offers = vec![OutputSpace::Sdr];
        if self.scrgb {
            offers.push(OutputSpace::Scrgb);
        }
        if self.hdr10 {
            offers.push(OutputSpace::Hdr10);
        }
        offers
    }

    /// The format and color space a space is configured with, SDR when the
    /// surface can't do it.
    pub(crate) fn configure(&self, space: OutputSpace) -> (TextureFormat, SurfaceColorSpace) {
        match space {
            OutputSpace::Scrgb if self.scrgb => (
                TextureFormat::Rgba16Float,
                SurfaceColorSpace::ExtendedSrgbLinear,
            ),
            OutputSpace::Hdr10 if self.hdr10 => {
                (TextureFormat::Rgb10a2Unorm, SurfaceColorSpace::Bt2100Pq)
            }
            _ => (self.sdr, SurfaceColorSpace::Auto),
        }
    }
}

/// Whether the device will take this pair. Adapter caps have claimed a
/// space the configure then refuses on some drivers, which fails the run,
/// so every HDR configure checks fresh caps first.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn takes(
    caps: &wgpu::SurfaceCapabilities,
    format: TextureFormat,
    color_space: SurfaceColorSpace,
) -> bool {
    let flag = match color_space {
        SurfaceColorSpace::ExtendedSrgbLinear => SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
        SurfaceColorSpace::Bt2100Pq => SurfaceColorSpaces::BT2100_PQ,
        _ => return true,
    };
    caps.color_spaces(format).contains(flag)
}

#[cfg(not(target_arch = "wasm32"))]
fn present_mode(mode: PresentMode, caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    use wgpu::PresentMode as P;
    let wanted: &[P] = match mode {
        PresentMode::AutoVsync => &[P::FifoRelaxed, P::Fifo],
        PresentMode::AutoNoVsync => &[P::Immediate, P::Mailbox, P::Fifo],
        PresentMode::Mailbox => &[P::Mailbox, P::Immediate, P::Fifo],
        PresentMode::Immediate => &[P::Immediate, P::Fifo],
        PresentMode::FifoRelaxed => &[P::FifoRelaxed, P::Fifo],
        PresentMode::Fifo => &[P::Fifo],
    };
    wanted
        .iter()
        .copied()
        .find(|mode| caps.present_modes.contains(mode))
        .unwrap_or(P::Fifo)
}

#[cfg(not(target_arch = "wasm32"))]
fn alpha_mode(mode: CompositeAlphaMode) -> wgpu::CompositeAlphaMode {
    match mode {
        CompositeAlphaMode::Auto => wgpu::CompositeAlphaMode::Auto,
        CompositeAlphaMode::Opaque => wgpu::CompositeAlphaMode::Opaque,
        CompositeAlphaMode::PreMultiplied => wgpu::CompositeAlphaMode::PreMultiplied,
        CompositeAlphaMode::PostMultiplied => wgpu::CompositeAlphaMode::PostMultiplied,
        CompositeAlphaMode::Inherit => wgpu::CompositeAlphaMode::Inherit,
    }
}

/// Views are always drawn through an sRGB view of an 8-bit surface, as
/// Bevy's own are; HDR formats have no sRGB twin.
#[cfg(not(target_arch = "wasm32"))]
fn view_format(format: TextureFormat) -> TextureFormat {
    format.add_srgb_suffix()
}

/// A window nobody has made a surface for yet.
#[cfg(not(target_arch = "wasm32"))]
type Unadopted = (
    Without<SurfaceData>,
    Without<OwnSurface>,
    Without<SurfaceDeclined>,
);

/// Makes a surface for each new window before Bevy can, and keeps it if
/// the display offers any HDR space.
#[cfg(not(target_arch = "wasm32"))]
fn adopt_window(
    mut commands: Commands,
    #[cfg(any(target_os = "macos", target_os = "ios"))] _main: bevy::ecs::system::NonSendMarker,
    windows: Query<(Entity, &ExtractedWindow, &RawHandleWrapper), Unadopted>,
    instance: Res<RenderInstance>,
    adapter: Res<RenderAdapter>,
    device: Res<RenderDevice>,
    offers: Res<DisplayOffers>,
    frame: Res<HdrFrame>,
) {
    for (entity, window, handle) in &windows {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(handle.get_display_handle()),
            raw_window_handle: handle.get_window_handle(),
        };
        // SAFETY: the handle is kept alive beside the surface, in `OwnSurface`.
        let Ok(surface) = (unsafe { instance.create_surface_unsafe(target) }) else {
            commands.entity(entity).insert(SurfaceDeclined);
            continue;
        };
        let caps = surface.get_capabilities(&adapter);
        let Some(mut formats) = Formats::of(&caps).filter(|f| f.scrgb || f.hdr10) else {
            offers.set(vec![OutputSpace::Sdr]);
            commands.entity(entity).insert(SurfaceDeclined);
            continue;
        };
        let (mut format, mut color_space) = formats.configure(frame.space);
        if !takes(&caps, format, color_space) {
            // Caps claimed a space the device refuses; stay SDR.
            formats.scrgb = false;
            formats.hdr10 = false;
            (format, color_space) = formats.configure(frame.space);
        }
        offers.set(formats.offers());
        let view = view_format(format);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space,
            width: window.physical_width,
            height: window.physical_height,
            present_mode: present_mode(window.present_mode, &caps),
            desired_maximum_frame_latency: window
                .desired_maximum_frame_latency
                .map(|latency| latency.get())
                .unwrap_or(2),
            alpha_mode: alpha_mode(window.alpha_mode),
            view_formats: if view != format { vec![view] } else { vec![] },
        };
        device.configure_surface(&surface, &config);
        info!(
            "Owning the window's surface for HDR: {:?}",
            formats.offers()
        );
        let metadata = frame.metadata();
        if let Some(metadata) = &metadata {
            send_metadata(&surface, device.wgpu_device(), metadata);
        }
        commands
            .entity(entity)
            .remove::<RawHandleWrapper>()
            .insert(OwnSurface {
                surface,
                config,
                formats,
                metadata,
                _handle: handle.clone(),
            });
    }
}

/// Reconfigures an owned surface when the window or the frame's space
/// changes, and hands Bevy this frame's texture to render into.
#[cfg(not(target_arch = "wasm32"))]
fn acquire_frame(
    #[cfg(any(target_os = "macos", target_os = "ios"))] _main: bevy::ecs::system::NonSendMarker,
    mut windows: Query<(&mut ExtractedWindow, &mut OwnSurface)>,
    adapter: Res<RenderAdapter>,
    device: Res<RenderDevice>,
    frame: Res<HdrFrame>,
    offers: Res<DisplayOffers>,
) {
    for (mut window, mut owned) in &mut windows {
        let (format, color_space) = owned.formats.configure(frame.space);
        let stale = owned.config.format != format
            || owned.config.color_space != color_space
            || owned.config.width != window.physical_width
            || owned.config.height != window.physical_height
            || window.present_mode_changed;
        if stale {
            drop(window.swap_chain_texture.take());
            drop(window.swap_chain_texture_view.take());
            let caps = owned.surface.get_capabilities(&adapter);
            let (format, color_space) = if takes(&caps, format, color_space) {
                (format, color_space)
            } else {
                // Caps claimed a space the device refuses; fall back to SDR
                // for the rest of the run rather than failing it.
                owned.formats.scrgb = false;
                owned.formats.hdr10 = false;
                offers.set(vec![OutputSpace::Sdr]);
                owned.formats.configure(frame.space)
            };
            let view = view_format(format);
            let config = &mut owned.config;
            config.format = format;
            config.color_space = color_space;
            config.width = window.physical_width;
            config.height = window.physical_height;
            config.present_mode = present_mode(window.present_mode, &caps);
            config.view_formats = if view != format { vec![view] } else { vec![] };
            device.configure_surface(&owned.surface, &owned.config);
            owned.metadata = None;
        }
        // Unpresented from last frame: keep drawing into it.
        if window.swap_chain_texture.is_some() && window.swap_chain_texture_view.is_some() {
            continue;
        }
        let mut texture = owned.surface.get_current_texture();
        if matches!(
            texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
        ) {
            device.configure_surface(&owned.surface, &owned.config);
            owned.metadata = None;
            texture = owned.surface.get_current_texture();
        }
        // Metadata belongs to the swapchain, so a new one is told again.
        let metadata = frame.metadata();
        if metadata != owned.metadata {
            if let Some(metadata) = &metadata {
                send_metadata(&owned.surface, device.wgpu_device(), metadata);
            }
            owned.metadata = metadata;
        }
        let texture = match texture {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            _ => continue,
        };
        let view = view_format(owned.config.format);
        window.swap_chain_texture_view = Some(TextureView::from(texture.texture.create_view(
            &wgpu::TextureViewDescriptor {
                format: Some(view),
                ..default()
            },
        )));
        window.swap_chain_texture = Some(SurfaceTexture::from(texture));
        window.swap_chain_texture_format = Some(owned.config.format);
        window.swap_chain_texture_view_format = Some(view);
    }
}

/// Turns on `VK_EXT_hdr_metadata` where the GPU has it, which wgpu leaves
/// off, so an HDR swapchain can say what it was mastered for.
#[cfg(target_os = "linux")]
pub fn add_vulkan_extensions(
    settings: &mut bevy::render::renderer::raw_vulkan_init::RawVulkanInitSettings,
) {
    use ash::ext;
    // SAFETY: only adds an extension the adapter reports supporting.
    unsafe {
        settings.add_create_device_callback(|args, adapter, _| {
            let name = ext::hdr_metadata::NAME;
            if adapter
                .physical_device_capabilities()
                .supports_extension(name)
                && !args.extensions.contains(&name)
            {
                args.extensions.push(name);
            }
        });
    }
}

/// Sends HDR10 static metadata for the surface's current swapchain. Vulkan
/// and DX12 take it; Metal's EDR has nothing to send.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send_metadata(
    surface: &wgpu::Surface,
    device: &wgpu::Device,
    metadata: &HdrMetadata,
) {
    #[cfg(target_os = "linux")]
    {
        use ash::{ext, vk};
        use wgpu::hal::api::Vulkan;
        // SAFETY: the swapchain and device are wgpu's own, alive for the
        // call, and the extension is checked before its function is loaded.
        unsafe {
            let (Some(hal_surface), Some(hal_device)) =
                (surface.as_hal::<Vulkan>(), device.as_hal::<Vulkan>())
            else {
                return;
            };
            let Some(swapchain) = hal_surface.raw_native_swapchain() else {
                return;
            };
            if !hal_device
                .enabled_device_extensions()
                .contains(&ext::hdr_metadata::NAME)
            {
                debug!("HDR metadata: VK_EXT_hdr_metadata isn't on");
                return;
            }
            let loader = ext::hdr_metadata::Device::new(
                hal_device.shared_instance().raw_instance(),
                hal_device.raw_device(),
            );
            let xy = |[x, y]: [f32; 2]| vk::XYColorEXT { x, y };
            let [red, green, blue, white] = metadata.primaries;
            let data = vk::HdrMetadataEXT::default()
                .display_primary_red(xy(red))
                .display_primary_green(xy(green))
                .display_primary_blue(xy(blue))
                .white_point(xy(white))
                .max_luminance(metadata.max_mastering_nits)
                .min_luminance(metadata.min_mastering_nits)
                .max_content_light_level(metadata.max_cll)
                .max_frame_average_light_level(metadata.max_fall);
            loader.set_hdr_metadata(&[swapchain], &[data]);
            info!(
                "HDR metadata sent: MaxCLL {} nits, MaxFALL {} nits",
                metadata.max_cll, metadata.max_fall
            );
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dxgi::{
            DXGI_HDR_METADATA_HDR10, DXGI_HDR_METADATA_TYPE_HDR10, IDXGISwapChain4,
        };
        use windows::core::Interface;
        let _ = device;
        // SAFETY: the swap chain is wgpu's own and alive for the call; the
        // metadata struct outlives it.
        unsafe {
            let Some(hal_surface) = surface.as_hal::<wgpu::hal::api::Dx12>() else {
                return;
            };
            let Some(swapchain) = hal_surface
                .swap_chain()
                .and_then(|chain| chain.cast::<IDXGISwapChain4>().ok())
            else {
                return;
            };
            // Chromaticities in 0.00002 steps, the floor in 0.0001 nits.
            let xy = |[x, y]: [f32; 2]| [(x * 50_000.0) as u16, (y * 50_000.0) as u16];
            let [red, green, blue, white] = metadata.primaries;
            let data = DXGI_HDR_METADATA_HDR10 {
                RedPrimary: xy(red),
                GreenPrimary: xy(green),
                BluePrimary: xy(blue),
                WhitePoint: xy(white),
                MaxMasteringLuminance: metadata.max_mastering_nits as u32,
                MinMasteringLuminance: (metadata.min_mastering_nits * 10_000.0) as u32,
                MaxContentLightLevel: metadata.max_cll as u16,
                MaxFrameAverageLightLevel: metadata.max_fall as u16,
            };
            if let Err(error) = swapchain.SetHDRMetaData(
                DXGI_HDR_METADATA_TYPE_HDR10,
                std::mem::size_of_val(&data) as u32,
                Some(std::ptr::from_ref(&data).cast()),
            ) {
                warn!("HDR metadata: {error}");
            }
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (surface, device, metadata);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(formats: &[(TextureFormat, SurfaceColorSpaces)]) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: formats.iter().map(|(format, _)| *format).collect(),
            format_capabilities: formats
                .iter()
                .map(|(format, color_spaces)| wgpu::SurfaceFormatCapabilities {
                    format: *format,
                    color_spaces: *color_spaces,
                })
                .collect(),
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        }
    }

    #[test]
    fn a_surface_offers_the_hdr_spaces_its_formats_carry() {
        let sdr_only = caps(&[(TextureFormat::Bgra8UnormSrgb, SurfaceColorSpaces::SRGB)]);
        let formats = Formats::of(&sdr_only).unwrap();
        assert_eq!(formats.offers(), [OutputSpace::Sdr]);
        assert_eq!(
            formats.configure(OutputSpace::Hdr10),
            (TextureFormat::Bgra8UnormSrgb, SurfaceColorSpace::Auto)
        );

        let hdr = caps(&[
            (TextureFormat::Bgra8Unorm, SurfaceColorSpaces::SRGB),
            (
                TextureFormat::Rgba16Float,
                SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
            ),
            (TextureFormat::Rgb10a2Unorm, SurfaceColorSpaces::BT2100_PQ),
        ]);
        let formats = Formats::of(&hdr).unwrap();
        assert_eq!(
            formats.offers(),
            [OutputSpace::Sdr, OutputSpace::Scrgb, OutputSpace::Hdr10]
        );
        assert_eq!(
            formats.configure(OutputSpace::Hdr10),
            (TextureFormat::Rgb10a2Unorm, SurfaceColorSpace::Bt2100Pq)
        );
        // An 8-bit surface is drawn through its sRGB view, as Bevy's is.
        assert_eq!(
            view_format(formats.configure(OutputSpace::Sdr).0),
            TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            view_format(TextureFormat::Rgba16Float),
            TextureFormat::Rgba16Float
        );
    }

    #[test]
    fn a_configure_is_checked_against_what_the_device_takes() {
        // Some drivers claim a space in caps that the configure then
        // refuses; that pair must read as unsupported.
        let lying = caps(&[(
            TextureFormat::Rgba16Float,
            SurfaceColorSpaces::SRGB,
        )]);
        assert!(!takes(
            &lying,
            TextureFormat::Rgba16Float,
            SurfaceColorSpace::ExtendedSrgbLinear
        ));
        let honest = caps(&[(
            TextureFormat::Rgba16Float,
            SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
        )]);
        assert!(takes(
            &honest,
            TextureFormat::Rgba16Float,
            SurfaceColorSpace::ExtendedSrgbLinear
        ));
        // SDR fallback pairs always pass.
        assert!(takes(
            &lying,
            TextureFormat::Bgra8UnormSrgb,
            SurfaceColorSpace::Auto
        ));
    }

    #[test]
    fn register_leaves_the_render_world_with_what_adopt_window_reads() {
        use crate::hdr::{DisplayOffers, HdrFrame};

        let mut app = App::new();
        // What the plugins would set up in a running game.
        bevy::tasks::IoTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        app.add_plugins((
            bevy::asset::AssetPlugin::default(),
            bevy::render::RenderPlugin::default(),
        ));
        crate::hdr::register(&mut app);
        super::register(&mut app);

        let render = app.get_sub_app(RenderApp).expect("a render app");
        // `HdrFrame` only arrives through extraction on the first frame,
        // while `adopt_window` already reads it there: without the seed its
        // very first run fails validation and the window is never adopted.
        assert!(render.world().contains_resource::<DisplayOffers>());
        assert!(render.world().contains_resource::<HdrFrame>());
    }
}
