// What a generated program is built on: a value type, the operators over it,
// and the boundary it asks the world through. Like `script/abi.rs`, this file
// is compiled twice - once into `blockloom-core`, where the tests can hold it
// against the VM's own semantics, and once as text into every program
// `codegen` emits. Editing it changes both at once, which is the point.
//
// Nothing here may use anything but `std`: a generated program is compiled by
// one `rustc` run with no dependencies at all.

use std::rc::Rc;

/// A value as a generated program carries it. The same three cases
/// `Evaluated` has, with the same coercions - a difference here is a
/// difference in what a game does.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Text(String),
    Bool(bool),
}

/// What an expression evaluates to. One error anywhere gives up the whole
/// tree, exactly as `Value::eval` does, and the caller reports it once.
pub type R = Result<Val, String>;

impl Val {
    pub fn as_number(&self) -> Result<f64, String> {
        match self {
            Val::Num(n) => Ok(*n),
            Val::Text(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("\"{s}\" is not a number")),
            Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0,
            Val::Text(s) => !s.is_empty() && s != "false",
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Val::Text(s) => s.clone(),
            Val::Num(n) => n.to_string(),
            Val::Bool(b) => b.to_string(),
        }
    }
}

/// What a generated program asks the world to do. One per leaf instruction
/// the blocks have, carrying values that are already evaluated - the host
/// never evaluates anything, just as it never does for the VM.
///
/// Names that are enums elsewhere (a body kind, a camera view) travel as the
/// text they serialize as, so this file stays free of everything but `std`.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Move {
        steps: f32,
    },
    GoTo {
        position: [f32; 3],
    },
    NavigateTo {
        target: [f32; 3],
        speed: f32,
    },
    BurstParticles {
        count: u32,
    },
    SetEmitterDial {
        dial: &'static str,
        value: f32,
    },
    SetTrailEnabled {
        enabled: bool,
    },
    SetEmitterPlaying {
        playing: bool,
    },
    ChangePosition {
        axis: usize,
        by: f32,
    },
    /// Starts a slide; the strand sleeps for exactly as long.
    Glide {
        seconds: f32,
        target: [f32; 3],
        easing: &'static str,
    },
    TweenScale {
        factor: f32,
        seconds: f32,
        easing: &'static str,
    },
    TweenRotation {
        axis: usize,
        degrees: f32,
        seconds: f32,
        easing: &'static str,
    },
    TweenColor {
        color: String,
        seconds: f32,
        easing: &'static str,
    },
    StopTweens,
    PlayAnimation {
        clip: String,
        speed: f32,
    },
    StopAnimation,
    SetAnimationSpeed {
        speed: f32,
    },
    FireAnimationTrigger {
        name: String,
    },
    SetRigSlot {
        slot: String,
        attachment: String,
    },
    SetSlotTint {
        slot: String,
        color: String,
    },
    SetIkTarget {
        constraint: String,
        x: f32,
        y: f32,
    },
    SetSpriteDial {
        dial: &'static str,
        value: f32,
    },
    Turn {
        axis: usize,
        degrees: f32,
    },
    SetRotation {
        axis: usize,
        degrees: f32,
    },
    PointTowards {
        target: &'static str,
    },
    SetScale {
        factor: f32,
    },
    /// EV100 for the rest of the run; world-global, like gravity.
    SetExposure {
        ev: f32,
    },
    SetLightIntensity {
        intensity: f32,
    },
    SetEmissiveStrength {
        strength: f32,
    },
    /// Window-global.
    SetHdrOutput {
        enabled: bool,
    },
    /// Nits; window-global.
    SetPeakBrightness {
        nits: f32,
    },
    /// An environment volume by id or name, empty for this actor.
    EnableVolume {
        volume: String,
        enabled: bool,
    },
    SetVolumeWeight {
        volume: String,
        weight: f32,
    },
    CaptureProbes,
    SetShadowDistance {
        distance: f32,
    },
    SetLightShadows {
        enabled: bool,
    },
    /// Window-global.
    SetRayTracing {
        enabled: bool,
    },
    SetGiBounces {
        bounces: f32,
    },
    SetGiSamples {
        samples: f32,
    },
    /// Window-global.
    SetFogDensity {
        density: f32,
    },
    SetAurora {
        kp: f32,
    },
    StrikeLightning {
        at: [f32; 3],
    },
    SpawnDecal {
        preset: &'static str,
        values: [f32; 9],
    },
    FadeDecals {
        values: [f32; 5],
    },
    SetLightningRate {
        rate: f32,
    },
    SetWind {
        property: &'static str,
        value: f32,
    },
    SetClouds {
        property: &'static str,
        value: f32,
    },
    SetWater {
        property: &'static str,
        value: f32,
    },
    PaintTile {
        map: String,
        tile: f32,
        x: f32,
        y: f32,
        z: f32,
    },
    SetParallax {
        layer: String,
        axis: &'static str,
        value: f32,
    },
    SetCloudLayer {
        layer: f32,
        property: &'static str,
        value: f32,
    },
    SetCloudDrift {
        drift: [f32; 3],
    },
    SetBody {
        body: &'static str,
    },
    ApplyImpulse {
        impulse: [f32; 3],
    },
    SetVelocity {
        velocity: [f32; 3],
    },
    SetGravity {
        gravity: [f32; 3],
    },
    SetDensity {
        density: f32,
    },
    SetMass {
        mass: f32,
    },
    SetTrigger {
        trigger: bool,
    },
    SetCollisionLayer {
        layer: u8,
    },
    SetCollisionMask {
        mask: u8,
    },
    RumbleGamepad {
        strength: f32,
        duration: f32,
    },
    BindAction {
        action: String,
        binding: String,
    },
    ClearActionBindings {
        action: String,
    },
    Say {
        text: String,
    },
    SetVisible {
        visible: bool,
    },
    SetColor {
        color: String,
    },
    /// Starts a voice for `sound`, a project-relative asset path. `volume`
    /// is a linear gain, `pitch` a speed factor. `at` is `None` for a global
    /// voice or the id a positional one follows - already resolved, so the
    /// host never evaluates anything.
    PlaySound {
        sound: String,
        volume: f32,
        pitch: f32,
        loop_: bool,
        bus: &'static str,
        at: Option<String>,
    },
    /// Stops the voices playing `sound`. Empty stops every voice at once.
    StopSound {
        sound: String,
    },
    /// Retunes the live voices playing `sound`.
    SetSoundVolume {
        sound: String,
        volume: f32,
    },
    /// Rebends the live voices playing `sound`.
    SetSoundPitch {
        sound: String,
        pitch: f32,
    },
    /// Moves a whole mixing bus; window-global, like gravity.
    SetBusVolume {
        bus: &'static str,
        volume: f32,
    },
    SetComponentField {
        component: &'static str,
        field: &'static str,
        value: Val,
    },
    SetCameraView {
        view: &'static str,
    },
    SetCameraPitch {
        degrees: f32,
    },
    SetCameraFov {
        fov: f32,
    },
    AttachComponent {
        component: &'static str,
    },
    DetachComponent {
        component: &'static str,
    },
    /// Hangs the actor off another one, by id or name; empty takes it off.
    /// A slot rather than a fixed name, so it travels as a `String`.
    SetParent {
        target: String,
    },
    /// A copy of `of`, which the program has already given an id and its own
    /// scripts. The host's half is the entity.
    CreateClone {
        of: String,
        clone: String,
    },
    /// An actor the document never had, under an id the program minted.
    CreateActor {
        id: String,
        name: String,
        position: [f32; 3],
    },
    /// Takes `target` - an id, already resolved - out of the run.
    DeleteActor {
        target: String,
    },
    Broadcast {
        name: &'static str,
    },
    /// Loads another scene by name; `transition` is `none`, `fade`, `wipe`
    /// or `circle`. The strand that asked ends where it stands.
    SwitchScene {
        scene: String,
        transition: String,
    },
    /// Grabs or frees the pointer; window-global, like gravity.
    SetMouseLocked {
        locked: bool,
    },
    /// Makes an interface element, or updates the one `id` already names.
    /// The kind and the anchor travel as their index, the way an axis does.
    ShowElement {
        id: String,
        kind: usize,
        content: String,
        anchor: usize,
        offset: [f32; 2],
        size: [f32; 2],
        parent: String,
        /// A panel's modal, a toggle's on.
        flag: bool,
        /// A slider's ends.
        range: [f32; 2],
        /// What the widget starts at.
        value: Val,
    },
    SetUiProp {
        id: String,
        prop: &'static str,
        value: Val,
    },
    /// `all` hides everything, and the id is then empty.
    HideElement {
        id: String,
        all: bool,
    },
    DeleteElement {
        id: String,
    },
    /// Hands the keyboard to a text input; an empty id takes it back.
    SetFocus {
        id: String,
    },
    SetUiTheme {
        theme: usize,
    },
    /// Freezes or thaws the world; window-global, like gravity.
    SetPaused {
        paused: bool,
    },
    SaveVariable {
        name: &'static str,
        clear: bool,
    },
    /// Appends a number/text value to a named list. A boolean value reports
    /// itself and is dropped, the way a bad slot is.
    AddToList {
        name: &'static str,
        value: Val,
    },
    /// Removes the 1-based item at `index` from a named list.
    DeleteOfList {
        name: &'static str,
        index: f64,
    },
    /// Removes every item from a named list.
    DeleteAllOfList {
        name: &'static str,
    },
    /// Rotates a named list by `amount` positions.
    ShiftList {
        name: &'static str,
        amount: f64,
    },
    /// Inserts a number/text value at the 1-based `index` in a named list.
    InsertIntoList {
        name: &'static str,
        index: f64,
        value: Val,
    },
    /// Replaces the 1-based item at `index` in a named list.
    ReplaceItemOfList {
        name: &'static str,
        index: f64,
        value: Val,
    },
    /// Reverses a named list in place.
    ReverseList {
        name: &'static str,
    },
    /// Sets `key` in a named dict to a number/text value. A boolean value
    /// reports itself and is dropped, the way a bad slot is.
    SetDictValue {
        name: &'static str,
        key: String,
        value: Val,
    },
    /// Removes `key` from a named dict.
    DeleteDictKey {
        name: &'static str,
        key: String,
    },
    /// Removes every entry from a named dict.
    DeleteAllOfDict {
        name: &'static str,
    },
    /// Parses a JSON object and loads it into a named dict. A parse error
    /// reports itself and leaves the dict.
    LoadJsonIntoDict {
        name: &'static str,
        json: Val,
    },
    /// Parses a JSON array and loads it into a named list. A parse error
    /// reports itself and leaves the list.
    LoadJsonIntoList {
        name: &'static str,
        json: Val,
    },
}

