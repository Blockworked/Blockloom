//! Components attached and detached mid-run, actors made and unmade, parenting.

use super::*;

/// The effects that are about an actor's components rather than about the
/// world: writing a custom component's field, switching the camera's view,
/// and attaching or detaching a whole component mid-run.
///
/// Camera tweaks apply while paused too, so a settings menu can preview
/// them. Everything structural waits for the thaw, like the rest of the
/// simulation.
///
/// The dimension-specific halves of attach/detach - the ones that need a
/// sprite, a mesh or a physics body - are in `dim2`/`dim3`; this owns the
/// bookkeeping, so `engine.attached` is written in exactly one place.
pub fn apply_component_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    mut customs: Query<&mut CustomComponents>,
    mut visibilities: Query<&mut Visibility>,
    mut rigs: Query<&mut CameraRig>,
    mut transforms: Query<&mut Transform>,
) {
    if !engine.running {
        return;
    }
    let paused = engine.paused;
    for effect in &effects.0 {
        // While paused only interface-started strands run, and those are a
        // settings menu: its camera tweaks preview live, while structural
        // changes wait for the world to thaw like everything else does.
        if paused
            && !matches!(
                effect,
                Effect::SetCameraView { .. }
                    | Effect::SetCameraPitch { .. }
                    | Effect::SetCameraFov { .. }
            )
        {
            continue;
        }
        match effect {
            Effect::AttachComponent { actor, component } => attach(
                &mut commands,
                &mut engine,
                &mut customs,
                &mut visibilities,
                &mut transforms,
                actor,
                component,
            ),
            Effect::DetachComponent { actor, component } => detach(
                &mut commands,
                &mut engine,
                &mut customs,
                &mut visibilities,
                actor,
                component,
            ),
            Effect::SetComponentField {
                actor,
                component,
                field,
                value,
            } => {
                let Some(mut custom) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| customs.get_mut(*entity).ok())
                else {
                    continue;
                };
                // Only the editor declares components, so a block can write a
                // new field but not conjure the component holding it.
                if let Some(fields) = custom.0.get_mut(component) {
                    fields.insert(field.clone(), value.clone());
                }
            }
            Effect::SetCameraView { actor, view } => {
                // A settings menu drives the camera from an actor that does
                // not hold it, so fall back to whoever does.
                if let Some(entity) = engine.entities.get(actor).copied()
                    && let Ok(mut rig) = rigs.get_mut(entity)
                {
                    rig.0.view = *view;
                    continue;
                }
                if let Some(mut rig) = rigs.iter_mut().next() {
                    rig.0.view = *view;
                }
            }
            Effect::SetCameraPitch { actor, degrees } => {
                if let Some(entity) = engine.entities.get(actor).copied()
                    && let Ok(mut rig) = rigs.get_mut(entity)
                {
                    // Just short of vertical either way: at the pole the view
                    // flips over instead of stopping.
                    rig.0.pitch = degrees.clamp(-89.0, 89.0);
                    continue;
                }
                // The live rig, not the authored one, so a pitch a view
                // change just reset can be set again straight after.
                if let Some(mut rig) = rigs.iter_mut().next() {
                    rig.0.pitch = degrees.clamp(-89.0, 89.0);
                }
            }
            Effect::SetCameraFov { actor, fov } => {
                if let Some(entity) = engine.entities.get(actor).copied()
                    && let Ok(mut rig) = rigs.get_mut(entity)
                {
                    rig.0.fov = fov.clamp(30.0, 110.0);
                    continue;
                }
                if let Some(mut rig) = rigs.iter_mut().next() {
                    rig.0.fov = fov.clamp(30.0, 110.0);
                }
            }
            _ => {}
        }
    }
}

/// The camera settings an actor is running with: whatever the project gave
/// it, or the defaults if a script attached the camera mid-run.
pub(crate) fn camera_of(
    engine: &Engine,
    actor: &str,
) -> Option<blockloom_core::components::CameraAttach> {
    if !engine.has_component(actor, "Camera") {
        return None;
    }
    Some(
        engine
            .actor(actor)
            .and_then(|actor| actor.camera())
            .copied()
            .unwrap_or_default(),
    )
}

