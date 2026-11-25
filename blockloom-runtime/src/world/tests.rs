use super::*;
use crate::engine::InsertEngine;
use bevy::time::{TimePlugin, TimeUpdateStrategy};

#[test]
fn a_pose_mode_decides_how_a_step_is_drawn() {
    use blockloom_core::physics::Interpolation;
    let before = Transform::from_xyz(0.0, 0.0, 0.0);
    let after = Transform::from_xyz(10.0, 0.0, 0.0);
    let x = |mode, alpha| blend_pose(mode, &before, &after, alpha).translation.x;
    // None snaps to the step, Interpolate lags it, Extrapolate leads it.
    assert_eq!(x(Interpolation::None, 0.5), 10.0);
    assert_eq!(x(Interpolation::Interpolate, 0.5), 5.0);
    assert_eq!(x(Interpolation::Extrapolate, 0.5), 15.0);
    // A body at rest is left exactly where it is.
    assert_eq!(
        blend_pose(Interpolation::Extrapolate, &after, &after, 0.9),
        after
    );
}

#[test]
fn an_idle_vm_keeps_the_play_session_running() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;

    let mut app = fixed_step_app(engine);
    app.update();

    assert!(app.world().non_send::<Engine>().running);
}

/// An app that drives the fixed schedule deterministically: one
/// `app.update()` is exactly one fixed step at 60 Hz.
fn fixed_step_app(engine: Engine) -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin);
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.init_resource::<PendingEffects>();
    app.insert_engine(engine);
    app.add_systems(FixedUpdate, step_vm);
    app
}

#[test]
fn pause_freezes_the_timer_and_unpause_resumes_it() {
    let (sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.started_at = 10.0;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    app.init_resource::<PendingEffects>();
    app.init_resource::<crate::ui::UiManager>();
    app.init_resource::<Time<Real>>();
    app.insert_engine(engine);
    app.add_systems(Update, pump_editor);
    app.update();

    // Pause at t=15: timer freezes at 5.
    sender.send(EditorMessage::Pause { paused: true }).unwrap();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_to(std::time::Duration::from_secs(15));
    app.update();
    let engine = app.world().non_send::<Engine>();
    assert!(engine.paused);
    assert_eq!(engine.run_time(20.0), 5.0);
    assert_eq!(engine.run_time(15.0), 5.0);

    // Resume at t=25: timer continues from 5, not jumping to 15.
    sender.send(EditorMessage::Pause { paused: false }).unwrap();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_to(std::time::Duration::from_secs(25));
    app.update();
    let engine = app.world().non_send::<Engine>();
    assert!(!engine.paused);
    assert_eq!(engine.run_time(25.0), 5.0);
    assert_eq!(engine.run_time(30.0), 10.0);
}

#[test]
fn the_wall_clock_runs_through_a_pause_while_the_run_timer_freezes() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    begin_run(&mut engine, 10.0, 100.0);

    set_paused(&mut engine, true, 15.0);
    assert_eq!(engine.run_time(20.0), 5.0);
    assert_eq!(engine.wall_time(120.0), 20.0);

    // Resuming shifts the run timer, never the wall clock.
    set_paused(&mut engine, false, 25.0);
    assert_eq!(engine.run_time(30.0), 10.0);
    assert_eq!(engine.wall_time(130.0), 30.0);
    assert!(engine.wall_time(130.0) > engine.wall_time(120.0));
}

#[test]
fn the_wall_clock_ignores_game_speed_and_before_the_run_is_zero() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    begin_run(&mut engine, 0.0, 50.0);
    assert_eq!(engine.wall_time(40.0), 0.0);
    // 10 game seconds elapsed over 20 real ones (half speed).
    assert_eq!(engine.run_time(10.0), 10.0);
    assert_eq!(engine.wall_time(70.0), 20.0);
}

#[test]
fn step_vm_hands_the_scheduler_real_time_as_the_wall_clock() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    let mut app = fixed_step_app(engine);
    app.init_resource::<Time<Real>>();
    // Slow the game to a quarter: virtual and fixed time lag real time.
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_relative_speed(0.25);
    // A quarter-speed clock steps the fixed schedule only on some frames:
    // each step must see the real time of its own frame.
    let mut seen = Vec::new();
    for _ in 0..24 {
        app.update();
        let wall = app.world().non_send::<Engine>().vm.wall();
        let real = app.world().resource::<Time<Real>>().elapsed_secs_f64();
        if seen.last().is_none_or(|&(w, _)| w != wall) {
            seen.push((wall, real));
        }
    }
    assert!(seen.len() >= 3, "stepped {} times", seen.len());
    for (wall, real) in seen {
        assert!((wall - real).abs() < 1e-9, "wall {wall} vs real {real}");
    }
    let real = app.world().resource::<Time<Real>>().elapsed_secs_f64();
    let fixed = app.world().resource::<Time<Fixed>>().elapsed_secs_f64();
    assert!(fixed < real * 0.5);
}