/// One compiled strand, and what starts it. The trigger travels as text for
/// the same reason [`Act`]'s names do.
pub struct Entry {
    pub actor: &'static str,
    pub strand: &'static str,
    /// `Started`, `Key`, `Clicked`, `Collision`, `Message`, `Cloned`,
    /// `UiClicked` or `UiChanged`.
    pub trigger: &'static str,
    /// The key, the other actor or the message name; empty for the rest.
    pub detail: &'static str,
    /// Where this strand starts, in the same step numbering the VM uses.
    pub start: usize,
    /// How many `repeat` counters one run of it needs.
    pub counters: usize,
    pub run: fn(&mut dyn Host, &mut State, &mut Actors),
}

/// One live entry in a compiled run. `actor` is who is running it: the
/// entry's own actor for an authored strand, a clone's id for a copy.
struct Live {
    entry: usize,
    actor: Rc<str>,
    state: State,
}

/// Every actor a compiled run knows about: the ones the document had, and
/// the clones and creations the run has made.
///
/// The VM keeps the same table, for the same reason - `create a clone of`
/// and `delete` name an actor the way every block does, by id or by name,
/// and both have to be answerable before the host has done anything about
/// them.
pub struct Actors {
    /// Id and name, authored first and runtime ones appended.
    live: Vec<(Rc<str>, String)>,
    /// How many actors this run has made, which is where the next one's id
    /// comes from. Counted the way `Vm::new_actor_id` counts, so one project
    /// run either way makes the same ids.
    made: usize,
    /// Clone and template, waiting for `when I start as a clone` to run at
    /// the top of the next tick - the VM's `Event::Cloned` queue.
    fresh: Vec<(Rc<str>, Rc<str>)>,
    /// Ids taken out of the run. Their strands go at the end of the tick,
    /// and one that asked for its own deletion stops where it stands.
    gone: Vec<Rc<str>>,
    /// Clone id -> the authored actor it is a copy of, which is whose
    /// strands it runs.
    clones: Vec<(Rc<str>, Rc<str>)>,
    /// Whether `pause game` has the world frozen. It lives here rather than
    /// on the [`Runner`] because a generated function is handed this table
    /// and not the runner, and the VM freezes the moment the block runs -
    /// so the rest of the tick has to see it, not the host's reply.
    paused: bool,
}

impl Actors {
    fn new(names: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            live: names
                .iter()
                .map(|(id, name)| (Rc::from(*id), (*name).to_string()))
                .collect(),
            made: 0,
            fresh: Vec::new(),
            gone: Vec::new(),
            clones: Vec::new(),
            paused: false,
        }
    }

    /// Freezes or thaws the world, as `pause game` does.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Which actor `wanted` means: itself when the slot is empty or says so,
    /// then an id, then a name. The same order `Vm::find_actor` uses, so a
    /// block means the same actor whichever scheduler ran it.
    pub fn find(&self, running: &str, wanted: &str) -> Option<Rc<str>> {
        let wanted = wanted.trim();
        if wanted.is_empty()
            || wanted.eq_ignore_ascii_case("myself")
            || wanted.eq_ignore_ascii_case("me")
        {
            return Some(Rc::from(running));
        }
        if let Some((id, _)) = self.live.iter().find(|(id, _)| &**id == wanted) {
            return Some(Rc::clone(id));
        }
        self.live
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .map(|(id, _)| Rc::clone(id))
    }

    fn mint(&mut self) -> Rc<str> {
        self.made += 1;
        Rc::from(format!("~{}", self.made).as_str())
    }

    /// Registers a copy of `template` and answers its id. It shares the
    /// template's name, so a broadcast and a `when I touch` reach it too.
    pub fn clone_of(&mut self, template: &str) -> Rc<str> {
        let id = self.mint();
        self.register_clone(Rc::clone(&id), template);
        id
    }

    /// The same, for a clone something outside the program made and named -
    /// a script's, which the host hands over through `fire`.
    fn adopt(&mut self, clone: &str, template: &str) {
        if clone.is_empty() || self.live.iter().any(|(id, _)| &**id == clone) {
            return;
        }
        self.register_clone(Rc::from(clone), template);
    }

    /// Registers an actor something outside the program made, so a block
    /// can still name it. It has no strands, so nothing is scheduled.
    fn adopt_created(&mut self, actor: &str, name: &str) {
        if actor.is_empty() || self.live.iter().any(|(id, _)| &**id == actor) {
            return;
        }
        self.live.push((Rc::from(actor), name.to_string()));
    }

    fn register_clone(&mut self, id: Rc<str>, template: &str) {
        let name = self
            .live
            .iter()
            .find(|(other, _)| &**other == template)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        self.live.push((Rc::clone(&id), name));
        let root = self.template_of(template);
        self.clones.push((Rc::clone(&id), Rc::clone(&root)));
        self.fresh.push((id, root));
    }

    /// Registers an actor the document never had. It has no blocks, so
    /// nothing schedules it - only naming it has to keep working.
    pub fn create(&mut self, name: &str) -> Rc<str> {
        let id = self.mint();
        self.live.push((Rc::clone(&id), name.to_string()));
        id
    }

    /// Takes an actor out of the run.
    pub fn remove(&mut self, actor: &str) {
        self.live.retain(|(id, _)| &**id != actor);
        self.clones.retain(|(id, _)| &**id != actor);
        self.fresh.retain(|(id, _)| &**id != actor);
        if !self.is_gone(actor) {
            self.gone.push(Rc::from(actor));
        }
    }

    /// Whether `actor` has been deleted this tick, which is what ends the
    /// strand that asked.
    pub fn is_gone(&self, actor: &str) -> bool {
        self.gone.iter().any(|id| &**id == actor)
    }

    /// Whether `actor` is still in the run at all.
    fn is_live(&self, actor: &str) -> bool {
        self.live.iter().any(|(id, _)| &**id == actor)
    }

    /// The authored actor a runtime one runs the strands of. A clone of a
    /// clone is a clone of the same authored actor.
    fn template_of(&self, actor: &str) -> Rc<str> {
        self.clones
            .iter()
            .find(|(id, _)| &**id == actor)
            .map(|(_, root)| Rc::clone(root))
            .unwrap_or_else(|| Rc::from(actor))
    }

    /// Every runtime actor running an authored actor's strands: that actor,
    /// and each clone of it.
    fn copies_of(&self, authored: &str) -> Vec<Rc<str>> {
        let mut found = vec![Rc::from(authored)];
        found.extend(
            self.clones
                .iter()
                .filter(|(_, root)| &**root == authored)
                .map(|(id, _)| Rc::clone(id)),
        );
        found
    }
}

/// The scheduler held inside a compiled logic library.
pub struct Runner {
    live: Vec<Live>,
    actors: Actors,
    names: &'static [(&'static str, &'static str)],
}

