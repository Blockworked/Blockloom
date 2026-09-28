//! One quality controller for both the editor view and shipped players.
use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::visibility::VisibleEntities;
use bevy::camera::{CameraOutputMode, CameraUpdateSystems, RenderTarget};
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::BlendState;
use bevy::render::renderer::{RenderAdapter, RenderDevice};
use bevy::render::sync_world::RenderEntity;
use bevy::render::view::{ExtractedView, Msaa};
use bevy::render::{Extract, ExtractSchedule, RenderApp};
use bevy::ui::IsDefaultUiCamera;
pub use blockloom_core::quality::DrawCost;
use blockloom_core::quality::{Budget, Controller, GeometryController, Settings, Upscaler};

#[derive(Resource)]
pub struct Scaling {
    pub settings: Settings,
    pub controller: Controller,
    pub geometry: GeometryController,
    overrides: Vec<(blockloom_core::quality::Setting, String)>,
    dropped: bool,
    pub draw_calls: u32,
    pub triangles: u64,
    /// Groups serving more than one visible mesh: one GPU draw covers a
    /// whole instanced or merged batch. `batched_instances` is the extra
    /// meshes those draws absorb, so `draw_calls + batched_instances` is
    /// the visible mesh count the estimate started from.
    pub instanced_draws: u32,
    pub batched_instances: u32,
    pub costs: [DrawCost; 6],
    /// True once the renderer reports `DlssSuperResolutionSupported` under
    /// the `dlss` cargo feature. Without that feature this stays false and
    /// the probe only fills `dlss_reason`.
    pub dlss_available: bool,
    pub dlss_reason: String,
}
pub const SYSTEMS: [&str; 6] = ["terrain", "vegetation", "props", "vfx", "debris", "water"];

#[derive(Component)]
struct NativeUiCamera;

impl Default for Scaling {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            controller: Controller::new(&settings),
            geometry: GeometryController::default(),
            settings,
            overrides: Vec::new(),
            dropped: false,
            draw_calls: 0,
            triangles: 0,
            instanced_draws: 0,
            batched_instances: 0,
            costs: [DrawCost::default(); 6],
            dlss_available: false,
            dlss_reason: String::new(),
        }
    }
}
impl Scaling {
    pub fn sample(&self) -> blockloom_core::quality::Sample {
        blockloom_core::quality::Sample {
            frame_ms: self.controller.frame_ms as f64,
            draw_calls: self.draw_calls,
            quality: self.controller.quality,
            dlss_available: self.dlss_available,
        }
    }
    pub fn particle_budget(&self, authored: u32) -> u32 {
        (authored.min(self.budget().particles) as f32 * self.geometry.factors[3]) as u32
    }
    pub fn shard_budget(&self, authored: usize) -> usize {
        (authored.min(self.budget().shards) as f32 * self.geometry.factors[4]) as usize
    }
    pub fn water_detail(&self) -> f32 {
        self.budget().density * self.geometry.factors[5]
    }
    pub fn budget(&self) -> Budget {
        self.controller.quality.budget()
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Scaling>()
        .add_systems(PreUpdate, feedback)
        .add_systems(
            Update,
            reset
                .after(crate::world::pump_editor)
                .before(crate::world::rebuild_world),
        )
        .add_systems(
            FixedUpdate,
            (
                fire_drop.before(crate::world::step_vm),
                apply_effects
                    .after(crate::world::apply_common)
                    .before(crate::world::clear_effects),
            )
                .in_set(crate::world::SimulationSet),
        )
        .add_systems(
            PostUpdate,
            (
                configure_cameras.after(crate::environment::apply_environment),
                probe_dlss.after(configure_cameras),
                configure_ui_camera.before(CameraUpdateSystems),
            ),
        )
        .add_systems(Last, measure_draws);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            ExtractSchedule,
            scale_views.after(bevy::render::camera::extract_cameras),
        );
    }
}

/// Composite SDR UI after the scene's spatial blit. HDR keeps its linear UI
/// blend before encoding until that chain has a native composition target.
fn configure_ui_camera(
    mut commands: Commands,
    scaling: Res<Scaling>,
    frame: Res<crate::hdr::HdrFrame>,
    roots: Query<
        (&Node, Has<crate::ui::UiRoot>, Option<&Children>),
        (Without<ChildOf>, Without<crate::overlay::OverlayText>),
    >,
    worlds: Query<(Entity, &Camera, &RenderTarget, Has<IsDefaultUiCamera>), With<WorldCamera>>,
    mut overlays: Query<
        (
            Entity,
            &mut Camera,
            &mut RenderTarget,
            Has<IsDefaultUiCamera>,
        ),
        (With<NativeUiCamera>, Without<WorldCamera>),
    >,
) {
    let world = worlds.iter().find(|(_, camera, ..)| camera.is_active);
    let Some((world_entity, world_camera, target, world_ui)) = world else {
        for (entity, ..) in &overlays {
            commands.entity(entity).despawn();
        }
        return;
    };
    let native = scaling.controller.scale < 1.0
        && world_camera.viewport.is_none()
        && !frame.is_hdr()
        && roots.iter().any(|(node, interface, children)| {
            node.display != Display::None
                && (!interface || children.is_some_and(|children| !children.is_empty()))
        });
    if world_ui == native {
        if native {
            commands.entity(world_entity).remove::<IsDefaultUiCamera>();
        } else {
            commands.entity(world_entity).insert(IsDefaultUiCamera);
        }
    }
    if let Ok((entity, mut camera, mut overlay_target, ui)) = overlays.single_mut() {
        if camera.is_active != native {
            camera.is_active = native;
        }
        let order = world_camera.order.saturating_add(1);
        if camera.order != order {
            camera.order = order;
        }
        if overlay_target.normalize(Some(Entity::PLACEHOLDER))
            != target.normalize(Some(Entity::PLACEHOLDER))
        {
            *overlay_target = target.clone();
        }
        if ui != native {
            if native {
                commands.entity(entity).insert(IsDefaultUiCamera);
            } else {
                commands.entity(entity).remove::<IsDefaultUiCamera>();
            }
        }
    } else if native {
        commands.spawn((
            NativeUiCamera,
            Camera2d,
            Camera {
                order: world_camera.order.saturating_add(1),
                clear_color: ClearColorConfig::Custom(Color::NONE),
                output_mode: CameraOutputMode::Write {
                    blend_state: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    clear_color: ClearColorConfig::None,
                },
                ..default()
            },
            target.clone(),
            RenderLayers::none(),
            IsDefaultUiCamera,
            Msaa::Off,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
        ));
    }
}

