//! Custom shader materials, tilemap meshes and PBR helpers.
//!
//! [`GraphMaterial2d`] and [`GraphMaterial3d`] render a [`GraphEffect`] each
//! through one shared ubershader (`shaders/graph_2d.wesl` and
//! `shaders/graph_3d.wesl`, embedded at compile time). The uniforms carry the
//! look's tint, the effect's second color, and `(mode, speed, strength,
//! time)`; `time` is refreshed every frame by [`tick_graph_time`], so motion
//! never needs the actor to do anything. A textured look keeps its image
//! under the effect; a circle look renders its quad round.
//!
//! An effect whose `source` names a `.wesl` asset draws with that file
//! instead: [`surface_shader`] wraps the file's `graph_main` in the same
//! bindings and fragment entry the ubershader has, and the material's
//! `specialize` swaps it in as the fragment shader, per material.
//!
//! PBR-only materials need no plugin: [`surface_standard`] folds a
//! [`SurfaceMaterial`] into the `StandardMaterial` the 3D pipeline already
//! uses. [`tilemesh_to_bevy`] turns a [`Tilemap`] into an uploadable mesh.
//!
//! [`GraphEffect`]: blockloom_core::material::GraphEffect
//! [`SurfaceMaterial`]: blockloom_core::material::SurfaceMaterial
//! [`Tilemap`]: blockloom_core::material::Tilemap

use bevy::asset::{AssetEvent, AssetId, Assets, Handle};
use bevy::color::Color;
use bevy::ecs::system::SystemParam;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec4;
use bevy::mesh::Mesh2d;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, Material, MaterialExtension, MaterialPipeline, MaterialPipelineKey,
    MaterialPlugin,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{
    AlphaMode2d, ColorMaterial, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin,
    MeshMaterial2d,
};
use blockloom_core::material::{
    GraphEffect, MaskStack, SurfaceDetail, SurfaceMaterial, TextureSampler, TileMesh, Tilemap,
};
use blockloom_protocol::RuntimeMessage;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Register the graph materials and their embedded shaders. Every dimension
/// registers both: systems take their asset stores unconditionally, and an
/// unused plugin costs nothing at runtime. The tilemap material
/// (`ColorMaterial`) comes from `DefaultPlugins`.
pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/graph_2d.wesl");
    bevy::asset::embedded_asset!(app, "shaders/graph_3d.wesl");
    bevy::asset::embedded_asset!(app, "shaders/box_pbr.wesl");
    bevy::asset::embedded_asset!(app, "shaders/terrain.wesl");
    bevy::asset::embedded_asset!(app, "shaders/grass.wesl");
    bevy::asset::embedded_asset!(app, "shaders/water_surface.wesl");
    bevy::asset::embedded_asset!(app, "shaders/water_2d.wesl");
    bevy::asset::embedded_asset!(app, "shaders/underwater.wesl");
    app.add_plugins(Material2dPlugin::<GraphMaterial2d>::default());
    app.add_plugins(MaterialPlugin::<GraphMaterial3d>::default());
    app.add_plugins(MaterialPlugin::<BoxMaterial>::default());
    app.init_resource::<TextureVariants>();
    app.init_resource::<SurfaceGlobals>();
    app.add_systems(Update, (sync_texture_variants, update_surface_globals));
}

#[derive(Resource, Default)]
struct TextureVariants {
    variants: HashMap<AssetId<Image>, TextureVariant>,
    applied_bias: f32,
    applied_aniso: u8,
}

struct TextureVariant {
    source: Handle<Image>,
    sampler: TextureSampler,
    anisotropy: u8,
    srgb: bool,
}