impl Runner {
    pub fn new(names: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            live: Vec::new(),
            actors: Actors::new(names),
            names,
        }
    }

    pub fn reset(&mut self) {
        self.live.clear();
        self.actors = Actors::new(self.names);
    }

    /// True while any strand is still live.
    pub fn is_running(&self) -> bool {
        self.live.iter().any(|live| !live.state.done())
    }

    /// Starts every matching entry, replacing an existing run of the same
    /// strand in place as the VM does.
    ///
    /// Three kinds are the host's word about an actor rather than a trigger:
    /// `Cloned` is a copy something outside the program made - a script's
    /// `create_clone` - `Created` an actor it conjured, and `Deleted` one it
    /// took out of the run. The program's own clones, creations and deletions
    /// go straight into [`Actors`] as they happen.
    pub fn fire(
        &mut self,
        entries: &[Entry],
        kind: &str,
        actor: &str,
        detail: &str,
        other_name: &str,
    ) {
        match kind {
            "Cloned" => {
                self.actors.adopt(actor, detail);
                return;
            }
            "Created" => {
                self.actors.adopt_created(actor, detail);
                return;
            }
            "Deleted" => {
                self.actors.remove(actor);
                self.drop_deleted();
                return;
            }
            _ => {}
        }
        // A clone answers to its template's entries, so what an event means
        // is worked out against the actor those entries were written for.
        let template = self.actors.template_of(actor);
        for (index, entry) in entries.iter().enumerate() {
            let matches = match (entry.trigger, kind) {
                ("Started", "Started")
                | ("SceneStarted", "SceneStarted")
                | ("SceneEnded", "SceneEnded") => true,
                ("Key", "Key") | ("Message", "Message") => entry.detail == detail,
                ("Action", "Action") => entry.detail == detail,
                ("Touched", "Touched") => true,
                ("UiEvent", "UiEvent")
                | ("UiClicked", "UiClicked")
                | ("UiChanged", "UiChanged") => entry.detail == detail,
                ("Clicked", "Clicked") => entry.actor == &*template,
                ("Collision", "Collision") => {
                    entry.actor == &*template
                        && (entry.detail.is_empty()
                            || entry.detail == detail
                            || entry.detail.eq_ignore_ascii_case(other_name))
                }
                ("AnimationEnded", "AnimationEnded")
                | ("AnimationMarker", "AnimationMarker")
                | ("EnteredRoom", "EnteredRoom") => {
                    entry.actor == &*template
                        && (entry.detail.is_empty() || entry.detail.eq_ignore_ascii_case(detail))
                }
                ("Particles", "Particles") => entry.actor == &*template && entry.detail == detail,
                _ => false,
            };
            if !matches {
                continue;
            }
            // An event aimed at one actor starts that actor's copy of the
            // strand; a broadcast, or an interface element nobody owns,
            // starts every copy's.
            let running = match kind {
                "Clicked" | "Collision" | "AnimationEnded" | "AnimationMarker" | "Particles"
                | "EnteredRoom" => {
                    vec![Rc::from(actor)]
                }
                _ => self.actors.copies_of(entry.actor),
            };
            for id in running {
                self.begin(entries, index, id);
            }
        }
    }

    /// Starts one entry under one actor, replacing that actor's own run of
    /// the same strand.
    fn begin(&mut self, entries: &[Entry], index: usize, actor: Rc<str>) {
        // A deleted actor has no strands to start: the VM drops its program,
        // so nothing of its matches an event any more.
        if !self.actors.is_live(&actor) {
            return;
        }
        let entry = &entries[index];
        let mut state = entry.begin(&actor);
        // Escape is the pause key: while paused it starts key strands as
        // interface strands, so a pause menu can toggle itself shut.
        if self.actors.paused && entry.trigger == "Key" && entry.detail == "escape" {
            state.ui = true;
        }
        let fresh = Live {
            entry: index,
            actor: Rc::clone(&actor),
            state,
        };
        match self.live.iter_mut().find(|live| {
            let old = &entries[live.entry];
            live.actor == actor && old.strand == entry.strand
        }) {
            Some(old) => *old = fresh,
            None => self.live.push(fresh),
        }
    }

    /// Freezes or thaws the world. While it holds, only strands the
    /// interface started are given a slice - the same rule the VM keeps.
    pub fn set_paused(&mut self, paused: bool) {
        self.actors.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.actors.paused
    }

    /// Gives every live strand one slice. True means `stop all` ended the run.
    pub fn tick(&mut self, entries: &[Entry], host: &mut dyn Host, now: f64) -> bool {
        self.tick_at(entries, host, now, now)
    }

    /// The same, with the world's clock and the wall's told apart. A strand
    /// the interface started runs on `wall`, so a `wait` on a pause menu
    /// finishes while the world stands still.
    pub fn tick_at(&mut self, entries: &[Entry], host: &mut dyn Host, now: f64, wall: f64) -> bool {
        // A clone made last tick starts its own strands now, by which time
        // the host has built the actor those blocks read through.
        for (clone, template) in std::mem::take(&mut self.actors.fresh) {
            for (index, entry) in entries.iter().enumerate() {
                if entry.trigger == "Cloned" && entry.actor == &*template {
                    self.begin(entries, index, Rc::clone(&clone));
                }
            }
        }
        let mut index = 0;
        while index < self.live.len() {
            // The flag the strand started with, so one escape began while
            // paused keeps the slice that began it.
            let ui = self.live[index].state.ui;
            // A paused world advances only what the interface started.
            if self.actors.paused && !ui {
                index += 1;
                continue;
            }
            let Self { live, actors, .. } = self;
            let slice = &mut live[index];
            slice.state.now = if ui { wall } else { now };
            host.set_clock(ui);
            (entries[slice.entry].run)(host, &mut slice.state, actors);
            if slice.state.stopping {
                self.live.clear();
                return true;
            }
            index += 1;
        }
        self.live.retain(|live| !live.state.done());
        // A deleted actor's other strands go with it, wherever in the tick
        // they were - the one that ran the block already stopped itself.
        self.drop_deleted();
        false
    }

    fn drop_deleted(&mut self) {
        if self.actors.gone.is_empty() {
            return;
        }
        let gone = std::mem::take(&mut self.actors.gone);
        self.live.retain(|live| !gone.contains(&live.actor));
    }
}

impl Entry {
    /// True for a strand the interface starts. One of those keeps running
    /// while the game is paused - a pause menu's own buttons have to work -
    /// and it sleeps against the wall clock rather than the frozen one.
    pub fn is_ui(&self) -> bool {
        matches!(self.trigger, "UiEvent" | "UiClicked" | "UiChanged")
    }

    /// A fresh run of this strand under `actor`, ready for the first
    /// [`Entry::run`]. The actor is the entry's own for an authored strand
    /// and a clone's id for a copy, which is the whole of what lets one
    /// emitted function run under many actors.
    pub fn begin(&self, actor: &Rc<str>) -> State {
        let mut state = State::new(actor, self.start, self.counters);
        state.ui = self.is_ui();
        state
    }
}

/// Where a suspended strand is. The same three the VM's scripts have, so a
/// `wait` means the same thing on both sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Run,
    /// Asleep until this timestamp, in run seconds.
    Sleep(f64),
    Done,
}

/// How deeply reporter blocks may call each other, and how many steps one
/// reporter body may take before it is given up on. Both are the VM's own
/// limits; `codegen`'s tests hold these against them.
pub const MAX_REPORTER_DEPTH: usize = 32;
pub const STEP_BUDGET: usize = 10_000;

/// One custom block being run as a statement: where to carry on afterwards,
/// what its inputs were bound to, and the caller's loop tallies. Each
/// invocation keeps its own counters, the way the VM gives every call a frame
/// of its own, so a block that calls itself doesn't trample its caller.
pub struct CallFrame {
    return_pc: usize,
    params: Vec<Val>,
    counters: Vec<i64>,
    /// The caller's reporter temps, restored when this call leaves.
    temps: Vec<Val>,
    /// Where an `Invoke` stores its value in the caller, if anywhere.
    temp: Option<usize>,
}

/// One run of one strand: where it is, and what it was in the middle of.
///
/// A compiled strand is a `match` over the program counter rather than a
/// recursive walk, for the same reason the VM flattens its blocks - a nested
/// body can't be suspended, but a number can be put down and picked up.
pub struct State {
    /// Whose strand this is. An `Rc` because every reporter body this strand
    /// runs is handed the same one.
    pub me: Rc<str>,
    pub pc: usize,
    pub status: Status,
    /// Seconds since the run started. The scheduler sets it before each call.
    pub now: f64,
    /// `stop all` was reached, and the scheduler should end the whole run.
    pub stopping: bool,
    /// Whether the interface started this strand. One of those keeps running
    /// while the world is frozen, so a `pause game` inside one doesn't stop
    /// it the way it stops a world strand.
    pub ui: bool,
    /// Iterations left, one slot per `repeat` in the actor. Loop nesting is
    /// known when the code is emitted, so this is a flat array rather than
    /// the VM's frame stack - but one array per live call, saved and restored
    /// across the boundary, which is what lets a block call itself.
    counters: Vec<i64>,
    /// Reporter results waiting for the step that asked for them, one array
    /// per live call for the same reason.
    temps: Vec<Val>,
    /// Custom blocks entered as statements, innermost last.
    calls: Vec<CallFrame>,
}

impl State {
    /// A return address that means "hand the value back to whoever asked"
    /// rather than "carry on at this step": a reporter body's own boundary.
    pub const RETURN: usize = usize::MAX;

    pub fn new(me: &Rc<str>, start: usize, counters: usize) -> Self {
        Self {
            me: Rc::clone(me),
            pc: start,
            status: Status::Run,
            now: 0.0,
            stopping: false,
            ui: false,
            counters: vec![0; counters],
            temps: Vec::new(),
            calls: Vec::new(),
        }
    }

    pub fn enter_call(&mut self, return_pc: usize, params: Vec<Val>) {
        self.enter_call_with(return_pc, params, None);
    }

    /// The same, for an `Invoke`: `temp` is where the value lands in the
    /// caller when the body returns or falls off its end.
    pub fn enter_invoke(&mut self, return_pc: usize, params: Vec<Val>, temp: usize) {
        self.enter_call_with(return_pc, params, Some(temp));
    }

    fn enter_call_with(&mut self, return_pc: usize, params: Vec<Val>, temp: Option<usize>) {
        let width = self.counters.len();
        let saved_counters = std::mem::replace(&mut self.counters, vec![0; width]);
        let saved_temps = std::mem::take(&mut self.temps);
        self.calls.push(CallFrame {
            return_pc,
            params,
            counters: saved_counters,
            temps: saved_temps,
            temp,
        });
    }

    /// Leaves the innermost custom block, answering the step to carry on at.
    /// `None` ends the run instead: either nothing called this, or what did
    /// was a reporter waiting on the value.
    pub fn resume_at(&mut self) -> Option<usize> {
        self.leave_with(None)
    }

    /// The same, for a body that returns a value: an `Invoke` stores it in the
    /// caller's temp slot on its way home.
    pub fn return_with(&mut self, result: Val) -> Option<usize> {
        self.leave_with(Some(result))
    }

    fn leave_with(&mut self, result: Option<Val>) -> Option<usize> {
        match self.calls.pop() {
            Some(frame) if frame.return_pc != Self::RETURN => {
                self.counters = frame.counters;
                self.temps = frame.temps;
                if let (Some(temp), Some(value)) = (frame.temp, result) {
                    self.store_temp(temp, value);
                } else if let Some(temp) = frame.temp {
                    self.store_temp(temp, Val::Num(0.0));
                }
                Some(frame.return_pc)
            }
            Some(frame) => {
                self.counters = frame.counters;
                self.temps = frame.temps;
                None
            }
            None => None,
        }
    }

    /// One of the innermost call's bound inputs. Anything a caller didn't
    /// pass reads as zero, as an unbound name does in the VM.
    pub fn param(&self, index: usize) -> Val {
        self.calls
            .last()
            .and_then(|frame| frame.params.get(index))
            .cloned()
            .unwrap_or(Val::Num(0.0))
    }

