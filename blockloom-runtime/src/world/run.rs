//! The editor link and a run's life: messages from the editor, Play and Stop, saves, slots and pause.

use super::*;

pub fn pump_editor(
    mut engine: NonSendMut<Engine>,
    mut preview: Option<ResMut<crate::preview::PreviewState>>,
    mut design: Option<ResMut<crate::ui_design::DesignSession>>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut scene: Option<ResMut<crate::edit::SceneEditor>>,
    mut debug: Option<ResMut<crate::hdr::HdrDebug>>,
    mut volume_debug: Option<ResMut<crate::volumes::VolumeDebugView>>,
    mut captures: Option<ResMut<crate::capture::ExrCaptures>>,
    mut bakes: Option<ResMut<crate::light_probes::ProbeBaker>>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut fixed: ResMut<Time<Fixed>>,
    mut exit: MessageWriter<AppExit>,
) {
    // The project names its own fixed rate. Step-sized here, once a frame, so
    // the next FixedUpdate runs at whatever the loaded project asked for.
    let rate = engine.project.world.fixed_rate;
    if rate.is_finite() {
        fixed.set_timestep_hz(rate.clamp(1.0, 1000.0) as f64);
    }
    let now = time.elapsed_secs_f64();
    // A patch that can't respawn actors on their own loads the patched copy.
    let mut again: Option<EditorMessage> = None;
    loop {
        let message = match again
            .take()
            .map(Ok)
            .unwrap_or_else(|| engine.incoming.try_recv())
        {
            Ok(message) => message,
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            // The editor closed the pipe: nothing left to render for.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                exit.write(AppExit::Success);
                return;
            }
        };
        match message {
            EditorMessage::OpenLan { bind, max_guests } => {
                crate::lan::open(&mut engine, &bind, max_guests)
            }
            EditorMessage::CloseLan => crate::lan::close(&mut engine),
            EditorMessage::LanStatus => crate::lan::report(&engine, None),
            EditorMessage::KickGuest { id } => crate::lan::kick(&mut engine, id),
            EditorMessage::Patch { change } => {
                if crate::lan::status(&engine).open {
                    crate::lan::report(
                        &engine,
                        Some("Close LAN before loading edited content into the runtime".into()),
                    );
                    continue;
                }
                change.apply(&mut engine.project, true);
                let idle = !engine.running && !engine.starting && !engine.rebuild;
                match respawnable(&engine, &change).filter(|_| idle) {
                    Some(ids) => {
                        for id in ids {
                            if !engine.respawn.contains(&id) {
                                engine.respawn.push(id);
                            }
                        }
                    }
                    None => {
                        let project = Box::new(engine.project.clone());
                        let dir = engine
                            .project_dir
                            .as_ref()
                            .map(|dir| dir.to_string_lossy().into_owned());
                        again = Some(EditorMessage::Load { project, dir });
                    }
                }
                crate::plugins::preview(&mut engine);
            }
            EditorMessage::Load { project, dir } => {
                if crate::lan::status(&engine).open {
                    crate::lan::report(
                        &engine,
                        Some("Close LAN before loading edited content into the runtime".into()),
                    );
                    continue;
                }
                crate::lan::close(&mut engine);
                engine.project = *project;
                engine.project_dir = dir.map(std::path::PathBuf::from);
                let loaded = engine.project.clone();
                engine.vm.load(&loaded);
                load_saved_data(&mut engine);
                open_logic(&mut engine);
                engine.speech.clear();
                engine.pending_scene = None;
                engine.veil.reset();
                // The interface goes with the world it belonged to. Cleared
                // here rather than in `rebuild_world`, which runs after the
                // fixed step that builds this run's interface.
                manager.clear();
                engine.running = false;
                engine.starting = false;
                engine.paused = false;
                engine.pause_began = None;
                engine.rebuild = true;
                if let Some(scene) = scene.as_mut() {
                    scene.loaded = true;
                }
                // An open design session outlives the reload: the designer keeps waiting on it.
                if let Some(design) = design.as_mut()
                    && let Err(message) = design.reload(&engine, &mut manager)
                {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message,
                    });
                }
                crate::plugins::preview(&mut engine);
            }
            EditorMessage::Plugins { loadout } => {
                if engine.plugins.loadout.shaders != loadout.shaders {
                    engine.plugins.shaders_serial += 1;
                }
                #[cfg(feature = "plugins")]
                if engine.plugins.loadout.kernels != loadout.kernels {
                    engine.plugins.kernels_serial += 1;
                }
                let old = std::mem::replace(&mut engine.plugins.loadout, loadout);
                crate::plugins::reload(&mut engine, &old);
                crate::plugins::preview(&mut engine);
            }
            EditorMessage::InterfaceDesign { design: request } => {
                if let Some(design) = design.as_mut() {
                    if let Err(message) = design.apply(request, &engine, &mut manager) {
                        bridge::send(&RuntimeMessage::Error {
                            actor: String::new(),
                            message,
                        });
                    } else {
                        engine.preview_inputs.clear();
                    }
                }
            }
            EditorMessage::Start => {
                crate::lan::close(&mut engine);
                if let Some(design) = design.as_mut() {
                    design.clear();
                }
                let project = engine.project.clone();
                engine.vm.load(&project);
                load_saved_data(&mut engine);
                if let Some(logic) = &mut engine.logic {
                    logic.reset();
                }
                engine.touching.clear();
                engine.contacts.clear();
                engine.contact_ticks = 0;
                blockloom_core::physics::query::reset();
                engine.speech.clear();
                engine.pending_scene = None;
                engine.veil.reset();
                load_interface(&mut engine, &mut manager);
                engine.rebuild = true;
                engine.paused = false;
                engine.pause_began = None;
                // A rendering world holds the green flag until its first
                // frames can draw without compiling or loading anything.
                if engine.prewarm {
                    engine.running = false;
                    engine.starting = true;
                } else {
                    begin_run(&mut engine, now, real.elapsed_secs_f64());
                }
            }
            EditorMessage::Stop => {
                crate::lan::close(&mut engine);
                if let Some(design) = design.as_mut() {
                    design.clear();
                }
                end_run(&mut engine, &mut manager);
                bridge::send(&RuntimeMessage::Stopped);
            }
            EditorMessage::Pause { paused } => set_paused(&mut engine, paused, now),
            EditorMessage::Step => {
                // One fixed tick while paused, then re-pause at the end of
                // the fixed chain (`finish_step`). Ignored unless paused:
                // a running world is already stepping.
                if engine.running && engine.paused && !engine.pause_after_tick {
                    engine.paused = false;
                    engine.pause_began.take();
                    engine.vm.set_paused(false);
                    if let Some(logic) = &mut engine.logic {
                        logic.set_paused(false);
                    }
                    engine.pause_after_tick = true;
                }
            }
            EditorMessage::Preview { enabled, headless } => {
                let Some(preview) = preview.as_mut() else {
                    continue;
                };
                if enabled {
                    // Recorded before serving so a re-sent toggle applies
                    // while the sidecar stays up; visibility follows next
                    // frame in `apply_preview_visibility`.
                    preview.headless = headless;
                    match crate::preview::start_preview(preview) {
                        Some(port) => bridge::send(&RuntimeMessage::PreviewReady { port }),
                        None => bridge::send(&RuntimeMessage::PreviewStopped),
                    }
                } else {
                    crate::preview::stop_preview(preview);
                    bridge::send(&RuntimeMessage::PreviewStopped);
                }
            }
            EditorMessage::PreviewInput { input } => {
                if design.as_ref().is_none_or(|d| !d.active()) {
                    engine.preview_inputs.push(input);
                }
            }
            EditorMessage::SceneView(view) => {
                if let Some(debug) = debug.as_mut() {
                    debug.set_if_neq(crate::hdr::HdrDebug(view.debug_view));
                }
                if let Some(volumes) = volume_debug.as_mut() {
                    volumes.set_if_neq(crate::volumes::VolumeDebugView(view.volumes));
                }
                let Some(scene) = scene.as_mut() else {
                    continue;
                };
                // Turning the scene view off shows the game's own camera,
                // which only a rebuild puts back where the project says.
                if scene.view.enabled && !view.enabled && !engine.running {
                    engine.rebuild = true;
                }
                scene.view = view;
            }
            EditorMessage::Select { actor } => {
                if let Some(scene) = scene.as_mut() {
                    scene.selected = actor;
                }
            }
            EditorMessage::FrameSelected => {
                if let Some(scene) = scene.as_mut() {
                    scene.frame = true;
                }
            }
            EditorMessage::CaptureExr { path } => match captures.as_mut() {
                Some(captures) => captures.request(path),
                None => bridge::send(&RuntimeMessage::Error {
                    actor: "Blockloom".into(),
                    message: "This world can't capture EXR screenshots".into(),
                }),
            },
            EditorMessage::BakeProbes { actors } => match bakes.as_mut() {
                Some(bakes) => bakes.request(actors, true),
                None => bridge::send(&RuntimeMessage::Error {
                    actor: "Blockloom".into(),
                    message: "Light probes need a 3D world".into(),
                }),
            },
            EditorMessage::PreviewErosion { actor, erosion } => {
                engine.terrain_previews.push((actor, erosion));
            }
            EditorMessage::Shutdown => {
                crate::lan::close(&mut engine);
                exit.write(AppExit::Success);
                return;
            }
        }
    }
}