fn feedback(
    engine: NonSend<Engine>,
    diagnostics: Option<Res<DiagnosticsStore>>,
    pace: Option<Res<crate::performance::LoopPace>>,
    warmup: Option<Res<crate::streaming::Warmup>>,
    mut scaling: ResMut<Scaling>,
    lod: Option<ResMut<crate::culling::LodPolicy>>,
    batches: Option<ResMut<crate::batching::BatchPolicy>>,
) {
    let mut settings = engine.project.world.quality.clone();
    if engine.rebuild {
        scaling.overrides.clear();
        scaling.dropped = false;
    }
    for (key, value) in &scaling.overrides {
        let _ = settings.set(*key, value);
    }
    settings.normalize();
    if settings != scaling.settings || engine.rebuild {
        scaling.controller = Controller::new(&settings);
        scaling.geometry = GeometryController::default();
        scaling.settings = settings.clone();
    }
    if !engine.rebuild && !engine.paused && !warmup.is_some_and(|w| w.active()) {
        let gpu = diagnostics
            .as_ref()
            .and_then(|d| d.get(&crate::gpu::FRAME_GPU))
            .and_then(|d| d.value())
            .unwrap_or(0.0);
        // Work time excludes the embedded view's presentation wait.
        let cpu = pace.map_or(0.0, |p| p.update_ms.max(p.main_ms));
        let limits = scaling.budget();
        let costs = scaling.costs;
        scaling.geometry.sample(&settings, &limits, &costs);
        let geometry_over = scaling.geometry.needs_preset_drop(&limits, &costs);
        let dropped =
            scaling
                .controller
                .sample_with_cost(&settings, gpu.max(cpu) as f32, geometry_over);
        scaling.dropped |= dropped && engine.running;
    }
    let budget = scaling.budget();
    if let Some(mut lod) = lod {
        lod.bias = budget.distance;
    }
    if let Some(mut batches) = batches {
        batches.instance_threshold = 2;
    }
}

/// PCI vendor id for NVIDIA adapters, the only DLSS target.
const NVIDIA_VENDOR: u32 = 0x10DE;

/// Whether this build can even try DLSS: the `dlss` cargo feature links the
/// SDK path (needs `DLSS_SDK`, the Vulkan SDK and clang at build time).
/// Web builds never set it.
const DLSS_BUILD: bool = cfg!(feature = "dlss") && !cfg!(target_arch = "wasm32");

/// Whether a DLSS ask falls back to spatial-only instead of TAA plus
/// spatial. Web builds keep the cheap path; native rides TAA in 3D.
fn dlss_spatial_only() -> bool {
    cfg!(target_arch = "wasm32")
}

fn temporal_for(upscaler: Upscaler, dlss_spatial_only: bool) -> bool {
    match upscaler {
        Upscaler::Spatial => false,
        Upscaler::Taa => true,
        // Without DLSS running, the ask rides the TAA path in 3D -
        // except where it falls back to spatial-only like 2D does.
        Upscaler::Dlss => !dlss_spatial_only,
    }
}

/// Whether the world wants the real DLSS component rather than the TAA
/// fallback: the ask, the build, the probe, HDR (which `Dlss` requires)
/// and a perspective projection (which its extract requires).
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
fn wants_dlss(upscaler: Upscaler, available: bool, hdr: bool, perspective: bool) -> bool {
    upscaler == Upscaler::Dlss && available && hdr && perspective && DLSS_BUILD
}

/// Whether DLSS drives render resolution itself, so the spatial
/// `scale_views` stands down. Per-view HDR and projection are checked where
/// the component is inserted; here the global ask plus probe is enough to
/// avoid scaling twice.
fn dlss_drives_resolution(upscaler: Upscaler, available: bool) -> bool {
    upscaler == Upscaler::Dlss && available && DLSS_BUILD
}

/// Why DLSS can't run here. With the `dlss` feature the adapter only decides
/// how specific the driver/DLL half is; without it every branch names the
/// missing SDK build instead.
fn dlss_unavailable(wasm: bool, vendor: Option<u32>, name: &str, has_sdk: bool) -> Option<String> {
    if wasm {
        return Some("DLSS is off on web builds".into());
    }
    if !has_sdk {
        return match vendor {
            Some(NVIDIA_VENDOR) => Some(format!(
                "the {name} speaks DLSS, but this build leaves the DLSS SDK out (build with --features dlss)"
            )),
            Some(_) => Some(format!(
                "the {name} has no DLSS driver support, and this build leaves the SDK out either"
            )),
            None => Some("this build leaves the DLSS SDK out (build with --features dlss)".into()),
        };
    }
    match vendor {
        Some(NVIDIA_VENDOR) => Some(format!(
            "the {name} refused DLSS: needs an RTX GPU, Vulkan, a new driver and nvngx_dlss beside the player"
        )),
        Some(_) => Some(format!(
            "the {name} has no DLSS driver support (needs NVIDIA RTX)"
        )),
        None => Some("DLSS found no usable adapter on this renderer".into()),
    }
}

#[cfg(feature = "dlss")]
fn dlss_perf_mode(
    mode: blockloom_core::quality::DlssMode,
) -> bevy::anti_alias::dlss::DlssPerfQualityMode {
    use bevy::anti_alias::dlss::DlssPerfQualityMode as Bevy;
    match mode {
        blockloom_core::quality::DlssMode::Auto => Bevy::Auto,
        blockloom_core::quality::DlssMode::Dlaa => Bevy::Dlaa,
        blockloom_core::quality::DlssMode::Quality => Bevy::Quality,
        blockloom_core::quality::DlssMode::Balanced => Bevy::Balanced,
        blockloom_core::quality::DlssMode::Performance => Bevy::Performance,
        blockloom_core::quality::DlssMode::UltraPerformance => Bevy::UltraPerformance,
    }
}

/// Rank of a manual DLSS mode, cheapest last. `Auto` has no rank: the SDK
/// decides, and the dynamic-resolution signal steps aside.
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
fn dlss_rank(mode: blockloom_core::quality::DlssMode) -> u8 {
    use blockloom_core::quality::DlssMode as M;
    match mode {
        M::Auto => 0,
        M::Dlaa => 0,
        M::Quality => 1,
        M::Balanced => 2,
        M::Performance => 3,
        M::UltraPerformance => 4,
    }
}

#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
fn dlss_unrank(rank: u8) -> blockloom_core::quality::DlssMode {
    use blockloom_core::quality::DlssMode as M;
    match rank {
        0 => M::Dlaa,
        1 => M::Quality,
        2 => M::Balanced,
        3 => M::Performance,
        _ => M::UltraPerformance,
    }
}

/// The mode the `Dlss` component actually carries: the project's, stepped
/// down by the dynamic-resolution signal while it asks for less pixels.
/// Each 0.05 of scale below the ceiling is one step towards Performance,
/// floored at UltraPerformance; `Auto` stays `Auto` since the SDK is
/// already deciding, and without dynamic resolution the ask is the answer.
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
fn dlss_effective_mode(settings: &Settings, scale: f32) -> blockloom_core::quality::DlssMode {
    use blockloom_core::quality::{DlssMode as M, Upscaler};
    let selected = settings.dlss_mode;
    if settings.upscaler != Upscaler::Dlss
        || selected == M::Auto
        || !settings.dynamic_resolution
        || !scale.is_finite()
    {
        return selected;
    }
    let span = (settings.resolution_scale - settings.min_scale).max(0.05);
    let drop = (settings.resolution_scale - scale).clamp(0.0, span);
    let steps = (drop / 0.05).round() as u8;
    dlss_unrank(dlss_rank(selected).saturating_add(steps).min(4))
}

