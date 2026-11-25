//! Small shared helpers: who an effect is about, axes and rotations, key names.

use super::*;

pub(crate) fn effect_actor(effect: &Effect) -> Option<&String> {
    match effect {
        Effect::Move { actor, .. }
        | Effect::GoTo { actor, .. }
        | Effect::NavigateTo { actor, .. }
        | Effect::BurstParticles { actor, .. }
        | Effect::SetEmitterDial { actor, .. }
        | Effect::SetTrailEnabled { actor, .. }
        | Effect::SetEmitterPlaying { actor, .. }
        | Effect::SetLightShadows { actor, .. }
        | Effect::ChangePosition { actor, .. }
        | Effect::Glide { actor, .. }
        | Effect::TweenScale { actor, .. }
        | Effect::TweenRotation { actor, .. }
        | Effect::TweenColor { actor, .. }
        | Effect::StopTweens { actor, .. }
        | Effect::PlayAnimation { actor, .. }
        | Effect::StopAnimation { actor, .. }
        | Effect::SetAnimationSpeed { actor, .. }
        | Effect::FireAnimationTrigger { actor, .. }
        | Effect::SetRigSlot { actor, .. }
        | Effect::SetSlotTint { actor, .. }
        | Effect::SetIkTarget { actor, .. }
        | Effect::SetSpriteDial { actor, .. }
        | Effect::Turn { actor, .. }
        | Effect::SetRotation { actor, .. }
        | Effect::PointTowards { actor, .. }
        | Effect::SetScale { actor, .. }
        | Effect::SetLightIntensity { actor, .. }
        | Effect::SetEmissiveStrength { actor, .. }
        | Effect::SetBody { actor, .. }
        | Effect::ApplyImpulse { actor, .. }
        | Effect::SetVelocity { actor, .. }
        | Effect::AddForce { actor, .. }
        | Effect::SetDensity { actor, .. }
        | Effect::SetMass { actor, .. }
        | Effect::SetTrigger { actor, .. }
        | Effect::SetCollisionLayer { actor, .. }
        | Effect::SetCollisionMask { actor, .. }
        | Effect::Say { actor, .. }
        | Effect::SetVisible { actor, .. }
        | Effect::SetColor { actor, .. }
        | Effect::SetComponentField { actor, .. }
        |         Effect::SetCameraView { actor, .. }
        | Effect::SetCameraPitch { actor, .. }
        | Effect::SetCameraFov { actor, .. }
        | Effect::AttachComponent { actor, .. }
        | Effect::DetachComponent { actor, .. }
        | Effect::BindAction { actor, .. }
        | Effect::ClearActionBindings { actor, .. }
        | Effect::Error { actor, .. } => Some(actor),
        // Making, deleting and re-parenting an actor are `apply_lifetimes`'s
        // to carry out, and none of them is a change to a transform.
        Effect::SetGravity { .. }
        | Effect::SetRenderSetting { .. }
        | Effect::SetLook2d { .. }
        | Effect::SetExposure { .. }
        | Effect::SetHdrOutput { .. }
        | Effect::SetPeakBrightness { .. }
        | Effect::SetVolumeEnabled { .. }
        | Effect::SetVolumeWeight { .. }
        | Effect::CaptureProbes
        | Effect::SetShadowDistance { .. }
        | Effect::SetRayTracing { .. }
        | Effect::SetGiBounces { .. }
        | Effect::SetGiSamples { .. }
        | Effect::SetFogDensity { .. }
        | Effect::SetAurora { .. }
        | Effect::StrikeLightning { .. }
        | Effect::Fracture { .. }
        | Effect::Splash { .. }
        | Effect::PuffSmoke { .. }
        | Effect::SpawnDecal(_)
        | Effect::FadeDecals { .. }
        | Effect::SetLightningRate { .. }
        | Effect::SetWind { .. }
        | Effect::SetCloudDrift { .. }
        | Effect::SetClouds { .. }
        | Effect::SetTimeOfDay { .. }
        | Effect::AdvanceTime { .. }
        | Effect::SetPrecipitation { .. }
        | Effect::BlendWeather { .. }
        | Effect::PlayCutscene { .. }
        | Effect::SkipCutscene
        | Effect::CameraShake { .. }
        | Effect::SetTimeScale { .. }
        | Effect::Hitstop { .. }
        | Effect::SetLetterbox { .. }
        | Effect::FadeScreen { .. }
        | Effect::SetCloudLayer { .. }
        | Effect::SetWater { .. }
        // The level's, applied by `tiles`.
        | Effect::PaintTile { .. }
        | Effect::SetParallax { .. }
        | Effect::SetBusVolume { .. }
        | Effect::SetReverb { .. }
        | Effect::SetUnderwaterMuffle { .. }
        | Effect::RumbleGamepad { .. }
        | Effect::Stopped
        // Sound is the sound module's to play, like UI is the overlay's.
        | Effect::PlaySound { .. }
        | Effect::StopSound { .. }
        | Effect::SetSoundVolume { .. }
        | Effect::SetSoundPitch { .. }
        | Effect::SetMouseLocked { .. }
        // Screen-space, so against no actor at all - `overlay` applies them.
        | Effect::ShowElement { .. }
        | Effect::HideElement { .. }
        | Effect::DeleteElement { .. }
        | Effect::SetScreenOpen { .. }
        | Effect::SetUiProp { .. }
        | Effect::SetFocus { .. }
        | Effect::SetUiTheme { .. }
        | Effect::SetPaused { .. }
        | Effect::SaveVariable { .. }
        | Effect::PluginCall { .. }
        | Effect::Verb { .. }
        | Effect::PhysicsQuery { .. }
        | Effect::Controller { .. }
        | Effect::SetParent { .. }
        | Effect::CreateClone { .. }
        | Effect::CreateActor { .. }
        | Effect::DeleteActor { .. }
        | Effect::SwitchScene { .. }
        // Window-global: the actor is only who to blame, so no transform
        // applies them here.
        | Effect::SwitchSaveSlot { .. }
        | Effect::DeleteSaveSlot { .. }
        | Effect::SetLanguage { .. } => None,
    }
}

