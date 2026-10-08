//! The world state reporter blocks read.
//!
//! blockstitch evaluates a value tree through plain functions with no
//! context, so the sensing operators in [`crate::value`] read a snapshot the
//! host refreshes at simulation and presentation boundaries instead. [`with_actor`] supplies the other
//! half: which actor's script is being evaluated right now, so "x position"
//! means the running actor's own.

use crate::input::ActionSense;
use crate::sound::SoundBus;
use crate::value::Evaluated;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// One actor, as its own blocks, its script, and other actors can see it.
#[derive(Debug, Clone)]
pub struct ActorSense {
    pub name: String,
    pub position: [f32; 3],
    /// Where the actor stands in its parent's frame - the parent's world
    /// transform inverted onto the world position above. The world position
    /// itself when the actor hangs off nothing.
    pub local_position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
    pub visible: bool,
    /// Ids of the actors this one is currently touching.
    pub touching: HashSet<String>,
    /// The actor this one hangs off right now, by id, or empty for none.
    pub parent: String,
    /// True for an actor a `create clone` block made rather than the editor.
    pub is_clone: bool,
    /// True while any tween (a glide or a `tween ...` block) is still moving
    /// this actor. What `is tweening?` answers.
    pub tweening: bool,
    /// The clip the animation player is holding, by name, or empty for none.
    /// What `current clip` answers.
    pub anim_clip: String,
    /// The 1-based frame showing right now. Zero with no clip.
    pub anim_frame: usize,
    /// Whether that clip is still advancing. A `Once` clip at its end, or a
    /// stopped player, reads as false.
    pub anim_playing: bool,
    /// The id of the last actor or clone this one made, so a block can move,
    /// parent or delete what it just created. Empty until it makes one.
    pub last_created: String,
    /// Which components the actor is carrying right now, which an `attach`
    /// or `detach` earlier in the run may have changed.
    pub attached: HashSet<String>,
    /// The actor's custom components as they stand this frame, by component
    /// name then field name. Live values, not the authored ones: a block that
    /// wrote a field last frame reads its own number back.
    pub components: HashMap<String, HashMap<String, Evaluated>>,
    /// False for an actor with no `Body`: ray and overlap queries skip it,
    /// the way the solver does.
    pub has_body: bool,
    /// True for a sensor collider: it fires touches without pushing back.
    pub trigger: bool,
    /// True while the actor carries a light that casts shadow maps. What
    /// `casts shadows?` answers.
    pub casts_shadows: bool,
    /// The layer the actor lives on, 1-8.
    pub layer: u8,
    /// Bitmask of the layers this actor pairs with. A query fired from this
    /// actor only sees actors its own mask names.
    pub mask: u8,
    /// What the actor collides (and raycasts) with, in world units.
    pub shape: ColliderShape,
    /// Linear velocity in world units per second: pixels in 2D, metres in
    /// 3D. Zero with no body. What `velocity` answers.
    pub velocity: [f32; 3],
    /// Angular velocity in radians per second about x/y/z. A 2D body turns
    /// about z only. Zero with no body.
    pub angular_velocity: [f32; 3],
    /// The body's mass in kilograms, from its shapes (or its own when it
    /// carries none). Zero with no body. What `mass of` answers.
    pub mass: f32,
    /// Whether a solid contact holds this actor up. Needs no controller
    /// move, unlike the controller's own `grounded`. What `is grounded?`
    /// answers.
    pub grounded: bool,
    /// What its emitter's particles did this frame, and where last.
    pub particles: crate::vfx::ParticleSense,
}

/// What shape an actor collides (and raycasts) with, in world units, in the
/// actor's own frame: queries turn it by the actor's rotation.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ColliderShape {
    /// Nothing to hit: no body, or a visual with no collider in this mode.
    #[default]
    None,
    /// Half-extents in x/y (2D) or x/y/z (3D).
    Box { half: [f32; 3] },
    /// A disc (2D) or ball (3D).
    Ball { radius: f32 },
    /// Several boxes, each offset from the actor: a solid tilemap's cells.
    Parts(std::sync::Arc<[ShapePart]>),
}

/// One box of a [`ColliderShape::Parts`], offset from the actor's position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapePart {
    pub offset: [f32; 3],
    pub half: [f32; 3],
}

/// One touch point, as the touch reporters see it: where it is in world
/// units, in the same frame as `mouse`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CameraSense {
    /// Where the world camera stands, in world units.
    pub position: [f32; 3],
    /// Where it looks, unit length in world units while a game runs.
    /// What the middle of the screen points at.
    pub forward: [f32; 3],
}

