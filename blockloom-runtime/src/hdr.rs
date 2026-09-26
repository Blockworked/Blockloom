//! The HDR frame: every world camera renders linear FP16 from the sky through
//! the lights to post, and only the end of the chain turns that into display
//! values. On an SDR display that is the project's tonemapper, as always. On
//! an HDR one (see `display`) the tonemapper stands aside for a tone curve
//! with the display's headroom, and an encode pass after the UI writes scRGB
//! or PQ, so the HUD lands at paper white.
//!
//! This module also holds the Game view's exposure debug views, which read
//! the scene before tonemapping, and `HdrFrame`, the one answer to "what is
//! this frame going out as" that the camera, the render world and the
//! reporters share.

use crate::engine::{Engine, PendingEffects};
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::upscaling::upscaling;
use bevy::core_pipeline::{Core2d, Core2dSystems, Core3dSystems};
use bevy::ecs::schedule::{ScheduleConfigs, ScheduleLabel};
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use bevy::ui_render::ui_pass;
use blockloom_core::scene::{DisplayOutput, OutputSpace};
use blockloom_core::vm::Effect;
use blockloom_protocol::DebugView;
use std::sync::{Arc, Mutex};

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/hdr.wesl");
    app.init_resource::<HdrDebug>()
        .init_resource::<HdrFrame>()
        .init_resource::<DisplayOffers>()
        .add_plugins((
            ExtractResourcePlugin::<HdrFrame>::default(),
            FullscreenMaterialPlugin::<HdrDebugView3d>::default(),
            FullscreenMaterialPlugin::<HdrDebugView2d>::default(),
            FullscreenMaterialPlugin::<HdrTone3d>::default(),
            FullscreenMaterialPlugin::<HdrTone2d>::default(),
            FullscreenMaterialPlugin::<HdrEncode3d>::default(),
            FullscreenMaterialPlugin::<HdrEncode2d>::default(),
        ));
    if !app.world().contains_resource::<HdrPolicy>() {
        app.init_resource::<HdrPolicy>();
    }
    let offers = app.world().resource::<DisplayOffers>().clone();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.insert_resource(offers);
        // The extractor only carries this over once the first frame runs,
        // while the windowed surface takeover reads it in that same frame:
        // seed the default so frame one never finds it missing.
        render.init_resource::<HdrFrame>();
    }
}

/// What this runtime may do about HDR: a build made SDR-only never renders
/// FP16 or leaves SDR, and only a real window has a swapchain to take over.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HdrPolicy {
    pub allow: bool,
    pub windowed: bool,
}

impl Default for HdrPolicy {
    fn default() -> Self {
        Self {
            allow: true,
            windowed: false,
        }
    }
}

/// The output spaces the window's display takes, once the render world has
/// asked its surface. `None` until then, and forever with no window.
#[derive(Resource, Clone, Default)]
pub struct DisplayOffers(pub Arc<Mutex<Option<Vec<OutputSpace>>>>);

impl DisplayOffers {
    pub fn get(&self) -> Option<Vec<OutputSpace>> {
        self.0.lock().ok().and_then(|offers| offers.clone())
    }

    /// Records what the display offers. Only the native HDR surface takeover
    /// calls this, so web builds never do.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set(&self, offers: Vec<OutputSpace>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(offers);
        }
    }
}

/// What this frame goes out as. Both worlds decide from this one value, so
/// the surface and the passes change on the same frame.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct HdrFrame {
    /// The signal the window is configured in.
    pub space: OutputSpace,
    /// Whether the project asks for HDR, whatever the display offers.
    pub wanted: bool,
    pub peak_nits: f32,
    pub paper_white_nits: f32,
    /// Whether cameras render FP16 at all.
    pub fp16: bool,
}

impl Default for HdrFrame {
    fn default() -> Self {
        Self::resolve(DisplayOutput::default(), None, HdrPolicy::default())
    }
}

impl ExtractResource<RenderApp> for HdrFrame {
    type Source = HdrFrame;

    fn extract_resource(source: &Self::Source) -> Self {
        *source
    }
}