    /// One of this invocation's reporter temps. Anything not yet invoked reads
    /// as zero, as an unbound name does in the VM.
    pub fn temp(&self, index: usize) -> Val {
        self.temps.get(index).cloned().unwrap_or(Val::Num(0.0))
    }

    pub fn store_temp(&mut self, index: usize, value: Val) {
        if self.temps.len() <= index {
            self.temps.resize(index + 1, Val::Num(0.0));
        }
        self.temps[index] = value;
    }

    /// Whether there is anything to do this frame. A sleeping strand wakes on
    /// the first frame at or past its deadline, as the VM's does.
    pub fn resume(&mut self) -> bool {
        match self.status {
            Status::Done => false,
            Status::Sleep(until) => {
                if self.now < until {
                    return false;
                }
                self.status = Status::Run;
                true
            }
            Status::Run => true,
        }
    }

    pub fn done(&self) -> bool {
        self.status == Status::Done
    }

    pub fn sleep(&mut self, seconds: f64) {
        self.status = Status::Sleep(self.now + seconds);
    }

    pub fn finish(&mut self) {
        self.status = Status::Done;
    }

    /// Enters a `repeat` with `n` iterations to go.
    pub fn enter_repeat(&mut self, loop_index: usize, n: i64) {
        self.counters[loop_index] = n;
    }

    /// Counts one iteration off, and answers how many are left.
    pub fn next_iteration(&mut self, loop_index: usize) -> i64 {
        self.counters[loop_index] -= 1;
        self.counters[loop_index]
    }
}

/// Everything a generated program can reach outside itself. Five methods
/// rather than one per verb: a new block is a new [`Act`] or a new `kind`
/// string, never a change to this trait, which is what keeps a program built
/// against an older Blockloom loadable by a newer one.
pub trait Host {
    fn act(&mut self, actor: &str, act: Act);
    /// A sensing reporter, by the name its operator is registered under.
    /// Arguments arrive evaluated, and an error comes back as one, the way
    /// the operator itself would answer.
    fn sense(&mut self, actor: &str, kind: &str, args: &[Val]) -> R;
    fn variable(&mut self, actor: &str, name: &str) -> Val;
    fn set_variable(&mut self, actor: &str, name: &str, value: Val);
    /// A value that wouldn't evaluate. The program carries on with a zero.
    fn error(&mut self, actor: &str, message: &str);
    /// Which clock the strand about to be stepped runs on - the wall's for
    /// one the interface started, the world's for everything else. Not a
    /// verb, so it has a default: a host that never pauses need not care.
    fn set_clock(&mut self, _ui: bool) {}
}

// --- Native logic boundary -------------------------------------------------

pub const LOGIC_ABI_VERSION: u32 = 28;
pub const ABI_OK: u32 = 0;
pub const ABI_TOO_LONG: u32 = 1;
pub const ABI_MISSING: u32 = 2;
pub const ABI_PANIC: u32 = 3;
pub const TICK_STOPPED: u32 = 1;

pub const VALUE_NUMBER: u32 = 0;
pub const VALUE_TEXT: u32 = 1;
pub const VALUE_BOOL: u32 = 2;
pub const VALUE_ERROR: u32 = 3;

pub const READ_SENSE: u32 = 1;
pub const READ_VARIABLE: u32 = 2;

pub const ACT_MOVE: u32 = 1;
pub const ACT_GO_TO: u32 = 2;
pub const ACT_CHANGE_POSITION: u32 = 3;
pub const ACT_GLIDE: u32 = 4;
pub const ACT_TURN: u32 = 5;
pub const ACT_SET_ROTATION: u32 = 6;
pub const ACT_POINT_TOWARDS: u32 = 7;
pub const ACT_SET_SCALE: u32 = 8;
pub const ACT_SET_BODY: u32 = 9;
pub const ACT_APPLY_IMPULSE: u32 = 10;
pub const ACT_SET_VELOCITY: u32 = 11;
pub const ACT_SET_GRAVITY: u32 = 12;
pub const ACT_SET_DENSITY: u32 = 13;
pub const ACT_SET_MASS: u32 = 14;
pub const ACT_SAY: u32 = 15;
pub const ACT_SET_VISIBLE: u32 = 16;
pub const ACT_SET_COLOR: u32 = 17;
pub const ACT_SET_FIELD: u32 = 18;
pub const ACT_SET_CAMERA_VIEW: u32 = 19;
pub const ACT_ATTACH: u32 = 20;
pub const ACT_DETACH: u32 = 21;
pub const ACT_BROADCAST: u32 = 22;
pub const ACT_ERROR: u32 = 23;
pub const ACT_SET_PARENT: u32 = 24;
pub const ACT_CREATE_CLONE: u32 = 25;
pub const ACT_CREATE_ACTOR: u32 = 26;
pub const ACT_DELETE_ACTOR: u32 = 27;
/// `n0` != 0 grabs the pointer and hides it.
pub const ACT_SET_MOUSE_LOCKED: u32 = 28;
/// `n0` = pitch in degrees; positive looks up.
pub const ACT_SET_CAMERA_PITCH: u32 = 29;
/// `n0` = vertical field of view in degrees.
pub const ACT_SET_CAMERA_FOV: u32 = 38;
/// `a` = id, `b` = content, `c` = parent, `value` = what it starts at, and
/// the numbers are kind, anchor, x, y, width, height, flag, min, max.
pub const ACT_SHOW_ELEMENT: u32 = 30;
/// `a` = id, `b` = the property's wire name, `value` = what to write.
pub const ACT_SET_UI_PROP: u32 = 31;
/// `a` = id; `n0` != 0 hides everything instead.
pub const ACT_HIDE_ELEMENT: u32 = 32;
/// `a` = id.
pub const ACT_DELETE_ELEMENT: u32 = 33;
/// `n0` != 0 freezes the world.
pub const ACT_SET_PAUSED: u32 = 34;
/// Gives a text input the keyboard, or takes it back.
pub const ACT_SET_FOCUS: u32 = 35;
/// `n0` is a `UiTheme` index.
pub const ACT_SET_UI_THEME: u32 = 36;
/// `a` = variable name; `n0` != 0 clears instead of writing.
pub const ACT_SAVE_VARIABLE: u32 = 37;
/// `n0`, `n1`, `n2` = target; `n3` = speed in units per second.
pub const ACT_NAVIGATE_TO: u32 = 39;
/// `a` = list name, `value` = what to append.
pub const ACT_LIST_ADD: u32 = 40;
/// `a` = list name; `n0` = 1-based item to remove.
pub const ACT_LIST_DELETE: u32 = 41;
/// `a` = list name.
pub const ACT_LIST_CLEAR: u32 = 42;
/// `a` = list name; `n0` = positions to rotate by.
pub const ACT_LIST_SHIFT: u32 = 43;
/// `a` = list name; `n0` = 1-based position, `value` = what to insert.
pub const ACT_LIST_INSERT: u32 = 44;
/// `a` = list name; `n0` = 1-based position, `value` = its replacement.
pub const ACT_LIST_REPLACE: u32 = 45;
/// `a` = list name.
pub const ACT_LIST_REVERSE: u32 = 46;
/// `a` = dict name, `b` = key, `value` = what to set.
pub const ACT_DICT_SET: u32 = 52;
/// `a` = dict name, `b` = key to remove.
pub const ACT_DICT_DELETE_KEY: u32 = 53;
/// `a` = dict name.
pub const ACT_DICT_CLEAR: u32 = 54;
/// `a` = dict name, `value` = JSON object text to load.
pub const ACT_JSON_TO_DICT: u32 = 55;
/// `a` = list name, `value` = JSON array text to load.
pub const ACT_JSON_TO_LIST: u32 = 56;
/// `a` = asset path; `n0` = linear gain, `n1` = pitch; `n2` != 0 loops;
/// `b` = bus name; `c` = followed actor id, or empty for a global voice.
pub const ACT_PLAY_SOUND: u32 = 47;
/// `a` = asset path; empty stops every voice at once.
pub const ACT_STOP_SOUND: u32 = 48;
/// `a` = asset path; `n0` = linear gain.
pub const ACT_SET_SOUND_VOLUME: u32 = 49;
/// `a` = asset path; `n0` = pitch.
pub const ACT_SET_SOUND_PITCH: u32 = 50;
/// `b` = bus name; `n0` = linear gain. Window-global, like gravity.
pub const ACT_SET_BUS_VOLUME: u32 = 51;
/// `n0` != 0 senses overlap without pushing back.
pub const ACT_SET_TRIGGER: u32 = 57;
/// `n0` = layer 1-8.
pub const ACT_SET_COLLISION_LAYER: u32 = 58;
/// `n0` = bitmask of the layers the actor pairs with.
pub const ACT_SET_COLLISION_MASK: u32 = 59;
/// `n0` = count, capped by the emitter and global pools.
pub const ACT_BURST_PARTICLES: u32 = 64;
/// `a` = emitter dial name, `n0` = value.
pub const ACT_SET_EMITTER_DIAL: u32 = 65;
/// `n0` != 0 records trail snapshots.
pub const ACT_SET_TRAIL_ENABLED: u32 = 66;
/// `n0` = strength 0-100, `n1` = seconds. Window-global: no actor.
pub const ACT_RUMBLE_GAMEPAD: u32 = 60;
/// `a` = action, `b` = binding text.
pub const ACT_BIND_ACTION: u32 = 61;
/// `a` = action.
pub const ACT_CLEAR_ACTION_BINDINGS: u32 = 62;
/// `n0` = EV100. Window-global: no actor.
pub const ACT_SET_EXPOSURE: u32 = 67;
/// `n0` = lumens.
pub const ACT_SET_LIGHT_INTENSITY: u32 = 68;
/// A position tween; `a` = easing name, numbers are seconds, x, y, and the
/// value is z. The strand sleeps for exactly as long.
pub const ACT_TWEEN_GLIDE_EASING: &str = "easing";
/// `n0` = factor, `n1` = seconds; `a` = easing name.
pub const ACT_TWEEN_SCALE: u32 = 69;
/// `n0` = axis, `n1` = degrees, `n2` = seconds; `a` = easing name.
pub const ACT_TWEEN_ROTATION: u32 = 70;
/// `a` = `#RRGGBB`, `b` = easing name; `n0` = seconds.
pub const ACT_TWEEN_COLOR: u32 = 71;
/// No numbers: stops every tween on the actor where it stands.
pub const ACT_STOP_TWEENS: u32 = 72;
/// `a` = clip name; `n0` = speed.
pub const ACT_PLAY_ANIMATION: u32 = 73;
/// No numbers: stops the animation player, keeping the frame.
pub const ACT_STOP_ANIMATION: u32 = 74;
/// `n0` = speed. 1 is as authored, 0 freezes.
pub const ACT_SET_ANIMATION_SPEED: u32 = 75;
/// `a` = trigger name, for the animation state machine this tick.
pub const ACT_FIRE_ANIMATION_TRIGGER: u32 = 96;
/// `a` = rig slot, `b` = attachment; empty hides the slot.
pub const ACT_SET_RIG_SLOT: u32 = 97;
/// `a` = rig slot, `b` = `#RRGGBB`.
pub const ACT_SET_SLOT_TINT: u32 = 98;
/// `a` = IK constraint; `n0`, `n1` = where, relative to the actor.
pub const ACT_SET_IK_TARGET: u32 = 99;
/// `a` = sprite dial name, `n0` = value.
pub const ACT_SET_SPRITE_DIAL: u32 = 100;
/// `n0` = multiple of the emissive tint.
pub const ACT_SET_EMISSIVE_STRENGTH: u32 = 76;
/// `n0` != 0 turns HDR output on. Window-global: no actor.
pub const ACT_SET_HDR_OUTPUT: u32 = 77;
/// `n0` = nits. Window-global: no actor.
pub const ACT_SET_PEAK_BRIGHTNESS: u32 = 78;
/// `a` = volume by id or name, empty for this actor; `n0` != 0 turns it on.
pub const ACT_ENABLE_VOLUME: u32 = 79;
/// `a` = volume by id or name, empty for this actor; `n0` = weight 0-1.
pub const ACT_SET_VOLUME_WEIGHT: u32 = 80;
/// Window-global: no actor.
pub const ACT_CAPTURE_PROBES: u32 = 81;
/// `n0` = metres. Window-global: no actor.
pub const ACT_SET_SHADOW_DISTANCE: u32 = 82;
/// `n0` != 0 turns the actor's light's shadows on.
pub const ACT_SET_LIGHT_SHADOWS: u32 = 83;
/// `n0` != 0 turns ray-traced lighting on. Window-global: no actor.
pub const ACT_SET_RAY_TRACING: u32 = 84;
/// `n0` = most bounces. Window-global: no actor.
pub const ACT_SET_GI_BOUNCES: u32 = 85;
/// `n0` = samples per pixel. Window-global: no actor.
pub const ACT_SET_GI_SAMPLES: u32 = 86;
/// `n0` = extinction per metre. Window-global: no actor.
pub const ACT_SET_FOG_DENSITY: u32 = 87;
/// `n0` = KP index 0-9. Window-global: no actor.
pub const ACT_SET_AURORA: u32 = 88;
/// `n0..n2` = where it lands. Window-global: no actor.
pub const ACT_STRIKE_LIGHTNING: u32 = 89;
/// `n0` = strikes a minute. Window-global: no actor.
pub const ACT_SET_LIGHTNING_RATE: u32 = 90;
/// `a` = wind dial (`Direction`, `Speed`, `Gust`, `Storm`); `n0` = value.
/// Window-global: no actor.
pub const ACT_SET_WIND: u32 = 91;
/// `n0..n2` = extra cloud drift per second. Window-global: no actor.
pub const ACT_SET_CLOUD_DRIFT: u32 = 92;
/// `a` = cloud dial (`Coverage`, `Density`, `Type`); `n0` = value.
/// Window-global: no actor.
pub const ACT_SET_CLOUDS: u32 = 93;
/// `a` = cloud layer dial (`Coverage`, `Opacity`, ...); `n0` = layer from 1,
/// `n1` = value. Window-global: no actor.
pub const ACT_SET_CLOUD_LAYER: u32 = 94;
/// `a` = water dial (`Level`, `Chop`, `Foam`); `n0` = value. The actor's own
/// water when it has some, every body's otherwise.
pub const ACT_SET_WATER: u32 = 95;
/// `n0` != 0 lets the emitter spawn.
pub const ACT_SET_EMITTER_PLAYING: u32 = 101;
/// `a` = tilemap actor (empty for this actor, or whichever map covers the
/// point); `n0` = tile, `n1`/`n2`/`n3` = world x/y/z.
pub const ACT_PAINT_TILE: u32 = 102;
/// `a` = layer actor, `b` = axis (`Both`, `X`, `Y`); `n0` = scroll factor.
pub const ACT_SET_PARALLAX: u32 = 103;
/// `a` = preset; numbers = position, normal, size, lifetime, fade.
pub const ACT_SPAWN_DECAL: u32 = 104;
/// Numbers = centre x/y/z, radius, fade seconds. Window-global.
pub const ACT_FADE_DECALS: u32 = 105;
/// `a` = scene name, `b` = transition (`none`, `fade`, `wipe`, `circle`).
pub const ACT_SWITCH_SCENE: u32 = 106;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl AbiStr {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn borrow(value: &str) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }

    /// # Safety
    /// The pointer must name UTF-8 for `len` bytes for this call.
    pub unsafe fn as_str<'a>(self) -> &'a str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        std::str::from_utf8(bytes).unwrap_or("")
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiValue {
    pub kind: u32,
    pub number: f64,
    pub text: AbiStr,
}