fn sync_texture_variants(
    mut variants: ResMut<TextureVariants>,
    mut images: ResMut<Assets<Image>>,
    mut events: MessageReader<AssetEvent<Image>>,
    scaling: Option<Res<crate::quality::Scaling>>,
) {
    // One quality preset maps onto every streaming dial: the lod clamp
    // pushes mip selection coarser and the anisotropy cap trims bandwidth.
    // Baked data textures (cloud noise volumes, grading LUTs, lens dirt)
    // never register here, so they keep full resolution.
    let (bias, cap) = scaling.map_or((0.0, u8::MAX), |s| {
        let q = s.controller.quality;
        (q.texture_lod_bias(), q.anisotropy_cap())
    });
    let streaming_moved = bias != variants.applied_bias || cap != variants.applied_aniso;
    if streaming_moved {
        variants.applied_bias = bias;
        variants.applied_aniso = cap;
    }
    let changed: HashSet<_> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    for (id, request) in &mut variants.variants {
        if images.get(*id).is_some() && !streaming_moved && !changed.contains(&request.source.id())
        {
            continue;
        }
        let Some(mut image) = images.get(&request.source).cloned() else {
            continue;
        };
        let mode = match request.sampler {
            TextureSampler::Repeat => ImageAddressMode::Repeat,
            TextureSampler::Mirror => ImageAddressMode::MirrorRepeat,
            TextureSampler::Clamp => ImageAddressMode::ClampToEdge,
        };
        let mut descriptor = ImageSamplerDescriptor::linear();
        descriptor.address_mode_u = mode;
        descriptor.address_mode_v = mode;
        descriptor.anisotropy_clamp = u16::from(request.anisotropy.min(cap).max(1));
        descriptor.lod_min_clamp = bias;
        image.sampler = ImageSampler::Descriptor(descriptor);
        image.texture_descriptor.format = if request.srgb {
            image.texture_descriptor.format.add_srgb_suffix()
        } else {
            image.texture_descriptor.format.remove_srgb_suffix()
        };
        let _ = images.insert(*id, image);
    }
}

pub type BoxMaterial = ExtendedMaterial<StandardMaterial, BoxProjection>;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct BoxProjection {
    /// Tiling X/Y, offset X/Y.
    #[uniform(100)]
    pub scale_offset: Vec4,
    /// Rotation in radians, tiles per world unit, map flags.
    #[uniform(101)]
    pub options: Vec4,
    #[texture(102)]
    #[sampler(103)]
    pub albedo: Option<Handle<Image>>,
    #[texture(104)]
    #[sampler(105)]
    pub normal: Option<Handle<Image>>,
    #[texture(106)]
    #[sampler(107)]
    pub roughness: Option<Handle<Image>>,
    #[uniform(108)]
    pub masks: MaskUniforms,
    #[uniform(109)]
    pub detail: DetailUniforms,
    #[texture(110)]
    #[sampler(111)]
    pub macro_map: Option<Handle<Image>>,
    #[texture(112)]
    #[sampler(113)]
    pub detail_map: Option<Handle<Image>>,
    #[storage(114, read_only)]
    pub globals: Handle<ShaderBuffer>,
    #[texture(116)]
    pub surface_state: Option<Handle<Image>>,
}

impl MaterialExtension for BoxProjection {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
                "shaders/box_pbr.wesl"
            ))
            .with_source("embedded"),
        )
    }
}

pub fn terrain_shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/terrain.wesl"))
            .with_source("embedded"),
    )
}

pub fn grass_shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/grass.wesl"))
            .with_source("embedded"),
    )
}

pub fn water_shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
            "shaders/water_surface.wesl"
        ))
        .with_source("embedded"),
    )
}

pub fn water_2d_shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/water_2d.wesl"))
            .with_source("embedded"),
    )
}

pub fn underwater_shader() -> bevy::asset::AssetPath<'static> {
    bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/underwater.wesl"))
        .with_source("embedded")
}

/// `MaskUniforms` in `blockloom::texturing`, lane for lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct MaskUniforms {
    pub slope: Vec4,
    pub slope_color: Vec4,
    pub height: Vec4,
    pub height_color: Vec4,
    pub cavity: Vec4,
    pub cavity_color: Vec4,
    pub snow: Vec4,
    pub snow_color: Vec4,
    pub wetness: Vec4,
}

impl MaskUniforms {
    pub fn of(masks: &MaskStack) -> Self {
        let l = masks.pack().map(Vec4::from_array);
        Self {
            slope: l[0],
            slope_color: l[1],
            height: l[2],
            height_color: l[3],
            cavity: l[4],
            cavity_color: l[5],
            snow: l[6],
            snow_color: l[7],
            wetness: l[8],
        }
    }
}

/// `DetailUniforms` in `blockloom::texturing`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct DetailUniforms {
    pub tiling: Vec4,
    pub detail: Vec4,
}

impl DetailUniforms {
    pub fn of(detail: &SurfaceDetail, macro_map: bool) -> Self {
        let [tiling, lanes] = detail.pack(macro_map).map(Vec4::from_array);
        Self {
            tiling,
            detail: lanes,
        }
    }
}

/// `SurfaceGlobals` in `blockloom::texturing`: what every projected surface,
/// the terrain and the grass read each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SurfaceGlobalsData {
    /// Snow cover, wetness, debug view (1 blends), unused.
    pub weather: Vec4,
    /// Wind velocity x/z at the camera, its speed, gusts per second.
    pub wind: Vec4,
    pub surface_frame: Vec4,
}