#[test]
fn only_persist_carriers_survive_a_scene_switch_and_never_clones() {
    use blockloom_core::components::ActorComponent;
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    // Authored carrier with Persist, authored passer-by without, a clone
    // of the carrier, and a spawned actor with runtime-attached Persist.
    let mut carrier = blockloom_core::project::Actor::new(
        "Carrier",
        blockloom_core::scene::Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    carrier.components.insert(ActorComponent::Persist);
    let carrier_id = carrier.id.clone();
    let passer = blockloom_core::project::Actor::new(
        "Passer",
        blockloom_core::scene::Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    let passer_id = passer.id.clone();
    engine.project.scenes[0].actors = vec![carrier, passer];
    engine.project.active_scene = engine.project.scenes[0].id.clone();
    let clone_id = "~1".to_string();
    engine.clones.insert(clone_id.clone(), carrier_id.clone());
    let made = blockloom_core::project::Actor::new(
        "Made",
        blockloom_core::scene::Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    let made_id = made.id.clone();
    engine.spawned.insert(made_id.clone(), made);
    // All alive with entities; the clone carries Persist too but still dies.
    for id in [&carrier_id, &passer_id, &clone_id, &made_id] {
        engine.entities.insert(id.clone(), Entity::PLACEHOLDER);
        engine
            .attached
            .insert(id.clone(), ["Place".to_string()].into_iter().collect());
    }
    engine
        .attached
        .get_mut(&carrier_id)
        .unwrap()
        .insert("Persist".to_string());
    engine
        .attached
        .get_mut(&clone_id)
        .unwrap()
        .insert("Persist".to_string());
    engine
        .attached
        .get_mut(&made_id)
        .unwrap()
        .insert("Persist".to_string());
    let keep = collect_survivors(&engine);
    assert!(keep.contains(&carrier_id));
    assert!(keep.contains(&made_id));
    assert!(!keep.contains(&passer_id));
    assert!(!keep.contains(&clone_id));
}

#[test]
fn a_scene_switch_loads_the_new_scenes_interface() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    let first = engine.project.active_scene.clone();
    let cave = engine.project.add_scene("Cave", None);
    engine.project.active_scene = first;
    let mut torch = blockloom_core::ui::UiWidget::default();
    torch.element.id = "torch".into();
    engine
        .project
        .scenes
        .iter_mut()
        .find(|scene| scene.id == cave)
        .unwrap()
        .world
        .interface
        .widgets
        .push(torch);
    let mut manager = crate::ui::UiManager::default();
    load_interface(&mut engine, &mut manager);
    assert!(manager.get("torch").is_none());
    engine.project.active_scene = cave;
    engine.interface_stale = true;
    load_interface(&mut engine, &mut manager);
    assert!(!engine.interface_stale);
    assert!(manager.get("torch").is_some());
}

#[test]
fn preview_step_ticks_once_then_repauses_and_queues_input() {
    use blockloom_protocol::PreviewInput;

    let (sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.paused = true;
    engine.vm.set_paused(true);

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    app.init_resource::<Time<Real>>();
    app.init_resource::<PendingEffects>();
    app.init_resource::<crate::ui::UiManager>();
    app.insert_engine(engine);
    // No PreviewState resource: the sidecar arms are skipped, nothing panics.
    app.add_systems(Update, (pump_editor, finish_step).chain());

    sender.send(EditorMessage::Step).unwrap();
    sender
        .send(EditorMessage::PreviewInput {
            input: PreviewInput::Key {
                code: "Space".to_string(),
                down: true,
            },
        })
        .unwrap();
    sender
        .send(EditorMessage::Preview {
            enabled: true,
            headless: false,
        })
        .unwrap();
    app.update();

    let engine = app.world().non_send::<Engine>();
    // Stepped, then re-paused by the end of the tick.
    assert!(engine.paused);
    assert!(!engine.pause_after_tick);
    assert_eq!(engine.preview_inputs.len(), 1);
}

/// Runs `drive_camera` once over a world holding one rigged actor and one
/// camera, and hands back where the camera ended up.
fn camera_after(
    mode: Mode,
    rig: blockloom_core::components::CameraAttach,
    actor: Transform,
) -> Transform {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut app = App::new();
    app.insert_resource(Dimension(mode));
    app.init_resource::<Time>();
    app.init_resource::<crate::player_camera::BodyFacing>();
    app.add_message::<MouseWheel>();
    app.insert_engine(Engine::new(incoming, mode));
    let camera = app
        .world_mut()
        .spawn((WorldCamera, Transform::default()))
        .id();
    app.world_mut().spawn((CameraRig(rig), actor));
    app.add_systems(Update, drive_camera);
    app.update();
    *app.world().entity(camera).get::<Transform>().unwrap()
}

#[test]
fn a_first_person_camera_sits_at_the_actors_eye_facing_where_it_faces() {
    use blockloom_core::components::{CameraAttach, CameraView};

    let facing = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let camera = camera_after(
        Mode::ThreeD,
        CameraAttach {
            view: CameraView::FirstPerson,
            offset: [0.0, 2.0, 0.0],
            pitch: 30.0,
            ..CameraAttach::default()
        },
        Transform::from_xyz(5.0, 0.0, 0.0).with_rotation(facing),
    );

    assert!(
        camera
            .translation
            .abs_diff_eq(Vec3::new(5.0, 2.0, 0.0), 0.001)
    );
    // Yaw from the body, pitch from the rig: the FPS composition, so a
    // level body turning never weakens looking up and down.
    let looking = facing * Quat::from_rotation_x(30.0f32.to_radians());
    assert!(camera.rotation.abs_diff_eq(looking, 0.001));
}

#[test]
fn a_third_person_camera_sits_behind_and_above_what_it_looks_at() {
    use blockloom_core::components::{CameraAttach, CameraView};

    let camera = camera_after(
        Mode::ThreeD,
        CameraAttach {
            view: CameraView::ThirdPerson,
            offset: [0.0, 0.0, 0.0],
            distance: 10.0,
            pitch: 30.0,
            ..CameraAttach::default()
        },
        Transform::IDENTITY,
    );

    // Bevy's forward is -Z, so "behind" an unturned actor is +Z.
    assert!(camera.translation.z > 0.0);
    assert!((camera.translation.y - 5.0).abs() < 0.001);
    assert!(camera.translation.x.abs() < 0.001);
}

#[test]
fn a_2d_camera_only_ever_centres_on_its_actor() {
    use blockloom_core::components::{CameraAttach, CameraView};

    let camera = camera_after(
        Mode::TwoD,
        CameraAttach {
            // First person means nothing in a plane, so it still centres.
            view: CameraView::FirstPerson,
            offset: [0.0, 20.0, 0.0],
            ..CameraAttach::default()
        },
        Transform::from_xyz(100.0, 40.0, 0.0),
    );

    assert_eq!(camera.translation.truncate(), Vec2::new(100.0, 60.0));
}

#[test]
fn a_settings_slider_retunes_the_camera_while_paused() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    engine.paused = true;
    engine.vm.set_paused(true);

    let mut app = App::new();
    app.insert_resource(PendingEffects(vec![
        Effect::SetCameraFov {
            actor: "player".to_string(),
            fov: 90.0,
        },
        Effect::AttachComponent {
            actor: "player".to_string(),
            component: "Body".to_string(),
        },
    ]));
    app.insert_engine(engine);
    let actor = app
        .world_mut()
        .spawn((
            ActorId("player".to_string()),
            CameraRig(blockloom_core::components::CameraAttach::default()),
        ))
        .id();
    app.world_mut()
        .non_send_mut::<Engine>()
        .entities
        .insert("player".to_string(), actor);
    app.add_systems(Update, apply_component_effects);
    app.update();

    // The slider's tweak previews live, while structural changes wait
    // for the thaw like the rest of the simulation.
    let rig = app.world().entity(actor).get::<CameraRig>().unwrap();
    assert_eq!(rig.0.fov, 90.0);
    assert!(
        !app.world()
            .non_send::<Engine>()
            .has_component("player", "Body")
    );
}

#[test]
fn a_camera_tweak_from_an_actor_without_one_drives_the_holder() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    engine.paused = true;
    engine.vm.set_paused(true);

    let mut app = App::new();
    app.insert_resource(PendingEffects(vec![
        Effect::SetCameraFov {
            actor: "director".to_string(),
            fov: 95.0,
        },
        Effect::SetCameraPitch {
            actor: "director".to_string(),
            degrees: 20.0,
        },
        Effect::SetCameraView {
            actor: "director".to_string(),
            view: blockloom_core::components::CameraView::FirstPerson,
        },
    ]));
    app.insert_engine(engine);
    let player = app
        .world_mut()
        .spawn((
            ActorId("player".to_string()),
            CameraRig(blockloom_core::components::CameraAttach {
                fov: 80.0,
                pitch: 0.0,
                view: blockloom_core::components::CameraView::ThirdPerson,
                ..Default::default()
            }),
        ))
        .id();
    let director = app
        .world_mut()
        .spawn((ActorId("director".to_string()),))
        .id();
    {
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.entities.insert("player".to_string(), player);
        engine.entities.insert("director".to_string(), director);
    }
    app.add_systems(Update, apply_component_effects);
    app.update();

    let rig = app.world().entity(player).get::<CameraRig>().unwrap();
    assert_eq!(rig.0.fov, 95.0);
    assert_eq!(rig.0.pitch, 20.0);
    assert_eq!(
        rig.0.view,
        blockloom_core::components::CameraView::FirstPerson
    );
}

/// Runs `apply_cursor_lock` once over one primary window and hands back
/// what the engine wants plus what the window holds.
fn lock_after(app: &mut App, window_entity: Entity) -> (bool, CursorGrabMode, bool) {
    app.update();
    let engine = app.world().non_send::<Engine>();
    let wants = engine.wants_cursor_locked;
    let cursor = app
        .world()
        .entity(window_entity)
        .get::<CursorOptions>()
        .expect("the window keeps its cursor options");
    (wants, cursor.grab_mode, cursor.visible)
}

#[test]
fn a_lock_request_waits_for_focus_then_grabs() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    // No focus event has arrived: the window a fresh Play opens behind.
    // The component still says focused - its default - which is exactly
    // why the engine tracks event truth instead of reading it.

    let mut app = App::new();
    app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
        locked: true,
    }]));
    app.insert_engine(engine);
    let window_entity = app
        .world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
        .id();
    app.add_systems(Update, apply_cursor_lock);

    // Wanted, but unwritable: no grab while unfocused, where every
    // attempt only logs another failure.
    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::None, true)
    );

    // The effect is spent; a real focus event retries the same want
    // into a grab.
    app.world_mut().resource_mut::<PendingEffects>().0.clear();
    app.world_mut().non_send_mut::<Engine>().window_focused = true;
    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::Locked, false)
    );
    // ...centered first, so the pin lands inside the game instead of
    // freezing over whatever the cursor sat on. Default window,
    // scale one: middle pixel.
    let position = app
        .world()
        .entity(window_entity)
        .get::<Window>()
        .expect("the window")
        .physical_cursor_position();
    assert_eq!(position, Some(Vec2::new(640.0, 360.0)));
}

#[test]
fn ending_a_run_rebuilds_the_world_from_the_document() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.paused = true;
    engine.rebuild = false;
    engine.note_say("player", "Game over");
    let mut manager = crate::ui::UiManager::default();
    manager.show(element("score", blockloom_core::ui::UiKind::Label));

    end_run(&mut engine, &mut manager);

    // `stop all` lands here as well as the Stop button, so the Game view
    // goes back to the authored scene either way.
    assert!(engine.rebuild);
    assert!(!engine.running && !engine.paused && !engine.starting);
    assert!(engine.speech.is_empty());
    assert!(manager.get("score").is_none());
}

#[test]
fn stopping_the_run_releases_the_pointer() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    engine.window_focused = true;

    let mut app = App::new();
    app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
        locked: true,
    }]));
    app.insert_engine(engine);
    let window_entity = app
        .world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
        .id();
    app.add_systems(Update, apply_cursor_lock);

    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::Locked, false)
    );

    // Stop: the want goes with the run, so a finished game never keeps
    // the user's mouse.
    app.world_mut().non_send_mut::<Engine>().running = false;
    assert_eq!(
        lock_after(&mut app, window_entity),
        (false, CursorGrabMode::None, true)
    );
}

#[test]
fn a_tab_out_releases_and_the_return_trip_regrabs() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    engine.window_focused = true;

    let mut app = App::new();
    app.insert_resource(PendingEffects(vec![Effect::SetMouseLocked {
        locked: true,
    }]));
    app.insert_engine(engine);
    let window_entity = app
        .world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
        .id();
    app.add_systems(Update, apply_cursor_lock);

    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::Locked, false)
    );

    // Tabbing out drops the OS grab behind the component's back, so the
    // runtime lets go too - browser-style - while keeping the want.
    app.world_mut().non_send_mut::<Engine>().window_focused = false;
    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::None, true)
    );

    // Back in: the same want re-grabs without another block running.
    app.world_mut().non_send_mut::<Engine>().window_focused = true;
    assert_eq!(
        lock_after(&mut app, window_entity),
        (true, CursorGrabMode::Locked, false)
    );
}

/// Runs `publish_sensors` once over a primary window, after feeding one
/// motion event and a held left button, and hands back the published
/// delta and button state. `focused` drives real focus events: `None`
/// means none ever arrived, which is exactly what a window opened behind
/// the editor looks like.
fn motion_after(focused: Option<bool>) -> ([f32; 2], bool) {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut buttons = ButtonInput::<MouseButton>::default();
    buttons.press(MouseButton::Left);

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.init_resource::<ButtonInput<KeyCode>>();
    app.insert_resource(buttons);
    app.init_resource::<Messages<MouseMotion>>();
    app.init_resource::<Messages<WindowFocused>>();
    app.init_resource::<crate::ui::UiManager>();
    app.init_resource::<crate::sound::SoundState>();
    app.init_resource::<bevy::input::touch::Touches>();
    app.insert_engine(engine);
    let window_entity = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    app.world_mut()
        .resource_mut::<Messages<MouseMotion>>()
        .write(MouseMotion {
            delta: Vec2::new(5.0, -3.0),
        });
    if let Some(focused) = focused {
        app.world_mut()
            .resource_mut::<Messages<WindowFocused>>()
            .write(WindowFocused {
                window: window_entity,
                focused,
            });
    }
    app.init_resource::<Time<Real>>();
    app.add_systems(Update, publish_sensors);
    app.update();

    blockloom_core::sense::read(|sensors| (sensors.mouse_delta, sensors.mouse_down))
}

#[test]
fn an_unfocused_window_feeds_the_game_no_pointer() {
    // Raw device motion arrives whichever window holds the cursor -
    // including the editor beside a running game - so only a focused
    // window passes it on. The drain still runs, so focus landing later
    // starts from zero rather than a stale burst. No event at all reads
    // as unfocused: the component defaults to focused.
    assert_eq!(motion_after(None), ([0.0, 0.0], false));
    assert_eq!(motion_after(Some(false)), ([0.0, 0.0], false));
    assert_eq!(motion_after(Some(true)), ([5.0, -3.0], true));
}