/// A block's `z` is ignored in a 2D project, so the actor keeps its own.
pub(crate) fn vec3_in(mode: Mode, value: [f32; 3], current: Vec3) -> Vec3 {
    match mode {
        Mode::TwoD => Vec3::new(value[0], value[1], current.z),
        Mode::ThreeD => Vec3::new(value[0], value[1], value[2]),
    }
}

/// Which way an actor faces, as a unit vector. In 2D a rotation of zero faces
/// right, so turning anticlockwise raises the angle.
pub fn forward_of(transform: &Transform, mode: Mode) -> Vec3 {
    match mode {
        Mode::TwoD => {
            let angle = transform.rotation.to_euler(EulerRot::XYZ).2;
            Vec3::new(angle.cos(), angle.sin(), 0.0)
        }
        Mode::ThreeD => *transform.forward(),
    }
}

/// True when the physics engine owns this actor's movement.
pub fn is_dynamic(engine: &Engine, actor: &str) -> bool {
    engine.actor(actor).is_some_and(|actor| {
        actor.physics().body == BodyKind::Dynamic
            || actor.components.rigidbody().is_some_and(|body| {
                body.simulated && body.body_type == blockloom_core::physics::BodyType::Dynamic
            })
    })
}

pub fn is_character(engine: &Engine, actor: &str) -> bool {
    engine.actor(actor).is_some_and(|actor| {
        let physics = actor.physics();
        physics.body == BodyKind::Kinematic && physics.character_controller
    })
}

/// Which coordinate an axis names. A 2D project has no depth to change, so Z
/// is ignored there rather than moving a sprite out of view.
pub fn position_axis(mode: Mode, axis: Axis) -> Option<usize> {
    match (mode, axis) {
        (Mode::TwoD, Axis::Z) => None,
        _ => Some(axis.index()),
    }
}

/// Which rotation an axis names. In 2D only Z is a rotation at all; X and Y
/// would tip a sprite out of the plane.
pub(crate) fn rotation_axis(mode: Mode, axis: Axis) -> Option<usize> {
    match (mode, axis) {
        (Mode::TwoD, Axis::Z) => Some(2),
        (Mode::TwoD, _) => None,
        (Mode::ThreeD, _) => Some(axis.index()),
    }
}

pub(crate) fn euler_of(transform: &Transform) -> [f32; 3] {
    let (x, y, z) = transform.rotation.to_euler(EulerRot::XYZ);
    [x, y, z]
}

pub(crate) fn quat_of(euler: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::XYZ, euler[0], euler[1], euler[2])
}

/// A 2D turn raises the one rotation a sprite has.
pub fn turn_2d(transform: &mut Transform, radians: f32) {
    let mut euler = euler_of(transform);
    euler[2] += radians;
    transform.rotation = quat_of(euler);
}

/// A 3D turn spins about the actor's own axis, composed straight onto the
/// quaternion. Reading the yaw back out of XYZ euler instead folds it into
/// [-90, 90] past the first quarter turn, which walled every 3D game's
/// horizontal look at about 180 degrees of freedom.
pub fn turn_3d(transform: &mut Transform, axis: Axis, radians: f32) {
    let spin = Quat::from_axis_angle(axis_vector(axis), radians);
    transform.rotation = (transform.rotation * spin).normalize();
}

/// A 3D absolute rotation sets one component through YXZ euler, which spells
/// yaw first over the full circle. The XYZ spelling folds a yaw past 90
/// degrees into flipped pitch and roll, so zeroing X or Z through it snaps
/// the yaw back inside - the same 180-degree wall from the other side.
pub(crate) fn set_rotation_3d(transform: &mut Transform, axis: Axis, radians: f32) {
    let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let mut angles = [pitch, yaw, roll];
    angles[axis.index()] = radians;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, angles[1], angles[0], angles[2]);
}

