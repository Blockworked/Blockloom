//! The 2D level at run time, 2D only: the live tilemaps `paint tile`
//! writes, tile regions (spawn, checkpoint, kill, ladder, water), rooms with
//! camera handoff and streaming, parallax layers, and the scene view's
//! Tiles tool and overlays.
//!
//! [`Level`] is seeded from the document on every rebuild. A painted map is
//! marked dirty and [`redraw_maps`] rebuilds its mesh and compound collider.
//! Rooms that stream are payloads on the Phase 4 cell system: their maps
//! are meshed on a `CellTasks` task once a cell overlapping the room is
//! active, and dropped (collision kept) once none is. Parallax is
//! render-only, like sort depth: added in PostUpdate, taken off in `First`.

use crate::edit::{SceneEditor, editing};
use crate::engine::{ActorId, CameraRig, Engine, PendingEffects, PhysicsPose, PrevPose};
use crate::materials::{AnimatedTiles, TilemapMesh};
use crate::streaming::{CellEntered, CellLeft, CellTasks, StreamingCells, cell_rect_2d};
use crate::world::WorldCamera;
use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use bevy_rapier2d::prelude as rp;
use blockloom_core::material::{TileMesh, Tilemap};
use blockloom_core::project::Project;
use blockloom_core::scene::Visual;
use blockloom_core::tilemap::{
    BrushTool, LevelSense, ParallaxSpec, RegionKind, RoomBounds, RoomSense, RoomSpec, TilemapSense,
    smallest_room,
};
use blockloom_core::vm::{Effect, Event};
use blockloom_protocol::{RuntimeMessage, SceneTool};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The level's live state for one build of the world.
#[derive(Resource, Default)]
pub struct Level {
    /// Each tilemap actor's map as the run has it, by actor id.
    pub maps: HashMap<String, Arc<Tilemap>>,
    /// Maps whose mesh and collider need redoing.
    dirty: HashSet<String>,
    /// Each parallax layer's live spec, by actor id.
    pub parallax: HashMap<String, ParallaxSpec>,
    /// Each room's spec, by actor id.
    pub rooms: HashMap<String, RoomSpec>,
    /// Streamed map id -> the room that streams it.
    streamed: HashMap<String, String>,
    /// Rooms whose maps are built or building.
    loaded: HashSet<String>,
    tasks: CellTasks<String, TileMesh>,
    /// Which room each actor stood in last tick.
    in_room: HashMap<String, String>,
    /// Where each body goes back to when it touches a kill tile.
    respawn: HashMap<String, Vec2>,
    handoff: Option<Handoff>,
    /// Where the camera ended up last frame.
    last_camera: Option<Vec2>,
}

/// The camera sliding into a room it just entered.
struct Handoff {
    room: String,
    from: Vec2,
    elapsed: f32,
}

/// Gravity a body had before a ladder or water region changed it.
#[derive(Component)]
pub struct RegionHold {
    gravity: f32,
}

/// What `apply_parallax` moved a layer by this frame, taken off in `First`.
#[derive(Component)]
pub struct ParallaxOffset(Vec2);

/// A sprite's own color under a parallax layer's dimming.
#[derive(Component)]
pub struct ParallaxTint(Color);

/// The copies a wrapped layer draws beside itself.
#[derive(Component)]
pub struct ParallaxCopies(Vec<(Entity, IVec2)>);

#[derive(Component)]
pub struct ParallaxCopy;

fn scale2(placement: &blockloom_core::scene::Placement) -> [f32; 2] {
    [
        placement.scale * placement.stretch[0],
        placement.scale * placement.stretch[1],
    ]
}

impl Level {
    /// The level as the document authored it.
    pub fn seed(project: &Project) -> Self {
        let mut level = Level::default();
        for actor in &project.actors {
            if let Some(Visual::Tilemap { tilemap }) = actor.visual() {
                level
                    .maps
                    .insert(actor.id.clone(), Arc::new(tilemap.clone()));
            }
            if let Some(parallax) = actor.components.parallax() {
                level.parallax.insert(actor.id.clone(), parallax.clone());
            }
            if let Some(room) = actor.components.room() {
                level.rooms.insert(actor.id.clone(), room.clone());
            }
        }
        // A map streams with the smallest streaming room its centre is in.
        let rooms: Vec<RoomSense> = project
            .actors
            .iter()
            .filter_map(|actor| {
                let room = actor.components.room().filter(|room| room.stream)?;
                let place = actor.components.placement();
                Some(RoomSense {
                    id: actor.id.clone(),
                    name: actor.name.clone(),
                    bounds: room.bounds([place.position[0], place.position[1]], scale2(&place)),
                })
            })
            .collect();
        for actor in &project.actors {
            if !level.maps.contains_key(&actor.id) {
                continue;
            }
            let at = actor.components.placement().position;
            if let Some(room) = smallest_room(&rooms, [at[0], at[1]]) {
                level.streamed.insert(actor.id.clone(), room.id.clone());
            }
        }
        level
    }

    /// The maps a streaming room builds, which the rebuild spawns bare.
    pub fn streamed_maps(&self) -> HashSet<String> {
        self.streamed.keys().cloned().collect()
    }