/// Whether the world wants ray reconstruction rather than plain super
/// resolution: the DLSS ask, both SDK probes, Hybrid Solari actually
/// lighting the view (which is what provides the reconstruction inputs),
/// HDR and a perspective projection.
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
fn wants_dlss_rr(
    upscaler: Upscaler,
    available: bool,
    rr_supported: bool,
    hybrid: bool,
    hdr: bool,
    perspective: bool,
) -> bool {
    upscaler == Upscaler::Dlss
        && available
        && rr_supported
        && hybrid
        && hdr
        && perspective
        && DLSS_BUILD
}

/// Probes DLSS capability once the renderer is up, and says why once when
/// a run asks for it. Under the `dlss` feature availability comes from
/// Bevy's `DlssSuperResolutionSupported`; without it availability stays
/// false and the reason names the missing SDK build.
#[cfg(feature = "dlss")]
fn probe_dlss(
    engine: NonSend<Engine>,
    device: Option<Res<RenderDevice>>,
    adapter: Option<Res<RenderAdapter>>,
    supported: Option<Res<bevy::anti_alias::dlss::DlssSuperResolutionSupported>>,
    mut scaling: ResMut<Scaling>,
    mut warned: Local<bool>,
) {
    let was_available = scaling.dlss_available;
    scaling.dlss_available = supported.is_some();
    if scaling.dlss_available {
        scaling.dlss_reason.clear();
    } else if scaling.dlss_reason.is_empty() || was_available {
        if cfg!(target_arch = "wasm32") {
            scaling.dlss_reason = dlss_unavailable(true, None, "", true).unwrap_or_default();
        } else if let Some(adapter) = adapter.as_deref() {
            let info = adapter.get_info();
            scaling.dlss_reason =
                dlss_unavailable(false, Some(info.vendor), &info.name, true).unwrap_or_default();
        } else if device.is_some() {
            scaling.dlss_reason = dlss_unavailable(false, None, "", true).unwrap_or_default();
        } else {
            scaling.dlss_available = was_available;
            return;
        }
    }
    warn_if_dlss_off(&engine, &scaling, &mut warned);
}

/// Probes DLSS capability once the renderer is up, and says why once when
/// a run asks for it. Availability itself stays false until some build
/// vendors the SDK; the probe only makes the reason adapter-aware.
#[cfg(not(feature = "dlss"))]
fn probe_dlss(
    engine: NonSend<Engine>,
    device: Option<Res<RenderDevice>>,
    adapter: Option<Res<RenderAdapter>>,
    mut scaling: ResMut<Scaling>,
    mut warned: Local<bool>,
) {
    if scaling.dlss_reason.is_empty() {
        if cfg!(target_arch = "wasm32") {
            scaling.dlss_reason = dlss_unavailable(true, None, "", false).unwrap_or_default();
        } else if let Some(adapter) = adapter.as_deref() {
            let info = adapter.get_info();
            scaling.dlss_reason =
                dlss_unavailable(false, Some(info.vendor), &info.name, false).unwrap_or_default();
        } else if device.is_some() {
            scaling.dlss_reason = dlss_unavailable(false, None, "", false).unwrap_or_default();
        } else {
            return;
        }
    }
    warn_if_dlss_off(&engine, &scaling, &mut warned);
}

fn warn_if_dlss_off(engine: &Engine, scaling: &Scaling, warned: &mut bool) {
    if engine.running
        && scaling.settings.upscaler == Upscaler::Dlss
        && !scaling.dlss_available
        && !*warned
    {
        *warned = true;
        let text = format!(
            "DLSS is off: {}. The built-in upscaler carries on (TAA plus spatial in 3D, spatial in 2D and on web).",
            scaling.dlss_reason
        );
        warn!("{text}");
        crate::bridge::send(&blockloom_protocol::RuntimeMessage::Say {
            actor: "Blockloom".into(),
            text,
        });
    }
}

/// Real DLSS path: ray reconstruction while Hybrid Solari lights the view
/// (it denoises Solari's output where Bevy exposes it), plain super
/// resolution otherwise, and TAA plus spatial when neither can run. The
/// mode rides the dynamic-resolution signal; DLSS drives its own render
/// resolution through `MainPassResolutionOverride`, so the spatial
/// `scale_views` stands down while it is on (see below).
#[cfg(feature = "dlss")]
fn configure_cameras(
    mut commands: Commands,
    engine: NonSend<Engine>,
    scaling: Res<Scaling>,
    tracing: Option<Res<crate::ray_tracing::RayTracingState>>,
    rr_supported: Option<Res<bevy::anti_alias::dlss::DlssRayReconstructionSupported>>,
    environment: Res<crate::environment::Environment>,
    cameras: Query<
        (
            Entity,
            Option<&TemporalAntiAliasing>,
            Option<&bevy::anti_alias::dlss::Dlss>,
            Option<
                &bevy::anti_alias::dlss::Dlss<bevy::anti_alias::dlss::DlssRayReconstructionFeature>,
            >,
            &mut Msaa,
            Has<bevy::camera::Hdr>,
            Option<&Projection>,
        ),
        (With<WorldCamera>, With<Camera3d>),
    >,
) {
    use bevy::anti_alias::dlss::{Dlss, DlssRayReconstructionFeature, DlssSuperResolutionFeature};
    let hybrid = tracing.as_ref().is_some_and(|state| state.active)
        && engine.project.world.lighting.ray_tracing.enabled
        && engine.project.world.lighting.ray_tracing.mode
            == blockloom_core::scene::TracingMode::Hybrid;
    let rr = rr_supported.is_some();
    let mode = dlss_perf_mode(dlss_effective_mode(
        &scaling.settings,
        scaling.controller.scale,
    ));
    for (entity, taa, sr, reconstruction, mut msaa, hdr, projection) in cameras {
        let perspective = projection.is_none_or(|p| matches!(p, Projection::Perspective(_)));
        let reconstruct = wants_dlss_rr(
            scaling.settings.upscaler,
            scaling.dlss_available,
            rr,
            hybrid,
            hdr,
            perspective,
        );
        let upscale = !reconstruct
            && wants_dlss(
                scaling.settings.upscaler,
                scaling.dlss_available,
                hdr,
                perspective,
            );
        if reconstruct || upscale {
            *msaa = Msaa::Off;
            // `Dlss` requires jitter, mip bias, depth and motion-vector
            // prepasses plus HDR; its `#[require]` adds what is missing.
            if taa.is_some() {
                commands.entity(entity).remove::<TemporalAntiAliasing>();
            }
        }
        if reconstruct {
            if sr.is_some() {
                commands
                    .entity(entity)
                    .remove::<Dlss<DlssSuperResolutionFeature>>();
            }
            let stale = reconstruction.is_none_or(|live| live.perf_quality_mode != mode);
            if stale {
                commands
                    .entity(entity)
                    .insert(Dlss::<DlssRayReconstructionFeature> {
                        perf_quality_mode: mode,
                        reset: false,
                        ..default()
                    });
            }
            continue;
        }
        if upscale {
            if reconstruction.is_some() {
                commands
                    .entity(entity)
                    .remove::<Dlss<DlssRayReconstructionFeature>>();
            }
            let stale = sr.is_none_or(|live| live.perf_quality_mode != mode);
            if stale {
                commands
                    .entity(entity)
                    .insert(Dlss::<DlssSuperResolutionFeature> {
                        perf_quality_mode: mode,
                        reset: false,
                        ..default()
                    });
            }
            continue;
        }
        if sr.is_some() {
            commands.entity(entity).remove::<Dlss>();
        }
        if reconstruction.is_some() {
            commands
                .entity(entity)
                .remove::<Dlss<DlssRayReconstructionFeature>>();
        }
        let temporal = temporal_for(scaling.settings.upscaler, dlss_spatial_only());
        if temporal {
            *msaa = Msaa::Off;
            if taa.is_none() {
                commands
                    .entity(entity)
                    .insert(TemporalAntiAliasing::default());
            }
        } else if taa.is_some() {
            *msaa = if environment.wants_msaa_off() {
                Msaa::Off
            } else {
                Msaa::default()
            };
            commands.entity(entity).remove::<TemporalAntiAliasing>();
            commands
                .entity(entity)
                .remove::<bevy::render::camera::TemporalJitter>();
        }
    }
}