/// Gives every script its `stop` call as the run ends, in component order.
/// Movement and world effects are moot - the world is going away - so only
/// speech, errors and saves are honoured: a `stop` that logs or saves still
/// lands, one that moves does not.
pub(crate) fn run_script_stops(engine: &mut Engine) {
    if !engine.running || engine.scripts.is_empty() {
        return;
    }
    let mut actors: Vec<String> = engine.scripts.keys().cloned().collect();
    actors.sort();
    let mut asked = crate::script::Asked::default();
    crate::script::with_script_stores(&engine.variables, &engine.lists, &engine.dicts, || {
        for actor in &actors {
            let Some(scripts) = engine.scripts.get(actor) else {
                continue;
            };
            for entry in scripts {
                entry.script.stop(actor, &mut asked);
            }
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
            Effect::SaveVariable { actor, name, clear } => {
                apply_stop_save(engine, &actor, &name, clear);
            }
            _ => {}
        }
    }
}

/// Applies one `save variable` from a `stop` call at once, since the fixed
/// chain that would have carried it is done. Mirrors `apply_saved_data`.
pub(crate) fn apply_stop_save(engine: &mut Engine, actor: &str, name: &str, clear: bool) {
    let owner = engine
        .clones
        .get(actor)
        .cloned()
        .unwrap_or_else(|| actor.to_string());
    let project = engine.project.clone();
    let snapshot = engine.variables.snapshot();
    let changed = if clear {
        engine.save_data.clear(&project, &owner, name)
    } else {
        engine
            .save_data
            .capture(&project, &snapshot, &owner, actor, name)
    };
    if !changed && !clear {
        bridge::send(&RuntimeMessage::Error {
            actor: actor.to_string(),
            message: format!("there's no variable called \"{name}\" to save"),
        });
        return;
    }
    if changed {
        #[cfg(target_arch = "wasm32")]
        {
            if let Err(message) = crate::web::store_save_slot(
                &engine.project.id,
                &engine.save_slot,
                &engine.save_data,
            ) {
                bridge::send(&RuntimeMessage::Error {
                    actor: actor.to_string(),
                    message: format!("couldn't save variable \"{name}\": {message}"),
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Err(message) = blockloom_core::save::write(&engine.save_path, &engine.save_data)
            {
                bridge::send(&RuntimeMessage::Error {
                    actor: actor.to_string(),
                    message: format!("couldn't save variable \"{name}\": {message}"),
                });
            }
        }
    }
}

/// Ends the run and puts the world back as the document authored it, so
/// the editor's Game view never keeps what the run did to it.
pub fn end_run(engine: &mut Engine, manager: &mut crate::ui::UiManager) {
    crate::plugins::end(engine);
    run_script_stops(engine);
    engine.stop_program();
    engine.speech.clear();
    engine.pending_scene = None;
    engine.veil.reset();
    blockloom_core::sense::set_locale_table(None);
    manager.clear();
    engine.running = false;
    engine.starting = false;
    engine.paused = false;
    engine.pause_began = None;
    engine.rebuild = true;
    crate::plugins::preview(engine);
}

/// Presses the green flag: the run's clock starts now.
pub fn begin_run(engine: &mut Engine, now: f64, real_now: f64) {
    crate::script::clear_script_data();
    engine.starting = false;
    engine.running = true;
    engine.started_at = now;
    engine.wall_started_at = real_now;
    crate::lan::begin(engine);
    crate::plugins::begin(engine);
    engine.fire(Event::Started);
}

/// Where this run's saves live. Desktop uses the data dir; an APK's assets
/// are read-only, so Android uses the app's internal data dir instead.
#[cfg(target_os = "android")]
pub(crate) fn save_path_for(project_id: &str, slot: &str) -> std::path::PathBuf {
    crate::android::save_slot_path(project_id, slot)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn save_path_for(project_id: &str, slot: &str) -> std::path::PathBuf {
    blockloom_core::save::slot_path(project_id, slot)
}

/// Switches which save slot this run writes to and loads that slot's saved
/// variables into the run. A name with no file yet starts fresh; future
/// `save variable` blocks write to the new file.
pub(crate) fn apply_save_slot_switch(engine: &mut Engine, actor: &str, slot: &str) {
    use blockloom_core::save;
    let slot = save::normalize_slot(slot);
    if slot == engine.save_slot {
        return;
    }
    engine.save_slot = slot.clone();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = actor;
        engine.save_path = std::path::PathBuf::new();
        let data = crate::web::load_save_slot(&engine.project.id, &slot);
        data.apply(&engine.project, &engine.variables);
        engine.save_data = data;
        engine.save_slots = crate::web::list_save_slots(&engine.project.id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        engine.save_path = save_path_for(&engine.project.id, &slot);
        match save::read(&engine.save_path) {
            Ok(data) => {
                data.apply(&engine.project, &engine.variables);
                engine.save_data = data;
            }
            Err(message) => {
                engine.save_data = Default::default();
                bridge::send(&RuntimeMessage::Error {
                    actor: actor.to_string(),
                    message: format!("couldn't load save slot \"{slot}\": {message}"),
                });
            }
        }
        engine.save_slots = save::list_slots(&engine.project.id);
    }
    // The file may be new: make sure the slot lists even before anything
    // in it is saved.
    if !engine.save_slots.contains(&slot) {
        engine.save_slots.push(slot.clone());
        engine.save_slots.sort();
    }
    blockloom_core::sense::publish_saves(&engine.save_slot, &engine.save_slots);
}

/// Deletes one save slot's file without touching the live run. Quiet when
/// nothing by that name was saved.
pub(crate) fn apply_save_slot_delete(engine: &mut Engine, actor: &str, slot: &str) {
    use blockloom_core::save;
    let slot = save::normalize_slot(slot);
    #[cfg(target_arch = "wasm32")]
    {
        if let Err(message) = crate::web::delete_save_slot(&engine.project.id, &slot) {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.to_string(),
                message: format!("couldn't delete save slot \"{slot}\": {message}"),
            });
        }
        engine.save_slots = crate::web::list_save_slots(&engine.project.id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match save::delete_slot(&engine.project.id, &slot) {
            Ok(_) => {}
            Err(message) => {
                bridge::send(&RuntimeMessage::Error {
                    actor: actor.to_string(),
                    message: format!("couldn't delete save slot \"{slot}\": {message}"),
                });
                return;
            }
        }
        engine.save_slots = save::list_slots(&engine.project.id);
    }
    blockloom_core::sense::publish_saves(&engine.save_slot, &engine.save_slots);
}

/// Speaks the run's language for the rest of the run. Empty reads as the
/// project's default language.
pub(crate) fn apply_set_language(engine: &mut Engine, language: &str) {
    use blockloom_core::locale::Localization;
    let language = Localization::normalize_language(language);
    engine.language = if language.is_empty() {
        Localization::normalize_language(engine.project.localization.language_or_default())
    } else {
        language
    };
    if engine.language.is_empty() {
        engine.language = blockloom_core::locale::DEFAULT_LANGUAGE.to_string();
    }
    blockloom_core::sense::publish_language(&engine.language);
}

pub(crate) fn load_saved_data(engine: &mut Engine) {
    use blockloom_core::locale::{DEFAULT_LANGUAGE, Localization};
    use blockloom_core::save;
    // A fresh run opens the default slot speaking the project's language.
    // A slot name doubles as a profile name, so "Slot 1" and a player name
    // are the same file either way.
    engine.save_slot = save::DEFAULT_SLOT.to_string();
    let language =
        Localization::normalize_language(engine.project.localization.language_or_default());
    engine.language = if language.is_empty() {
        DEFAULT_LANGUAGE.to_string()
    } else {
        language
    };
    blockloom_core::sense::set_locale_table(Some(engine.project.localization.clone()));
    // The browser has no files: saves live in localStorage under the pack's
    // id instead (see `web`).
    #[cfg(target_arch = "wasm32")]
    {
        engine.save_path = std::path::PathBuf::new();
        let data = crate::web::load_save_slot(&engine.project.id, &engine.save_slot);
        data.apply(&engine.project, &engine.variables);
        engine.save_data = data;
        engine.save_slots = crate::web::list_save_slots(&engine.project.id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        engine.save_path = save_path_for(&engine.project.id, &engine.save_slot);
        match blockloom_core::save::read(&engine.save_path) {
            Ok(data) => {
                data.apply(&engine.project, &engine.variables);
                engine.save_data = data;
            }
            Err(message) => {
                engine.save_data = Default::default();
                bridge::send(&RuntimeMessage::Error {
                    actor: String::new(),
                    message: format!("couldn't load saved variables: {message}"),
                });
            }
        }
        engine.save_slots = save::list_slots(&engine.project.id);
    }
    blockloom_core::sense::publish_saves(&engine.save_slot, &engine.save_slots);
    blockloom_core::sense::publish_language(&engine.language);
}

/// Persists the variable slots named by this step's save-data effects. This
/// runs while paused too, since a settings menu is the common caller.
pub fn apply_saved_data(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        let Effect::SaveVariable { actor, name, clear } = effect else {
            continue;
        };
        let owner = engine
            .clones
            .get(actor)
            .cloned()
            .unwrap_or_else(|| actor.clone());
        let project = engine.project.clone();
        let snapshot = engine.variables.snapshot();
        let changed = if *clear {
            engine.save_data.clear(&project, &owner, name)
        } else {
            engine
                .save_data
                .capture(&project, &snapshot, &owner, actor, name)
        };
        if !changed && !clear {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: format!("there's no variable called \"{name}\" to save"),
            });
            continue;
        }
        if changed
            && let Err(message) = {
                #[cfg(target_arch = "wasm32")]
                {
                    crate::web::store_save_slot(
                        &engine.project.id,
                        &engine.save_slot,
                        &engine.save_data,
                    )
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    blockloom_core::save::write(&engine.save_path, &engine.save_data)
                }
            }
        {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: format!("couldn't save variable \"{name}\": {message}"),
            });
        }
    }
}

/// Freezes or thaws the world. The editor's Pause button and the `pause
/// game` block both land here, so the timer is shifted forward by the paused
/// span exactly once however the pause was asked for.
pub fn set_paused(engine: &mut Engine, paused: bool, now: f64) {
    if paused == engine.paused {
        return;
    }
    engine.paused = paused;
    if paused {
        engine.pause_began = Some(now);
    } else if let Some(began) = engine.pause_began.take() {
        // Shift the start forward by the paused span so the timer resumes
        // where it froze instead of jumping.
        engine.started_at += now - began;
    }
    // The schedulers keep their own copy: a paused world still gives a slice
    // to strands the interface started, so a pause menu's buttons work.
    engine.vm.set_paused(paused);
    if let Some(logic) = &mut engine.logic {
        logic.set_paused(paused);
    }
}

/// Re-pauses after a stepped tick (`EditorMessage::Step`). Runs last in the
/// fixed chain, so the one tick simulated fully before freezing again.
pub fn finish_step(mut engine: NonSendMut<Engine>, time: Res<Time>) {
    if !engine.pause_after_tick {
        return;
    }
    engine.pause_after_tick = false;
    if engine.running && !engine.paused {
        set_paused(&mut engine, true, time.elapsed_secs_f64());
    }
}
