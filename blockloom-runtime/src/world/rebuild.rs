//! Rebuilding the world from the document.

use super::*;

// ─── Building the world ────────────────────────────────────────────────────
/// Despawns everything and spawns it again from the project. Called on load,
/// on every Start (so actors go back where they were authored), and on stop.
/// How long the last [`rebuild_world`] took, in microseconds, for the
/// profiler's `world/rebuild` row. Spawning happens in the commands it queues,
/// so this is the system's own work, not the flush after it.
pub(crate) static LAST_REBUILD_US: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Times one rebuild, however it returns.
pub(crate) struct RebuildTimer {
    started: web_time::Instant,
    actors: usize,
}

impl RebuildTimer {
    fn start(actors: usize) -> Self {
        Self {
            started: web_time::Instant::now(),
            actors,
        }
    }
}

impl Drop for RebuildTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        LAST_REBUILD_US.store(
            elapsed.as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        tracing::debug!(
            "rebuilt the world in {:.2} ms ({} actors)",
            elapsed.as_secs_f64() * 1000.0,
            self.actors
        );
    }
}

/// The last rebuild's duration in milliseconds, or none before the first.
pub fn last_rebuild_ms() -> Option<f64> {
    let us = LAST_REBUILD_US.load(std::sync::atomic::Ordering::Relaxed);
    (us > 0).then(|| us as f64 / 1000.0)
}

