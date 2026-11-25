//! Publishing the sensor snapshot the reporter blocks and scripts read.

use super::*;

/// Samples authoritative actor state before the fixed-step schedulers.
pub fn publish_simulation_actors(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    actors: Query<(
        &ActorId,
        &Transform,
        &Visibility,
        Option<&CustomComponents>,
        Option<&Gliding>,
        Option<&TweeningScale>,
        Option<&TweeningRotation>,
        Option<&TweeningColor>,
        Option<&AnimationPlayer>,
        Option<&PhysicsPose>,
        Option<&bevy_rapier2d::prelude::Velocity>,
        Option<&bevy_rapier3d::prelude::Velocity>,
        Option<&bevy_rapier2d::prelude::ReadMassProperties>,
        Option<&bevy_rapier3d::prelude::ReadMassProperties>,
    )>,
    (particles, lights): (
        Option<Res<crate::vfx::ParticleSenses>>,
        Option<Res<crate::lights::LightsRun>>,
    ),
) {
    if engine.running {
        let none = crate::lights::LightsRun::default();
        let lights = lights.as_deref().unwrap_or(&none);
        let actors =
            sample_actor_senses(&engine, dimension.0, &actors, particles.as_deref(), lights);
        blockloom_core::sense::publish_actors(actors);
    }
}

pub(crate) fn sample_actor_senses(
    engine: &Engine,
    mode: Mode,
    actors: &Query<(
        &ActorId,
        &Transform,
        &Visibility,
        Option<&CustomComponents>,
        Option<&Gliding>,
        Option<&TweeningScale>,
        Option<&TweeningRotation>,
        Option<&TweeningColor>,
        Option<&AnimationPlayer>,
        Option<&PhysicsPose>,
        Option<&bevy_rapier2d::prelude::Velocity>,
        Option<&bevy_rapier3d::prelude::Velocity>,
        Option<&bevy_rapier2d::prelude::ReadMassProperties>,
        Option<&bevy_rapier3d::prelude::ReadMassProperties>,
    )>,
    particles: Option<&crate::vfx::ParticleSenses>,
    lights: &crate::lights::LightsRun,
) -> HashMap<String, ActorSense> {
    // Retained entries are refreshed in place: most of an actor's strings
    // and sets are unchanged, so `clone_from` keeps their allocations.
    let mut senses = blockloom_core::sense::take_actors();
    let mut seen = 0usize;
    for (
        id,
        transform,
        visibility,
        custom,
        glide,
        scale,
        rotation,
        color,
        player,
        pose,
        velocity_2d,
        velocity_3d,
        mass_2d,
        mass_3d,
    ) in actors
    {
        let transform = if engine.running {
            pose.map(|pose| &pose.0).unwrap_or(transform)
        } else {
            transform
        };
        seen += 1;
        // The parent's world transform inverted onto this actor's own: the
        // world position itself when it hangs off nothing, or its parent is
        // gone. The inverse of `world_of`, which places an offset.
        let parent = engine.parents.get(&id.0);
        let local_position = parent
            .and_then(|parent| engine.entities.get(parent))
            .and_then(|&entity| actors.get(entity).ok())
            .map(|(_, parent, _, _, _, _, _, _, _, pose, ..)| {
                let parent = if engine.running {
                    pose.map(|pose| &pose.0).unwrap_or(parent)
                } else {
                    parent
                };
                local_of(parent, transform.translation)
            })
            .unwrap_or(transform.translation.to_array());
        let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
        let (layer, mask, trigger) = engine.filter_of(&id.0);
        let has_body = engine.has_component(&id.0, "Body");
        let shape = collider_shape(engine, &id.0, mode, transform);
        if !senses.contains_key(id.0.as_str()) {
            senses.insert(id.0.clone(), ActorSense::default());
        }
        let Some(sense) = senses.get_mut(id.0.as_str()) else {
            continue;
        };
        match engine.actor(&id.0) {
            Some(actor) => sense.name.clone_from(&actor.name),
            None => sense.name.clear(),
        }
        sense.position = transform.translation.to_array();
        sense.local_position = local_position;
        sense.rotation = [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()];
        sense.scale = size_of(transform, engine.stretch_of(&id.0));
        sense.visible = *visibility != Visibility::Hidden;
        match parent {
            Some(parent) => sense.parent.clone_from(parent),
            None => sense.parent.clear(),
        }
        sense.is_clone = engine.clones.contains_key(&id.0);
        sense.tweening =
            glide.is_some() || scale.is_some() || rotation.is_some() || color.is_some();
        match player {
            Some(player) => {
                sense.anim_clip.clone_from(&player.clip);
                sense.anim_frame = player.frame;
                sense.anim_playing = player.playing;
            }
            None => {
                sense.anim_clip.clear();
                sense.anim_frame = 0;
                sense.anim_playing = false;
            }
        }
        match engine.last_created.get(&id.0) {
            Some(made) => sense.last_created.clone_from(made),
            None => sense.last_created.clear(),
        }
        match engine.touching.get(&id.0) {
            Some(touching) => sense.touching.clone_from(touching),
            None => sense.touching.clear(),
        }
        match engine.attached.get(&id.0) {
            Some(attached) => sense.attached.clone_from(attached),
            None => sense.attached.clear(),
        }
        match custom {
            Some(custom) => sense.components.clone_from(&custom.0),
            None => sense.components.clear(),
        }
        sense.has_body = has_body;
        sense.trigger = trigger;
        sense.casts_shadows = crate::lights::casts_shadows(engine, lights, &id.0);
        sense.layer = layer;
        sense.mask = mask;
        sense.shape = shape;
        // Rapier's own motion and mass, sampled with the rest of the fixed
        // tick so blocks, compiled logic and scripts agree. A 2D body turns
        // about z only; anything with no body reads zero.
        let (linear, angular) = match (velocity_2d, velocity_3d) {
            (Some(body), _) => (
                [body.linear.x, body.linear.y, 0.0],
                [0.0, 0.0, body.angular],
            ),
            (_, Some(body)) => (body.linear.to_array(), body.angular.to_array()),
            _ => ([0.0; 3], [0.0; 3]),
        };
        sense.velocity = linear;
        sense.angular_velocity = angular;
        sense.mass = mass_2d
            .map(|mass| mass.get().mass)
            .or_else(|| mass_3d.map(|mass| mass.get().mass))
            .unwrap_or(0.0);
        sense.grounded = engine.contacts.grounded(&id.0);
        sense.particles = particles
            .and_then(|particles| particles.0.get(&id.0).copied())
            .unwrap_or_default();
        let spec = engine.spec_of(&id.0);
        if sense.spec.as_ref().map(std::sync::Arc::as_ptr)
            != spec.as_ref().map(std::sync::Arc::as_ptr)
        {
            sense.spec = spec;
        }
    }
    // Actors that left the world since the last sample.
    if senses.len() != seen {
        let live: HashSet<&str> = actors.iter().map(|(id, ..)| id.0.as_str()).collect();
        senses.retain(|id, _| live.contains(id.as_str()));
    }

    senses
}

