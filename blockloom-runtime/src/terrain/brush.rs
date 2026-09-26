//! The scene view's terrain brush. While the left button is down with the
//! Brush tool on a terrain, each stamp lands on a working copy of its grids
//! and the chunks, weights and surface map it touched are redrawn on the
//! spot. On release the stroke goes to the editor, which applies it again
//! to the saved grids, and the result is filed in the cache under the
//! document the editor will send back, so that reload draws at once.

use super::material::TerrainMaterial;
use super::{Built, Geometry, TerrainCache, TerrainChunk, Terrained, paint_from, raycast};
use crate::culling::LodGroup;
use crate::edit::{SceneEditor, editing};
use crate::engine::Engine;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use blockloom_core::terrain::mesh::chunk_mesh;
use blockloom_core::terrain::sculpt::{self, BrushTarget, Dirty, Editable, Stroke};
use blockloom_core::terrain::store::{self, Grid};
use blockloom_core::terrain::bake_weight;
use blockloom_protocol::{RuntimeMessage, SceneTool};
use std::sync::Arc;

/// Samples past a height change whose baked maps can move: curvature looks
/// up to eight samples out.
const REACH: u32 = 9;

#[derive(Resource, Default)]
pub struct LiveStroke(Option<Live>);

struct Live {
    actor: String,
    root: Entity,
    stroke: Stroke,
    grids: Editable,
    weights: Vec<[u8; 4]>,
    surface: Vec<[u8; 4]>,
    last: Vec2,
}

#[allow(clippy::too_many_arguments)]
pub fn paint(
    mut commands: Commands,
    mut editor: ResMut<SceneEditor>,
    engine: NonSend<Engine>,
    mut live: ResMut<LiveStroke>,
    mut cache: ResMut<TerrainCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    materials: Res<Assets<TerrainMaterial>>,
    roots: Query<(&Transform, &Terrained)>,
    mut chunks: Query<(&LodGroup, &mut Transform), (With<TerrainChunk>, Without<Terrained>)>,
    mut gizmos: Gizmos,
) {
    let target = editor
        .selected
        .clone()
        .filter(|_| editing(&engine, &editor) && editor.view.tool == SceneTool::Brush)
        .and_then(|id| Some((engine.entities.get(&id).copied()?, id)))
        .filter(|(root, _)| roots.get(*root).is_ok_and(|(_, t)| t.built.is_some()));
    let Some((root, actor)) = target else {
        if let Some(done) = live.0.take() {
            finish(&mut editor, &engine, &mut cache, &roots, done);
        }
        return;
    };
    if live.0.as_ref().is_some_and(|l| l.actor != actor) {
        let done = live.0.take().unwrap();
        finish(&mut editor, &engine, &mut cache, &roots, done);
    }
    let Ok((transform, terrained)) = roots.get(root) else {
        return;
    };
    let Some(built) = terrained.built.clone() else {
        return;
    };
    let geometry = &built.geometry;
    let shape = geometry.shape;
    let affine = transform.compute_affine();
    let inverse = affine.inverse();
    let field = live.0.as_ref().map_or(&geometry.field, |l| &l.grids.heights);
    let hit = editor.pointer_ray.and_then(|ray| {
        let origin = inverse.transform_point3(ray.origin);
        let direction = inverse.transform_vector3(*ray.direction).normalize_or_zero();
        raycast(field, &shape, origin, direction, 20_000.0)
    });
    let brush = editor.view.brush;
    if let Some(at) = hit {
        let centre = affine.transform_point3(at + Vec3::Y * 0.05);
        let up = affine.transform_vector3(Vec3::Y).normalize_or_zero();
        let scale = transform.scale.x.abs().max(1e-4);
        let facing = Quat::from_rotation_arc(Vec3::Z, up);
        let color = if editor.brushing {
            Color::srgb(1.0, 0.75, 0.2)
        } else {
            Color::srgb(0.9, 0.9, 0.95)
        };
        gizmos
            .circle(Isometry3d::new(centre, facing), brush.radius * scale, color)
            .resolution(48);
        gizmos
            .circle(
                Isometry3d::new(centre, facing),
                brush.radius * scale * (1.0 - brush.falloff).max(0.05),
                color.with_alpha(0.5),
            )
            .resolution(32);
    }
    if !editor.brushing {
        if let Some(done) = live.0.take() {
            finish(&mut editor, &engine, &mut cache, &roots, done);
        }
        return;
    }
    let Some(at) = hit else {
        return;
    };
    let point = Vec2::new(at.x, at.z);
    let spacing = shape.spacing()[0].min(shape.spacing()[1]);
    let state = live.0.get_or_insert_with(|| Live {
        actor: actor.clone(),
        root,
        stroke: Stroke {
            brush,
            stamps: Vec::new(),
        },
        grids: Editable {
            heights: geometry.field.clone(),
            splat: built.splat.clone(),
            holes: geometry.holes.clone(),
            grass: built.grass_maps.clone(),
            scatter: built.scatter_maps.clone(),
        },
        weights: built.weights.as_ref().clone(),
        surface: geometry.surface.clone(),
        last: Vec2::splat(f32::MAX),
    });
    if state.last.distance(point) < (brush.radius * 0.2).max(spacing) {
        return;
    }
    state.last = point;
    let stamp = [point.x, point.y];
    if state.stroke.stamps.is_empty() {
        state.stroke.stamps.push(stamp);
        sculpt::settle_level(&mut state.stroke, &state.grids.heights, &shape);
    } else {
        state.stroke.stamps.push(stamp);
    }
    let one = Stroke {
        brush: state.stroke.brush,
        stamps: vec![stamp],
    };
    let Some(dirty) = state.grids.apply(&shape, &one) else {
        return;
    };
    let Some(material) = materials.get(&terrained.material) else {
        return;
    };
    let reshaped = matches!(one.brush.target, BrushTarget::Heights | BrushTarget::Holes);
    let region = if reshaped { grow(dirty, REACH, shape.side) } else { dirty };
    rebake(state, &terrained.spec, &shape, region, reshaped);
    if let Some(mut image) = images.get_mut(&material.extension.weights) {
        image.data = Some(state.weights.iter().flatten().copied().collect());
    }
    if reshaped {
        if let Some(mut image) = images.get_mut(&material.extension.surface) {
            image.data = Some(state.surface.iter().flatten().copied().collect());
        }
        remesh(
            &mut commands,
            state,
            geometry,
            grow(dirty, 1, shape.side),
            &terrained.chunks,
            &mut chunks,
            &mut meshes,
        );
    }
}

