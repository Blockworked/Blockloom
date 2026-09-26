//! The world editor. While nothing runs, the Game view is a scene view: the
//! world camera is the editor's own (flying in 3D, panning in 2D), a click
//! picks an actor, and a gizmo moves, turns or scales the selected one. A
//! finished drag goes back to the editor as `Placed`, which writes the
//! document and reloads the world like any other edit.
//!
//! Input arrives as the same forwarded `PreviewInput`s a running game reads,
//! taken in order before the game's own input systems see them, so a press
//! and its release inside one frame still make a click.

use crate::engine::{ActorId, Dimension, Engine, PhysicsPose, PrevPose};
use crate::world::{WorldCamera, half_extents, half_extents3};
use bevy::prelude::*;
use bevy::ui::UiScale;
use bevy::window::PrimaryWindow;
use blockloom_core::scene::{Mode, Visual};
use blockloom_core::volume::{VolumeShape, VolumeSpec};
use blockloom_protocol::{PreviewInput, RuntimeMessage, SceneTool, SceneView, VolumeBounds};
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;

/// Handles draw over the world, so a gizmo inside a wall is still usable.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct HandleGizmos;

const AXIS_COLORS: [Color; 3] = [
    Color::srgb(0.90, 0.28, 0.30),
    Color::srgb(0.35, 0.72, 0.36),
    Color::srgb(0.27, 0.51, 0.98),
];
const HOVER: Color = Color::srgb(1.0, 0.84, 0.04);
const GRIP: Color = Color::srgb(0.55, 0.85, 1.0);
const SELECTED: Color = Color::srgb(1.0, 0.62, 0.11);
/// How long a gizmo's axis is on screen, in logical pixels.
const HANDLE_PX: f32 = 90.0;
/// How near the pointer has to be to grab a handle, in logical pixels.
const GRAB_PX: f32 = 9.0;
/// How far a press on an actor travels before it counts as a drag.
const DRAG_PX: f32 = 3.0;
const LOOK_RADIANS_PER_PX: f32 = 0.0035;
const FOV: f32 = 75.0;

/// The 3D editor camera: a position and a heading, plus how far ahead the
/// thing being looked at is, which orbit and pan turn around.
#[derive(Clone, Copy, Debug)]
struct Fly {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    focus: f32,
    speed: f32,
}

impl Fly {
    fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    fn pivot(&self) -> Vec3 {
        self.position + self.forward() * self.focus
    }

    fn look_at(&mut self, from: Vec3, at: Vec3) {
        let dir = (at - from).normalize_or(Vec3::NEG_Z);
        self.position = from;
        self.yaw = f32::atan2(-dir.x, -dir.z);
        self.pitch = dir.y.clamp(-1.0, 1.0).asin();
        self.focus = (at - from).length().max(0.5);
    }
}

/// The 2D editor camera: where it's centred and how far in.
#[derive(Clone, Copy, Debug)]
struct Flat {
    center: Vec2,
    zoom: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Handle {
    /// Along one of the gizmo's axes.
    Axis(usize),
    /// Across the plane square facing this axis.
    Plane(usize),
    /// The centre: across the view.
    Free,
    /// The ring around this axis.
    Ring(usize),
    /// The actor itself, grabbed rather than a handle.
    Body,
    /// A volume's face on this axis, the positive one or the negative.
    Face(usize, bool),
    /// A volume's outer blend edge.
    Feather,
}

/// A volume grip being dragged: the line it slides along, how far along
/// it the grip sat and the pointer pressed, and the spec it started from.
struct Grip {
    origin: Vec3,
    dir: Vec3,
    start_len: f32,
    press: f32,
    start: VolumeSpec,
}

/// A drag in progress: everything as it stood when the button went down.
struct Drag {
    actor: String,
    handle: Handle,
    start: Transform,
    press: Vec2,
    axes: [Vec3; 3],
    /// Where the pointer first met the line or plane being dragged along.
    anchor: Vec3,
    /// Screen-space turn so far, and the pointer's last angle.
    turned: f32,
    last_angle: f32,
    /// A press on an actor only becomes a drag once it has travelled.
    moved: bool,
    /// Size and stretch at the press, and as the drag has them now.
    start_size: f32,
    start_stretch: Vec3,
    size: f32,
    stretch: Vec3,
    grip: Option<Grip>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Nav {
    Look,
    Pan,
    Orbit,
}

/// Everything the scene view knows.
#[derive(Resource)]
pub struct SceneEditor {
    pub view: SceneView,
    pub selected: Option<String>,
    /// Set by `FrameSelected`; the next frame points the camera.
    pub frame: bool,
    /// Whether a project has been loaded, which the camera starts from.
    pub loaded: bool,
    seeded: bool,
    fly: Fly,
    flat: Flat,
    pointer: Option<Vec2>,
    buttons: [bool; 3],
    keys: HashSet<String>,
    nav: Option<Nav>,
    drag: Option<Drag>,
    hover: Option<Handle>,
    /// What to tell the editor, sent by [`report`] once input is done.
    outbox: Vec<RuntimeMessage>,
}

impl Default for SceneEditor {
    fn default() -> Self {
        Self {
            view: SceneView::default(),
            selected: None,
            frame: false,
            loaded: false,
            seeded: false,
            fly: Fly {
                position: Vec3::new(0.0, 6.0, 14.0),
                yaw: 0.0,
                pitch: -0.4,
                focus: 15.0,
                speed: 8.0,
            },
            flat: Flat {
                center: Vec2::ZERO,
                zoom: 1.0,
            },
            pointer: None,
            buttons: [false; 3],
            keys: HashSet::new(),
            nav: None,
            drag: None,
            hover: None,
            outbox: Vec::new(),
        }
    }
}

impl SceneEditor {
    fn held(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.keys.contains(*name))
    }

    fn snapping(&self) -> bool {
        // Ctrl flips the toolbar's snap for one drag, the way most editors do.
        self.view.snap != self.held(&["ControlLeft", "ControlRight"])
    }

    /// Drops everything half done, for a world that started running.
    fn let_go(&mut self) {
        self.nav = None;
        self.drag = None;
        self.hover = None;
        self.buttons = [false; 3];
        self.keys.clear();
    }
}

/// Whether the scene view is what the Game view shows right now.
pub fn editing(engine: &Engine, editor: &SceneEditor) -> bool {
    !engine.running && !engine.starting && editor.view.enabled
}

/// How the scene view's camera sees, worked out from its own pose rather
/// than the camera's last-propagated transform, so a camera moved this frame
/// is already where the pointer maths expects it.
enum Lens<'a> {
    Flat {
        center: Vec2,
        /// Screen pixels per world unit.
        scale: f32,
        size: Vec2,
    },
    Deep {
        camera: &'a Camera,
        transform: GlobalTransform,
    },
}

impl Lens<'_> {
    fn screen(&self, point: Vec3) -> Option<Vec2> {
        match self {
            Lens::Flat {
                center,
                scale,
                size,
            } => {
                let d = (point.truncate() - *center) * *scale;
                Some(Vec2::new(size.x / 2.0 + d.x, size.y / 2.0 - d.y))
            }
            Lens::Deep { camera, transform } => {
                let at = camera.world_to_viewport_with_depth(transform, point).ok()?;
                (at.z > 0.0).then_some(at.truncate())
            }
        }
    }

    fn ray(&self, px: Vec2) -> Option<Ray3d> {
        match self {
            Lens::Flat {
                center,
                scale,
                size,
            } => {
                let d = Vec2::new(px.x - size.x / 2.0, size.y / 2.0 - px.y) / *scale;
                let at = *center + d;
                Some(Ray3d::new(Vec3::new(at.x, at.y, 1.0e5), Dir3::NEG_Z))
            }
            Lens::Deep { camera, transform } => camera.viewport_to_world(transform, px).ok(),
        }
    }

    fn forward(&self) -> Vec3 {
        match self {
            Lens::Flat { .. } => Vec3::NEG_Z,
            Lens::Deep { transform, .. } => transform.forward().as_vec3(),
        }
    }

    fn right(&self) -> Vec3 {
        match self {
            Lens::Flat { .. } => Vec3::X,
            Lens::Deep { transform, .. } => transform.right().as_vec3(),
        }
    }

    /// World units one screen pixel spans at `point`.
    fn world_per_px(&self, point: Vec3) -> f32 {
        if let Lens::Flat { scale, .. } = self {
            return 1.0 / scale.max(1e-6);
        }
        let (Some(a), Some(b)) = (self.screen(point), self.screen(point + self.right())) else {
            return 0.01;
        };
        1.0 / (a - b).length().max(1e-4)
    }
}

// ─── Input ─────────────────────────────────────────────────────────────────

