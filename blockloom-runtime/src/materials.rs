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
//! PBR-only materials need no plugin: [`surface_standard`] folds a
//! [`SurfaceMaterial`] into the `StandardMaterial` the 3D pipeline already
//! uses. [`tilemesh_to_bevy`] turns a [`Tilemap`] into an uploadable mesh.
//!
//! [`GraphEffect`]: blockloom_core::material::GraphEffect
//! [`SurfaceMaterial`]: blockloom_core::material::SurfaceMaterial
//! [`Tilemap`]: blockloom_core::material::Tilemap

use bevy::asset::{Assets, Handle};
use bevy::color::Color;
use bevy::image::Image;
use bevy::math::Vec4;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy::sprite::{AlphaMode2d, Material2d, Material2dPlugin, Mesh2d};
use blockloom_core::material::{GraphEffect, SurfaceMaterial, TileMesh};
use std::path::Path;

/// Register the graph materials, the tilemap material, and their embedded
/// shaders. Every dimension registers all three: systems take their asset
/// stores unconditionally, and an unused plugin costs nothing at runtime.
pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/graph_2d.wgsl");
    bevy::asset::embedded_asset!(app, "shaders/graph_3d.wgsl");
    app.add_plugins(Material2dPlugin::<GraphMaterial2d>::default());
    app.add_plugins(MaterialPlugin::<GraphMaterial3d>::default());
    app.add_plugins(bevy::sprite::ColorMaterialPlugin);
}

/// One custom-shaded 2D actor: tint and second color, effect params, and the
/// look's own image when it has one.
#[derive(Asset, AsBindGroup, Clone)]
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
}

/// One custom-shaded 3D actor. Unlit by design: an effect is its own light,
/// so the scene's lamps leave it alone.
#[derive(Asset, AsBindGroup, Clone)]
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
}

/// Build the 2D material for an actor carrying `effect`.
pub fn graph_material_2d(
    effect: &GraphEffect,
    tint: Color,
    secondary: Color,
    texture: Option<Handle<Image>>,
    rounded: bool,
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
        flags: Vec4::new(
            f32::from(texture.is_some()),
            f32::from(rounded),
            0.0,
            0.0,
        ),
        texture,
    }
}

/// Build the 3D material for an actor carrying `effect`.
pub fn graph_material_3d(
    effect: &GraphEffect,
    tint: Color,
    secondary: Color,
    texture: Option<Handle<Image>>,
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
        emissive: bevy::color::LinearRgba::from(
            crate::world::parse_color(&material.emissive),
        ) * material.emissive_energy.max(0.0),
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
        mesh.positions.iter().map(|p| [p[0], p[1], p[2]]).collect::<Vec<_>>(),
    );
    bevy.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        mesh.normals.iter().map(|n| [n[0], n[1], n[2]]).collect::<Vec<_>>(),
    );
    bevy.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        mesh.uvs.iter().map(|uv| [uv[0], uv[1]]).collect::<Vec<_>>(),
    );
    bevy.set_indices(Some(bevy::render::mesh::Indices::U32(mesh.indices.clone())));
    bevy
}

/// The 2D quad a custom-shaded look renders on, or `None` for a 3D-only
/// shape. Circles come out as quads; the shader discards the corners.
pub fn custom_quad_size(visual: &blockloom_core::scene::Visual) -> Option<Vec2> {
    use blockloom_core::scene::Visual;
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => {
            Some(Vec2::new(size[0], size[1]))
        }
        Visual::Circle { radius, .. } => Some(Vec2::splat(radius * 2.0)),
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Some(Vec2::new(size[0], size[1]))
        }
        _ => None,
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
    use bevy::sprite::ColorMaterial;
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
    let child = commands
        .spawn((Mesh2d(mesh), bevy::sprite::MeshMaterial2d(material)))
        .id();
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