impl HdrFrame {
    /// The project's wish against what the display offers: the chosen HDR
    /// space, else the other one, else SDR.
    pub fn resolve(
        wanted: DisplayOutput,
        offers: Option<&[OutputSpace]>,
        policy: HdrPolicy,
    ) -> Self {
        let offered = |space| offers.is_some_and(|offers| offers.contains(&space));
        let space = match wanted.space {
            _ if !policy.allow => OutputSpace::Sdr,
            OutputSpace::Sdr => OutputSpace::Sdr,
            first => {
                let second = if first == OutputSpace::Hdr10 {
                    OutputSpace::Scrgb
                } else {
                    OutputSpace::Hdr10
                };
                [first, second]
                    .into_iter()
                    .find(|space| offered(*space))
                    .unwrap_or(OutputSpace::Sdr)
            }
        };
        Self {
            space,
            wanted: wanted.space.is_hdr() && policy.allow,
            peak_nits: wanted.peak_nits,
            paper_white_nits: wanted.paper_white_nits,
            fp16: policy.allow,
        }
    }

    pub fn is_hdr(&self) -> bool {
        self.space.is_hdr()
    }

    pub fn headroom(&self) -> f32 {
        (self.peak_nits / self.paper_white_nits.max(1.0)).max(1.0)
    }

    /// Exposed luminance the debug views call clipped: the display's
    /// headroom when the project asks for HDR, paper white otherwise.
    fn clip(&self) -> f32 {
        if self.wanted { self.headroom() } else { 1.0 }
    }

    fn pass(&self, mode: u32, white: f32) -> HdrPass {
        HdrPass {
            mode,
            white,
            headroom: self.headroom(),
            level: DisplayOutput::MAX_NITS / self.paper_white_nits.max(1.0),
        }
    }

    /// HDR10 static metadata for this frame, none when it goes out SDR. The
    /// tone curve never passes the peak, so that is MaxCLL too; a frame
    /// mostly sits at or under paper white, which stands in for MaxFALL.
    pub fn metadata(&self) -> Option<HdrMetadata> {
        let primaries = match self.space {
            OutputSpace::Sdr => return None,
            OutputSpace::Hdr10 => BT2020,
            OutputSpace::Scrgb => BT709,
        };
        let peak = self.peak_nits.max(1.0);
        Some(HdrMetadata {
            primaries,
            max_mastering_nits: peak,
            min_mastering_nits: 0.0001,
            max_cll: peak,
            max_fall: self.paper_white_nits.clamp(1.0, peak),
        })
    }

    /// Puts the tone curve and the encode on a world camera when the frame
    /// goes out HDR, and takes them off when it doesn't.
    pub fn apply(&self, camera: &mut EntityCommands, is_3d: bool) {
        camera.remove::<(HdrTone3d, HdrTone2d, HdrEncode3d, HdrEncode2d)>();
        let encode = match self.space {
            OutputSpace::Sdr => return,
            OutputSpace::Scrgb => 11,
            OutputSpace::Hdr10 => 12,
        };
        let tone = self.pass(10, 1.0);
        let encode = self.pass(encode, self.paper_white_nits);
        if is_3d {
            camera.insert((HdrTone3d::from(tone), HdrEncode3d::from(encode)));
        } else {
            camera.insert((HdrTone2d::from(tone), HdrEncode2d::from(encode)));
        }
    }
}

/// CIE 1931 xy of red, green, blue and the D65 white point.
const BT2020: [[f32; 2]; 4] = [
    [0.708, 0.292],
    [0.170, 0.797],
    [0.131, 0.046],
    [0.3127, 0.3290],
];
const BT709: [[f32; 2]; 4] = [
    [0.640, 0.330],
    [0.300, 0.600],
    [0.150, 0.060],
    [0.3127, 0.3290],
];

/// What an HDR signal was mastered for, sent beside the swapchain so a
/// display with less range tone maps it rather than clipping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrMetadata {
    /// Red, green, blue and white, as CIE 1931 xy.
    pub primaries: [[f32; 2]; 4],
    pub max_mastering_nits: f32,
    pub min_mastering_nits: f32,
    /// Brightest pixel (MaxCLL) and brightest frame average (MaxFALL).
    pub max_cll: f32,
    pub max_fall: f32,
}

/// The project's display settings with this run's `set HDR output` and
/// `set peak brightness` laid over them.
pub fn wanted_output(engine: &Engine) -> DisplayOutput {
    let mut display = engine.project.world.display;
    if let Some(nits) = engine.peak_nits {
        display.peak_nits = nits;
    }
    match engine.hdr_output {
        Some(false) => display.space = OutputSpace::Sdr,
        Some(true) if !display.space.is_hdr() => display.space = OutputSpace::Scrgb,
        _ => {}
    }
    display.normalize();
    display
}

