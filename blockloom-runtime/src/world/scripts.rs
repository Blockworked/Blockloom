//! Opening and stepping a run's scripts and compiled logic.

use super::*;

/// Opens every actor's compiled scripts, in component order. The editor
/// builds them before Play, and reports missing libraries and loader errors
/// to the editor.
pub(crate) fn open_scripts(engine: &mut Engine, project: &blockloom_core::project::Project) {
    let Some(dir) = engine.project_dir.clone() else {
        return;
    };
    for actor in project.actors.iter().chain(engine.spawned.values()) {
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
}

/// Opens native block logic only in a shipped player. Editor Play remains the
/// reference VM, including while an older compiled library is still present.
pub(crate) fn open_logic(engine: &mut Engine) {
    engine.logic = None;
    if bridge::attached() {
        return;
    }
    let Some(dir) = engine.project_dir.as_deref() else {
        return;
    };
    if !crate::logic::LoadedLogic::is_built(dir) {
        return;
    }
    match crate::logic::LoadedLogic::load(dir) {
        Ok(logic) => engine.logic = Some(logic),
        Err(message) => bridge::send(&RuntimeMessage::Error {
            actor: String::new(),
            message,
        }),
    }
}

/// Runs every script for this fixed step: `start` once per run, then `tick`
/// with the step's delta. An actor with several scripts runs them in
/// component order; their effects join the VM's in the same list, so scripts
/// and a canvas driving one actor are applied together, in order.
pub fn step_scripts(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut effects: ResMut<PendingEffects>,
    queries: crate::queries::QueryAccess,
) {
    if !engine.running || engine.paused || engine.scripts.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let mut actors: Vec<String> = engine.scripts.keys().cloned().collect();
    actors.sort();
    // `start` runs once per script rather than once per run, so a clone made
    // half way through gets its own, on the first step it exists for.
    let fresh: Vec<(String, String)> = actors
        .iter()
        .flat_map(|actor| {
            engine
                .scripts
                .get(actor)
                .into_iter()
                .flatten()
                .map(|entry| (actor.clone(), entry.path.clone()))
        })
        .filter(|key| !engine.scripts_started.contains(key))
        .collect();

    // What happened since the last step, as each script will hear it.
    let fired = std::mem::take(&mut engine.script_events);
    let name_of = |id: &str| {
        engine
            .actor(id)
            .map(|actor| actor.name.clone())
            .unwrap_or_default()
    };
    let heard: Vec<_> = fired
        .iter()
        .flat_map(|event| {
            let touch = crate::script::ScriptEvent::contact_of(event, name_of)
                .map(|(to, heard)| (Some(to), heard));
            crate::script::ScriptEvent::of(event, name_of)
                .into_iter()
                .chain(touch)
        })
        .collect();

    let mut asked = crate::script::Asked::default();
    // Scripts read and write the run's block variables and lists through the
    // same stores the VM does, so the two stay one working memory.
    let mut timed: Vec<(String, f64)> = Vec::new();
    crate::script::with_script_stores(&engine.variables, &engine.lists, &engine.dicts, || {
        queries.scope(engine.contact_ticks, || {
            for actor in &actors {
                let Some(scripts) = engine.scripts.get(actor) else {
                    continue;
                };
                for entry in scripts {
                    let started = web_time::Instant::now();
                    if fresh.contains(&(actor.clone(), entry.path.clone())) {
                        entry.script.start(actor, &mut asked);
                    }
                    for (to, event) in &heard {
                        if to.as_ref().is_none_or(|to| to == actor) {
                            entry.script.event(actor, &mut asked, event);
                        }
                    }
                    entry.script.tick(actor, &mut asked, dt);
                    timed.push((entry.path.clone(), started.elapsed().as_secs_f64() * 1000.0));
                }
            }
        });
    });
    for (path, ms) in timed {
        let slot = engine.script_times.entry(path).or_insert(ms);
        *slot += (ms - *slot) * 0.1;
    }
    engine.scripts_started.extend(fresh);
    for message in asked.messages.drain(..) {
        engine.fire(Event::Message(message));
    }
    script_lifetimes(&mut engine, &mut asked);
    let mut switch_request: Option<(String, String)> = None;
    for effect in &asked.effects {
        // Says and errors go to the editor the same way the VM's do.
        match effect {
            Effect::Say { actor, text } => {
                engine.note_say(actor, text);
                bridge::send(&RuntimeMessage::Say {
                    actor: actor.clone(),
                    text: text.clone(),
                });
            }
            Effect::Error { actor, message } => bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: message.clone(),
            }),
            Effect::SwitchScene {
                actor,
                scene,
                transition,
            } if switch_request.is_none() && engine.pending_scene.is_none() => {
                switch_request = Some((scene.clone(), transition.clone()));
                // A script's ask names who asked only for the log line;
                // the switch itself is window-global.
                let _ = actor;
            }
            _ => {}
        }
    }
    if let Some((scene, transition)) = switch_request {
        // Scripts report through the same validation as blocks: a typo
        // reports on the tick it was asked.
        match resolve_scene(&engine, &scene) {
            None => bridge::send(&RuntimeMessage::Error {
                actor: String::new(),
                message: format!("there's no scene named \"{scene}\""),
            }),
            Some(target) if target == engine.project.active_scene => {}
            Some(_) => {
                engine.fire(Event::SceneEnded);
                engine.veil.start(&transition);
                engine.pending_scene = Some((scene, transition, 2));
            }
        }
    }
    effects.0.append(&mut asked.effects);
}