/// The one buffer [`SurfaceGlobalsData`] lives in, shared by every material
/// that reads it.
#[derive(Resource, Clone)]
pub struct SurfaceGlobals {
    pub buffer: Handle<ShaderBuffer>,
    pub data: SurfaceGlobalsData,
}

impl FromWorld for SurfaceGlobals {
    fn from_world(world: &mut World) -> Self {
        let data = SurfaceGlobalsData::default();
        let buffer = world
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![data]));
        Self { buffer, data }
    }
}

/// Copy snow, wetness, the blend debug view and the camera's wind into the
/// shared buffer, only when something moved enough to see.
pub fn update_surface_globals(
    environment: Option<Res<crate::environment::Environment>>,
    debug: Option<Res<crate::hdr::HdrDebug>>,
    wind: Option<Res<crate::wind::WindField>>,
    destruction: Option<Res<crate::destruction::Destruction>>,
    mut globals: ResMut<SurfaceGlobals>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let (snow, wetness) = environment
        .map(|env| (env.snow, env.wetness))
        .unwrap_or_default();
    let blend = debug.is_some_and(|debug| debug.0 == blockloom_protocol::DebugView::SurfaceBlend);
    let (air, gusts) = wind
        .map(|field| (field.at_camera, field.wind.gust_frequency))
        .unwrap_or_default();
    let data = SurfaceGlobalsData {
        weather: Vec4::new(snow, wetness, f32::from(blend), 0.0),
        wind: Vec4::new(air.x, air.z, air.length(), gusts),
        surface_frame: destruction.map_or(Vec4::ZERO, |state| {
            Vec4::new(
                state.map.origin[0],
                state.map.origin[1],
                state.map.extent,
                0.0,
            )
        }),
    };
    if (data.weather - globals.data.weather).abs().max_element() < 1e-3
        && (data.wind - globals.data.wind).abs().max_element() < 0.05
        && data.surface_frame == globals.data.surface_frame
    {
        return;
    }
    globals.data = data;
    if let Some(mut buffer) = buffers.get_mut(&globals.buffer) {
        *buffer = ShaderBuffer::from(vec![data]);
    }
}

pub fn box_material(
    commands: &mut Commands,
    material: &SurfaceMaterial,
    tint: Color,
    dir: Option<&Path>,
    assets: &AssetServer,
) -> BoxMaterial {
    let mut base = surface_standard(commands, material, tint, dir, assets);
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    let detail = &material.detail;
    // Stochastic tiling samples albedo itself, projected or not.
    let albedo = if material.box_projection || detail.stochastic {
        base.base_color_texture.take()
    } else {
        None
    };
    let normal = if material.box_projection {
        base.normal_map_texture.take()
    } else {
        None
    };
    let roughness = base.metallic_roughness_texture.take();
    // Macro and detail maps always repeat, whatever the surface's sampler.
    let tiled = SurfaceMaterial {
        sampler: TextureSampler::Repeat,
        ..material.clone()
    };
    let macro_map = load_surface_image(commands, &detail.macro_texture, &tiled, dir, assets, false);
    let detail_map =
        load_surface_image(commands, &detail.detail_texture, &tiled, dir, assets, false);
    BoxMaterial {
        base,
        extension: BoxProjection {
            scale_offset: uv_scale_offset(material),
            options: Vec4::new(
                material.rotation.to_radians(),
                if material.box_projection {
                    material.texel_density
                } else {
                    0.0
                },
                f32::from(albedo.is_some()),
                f32::from(normal.is_some()) + 2.0 * f32::from(roughness.is_some()),
            ),
            albedo,
            normal,
            roughness,
            masks: MaskUniforms::of(&detail.masks),
            detail: DetailUniforms::of(detail, macro_map.is_some()),
            macro_map,
            detail_map,
            // Filled in from `SurfaceGlobals` where the material is stored.
            globals: Handle::default(),
            surface_state: None,
        },
    }
}

/// Which fragment shader a graph material draws with: the ubershader, or a
/// project's own surface shader. Part of the pipeline key, so two materials
/// with different files get different pipelines.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GraphKey {
    shader: Option<Handle<Shader>>,
}