/// Publishes the snapshot reporter blocks read, and starts `when key pressed`
/// strands for keys that went down this frame.
///
/// Runs after the frame's input has landed (`type_into_focused_input`,
/// `detect_clicks`), so the snapshot reflects the click that fired a strand
/// rather than the frame before it - a one-shot `when input changed` strand
/// would otherwise read the value the click just moved away from.
pub fn publish_sensors(
    mut engine: NonSendMut<Engine>,
    manager: Res<crate::ui::UiManager>,
    time: (Res<Time>, Res<Time<Real>>),
    dimension: Res<Dimension>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    touches: Res<Touches>,
    mut motion: MessageReader<MouseMotion>,
    mut focus: MessageReader<WindowFocused>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    actors: Query<(
        &ActorId,
        &Transform,
        &Visibility,
        Option<&CustomComponents>,
        Option<&Gliding>,
        Option<&TweeningScale>,
        Option<&TweeningRotation>,
        Option<&TweeningColor>,
        Option<&AnimationPlayer>,
        Option<&PhysicsPose>,
        Option<&bevy_rapier2d::prelude::Velocity>,
        Option<&bevy_rapier3d::prelude::Velocity>,
        Option<&bevy_rapier2d::prelude::ReadMassProperties>,
        Option<&bevy_rapier3d::prelude::ReadMassProperties>,
    )>,
    (sound, cine, input_run): (
        Res<crate::sound::SoundState>,
        Option<Res<crate::cinematic::CinePlayer>>,
        Option<Res<InputRun>>,
    ),
    (atmosphere, water, particles, scaling, lights): (
        Option<Res<crate::atmosphere::Atmosphere>>,
        Option<Res<crate::water::WaterSample>>,
        Option<Res<crate::vfx::ParticleSenses>>,
        Option<Res<crate::quality::Scaling>>,
        Option<Res<crate::lights::LightsRun>>,
    ),
    preview_pointer: Option<ResMut<crate::preview::PreviewPointer>>,
) {
    let (time, real) = time;
    let now = time.elapsed_secs_f64();
    let held: HashSet<String> = keys.get_pressed().filter_map(key_name).collect();
    // OS-confirmed focus, not the component: `Window::focused` defaults to
    // true and winit only reports changes, so a game opened behind the
    // editor would otherwise read focused until the heat death of the run.
    for event in focus.read() {
        engine.window_focused = event.focused;
    }
    // A live preview pointer counts as attention: the OS window sits behind
    // the editor while the viewport is used, so it would otherwise never
    // report focus.
    let preview_live = preview_pointer
        .as_ref()
        .is_some_and(|pointer| crate::preview::pointer_live(pointer));
    let focused = engine.window_focused || preview_live;
    let mouse = preview_pointer
        .as_ref()
        .and_then(|pointer| {
            let pos = pointer.pos.filter(|_| preview_live)?;
            screen_to_world(dimension.0, pos, &cameras)
        })
        .or_else(|| mouse_world_position(dimension.0, &windows, &cameras))
        .unwrap_or_default();
    // The pointer travels a few pixels a frame, not a teleport: sum the
    // motion events into one delta so a reporter reads what moved since last
    // frame, whichever half of the window it crossed.
    let mut mouse_delta = [0.0f32; 2];
    for moved in motion.read() {
        mouse_delta[0] += moved.delta.x;
        mouse_delta[1] += moved.delta.y;
    }
    if let Some(mut pointer) = preview_pointer {
        mouse_delta[0] += pointer.delta.x;
        mouse_delta[1] += pointer.delta.y;
        pointer.delta = Vec2::ZERO;
    }
    // Motion events are raw device input: they arrive whichever window holds
    // the cursor, including the editor beside this one. A game that answered
    // those would turn under a cursor it can't see, so only a focused window
    // feeds its game. The drain above still runs, so nothing stale bursts
    // out the moment focus lands.
    if !focused {
        mouse_delta = [0.0; 2];
    }

    let senses = if engine.running {
        blockloom_core::sense::take_actors()
    } else {
        let none = crate::lights::LightsRun::default();
        let lights = lights.as_deref().unwrap_or(&none);
        sample_actor_senses(&engine, dimension.0, &actors, particles.as_deref(), lights)
    };

    // Wanted and focused reads as held: the component alone would still say
    // Locked after an unfocused request the backend silently dropped, and
    // reading the window back can't see that drop either.
    let mouse_locked = engine.wants_cursor_locked && focused;

    // A focused text input owns the keyboard: while one does, game strands
    // see no keys at all, so typing a name never also fires the gun.
    let typing = manager.focus().is_some();
    let keys_live = if typing { HashSet::new() } else { held };

    // Gamepads: every connected pad feeds one shared state, strongest wins.
    let mut pad_buttons: HashSet<String> = HashSet::new();
    let mut pad_axes: HashMap<String, f32> = HashMap::new();
    let mut pad_count = 0;
    // Each pad on its own too, for actions that belong to one local player.
    let mut each_pad: Vec<(HashSet<String>, HashMap<String, f32>)> = Vec::new();
    for pad in &pads {
        pad_count += 1;
        let mut own_buttons: HashSet<String> = HashSet::new();
        let mut own_axes: HashMap<String, f32> = HashMap::new();
        for button in GamepadButton::all() {
            if pad.pressed(button) {
                pad_buttons.insert(pad_button_name(button));
                own_buttons.insert(pad_button_name(button));
            }
        }
        for axis in GamepadAxis::all() {
            if let Some(value) = pad.get(axis) {
                let name = pad_axis_name(axis);
                own_axes.insert(name.clone(), value);
                let kept = pad_axes.get(&name).copied().unwrap_or(0.0);
                if value.abs() > kept.abs() {
                    pad_axes.insert(name, value);
                }
            }
        }
        each_pad.push((own_buttons, own_axes));
    }
    // Pad buttons and sticks keep working while typing: only the keyboard
    // belongs to the text input, so a gamepad pause button still pauses.
    let mut mouse_buttons: HashSet<String> = HashSet::new();
    if focused && buttons.pressed(MouseButton::Left) {
        mouse_buttons.insert("left".to_string());
    }
    if focused && buttons.pressed(MouseButton::Right) {
        mouse_buttons.insert("right".to_string());
    }
    if focused && buttons.pressed(MouseButton::Middle) {
        mouse_buttons.insert("middle".to_string());
    }
    let live = LiveInput {
        keys: keys_live.clone(),
        mouse: mouse_buttons.clone(),
        pad_buttons: pad_buttons.clone(),
        axes: pad_axes.clone(),
        // Up is positive, as a stick's is.
        mouse_delta: [mouse_delta[0], -mouse_delta[1]],
    };

    // Named actions, with run-scoped remaps winning over the document.
    // Pressed/released are edges against last frame's held.
    let mut actions: HashMap<String, ActionSense> = HashMap::new();
    let mut fired_actions: Vec<String> = Vec::new();
    let action_defs = engine.project.settings.input.actions.clone();
    for action in &action_defs {
        let bindings = input_run
            .as_ref()
            .and_then(|run| run.0.get(&action.name.to_lowercase()))
            .cloned()
            .unwrap_or_else(|| action.bindings.clone());
        let scoped = blockloom_core::input::InputAction {
            bindings,
            ..action.clone()
        };
        // A later player reads only their own pad; an action in a map that
        // is off reads as released.
        let own = (action.player > 0).then(|| {
            let (buttons, axes) = each_pad
                .get(usize::from(action.player) - 1)
                .cloned()
                .unwrap_or_default();
            LiveInput {
                pad_buttons: buttons,
                axes,
                ..Default::default()
            }
        });
        let source = own.as_ref().unwrap_or(&live);
        let on = blockloom_core::input::map_enabled(&action.map);
        let held_now = on && source.action_held(&scoped);
        let value = if on {
            source.action_value(&scoped)
        } else {
            0.0
        };
        let vector = if on {
            source.action_vector(&scoped)
        } else {
            [0.0; 2]
        };
        let was = engine
            .prev_action_held
            .get(&action.name.to_lowercase())
            .copied()
            .unwrap_or(false);
        let pressed = held_now && !was;
        let released = !held_now && was;
        engine
            .prev_action_held
            .insert(action.name.to_lowercase(), held_now);
        actions.insert(
            action.name.clone(),
            ActionSense {
                held: held_now,
                pressed,
                released,
                value,
                vector,
            },
        );
        if pressed {
            fired_actions.push(action.name.to_lowercase());
        }
    }
    // Remapped-away actions leave no stale edge behind.
    let valid_actions: HashSet<String> = engine
        .project
        .settings
        .input
        .actions
        .iter()
        .map(|action| action.name.to_lowercase())
        .collect();
    engine
        .prev_action_held
        .retain(|name, _| valid_actions.contains(name));

    // Touches in world units, press order. Unfocused windows read none, the
    // same gate the pointer delta keeps.
    let mut touch_points: Vec<TouchSense> = Vec::new();
    let mut touch_started = false;
    // The world camera's live pose, so aim blocks can read the view itself
    // instead of tracking one by hand. First camera wins, the way the
    // motor's camera-relative steering reads it.
    let camera = cameras.iter().next().map(|(_, pose)| {
        let view = pose.compute_transform();
        let at = view.translation.to_array();
        let ahead = view.forward().to_array();
        blockloom_core::sense::CameraSense {
            position: at,
            forward: ahead,
        }
    });
    if focused {
        for touch in touches.iter() {
            if let Some(point) = screen_to_world(dimension.0, touch.position(), &cameras) {
                touch_points.push(TouchSense {
                    id: touch.id(),
                    position: point,
                });
            }
        }
        touch_started = touches.iter_just_pressed().next().is_some();
    }

    blockloom_core::sense::publish(Sensors {
        time: if engine.running {
            blockloom_core::sense::read(|s| s.time)
        } else {
            engine.run_time(now)
        },
        // Never frozen: what a strand the interface started reads, so a
        // clock on a pause menu keeps ticking.
        wall_time: engine.wall_time(real.elapsed_secs_f64()),
        paused: engine.paused,
        keys: keys_live,
        mouse,
        mouse_delta,
        mouse_locked,
        mouse_down: focused && buttons.pressed(MouseButton::Left),
        mouse_buttons,
        actions,
        touches: touch_points,
        gamepad_connected: pad_count > 0,
        gamepad_axes: pad_axes,
        gamepad_buttons: pad_buttons,
        actors: senses,
        camera: camera.unwrap_or_default(),
        ui: manager.senses(),
        ui_focus: manager.focus().unwrap_or_default().to_string(),
        screens: manager.screen_states(),
        sounds: sound.playing(),
        bus_volumes: sound.bus_volumes(),
        // The last fixed tick's, not a fresh one: a frame between ticks
        // reads what the schedulers read.
        performance: scaling.map(|s| s.sample()).unwrap_or_default(),
        atmosphere: atmosphere.map(|air| air.0.clone()).unwrap_or_default(),
        water: water.map(|water| water.0.clone()).unwrap_or_default(),
        // `tiles::publish_level` fills it straight after.
        level: Default::default(),
        // `light2d::sync_lighting` fills it straight after.
        light2d: Default::default(),
        // `camera2d::apply_camera` fills it straight after.
        camera2d: Default::default(),
        current_scene: engine.project.active_scene().name.clone(),
        scene_names: engine
            .project
            .scenes
            .iter()
            .map(|s| s.name.clone())
            .collect(),
        // The last fixed tick's saves and language, not fresh ones: a frame
        // between ticks reads what the schedulers read.
        current_save_slot: blockloom_core::sense::read(|s| s.current_save_slot.clone()),
        save_slots: blockloom_core::sense::read(|s| s.save_slots.clone()),
        language: blockloom_core::sense::read(|s| s.language.clone()),
        cutscene_name: cine.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
        cutscene_time: cine.as_ref().map_or(0.0, |c| c.time),
        names: Default::default(),
    });

    // No world event queues while paused, so resuming never bursts.
    // Escape is the exception: a pause menu toggles on it, and the schedulers
    // run that strand as an interface one while the world stands still.
    if engine.running && !typing {
        for key in keys.get_just_pressed().filter_map(key_name) {
            if engine.paused && key != "escape" {
                continue;
            }
            engine.fire(Event::Key(key));
        }
        if !engine.paused {
            for action in fired_actions {
                engine.fire(Event::Action(action));
            }
            if touch_started {
                engine.fire(Event::Touched);
            }
        }
    }
}