/// The 2D camera's live finishing state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Camera2dSense {
    /// Pixels per world unit.
    pub zoom: f32,
    /// Degrees of roll, shake included.
    pub rotation: f32,
    /// The bounds are holding the view back.
    pub at_bounds: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchSense {
    pub id: u64,
    pub position: [f32; 2],
}

/// One interface element, as the reporter blocks see it. An element the
/// blocks have made but hidden is still here: `shown` is the question a
/// block asks, and not being here at all is a different question again.
#[derive(Debug, Clone)]
pub struct UiSense {
    /// A slider's number, a toggle's on/off, an input's text. What
    /// `value of (id)` reads.
    pub value: Evaluated,
    /// The words it is showing: a label's text, a button's caption, an
    /// input's typing or its placeholder. What `text of (id)` reads.
    pub text: String,
    /// Whether it and every element it flows inside are visible.
    pub shown: bool,
}

/// Everything sensible about the world this frame. Keyed by actor id.
#[derive(Debug, Clone, Default)]
pub struct Sensors {
    pub performance: crate::quality::Sample,
    /// Seconds since the run started. Frozen while the game is paused.
    pub time: f64,
    /// The same, unfrozen: what a strand the interface started reads, so a
    /// clock on a pause menu keeps its own time.
    pub wall_time: f64,
    /// Whether `pause game` has the world frozen right now.
    pub paused: bool,
    /// Held keys, in [`normalize_key`]'s spelling.
    pub keys: HashSet<String>,
    /// Pointer position in world units.
    pub mouse: [f32; 2],
    /// Raw pointer motion in pixels since the last frame: x grows as the
    /// pointer moves right, y as it moves down. What a first-person camera
    /// wants, where `mouse` is a place an actor can stand.
    pub mouse_delta: [f32; 2],
    /// Whether the pointer is grabbed and hidden for first-person play.
    pub mouse_locked: bool,
    pub mouse_down: bool,
    /// Held mouse buttons by name: `left`, `right`, `middle`. `mouse_down` is
    /// the left one, kept so old blocks keep meaning what they meant.
    pub mouse_buttons: HashSet<String>,
    /// Named input actions this frame, by action name as authored.
    pub actions: HashMap<String, ActionSense>,
    /// Live touch points in world units, in press order. Empty with no
    /// fingers down; more than one is a multitouch.
    pub touches: Vec<TouchSense>,
    /// True when at least one gamepad is connected.
    pub gamepad_connected: bool,
    /// Live stick and trigger values by canonical axis name
    /// (`leftstickx`, ...). What `gamepad axis` reads.
    pub gamepad_axes: HashMap<String, f32>,
    /// Held gamepad buttons by canonical name (`south`, `dpadup`, ...).
    pub gamepad_buttons: HashSet<String>,
    pub actors: HashMap<String, ActorSense>,
    /// The world camera's pose as of this frame. What `camera position` and
    /// `camera direction` read, so aim blocks can use the live view itself
    /// instead of tracking one by hand.
    pub camera: CameraSense,
    /// Every interface element the blocks have made, by id.
    pub ui: HashMap<String, UiSense>,
    /// The text input holding the keyboard, by id, or empty for none.
    pub ui_focus: String,
    /// Asset paths with at least one live voice, so `is sound playing?`
    /// answers. Spelled the way the play blocks spell them.
    pub sounds: HashSet<String>,
    /// The live gain of each mixing bus, in 0-100 block scale. Seeded from
    /// the saved mix and moved by `set bus volume` mid-run.
    pub bus_volumes: HashMap<SoundBus, f32>,
    /// Sun, wind, fog and weather as of the last fixed tick.
    pub atmosphere: AtmosphereSense,
    /// Every water body, sampled with the atmosphere on the fixed tick.
    pub water: crate::water::WaterSense,
    /// The 2D lit look: what `light level at` and `is night?` read.
    pub light2d: crate::light2d::Light2dSense,
    /// The 2D camera: what `camera zoom` and `is camera at bounds?` read.
    pub camera2d: Camera2dSense,
    /// Live tilemaps and room bounds, which `tile at` and `room containing`
    /// read.
    pub level: crate::tilemap::LevelSense,
    /// The scene running right now, by name. What `current scene` reads;
    /// sampled on the fixed tick so VM and compiled logic agree.
    pub current_scene: String,
    /// Every scene's name, in project order. What `scene names` reports as
    /// a JSON list, so `load json into list` takes it.
    pub scene_names: Vec<String>,
    /// The save slot this run writes to, by name. What `save slot` reads;
    /// sampled on the fixed tick so VM and compiled logic agree.
    pub current_save_slot: String,
    /// Every slot with a file on disk, default first. What `save slots`
    /// reports as a JSON list, so `load json into list` takes it.
    pub save_slots: Vec<String>,
    /// The language a fresh run speaks, lowercased. What `language` reads
    /// and `text for key` answers in; moved by `set language to`.
    pub language: String,
    /// The cutscene playing right now, by name, or empty for none. What
    /// `is cutscene playing?` reads; sampled with the sensors each frame
    /// from the wall-clock player, so VM and compiled logic agree.
    pub cutscene_name: String,
    /// Seconds into the playing cutscene. What `cutscene time` reads.
    pub cutscene_time: f32,
    /// Actors by name, built on demand. Leave it to `Default`.
    pub names: NameIndex,
}