/// One custom-shaded 2D actor: tint and second color, effect params, and the
/// look's own image when it has one.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GraphKey)]
pub struct GraphMaterial2d {
    #[uniform(0)]
    pub tint: Vec4,
    #[uniform(1)]
    pub secondary: Vec4,
    /// mode, speed, strength, time. Time is rewritten every frame.
    #[uniform(2)]
    pub params: Vec4,
    /// use_texture, rounded, 0, 0.
    #[uniform(3)]
    pub flags: Vec4,
    #[texture(4)]
    #[sampler(5)]
    pub texture: Option<Handle<Image>>,
    #[uniform(6)]
    pub uv_scale_offset: Vec4,
    #[uniform(7)]
    pub uv_options: Vec4,
    /// A project surface shader replacing the ubershader, if any.
    pub shader: Option<Handle<Shader>>,
}

impl From<&GraphMaterial2d> for GraphKey {
    fn from(material: &GraphMaterial2d) -> Self {
        Self {
            shader: material.shader.clone(),
        }
    }
}

impl Material2d for GraphMaterial2d {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
                "shaders/graph_2d.wesl"
            ))
            .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let (Some(shader), Some(fragment)) =
            (key.bind_group_data.shader, descriptor.fragment.as_mut())
        {
            fragment.shader = shader;
        }
        Ok(())
    }
}

/// One custom-shaded 3D actor. Unlit by design: an effect is its own light,
/// so the scene's lamps leave it alone.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GraphKey)]
pub struct GraphMaterial3d {
    #[uniform(0)]
    pub tint: Vec4,
    #[uniform(1)]
    pub secondary: Vec4,
    /// mode, speed, strength, time. Time is rewritten every frame.
    #[uniform(2)]
    pub params: Vec4,
    /// use_texture, 0, 0, 0.
    #[uniform(3)]
    pub flags: Vec4,
    #[texture(4)]
    #[sampler(5)]
    pub texture: Option<Handle<Image>>,
    #[uniform(6)]
    pub uv_scale_offset: Vec4,
    #[uniform(7)]
    pub uv_options: Vec4,
    /// A project surface shader replacing the ubershader, if any.
    pub shader: Option<Handle<Shader>>,
}

impl From<&GraphMaterial3d> for GraphKey {
    fn from(material: &GraphMaterial3d) -> Self {
        Self {
            shader: material.shader.clone(),
        }
    }
}

impl Material for GraphMaterial3d {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
                "shaders/graph_3d.wesl"
            ))
            .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let (Some(shader), Some(fragment)) =
            (key.bind_group_data.shader, descriptor.fragment.as_mut())
        {
            fragment.shader = shader;
        }
        Ok(())
    }
}

/// The shader an effect's `source` file compiles to, or `None` for the
/// built-in ubershader. A file that won't read or doesn't check is reported
/// against the actor and falls back to the ubershader, so a typo shows up in
/// the run log rather than as an actor that vanished.
///
/// The handle is keyed by the text itself, so every actor sharing a file
/// shares one pipeline, and an edited file is a new shader on the next build.
pub fn surface_shader(
    commands: &mut Commands,
    actor: &str,
    effect: &GraphEffect,
    dir: Option<&Path>,
    dim3: bool,
) -> Option<Handle<Shader>> {
    let source = effect.source.trim();
    if source.is_empty() {
        return None;
    }
    let report = |message: String| {
        crate::bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message,
        })
    };
    let path = crate::world::asset_path(dir, source);
    let text = match blockloom_core::vfs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            report(format!(
                "couldn't read the surface shader {source}: {error}"
            ));
            return None;
        }
    };
    if let Err(error) = blockloom_core::material::check_surface_wesl(&text, dim3) {
        report(format!("{source} doesn't compile:\n{error}"));
        return None;
    }
    let full = blockloom_core::material::surface_module(&text, dim3);
    let mut low = std::collections::hash_map::DefaultHasher::new();
    full.hash(&mut low);
    let mut high = std::collections::hash_map::DefaultHasher::new();
    ("blockloom-surface", &full).hash(&mut high);
    let uuid = bevy::asset::uuid::Uuid::from_u64_pair(high.finish(), low.finish());
    let handle = Handle::<Shader>::from(uuid);
    // The path is a module name, so no file name.
    let shader = Shader::from_wesl(full, format!("blockloom_surface/s{}.wesl", uuid.simple()));
    let id = handle.id();
    commands.queue(move |world: &mut World| {
        if let Some(mut shaders) = world.get_resource_mut::<Assets<Shader>>() {
            let _ = shaders.insert(id, shader);
        }
    });
    Some(handle)
}