/// Canonical sensor spelling of a pad button.
pub(crate) fn pad_button_name(button: GamepadButton) -> String {
    normalize_pad_button(match button {
        GamepadButton::South => "south",
        GamepadButton::East => "east",
        GamepadButton::North => "north",
        GamepadButton::West => "west",
        GamepadButton::C => "c",
        GamepadButton::Z => "z",
        GamepadButton::LeftTrigger => "lefttrigger",
        GamepadButton::LeftTrigger2 => "lefttrigger2",
        GamepadButton::RightTrigger => "righttrigger",
        GamepadButton::RightTrigger2 => "righttrigger2",
        GamepadButton::Select => "select",
        GamepadButton::Start => "start",
        GamepadButton::Mode => "mode",
        GamepadButton::LeftThumb => "leftthumb",
        GamepadButton::RightThumb => "rightthumb",
        GamepadButton::DPadUp => "dpadup",
        GamepadButton::DPadDown => "dpaddown",
        GamepadButton::DPadLeft => "dpadleft",
        GamepadButton::DPadRight => "dpadright",
        GamepadButton::Other(n) => return format!("other{n}"),
    })
}

/// Canonical sensor spelling of a pad axis.
pub(crate) fn pad_axis_name(axis: GamepadAxis) -> String {
    normalize_pad_axis(match axis {
        GamepadAxis::LeftStickX => "leftstickx",
        GamepadAxis::LeftStickY => "leftsticky",
        GamepadAxis::LeftZ => "leftz",
        GamepadAxis::RightStickX => "rightstickx",
        GamepadAxis::RightStickY => "rightsticky",
        GamepadAxis::RightZ => "rightz",
        GamepadAxis::Other(n) => return format!("other{n}"),
    })
}

/// Where the pointer is in world units: straight out in 2D, and where it meets
/// the ground plane in 3D (so `mouse x`/`mouse y` name a place an actor can
/// actually stand).
pub(crate) fn mouse_world_position(
    mode: Mode,
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<[f32; 2]> {
    let window = windows.iter().next()?;
    let cursor = window.cursor_position()?;
    screen_to_world(mode, cursor, cameras)
}

/// The same projection for a touch point: a finger names the same place the
/// pointer would at those window coordinates.
pub(crate) fn screen_to_world(
    mode: Mode,
    screen: Vec2,
    cameras: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<[f32; 2]> {
    let (camera, camera_transform) = cameras.iter().next()?;
    match mode {
        Mode::TwoD => {
            let point = camera.viewport_to_world_2d(camera_transform, screen).ok()?;
            Some([point.x, point.y])
        }
        Mode::ThreeD => {
            let ray = camera.viewport_to_world(camera_transform, screen).ok()?;
            let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
            let point = ray.get_point(distance);
            Some([point.x, point.z])
        }
    }
}