/// Components that aren't this module's to attach: bodies, looks and
/// materials need the dimension's own pipeline, so `dim2`/`dim3` pick those
/// up from the same effect list.
pub(crate) fn is_dimensions_own(component: &str) -> bool {
    matches!(
        component,
        "Body" | "Joint" | "Brain" | "Look" | "Material" | "Light"
    )
}

pub(crate) fn attach(
    commands: &mut Commands,
    engine: &mut Engine,
    customs: &mut Query<&mut CustomComponents>,
    visibilities: &mut Query<&mut Visibility>,
    transforms: &mut Query<&mut Transform>,
    actor: &str,
    component: &str,
) {
    let Some(entity) = engine.entities.get(actor).copied() else {
        return;
    };
    if engine.has_component(actor, component) {
        return;
    }
    // A script can't be loaded mid-frame: its library is opened when the
    // world is built, which is where a rebuilt one is picked up.
    if component == "Script" {
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: "a script can only be attached in the editor, not mid-run".to_string(),
        });
        return;
    }
    engine
        .attached
        .entry(actor.to_string())
        .or_default()
        .insert(component.to_string());
    if is_dimensions_own(component) {
        return;
    }
    match component {
        "Render" => {
            // The project's own answer if it has one; a fresh Render shows.
            let visible = engine
                .actor(actor)
                .map(|actor| actor.visible())
                .unwrap_or(true);
            if let Ok(mut slot) = visibilities.get_mut(entity) {
                *slot = if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
        "Camera" => {
            // One camera: taking it means taking it off whoever had it.
            for (other, held) in engine.attached.iter_mut() {
                if other != actor {
                    held.remove("Camera");
                }
            }
            for (id, other) in &engine.entities {
                if id != actor {
                    commands.entity(*other).remove::<CameraRig>();
                }
            }
            let rig = camera_of(engine, actor).unwrap_or_default();
            commands.entity(entity).insert(CameraRig(rig));
        }
        // The hierarchy is `engine.parents`, not anything on the entity. Like
        // `set my parent to`, hanging it off its authored parent puts it at
        // its authored offset when it carries one.
        "Parent" => {
            let Some(parent) = engine
                .actor(actor)
                .and_then(|actor| actor.parent())
                .map(str::to_string)
            else {
                return;
            };
            set_parent(engine, actor, &parent, transforms);
        }
        // Emitters and trails run while attached: hanging one starts the
        // spray, and the state is what the FX systems read.
        "Emitter" => {
            commands
                .entity(entity)
                .insert(crate::fx::EmitterState::fresh());
        }
        "Trail" => {
            commands
                .entity(entity)
                .insert(crate::fx::TrailState::fresh());
        }
        // Clips come back with the run stopped: attaching the component
        // gives the player somewhere to play, not something playing.
        "Animation" => {
            commands.entity(entity).remove::<AnimationPlayer>();
        }
        // The dials come back as authored, or at their defaults.
        "Sprite" => {
            let spec = engine
                .actor(actor)
                .and_then(|actor| actor.components.sprite())
                .cloned()
                .unwrap_or_default();
            commands
                .entity(entity)
                .insert(crate::sprites::SpriteDials(spec));
        }
        // Anything else is a custom component: it comes back with the fields
        // the editor gave it, or empty if the project never had one.
        name => {
            let fields = engine
                .actor(actor)
                .and_then(|actor| match actor.components.get(name) {
                    Some(blockloom_core::components::ActorComponent::Custom { fields, .. }) => {
                        Some(fields.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            if let Ok(mut custom) = customs.get_mut(entity) {
                custom.0.insert(
                    name.to_string(),
                    fields
                        .into_iter()
                        .map(|field| (field.name, field.value))
                        .collect(),
                );
            }
        }
    }
}

pub(crate) fn detach(
    commands: &mut Commands,
    engine: &mut Engine,
    customs: &mut Query<&mut CustomComponents>,
    visibilities: &mut Query<&mut Visibility>,
    actor: &str,
    component: &str,
) {
    let Some(entity) = engine.entities.get(actor).copied() else {
        return;
    };
    // There would be nowhere left for the actor to be.
    if component == "Place" || !engine.has_component(actor, component) {
        return;
    }
    if let Some(held) = engine.attached.get_mut(actor) {
        held.remove(component);
    }
    if is_dimensions_own(component) {
        return;
    }
    match component {
        // No Render component means visible, the same as it does in the
        // document - hiding an actor is `hide`, not detaching anything.
        "Render" => {
            if let Ok(mut slot) = visibilities.get_mut(entity) {
                *slot = Visibility::Inherited;
            }
        }
        "Camera" => {
            commands.entity(entity).remove::<CameraRig>();
        }
        "Script" => {
            // Detaching the script takes its code away mid-run, so its
            // `destroy` runs first, like a deleted actor's does.
            if let Some(scripts) = engine.scripts.remove(actor) {
                let mut asked = crate::script::Asked::default();
                crate::script::with_script_stores(
                    &engine.variables,
                    &engine.lists,
                    &engine.dicts,
                    || {
                        for entry in &scripts {
                            entry.script.destroy(actor, &mut asked);
                        }
                    },
                );
                for effect in asked.effects {
                    match effect {
                        Effect::Say { actor, text } => {
                            engine.note_say(&actor, &text);
                            bridge::send(&RuntimeMessage::Say { actor, text });
                        }
                        Effect::Error { actor, message } => {
                            bridge::send(&RuntimeMessage::Error { actor, message })
                        }
                        _ => {}
                    }
                }
            }
            engine.scripts_started.retain(|(owner, _)| owner != actor);
        }
        "Parent" => {
            engine.parents.remove(actor);
        }
        // Taking the spray or the trail away stops it: live particles and
        // ghosts fade out on their own rather than vanishing mid-flight.
        "Emitter" => {
            commands.entity(entity).remove::<crate::fx::EmitterState>();
        }
        "Trail" => {
            commands.entity(entity).remove::<crate::fx::TrailState>();
        }
        // Taking the clips away stops the player with them.
        "Animation" => {
            commands.entity(entity).remove::<AnimationPlayer>();
        }
        "Sprite" => {
            commands
                .entity(entity)
                .remove::<crate::sprites::SpriteDials>();
        }
        name => {
            if let Ok(mut custom) = customs.get_mut(entity) {
                custom.0.remove(name);
            }
        }
    }
}

// ─── Actors that come and go ───────────────────────────────────────────────

/// Making, deleting and re-parenting actors, before anything else this step
/// is applied - so a clone made now is already somewhere for the rest of the
/// step's effects to land.
///
/// A clone is the actor it was copied from as the editor authored it,
/// standing where that actor stands at this moment, carrying its live custom
/// component values. What `attach`/`detach` did to the template since Play
/// doesn't carry over: re-attaching a component has always meant the
/// authored one, and a clone is no different.
pub fn apply_lifetimes(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    live: Query<(&Visibility, Option<&CustomComponents>)>,
    mut transforms: Query<&mut Transform>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::CreateClone { actor, clone, of } => {
                let Some(mut copy) = engine.actor(of).cloned() else {
                    continue;
                };
                copy.id = clone.clone();
                copy.refresh_physics_ids();
                if let Some(entity) = engine.entities.get(of).copied() {
                    if let Ok(transform) = transforms.get(entity) {
                        let stretch = copy.placement().stretch;
                        copy.components
                            .set_placement(placement_of(transform, stretch));
                        // The live z carries the sort layer, and the spawner
                        // re-adds it: take it back off so a clone of a layered
                        // actor doesn't sort twice as high.
                        copy.components.placement_mut().position[2] -=
                            copy.components.layer() as f32;
                    }
                    if let Ok((visibility, custom)) = live.get(entity) {
                        copy.components
                            .set_visible(*visibility != Visibility::Hidden);
                        if let Some(custom) = custom {
                            carry_custom(&mut copy, custom);
                        }
                    }
                }
                // A clone hangs where its template hangs. Its own blocks can
                // move it off, the same as anything else about it.
                let parent = engine.parents.get(of).cloned();
                copy.components.set_parent(parent.as_deref().unwrap_or(""));
                engine.clones.insert(clone.clone(), of.clone());
                engine.last_created.insert(actor.clone(), clone.clone());
                spawn_runtime_actor(&mut commands, &mut engine, dimension.0, copy);
            }
            Effect::CreateActor {
                actor,
                id,
                name,
                position,
            } => {
                let mut made = Actor::blank(name.clone(), dimension.0);
                made.id = id.clone();
                made.components.placement_mut().position = *position;
                engine.last_created.insert(actor.clone(), id.clone());
                spawn_runtime_actor(&mut commands, &mut engine, dimension.0, made);
            }
            Effect::DeleteActor { actor } => delete_actor(&mut commands, &mut engine, actor),
            Effect::SetParent { actor, parent } => {
                set_parent(&mut engine, actor, parent, &mut transforms)
            }
            _ => {}
        }
    }
}

/// A transform, as the `Place` component spells one.
/// `stretch` is the actor's own, since a transform can't tell it from size.
pub(crate) fn placement_of(
    transform: &Transform,
    stretch: [f32; 3],
) -> blockloom_core::scene::Placement {
    let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
    blockloom_core::scene::Placement {
        position: transform.translation.to_array(),
        rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
        scale: size_of(transform, stretch),
        stretch,
    }
}

/// The uniform size a transform stands at, with the actor's stretch taken out.
pub(crate) fn size_of(transform: &Transform, stretch: [f32; 3]) -> f32 {
    let x = stretch[0];
    if x.abs() < 1e-6 {
        transform.scale.x
    } else {
        transform.scale.x / x
    }
}

/// Writes the template's live custom-component values onto the copy, so a
/// clone starts with the numbers its template is carrying rather than the
/// ones the editor typed.
pub(crate) fn carry_custom(copy: &mut Actor, custom: &CustomComponents) {
    for (name, fields) in &custom.0 {
        for (field, value) in fields {
            copy.components.set_field(name, field, value.clone());
        }
    }
}

/// Components whose actor can't be respawned alone: they reach other actors,
/// the camera, the level or baked data that only a rebuild redoes.
pub(crate) const REBUILD_ONLY: &[&str] = &[
    "Parent",
    "Camera",
    "PlayerCamera",
    "CharacterController",
    "CharacterMotor",
    "Constraint",
    "Joint",
    "Terrain",
    "Water",
    "Room",
    "Parallax",
    "Probe",
];

pub(crate) fn alone(actor: &Actor) -> bool {
    !actor
        .components
        .iter()
        .any(|c| REBUILD_ONLY.contains(&c.name()))
        && !matches!(
            actor.visual(),
            Some(blockloom_core::scene::Visual::Tilemap { .. })
        )
        && actor.components.colliders().all(|c| {
            let geometry = serde_json::to_string(&c.geometry).unwrap_or_default();
            !["Hull", "Mesh", "Terrain"]
                .iter()
                .any(|k| geometry.contains(k))
        })
}

/// The actors a patch replaced, when each can be respawned on its own: only
/// actors of the open scene changed, and none of them reaches anything a
/// rebuild has to redo.
pub(crate) fn respawnable(
    engine: &Engine,
    change: &blockloom_core::change::Change,
) -> Option<Vec<String>> {
    if change.whole.is_some() || change.header.is_some() || change.scene_order.is_some() {
        return None;
    }
    let active = engine.project.active_scene();
    let mut ids = Vec::new();
    for scene in &change.scenes {
        if scene.id != active.id {
            continue;
        }
        if scene.settings.is_some() || scene.all_actors.is_some() {
            return None;
        }
        for actor in &scene.actors {
            let sides = [&actor.before, &actor.after];
            if sides
                .iter()
                .any(|side| side.as_deref().is_some_and(|a| !alone(a)))
            {
                return None;
            }
            // Somebody hangs off it.
            if active
                .actors
                .iter()
                .any(|a| a.parent() == Some(actor.id.as_str()))
            {
                return None;
            }
            ids.push(actor.id.clone());
        }
    }
    Some(ids)
}

/// Respawns the actors a patch replaced, leaving the rest of the world as it
/// stands. Only while nothing runs; a rebuild that is due anyway wins.
#[allow(clippy::too_many_arguments)]
pub fn respawn_actors(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    mut navmesh: Option<ResMut<NavMesh>>,
) {
    if engine.respawn.is_empty() {
        return;
    }
    let ids = std::mem::take(&mut engine.respawn);
    if engine.rebuild || engine.running || engine.starting {
        return;
    }
    for id in &ids {
        forget_actor(&mut commands, &mut engine, id);
        let Some(actor) = engine.project.actor(id).cloned() else {
            continue;
        };
        spawn_actor_entity(&mut commands, &mut engine, dimension.0, &actor);
    }
    // Static solids are the navmesh's ground and holes.
    if let Some(nav) = navmesh.as_deref_mut() {
        nav.mesh = nav::build_mesh(&engine.project, nav::DEFAULT_AGENT_RADIUS)
            .map_err(|error| tracing::warn!("navmesh bake failed: {error}"))
            .ok()
            .map(Arc::new);
        crate::plugins::publish_nav(nav);
    }
}

/// Puts an actor the run made into the world and into every map that answers
/// a question about it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_runtime_actor(
    commands: &mut Commands,
    engine: &mut Engine,
    mode: Mode,
    actor: Actor,
) {
    spawn_actor_entity(commands, engine, mode, &actor);
    engine.spawned.insert(actor.id.clone(), actor);
}

/// Spawns one actor's entity, physics and scripts and files it in every map
/// that answers a question about it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_actor_entity(
    commands: &mut Commands,
    engine: &mut Engine,
    mode: Mode,
    actor: &Actor,
) {
    let entity = match mode {
        Mode::TwoD => dim2::spawn_actor(commands, actor, false),
        Mode::ThreeD => dim3::spawn_actor(commands, actor),
    }
    .unwrap_or_else(|| spawn_unseen(commands, actor, mode));
    attach_camera(commands, actor, entity);
    crate::fx::insert_fx_state(commands, actor, entity);
    // One camera in the world: a clone of the actor holding it takes it.
    if actor.camera().is_some() {
        claim_camera(commands, engine, &actor.id);
    }
    let held = actor
        .components
        .iter()
        .map(|component| component.name().to_string())
        .collect();
    engine.attached.insert(actor.id.clone(), held);
    let physics = actor.physics();
    engine.physics_filter.insert(
        actor.id.clone(),
        (physics.layer(), physics.collision_mask, physics.trigger),
    );
    if let Some(parent) = actor.parent() {
        engine.parents.insert(actor.id.clone(), parent.to_string());
    }
    engine.entities.insert(actor.id.clone(), entity);
    // Its colliders, body and constraints, installed the way a rebuild would.
    let targets: Vec<Actor> = actor
        .components
        .constraints()
        .filter(|c| !c.target.is_empty())
        .filter_map(|c| engine.actor(&c.target).cloned())
        .collect();
    crate::physics_install::install_actor(
        commands,
        &engine.project,
        actor,
        &targets,
        &engine.entities,
    );
    // A scripted actor's library is opened here rather than when the world
    // was built, because this one didn't exist then.
    open_script_for(engine, actor);
}

/// Takes the camera off everyone but `actor`.
pub(crate) fn claim_camera(commands: &mut Commands, engine: &mut Engine, actor: &str) {
    for (other, held) in engine.attached.iter_mut() {
        if other != actor {
            held.remove("Camera");
        }
    }
    for (id, entity) in &engine.entities {
        if id != actor {
            commands.entity(*entity).remove::<CameraRig>();
        }
    }
}

/// Opens one actor's scripts and reports each load result to the editor.
pub(crate) fn open_script_for(engine: &mut Engine, actor: &Actor) {
    let Some(dir) = engine.project_dir.clone() else {
        return;
    };
    for path in actor.components.scripts() {
        if !crate::engine::ScriptBackend::is_built(&dir, path) {
            bridge::send(&RuntimeMessage::ScriptLoaded {
                actor: actor.id.clone(),
                path: path.to_string(),
                error: Some(
                    "No compiled library found. Check the script or press Play to build it."
                        .to_string(),
                ),
            });
            continue;
        }
        match crate::engine::ScriptBackend::load(&dir, path) {
            Ok(script) => {
                engine.scripts.entry(actor.id.clone()).or_default().push(
                    crate::engine::ActorScript {
                        path: path.to_string(),
                        script,
                    },
                );
                bridge::send(&RuntimeMessage::ScriptLoaded {
                    actor: actor.id.clone(),
                    path: path.to_string(),
                    error: None,
                });
            }
            Err(message) => bridge::send(&RuntimeMessage::ScriptLoaded {
                actor: actor.id.clone(),
                path: path.to_string(),
                error: Some(message),
            }),
        }
    }
}

/// Takes an actor out of the world for the rest of the run. An authored one
/// comes back on the next Play: the document was never touched.
pub(crate) fn delete_actor(commands: &mut Commands, engine: &mut Engine, actor: &str) {
    // A dying actor's scripts get their `destroy` call first, while their
    // storage is still there to read. Only speech and errors are honoured;
    // the actor is already going away.
    if let Some(scripts) = engine.scripts.get(actor) {
        let mut asked = crate::script::Asked::default();
        crate::script::with_script_stores(&engine.variables, &engine.lists, &engine.dicts, || {
            for entry in scripts {
                entry.script.destroy(actor, &mut asked);
            }
        });
        for effect in asked.effects {
            match effect {
                Effect::Say { actor, text } => {
                    engine.note_say(&actor, &text);
                    bridge::send(&RuntimeMessage::Say { actor, text });
                }
                Effect::Error { actor, message } => {
                    bridge::send(&RuntimeMessage::Error { actor, message })
                }
                _ => {}
            }
        }
    }
    forget_actor(commands, engine, actor);
}

/// Takes an actor's entity out of the world and out of every map that names
/// it, without telling its scripts.
pub(crate) fn forget_actor(commands: &mut Commands, engine: &mut Engine, actor: &str) {
    let Some(entity) = engine.entities.remove(actor) else {
        return;
    };
    commands.entity(entity).despawn();
    engine.spawned.remove(actor);
    engine.clones.remove(actor);
    engine.attached.remove(actor);
    engine.physics_filter.remove(actor);
    engine.touching.remove(actor);
    engine.contacts.remove_actor(actor);
    engine.speech.remove(actor);
    engine.scripts.remove(actor);
    engine.scripts_started.retain(|(owner, _)| owner != actor);
    engine.parents.remove(actor);
    // Its children are let go rather than deleted with it, and nobody is
    // left pointing at it as the actor they just made.
    engine.parents.retain(|_, parent| parent != actor);
    engine.last_created.retain(|_, made| made != actor);
    for touching in engine.touching.values_mut() {
        touching.remove(actor);
    }
}

/// Hangs `actor` off `wanted`, or takes it off when that names nothing. A
/// child carrying an authored offset is placed at it - that far from its new
/// parent, in the parent's own frame, the same `parent * offset` the world is
/// built from - while one without an offset stays exactly where it is.
pub(crate) fn set_parent(
    engine: &mut Engine,
    actor: &str,
    wanted: &str,
    transforms: &mut Query<&mut Transform>,
) {
    if !engine.entities.contains_key(actor) {
        return;
    }
    if wanted.trim().is_empty() {
        engine.parents.remove(actor);
        return;
    }
    let Some(parent) = resolve_actor(engine, actor, wanted) else {
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: format!("there's no actor named \"{wanted}\" to hang off"),
        });
        return;
    };
    if parent == actor {
        engine.parents.remove(actor);
        return;
    }
    if engine.would_loop(actor, &parent) {
        let name = engine
            .actor(&parent)
            .map(|other| other.name.clone())
            .unwrap_or_else(|| parent.clone());
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: format!("{name} already hangs off this actor, so it can't be its parent"),
        });
        return;
    }
    engine.parents.insert(actor.to_string(), parent.clone());
    // The offset is where the child stands in its parent's frame, so hanging
    // it off someone new puts it there rather than leaving it where it
    // stood. Without one there is nowhere to put it, and it keeps its world
    // place, as before.
    let offset = engine.actor(actor).and_then(|actor| actor.parent_offset());
    let Some(offset) = offset else {
        return;
    };
    let parent_pose = engine
        .entities
        .get(&parent)
        .and_then(|entity| transforms.get(*entity).ok())
        .copied();
    let (Some(parent), Some(child)) = (parent_pose, engine.entities.get(actor).copied()) else {
        return;
    };
    if let Ok(mut mine) = transforms.get_mut(child) {
        mine.translation = world_of(&parent, offset);
    }
}

