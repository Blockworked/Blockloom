//! The post stack's own passes, both dimensions, over what Bevy already
//! does (depth of field, motion blur, SSAO, SSR, chromatic aberration,
//! vignette, the tonemapper and sharpening, set up in `environment`).
//!
//! `post_hdr` runs last before the tonemapper: a five-level bloom chain
//! (thresholded prefilter, 13-tap downsamples, scattering tent upsamples),
//! the bloom and its lens dirt laid over the scene, white balance, grading
//! and the toe/shoulder shape. It also draws the bloom mip, circle of
//! confusion and AO debug views. `post_ldr` runs after the tonemapper and
//! sharpening: the LUT and film grain. Both read `PostStack`, which
//! `apply_post` resolves from the blended `Environment` every frame.
//!
//! Also here: auto-exposure, which meters through `luminance` and claims
//! `ExposureClaims::auto`, and depth of field's autofocus.

use crate::edit::SceneEditor;
use crate::engine::{ActorId, Dimension, Engine};
use crate::environment::{Environment, ExposureClaims};
use crate::hdr::{HdrDebug, HdrDebugView2d, HdrDebugView3d, HdrFrame};
use crate::luminance::SceneLuminance;
use crate::world::WorldCamera;
use bevy::anti_alias::contrast_adaptive_sharpening::cas;
use bevy::anti_alias::fxaa::fxaa;
use bevy::anti_alias::smaa::smaa;
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::fullscreen_material::fullscreen_material_system;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core2d, Core2dSystems, Core3d, Core3dSystems, FullscreenShader};
use bevy::diagnostic::FrameCount;
use bevy::pbr::ScreenSpaceAmbientOcclusionResources;
use bevy::post_process::bloom::bloom;
use bevy::post_process::dof::DepthOfField;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_3d, texture_depth_2d, texture_depth_2d_multisampled,
    uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, FallbackImage, GpuImage, TextureCache};
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use blockloom_core::scene::{Grading, Metering, Mode, PostProcess};
use blockloom_protocol::DebugView;

/// Levels in the bloom chain, the first at half resolution.
pub const BLOOM_LEVELS: u32 = 5;
const BLOOM_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// Bevy's xy chromaticity shift at a project's full warmth or tint.
const WHITE_BALANCE_RANGE: f32 = 0.08;

pub fn register(app: &mut App) {
    app.init_resource::<LoadedLut>();
    if app.get_sub_app(RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/post.wesl");
    app.add_plugins(ExtractComponentPlugin::<PostStack>::default());
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .init_gpu_resource::<SpecializedRenderPipelines<PostPipelines>>()
        .init_resource::<PostUniformBuffer>()
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(Render, prepare_post.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            (
                post_hdr
                    .in_set(Core3dSystems::PostProcess)
                    .after(bloom)
                    .after(crate::luminance::MeterPass)
                    .before(fullscreen_material_system::<HdrDebugView3d>)
                    .before(tonemapping),
                post_ldr
                    .in_set(PostLdrPass)
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping)
                    .after(fxaa)
                    .after(smaa)
                    .after(cas),
            ),
        )
        .add_systems(
            Core2d,
            (
                post_hdr
                    .in_set(Core2dSystems::PostProcess)
                    .after(bloom)
                    .after(crate::luminance::MeterPass)
                    .before(fullscreen_material_system::<HdrDebugView2d>)
                    .before(tonemapping),
                post_ldr
                    .in_set(PostLdrPass)
                    .in_set(Core2dSystems::PostProcess)
                    .after(tonemapping)
                    .after(fxaa)
                    .after(smaa)
                    .after(cas),
            ),
        );
}

/// The pass after the tonemapper, for passes that draw over its result.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PostLdrPass;

/// The HDR pass, as `hdr::ToneInputs` finds it in a schedule.
pub(crate) fn hdr_type() -> std::any::TypeId {
    crate::hdr::system_type(post_hdr)
}

// ─── Main world ──────────────────────────────────────────────────────────

/// Which debug view the HDR pass draws in place of the scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostDebug {
    #[default]
    None,
    /// One level of the bloom chain, 0 the sharpest.
    BloomMip(u32),
    CircleOfConfusion,
    AmbientOcclusion,
}

impl PostDebug {
    pub fn of(view: DebugView, bloom_mip: u32) -> Self {
        match view {
            DebugView::BloomMip => Self::BloomMip(bloom_mip.min(BLOOM_LEVELS - 1)),
            DebugView::CircleOfConfusion => Self::CircleOfConfusion,
            DebugView::AmbientOcclusion => Self::AmbientOcclusion,
            _ => Self::None,
        }
    }

    fn mode(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::BloomMip(_) => 1.0,
            Self::CircleOfConfusion => 2.0,
            Self::AmbientOcclusion => 3.0,
        }
    }
}