/// One axis of a transform in degrees, as the tween blocks speak it. 2D only
/// turns about Z; the other axes read as zero rather than an error.
pub(crate) fn rotation_of(transform: &Transform, mode: Mode, axis: Axis) -> f32 {
    if mode == Mode::ThreeD {
        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
        let angles = [pitch, yaw, roll];
        angles[axis.index()].to_degrees()
    } else {
        match rotation_axis(mode, axis) {
            Some(index) => euler_of(transform)[index].to_degrees(),
            None => 0.0,
        }
    }
}

/// Writes one axis of a transform in degrees, the inverse of `rotation_of`.
pub(crate) fn set_rotation_of(transform: &mut Transform, mode: Mode, axis: Axis, degrees: f32) {
    if mode == Mode::ThreeD {
        set_rotation_3d(transform, axis, degrees.to_radians());
    } else if let Some(index) = rotation_axis(mode, axis) {
        let mut euler = euler_of(transform);
        euler[index] = degrees.to_radians();
        transform.rotation = quat_of(euler);
    }
}

pub(crate) fn axis_vector(axis: Axis) -> Vec3 {
    match axis {
        Axis::X => Vec3::X,
        Axis::Y => Vec3::Y,
        Axis::Z => Vec3::Z,
    }
}

/// Which linear axes a released walk brakes: every axis but the one gravity
/// pulls along, so a released key stops the run without hanging a fall - and
/// a gravity-free game stops dead on all of them.
pub fn stop_axes(gravity: Vec3) -> [bool; 3] {
    if gravity.length_squared() < 1e-6 {
        return [true, true, true];
    }
    let pull = [gravity.x.abs(), gravity.y.abs(), gravity.z.abs()];
    let mainly = if pull[0] >= pull[1] && pull[0] >= pull[2] {
        0
    } else if pull[1] >= pull[2] {
        1
    } else {
        2
    };
    [mainly != 0, mainly != 1, mainly != 2]
}

/// Where a `point towards` target is: another actor, or the mouse.
pub(crate) fn target_position(
    engine: &Engine,
    positions: &HashMap<String, Vec3>,
    target: &str,
    here: Vec3,
) -> Option<Vec3> {
    if target.eq_ignore_ascii_case("mouse") {
        return blockloom_core::sense::read(|sensors| {
            Some(Vec3::new(sensors.mouse[0], sensors.mouse[1], here.z))
        });
    }
    let id = resolve_actor(engine, target, target)?;
    positions.get(&id).copied()
}

/// Our name for a key, matching what the blocks are written with.
pub(crate) fn key_name(code: &KeyCode) -> Option<String> {
    let name = match code {
        KeyCode::Space => "space",
        KeyCode::ArrowUp => "up arrow",
        KeyCode::ArrowDown => "down arrow",
        KeyCode::ArrowLeft => "left arrow",
        KeyCode::ArrowRight => "right arrow",
        KeyCode::Enter | KeyCode::NumpadEnter => "enter",
        KeyCode::Escape => "escape",
        KeyCode::ShiftLeft | KeyCode::ShiftRight => "shift",
        KeyCode::ControlLeft | KeyCode::ControlRight => "control",
        KeyCode::AltLeft | KeyCode::AltRight => "alt",
        KeyCode::Tab => "tab",
        KeyCode::Backspace => "backspace",
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        KeyCode::Digit0 | KeyCode::Numpad0 => "0",
        KeyCode::Digit1 | KeyCode::Numpad1 => "1",
        KeyCode::Digit2 | KeyCode::Numpad2 => "2",
        KeyCode::Digit3 | KeyCode::Numpad3 => "3",
        KeyCode::Digit4 | KeyCode::Numpad4 => "4",
        KeyCode::Digit5 | KeyCode::Numpad5 => "5",
        KeyCode::Digit6 | KeyCode::Numpad6 => "6",
        KeyCode::Digit7 | KeyCode::Numpad7 => "7",
        KeyCode::Digit8 | KeyCode::Numpad8 => "8",
        KeyCode::Digit9 | KeyCode::Numpad9 => "9",
        // Punctuation answers to what it prints on a US layout.
        KeyCode::Minus | KeyCode::NumpadSubtract => "-",
        KeyCode::Equal => "=",
        KeyCode::NumpadAdd => "+",
        KeyCode::NumpadMultiply => "*",
        KeyCode::Slash | KeyCode::NumpadDivide => "/",
        KeyCode::Period | KeyCode::NumpadDecimal => ".",
        KeyCode::Comma => ",",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Backquote => "`",
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::Backslash => "\\",
        _ => return None,
    };
    Some(normalize_key(name))
}