/// Which actor a name or an id means right now. Clones share their
/// template's name, so a name answers with whichever one comes to hand -
/// `the actor I made` is how a block names one in particular.
pub fn resolve_actor(engine: &Engine, running: &str, wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    if wanted.eq_ignore_ascii_case("myself") || wanted.eq_ignore_ascii_case("me") {
        return Some(running.to_string());
    }
    if engine.entities.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    engine
        .actor_ids()
        .find(|id| {
            engine
                .actor(id)
                .is_some_and(|actor| actor.name.eq_ignore_ascii_case(wanted))
        })
        .cloned()
}

/// Where `child` stands in its parent's frame: the parent's world transform
/// inverted onto the child's world position. What the local-position
/// reporters read, and the inverse of [`world_of`].
pub(crate) fn local_of(parent: &Transform, child: Vec3) -> [f32; 3] {
    parent
        .compute_affine()
        .inverse()
        .transform_point3(child)
        .to_array()
}

/// Where an offset in a parent's frame lands in the world: the same
/// `parent * offset` the world is built from, which is also where `set my
/// parent to` puts a child carrying an authored offset.
pub(crate) fn world_of(parent: &Transform, offset: [f32; 3]) -> Vec3 {
    parent.transform_point(Vec3::from(offset))
}

/// Resolves every authored local offset into the world placement the actor
/// is spawned at.
///
/// `Place` stays what the world is built from, so this is the first moment an
/// offset is read: a child that carries one stands at
/// `parent placement * offset`, and one that doesn't stays exactly where its
/// own `Place` puts it. `set my parent to` reads it again at run time, when
/// it hangs the actor off someone new. Parents are laid out before their
/// children, so an offset down a chain is measured against a parent that has
/// already moved.
pub(crate) fn place_authored_children(project: &mut blockloom_core::project::Project) {
    let parents: HashMap<String, String> = project
        .actors
        .iter()
        .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
        .collect();
    if parents.is_empty() {
        return;
    }
    let mut placed: HashMap<String, Transform> = project
        .actors
        .iter()
        .map(|actor| (actor.id.clone(), transform_of(&actor.placement())))
        .collect();
    let mut order: Vec<&String> = parents.keys().collect();
    order.sort_by_key(|id| depth_of(&parents, id));
    let order: Vec<String> = order.into_iter().cloned().collect();

    for child in order {
        let Some(offset) = project
            .actor(&child)
            .and_then(|actor| actor.parent_offset())
        else {
            continue;
        };
        let Some(parent) = parents.get(&child).and_then(|id| placed.get(id)).copied() else {
            continue;
        };
        let Some(mine) = placed.get_mut(&child) else {
            continue;
        };
        mine.translation = world_of(&parent, offset);
        let world = *mine;
        if let Some(actor) = project.actor_mut(&child) {
            actor.components.placement_mut().position = world.translation.to_array();
        }
    }
}