/// Resolves this frame's output. Only a real change marks it changed.
pub fn resolve_frame(
    engine: NonSend<Engine>,
    offers: Res<DisplayOffers>,
    policy: Res<HdrPolicy>,
    mut frame: ResMut<HdrFrame>,
) {
    let offers = offers.get();
    frame.set_if_neq(HdrFrame::resolve(
        wanted_output(&engine),
        offers.as_deref(),
        *policy,
    ));
}

/// `set HDR output` and `set peak brightness` for the rest of the run.
pub fn apply_hdr_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetHdrOutput { enabled } => engine.hdr_output = Some(*enabled),
            Effect::SetPeakBrightness { nits } if nits.is_finite() => {
                engine.peak_nits = Some(*nits);
            }
            _ => {}
        }
    }
}

/// GPU time spent turning the HDR frame into display values: Bevy's
/// tonemapper, or the tone curve and encode that stand in for it.
pub fn tonemap_ms(metrics: &[blockloom_protocol::RenderMetric]) -> Option<f64> {
    let passes: Vec<f64> = metrics
        .iter()
        .filter(|metric| metric.name.ends_with("/elapsed_gpu"))
        .filter(|metric| {
            ["tonemapping", "HdrTone", "HdrEncode"]
                .iter()
                .any(|pass| metric.name.contains(pass))
        })
        .map(|metric| metric.value)
        .collect();
    (!passes.is_empty()).then(|| passes.iter().sum())
}

/// The debug view the editor asked for. An editor preference, re-sent with
/// the scene view to every world that comes up.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HdrDebug(pub DebugView);

impl HdrDebug {
    /// Views that replace the image outright, or apply their own curve,
    /// need the tonemapper out of the way or it would bend them.
    pub fn bypasses_tonemapping(self) -> bool {
        matches!(
            self.0,
            DebugView::FalseColor | DebugView::Calibration | DebugView::HdrPreview
        )
    }

    fn mode(self) -> Option<u32> {
        match self.0 {
            // The surface shaders draw the blend view themselves.
            DebugView::Lit | DebugView::SurfaceBlend => None,
            DebugView::FalseColor => Some(1),
            DebugView::Clipping => Some(2),
            DebugView::Histogram => Some(3),
            DebugView::Waveform => Some(4),
            DebugView::Calibration => Some(5),
            DebugView::HdrPreview => Some(6),
        }
    }

    /// Puts the matching pass on a world camera, or takes it off.
    pub fn apply(self, camera: &mut EntityCommands, is_3d: bool, frame: &HdrFrame) {
        camera.remove::<(HdrDebugView3d, HdrDebugView2d)>();
        let Some(mode) = self.mode() else {
            return;
        };
        let pass = frame.pass(mode, frame.clip());
        if is_3d {
            camera.insert(HdrDebugView3d::from(pass));
        } else {
            camera.insert(HdrDebugView2d::from(pass));
        }
    }
}

/// The uniform every HDR pass reads; the fields match the shader's
/// `HdrPass`.
#[derive(Clone, Copy, Default, Debug, PartialEq, ShaderType)]
pub struct HdrPass {
    mode: u32,
    /// Exposed luminance that counts as clipped, or paper white nits.
    white: f32,
    headroom: f32,
    level: f32,
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/hdr.wesl"))
            .with_source("embedded"),
    )
}

/// One material per pass and dimension, since a fullscreen material names
/// the one schedule and slot it runs in.
macro_rules! hdr_pass {
    ($name:ident) => {
        #[derive(
            Component, ExtractComponent, Clone, Copy, Default, Debug, PartialEq, ShaderType,
        )]
        #[extract_app(RenderApp)]
        pub struct $name {
            pass: HdrPass,
        }

        impl From<HdrPass> for $name {
            fn from(pass: HdrPass) -> Self {
                Self { pass }
            }
        }
    };
}

hdr_pass!(HdrDebugView3d);
hdr_pass!(HdrDebugView2d);
hdr_pass!(HdrTone3d);
hdr_pass!(HdrTone2d);
hdr_pass!(HdrEncode3d);
hdr_pass!(HdrEncode2d);