/// How many name lookups a snapshot answers by scanning before it builds an
/// index: one lookup is cheaper walked than hashed, a crowd of actors each
/// asking about one is not.
const SCAN_LOOKUPS: u32 = 2;

/// The actors of one snapshot by lower-cased name: how many answer to it, and
/// the smallest id among them, so a name shared by clones picks one actor
/// every time. Rebuilt when the number of actors changes.
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    lookups: Cell<u32>,
    built: RefCell<Option<(usize, NamedActors)>>,
}

/// Lower-cased name -> (how many answer to it, the smallest id among them).
type NamedActors = HashMap<String, (usize, String)>;

impl NameIndex {
    /// Runs `f` on the index of `actors`, or `None` while scanning is still
    /// the cheaper way to answer.
    fn with<R>(
        &self,
        actors: &HashMap<String, ActorSense>,
        f: impl FnOnce(&NamedActors) -> R,
    ) -> Option<R> {
        let seen = self.lookups.get();
        self.lookups.set(seen.saturating_add(1));
        let mut built = self.built.borrow_mut();
        if built.as_ref().is_none_or(|(len, _)| *len != actors.len()) {
            if seen < SCAN_LOOKUPS {
                return None;
            }
            let mut index = NamedActors::new();
            for (id, actor) in actors {
                match index.get_mut(&actor.name.to_ascii_lowercase()) {
                    Some((count, first)) => {
                        *count += 1;
                        if id < first {
                            first.clone_from(id);
                        }
                    }
                    None => {
                        index.insert(actor.name.to_ascii_lowercase(), (1, id.clone()));
                    }
                }
            }
            *built = Some((actors.len(), index));
        }
        built.as_ref().map(|(_, index)| f(index))
    }
}

/// The shape of [`AtmosphereSense`]. Bumped when a field changes meaning or
/// goes away; adding one doesn't need it, since a reading is asked for by name.
pub const ATMOSPHERE_VERSION: u32 = 1;

/// The world's air as blocks and scripts read it. Sampled once per fixed tick,
/// before any scheduler runs, so the VM, compiled logic and scripts all see
/// the same values on the same tick however many frames draw between.
#[derive(Debug, Clone, PartialEq)]
pub struct AtmosphereSense {
    /// [`ATMOSPHERE_VERSION`] when the host wrote it.
    pub version: u32,
    /// The fixed tick it was sampled on, counted from the start of the run.
    pub tick: u64,
    /// Unit vector towards the sun.
    pub sun_direction: [f32; 3],
    /// Linear RGB, 0-1.
    pub sun_color: [f32; 3],
    /// Lux.
    pub sun_illuminance: f32,
    /// Unit vector the wind blows towards, or zero when calm.
    pub wind_direction: [f32; 3],
    /// World units per second.
    pub wind_speed: f32,
    /// Extra speed a gust adds on top right now.
    pub wind_gust: f32,
    /// Degrees the wind blows towards, clockwise from north, as the wind's
    /// own direction dial measures it.
    pub wind_heading: f32,
    /// How stormy the wind is, 0-1.
    pub storm: f32,
    /// Extinction per world unit, 0 for clear air.
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    /// Fraction of sky covered, 0-1.
    pub cloud_cover: f32,
    /// Rain and snow intensity, 0-1.
    pub rain: f32,
    pub snow: f32,
    /// How wet surfaces are, 0-1. Lags the rain.
    pub wetness: f32,
    /// Degrees Celsius.
    pub temperature: f32,
    /// The camera's EV100 after every claim on it: lower is brighter.
    pub exposure: f32,
    /// Mean scene luminance the camera sees, in nits, before exposure.
    pub luminance: f32,
    /// Whether frames are going out as HDR right now.
    pub hdr_display: bool,
    /// The peak brightness the HDR tone curve aims at, in nits.
    pub peak_brightness: f32,
    /// Whether the world camera is lit by ray tracing right now.
    pub ray_tracing: bool,
    /// Whether this GPU and build can trace rays at all.
    pub ray_tracing_available: bool,
    /// Names of the environment volumes showing at the camera, lowest
    /// priority first. What `active volumes` reports.
    pub volumes: Vec<String>,
    /// Hours, 0-24. What `time of day` reports, sampled on the fixed tick.
    pub time_of_day: f32,
    /// The weather preset the air is in, by name. Empty before the first
    /// blend. What `current weather` reports.
    pub weather: String,
    /// The aurora's KP index, 0 when there is none.
    pub aurora: f32,
    /// How bright the latest lightning flash is right now, 0-1.
    pub lightning: f32,
}