#[test]
fn paused_vm_produces_no_effects() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.paused = true;
    engine.pause_began = Some(0.0);

    let mut app = fixed_step_app(engine);
    app.update();

    assert!(app.world().resource::<PendingEffects>().0.is_empty());
}

/// A world of loose actors with poses, and the parenting pass run once
/// over it. Each actor is `(id, pose at the start of the step, where it
/// is now)`, and the answer is where each one ends up.
fn after_parenting(
    actors: &[(&str, Transform, Transform)],
    parents: &[(&str, &str)],
) -> HashMap<String, Transform> {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;

    let mut app = App::new();
    let mut ids = Vec::new();
    for (id, pose, now) in actors {
        let entity = app
            .world_mut()
            .spawn((*now, PhysicsPose(*pose), PrevPose(*pose)))
            .id();
        engine.entities.insert((*id).to_string(), entity);
        ids.push(((*id).to_string(), entity));
    }
    for (child, parent) in parents {
        engine
            .parents
            .insert((*child).to_string(), (*parent).to_string());
    }
    app.insert_engine(engine);
    app.add_systems(Update, apply_parenting);
    app.update();

    ids.into_iter()
        .map(|(id, entity)| (id, *app.world().entity(entity).get::<Transform>().unwrap()))
        .collect()
}

#[test]
fn a_child_is_carried_by_exactly_what_its_parent_moved() {
    let was = Transform::from_xyz(0.0, 0.0, 0.0);
    let now = Transform::from_xyz(10.0, 4.0, 0.0);
    let child = Transform::from_xyz(100.0, 0.0, 0.0);
    let after = after_parenting(
        &[("parent", was, now), ("child", child, child)],
        &[("child", "parent")],
    );

    assert!(
        after["child"]
            .translation
            .abs_diff_eq(Vec3::new(110.0, 4.0, 0.0), 0.001)
    );
    // The parent itself is left exactly where it got to.
    assert!(
        after["parent"]
            .translation
            .abs_diff_eq(now.translation, 0.001)
    );
}

#[test]
fn a_turning_parent_swings_its_child_around_rather_than_sliding_it() {
    let was = Transform::IDENTITY;
    let now = Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let child = Transform::from_xyz(10.0, 0.0, 0.0);
    let after = after_parenting(
        &[("parent", was, now), ("child", child, child)],
        &[("child", "parent")],
    );

    // A quarter turn about the origin takes (10, 0) to (0, 10)...
    assert!(
        after["child"]
            .translation
            .abs_diff_eq(Vec3::new(0.0, 10.0, 0.0), 0.001)
    );
    // ...and turns the child with it.
    assert!(after["child"].rotation.abs_diff_eq(now.rotation, 0.001));
}

#[test]
fn a_chain_carries_the_whole_way_down_in_one_pass() {
    let was = Transform::from_xyz(0.0, 0.0, 0.0);
    let now = Transform::from_xyz(5.0, 0.0, 0.0);
    let middle = Transform::from_xyz(20.0, 0.0, 0.0);
    let leaf = Transform::from_xyz(30.0, 0.0, 0.0);
    let after = after_parenting(
        &[
            ("root", was, now),
            ("middle", middle, middle),
            ("leaf", leaf, leaf),
        ],
        &[("middle", "root"), ("leaf", "middle")],
    );

    assert!((after["middle"].translation.x - 25.0).abs() < 0.001);
    // The leaf gets the root's five once, not twice: the middle is done
    // before it is, and the delta it passes on is measured from its own
    // start-of-step pose.
    assert!((after["leaf"].translation.x - 35.0).abs() < 0.001);
}

#[test]
fn a_child_that_moved_itself_keeps_that_move_and_its_parents() {
    let was = Transform::from_xyz(0.0, 0.0, 0.0);
    let now = Transform::from_xyz(5.0, 0.0, 0.0);
    let child_was = Transform::from_xyz(0.0, 0.0, 0.0);
    let child_now = Transform::from_xyz(0.0, 7.0, 0.0);
    let after = after_parenting(
        &[("parent", was, now), ("child", child_was, child_now)],
        &[("child", "parent")],
    );

    assert!(
        after["child"]
            .translation
            .abs_diff_eq(Vec3::new(5.0, 7.0, 0.0), 0.001)
    );
}

#[test]
fn a_parent_that_did_not_move_leaves_its_child_alone() {
    let still = Transform::from_xyz(3.0, 3.0, 0.0);
    let child = Transform::from_xyz(9.0, 0.0, 0.0);
    let after = after_parenting(
        &[("parent", still, still), ("child", child, child)],
        &[("child", "parent")],
    );

    assert!(
        after["child"]
            .translation
            .abs_diff_eq(child.translation, 0.001)
    );
}

// ─── Authored local offsets ─────────────────────────────────────────────

/// A project of plain actors, each at a placement, with the hierarchy
/// and offsets the test asks for.
fn project_of(
    actors: &[(&str, blockloom_core::scene::Placement)],
    parents: &[(&str, &str, Option<[f32; 3]>)],
) -> blockloom_core::project::Project {
    let mut project = blockloom_core::project::Project::starter("Offsets", Mode::TwoD);
    project.actors.clear();
    for (id, placement) in actors {
        let mut actor = Actor::new(
            *id,
            Visual::Rect {
                color: "#fff".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.id = (*id).to_string();
        actor.components.set_placement(*placement);
        project.actors.push(actor);
    }
    for (child, parent, offset) in parents {
        let actor = project.actor_mut(child).unwrap();
        actor.components.set_parent(parent);
        actor.components.set_parent_offset(*offset);
    }
    project
}

fn at(x: f32, y: f32) -> blockloom_core::scene::Placement {
    blockloom_core::scene::Placement {
        position: [x, y, 0.0],
        ..Default::default()
    }
}

fn position_of(project: &blockloom_core::project::Project, id: &str) -> [f32; 3] {
    project.actor(id).unwrap().placement().position
}

#[test]
fn an_offset_child_is_placed_in_its_parents_frame() {
    let mut project = project_of(
        &[("parent", at(100.0, 50.0)), ("child", at(0.0, 0.0))],
        &[("child", "parent", Some([10.0, -5.0, 0.0]))],
    );
    place_authored_children(&mut project);

    assert_eq!(position_of(&project, "child"), [110.0, 45.0, 0.0]);
    assert_eq!(position_of(&project, "parent"), [100.0, 50.0, 0.0]);
}

#[test]
fn a_child_without_an_offset_stays_in_world_coordinates() {
    let mut project = project_of(
        &[("parent", at(100.0, 50.0)), ("child", at(7.0, 7.0))],
        &[("child", "parent", None)],
    );
    place_authored_children(&mut project);

    assert_eq!(position_of(&project, "child"), [7.0, 7.0, 0.0]);
}

#[test]
fn a_turned_parent_turns_the_offset_with_it() {
    let mut project = project_of(
        &[
            (
                "parent",
                blockloom_core::scene::Placement {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 90.0],
                    ..Default::default()
                },
            ),
            ("child", at(0.0, 0.0)),
        ],
        &[("child", "parent", Some([10.0, 0.0, 0.0]))],
    );
    place_authored_children(&mut project);

    // A quarter turn puts ten to the right ten above instead.
    let position = position_of(&project, "child");
    assert!(position[0].abs() < 0.001, "{position:?}");
    assert!((position[1] - 10.0).abs() < 0.001, "{position:?}");
}

#[test]
fn a_placed_child_reads_back_the_offset_it_was_placed_at() {
    let parent = transform_of(&at(100.0, 50.0));
    let child = world_of(&parent, [10.0, -5.0, 0.0]);

    assert_eq!(child.to_array(), [110.0, 45.0, 0.0]);
    // The reporters' read is the inverse of `set my parent to`'s write.
    assert_eq!(local_of(&parent, child), [10.0, -5.0, 0.0]);
}

#[test]
fn a_turned_parent_turns_the_local_reading_with_it() {
    let parent = transform_of(&blockloom_core::scene::Placement {
        position: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 90.0],
        ..Default::default()
    });
    let child = world_of(&parent, [10.0, 0.0, 0.0]);

    // Ten to the right in the parent's frame is ten above in the world.
    assert!(child.x.abs() < 0.001, "{child:?}");
    assert!((child.y - 10.0).abs() < 0.001, "{child:?}");
    let local = local_of(&parent, child);
    assert!((local[0] - 10.0).abs() < 0.001, "{local:?}");
    assert!(local[1].abs() < 0.001, "{local:?}");
}

#[test]
fn a_controller_teleport_updates_the_pose_physics_reads() {
    let (_, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut actor = Actor::new(
        "walker",
        Visual::Cuboid {
            color: "#fff".into(),
            size: [1.0; 3],
        },
    );
    actor.id = "walker".into();
    actor.components.insert(
        blockloom_core::components::ActorComponent::CharacterController {
            controller: Default::default(),
        },
    );
    engine.project.actors = vec![actor];
    let mut app = App::new();
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    app.insert_resource(Dimension(Mode::ThreeD));
    app.insert_resource(PendingEffects(vec![Effect::GoTo {
        actor: "walker".into(),
        position: [42.0, 35.0, 32.0],
    }]));
    let entity = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
            Visibility::Hidden,
        ))
        .id();
    engine.entities.insert("walker".into(), entity);
    app.insert_engine(engine);
    app.add_systems(Update, apply_common);
    app.update();
    assert_eq!(
        app.world()
            .get::<GlobalTransform>(entity)
            .unwrap()
            .translation(),
        Vec3::new(42.0, 35.0, 32.0)
    );
}