/// A loaded LUT: its 3D image and the input range it maps from.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub image: Handle<Image>,
    pub size: u32,
    pub domain_min: Vec3,
    pub domain_max: Vec3,
}

/// Everything the post passes need for one camera, resolved each frame.
#[derive(Component, ExtractComponent, Clone, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct PostStack {
    pub post: PostProcess,
    /// The lens dirt image, when the project names one.
    pub dirt: Option<Handle<Image>>,
    pub lut: Option<Lut>,
    pub debug: PostDebug,
    /// The tonemapper has stood aside for HDR output.
    pub hdr_output: bool,
    pub is_3d: bool,
}

impl PostStack {
    /// The pass before the tonemapper changes a pixel.
    pub fn hdr_active(&self) -> bool {
        self.post.bloom_enabled
            || self.post.grading.is_active()
            || self.tone_active()
            || self.debug != PostDebug::None
    }

    /// Just the bloom, for a capture of the linear scene.
    pub fn linear_only(&self) -> Self {
        let defaults = PostProcess::default();
        Self {
            post: PostProcess {
                bloom_enabled: self.post.bloom_enabled,
                bloom_threshold: self.post.bloom_threshold,
                bloom_intensity: self.post.bloom_intensity,
                bloom_knee: self.post.bloom_knee,
                bloom_scatter: self.post.bloom_scatter,
                bloom_dirt: self.post.bloom_dirt.clone(),
                bloom_dirt_intensity: self.post.bloom_dirt_intensity,
                ..defaults
            },
            dirt: self.dirt.clone(),
            lut: None,
            debug: PostDebug::None,
            hdr_output: false,
            is_3d: self.is_3d,
        }
    }

    fn tone_active(&self) -> bool {
        self.post.tone.toe != 0.0 || self.post.tone.shoulder != 0.0
    }

    /// The pass after the tonemapper changes a pixel.
    pub fn ldr_active(&self) -> bool {
        let lut = self.lut.is_some() && self.post.grading.lut_contribution > 0.0;
        (lut && !self.hdr_output) || self.post.grain.intensity > 0.0
    }
}

/// The white balance as Bevy computes it: the white point moved in xy,
/// then an LMS von Kries scale, as one matrix on linear RGB.
pub fn white_balance(grading: &Grading) -> Mat3 {
    const RGB_TO_LMS: Mat3 = Mat3::from_cols(
        Vec3::new(0.311692, 0.0905138, 0.00764433),
        Vec3::new(0.652085, 0.901341, 0.0486554),
        Vec3::new(0.0362225, 0.00814478, 0.943700),
    );
    const LMS_TO_RGB: Mat3 = Mat3::from_cols(
        Vec3::new(4.06305, -0.40791, -0.0118812),
        Vec3::new(-2.93241, 1.40437, -0.0486532),
        Vec3::new(-0.130646, 0.00353630, 1.0605344),
    );
    const D65_XY: Vec2 = Vec2::new(0.31272, 0.32903);
    const D65_LMS: Vec3 = Vec3::new(0.975538, 1.01648, 1.08475);
    if grading.temperature == 0.0 && grading.tint == 0.0 {
        return Mat3::IDENTITY;
    }
    let xy = D65_XY + Vec2::new(-grading.temperature, grading.tint) * WHITE_BALANCE_RANGE;
    let lms = Vec3::new(0.701634, 1.15856, -0.904175)
        + (Vec3::new(-0.051461, 0.045854, 0.953127)
            + Vec3::new(0.452749, -0.296122, -0.955206) * xy.x)
            / xy.y;
    LMS_TO_RGB * Mat3::from_diagonal(D65_LMS / lms) * RGB_TO_LMS
}

/// The LUT file the project names, read once per change.
#[derive(Resource, Default)]
pub struct LoadedLut {
    key: Option<(Option<std::path::PathBuf>, String)>,
    lut: Option<Lut>,
}

fn load_lut(dir: &std::path::Path, path: &str, images: &mut Assets<Image>) -> Result<Lut, String> {
    let volume = blockloom_core::pipeline::load_volume(dir, path)?;
    let [x, y, z] = volume.info.size;
    if x != y || y != z || x < 2 {
        return Err(format!("{path} isn't a cube LUT ({x}x{y}x{z})"));
    }
    let bytes = volume
        .texels
        .iter()
        .flat_map(|texel| texel.map(|c| half::f16::from_f32(c).to_le_bytes()))
        .flatten()
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: x,
            height: y,
            depth_or_array_layers: z,
        },
        TextureDimension::D3,
        bytes,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.label = Some("post_lut");
    Ok(Lut {
        image: images.add(image),
        size: x,
        domain_min: Vec3::from(volume.domain_min),
        domain_max: Vec3::from(volume.domain_max),
    })
}