impl Default for AtmosphereSense {
    fn default() -> Self {
        Self {
            version: ATMOSPHERE_VERSION,
            tick: 0,
            sun_direction: [0.0, 1.0, 0.0],
            sun_color: [1.0; 3],
            sun_illuminance: 0.0,
            wind_direction: [0.0; 3],
            wind_speed: 0.0,
            wind_gust: 0.0,
            wind_heading: 0.0,
            storm: 0.0,
            fog_density: 0.0,
            fog_color: [1.0; 3],
            cloud_cover: 0.0,
            rain: 0.0,
            snow: 0.0,
            wetness: 0.0,
            temperature: 20.0,
            exposure: 9.7,
            luminance: 0.0,
            hdr_display: false,
            peak_brightness: 1000.0,
            ray_tracing: false,
            ray_tracing_available: false,
            volumes: Vec::new(),
            time_of_day: 12.0,
            weather: String::new(),
            aurora: 0.0,
            lightning: 0.0,
        }
    }
}

/// Every reading [`AtmosphereSense::field`] answers, in the spelling the
/// reporter's dropdown and a script's `atmosphere` call use.
pub const ATMOSPHERE_FIELDS: &[&str] = &[
    "sun x",
    "sun y",
    "sun z",
    "sun brightness",
    "wind x",
    "wind y",
    "wind z",
    "wind speed",
    "wind gust",
    "wind direction",
    "storm",
    "fog density",
    "cloud cover",
    "rain",
    "snow",
    "wetness",
    "temperature",
    "exposure",
    "luminance",
    "hdr",
    "peak brightness",
    "ray tracing",
    "ray tracing available",
    "aurora",
    "lightning",
    "time of day",
    "sun elevation",
];

impl AtmosphereSense {
    /// One reading by name, case and spacing ignored. The one lookup every
    /// reader goes through, so a new field is one arm here.
    pub fn field(&self, name: &str) -> Option<f64> {
        let key: String = name
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_')
            .flat_map(char::to_lowercase)
            .collect();
        let value = match key.as_str() {
            "sunx" => self.sun_direction[0],
            "suny" => self.sun_direction[1],
            "sunz" => self.sun_direction[2],
            "sunbrightness" | "sunilluminance" => self.sun_illuminance,
            "windx" => self.wind_direction[0] * self.wind_speed,
            "windy" => self.wind_direction[1] * self.wind_speed,
            "windz" => self.wind_direction[2] * self.wind_speed,
            "windspeed" => self.wind_speed,
            "windgust" => self.wind_gust,
            "winddirection" | "windheading" => self.wind_heading,
            "storm" => self.storm,
            "fogdensity" | "fog" => self.fog_density,
            "cloudcover" | "clouds" => self.cloud_cover,
            "rain" => self.rain,
            "snow" => self.snow,
            "wetness" => self.wetness,
            "temperature" => self.temperature,
            "exposure" | "ev" => self.exposure,
            "luminance" | "sceneluminance" => self.luminance,
            "hdr" | "hdrdisplay" => {
                if self.hdr_display {
                    1.0
                } else {
                    0.0
                }
            }
            "peakbrightness" | "peak" => self.peak_brightness,
            "raytracing" | "rt" => f32::from(u8::from(self.ray_tracing)),
            "raytracingavailable" => f32::from(u8::from(self.ray_tracing_available)),
            "aurora" | "kp" => self.aurora,
            "lightning" | "flash" => self.lightning,
            "timeofday" | "time" => self.time_of_day,
            "sunelevation" | "sunelev" | "sune" => {
                self.sun_direction[1].clamp(-1.0, 1.0).asin().to_degrees()
            }
            _ => return None,
        };
        Some(value as f64)
    }
}

impl Default for UiSense {
    fn default() -> Self {
        Self {
            value: Evaluated::Text(String::new()),
            text: String::new(),
            shown: true,
        }
    }
}

impl Default for ActorSense {
    fn default() -> Self {
        Self {
            name: String::new(),
            position: [0.0; 3],
            local_position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            visible: true,
            parent: String::new(),
            is_clone: false,
            tweening: false,
            anim_clip: String::new(),
            anim_frame: 0,
            anim_playing: false,
            last_created: String::new(),
            touching: HashSet::new(),
            attached: HashSet::new(),
            components: HashMap::new(),
            has_body: false,
            trigger: false,
            casts_shadows: false,
            layer: 1,
            mask: 0xFF,
            shape: ColliderShape::None,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            mass: 0.0,
            grounded: false,
            particles: crate::vfx::ParticleSense::default(),
        }
    }
}

