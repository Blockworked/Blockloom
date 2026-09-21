
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
        unsafe { ((*self.api).act)(self.ctx, what, a, b, c, n0, n1, n2) }
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

    pub fn touching(&self, actor: &str) -> bool {
        self.number(READ_TOUCHING, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
            != 0.0
    }

    pub fn touching_anything(&self) -> bool {
        self.touching("")
    }

    pub fn distance_to(&self, actor: &str) -> f32 {
        self.number(READ_DISTANCE_TO, Str::borrow(actor), Str::EMPTY, 0.0)
            .unwrap_or(0.0) as f32
    }

    // ─── Components ────────────────────────────────────────────────────────

    /// Whether the actor is carrying that component right now - which a
    /// [`Actor::detach`] earlier in the run may have changed.
    pub fn has(&self, component: &str) -> bool {
        self.number(
            READ_HAS_COMPONENT,
            Str::borrow(component),
            Str::EMPTY,
            0.0,
        )
        .unwrap_or(0.0)
            != 0.0
    }

    /// One field of one of the actor's custom components, or `None` when the
    /// actor has no such component or it has no such field.
    pub fn field(&self, component: &str, field: &str) -> Option<f64> {
        self.number(
            READ_FIELD,
            Str::borrow(component),
            Str::borrow(field),
            0.0,
        )
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
    /// together. The actor keeps the place it is standing in.
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
}

/// Runs `f`, turning a panic into a log line instead of letting it cross the
/// C boundary - which would take the whole game window down with it.
#[doc(hidden)]
pub fn guard(actor: &Actor, what: &str, f: impl FnOnce() + std::panic::UnwindSafe) {
    if std::panic::catch_unwind(f).is_err() {
        actor.log(&format!("the script panicked in {what}"));
    }
}

/// Names the functions the runtime should call, and writes the entry points
/// that call them.
///
/// ```ignore
/// use blockloom::*;
///
/// fn start(me: &Actor) { me.say("hello"); }
/// fn tick(me: &Actor, dt: f32) { me.move_forward(60.0 * dt); }
///
/// blockloom::export!(start = start, tick = tick);
/// ```
#[macro_export]
macro_rules! export {
    (start = $start:path, tick = $tick:path) => {
        $crate::export!(@abi);
        $crate::export!(@start $start);
        $crate::export!(@tick $tick);
    };
    (tick = $tick:path, start = $start:path) => {
        $crate::export!(start = $start, tick = $tick);
    };
    (start = $start:path) => {
        $crate::export!(@abi);
        $crate::export!(@start $start);
        $crate::export!(@tick_empty);
    };
    (tick = $tick:path) => {
        $crate::export!(@abi);
        $crate::export!(@start_empty);
        $crate::export!(@tick $tick);
    };

    (@abi) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_abi() -> u32 {
            $crate::ABI_VERSION
        }
    };
    (@start $start:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_start(
            ctx: *mut ::std::ffi::c_void,
            api: *const $crate::HostApi,
        ) {
            let me = unsafe { $crate::Actor::from_raw(ctx, api) };
            $crate::guard(&me, "start", || $start(&me));
        }
    };
    (@start_empty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_start(
            _ctx: *mut ::std::ffi::c_void,
            _api: *const $crate::HostApi,
        ) {
        }
    };
    (@tick $tick:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_tick(
            ctx: *mut ::std::ffi::c_void,
            api: *const $crate::HostApi,
            dt: f32,
        ) {
            let me = unsafe { $crate::Actor::from_raw(ctx, api) };
            $crate::guard(&me, "tick", || $tick(&me, dt));
        }
    };
    (@tick_empty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_tick(
            _ctx: *mut ::std::ffi::c_void,
            _api: *const $crate::HostApi,
            _dt: f32,
        ) {
        }
    };
}