/// A `Place` as the transform the world is built with.
pub(crate) fn transform_of(placement: &blockloom_core::scene::Placement) -> Transform {
    let [rx, ry, rz] = placement.rotation;
    Transform {
        translation: Vec3::from(placement.position),
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            rx.to_radians(),
            ry.to_radians(),
            rz.to_radians(),
        ),
        scale: Vec3::from(placement.scale3()),
    }
}

/// Carries every parent's motion this step onto everything hanging off it.
///
/// Rather than reparenting Bevy's own transforms - which would make every
/// position in the engine relative to somebody and leave rapier owning half
/// of them - a child is moved by exactly the change its parent underwent
/// since the last step: `child = (parent now / parent then) * child`. A child
/// that moved itself this step keeps that motion, and one that is a parent in
/// turn passes the whole of it on, which is what the root-first order is for.
///
/// It runs in `FixedPostUpdate` before `record_poses`, so the parent's change
/// includes whatever physics wrote this step, and the pose the renderer
/// interpolates towards is the one the child ends up at.
pub fn apply_parenting(engine: NonSend<Engine>, mut posed: Query<(&mut Transform, &PhysicsPose)>) {
    if !engine.running || engine.paused || engine.parents.is_empty() {
        return;
    }
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (child, parent) in &engine.parents {
        children
            .entry(parent.as_str())
            .or_default()
            .push(child.as_str());
    }
    for parent in roots_first(&engine.parents) {
        let Some(mine) = children.get(parent.as_str()) else {
            continue;
        };
        let Some(entity) = engine.entities.get(&parent).copied() else {
            continue;
        };
        let Ok((transform, pose)) = posed.get(entity) else {
            continue;
        };
        if transform.translation.abs_diff_eq(pose.0.translation, 1e-6)
            && transform.rotation.abs_diff_eq(pose.0.rotation, 1e-6)
            && transform.scale.abs_diff_eq(pose.0.scale, 1e-6)
        {
            continue;
        }
        let delta = transform.compute_affine() * pose.0.compute_affine().inverse();
        for child in mine {
            let Some(entity) = engine.entities.get(*child).copied() else {
                continue;
            };
            if let Ok((mut transform, _)) = posed.get_mut(entity) {
                *transform = Transform::from_matrix(Mat4::from(delta * transform.compute_affine()));
            }
        }
    }
}

/// Every actor that is somebody's parent, parents before their own children,
/// so one pass carries a move all the way down a chain. `prune_parents` and
/// `set_parent` between them rule loops out, and the depth cap is what keeps
/// a hand-made one from hanging this.
pub(crate) fn roots_first(parents: &HashMap<String, String>) -> Vec<String> {
    let mut depths: Vec<(usize, &String)> = parents
        .values()
        .collect::<HashSet<&String>>()
        .into_iter()
        .map(|id| (depth_of(parents, id), id))
        .collect();
    depths.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    depths.into_iter().map(|(_, id)| id.clone()).collect()
}

/// How many parents deep `id` sits. The cap is the whole map, so a loop that
/// somehow got in answers rather than spinning.
pub(crate) fn depth_of(parents: &HashMap<String, String>, id: &str) -> usize {
    let mut at = id;
    for depth in 0..parents.len() {
        match parents.get(at) {
            Some(parent) => at = parent.as_str(),
            None => return depth,
        }
    }
    parents.len()
}
