
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
    pub fn ui_value(&self, id: &str) -> f64 {
        self.number(READ_UI_VALUE, Str::borrow(id), Str::EMPTY, 0.0)
            .unwrap_or(0.0)
    }

    /// A text input's typed text, or an empty string.
    pub fn ui_text(&self, id: &str) -> String {
        self.text(TEXT_UI_VALUE, Str::borrow(id), Str::EMPTY)
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
