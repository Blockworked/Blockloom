//! The Sprite component in 2D: flips, 9-slice panels, stacked slices, the
//! palette-swap and outline effect, and sort depth inside a Render layer.
//!
//! `SpriteDials` is the live copy of the authored spec; `set sprite [dial]`
//! writes it on the fixed tick (`anim2d::apply_animation_effects`), and
//! [`sync_sprites`] makes the actor's drawing match every frame. A stack or
//! the effect draws through children and hides the actor's own sprite with
//! an empty `RenderLayers`, the way batching hides a merged actor in 3D.
//! [`sync_part_palettes`] does the same for each stack slice and rig part,
//! palette only: an outline per piece would line the seams between them.

use bevy::camera::visibility::RenderLayers;
use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::sprite::{BorderRect, SliceScaleMode, SpriteImageMode, TextureSlicer};
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d};
use blockloom_core::blocks::SpriteDial;
use blockloom_core::sprite2d::{
    NineSlice, SliceFill, SpriteSpec, SpriteStack, clamp_outline, order_depth, stack_slice_offset,
    y_sort_depths,
};

use crate::engine::{ActorId, Engine};
use crate::world::{asset_path, parse_color};
use std::collections::HashMap;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/sprite_fx.wesl");
    app.add_plugins(Material2dPlugin::<SpriteFxMaterial>::default());
}

/// The live 2D dials on an actor.
#[derive(Component, Debug, Clone, Default)]
pub struct SpriteDials(pub SpriteSpec);

impl SpriteDials {
    /// What `set sprite [dial] to` does. Switches read nonzero as on.
    pub fn set(&mut self, dial: SpriteDial, value: f32) {
        let spec = &mut self.0;
        let on = value != 0.0 && value.is_finite();
        match dial {
            SpriteDial::FlipX => spec.flip_x = on,
            SpriteDial::FlipY => spec.flip_y = on,
            SpriteDial::YSort => spec.y_sort = on,
            SpriteDial::Order => {
                spec.order = if value.is_finite() {
                    value.round() as i32
                } else {
                    0
                };
                spec.normalize();
            }
            SpriteDial::Palette => {
                spec.palette_index = if value.is_finite() {
                    value.round().clamp(0.0, 255.0) as u32
                } else {
                    0
                };
            }
            SpriteDial::OutlineWidth => spec.outline_width = clamp_outline(value),
        }
    }
}

/// What [`sync_sprites`] built for an actor, so it can take it down again.
#[derive(Component, Default)]
pub struct SpriteDecor {
    stack: Vec<Entity>,
    fx: FxQuad,
    touched: bool,
}

/// An effect quad child: the entity, its material and the size its mesh is.
type FxQuad = Option<(Entity, Handle<SpriteFxMaterial>, Vec2)>;

/// The palette quad a stack slice or rig part draws through.
#[derive(Component, Default)]
pub struct PartFx(FxQuad);

/// One slice of a stacked sprite.
#[derive(Component)]
pub struct StackSlice;

/// The quad the palette/outline effect draws on.
#[derive(Component)]
pub struct SpriteFxQuad;

/// Depth between stacked slices, inside one order step.
const SLICE_DEPTH: f32 = 0.00005;

/// Palette swap and outline over one sprite frame.
#[derive(Asset, TypePath, AsBindGroup, Clone, PartialEq)]
pub struct SpriteFxMaterial {
    #[uniform(0)]
    pub tint: Vec4,
    #[uniform(1)]
    pub uv_rect: Vec4,
    /// Width, height, outline width, flags (1 flip x, 2 flip y, 4 palette).
    #[uniform(2)]
    pub shape: Vec4,
    #[uniform(3)]
    pub outline_color: Vec4,
    /// Palette row, then unused.
    #[uniform(4)]
    pub palette_params: Vec4,
    #[texture(5)]
    #[sampler(6)]
    pub texture: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub palette: Option<Handle<Image>>,
}