/// Takes this frame's forwarded input while the scene view is up, and turns
/// it into camera moves, picks and drags. Runs straight after `pump_editor`,
/// ahead of the game's own input systems.
pub fn interact(
    mut engine: NonSendMut<Engine>,
    mut editor: ResMut<SceneEditor>,
    dimension: Res<Dimension>,
    time: Res<Time<Real>>,
    ui_scale: Res<UiScale>,
    cameras: Query<&Camera, With<WorldCamera>>,
    actors: Query<(&ActorId, &Visibility)>,
    mut posed: Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    #[cfg(target_os = "linux")] surface: Option<Res<crate::embed::GameSurface>>,
) {
    if !editing(&engine, &editor) {
        editor.let_go();
        return;
    }
    let editor = editor.as_mut();
    seed_camera(&engine, editor, dimension.0);

    let space = windows
        .single()
        .map(|window| Vec2::new(window.width(), window.height()))
        .unwrap_or(Vec2::new(960.0, 720.0));
    #[cfg(target_os = "linux")]
    let space = surface.map_or(space, |surface| surface.size());
    let px_scale = ui_scale.0.max(0.25);
    let mode = dimension.0;

    // The rest of the world's input reads nothing while the scene is edited.
    let inputs = std::mem::take(&mut engine.preview_inputs);
    let mut raw = Vec2::ZERO;
    let mut moved = Vec2::ZERO;
    let mut notches = 0.0;
    let Ok(camera) = cameras.single() else {
        return;
    };
    for input in inputs {
        match input {
            PreviewInput::MouseMove { x, y, w, h } => {
                let at = crate::preview::preview_to_window(x, y, w, h, space);
                if let Some(was) = editor.pointer {
                    moved += at - was;
                }
                editor.pointer = Some(at);
            }
            PreviewInput::MouseDelta { dx, dy } => raw += Vec2::new(dx, dy),
            PreviewInput::MouseButton {
                button,
                down,
                x,
                y,
                w,
                h,
            } => {
                let at = crate::preview::preview_to_window(x, y, w, h, space);
                editor.pointer = Some(at);
                let index = (button as usize).min(2);
                editor.buttons[index] = down;
                let lens = lens(editor, mode, camera, space, px_scale);
                if down {
                    press(
                        &mut engine,
                        editor,
                        &lens,
                        index,
                        at,
                        px_scale,
                        &actors,
                        &posed,
                    );
                } else {
                    // The release is where the drag ends, not where the
                    // pointer last moved.
                    if index == 0 && editor.drag.is_some() {
                        update_drag(&mut engine, editor, &lens, at, mode, px_scale, &mut posed);
                        finish_drag(&engine, editor, &posed);
                    }
                    release_nav(editor, index);
                }
            }
            PreviewInput::Scroll { dy, line, .. } => {
                notches += if line { dy } else { dy / 40.0 };
            }
            PreviewInput::Key { code, down } => {
                if down {
                    if code == "Escape" {
                        cancel_drag(&mut engine, editor, &mut posed);
                    }
                    editor.keys.insert(code);
                } else {
                    editor.keys.remove(&code);
                }
            }
            PreviewInput::Focus { focused: false } => {
                cancel_drag(&mut engine, editor, &mut posed);
                editor.keys.clear();
                editor.buttons = [false; 3];
                editor.nav = None;
            }
            PreviewInput::Focus { .. } | PreviewInput::Text { .. } | PreviewInput::Touch { .. } => {
            }
        }
    }

    // A locked pointer reports raw motion and no moves; otherwise the moves
    // are the motion.
    let motion = if raw != Vec2::ZERO { raw } else { moved };
    let dt = time.delta_secs().min(0.1);
    match mode {
        Mode::ThreeD => steer_3d(editor, space, px_scale, motion, notches, dt),
        Mode::TwoD => steer_2d(editor, space, px_scale, motion, notches),
    }

    let lens = lens(editor, mode, camera, space, px_scale);
    if editor.frame {
        editor.frame = false;
        frame_selected(&engine, editor, mode, &posed);
    }
    if let Some(at) = editor.pointer {
        if editor.drag.is_some() {
            update_drag(&mut engine, editor, &lens, at, mode, px_scale, &mut posed);
        } else if editor.nav.is_none() {
            editor.hover = hovered(&engine, editor, &lens, at, px_scale, &posed);
        }
    }
}

/// The first time a project arrives, the scene view starts where its camera
/// stands.
fn seed_camera(engine: &Engine, editor: &mut SceneEditor, mode: Mode) {
    if editor.seeded || !editor.loaded {
        return;
    }
    editor.seeded = true;
    let camera = &engine.project.world.camera;
    match mode {
        Mode::ThreeD => editor
            .fly
            .look_at(Vec3::from(camera.position), Vec3::from(camera.look_at)),
        Mode::TwoD => {
            editor.flat = Flat {
                center: Vec2::ZERO,
                zoom: camera.zoom.max(0.05),
            };
        }
    }
}

fn lens<'a>(
    editor: &SceneEditor,
    mode: Mode,
    camera: &'a Camera,
    size: Vec2,
    px_scale: f32,
) -> Lens<'a> {
    match mode {
        Mode::TwoD => Lens::Flat {
            center: editor.flat.center,
            scale: editor.flat.zoom * px_scale,
            size,
        },
        Mode::ThreeD => Lens::Deep {
            camera,
            transform: GlobalTransform::from(
                Transform::from_translation(editor.fly.position)
                    .with_rotation(editor.fly.rotation()),
            ),
        },
    }
}

fn press(
    engine: &mut Engine,
    editor: &mut SceneEditor,
    lens: &Lens,
    button: usize,
    at: Vec2,
    px_scale: f32,
    actors: &Query<(&ActorId, &Visibility)>,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let alt = editor.held(&["AltLeft", "AltRight"]);
    let flat = matches!(lens, Lens::Flat { .. });
    match button {
        0 if alt && !flat => editor.nav = Some(Nav::Orbit),
        0 => {
            if let Some(handle) = hovered(engine, editor, lens, at, px_scale, posed) {
                start_drag(engine, editor, lens, handle, at, posed);
                return;
            }
            let Some(actor) = pick(engine, lens, at, actors, posed) else {
                return;
            };
            if editor.selected.as_deref() != Some(actor.as_str()) {
                editor.selected = Some(actor.clone());
                editor.outbox.push(RuntimeMessage::Picked { actor });
            }
            // A press on an actor with the move tool out drags it straight away.
            if editor.view.tool == SceneTool::Move {
                start_drag(engine, editor, lens, Handle::Body, at, posed);
            }
        }
        1 if flat => editor.nav = Some(Nav::Pan),
        1 => editor.nav = Some(Nav::Look),
        _ => editor.nav = Some(Nav::Pan),
    }
}

fn release_nav(editor: &mut SceneEditor, button: usize) {
    let ended = match editor.nav {
        Some(Nav::Orbit) => button == 0,
        Some(Nav::Look) => button == 1,
        Some(Nav::Pan) => button != 0,
        None => false,
    };
    if ended {
        editor.nav = None;
    }
}

fn steer_3d(
    editor: &mut SceneEditor,
    size: Vec2,
    px_scale: f32,
    motion: Vec2,
    notches: f32,
    dt: f32,
) {
    let fast = editor.held(&["ShiftLeft", "ShiftRight"]);
    let fly = &mut editor.fly;
    match editor.nav {
        Some(Nav::Look) => {
            fly.yaw -= motion.x * LOOK_RADIANS_PER_PX;
            fly.pitch = (fly.pitch - motion.y * LOOK_RADIANS_PER_PX).clamp(-1.55, 1.55);
            // The wheel sets the flying speed while looking around.
            if notches != 0.0 {
                fly.speed = (fly.speed * 1.2f32.powf(notches)).clamp(0.2, 500.0);
            }
            let keys = &editor.keys;
            let axis = |plus: &str, minus: &str| {
                (keys.contains(plus) as i32 - keys.contains(minus) as i32) as f32
            };
            let rotation = fly.rotation();
            let wish = rotation * Vec3::NEG_Z * axis("KeyW", "KeyS")
                + rotation * Vec3::X * axis("KeyD", "KeyA")
                + Vec3::Y * axis("KeyE", "KeyQ");
            let speed = fly.speed * if fast { 4.0 } else { 1.0 };
            fly.position += wish.normalize_or_zero() * speed * dt;
        }
        Some(Nav::Orbit) => {
            let pivot = fly.pivot();
            fly.yaw -= motion.x * LOOK_RADIANS_PER_PX;
            fly.pitch = (fly.pitch - motion.y * LOOK_RADIANS_PER_PX).clamp(-1.55, 1.55);
            fly.position = pivot - fly.forward() * fly.focus;
        }
        Some(Nav::Pan) => {
            // As far as the pivot moves under the pointer.
            let height = size.y / px_scale;
            let per_px =
                2.0 * fly.focus * (FOV.to_radians() / 2.0).tan() / height.max(1.0) / px_scale;
            let rotation = fly.rotation();
            fly.position +=
                (rotation * Vec3::NEG_X * motion.x + rotation * Vec3::Y * motion.y) * per_px;
        }
        None => {}
    }
    // The wheel otherwise dollies towards what's ahead, slower up close.
    if editor.nav != Some(Nav::Look) && notches != 0.0 {
        let fly = &mut editor.fly;
        let step = (fly.focus * 0.15).max(0.2) * notches;
        fly.position += fly.forward() * step;
        fly.focus = (fly.focus - step).max(0.5);
    }
}

fn steer_2d(editor: &mut SceneEditor, size: Vec2, px_scale: f32, motion: Vec2, notches: f32) {
    let flat = &mut editor.flat;
    let scale = flat.zoom * px_scale;
    if editor.nav == Some(Nav::Pan) {
        flat.center += Vec2::new(-motion.x, motion.y) / scale;
    }
    // Zooms about the pointer, so what's under it stays under it.
    if notches != 0.0 {
        let next = (flat.zoom * 1.15f32.powf(notches)).clamp(0.02, 50.0);
        if let Some(at) = editor.pointer {
            let offset = Vec2::new(at.x - size.x / 2.0, size.y / 2.0 - at.y);
            let under = flat.center + offset / scale;
            flat.center = under - offset / (next * px_scale);
        }
        flat.zoom = next;
    }
}