impl Sensors {
    /// How many actors answer to `name` right now - clones included, since
    /// they share their template's. An empty name counts every actor.
    pub fn count_named(&self, name: &str) -> usize {
        let name = name.trim();
        if name.is_empty() {
            return self.actors.len();
        }
        let indexed = self.names.with(&self.actors, |index| {
            index
                .get(&name.to_ascii_lowercase())
                .map_or(0, |(count, _)| *count)
        });
        indexed.unwrap_or_else(|| {
            self.actors
                .values()
                .filter(|actor| actor.name.eq_ignore_ascii_case(name))
                .count()
        })
    }

    /// Looks an actor up by id first, then by name (case-insensitively), so a
    /// block can name either.
    pub fn find(&self, id_or_name: &str) -> Option<&ActorSense> {
        if let Some(actor) = self.actors.get(id_or_name) {
            return Some(actor);
        }
        let indexed = self.names.with(&self.actors, |index| {
            index
                .get(&id_or_name.to_ascii_lowercase())
                .and_then(|(_, id)| self.actors.get(id))
        });
        match indexed {
            Some(found) => found,
            None => self
                .actors
                .values()
                .find(|actor| actor.name.eq_ignore_ascii_case(id_or_name)),
        }
    }

    /// The clock a script should read: the world's, or the wall's when the
    /// interface is what started it.
    pub fn clock(&self, ui: bool) -> f64 {
        if ui { self.wall_time } else { self.time }
    }
}

/// Answers a plugin reporter: the plugin, the block and its slot values in.
pub type PluginReader =
    dyn FnMut(&str, &str, &[crate::value::Evaluated]) -> Result<crate::value::Evaluated, String>;

thread_local! {
    /// What answers plugin reporters on this thread: the world installs it
    /// while a run has plugin code open, and the editor has none.
    static PLUGIN_READER: RefCell<Option<Box<PluginReader>>> = const { RefCell::new(None) };

    /// The snapshot this thread published. Thread-local rather than global
    /// because the VM is `!Send` and always runs on the same thread that
    /// publishes for it - the renderer's main thread - so a lock here would
    /// only ever let unrelated threads (or tests) interfere with each other.
    static SENSORS: RefCell<Sensors> = RefCell::new(Sensors::default());

    /// Ids of the scripts being stepped, innermost last. Only the first
    /// `depth` are live; the rest keep their buffers so entering a script
    /// allocates nothing once warm.
    static CURRENT_ACTOR: RefCell<ActorStack> = const { RefCell::new(ActorStack::new()) };

    /// Whether that script was started by the interface. Kept beside the
    /// actor for the same reason: the operators are plain `fn`s with no
    /// context, and `timer` has to know which clock it is being asked for.
    static UI_STRAND: RefCell<bool> = const { RefCell::new(false) };

    /// The project's string table this run reads `text for key` through.
    /// Static per run like the project itself: the world installs it when
    /// a run loads and clears it when the run ends, so a reporter answers
    /// without the snapshot copying the whole table every tick.
    static LOCALE: RefCell<crate::locale::Localization> = RefCell::new(crate::locale::Localization::default());
}

/// Actor ids of nested `with_script` calls, buffers reused across calls.
struct ActorStack {
    ids: Vec<String>,
    depth: usize,
}

impl ActorStack {
    const fn new() -> Self {
        Self {
            ids: Vec::new(),
            depth: 0,
        }
    }

    fn push(&mut self, id: &str) {
        if let Some(slot) = self.ids.get_mut(self.depth) {
            slot.clear();
            slot.push_str(id);
        } else {
            self.ids.push(id.to_string());
        }
        self.depth += 1;
    }