impl Material2d for SpriteFxMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
                "shaders/sprite_fx.wesl"
            ))
            .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

pub fn slicer(slice: &NineSlice) -> TextureSlicer {
    let fill = |fill: SliceFill| match fill {
        SliceFill::Stretch => SliceScaleMode::Stretch,
        SliceFill::Tile => SliceScaleMode::Tile { stretch_value: 1.0 },
    };
    let [left, right, top, bottom] = slice.border;
    TextureSlicer {
        border: BorderRect {
            min_inset: Vec2::new(left, top),
            max_inset: Vec2::new(right, bottom),
        },
        center_scale_mode: fill(slice.center),
        sides_scale_mode: fill(slice.sides),
        max_corner_scale: slice.max_corner_scale,
    }
}

/// The frame a sprite shows, as a size in world units and a UV rectangle,
/// once its image has loaded.
fn frame_of(sprite: &Sprite, images: &Assets<Image>) -> Option<(Vec2, Vec4)> {
    let image = images.get(&sprite.image)?.size_f32().max(Vec2::ONE);
    let rect = sprite.rect.unwrap_or(Rect::from_corners(Vec2::ZERO, image));
    let size = sprite.custom_size.unwrap_or(rect.size());
    Some((
        size,
        Vec4::new(
            rect.min.x / image.x,
            rect.min.y / image.y,
            rect.max.x / image.x,
            rect.max.y / image.y,
        ),
    ))
}

/// Makes every actor's drawing match its dials: flips and slicing on the
/// sprite, sort depth on z, and the stack and effect children. Runs after
/// the poses are interpolated and the camera has moved, each frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sync_sprites(
    mut commands: Commands,
    engine: NonSend<Engine>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SpriteFxMaterial>>,
    mut actors: Query<
        (
            Entity,
            &Transform,
            Option<&SpriteDials>,
            Option<&mut Sprite>,
            Option<&mut SpriteDecor>,
            Option<&RenderLayers>,
            Option<&crate::anim2d::RigInstance>,
        ),
        With<ActorId>,
    >,
    mut slices: Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<StackSlice>, Without<ActorId>),
    >,
) {
    let dir = engine.project_dir.clone();
    for (entity, transform, dials, sprite, decor, layers, rig) in &mut actors {
        let spec = dials.map(|dials| &dials.0);
        let rig_active = rig.is_some_and(|rig| rig.active);
        let drawn = sprite.is_some() && !rig_active;
        let stack = spec.and_then(|spec| spec.stack.as_ref()).filter(|_| drawn);
        let effect = spec.filter(|spec| drawn && stack.is_none() && spec.needs_effect());
        let hide = rig_active || stack.is_some() || effect.is_some();
        match (hide, layers) {
            (true, None) => {
                commands.entity(entity).insert(RenderLayers::none());
            }
            (false, Some(_)) => {
                commands.entity(entity).remove::<RenderLayers>();
            }
            _ => {}
        }
        if spec.is_none() && decor.is_none() {
            continue;
        }
        let mut decor = match decor {
            Some(decor) => decor,
            None => {
                commands.entity(entity).insert(SpriteDecor::default());
                continue;
            }
        };
        // Only written through when something differs, so an unchanged
        // sprite isn't re-extracted every frame.
        let mut sprite = sprite;
        if let Some(sprite) = sprite.as_mut() {
            let (flip_x, flip_y, mode) = match spec {
                Some(spec) => (
                    spec.flip_x,
                    spec.flip_y,
                    spec.slice.as_ref().map_or(SpriteImageMode::Auto, |slice| {
                        SpriteImageMode::Sliced(slicer(slice))
                    }),
                ),
                None => (false, false, SpriteImageMode::Auto),
            };
            if spec.is_some() || decor.touched {
                if sprite.flip_x != flip_x {
                    sprite.flip_x = flip_x;
                }
                if sprite.flip_y != flip_y {
                    sprite.flip_y = flip_y;
                }
                if sprite.image_mode != mode {
                    sprite.image_mode = mode;
                }
            }
        }
        decor.touched = spec.is_some();
        // Stacked slices, one child each.
        let wanted = stack.map_or(0, |stack| stack.layers as usize);
        while decor.stack.len() > wanted {
            if let Some(slice) = decor.stack.pop() {
                commands.entity(slice).despawn();
            }
        }
        while decor.stack.len() < wanted {
            let index = decor.stack.len();
            let slice = commands
                .spawn((
                    Sprite::default(),
                    StackSlice,
                    Transform::from_xyz(0.0, 0.0, index as f32 * SLICE_DEPTH),
                    Visibility::Hidden,
                ))
                .id();
            commands.entity(entity).add_child(slice);
            decor.stack.push(slice);
        }
        if let (Some(stack), Some(base)) = (stack, sprite.as_deref()) {
            draw_stack(
                stack,
                base,
                transform,
                &decor.stack,
                &mut slices,
                dir.as_deref(),
                &assets,
                &images,
            );
        }
        // The palette/outline quad.
        let frame = sprite
            .as_deref()
            .and_then(|sprite| frame_of(sprite, &images));
        match (effect, sprite.as_deref(), frame) {
            (Some(spec), Some(base), Some((size, uv_rect))) => {
                let width = spec.outline_width;
                let quad = size + Vec2::splat(width * 2.0);
                let palette = (!spec.palette.is_empty())
                    .then(|| assets.load(asset_path(dir.as_deref(), &spec.palette)));
                let material = fx_material(
                    base,
                    size,
                    uv_rect,
                    width,
                    parse_color(&spec.outline_color).to_linear().to_vec4(),
                    palette,
                    spec.palette_index,
                );
                place_quad(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    entity,
                    &mut decor.fx,
                    material,
                    quad,
                    SLICE_DEPTH,
                );
            }
            // Still loading: keep what was drawn.
            (Some(_), Some(_), None) => {}
            _ => {
                if let Some((quad_entity, _, _)) = decor.fx.take() {
                    commands.entity(quad_entity).despawn();
                }
            }
        }
    }
}