/// Resolves every world camera's `PostStack` from the blended environment.
#[allow(clippy::too_many_arguments)]
pub fn apply_post(
    mut commands: Commands,
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    debug: Res<HdrDebug>,
    frame: Res<HdrFrame>,
    editor: Option<Res<SceneEditor>>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut loaded: ResMut<LoadedLut>,
    mut dirt: Local<Option<(String, Handle<Image>)>>,
    patches: Option<ResMut<crate::pbr_patch::PbrPatches>>,
    cameras: Query<(Entity, Option<&PostStack>, Has<Camera3d>), With<WorldCamera>>,
) {
    let post = &environment.post;
    // Baked into the SSAO shader, so it is the project's, not blended.
    if let Some(mut patches) = patches {
        let intensity = engine.project.world.post.ao.intensity;
        if patches.ao_intensity != intensity {
            patches.ao_intensity = intensity;
        }
    }
    let dir = engine.project_dir.clone();
    let key = (dir.clone(), post.grading.lut.trim().to_string());
    if loaded.key.as_ref() != Some(&key) {
        loaded.lut = match (&key.0, key.1.as_str()) {
            (Some(dir), path) if !path.is_empty() => load_lut(dir, path, &mut images)
                .map_err(|error| {
                    crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
                        actor: "Blockloom".into(),
                        message: format!("The grading LUT didn't load: {error}"),
                    })
                })
                .ok(),
            _ => None,
        };
        loaded.key = Some(key);
    }
    let dirt_path = post.bloom_dirt.trim();
    if dirt_path.is_empty() {
        *dirt = None;
    } else if dirt.as_ref().is_none_or(|(path, _)| path != dirt_path) {
        let resolved = crate::world::asset_path(dir.as_deref(), dirt_path);
        *dirt = Some((dirt_path.to_string(), assets.load::<Image>(resolved)));
    }
    let bloom_mip = editor.map(|editor| editor.view.bloom_mip).unwrap_or(0);
    for (entity, current, is_3d) in &cameras {
        let mut post = post.clone();
        if !is_3d {
            // Auto-exposure, depth of field and the screen-space passes are
            // 3D; the rest of the chain is the same in both.
            post.auto_exposure.enabled = false;
        }
        let next = PostStack {
            post,
            dirt: dirt.as_ref().map(|(_, handle)| handle.clone()),
            lut: loaded.lut.clone(),
            debug: PostDebug::of(debug.0, bloom_mip),
            hdr_output: frame.is_hdr(),
            is_3d,
        };
        if current != Some(&next) {
            commands.entity(entity).insert(next);
        }
    }
}

/// Keeps the named actor in focus: the depth of field's distance becomes
/// its depth along the camera's view.
pub fn focus_depth_of_field(
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    mut cameras: Query<(&GlobalTransform, &mut DepthOfField), With<WorldCamera>>,
    actors: Query<(&ActorId, &GlobalTransform), Without<WorldCamera>>,
) {
    let target = environment.post.depth_of_field.target.trim();
    if target.is_empty() {
        return;
    }
    let named = |id: &str| engine.actor(id).is_some_and(|actor| actor.name == target);
    for (camera, mut dof) in &mut cameras {
        let forward = camera.forward();
        let nearest = actors
            .iter()
            .filter(|(id, _)| named(&id.0))
            .map(|(_, at)| (at.translation() - camera.translation()).dot(*forward))
            .filter(|depth| *depth > 0.0)
            .min_by(f32::total_cmp);
        if let Some(depth) = nearest {
            let depth = depth.max(0.05);
            if dof.focal_distance != depth {
                dof.focal_distance = depth;
            }
        }
    }
}

/// Auto-exposure: moves `ExposureClaims::auto` towards what the meter asks
/// for at the project's speed. Off (or in 2D) it lets go, so the manual EV
/// shows again.
pub fn auto_expose(
    time: Res<Time>,
    dimension: Res<Dimension>,
    environment: Res<Environment>,
    luminance: Res<SceneLuminance>,
    mut claims: ResMut<ExposureClaims>,
) {
    let auto = &environment.post.auto_exposure;
    if !auto.enabled || dimension.0 != Mode::ThreeD {
        if claims.auto.is_some() {
            claims.auto = None;
        }
        return;
    }
    // The frame was metered at what the camera used, which is the blended
    // EV unless the director holds it.
    let current = environment.exposure;
    let Some(measured) = luminance.metered(auto.metering) else {
        return;
    };
    let from = claims.auto.unwrap_or(current);
    let target = auto.target(current, measured);
    let next = auto.step(from, target, time.delta_secs());
    if claims.auto != Some(next) {
        claims.auto = Some(next);
    }
}