/// Once per rendered frame: `frame` while unpaused (camera and UI motion on
/// the variable frame delta), and `ui` always (menus, on the same delta).
/// Fixed-step `tick` stays the physics clock; these are the presentation one.
///
/// While paused the fixed step skips scripts, so this drains the queued events
/// and hands them to `event` alongside `ui` - which is what lets a script
/// menu answer clicks while the world is frozen, the way UI strands do. While
/// unpaused events stay for the fixed step, so nothing is heard twice.
pub fn step_scripts_frame_ui(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut effects: ResMut<PendingEffects>,
    queries: crate::queries::QueryAccess,
) {
    if !engine.running || engine.scripts.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let paused = engine.paused;
    let mut actors: Vec<String> = engine.scripts.keys().cloned().collect();
    actors.sort();
    // While paused, the fixed step leaves events queued; hand them over here.
    // While unpaused, leave them for the fixed step.
    let heard: Vec<_> = if paused {
        let fired = std::mem::take(&mut engine.script_events);
        let name_of = |id: &str| {
            engine
                .actor(id)
                .map(|actor| actor.name.clone())
                .unwrap_or_default()
        };
        fired
            .iter()
            .flat_map(|event| {
                let touch = crate::script::ScriptEvent::contact_of(event, name_of)
                    .map(|(to, heard)| (Some(to), heard));
                crate::script::ScriptEvent::of(event, name_of)
                    .into_iter()
                    .chain(touch)
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut asked = crate::script::Asked::default();
    let mut timed: Vec<(String, f64)> = Vec::new();
    crate::script::with_script_stores(&engine.variables, &engine.lists, &engine.dicts, || {
        queries.scope(engine.contact_ticks, || {
            for actor in &actors {
                let Some(scripts) = engine.scripts.get(actor) else {
                    continue;
                };
                for entry in scripts {
                    let started = web_time::Instant::now();
                    if !paused {
                        entry.script.frame(actor, &mut asked, dt);
                    }
                    entry.script.ui(actor, &mut asked, dt);
                    if paused {
                        for (to, event) in &heard {
                            if to.as_ref().is_none_or(|to| to == actor) {
                                entry.script.event(actor, &mut asked, event);
                            }
                        }
                    }
                    timed.push((entry.path.clone(), started.elapsed().as_secs_f64() * 1000.0));
                }
            }
        });
    });
    for (path, ms) in timed {
        let slot = engine.script_times.entry(path).or_insert(ms);
        *slot += (ms - *slot) * 0.1;
    }
    for message in asked.messages.drain(..) {
        engine.fire(Event::Message(message));
    }
    script_lifetimes(&mut engine, &mut asked);
    for effect in &asked.effects {
        match effect {
            Effect::Say { actor, text } => {
                engine.note_say(actor, text);
                bridge::send(&RuntimeMessage::Say {
                    actor: actor.clone(),
                    text: text.clone(),
                });
            }
            Effect::Error { actor, message } => bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: message.clone(),
            }),
            _ => {}
        }
    }
    effects.0.append(&mut asked.effects);
}

/// The actor a script asked to make or unmake. Clones and fresh actors need
/// a scheduler slot before they need an entity, so they are registered with
/// whichever scheduler this run has and come back out as the same effects.
///
/// The VM mints its own ids; a compiled program mints its own too, so when
/// that is the scheduler the host mints a differently shaped one and hands
/// it over, which is what keeps the two from ever picking the same id.
pub(crate) fn script_lifetimes(engine: &mut Engine, asked: &mut crate::script::Asked) {
    for (actor, wanted) in asked.clones.drain(..) {
        let made = if engine.logic.is_some() {
            clone_for_logic(engine, &actor, &wanted)
        } else {
            engine.vm.clone_actor(&actor, &wanted)
        };
        match made {
            Some((clone, of)) => asked.effects.push(Effect::CreateClone { actor, clone, of }),
            None => bridge::send(&RuntimeMessage::Error {
                actor,
                message: format!("there's no actor named \"{wanted}\" to clone"),
            }),
        }
    }
    for (actor, name, position) in asked.created.drain(..) {
        let id = if engine.logic.is_some() {
            let id = engine.new_actor_id();
            if let Some(logic) = &mut engine.logic {
                logic.created(&id, &name);
            }
            id
        } else {
            engine.vm.create_actor(&name)
        };
        asked.effects.push(Effect::CreateActor {
            actor,
            id,
            name,
            position,
        });
    }
    for (actor, wanted) in asked.deleted.drain(..) {
        let gone = if engine.logic.is_some() {
            let gone = named_or_self(engine, &actor, &wanted);
            if let Some(gone) = gone.as_deref() {
                engine.variables.forget_actor(gone);
                engine.lists.forget_actor(gone);
                engine.dicts.forget_actor(gone);
                if let Some(logic) = &mut engine.logic {
                    logic.deleted(gone);
                }
            }
            gone
        } else {
            engine.vm.delete_actor(&actor, &wanted)
        };
        match gone {
            Some(gone) => asked.effects.push(Effect::DeleteActor { actor: gone }),
            None => bridge::send(&RuntimeMessage::Error {
                actor,
                message: format!("there's no actor named \"{wanted}\" to delete"),
            }),
        }
    }
}

/// Makes a clone on a script's behalf while a compiled program is the
/// scheduler: the host names the copy and the program adopts it, so the
/// copy's `when I start as a clone` strands run like any other clone's.
pub(crate) fn clone_for_logic(
    engine: &mut Engine,
    running: &str,
    wanted: &str,
) -> Option<(String, String)> {
    let of = named_or_self(engine, running, wanted)?;
    let clone = engine.new_actor_id();
    engine.variables.copy_actor(&of, &clone);
    engine.lists.copy_actor(&of, &clone);
    engine.dicts.copy_actor(&of, &clone);
    if let Some(logic) = &mut engine.logic {
        logic.cloned(&clone, &of);
    }
    Some((clone, of))
}

/// Which actor a script means, with an empty slot meaning itself - the rule
/// `Vm::find_actor` uses, and the one thing [`resolve_actor`] leaves out
/// because an empty slot elsewhere means nothing at all.
pub(crate) fn named_or_self(engine: &Engine, running: &str, wanted: &str) -> Option<String> {
    if wanted.trim().is_empty() {
        return Some(running.to_string());
    }
    resolve_actor(engine, running, wanted)
}

/// An actor with nothing to draw: it has no `Look` component, or one for the
/// other dimension. It still gets an entity, so its blocks can move it, other
/// actors can sense it, and adding a `Look` later is all it takes to see it.
pub(crate) fn spawn_unseen(commands: &mut Commands, actor: &Actor, mode: Mode) -> Entity {
    if actor
        .visual()
        .is_some_and(|visual| visual.is_3d() != mode.is_3d())
    {
        let wanted = if mode.is_3d() { "2D" } else { "3D" };
        warn!("{} has a {wanted} shape in a {mode:?} project", actor.name);
    }
    commands.spawn(actor_bundle(actor)).id()
}

/// Gives the actor its camera component, if the project put one on it.
pub(crate) fn attach_camera(commands: &mut Commands, actor: &Actor, entity: Entity) {
    if let Some(camera) = actor.camera() {
        commands.entity(entity).insert(CameraRig(*camera));
    }
}

// ─── Sensing ───────────────────────────────────────────────────────────────