    fn pop(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn top(&self) -> Option<&str> {
        self.depth
            .checked_sub(1)
            .and_then(|i| self.ids.get(i))
            .map(String::as_str)
    }
}

/// Replaces this thread's snapshot. Fixed-step hosts can refresh individual
/// slots before stepping scripts.
pub fn publish(sensors: Sensors) {
    SENSORS.with(|slot| *slot.borrow_mut() = sensors);
}

/// Hands the published actors back to the host, leaving none, so a frame's
/// snapshot can be refreshed in place instead of rebuilt from nothing. The
/// next [`publish`] puts them back.
pub fn take_actors() -> HashMap<String, ActorSense> {
    SENSORS.with(|slot| std::mem::take(&mut slot.borrow_mut().actors))
}

/// Replaces authoritative actors without changing client input or UI.
pub fn publish_actors(actors: HashMap<String, ActorSense>) {
    SENSORS.with(|slot| {
        let mut sensors = slot.borrow_mut();
        sensors.actors = actors;
        sensors.names = Default::default();
    });
}

/// Refreshes run context before gameplay reads it on this thread.
pub fn publish_run(time: f64, wall_time: f64, paused: bool, scene: &str, scenes: &[String]) {
    SENSORS.with(|snapshot| {
        let mut s = snapshot.borrow_mut();
        s.time = time;
        s.wall_time = wall_time;
        s.paused = paused;
        scene.clone_into(&mut s.current_scene);
        scenes.clone_into(&mut s.scene_names);
    });
}

/// Samples the retained interface before either block scheduler runs.
pub fn publish_ui(ui: HashMap<String, UiSense>, focus: String) {
    SENSORS.with(|snapshot| {
        let mut s = snapshot.borrow_mut();
        s.ui = ui;
        s.ui_focus = focus;
    });
}

/// Publishes the 2D camera for `camera zoom` and `is camera at bounds?`.
pub fn publish_camera2d(camera2d: Camera2dSense) {
    SENSORS.with(|slot| slot.borrow_mut().camera2d = camera2d);
}

/// Publishes the frame's 2D lights for `light level at` and `is night?`.
pub fn publish_light2d(light2d: crate::light2d::Light2dSense) {
    SENSORS.with(|slot| slot.borrow_mut().light2d = light2d);
}

/// Samples the atmosphere at the head of each fixed tick.
pub fn publish_atmosphere(atmosphere: AtmosphereSense) {
    SENSORS.with(|slot| slot.borrow_mut().atmosphere = atmosphere);
}

/// Samples which save slot this run writes to and which slots have files,
/// beside the scene names. Refreshed on the fixed tick like the scenes, so
/// the VM and compiled logic agree; the world refreshes the list when a
/// slot is switched or deleted rather than listing the dir every tick.
pub fn publish_saves(slot: &str, slots: &[String]) {
    SENSORS.with(|snapshot| {
        let mut s = snapshot.borrow_mut();
        slot.clone_into(&mut s.current_save_slot);
        slots.clone_into(&mut s.save_slots);
    });
}

/// Samples the run's language. Moved by `set language to`, read by
/// `language` and the `text for key` lookup.
pub fn publish_language(language: &str) {
    SENSORS.with(|snapshot| language.clone_into(&mut snapshot.borrow_mut().language));
}

/// Installs (or, with `None`, removes) this run's string table: what
/// `text for key` answers through. The world sets it when a run loads.
pub fn set_locale_table(table: Option<crate::locale::Localization>) {
    LOCALE.with(|slot| *slot.borrow_mut() = table.unwrap_or_default());
}

/// The text for `key` in the published language: the language itself, then
/// the table's default, then English, then the key. Never blank for a
/// non-blank key.
pub fn locale_text(key: &str) -> String {
    let language = SENSORS.with(|slot| slot.borrow().language.clone());
    LOCALE.with(|slot| slot.borrow().text(key, &language))
}

/// Samples the water at the head of each fixed tick, beside the atmosphere.
pub fn publish_water(water: crate::water::WaterSense) {
    SENSORS.with(|slot| slot.borrow_mut().water = water);
}

/// The live tilemaps and rooms, published each frame after the rest.
pub fn publish_level(level: crate::tilemap::LevelSense) {
    SENSORS.with(|slot| slot.borrow_mut().level = level);
}

/// Replaces a bodied actor's query shape: a painted tilemap's live cells.
pub fn publish_shape(id: &str, shape: ColliderShape) {
    SENSORS.with(|slot| {
        if let Some(actor) = slot.borrow_mut().actors.get_mut(id)
            && actor.has_body
        {
            actor.shape = shape;
        }
    });
}

/// Who entered which room this fixed tick, for a script's `entered_room`.
pub fn publish_entered(entered: HashMap<String, String>) {
    SENSORS.with(|slot| slot.borrow_mut().level.entered = entered);
}

/// Installs (or, with `None`, removes) this thread's plugin reporter answerer.
pub fn set_plugin_reader(reader: Option<Box<PluginReader>>) {
    PLUGIN_READER.with(|slot| *slot.borrow_mut() = reader);
}

/// Reads a plugin reporter through whatever answers for this thread.
pub fn plugin_read(
    plugin: &str,
    block: &str,
    args: &[crate::value::Evaluated],
) -> Result<crate::value::Evaluated, String> {
    PLUGIN_READER.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| "a plugin reporter was read while another was being read".to_string())?;
        match slot.as_mut() {
            Some(reader) => reader(plugin, block, args),
            None => Err(format!(
                "{plugin}/{block} only answers while the game is running"
            )),
        }
    })
}