fn frame_selected(
    engine: &Engine,
    editor: &mut SceneEditor,
    mode: Mode,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let Some(transform) = selected_transform(engine, editor, posed) else {
        return;
    };
    let visual = editor
        .selected
        .as_deref()
        .and_then(|id| engine.actor(id))
        .and_then(|actor| actor.visual());
    match mode {
        Mode::ThreeD => {
            let radius = visual
                .map(|visual| (half_extents3(visual) * transform.scale.abs()).length())
                .unwrap_or(0.5)
                .max(0.5);
            let fly = &mut editor.fly;
            fly.focus = radius * 3.0;
            fly.position = transform.translation - fly.forward() * fly.focus;
        }
        Mode::TwoD => editor.flat.center = transform.translation.truncate(),
    }
}

fn selected_transform(
    engine: &Engine,
    editor: &SceneEditor,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<Transform> {
    let entity = engine.entities.get(editor.selected.as_deref()?)?;
    posed.get(*entity).ok().map(|(_, pose, _)| pose.0)
}

// ─── Picking ───────────────────────────────────────────────────────────────

/// The actor under the pointer: the nearest one along the ray in 3D, the
/// topmost in 2D. Hidden actors aren't there to click.
fn pick(
    engine: &Engine,
    lens: &Lens,
    at: Vec2,
    actors: &Query<(&ActorId, &Visibility)>,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<String> {
    let ray = lens.ray(at)?;
    let mut best: Option<(f32, String)> = None;
    for (id, visibility) in actors {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let Some(entity) = engine.entities.get(&id.0) else {
            continue;
        };
        let Ok((_, pose, _)) = posed.get(*entity) else {
            continue;
        };
        let visual = engine.actor(&id.0).and_then(|actor| actor.visual());
        let hit = match lens {
            Lens::Flat { scale, .. } => hit_2d(visual, &pose.0, ray.origin.truncate(), *scale)
                .map(|_| -pose.0.translation.z),
            Lens::Deep { .. } => hit_3d(visual, &pose.0, ray),
        };
        if let Some(distance) = hit
            && best
                .as_ref()
                .is_none_or(|(nearest, _)| distance <= *nearest)
        {
            best = Some((distance, id.0.clone()));
        }
    }
    best.map(|(_, id)| id)
}

fn hit_2d(visual: Option<&Visual>, transform: &Transform, point: Vec2, scale: f32) -> Option<()> {
    let local = transform.rotation.inverse() * (point.extend(0.0) - transform.translation);
    let local = local.truncate() / transform.scale.truncate().abs().max(Vec2::splat(1e-4));
    let inside = match visual {
        Some(Visual::Circle { radius, .. }) => local.length() <= *radius,
        Some(visual) => {
            let half = half_extents(visual);
            local.x.abs() <= half.x && local.y.abs() <= half.y
        }
        // Nothing to draw: the marker stands in for it.
        None => local.length() * transform.scale.x.abs() <= 10.0 / scale,
    };
    inside.then_some(())
}

/// How far along `ray` it meets the actor's box, turned with it.
fn hit_3d(visual: Option<&Visual>, transform: &Transform, ray: Ray3d) -> Option<f32> {
    if let Some(Visual::Sphere { radius, .. }) = visual {
        let radius = radius * transform.scale.abs().max_element();
        let to = transform.translation - ray.origin;
        let along = to.dot(*ray.direction);
        let apart = to.length_squared() - along * along;
        if apart > radius * radius {
            return None;
        }
        let t = along - (radius * radius - apart).sqrt();
        return (along + (radius * radius - apart).sqrt() >= 0.0).then_some(t.max(0.0));
    }
    let half = visual
        .map_or(Vec3::splat(0.25), half_extents3)
        .max(Vec3::splat(0.02));
    let inverse = transform.rotation.inverse();
    let scale = transform.scale.abs().max(Vec3::splat(1e-4));
    let origin = inverse * (ray.origin - transform.translation) / scale;
    let direction = inverse * *ray.direction / scale;
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            if origin[axis].abs() > half[axis] {
                return None;
            }
            continue;
        }
        let a = (-half[axis] - origin[axis]) / direction[axis];
        let b = (half[axis] - origin[axis]) / direction[axis];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
}

// ─── The gizmo ─────────────────────────────────────────────────────────────

/// The selected actor's gizmo: where it stands, its axes, and how long an
/// axis is in world units.
struct Frame {
    origin: Vec3,
    axes: [Vec3; 3],
    length: f32,
}

/// Stretch is in the actor's own frame, so scale handles always follow it.
fn along_actor(view: &SceneView) -> bool {
    view.local || view.tool == SceneTool::Scale
}

fn gizmo_frame(
    engine: &Engine,
    editor: &SceneEditor,
    lens: &Lens,
    px_scale: f32,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<Frame> {
    let transform = selected_transform(engine, editor, posed)?;
    let turned = along_actor(&editor.view);
    let rotation = if turned {
        transform.rotation
    } else {
        Quat::IDENTITY
    };
    let origin = transform.translation;
    Some(Frame {
        origin,
        axes: [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z],
        length: HANDLE_PX * px_scale * lens.world_per_px(origin),
    })
}

/// Which axes a gizmo offers: all three in 3D, the plane's two in 2D (and
/// only the ring around the one facing out of it).
fn offered(lens: &Lens, handle_axes: bool) -> &'static [usize] {
    match (lens, handle_axes) {
        (Lens::Flat { .. }, true) => &[0, 1],
        (Lens::Flat { .. }, false) => &[2],
        (Lens::Deep { .. }, _) => &[0, 1, 2],
    }
}

fn ring_points(frame: &Frame, axis: usize) -> Vec<Vec3> {
    let normal = frame.axes[axis];
    let rotation = Quat::from_rotation_arc(Vec3::Z, normal);
    let radius = frame.length * 0.85;
    (0..=48)
        .map(|i| {
            let angle = i as f32 / 48.0 * std::f32::consts::TAU;
            frame.origin + rotation * Vec3::new(angle.cos(), angle.sin(), 0.0) * radius
        })
        .collect()
}

/// The square for dragging across the plane that faces `axis`.
fn plane_square(frame: &Frame, axis: usize) -> [Vec3; 4] {
    let (a, b) = (frame.axes[(axis + 1) % 3], frame.axes[(axis + 2) % 3]);
    let (near, far) = (frame.length * 0.22, frame.length * 0.42);
    let o = frame.origin;
    [
        o + a * near + b * near,
        o + a * far + b * near,
        o + a * far + b * far,
        o + a * near + b * far,
    ]
}

fn hovered(
    engine: &Engine,
    editor: &SceneEditor,
    lens: &Lens,
    at: Vec2,
    px_scale: f32,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<Handle> {
    // A grip is a small target that can sit on a gizmo arrow, so it wins.
    let grab = GRAB_PX * px_scale;
    volume_grips(engine, editor, lens, posed)
        .into_iter()
        .filter_map(|(handle, point)| Some((lens.screen(point)?.distance(at), handle)))
        .filter(|(distance, _)| *distance <= grab)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, handle)| handle)
        .or_else(|| gizmo_hovered(engine, editor, lens, at, px_scale, posed))
}

fn gizmo_hovered(
    engine: &Engine,
    editor: &SceneEditor,
    lens: &Lens,
    at: Vec2,
    px_scale: f32,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<Handle> {
    let frame = gizmo_frame(engine, editor, lens, px_scale, posed)?;
    let center = lens.screen(frame.origin)?;
    let grab = GRAB_PX * px_scale;
    let mut best: Option<(f32, Handle)> = None;
    let mut consider = |distance: f32, handle: Handle| {
        if distance <= grab && best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, handle));
        }
    };
    match editor.view.tool {
        SceneTool::Move | SceneTool::Scale => {
            if (at - center).length() <= grab * 1.2 {
                return Some(Handle::Free);
            }
            for &axis in offered(lens, true) {
                let end = frame.origin + frame.axes[axis] * frame.length;
                let (Some(a), Some(b)) = (
                    lens.screen(frame.origin + frame.axes[axis] * frame.length * 0.15),
                    lens.screen(end),
                ) else {
                    continue;
                };
                consider(segment_distance(at, a, b), Handle::Axis(axis));
            }
            if editor.view.tool == SceneTool::Move && matches!(lens, Lens::Deep { .. }) {
                for axis in 0..3 {
                    let square: Option<Vec<Vec2>> = plane_square(&frame, axis)
                        .iter()
                        .map(|p| lens.screen(*p))
                        .collect();
                    if square.is_some_and(|square| inside_quad(at, &square)) {
                        consider(0.0, Handle::Plane(axis));
                    }
                }
            }
        }
        SceneTool::Rotate => {
            for &axis in offered(lens, false) {
                let points: Option<Vec<Vec2>> = ring_points(&frame, axis)
                    .iter()
                    .map(|p| lens.screen(*p))
                    .collect();
                let Some(points) = points else {
                    continue;
                };
                let distance = points
                    .windows(2)
                    .map(|pair| segment_distance(at, pair[0], pair[1]))
                    .fold(f32::MAX, f32::min);
                consider(distance, Handle::Ring(axis));
            }
        }
    }
    best.map(|(_, handle)| handle)
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