/// Build the 2D material for an actor carrying `effect`.
pub fn graph_material_2d(
    material: &SurfaceMaterial,
    effect: &GraphEffect,
    tint: Color,
    secondary: Color,
    texture: Option<Handle<Image>>,
    rounded: bool,
    shader: Option<Handle<Shader>>,
) -> GraphMaterial2d {
    GraphMaterial2d {
        tint: tint.to_linear().to_vec4(),
        secondary: secondary.to_linear().to_vec4(),
        params: Vec4::new(
            effect.mode.mode_id() as f32,
            effect.speed,
            effect.strength,
            0.0,
        ),
        flags: Vec4::new(f32::from(texture.is_some()), f32::from(rounded), 0.0, 0.0),
        texture,
        uv_scale_offset: Vec4::new(
            material.tiling[0] * material.texel_density,
            material.tiling[1] * material.texel_density,
            material.offset[0],
            material.offset[1],
        ),
        uv_options: Vec4::new(
            material.rotation.to_radians(),
            0.0,
            material.texel_density,
            0.0,
        ),
        shader,
    }
}

/// Build the 3D material for an actor carrying `effect`.
pub fn graph_material_3d(
    material: &SurfaceMaterial,
    effect: &GraphEffect,
    tint: Color,
    secondary: Color,
    texture: Option<Handle<Image>>,
    shader: Option<Handle<Shader>>,
) -> GraphMaterial3d {
    GraphMaterial3d {
        tint: tint.to_linear().to_vec4(),
        secondary: secondary.to_linear().to_vec4(),
        params: Vec4::new(
            effect.mode.mode_id() as f32,
            effect.speed,
            effect.strength,
            0.0,
        ),
        flags: Vec4::new(f32::from(texture.is_some()), 0.0, 0.0, 0.0),
        texture,
        uv_scale_offset: uv_scale_offset(material),
        uv_options: uv_options(material),
        shader,
    }
}

/// Keep every graph's clock running: without this the effects freeze on
/// their first frame, since a material asset has no per-entity time.
pub fn tick_graph_time(
    time: Res<Time>,
    mut materials_2d: ResMut<Assets<GraphMaterial2d>>,
    mut materials_3d: ResMut<Assets<GraphMaterial3d>>,
) {
    let now = time.elapsed_secs();
    for (_, material) in materials_2d.iter_mut() {
        material.params.w = now;
    }
    for (_, material) in materials_3d.iter_mut() {
        material.params.w = now;
    }
}

/// Fold a [`SurfaceMaterial`] into the standard material a 3D actor renders
/// with. `tint` is the look's own color; the albedo texture multiplies over
/// it when one is authored.
pub fn surface_standard(
    commands: &mut Commands,
    material: &SurfaceMaterial,
    tint: Color,
    dir: Option<&Path>,
    assets: &AssetServer,
) -> StandardMaterial {
    let albedo = load_surface_image(
        commands,
        &material.albedo_texture,
        material,
        dir,
        assets,
        true,
    );
    let normal = load_surface_image(
        commands,
        &material.normal_texture,
        material,
        dir,
        assets,
        false,
    );
    let roughness = load_surface_image(
        commands,
        &material.roughness_texture,
        material,
        dir,
        assets,
        false,
    );
    StandardMaterial {
        base_color: tint,
        base_color_texture: albedo,
        normal_map_texture: normal,
        metallic_roughness_texture: roughness,
        uv_transform: uv_transform(material),
        metallic: material.metallic.clamp(0.0, 1.0),
        perceptual_roughness: material.roughness.clamp(0.0, 1.0),
        emissive: bevy::color::LinearRgba::from(crate::world::parse_color(&material.emissive))
            * material.emissive_energy.max(0.0),
        double_sided: material.double_sided,
        ..default()
    }
}

pub fn uv_scale_offset(material: &SurfaceMaterial) -> Vec4 {
    let density = if material.box_projection {
        1.0
    } else {
        material.texel_density
    };
    Vec4::new(
        material.tiling[0] * density,
        material.tiling[1] * density,
        material.offset[0],
        material.offset[1],
    )
}

pub fn uv_options(material: &SurfaceMaterial) -> Vec4 {
    Vec4::new(
        material.rotation.to_radians(),
        f32::from(material.box_projection),
        material.texel_density,
        0.0,
    )
}

pub fn uv_transform(material: &SurfaceMaterial) -> bevy::math::Affine2 {
    let density = if material.box_projection {
        1.0
    } else {
        material.texel_density
    };
    bevy::math::Affine2::from_translation(Vec2::from(material.offset))
        * bevy::math::Affine2::from_angle(material.rotation.to_radians())
        * bevy::math::Affine2::from_scale(Vec2::from(material.tiling) * density)
}