/// Reads the published snapshot. `f` sees a default-empty one before the
/// first [`publish`], so a reporter previewed in the editor still answers.
pub fn read<R>(f: impl FnOnce(&Sensors) -> R) -> R {
    SENSORS.with(|slot| f(&slot.borrow()))
}

/// Runs `f` with `actor_id` as the actor "my x position" and friends refer
/// to. Nested calls restore the previous actor on the way out.
pub fn with_actor<R>(actor_id: &str, f: impl FnOnce() -> R) -> R {
    with_script(actor_id, in_ui_strand(), f)
}

/// The same, also saying whether the interface started this script - which
/// is what decides the clock `timer` answers with.
pub fn with_script<R>(actor_id: &str, ui: bool, f: impl FnOnce() -> R) -> R {
    CURRENT_ACTOR.with(|cell| cell.borrow_mut().push(actor_id));
    let was_ui = UI_STRAND.with(|cell| cell.replace(ui));
    let result = f();
    CURRENT_ACTOR.with(|cell| cell.borrow_mut().pop());
    UI_STRAND.with(|cell| *cell.borrow_mut() = was_ui);
    result
}

/// The actor whose script is being stepped, if any.
pub fn current_actor() -> Option<String> {
    CURRENT_ACTOR.with(|cell| cell.borrow().top().map(str::to_string))
}

/// Runs `f` on the stepping actor's own snapshot entry, borrowed rather than
/// copied. Errors say why there is none: a reporter previewed in the editor
/// has no actor, and a deleted one is gone from the world.
pub fn with_me<R>(f: impl FnOnce(&ActorSense) -> R) -> Result<R, String> {
    CURRENT_ACTOR.with(|current| {
        let current = current.borrow();
        let id = current.top().ok_or("no actor is running this script")?;
        read(|sensors| {
            sensors
                .actors
                .get(id)
                .map(f)
                .ok_or_else(|| "this actor isn't in the running world".to_string())
        })
    })
}

/// Whether the script being stepped was started by the interface.
pub fn in_ui_strand() -> bool {
    UI_STRAND.with(|cell| *cell.borrow())
}

/// Says which kind of strand the host is about to step, for a scheduler
/// that isn't the VM. The compiled runner calls this through the ABI before
/// each slice, so `timer` answers the same clock on both sides.
pub fn set_ui_strand(ui: bool) {
    UI_STRAND.with(|cell| *cell.borrow_mut() = ui);
}

/// The key names blocks and the runtime agree on. Anything not listed here
/// is still usable - a single printable character works as itself - this is
/// just what the editor's dropdown offers.
pub const KEY_NAMES: &[&str] = &[
    "space",
    "up arrow",
    "down arrow",
    "left arrow",
    "right arrow",
    "enter",
    "escape",
    "shift",
    "control",
    "alt",
    "tab",
    "backspace",
    // The Android back button (winit's `GoBack`): the phone's pause key, the
    // way escape is the desktop's. Never produced on desktop.
    "back",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
];