impl SceneLuminance {
    /// The log-average the metering mode reads, once a frame has been
    /// metered.
    pub fn metered(&self, metering: Metering) -> Option<f32> {
        if !self.measured {
            return None;
        }
        Some(match metering {
            Metering::Average => self.average,
            Metering::CenterWeighted => self.center,
            Metering::Spot => self.spot,
        })
    }
}

// ─── Render world ────────────────────────────────────────────────────────

/// `PostUniforms` in `post.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct PostUniforms {
    bloom: Vec4,
    flags: Vec4,
    tone: Vec4,
    balance_x: Vec4,
    balance_y: Vec4,
    balance_z: Vec4,
    lift: Vec4,
    gamma: Vec4,
    gain: Vec4,
    lut: Vec4,
    lut_min: Vec4,
    lut_max: Vec4,
    grain: Vec4,
    dof: Vec4,
    dof_view: Vec4,
}

impl PostUniforms {
    /// `ao` and `depth` say whether those textures are bound this frame.
    /// `focus` is the depth of field's distance after autofocus.
    fn new(
        stack: &PostStack,
        view: &ExtractedView,
        frame: u32,
        focus: Option<f32>,
        (ao, depth): (bool, bool),
    ) -> Self {
        let post = &stack.post;
        let grading = &post.grading;
        let balance = white_balance(grading);
        let dof = &post.depth_of_field;
        // Bevy's circle of confusion: focal length from the vertical field
        // of view over a Super 35 sensor.
        let sensor = 0.01866;
        let tan_half = 1.0 / view.clip_from_view.y_axis.y.max(1e-6);
        let focal_length = 0.5 * sensor / tan_half;
        let coc_scale = focal_length * focal_length / (sensor * dof.f_stops.max(0.01));
        let far = if dof.far_limit > 0.0 {
            dof.far_limit
        } else {
            f32::MAX
        };
        let focus = focus.unwrap_or(dof.focus_distance);
        let lut = stack.lut.as_ref();
        let bloom_on = post.bloom_enabled || matches!(stack.debug, PostDebug::BloomMip(_));
        Self {
            bloom: Vec4::new(
                post.bloom_threshold,
                post.bloom_knee,
                post.bloom_scatter,
                post.bloom_intensity,
            ),
            flags: Vec4::new(
                if stack.dirt.is_some() {
                    post.bloom_dirt_intensity
                } else {
                    0.0
                },
                f32::from(bloom_on && post.bloom_enabled),
                stack.debug.mode(),
                f32::from(grading.is_active()),
            ),
            tone: Vec4::new(
                post.tone.toe,
                post.tone.shoulder,
                grading.saturation,
                grading.contrast,
            ),
            balance_x: balance.x_axis.extend(0.0),
            balance_y: balance.y_axis.extend(0.0),
            balance_z: balance.z_axis.extend(0.0),
            lift: Vec3::from(grading.lift).extend(f32::from(ao)),
            gamma: Vec3::from(grading.gamma).extend(f32::from(depth)),
            gain: Vec3::from(grading.gain).extend(f32::from(stack.tone_active())),
            lut: Vec4::new(
                grading.lut_contribution,
                lut.map_or(2.0, |lut| lut.size as f32),
                f32::from(lut.is_some()),
                f32::from(stack.hdr_output),
            ),
            lut_min: lut.map_or(Vec3::ZERO, |lut| lut.domain_min).extend(0.0),
            lut_max: lut.map_or(Vec3::ONE, |lut| lut.domain_max).extend(0.0),
            grain: Vec4::new(
                post.grain.intensity,
                post.grain.size,
                post.grain.response,
                (frame % 65_536) as f32,
            ),
            dof: Vec4::new(focus, coc_scale, focal_length, dof.max_blur),
            dof_view: Vec4::new(
                if dof.near { far } else { -far },
                view.clip_from_view.w_axis.z,
                0.0,
                0.0,
            ),
        }
    }
}

#[derive(Resource, Default)]
struct PostUniformBuffer(DynamicUniformBuffer<PostUniforms>);

/// How the composite finds depth: none in 2D, the prepass in 3D.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Guide {
    Flat,
    Depth,
    DepthMultisampled,
}

impl Guide {
    const ALL: [Guide; 3] = [Self::Flat, Self::Depth, Self::DepthMultisampled];