/// The effect material for `base`'s frame. Flips come off the sprite.
fn fx_material(
    base: &Sprite,
    size: Vec2,
    uv_rect: Vec4,
    outline_width: f32,
    outline_color: Vec4,
    palette: Option<Handle<Image>>,
    palette_row: u32,
) -> SpriteFxMaterial {
    let flags =
        f32::from(base.flip_x) + 2.0 * f32::from(base.flip_y) + 4.0 * f32::from(palette.is_some());
    SpriteFxMaterial {
        tint: base.color.to_linear().to_vec4(),
        uv_rect,
        shape: Vec4::new(size.x, size.y, outline_width, flags),
        outline_color,
        palette_params: Vec4::new(palette_row as f32, 0.0, 0.0, 0.0),
        texture: Some(base.image.clone()),
        palette,
    }
}

/// Points `slot`'s quad at `material`, making it under `parent` if there is
/// none yet. Assets are only written when they differ.
#[allow(clippy::too_many_arguments)]
fn place_quad(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SpriteFxMaterial>,
    parent: Entity,
    slot: &mut FxQuad,
    material: SpriteFxMaterial,
    quad: Vec2,
    depth: f32,
) {
    match slot {
        Some((quad_entity, handle, drawn_quad)) => {
            if materials
                .get(&*handle)
                .is_some_and(|existing| *existing != material)
                && let Some(mut existing) = materials.get_mut(&*handle)
            {
                *existing = material;
            }
            if *drawn_quad != quad {
                *drawn_quad = quad;
                commands
                    .entity(*quad_entity)
                    .insert(Mesh2d(meshes.add(Rectangle::new(quad.x, quad.y))));
            }
        }
        None => {
            let handle = materials.add(material);
            let quad_entity = commands
                .spawn((
                    Mesh2d(meshes.add(Rectangle::new(quad.x, quad.y))),
                    MeshMaterial2d(handle.clone()),
                    SpriteFxQuad,
                    Transform::from_xyz(0.0, 0.0, depth),
                ))
                .id();
            commands.entity(parent).add_child(quad_entity);
            *slot = Some((quad_entity, handle, quad));
        }
    }
}

