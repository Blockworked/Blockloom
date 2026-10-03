// ─── The API a script writes against ───────────────────────────────────────
//
// Everything above this line is the raw boundary; everything below is the
// crate a script says `use blockloom::*;` to get. Nothing here keeps global
// state: an entry point is handed its context and passes it along, so two
// actors running the same script never see each other's.

/// Which axis a reading or a movement is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn index(self) -> f64 {
        match self {
            Axis::X => 0.0,
            Axis::Y => 1.0,
            Axis::Z => 2.0,
        }
    }
}

/// The water surface at one point, as [`Actor::water_at`] reads it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WaterSample {
    /// The surface's height there.
    pub height: f32,
    pub normal: (f32, f32, f32),
    /// How fast the surface there moves, the current included: what a boat
    /// drifts with.
    pub velocity: (f32, f32, f32),
    /// 0-1: how pinched the crest there is, which is where foam gathers.
    pub foam: f32,
}

/// One thing a physics query found, as [`Actor::raycast`] and friends report
/// it. `distance` is along the query, `fraction` how far along (0-1).
#[derive(Clone, PartialEq, Debug)]
pub struct Hit {
    pub actor: String,
    pub body: String,
    pub collider: String,
    pub part: u32,
    pub point: (f32, f32, f32),
    pub normal: (f32, f32, f32),
    pub distance: f32,
    pub fraction: f32,
    pub started_inside: bool,
    pub trigger: bool,
}

/// What a character controller move did, as [`Actor::move_controller`] and
/// [`Actor::simple_move_controller`] report it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Moved {
    pub sides: bool,
    pub above: bool,
    pub below: bool,
    /// Whether the controller ended standing on something.
    pub grounded: bool,
    /// How far it really went.
    pub moved: (f32, f32, f32),
    /// How many obstacles it met.
    pub hits: usize,
}

/// This actor's particles, as [`Actor::particles`] reads them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Particles {
    pub alive: u32,
    /// This frame's events of each kind.
    pub spawned: u32,
    pub died: u32,
    pub collided: u32,
    /// Where the last of each happened; the actor's position before any.
    pub spawn_at: (f32, f32, f32),
    pub die_at: (f32, f32, f32),
    pub collide_at: (f32, f32, f32),
}

/// How an attached camera frames its actor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraView {
    Follow,
    FirstPerson,
    ThirdPerson,
}

impl CameraView {
    fn index(self) -> f64 {
        match self {
            CameraView::Follow => 0.0,
            CameraView::FirstPerson => 1.0,
            CameraView::ThirdPerson => 2.0,
        }
    }
}

/// Which mixing bus a sound routes through. `Master` scales everything;
/// `Music` and `Sfx` scale their own voices on top of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundBus {
    Master,
    Music,
    Sfx,
}

impl SoundBus {
    fn name(self) -> &'static str {
        match self {
            SoundBus::Master => "Master",
            SoundBus::Music => "Music",
            SoundBus::Sfx => "Sfx",
        }
    }
}

/// The actor this script is attached to.
///
/// Reads answer from the frame the runtime is part-way through, and writes are
/// queued as the same effects a block produces - so they land at the end of
/// the frame, in the order they were asked for, whether a block or a script
/// asked. Reading straight back after a write gives the old value, exactly as
/// it does in the block editor.
pub struct Actor {
    ctx: *mut std::ffi::c_void,
    api: *const HostApi,
}

impl Actor {
    /// # Safety
    /// Only the generated entry points call this, with what the host handed
    /// them; the borrow lasts no longer than the call.
    #[doc(hidden)]
    pub unsafe fn from_raw(ctx: *mut std::ffi::c_void, api: *const HostApi) -> Actor {
        // A browser host hands no table: its calls are this module's imports.
        #[cfg(target_arch = "wasm32")]
        let api = if api.is_null() {
            web::enter(ctx);
            &raw const web::HOST
        } else {
            api
        };
        Actor { ctx, api }
    }

    fn number(&self, what: u32, a: Str, b: Str, arg: f64) -> Option<f64> {
        let mut out = 0.0;
        let code = unsafe { ((*self.api).read_number)(self.ctx, what, a, b, arg, &mut out) };
        (code == OK).then_some(out)
    }

    fn text(&self, what: u32, a: Str, b: Str) -> Option<String> {
        let mut buffer = vec![0u8; 128];
        for _ in 0..2 {
            let mut needed = 0usize;
            let code = unsafe {
                ((*self.api).read_text)(
                    self.ctx,
                    what,
                    a,
                    b,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut needed,
                )
            };
            match code {
                OK => {
                    buffer.truncate(needed);
                    return String::from_utf8(buffer).ok();
                }
                // The host wrote the length it wanted; go round once with it.
                TOO_LONG => buffer = vec![0u8; needed],
                _ => return None,
            }
        }
        None
    }

    fn act(&self, what: u32, a: Str, b: Str, c: Str, n0: f64, n1: f64, n2: f64) {
        self.act_many(what, a, b, c, &[n0, n1, n2]);
    }

    fn act_many(&self, what: u32, a: Str, b: Str, c: Str, numbers: &[f64]) {
        unsafe { ((*self.api).act)(self.ctx, what, a, b, c, numbers.as_ptr(), numbers.len()) }
    }

    // ─── Reading the world ─────────────────────────────────────────────────

    pub fn position(&self, axis: Axis) -> f32 {
        self.number(READ_POSITION, Str::EMPTY, Str::EMPTY, axis.index())
            .unwrap_or(0.0) as f32
    }

    pub fn x(&self) -> f32 {
        self.position(Axis::X)
    }

    pub fn y(&self) -> f32 {
        self.position(Axis::Y)
    }

    pub fn z(&self) -> f32 {
        self.position(Axis::Z)
    }

    /// Another actor's position on an axis, by name or id.
    pub fn position_of(&self, actor: &str, axis: Axis) -> f32 {
        self.number(
            READ_POSITION_OF,
            Str::borrow(actor),
            Str::EMPTY,
            axis.index(),
        )
        .unwrap_or(0.0) as f32
    }

    /// Where this actor stands in its parent's frame - the world position
    /// itself when it hangs off nothing.
    pub fn local_position(&self, axis: Axis) -> f32 {
        self.number(READ_LOCAL_POSITION, Str::EMPTY, Str::EMPTY, axis.index())
            .unwrap_or(0.0) as f32
    }

    pub fn local_x(&self) -> f32 {
        self.local_position(Axis::X)
    }

    pub fn local_y(&self) -> f32 {
        self.local_position(Axis::Y)
    }

    pub fn local_z(&self) -> f32 {
        self.local_position(Axis::Z)
    }

    /// Another actor's place in its own parent's frame, by name or id.
    pub fn local_position_of(&self, actor: &str, axis: Axis) -> f32 {
        self.number(
            READ_LOCAL_POSITION_OF,
            Str::borrow(actor),
            Str::EMPTY,
            axis.index(),
        )
        .unwrap_or(0.0) as f32
    }

    /// Euler degrees about `axis`.
    pub fn rotation(&self, axis: Axis) -> f32 {
        self.number(READ_ROTATION, Str::EMPTY, Str::EMPTY, axis.index())
            .unwrap_or(0.0) as f32
    }