#[cfg(test)]
impl HdrDebugView3d {
    pub fn mode(&self) -> u32 {
        self.pass.mode
    }
}

#[cfg(test)]
impl HdrDebugView2d {
    pub fn mode(&self) -> u32 {
        self.pass.mode
    }
}

// Debug views and the tone curve read the exposed scene, before the
// tonemapper; the encode goes after the UI so the HUD is encoded too.
impl FullscreenMaterial for HdrDebugView3d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
}

impl FullscreenMaterial for HdrTone3d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
}

impl FullscreenMaterial for HdrEncode3d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .after(Core3dSystems::PostProcess)
            .after(ui_pass)
            .before(upscaling)
    }
}

impl FullscreenMaterial for HdrDebugView2d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule() -> impl ScheduleLabel + Clone {
        Core2d
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(Core2dSystems::PostProcess)
            .before(tonemapping)
    }
}

impl FullscreenMaterial for HdrTone2d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule() -> impl ScheduleLabel + Clone {
        Core2d
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(Core2dSystems::PostProcess)
            .before(tonemapping)
    }
}

impl FullscreenMaterial for HdrEncode2d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule() -> impl ScheduleLabel + Clone {
        Core2d
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .after(Core2dSystems::PostProcess)
            .after(ui_pass)
            .before(upscaling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonemap_cost_sums_whichever_passes_ran() {
        let metric = |name: &str, value| blockloom_protocol::RenderMetric {
            name: name.into(),
            value,
            unit: "ms".into(),
        };
        let sdr = [
            metric("render/tonemapping/elapsed_gpu", 0.2),
            metric("render/bloom/elapsed_gpu", 1.0),
        ];
        assert_eq!(tonemap_ms(&sdr), Some(0.2));
        let hdr = [
            metric(
                "render/fullscreen_material_pass<blockloom_runtime::hdr::HdrTone3d>/elapsed_gpu",
                0.25,
            ),
            metric(
                "render/fullscreen_material_pass<blockloom_runtime::hdr::HdrEncode3d>/elapsed_gpu",
                0.5,
            ),
            metric("render/tonemapping/elapsed_cpu", 9.0),
        ];
        assert_eq!(tonemap_ms(&hdr), Some(0.75));
        assert_eq!(tonemap_ms(&[]), None);
    }

    #[test]
    fn the_hdr_shader_compiles() {
        blockloom_core::shader_lib::validate(include_str!("shaders/hdr.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn views_with_their_own_curve_bypass_the_tonemapper() {
        assert!(HdrDebug(DebugView::FalseColor).bypasses_tonemapping());
        assert!(HdrDebug(DebugView::Calibration).bypasses_tonemapping());
        assert!(HdrDebug(DebugView::HdrPreview).bypasses_tonemapping());
        assert!(!HdrDebug(DebugView::Clipping).bypasses_tonemapping());
        assert!(!HdrDebug(DebugView::Histogram).bypasses_tonemapping());
        assert!(!HdrDebug(DebugView::Lit).bypasses_tonemapping());
    }

    #[test]
    fn a_debug_view_lands_on_the_camera_for_its_dimension() {
        let mut app = App::new();
        let camera = app.world_mut().spawn_empty().id();
        let frame = HdrFrame::default();
        let mut commands = app.world_mut().commands();
        HdrDebug(DebugView::Waveform).apply(&mut commands.entity(camera), false, &frame);
        app.world_mut().flush();
        assert_eq!(
            app.world()
                .get::<HdrDebugView2d>(camera)
                .map(|view| view.mode()),
            Some(4)
        );
        assert_eq!(
            app.world()
                .get::<HdrDebugView3d>(camera)
                .map(|view| view.mode()),
            None
        );

        let mut commands = app.world_mut().commands();
        HdrDebug(DebugView::Lit).apply(&mut commands.entity(camera), false, &frame);
        app.world_mut().flush();
        assert!(app.world().get::<HdrDebugView2d>(camera).is_none());
    }

    fn hdr10() -> DisplayOutput {
        DisplayOutput {
            space: OutputSpace::Hdr10,
            ..DisplayOutput::default()
        }
    }

    #[test]
    fn hdr_goes_out_only_where_the_display_offers_it() {
        let policy = HdrPolicy::default();
        // No window, or a display that only does SDR.
        assert_eq!(
            HdrFrame::resolve(hdr10(), None, policy).space,
            OutputSpace::Sdr
        );
        let sdr = [OutputSpace::Sdr];
        assert_eq!(
            HdrFrame::resolve(hdr10(), Some(&sdr), policy).space,
            OutputSpace::Sdr
        );
        // The other HDR space stands in for the chosen one.
        let scrgb = [OutputSpace::Sdr, OutputSpace::Scrgb];
        assert_eq!(
            HdrFrame::resolve(hdr10(), Some(&scrgb), policy).space,
            OutputSpace::Scrgb
        );
        let both = [OutputSpace::Sdr, OutputSpace::Scrgb, OutputSpace::Hdr10];
        let frame = HdrFrame::resolve(hdr10(), Some(&both), policy);
        assert_eq!(frame.space, OutputSpace::Hdr10);
        assert_eq!(frame.headroom(), 5.0);
        // An SDR-only build never leaves SDR, or renders FP16.
        let clamped = HdrPolicy {
            allow: false,
            windowed: true,
        };
        let frame = HdrFrame::resolve(hdr10(), Some(&both), clamped);
        assert_eq!(frame.space, OutputSpace::Sdr);
        assert!(!frame.fp16 && !frame.wanted);
    }

    #[test]
    fn an_hdr_frame_carries_the_tone_curve_and_its_encode() {
        let mut app = App::new();
        let camera = app.world_mut().spawn_empty().id();
        let both = [OutputSpace::Sdr, OutputSpace::Scrgb, OutputSpace::Hdr10];
        let frame = HdrFrame::resolve(hdr10(), Some(&both), HdrPolicy::default());
        let mut commands = app.world_mut().commands();
        frame.apply(&mut commands.entity(camera), true);
        app.world_mut().flush();
        let encode = app.world().get::<HdrEncode3d>(camera).unwrap();
        assert_eq!((encode.pass.mode, encode.pass.white), (12, 200.0));
        assert_eq!(app.world().get::<HdrTone3d>(camera).unwrap().pass.mode, 10);

        let sdr = HdrFrame::default();
        let mut commands = app.world_mut().commands();
        sdr.apply(&mut commands.entity(camera), true);
        app.world_mut().flush();
        assert!(app.world().get::<HdrEncode3d>(camera).is_none());
        assert!(app.world().get::<HdrTone3d>(camera).is_none());
    }

    #[test]
    fn hdr_metadata_follows_the_space_and_the_peak() {
        let both = [OutputSpace::Sdr, OutputSpace::Scrgb, OutputSpace::Hdr10];
        let policy = HdrPolicy::default();
        assert_eq!(HdrFrame::default().metadata(), None);
        let meta = HdrFrame::resolve(hdr10(), Some(&both), policy)
            .metadata()
            .unwrap();
        assert_eq!(meta.primaries, BT2020);
        assert_eq!((meta.max_cll, meta.max_fall), (1000.0, 200.0));
        assert_eq!(meta.max_mastering_nits, 1000.0);
        let scrgb = DisplayOutput {
            space: OutputSpace::Scrgb,
            peak_nits: 150.0,
            paper_white_nits: 200.0,
        };
        let meta = HdrFrame::resolve(scrgb, Some(&both), policy)
            .metadata()
            .unwrap();
        assert_eq!(meta.primaries, BT709);
        // A frame's average can't pass its brightest pixel.
        assert_eq!((meta.max_cll, meta.max_fall), (150.0, 150.0));
    }

    #[test]
    fn blocks_override_the_projects_display_for_the_run() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, blockloom_core::scene::Mode::TwoD);
        assert_eq!(wanted_output(&engine).space, OutputSpace::Sdr);
        engine.hdr_output = Some(true);
        engine.peak_nits = Some(50_000.0);
        let wanted = wanted_output(&engine);
        assert_eq!(wanted.space, OutputSpace::Scrgb);
        assert_eq!(wanted.peak_nits, 10_000.0);
        engine.project.world.display.space = OutputSpace::Hdr10;
        assert_eq!(wanted_output(&engine).space, OutputSpace::Hdr10);
        engine.hdr_output = Some(false);
        assert_eq!(wanted_output(&engine).space, OutputSpace::Sdr);
    }
}