#[test]
fn navigation_drives_a_dynamic_body_without_teleporting_it() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut actor = Actor::new(
        "walker",
        Visual::Cuboid {
            color: "#fff".into(),
            size: [1.0; 3],
        },
    );
    actor.id = "walker".into();
    actor
        .components
        .set_physics(blockloom_core::scene::Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    engine.project.actors = vec![actor];
    let mut app = App::new();
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    app.insert_resource(Dimension(Mode::ThreeD));
    app.insert_resource(PendingEffects(vec![Effect::NavigateTo {
        actor: "walker".into(),
        target: [10.0, 0.0, 0.0],
        speed: 4.0,
    }]));
    let entity = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            bevy_rapier3d::prelude::Velocity::zero(),
        ))
        .id();
    engine.entities.insert("walker".into(), entity);
    app.insert_engine(engine);
    app.add_systems(Update, apply_common);
    app.update();
    let body = app.world().entity(entity);
    assert_eq!(body.get::<Transform>().unwrap().translation, Vec3::ZERO);
    assert!(
        (body
            .get::<bevy_rapier3d::prelude::Velocity>()
            .unwrap()
            .linear
            .x
            - 4.0)
            .abs()
            < 1e-4
    );
}

/// A running app with a parent at (100, 50) and a child at (7, 7)
/// carrying `offset`, with `set my parent to parent` queued for it.
fn reparent_app(offset: Option<[f32; 3]>) -> App {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    // The offset only places the child once something hangs it: at build
    // time that is the authored parent, at run time `set my parent to`.
    engine.project = project_of(
        &[("parent", at(100.0, 50.0)), ("child", at(7.0, 7.0))],
        &[("child", "parent", offset)],
    );
    engine.parents.clear();

    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<Assets<crate::materials::GraphMaterial2d>>();
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<bevy::sprite_render::ColorMaterial>>();
    app.insert_resource(Dimension(Mode::TwoD));
    app.insert_resource(PendingEffects(vec![Effect::SetParent {
        actor: "child".to_string(),
        parent: "parent".to_string(),
    }]));
    app.insert_engine(engine);
    for (id, x, y) in [("parent", 100.0, 50.0), ("child", 7.0, 7.0)] {
        let entity = app
            .world_mut()
            .spawn((
                ActorId(id.to_string()),
                Transform::from_xyz(x, y, 0.0),
                Visibility::Inherited,
            ))
            .id();
        app.world_mut()
            .non_send_mut::<Engine>()
            .entities
            .insert(id.to_string(), entity);
    }
    app.add_systems(Update, apply_lifetimes);
    app
}

#[test]
fn setting_a_parent_at_run_time_places_an_offset_child_in_its_frame() {
    let mut app = reparent_app(Some([10.0, -5.0, 0.0]));
    app.update();

    let engine = app.world().non_send::<Engine>();
    assert_eq!(
        engine.parents.get("child").map(String::as_str),
        Some("parent")
    );
    let child = engine.entities["child"];
    let transform = app.world().entity(child).get::<Transform>().unwrap();
    assert_eq!(transform.translation.to_array(), [110.0, 45.0, 0.0]);
}

#[test]
fn setting_a_parent_at_run_time_leaves_a_child_without_an_offset_where_it_stands() {
    let mut app = reparent_app(None);
    app.update();

    let engine = app.world().non_send::<Engine>();
    assert_eq!(
        engine.parents.get("child").map(String::as_str),
        Some("parent")
    );
    let child = engine.entities["child"];
    let transform = app.world().entity(child).get::<Transform>().unwrap();
    assert_eq!(transform.translation.to_array(), [7.0, 7.0, 0.0]);
}

#[test]
fn an_offset_down_a_chain_is_measured_against_a_parent_already_placed() {
    let mut project = project_of(
        &[
            ("root", at(100.0, 0.0)),
            ("middle", at(0.0, 0.0)),
            ("leaf", at(0.0, 0.0)),
        ],
        &[
            ("middle", "root", Some([10.0, 0.0, 0.0])),
            ("leaf", "middle", Some([1.0, 0.0, 0.0])),
        ],
    );
    place_authored_children(&mut project);

    assert_eq!(position_of(&project, "middle"), [110.0, 0.0, 0.0]);
    assert_eq!(position_of(&project, "leaf"), [111.0, 0.0, 0.0]);
}

#[test]
fn a_3d_yaw_spins_past_ninety_degrees_without_folding() {
    // Two hundred one-degree turns, the way a mouse-look strand
    // drives them. XYZ euler reads the yaw back folded into
    // [-90, 90] past the first quarter turn, which used to wall
    // every 3D game at about 180 degrees of horizontal look.
    let mut transform = Transform::IDENTITY;
    for _ in 0..200 {
        turn_3d(&mut transform, Axis::Y, 1f32.to_radians());
    }

    // 200 degrees wraps to -160, still level, and facing where a
    // full 200-degree yaw faces rather than folded back inside 90.
    let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
    assert!((yaw.to_degrees() + 160.0).abs() < 0.5, "{yaw:?}");
    assert!(pitch.abs() < 0.001, "{pitch:?}");
    assert!(roll.abs() < 0.001, "{roll:?}");
    let facing = *transform.forward();
    let expect = Vec3::new(-200f32.to_radians().sin(), 0.0, -200f32.to_radians().cos());
    assert!(facing.dot(expect) > 0.999, "{facing:?} vs {expect:?}");
}

#[test]
fn a_turn_move_turn_sandwich_strafes_a_dynamic_body() {
    use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

    // How strafe is spelled in blocks: face sideways, step, face back.
    // A dynamic body's `move` is deferred to the dimension pass as a
    // velocity, so turning in the common pass used to net the sandwich
    // to zero first and both strafe keys walked forward instead.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut project = blockloom_core::project::Project::starter("Strafe", Mode::ThreeD);
    project.actors.clear();
    let mut actor = Actor::new(
        "player",
        Visual::Rect {
            color: "#fff".to_string(),
            size: [1.0, 1.0],
        },
    );
    actor.id = "player".to_string();
    actor
        .components
        .set_physics(blockloom_core::scene::Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    project.actors.push(actor);
    engine.project = project;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    app.insert_resource(PendingEffects(vec![
        Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees: 90.0,
        },
        Effect::Move {
            actor: "player".to_string(),
            steps: 0.18,
        },
        Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees: -90.0,
        },
    ]));
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    let entity = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    app.world_mut().insert_engine(engine);
    app.world_mut()
        .non_send_mut::<Engine>()
        .entities
        .insert("player".to_string(), entity);
    app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
    app.update();

    // Sideways velocity, not forward: facing -Z, a +90 yaw looks down
    // -X, and the sandwich faces back afterwards.
    let velocity = app
        .world()
        .entity(entity)
        .get::<Velocity>()
        .expect("the body");
    assert!(velocity.linear.x < -100.0, "{:?}", velocity.linear);
    assert!(velocity.linear.z.abs() < 1.0, "{:?}", velocity.linear);
    assert_eq!(velocity.linear.y, 0.0);
    let facing = app.world().entity(entity).get::<Transform>().expect("pose");
    assert!(
        facing.rotation.angle_between(Quat::IDENTITY) < 0.01,
        "{:?}",
        facing.rotation
    );
}