    pub fn scale(&self) -> f32 {
        self.number(READ_SCALE, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(1.0) as f32
    }

    pub fn is_visible(&self) -> bool {
        self.number(READ_VISIBLE, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(1.0)
            != 0.0
    }

    /// Seconds since the green flag.
    pub fn timer(&self) -> f64 {
        self.number(READ_TIMER, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
    }

    pub fn key_down(&self, key: &str) -> bool {
        self.number(READ_KEY_DOWN, Str::borrow(key), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// The pointer, in world units.
    pub fn mouse(&self) -> (f32, f32) {
        (
            self.number(READ_MOUSE, Str::EMPTY, Str::EMPTY, 0.0)
                .unwrap_or(0.0) as f32,
            self.number(READ_MOUSE, Str::EMPTY, Str::EMPTY, 1.0)
                .unwrap_or(0.0) as f32,
        )
    }

    pub fn mouse_down(&self) -> bool {
        self.number(READ_MOUSE_DOWN, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// Whether the pointer is grabbed and hidden for first-person play.
    pub fn mouse_locked(&self) -> bool {
        self.number(READ_MOUSE_LOCKED, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// The pointer's motion in pixels since the last frame: x grows as it
    /// moves right, y as it moves down. What a first-person camera wants.
    pub fn mouse_delta(&self) -> (f32, f32) {
        (
            self.number(READ_MOUSE_DELTA, Str::EMPTY, Str::EMPTY, 0.0)
                .unwrap_or(0.0) as f32,
            self.number(READ_MOUSE_DELTA, Str::EMPTY, Str::EMPTY, 1.0)
                .unwrap_or(0.0) as f32,
        )
    }

    /// Whether a mouse button is held: `left`, `right` or `middle`.
    pub fn mouse_button_down(&self, button: &str) -> bool {
        self.number(READ_MOUSE_BUTTON, Str::borrow(button), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// Whether a named input action is held right now.
    pub fn action_down(&self, action: &str) -> bool {
        self.number(READ_ACTION_DOWN, Str::borrow(action), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// True only on the frame the action went down.
    pub fn action_pressed(&self, action: &str) -> bool {
        self.number(READ_ACTION_PRESSED, Str::borrow(action), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// True only on the frame the action went up.
    pub fn action_released(&self, action: &str) -> bool {
        self.number(READ_ACTION_RELEASED, Str::borrow(action), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// The strongest binding's analog value: 0/1 for buttons, -1..1 for a
    /// whole stick axis, 0..1 for a directed half.
    pub fn action_value(&self, action: &str) -> f32 {
        self.number(READ_ACTION_VALUE, Str::borrow(action), Str::EMPTY, 0.0)
            .unwrap_or(0.0) as f32
    }

    /// How many fingers are down right now.
    pub fn touch_count(&self) -> usize {
        self.number(READ_TOUCH_COUNT, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            .max(0.0) as usize
    }

    /// The 1-based touch point's world position. Out of range reads as zero.
    pub fn touch(&self, index: usize) -> (f32, f32) {
        (
            self.number(READ_TOUCH, Str::EMPTY, Str::EMPTY, index as f64 * 2.0)
                .unwrap_or(0.0) as f32,
            self.number(READ_TOUCH, Str::EMPTY, Str::EMPTY, index as f64 * 2.0 + 1.0)
                .unwrap_or(0.0) as f32,
        )
    }

    /// Whether at least one gamepad is connected.
    pub fn gamepad_connected(&self) -> bool {
        self.number(READ_GAMEPAD_CONNECTED, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// A live stick or trigger value, -1..1. Unknown names read as zero.
    pub fn gamepad_axis(&self, axis: &str) -> f32 {
        self.number(READ_GAMEPAD_AXIS, Str::borrow(axis), Str::EMPTY, 0.0)
            .unwrap_or(0.0) as f32
    }

    /// Whether the named pad button is held. Most games read this through
    /// an action instead, so remapping keeps working.
    pub fn gamepad_button_down(&self, button: &str) -> bool {
        self.number(READ_GAMEPAD_BUTTON, Str::borrow(button), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    pub fn touching(&self, actor: &str) -> bool {
        self.number(READ_TOUCHING, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    pub fn touching_anything(&self) -> bool {
        self.touching("")
    }

    /// Whether an actor's collider only senses overlap. Empty names this one.
    pub fn is_trigger(&self, actor: &str) -> bool {
        self.number(READ_IS_TRIGGER, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    pub fn am_trigger(&self) -> bool {
        self.is_trigger("")
    }

    /// Which layer an actor lives on, 1-8. Empty names this one.
    pub fn collision_layer(&self, actor: &str) -> u8 {
        self.number(READ_COLLISION_LAYER, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(1.0)
            .round()
            .clamp(1.0, 8.0) as u8
    }

    pub fn my_collision_layer(&self) -> u8 {
        self.collision_layer("")
    }

    /// Asks the physics world, as this actor, and files the answer for
    /// [`Actor::hit_count`], [`Actor::hit_number`] and [`Actor::hit_text`].
    /// `kind` is a query name (`"ray"`, `"rays"`, `"ball cast"`, `"ball
    /// overlap"`, `"box cast"`, `"box overlap"`, `"capsule cast"`, `"capsule
    /// overlap"`, `"closest"`), `triggers` is `"UseGlobal"`, `"Ignore"` or
    /// `"Include"`, `layers` a mask (0 for every layer) and `numbers` what
    /// the kind takes. Returns how many hits there were.
    pub fn query(&self, kind: &str, triggers: &str, layers: u32, numbers: &[f64]) -> usize {
        let mut all = Vec::with_capacity(numbers.len() + 1);
        all.push(layers as f64);
        all.extend_from_slice(numbers);
        self.act_many(
            ACT_PHYSICS_QUERY,
            Str::borrow(kind),
            Str::borrow(triggers),
            Str::EMPTY,
            &all,
        );
        self.hit_count()
    }

    /// How many hits the last query found.
    pub fn hit_count(&self) -> usize {
        self.hit_number(1, "count") as usize
    }

    /// A number from the `index`th (from 1) hit of the last query: `x`, `y`,
    /// `z`, `normal x`, `normal y`, `normal z`, `distance`, `fraction`,
    /// `part`, `started inside`, `is trigger`; or `count`, `overflowed` and
    /// `tick` for the query itself. Zero when there is no such hit.
    pub fn hit_number(&self, index: usize, field: &str) -> f64 {
        self.number(READ_QUERY, Str::borrow(field), Str::EMPTY, index as f64)
            .unwrap_or(0.0)
    }

    /// Words from a hit: `actor` (its name), `actor id`, `body`, `collider`;
    /// or `error` for the query itself. Empty when there is none.
    pub fn hit_text(&self, index: usize, field: &str) -> String {
        self.text(
            TEXT_QUERY,
            Str::borrow(field),
            Str::borrow(&index.to_string()),
        )
        .unwrap_or_default()
    }

    /// The nearest thing a segment crosses, as this actor.
    pub fn raycast(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> Option<Hit> {
        let numbers = [from.0, from.1, from.2, to.0, to.1, to.2].map(f64::from);
        (self.query("ray", "UseGlobal", 0, &numbers) > 0).then(|| self.hit(1))
    }

    /// Everything a segment crosses, nearest first.
    pub fn raycast_all(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> Vec<Hit> {
        let numbers = [from.0, from.1, from.2, to.0, to.1, to.2].map(f64::from);
        let count = self.query("rays", "UseGlobal", 0, &numbers);
        (1..=count).map(|index| self.hit(index)).collect()
    }

    /// The first thing a ball meets sweeping along a segment.
    pub fn cast_ball(
        &self,
        radius: f32,
        from: (f32, f32, f32),
        to: (f32, f32, f32),
    ) -> Option<Hit> {
        let numbers = [radius, from.0, from.1, from.2, to.0, to.1, to.2].map(f64::from);
        (self.query("ball cast", "UseGlobal", 0, &numbers) > 0).then(|| self.hit(1))
    }

    /// Everything a ball at a point overlaps, nearest first.
    pub fn overlap_ball(&self, at: (f32, f32, f32), radius: f32) -> Vec<Hit> {
        let numbers = [radius, at.0, at.1, at.2].map(f64::from);
        let count = self.query("ball overlap", "UseGlobal", 0, &numbers);
        (1..=count).map(|index| self.hit(index)).collect()
    }

    /// The collider nearest a point within `range`.
    pub fn closest(&self, at: (f32, f32, f32), range: f32) -> Option<Hit> {
        let numbers = [range, at.0, at.1, at.2].map(f64::from);
        (self.query("closest", "UseGlobal", 0, &numbers) > 0).then(|| self.hit(1))
    }

    /// The `index`th (from 1) hit of the last query.
    pub fn hit(&self, index: usize) -> Hit {
        let n = |field: &str| self.hit_number(index, field);
        Hit {
            actor: self.hit_text(index, "actor"),
            body: self.hit_text(index, "body"),
            collider: self.hit_text(index, "collider"),
            part: n("part") as u32,
            point: (n("x") as f32, n("y") as f32, n("z") as f32),
            normal: (n("normal x") as f32, n("normal y") as f32, n("normal z") as f32),
            distance: n("distance") as f32,
            fraction: n("fraction") as f32,
            started_inside: n("started inside") != 0.0,
            trigger: n("is trigger") != 0.0,
        }
    }

    /// The first body a segment hits, by name, or `None`. Bodies only; this
    /// actor itself is skipped; layers filter by this actor's own mask.
    pub fn ray_hit(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> Option<String> {
        self.text(
            TEXT_RAY_HIT,
            Str::borrow(&format!("{} {} {}", from.0, from.1, from.2)),
            Str::borrow(&format!("{} {} {}", to.0, to.1, to.2)),
        )
    }

    /// How far along the segment the first hit sits, or `None` for nothing.
    pub fn ray_distance(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> Option<f32> {
        self.number(
            READ_RAY_DISTANCE,
            Str::borrow(&format!("{} {} {}", from.0, from.1, from.2)),
            Str::borrow(&format!("{} {} {}", to.0, to.1, to.2)),
            0.0,
        )
        .map(|distance| distance as f32)
    }

    /// The nearest body a ball overlaps, by name, or `None`. A ground check
    /// is a small ball underfoot.
    pub fn circle_hit(&self, at: (f32, f32, f32), radius: f32) -> Option<String> {
        self.text(
            TEXT_CIRCLE_HIT,
            Str::borrow(&format!("{} {} {}", at.0, at.1, at.2)),
            Str::borrow(&radius.to_string()),
        )
    }

    /// Whether the collider pushes back (solid) or only senses (trigger).
    pub fn set_trigger(&self, trigger: bool) {
        self.act(
            ACT_SET_TRIGGER,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if trigger { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// Which layer the actor lives on, 1-8.
    pub fn set_collision_layer(&self, layer: u8) {
        self.act(
            ACT_SET_COLLISION_LAYER,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            layer.clamp(1, 8) as f64,
            0.0,
            0.0,
        );
    }

    /// Bitmask of the layers the actor pairs with, 0-255.
    pub fn set_collision_mask(&self, mask: u8) {
        self.act(
            ACT_SET_COLLISION_MASK,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            mask as f64,
            0.0,
            0.0,
        );
    }

    pub fn distance_to(&self, actor: &str) -> f32 {
        self.number(READ_DISTANCE_TO, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0) as f32
    }

    // ─── Components ────────────────────────────────────────────────────────

    /// Whether the actor is carrying that component right now - which a
    /// [`Actor::detach`] earlier in the run may have changed.
    pub fn has(&self, component: &str) -> bool {
        self.number(READ_HAS_COMPONENT, Str::borrow(component), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// One field of one of the actor's custom components, or `None` when the
    /// actor has no such component or it has no such field.
    pub fn field(&self, component: &str, field: &str) -> Option<f64> {
        self.number(READ_FIELD, Str::borrow(component), Str::borrow(field), 0.0)
    }

    pub fn field_or(&self, component: &str, field: &str, fallback: f64) -> f64 {
        self.field(component, field).unwrap_or(fallback)
    }

    pub fn text_field(&self, component: &str, field: &str) -> Option<String> {
        self.text(TEXT_FIELD, Str::borrow(component), Str::borrow(field))
    }

    pub fn set_field(&self, component: &str, field: &str, value: f64) {
        self.act(
            ACT_SET_FIELD,
            Str::borrow(component),
            Str::borrow(field),
            Str::EMPTY,
            value,
            0.0,
            0.0,
        );
    }

    pub fn set_text_field(&self, component: &str, field: &str, value: &str) {
        self.act(
            ACT_SET_FIELD_TEXT,
            Str::borrow(component),
            Str::borrow(field),
            Str::borrow(value),
            0.0,
            0.0,
            0.0,
        );
    }

    /// Gives the actor a component. One it was authored with comes back as
    /// the editor left it; anything else arrives with its defaults. Attaching
    /// what is already there changes nothing.
    pub fn attach(&self, component: &str) {
        self.act(
            ACT_ATTACH,
            Str::borrow(component),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Takes a component away. `Place` can't go - there would be nowhere for
    /// the actor to be.
    pub fn detach(&self, component: &str) {
        self.act(
            ACT_DETACH,
            Str::borrow(component),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    pub fn set_camera_view(&self, view: CameraView) {
        self.act(
            ACT_SET_CAMERA_VIEW,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            view.index(),
            0.0,
            0.0,
        );
    }

    pub fn name(&self) -> String {
        self.text(TEXT_ACTOR_NAME, Str::EMPTY, Str::EMPTY)
            .unwrap_or_default()
    }

    /// This actor's own id - what every call below takes, and the one way to
    /// name a particular clone, since clones share their template's name.
    pub fn id(&self) -> String {
        self.text(TEXT_ACTOR_ID, Str::EMPTY, Str::EMPTY)
            .unwrap_or_default()
    }

    // ─── Actors and the hierarchy ──────────────────────────────────────────

    /// True for an actor a clone made rather than one the editor authored.
    pub fn is_clone(&self) -> bool {
        self.number(READ_IS_CLONE, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// How many actors answer to `name` right now, clones included. An empty
    /// name counts every actor in the world.
    pub fn actor_count(&self, name: &str) -> usize {
        self.number(READ_ACTOR_COUNT, Str::borrow(name), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            .max(0.0) as usize
    }

    /// The id of the actor this one hangs off, or `None` when it is free.
    pub fn parent(&self) -> Option<String> {
        self.text(TEXT_PARENT, Str::EMPTY, Str::EMPTY)
            .filter(|id| !id.is_empty())
    }

    /// The id of the last actor or clone this one made, or `None` before it
    /// has made any. Hand it to [`Actor::set_parent`] or [`Actor::delete`].
    pub fn new_actor(&self) -> Option<String> {
        self.text(TEXT_NEW_ACTOR, Str::EMPTY, Str::EMPTY)
            .filter(|id| !id.is_empty())
    }

    /// Hangs this actor off another, by id or name, so the two move
    /// together. When the actor carries an authored offset it is placed at
    /// it - that far from its new parent, in the parent's own frame - and
    /// otherwise it keeps the place it is standing in.
    pub fn set_parent(&self, actor: &str) {
        self.act(
            ACT_SET_PARENT,
            Str::borrow(actor),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Takes this actor off whatever it was hanging from.
    pub fn clear_parent(&self) {
        self.set_parent("");
    }

    /// Makes a running copy of an actor, by id or name - its components as
    /// they stand and its whole canvas, whose `when I start as a clone`
    /// strands run next step. An empty name clones this actor.
    /// [`Actor::new_actor`] answers with the copy's id afterwards.
    pub fn create_clone(&self, actor: &str) {
        self.act(
            ACT_CREATE_CLONE,
            Str::borrow(actor),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    pub fn clone_myself(&self) {
        self.create_clone("");
    }

    /// Makes a brand-new actor with no blocks of its own, for this run only.
    pub fn create_actor(&self, name: &str, x: f32, y: f32, z: f32) {
        self.act(
            ACT_CREATE_ACTOR,
            Str::borrow(name),
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// Takes an actor out of the world for the rest of the run, by id or
    /// name. The saved document is untouched.
    pub fn delete(&self, actor: &str) {
        self.act(
            ACT_DELETE_ACTOR,
            Str::borrow(actor),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Deletes this actor. Its blocks stop; this call returns as usual, so
    /// there is nothing to do afterwards.
    pub fn delete_myself(&self) {
        self.delete("");
    }

    // ─── Changing the world ────────────────────────────────────────────────

    /// Forward along the actor's own facing.
    pub fn move_forward(&self, steps: f32) {
        self.act(
            ACT_MOVE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            steps as f64,
            0.0,
            0.0,
        );
    }

    pub fn go_to(&self, x: f32, y: f32, z: f32) {
        self.act(
            ACT_GO_TO,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// One step toward a position along the navmesh, at `speed` units per
    /// second. Steers around static obstacles; with no route it steps
    /// straight at the target instead.
    pub fn navigate_to(&self, x: f32, y: f32, z: f32, speed: f32) {
        self.act_many(
            ACT_NAVIGATE_TO,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            &[x as f64, y as f64, z as f64, speed as f64],
        );
    }

    pub fn change_position(&self, axis: Axis, by: f32) {
        self.act(
            ACT_CHANGE_POSITION,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            axis.index(),
            by as f64,
            0.0,
        );
    }

    pub fn turn(&self, axis: Axis, degrees: f32) {
        self.act(
            ACT_TURN,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            axis.index(),
            degrees as f64,
            0.0,
        );
    }

    pub fn set_rotation(&self, axis: Axis, degrees: f32) {
        self.act(
            ACT_SET_ROTATION,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            axis.index(),
            degrees as f64,
            0.0,
        );
    }

    /// Faces another actor by name, or `"mouse"`.
    pub fn point_towards(&self, target: &str) {
        self.act(
            ACT_POINT_TOWARDS,
            Str::borrow(target),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    pub fn set_scale(&self, factor: f32) {
        self.act(
            ACT_SET_SCALE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            factor as f64,
            0.0,
            0.0,
        );
    }

    /// Overrides a rendering setting for the rest of this run.
    pub fn set_render_setting(&self, setting: &str, value: &str) {
        self.act(
            ACT_SET_RENDER_SETTING,
            Str::borrow(setting), Str::borrow(value), Str::EMPTY,
            0.0, 0.0, 0.0,
        );
    }
    pub fn set_quality(&self, quality: &str) {
        self.set_render_setting("Quality", quality);
    }
    pub fn set_resolution_scale(&self, scale: f32) {
        self.set_render_setting("ResolutionScale", &scale.to_string());
    }
    pub fn set_upscaler(&self, upscaler: &str) {
        self.set_render_setting("Upscaler", upscaler);
    }
    pub fn set_dlss_mode(&self, mode: &str) {
        self.set_render_setting("DlssMode", mode);
    }

    /// The camera's exposure in EV100 for the rest of the run: lower is
    /// brighter. Outranks auto-exposure and the project's own value.
    pub fn set_exposure(&self, ev: f32) {
        self.act(
            ACT_SET_EXPOSURE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            ev as f64,
            0.0,
            0.0,
        );
    }

    /// This actor's light, in lumens. Nothing happens without a Light.
    pub fn set_light_intensity(&self, lumens: f32) {
        self.act(
            ACT_SET_LIGHT_INTENSITY,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            lumens as f64,
            0.0,
            0.0,
        );
    }

    /// Tweens the size towards `factor` over `seconds`, eased like the
    /// block. `easing` names a [`TweenEasing`]: "Linear", "EaseIn",
    /// "EaseOut", "EaseInOut", "Bounce" or "Elastic".
    pub fn tween_scale(&self, factor: f32, seconds: f32, easing: &str) {
        self.act(
            ACT_TWEEN_SCALE,
            Str::EMPTY,
            Str::borrow(easing),
            Str::EMPTY,
            factor as f64,
            seconds as f64,
            0.0,
        );
    }

    /// Tweens one axis towards `degrees` over `seconds`, eased.
    pub fn tween_rotation(&self, axis: Axis, degrees: f32, seconds: f32, easing: &str) {
        self.act(
            ACT_TWEEN_ROTATION,
            Str::EMPTY,
            Str::borrow(easing),
            Str::EMPTY,
            axis.index(),
            degrees as f64,
            seconds as f64,
        );
    }

    /// Tweens the tint towards a `#RRGGBB` color over `seconds`, eased.
    /// No-op on an image actor, like `set color`.
    pub fn tween_color(&self, color: &str, seconds: f32, easing: &str) {
        self.act(
            ACT_TWEEN_COLOR,
            Str::borrow(color),
            Str::borrow(easing),
            Str::EMPTY,
            seconds as f64,
            0.0,
            0.0,
        );
    }

    /// Stops every tween on this actor where it stands: glides included.
    pub fn stop_tweens(&self) {
        self.act(
            ACT_STOP_TWEENS,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Whether any tween is still moving this actor.
    pub fn is_tweening(&self) -> bool {
        self.number(READ_IS_TWEENING, Str::EMPTY, Str::EMPTY, 0.0)
            .is_some_and(|tweening| tweening != 0.0)
    }

    /// Plays the named flipbook clip at `speed` (1 is as authored).
    pub fn play_animation(&self, clip: &str, speed: f32) {
        self.act(
            ACT_PLAY_ANIMATION,
            Str::borrow(clip),
            Str::EMPTY,
            Str::EMPTY,
            speed as f64,
            0.0,
            0.0,
        );
    }

    /// Stops the animation player, keeping the frame it shows.
    pub fn stop_animation(&self) {
        self.act(
            ACT_STOP_ANIMATION,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Retunes the playing clip's speed. 1 is as authored, 0 freezes.
    pub fn set_animation_speed(&self, speed: f32) {
        self.act(
            ACT_SET_ANIMATION_SPEED,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            speed as f64,
            0.0,
            0.0,
        );
    }

    /// Fires a named trigger into the animation state machine.
    pub fn fire_animation_trigger(&self, name: &str) {
        self.act(
            ACT_FIRE_ANIMATION_TRIGGER,
            Str::borrow(name),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Shows `attachment` in one of the rig's slots; empty hides it.
    pub fn set_rig_slot(&self, slot: &str, attachment: &str) {
        self.act(
            ACT_SET_RIG_SLOT,
            Str::borrow(slot),
            Str::borrow(attachment),
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Tints one rig slot, `#RRGGBB`, over its authored color.
    pub fn set_slot_tint(&self, slot: &str, color: &str) {
        self.act(
            ACT_SET_SLOT_TINT,
            Str::borrow(slot),
            Str::borrow(color),
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Points a rig IK constraint at `(x, y)`, relative to this actor.
    pub fn set_ik_target(&self, constraint: &str, x: f32, y: f32) {
        self.act(
            ACT_SET_IK_TARGET,
            Str::borrow(constraint),
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            0.0,
        );
    }

    /// Changes a 2D sprite dial: `FlipX`, `FlipY`, `Order`, `YSort`,
    /// `Palette` or `OutlineWidth`. Switches read nonzero as on.
    pub fn set_sprite_dial(&self, dial: &str, value: f32) {
        self.act(
            ACT_SET_SPRITE_DIAL,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// Bursts `count` particles out of this actor's emitter now.
    pub fn burst_particles(&self, count: u32) {
        self.act(
            ACT_BURST_PARTICLES,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            count as f64,
            0.0,
            0.0,
        );
    }

    /// One of this actor's emitter dials for the rest of the run: `"rate"`,
    /// `"lifetime"`, `"speed"`, `"spread"`, `"gravity"`, `"size start"`,
    /// `"size end"` or `"max"`.
    pub fn set_emitter(&self, dial: &str, value: f32) {
        self.act(
            ACT_SET_EMITTER_DIAL,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// Starts or stops this actor's emitter. Live particles finish either way.
    pub fn set_emitter_playing(&self, playing: bool) {
        self.act(
            ACT_SET_EMITTER_PLAYING,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if playing { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// This actor's particles as of the last frame drawn.
    pub fn particles(&self) -> Particles {
        let read = |what: &str| {
            self.number(READ_PARTICLES, Str::borrow(what), Str::EMPTY, 0.0)
                .unwrap_or(0.0) as f32
        };
        let at = |event: &str| {
            (
                read(&format!("{event} x")),
                read(&format!("{event} y")),
                read(&format!("{event} z")),
            )
        };
        Particles {
            alive: read("alive") as u32,
            spawned: read("spawn") as u32,
            died: read("die") as u32,
            collided: read("collide") as u32,
            spawn_at: at("spawn"),
            die_at: at("die"),
            collide_at: at("collide"),
        }
    }

    /// The clip the animation player is holding, or `None` for none.
    pub fn current_clip(&self) -> Option<String> {
        self.text(TEXT_CURRENT_CLIP, Str::EMPTY, Str::EMPTY)
    }

    /// The 1-based flipbook frame showing right now. Zero with no clip.
    pub fn current_frame(&self) -> usize {
        self.number(READ_ANIM_FRAME, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0) as usize
    }

    /// Whether the player's clip is still advancing.
    pub fn is_animation_playing(&self) -> bool {
        self.number(READ_ANIM_PLAYING, Str::EMPTY, Str::EMPTY, 0.0)
            .is_some_and(|playing| playing != 0.0)
    }

    /// How strongly this actor's surface glows, times its emissive tint.
    pub fn set_emissive_strength(&self, strength: f32) {
        self.act(
            ACT_SET_EMISSIVE_STRENGTH,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            strength as f64,
            0.0,
            0.0,
        );
    }

    /// HDR output on or off for the rest of the run, where the display
    /// offers it.
    pub fn set_hdr_output(&self, enabled: bool) {
        self.act(
            ACT_SET_HDR_OUTPUT,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if enabled { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// The display's peak brightness in nits for the rest of the run.
    pub fn set_peak_brightness(&self, nits: f32) {
        self.act(
            ACT_SET_PEAK_BRIGHTNESS,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            nits as f64,
            0.0,
            0.0,
        );
    }

    /// Re-captures every light probe from where it stands now. Lasts the
    /// run; nothing is written to disk.
    pub fn capture_probes(&self) {
        self.act(
            ACT_CAPTURE_PROBES,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// How far the sun's shadows reach, in metres, for the rest of the run.
    pub fn set_shadow_distance(&self, metres: f32) {
        self.act(
            ACT_SET_SHADOW_DISTANCE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            metres as f64,
            0.0,
            0.0,
        );
    }

    /// Whether this actor's light casts shadows, for the rest of the run.
    pub fn set_light_shadows(&self, enabled: bool) {
        self.act(
            ACT_SET_LIGHT_SHADOWS,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if enabled { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// Ray-traced lighting on or off for the rest of the run, where the GPU
    /// can trace rays.
    pub fn set_ray_tracing(&self, enabled: bool) {
        self.act(
            ACT_SET_RAY_TRACING,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if enabled { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// Most bounces a traced light path takes, for the rest of the run.
    pub fn set_gi_bounces(&self, bounces: u32) {
        self.act(
            ACT_SET_GI_BOUNCES,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            bounces as f64,
            0.0,
            0.0,
        );
    }

    /// Light samples per pixel when tracing, for the rest of the run.
    pub fn set_gi_samples(&self, samples: u32) {
        self.act(
            ACT_SET_GI_SAMPLES,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            samples as f64,
            0.0,
            0.0,
        );
    }

    /// Height fog's extinction per metre at its base, for the rest of the
    /// run; volumetric fog and beams scale with it. 0 clears the air.
    pub fn set_fog_density(&self, density: f32) {
        self.act(
            ACT_SET_FOG_DENSITY,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            density as f64,
            0.0,
            0.0,
        );
    }

    /// The aurora's KP index, 0-9, for the rest of the run. 0 puts it out.
    pub fn set_aurora(&self, kp: f32) {
        self.act(
            ACT_SET_AURORA,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            kp as f64,
            0.0,
            0.0,
        );
    }

    /// A lightning strike landing at `x, y, z`: a flash, a pulse of the sky
    /// and thunder late by the distance.
    pub fn strike_lightning(&self, x: f32, y: f32, z: f32) {
        self.act(
            ACT_STRIKE_LIGHTNING,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// Strikes a minute the storm throws, for the rest of the run. 0 calms it.
    pub fn set_lightning_rate(&self, per_minute: f32) {
        self.act(
            ACT_SET_LIGHTNING_RATE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            per_minute as f64,
            0.0,
            0.0,
        );
    }

    /// One of the wind's dials for the rest of the run: `"direction"` in
    /// degrees clockwise from north, `"speed"`, `"gust"` or `"storm"` 0-1.
    pub fn set_wind(&self, dial: &str, value: f32) {
        self.act(
            ACT_SET_WIND,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// One of cloud layer `layer`'s dials (from 1) for the rest of the run:
    /// `"coverage"`, `"opacity"`, `"contrast"`, `"altitude"` or `"spin"`.
    pub fn set_cloud_layer(&self, layer: u32, dial: &str, value: f32) {
        self.act(
            ACT_SET_CLOUD_LAYER,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            layer as f64,
            value as f64,
            0.0,
        );
    }

    /// A water dial for the rest of the run: `"level"` (the surface's height
    /// at rest), `"chop"` 0-1 or `"foam"` 0-2. Moves this actor's own water
    /// when it has some, and every body's otherwise.
    pub fn set_water(&self, dial: &str, value: f32) {
        self.act(
            ACT_SET_WATER,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// Paints one tilemap cell at a world point for the rest of the run: a
    /// sheet index, or -1 to erase. `map` names the tilemap actor; empty
    /// means this actor if it is one, else whichever map covers the point.
    pub fn paint_tile(&self, map: &str, tile: i32, x: f32, y: f32) {
        self.act(
            ACT_PAINT_TILE,
            Str::borrow(map),
            Str::EMPTY,
            Str::EMPTY,
            tile as f64,
            x as f64,
            y as f64,
        );
    }

    /// [`Self::paint_tile`] at a 3D point, read on the map's own face.
    pub fn paint_tile_at(&self, map: &str, tile: i32, x: f32, y: f32, z: f32) {
        self.act_many(
            ACT_PAINT_TILE,
            Str::borrow(map),
            Str::EMPTY,
            Str::EMPTY,
            &[tile as f64, x as f64, y as f64, z as f64],
        );
    }

    /// [`Self::tile_at`] at a 3D point, read on each map's own face.
    pub fn tile_at_xyz(&self, map: &str, x: f32, y: f32, z: f32) -> Option<i32> {
        let at = format!("{x} {y} {z}");
        self.number(READ_TILE_AT, Str::borrow(&at), Str::borrow(map), 0.0)
            .map(|tile| tile as i32)
    }

    /// The sheet index at a world point, -1 for an empty cell or no map
    /// there. `map` names one tilemap, or empty for any; `None` when no map
    /// answers to that name.
    pub fn tile_at(&self, map: &str, x: f32, y: f32) -> Option<i32> {
        let at = format!("{x} {y}");
        self.number(READ_TILE_AT, Str::borrow(&at), Str::borrow(map), 0.0)
            .map(|tile| tile as i32)
    }

    /// A parallax layer's scroll factor (0-2) for the rest of the run, on
    /// `"both"` axes, `"x"` or `"y"`.
    pub fn set_parallax(&self, layer: &str, axis: &str, value: f32) {
        self.act(
            ACT_SET_PARALLAX,
            Str::borrow(layer),
            Str::borrow(axis),
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// The name of the smallest room an actor stands in, or `None` outside
    /// every room. Empty names this one.
    pub fn room_containing(&self, actor: &str) -> Option<String> {
        self.text(TEXT_ROOM, Str::borrow(actor), Str::EMPTY)
    }

    /// The room this actor walked into on the last tick, or `None`: what
    /// [`Event::EnteredRoom`] also says, for a script that polls from `tick`.
    /// Starting a run inside one isn't entering.
    pub fn entered_room(&self) -> Option<String> {
        self.text(TEXT_ENTERED_ROOM, Str::EMPTY, Str::EMPTY)
    }

    /// Loads another scene by name and continues the run there: globals and
    /// save data carry over, actor locals do not. `transition` is `none`,
    /// `fade`, `wipe` or `circle`; anything else reads as `none`.
    pub fn switch_scene(&self, scene: &str, transition: &str) {
        self.act(
            ACT_SWITCH_SCENE,
            Str::borrow(scene),
            Str::borrow(transition),
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// The scene running right now, by name. What `current scene` reports.
    pub fn current_scene(&self) -> Option<String> {
        self.text(TEXT_CURRENT_SCENE, Str::EMPTY, Str::EMPTY)
    }

    /// The weather preset the air is in, by name. What `current weather`
    /// reports. Empty before the first blend lands.
    pub fn current_weather(&self) -> Option<String> {
        self.text(TEXT_CURRENT_WEATHER, Str::EMPTY, Str::EMPTY)
    }

    /// The cutscene playing right now, by name. Empty with none playing,
    /// which is what `is cutscene playing?` reads.
    pub fn cutscene_name(&self) -> Option<String> {
        self.text(TEXT_CUTSCENE_NAME, Str::EMPTY, Str::EMPTY)
    }

    /// Seconds into the playing cutscene. What `cutscene time` reports.
    pub fn cutscene_time(&self) -> f32 {
        self.number(READ_CUTSCENE_TIME, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0) as f32
    }

    /// Hours, 0-24, as of this fixed tick. What `time of day` reports.
    pub fn time_of_day(&self) -> f32 {
        self.atmosphere("time of day").unwrap_or(12.0) as f32
    }

    /// Degrees above the horizon, as of this fixed tick. What
    /// `sun elevation` reports.
    pub fn sun_elevation(&self) -> f32 {
        self.atmosphere("sun elevation").unwrap_or(0.0) as f32
    }

    /// The clock in hours, 0-24, for the rest of the run. Moves the
    /// director's sun and every track reading from it.
    pub fn set_time_of_day(&self, hours: f32) {
        self.act(
            ACT_SET_TIME_OF_DAY,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            hours as f64,
            0.0,
            0.0,
        );
    }

    /// Moves the director's clock by hours (negative rewinds), for the rest
    /// of the run.
    pub fn advance_time(&self, hours: f32) {
        self.act(
            ACT_ADVANCE_TIME,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            hours as f64,
            0.0,
            0.0,
        );
    }

    /// Rain or snow intensity, 0-1, for the rest of the run: `"rain"` or
    /// `"snow"`.
    pub fn set_precipitation(&self, kind: &str, value: f32) {
        self.act(
            ACT_SET_PRECIPITATION,
            Str::borrow(kind),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// Blends the weather towards a preset (Clear, Overcast, Storm, Sunset,
    /// Night, or one the project authored) over seconds. A new blend begun
    /// mid-blend carries on from where the air is.
    pub fn blend_weather(&self, weather: &str, seconds: f32) {
        self.act(
            ACT_BLEND_WEATHER,
            Str::borrow(weather),
            Str::EMPTY,
            Str::EMPTY,
            seconds as f64,
            0.0,
            0.0,
        );
    }

    /// Plays the named cutscene reel on the wall clock. The strand carries
    /// on; `when cutscene ends` runs when the reel does.
    pub fn play_cutscene(&self, cutscene: &str) {
        self.act(
            ACT_PLAY_CUTSCENE,
            Str::borrow(cutscene),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Jumps the playing cutscene to its end marker. Quiet with none.
    pub fn skip_cutscene(&self) {
        self.act(
            ACT_SKIP_CUTSCENE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Kicks the camera trauma 0-1 higher. Adds to what is shaking.
    pub fn shake_camera(&self, amount: f32) {
        self.act(
            ACT_CAMERA_SHAKE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            amount as f64,
            0.0,
            0.0,
        );
    }

    /// Scales world time for the rest of the run: 1 is normal speed.
    pub fn set_time_scale(&self, scale: f32) {
        self.act(
            ACT_SET_TIME_SCALE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            scale as f64,
            0.0,
            0.0,
        );
    }

    /// Freezes world strands for `frames` render frames.
    pub fn hitstop(&self, frames: f32) {
        self.act(
            ACT_HITSTOP,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            frames as f64,
            0.0,
            0.0,
        );
    }

    /// Shows the letterbox bars for nonzero `on`, hides them for zero.
    pub fn set_letterbox(&self, on: f32) {
        self.act(
            ACT_SET_LETTERBOX,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            on as f64,
            0.0,
            0.0,
        );
    }

    /// Fades the screen to `black` or `white`, or clears it for `none`.
    pub fn fade_screen(&self, color: &str) {
        self.act(
            ACT_FADE_SCREEN,
            Str::borrow(color),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Runs a plugin block's command as the block would. `args` follow the
    /// block's slots in order; a whole number goes over as an integer.
    pub fn plugin_call(&self, plugin: &str, block: &str, args: &[PluginArg]) {
        let json = plugin_args_json(args);
        self.act(
            ACT_PLUGIN_CALL,
            Str::borrow(plugin),
            Str::borrow(block),
            Str::borrow(&json),
            0.0,
            0.0,
            0.0,
        );
    }

    /// A plugin reporter's answer as a number, or `None` when the game isn't
    /// running, the plugin has no such reading, or it doesn't read as one.
    pub fn plugin_number(&self, plugin: &str, block: &str, args: &[PluginArg]) -> Option<f64> {
        let asked = format!("{block}{PLUGIN_SEP}{}", plugin_args_json(args));
        self.number(READ_PLUGIN, Str::borrow(plugin), Str::borrow(&asked), 0.0)
    }

    /// The same reporter's answer as text.
    pub fn plugin_text(&self, plugin: &str, block: &str, args: &[PluginArg]) -> Option<String> {
        let asked = format!("{block}{PLUGIN_SEP}{}", plugin_args_json(args));
        self.text(TEXT_PLUGIN, Str::borrow(plugin), Str::borrow(&asked))
    }

    /// Every scene's name, in project order. What `scene names` reports.
    pub fn scene_names(&self) -> Option<String> {
        self.text(TEXT_SCENE_NAMES, Str::EMPTY, Str::EMPTY)
    }

    /// The water surface over (x, z) as of this fixed tick (z means nothing
    /// in 2D), or `None` over dry land. The highest where bodies overlap.
    pub fn water_at(&self, x: f32, z: f32) -> Option<WaterSample> {
        let at = format!("{x} {z}");
        let read = |what: &str| {
            self.number(READ_WATER, Str::borrow(&at), Str::borrow(what), 0.0)
                .map(|value| value as f32)
        };
        Some(WaterSample {
            height: read("height")?,
            normal: (read("normal x")?, read("normal y")?, read("normal z")?),
            velocity: (read("velocity x")?, read("velocity y")?, read("velocity z")?),
            foam: read("foam")?,
        })
    }

    /// Whether an actor is below a water surface and above its bottom.
    /// Empty names this one.
    pub fn is_underwater(&self, actor: &str) -> bool {
        self.number(READ_UNDERWATER, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// One of the volumetric clouds' dials for the rest of the run:
    /// `"coverage"` 0-1, `"density"` 0-10 or `"type"` 0-1 (stratus to cumulus).
    pub fn set_clouds(&self, dial: &str, value: f32) {
        self.act(
            ACT_SET_CLOUDS,
            Str::borrow(dial),
            Str::EMPTY,
            Str::EMPTY,
            value as f64,
            0.0,
            0.0,
        );
    }

    /// Extra cloud drift, world units per second, for the rest of the run.
    pub fn set_cloud_drift(&self, x: f32, y: f32, z: f32) {
        self.act(
            ACT_SET_CLOUD_DRIFT,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// The wind where the camera is, world units per second, as of this
    /// tick.
    pub fn wind_speed(&self) -> f32 {
        self.atmosphere("wind speed").unwrap_or(0.0) as f32
    }

    /// Degrees the wind blows towards, clockwise from north, as of this tick.
    pub fn wind_direction(&self) -> f32 {
        self.atmosphere("wind direction").unwrap_or(0.0) as f32
    }

    /// How stormy the wind is, 0-1, as of this tick.
    pub fn storm(&self) -> f32 {
        self.atmosphere("storm").unwrap_or(0.0) as f32
    }

    /// Whether the world is lit by ray tracing right now, as of this tick.
    pub fn ray_tracing_on(&self) -> bool {
        self.atmosphere("ray tracing").unwrap_or(0.0) != 0.0
    }

    /// Whether this GPU and build can trace rays at all.
    pub fn ray_tracing_available(&self) -> bool {
        self.atmosphere("ray tracing available").unwrap_or(0.0) != 0.0
    }

    /// Whether an actor carries a light casting shadows. Empty names this one.
    pub fn casts_shadows(&self, actor: &str) -> bool {
        self.number(READ_CASTS_SHADOWS, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// Switches an environment volume on or off for the rest of the run.
    /// Names it by id or name; empty means this actor.
    pub fn enable_volume(&self, volume: &str, enabled: bool) {
        self.act(
            ACT_ENABLE_VOLUME,
            Str::borrow(volume),
            Str::EMPTY,
            Str::EMPTY,
            if enabled { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// An environment volume's weight, 0-1, for the rest of the run.
    pub fn set_volume_weight(&self, volume: &str, weight: f32) {
        self.act(
            ACT_SET_VOLUME_WEIGHT,
            Str::borrow(volume),
            Str::EMPTY,
            Str::EMPTY,
            weight as f64,
            0.0,
            0.0,
        );
    }

    /// Names of the environment volumes showing at the camera, lowest
    /// priority first, as of this fixed tick.
    pub fn active_volumes(&self) -> Vec<String> {
        let json = self
            .text(TEXT_ACTIVE_VOLUMES, Str::EMPTY, Str::EMPTY)
            .unwrap_or_default();
        parse_names(&json)
    }

    /// A one-shot push. Only a dynamic body responds.
    pub fn push(&self, x: f32, y: f32, z: f32) {
        self.act(
            ACT_APPLY_IMPULSE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// A force on this body for one fixed step. `mode` is `"Force"`,
    /// `"Acceleration"`, `"Impulse"` or `"VelocityChange"`.
    pub fn add_force(&self, mode: &str, x: f32, y: f32, z: f32) {
        self.act(
            ACT_ADD_FORCE,
            Str::borrow(mode),
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// Moves this actor's character controller by a displacement in world
    /// units, sliding along what it hits. No gravity. Needs a
    /// CharacterController component.
    pub fn move_controller(&self, x: f32, y: f32, z: f32) -> Moved {
        self.controller_op("move", x, y, z)
    }

    /// Moves it at a speed in units a second with gravity applied; up is
    /// ignored.
    pub fn simple_move_controller(&self, x: f32, y: f32, z: f32) -> Moved {
        self.controller_op("simple move", x, y, z)
    }

    /// Changes one controller setting for the run: `"enabled"`, `"radius"`,
    /// `"height"`, `"slope limit"`, `"step offset"`, `"skin width"`,
    /// `"minimum move"`, `"detect collisions"` or `"overlap recovery"`.
    pub fn set_controller(&self, property: &str, value: f64) {
        let op = format!("set {property}");
        self.act(
            ACT_CONTROLLER,
            Str::borrow(&op),
            Str::EMPTY,
            Str::EMPTY,
            value,
            0.0,
            0.0,
        );
    }

    fn controller_op(&self, op: &str, x: f32, y: f32, z: f32) -> Moved {
        self.act(
            ACT_CONTROLLER,
            Str::borrow(op),
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
        let n = |field: &str| self.controller_number(field, 0);
        Moved {
            sides: n("sides") != 0.0,
            above: n("above") != 0.0,
            below: n("below") != 0.0,
            grounded: n("grounded") != 0.0,
            moved: (n("moved x") as f32, n("moved y") as f32, n("moved z") as f32),
            hits: n("hit count") as usize,
        }
    }

    /// A number from this actor's last controller move: `grounded`, `flags`,
    /// `sides`, `above`, `below`, `moved x/y/z`, `asked x/y/z`, `velocity
    /// x/y/z`, `fall speed`, `recovered`, `stepped`, `skipped`, `hit count`;
    /// and, for the `index`th (from 1) obstacle, `hit x/y/z`, `normal x/y/z`
    /// and `hit length`. Zero when there is none.
    pub fn controller_number(&self, field: &str, index: usize) -> f64 {
        self.number(READ_CONTROLLER, Str::borrow(field), Str::EMPTY, index as f64)
            .unwrap_or(0.0)
    }

    /// Words from the last controller move: the `index`th obstacle's `actor`,
    /// `body` or `collider`, or the `error` that stopped it.
    pub fn controller_text(&self, field: &str, index: usize) -> String {
        self.text(
            TEXT_CONTROLLER,
            Str::borrow(field),
            Str::borrow(&index.to_string()),
        )
        .unwrap_or_default()
    }

    /// Whether this controller stood on something after its last move.
    pub fn is_grounded(&self) -> bool {
        self.controller_number("grounded", 0) != 0.0
    }

    /// Steers this actor's character motor: a direction, length 1 for full
    /// speed. Needs a CharacterMotor owned by a script or an AI.
    pub fn motor_steer(&self, x: f32, y: f32, z: f32) {
        self.motor_op("intent", x, y, z);
    }

    /// Presses jump on this actor's motor.
    pub fn motor_jump(&self) {
        self.motor_op("jump", 0.0, 0.0, 0.0);
    }

    /// Lets go of jump, which cuts a rising jump short.
    pub fn motor_release_jump(&self) {
        self.motor_op("jump release", 0.0, 0.0, 0.0);
    }

    pub fn motor_sprint(&self, on: bool) {
        self.motor_op(if on { "sprint on" } else { "sprint off" }, 0.0, 0.0, 0.0);
    }

    pub fn motor_crouch(&self, on: bool) {
        self.motor_op(if on { "crouch on" } else { "crouch off" }, 0.0, 0.0, 0.0);
    }

    /// Knockback that fades by the motor's external drag.
    pub fn motor_push(&self, x: f32, y: f32, z: f32) {
        self.motor_op("push", x, y, z);
    }

    /// Clears every intent and the motor's speed.
    pub fn motor_stop(&self) {
        self.motor_op("stop", 0.0, 0.0, 0.0);
    }

    /// Changes one motor setting for the run: `"walk speed"`, `"jump
    /// height"`, `"max jumps"`, `"air control"` and so on.
    pub fn set_motor(&self, property: &str, value: f64) {
        self.motor_op(&format!("set {property}"), value as f32, 0.0, 0.0);
    }

    fn motor_op(&self, op: &str, x: f32, y: f32, z: f32) {
        let op = format!("motor {op}");
        self.act(
            ACT_CONTROLLER,
            Str::borrow(&op),
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// A number from this actor's motor: `grounded`, `rising`, `falling`,
    /// `landed`, `jumped`, `speed`, `vertical speed`, `jumps left`, `slope`,
    /// `knockback` and more. Zero without a motor.
    pub fn motor_number(&self, field: &str) -> f64 {
        self.controller_number(&format!("motor {field}"), 0)
    }

    /// Words from this actor's motor: `state`, `support`, `owner`, `warning`.
    pub fn motor_text(&self, field: &str) -> String {
        self.controller_text(&format!("motor {field}"), 0)
    }

    /// The same for a torque. A 2D body turns about z only.
    pub fn add_torque(&self, mode: &str, x: f32, y: f32, z: f32) {
        self.act(
            ACT_ADD_FORCE,
            Str::borrow(mode),
            Str::borrow("torque"),
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    pub fn set_velocity(&self, x: f32, y: f32, z: f32) {
        self.act(
            ACT_SET_VELOCITY,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            x as f64,
            y as f64,
            z as f64,
        );
    }

    /// A speech bubble over the actor; an empty text clears it.
    pub fn say(&self, text: &str) {
        self.act(
            ACT_SAY,
            Str::borrow(text),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    pub fn set_visible(&self, visible: bool) {
        self.act(
            ACT_SET_VISIBLE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if visible { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// A `#RRGGBB` string. Does nothing to an image actor.
    pub fn set_color(&self, color: &str) {
        self.act(
            ACT_SET_COLOR,
            Str::borrow(color),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Plays a sound file from the project's assets as a global voice.
    /// `volume` is 0-100, `pitch` is 1 for as recorded.
    pub fn play_sound(&self, sound: &str, volume: f64, pitch: f64, loop_: bool, bus: SoundBus) {
        self.act(
            ACT_PLAY_SOUND,
            Str::borrow(sound),
            Str::borrow(bus.name()),
            Str::EMPTY,
            volume,
            pitch,
            if loop_ { 1.0 } else { 0.0 },
        );
    }

    /// Plays a sound at an actor's place and follows it around, panned by
    /// position and quieter with distance. An empty target means here.
    pub fn play_sound_at(
        &self,
        sound: &str,
        target: &str,
        volume: f64,
        pitch: f64,
        loop_: bool,
        bus: SoundBus,
    ) {
        self.act(
            ACT_PLAY_SOUND,
            Str::borrow(sound),
            Str::borrow(bus.name()),
            Str::borrow(target),
            volume,
            pitch,
            if loop_ { 1.0 } else { 0.0 },
        );
    }

    /// Stops the voices playing a sound file. Empty stops every sound.
    pub fn stop_sound(&self, sound: &str) {
        self.act(
            ACT_STOP_SOUND,
            Str::borrow(sound),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Retunes the voices already playing a sound file, 0-100.
    pub fn set_sound_volume(&self, sound: &str, volume: f64) {
        self.act(
            ACT_SET_SOUND_VOLUME,
            Str::borrow(sound),
            Str::EMPTY,
            Str::EMPTY,
            volume,
            0.0,
            0.0,
        );
    }

    /// Rebends the voices already playing a sound file. 1 is as recorded.
    pub fn set_sound_pitch(&self, sound: &str, pitch: f64) {
        self.act(
            ACT_SET_SOUND_PITCH,
            Str::borrow(sound),
            Str::EMPTY,
            Str::EMPTY,
            pitch,
            0.0,
            0.0,
        );
    }

    /// Moves a whole mixing bus, 0-100.
    pub fn set_bus_volume(&self, bus: SoundBus, volume: f64) {
        self.act(
            ACT_SET_BUS_VOLUME,
            Str::borrow(bus.name()),
            Str::EMPTY,
            Str::EMPTY,
            volume,
            0.0,
            0.0,
        );
    }

    /// Whether any voice is playing that file right now.
    pub fn is_sound_playing(&self, sound: &str) -> bool {
        self.number(READ_SOUND_PLAYING, Str::borrow(sound), Str::EMPTY, 0.0)
            .is_some_and(|playing| playing != 0.0)
    }

    /// A mixing bus's live gain in 0-100. Unknown buses read as unity.
    pub fn bus_volume(&self, bus: SoundBus) -> f64 {
        self.number(READ_BUS_VOLUME, Str::borrow(bus.name()), Str::EMPTY, 0.0)
            .unwrap_or(100.0)
    }

    /// Smoothed frame work time in milliseconds.
    pub fn frame_time(&self) -> f64 {
        self.number(READ_FRAME_TIME, Str::EMPTY, Str::EMPTY, 0.0).unwrap_or(0.0)
    }
    /// Estimated visible mesh draws, excluding shadows and post-processing.
    pub fn draw_calls(&self) -> u32 {
        self.number(READ_DRAW_CALLS, Str::EMPTY, Str::EMPTY, 0.0).unwrap_or(0.0) as u32
    }
    pub fn dlss_available(&self) -> bool {
        self.number(READ_DLSS_AVAILABLE, Str::EMPTY, Str::EMPTY, 0.0).unwrap_or(0.0) != 0.0
    }
    pub fn current_quality(&self) -> Option<String> {
        self.text(TEXT_CURRENT_QUALITY, Str::EMPTY, Str::EMPTY)
    }

    /// One reading of the air as of this fixed tick: `sun x`, `wind speed`,
    /// `fog density`, `rain`, ... as the atmosphere reporter names them.
    /// `None` for a name the host doesn't know.
    pub fn atmosphere(&self, reading: &str) -> Option<f64> {
        self.number(READ_ATMOSPHERE, Str::borrow(reading), Str::EMPTY, 0.0)
    }

    /// Fires every `when I get` strand listening for it, in every actor.
    pub fn broadcast(&self, message: &str) {
        self.act(
            ACT_BROADCAST,
            Str::borrow(message),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// A line in the editor's run log, for working out what a script is doing.
    pub fn log(&self, message: &str) {
        self.act(
            ACT_LOG,
            Str::borrow(message),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Tilts a first-person camera up or down, in degrees. Positive looks up;
    /// clamped just short of vertical.
    pub fn set_camera_pitch(&self, degrees: f32) {
        self.act(
            ACT_SET_CAMERA_PITCH,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            degrees as f64,
            0.0,
            0.0,
        );
    }

    /// Sets the camera's vertical field of view, in degrees.
    pub fn set_camera_fov(&self, fov: f32) {
        self.act(
            ACT_SET_CAMERA_FOV,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            fov as f64,
            0.0,
            0.0,
        );
    }

    /// Grabs the pointer for first-person play, or shows it again. The run
    /// always ends unlocked.
    pub fn set_mouse_locked(&self, locked: bool) {
        self.act(
            ACT_SET_MOUSE_LOCKED,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if locked { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    /// Rumbles every connected gamepad at 0-100 strength for seconds.
    /// Zero of either stops instead.
    pub fn rumble_gamepad(&self, strength: f32, duration: f32) {
        self.act(
            ACT_RUMBLE_GAMEPAD,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            strength as f64,
            duration as f64,
            0.0,
        );
    }

    /// Adds one binding (`space`, `mouse:left`, `gamepad:south`) to an
    /// action for the rest of the run. What a settings screen calls.
    pub fn bind_action(&self, action: &str, binding: &str) {
        self.act(
            ACT_BIND_ACTION,
            Str::borrow(action),
            Str::borrow(binding),
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Forgets every binding an action has for the rest of the run.
    pub fn clear_action_bindings(&self, action: &str) {
        self.act(
            ACT_CLEAR_ACTION_BINDINGS,
            Str::borrow(action),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Stops every script in the project, blocks included.
    pub fn stop_all(&self) {
        self.act(
            ACT_STOP_ALL,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    // ─── The interface ─────────────────────────────────────────────────────
    // The same elements the `show` blocks make, named by the same ids: a
    // script and a canvas can build one menu between them.

    /// Makes an interface element, or updates the one `id` already names.
    /// Build one with [`Ui::at`] and friends, then hand it here.
    pub fn show(&self, element: &Ui) {
        self.act_many(
            ACT_UI_SHOW,
            Str::borrow(element.id),
            Str::borrow(element.content),
            Str::borrow(element.parent),
            &[
                element.kind as u32 as f64,
                element.anchor as u32 as f64,
                element.offset.0 as f64,
                element.offset.1 as f64,
                element.size.0 as f64,
                element.size.1 as f64,
                if element.flag { 1.0 } else { 0.0 },
                element.range.0 as f64,
                element.range.1 as f64,
                element.value,
            ],
        );
    }

    /// Writes one numeric property of an element - `"width"`, `"value"`,
    /// `"visible"` and the rest, spelled as [`UiProp`]'s wire names.
    pub fn set_ui(&self, id: &str, property: &str, value: f64) {
        self.act(
            ACT_UI_SET,
            Str::borrow(id),
            Str::borrow(property),
            Str::EMPTY,
            value,
            0.0,
            0.0,
        );
    }

    /// The same, writing text - `"Text"`, `"Background"`, `"TextColor"`.
    pub fn set_ui_text(&self, id: &str, property: &str, value: &str) {
        self.act(
            ACT_UI_SET_TEXT,
            Str::borrow(id),
            Str::borrow(property),
            Str::borrow(value),
            0.0,
            0.0,
            0.0,
        );
    }

    /// Takes an element off the screen, children and all, without forgetting
    /// it.
    pub fn hide_ui(&self, id: &str) {
        self.act(
            ACT_UI_HIDE,
            Str::borrow(id),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Hides every element at once, and drops keyboard focus with them.
    pub fn hide_all_ui(&self) {
        self.act(
            ACT_UI_HIDE,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            1.0,
            0.0,
            0.0,
        );
    }

    /// Forgets an element entirely, children and all.
    pub fn delete_ui(&self, id: &str) {
        self.act(
            ACT_UI_DELETE,
            Str::borrow(id),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// A slider's number, or a toggle as `1.0`/`0.0`. Zero for an id nothing
    /// answers to.
    pub fn bind_ui(&self, id: &str, bindings_json: &str) { self.set_ui_text(id, "Bind", bindings_json); }
    pub fn set_ui_items(&self, id: &str, items_json: &str) { self.set_ui_text(id, "Items", items_json); }
    pub fn scroll_ui_to(&self, id: &str, offset: f64) { self.set_ui(id, "Scroll", offset); }
    pub fn ui_selected_index(&self, id: &str) -> f64 { self.ui_value(id) }
    pub fn set_widget_theme(&self, id: &str, theme: &str) { self.set_ui_text(id, "Theme", theme); }

    pub fn ui_value(&self, id: &str) -> f64 {
        self.number(READ_UI_VALUE, Str::borrow(id), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
    }

    /// A text input's typed text, or an empty string.
    pub fn ui_text(&self, id: &str) -> String {
        self.text(TEXT_UI_VALUE, Str::borrow(id), Str::EMPTY)
            .unwrap_or_default()
    }

    /// The words an element is showing - a label's text, a button's caption,
    /// or an empty input's placeholder.
    pub fn ui_caption(&self, id: &str) -> String {
        self.text(TEXT_UI_TEXT, Str::borrow(id), Str::EMPTY)
            .unwrap_or_default()
    }

    /// Whether an element is on the screen right now, which a hidden parent
    /// decides as surely as its own `hide` does.
    pub fn ui_shown(&self, id: &str) -> bool {
        self.number(READ_UI_SHOWN, Str::borrow(id), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// Whether an element by that name has been made at all. A hidden one
    /// still counts - `hide` doesn't forget an element, `delete` does.
    pub fn ui_exists(&self, id: &str) -> bool {
        self.number(READ_UI_EXISTS, Str::borrow(id), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    /// Hands the keyboard to a text input without waiting for a click.
    pub fn focus_ui(&self, id: &str) {
        self.act(
            ACT_UI_FOCUS,
            Str::borrow(id),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Takes the keyboard back off whichever input holds it.
    pub fn clear_ui_focus(&self) {
        self.act(
            ACT_UI_FOCUS,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Changes the defaults for interface elements without replacing any
    /// property written directly on an element.
    pub fn set_ui_theme(&self, theme: UiTheme) {
        self.act(
            ACT_UI_THEME,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            theme as u32 as f64,
            0.0,
            0.0,
        );
    }

    /// Saves the currently visible variable slot for the next run.
    pub fn save_variable(&self, name: &str) {
        self.act(
            ACT_SAVE_VARIABLE,
            Str::borrow(name),
            Str::EMPTY,
            Str::EMPTY,
            0.0,
            0.0,
            0.0,
        );
    }

    /// Forgets a saved value without changing the variable in this run.
    pub fn clear_saved_variable(&self, name: &str) {
        self.act(
            ACT_SAVE_VARIABLE,
            Str::borrow(name),
            Str::EMPTY,
            Str::EMPTY,
            1.0,
            0.0,
            0.0,
        );
    }

    /// Which text input holds the keyboard, or an empty string. While one
    /// does, a script's `key_down` sees nothing, the same as a block's.
    pub fn ui_focus(&self) -> String {
        self.text(TEXT_UI_FOCUS, Str::EMPTY, Str::EMPTY)
            .unwrap_or_default()
    }

    /// Freezes the world, as the `pause game` block does. Scripts and blocks
    /// alike stop; a strand the interface started carries on.
    pub fn set_paused(&self, paused: bool) {
        self.act(
            ACT_SET_PAUSED,
            Str::EMPTY,
            Str::EMPTY,
            Str::EMPTY,
            if paused { 1.0 } else { 0.0 },
            0.0,
            0.0,
        );
    }

    pub fn is_paused(&self) -> bool {
        self.number(READ_GAME_PAUSED, Str::EMPTY, Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }
}

/// What kind of interface element [`Ui`] describes. The numbers are the wire
/// ones, shared with `blockloom_core::ui::UiKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum UiKind {
    Panel = 0,
    Label = 1,
    Button = 2,
    Image = 3,
    Input = 4,
    Slider = 5,
    Toggle = 6,
    List = 7,
    VerticalBox = 8,
    HorizontalBox = 9,
    Grid = 10,
    Canvas = 11,
    WrapBox = 12,
    SizeBox = 13,
    Spacer = 14,
    Progress = 15,
    RadialProgress = 16,
    ListView = 17,
    Tabs = 18,
    Select = 19,
    Scrollbar = 20,
    RichText = 21,
    Tooltip = 22,

}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Dark = 0,
    Light = 1,
    HighContrast = 2,
}

/// Which corner, edge or centre of the window an element hangs off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum UiAnchor {
    TopLeft = 0,
    Top = 1,
    TopRight = 2,
    Left = 3,
    Center = 4,
    Right = 5,
    BottomLeft = 6,
    Bottom = 7,
    BottomRight = 8,
}

/// One interface element, as a script describes it before handing it to
/// [`Actor::show`]. Borrowed strings throughout: nothing here outlives the
/// call that carries it over the boundary.
///
/// ```ignore
/// me.show(&Ui::panel("menu", "Paused").modal().at(UiAnchor::Center, 0.0, 0.0));
/// me.show(&Ui::button("resume", "Resume").inside("menu"));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Ui<'a> {
    pub id: &'a str,
    pub kind: UiKind,
    pub content: &'a str,
    pub parent: &'a str,
    pub anchor: UiAnchor,
    pub offset: (f32, f32),
    pub size: (f32, f32),
    pub range: (f32, f32),
    pub value: f64,
    pub flag: bool,
}

impl<'a> Ui<'a> {
    pub fn new(id: &'a str, kind: UiKind, content: &'a str) -> Self {
        Self {
            id,
            kind,
            content,
            parent: "",
            anchor: UiAnchor::Center,
            offset: (0.0, 0.0),
            size: (0.0, 0.0),
            range: (0.0, 1.0),
            value: 0.0,
            flag: false,
        }
    }

    pub fn panel(id: &'a str, title: &'a str) -> Self {
        Self::new(id, UiKind::Panel, title)
    }

    pub fn label(id: &'a str, text: &'a str) -> Self {
        Self::new(id, UiKind::Label, text)
    }

    pub fn button(id: &'a str, label: &'a str) -> Self {
        Self::new(id, UiKind::Button, label)
    }

    pub fn image(id: &'a str, asset: &'a str) -> Self {
        Self::new(id, UiKind::Image, asset)
    }

    pub fn input(id: &'a str, placeholder: &'a str) -> Self {
        Self::new(id, UiKind::Input, placeholder)
    }

    pub fn slider(id: &'a str, min: f32, max: f32, value: f64) -> Self {
        let mut ui = Self::new(id, UiKind::Slider, "");
        ui.range = (min, max);
        ui.value = value;
        ui
    }

    pub fn toggle(id: &'a str, label: &'a str, on: bool) -> Self {
        let mut ui = Self::new(id, UiKind::Toggle, label);
        ui.flag = on;
        ui
    }

    pub fn list(id: &'a str) -> Self {
        Self::new(id, UiKind::List, "").sized(280.0, 240.0)
    }

    /// Where it hangs off the window, and how far from there in pixels.
    /// Ignored once [`Ui::inside`] names a parent.
    pub fn at(mut self, anchor: UiAnchor, x: f32, y: f32) -> Self {
        self.anchor = anchor;
        self.offset = (x, y);
        self
    }

    /// How big it is; a zero means "as big as the content needs".
    pub fn sized(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    /// Flows it inside another element rather than against the window.
    pub fn inside(mut self, parent: &'a str) -> Self {
        self.parent = parent;
        self
    }

    /// A panel that swallows the world clicks behind it.
    pub fn modal(mut self) -> Self {
        self.flag = true;
        self
    }
}

/// The strings of a JSON array of strings. A script gets `std` alone, so this
/// is the little of JSON the host hands back.
fn parse_names(json: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = json.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut name = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => name.push('\n'),
                    Some('t') => name.push('\t'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            name.push(c);
                        }
                    }
                    Some(other) => name.push(other),
                    None => break,
                },
                c => name.push(c),
            }
        }
        names.push(name);
    }
    names
}

/// The host calls of a script built for the browser, where the host is
/// another wasm module: each one packs its arguments into a [`WasmCall`] and
/// goes through an import (see the end of the ABI).
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The module name is `WASM_MODULE`, spelled out because `link` takes a
    // literal.
    #[link(wasm_import_module = "blockloom")]
    unsafe extern "C" {
        #[link_name = "read_number"]
        fn import_read_number(ctx: *mut c_void, what: u32, call: *mut WasmCall) -> u32;
        #[link_name = "read_text"]
        fn import_read_text(ctx: *mut c_void, what: u32, call: *mut WasmCall) -> u32;
        #[link_name = "act"]
        fn import_act(ctx: *mut c_void, what: u32, call: *mut WasmCall) -> u32;
    }

    fn call(a: Str, b: Str, c: Str) -> WasmCall {
        WasmCall {
            a_ptr: a.ptr as usize as u32,
            a_len: a.len as u32,
            b_ptr: b.ptr as usize as u32,
            b_len: b.len as u32,
            c_ptr: c.ptr as usize as u32,
            c_len: c.len as u32,
            ..WasmCall::default()
        }
    }

    extern "C" fn read_number(
        ctx: *mut c_void,
        what: u32,
        a: Str,
        b: Str,
        arg: f64,
        out: *mut f64,
    ) -> u32 {
        let mut call = call(a, b, Str::EMPTY);
        call.arg = arg;
        call.out = out as usize as u32;
        unsafe { import_read_number(ctx, what, &mut call) }
    }

    extern "C" fn read_text(
        ctx: *mut c_void,
        what: u32,
        a: Str,
        b: Str,
        out: *mut u8,
        capacity: usize,
        length: *mut usize,
    ) -> u32 {
        let mut needed: u32 = 0;
        let mut call = call(a, b, Str::EMPTY);
        call.out = out as usize as u32;
        call.out_cap = capacity as u32;
        call.out_len = (&raw mut needed) as usize as u32;
        let code = unsafe { import_read_text(ctx, what, &mut call) };
        unsafe { *length = needed as usize };
        code
    }

    extern "C" fn act(
        ctx: *mut c_void,
        what: u32,
        a: Str,
        b: Str,
        c: Str,
        numbers: *const f64,
        count: usize,
    ) {
        let mut call = call(a, b, c);
        call.numbers = numbers as usize as u32;
        call.count = count as u32;
        unsafe { import_act(ctx, what, &mut call) };
    }

    pub static HOST: HostApi = HostApi {
        abi: ABI_VERSION,
        read_number,
        read_text,
        act,
    };

    /// The context of the call in progress, for the panic hook.
    static CURRENT: AtomicUsize = AtomicUsize::new(0);

    /// Notes the call now running and, once, hooks panics: a browser build
    /// aborts on panic rather than unwinding into `guard`, so the message is
    /// logged here before the host sees the trap and stops the script.
    pub fn enter(ctx: *mut c_void) {
        CURRENT.store(ctx as usize, Ordering::Relaxed);
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                let ctx = CURRENT.load(Ordering::Relaxed) as *mut c_void;
                let text = format!("the script panicked: {info}");
                act(
                    ctx,
                    ACT_LOG,
                    Str::borrow(&text),
                    Str::EMPTY,
                    Str::EMPTY,
                    std::ptr::null(),
                    0,
                );
            }));
        });
    }
}

/// Runs `f`, turning a panic into a log line instead of letting it cross the
/// C boundary - which would take the whole game window down with it.
#[doc(hidden)]
pub fn guard(actor: &Actor, what: &str, f: impl FnOnce() + std::panic::UnwindSafe) {
    if std::panic::catch_unwind(f).is_err() {
        actor.log(&format!("the script panicked in {what}"));
    }
}

/// Something that happened to this actor, or to the whole game, which the
/// runtime hands the `event` entry point the frame it happens: the same
/// events a canvas's hat blocks start on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A broadcast.
    Message(String),
    /// A key went down, as `key_down` spells it.
    Key(String),
    /// An input action went down.
    Action(String),
    /// This actor was clicked.
    Clicked,
    /// A finger touched the screen.
    Touched,
    /// This actor started touching another: its name and its id.
    Collision { with: String, id: String },
    /// This actor entered, kept or left a touch: the other actor's name and
    /// id, the phase, whether it is a trigger overlap, and for a solid touch
    /// the impulse the solver spent and the relative speed.
    Contact {
        with: String,
        id: String,
        phase: ContactPhase,
        trigger: bool,
        impulse: f32,
        speed: f32,
    },
    /// This actor's particles spawned, died or collided this frame: how many,
    /// and where the last one did.
    Particles {
        kind: ParticleKind,
        count: u32,
        at: (f32, f32, f32),
    },
    /// A `Once` clip ended.
    AnimationEnded(String),
    /// The clip reached a marker.
    AnimationMarker(String),
    /// An interface element was clicked.
    UiClicked(String),
    /// An input element changed, and its value as text.
    UiChanged { element: String, value: String },
    /// Any other interface event: the element and the event.
    Ui { element: String, event: String },
    /// This actor walked into a room, by the room's name.
    EnteredRoom(String),
    /// The weather blend landed on a preset, by name.
    Weather(String),
    /// The playing cutscene passed a signal marker, by name.
    CutsceneSignal(String),
    /// The playing cutscene reached its end marker, by name.
    CutsceneEnded(String),
    /// A plugin raised an event: its id, the event, and the text of each
    /// slot it carried.
    Plugin {
        plugin: String,
        event: String,
        args: Vec<String>,
    },
    /// The newly loaded scene finished warming up.
    SceneStarted,
    /// The outgoing scene is about to unload.
    SceneEnded,
}

/// One slot value of a plugin block: what `plugin_call` and the plugin
/// readers take, so `&["Hero".into(), 5.0.into()]` spells two.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginArg {
    Number(f64),
    Text(String),
    Bool(bool),
}

impl From<f64> for PluginArg {
    fn from(value: f64) -> Self {
        PluginArg::Number(value)
    }
}
impl From<f32> for PluginArg {
    fn from(value: f32) -> Self {
        PluginArg::Number(value as f64)
    }
}
impl From<i32> for PluginArg {
    fn from(value: i32) -> Self {
        PluginArg::Number(value as f64)
    }
}
impl From<bool> for PluginArg {
    fn from(value: bool) -> Self {
        PluginArg::Bool(value)
    }
}
impl From<&str> for PluginArg {
    fn from(value: &str) -> Self {
        PluginArg::Text(value.to_string())
    }
}
impl From<String> for PluginArg {
    fn from(value: String) -> Self {
        PluginArg::Text(value)
    }
}

/// Slot values as a JSON array, a whole number as an integer.
fn plugin_args_json(args: &[PluginArg]) -> String {
    let mut out = String::from("[");
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match arg {
            PluginArg::Number(n) if !n.is_finite() => out.push_str("null"),
            PluginArg::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => {
                out.push_str(&(*n as i64).to_string())
            }
            PluginArg::Number(n) => out.push_str(&format!("{n:?}")),
            PluginArg::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            PluginArg::Text(text) => {
                out.push('"');
                for ch in text.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        ch if (ch as u32) < 0x20 => {
                            out.push_str(&format!("\\u{:04x}", ch as u32))
                        }
                        ch => out.push(ch),
                    }
                }
                out.push('"');
            }
        }
    }
    out.push(']');
    out
}

/// Which part of a touch an [`Event::Contact`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactPhase {
    Enter,
    Stay,
    Exit,
}

/// Which particle event an [`Event::Particles`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleKind {
    Spawn,
    Die,
    Collide,
}

impl Event {
    /// What the host called the entry point with, or `None` for a kind this
    /// build doesn't know.
    #[doc(hidden)]
    pub fn from_raw(me: &Actor, kind: u32, n: [f64; 4]) -> Option<Event> {
        let word = |what: &str| {
            me.text(TEXT_EVENT, Str::borrow(what), Str::EMPTY)
                .unwrap_or_default()
        };
        let subject = word("");
        Some(match kind {
            EVENT_MESSAGE => Event::Message(subject),
            EVENT_KEY => Event::Key(subject),
            EVENT_ACTION => Event::Action(subject),
            EVENT_CLICKED => Event::Clicked,
            EVENT_TOUCHED => Event::Touched,
            EVENT_COLLISION => Event::Collision {
                with: subject,
                id: word("detail"),
            },
            EVENT_CONTACT => Event::Contact {
                with: subject,
                id: word("detail"),
                phase: match n[0] as u32 {
                    0 => ContactPhase::Enter,
                    1 => ContactPhase::Stay,
                    _ => ContactPhase::Exit,
                },
                trigger: n[1] != 0.0,
                impulse: n[2] as f32,
                speed: n[3] as f32,
            },
            EVENT_PARTICLES => Event::Particles {
                kind: match subject.as_str() {
                    "spawn" => ParticleKind::Spawn,
                    "die" => ParticleKind::Die,
                    "collide" => ParticleKind::Collide,
                    _ => return None,
                },
                count: n[0] as u32,
                at: (n[1] as f32, n[2] as f32, n[3] as f32),
            },
            EVENT_ANIMATION_ENDED => Event::AnimationEnded(subject),
            EVENT_ANIMATION_MARKER => Event::AnimationMarker(subject),
            EVENT_UI_CLICKED => Event::UiClicked(subject),
            EVENT_UI_CHANGED => Event::UiChanged {
                element: subject,
                value: word("detail"),
            },
            EVENT_UI => Event::Ui {
                element: subject,
                event: word("detail"),
            },
            EVENT_ENTERED_ROOM => Event::EnteredRoom(subject),
            EVENT_WEATHER => Event::Weather(subject),
            EVENT_CUTSCENE_SIGNAL => Event::CutsceneSignal(subject),
            EVENT_CUTSCENE_ENDED => Event::CutsceneEnded(subject),
            EVENT_PLUGIN => {
                let detail = word("detail");
                let mut parts = detail.split(PLUGIN_SEP).map(str::to_string);
                Event::Plugin {
                    plugin: parts.next().unwrap_or_default(),
                    event: subject,
                    args: parts.collect(),
                }
            }
            EVENT_SCENE_STARTED => Event::SceneStarted,
            EVENT_SCENE_ENDED => Event::SceneEnded,
            _ => return None,
        })
    }
}

/// The entry points a script leaves out.
#[doc(hidden)]
pub fn no_start(_: &Actor) {}
#[doc(hidden)]
pub fn no_tick(_: &Actor, _: f32) {}
#[doc(hidden)]
pub fn no_event(_: &Actor, _: &Event) {}

/// Names the functions the runtime should call, and writes the entry points
/// that call them. Any of the three may be left out, in any order.
///
/// ```ignore
/// use blockloom::*;
///
/// fn start(me: &Actor) { me.say("hello"); }
/// fn tick(me: &Actor, dt: f32) { me.move_forward(60.0 * dt); }
/// fn event(me: &Actor, event: &Event) {
///     if let Event::Particles { kind: ParticleKind::Collide, at, .. } = event {
///         me.log(&format!("a spark hit at {at:?}"));
///     }
/// }
///
/// blockloom::export!(start = start, tick = tick, event = event);
/// ```
#[macro_export]
macro_rules! export {
    (@take [$start:path, $tick:path, $event:path]) => {
        $crate::export!(@emit $start, $tick, $event);
    };
    (@take [$start:path, $tick:path, $event:path] start = $value:path $(, $($rest:tt)*)?) => {
        $crate::export!(@take [$value, $tick, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $event:path] tick = $value:path $(, $($rest:tt)*)?) => {
        $crate::export!(@take [$start, $value, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $event:path] event = $value:path $(, $($rest:tt)*)?) => {
        $crate::export!(@take [$start, $tick, $value] $($($rest)*)?);
    };
    (@emit $start:path, $tick:path, $event:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_abi() -> u32 {
            $crate::ABI_VERSION
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_start(
            ctx: *mut ::std::ffi::c_void,
            api: *const $crate::HostApi,
        ) {
            let me = unsafe { $crate::Actor::from_raw(ctx, api) };
            $crate::guard(&me, "start", || $start(&me));
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_tick(
            ctx: *mut ::std::ffi::c_void,
            api: *const $crate::HostApi,
            dt: f32,
        ) {
            let me = unsafe { $crate::Actor::from_raw(ctx, api) };
            $crate::guard(&me, "tick", || $tick(&me, dt));
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_event(
            ctx: *mut ::std::ffi::c_void,
            api: *const $crate::HostApi,
            kind: u32,
            n0: f64,
            n1: f64,
            n2: f64,
            n3: f64,
        ) {
            let me = unsafe { $crate::Actor::from_raw(ctx, api) };
            if let Some(event) = $crate::Event::from_raw(&me, kind, [n0, n1, n2, n3]) {
                $crate::guard(&me, "event", || $event(&me, &event));
            }
        }
    };
    ($($rest:tt)*) => {
        $crate::export!(@take [$crate::no_start, $crate::no_tick, $crate::no_event] $($rest)*);
    };
}