#[cfg(not(feature = "dlss"))]
fn configure_cameras(
    mut commands: Commands,
    scaling: Res<Scaling>,
    environment: Res<crate::environment::Environment>,
    cameras: Query<
        (Entity, Option<&TemporalAntiAliasing>, &mut Msaa),
        (With<WorldCamera>, With<Camera3d>),
    >,
) {
    let temporal = temporal_for(scaling.settings.upscaler, dlss_spatial_only());
    for (entity, taa, mut msaa) in cameras {
        if temporal {
            *msaa = Msaa::Off;
            if taa.is_none() {
                commands
                    .entity(entity)
                    .insert(TemporalAntiAliasing::default());
            }
        } else if taa.is_some() {
            *msaa = if environment.wants_msaa_off() {
                Msaa::Off
            } else {
                Msaa::default()
            };
            commands.entity(entity).remove::<TemporalAntiAliasing>();
            commands
                .entity(entity)
                .remove::<bevy::render::camera::TemporalJitter>();
        }
    }
}

/// Scale working targets, leaving the output attachment and input coordinates
/// native. Bevy's final spatial blit fills that attachment in both dimensions.
/// While real DLSS runs it drives its own render resolution through
/// `MainPassResolutionOverride`, so this stands down rather than scaling
/// twice.
fn scale_views(
    scaling: Extract<Res<Scaling>>,
    worlds: Extract<Query<RenderEntity, With<WorldCamera>>>,
    mut views: Query<(&mut ExtractedCamera, &mut ExtractedView)>,
) {
    if dlss_drives_resolution(scaling.settings.upscaler, scaling.dlss_available) {
        return;
    }
    let scale = scaling.controller.scale;
    if scale >= 1.0 {
        return;
    }
    for entity in &worlds {
        let Ok((mut camera, mut view)) = views.get_mut(entity) else {
            continue;
        };
        // Sub-viewports need an output rectangle separate from their work size.
        if camera.viewport.is_some() {
            continue;
        }
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        let size = (size.as_vec2() * scale).round().as_uvec2().max(UVec2::ONE);
        camera.physical_target_size = Some(size);
        camera.physical_viewport_size = Some(size);
        view.viewport.z = size.x;
        view.viewport.w = size.y;
    }
}

fn apply_effects(mut scaling: ResMut<Scaling>, effects: Res<crate::engine::PendingEffects>) {
    for effect in &effects.0 {
        if let blockloom_core::vm::Effect::SetRenderSetting { setting, value } = effect {
            let mut settings = scaling.settings.clone();
            if let Err(error) = settings.set(*setting, value) {
                tracing::warn!("{error}");
                continue;
            }
            scaling.overrides.retain(|(key, _)| key != setting);
            scaling.overrides.push((*setting, value.clone()));
            scaling.controller = Controller::new(&settings);
            scaling.geometry = GeometryController::default();
            scaling.settings = settings;
        }
    }
}
fn fire_drop(mut engine: NonSendMut<Engine>, mut scaling: ResMut<Scaling>) {
    if engine.running && !engine.paused && scaling.dropped {
        scaling.dropped = false;
        engine.fire(blockloom_core::vm::Event::QualityDropped);
    }
}

/// Visible mesh/material groups, excluding shadow and post passes. Transparent
/// and custom materials count separately because their ordering can split draws.
pub(crate) fn measure_draws(
    mut every: Local<u32>,
    mut triangles_by_mesh: Local<std::collections::HashMap<AssetId<Mesh>, u64>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut scaling: ResMut<Scaling>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    cameras: Query<(&Camera, &VisibleEntities), With<WorldCamera>>,
    query: Query<(
        Entity,
        &Mesh3d,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<crate::batching::InstancedMaterial>>,
        Has<crate::terrain::TerrainChunk>,
        Has<crate::terrain::vegetation::GrassCell>,
        Has<crate::terrain::vegetation::ScatterInstance>,
        Has<MeshMaterial3d<crate::vfx::ParticleMaterial>>,
        Has<crate::destruction::Shard>,
        Has<crate::water::WaterSurface>,
    )>,
    parents: Query<&ChildOf>,
    scatter_roots: Query<(), With<crate::terrain::vegetation::ScatterInstance>>,
    shard_roots: Query<(), With<crate::destruction::Shard>>,
    water_roots: Query<(), With<crate::water::WaterSurface>>,
) {
    // Capture counts before extraction moves render-only vertex data to the GPU.
    for event in mesh_events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => {
                if let Some(count) = meshes.get(*id).and_then(triangle_count) {
                    triangles_by_mesh.insert(*id, count);
                }
            }
            AssetEvent::Removed { id } => {
                triangles_by_mesh.remove(id);
            }
            AssetEvent::Unused { .. } => {}
        }
    }
    *every = (*every + 1) % 30;
    if *every != 1 {
        return;
    }
    let mut groups = std::collections::HashMap::new();
    let mut triangles = 0;
    let mut meshes_seen = 0u32;
    let mut costs = [DrawCost::default(); 6];
    let visible = cameras
        .iter()
        .find(|(camera, _)| camera.is_active)
        .map(|(_, visible)| visible.get(std::any::TypeId::of::<Mesh3d>()))
        .unwrap_or(&[]);
    for &entity in visible {
        let Ok((entity, mesh, standard, instanced, terrain, grass, scatter, vfx, debris, water)) =
            query.get(entity)
        else {
            continue;
        };
        let Some(geometry) = meshes.get(&mesh.0) else {
            continue;
        };
        let mut ancestor = entity;
        let mut vegetation = grass || scatter;
        let mut debris = debris;
        let mut water = water;
        while !vegetation
            && !debris
            && !water
            && let Ok(parent) = parents.get(ancestor)
        {
            ancestor = parent.parent();
            vegetation |= scatter_roots.contains(ancestor);
            debris |= shard_roots.contains(ancestor);
            water |= water_roots.contains(ancestor);
        }
        let system = if terrain {
            0
        } else if vegetation {
            1
        } else if vfx {
            3
        } else if debris {
            4
        } else if water {
            5
        } else {
            2
        };
        let mesh_triangles = triangle_count(geometry)
            .or_else(|| triangles_by_mesh.get(&mesh.0.id()).copied())
            .unwrap_or(0);
        triangles += mesh_triangles;
        costs[system].triangles += mesh_triangles;
        let key = if let Some(material) = instanced {
            (mesh.0.id(), Some(material.0.id().untyped()), None)
        } else if let Some(material) = standard.filter(|m| {
            materials
                .get(&m.0)
                .is_some_and(|m| matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)))
        }) {
            (mesh.0.id(), Some(material.0.id().untyped()), None)
        } else {
            (mesh.0.id(), None, Some(entity))
        };
        let entry = groups.entry((system, key)).or_insert(0u32);
        if *entry == 0 {
            costs[system].draws += 1;
        }
        *entry += 1;
        meshes_seen += 1;
    }
    scaling.draw_calls = groups.len() as u32;
    scaling.instanced_draws = groups.values().filter(|&&n| n > 1).count() as u32;
    scaling.batched_instances = meshes_seen.saturating_sub(groups.len() as u32);
    scaling.triangles = triangles;
    scaling.costs = costs;
}