    /// An actor's live map, taken from its look the first time (a clone's).
    fn map_mut(&mut self, engine: &Engine, id: &str) -> Option<&mut Arc<Tilemap>> {
        if !self.maps.contains_key(id) {
            let Some(Visual::Tilemap { tilemap }) = engine.actor(id).and_then(|a| a.visual())
            else {
                return None;
            };
            self.maps.insert(id.to_string(), Arc::new(tilemap.clone()));
        }
        self.maps.get_mut(id)
    }

    fn drawn(&self, id: &str) -> bool {
        self.streamed
            .get(id)
            .is_none_or(|room| self.loaded.contains(room))
    }

    /// Room bounds as they stand, ordered by id so a replay walks them alike.
    fn room_senses(
        &self,
        engine: &Engine,
        placed: &HashMap<String, Transform>,
    ) -> Vec<(RoomSense, bool)> {
        let mut rooms: Vec<(RoomSense, bool)> = self
            .rooms
            .iter()
            .filter_map(|(id, spec)| {
                let t = placed.get(id)?;
                Some((
                    RoomSense {
                        id: id.clone(),
                        name: engine.actor(id).map(|a| a.name.clone()).unwrap_or_default(),
                        bounds: spec
                            .bounds([t.translation.x, t.translation.y], [t.scale.x, t.scale.y]),
                    },
                    spec.camera,
                ))
            })
            .collect();
        rooms.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        rooms
    }

    fn map_senses(
        &self,
        engine: &Engine,
        placed: &HashMap<String, Transform>,
    ) -> Vec<TilemapSense> {
        let mut maps: Vec<TilemapSense> = self
            .maps
            .iter()
            .filter_map(|(id, map)| {
                let t = placed.get(id)?;
                Some(TilemapSense {
                    id: id.clone(),
                    name: engine.actor(id).map(|a| a.name.clone()).unwrap_or_default(),
                    center: [t.translation.x, t.translation.y],
                    scale: [t.scale.x, t.scale.y],
                    map: Arc::clone(map),
                })
            })
            .collect();
        maps.sort_by(|a, b| a.id.cmp(&b.id));
        maps
    }
}

fn report(actor: &str, message: String) {
    crate::bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

// ─── Blocks ────────────────────────────────────────────────────────────────

/// `paint tile` and `set parallax`, on the fixed tick.
pub fn apply_level_effects(
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    mut level: ResMut<Level>,
    placed: Query<(&ActorId, &Transform)>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::PaintTile {
                actor,
                map,
                tile,
                x,
                y,
            } => {
                let point = Vec2::new(*x, *y);
                let local = |id: &str| {
                    let entity = engine.entities.get(id)?;
                    let (_, t) = placed.get(*entity).ok()?;
                    Some(
                        (point - t.translation.truncate())
                            / t.scale.truncate().abs().max(Vec2::splat(1e-6)),
                    )
                };
                let target = if map.is_empty() {
                    let own = matches!(
                        engine.actor(actor).and_then(|a| a.visual()),
                        Some(Visual::Tilemap { .. })
                    );
                    if own {
                        Some(actor.clone())
                    } else {
                        // The first map covering the point, in document order.
                        engine
                            .actor_ids()
                            .filter(|id| {
                                matches!(
                                    engine.actor(id).and_then(|a| a.visual()),
                                    Some(Visual::Tilemap { .. })
                                )
                            })
                            .find(|id| {
                                let map = level.maps.get(*id).cloned().or_else(|| {
                                    match engine.actor(id)?.visual()? {
                                        Visual::Tilemap { tilemap } => {
                                            Some(Arc::new(tilemap.clone()))
                                        }
                                        _ => None,
                                    }
                                });
                                map.zip(local(id))
                                    .is_some_and(|(map, at)| map.cell_at_local(at.into()).is_some())
                            })
                            .cloned()
                    }
                } else {
                    match crate::world::resolve_actor(&engine, actor, map) {
                        Some(id) => Some(id),
                        None => {
                            report(actor, format!("there's no tilemap named \"{map}\""));
                            continue;
                        }
                    }
                };
                let Some(id) = target else {
                    continue;
                };
                let Some(at) = local(&id) else {
                    continue;
                };
                let Some(live) = level.map_mut(&engine, &id) else {
                    report(actor, format!("\"{map}\" isn't a tilemap"));
                    continue;
                };
                let cells = (live.sheet_columns * live.sheet_rows) as i32;
                if *tile >= cells {
                    report(
                        actor,
                        format!("tile {tile} isn't on the sheet, which has {cells} tiles"),
                    );
                    continue;
                }
                let Some((cx, cy)) = live.cell_at_local(at.into()) else {
                    continue;
                };
                let live = Arc::make_mut(live);
                if live.tile_at(cx, cy) == Some(*tile) {
                    continue;
                }
                live.set_tile(cx, cy, *tile);
                live.retile(&[(cx as i32, cy as i32)]);
                level.dirty.insert(id);
            }
            Effect::SetParallax {
                actor,
                layer,
                axis,
                value,
            } => {
                let Some(id) = crate::world::resolve_actor(&engine, actor, layer) else {
                    report(actor, format!("there's no actor named \"{layer}\""));
                    continue;
                };
                match level.parallax.get_mut(&id) {
                    Some(spec) => axis.apply(&mut spec.scroll, *value),
                    None => report(actor, format!("\"{layer}\" has no Parallax component")),
                }
            }
            _ => {}
        }
    }
}