/// Gives each stack slice and rig part of a palette-swapped actor its own
/// palette quad, and takes it away again when the palette goes.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sync_part_palettes(
    mut commands: Commands,
    engine: NonSend<Engine>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SpriteFxMaterial>>,
    actors: Query<
        (
            &SpriteDials,
            Option<&SpriteDecor>,
            Option<&crate::anim2d::RigInstance>,
        ),
        With<ActorId>,
    >,
    mut parts: Query<
        (Entity, &Sprite, Option<&mut PartFx>, Has<RenderLayers>),
        Or<(With<StackSlice>, With<crate::anim2d::RigPart>)>,
    >,
) {
    let dir = engine.project_dir.clone();
    let mut wanted: HashMap<Entity, (Handle<Image>, u32)> = HashMap::default();
    for (dials, decor, rig) in &actors {
        let spec = &dials.0;
        if spec.palette.is_empty() {
            continue;
        }
        let palette: Handle<Image> = assets.load(asset_path(dir.as_deref(), &spec.palette));
        let rig_parts = rig.filter(|rig| rig.active).map(|rig| rig.parts.as_slice());
        let slices = decor.map(|decor| decor.stack.as_slice());
        for part in rig_parts.into_iter().chain(slices).flatten() {
            wanted.insert(*part, (palette.clone(), spec.palette_index));
        }
    }
    for (entity, sprite, fx, hidden) in &mut parts {
        match (wanted.remove(&entity), fx) {
            (Some((palette, row)), fx) => {
                let Some((size, uv_rect)) = frame_of(sprite, &images) else {
                    continue;
                };
                let material =
                    fx_material(sprite, size, uv_rect, 0.0, Vec4::ZERO, Some(palette), row);
                match fx {
                    Some(mut fx) => place_quad(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        entity,
                        &mut fx.0,
                        material,
                        size,
                        0.0,
                    ),
                    None => {
                        let mut slot = None;
                        place_quad(
                            &mut commands,
                            &mut meshes,
                            &mut materials,
                            entity,
                            &mut slot,
                            material,
                            size,
                            0.0,
                        );
                        commands.entity(entity).insert(PartFx(slot));
                    }
                }
                if !hidden {
                    commands.entity(entity).insert(RenderLayers::none());
                }
            }
            (None, Some(mut fx)) => {
                if let Some((quad_entity, _, _)) = fx.0.take() {
                    commands.entity(quad_entity).despawn();
                }
                commands.entity(entity).remove::<(PartFx, RenderLayers)>();
            }
            (None, None) => {}
        }
    }
}

/// The sort depth an actor's z is carrying for this frame's render only.
#[derive(Component)]
pub struct SortDepth(f32);

/// Adds each dialed actor's order and Y-sort to its z just before transforms
/// propagate. It comes off again in [`clear_sort_depth`] at the top of the
/// next frame, so no pose, drag or physics step ever sees it.
pub fn apply_sort_depth(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut Transform, &SpriteDials), With<ActorId>>,
) {
    // Y-sort ranks among actors sharing one layer z and one order.
    let mut groups: HashMap<(u32, i32), Vec<(Entity, f32)>> = HashMap::default();
    for (entity, transform, dials) in &actors {
        let spec = &dials.0;
        if spec.y_sort {
            let z = transform.translation.z + 0.0;
            groups
                .entry((z.to_bits(), spec.order))
                .or_default()
                .push((entity, transform.translation.y));
        }
    }
    let mut ranked: HashMap<Entity, f32> = HashMap::default();
    for members in groups.values() {
        let heights: Vec<f32> = members.iter().map(|(_, y)| *y).collect();
        for ((entity, _), depth) in members.iter().zip(y_sort_depths(&heights)) {
            ranked.insert(*entity, depth);
        }
    }
    for (entity, mut transform, dials) in &mut actors {
        let offset = order_depth(dials.0.order) + ranked.get(&entity).copied().unwrap_or(0.0);
        if offset != 0.0 {
            transform.translation.z += offset;
            commands.entity(entity).insert(SortDepth(offset));
        }
    }
}