#[test]
fn holding_forward_and_strafe_walks_diagonal() {
    use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

    // One tick of W+A in the First Person game: W's forward step and
    // A's turn-step-turn-back sandwich. Each deferred move writes only
    // its nonzero axes, so both survive into the velocity.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut project = blockloom_core::project::Project::starter("Diagonal", Mode::ThreeD);
    project.actors.clear();
    let mut actor = Actor::new(
        "player",
        Visual::Rect {
            color: "#fff".to_string(),
            size: [1.0, 1.0],
        },
    );
    actor.id = "player".to_string();
    actor
        .components
        .set_physics(blockloom_core::scene::Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    project.actors.push(actor);
    engine.project = project;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    let walk = |steps: f32| Effect::Move {
        actor: "player".to_string(),
        steps,
    };
    let level = Effect::SetRotation {
        actor: "player".to_string(),
        axis: Axis::X,
        degrees: 0.0,
    };
    let sideways = |degrees: f32| Effect::Turn {
        actor: "player".to_string(),
        axis: Axis::Y,
        degrees,
    };
    app.insert_resource(PendingEffects(vec![
        level.clone(),
        walk(0.22),
        level.clone(),
        sideways(90.0),
        walk(0.18),
        sideways(-90.0),
    ]));
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    let entity = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    app.world_mut().insert_engine(engine);
    app.world_mut()
        .non_send_mut::<Engine>()
        .entities
        .insert("player".to_string(), entity);
    app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
    app.update();

    // Forward (-Z) from W and left (-X) from A, both present.
    let velocity = app
        .world()
        .entity(entity)
        .get::<Velocity>()
        .expect("walker")
        .linear;
    assert!(velocity.x < -100.0, "{velocity:?}");
    assert!(velocity.z < -100.0, "{velocity:?}");
    assert_eq!(velocity.y, 0.0);
}

#[test]
fn holding_forward_and_strafe_walks_diagonal_after_a_real_turn_history() {
    use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

    // The live-game shape of W+A: the yaw came from ninety small mouse
    // turns, so the quaternion is inexact the way played yaw always is.
    // A turned facing reads back with float dust on its true-zero axes,
    // and each deferred move writing straight through let that dust -
    // past epsilon - clobber the other move's axis: W+A walked purely
    // sideways. Composing one tick's walks first keeps both.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut project = blockloom_core::project::Project::starter("Turned", Mode::ThreeD);
    project.actors.clear();
    let mut actor = Actor::new(
        "player",
        Visual::Rect {
            color: "#fff".to_string(),
            size: [1.0, 1.0],
        },
    );
    actor.id = "player".to_string();
    actor
        .components
        .set_physics(blockloom_core::scene::Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    project.actors.push(actor);
    engine.project = project;

    let mut start = Transform::IDENTITY;
    for _ in 0..90 {
        turn_3d(&mut start, Axis::Y, 1f32.to_radians());
    }
    let (yaw, _, _) = start.rotation.to_euler(EulerRot::YXZ);
    assert!((yaw.to_degrees() - 90.0).abs() < 1.0, "{yaw:?}");

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    let walk = |steps: f32| Effect::Move {
        actor: "player".to_string(),
        steps,
    };
    let level = Effect::SetRotation {
        actor: "player".to_string(),
        axis: Axis::X,
        degrees: 0.0,
    };
    let sideways = |degrees: f32| Effect::Turn {
        actor: "player".to_string(),
        axis: Axis::Y,
        degrees,
    };
    app.insert_resource(PendingEffects(vec![
        level.clone(),
        walk(0.22),
        level.clone(),
        sideways(90.0),
        walk(0.18),
        sideways(-90.0),
    ]));
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    let entity = app
        .world_mut()
        .spawn((
            start,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    app.world_mut().insert_engine(engine);
    app.world_mut()
        .non_send_mut::<Engine>()
        .entities
        .insert("player".to_string(), entity);
    app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
    app.update();

    // Facing ~90 degrees: W runs down -X, the sandwich steps up +Z.
    let velocity = app
        .world()
        .entity(entity)
        .get::<Velocity>()
        .expect("walker")
        .linear;
    assert!(velocity.x < -100.0, "{velocity:?}");
    assert!(velocity.z > 100.0, "{velocity:?}");
}

#[test]
fn opposing_walks_cancel_instead_of_cruising_stale() {
    use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

    // W+S in one tick name Z with equal and opposite steps: the axis
    // is genuinely walked, so it writes zero and stops rather than
    // keeping whatever the last tick left there. Unwalked axes cruise.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut project = blockloom_core::project::Project::starter("Cancel", Mode::ThreeD);
    project.actors.clear();
    let mut actor = Actor::new(
        "player",
        Visual::Rect {
            color: "#fff".to_string(),
            size: [1.0, 1.0],
        },
    );
    actor.id = "player".to_string();
    actor
        .components
        .set_physics(blockloom_core::scene::Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    project.actors.push(actor);
    engine.project = project;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    app.insert_resource(PendingEffects(vec![
        Effect::Move {
            actor: "player".to_string(),
            steps: 0.22,
        },
        Effect::Move {
            actor: "player".to_string(),
            steps: -0.22,
        },
    ]));
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    let entity = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    app.world_mut().insert_engine(engine);
    app.world_mut()
        .non_send_mut::<Engine>()
        .entities
        .insert("player".to_string(), entity);
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Velocity>()
        .expect("walker")
        .linear = Vec3::new(9.0, -2.0, 7.0);
    app.add_systems(Update, (apply_common, dim3::apply_effects).chain());
    app.update();

    let velocity = app
        .world()
        .entity(entity)
        .get::<Velocity>()
        .expect("walker")
        .linear;
    assert_eq!(velocity, Vec3::new(9.0, -2.0, 0.0));
}

#[test]
fn a_released_walk_brakes_but_leaves_falls_and_flying_balls_alone() {
    use bevy_rapier3d::prelude::{ExternalImpulse, Velocity};

    // A walk writes an absolute velocity every tick it runs, so the
    // tick after the strand goes quiet that speed used to glide on.
    // Braking only ever touches walk-driven actors, on every axis but
    // gravity's: falls, collisions and impulses keep their inertia.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::ThreeD);
    engine.running = true;
    let mut project = blockloom_core::project::Project::starter("Brakes", Mode::ThreeD);
    project.actors.clear();
    for id in ["player", "ball"] {
        let mut actor = Actor::new(
            id,
            Visual::Rect {
                color: "#fff".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor.id = id.to_string();
        actor
            .components
            .set_physics(blockloom_core::scene::Physics {
                body: BodyKind::Dynamic,
                ..Default::default()
            });
        project.actors.push(actor);
    }
    engine.project = project;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::ThreeD));
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_resource::<Assets<crate::materials::GraphMaterial3d>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<crate::performance::RenderCache>();
    app.init_resource::<crate::streaming::StreamingCells>();
    app.insert_resource(PendingEffects(vec![
        Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees: 90.0,
        },
        Effect::Move {
            actor: "player".to_string(),
            steps: 0.18,
        },
        Effect::Turn {
            actor: "player".to_string(),
            axis: Axis::Y,
            degrees: -90.0,
        },
    ]));
    app.add_message::<AppExit>();
    app.init_resource::<crate::ui::UiManager>();
    let player = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    let ball = app
        .world_mut()
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Velocity::default(),
            ExternalImpulse::default(),
        ))
        .id();
    app.world_mut().insert_engine(engine);
    {
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.entities.insert("player".to_string(), player);
        engine.entities.insert("ball".to_string(), ball);
    }
    // A mid-fall walker and a steadily rolling ball.
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Velocity>()
        .expect("walker")
        .linear
        .y = -2.0;
    app.world_mut()
        .entity_mut(ball)
        .get_mut::<Velocity>()
        .expect("ball")
        .linear = Vec3::new(5.0, -1.0, -3.0);
    app.add_systems(Update, (apply_common, dim3::apply_effects).chain());

    // Tick one: the sandwich strafes, and the fall is untouched.
    app.update();
    let walked = app
        .world()
        .entity(player)
        .get::<Velocity>()
        .expect("walker")
        .linear;
    assert!(walked.x < -100.0, "{walked:?}");
    assert_eq!(walked.y, -2.0);

    // Tick two: silence. The walker stops dead but keeps falling; the
    // never-walk-driven ball rolls on untouched.
    app.world_mut().resource_mut::<PendingEffects>().0.clear();
    app.update();
    let stopped = app
        .world()
        .entity(player)
        .get::<Velocity>()
        .expect("walker")
        .linear;
    assert_eq!(stopped, Vec3::new(0.0, -2.0, 0.0));
    let rolling = app
        .world()
        .entity(ball)
        .get::<Velocity>()
        .expect("ball")
        .linear;
    assert_eq!(rolling, Vec3::new(5.0, -1.0, -3.0));

    // An explicit impulse is a physics verb, not a walk: it persists
    // through the quiet ticks exactly like a collision would.
    app.world_mut()
        .resource_mut::<PendingEffects>()
        .0
        .push(Effect::ApplyImpulse {
            actor: "ball".to_string(),
            impulse: [1.0, 0.0, 0.0],
        });
    app.update();
    app.world_mut().resource_mut::<PendingEffects>().0.clear();
    app.update();
    let thrown = app
        .world()
        .entity(ball)
        .get::<Velocity>()
        .expect("ball")
        .linear;
    assert_eq!(thrown, Vec3::new(6.0, -1.0, -3.0));
}

#[test]
fn zeroing_pitch_and_roll_leaves_a_far_3d_yaw_alone() {
    // The game's anti-capsize strand zeroes X and Z every tick.
    // Through XYZ euler that snaps a yaw past 90 back inside it;
    // through YXZ it is an identity write that changes nothing.
    let mut transform = Transform::IDENTITY;
    turn_3d(&mut transform, Axis::Y, (-130f32).to_radians());
    set_rotation_3d(&mut transform, Axis::X, 0.0);
    set_rotation_3d(&mut transform, Axis::Z, 0.0);

    let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
    assert!((yaw.to_degrees() + 130.0).abs() < 0.5, "{yaw:?}");
    assert!(pitch.abs() < 0.001, "{pitch:?}");
    assert!(roll.abs() < 0.001, "{roll:?}");
}

// ─── Routing a click through the interface ─────────────────────────────

/// One element, drawn where a layout pass would have put it: a box of
/// `size` centred on `centre`. That rectangle is all the hit test reads.
fn drawn_element(app: &mut App, spec: blockloom_core::ui::UiElement, centre: Vec2, size: Vec2) {
    let entity = app
        .world_mut()
        .spawn((
            ComputedNode {
                size,
                ..Default::default()
            },
            UiGlobalTransform::from_translation(centre),
        ))
        .id();
    let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
    let id = spec.id.clone();
    manager.show(spec);
    manager.take_pending();
    manager.attach(&id, entity);
}

fn element(id: &str, kind: blockloom_core::ui::UiKind) -> blockloom_core::ui::UiElement {
    blockloom_core::ui::UiElement {
        id: id.to_string(),
        kind,
        ..Default::default()
    }
}

/// An app with one clickable world actor at the origin whose canvas says
/// which route a click took, a pointer parked at `cursor`, and the left
/// button just pressed.
fn click_harness(cursor: Vec2) -> App {
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};

    let headed = |header: K, text: &str| {
        Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(header),
                Instruction::new(K::Say {
                    text: blockloom_core::value::Value::text(text),
                }),
            ],
        )
    };
    let mut actor = Actor::new(
        "Target",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [400.0, 400.0],
        },
    );
    actor.id = "a1".to_string();
    actor.graph.strands = vec![
        headed(K::WhenClicked, "world"),
        headed(
            K::WhenUiClicked {
                element: "resume".to_string(),
            },
            "ui",
        ),
        headed(
            K::WhenUiChanged {
                element: "shadows".to_string(),
            },
            "changed",
        ),
        headed(
            K::WhenUiChanged {
                element: "volume".to_string(),
            },
            "moved",
        ),
    ];

    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.project.actors = vec![actor];
    let project = engine.project.clone();
    engine.vm.load(&project);
    engine.running = true;
    engine.window_focused = true;

    let mut buttons = ButtonInput::<MouseButton>::default();
    buttons.press(MouseButton::Left);

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.insert_resource(buttons);
    app.init_resource::<crate::ui::UiManager>();

    let entity = app
        .world_mut()
        .spawn((ActorId("a1".to_string()), Transform::default()))
        .id();
    engine.entities.insert("a1".to_string(), entity);
    app.insert_engine(engine);

    let mut window = Window::default();
    window.set_physical_cursor_position(Some(bevy::math::DVec2::new(
        cursor.x as f64,
        cursor.y as f64,
    )));
    app.world_mut().spawn((window, PrimaryWindow));
    // A camera the world pick projects through, parked so the origin is
    // under the middle of the window.
    app.world_mut().spawn((
        Camera2d,
        Camera::default(),
        Transform::default(),
        GlobalTransform::default(),
        WorldCamera,
    ));
    app.add_systems(Update, detect_clicks);
    app
}