fn inside_quad(p: Vec2, quad: &[Vec2]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..quad.len() {
        let (a, b) = (quad[i], quad[(i + 1) % quad.len()]);
        let cross = (b - a).perp_dot(p - a);
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

// ─── Volume grips ──────────────────────────────────────────────────────────

/// The selected actor's volume, as the document has it, and where it stands.
fn selected_volume(
    engine: &Engine,
    editor: &SceneEditor,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Option<(VolumeSpec, Transform)> {
    let id = editor.selected.as_deref()?;
    if !engine.has_component(id, "Volume") {
        return None;
    }
    let spec = engine.actor(id)?.components.volume()?.clone();
    if spec.shape == VolumeShape::Global {
        return None;
    }
    Some((spec, selected_transform(engine, editor, posed)?))
}

/// How far the shape reaches along its own `axis` from the centre, in
/// world units.
fn reach(spec: &VolumeSpec, pose: &Transform, axis: usize) -> f32 {
    let scale = pose.scale.abs();
    match spec.shape {
        VolumeShape::Sphere => spec.radius.max(0.0) * scale.max_element(),
        _ => spec.half_extents[axis].max(0.0) * scale[axis],
    }
}

/// A selected volume's grips: one on each face (both ways along each axis
/// the view offers) and one on the blend edge along x.
fn volume_grips(
    engine: &Engine,
    editor: &SceneEditor,
    lens: &Lens,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) -> Vec<(Handle, Vec3)> {
    let Some((spec, pose)) = selected_volume(engine, editor, posed) else {
        return Vec::new();
    };
    let mut grips = Vec::new();
    for &axis in offered(lens, true) {
        let dir = pose.rotation * Vec3::AXES[axis];
        let reach = reach(&spec, &pose, axis);
        for positive in [true, false] {
            let sign = if positive { 1.0 } else { -1.0 };
            grips.push((
                Handle::Face(axis, positive),
                pose.translation + dir * reach * sign,
            ));
        }
    }
    let blend = spec.blend_distance.max(0.0);
    grips.push((
        Handle::Feather,
        pose.translation + pose.rotation * Vec3::X * (reach(&spec, &pose, 0) + blend),
    ));
    grips
}

/// The line a grip slides along, and how far along it the grip sits: a box
/// face from the opposite face, a sphere's edge and the blend edge from the
/// centre.
fn grip_line(spec: &VolumeSpec, pose: &Transform, handle: Handle) -> Option<(Vec3, Vec3, f32)> {
    match handle {
        Handle::Face(axis, positive) => {
            let sign = if positive { 1.0 } else { -1.0 };
            let dir = pose.rotation * Vec3::AXES[axis] * sign;
            let reach = reach(spec, pose, axis);
            Some(match spec.shape {
                VolumeShape::Sphere => (pose.translation, dir, reach),
                _ => (pose.translation - dir * reach, dir, reach * 2.0),
            })
        }
        Handle::Feather => {
            let dir = pose.rotation * Vec3::X;
            let length = reach(spec, pose, 0) + spec.blend_distance.max(0.0);
            Some((pose.translation, dir, length))
        }
        _ => None,
    }
}

/// Slides a grip to `length` along its line: the spec and pose it leaves.
fn resize(grip: &Grip, handle: Handle, pose: &Transform, length: f32) -> (VolumeSpec, Transform) {
    let mut spec = grip.start.clone();
    let mut pose = *pose;
    let scale = pose.scale.abs().max(Vec3::splat(1e-4));
    match (handle, spec.shape) {
        (Handle::Feather, _) => {
            let start = grip.start_len - grip.start.blend_distance.max(0.0);
            spec.blend_distance = (length - start).max(0.0);
        }
        (Handle::Face(..), VolumeShape::Sphere) => {
            spec.radius = length.max(0.01) / scale.max_element();
        }
        (Handle::Face(axis, _), _) => {
            // The opposite face stays put, so the centre follows half way.
            let length = length.max(0.02);
            spec.half_extents[axis] = length / 2.0 / scale[axis];
            pose.translation = grip.origin + grip.dir * length / 2.0;
        }
        _ => {}
    }
    (spec, pose)
}

fn bounds_of(spec: &VolumeSpec) -> VolumeBounds {
    VolumeBounds {
        half_extents: spec.half_extents,
        radius: spec.radius,
        blend_distance: spec.blend_distance,
    }
}

/// Puts `spec` on the document's copy for the drag, so the bounds, the heat
/// map and the blend follow it live. The reload after `Placed` settles it.
fn set_volume(engine: &mut Engine, actor: &str, spec: VolumeSpec) {
    if let Some(actor) = engine.project.actor_mut(actor) {
        actor
            .components
            .insert(blockloom_core::components::ActorComponent::Volume { volume: spec });
    }
}

// ─── Dragging ──────────────────────────────────────────────────────────────

fn start_drag(
    engine: &Engine,
    editor: &mut SceneEditor,
    lens: &Lens,
    handle: Handle,
    at: Vec2,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let Some(actor) = editor.selected.clone() else {
        return;
    };
    let Some(start) = selected_transform(engine, editor, posed) else {
        return;
    };
    let turned = along_actor(&editor.view);
    let rotation = if turned {
        start.rotation
    } else {
        Quat::IDENTITY
    };
    let axes = [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z];
    let stretch = engine.stretch_of(&actor);
    let size = crate::world::size_of(&start, stretch);
    let mut drag = Drag {
        actor,
        handle,
        start,
        press: at,
        axes,
        anchor: Vec3::ZERO,
        turned: 0.0,
        last_angle: 0.0,
        moved: handle != Handle::Body,
        start_size: size,
        start_stretch: Vec3::from(stretch),
        size,
        stretch: Vec3::from(stretch),
        grip: None,
    };
    if let Some((spec, pose)) = selected_volume(engine, editor, posed)
        && let Some((origin, dir, start_len)) = grip_line(&spec, &pose, handle)
    {
        let press = lens
            .ray(at)
            .and_then(|ray| closest_on_line(origin, dir, ray))
            .unwrap_or(start_len);
        drag.grip = Some(Grip {
            origin,
            dir,
            start_len,
            press,
            start: spec,
        });
    }
    if let Some(ray) = lens.ray(at) {
        drag.anchor = anchor_for(&drag, lens, ray).unwrap_or(start.translation);
    }
    if let Some(center) = lens.screen(start.translation) {
        let d = at - center;
        drag.last_angle = d.y.atan2(d.x);
    }
    editor.drag = Some(drag);
    editor.hover = Some(handle);
}

/// Where `ray` meets what the handle drags along: a point on the axis line,
/// or on the plane.
fn anchor_for(drag: &Drag, lens: &Lens, ray: Ray3d) -> Option<Vec3> {
    let origin = drag.start.translation;
    match drag.handle {
        Handle::Axis(axis) => {
            let s = closest_on_line(origin, drag.axes[axis], ray)?;
            Some(origin + drag.axes[axis] * s)
        }
        _ => {
            let normal = drag_plane(drag, lens, ray);
            let t = ray.intersect_plane(origin, InfinitePlane3d::new(normal))?;
            Some(ray.get_point(t))
        }
    }
}

/// The plane a free or body drag slides the actor across.
fn drag_plane(drag: &Drag, lens: &Lens, ray: Ray3d) -> Vec3 {
    match (drag.handle, lens) {
        (_, Lens::Flat { .. }) => Vec3::Z,
        (Handle::Plane(axis), _) => drag.axes[axis],
        // Across the ground, unless the ground is seen edge on.
        (Handle::Body, _) if ray.direction.y.abs() > 0.2 => Vec3::Y,
        _ => lens.forward(),
    }
}

/// How far along the line through `origin` in `axis` comes nearest `ray`.
fn closest_on_line(origin: Vec3, axis: Vec3, ray: Ray3d) -> Option<f32> {
    let d = *ray.direction;
    let w = origin - ray.origin;
    let b = axis.dot(d);
    let denominator = 1.0 - b * b;
    if denominator < 1e-4 {
        return None;
    }
    Some((b * d.dot(w) - axis.dot(w)) / denominator)
}

fn snap(value: f32, step: f32) -> f32 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

fn update_drag(
    engine: &mut Engine,
    editor: &mut SceneEditor,
    lens: &Lens,
    at: Vec2,
    mode: Mode,
    px_scale: f32,
    posed: &mut Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let snapping = editor.snapping();
    let view = editor.view.clone();
    let Some(drag) = editor.drag.as_mut() else {
        return;
    };
    if !drag.moved {
        if at.distance(drag.press) < DRAG_PX {
            return;
        }
        drag.moved = true;
    }
    let Some(entity) = engine.entities.get(&drag.actor).copied() else {
        editor.drag = None;
        return;
    };
    let mut next = drag.start;
    if let Some(grip) = &drag.grip {
        let Some(ray) = lens.ray(at) else {
            return;
        };
        let Some(along) = closest_on_line(grip.origin, grip.dir, ray) else {
            return;
        };
        let mut length = grip.start_len + along - grip.press;
        if snapping {
            length = snap(length, view.grid);
        }
        let (spec, pose) = resize(grip, drag.handle, &drag.start, length);
        let actor = drag.actor.clone();
        set_pose(posed, entity, pose);
        set_volume(engine, &actor, spec);
        carry_children(engine, &actor, posed);
        return;
    }
    match (view.tool, drag.handle) {
        (SceneTool::Rotate, Handle::Ring(axis)) => {
            let Some(center) = lens.screen(drag.start.translation) else {
                return;
            };
            let d = at - center;
            let angle = d.y.atan2(d.x);
            let mut step = angle - drag.last_angle;
            if step > std::f32::consts::PI {
                step -= std::f32::consts::TAU;
            } else if step < -std::f32::consts::PI {
                step += std::f32::consts::TAU;
            }
            drag.turned += step;
            drag.last_angle = angle;
            // Screen y points down, so a turn that looks anticlockwise about
            // an axis facing the camera is a positive one.
            let facing = drag.axes[axis].dot(lens.forward());
            let sign = if facing > 0.0 { 1.0 } else { -1.0 };
            let mut turn = drag.turned * sign;
            if snapping {
                turn = snap(turn, view.angle.to_radians());
            }
            next.rotation =
                (Quat::from_axis_angle(drag.axes[axis], turn) * drag.start.rotation).normalize();
        }
        (SceneTool::Scale, Handle::Axis(axis)) => {
            // How far along the handle's own screen direction the pointer went.
            let (Some(center), Some(tip)) = (
                lens.screen(drag.start.translation),
                lens.screen(drag.start.translation + drag.axes[axis]),
            ) else {
                return;
            };
            let Some(along) = (tip - center).try_normalize() else {
                return;
            };
            let from = (drag.press - center).dot(along).max(4.0);
            let ratio = (at - center).dot(along) / from;
            let mut stretch = drag.start_stretch[axis] * ratio;
            if snapping {
                stretch = snap(stretch, view.scale).max(view.scale);
            }
            drag.stretch[axis] = stretch.max(0.01);
            drag.size = drag.start_size;
            next.scale = drag.stretch * drag.size;
        }
        (SceneTool::Scale, Handle::Free) => {
            // Up grows and down shrinks, doubling every 100 pixels.
            let rise = (drag.press.y - at.y) / (100.0 * px_scale);
            let mut size = drag.start_size * 2f32.powf(rise);
            if snapping {
                size = snap(size, view.scale).max(view.scale);
            }
            drag.size = size.max(0.01);
            drag.stretch = drag.start_stretch;
            next.scale = drag.stretch * drag.size;
        }
        (_, handle) => {
            let Some(ray) = lens.ray(at) else {
                return;
            };
            let Some(point) = anchor_for(drag, lens, ray) else {
                return;
            };
            let delta = point - drag.anchor;
            let moved_axes: &[usize] = match (handle, mode) {
                (Handle::Axis(axis), _) => &[[0], [1], [2]][axis],
                (Handle::Plane(0), _) => &[1, 2],
                (Handle::Plane(1), _) => &[0, 2],
                (Handle::Plane(_), _) => &[0, 1],
                (_, Mode::TwoD) => &[0, 1],
                (Handle::Body, _) if drag_plane(drag, lens, ray) == Vec3::Y => &[0, 2],
                _ => &[0, 1, 2],
            };
            let gizmo_axes = matches!(handle, Handle::Axis(_) | Handle::Plane(_));
            if snapping && view.local && gizmo_axes {
                // Along the actor's own axes: the distance snaps, not the spot.
                let mut snapped = Vec3::ZERO;
                for &axis in moved_axes {
                    snapped += drag.axes[axis] * snap(delta.dot(drag.axes[axis]), view.grid);
                }
                next.translation = drag.start.translation + snapped;
            } else {
                next.translation = drag.start.translation + delta;
                if snapping {
                    for &axis in moved_axes {
                        next.translation[axis] = snap(next.translation[axis], view.grid);
                    }
                }
            }
        }
    }
    set_pose(posed, entity, next);
    let actor = drag.actor.clone();
    carry_children(engine, &actor, posed);
}

fn set_pose(
    posed: &mut Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
    entity: Entity,
    pose: Transform,
) {
    if let Ok((mut transform, mut current, mut previous)) = posed.get_mut(entity) {
        *transform = pose;
        current.0 = pose;
        previous.0 = pose;
    }
}

/// Children placed in their parent's frame follow the parent while it's
/// dragged, down the chain, the way the world will build them once the drag
/// lands. One placed by its own `Place` stays put, as it will then too.
fn carry_children(
    engine: &Engine,
    actor: &str,
    posed: &mut Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let mut children: HashMap<&str, Vec<(&str, [f32; 3])>> = HashMap::new();
    for child in &engine.project.actors {
        if let (Some(parent), Some(offset)) = (child.parent(), child.parent_offset()) {
            children
                .entry(parent)
                .or_default()
                .push((child.id.as_str(), offset));
        }
    }
    let mut queue = vec![actor.to_string()];
    let mut seen = HashSet::new();
    while let Some(parent) = queue.pop() {
        if !seen.insert(parent.clone()) {
            continue;
        }
        let Some(entity) = engine.entities.get(&parent) else {
            continue;
        };
        let Ok((_, pose, _)) = posed.get(*entity) else {
            continue;
        };
        let parent_pose = pose.0;
        for (child, offset) in children.get(parent.as_str()).into_iter().flatten() {
            let Some(child_entity) = engine.entities.get(*child).copied() else {
                continue;
            };
            let Ok((_, pose, _)) = posed.get(child_entity) else {
                continue;
            };
            let mut next = pose.0;
            next.translation = crate::world::world_of(&parent_pose, *offset);
            set_pose(posed, child_entity, next);
            queue.push(child.to_string());
        }
    }
}

/// Tells the editor where the dragged actor ended up.
fn finish_drag(
    engine: &Engine,
    editor: &mut SceneEditor,
    posed: &Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let Some(drag) = editor.drag.take() else {
        return;
    };
    if !drag.moved {
        return;
    }
    let Some(pose) = engine
        .entities
        .get(&drag.actor)
        .and_then(|entity| posed.get(*entity).ok())
        .map(|(_, pose, _)| pose.0)
    else {
        return;
    };
    let actor = engine.actor(&drag.actor);
    let volume = drag.grip.as_ref().and_then(|grip| {
        let now = actor?.components.volume()?;
        (now != &grip.start).then(|| bounds_of(now))
    });
    if pose == drag.start && volume.is_none() {
        return;
    }
    // A child placed in its parent's frame keeps that frame.
    let offset = actor
        .filter(|actor| actor.parent_offset().is_some())
        .and_then(|actor| actor.parent())
        .and_then(|parent| engine.entities.get(parent))
        .and_then(|entity| posed.get(*entity).ok())
        .map(|(_, parent, _)| crate::world::local_of(&parent.0, pose.translation));
    editor.outbox.push(RuntimeMessage::Placed {
        actor: drag.actor,
        placement: crate::world::placement_of(&pose, drag.stretch.to_array()),
        offset,
        volume,
    });
}

fn cancel_drag(
    engine: &mut Engine,
    editor: &mut SceneEditor,
    posed: &mut Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
) {
    let Some(drag) = editor.drag.take() else {
        return;
    };
    if let Some(grip) = drag.grip {
        set_volume(engine, &drag.actor, grip.start);
    }
    if let Some(entity) = engine.entities.get(&drag.actor).copied() {
        set_pose(posed, entity, drag.start);
        carry_children(engine, &drag.actor, posed);
    }
}

/// Sends the editor what the scene view did this frame.
pub fn report(mut editor: ResMut<SceneEditor>) {
    for message in editor.outbox.drain(..) {
        crate::bridge::send(&message);
    }
}

// ─── The camera ────────────────────────────────────────────────────────────

/// Puts the world camera where the scene view has it. Runs after
/// `drive_camera`, so an actor holding the camera doesn't pull it away.
pub fn apply_view(
    engine: NonSend<Engine>,
    editor: Res<SceneEditor>,
    dimension: Res<Dimension>,
    ui_scale: Res<UiScale>,
    mut store: ResMut<GizmoConfigStore>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    if !editing(&engine, &editor) {
        return;
    }
    // Lines as thick on a HiDPI view as on any other.
    let px_scale = ui_scale.0.max(0.25);
    store.config_mut::<HandleGizmos>().0.line.width = 2.5 * px_scale;
    store.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 1.5 * px_scale;
    // Written only when it moves: the path tracer starts over on any change.
    for (mut transform, mut projection) in &mut cameras {
        let mut next = *transform;
        match dimension.0 {
            Mode::ThreeD => {
                next.translation = editor.fly.position;
                next.rotation = editor.fly.rotation();
                if let Projection::Perspective(perspective) = projection.as_ref()
                    && perspective.fov != FOV.to_radians()
                    && let Projection::Perspective(perspective) = projection.as_mut()
                {
                    perspective.fov = FOV.to_radians();
                }
            }
            Mode::TwoD => {
                next.translation.x = editor.flat.center.x;
                next.translation.y = editor.flat.center.y;
                if let Projection::Orthographic(ortho) = projection.as_ref()
                    && ortho.scale != 1.0 / editor.flat.zoom
                    && let Projection::Orthographic(ortho) = projection.as_mut()
                {
                    ortho.scale = 1.0 / editor.flat.zoom;
                }
            }
        }
        transform.set_if_neq(next);
    }
}

// ─── Drawing ───────────────────────────────────────────────────────────────

/// The grid, the selection and its gizmo, drawn fresh every frame the scene
/// view is up.
pub fn draw(
    engine: NonSend<Engine>,
    editor: Res<SceneEditor>,
    dimension: Res<Dimension>,
    environment: Res<crate::environment::Environment>,
    ui_scale: Res<UiScale>,
    cameras: Query<&Camera, With<WorldCamera>>,
    actors: Query<(&ActorId, &Visibility)>,
    posed: Query<(&mut Transform, &mut PhysicsPose, &mut PrevPose)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    #[cfg(target_os = "linux")] surface: Option<Res<crate::embed::GameSurface>>,
    mut lines: Gizmos,
    mut handles: Gizmos<HandleGizmos>,
) {
    if !editing(&engine, &editor) {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let px_scale = ui_scale.0.max(0.25);
    let size = windows
        .single()
        .map(|window| Vec2::new(window.width(), window.height()))
        .unwrap_or(Vec2::new(960.0, 720.0));
    #[cfg(target_os = "linux")]
    let size = surface.map_or(size, |surface| surface.size());
    let mode = dimension.0;
    let lens = lens(&editor, mode, camera, size, px_scale);

    if editor.view.show_grid {
        let ink = grid_ink(environment.background);
        match mode {
            Mode::ThreeD => draw_grid_3d(&mut lines, &editor, ink),
            Mode::TwoD => draw_grid_2d(&mut lines, &editor, size, px_scale, ink),
        }
    }
    // The game's camera, unless an actor holds it or the view is standing
    // right where it is.
    let game_camera = &engine.project.world.camera;
    if mode == Mode::ThreeD
        && Vec3::from(game_camera.position).distance(editor.fly.position) > 1.0
        && !engine
            .project
            .actors
            .iter()
            .any(|a| a.components.contains("Camera"))
    {
        draw_game_camera(&mut lines, game_camera);
    }

    if mode == Mode::ThreeD {
        draw_cloud_layers(&mut lines, &engine.project.world.cloud_layers, &editor);
    }

    // Actors with nothing to draw still get a marker, so they can be found.
    for (id, visibility) in &actors {
        let Some(entity) = engine.entities.get(&id.0) else {
            continue;
        };
        let Ok((_, pose, _)) = posed.get(*entity) else {
            continue;
        };
        let visual = engine.actor(&id.0).and_then(|actor| actor.visual());
        let hidden = *visibility == Visibility::Hidden;
        let selected = editor.selected.as_deref() == Some(id.0.as_str());
        if visual.is_none() || hidden {
            let size = 10.0 * px_scale * lens.world_per_px(pose.0.translation);
            let color = Color::srgba(1.0, 1.0, 1.0, if hidden { 0.25 } else { 0.6 });
            lines.cross(
                Isometry3d::from_translation(pose.0.translation),
                size,
                color,
            );
        }
        if selected {
            outline(&mut handles, visual, &pose.0, mode, &lens, px_scale);
        }
    }

    let active = editor
        .drag
        .as_ref()
        .map(|drag| drag.handle)
        .or(editor.hover);
    for (handle, point) in volume_grips(&engine, &editor, &lens, &posed) {
        let color = if active == Some(handle) { HOVER } else { GRIP };
        let radius = 5.0 * px_scale * lens.world_per_px(point);
        let facing = Quat::from_rotation_arc(Vec3::Z, -lens.forward());
        let isometry = Isometry3d::new(point, facing);
        if handle == Handle::Feather {
            handles.circle(isometry, radius, color).resolution(16);
        } else {
            handles.rect(isometry, Vec2::splat(radius * 2.0), color);
        }
    }

    let Some(frame) = gizmo_frame(&engine, &editor, &lens, px_scale, &posed) else {
        return;
    };
    let tint = |handle: Handle, color: Color| if active == Some(handle) { HOVER } else { color };
    match editor.view.tool {
        SceneTool::Move => {
            for &axis in offered(&lens, true) {
                let end = frame.origin + frame.axes[axis] * frame.length;
                handles
                    .arrow(
                        frame.origin,
                        end,
                        tint(Handle::Axis(axis), AXIS_COLORS[axis]),
                    )
                    .with_tip_length(frame.length * 0.2);
            }
            if mode == Mode::ThreeD {
                for (axis, color) in AXIS_COLORS.iter().enumerate() {
                    let square = plane_square(&frame, axis);
                    handles.linestrip(
                        [square[0], square[1], square[2], square[3], square[0]],
                        tint(Handle::Plane(axis), color.with_alpha(0.9)),
                    );
                }
            }
            centre_handle(
                &mut handles,
                &frame,
                &lens,
                tint(Handle::Free, Color::WHITE),
            );
        }
        SceneTool::Rotate => {
            for &axis in offered(&lens, false) {
                handles.linestrip(
                    ring_points(&frame, axis),
                    tint(Handle::Ring(axis), AXIS_COLORS[axis]),
                );
            }
        }
        SceneTool::Scale => {
            for &axis in offered(&lens, true) {
                let end = frame.origin + frame.axes[axis] * frame.length;
                let color = tint(Handle::Axis(axis), AXIS_COLORS[axis]);
                handles.line(frame.origin, end, color);
                let cube = frame.length * 0.08;
                handles.cube(
                    Transform::from_translation(end).with_scale(Vec3::splat(cube * 2.0)),
                    color,
                );
            }
            centre_handle(
                &mut handles,
                &frame,
                &lens,
                tint(Handle::Free, Color::WHITE),
            );
        }
    }
}

fn centre_handle(handles: &mut Gizmos<HandleGizmos>, frame: &Frame, lens: &Lens, color: Color) {
    // Faces the camera, so it reads as a dot from anywhere.
    let facing = Quat::from_rotation_arc(Vec3::Z, -lens.forward());
    handles.circle(
        Isometry3d::new(frame.origin, facing),
        frame.length * 0.1,
        color,
    );
}

fn outline(
    handles: &mut Gizmos<HandleGizmos>,
    visual: Option<&Visual>,
    pose: &Transform,
    mode: Mode,
    lens: &Lens,
    px_scale: f32,
) {
    let color = SELECTED;
    match (mode, visual) {
        (Mode::TwoD, Some(Visual::Circle { radius, .. })) => {
            handles
                .circle(
                    Isometry3d::from_translation(pose.translation),
                    radius * pose.scale.x.abs(),
                    color,
                )
                .resolution(48);
        }
        (Mode::TwoD, Some(visual)) => {
            let half = half_extents(visual) * pose.scale.truncate();
            handles.rect(
                Isometry3d::new(pose.translation, pose.rotation),
                half * 2.0,
                color,
            );
        }
        (Mode::ThreeD, Some(Visual::Sphere { radius, .. })) => {
            handles.sphere(
                Isometry3d::new(pose.translation, pose.rotation),
                radius * pose.scale.abs().max_element(),
                color,
            );
        }
        (Mode::ThreeD, Some(visual)) => {
            let half = half_extents3(visual).max(Vec3::splat(0.02));
            handles.cube(
                Transform {
                    translation: pose.translation,
                    rotation: pose.rotation,
                    scale: half * 2.0 * pose.scale,
                },
                color,
            );
        }
        (_, None) => {
            let size = 14.0 * px_scale * lens.world_per_px(pose.translation);
            handles.cross(Isometry3d::from_translation(pose.translation), size, color);
        }
    }
}

fn grid_spacing(step: f32, world_per_px: f32, min_px: f32) -> f32 {
    let mut spacing = if step > 0.0 { step } else { 1.0 };
    while spacing / world_per_px < min_px {
        spacing *= 10.0;
    }
    spacing
}

/// Dark lines over a light background, light over a dark one.
fn grid_ink(background: Color) -> Color {
    let luminance = background.to_linear().luminance();
    if luminance > 0.35 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

fn draw_grid_3d(lines: &mut Gizmos, editor: &SceneEditor, ink: Color) {
    let fly = &editor.fly;
    // Enough cells to reach the horizon from this high up, never too many.
    let reach = (fly.position.y.abs() * 12.0).clamp(20.0, 4000.0);
    let base = if editor.view.grid > 0.0 {
        editor.view.grid
    } else {
        1.0
    };
    let mut spacing = base;
    while reach * 2.0 / spacing > 160.0 {
        spacing *= 10.0;
    }
    let major = spacing * 10.0;
    let center = Vec3::new(
        snap(fly.position.x, major),
        0.0,
        snap(fly.position.z, major),
    );
    let flat = Quat::from_rotation_x(-FRAC_PI_2);
    let cells = ((reach * 2.0 / spacing).round() as u32).max(2);
    lines.grid(
        Isometry3d::new(center, flat),
        UVec2::splat(cells),
        Vec2::splat(spacing),
        ink.with_alpha(0.2),
    );
    let majors = ((reach * 2.0 / major).round() as u32).max(2);
    lines.grid(
        Isometry3d::new(center, flat),
        UVec2::splat(majors),
        Vec2::splat(major),
        ink.with_alpha(0.45),
    );
    lines.line(
        Vec3::new(center.x - reach, 0.0, 0.0),
        Vec3::new(center.x + reach, 0.0, 0.0),
        AXIS_COLORS[0].with_alpha(0.9),
    );
    lines.line(
        Vec3::new(0.0, 0.0, center.z - reach),
        Vec3::new(0.0, 0.0, center.z + reach),
        AXIS_COLORS[2].with_alpha(0.9),
    );
}

fn draw_grid_2d(lines: &mut Gizmos, editor: &SceneEditor, size: Vec2, px_scale: f32, ink: Color) {
    let flat = &editor.flat;
    let world_per_px = 1.0 / (flat.zoom * px_scale);
    let spacing = grid_spacing(editor.view.grid, world_per_px, 8.0 * px_scale);
    let major = spacing * 10.0;
    let half = size * world_per_px / 2.0;
    let center = Vec2::new(snap(flat.center.x, major), snap(flat.center.y, major));
    let reach = half.max_element() + major;
    let cells = ((reach * 2.0 / spacing).ceil() as u32).max(2);
    // Behind every actor, which all stand at z >= 0 unless told otherwise.
    let at = center.extend(-900.0);
    lines.grid(
        Isometry3d::from_translation(at),
        UVec2::splat(cells),
        Vec2::splat(spacing),
        ink.with_alpha(0.18),
    );
    lines.grid(
        Isometry3d::from_translation(at),
        UVec2::splat(((reach * 2.0 / major).ceil() as u32).max(2)),
        Vec2::splat(major),
        ink.with_alpha(0.4),
    );
    lines.line(
        Vec3::new(center.x - reach, 0.0, -900.0),
        Vec3::new(center.x + reach, 0.0, -900.0),
        AXIS_COLORS[0].with_alpha(0.9),
    );
    lines.line(
        Vec3::new(0.0, center.y - reach, -900.0),
        Vec3::new(0.0, center.y + reach, -900.0),
        AXIS_COLORS[1].with_alpha(0.9),
    );
}

/// Where the game's own camera stands, and which way it looks.
/// Each cloud layer as one tile of grid at its altitude, under the view,
/// plus a ring round the pivot of a spinning one.
fn draw_cloud_layers(
    lines: &mut Gizmos,
    layers: &[blockloom_core::cloud_layers::CloudLayer],
    editor: &SceneEditor,
) {
    let flat = Quat::from_rotation_x(-FRAC_PI_2);
    for layer in layers.iter().filter(|l| l.enabled) {
        let tile = layer.tiling_km * 1000.0;
        let color = Color::srgba(0.7, 0.85, 1.0, 0.35);
        let center = Vec3::new(
            snap(editor.fly.position.x, tile),
            layer.altitude,
            snap(editor.fly.position.z, tile),
        );
        lines.grid(
            Isometry3d::new(center, flat),
            UVec2::splat(8),
            Vec2::splat(tile / 8.0),
            color,
        );
        if layer.spin != 0.0 {
            let pivot = Vec3::new(layer.pivot[0], layer.altitude, layer.pivot[1]);
            lines.circle(
                Isometry3d::new(pivot, flat),
                tile * 0.1,
                color.with_alpha(0.7),
            );
        }
    }
}

fn draw_game_camera(lines: &mut Gizmos, camera: &blockloom_core::scene::Camera) {
    let from = Vec3::from(camera.position);
    let at = Vec3::from(camera.look_at);
    let Some(forward) = (at - from).try_normalize() else {
        return;
    };
    let rotation = Transform::from_translation(from)
        .looking_to(forward, Vec3::Y)
        .rotation;
    let color = Color::srgba(0.8, 0.85, 1.0, 0.7);
    let depth = 0.8;
    let (w, h) = (0.5, 0.375);
    let corners = [
        Vec3::new(-w, -h, -depth),
        Vec3::new(w, -h, -depth),
        Vec3::new(w, h, -depth),
        Vec3::new(-w, h, -depth),
    ]
    .map(|corner| from + rotation * corner);
    for corner in corners {
        lines.line(from, corner, color);
    }
    lines.linestrip(
        [corners[0], corners[1], corners[2], corners[3], corners[0]],
        color,
    );
    // Which way is up, as a tick on the top edge.
    let top = (corners[2] + corners[3]) / 2.0;
    lines.line(top, top + rotation * Vec3::Y * 0.2, color);
}

/// Handles draw over what's in front of them. The grid only edges ahead,
/// so it wins against a ground plane at y = 0 without showing through walls.
pub fn configure(app: &mut App) {
    app.init_gizmo_group::<HandleGizmos>()
        .init_resource::<SceneEditor>();
    let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
    store.config_mut::<HandleGizmos>().0.depth_bias = -1.0;
    store.config_mut::<DefaultGizmoConfigGroup>().0.depth_bias = -0.005;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_point_on_an_axis_is_where_the_ray_passes_it() {
        // A ray straight down through x = 3 meets the x axis at 3.
        let ray = Ray3d::new(Vec3::new(3.0, 10.0, 0.0), Dir3::NEG_Y);
        let s = closest_on_line(Vec3::ZERO, Vec3::X, ray).unwrap();
        assert!((s - 3.0).abs() < 1e-4);
        // Looking straight along the axis says nothing about where on it.
        let along = Ray3d::new(Vec3::new(-5.0, 0.0, 0.0), Dir3::X);
        assert!(closest_on_line(Vec3::ZERO, Vec3::X, along).is_none());
    }

    #[test]
    fn a_ray_hits_a_turned_box_only_where_it_is() {
        let visual = Visual::Cuboid {
            color: String::new(),
            size: [4.0, 1.0, 1.0],
        };
        let turned = Transform::from_rotation(Quat::from_rotation_y(FRAC_PI_2));
        // Turned a quarter, the long side runs along z.
        let down_z = Ray3d::new(Vec3::new(0.0, 5.0, 1.8), Dir3::NEG_Y);
        let down_x = Ray3d::new(Vec3::new(1.8, 5.0, 0.0), Dir3::NEG_Y);
        assert!(hit_3d(Some(&visual), &turned, down_z).is_some());
        assert!(hit_3d(Some(&visual), &turned, down_x).is_none());
        let t = hit_3d(Some(&visual), &Transform::IDENTITY, down_x).unwrap();
        assert!((t - 4.5).abs() < 1e-4);
    }

    #[test]
    fn a_circle_is_picked_by_its_round_edge() {
        let ball = Visual::Circle {
            color: String::new(),
            radius: 10.0,
        };
        let at = Transform::from_xyz(100.0, 0.0, 0.0);
        assert!(hit_2d(Some(&ball), &at, Vec2::new(107.0, 0.0), 1.0).is_some());
        // Inside the bounding square, outside the circle.
        assert!(hit_2d(Some(&ball), &at, Vec2::new(108.0, 8.0), 1.0).is_none());
    }

    #[test]
    fn the_flat_lens_maps_screen_and_world_both_ways() {
        let lens = Lens::Flat {
            center: Vec2::new(50.0, 20.0),
            scale: 2.0,
            size: Vec2::new(200.0, 100.0),
        };
        let px = lens.screen(Vec3::new(60.0, 30.0, 0.0)).unwrap();
        assert_eq!(px, Vec2::new(120.0, 30.0));
        let ray = lens.ray(px).unwrap();
        assert!(
            ray.origin
                .truncate()
                .abs_diff_eq(Vec2::new(60.0, 30.0), 1e-4)
        );
    }

    #[test]
    fn a_fly_camera_looks_where_it_was_pointed() {
        let mut fly = SceneEditor::default().fly;
        fly.look_at(Vec3::new(0.0, 6.0, 14.0), Vec3::ZERO);
        let expected = (Vec3::ZERO - Vec3::new(0.0, 6.0, 14.0)).normalize();
        assert!(fly.forward().abs_diff_eq(expected, 1e-4));
        assert!(fly.pivot().abs_diff_eq(Vec3::ZERO, 1e-3));
    }

    /// A 2D scene with a square at (100, 0) and a child hung 100 to its right,
    /// seen from the origin at zoom 1 on a 960 x 720 view.
    fn scene() -> (App, String, String) {
        use blockloom_core::components::ActorComponent;
        use blockloom_core::project::{Actor, Project};
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        let mut project = Project::starter("Scene", Mode::TwoD);
        project.actors.clear();
        let square = Visual::Rect {
            color: "#ffffff".to_string(),
            size: [60.0, 60.0],
        };
        let mut boxed = Actor::new("Box".to_string(), square.clone());
        boxed.components.placement_mut().position = [100.0, 0.0, 0.0];
        let parent = project.add_actor(boxed);
        let mut child = Actor::new("Tag".to_string(), square);
        child.components.insert(ActorComponent::Parent {
            parent: parent.clone(),
            offset: Some([100.0, 0.0, 0.0]),
        });
        let child = project.add_actor(child);
        engine.project = project;

        let mut app = App::new();
        app.insert_resource(Time::<Real>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.init_resource::<UiScale>();
        app.init_resource::<SceneEditor>();
        app.world_mut().spawn((WorldCamera, Camera::default()));
        for (id, at) in [
            (&parent, Vec3::new(100.0, 0.0, 0.0)),
            (&child, Vec3::new(200.0, 0.0, 0.0)),
        ] {
            let pose = Transform::from_translation(at);
            let entity = app
                .world_mut()
                .spawn((
                    ActorId(id.clone()),
                    pose,
                    PhysicsPose(pose),
                    PrevPose(pose),
                    Visibility::Inherited,
                ))
                .id();
            engine.entities.insert(id.clone(), entity);
        }
        app.insert_non_send(engine);
        app.add_systems(Update, interact);
        (app, parent, child)
    }

    fn button(button: u8, down: bool, x: f32, y: f32) -> PreviewInput {
        PreviewInput::MouseButton {
            button,
            down,
            x,
            y,
            w: 960.0,
            h: 720.0,
        }
    }

    fn pose_of(app: &App, id: &str) -> Vec3 {
        let entity = app.world().non_send::<Engine>().entities[id];
        app.world()
            .entity(entity)
            .get::<PhysicsPose>()
            .unwrap()
            .0
            .translation
    }

    #[test]
    fn dragging_an_actor_picks_it_moves_it_and_reports_where_it_landed() {
        let (mut app, parent, child) = scene();
        // The square's centre is 100 right of the view's middle.
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![
            button(0, true, 580.0, 360.0),
            PreviewInput::MouseMove {
                x: 590.0,
                y: 350.0,
                w: 960.0,
                h: 720.0,
            },
            button(0, false, 600.0, 340.0),
        ];
        app.update();

        assert!(pose_of(&app, &parent).abs_diff_eq(Vec3::new(120.0, 20.0, 0.0), 1e-3));
        // The child hangs in its parent's frame, so it came along.
        assert!(pose_of(&app, &child).abs_diff_eq(Vec3::new(220.0, 20.0, 0.0), 1e-3));
        let editor = app.world().resource::<SceneEditor>();
        assert_eq!(editor.selected.as_deref(), Some(parent.as_str()));
        assert_eq!(editor.outbox.len(), 2);
        assert_eq!(
            editor.outbox[0],
            RuntimeMessage::Picked {
                actor: parent.clone()
            }
        );
        let RuntimeMessage::Placed {
            actor,
            placement,
            offset,
            ..
        } = &editor.outbox[1]
        else {
            panic!("expected a placement, got {:?}", editor.outbox[1]);
        };
        assert_eq!(actor, &parent);
        assert!(Vec3::from(placement.position).abs_diff_eq(Vec3::new(120.0, 20.0, 0.0), 1e-3));
        assert_eq!(*offset, None);
    }

    #[test]
    fn a_child_dragged_in_its_parents_frame_reports_its_new_offset() {
        let (mut app, _, child) = scene();
        app.world_mut().resource_mut::<SceneEditor>().view.snap = true;
        app.world_mut().resource_mut::<SceneEditor>().view.grid = 5.0;
        // Grab the child and pull it 23 right, 12 up.
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![
            button(0, true, 680.0, 360.0),
            button(0, false, 703.0, 348.0),
        ];
        app.update();
        let editor = app.world().resource::<SceneEditor>();
        let Some(RuntimeMessage::Placed {
            actor,
            placement,
            offset,
            ..
        }) = editor.outbox.last()
        else {
            panic!("expected a placement, got {:?}", editor.outbox);
        };
        assert_eq!(actor, &child);
        // Snapped to the 5 grid, then measured from the parent at (100, 0).
        assert!(Vec3::from(placement.position).abs_diff_eq(Vec3::new(225.0, 10.0, 0.0), 1e-3));
        let offset = Vec3::from(offset.expect("a child placed in its parent's frame"));
        assert!(offset.abs_diff_eq(Vec3::new(125.0, 10.0, 0.0), 1e-3));
    }

    /// Presses and releases the left button with the scale tool on the parent.
    fn scale_drag(
        from: (f32, f32),
        to: (f32, f32),
    ) -> (App, String, blockloom_core::scene::Placement) {
        let (mut app, parent, _) = scene();
        let mut editor = app.world_mut().resource_mut::<SceneEditor>();
        editor.selected = Some(parent.clone());
        editor.view.tool = SceneTool::Scale;
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![
            button(0, true, from.0, from.1),
            button(0, false, to.0, to.1),
        ];
        app.update();
        let editor = app.world().resource::<SceneEditor>();
        let Some(RuntimeMessage::Placed { placement, .. }) = editor.outbox.last() else {
            panic!("expected a placement, got {:?}", editor.outbox);
        };
        let placement = *placement;
        (app, parent, placement)
    }

    #[test]
    fn an_axis_scale_handle_stretches_only_its_own_axis() {
        // The x handle runs right from the square's centre at (580, 360).
        let (app, parent, placement) = scale_drag((640.0, 360.0), (700.0, 330.0));
        assert!((placement.scale - 1.0).abs() < 1e-4);
        assert!(Vec3::from(placement.stretch).abs_diff_eq(Vec3::new(2.0, 1.0, 1.0), 1e-4));
        let entity = app.world().non_send::<Engine>().entities[&parent];
        let pose = app.world().entity(entity).get::<PhysicsPose>().unwrap().0;
        assert!(pose.scale.abs_diff_eq(Vec3::new(2.0, 1.0, 1.0), 1e-4));
    }

    #[test]
    fn the_centre_scale_handle_grows_upwards_and_shrinks_downwards() {
        let (_, _, up) = scale_drag((580.0, 360.0), (580.0, 260.0));
        assert!((up.scale - 2.0).abs() < 1e-4);
        assert_eq!(up.stretch, [1.0; 3]);
        let (_, _, down) = scale_drag((580.0, 360.0), (580.0, 460.0));
        assert!((down.scale - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_click_that_never_moves_places_nothing() {
        let (mut app, parent, _) = scene();
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![
            button(0, true, 560.0, 360.0),
            button(0, false, 561.0, 360.0),
        ];
        app.update();
        let editor = app.world().resource::<SceneEditor>();
        assert_eq!(
            editor.outbox,
            vec![RuntimeMessage::Picked { actor: parent }]
        );
    }

    #[test]
    fn a_running_world_is_left_alone() {
        let (mut app, parent, _) = scene();
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.running = true;
        engine.preview_inputs = vec![
            button(0, true, 580.0, 360.0),
            button(0, false, 600.0, 340.0),
        ];
        app.update();
        assert!(pose_of(&app, &parent).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-3));
        // The game's own input systems get the clicks instead.
        assert_eq!(app.world().non_send::<Engine>().preview_inputs.len(), 2);
    }

    /// The parent square made a box volume 100 wide with a 20 blend, and
    /// selected, so its grips are out.
    fn volume_scene(shape: VolumeShape) -> (App, String) {
        use blockloom_core::components::ActorComponent;
        let (mut app, parent, _) = scene();
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        let spec = VolumeSpec {
            shape,
            half_extents: [50.0, 50.0, 5.0],
            radius: 50.0,
            blend_distance: 20.0,
            ..VolumeSpec::default()
        };
        engine
            .project
            .actor_mut(&parent)
            .unwrap()
            .components
            .insert(ActorComponent::Volume { volume: spec });
        engine
            .attached
            .entry(parent.clone())
            .or_default()
            .insert("Volume".to_string());
        app.world_mut().resource_mut::<SceneEditor>().selected = Some(parent.clone());
        (app, parent)
    }

    fn grip_drag(app: &mut App, from: f32, to: f32) -> Option<RuntimeMessage> {
        app.world_mut().non_send_mut::<Engine>().preview_inputs =
            vec![button(0, true, from, 360.0), button(0, false, to, 360.0)];
        app.update();
        app.world().resource::<SceneEditor>().outbox.last().cloned()
    }

    #[test]
    fn a_box_face_grip_moves_that_face_and_keeps_the_other() {
        let (mut app, parent) = volume_scene(VolumeShape::Box);
        // The +x face sits at world 150, screen 630, on the move arrow.
        let Some(RuntimeMessage::Placed {
            placement, volume, ..
        }) = grip_drag(&mut app, 630.0, 650.0)
        else {
            panic!("expected a placement");
        };
        let volume = volume.expect("the grip resized the volume");
        assert!((volume.half_extents[0] - 60.0).abs() < 1e-3);
        assert_eq!(volume.half_extents[1], 50.0);
        // The -x face stayed at 50, so the centre moved to 110.
        assert!((placement.position[0] - 110.0).abs() < 1e-3);
        assert!(pose_of(&app, &parent).abs_diff_eq(Vec3::new(110.0, 0.0, 0.0), 1e-3));
        // The world's copy follows the drag, for the bounds and the blend.
        let engine = app.world().non_send::<Engine>();
        let live = engine.actor(&parent).unwrap().components.volume().unwrap();
        assert!((live.half_extents[0] - 60.0).abs() < 1e-3);
    }

    #[test]
    fn sphere_and_blend_grips_resize_around_the_centre() {
        let (mut app, parent) = volume_scene(VolumeShape::Sphere);
        let Some(RuntimeMessage::Placed {
            placement, volume, ..
        }) = grip_drag(&mut app, 630.0, 640.0)
        else {
            panic!("expected a placement");
        };
        assert!((volume.unwrap().radius - 60.0).abs() < 1e-3);
        assert_eq!(placement.position, [100.0, 0.0, 0.0]);

        // The blend edge is 20 past the new radius: world 180, screen 660.
        let Some(RuntimeMessage::Placed { volume, .. }) = grip_drag(&mut app, 660.0, 675.0) else {
            panic!("expected a placement");
        };
        let volume = volume.unwrap();
        assert!((volume.blend_distance - 35.0).abs() < 1e-3);
        assert!((volume.radius - 60.0).abs() < 1e-3);
        assert!(pose_of(&app, &parent).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-3));
    }

    #[test]
    fn escape_puts_a_grip_drag_back() {
        let (mut app, parent) = volume_scene(VolumeShape::Box);
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![
            button(0, true, 630.0, 360.0),
            PreviewInput::MouseMove {
                x: 700.0,
                y: 360.0,
                w: 960.0,
                h: 720.0,
            },
        ];
        app.update();
        app.world_mut().non_send_mut::<Engine>().preview_inputs = vec![PreviewInput::Key {
            code: "Escape".into(),
            down: true,
        }];
        app.update();
        let engine = app.world().non_send::<Engine>();
        let live = engine.actor(&parent).unwrap().components.volume().unwrap();
        assert_eq!(live.half_extents[0], 50.0);
        assert!(pose_of(&app, &parent).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-3));
    }

    #[test]
    fn snapping_rounds_to_the_nearest_step() {
        assert_eq!(snap(1.26, 0.5), 1.5);
        assert_eq!(snap(-0.74, 0.5), -0.5);
        assert_eq!(snap(3.3, 0.0), 3.3);
    }
}