impl AbiValue {
    fn borrow(value: &Val) -> Self {
        match value {
            Val::Num(number) => Self {
                kind: VALUE_NUMBER,
                number: *number,
                text: AbiStr::EMPTY,
            },
            Val::Text(text) => Self {
                kind: VALUE_TEXT,
                number: 0.0,
                text: AbiStr::borrow(text),
            },
            Val::Bool(value) => Self {
                kind: VALUE_BOOL,
                number: if *value { 1.0 } else { 0.0 },
                text: AbiStr::EMPTY,
            },
        }
    }
}

#[repr(C)]
pub struct LogicHostApi {
    pub abi: u32,
    pub read: extern "C" fn(
        *mut std::ffi::c_void,
        AbiStr,
        u32,
        AbiStr,
        *const AbiValue,
        usize,
        *mut AbiValue,
        *mut u8,
        usize,
        *mut usize,
    ) -> u32,
    pub set_variable: extern "C" fn(*mut std::ffi::c_void, AbiStr, AbiStr, AbiValue),
    /// Three strings and a run of numbers rather than a fixed three, because
    /// one interface element names nine of them at once.
    pub act: extern "C" fn(
        *mut std::ffi::c_void,
        AbiStr,
        u32,
        AbiStr,
        AbiStr,
        AbiStr,
        *const f64,
        usize,
        AbiValue,
    ),
    /// Which clock the strand about to run keeps - see [`Host::set_clock`].
    pub set_clock: extern "C" fn(*mut std::ffi::c_void, u32),
}

/// A generated program's safe view of the runtime callbacks.
pub struct AbiHost {
    ctx: *mut std::ffi::c_void,
    api: *const LogicHostApi,
}

impl AbiHost {
    /// # Safety
    /// Both pointers are supplied by the runtime for the duration of a call.
    pub unsafe fn new(ctx: *mut std::ffi::c_void, api: *const LogicHostApi) -> Self {
        Self { ctx, api }
    }

    fn api(&self) -> &LogicHostApi {
        unsafe { &*self.api }
    }

    fn read_value(&mut self, actor: &str, what: u32, name: &str, args: &[Val]) -> R {
        let wire: Vec<AbiValue> = args.iter().map(AbiValue::borrow).collect();
        let mut value = AbiValue {
            kind: VALUE_NUMBER,
            number: 0.0,
            text: AbiStr::EMPTY,
        };
        let mut needed = 0;
        let mut text = vec![0_u8; 256];
        loop {
            let status = (self.api().read)(
                self.ctx,
                AbiStr::borrow(actor),
                what,
                AbiStr::borrow(name),
                wire.as_ptr(),
                wire.len(),
                &mut value,
                text.as_mut_ptr(),
                text.len(),
                &mut needed,
            );
            if status == ABI_TOO_LONG {
                text.resize(needed, 0);
                continue;
            }
            if status == ABI_MISSING {
                return Ok(Val::Num(0.0));
            }
            if status != ABI_OK {
                return Err("compiled logic host failed to answer".to_string());
            }
            return match value.kind {
                VALUE_NUMBER => Ok(Val::Num(value.number)),
                VALUE_BOOL => Ok(Val::Bool(value.number != 0.0)),
                VALUE_TEXT => Ok(Val::Text(
                    String::from_utf8_lossy(&text[..needed]).into_owned(),
                )),
                VALUE_ERROR => Err(String::from_utf8_lossy(&text[..needed]).into_owned()),
                _ => Err("compiled logic host returned an invalid value".to_string()),
            };
        }
    }

    fn act_wire(
        &mut self,
        actor: &str,
        kind: u32,
        a: &str,
        b: &str,
        numbers: [f64; 3],
        value: &Val,
    ) {
        self.act_many(actor, kind, a, b, "", &numbers, value);
    }

    #[allow(clippy::too_many_arguments)]
    fn act_many(
        &mut self,
        actor: &str,
        kind: u32,
        a: &str,
        b: &str,
        c: &str,
        numbers: &[f64],
        value: &Val,
    ) {
        (self.api().act)(
            self.ctx,
            AbiStr::borrow(actor),
            kind,
            AbiStr::borrow(a),
            AbiStr::borrow(b),
            AbiStr::borrow(c),
            numbers.as_ptr(),
            numbers.len(),
            AbiValue::borrow(value),
        );
    }
}

