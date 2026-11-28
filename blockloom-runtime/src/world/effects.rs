//! Effects shared by both dimensions, glides, tweens, the camera and pose smoothing.

use super::*;

/// The dimension-agnostic effects: anything that's a transform, a scale or a
/// visibility change.
pub fn apply_common(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    navmesh: Option<Res<NavMesh>>,
    mut exit: MessageWriter<AppExit>,
    mut manager: ResMut<crate::ui::UiManager>,
    mut transforms: Query<(
        &mut Transform,
        &mut Visibility,
        Option<&mut GlobalTransform>,
    )>,
    mut controllers_2d: Query<&mut bevy_rapier2d::prelude::KinematicCharacterController>,
    mut controllers_3d: Query<&mut bevy_rapier3d::prelude::KinematicCharacterController>,
    mut velocities_2d: Query<&mut bevy_rapier2d::prelude::Velocity>,
    mut velocities_3d: Query<&mut bevy_rapier3d::prelude::Velocity>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let positions: HashMap<String, Vec3> = engine
        .entities
        .iter()
        .filter_map(|(id, entity)| {
            transforms
                .get(*entity)
                .ok()
                .map(|(transform, _, _)| (id.clone(), transform.translation))
        })
        .collect();

    for effect in &effects.0 {
        let Some(actor) = effect_actor(effect) else {
            if let Effect::Stopped = effect {
                bridge::send(&RuntimeMessage::Stopped);
                if bridge::attached() {
                    // The editor's Stop button does the same: the Game view
                    // goes back to the document, not where the run ended.
                    end_run(&mut engine, &mut manager);
                    break;
                }
                engine.running = false;
                engine.speech.clear();
                // Nothing can press Play again in a built game, so a stopped
                // world is a finished one: `stop all` is how a game quits.
                exit.write(AppExit::Success);
            }
            continue;
        };
        let Some(entity) = engine.entities.get(actor).copied() else {
            continue;
        };
        let Ok((mut transform, mut visibility, mut global)) = transforms.get_mut(entity) else {
            continue;
        };
        match effect {
            Effect::Move { steps, .. } => {
                // A dynamic body is moved through its velocity instead, by the
                // dimension's own system - teleporting one every frame stops
                // the solver ever resolving a contact, and it walks through
                // walls and floors. See `dim2::apply_effects`.
                if is_dynamic(&engine, actor) || is_character(&engine, actor) {
                    continue;
                }
                let forward = forward_of(&transform, dimension.0);
                transform.translation += forward * *steps;
            }
            Effect::GoTo { position, .. } => {
                let before = *transform;
                transform.translation = vec3_in(dimension.0, *position, transform.translation);
                // Controller roots have child colliders; physics reads their global pose.
                if engine
                    .actor(actor)
                    .is_some_and(|a| a.components.character_controller().is_some())
                    && let Some(global) = global.as_deref_mut()
                {
                    *global = GlobalTransform::from(
                        global.affine()
                            * before.compute_affine().inverse()
                            * transform.compute_affine(),
                    );
                }
            }
            Effect::NavigateTo { target, speed, .. } => {
                // One step along the baked mesh at `speed` units per second.
                // No route, or no mesh at all, steps straight at the target,
                // so the actor never idles where `go to` would have moved.
                let mode = dimension.0;
                let from3 = transform.translation;
                let from = nav::plane_coords(mode, [from3.x, from3.y, from3.z]);
                let to = nav::plane_coords(mode, *target);
                let rate = engine.project.world.fixed_rate.clamp(1.0, 1000.0);
                let max_step = speed.max(0.0) / rate;
                let nav = navmesh.as_deref();
                let layer = engine
                    .actor(actor)
                    .and_then(|a| a.components.brain())
                    .map(|brain| brain.layer)
                    .unwrap_or(1);
                let route = nav.and_then(|nav| {
                    nav.mesh.as_ref().and_then(|mesh| {
                        nav::find_route(mesh, from, to, &nav.settings, layer)
                            .map(|path| (path, &nav.settings))
                    })
                });
                let (mut next, linked) = match route {
                    Some((path, settings)) => {
                        nav::next_route_step(&path, from, max_step, settings, layer)
                    }
                    None => (nav::next_step(&[to], from, max_step), false),
                };
                if !linked
                    && let Some(radius) = engine
                        .actor(actor)
                        .and_then(|a| a.components.brain())
                        .map(|brain| brain.separation.max(0.0))
                        .filter(|radius| *radius > 0.0)
                {
                    let mut away = Vec2::ZERO;
                    for (other, position) in &positions {
                        if other == actor || !engine.has_component(other, "Brain") {
                            continue;
                        }
                        let point = nav::plane_coords(mode, position.to_array());
                        if (point[0] - to[0]).hypot(point[1] - to[1]) < radius * 0.5 {
                            continue;
                        }
                        let offset = Vec2::new(from[0] - point[0], from[1] - point[1]);
                        let distance = offset.length();
                        if distance > 1e-4 && distance < radius {
                            away += offset / distance * (1.0 - distance / radius);
                        }
                    }
                    let desired = Vec2::new(next[0] - from[0], next[1] - from[1]);
                    let step = (desired + away * max_step).clamp_length_max(max_step);
                    next = [from[0] + step.x, from[1] + step.y];
                }
                let destination = match mode {
                    Mode::TwoD => Vec3::new(next[0], next[1], from3.z),
                    Mode::ThreeD => Vec3::new(next[0], from3.y, next[1]),
                };
                if linked {
                    transform.translation = destination;
                } else if is_character(&engine, actor) {
                    let delta = destination - from3;
                    match mode {
                        Mode::TwoD => {
                            if let Ok(mut controller) = controllers_2d.get_mut(entity) {
                                controller.translation = Some(
                                    controller.translation.unwrap_or(Vec2::ZERO) + delta.truncate(),
                                );
                            }
                        }
                        Mode::ThreeD => {
                            if let Ok(mut controller) = controllers_3d.get_mut(entity) {
                                controller.translation =
                                    Some(controller.translation.unwrap_or(Vec3::ZERO) + delta);
                            }
                        }
                    }
                } else if is_dynamic(&engine, actor) {
                    let delta = (destination - from3) * rate;
                    match mode {
                        Mode::TwoD => {
                            if let Ok(mut velocity) = velocities_2d.get_mut(entity) {
                                velocity.linear = delta.truncate();
                            }
                        }
                        Mode::ThreeD => {
                            if let Ok(mut velocity) = velocities_3d.get_mut(entity) {
                                velocity.linear.x = delta.x;
                                velocity.linear.z = delta.z;
                            }
                        }
                    }
                } else {
                    transform.translation = destination;
                }
            }
            Effect::ChangePosition { axis, by, .. } => {
                if is_dynamic(&engine, actor) || is_character(&engine, actor) {
                    continue;
                }
                if let Some(index) = position_axis(dimension.0, *axis) {
                    transform.translation[index] += *by;
                }
            }
            Effect::Turn { axis, degrees, .. } => {
                // A dynamic body turns in its dimension's own system, in
                // effect order with its deferred `move`. Turning here would
                // net a turn-move-turn sandwich (how strafe is spelled) back
                // to zero before the move ever sees the turn, so both strafe
                // keys would walk forward. See `dim2|3::apply_effects`.
                if is_dynamic(&engine, actor) {
                    continue;
                }
                if dimension.0 == Mode::ThreeD {
                    turn_3d(&mut transform, *axis, degrees.to_radians());
                } else if rotation_axis(dimension.0, *axis).is_some() {
                    turn_2d(&mut transform, degrees.to_radians());
                }
            }
            Effect::SetRotation { axis, degrees, .. } => {
                if dimension.0 == Mode::ThreeD {
                    set_rotation_3d(&mut transform, *axis, degrees.to_radians());
                } else if let Some(index) = rotation_axis(dimension.0, *axis) {
                    let mut euler = euler_of(&transform);
                    euler[index] = degrees.to_radians();
                    transform.rotation = quat_of(euler);
                }
            }
            Effect::PointTowards { target, .. } => {
                let here = transform.translation;
                let Some(to) = target_position(&engine, &positions, target, here) else {
                    continue;
                };
                match dimension.0 {
                    Mode::TwoD => {
                        let delta = to - here;
                        let mut euler = euler_of(&transform);
                        euler[2] = delta.y.atan2(delta.x);
                        transform.rotation = quat_of(euler);
                    }
                    Mode::ThreeD => transform.look_at(to, Vec3::Y),
                }
            }
            Effect::SetScale { factor, .. } => {
                transform.scale = Vec3::from(engine.stretch_of(actor)) * *factor
            }
            Effect::SetVisible { visible, .. } => {
                *visibility = if *visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            Effect::Glide {
                seconds,
                target,
                easing,
                ..
            } => {
                let to = vec3_in(dimension.0, *target, transform.translation);
                if *seconds <= 0.0 {
                    transform.translation = to;
                    commands.entity(entity).remove::<Gliding>();
                } else {
                    commands.entity(entity).insert(Gliding {
                        from: transform.translation,
                        to,
                        elapsed: 0.0,
                        duration: *seconds,
                        easing: *easing,
                    });
                }
            }
            Effect::TweenScale {
                factor,
                seconds,
                easing,
                ..
            } => {
                let to = factor.max(0.0);
                let from = size_of(&transform, engine.stretch_of(actor));
                if *seconds <= 0.0 {
                    transform.scale = Vec3::from(engine.stretch_of(actor)) * to;
                    commands.entity(entity).remove::<TweeningScale>();
                } else {
                    commands.entity(entity).insert(TweeningScale {
                        from,
                        to,
                        elapsed: 0.0,
                        duration: *seconds,
                        easing: *easing,
                    });
                }
            }
            Effect::TweenRotation {
                axis,
                degrees,
                seconds,
                easing,
                ..
            } => {
                let from = rotation_of(&transform, dimension.0, *axis);
                if *seconds <= 0.0 {
                    set_rotation_of(&mut transform, dimension.0, *axis, *degrees);
                    commands.entity(entity).remove::<TweeningRotation>();
                } else {
                    commands.entity(entity).insert(TweeningRotation {
                        axis: *axis,
                        from,
                        to: *degrees,
                        elapsed: 0.0,
                        duration: *seconds,
                        easing: *easing,
                    });
                }
            }
            Effect::TweenColor {
                color,
                seconds,
                easing,
                ..
            } => {
                let to = parse_color(color);
                if *seconds <= 0.0 {
                    commands.entity(entity).remove::<TweeningColor>();
                    // The dimensions own the tint; leave a finished tween so
                    // the stepper paints the final color once, in both.
                    commands.entity(entity).insert(TweeningColor {
                        from: Some(to),
                        to,
                        elapsed: 1.0,
                        duration: 1.0,
                        easing: *easing,
                    });
                } else {
                    // `from` fills in on the first step from whatever is
                    // showing, so a mid-tween retarget eases out of the live
                    // color rather than snapping back to the authored one.
                    commands.entity(entity).insert(TweeningColor {
                        from: None,
                        to,
                        elapsed: 0.0,
                        duration: *seconds,
                        easing: *easing,
                    });
                }
            }
            Effect::StopTweens { .. } => {
                commands.entity(entity).remove::<Gliding>();
                commands.entity(entity).remove::<TweeningScale>();
                commands.entity(entity).remove::<TweeningRotation>();
                commands.entity(entity).remove::<TweeningColor>();
            }
            // Physics, colors and speech are somebody else's job.
            _ => {}
        }
    }
}

/// Advances every glide, and drops the component when it arrives.
/// Frozen while paused or stopped, like the VM and physics.
pub fn step_glides(
    mut commands: Commands,
    time: Res<Time>,
    engine: NonSend<Engine>,
    mut gliding: Query<(Entity, &mut Transform, &mut Gliding)>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for (entity, mut transform, mut glide) in &mut gliding {
        glide.elapsed += time.delta_secs();
        let linear = (glide.elapsed / glide.duration).clamp(0.0, 1.0);
        transform.translation = glide.from.lerp(glide.to, glide.easing.apply(linear));
        if linear >= 1.0 {
            commands.entity(entity).remove::<Gliding>();
        }
    }
}

/// Advances every scale, rotation and color tween. Frozen while paused or
/// stopped, like glides. Color writes land on the sprite in 2D and the PBR
/// material in 3D, the same halves `set color` writes through.
pub fn step_tweens(
    mut commands: Commands,
    time: Res<Time>,
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    mut motion: Query<(
        Entity,
        &mut Transform,
        Option<&mut TweeningScale>,
        Option<&mut TweeningRotation>,
    )>,
    mut colors: Query<(
        Entity,
        &mut TweeningColor,
        Option<&mut Sprite>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    // Color is presentation; a headless world tweens motion only.
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut transform, scale, rotation) in &mut motion {
        if let Some(mut tween) = scale {
            tween.elapsed += dt;
            let linear = (tween.elapsed / tween.duration.max(1e-6)).clamp(0.0, 1.0);
            let value = tween.from + (tween.to - tween.from) * tween.easing.apply(linear);
            let stretch = engine
                .actor_id_of(entity)
                .and_then(|id| engine.actor(id))
                .map(|actor| actor.placement().stretch)
                .unwrap_or([1.0; 3]);
            transform.scale = Vec3::from(stretch) * value.max(0.0);
            if linear >= 1.0 {
                commands.entity(entity).remove::<TweeningScale>();
            }
        }
        if let Some(mut tween) = rotation {
            tween.elapsed += dt;
            let linear = (tween.elapsed / tween.duration.max(1e-6)).clamp(0.0, 1.0);
            let value = tween.from + (tween.to - tween.from) * tween.easing.apply(linear);
            set_rotation_of(&mut transform, dimension.0, tween.axis, value);
            if linear >= 1.0 {
                commands.entity(entity).remove::<TweeningRotation>();
            }
        }
    }
    for (entity, mut tween, sprite, material) in &mut colors {
        // First step paints from whatever is showing, so a retarget eases
        // out of the live color.
        if tween.from.is_none() {
            tween.from = Some(
                sprite
                    .as_ref()
                    .map(|sprite| sprite.color)
                    .or_else(|| {
                        material.as_ref().and_then(|handle| {
                            materials.as_ref()?.get(&handle.0).map(|mat| mat.base_color)
                        })
                    })
                    .unwrap_or(Color::WHITE),
            );
        }
        tween.elapsed += dt;
        let linear = (tween.elapsed / tween.duration.max(1e-6)).clamp(0.0, 1.0);
        let eased = tween.easing.apply(linear);
        let from = tween.from.unwrap_or(Color::WHITE);
        let mixed = mix_color(from, tween.to, eased);
        if let Some(mut sprite) = sprite {
            sprite.color = mixed;
        }
        if let Some(handle) = material
            && let Some(materials) = materials.as_mut()
            && let Some(mut mat) = materials.get_mut(&handle.0)
        {
            mat.base_color = mixed;
        }
        if linear >= 1.0 {
            commands.entity(entity).remove::<TweeningColor>();
        }
    }
}

pub(crate) fn mix_color(from: Color, to: Color, t: f32) -> Color {
    let from = from.to_srgba();
    let to = to.to_srgba();
    Color::srgba(
        from.red + (to.red - from.red) * t,
        from.green + (to.green - from.green) * t,
        from.blue + (to.blue - from.blue) * t,
        from.alpha + (to.alpha - from.alpha) * t,
    )
}

/// Drives the world camera from the actor carrying a camera component.
/// Nothing attached leaves the camera where `rebuild_world` put it.
///
/// A 2D project has no depth to stand in, so all three views mean the same
/// thing there: centre on the actor, offset by the rig's x and y.
pub fn drive_camera(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    editor: Option<Res<crate::edit::SceneEditor>>,
    time: Res<Time>,
    mut wheel: MessageReader<MouseWheel>,
    // The query service reads every Transform, so the camera's write is a separate set member.
    mut sets: ParamSet<(
        crate::queries::QueryAccess,
        Query<(&mut Transform, Option<&mut Projection>), With<WorldCamera>>,
    )>,
    mut facing: ResMut<crate::player_camera::BodyFacing>,
    mut state: Local<crate::player_camera::RigState>,
    mut rigs: Query<(&mut CameraRig, &Transform, Option<&ActorId>), Without<WorldCamera>>,
    bodies: Query<&Transform, Without<WorldCamera>>,
    following: Option<Res<crate::server::Following>>,
) {
    // Scroll is read every frame so none queues up behind a held-off camera.
    let scroll: f32 = wheel
        .read()
        .map(|w| match w.unit {
            MouseScrollUnit::Line => w.y,
            MouseScrollUnit::Pixel => w.y / 40.0,
        })
        .sum();
    // The scene view flies the camera itself.
    if editor.is_some_and(|editor| crate::edit::editing(&engine, &editor)) {
        return;
    }
    let Some((mut rig, rig_pose, rig_id)) = rigs.iter_mut().next() else {
        return;
    };
    let rig_id = rig_id.map_or("", |id| id.0.as_str());
    // A LAN guest's camera follows its own seat's actor, framed by the rig.
    let seat = following.as_ref().map_or(0, |f| f.seat());
    let seated = (seat > 0)
        .then(|| crate::player_camera::seat_actor(&engine, seat))
        .flatten()
        .and_then(|actor| {
            let pose = bodies.get(*engine.entities.get(&actor)?).ok()?;
            Some((actor, *pose))
        });
    let (id, target) = match &seated {
        Some((actor, pose)) => (actor.as_str(), pose),
        None => (rig_id, rig_pose),
    };
    let Ok(live) = sets.p1().single_mut().map(|(t, _)| *t) else {
        return;
    };
    let camera_of = |id: &str| {
        engine
            .actor(id)
            .and_then(|actor| actor.components.player_camera().copied())
            .filter(|spec| spec.enabled)
    };
    let spec = camera_of(id).or_else(|| camera_of(rig_id));
    facing.0.clear();
    let mut next = live;
    let dt = time.delta_secs();
    let mode = dimension.0;
    match (&spec, mode) {
        (Some(spec), Mode::TwoD) => {
            crate::player_camera::follow_2d(spec, &mut state, &rig.0, target, &mut next, dt, id);
            write_camera(&mut sets.p1(), next, None);
            return;
        }
        (None, Mode::TwoD) => {
            let offset = Vec3::from(rig.0.offset);
            next.translation = Vec3::new(
                target.translation.x + offset.x,
                target.translation.y + offset.y,
                next.translation.z,
            );
            write_camera(&mut sets.p1(), next, None);
            return;
        }
        _ => {}
    }
    let live_rig = rig.0;
    let mut edited = live_rig;
    let access = sets.p0();
    crate::player_camera::pose_3d(
        &engine,
        &access,
        spec.as_ref(),
        &mut state,
        &mut edited,
        target,
        id,
        scroll,
        dt,
        &mut next,
        &mut facing,
    );
    if edited != live_rig {
        rig.0 = edited;
    }
    write_camera(&mut sets.p1(), next, Some(edited.fov));
}

/// Lays the camera's pose (and its field of view, when the rig owns it) on the world camera.
pub(crate) fn write_camera(
    cameras: &mut Query<(&mut Transform, Option<&mut Projection>), With<WorldCamera>>,
    next: Transform,
    fov: Option<f32>,
) {
    let Ok((mut live, projection)) = cameras.single_mut() else {
        return;
    };
    live.set_if_neq(next);
    if let (Some(fov), Some(mut projection)) = (fov, projection)
        && let Projection::Perspective(perspective) = projection.as_mut()
    {
        perspective.fov = fov.clamp(30.0, 110.0).to_radians();
    }
}

/// Puts every actor back at the pose its last fixed step left it at, before
/// this one starts.
///
/// [`interpolate_poses`] writes a transform part-way between two steps so the
/// display is smooth, and that drawn pose is what a fixed step would
/// otherwise find in `Transform` and build on: a `move` would add to it, and
/// `apply_parenting` would read the difference as motion the parent never
/// made. Simulation starts from the settled pose; the renderer's is only for
/// the frames in between.
pub fn restore_poses(engine: NonSend<Engine>, mut posed: Query<(&mut Transform, &PhysicsPose)>) {
    if !engine.running || engine.paused {
        return;
    }
    for (mut transform, pose) in &mut posed {
        if *transform != pose.0 {
            *transform = pose.0;
        }
    }
}

/// Where an actor is drawn, `alpha` of the way through a fixed step.
pub fn blend_pose(
    mode: blockloom_core::physics::Interpolation,
    previous: &Transform,
    current: &Transform,
    alpha: f32,
) -> Transform {
    use blockloom_core::physics::Interpolation;
    // At rest there is nothing to blend, and a slerp of equal rotations can
    // still come back an ulp off.
    if previous == current || mode == Interpolation::None {
        return *current;
    }
    // Extrapolating carries the last step's motion on by the fraction shown.
    let t = if mode == Interpolation::Extrapolate {
        1.0 + alpha
    } else {
        alpha
    };
    Transform {
        translation: previous.translation.lerp(current.translation, t),
        rotation: previous.rotation.slerp(current.rotation, t),
        scale: if mode == Interpolation::Extrapolate {
            current.scale
        } else {
            previous.scale.lerp(current.scale, t)
        },
    }
}

/// Renders actors between the poses they settled at, so nothing on screen
/// marches along at the fixed step rate - whether physics wrote the pose or a
/// step's effects did. On a high-refresh display that step pattern would
/// otherwise be visible as stutter.
pub fn interpolate_poses(
    engine: NonSend<Engine>,
    fixed: Res<Time<Fixed>>,
    mut posed: Query<(
        &mut Transform,
        &PhysicsPose,
        &PrevPose,
        Option<&crate::physics_install::PoseSmoothing>,
    )>,
) {
    // Writes only what differs: an assignment through `DerefMut` marks the
    // transform changed, and a frame of that on every actor makes Bevy
    // propagate, re-bound and re-extract the whole scene.
    if !engine.running || engine.paused {
        // Frozen: put each actor back exactly where its last step left it.
        for (mut transform, current, _, _) in &mut posed {
            if *transform != current.0 {
                *transform = current.0;
            }
        }
        return;
    }
    let alpha = fixed.overstep_fraction();
    for (mut transform, current, previous, smoothing) in &mut posed {
        use blockloom_core::physics::Interpolation;
        let mode = smoothing.map_or(Interpolation::Interpolate, |s| s.0);
        let next = blend_pose(mode, &previous.0, &current.0, alpha);
        if *transform != next {
            *transform = next;
        }
    }
}