    fn features(self) -> [(&'static str, bool); 2] {
        [
            ("DEPTH", self != Self::Flat),
            ("MULTISAMPLED", self == Self::DepthMultisampled),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PostKey {
    Composite(Guide, TextureFormat),
    Finish(TextureFormat),
    Prefilter,
    Downsample,
    Upsample,
}

#[derive(Resource)]
struct PostPipelines {
    /// Down chain: uniforms, source, sampler.
    down: BindGroupLayoutDescriptor,
    /// Up chain: those plus the level below.
    up: BindGroupLayoutDescriptor,
    composite: [BindGroupLayoutDescriptor; 3],
    finish: BindGroupLayoutDescriptor,
    sampler: Sampler,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
}

fn filterable() -> BindGroupLayoutEntryBuilder {
    texture_2d(TextureSampleType::Float { filterable: true })
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let stage = ShaderStages::FRAGMENT;
    let uniforms = || uniform_buffer::<PostUniforms>(true);
    let linear = || sampler(SamplerBindingType::Filtering);
    let down = BindGroupLayoutDescriptor::new(
        "post_down_layout",
        &BindGroupLayoutEntries::with_indices(
            stage,
            ((0, uniforms()), (1, filterable()), (2, linear())),
        ),
    );
    let up = BindGroupLayoutDescriptor::new(
        "post_up_layout",
        &BindGroupLayoutEntries::with_indices(
            stage,
            (
                (0, uniforms()),
                (1, filterable()),
                (2, linear()),
                (3, filterable()),
            ),
        ),
    );
    let composite = Guide::ALL.map(|guide| {
        let base = (
            (0, uniforms()),
            (1, filterable()),
            (2, linear()),
            (4, filterable()),
            (5, filterable()),
            (
                6,
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        );
        let entries = match guide {
            Guide::Flat => BindGroupLayoutEntries::with_indices(stage, base).to_vec(),
            Guide::Depth => BindGroupLayoutEntries::with_indices(
                stage,
                (
                    base.0,
                    base.1,
                    base.2,
                    base.3,
                    base.4,
                    base.5,
                    (7, texture_depth_2d()),
                ),
            )
            .to_vec(),
            Guide::DepthMultisampled => BindGroupLayoutEntries::with_indices(
                stage,
                (
                    base.0,
                    base.1,
                    base.2,
                    base.3,
                    base.4,
                    base.5,
                    (7, texture_depth_2d_multisampled()),
                ),
            )
            .to_vec(),
        };
        BindGroupLayoutDescriptor::new("post_composite_layout", &entries)
    });
    let finish = BindGroupLayoutDescriptor::new(
        "post_finish_layout",
        &BindGroupLayoutEntries::with_indices(
            stage,
            (
                (0, uniforms()),
                (1, filterable()),
                (2, linear()),
                (8, texture_3d(TextureSampleType::Float { filterable: true })),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("post_linear_clamp"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        ..default()
    });
    commands.insert_resource(PostPipelines {
        down,
        up,
        composite,
        finish,
        sampler,
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "shaders/post.wesl"),
        fullscreen: fullscreen.clone(),
    });
}

impl SpecializedRenderPipeline for PostPipelines {
    type Key = PostKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let (label, entry, layout, format, guide) = match key {
            PostKey::Composite(guide, format) => (
                "post_composite",
                "composite",
                self.composite[guide as usize].clone(),
                format,
                guide,
            ),
            PostKey::Finish(format) => (
                "post_finish",
                "finish",
                self.finish.clone(),
                format,
                Guide::Flat,
            ),
            PostKey::Prefilter => (
                "post_bloom_prefilter",
                "prefilter",
                self.down.clone(),
                BLOOM_FORMAT,
                Guide::Flat,
            ),
            PostKey::Downsample => (
                "post_bloom_downsample",
                "downsample",
                self.down.clone(),
                BLOOM_FORMAT,
                Guide::Flat,
            ),
            PostKey::Upsample => (
                "post_bloom_upsample",
                "upsample",
                self.up.clone(),
                BLOOM_FORMAT,
                Guide::Flat,
            ),
        };
        let shader_defs = guide
            .features()
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name.into())
            .collect();
        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

/// The bloom chain's levels, as views of two mipped textures: `down` from
/// the prefilter, `up` what each level scatters upwards.
struct BloomChain {
    down: Vec<TextureView>,
    up: Vec<TextureView>,
    // Kept alive for the frame.
    _textures: [CachedTexture; 2],
}

impl BloomChain {
    fn levels(&self) -> usize {
        self.down.len()
    }

    /// The finished bloom: the top of the up chain, or the one level.
    fn result(&self) -> &TextureView {
        self.up.first().unwrap_or(&self.down[0])
    }

    /// Level `mip` as the debug view shows it: scattered, bar the last.
    fn level(&self, mip: u32) -> &TextureView {
        let mip = (mip as usize).min(self.levels() - 1);
        self.up.get(mip).unwrap_or(&self.down[mip])
    }
}

/// The pass before the tonemapper, for a view that needs it.
#[derive(Component)]
struct ViewPostHdr {
    offset: u32,
    guide: Guide,
    composite: CachedRenderPipelineId,
    bloom: Option<BloomChain>,
}

/// The pass after the tonemapper.
#[derive(Component)]
struct ViewPostLdr {
    offset: u32,
    finish: CachedRenderPipelineId,
}

#[derive(Component, Clone, Copy)]
struct BloomPipelineIds {
    prefilter: CachedRenderPipelineId,
    downsample: CachedRenderPipelineId,
    upsample: CachedRenderPipelineId,
}

/// Levels that fit a first level of `size`, at most `BLOOM_LEVELS`.
fn bloom_levels(size: UVec2) -> u32 {
    let smallest = size.min_element().max(1);
    (32 - smallest.leading_zeros()).clamp(1, BLOOM_LEVELS)
}

fn mip_views(texture: &CachedTexture, levels: u32, label: &'static str) -> Vec<TextureView> {
    (0..levels)
        .map(|level| {
            texture.texture.create_view(&TextureViewDescriptor {
                label: Some(label),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..default()
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare_post(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut specialized: ResMut<SpecializedRenderPipelines<PostPipelines>>,
    pipelines: Option<Res<PostPipelines>>,
    mut textures: ResMut<TextureCache>,
    mut buffer: ResMut<PostUniformBuffer>,
    frame: Res<FrameCount>,
    views: Query<(
        Entity,
        &ExtractedView,
        &ExtractedCamera,
        &PostStack,
        Option<&Msaa>,
        Option<&ViewPrepassTextures>,
        Has<ScreenSpaceAmbientOcclusionResources>,
        Option<&DepthOfField>,
    )>,
    stale: Query<
        Entity,
        (
            Or<(With<ViewPostHdr>, With<ViewPostLdr>)>,
            Without<PostStack>,
        ),
    >,
) {
    for entity in &stale {
        commands
            .entity(entity)
            .remove::<(ViewPostHdr, ViewPostLdr, BloomPipelineIds)>();
    }
    buffer.0.clear();
    let Some(pipelines) = pipelines else {
        return;
    };
    for (entity, view, camera, stack, msaa, prepass, ao, dof) in &views {
        let mut entity = commands.entity(entity);
        let depth = prepass.is_some_and(|prepass| prepass.depth.is_some());
        let focus = dof.map(|dof| dof.focal_distance);
        let uniforms = PostUniforms::new(stack, view, frame.0, focus, (ao, depth));
        let offset = buffer.0.push(&uniforms);
        let format = view.target_format;
        if stack.ldr_active() {
            let finish = specialized.specialize(&cache, &pipelines, PostKey::Finish(format));
            entity.insert(ViewPostLdr { offset, finish });
        } else {
            entity.remove::<ViewPostLdr>();
        }
        if !stack.hdr_active() {
            entity.remove::<ViewPostHdr>();
            continue;
        }
        let guide = match (depth, msaa.is_some_and(|msaa| msaa.samples() > 1)) {
            (false, _) => Guide::Flat,
            (true, false) => Guide::Depth,
            (true, true) => Guide::DepthMultisampled,
        };
        let composite =
            specialized.specialize(&cache, &pipelines, PostKey::Composite(guide, format));
        let wants_bloom = stack.post.bloom_enabled || matches!(stack.debug, PostDebug::BloomMip(_));
        let size = camera
            .physical_viewport_size
            .unwrap_or(UVec2::ONE)
            .max(UVec2::ONE);
        let bloom = wants_bloom.then(|| {
            let first = (size / 2).max(UVec2::ONE);
            let levels = bloom_levels(first);
            let descriptor = |label: &'static str, mips: u32| TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: first.x,
                    height: first.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips.max(1),
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: BLOOM_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };
            let down = textures.get(&device, descriptor("post_bloom_down", levels));
            let up = textures.get(&device, descriptor("post_bloom_up", levels - 1));
            BloomChain {
                down: mip_views(&down, levels, "post_bloom_down_level"),
                up: mip_views(&up, levels - 1, "post_bloom_up_level"),
                _textures: [down, up],
            }
        });
        entity.insert(BloomPipelineIds {
            prefilter: specialized.specialize(&cache, &pipelines, PostKey::Prefilter),
            downsample: specialized.specialize(&cache, &pipelines, PostKey::Downsample),
            upsample: specialized.specialize(&cache, &pipelines, PostKey::Upsample),
        });
        entity.insert(ViewPostHdr {
            offset,
            guide,
            composite,
            bloom,
        });
    }
    buffer.0.write_buffer(&device, &queue);
}

/// One fullscreen draw into `target`.
fn draw(
    ctx: &mut RenderContext,
    label: &'static str,
    pipeline: &RenderPipeline,
    bind_group: &BindGroup,
    offset: u32,
    target: &TextureView,
) {
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    let span = diagnostics.pass_span(&mut pass, label);
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[offset]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}

/// Bloom, white balance, grading and the tone shape, before the tonemapper.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn post_hdr(
    view: ViewQuery<(
        &ViewTarget,
        &ViewPostHdr,
        &BloomPipelineIds,
        &PostStack,
        Option<&ViewPrepassTextures>,
        Option<&ScreenSpaceAmbientOcclusionResources>,
    )>,
    pipelines: Option<Res<PostPipelines>>,
    buffer: Res<PostUniformBuffer>,
    cache: Res<PipelineCache>,
    fallback: Res<FallbackImage>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, post_view, bloom_ids, stack, prepass, ao) = view.into_inner();
    let Some(pipelines) = pipelines else {
        return;
    };
    let (Some(composite), Some(uniforms)) = (
        cache.get_render_pipeline(post_view.composite),
        buffer.0.binding(),
    ) else {
        return;
    };
    let depth = prepass.and_then(ViewPrepassTextures::depth_only_view);
    if post_view.guide != Guide::Flat && depth.is_none() {
        return;
    }
    // Bloom's pipelines compile with the rest; until they have, the scene
    // goes through without it rather than skipping the grade too.
    let chain = post_view.bloom.as_ref().and_then(|chain| {
        let prefilter = cache.get_render_pipeline(bloom_ids.prefilter)?;
        let downsample = cache.get_render_pipeline(bloom_ids.downsample)?;
        let upsample = cache.get_render_pipeline(bloom_ids.upsample)?;
        Some((chain, prefilter, downsample, upsample))
    });
    let post = target.post_process_write();
    let device = ctx.render_device().clone();
    let sampler = &pipelines.sampler;
    let offset = post_view.offset;
    if let Some((chain, prefilter, downsample, upsample)) = chain {
        let down_layout = cache.get_bind_group_layout(&pipelines.down);
        let up_layout = cache.get_bind_group_layout(&pipelines.up);
        let from_scene = device.create_bind_group(
            "post_bloom_prefilter",
            &down_layout,
            &BindGroupEntries::with_indices((
                (0, uniforms.clone()),
                (1, post.source),
                (2, sampler),
            )),
        );
        draw(
            &mut ctx,
            "post_bloom_prefilter",
            prefilter,
            &from_scene,
            offset,
            &chain.down[0],
        );
        for level in 1..chain.levels() {
            let group = device.create_bind_group(
                "post_bloom_downsample",
                &down_layout,
                &BindGroupEntries::with_indices((
                    (0, uniforms.clone()),
                    (1, &chain.down[level - 1]),
                    (2, sampler),
                )),
            );
            draw(
                &mut ctx,
                "post_bloom_downsample",
                downsample,
                &group,
                offset,
                &chain.down[level],
            );
        }
        // up[i] = down[i] with the level below scattered in; the last up
        // level reads the bottom of the down chain.
        for level in (0..chain.up.len()).rev() {
            let below = chain.up.get(level + 1).unwrap_or(&chain.down[level + 1]);
            let group = device.create_bind_group(
                "post_bloom_upsample",
                &up_layout,
                &BindGroupEntries::with_indices((
                    (0, uniforms.clone()),
                    (1, &chain.down[level]),
                    (2, sampler),
                    (3, below),
                )),
            );
            draw(
                &mut ctx,
                "post_bloom_upsample",
                upsample,
                &group,
                offset,
                &chain.up[level],
            );
        }
    }
    let blank = &fallback.d2.texture_view;
    let bloom_view = match (chain, stack.debug) {
        (Some((chain, ..)), PostDebug::BloomMip(mip)) => chain.level(mip),
        (Some((chain, ..)), _) => chain.result(),
        (None, _) => blank,
    };
    let dirt = stack
        .dirt
        .as_ref()
        .and_then(|handle| images.get(handle))
        .map_or(blank, |image| &image.texture_view);
    let ao = ao.map_or(blank, |ao| {
        &ao.screen_space_ambient_occlusion_texture.default_view
    });
    let layout = cache.get_bind_group_layout(&pipelines.composite[post_view.guide as usize]);
    let base = (
        (0, uniforms.clone()),
        (1, post.source),
        (2, sampler),
        (4, bloom_view),
        (5, dirt),
        (6, ao),
    );
    let group = match depth {
        Some(depth) if post_view.guide != Guide::Flat => device.create_bind_group(
            "post_composite",
            &layout,
            &BindGroupEntries::with_indices((
                base.0,
                base.1,
                base.2,
                base.3,
                base.4,
                base.5,
                (7, depth),
            )),
        ),
        _ => device.create_bind_group(
            "post_composite",
            &layout,
            &BindGroupEntries::with_indices(base),
        ),
    };
    draw(
        &mut ctx,
        "post_composite",
        composite,
        &group,
        offset,
        post.destination,
    );
}

/// The LUT and film grain, after the tonemapper and sharpening.
fn post_ldr(
    view: ViewQuery<(&ViewTarget, &ViewPostLdr, &PostStack)>,
    pipelines: Option<Res<PostPipelines>>,
    buffer: Res<PostUniformBuffer>,
    cache: Res<PipelineCache>,
    fallback: Res<FallbackImage>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, post_view, stack) = view.into_inner();
    let Some(pipelines) = pipelines else {
        return;
    };
    let (Some(finish), Some(uniforms)) = (
        cache.get_render_pipeline(post_view.finish),
        buffer.0.binding(),
    ) else {
        return;
    };
    let lut = stack
        .lut
        .as_ref()
        .and_then(|lut| images.get(&lut.image))
        .map_or(&fallback.d3.texture_view, |image| &image.texture_view);
    let post = target.post_process_write();
    let layout = cache.get_bind_group_layout(&pipelines.finish);
    let group = ctx.render_device().create_bind_group(
        "post_finish",
        &layout,
        &BindGroupEntries::with_indices((
            (0, uniforms),
            (1, post.source),
            (2, &pipelines.sampler),
            (8, lut),
        )),
    );
    draw(
        &mut ctx,
        "post_finish",
        finish,
        &group,
        post_view.offset,
        post.destination,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    #[test]
    fn the_post_shader_compiles_for_every_guide() {
        let source = include_str!("shaders/post.wesl");
        for guide in Guide::ALL {
            shader_lib::validate(source, &guide.features())
                .unwrap_or_else(|error| panic!("{guide:?}: {error}"));
        }
    }

    #[test]
    fn post_uniforms_match_the_wesl_layout() {
        // Fifteen vec4s.
        assert_eq!(PostUniforms::min_size().get(), 15 * 16);
        let shader = include_str!("shaders/post.wesl");
        let start = shader.find("struct PostUniforms {").unwrap();
        let body = &shader[start..start + shader[start..].find('}').unwrap()];
        let fields = body
            .lines()
            .filter(|l| l.trim_end().ends_with("vec4<f32>,"));
        assert_eq!(fields.count(), 15);
    }

    #[test]
    fn neutral_white_balance_is_the_identity() {
        assert_eq!(white_balance(&Grading::default()), Mat3::IDENTITY);
        let warm = white_balance(&Grading {
            temperature: 0.5,
            ..default()
        });
        let white = warm * Vec3::ONE;
        assert!(white.x > white.z, "warmer white leans red: {white}");
    }

    #[test]
    fn the_chain_fits_small_views() {
        assert_eq!(bloom_levels(UVec2::new(960, 540)), BLOOM_LEVELS);
        assert_eq!(bloom_levels(UVec2::new(1, 1)), 1);
        assert_eq!(bloom_levels(UVec2::new(8, 4)), 3);
    }

    #[test]
    fn a_default_stack_draws_nothing() {
        let stack = PostStack::default();
        assert!(!stack.hdr_active());
        assert!(!stack.ldr_active());
        let bloom = PostStack {
            post: PostProcess {
                bloom_enabled: true,
                ..default()
            },
            ..default()
        };
        assert!(bloom.hdr_active());
        let grain = PostStack {
            post: PostProcess {
                grain: blockloom_core::scene::Grain {
                    intensity: 0.3,
                    ..default()
                },
                ..default()
            },
            ..default()
        };
        assert!(grain.ldr_active());
    }

    #[test]
    fn auto_exposure_follows_the_meter_at_its_speed() {
        let mut app = App::new();
        let mut env = Environment::default();
        env.post.auto_exposure.enabled = true;
        env.post.auto_exposure.metering = Metering::Average;
        env.exposure = 10.0;
        app.insert_resource(env)
            .insert_resource(Dimension(Mode::ThreeD))
            .insert_resource(SceneLuminance {
                // Two stops over middle grey.
                average: 0.72,
                measured: true,
                ..default()
            })
            .init_resource::<ExposureClaims>()
            .init_resource::<Time>()
            .add_systems(Update, auto_expose);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(250));
        app.update();
        // Speeding up at 3 EV a second, a quarter second in.
        let ev = app.world().resource::<ExposureClaims>().auto.unwrap();
        assert!((ev - 10.75).abs() < 1e-4, "{ev}");

        app.world_mut()
            .resource_mut::<Environment>()
            .post
            .auto_exposure
            .enabled = false;
        app.update();
        assert_eq!(app.world().resource::<ExposureClaims>().auto, None);
    }
}
