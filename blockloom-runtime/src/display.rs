//! True HDR output for a windowed runtime. Bevy only ever configures an 8-bit
//! sRGB swapchain, so where the display also offers an HDR color space this
//! module owns the window's surface instead: it takes the window's handle
//! before Bevy's `create_surfaces` sees it, configures the surface in the
//! space `HdrFrame` names (SDR, scRGB or HDR10), and hands Bevy each frame's
//! texture the way `prepare_windows` would. Bevy still renders and presents.
//! A display with no HDR space is left to Bevy entirely.

#[cfg(not(target_arch = "wasm32"))]
use crate::hdr::{DisplayOffers, HdrFrame};
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
struct Formats {
    sdr: TextureFormat,
    scrgb: bool,
    hdr10: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Formats {
    fn of(caps: &wgpu::SurfaceCapabilities) -> Option<Formats> {
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

    fn offers(&self) -> Vec<OutputSpace> {
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
    fn configure(&self, space: OutputSpace) -> (TextureFormat, SurfaceColorSpace) {
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
        let Some(formats) = Formats::of(&caps).filter(|f| f.scrgb || f.hdr10) else {
            offers.set(vec![OutputSpace::Sdr]);
            commands.entity(entity).insert(SurfaceDeclined);
            continue;
        };
        offers.set(formats.offers());
        let (format, color_space) = formats.configure(frame.space);
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
        commands
            .entity(entity)
            .remove::<RawHandleWrapper>()
            .insert(OwnSurface {
                surface,
                config,
                formats,
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
            let view = view_format(format);
            let config = &mut owned.config;
            config.format = format;
            config.color_space = color_space;
            config.width = window.physical_width;
            config.height = window.physical_height;
            config.present_mode = present_mode(window.present_mode, &caps);
            config.view_formats = if view != format { vec![view] } else { vec![] };
            device.configure_surface(&owned.surface, &owned.config);
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
            texture = owned.surface.get_current_texture();
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
