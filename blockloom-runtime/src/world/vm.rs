//! Stepping the block VM and switching scenes.

use super::*;

/// One block-program tick per fixed step, through native logic in a fast build
/// and the VM otherwise. Effects take the same path after either scheduler.
pub fn step_vm(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut effects: ResMut<PendingEffects>,
    transforms: Query<&Transform, With<ActorId>>,
    queries: crate::queries::QueryAccess,
) {
    // A paused world is still stepped: the scheduler gives a slice to the
    // strands the interface started and skips everything else, which is what
    // keeps a pause menu's own buttons alive.
    if !engine.running {
        return;
    }
    // A scene switch the blocks asked for earlier: count down so the
    // outgoing scene's `when scene ends` strands run first, then unload and
    // load. Two ticks: one for the ended strands to start, one for them to
    // run before the world goes away. A named transition also covers the
    // outgoing scene on the wall clock first, so the swap waits for cover.
    if engine.pending_scene.is_some() {
        if let Some((_, _, ticks)) = engine.pending_scene.as_mut()
            && *ticks > 0
        {
            *ticks -= 1;
        }
        let due = engine
            .pending_scene
            .as_ref()
            .is_some_and(|(_, _, ticks)| *ticks == 0);
        if due && engine.veil.ready_to_switch() {
            let (scene, transition, _) = engine.pending_scene.take().expect("checked above");
            perform_scene_switch(&mut engine, &transforms, &scene, &transition);
        }
    }
    deliver_contacts(&mut engine);
    let elapsed = time.elapsed_secs_f64();
    let now = engine.run_time(elapsed);
    let wall = engine.wall_time(real.elapsed_secs_f64());
    let scenes: Vec<_> = engine
        .project
        .scenes
        .iter()
        .map(|s| s.name.clone())
        .collect();
    blockloom_core::sense::publish_run(
        now,
        wall,
        engine.paused,
        &engine.project.active_scene().name,
        &scenes,
    );
    // Which slot this run writes to and which slots have files, plus the
    // run's language: sampled with the scenes so the VM, compiled logic
    // and reporters agree on the same tick.
    blockloom_core::sense::publish_saves(&engine.save_slot, &engine.save_slots);
    blockloom_core::sense::publish_language(&engine.language);
    let mut produced = Vec::new();
    let mut messages = Vec::new();
    let tick = engine.contact_ticks;
    queries.scope(tick, || {
        if engine.logic.is_some() {
            let variables = engine.variables.clone();
            let lists = engine.lists.clone();
            let dicts = engine.dicts.clone();
            engine.logic.as_mut().expect("checked above").tick(
                now,
                wall,
                variables,
                lists,
                dicts,
                &mut produced,
                &mut messages,
            );
        } else {
            engine.vm.tick_at(now, wall, &mut produced);
        }
    });
    for message in messages {
        engine.fire(Event::Message(message));
    }
    // The first `switch scene to` of the tick wins; the rest are dropped,
    // the way a second `stop all` is meaningless after the first.
    let mut switch_request: Option<(String, String, String)> = None;
    for effect in &produced {
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
            Effect::PluginCall {
                actor,
                plugin,
                block,
                args,
            } => {
                // A block whose command is a module op runs here, in the
                // plugin's world module; the editor runs the rest.
                if !crate::plugins::run_block(&mut engine, actor, plugin, block, args) {
                    bridge::send(&RuntimeMessage::PluginCall {
                        actor: actor.clone(),
                        plugin: plugin.clone(),
                        block: block.clone(),
                        args: args.clone(),
                    });
                }
            }
            Effect::SwitchScene {
                actor,
                scene,
                transition,
            } if switch_request.is_none() && engine.pending_scene.is_none() => {
                switch_request = Some((actor.clone(), scene.clone(), transition.clone()));
            }
            Effect::SwitchSaveSlot { actor, slot } => {
                apply_save_slot_switch(&mut engine, actor, slot);
            }
            Effect::DeleteSaveSlot { actor, slot } => {
                apply_save_slot_delete(&mut engine, actor, slot);
            }
            Effect::SetLanguage { language, .. } => {
                apply_set_language(&mut engine, language);
            }
            _ => {}
        }
    }
    if let Some((actor, scene, transition)) = switch_request {
        // Validate now so a typo reports on the tick it was asked, not two
        // ticks later when the switch would land.
        match resolve_scene(&engine, &scene) {
            None => bridge::send(&RuntimeMessage::Error {
                actor,
                message: format!("there's no scene named \"{scene}\""),
            }),
            Some(target) if target == engine.project.active_scene => {}
            Some(_) => {
                engine.fire(Event::SceneEnded);
                // Every switch covers first on the wall clock - including
                // across dimensions, since the rebuild swaps the pipeline
                // live under cover.
                engine.veil.start(&transition);
                engine.pending_scene = Some((scene, transition, 2));
            }
        }
    }
    effects.0.extend(produced);

    // An idle VM is still a live play session. It must keep accepting key,
    // click, collision, and broadcast events until Stop or `stop all` ends it.
}

/// A scene name as the blocks spell it into the scene id it names, or `None`
/// for nothing by that name. Empty names no scene, the way an empty actor
/// slot means "myself" rather than a scene.
pub(crate) fn resolve_scene(engine: &Engine, wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    engine
        .project
        .scenes
        .iter()
        .find(|scene| scene.name.eq_ignore_ascii_case(wanted))
        .map(|scene| scene.id.clone())
}