pub fn load_surface_image(
    commands: &mut Commands,
    name: &str,
    material: &SurfaceMaterial,
    dir: Option<&Path>,
    assets: &AssetServer,
    srgb: bool,
) -> Option<Handle<Image>> {
    if name.trim().is_empty() {
        return None;
    }
    let path = crate::world::asset_path(dir, name.trim());
    let source = assets.load::<Image>(path.clone());
    let mut low = std::collections::hash_map::DefaultHasher::new();
    (
        path.as_os_str(),
        material.sampler,
        material.anisotropy,
        srgb,
    )
        .hash(&mut low);
    let mut high = std::collections::hash_map::DefaultHasher::new();
    (
        "blockloom-texture",
        path.as_os_str(),
        material.sampler,
        material.anisotropy,
        srgb,
    )
        .hash(&mut high);
    let handle = Handle::<Image>::from(bevy::asset::uuid::Uuid::from_u64_pair(
        high.finish(),
        low.finish(),
    ));
    let id = handle.id();
    let sampler = material.sampler;
    let anisotropy = material.anisotropy;
    commands.queue(move |world: &mut World| {
        world.init_resource::<TextureVariants>();
        world.resource_mut::<TextureVariants>().variants.insert(
            id,
            TextureVariant {
                source,
                sampler,
                anisotropy,
                srgb,
            },
        );
    });
    Some(handle)
}

/// Turn a tilemap mesh into an uploadable Bevy mesh, with normals for the
/// 3D pipeline (+Z, since tilemaps face the viewer).
pub fn tilemesh_to_bevy(mesh: &TileMesh) -> Mesh {
    let mut bevy = Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    bevy.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        mesh.positions
            .iter()
            .map(|p| [p[0], p[1], p[2]])
            .collect::<Vec<_>>(),
    );
    bevy.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        mesh.normals
            .iter()
            .map(|n| [n[0], n[1], n[2]])
            .collect::<Vec<_>>(),
    );
    bevy.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        mesh.uvs.iter().map(|uv| [uv[0], uv[1]]).collect::<Vec<_>>(),
    );
    bevy.insert_indices(bevy::render::mesh::Indices::U32(mesh.indices.clone()));
    bevy
}

/// The 2D quad a custom-shaded look renders on, or `None` for a 3D-only
/// shape. Circles come out as quads; the shader discards the corners.
pub fn custom_quad_size(visual: &blockloom_core::scene::Visual) -> Option<Vec2> {
    use blockloom_core::scene::Visual;
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => Some(Vec2::new(size[0], size[1])),
        Visual::Circle { radius, .. } => Some(Vec2::splat(radius * 2.0)),
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Some(Vec2::new(size[0], size[1]))
        }
        _ => None,
    }
}

/// The material asset stores, as one system param: graph effects for both
/// dimensions plus the tilemap material. Bundled because a system takes at
/// most sixteen params, and the spawners already need most of them.
#[derive(SystemParam)]
pub struct MaterialStores<'w> {
    pub graph_2d: ResMut<'w, Assets<GraphMaterial2d>>,
    pub graph_3d: ResMut<'w, Assets<GraphMaterial3d>>,
    pub tiles: ResMut<'w, Assets<ColorMaterial>>,
}

/// A tilemap mesh with animated tiles: the map it was built from, the mesh
/// to rewrite, and which frames that mesh shows now.
#[derive(Component)]
pub struct AnimatedTiles {
    pub tilemap: Tilemap,
    pub mesh: Handle<Mesh>,
    shown: Vec<usize>,
}

impl AnimatedTiles {
    /// `None` for a map with nothing to animate, which then costs nothing.
    pub fn of(tilemap: &Tilemap, mesh: &Handle<Mesh>) -> Option<Self> {
        tilemap.is_animated().then(|| Self {
            tilemap: tilemap.clone(),
            mesh: mesh.clone(),
            shown: tilemap.frame_key(0.0),
        })
    }
}