impl Host for AbiHost {
    fn act(&mut self, actor: &str, act: Act) {
        let zero = Val::Num(0.0);
        match act {
            Act::Move { steps } => {
                self.act_wire(actor, ACT_MOVE, "", "", [steps as f64, 0.0, 0.0], &zero)
            }
            Act::GoTo { position } => {
                self.act_wire(actor, ACT_GO_TO, "", "", position.map(f64::from), &zero)
            }
            Act::NavigateTo { target, speed } => self.act_many(
                actor,
                ACT_NAVIGATE_TO,
                "",
                "",
                "",
                &[
                    target[0] as f64,
                    target[1] as f64,
                    target[2] as f64,
                    speed as f64,
                ],
                &zero,
            ),
            Act::BurstParticles { count } => self.act_wire(
                actor,
                ACT_BURST_PARTICLES,
                "",
                "",
                [count as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetEmitterDial { dial, value } => self.act_wire(
                actor,
                ACT_SET_EMITTER_DIAL,
                dial,
                "",
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetTrailEnabled { enabled } => self.act_wire(
                actor,
                ACT_SET_TRAIL_ENABLED,
                "",
                "",
                [f64::from(enabled as u8), 0.0, 0.0],
                &zero,
            ),
            Act::SetEmitterPlaying { playing } => self.act_wire(
                actor,
                ACT_SET_EMITTER_PLAYING,
                "",
                "",
                [f64::from(playing as u8), 0.0, 0.0],
                &zero,
            ),
            Act::ChangePosition { axis, by } => self.act_wire(
                actor,
                ACT_CHANGE_POSITION,
                "",
                "",
                [axis as f64, by as f64, 0.0],
                &zero,
            ),
            Act::Glide {
                seconds,
                target,
                easing,
            } => self.act_wire(
                actor,
                ACT_GLIDE,
                easing,
                "",
                [seconds as f64, target[0] as f64, target[1] as f64],
                &Val::Num(target[2] as f64),
            ),
            Act::TweenScale {
                factor,
                seconds,
                easing,
            } => self.act_wire(
                actor,
                ACT_TWEEN_SCALE,
                easing,
                "",
                [factor as f64, seconds as f64, 0.0],
                &zero,
            ),
            Act::TweenRotation {
                axis,
                degrees,
                seconds,
                easing,
            } => self.act_wire(
                actor,
                ACT_TWEEN_ROTATION,
                easing,
                "",
                [axis as f64, degrees as f64, seconds as f64],
                &zero,
            ),
            Act::TweenColor {
                color,
                seconds,
                easing,
            } => self.act_wire(
                actor,
                ACT_TWEEN_COLOR,
                &color,
                easing,
                [seconds as f64, 0.0, 0.0],
                &zero,
            ),
            Act::StopTweens => self.act_wire(actor, ACT_STOP_TWEENS, "", "", [0.0; 3], &zero),
            Act::PlayAnimation { clip, speed } => self.act_wire(
                actor,
                ACT_PLAY_ANIMATION,
                &clip,
                "",
                [speed as f64, 0.0, 0.0],
                &zero,
            ),
            Act::StopAnimation => self.act_wire(actor, ACT_STOP_ANIMATION, "", "", [0.0; 3], &zero),
            Act::SetAnimationSpeed { speed } => self.act_wire(
                actor,
                ACT_SET_ANIMATION_SPEED,
                "",
                "",
                [speed as f64, 0.0, 0.0],
                &zero,
            ),
            Act::FireAnimationTrigger { name } => self.act_wire(
                actor,
                ACT_FIRE_ANIMATION_TRIGGER,
                &name,
                "",
                [0.0; 3],
                &zero,
            ),
            Act::SetRigSlot { slot, attachment } => {
                self.act_wire(actor, ACT_SET_RIG_SLOT, &slot, &attachment, [0.0; 3], &zero)
            }
            Act::SetSlotTint { slot, color } => {
                self.act_wire(actor, ACT_SET_SLOT_TINT, &slot, &color, [0.0; 3], &zero)
            }
            Act::SetIkTarget { constraint, x, y } => self.act_wire(
                actor,
                ACT_SET_IK_TARGET,
                &constraint,
                "",
                [x as f64, y as f64, 0.0],
                &zero,
            ),
            Act::SetSpriteDial { dial, value } => self.act_wire(
                actor,
                ACT_SET_SPRITE_DIAL,
                dial,
                "",
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::Turn { axis, degrees } => self.act_wire(
                actor,
                ACT_TURN,
                "",
                "",
                [axis as f64, degrees as f64, 0.0],
                &zero,
            ),
            Act::SetRotation { axis, degrees } => self.act_wire(
                actor,
                ACT_SET_ROTATION,
                "",
                "",
                [axis as f64, degrees as f64, 0.0],
                &zero,
            ),
            Act::PointTowards { target } => {
                self.act_wire(actor, ACT_POINT_TOWARDS, target, "", [0.0; 3], &zero)
            }
            Act::SetScale { factor } => self.act_wire(
                actor,
                ACT_SET_SCALE,
                "",
                "",
                [factor as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetExposure { ev } => self.act_wire(
                actor,
                ACT_SET_EXPOSURE,
                "",
                "",
                [ev as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetLightIntensity { intensity } => self.act_wire(
                actor,
                ACT_SET_LIGHT_INTENSITY,
                "",
                "",
                [intensity as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetEmissiveStrength { strength } => self.act_wire(
                actor,
                ACT_SET_EMISSIVE_STRENGTH,
                "",
                "",
                [strength as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetHdrOutput { enabled } => self.act_wire(
                actor,
                ACT_SET_HDR_OUTPUT,
                "",
                "",
                [if enabled { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetPeakBrightness { nits } => self.act_wire(
                actor,
                ACT_SET_PEAK_BRIGHTNESS,
                "",
                "",
                [nits as f64, 0.0, 0.0],
                &zero,
            ),
            Act::EnableVolume { volume, enabled } => self.act_wire(
                actor,
                ACT_ENABLE_VOLUME,
                &volume,
                "",
                [if enabled { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetVolumeWeight { volume, weight } => self.act_wire(
                actor,
                ACT_SET_VOLUME_WEIGHT,
                &volume,
                "",
                [weight as f64, 0.0, 0.0],
                &zero,
            ),
            Act::CaptureProbes => self.act_wire(actor, ACT_CAPTURE_PROBES, "", "", [0.0; 3], &zero),
            Act::SetShadowDistance { distance } => self.act_wire(
                actor,
                ACT_SET_SHADOW_DISTANCE,
                "",
                "",
                [distance as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetLightShadows { enabled } => self.act_wire(
                actor,
                ACT_SET_LIGHT_SHADOWS,
                "",
                "",
                [if enabled { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetRayTracing { enabled } => self.act_wire(
                actor,
                ACT_SET_RAY_TRACING,
                "",
                "",
                [if enabled { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetGiBounces { bounces } => self.act_wire(
                actor,
                ACT_SET_GI_BOUNCES,
                "",
                "",
                [bounces as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetGiSamples { samples } => self.act_wire(
                actor,
                ACT_SET_GI_SAMPLES,
                "",
                "",
                [samples as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetFogDensity { density } => self.act_wire(
                actor,
                ACT_SET_FOG_DENSITY,
                "",
                "",
                [density as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetAurora { kp } => {
                self.act_wire(actor, ACT_SET_AURORA, "", "", [kp as f64, 0.0, 0.0], &zero)
            }
            Act::SpawnDecal { preset, values } => {
                self.act_many(
                    actor,
                    ACT_SPAWN_DECAL,
                    preset,
                    "",
                    "",
                    &values.map(f64::from),
                    &zero,
                );
            }
            Act::FadeDecals { values } => {
                self.act_many(
                    actor,
                    ACT_FADE_DECALS,
                    "",
                    "",
                    "",
                    &values.map(f64::from),
                    &zero,
                );
            }
            Act::StrikeLightning { at } => self.act_wire(
                actor,
                ACT_STRIKE_LIGHTNING,
                "",
                "",
                at.map(f64::from),
                &zero,
            ),
            Act::SetLightningRate { rate } => self.act_wire(
                actor,
                ACT_SET_LIGHTNING_RATE,
                "",
                "",
                [rate as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetWind { property, value } => self.act_wire(
                actor,
                ACT_SET_WIND,
                property,
                "",
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetClouds { property, value } => self.act_wire(
                actor,
                ACT_SET_CLOUDS,
                property,
                "",
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetWater { property, value } => self.act_wire(
                actor,
                ACT_SET_WATER,
                property,
                "",
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::PaintTile { map, tile, x, y, z } => self.act_many(
                actor,
                ACT_PAINT_TILE,
                &map,
                "",
                "",
                &[tile as f64, x as f64, y as f64, z as f64],
                &zero,
            ),
            Act::SetParallax { layer, axis, value } => self.act_wire(
                actor,
                ACT_SET_PARALLAX,
                &layer,
                axis,
                [value as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetCloudLayer {
                layer,
                property,
                value,
            } => self.act_wire(
                actor,
                ACT_SET_CLOUD_LAYER,
                property,
                "",
                [layer as f64, value as f64, 0.0],
                &zero,
            ),
            Act::SetCloudDrift { drift } => self.act_wire(
                actor,
                ACT_SET_CLOUD_DRIFT,
                "",
                "",
                drift.map(f64::from),
                &zero,
            ),
            Act::SetBody { body } => self.act_wire(actor, ACT_SET_BODY, body, "", [0.0; 3], &zero),
            Act::ApplyImpulse { impulse } => self.act_wire(
                actor,
                ACT_APPLY_IMPULSE,
                "",
                "",
                impulse.map(f64::from),
                &zero,
            ),
            Act::SetVelocity { velocity } => self.act_wire(
                actor,
                ACT_SET_VELOCITY,
                "",
                "",
                velocity.map(f64::from),
                &zero,
            ),
            Act::SetGravity { gravity } => self.act_wire(
                actor,
                ACT_SET_GRAVITY,
                "",
                "",
                gravity.map(f64::from),
                &zero,
            ),
            Act::SetDensity { density } => self.act_wire(
                actor,
                ACT_SET_DENSITY,
                "",
                "",
                [density as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetMass { mass } => {
                self.act_wire(actor, ACT_SET_MASS, "", "", [mass as f64, 0.0, 0.0], &zero)
            }
            Act::SetTrigger { trigger } => self.act_wire(
                actor,
                ACT_SET_TRIGGER,
                "",
                "",
                [if trigger { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetCollisionLayer { layer } => self.act_wire(
                actor,
                ACT_SET_COLLISION_LAYER,
                "",
                "",
                [layer as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetCollisionMask { mask } => self.act_wire(
                actor,
                ACT_SET_COLLISION_MASK,
                "",
                "",
                [mask as f64, 0.0, 0.0],
                &zero,
            ),
            Act::RumbleGamepad { strength, duration } => self.act_wire(
                actor,
                ACT_RUMBLE_GAMEPAD,
                "",
                "",
                [strength as f64, duration as f64, 0.0],
                &zero,
            ),
            Act::BindAction { action, binding } => {
                self.act_wire(actor, ACT_BIND_ACTION, &action, &binding, [0.0; 3], &zero)
            }
            Act::ClearActionBindings { action } => self.act_wire(
                actor,
                ACT_CLEAR_ACTION_BINDINGS,
                &action,
                "",
                [0.0; 3],
                &zero,
            ),
            Act::Say { text } => self.act_wire(actor, ACT_SAY, &text, "", [0.0; 3], &zero),
            Act::SetVisible { visible } => self.act_wire(
                actor,
                ACT_SET_VISIBLE,
                "",
                "",
                [if visible { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SetColor { color } => {
                self.act_wire(actor, ACT_SET_COLOR, &color, "", [0.0; 3], &zero)
            }
            Act::PlaySound {
                sound,
                volume,
                pitch,
                loop_,
                bus,
                at,
            } => self.act_many(
                actor,
                ACT_PLAY_SOUND,
                &sound,
                bus,
                at.as_deref().unwrap_or(""),
                &[volume as f64, pitch as f64, if loop_ { 1.0 } else { 0.0 }],
                &zero,
            ),
            Act::StopSound { sound } => {
                self.act_wire(actor, ACT_STOP_SOUND, &sound, "", [0.0; 3], &zero)
            }
            Act::SetSoundVolume { sound, volume } => self.act_wire(
                actor,
                ACT_SET_SOUND_VOLUME,
                &sound,
                "",
                [volume as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetSoundPitch { sound, pitch } => self.act_wire(
                actor,
                ACT_SET_SOUND_PITCH,
                &sound,
                "",
                [pitch as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetBusVolume { bus, volume } => self.act_wire(
                actor,
                ACT_SET_BUS_VOLUME,
                "",
                bus,
                [volume as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetComponentField {
                component,
                field,
                value,
            } => self.act_wire(actor, ACT_SET_FIELD, component, field, [0.0; 3], &value),
            Act::SetCameraView { view } => {
                self.act_wire(actor, ACT_SET_CAMERA_VIEW, view, "", [0.0; 3], &zero)
            }
            Act::SetCameraPitch { degrees } => self.act_wire(
                actor,
                ACT_SET_CAMERA_PITCH,
                "",
                "",
                [degrees as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetCameraFov { fov } => self.act_wire(
                actor,
                ACT_SET_CAMERA_FOV,
                "",
                "",
                [fov as f64, 0.0, 0.0],
                &zero,
            ),
            Act::AttachComponent { component } => {
                self.act_wire(actor, ACT_ATTACH, component, "", [0.0; 3], &zero)
            }
            Act::DetachComponent { component } => {
                self.act_wire(actor, ACT_DETACH, component, "", [0.0; 3], &zero)
            }
            Act::SetParent { target } => {
                self.act_wire(actor, ACT_SET_PARENT, &target, "", [0.0; 3], &zero)
            }
            Act::CreateClone { of, clone } => {
                self.act_wire(actor, ACT_CREATE_CLONE, &of, &clone, [0.0; 3], &zero)
            }
            Act::CreateActor { id, name, position } => self.act_wire(
                actor,
                ACT_CREATE_ACTOR,
                &id,
                &name,
                position.map(f64::from),
                &zero,
            ),
            Act::DeleteActor { target } => {
                self.act_wire(actor, ACT_DELETE_ACTOR, &target, "", [0.0; 3], &zero)
            }
            Act::Broadcast { name } => {
                self.act_wire(actor, ACT_BROADCAST, name, "", [0.0; 3], &zero)
            }
            Act::SetMouseLocked { locked } => self.act_wire(
                actor,
                ACT_SET_MOUSE_LOCKED,
                "",
                "",
                [if locked { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::ShowElement {
                id,
                kind,
                content,
                anchor,
                offset,
                size,
                parent,
                flag,
                range,
                value,
            } => self.act_many(
                actor,
                ACT_SHOW_ELEMENT,
                &id,
                &content,
                &parent,
                &[
                    kind as f64,
                    anchor as f64,
                    offset[0] as f64,
                    offset[1] as f64,
                    size[0] as f64,
                    size[1] as f64,
                    if flag { 1.0 } else { 0.0 },
                    range[0] as f64,
                    range[1] as f64,
                ],
                &value,
            ),
            Act::SetUiProp { id, prop, value } => {
                self.act_many(actor, ACT_SET_UI_PROP, &id, prop, "", &[], &value)
            }
            Act::HideElement { id, all } => self.act_wire(
                actor,
                ACT_HIDE_ELEMENT,
                &id,
                "",
                [if all { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::DeleteElement { id } => {
                self.act_wire(actor, ACT_DELETE_ELEMENT, &id, "", [0.0; 3], &zero)
            }
            Act::SetFocus { id } => self.act_wire(actor, ACT_SET_FOCUS, &id, "", [0.0; 3], &zero),
            Act::SetUiTheme { theme } => self.act_wire(
                actor,
                ACT_SET_UI_THEME,
                "",
                "",
                [theme as f64, 0.0, 0.0],
                &zero,
            ),
            Act::SetPaused { paused } => self.act_wire(
                actor,
                ACT_SET_PAUSED,
                "",
                "",
                [if paused { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::SaveVariable { name, clear } => self.act_wire(
                actor,
                ACT_SAVE_VARIABLE,
                name,
                "",
                [if clear { 1.0 } else { 0.0 }, 0.0, 0.0],
                &zero,
            ),
            Act::AddToList { name, value } => {
                self.act_wire(actor, ACT_LIST_ADD, name, "", [0.0; 3], &value)
            }
            Act::DeleteOfList { name, index } => {
                self.act_wire(actor, ACT_LIST_DELETE, name, "", [index, 0.0, 0.0], &zero)
            }
            Act::DeleteAllOfList { name } => {
                self.act_wire(actor, ACT_LIST_CLEAR, name, "", [0.0; 3], &zero)
            }
            Act::ShiftList { name, amount } => {
                self.act_wire(actor, ACT_LIST_SHIFT, name, "", [amount, 0.0, 0.0], &zero)
            }
            Act::InsertIntoList { name, index, value } => {
                self.act_wire(actor, ACT_LIST_INSERT, name, "", [index, 0.0, 0.0], &value)
            }
            Act::ReplaceItemOfList { name, index, value } => {
                self.act_wire(actor, ACT_LIST_REPLACE, name, "", [index, 0.0, 0.0], &value)
            }
            Act::ReverseList { name } => {
                self.act_wire(actor, ACT_LIST_REVERSE, name, "", [0.0; 3], &zero)
            }
            Act::SetDictValue { name, key, value } => {
                self.act_wire(actor, ACT_DICT_SET, name, &key, [0.0; 3], &value)
            }
            Act::DeleteDictKey { name, key } => {
                self.act_wire(actor, ACT_DICT_DELETE_KEY, name, &key, [0.0; 3], &zero)
            }
            Act::DeleteAllOfDict { name } => {
                self.act_wire(actor, ACT_DICT_CLEAR, name, "", [0.0; 3], &zero)
            }
            Act::LoadJsonIntoDict { name, json } => {
                self.act_wire(actor, ACT_JSON_TO_DICT, name, "", [0.0; 3], &json)
            }
            Act::LoadJsonIntoList { name, json } => {
                self.act_wire(actor, ACT_JSON_TO_LIST, name, "", [0.0; 3], &json)
            }
            Act::SwitchScene { scene, transition } => {
                self.act_wire(actor, ACT_SWITCH_SCENE, &scene, &transition, [0.0; 3], &zero)
            }
        }
    }

    fn sense(&mut self, actor: &str, kind: &str, args: &[Val]) -> R {
        self.read_value(actor, READ_SENSE, kind, args)
    }

    fn variable(&mut self, actor: &str, name: &str) -> Val {
        self.read_value(actor, READ_VARIABLE, name, &[])
            .unwrap_or(Val::Num(0.0))
    }

    fn set_variable(&mut self, actor: &str, name: &str, value: Val) {
        (self.api().set_variable)(
            self.ctx,
            AbiStr::borrow(actor),
            AbiStr::borrow(name),
            AbiValue::borrow(&value),
        );
    }

    fn error(&mut self, actor: &str, message: &str) {
        self.act_wire(actor, ACT_ERROR, message, "", [0.0; 3], &Val::Num(0.0));
    }

    fn set_clock(&mut self, ui: bool) {
        (self.api().set_clock)(self.ctx, u32::from(ui));
    }
}

pub const SYM_LOGIC_ABI: &[u8] = b"blockloom_logic_abi";
pub const SYM_LOGIC_NEW: &[u8] = b"blockloom_logic_new";
pub const SYM_LOGIC_FREE: &[u8] = b"blockloom_logic_free";
pub const SYM_LOGIC_RESET: &[u8] = b"blockloom_logic_reset";
pub const SYM_LOGIC_FIRE: &[u8] = b"blockloom_logic_fire";
pub const SYM_LOGIC_TICK: &[u8] = b"blockloom_logic_tick";
pub const SYM_LOGIC_PAUSE: &[u8] = b"blockloom_logic_pause";

// ─── Reading a value at an instruction's slot ───────────────────────────────
// The VM reports a bad slot once and stands a zero in its place; a slot that
// evaluated fine but isn't a number is simply zero, with nothing reported.
// Both halves of that matter, so each has a function here.

/// What a fresh element of one kind reports before anybody has touched it.
/// The same rule `ui::UiElement::initial_value` keeps, and for the same
/// reason: `value of (id)` must mean one thing whichever scheduler ran the
/// `show` block. The kind arrives as its index, as everything else does here.
pub fn ui_value(kind: usize, value: Val) -> Val {
    match kind {
        // Toggle, Slider, Button; everything else reports its own words.
        6 => Val::Bool(value.as_bool()),
        5 | 15..=20 => Val::Num(value.as_number().unwrap_or(0.0)),
        2 => Val::Bool(false),
        _ => Val::Text(value.as_text()),
    }
}

/// What a fresh element of a kind whose row has no value slot starts at.
/// `ui::UiElement::blank`'s rule, for the same reason - only a toggle
/// carries a starting state, and a new text input is empty, not the word
/// "false".
pub fn ui_blank(kind: usize, flag: bool) -> Val {
    match kind {
        6 => Val::Bool(flag),
        5 | 15..=20 => Val::Num(0.0),
        2 => Val::Bool(false),
        _ => Val::Text(String::new()),
    }
}

/// A Scratch-style 1-based list position as a vector offset. An insert may
/// target the slot just past the final item; every other list command needs
/// an item that is already there. The same rule blockstitch's `list_index`
/// keeps for the VM, so the two halves agree about every edge.
pub fn list_index_pos(index: f64, len: usize, allow_end: bool) -> Option<usize> {
    if !index.is_finite() || index < 1.0 {
        return None;
    }
    let at = index.floor() as usize - 1;
    if at < len || (allow_end && at == len) {
        Some(at)
    } else {
        None
    }
}

pub fn number(host: &mut dyn Host, actor: &str, value: R) -> f32 {
    number_f64(host, actor, value) as f32
}

pub fn number_f64(host: &mut dyn Host, actor: &str, value: R) -> f64 {
    match value {
        Ok(value) => value.as_number().unwrap_or(0.0),
        Err(message) => {
            host.error(actor, &message);
            0.0
        }
    }
}

/// A reporter block nested too deeply to run. The complaint is the VM's, word
/// for word, and the call reads as zero.
pub fn too_deep(host: &mut dyn Host, actor: &str) -> Val {
    host.error(actor, "a reporter block calls itself too deeply");
    Val::Num(0.0)
}

pub fn text(host: &mut dyn Host, actor: &str, value: R) -> String {
    evaluated(host, actor, value).as_text()
}

/// A 0-100 block volume into the linear gain an [`Act`] carries. Mirrors
/// `blockloom_core::sound::user_to_gain`, which the VM calls instead -
/// duplicated here because this file is pasted whole into generated programs
/// that have no dependencies.
pub fn sound_gain(volume: f64) -> f32 {
    (volume as f32 / 100.0).clamp(0.0, 2.0)
}

/// A pitch slot into the playable range. Mirrors
/// `blockloom_core::sound::clamp_pitch`, for the same reason.
pub fn sound_pitch(pitch: f32) -> f32 {
    if !pitch.is_finite() {
        return 1.0;
    }
    pitch.clamp(0.125, 4.0)
}

/// A scene transition by the name a block spells it. Unknown spellings read
/// as `none`, the same rule the VM keeps in `vm::normalize_transition`.
pub fn normalize_transition(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            continue;
        }
        for l in c.to_lowercase() {
            key.push(l);
        }
    }
    match key.as_str() {
        "fade" => "fade".to_string(),
        "wipe" => "wipe".to_string(),
        "circle" => "circle".to_string(),
        _ => "none".to_string(),
    }
}

/// A condition slot: an `if`, a `while`, a `wait until`. A bad one reports
/// itself and reads as false, since a zero isn't true.
pub fn boolean(host: &mut dyn Host, actor: &str, value: R) -> bool {
    evaluated(host, actor, value).as_bool()
}

pub fn evaluated(host: &mut dyn Host, actor: &str, value: R) -> Val {
    match value {
        Ok(value) => value,
        Err(message) => {
            host.error(actor, &message);
            Val::Num(0.0)
        }
    }
}

/// A sensing reporter, with its arguments evaluated left to right first - an
/// extension operator can't short-circuit, so a bad argument is the answer.
pub fn sense(host: &mut dyn Host, actor: &str, kind: &str, args: Vec<R>) -> R {
    let mut evaluated = Vec::with_capacity(args.len());
    for arg in args {
        evaluated.push(arg?);
    }
    host.sense(actor, kind, &evaluated)
}

// ─── Operators ──────────────────────────────────────────────────────────────

pub fn add(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? + b?.as_number()?))
}

pub fn sub(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? - b?.as_number()?))
}

pub fn mul(a: R, b: R) -> R {
    Ok(Val::Num(a?.as_number()? * b?.as_number()?))
}

pub fn div(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Val::Num(l / r))
}

pub fn modulo(a: R, b: R) -> R {
    let (l, r) = (a?.as_number()?, b?.as_number()?);
    if r == 0.0 {
        return Err("mod by zero".to_string());
    }
    Ok(Val::Num(l.rem_euclid(r)))
}

pub fn round(a: R) -> R {
    Ok(Val::Num(a?.as_number()?.round()))
}

pub fn math(function: R, a: R) -> R {
    let function = function?.as_text();
    let n = a?.as_number()?;
    let to_radians = std::f64::consts::PI / 180.0;
    let to_degrees = 180.0 / std::f64::consts::PI;
    let result = match function.as_str() {
        "Abs" => n.abs(),
        "Floor" => n.floor(),
        "Ceiling" => n.ceil(),
        "Sign" => {
            if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "Sqrt" => n.sqrt(),
        "Sin" => (n * to_radians).sin(),
        "Cos" => (n * to_radians).cos(),
        "Tan" => (n * to_radians).tan(),
        "Asin" => n.asin() * to_degrees,
        "Acos" => n.acos() * to_degrees,
        "Atan" => n.atan() * to_degrees,
        "Ln" => n.ln(),
        "Log" => n.log10(),
        "Log2" => n.log2(),
        "EPower" => n.exp(),
        "TenPower" => 10.0_f64.powf(n),
        other => return Err(format!("unknown math function '{other}'")),
    };
    Ok(Val::Num(result))
}

pub fn join(parts: Vec<R>) -> R {
    let mut joined = String::new();
    for part in parts {
        joined.push_str(&part?.as_text());
    }
    Ok(Val::Text(joined))
}

pub fn length(a: R) -> R {
    Ok(Val::Num(a?.as_text().chars().count() as f64))
}

pub fn index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_index_of(&haystack, &needle) as f64))
}

pub fn last_index_of(needle: R, haystack: R) -> R {
    let needle = needle?.as_text();
    let haystack = haystack?.as_text();
    Ok(Val::Num(char_last_index_of(&haystack, &needle) as f64))
}

pub fn letter_of(index: R, a: R) -> R {
    let index = index?.as_number()? as i64;
    let text = a?.as_text();
    let chars: Vec<char> = text.chars().collect();
    if index < 1 || index as usize > chars.len() {
        return Err(format!(
            "letter {index} is out of range for a {}-character value",
            chars.len()
        ));
    }
    Ok(Val::Text(chars[index as usize - 1].to_string()))
}

pub fn case(a: R, which: R) -> R {
    let text = a?.as_text();
    let upper = which?.as_text() == "Upper";
    Ok(Val::Text(if upper {
        text.to_uppercase()
    } else {
        text.to_lowercase()
    }))
}

pub fn eq(a: R, b: R) -> R {
    Ok(Val::Bool(values_equal(&a?, &b?)))
}

pub fn neq(a: R, b: R) -> R {
    Ok(Val::Bool(!values_equal(&a?, &b?)))
}

pub fn gt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? > b?.as_number()?))
}

pub fn lt(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? < b?.as_number()?))
}

pub fn gte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? >= b?.as_number()?))
}

pub fn lte(a: R, b: R) -> R {
    Ok(Val::Bool(a?.as_number()? <= b?.as_number()?))
}

/// `and` and `or` take the second operand unevaluated, because the VM's
/// short-circuit is observable: `false and <a bad slot>` reports nothing.
pub fn and(a: R, b: impl FnOnce() -> R) -> R {
    if !a?.as_bool() {
        return Ok(Val::Bool(false));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn or(a: R, b: impl FnOnce() -> R) -> R {
    if a?.as_bool() {
        return Ok(Val::Bool(true));
    }
    Ok(Val::Bool(b()?.as_bool()))
}

pub fn not(a: R) -> R {
    Ok(Val::Bool(!a?.as_bool()))
}

/// `Op::Eq`'s rule: numeric when both sides parse as numbers, text otherwise.
fn values_equal(l: &Val, r: &Val) -> bool {
    match (l.as_number(), r.as_number()) {
        (Ok(l), Ok(r)) => l == r,
        _ => l.as_text() == r.as_text(),
    }
}

/// 1-based char index of the first `needle` in `haystack`, 0 for none.
fn char_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}

/// [`char_index_of`] from the other end.
fn char_last_index_of(haystack: &str, needle: &str) -> usize {
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || n.len() > h.len() {
        return 0;
    }
    (0..=h.len() - n.len())
        .rev()
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(0, |i| i + 1)
}