pub fn rebuild_world(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut dimension: ResMut<Dimension>,
    mut effects: ResMut<PendingEffects>,
    actors: Query<Entity, With<ActorId>>,
    scenery: Query<Entity, Or<(With<WorldCamera>, With<WorldLight>)>>,
    voices: Query<Entity, With<crate::sound::VoiceTag>>,
    mut sound: ResMut<crate::sound::SoundState>,
    mut navmesh: Option<ResMut<NavMesh>>,
    mut performance: crate::performance::PerformanceStores,
    claims: Option<ResMut<crate::environment::ExposureClaims>>,
) {
    if !engine.rebuild {
        return;
    }
    engine.rebuild = false;
    commands.insert_resource(engine.document());
    commands.write_message(crate::module::WorldRebuilt);
    let _timer = RebuildTimer::start(engine.project.actors.len());
    // A scene switch across dimensions swaps the pipeline live: the Dimension
    // resource follows the newly active scene, so the spawn below picks up
    // the other side without a fresh process. Audio scale follows in
    // `sync_audio_scale` (a separate system, so this one stays within
    // Bevy's system-parameter limit).
    let mode = engine.project.active_scene().world.mode;
    if dimension.0 != mode {
        dimension.0 = mode;
    }
    // A `set exposure` lasts exactly as long as the run.
    if let Some(mut claims) = claims {
        claims.director = None;
    }
    if let Some(cache) = performance.cache.as_mut() {
        cache.clear();
    }
    if let Some(cells) = performance.cells.as_mut() {
        cells.clear();
    }
    if let Some(warmup) = performance.warmup.as_mut() {
        warmup.open();
    }

    for entity in &actors {
        commands.entity(entity).despawn();
    }
    for entity in &scenery {
        commands.entity(entity).despawn();
    }
    // Voices are neither actors nor cameras, so the passes above miss them:
    // a rebuild starts from silence at the saved mix.
    for entity in &voices {
        commands.entity(entity).despawn();
    }
    // Particles and ghosts go in `fx::despawn_fx`, just before this system.
    sound.reset(
        project_sound(&engine),
        engine.project.world.soundscape.clone(),
    );
    engine.entities.clear();
    engine.touching.clear();
    engine.contacts.clear();
    engine.contact_ticks = 0;
    blockloom_core::physics::query::reset();
    // Remaps last exactly as long as the run, like everything else live.
    engine.reset_input_run();
    // Everything the last run made goes with it - except opt-in survivors
    // carried across a scene switch, which live in `spawned` already and are
    // named by `survivor_keep`. Play/Stop/Load clears all; a switch keeps its
    // carriers. Clones never survive, even of a survivor.
    let keep = std::mem::take(&mut engine.survivor_keep);
    engine.spawned.retain(|id, _| keep.contains(id));
    engine.clear_live();
    // Survivor live state the reseed below would otherwise drop: attached
    // components, parents and collision filters ride along.
    let mut kept_attached: HashMap<String, HashSet<String>> = HashMap::new();
    let mut kept_parents: HashMap<String, String> = HashMap::new();
    let mut kept_filters: HashMap<String, (u8, u8, bool)> = HashMap::new();
    for id in &keep {
        if let Some(held) = engine.attached.get(id) {
            kept_attached.insert(id.clone(), held.clone());
        }
        if let Some(parent) = engine.parents.get(id) {
            kept_parents.insert(id.clone(), parent.clone());
        }
        if let Some(filter) = engine.physics_filter.get(id) {
            kept_filters.insert(id.clone(), *filter);
        }
    }
    engine.clones.clear();
    engine.last_created.clear();
    engine.weather = Default::default();
    engine.parents = engine
        .project
        .actors
        .iter()
        .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
        .collect();
    // Dropped here rather than left open: a rebuild follows a fresh build of
    // the libraries, and the old ones must be closed before the new ones open.
    engine.scripts.clear();
    engine.scripts_started.clear();
    engine.script_events.clear();
    engine.attached = engine
        .project
        .actors
        .iter()
        .map(|actor| {
            let held = actor
                .components
                .iter()
                .map(|component| component.name().to_string())
                .collect();
            (actor.id.clone(), held)
        })
        .collect();
    // Live collision filters start as the document authored them; the `set
    // trigger` / `set collision` blocks move them from there.
    engine.physics_filter = engine
        .project
        .actors
        .iter()
        .map(|actor| {
            let physics = actor.physics();
            (
                actor.id.clone(),
                (physics.layer(), physics.collision_mask, physics.trigger),
            )
        })
        .collect();
    // Survivors ride along: their live components, parents and filters
    // survive the reseed above.
    for (id, held) in kept_attached {
        engine.attached.insert(id, held);
    }
    for (id, parent) in kept_parents {
        // A survivor hanging off something gone hangs off nothing now.
        let parent_alive = engine.attached.contains_key(&parent)
            || engine.project.actor(&parent).is_some()
            || engine.spawned.contains_key(&parent);
        if parent_alive {
            engine.parents.insert(id, parent);
        }
    }
    for (id, filter) in kept_filters {
        engine.physics_filter.insert(id, filter);
    }

    let mut project = engine.project.clone();
    // A child authored in its parent's frame is put where that works out to,
    // once, before anything is spawned from these placements.
    place_authored_children(&mut project);
    // The level's live maps, parallax and rooms start from the document.
    let level = crate::tiles::Level::seed(&project);
    let streamed = level.streamed_maps();
    commands.insert_resource(level);
    match dimension.0 {
        Mode::TwoD => {
            // Exposure, tonemapping and post come from the blended
            // environment (`environment::apply_environment`).
            commands.spawn((
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scale: 1.0 / project.world.camera.zoom.max(0.05),
                    ..OrthographicProjection::default_2d()
                }),
                WorldCamera,
                // The one listener positional voices pan against. It rides
                // the camera, so what the player sees is what they hear.
                bevy::audio::SpatialListener::default(),
            ));
            for actor in &project.actors {
                let entity = dim2::spawn_actor(&mut commands, actor, streamed.contains(&actor.id))
                    .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::TwoD));
                attach_camera(&mut commands, actor, entity);
                crate::fx::insert_fx_state(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
            // Survivors stand where they stood, in the new scene alongside
            // its own actors, converted when crossing dimensions.
            for actor in engine.spawned.values().cloned().collect::<Vec<_>>() {
                let mut actor = actor;
                if let Some(visual) = actor.visual().cloned()
                    && visual.is_3d()
                {
                    actor
                        .components
                        .set_visual(blockloom_core::project::visual_for_mode(
                            &visual,
                            Mode::TwoD,
                        ));
                    actor.components.placement_mut().position[2] = 0.0;
                }
                let entity = dim2::spawn_actor(&mut commands, &actor, streamed.contains(&actor.id))
                    .unwrap_or_else(|| spawn_unseen(&mut commands, &actor, Mode::TwoD));
                attach_camera(&mut commands, &actor, entity);
                crate::fx::insert_fx_state(&mut commands, &actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
        Mode::ThreeD => {
            dim3::spawn_scenery(&mut commands, &project.world.camera);
            for actor in &project.actors {
                let entity = dim3::spawn_actor(&mut commands, actor)
                    .unwrap_or_else(|| spawn_unseen(&mut commands, actor, Mode::ThreeD));
                attach_camera(&mut commands, actor, entity);
                crate::fx::insert_fx_state(&mut commands, actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
            for actor in engine.spawned.values().cloned().collect::<Vec<_>>() {
                let mut actor = actor;
                if let Some(visual) = actor.visual().cloned()
                    && !visual.is_3d()
                    && !matches!(visual, blockloom_core::scene::Visual::Tilemap { .. })
                {
                    actor
                        .components
                        .set_visual(blockloom_core::project::visual_for_mode(
                            &visual,
                            Mode::ThreeD,
                        ));
                }
                let entity = dim3::spawn_actor(&mut commands, &actor)
                    .unwrap_or_else(|| spawn_unseen(&mut commands, &actor, Mode::ThreeD));
                attach_camera(&mut commands, &actor, entity);
                crate::fx::insert_fx_state(&mut commands, &actor, entity);
                engine.entities.insert(actor.id.clone(), entity);
            }
        }
    }
    // Rigidbody and Collider components become rapier bodies and child shapes.
    // A game in the editor cooks (and caches) meshes from their model files; a
    // shipped one only reads what its build carried.
    let collision: Box<dyn blockloom_core::physics::CollisionLookup> =
        match engine.project_dir.as_deref() {
            Some(dir) => Box::new(blockloom_core::physics::cook::FolderCollision::new(
                dir,
                if bridge::attached() {
                    blockloom_core::physics::cook::Source::Cook
                } else {
                    blockloom_core::physics::cook::Source::Shipped
                },
            )),
            None => Box::new(blockloom_core::physics::cook::NoCollisionData),
        };
    crate::physics_install::install_with(&mut commands, &project, &engine.entities, &*collision);
    // The dimension's own effect system owns the physics pipeline, so gravity
    // is set the same way a `set gravity` block would set it.
    effects.0.push(Effect::SetGravity {
        gravity: project.world.gravity,
    });
    // The navmesh walks what the editor shows: static ground as the boundary,
    // every other static solid as a hole. A bake that finds nothing to stand
    // on leaves `None`, and navigation steps straight at its target.
    if let Some(nav) = navmesh.as_deref_mut() {
        nav.mesh = nav::build_mesh(&project, nav::DEFAULT_AGENT_RADIUS)
            .map_err(|error| tracing::warn!("navmesh bake failed: {error}"))
            .ok()
            .map(Arc::new);
        nav.mode = project.world.mode;
        nav.settings = project.world.navigation.clone();
        crate::plugins::publish_nav(nav);
    }
    open_scripts(&mut engine, &project);
    // A built game answers to nobody, so it says what it built in its own
    // log. The editor already shows all of this in its own panels.
    if !bridge::attached() {
        let scripts: usize = engine.scripts.values().map(Vec::len).sum();
        info!(
            "built {} actors and opened {} scripts for '{}'",
            engine.entities.len(),
            scripts,
            engine.project.name
        );
    }
}

/// Keeps the audio listener's scale matched to the live dimension: pixels
/// in 2D hear in hundreds, metres in 3D hear as-is. Runs after every rebuild
/// (which is where a cross-dimension switch lands), so a separate system
/// keeps `rebuild_world` within Bevy's system-parameter limit.
pub fn sync_audio_scale(
    engine: NonSend<Engine>,
    scale: Option<ResMut<bevy::audio::DefaultSpatialScale>>,
) {
    let Some(mut scale) = scale else {
        return;
    };
    // Idempotent: setting the same scale every rebuild costs nothing and
    // keeps a cross-dimension switch hearing in the right units.
    *scale = if engine.project.active_scene().world.mode.is_3d() {
        bevy::audio::DefaultSpatialScale::default()
    } else {
        bevy::audio::DefaultSpatialScale(bevy::audio::SpatialScale::new_2d(1.0 / 500.0))
    };
}

/// The saved mix, for reseeding the runtime's live gains on a rebuild.
pub(crate) fn project_sound(engine: &Engine) -> blockloom_core::sound::SoundMixer {
    engine.project.settings.sound
}