/// Routes the click, then gives the VM one tick and answers with what
/// the strands the click started had to say.
fn routed(app: &mut App) -> Vec<String> {
    app.update();
    let mut effects = Vec::new();
    let mut engine = app.world_mut().non_send_mut::<Engine>();
    let paused = engine.paused;
    engine.vm.set_paused(paused);
    engine.vm.tick(0.0, &mut effects);
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Say { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_click_on_an_element_starts_its_strand_and_never_reaches_the_world() {
    let mut app = click_harness(Vec2::new(100.0, 100.0));
    drawn_element(
        &mut app,
        element("resume", blockloom_core::ui::UiKind::Button),
        Vec2::new(100.0, 100.0),
        Vec2::new(80.0, 30.0),
    );
    assert_eq!(routed(&mut app), vec!["ui".to_string()]);
}

/// Whether the world would be offered the click this app just routed.
/// The pick itself needs a render target, which a headless app has none
/// of, so the rule is checked where it is decided.
fn world_offered(app: &mut App) -> bool {
    app.update();
    let engine = app.world().non_send::<Engine>();
    let manager = app.world().resource::<crate::ui::UiManager>();
    world_takes_clicks(engine, manager)
}

#[test]
fn a_click_that_misses_every_element_is_offered_to_the_world() {
    let mut app = click_harness(Vec2::new(480.0, 360.0));
    drawn_element(
        &mut app,
        element("resume", blockloom_core::ui::UiKind::Button),
        Vec2::new(100.0, 100.0),
        Vec2::new(80.0, 30.0),
    );
    // Nothing in the interface answered, and nothing is swallowing.
    assert!(routed(&mut app).is_empty());
    assert!(world_offered(&mut app));
}

#[test]
fn a_click_beside_a_modal_panel_is_swallowed_rather_than_hitting_the_world() {
    let mut app = click_harness(Vec2::new(480.0, 360.0));
    let mut menu = element("menu", blockloom_core::ui::UiKind::Panel);
    menu.modal = true;
    // Drawn somewhere the pointer isn't: the swallow is about the panel
    // being up, not about what the click landed on.
    drawn_element(&mut app, menu, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
    assert!(routed(&mut app).is_empty());
    assert!(!world_offered(&mut app));
}

#[test]
fn the_same_click_reaches_the_world_once_the_panel_is_hidden() {
    let mut app = click_harness(Vec2::new(480.0, 360.0));
    let mut menu = element("menu", blockloom_core::ui::UiKind::Panel);
    menu.modal = true;
    drawn_element(&mut app, menu, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
    app.world_mut()
        .resource_mut::<crate::ui::UiManager>()
        .hide("menu", false);
    assert!(world_offered(&mut app));
}

#[test]
fn a_world_click_is_dead_while_paused_but_an_element_still_answers() {
    let mut app = click_harness(Vec2::new(100.0, 100.0));
    drawn_element(
        &mut app,
        element("resume", blockloom_core::ui::UiKind::Button),
        Vec2::new(100.0, 100.0),
        Vec2::new(80.0, 30.0),
    );
    app.world_mut().non_send_mut::<Engine>().paused = true;
    // The strand a UI click started runs on, frozen world or not.
    assert_eq!(routed(&mut app), vec!["ui".to_string()]);

    let mut beside = click_harness(Vec2::new(480.0, 360.0));
    beside.world_mut().non_send_mut::<Engine>().paused = true;
    assert!(routed(&mut beside).is_empty());
    assert!(!world_offered(&mut beside));
}

#[test]
fn clicking_a_text_input_hands_it_the_keyboard_and_clicking_away_takes_it_back() {
    let mut app = click_harness(Vec2::new(100.0, 100.0));
    drawn_element(
        &mut app,
        element("name", blockloom_core::ui::UiKind::Input),
        Vec2::new(100.0, 100.0),
        Vec2::new(180.0, 30.0),
    );
    app.update();
    assert_eq!(
        app.world().resource::<crate::ui::UiManager>().focus(),
        Some("name")
    );

    // The pointer moves off it; the next click releases the keyboard.
    let mut windows = app.world_mut().query::<&mut Window>();
    for mut window in windows.iter_mut(app.world_mut()) {
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(500.0, 400.0)));
    }
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert_eq!(app.world().resource::<crate::ui::UiManager>().focus(), None);
}

#[test]
fn clicking_a_toggle_turns_it_over_and_reports_the_change() {
    let mut app = click_harness(Vec2::new(100.0, 100.0));
    let mut toggle = element("shadows", blockloom_core::ui::UiKind::Toggle);
    toggle.value = blockloom_core::value::Evaluated::Bool(false);
    drawn_element(
        &mut app,
        toggle,
        Vec2::new(100.0, 100.0),
        Vec2::new(120.0, 30.0),
    );
    // The click and the change both land, in that order.
    assert_eq!(routed(&mut app), vec!["changed".to_string()]);
    assert_eq!(
        app.world()
            .resource::<crate::ui::UiManager>()
            .get("shadows")
            .map(|node| node.value.clone()),
        Some(blockloom_core::value::Evaluated::Bool(true))
    );
}

#[test]
fn dragging_a_slider_lands_on_the_number_its_own_ends_name() {
    // Two thirds of the way across a 90-wide track centred on 100.
    let mut app = click_harness(Vec2::new(115.0, 100.0));
    let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
    slider.range = [0.0, 90.0];
    slider.value = blockloom_core::value::Evaluated::Number(0.0);
    drawn_element(
        &mut app,
        slider,
        Vec2::new(100.0, 100.0),
        Vec2::new(90.0, 20.0),
    );
    assert_eq!(routed(&mut app), vec!["moved".to_string()]);

    let at = app
        .world()
        .resource::<crate::ui::UiManager>()
        .get("volume")
        .and_then(|node| node.value.as_number().ok());
    assert_eq!(at, Some(60.0));
}

#[test]
fn a_slider_click_reaches_the_sensor_snapshot_the_same_frame() {
    // Two thirds of the way across a 90-wide track centred on 100. A
    // `when input changed` strand reads the new number through the
    // snapshot, so the snapshot has to hold 60 - the click's own value -
    // rather than the 0 it just moved away from. Input lands before
    // publishing: the chain below mirrors main.rs, where
    // `publish_sensors` follows `detect_clicks`.
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.window_focused = true;

    let mut buttons = ButtonInput::<MouseButton>::default();
    buttons.press(MouseButton::Left);

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.insert_resource(buttons);
    app.init_resource::<ButtonInput<KeyCode>>();
    app.init_resource::<Messages<MouseMotion>>();
    app.init_resource::<Messages<WindowFocused>>();
    app.init_resource::<crate::ui::UiManager>();
    app.init_resource::<crate::sound::SoundState>();
    app.init_resource::<bevy::input::touch::Touches>();
    app.insert_engine(engine);

    let mut window = Window::default();
    window.set_physical_cursor_position(Some(bevy::math::DVec2::new(115.0, 100.0)));
    app.world_mut().spawn((window, PrimaryWindow));

    let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
    slider.range = [0.0, 90.0];
    slider.value = blockloom_core::value::Evaluated::Number(0.0);
    drawn_element(
        &mut app,
        slider,
        Vec2::new(100.0, 100.0),
        Vec2::new(90.0, 20.0),
    );
    app.init_resource::<Time<Real>>();
    app.add_systems(Update, (detect_clicks, publish_sensors).chain());
    app.update();

    let seen = blockloom_core::sense::read(|sensors| {
        sensors.ui.get("volume").map(|sense| sense.value.clone())
    });
    assert_eq!(seen.and_then(|value| value.as_number().ok()), Some(60.0));
}

/// The whole menu stack with nothing mocked: the blocks show a modal
/// panel with a parented button, real Bevy layout draws it, and a click
/// on the button's own drawn box starts its strand while paused.
#[test]
fn a_drawn_menu_button_answers_a_click_on_its_own_box() {
    use bevy::asset::AssetPlugin;
    use bevy::text::TextPlugin;
    use bevy::time::TimePlugin;
    use bevy::ui::UiPlugin;
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
    use blockloom_core::ui::{UiAnchor, UiElement, UiKind};
    use blockloom_core::value::{Evaluated, Value};

    let mut actor = Actor::new(
        "Player",
        Visual::Rect {
            color: "#fff".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = "a1".to_string();
    actor.graph.strands = vec![Strand::with_instructions(
        0,
        0,
        vec![
            Instruction::new(K::WhenUiClicked {
                element: "resume_btn".to_string(),
            }),
            Instruction::new(K::Say {
                text: Value::text("ui"),
            }),
        ],
    )];

    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.project.actors = vec![actor];
    let project = engine.project.clone();
    engine.vm.load(&project);
    engine.running = true;
    engine.paused = true;
    engine.vm.set_paused(true);
    engine.window_focused = true;

    let mut manager = crate::ui::UiManager::default();
    manager.show(UiElement {
        id: "pause_menu".to_string(),
        kind: UiKind::Panel,
        content: "Paused".to_string(),
        anchor: UiAnchor::Center,
        offset: [0.0, 0.0],
        size: [320.0, 0.0],
        parent: String::new(),
        modal: true,
        range: [0.0, 100.0],
        value: Evaluated::Text(String::new()),
    });
    manager.show(UiElement {
        id: "resume_btn".to_string(),
        kind: UiKind::Button,
        content: "Resume".to_string(),
        anchor: UiAnchor::Center,
        offset: [0.0, 0.0],
        size: [0.0, 0.0],
        parent: "pause_menu".to_string(),
        modal: false,
        range: [0.0, 100.0],
        value: Evaluated::Bool(false),
    });

    let mut app = App::new();
    app.add_plugins((AssetPlugin::default(), TimePlugin, TextPlugin, UiPlugin));
    // Bevy's own viewport picking runs but has nothing to say here; it
    // only needs its two maps and its messages to exist.
    app.init_resource::<bevy::picking::hover::HoverMap>();
    app.init_resource::<bevy::picking::events::PointerState>();
    app.add_message::<bevy::picking::pointer::PointerInput>();
    app.add_message::<bevy::picking::backend::PointerHits>();
    app.init_asset::<bevy::image::Image>();
    app.init_asset::<bevy::image::TextureAtlasLayout>();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.init_resource::<ButtonInput<MouseButton>>();
    app.init_resource::<bevy::input::touch::Touches>();
    app.insert_engine(engine);
    app.insert_resource(manager);
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut().spawn((
        Camera2d,
        Camera::default(),
        Transform::default(),
        GlobalTransform::default(),
        WorldCamera,
    ));
    app.add_systems(Update, (crate::overlay::draw_ui, detect_clicks).chain());

    // Spawn, then let layout measure the button's real box.
    for _ in 0..5 {
        app.update();
    }
    let centre: Vec2 = {
        let entity = app
            .world()
            .resource::<crate::ui::UiManager>()
            .get("resume_btn")
            .expect("the button was drawn")
            .entity;
        let world = app.world_mut();
        let mut laid_out = world.query::<(&ComputedNode, &UiGlobalTransform)>();
        let (node, transform) = laid_out.get(world, entity).expect("a laid-out box");
        assert!(
            node.size().x > 0.0 && node.size().y > 0.0,
            "the button drew at no size: {node:?}"
        );
        transform.translation
    };

    let mut windows = app.world_mut().query::<&mut Window>();
    for mut window in windows.iter_mut(app.world_mut()) {
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(
            centre.x as f64,
            centre.y as f64,
        )));
    }
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();

    let mut effects = Vec::new();
    {
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.vm.tick(0.0, &mut effects);
    }
    let said: Vec<String> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Say { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(said, vec!["ui".to_string()]);
}

/// A click on a high-DPI display: layout boxes are physical pixels while
/// the cursor reads logical ones, so the router has to scale the pointer
/// up the way Bevy's own picking does. The box below is the live report
/// that found this: a 200% menu the clicks fell straight through.
#[test]
fn a_click_at_double_scale_lands_on_the_physical_box() {
    // A 1920x1440 window at 200%: logical space is 960x720.
    let mut app = click_harness(Vec2::new(480.0, 360.0));
    drawn_element(
        &mut app,
        element("resume", blockloom_core::ui::UiKind::Button),
        Vec2::new(960.0, 720.0),
        Vec2::new(592.0, 72.0),
    );
    // The physical point above reads as half in logical pixels - which
    // is what the router is handed.
    let mut windows = app.world_mut().query::<&mut Window>();
    for mut window in windows.iter_mut(app.world_mut()) {
        window.resolution =
            bevy::window::WindowResolution::new(1920, 1440).with_scale_factor_override(2.0);
        // The OS cursor sits over the rendered button, in raw physical
        // pixels like a real pointer would.
        window.set_physical_cursor_position(Some(bevy::math::DVec2::new(960.0, 720.0)));
    }
    app.world_mut().non_send_mut::<Engine>().paused = true;
    assert_eq!(routed(&mut app), vec!["ui".to_string()]);
}

/// An app with one text input holding the keyboard, ready to be typed
/// into, and an actor whose canvas says when that input changed. The
/// element carries whatever `rules` writes to it.
fn typing_harness(rules: impl FnOnce(&mut crate::ui::UiManager)) -> App {
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};

    let mut actor = Actor::new(
        "Target",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = "a1".to_string();
    actor.graph.strands = vec![Strand::with_instructions(
        0,
        0,
        vec![
            Instruction::new(K::WhenUiChanged {
                element: "name".to_string(),
            }),
            Instruction::new(K::Say {
                text: blockloom_core::value::Value::text("typed"),
            }),
        ],
    )];

    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.project.actors = vec![actor];
    let project = engine.project.clone();
    engine.vm.load(&project);
    engine.running = true;
    engine.window_focused = true;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.init_resource::<Messages<KeyboardInput>>();
    app.init_resource::<crate::ui::UiManager>();
    app.insert_engine(engine);
    app.add_systems(Update, type_into_focused_input);

    let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
    manager.show(element("name", blockloom_core::ui::UiKind::Input));
    manager.take_pending();
    manager.focus_on(Some("name"));
    rules(&mut manager);
    app
}

/// Queues one keypress per character, as a keyboard would.
fn keys_in(app: &mut App, word: &str) {
    for ch in word.chars() {
        let key = if ch == ' ' {
            Key::Space
        } else {
            Key::Character(ch.to_string().into())
        };
        app.world_mut()
            .resource_mut::<Messages<KeyboardInput>>()
            .write(KeyboardInput {
                key_code: KeyCode::KeyA,
                logical_key: key,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
    }
}

fn type_word(app: &mut App, word: &str) {
    keys_in(app, word);
    app.update();
}

fn typed_text(app: &App) -> String {
    app.world()
        .resource::<crate::ui::UiManager>()
        .get("name")
        .map(|node| node.value.as_text())
        .unwrap_or_default()
}

#[test]
fn an_input_takes_every_character_until_its_rules_say_otherwise() {
    let mut app = typing_harness(|_| {});
    type_word(&mut app, "Ada 7!");
    assert_eq!(typed_text(&app), "Ada 7!");
}

fn press(app: &mut App, logical_key: Key) {
    app.world_mut()
        .resource_mut::<Messages<KeyboardInput>>()
        .write(KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    app.update();
}

#[test]
fn arrows_home_delete_and_composition_edit_around_the_caret() {
    let mut app = typing_harness(|_| {});
    type_word(&mut app, "held");
    press(&mut app, Key::ArrowLeft);
    press(&mut app, Key::ArrowLeft);
    type_word(&mut app, "x");
    assert_eq!(typed_text(&app), "hexld");
    press(&mut app, Key::Delete);
    assert_eq!(typed_text(&app), "hexd");
    press(&mut app, Key::Home);
    press(&mut app, Key::Delete);
    assert_eq!(typed_text(&app), "exd");
    press(&mut app, Key::End);
    press(&mut app, Key::Backspace);
    assert_eq!(typed_text(&app), "ex");
    // The caret is drawn only while the input holds the keyboard.
    let shown = |app: &App, focused| {
        let manager = app.world().resource::<crate::ui::UiManager>();
        crate::ui::display_text(manager.get("name").unwrap(), focused)
    };
    assert_eq!(shown(&app, true), "ex");
    assert_eq!(shown(&app, false), "ex");
    // An input method's composition is shown at the caret, then committed.
    app.init_resource::<Messages<Ime>>();
    app.world_mut()
        .resource_mut::<Messages<Ime>>()
        .write(Ime::Preedit {
            window: Entity::PLACEHOLDER,
            value: "ni".into(),
            cursor: Some((2, 2)),
        });
    app.update();
    assert_eq!(shown(&app, true), "exni");
    assert_eq!(typed_text(&app), "ex");
    app.world_mut()
        .resource_mut::<Messages<Ime>>()
        .write(Ime::Commit {
            window: Entity::PLACEHOLDER,
            value: "你".into(),
        });
    app.update();
    assert_eq!(typed_text(&app), "ex你");
    assert_eq!(shown(&app, true), "ex你");
}

#[test]
fn a_numeric_input_turns_away_what_is_not_a_number() {
    let mut app = typing_harness(|manager| {
        manager.set(
            "name",
            blockloom_core::ui::UiProp::Allow,
            &Evaluated::Text("numbers".to_string()),
        );
    });
    // The letters are simply not there: a refused keystroke rubs itself
    // out rather than reporting, since a person holding a key down
    // means no harm by it.
    type_word(&mut app, "-12a.5b");
    assert_eq!(typed_text(&app), "-12.5");
}

#[test]
fn a_full_input_takes_no_more_however_long_the_key_is_held() {
    let mut app = typing_harness(|manager| {
        manager.set(
            "name",
            blockloom_core::ui::UiProp::MaxLength,
            &Evaluated::Number(3.0),
        );
    });
    type_word(&mut app, "Adamant");
    assert_eq!(typed_text(&app), "Ada");

    // Rubbing one out makes room for exactly one more.
    app.world_mut()
        .resource_mut::<Messages<KeyboardInput>>()
        .write(KeyboardInput {
            key_code: KeyCode::Backspace,
            logical_key: Key::Backspace,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    app.update();
    type_word(&mut app, "ze");
    assert_eq!(typed_text(&app), "Adz");
}

#[test]
fn a_keystroke_an_input_took_starts_its_changed_strand() {
    let mut app = typing_harness(|_| {});
    keys_in(&mut app, "h");
    assert_eq!(routed(&mut app), vec!["typed".to_string()]);
}

#[test]
fn a_keystroke_an_input_refused_changes_nothing_and_starts_nothing() {
    let mut app = typing_harness(|manager| {
        manager.set(
            "name",
            blockloom_core::ui::UiProp::Allow,
            &Evaluated::Text("digits".to_string()),
        );
    });
    keys_in(&mut app, "h");
    assert!(routed(&mut app).is_empty());
    assert_eq!(typed_text(&app), "");
}

/// An app with one actor whose canvas answers the back button, and the
/// `back_button` system on Update. The physical code is arbitrary here:
/// on a phone winit sends `Unidentified` and only the logical key names
/// the button.
fn back_harness() -> App {
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};

    let mut actor = Actor::new(
        "Hero",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = "hero".to_string();
    actor.graph.strands = vec![Strand::with_instructions(
        0,
        0,
        vec![
            Instruction::new(K::WhenKeyPressed {
                key: "back".to_string(),
            }),
            Instruction::new(K::Say {
                text: blockloom_core::value::Value::text("went-back"),
            }),
        ],
    )];

    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.project.actors = vec![actor];
    let project = engine.project.clone();
    engine.vm.load(&project);
    engine.running = true;

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.init_resource::<Messages<KeyboardInput>>();
    app.init_resource::<crate::ui::UiManager>();
    app.insert_engine(engine);
    app.add_systems(Update, back_button);
    app
}

fn press_back(app: &mut App) {
    app.world_mut()
        .resource_mut::<Messages<KeyboardInput>>()
        .write(KeyboardInput {
            key_code: KeyCode::Space,
            logical_key: Key::BrowserBack,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
}

#[test]
fn the_back_button_starts_its_key_strand() {
    let mut app = back_harness();
    press_back(&mut app);
    assert_eq!(routed(&mut app), vec!["went-back".to_string()]);
}

#[test]
fn a_modal_swallows_the_back_button_like_a_click() {
    let mut app = back_harness();
    let mut menu = element("menu", blockloom_core::ui::UiKind::Panel);
    menu.modal = true;
    let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
    manager.show(menu);
    manager.take_pending();
    press_back(&mut app);
    assert!(routed(&mut app).is_empty());
}

#[test]
fn the_back_button_fires_while_paused_like_escape() {
    let mut app = back_harness();
    app.world_mut().non_send_mut::<Engine>().paused = true;
    press_back(&mut app);
    assert_eq!(routed(&mut app), vec!["went-back".to_string()]);
}

#[test]
fn a_focused_input_keeps_the_back_button_for_its_keyboard() {
    let mut app = back_harness();
    let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
    manager.show(element("name", blockloom_core::ui::UiKind::Input));
    manager.take_pending();
    manager.focus_on(Some("name"));
    press_back(&mut app);
    assert!(routed(&mut app).is_empty());
}

#[test]
fn the_focus_block_hands_the_keyboard_over_without_a_click() {
    let mut app = typing_harness(|manager| {
        manager.show(element("other", blockloom_core::ui::UiKind::Input));
        manager.take_pending();
    });
    app.init_resource::<PendingEffects>();
    app.add_systems(Update, crate::overlay::apply_ui_effects);

    app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::SetFocus {
        id: "other".to_string(),
    }];
    app.update();
    assert_eq!(
        app.world().resource::<crate::ui::UiManager>().focus(),
        Some("other")
    );

    // `clear focus` is the same effect naming nobody.
    app.world_mut().resource_mut::<PendingEffects>().0 =
        vec![Effect::SetFocus { id: String::new() }];
    app.update();
    assert_eq!(app.world().resource::<crate::ui::UiManager>().focus(), None);
}

#[test]
fn dragging_a_slider_with_a_step_lands_on_one_of_its_stops() {
    // Two thirds of the way across a 90-wide track centred on 100.
    let mut app = click_harness(Vec2::new(115.0, 100.0));
    let mut slider = element("volume", blockloom_core::ui::UiKind::Slider);
    slider.range = [0.0, 90.0];
    slider.value = Evaluated::Number(0.0);
    drawn_element(
        &mut app,
        slider,
        Vec2::new(100.0, 100.0),
        Vec2::new(90.0, 20.0),
    );
    app.world_mut().resource_mut::<crate::ui::UiManager>().set(
        "volume",
        blockloom_core::ui::UiProp::Step,
        &Evaluated::Number(45.0),
    );
    assert_eq!(routed(&mut app), vec!["moved".to_string()]);

    // 60 along the track, rounded to the nearest stop.
    let at = app
        .world()
        .resource::<crate::ui::UiManager>()
        .get("volume")
        .and_then(|node| node.value.as_number().ok());
    assert_eq!(at, Some(45.0));
}

#[test]
fn a_focused_input_keeps_every_key_from_the_game() {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    engine.window_focused = true;

    let mut keys = ButtonInput::<KeyCode>::default();
    keys.press(KeyCode::KeyW);

    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.insert_resource(keys);
    app.init_resource::<ButtonInput<MouseButton>>();
    app.init_resource::<Messages<MouseMotion>>();
    app.init_resource::<Messages<WindowFocused>>();
    app.init_resource::<crate::ui::UiManager>();
    app.init_resource::<crate::sound::SoundState>();
    app.init_resource::<bevy::input::touch::Touches>();
    app.insert_engine(engine);
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.init_resource::<Time<Real>>();
    app.add_systems(Update, publish_sensors);

    app.update();
    assert!(blockloom_core::sense::read(|sensors| sensors
        .keys
        .contains("w")));

    let mut manager = app.world_mut().resource_mut::<crate::ui::UiManager>();
    manager.show(element("name", blockloom_core::ui::UiKind::Input));
    manager.take_pending();
    manager.focus_on(Some("name"));
    app.update();
    assert!(blockloom_core::sense::read(|sensors| sensors
        .keys
        .is_empty()));
}
/// A running app that does nothing but publish the sensors each update,
/// over `count` rect actors named `A0`, `A1`, ... standing in a row.
fn sensing_app(count: usize) -> App {
    let (_sender, incoming) = std::sync::mpsc::channel();
    let mut engine = Engine::new(incoming, Mode::TwoD);
    engine.running = true;
    let ids: Vec<String> = (0..count).map(|i| format!("A{i}")).collect();
    let placed: Vec<(&str, blockloom_core::scene::Placement)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), at(i as f32, 0.0)))
        .collect();
    engine.project = project_of(&placed, &[]);
    engine.parents.clear();
    let mut app = App::new();
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Dimension(Mode::TwoD));
    app.init_resource::<ButtonInput<KeyCode>>();
    app.init_resource::<ButtonInput<MouseButton>>();
    app.init_resource::<Messages<MouseMotion>>();
    app.init_resource::<Messages<WindowFocused>>();
    app.init_resource::<crate::ui::UiManager>();
    app.init_resource::<crate::sound::SoundState>();
    app.init_resource::<bevy::input::touch::Touches>();
    for (i, id) in ids.iter().enumerate() {
        let entity = app
            .world_mut()
            .spawn((
                ActorId(id.clone()),
                Transform::from_xyz(i as f32, 0.0, 0.0),
                Visibility::Inherited,
            ))
            .id();
        engine.entities.insert(id.clone(), entity);
    }
    app.insert_engine(engine);
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.init_resource::<Time<Real>>();
    app.add_systems(Update, (publish_simulation_actors, publish_sensors).chain());
    app
}

#[test]
fn frame_sensing_retains_logical_time_and_advances_the_ui_clock() {
    let mut app = sensing_app(1);
    blockloom_core::sense::publish_run(2.0, 2.0, false, "Scene", &[]);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(3));
    app.world_mut()
        .resource_mut::<Time<Real>>()
        .advance_by(std::time::Duration::from_secs(4));
    app.update();
    blockloom_core::sense::read(|s| {
        assert_eq!(s.time, 2.0);
        assert_eq!(s.wall_time, 4.0);
    });
    app.world_mut().non_send_mut::<Engine>().running = false;
    app.update();
    assert_eq!(blockloom_core::sense::read(|s| s.time), 3.0);
}

#[test]
fn actor_sensing_uses_settled_poses_even_while_paused() {
    let mut app = sensing_app(1);
    let entity = app.world().non_send::<Engine>().entities["A0"];
    app.world_mut()
        .entity_mut(entity)
        .insert(PhysicsPose(Transform::from_xyz(12.0, 0.0, 0.0)));
    app.world_mut().non_send_mut::<Engine>().paused = true;
    app.update();
    assert_eq!(
        blockloom_core::sense::read(|s| s.actors["A0"].position),
        [12.0, 0.0, 0.0]
    );
    app.world_mut().non_send_mut::<Engine>().running = false;
    app.update();
    assert_eq!(
        blockloom_core::sense::read(|s| s.actors["A0"].position),
        [0.0, 0.0, 0.0],
        "stopped previews use the editable transform"
    );
}

#[test]
fn the_snapshot_is_refreshed_in_place_as_actors_change() {
    let mut app = sensing_app(3);
    app.update();
    let (position, name) = blockloom_core::sense::read(|sensors| {
        let a2 = &sensors.actors["A2"];
        (a2.position, a2.name.clone())
    });
    assert_eq!(position, [2.0, 0.0, 0.0]);
    assert_eq!(name, "A2");

    // A moves, gains a parent and a contact; B goes away; a new one comes.
    let (a1, a2) = {
        let engine = app.world().non_send::<Engine>();
        (engine.entities["A1"], engine.entities["A2"])
    };
    app.world_mut()
        .entity_mut(a2)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::new(10.0, 4.0, 0.0);
    app.world_mut().despawn(a1);
    {
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.entities.remove("A1");
        engine.parents.insert("A2".into(), "A0".into());
        engine
            .touching
            .insert("A2".into(), ["A0".to_string()].into_iter().collect());
        engine.last_created.insert("A2".into(), "~1".into());
    }
    app.update();
    blockloom_core::sense::read(|sensors| {
        assert!(!sensors.actors.contains_key("A1"), "A1 left the world");
        let a2 = &sensors.actors["A2"];
        assert_eq!(a2.position, [10.0, 4.0, 0.0]);
        assert_eq!(a2.parent, "A0");
        assert_eq!(
            a2.local_position,
            [10.0, 4.0, 0.0],
            "A0 stands at the origin"
        );
        assert!(a2.touching.contains("A0"));
        assert_eq!(a2.last_created, "~1");
        assert_eq!(sensors.actors["A0"].parent, "");
    });

    // Everything set above is gone again once the engine forgets it.
    {
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.parents.clear();
        engine.touching.clear();
        engine.last_created.clear();
    }
    app.update();
    blockloom_core::sense::read(|sensors| {
        let a2 = &sensors.actors["A2"];
        assert_eq!(a2.parent, "");
        assert!(a2.touching.is_empty());
        assert_eq!(a2.last_created, "");
        assert_eq!(
            sensors.find("a2").map(|a| a.position),
            Some([10.0, 4.0, 0.0])
        );
    });
}

/// `cargo test --release -p blockloom-runtime -- --ignored --nocapture
/// sensor_publish_cost`: what one frame's snapshot costs per actor.
#[test]
#[ignore = "timing, run by hand"]
fn sensor_publish_cost() {
    for count in [100, 1000, 4000] {
        let mut app = sensing_app(count);
        for _ in 0..3 {
            app.update();
        }
        let frames = 50;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            app.update();
        }
        let each = start.elapsed().as_secs_f64() / frames as f64;
        println!(
            "{count:>5} actors  {:>8.1} us/frame  {:>6.0} ns/actor",
            each * 1e6,
            each * 1e9 / count as f64
        );
    }
}