/// Rebuilds each dirty map's mesh and collider.
#[allow(clippy::too_many_arguments)]
pub fn redraw_maps(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut level: ResMut<Level>,
    markers: Query<&TilemapMesh>,
    handles: Query<&Mesh2d>,
    bodies: Query<(), With<rp::RigidBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    assets: Res<AssetServer>,
) {
    if level.dirty.is_empty() {
        return;
    }
    let dirty: Vec<String> = level.dirty.drain().collect();
    for id in dirty {
        let (Some(entity), Some(map)) = (engine.entities.get(&id).copied(), level.maps.get(&id))
        else {
            continue;
        };
        let map = Arc::clone(map);
        let collider = crate::dim2::tilemap_collider(&map);
        if bodies.get(entity).is_ok() {
            match collider {
                Some(collider) => {
                    commands.entity(entity).insert(collider);
                }
                None => {
                    commands.entity(entity).remove::<rp::Collider>();
                }
            }
        } else if collider.is_some()
            && let Some(actor) = engine.actor(&id)
        {
            // A body that had nothing to collide with until now.
            crate::dim2::insert_body_with(&mut commands.entity(entity), actor, collider);
        }
        if !level.drawn(&id) {
            continue;
        }
        let built = map.build_mesh();
        let child = markers.get(entity).ok().map(|marker| marker.0);
        match (child, built.is_empty()) {
            (Some(child), true) => {
                commands.entity(child).despawn();
                commands.entity(entity).remove::<TilemapMesh>();
            }
            (Some(child), false) => {
                let Ok(handle) = handles.get(child) else {
                    continue;
                };
                if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                    *mesh = crate::materials::tilemesh_to_bevy(&built);
                }
                match AnimatedTiles::of(&map, &handle.0) {
                    Some(animated) => commands.entity(child).insert(animated),
                    None => commands.entity(child).remove::<AnimatedTiles>(),
                };
            }
            (None, false) => {
                crate::materials::spawn_tilemap_mesh_2d(
                    &mut commands,
                    entity,
                    &map,
                    &built,
                    engine.project_dir.as_deref(),
                    &assets,
                    &mut meshes,
                    &mut materials,
                );
            }
            (None, true) => {}
        }
    }
}

/// Publishes the live maps and room bounds for `tile at` and `room
/// containing`, after the rest of the frame's snapshot.
pub fn publish_level(
    engine: NonSend<Engine>,
    level: Res<Level>,
    placed: Query<(&ActorId, &Transform)>,
) {
    let placed: HashMap<String, Transform> = placed
        .iter()
        .filter(|(id, _)| level.maps.contains_key(&id.0) || level.rooms.contains_key(&id.0))
        .map(|(id, t)| (id.0.clone(), *t))
        .collect();
    blockloom_core::sense::publish_level(LevelSense {
        tilemaps: level.map_senses(&engine, &placed),
        rooms: level
            .room_senses(&engine, &placed)
            .into_iter()
            .map(|(room, _)| room)
            .collect(),
    });
}

// ─── Rooms and regions ─────────────────────────────────────────────────────

/// Fires `when actor enters room` for every actor whose smallest room
/// changed this tick. An actor that starts the run inside a room enters it
/// on the first tick.
pub fn track_rooms(
    mut engine: NonSendMut<Engine>,
    mut level: ResMut<Level>,
    placed: Query<(&ActorId, &Transform)>,
) {
    if !engine.running || engine.paused || level.rooms.is_empty() {
        return;
    }
    let all: HashMap<String, Transform> = placed.iter().map(|(id, t)| (id.0.clone(), *t)).collect();
    let rooms: Vec<RoomSense> = level
        .room_senses(&engine, &all)
        .into_iter()
        .map(|(room, _)| room)
        .collect();
    let mut ids: Vec<&String> = all
        .keys()
        .filter(|id| !level.rooms.contains_key(*id))
        .collect();
    ids.sort();
    let mut entered = Vec::new();
    for id in ids {
        let at = all[id].translation;
        let now = smallest_room(&rooms, [at.x, at.y]);
        let was = level.in_room.get(id);
        if now.map(|room| &room.id) == was {
            continue;
        }
        match now {
            Some(room) => {
                level.in_room.insert(id.clone(), room.id.clone());
                entered.push((id.clone(), room.name.clone()));
            }
            None => {
                level.in_room.remove(id);
            }
        }
    }
    for (actor, room) in entered {
        engine.fire(Event::EnteredRoom { actor, room });
    }
}