/// Unloads the current scene and loads `wanted` (a scene name as the block
/// spelled it): globals and save data carry over, actor locals start fresh
/// except opt-in survivors, and the new scene rebuilds and warms up like a
/// fresh Play before its `when scene starts` strands run. Works across
/// dimensions too: the rebuild swaps the dim2/dim3 pipeline live under the
/// veil's cover.
/// Loads the active scene's interface and screens, with the game's, into the manager.
pub(crate) fn load_interface(engine: &mut Engine, manager: &mut crate::ui::UiManager) {
    engine.interface_stale = false;
    let composed = blockloom_core::ui_layers::scene_interface(
        engine.project_dir.as_deref(),
        &engine.project.active_scene().world,
        &engine.project.settings,
    );
    manager.load(&composed.document);
    manager.set_screens(composed.screens, &composed.closed);
    engine.interface = composed.document;
    for message in composed.errors {
        bridge::send(&RuntimeMessage::Error {
            actor: String::new(),
            message,
        });
    }
}

pub(crate) fn perform_scene_switch(
    engine: &mut Engine,
    transforms: &Query<&Transform, With<ActorId>>,
    wanted: &str,
    transition: &str,
) {
    let _ = transition;
    let Some(target) = resolve_scene(engine, wanted) else {
        engine.veil.reset();
        return;
    };
    if target == engine.project.active_scene {
        engine.veil.reset();
        return;
    }
    // Opt-in survivors: authored or spawned actors carrying `Persist` stay
    // alive across the switch with live position, variables, lists, dicts
    // and attached components. Clones die even when their template survives.
    let keep = collect_survivors(engine);
    // Snapshot live placements into the carried docs before the world goes.
    for id in &keep {
        let live = engine
            .entities
            .get(id)
            .and_then(|entity| transforms.get(*entity).ok())
            .map(|transform| transform.translation.to_array());
        if let Some(position) = live {
            if let Some(actor) = engine.spawned.get_mut(id) {
                actor.components.placement_mut().position = position;
            } else if let Some(actor) = engine.project.active_scene().actor(id).cloned() {
                let mut carried = actor;
                carried.components.placement_mut().position = position;
                // A survivor whose parent didn't survive (and isn't in the
                // new scene) hangs off nothing rather than a ghost.
                if let Some(parent) = carried.parent().map(str::to_string) {
                    let parent_survives = keep.contains(&parent);
                    let parent_in_new = engine
                        .project
                        .scene(&target)
                        .is_some_and(|scene| scene.actor(&parent).is_some());
                    if !parent_survives && !parent_in_new {
                        carried.components.remove("Parent");
                    }
                }
                engine.spawned.insert(id.clone(), carried);
            }
        } else if !engine.spawned.contains_key(id) {
            // Alive without a transform (unseen)? Carry the authored doc.
            if let Some(actor) = engine.project.active_scene().actor(id).cloned() {
                engine.spawned.insert(id.clone(), actor);
            }
        }
    }
    // Survivors carry live pixels or metres: put them in the new scene's.
    let from = engine.project.active_scene().world.mode;
    let to = engine.project.scene(&target).map_or(from, |s| s.world.mode);
    for id in &keep {
        if let Some(actor) = engine.spawned.get_mut(id) {
            actor.convert_units(from, to);
        }
    }
    engine.project.active_scene = target.clone();
    // Run-made actors die with the scene they were made in, survivors
    // excepted; a clone's template lives in the new scene's document or in
    // the carried survivors, or not at all.
    engine.spawned.retain(|id, _| keep.contains(id));
    engine.survivor_keep = keep.clone();
    engine.clones.clear();
    engine.last_created.clear();
    engine.speech.retain(|id, _| keep.contains(id));
    engine.touching.clear();
    engine.contacts.clear();
    engine.contact_ticks = 0;
    engine.driven.retain(|id| keep.contains(id));
    let project = engine.project.clone();
    if let Some(logic) = &mut engine.logic {
        // The generated program carries every scene, so it switches the way
        // the VM does below. A stale build that never saw this scene - or one
        // from before multi-scene logic - falls back to the VM instead. So
        // does a switch carrying survivors: native survivors ride the VM
        // until the runner learns them.
        let stale = !logic.load_scene(&target);
        if stale || !keep.is_empty() {
            engine.logic = None;
        }
    }
    engine.vm.load_scene_keep(&project, &keep);
    engine.interface_stale = true;
    crate::plugins::scene_changed(engine);
    engine.rebuild = true;
    engine.fire(Event::SceneStarted);
    // The swap lands under cover; the reveal waits out the rebuild's warmup.
    engine.veil.begin_reveal();
}

/// Who survives the switch: live authored or spawned actors carrying
/// `Persist` (authored or attached mid-run). Clones never do, even of a
/// survivor - they die with the scene they were made in.
pub(crate) fn collect_survivors(engine: &Engine) -> std::collections::HashSet<String> {
    let mut keep = std::collections::HashSet::new();
    // Authored survivors: in the old scene's document, alive, persisting.
    for actor in &engine.project.active_scene().actors {
        if engine.clones.contains_key(&actor.id) {
            continue;
        }
        if !engine.entities.contains_key(&actor.id) {
            continue;
        }
        let authored = actor.persists();
        let attached = engine
            .attached
            .get(&actor.id)
            .is_some_and(|held| held.contains("Persist"));
        if authored || attached {
            keep.insert(actor.id.clone());
        }
    }
    // Spawned survivors: created mid-run and given `Persist` since. Clones
    // still die - only non-clone spawned actors can carry.
    for (id, actor) in &engine.spawned {
        if engine.clones.contains_key(id) {
            continue;
        }
        if !engine.entities.contains_key(id) {
            continue;
        }
        let authored = actor.persists();
        let attached = engine
            .attached
            .get(id)
            .is_some_and(|held| held.contains("Persist"));
        if authored || attached {
            keep.insert(id.clone());
        }
    }
    keep
}