fn triangle_count(mesh: &Mesh) -> Option<u64> {
    let vertices = match mesh.try_indices_option().ok()? {
        Some(indices) => indices.len(),
        None => mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()?.len(),
    };
    Some(vertices as u64 / 3)
}

fn reset(engine: NonSend<Engine>, mut scaling: ResMut<Scaling>) {
    if engine.rebuild {
        let mut settings = engine.project.world.quality.clone();
        settings.normalize();
        scaling.controller = Controller::new(&settings);
        scaling.geometry = GeometryController::default();
        scaling.settings = settings;
        scaling.overrides.clear();
        scaling.dropped = false;
        scaling.costs = [DrawCost::default(); 6];
        scaling.draw_calls = 0;
        scaling.triangles = 0;
        scaling.instanced_draws = 0;
        scaling.batched_instances = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::Indices;
    use bevy::render::render_resource::PrimitiveTopology;
    use blockloom_core::quality::{Quality, Setting};
    use blockloom_core::vm::Effect;

    #[test]
    fn native_ui_tracks_output_and_returns_to_the_world_for_hdr_or_native_scale() {
        for three_d in [false, true] {
            let mut app = App::new();
            app.init_resource::<Scaling>()
                .init_resource::<crate::hdr::HdrFrame>()
                .add_systems(Update, configure_ui_camera);
            app.world_mut().resource_mut::<Scaling>().controller.scale = 0.5;
            app.world_mut().spawn((Node::default(), crate::ui::UiRoot));
            let world = app.world_mut().spawn(WorldCamera).id();
            if three_d {
                app.world_mut()
                    .entity_mut(world)
                    .insert(Camera3d::default());
            } else {
                app.world_mut().entity_mut(world).insert(Camera2d);
            }
            app.update();
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<NativeUiCamera>>()
                    .iter(app.world())
                    .count(),
                0
            );
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_some());
            let root = app.world_mut().spawn(Node::default()).id();
            app.update();
            let overlay = app
                .world_mut()
                .query_filtered::<Entity, With<NativeUiCamera>>()
                .single(app.world())
                .unwrap();
            assert!(app.world().get::<IsDefaultUiCamera>(overlay).is_some());
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_none());
            assert_eq!(
                app.world().get::<RenderLayers>(overlay).unwrap(),
                &RenderLayers::none()
            );
            let camera = app.world().get::<Camera>(overlay).unwrap();
            assert!(matches!(
                camera.output_mode,
                CameraOutputMode::Write {
                    blend_state: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    clear_color: ClearColorConfig::None,
                }
            ));

            let target = RenderTarget::TextureView(bevy::camera::ManualTextureViewHandle(42));
            app.world_mut().entity_mut(world).insert(target.clone());
            app.world_mut().get_mut::<Camera>(world).unwrap().order = 10;
            app.update();
            assert_eq!(
                app.world()
                    .get::<RenderTarget>(overlay)
                    .unwrap()
                    .normalize(None),
                target.normalize(None),
            );
            assert_eq!(app.world().get::<Camera>(overlay).unwrap().order, 11);
            app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
            app.update();
            assert!(!app.world().get::<Camera>(overlay).unwrap().is_active);
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_some());
            app.world_mut().get_mut::<Node>(root).unwrap().display = Display::Flex;
            app.update();
            assert!(app.world().get::<Camera>(overlay).unwrap().is_active);

            use blockloom_core::scene::{DisplayOutput, OutputSpace};
            let hdr = DisplayOutput {
                space: OutputSpace::Hdr10,
                ..default()
            };
            app.insert_resource(crate::hdr::HdrFrame::resolve(
                hdr,
                Some(&[OutputSpace::Hdr10]),
                crate::hdr::HdrPolicy::default(),
            ));
            app.update();
            assert!(!app.world().get::<Camera>(overlay).unwrap().is_active);
            assert!(app.world().get::<IsDefaultUiCamera>(overlay).is_none());
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_some());

            app.insert_resource(crate::hdr::HdrFrame::default());
            app.update();
            assert!(app.world().get::<Camera>(overlay).unwrap().is_active);
            app.world_mut().get_mut::<Camera>(world).unwrap().viewport =
                Some(bevy::camera::Viewport::default());
            app.update();
            assert!(!app.world().get::<Camera>(overlay).unwrap().is_active);
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_some());
            app.world_mut().get_mut::<Camera>(world).unwrap().viewport = None;
            app.world_mut().resource_mut::<Scaling>().controller.scale = 1.0;
            app.update();
            assert!(!app.world().get::<Camera>(overlay).unwrap().is_active);
            assert!(app.world().get::<IsDefaultUiCamera>(world).is_some());

            app.world_mut().despawn(world);
            app.update();
            assert!(app.world().get_entity(overlay).is_err());
        }
    }

    #[test]
    fn dlss_probe_names_the_adapter_and_sdk_build() {
        assert_eq!(
            dlss_unavailable(true, None, "", false).as_deref(),
            Some("DLSS is off on web builds")
        );
        assert_eq!(
            dlss_unavailable(true, None, "", true).as_deref(),
            Some("DLSS is off on web builds")
        );
        // Without the SDK build every branch names it.
        let nvidia =
            dlss_unavailable(false, Some(NVIDIA_VENDOR), "NVIDIA GeForce RTX 4070", false).unwrap();
        assert!(nvidia.contains("NVIDIA GeForce RTX 4070"));
        assert!(nvidia.contains("SDK"));
        let other = dlss_unavailable(false, Some(0x8086), "Intel Arc A770", false).unwrap();
        assert!(other.contains("Intel Arc A770"));
        assert!(other.contains("no DLSS driver support"));
        assert!(
            dlss_unavailable(false, None, "", false)
                .unwrap()
                .contains("SDK")
        );
        // With the SDK the reason is the driver, DLL or adapter instead.
        let refused =
            dlss_unavailable(false, Some(NVIDIA_VENDOR), "NVIDIA GeForce RTX 4070", true).unwrap();
        assert!(refused.contains("NVIDIA GeForce RTX 4070"));
        assert!(!refused.contains("leaves the DLSS SDK out"));
        let no_rtx = dlss_unavailable(false, Some(0x8086), "Intel Arc A770", true).unwrap();
        assert!(no_rtx.contains("Intel Arc A770"));
        assert!(no_rtx.contains("needs NVIDIA RTX"));
    }

    #[test]
    fn dlss_wants_the_component_only_when_everything_lines_up() {
        use blockloom_core::quality::Upscaler;
        // The ask alone is never enough.
        assert!(!wants_dlss(Upscaler::Spatial, true, true, true));
        assert!(!wants_dlss(Upscaler::Taa, true, true, true));
        assert!(!wants_dlss(Upscaler::Dlss, false, true, true));
        assert!(!wants_dlss(Upscaler::Dlss, true, false, true));
        assert!(!wants_dlss(Upscaler::Dlss, true, true, false));
        // With everything lined up the answer follows the build flag, so a
        // non-SDK build keeps the TAA fallback even on ideal hardware.
        assert_eq!(wants_dlss(Upscaler::Dlss, true, true, true), DLSS_BUILD);
        // Resolution follows the same flag: DLSS drives it, otherwise spatial.
        assert!(!dlss_drives_resolution(Upscaler::Spatial, true));
        assert!(!dlss_drives_resolution(Upscaler::Taa, true));
        assert!(!dlss_drives_resolution(Upscaler::Dlss, false));
        assert_eq!(dlss_drives_resolution(Upscaler::Dlss, true), DLSS_BUILD);
    }

    #[cfg(feature = "dlss")]
    #[test]
    fn dlss_modes_map_one_to_one() {
        use bevy::anti_alias::dlss::DlssPerfQualityMode as Bevy;
        use blockloom_core::quality::DlssMode;
        assert!(matches!(dlss_perf_mode(DlssMode::Auto), Bevy::Auto));
        assert!(matches!(dlss_perf_mode(DlssMode::Dlaa), Bevy::Dlaa));
        assert!(matches!(dlss_perf_mode(DlssMode::Quality), Bevy::Quality));
        assert!(matches!(dlss_perf_mode(DlssMode::Balanced), Bevy::Balanced));
        assert!(matches!(
            dlss_perf_mode(DlssMode::Performance),
            Bevy::Performance
        ));
        assert!(matches!(
            dlss_perf_mode(DlssMode::UltraPerformance),
            Bevy::UltraPerformance
        ));
    }

    #[test]
    fn dlss_effective_mode_steps_down_with_the_signal() {
        use blockloom_core::quality::{DlssMode as M, Settings, Upscaler};
        let mut settings = Settings::default();
        // Not a DLSS ask, or no signal: the ask is the answer.
        assert_eq!(dlss_effective_mode(&settings, 0.5), M::Auto);
        settings.upscaler = Upscaler::Dlss;
        assert_eq!(dlss_effective_mode(&settings, 0.5), M::Auto);
        settings.dlss_mode = M::Quality;
        assert_eq!(dlss_effective_mode(&settings, 1.0), M::Quality);
        // Dynamic resolution off: the ask is the answer at any scale.
        assert_eq!(dlss_effective_mode(&settings, 0.5), M::Quality);
        settings.dynamic_resolution = true;
        // Each 0.05 below the 1.0 ceiling is one step down.
        assert_eq!(dlss_effective_mode(&settings, 1.0), M::Quality);
        assert_eq!(dlss_effective_mode(&settings, 0.95), M::Balanced);
        assert_eq!(dlss_effective_mode(&settings, 0.9), M::Performance);
        assert_eq!(dlss_effective_mode(&settings, 0.85), M::UltraPerformance);
        // Floored at the cheapest, and clamped to the span.
        assert_eq!(dlss_effective_mode(&settings, 0.5), M::UltraPerformance);
        assert_eq!(dlss_effective_mode(&settings, 0.25), M::UltraPerformance);
        settings.dlss_mode = M::Dlaa;
        assert_eq!(dlss_effective_mode(&settings, 1.0), M::Dlaa);
        assert_eq!(dlss_effective_mode(&settings, 0.95), M::Quality);
        // A lower ceiling steps from there: 0.75 at a 0.5 floor.
        settings.dlss_mode = M::Balanced;
        settings.resolution_scale = 0.75;
        settings.min_scale = 0.5;
        assert_eq!(dlss_effective_mode(&settings, 0.75), M::Balanced);
        assert_eq!(dlss_effective_mode(&settings, 0.7), M::Performance);
        assert_eq!(dlss_effective_mode(&settings, 0.65), M::UltraPerformance);
    }

    #[test]
    fn dlss_reconstruction_wants_hybrid_and_both_probes() {
        use blockloom_core::quality::Upscaler;
        // Anything missing is plain super resolution at best.
        assert!(!wants_dlss_rr(
            Upscaler::Spatial,
            true,
            true,
            true,
            true,
            true
        ));
        assert!(!wants_dlss_rr(
            Upscaler::Dlss,
            false,
            true,
            true,
            true,
            true
        ));
        assert!(!wants_dlss_rr(
            Upscaler::Dlss,
            true,
            false,
            true,
            true,
            true
        ));
        assert!(!wants_dlss_rr(
            Upscaler::Dlss,
            true,
            true,
            false,
            true,
            true
        ));
        assert!(!wants_dlss_rr(
            Upscaler::Dlss,
            true,
            true,
            true,
            false,
            true
        ));
        assert!(!wants_dlss_rr(
            Upscaler::Dlss,
            true,
            true,
            true,
            true,
            false
        ));
        // Everything lined up follows the build flag.
        assert_eq!(
            wants_dlss_rr(Upscaler::Dlss, true, true, true, true, true),
            DLSS_BUILD
        );
    }

    #[test]
    fn upscaler_fallback_keeps_taa_off_spatial_and_web_dlss() {
        use blockloom_core::quality::Upscaler;
        assert!(!temporal_for(Upscaler::Spatial, false));
        assert!(!temporal_for(Upscaler::Spatial, true));
        assert!(temporal_for(Upscaler::Taa, false));
        assert!(temporal_for(Upscaler::Taa, true));
        assert!(temporal_for(Upscaler::Dlss, false));
        assert!(!temporal_for(Upscaler::Dlss, true));
    }

    #[test]
    fn sampled_dlss_flag_follows_the_probed_state() {
        let mut scaling = Scaling::default();
        assert!(!scaling.sample().dlss_available);
        scaling.dlss_available = true;
        assert!(scaling.sample().dlss_available);
    }

    #[test]
    fn dlss_selection_rides_taa_while_spatial_removes_it() {
        use blockloom_core::quality::Upscaler;
        let (_, incoming) = std::sync::mpsc::channel();
        let engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        let mut app = App::new();
        // The DLSS build reads the engine for its Hybrid check; without the
        // feature this resource just sits unused.
        app.insert_non_send(engine)
            .init_resource::<Scaling>()
            .init_resource::<crate::environment::Environment>()
            .add_systems(Update, configure_cameras);
        app.world_mut().resource_mut::<Scaling>().settings.upscaler = Upscaler::Dlss;
        let camera = app
            .world_mut()
            .spawn((WorldCamera, Camera3d::default(), Msaa::default()))
            .id();
        app.update();
        assert!(app.world().get::<TemporalAntiAliasing>(camera).is_some());
        app.world_mut().resource_mut::<Scaling>().settings.upscaler = Upscaler::Spatial;
        app.update();
        assert!(app.world().get::<TemporalAntiAliasing>(camera).is_none());
    }

    /// Ray reconstruction lands on a Hybrid-lit HDR camera while both SDK
    /// probes agree, yields to plain super resolution when its probe goes,
    /// and leaves entirely for the TAA fallback with any other upscaler.
    #[cfg(feature = "dlss")]
    #[test]
    fn dlss_reconstruction_lands_on_a_hybrid_camera() {
        use bevy::anti_alias::dlss::{
            Dlss, DlssPerfQualityMode, DlssRayReconstructionFeature,
            DlssRayReconstructionSupported, DlssSuperResolutionFeature,
        };
        use blockloom_core::quality::{DlssMode, Upscaler};
        use blockloom_core::scene::TracingMode;
        let (_, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        engine.project.world.lighting.ray_tracing.enabled = true;
        engine.project.world.lighting.ray_tracing.mode = TracingMode::Hybrid;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Scaling>()
            .init_resource::<crate::environment::Environment>()
            .init_resource::<crate::ray_tracing::RayTracingState>()
            .insert_resource(DlssRayReconstructionSupported)
            .add_systems(Update, configure_cameras);
        app.world_mut()
            .resource_mut::<crate::ray_tracing::RayTracingState>()
            .active = true;
        let mut scaling = app.world_mut().resource_mut::<Scaling>();
        scaling.settings.upscaler = Upscaler::Dlss;
        scaling.settings.dlss_mode = DlssMode::Quality;
        scaling.dlss_available = true;
        let camera = app
            .world_mut()
            .spawn((
                WorldCamera,
                Camera3d::default(),
                Msaa::default(),
                bevy::camera::Hdr,
                bevy::camera::Projection::Perspective(
                    bevy::camera::PerspectiveProjection::default(),
                ),
            ))
            .id();
        app.update();
        let reconstruction = app
            .world()
            .get::<Dlss<DlssRayReconstructionFeature>>(camera)
            .unwrap();
        assert!(matches!(
            reconstruction.perf_quality_mode,
            DlssPerfQualityMode::Quality
        ));
        assert!(
            app.world()
                .get::<Dlss<DlssSuperResolutionFeature>>(camera)
                .is_none()
        );
        assert!(app.world().get::<TemporalAntiAliasing>(camera).is_none());
        // Without the reconstruction probe it yields to super resolution.
        app.world_mut()
            .remove_resource::<DlssRayReconstructionSupported>();
        app.update();
        assert!(
            app.world()
                .get::<Dlss<DlssRayReconstructionFeature>>(camera)
                .is_none()
        );
        assert!(
            app.world()
                .get::<Dlss<DlssSuperResolutionFeature>>(camera)
                .is_some()
        );
        // Any other upscaler leaves DLSS entirely for the TAA fallback.
        app.world_mut().resource_mut::<Scaling>().settings.upscaler = Upscaler::Taa;
        app.update();
        assert!(
            app.world()
                .get::<Dlss<DlssSuperResolutionFeature>>(camera)
                .is_none()
        );
        assert!(app.world().get::<TemporalAntiAliasing>(camera).is_some());
    }

    fn show_meshes(app: &mut App) {
        let entities: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(app.world())
            .collect();
        let mut visible = VisibleEntities::default();
        visible
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .extend(entities);
        app.world_mut()
            .spawn((WorldCamera, Camera::default(), visible));
    }

    #[test]
    fn local_feedback_waits_while_paused_and_preserves_pixel_rate() {
        for system in [1, 2, 5] {
            let (_, incoming) = std::sync::mpsc::channel();
            let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
            engine.rebuild = false;
            engine.paused = true;
            engine.project.world.quality.auto_drop = true;
            engine.project.world.quality.dynamic_resolution = true;
            engine.project.world.quality.over_budget_frames = 15;
            let mut app = App::new();
            app.insert_non_send(engine)
                .init_resource::<Scaling>()
                .init_resource::<crate::performance::LoopPace>()
                .add_systems(Update, feedback);
            app.world_mut()
                .resource_mut::<crate::performance::LoopPace>()
                .update_ms = 5.0;
            app.world_mut().resource_mut::<Scaling>().costs[system].draws = 10_000;
            for _ in 0..30 {
                app.update();
            }
            assert_eq!(app.world().resource::<Scaling>().geometry.factors, [1.0; 6]);
            app.world_mut().non_send_mut::<Engine>().paused = false;
            for _ in 0..15 {
                app.update();
            }
            let scaling = app.world().resource::<Scaling>();
            let mut expected = [1.0; 6];
            expected[system] = 0.9;
            assert_eq!(scaling.geometry.factors, expected);
            assert_eq!(scaling.controller.quality, Quality::High);
            assert_eq!(scaling.controller.scale, 1.0);
        }
    }

    #[test]
    fn transient_limits_combine_authored_caps_presets_and_local_density() {
        let mut scaling = Scaling::default();
        scaling.geometry.factors[3] = 0.5;
        assert_eq!(scaling.particle_budget(100), 50);
        assert_eq!(scaling.particle_budget(u32::MAX), 100_000);
        assert_eq!(scaling.shard_budget(256), 256);
        scaling.geometry.factors[4] = 0.25;
        assert_eq!(scaling.shard_budget(100), 25);
        assert_eq!(scaling.shard_budget(usize::MAX), 64);
        scaling.controller.quality = Quality::Low;
        assert_eq!(scaling.particle_budget(u32::MAX), 12_500);
        assert_eq!(scaling.shard_budget(usize::MAX), 16);
        assert_eq!(scaling.particle_budget(0), 0);
        assert_eq!(scaling.shard_budget(0), 0);
    }

    #[test]
    fn water_detail_combines_the_preset_and_its_local_throttle() {
        let mut scaling = Scaling::default();
        assert_eq!(scaling.water_detail(), 1.0);
        scaling.geometry.factors[5] = 0.25;
        assert_eq!(scaling.water_detail(), 0.25);
        scaling.controller.quality = Quality::Low;
        assert_eq!(scaling.water_detail(), 0.0625);
    }

    #[test]
    fn water_floors_count_toward_water_and_keep_counts_after_upload() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let water = app
            .world_mut()
            .spawn((
                crate::water::WaterSurface,
                Mesh3d(mesh.clone()),
                ViewVisibility::VISIBLE,
            ))
            .id();
        let holder = app.world_mut().spawn(ChildOf(water)).id();
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            ViewVisibility::VISIBLE,
            ChildOf(holder),
        ));
        app.world_mut()
            .spawn((Mesh3d(mesh.clone()), ViewVisibility::VISIBLE));
        show_meshes(&mut app);
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Added { id: mesh.id() });
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert_eq!((scaling.costs[5].draws, scaling.costs[5].triangles), (2, 4));
        assert_eq!((scaling.costs[2].draws, scaling.costs[2].triangles), (1, 2));

        let _gpu_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .get_mut(&mesh)
            .unwrap()
            .take_gpu_data()
            .unwrap();
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Modified { id: mesh.id() });
        for _ in 0..30 {
            app.update();
        }
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.costs[5].triangles, 4);
        assert_eq!(scaling.costs[2].triangles, 2);
    }

    #[test]
    fn explicit_quality_restores_an_automatically_lowered_preset() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .insert_resource(crate::engine::PendingEffects(vec![
                Effect::SetRenderSetting {
                    setting: Setting::Quality,
                    value: "High".into(),
                },
            ]))
            .add_systems(Update, apply_effects);
        app.world_mut().resource_mut::<Scaling>().controller.quality = Quality::Low;
        app.world_mut().resource_mut::<Scaling>().geometry.factors = [0.25; 6];
        app.update();
        assert_eq!(
            app.world().resource::<Scaling>().controller.quality,
            Quality::High
        );
        assert_eq!(app.world().resource::<Scaling>().geometry.factors, [1.0; 6]);
    }

    #[test]
    fn rebuild_discards_run_overrides() {
        let (_, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        engine.rebuild = true;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Scaling>()
            .add_systems(Update, reset);
        {
            let mut scaling = app.world_mut().resource_mut::<Scaling>();
            scaling.overrides.push((Setting::Quality, "Low".into()));
            scaling.controller.quality = Quality::Low;
            scaling.geometry.factors = [0.25; 6];
            scaling.dropped = true;
            scaling.costs[2].draws = 10_000;
            scaling.draw_calls = 10_000;
            scaling.triangles = 100_000;
            scaling.instanced_draws = 10;
            scaling.batched_instances = 100;
        }
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert!(scaling.overrides.is_empty());
        assert!(!scaling.dropped);
        assert_eq!(scaling.controller.quality, Quality::High);
        assert_eq!(scaling.geometry.factors, [1.0; 6]);
        assert_eq!(scaling.costs[2].draws, 0);
        assert_eq!((scaling.draw_calls, scaling.triangles), (0, 0));
        assert_eq!((scaling.instanced_draws, scaling.batched_instances), (0, 0));
    }

    #[test]
    fn scattered_model_parts_keep_vegetation_costs_after_upload() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let root = app.world_mut().spawn_empty().id();
        let holder = app
            .world_mut()
            .spawn((
                crate::terrain::vegetation::ScatterInstance {
                    levels: Vec::new(),
                    parts: Vec::new(),
                },
                ChildOf(root),
            ))
            .id();
        let nested = app.world_mut().spawn(ChildOf(holder)).id();
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            ViewVisibility::VISIBLE,
            ChildOf(nested),
        ));
        app.world_mut()
            .spawn((Mesh3d(mesh.clone()), ViewVisibility::VISIBLE, ChildOf(root)));
        show_meshes(&mut app);
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Added { id: mesh.id() });
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.costs[1].draws, 1);
        assert_eq!(scaling.costs[1].triangles, 2);
        assert_eq!(scaling.costs[2].draws, 1);
        assert_eq!(scaling.costs[2].triangles, 2);

        let _gpu_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .get_mut(&mesh)
            .unwrap()
            .take_gpu_data()
            .unwrap();
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Modified { id: mesh.id() });
        for _ in 0..30 {
            app.update();
        }
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.costs[1].triangles, 2);
        assert_eq!(scaling.costs[2].triangles, 2);
    }

    #[test]
    fn draw_feedback_uses_the_active_world_view_instead_of_shadow_visibility() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let shown = app
            .world_mut()
            .spawn((Mesh3d(mesh.clone()), ViewVisibility::VISIBLE))
            .id();
        let shadow_only = app
            .world_mut()
            .spawn((Mesh3d(mesh), ViewVisibility::VISIBLE))
            .id();
        let mut inactive = VisibleEntities::default();
        inactive
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .push(shadow_only);
        app.world_mut().spawn((
            WorldCamera,
            Camera {
                is_active: false,
                ..default()
            },
            inactive,
        ));
        let mut visible = VisibleEntities::default();
        visible
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .push(shown);
        let camera = app
            .world_mut()
            .spawn((WorldCamera, Camera::default(), visible))
            .id();
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert_eq!((scaling.draw_calls, scaling.triangles), (1, 2));
        assert_eq!((scaling.costs[2].draws, scaling.costs[2].triangles), (1, 2));

        // A LOD/occlusion removal can leave ViewVisibility set by a shadow view.
        app.world_mut()
            .get_mut::<VisibleEntities>(camera)
            .unwrap()
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .clear();
        for _ in 0..30 {
            app.update();
        }
        let scaling = app.world().resource::<Scaling>();
        assert_eq!((scaling.draw_calls, scaling.triangles), (0, 0));
        assert_eq!(scaling.costs[2].draws, 0);
        assert!(app.world().get::<ViewVisibility>(shown).unwrap().get());
    }

    #[test]
    fn render_only_meshes_do_not_panic_the_draw_sampler() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 4])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 2, 3, 0]));
        assert_eq!(triangle_count(&mesh), Some(2));

        let _gpu_mesh = mesh.take_gpu_data().unwrap();
        assert_eq!(triangle_count(&mesh), None);
    }

    #[test]
    fn shared_meshes_count_one_draw_with_saved_instances() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..default()
            });
        for _ in 0..3 {
            app.world_mut().spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                ViewVisibility::VISIBLE,
            ));
        }
        show_meshes(&mut app);
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Added { id: mesh.id() });
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert_eq!((scaling.draw_calls, scaling.triangles), (1, 6));
        assert_eq!((scaling.costs[2].draws, scaling.costs[2].triangles), (1, 6));
        assert_eq!(scaling.instanced_draws, 1);
        assert_eq!(scaling.batched_instances, 2);
    }
}