/// What a moving body does in the region under its centre: checkpoints and
/// spawns move its respawn point, a kill tile sends it back there, a ladder
/// takes its gravity away and water weakens it and drags.
#[allow(clippy::type_complexity)]
pub fn apply_regions(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut level: ResMut<Level>,
    time: Res<Time>,
    mut bodies: Query<(
        Entity,
        &ActorId,
        &mut Transform,
        Option<&mut PhysicsPose>,
        Option<&mut PrevPose>,
        Option<&rp::RigidBody>,
        Option<&mut rp::Velocity>,
        Option<&mut rp::GravityScale>,
        Option<&RegionHold>,
    )>,
) {
    if !engine.running || engine.paused || level.maps.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let placed: HashMap<String, Transform> = bodies
        .iter()
        .filter(|(_, id, ..)| level.maps.contains_key(&id.0))
        .map(|(_, id, t, ..)| (id.0.clone(), *t))
        .collect();
    let maps = level.map_senses(&engine, &placed);
    if maps.iter().all(|sense| sense.map.regions.is_empty()) {
        return;
    }
    let spawn = maps.iter().find_map(|sense| {
        let (x, y) = sense.map.first_region_cell(RegionKind::Spawn)?;
        Some(Vec2::from(sense.world(sense.map.cell_center_local(x, y))))
    });
    let sense = LevelSense {
        tilemaps: maps,
        rooms: Vec::new(),
    };
    for (entity, id, mut transform, pose, prev, body, velocity, gravity, hold) in &mut bodies {
        let moving = matches!(
            body,
            Some(rp::RigidBody::Dynamic | rp::RigidBody::KinematicPositionBased)
        );
        if !moving {
            continue;
        }
        let at = transform.translation.truncate();
        let region = sense.region_at(at.into()).map(|(kind, map)| {
            let (x, y) = map.cell_at(at.into()).unwrap_or_default();
            (kind, Vec2::from(map.world(map.map.cell_center_local(x, y))))
        });
        let mut want_gravity = None;
        match region {
            Some((RegionKind::Spawn | RegionKind::Checkpoint, center)) => {
                level.respawn.insert(id.0.clone(), center);
            }
            Some((RegionKind::Kill, _)) => {
                let back = level
                    .respawn
                    .get(&id.0)
                    .copied()
                    .or(spawn)
                    .unwrap_or_else(|| {
                        let authored = engine
                            .actor(&id.0)
                            .map(|a| a.components.placement().position)
                            .unwrap_or_default();
                        Vec2::new(authored[0], authored[1])
                    });
                transform.translation.x = back.x;
                transform.translation.y = back.y;
                if let Some(mut pose) = pose {
                    pose.0.translation = transform.translation;
                }
                if let Some(mut prev) = prev {
                    prev.0.translation = transform.translation;
                }
                if let Some(mut velocity) = velocity {
                    *velocity = rp::Velocity::zero();
                }
                continue;
            }
            Some((RegionKind::Ladder, _)) => {
                want_gravity = Some(0.0);
                if let Some(mut velocity) = velocity {
                    velocity.linear.y *= (1.0 - 6.0 * dt).clamp(0.0, 1.0);
                }
            }
            Some((RegionKind::Water, _)) => {
                want_gravity = Some(0.3);
                if let Some(mut velocity) = velocity {
                    velocity.linear *= (1.0 - 2.0 * dt).clamp(0.0, 1.0);
                }
            }
            None => {}
        }
        let Some(mut gravity) = gravity else {
            continue;
        };
        match (want_gravity, hold) {
            (Some(factor), Some(hold)) => gravity.0 = hold.gravity * factor,
            (Some(factor), None) => {
                commands
                    .entity(entity)
                    .insert(RegionHold { gravity: gravity.0 });
                gravity.0 *= factor;
            }
            (None, Some(hold)) => {
                gravity.0 = hold.gravity;
                commands.entity(entity).remove::<RegionHold>();
            }
            (None, None) => {}
        }
    }
}