/// Canonical spelling of a key name - what both the `when key pressed`
/// header and the `key down?` reporter compare on.
pub fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actors_resolve_by_id_or_by_name() {
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Player".to_string(),
                ..Default::default()
            },
        );
        assert!(sensors.find("a1").is_some());
        assert!(sensors.find("player").is_some());
        assert!(sensors.find("Enemy").is_none());
    }

    fn crowd(names: &[(&str, &str)]) -> Sensors {
        let mut sensors = Sensors::default();
        for (id, name) in names {
            sensors.actors.insert(
                id.to_string(),
                ActorSense {
                    name: name.to_string(),
                    ..Default::default()
                },
            );
        }
        sensors
    }

    #[test]
    fn the_name_index_answers_what_a_scan_does() {
        let sensors = crowd(&[
            ("c2", "Ball"),
            ("c1", "ball"),
            ("a1", "Player"),
            ("c3", "Ball"),
        ]);
        // The first lookups scan, the later ones go through the index; the
        // answers stay the same, and a shared name always picks one actor.
        for round in 0..6 {
            assert_eq!(sensors.count_named("BALL"), 3, "round {round}");
            assert_eq!(sensors.count_named("nobody"), 0, "round {round}");
            assert_eq!(sensors.count_named(""), 4, "round {round}");
            assert_eq!(
                sensors.find("player").map(|a| a.name.as_str()),
                Some("Player")
            );
            assert!(sensors.find("nobody").is_none(), "round {round}");
            if round >= 2 {
                assert_eq!(sensors.find("ball").unwrap().name, "ball", "smallest id");
            }
        }
        assert_eq!(sensors.find("c3").unwrap().name, "Ball", "ids win");
    }

    #[test]
    fn the_name_index_follows_actors_added_after_it_was_built() {
        let mut sensors = crowd(&[("a1", "Player")]);
        for _ in 0..4 {
            assert!(sensors.find("Enemy").is_none());
        }
        sensors.actors.insert(
            "a2".to_string(),
            ActorSense {
                name: "Enemy".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(
            sensors.find("enemy").map(|a| a.name.as_str()),
            Some("Enemy")
        );
        assert_eq!(sensors.count_named("enemy"), 1);
    }

    #[test]
    fn with_me_reads_the_stepping_actor_in_place() {
        let mut sensors = crowd(&[("a1", "Player")]);
        sensors.actors.get_mut("a1").unwrap().position = [1.0, 2.0, 3.0];
        publish(sensors);
        assert!(with_me(|_| ()).is_err(), "nobody is stepping");
        let at = with_script("a1", false, || with_me(|me| me.position));
        assert_eq!(at, Ok([1.0, 2.0, 3.0]));
        let gone = with_script("a9", false, || with_me(|me| me.position));
        assert!(gone.is_err());
        // Nested scripts see their own actor, and leave the outer one behind.
        with_script("a1", false, || {
            with_script("a9", false, || {
                assert_eq!(current_actor().as_deref(), Some("a9"))
            });
            assert_eq!(current_actor().as_deref(), Some("a1"));
        });
        assert_eq!(current_actor(), None);
    }

    #[test]
    fn a_ui_strand_reads_the_wall_clock_and_everything_else_the_worlds() {
        let sensors = Sensors {
            time: 2.0,
            wall_time: 9.0,
            ..Default::default()
        };
        assert_eq!(sensors.clock(false), 2.0);
        assert_eq!(sensors.clock(true), 9.0);
    }

    #[test]
    fn the_ui_flag_is_restored_with_the_actor_after_a_nested_scope() {
        with_script("outer", true, || {
            assert!(in_ui_strand());
            with_script("inner", false, || assert!(!in_ui_strand()));
            assert!(in_ui_strand());
        });
        assert!(!in_ui_strand());
    }

    #[test]
    fn atmosphere_fields_all_answer_by_name() {
        let atmosphere = AtmosphereSense {
            wind_direction: [1.0, 0.0, 0.0],
            wind_speed: 4.0,
            ..Default::default()
        };
        for name in ATMOSPHERE_FIELDS {
            assert!(atmosphere.field(name).is_some(), "{name}");
        }
        assert_eq!(atmosphere.field("Wind_X"), Some(4.0));
        assert_eq!(atmosphere.field("humidity"), None);
    }

    #[test]
    fn publishing_actors_preserves_input_and_invalidates_cached_names() {
        let mut sensors = crowd(&[("a1", "Old")]);
        sensors.keys.insert("space".into());
        for _ in 0..4 {
            assert!(sensors.find("Old").is_some());
        }
        publish(sensors);
        publish_actors(crowd(&[("a1", "New")]).actors);
        read(|s| {
            assert!(s.keys.contains("space"));
            assert!(s.find("Old").is_none());
            assert_eq!(s.find("New").unwrap().name, "New");
        });
    }

    #[test]
    fn publishing_run_context_preserves_actor_and_client_state() {
        let mut sensors = crowd(&[("a1", "Player")]);
        sensors.keys.insert("space".into());
        sensors.ui_focus = "pause".into();
        publish(sensors);
        publish_run(2.0, 4.0, true, "Room", &["Room".into(), "Next".into()]);
        read(|s| {
            assert_eq!((s.time, s.wall_time, s.paused), (2.0, 4.0, true));
            assert_eq!(s.current_scene, "Room");
            assert_eq!(s.scene_names, ["Room", "Next"]);
            assert!(s.find("Player").is_some());
            assert!(s.keys.contains("space"));
            assert_eq!(s.ui_focus, "pause");
        });
        publish_run(3.0, 5.0, false, "Next", &["Next".into()]);
        read(|s| {
            assert_eq!(s.current_scene, "Next");
            assert_eq!(s.scene_names, ["Next"]);
        });
    }

    #[test]
    fn publishing_the_atmosphere_leaves_the_rest_of_the_snapshot() {
        publish(Sensors {
            time: 3.0,
            ..Default::default()
        });
        publish_atmosphere(AtmosphereSense {
            tick: 7,
            ..Default::default()
        });
        assert_eq!(read(|s| (s.time, s.atmosphere.tick)), (3.0, 7));
    }

    #[test]
    fn the_current_actor_is_restored_after_a_nested_scope() {
        with_actor("outer", || {
            with_actor("inner", || {
                assert_eq!(current_actor().as_deref(), Some("inner"))
            });
            assert_eq!(current_actor().as_deref(), Some("outer"));
        });
        assert_eq!(current_actor(), None);
    }
}