/// Step every animated tilemap to the frame the clock says, rewriting only
/// the UVs and only when a frame actually turns over. Runs on the wall clock
/// like the graph effects, so water still ripples in the scene view.
pub fn animate_tiles(
    time: Res<Time>,
    mut maps: Query<&mut AnimatedTiles>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let now = time.elapsed_secs();
    for mut map in &mut maps {
        let key = map.tilemap.frame_key(now);
        if key == map.shown {
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(&map.mesh) else {
            continue;
        };
        let uvs = map.tilemap.build_mesh_at(now).uvs;
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        map.shown = key;
    }
}

/// The entity carrying a tilemap mesh, so systems can find it. Stored on the
/// actor entity itself; the mesh entity is its child.
#[derive(Component)]
pub struct TilemapMesh(pub Entity);

/// A 3D tilemap drawing on its own entity. Kept out of mesh batching, since
/// `paint tile` swaps its mesh mid-run.
#[derive(Component)]
pub struct TilemapLook;

/// Spawn a tilemap's mesh as a child of `entity`, or `None` when the map is
/// empty. 2D gets a `Mesh2d` with a tinted tileset; 3D a lit `Mesh` with the
/// same texture, double-sided so the wall reads from both sides.
pub fn spawn_tilemap_2d(
    commands: &mut Commands,
    entity: Entity,
    tilemap: &blockloom_core::material::Tilemap,
    dir: Option<&Path>,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) -> Option<Entity> {
    spawn_tilemap_mesh_2d(
        commands,
        entity,
        tilemap,
        &tilemap.build_mesh(),
        dir,
        assets,
        meshes,
        materials,
    )
}

/// [`spawn_tilemap_2d`] with the mesh already built, which is what a room
/// streaming in builds off the main thread.
#[allow(clippy::too_many_arguments)]
pub fn spawn_tilemap_mesh_2d(
    commands: &mut Commands,
    entity: Entity,
    tilemap: &blockloom_core::material::Tilemap,
    built: &TileMesh,
    dir: Option<&Path>,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) -> Option<Entity> {
    use bevy::sprite_render::ColorMaterial;
    if built.is_empty() {
        return None;
    }
    let texture = if tilemap.tileset.trim().is_empty() {
        None
    } else {
        Some(assets.load(crate::world::asset_path(dir, tilemap.tileset.trim())))
    };
    let mesh = meshes.add(tilemesh_to_bevy(built));
    let material = materials.add(ColorMaterial {
        color: Color::WHITE,
        alpha_mode: AlphaMode2d::Blend,
        texture,
        ..default()
    });
    let animated = AnimatedTiles::of(tilemap, &mesh);
    let child = commands
        .spawn((Mesh2d(mesh), MeshMaterial2d(material)))
        .id();
    if let Some(animated) = animated {
        commands.entity(child).insert(animated);
    }
    commands.entity(entity).add_child(child);
    commands.entity(entity).insert(TilemapMesh(child));
    Some(child)
}

/// See [`spawn_tilemap_2d`]: the 3D half, standing the map up as a wall.
#[allow(dead_code)] // Not wired into dim3 yet.
pub fn spawn_tilemap_3d(
    commands: &mut Commands,
    entity: Entity,
    tilemap: &blockloom_core::material::Tilemap,
    dir: Option<&Path>,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Entity> {
    let built = tilemap.build_mesh();
    if built.is_empty() {
        return None;
    }
    let texture = if tilemap.tileset.trim().is_empty() {
        None
    } else {
        Some(assets.load(crate::world::asset_path(dir, tilemap.tileset.trim())))
    };
    let mesh = meshes.add(tilemesh_to_bevy(&built));
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: texture,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let child = commands
        .spawn((Mesh3d(mesh), MeshMaterial3d(material)))
        .id();
    commands.entity(entity).add_child(child);
    commands.entity(entity).insert(TilemapMesh(child));
    Some(child)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    // Just enough of Bevy's PBR modules for naga to type-check a fragment.
    pub(crate) const PBR_STUB: &str = "struct View { world_position: vec3<f32> }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32> }\n\
struct FragmentOutput { @location(0) color: vec4<f32> }\n\
struct StandardMaterial { base_color: vec4<f32>, perceptual_roughness: f32 }\n\
struct PbrInput { material: StandardMaterial, world_normal: vec3<f32>, N: vec3<f32> }\n\
fn pbr_input_from_standard_material(in: VertexOutput, is_front: bool) -> PbrInput { var p: PbrInput; p.world_normal = in.world_normal; return p; }\n\
fn apply_decals(p: ptr<function, PbrInput>) {}\n\
fn apply_pbr_lighting(p: PbrInput) -> vec4<f32> { return p.material.base_color; }\n\
fn main_pass_post_lighting_processing(p: PbrInput, c: vec4<f32>) -> vec4<f32> { return c; }\n";

    /// Drops every `import bevy_*` statement and binds the material group.
    pub(crate) fn stubbed(source: &str, stub: &str) -> String {
        let mut out = String::new();
        let mut skipping = false;
        for line in source.lines() {
            if line.starts_with("import bevy_") {
                skipping = !line.trim_end().ends_with(';');
                continue;
            }
            if skipping {
                skipping = !line.trim_end().ends_with(';');
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out.replace("constants::MATERIAL_BIND_GROUP", "2") + stub
    }

    // The stubbed tests drop Bevy imports, so check their first segment names
    // one of bevy_pbr's shader modules; a wrong path skips the whole material.
    #[test]
    fn bevy_pbr_imports_name_real_modules() {
        const MODULES: &[&str] = &[
            "atmosphere",
            "cluster",
            "decal",
            "deferred",
            "light_probe",
            "lightmap",
            "meshlet",
            "prepass",
            "render",
            "ssao",
            "ssr",
            "transmission",
            "volumetric_fog",
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let source = std::fs::read_to_string(&path).unwrap();
            for (at, _) in source.match_indices("bevy_pbr::") {
                let rest = &source[at + "bevy_pbr::".len()..];
                if rest.starts_with('{') {
                    continue;
                }
                let module: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                assert!(
                    MODULES.contains(&module.as_str()),
                    "{} imports bevy_pbr::{module}, which isn't a module",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn the_box_shader_compiles() {
        for uvs in [false, true] {
            shader_lib::validate(
                &stubbed(include_str!("shaders/box_pbr.wesl"), PBR_STUB),
                &[("VERTEX_UVS_A", uvs)],
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    #[test]
    fn the_terrain_shader_compiles() {
        shader_lib::validate(
            &stubbed(include_str!("shaders/terrain.wesl"), PBR_STUB),
            &[("VERTEX_UVS_A", true)],
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    const VERTEX_STUB: &str = "struct View { world_position: vec3<f32> }\n\
struct Globals { time: f32 }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
@group(0) @binding(11) var<uniform> globals: Globals;\n\
struct Vertex { @builtin(instance_index) instance_index: u32, @location(0) position: vec3<f32>, \
@location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(5) color: vec4<f32> }\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(5) color: vec4<f32>, \
@location(6) @interpolate(flat) instance_index: u32 }\n\
fn decompress_vertex(v: Vertex, i: u32) -> Vertex { return v; }\n\
fn get_world_from_local(i: u32) -> mat4x4<f32> { return mat4x4<f32>(); }\n\
fn mesh_normal_local_to_world(n: vec3<f32>, i: u32) -> vec3<f32> { return n; }\n\
fn position_world_to_clip(p: vec3<f32>) -> vec4<f32> { return vec4<f32>(p, 1.0); }\n";

    #[test]
    fn the_grass_shader_compiles() {
        let defs = [
            ("VERTEX_UVS_A", true),
            ("VERTEX_COLORS", true),
            ("VERTEX_NORMALS", true),
            ("VERTEX_OUTPUT_INSTANCE_INDEX", true),
        ];
        shader_lib::validate(
            &stubbed(include_str!("shaders/grass.wesl"), VERTEX_STUB),
            &defs,
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    fn sampler_of(image: &Image) -> ImageSamplerDescriptor {
        match &image.sampler {
            ImageSampler::Descriptor(descriptor) => descriptor.clone(),
            _ => ImageSamplerDescriptor::linear(),
        }
    }

    #[test]
    fn quality_streaming_clamps_mips_and_anisotropy() {
        let mut app = App::new();
        app.init_resource::<TextureVariants>()
            .init_resource::<Assets<Image>>()
            .init_resource::<crate::quality::Scaling>()
            .add_message::<AssetEvent<Image>>()
            .add_systems(Update, sync_texture_variants);
        let source = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let handle = Handle::<Image>::from(bevy::asset::uuid::Uuid::from_u64_pair(1, 2));
        let id = handle.id();
        app.world_mut()
            .resource_mut::<TextureVariants>()
            .variants
            .insert(
                id,
                TextureVariant {
                    source: source.clone(),
                    sampler: TextureSampler::Repeat,
                    anisotropy: 16,
                    srgb: true,
                },
            );
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        let descriptor = images.get(id).map(sampler_of).unwrap();
        assert_eq!(descriptor.lod_min_clamp, 0.0);
        assert_eq!(descriptor.anisotropy_clamp, 8);
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .controller
            .quality = blockloom_core::quality::Quality::Low;
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        let descriptor = images.get(id).map(sampler_of).unwrap();
        assert_eq!(descriptor.lod_min_clamp, 2.0);
        assert_eq!(descriptor.anisotropy_clamp, 2);
    }
}