/// Keeps the camera inside the room its target stands in, sliding into a
/// newly entered one over the room's blend time.
#[allow(clippy::type_complexity)]
pub fn confine_camera(
    engine: NonSend<Engine>,
    editor: Option<Res<SceneEditor>>,
    mut level: ResMut<Level>,
    time: Res<Time<Real>>,
    rigs: Query<&Transform, (With<CameraRig>, Without<WorldCamera>)>,
    placed: Query<(&ActorId, &Transform), Without<WorldCamera>>,
    mut cameras: Query<(&mut Transform, &Projection), With<WorldCamera>>,
) {
    if editor.is_some_and(|editor| editing(&engine, &editor)) || level.rooms.is_empty() {
        level.handoff = None;
        return;
    }
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let half = ortho.area.half_size();
    let Some(target) = rigs.iter().next() else {
        return;
    };
    let rooms_placed: HashMap<String, Transform> = placed
        .iter()
        .filter(|(id, _)| level.rooms.contains_key(&id.0))
        .map(|(id, t)| (id.0.clone(), *t))
        .collect();
    let rooms: Vec<RoomSense> = level
        .room_senses(&engine, &rooms_placed)
        .into_iter()
        .filter(|(_, camera)| *camera)
        .map(|(room, _)| room)
        .collect();
    let here = camera.translation.truncate();
    let at = target.translation;
    let Some(room) = smallest_room(&rooms, [at.x, at.y]).cloned() else {
        level.handoff = None;
        level.last_camera = Some(here);
        return;
    };
    if half.x <= 0.0 || half.y <= 0.0 {
        return;
    }
    let confined = Vec2::from(room.bounds.confine(here.into(), half.into()));
    if level.handoff.as_ref().is_none_or(|h| h.room != room.id) {
        let from = level.last_camera.unwrap_or(confined);
        level.handoff = Some(Handoff {
            room: room.id.clone(),
            from,
            elapsed: 0.0,
        });
    }
    let blend = level.rooms.get(&room.id).map_or(0.0, |spec| spec.blend);
    let handoff = level.handoff.as_mut().expect("just set");
    handoff.elapsed += time.delta_secs();
    let t = if blend <= 0.0 {
        1.0
    } else {
        let t = (handoff.elapsed / blend).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let next = handoff.from.lerp(confined, t);
    camera.translation.x = next.x;
    camera.translation.y = next.y;
    level.last_camera = Some(next);
}

/// Builds the maps of every streaming room a live cell overlaps, and drops
/// the drawing of those none does. Collision stays either way.
#[allow(clippy::too_many_arguments)]
pub fn stream_rooms(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut level: ResMut<Level>,
    mut cells: ResMut<StreamingCells>,
    mut entered: MessageReader<CellEntered>,
    mut left: MessageReader<CellLeft>,
    placed: Query<(&ActorId, &Transform)>,
    markers: Query<&TilemapMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    assets: Res<AssetServer>,
) {
    entered.clear();
    left.clear();
    if level.streamed.is_empty() {
        return;
    }
    let level = &mut *level;
    let all: HashMap<String, Transform> = placed
        .iter()
        .filter(|(id, _)| level.rooms.contains_key(&id.0))
        .map(|(id, t)| (id.0.clone(), *t))
        .collect();
    let bounds: HashMap<String, RoomBounds> = level
        .room_senses(&engine, &all)
        .into_iter()
        .map(|(room, _)| (room.id, room.bounds))
        .collect();
    let streaming: HashSet<&String> = level.streamed.values().collect();
    let mut wanted: HashMap<String, (i32, i32)> = HashMap::new();
    for room in streaming {
        let Some(b) = bounds.get(room) else {
            continue;
        };
        if let Some(cell) = cells.active.iter().copied().find(|&cell| {
            let (min, max) = cell_rect_2d(cell);
            b.overlaps(min.into(), max.into())
        }) {
            wanted.insert(room.clone(), cell);
        }
    }
    let mut maps: Vec<(String, String)> = level
        .streamed
        .iter()
        .map(|(map, room)| (map.clone(), room.clone()))
        .collect();
    maps.sort();
    for (map, room) in &maps {
        let loaded = level.loaded.contains(room);
        match (wanted.get(room), loaded) {
            (Some(&cell), false) => {
                if let Some(live) = level.maps.get(map).cloned() {
                    level
                        .tasks
                        .spawn(&mut cells, map.clone(), cell, move || live.build_mesh());
                }
            }
            (None, true) => {
                level.tasks.cancel(&mut cells, map);
                if let Some(entity) = engine.entities.get(map)
                    && let Ok(marker) = markers.get(*entity)
                {
                    commands.entity(marker.0).despawn();
                    commands.entity(*entity).remove::<TilemapMesh>();
                }
            }
            _ => {}
        }
    }
    for (_, room) in &maps {
        if wanted.contains_key(room) {
            level.loaded.insert(room.clone());
        } else {
            level.loaded.remove(room);
        }
    }
    for (map, built) in level.tasks.poll(&mut cells) {
        let (Some(entity), Some(live)) = (engine.entities.get(&map), level.maps.get(&map)) else {
            continue;
        };
        if let Ok(marker) = markers.get(*entity) {
            commands.entity(marker.0).despawn();
        }
        crate::materials::spawn_tilemap_mesh_2d(
            &mut commands,
            *entity,
            live,
            &built,
            engine.project_dir.as_deref(),
            &assets,
            &mut meshes,
            &mut materials,
        );
    }
}

// ─── Parallax ──────────────────────────────────────────────────────────────

fn parallax_shown(engine: &Engine, editor: Option<&SceneEditor>) -> bool {
    editor.is_none_or(|editor| !editing(engine, editor) || editor.view.tiles.parallax)
}

/// Moves each parallax layer to where its scroll factor puts it against the
/// camera, and fades a distant one towards the background. Render-only:
/// [`clear_parallax`] undoes it at the head of the next frame.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn apply_parallax(
    mut commands: Commands,
    engine: NonSend<Engine>,
    editor: Option<Res<SceneEditor>>,
    level: Res<Level>,
    environment: Res<crate::environment::Environment>,
    cameras: Query<&Transform, With<WorldCamera>>,
    mut layers: Query<
        (Entity, &ActorId, &mut Transform, Option<&mut Sprite>),
        Without<WorldCamera>,
    >,
    markers: Query<&TilemapMesh>,
    tile_materials: Query<&MeshMaterial2d<ColorMaterial>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if level.parallax.is_empty() || !parallax_shown(&engine, editor.as_deref()) {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let camera = camera.translation.truncate();
    for (entity, id, mut transform, sprite) in &mut layers {
        let Some(spec) = level.parallax.get(&id.0) else {
            continue;
        };
        let size = engine
            .actor(&id.0)
            .and_then(|a| a.visual())
            .map(crate::world::half_extents)
            .unwrap_or_default()
            * 2.0
            * transform.scale.truncate().abs();
        let at = transform.translation.truncate();
        let drawn = Vec2::from(spec.drawn_at(at.into(), camera.into(), camera.into(), size.into()));
        let offset = drawn - at;
        if offset != Vec2::ZERO {
            transform.translation += offset.extend(0.0);
            commands.entity(entity).insert(ParallaxOffset(offset));
        }
        let dim = spec.dimming();
        if let Some(mut sprite) = sprite
            && dim > 0.0
        {
            let own = sprite.color;
            sprite.color = crate::world::mix_color(own, environment.background, dim);
            commands.entity(entity).insert(ParallaxTint(own));
        }
        if let Ok(marker) = markers.get(entity)
            && let Ok(handle) = tile_materials.get(marker.0)
        {
            let want = crate::world::mix_color(Color::WHITE, environment.background, dim);
            if materials.get(&handle.0).is_some_and(|m| m.color != want)
                && let Some(mut material) = materials.get_mut(&handle.0)
            {
                material.color = want;
            }
        }
    }
}

/// Takes last frame's parallax offset and dimming back off.
pub fn clear_parallax(
    mut commands: Commands,
    mut layers: Query<(
        Entity,
        &mut Transform,
        Option<&ParallaxOffset>,
        Option<&ParallaxTint>,
        Option<&mut Sprite>,
    )>,
) {
    for (entity, mut transform, offset, tint, sprite) in &mut layers {
        if let Some(offset) = offset {
            transform.translation -= offset.0.extend(0.0);
            commands.entity(entity).remove::<ParallaxOffset>();
        }
        if let Some(tint) = tint {
            if let Some(mut sprite) = sprite {
                sprite.color = tint.0;
            }
            commands.entity(entity).remove::<ParallaxTint>();
        }
    }
}

/// Gives each wrapped layer its neighbour copies, drawing what it draws.
#[allow(clippy::type_complexity)]
pub fn sync_parallax_copies(
    mut commands: Commands,
    engine: NonSend<Engine>,
    level: Res<Level>,
    layers: Query<(
        Entity,
        &ActorId,
        Option<&Sprite>,
        Option<&TilemapMesh>,
        Option<&ParallaxCopies>,
    )>,
    drawing: Query<(&Mesh2d, &MeshMaterial2d<ColorMaterial>)>,
) {
    for (entity, id, sprite, marker, copies) in &layers {
        let wrap = level
            .parallax
            .get(&id.0)
            .map(|spec| spec.wrap)
            .unwrap_or([false; 2]);
        let mut want = Vec::new();
        for j in -1..=1 {
            for i in -1..=1 {
                if (i, j) != (0, 0) && (wrap[0] || i == 0) && (wrap[1] || j == 0) {
                    want.push(IVec2::new(i, j));
                }
            }
        }
        let have: Vec<IVec2> = copies
            .map(|copies| copies.0.iter().map(|(_, at)| *at).collect())
            .unwrap_or_default();
        let size = engine
            .actor(&id.0)
            .and_then(|a| a.visual())
            .map(crate::world::half_extents)
            .unwrap_or_default()
            * 2.0;
        let entities: Vec<(Entity, IVec2)> = if have != want {
            if let Some(copies) = copies {
                for (copy, _) in &copies.0 {
                    commands.entity(*copy).despawn();
                }
            }
            if want.is_empty() {
                if copies.is_some() {
                    commands.entity(entity).remove::<ParallaxCopies>();
                }
                continue;
            }
            let made: Vec<(Entity, IVec2)> = want
                .iter()
                .map(|at| {
                    let copy = commands
                        .spawn((
                            ParallaxCopy,
                            Transform::from_translation((at.as_vec2() * size).extend(0.0)),
                            Visibility::Inherited,
                        ))
                        .id();
                    commands.entity(entity).add_child(copy);
                    (copy, *at)
                })
                .collect();
            commands.entity(entity).insert(ParallaxCopies(made.clone()));
            made
        } else {
            copies.map(|copies| copies.0.clone()).unwrap_or_default()
        };
        for (copy, at) in entities {
            let mut copy = commands.entity(copy);
            copy.insert(Transform::from_translation(
                (at.as_vec2() * size).extend(0.0),
            ));
            if let Some(sprite) = sprite {
                copy.insert(sprite.clone());
            } else if let Some(marker) = marker
                && let Ok((mesh, material)) = drawing.get(marker.0)
            {
                copy.insert((mesh.clone(), material.clone()));
            }
        }
    }
}

// ─── Scene view ────────────────────────────────────────────────────────────

/// A Tiles stroke in progress.
#[derive(Default)]
pub struct LiveTiles {
    actor: String,
    brush: blockloom_core::tilemap::TileBrush,
    start: (i32, i32),
    last: (i32, i32),
    segments: Vec<[i32; 4]>,
    picked: bool,
}

/// The scene view's Tiles tool: paints on the level's copy of the selected
/// map as the pointer drags, and on release sends the editor the stroke to
/// run on the saved one.
#[allow(clippy::too_many_arguments)]
pub fn paint_tiles(
    mut editor: ResMut<SceneEditor>,
    engine: NonSend<Engine>,
    dimension: Res<crate::engine::Dimension>,
    mut level: ResMut<Level>,
    placed: Query<&Transform, With<ActorId>>,
    mut live: Local<Option<LiveTiles>>,
    mut gizmos: Gizmos,
) {
    let target = editor
        .selected
        .clone()
        .filter(|_| {
            dimension.0 == blockloom_core::scene::Mode::TwoD
                && editing(&engine, &editor)
                && editor.view.tool == SceneTool::Tiles
        })
        .and_then(|id| Some((*engine.entities.get(&id)?, id)))
        .filter(|(_, id)| level.maps.contains_key(id));
    let Some((entity, actor)) = target else {
        finish_tiles(&mut editor, &mut live);
        return;
    };
    if live.as_ref().is_some_and(|l| l.actor != actor) {
        finish_tiles(&mut editor, &mut live);
    }
    let Ok(transform) = placed.get(entity) else {
        return;
    };
    let Some(pointer) = editor.pointer_ray.map(|ray| ray.origin.truncate()) else {
        return;
    };
    let scale = transform.scale.truncate().abs().max(Vec2::splat(1e-6));
    let center = transform.translation.truncate();
    let map = Arc::clone(&level.maps[&actor]);
    let cell = map.cell_at_local_unclamped(((pointer - center) / scale).into());
    let brush = editor.view.tile_brush.clone();

    // The footprint under the pointer, or the line/rect being dragged.
    let cell_world = |cell: (i32, i32)| {
        let [w, h] = map.size();
        let [tw, th] = map.tile_size;
        let local = Vec2::new(
            cell.0 as f32 * tw + tw / 2.0 - w / 2.0,
            h / 2.0 - cell.1 as f32 * th - th / 2.0,
        );
        center + local * scale
    };
    let tile = Vec2::from(map.tile_size) * scale;
    let color = if editor.brushing {
        Color::srgb(1.0, 0.75, 0.2)
    } else {
        Color::srgb(0.9, 0.9, 0.95)
    };
    let (from, to) = match (&*live, brush.tool) {
        (Some(state), BrushTool::Line | BrushTool::Rect) => (state.start, cell),
        _ => {
            let n = match brush.tool {
                BrushTool::Paint | BrushTool::Erase | BrushTool::Scatter => {
                    brush.size.clamp(1, 16) as i32
                }
                _ => 1,
            };
            let lo = -(n - 1) / 2;
            (
                (cell.0 + lo, cell.1 + lo),
                (cell.0 + lo + n - 1, cell.1 + lo + n - 1),
            )
        }
    };
    if brush.tool == BrushTool::Line && live.is_some() {
        for (x, y) in blockloom_core::tilemap::line_cells(from, to) {
            gizmos.rect_2d(
                Isometry2d::from_translation(cell_world((x, y))),
                tile,
                color,
            );
        }
    } else {
        let (a, b) = (cell_world(from), cell_world(to));
        let min = a.min(b) - tile / 2.0;
        let max = a.max(b) + tile / 2.0;
        gizmos.rect_2d(
            Isometry2d::from_translation((min + max) / 2.0),
            max - min,
            color,
        );
    }

    if !editor.brushing {
        finish_tiles(&mut editor, &mut live);
        return;
    }
    let fresh = live.is_none();
    let state = live.get_or_insert_with(|| LiveTiles {
        actor: actor.clone(),
        brush: brush.clone(),
        start: cell,
        last: cell,
        segments: Vec::new(),
        picked: false,
    });
    let mut segment = None;
    match state.brush.tool {
        BrushTool::Pick => {
            if !state.picked {
                state.picked = true;
                let tile = cell_in(&map, cell).unwrap_or(-1);
                editor.tell(RuntimeMessage::TilePicked {
                    actor: actor.clone(),
                    tile,
                });
            }
        }
        BrushTool::Fill => {
            if fresh {
                segment = Some([cell.0, cell.1, cell.0, cell.1]);
            }
        }
        // Line and rect land on release.
        BrushTool::Line | BrushTool::Rect => state.last = cell,
        BrushTool::Paint | BrushTool::Erase | BrushTool::Scatter => {
            if fresh || cell != state.last {
                segment = Some([state.last.0, state.last.1, cell.0, cell.1]);
                state.last = cell;
            }
        }
    }
    if let Some(segment) = segment
        && let Some(map) = level.maps.get_mut(&actor)
    {
        let changed = Arc::make_mut(map).apply_brush(
            &state.brush,
            (segment[0], segment[1]),
            (segment[2], segment[3]),
        );
        state.segments.push(segment);
        if !changed.is_empty() {
            level.dirty.insert(actor.clone());
        }
    }
}

fn cell_in(map: &Tilemap, cell: (i32, i32)) -> Option<i32> {
    (cell.0 >= 0 && cell.1 >= 0)
        .then(|| map.tile_at(cell.0 as u32, cell.1 as u32))
        .flatten()
}

/// Ends a stroke: a line or rect runs now, and whatever ran goes to the
/// editor as one undo step.
fn finish_tiles(editor: &mut SceneEditor, live: &mut Option<LiveTiles>) {
    let Some(mut state) = live.take() else {
        return;
    };
    if matches!(state.brush.tool, BrushTool::Line | BrushTool::Rect) {
        state
            .segments
            .push([state.start.0, state.start.1, state.last.0, state.last.1]);
    }
    if state.segments.is_empty() {
        return;
    }
    editor.tell(RuntimeMessage::TileStroke {
        actor: state.actor,
        brush: state.brush,
        segments: state.segments,
    });
}

/// The level overlays: collision rects, region tiles and room bounds.
pub fn draw_overlays(
    engine: NonSend<Engine>,
    editor: Option<Res<SceneEditor>>,
    level: Res<Level>,
    placed: Query<(&ActorId, &Transform)>,
    mut gizmos: Gizmos,
) {
    let Some(editor) = editor else {
        return;
    };
    let flags = editor.view.tiles;
    if !editor.view.enabled || !(flags.collision || flags.regions || flags.rooms) {
        return;
    }
    let placed: HashMap<String, Transform> = placed
        .iter()
        .filter(|(id, _)| level.maps.contains_key(&id.0) || level.rooms.contains_key(&id.0))
        .map(|(id, t)| (id.0.clone(), *t))
        .collect();
    let rect = |gizmos: &mut Gizmos,
                sense: &TilemapSense,
                center: [f32; 2],
                half: [f32; 2],
                color: Color| {
        let at = Vec2::from(sense.world(center));
        let size = Vec2::from(half) * 2.0 * Vec2::from(sense.scale).abs();
        gizmos.rect_2d(Isometry2d::from_translation(at), size, color);
    };
    for sense in level.map_senses(&engine, &placed) {
        if flags.collision {
            for r in sense.map.solid_rects() {
                rect(
                    &mut gizmos,
                    &sense,
                    r.center,
                    r.half,
                    Color::srgba(1.0, 0.3, 0.3, 0.9),
                );
            }
        }
        if flags.regions {
            for kind in RegionKind::ALL {
                let color = match kind {
                    RegionKind::Spawn => Color::srgb(0.3, 0.9, 0.4),
                    RegionKind::Checkpoint => Color::srgb(0.3, 0.7, 1.0),
                    RegionKind::Kill => Color::srgb(1.0, 0.2, 0.6),
                    RegionKind::Ladder => Color::srgb(0.95, 0.8, 0.3),
                    RegionKind::Water => Color::srgb(0.2, 0.5, 1.0),
                };
                for r in sense.map.region_rects(kind) {
                    rect(&mut gizmos, &sense, r.center, r.half, color);
                }
            }
        }
    }
    if flags.rooms {
        for (room, camera) in level.room_senses(&engine, &placed) {
            let min = Vec2::from(room.bounds.min);
            let max = Vec2::from(room.bounds.max);
            let color = if camera {
                Color::srgba(0.8, 0.6, 1.0, 0.9)
            } else {
                Color::srgba(0.8, 0.6, 1.0, 0.4)
            };
            gizmos.rect_2d(
                Isometry2d::from_translation((min + max) / 2.0),
                max - min,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::project::Actor;

    fn actor(id: &str, visual: Visual, x: f32) -> Actor {
        let mut actor = Actor::new(id, visual);
        actor.id = id.to_string();
        actor.components.placement_mut().position = [x, 0.0, 0.0];
        actor
    }

    #[test]
    fn a_map_inside_a_streaming_room_is_streamed() {
        let mut project = Project::starter("Level", blockloom_core::scene::Mode::TwoD);
        project.actors.clear();
        project.actors.push(actor(
            "near",
            Visual::Tilemap {
                tilemap: Tilemap::default(),
            },
            0.0,
        ));
        project.actors.push(actor(
            "far",
            Visual::Tilemap {
                tilemap: Tilemap::default(),
            },
            5000.0,
        ));
        let mut room = actor(
            "room",
            Visual::Rect {
                color: "#000000".into(),
                size: [1.0, 1.0],
            },
            0.0,
        );
        room.components.insert(ActorComponent::Room {
            room: RoomSpec {
                stream: true,
                ..RoomSpec::default()
            },
        });
        project.actors.push(room);
        let level = Level::seed(&project);
        assert_eq!(level.maps.len(), 2);
        assert_eq!(level.streamed_maps(), HashSet::from(["near".to_string()]));
        assert!(!level.drawn("near"));
        assert!(level.drawn("far"));
    }
}