fn grow(dirty: Dirty, by: u32, side: u32) -> Dirty {
    Dirty {
        min: dirty.min.map(|v| v.saturating_sub(by)),
        max: dirty.max.map(|v| (v + by).min(side - 1)),
    }
}

/// Re-bakes weights (and, after a height change, the surface map) over
/// `region`.
fn rebake(state: &mut Live, spec: &blockloom_core::terrain::TerrainSpec, shape: &blockloom_core::terrain::Shape, region: Dirty, reshaped: bool) {
    let painted = state.grids.splat.as_ref().map(|grid| grid.bytes.as_chunks::<4>().0);
    let field = &state.grids.heights;
    for j in region.min[1]..=region.max[1] {
        for i in region.min[0]..=region.max[0] {
            let k = field.index(i, j);
            state.weights[k] = bake_weight(field, shape, &spec.layers, painted, i, j);
            if reshaped {
                state.surface[k] = field.surface_texel(shape, i, j);
            }
        }
    }
}

/// Rebuilds every level of the chunks `region` reaches, in place, so the LOD
/// groups keep their handles.
fn remesh(
    commands: &mut Commands,
    state: &Live,
    geometry: &Geometry,
    region: Dirty,
    entities: &[Entity],
    chunks: &mut Query<(&LodGroup, &mut Transform), (With<TerrainChunk>, Without<Terrained>)>,
    meshes: &mut Assets<Mesh>,
) {
    let layout = geometry.layout;
    let quads = layout.quads;
    let holes = state.grids.holes.as_ref().and_then(Grid::mask);
    let (cx0, cz0) = (region.min[0] / quads, region.min[1] / quads);
    let (cx1, cz1) = (
        (region.max[0] / quads).min(layout.chunks - 1),
        (region.max[1] / quads).min(layout.chunks - 1),
    );
    for cz in cz0..=cz1 {
        for cx in cx0..=cx1 {
            let index = (cz * layout.chunks + cx) as usize;
            let Some(&entity) = entities.get(index) else {
                continue;
            };
            let Ok((group, mut transform)) = chunks.get_mut(entity) else {
                continue;
            };
            let first = layout.levels as usize - group.levels().len();
            let mut origin = None;
            for (k, level) in group.levels().iter().enumerate() {
                let Some(handle) = &level.mesh else {
                    continue;
                };
                let (mesh, at) = chunk_mesh(
                    &state.grids.heights,
                    &geometry.shape,
                    &layout,
                    cx,
                    cz,
                    (first + k) as u32,
                    holes,
                    geometry.skirt,
                );
                let _ = meshes.insert(handle, super::to_bevy(&mesh));
                origin = Some(at);
            }
            if let Some(at) = origin {
                transform.translation = Vec3::from(at);
                // The bounds are worked out again from the new shape.
                commands.entity(entity).remove::<Aabb>();
            }
        }
    }
}

/// Hands the stroke to the editor and files what the reload will ask for.
fn finish(
    editor: &mut SceneEditor,
    engine: &Engine,
    cache: &mut TerrainCache,
    roots: &Query<(&Transform, &Terrained)>,
    live: Live,
) {
    if live.stroke.stamps.is_empty() {
        return;
    }
    editor.tell(RuntimeMessage::TerrainStroke {
        actor: live.actor.clone(),
        stroke: live.stroke.clone(),
    });
    let Ok((_, terrained)) = roots.get(live.root) else {
        return;
    };
    let Some(built) = terrained.built.clone() else {
        return;
    };
    if let Some(predicted) = predict(&terrained.spec, &built, live) {
        cache.predict(engine.project_dir.as_deref(), &predicted.0, predicted.1);
    }
}

/// The document and build the editor's reload will bring: the same grid
/// the editor stores, named the way the store names it.
fn predict(
    spec: &blockloom_core::terrain::TerrainSpec,
    built: &Arc<Built>,
    live: Live,
) -> Option<(blockloom_core::terrain::TerrainSpec, Built)> {
    let target = live.stroke.brush.target;
    let mut grids = live.grids;
    if target == BrushTarget::Heights {
        // What the store keeps is 16-bit; draw exactly that.
        grids.heights = Grid::from_heights(&grids.heights).to_heights()?;
    }
    let grid = grids.grid(target)?;
    let mut spec = spec.clone();
    sculpt::set_target(&mut spec, target, store::name_of(&grid));
    let geometry = if matches!(target, BrushTarget::Heights | BrushTarget::Holes) {
        Arc::new(Geometry::from_parts(grids.heights, grids.holes.clone(), &spec))
    } else {
        built.geometry.clone()
    };
    let predicted = paint_from(
        &spec,
        geometry,
        grids.splat,
        grids.grass,
        grids.scatter,
        &built.avoid,
    );
    Some((spec, predicted))
}