/// Takes the render-only sort depth back off.
pub fn clear_sort_depth(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut Transform, &SortDepth)>,
) {
    for (entity, mut transform, depth) in &mut actors {
        transform.translation.z -= depth.0;
        commands.entity(entity).remove::<SortDepth>();
    }
}

/// Lays each slice of a stack over the one below, offset up the screen
/// whichever way the actor faces.
#[allow(clippy::too_many_arguments)]
fn draw_stack(
    stack: &SpriteStack,
    base: &Sprite,
    transform: &Transform,
    parts: &[Entity],
    slices: &mut Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<StackSlice>, Without<ActorId>),
    >,
    dir: Option<&std::path::Path>,
    assets: &AssetServer,
    images: &Assets<Image>,
) {
    let image = if stack.image.is_empty() {
        base.image.clone()
    } else {
        assets.load(asset_path(dir, &stack.image))
    };
    let turn = transform.rotation.to_euler(EulerRot::ZYX).0;
    let scale = transform.scale.truncate().max(Vec2::splat(1e-4));
    for (index, part) in parts.iter().enumerate() {
        let Ok((mut sprite, mut local, mut visibility)) = slices.get_mut(*part) else {
            continue;
        };
        let Some(rect) =
            crate::anim2d::cell_rect(images, &image, index as u32, 1, stack.layers.max(1))
        else {
            continue;
        };
        let offset = stack_slice_offset(stack, index as u32, turn);
        let next = Vec3::new(
            offset[0] / scale.x,
            offset[1] / scale.y,
            index as f32 * SLICE_DEPTH,
        );
        if local.translation != next {
            local.translation = next;
        }
        let drawn = Sprite {
            image: image.clone(),
            rect: Some(rect),
            custom_size: base.custom_size,
            color: base.color,
            flip_x: base.flip_x,
            flip_y: base.flip_y,
            ..default()
        };
        if sprite.image != drawn.image
            || sprite.rect != drawn.rect
            || sprite.custom_size != drawn.custom_size
            || sprite.color != drawn.color
            || sprite.flip_x != drawn.flip_x
            || sprite.flip_y != drawn.flip_y
        {
            *sprite = drawn;
        }
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::shader_lib;

    const SPRITE_STUB: &str = "struct View { world_position: vec3<f32> }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world_position: vec4<f32>, \
@location(1) world_normal: vec3<f32>, @location(2) uv: vec2<f32> }\n";

    #[test]
    fn the_sprite_effect_shader_compiles() {
        // The output conversions are guarded imports of Bevy's; with the
        // imports stubbed away their guards go too, and the defs stay off.
        let source = include_str!("shaders/sprite_fx.wesl")
            .lines()
            .filter(|line| !line.starts_with("@if("))
            .collect::<Vec<_>>()
            .join("\n");
        shader_lib::validate(
            &crate::materials::tests::stubbed(&source, SPRITE_STUB),
            &[
                ("TONEMAP_IN_SHADER", false),
                ("SRGB_OUTPUT", false),
                ("OKLAB_OUTPUT", false),
            ],
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn dials_take_block_values() {
        let mut dials = SpriteDials::default();
        dials.set(SpriteDial::FlipX, 1.0);
        dials.set(SpriteDial::Order, 99.4);
        dials.set(SpriteDial::OutlineWidth, -3.0);
        dials.set(SpriteDial::Palette, 2.6);
        assert!(dials.0.flip_x);
        assert_eq!(dials.0.order, blockloom_core::sprite2d::ORDER_LIMIT);
        assert_eq!(dials.0.outline_width, 0.0);
        assert_eq!(dials.0.palette_index, 3);
        dials.set(SpriteDial::FlipX, 0.0);
        assert!(!dials.0.flip_x);
    }

    #[test]
    fn slices_map_left_right_top_bottom() {
        let slicer = slicer(&NineSlice {
            border: [1.0, 2.0, 3.0, 4.0],
            center: SliceFill::Tile,
            sides: SliceFill::Stretch,
            max_corner_scale: 1.0,
        });
        assert_eq!(slicer.border.min_inset, Vec2::new(1.0, 3.0));
        assert_eq!(slicer.border.max_inset, Vec2::new(2.0, 4.0));
        assert!(matches!(
            slicer.center_scale_mode,
            SliceScaleMode::Tile { .. }
        ));
    }

    fn dialed(y_sort: bool, order: i32, palette: &str) -> SpriteDials {
        SpriteDials(SpriteSpec {
            y_sort,
            order,
            palette: palette.to_string(),
            ..SpriteSpec::default()
        })
    }

    #[test]
    fn y_sort_orders_actors_far_apart_and_comes_back_off() {
        let mut app = App::new();
        app.add_systems(Update, apply_sort_depth);
        app.add_systems(PostUpdate, clear_sort_depth);
        let mut spawn = |y: f32, order: i32| {
            app.world_mut()
                .spawn((
                    ActorId(format!("a{y}")),
                    Transform::from_xyz(0.0, y, 3.0),
                    dialed(true, order, ""),
                ))
                .id()
        };
        let [far_up, far_down, near, ranked] = [
            spawn(90_000.0, 0),
            spawn(-90_000.0, 0),
            spawn(10.0, 0),
            spawn(90_000.0, 1),
        ];
        app.world_mut().run_schedule(Update);
        let z = |app: &App, entity| app.world().get::<Transform>(entity).unwrap().translation.z;
        assert!(z(&app, far_down) > z(&app, near));
        assert!(z(&app, near) > z(&app, far_up));
        // A higher order beats any height.
        assert!(z(&app, ranked) > z(&app, far_down));
        app.world_mut().run_schedule(PostUpdate);
        for entity in [far_up, far_down, near, ranked] {
            assert_eq!(z(&app, entity), 3.0);
        }
    }

    #[test]
    fn stack_slices_take_the_palette_and_give_it_back() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ));
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        app.init_asset::<SpriteFxMaterial>();
        app.insert_non_send(Engine::new(incoming, blockloom_core::scene::Mode::TwoD));
        app.add_systems(Update, sync_part_palettes);
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let slices: Vec<Entity> = (0..2)
            .map(|_| {
                app.world_mut()
                    .spawn((
                        StackSlice,
                        Sprite::from_image(image.clone()),
                        Transform::default(),
                    ))
                    .id()
            })
            .collect();
        let actor = app
            .world_mut()
            .spawn((
                ActorId("stack".to_string()),
                dialed(false, 0, "assets/palette.png"),
                SpriteDecor {
                    stack: slices.clone(),
                    ..SpriteDecor::default()
                },
            ))
            .id();
        app.update();
        let mut quads = Vec::new();
        for slice in &slices {
            let fx = app.world().get::<PartFx>(*slice).unwrap();
            let (quad, _, _) = fx.0.as_ref().unwrap();
            quads.push(*quad);
            assert!(app.world().get::<RenderLayers>(*slice).is_some());
        }
        // A second frame reuses the quads rather than making more.
        app.update();
        let again = app.world().get::<PartFx>(slices[0]).unwrap();
        assert_eq!(again.0.as_ref().unwrap().0, quads[0]);
        app.world_mut()
            .get_mut::<SpriteDials>(actor)
            .unwrap()
            .0
            .palette
            .clear();
        app.update();
        for (slice, quad) in slices.iter().zip(quads) {
            assert!(app.world().get::<PartFx>(*slice).is_none());
            assert!(app.world().get::<RenderLayers>(*slice).is_none());
            assert!(app.world().get_entity(quad).is_err());
        }
    }
}
