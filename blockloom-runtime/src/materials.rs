//! Custom shader materials, tilemap meshes and PBR helpers.
//!
//! [`GraphMaterial2d`] and [`GraphMaterial3d`] render a [`GraphEffect`] each
//! through one shared ubershader (`shaders/graph_2d.wgsl` and
//! `shaders/graph_3d.wgsl`, embedded at compile time). The uniforms carry the
//! look's tint, the effect's second color, and `(mode, speed, strength,
//! time)`; `time` is refreshed every frame by [`tick_graph_time`], so motion
//! never needs the actor to do anything. A textured look keeps its image
//! under the effect; a circle look renders its quad round.
//!
//! An effect whose `source` names a `.wgsl` asset draws with that file
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

use bevy::asset::{Assets, Handle};
use bevy::color::Color;
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::math::Vec4;
use bevy::mesh::Mesh2d;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{
    AlphaMode2d, ColorMaterial, Material2d, Material2dKey, Material2dPlugin, MeshMaterial2d,
};
use blockloom_core::material::{GraphEffect, SurfaceMaterial, TileMesh, Tilemap};
use blockloom_protocol::RuntimeMessage;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Register the graph materials and their embedded shaders. Every dimension
/// registers both: systems take their asset stores unconditionally, and an
/// unused plugin costs nothing at runtime. The tilemap material
/// (`ColorMaterial`) comes from `DefaultPlugins`.
pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/graph_2d.wgsl");
    bevy::asset::embedded_asset!(app, "shaders/graph_3d.wgsl");
    app.add_plugins(Material2dPlugin::<GraphMaterial2d>::default());
    app.add_plugins(MaterialPlugin::<GraphMaterial3d>::default());
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
                "shaders/graph_2d.wgsl"
            ))
            .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
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
                "shaders/graph_3d.wgsl"
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

/// The imports and fragment entry a project surface shader is wrapped in,
/// per dimension. The bindings between them are core's, the same text the
/// editor checks a file against.
const SURFACE_2D_HEAD: &str = "\
#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
    mesh2d_view_bindings::view,
}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
#ifdef SRGB_OUTPUT
#import bevy_render::color_operations::linear_to_srgb
#endif
#ifdef OKLAB_OUTPUT
#import bevy_render::color_operations::linear_rgb_to_oklab
#endif
";

const SURFACE_2D_TAIL: &str = "\
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    if (flags.y > 0.5 && length((mesh.uv - 0.5) * 2.0) > 1.0) {
        discard;
    }
    var output_color = graph_main(mesh.uv, params.w);
#ifdef TONEMAP_IN_SHADER
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
#endif
#ifdef SRGB_OUTPUT
    output_color = vec4(linear_to_srgb(output_color.rgb), output_color.a);
#endif
#ifdef OKLAB_OUTPUT
    output_color = vec4(linear_rgb_to_oklab(output_color.rgb), output_color.a);
#endif
    return output_color;
}
";

const SURFACE_3D_HEAD: &str = "\
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping
#endif
";

const SURFACE_3D_TAIL: &str = "\
@fragment
fn fragment(mesh: VertexOutput) -> FragmentOutput {
    var output_color = graph_main(mesh.uv, params.w);
#ifdef TONEMAP_IN_SHADER
    output_color = tonemapping::tone_mapping(output_color, view.color_grading);
#endif
    var out: FragmentOutput;
    out.color = output_color;
    return out;
}
";

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
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            report(format!(
                "couldn't read the surface shader {source}: {error}"
            ));
            return None;
        }
    };
    if let Err(error) = blockloom_core::material::check_surface_wgsl(&text) {
        report(format!("{source} doesn't compile:\n{error}"));
        return None;
    }
    let (head, tail) = if dim3 {
        (SURFACE_3D_HEAD, SURFACE_3D_TAIL)
    } else {
        (SURFACE_2D_HEAD, SURFACE_2D_TAIL)
    };
    let full = format!(
        "{head}\n{}\n{text}\n{tail}",
        blockloom_core::material::SURFACE_BINDINGS
    );
    let mut low = std::collections::hash_map::DefaultHasher::new();
    full.hash(&mut low);
    let mut high = std::collections::hash_map::DefaultHasher::new();
    ("blockloom-surface", &full).hash(&mut high);
    let uuid = bevy::asset::uuid::Uuid::from_u64_pair(high.finish(), low.finish());
    let handle = Handle::<Shader>::from(uuid);
    let shader = Shader::from_wgsl(full, format!("blockloom://surface/{source}"));
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
        shader,
    }
}

/// Build the 3D material for an actor carrying `effect`.
pub fn graph_material_3d(
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
    material: &SurfaceMaterial,
    tint: Color,
    dir: Option<&Path>,
    assets: &AssetServer,
) -> StandardMaterial {
    let albedo = if material.albedo_texture.trim().is_empty() {
        None
    } else {
        let path = crate::world::asset_path(dir, material.albedo_texture.trim());
        Some(assets.load(path))
    };
    StandardMaterial {
        base_color: tint,
        base_color_texture: albedo,
        metallic: material.metallic.clamp(0.0, 1.0),
        perceptual_roughness: material.roughness.clamp(0.0, 1.0),
        emissive: bevy::color::LinearRgba::from(crate::world::parse_color(&material.emissive))
            * material.emissive_energy.max(0.0),
        double_sided: material.double_sided,
        ..default()
    }
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
    use bevy::sprite_render::ColorMaterial;
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
