//! The runtime's own state: the loaded project, its block scheduler, and which
//! entity each actor is.
//!
//! This is a `!Send` resource (the VM holds `Rc`s), which is exactly what's
//! wanted: every system that touches it is therefore scheduled on the main
//! thread, and so is the thread-local sensor snapshot the VM reads through.

use bevy::prelude::*;
use blockloom_core::components::CameraAttach;
use blockloom_core::project::{Actor, Project};
use blockloom_core::save::SaveData;
use blockloom_core::scene::Mode;
use blockloom_core::value::Evaluated;
use blockloom_core::vm::{Dicts, Lists, Variables, Vm};
use blockloom_protocol::EditorMessage;
use blockloom_protocol::PreviewInput;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// Marks a spawned actor and ties it back to its id in the project.
#[derive(Component, Debug, Clone)]
pub struct ActorId(pub String);

/// One loaded script on one actor, with the path that names it. An actor
/// runs its scripts in list order: `start`, then `event`, then `tick`.
pub struct ActorScript {
    pub path: String,
    pub script: ScriptBackend,
}

/// Which code runs a script: the native library Play builds, or the
/// sandboxed wasm module beside it. Native is preferred for speed; wasm is
/// the fallback (and the opt-in sandbox via `BLOCKLOOM_SCRIPT_BACKEND=wasm`),
/// so a project with only web-built scripts still plays on desktop and any
/// language targeting the three host imports runs untrusted.
pub enum ScriptBackend {
    Native(crate::script::LoadedScript),
    #[cfg(not(target_arch = "wasm32"))]
    Wasm(Box<crate::script_wasm::WasmScript>),
}

impl ScriptBackend {
    /// Whether the sandbox is preferred over native speed for this run.
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    fn prefers_wasm() -> bool {
        std::env::var("BLOCKLOOM_SCRIPT_BACKEND").is_ok_and(|value| value == "wasm")
    }

    /// Whether Play can run `relative` without rebuilding: either artifact
    /// counts. On Android only the native library beside the runtime does.
    pub fn is_built(project_dir: &std::path::Path, relative: &str) -> bool {
        #[cfg(any(target_os = "android", target_arch = "wasm32"))]
        {
            crate::script::LoadedScript::is_built(project_dir, relative)
        }
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        {
            crate::script::LoadedScript::is_built(project_dir, relative)
                || blockloom_core::script::library_path_for(
                    project_dir,
                    relative,
                    Some(blockloom_core::script::WEB_TARGET),
                )
                .is_file()
        }
    }

    /// Opens `relative`'s script, native first (or wasm first under
    /// `BLOCKLOOM_SCRIPT_BACKEND=wasm`), falling back to whichever built
    /// artifact loads. When both fail the primary error is reported, so a
    /// broken native build doesn't surface as a confusing wasm complaint.
    pub fn load(project_dir: &std::path::Path, relative: &str) -> Result<ScriptBackend, String> {
        #[cfg(any(target_os = "android", target_arch = "wasm32"))]
        {
            crate::script::LoadedScript::load(project_dir, relative).map(ScriptBackend::Native)
        }
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        {
            if Self::prefers_wasm() {
                match crate::script_wasm::WasmScript::load_file(project_dir, relative) {
                    Ok(script) => Ok(ScriptBackend::Wasm(Box::new(script))),
                    Err(wasm_error) => {
                        match crate::script::LoadedScript::load(project_dir, relative) {
                            Ok(script) => Ok(ScriptBackend::Native(script)),
                            Err(_) => Err(wasm_error),
                        }
                    }
                }
            } else {
                match crate::script::LoadedScript::load(project_dir, relative) {
                    Ok(script) => Ok(ScriptBackend::Native(script)),
                    Err(native_error) => {
                        match crate::script_wasm::WasmScript::load_file(project_dir, relative) {
                            Ok(script) => Ok(ScriptBackend::Wasm(Box::new(script))),
                            Err(_) => Err(native_error),
                        }
                    }
                }
            }
        }
    }

    pub fn start(&self, actor: &str, asked: &mut crate::script::Asked) {
        match self {
            ScriptBackend::Native(script) => script.start(actor, asked),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.start(actor, asked),
        }
    }

    pub fn tick(&self, actor: &str, asked: &mut crate::script::Asked, dt: f32) {
        match self {
            ScriptBackend::Native(script) => script.tick(actor, asked, dt),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.tick(actor, asked, dt),
        }
    }

    pub fn frame(&self, actor: &str, asked: &mut crate::script::Asked, dt: f32) {
        match self {
            ScriptBackend::Native(script) => script.frame(actor, asked, dt),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.frame(actor, asked, dt),
        }
    }

    pub fn ui(&self, actor: &str, asked: &mut crate::script::Asked, dt: f32) {
        match self {
            ScriptBackend::Native(script) => script.ui(actor, asked, dt),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.ui(actor, asked, dt),
        }
    }

    pub fn stop(&self, actor: &str, asked: &mut crate::script::Asked) {
        match self {
            ScriptBackend::Native(script) => script.stop(actor, asked),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.stop(actor, asked),
        }
    }

    pub fn destroy(&self, actor: &str, asked: &mut crate::script::Asked) {
        match self {
            ScriptBackend::Native(script) => script.destroy(actor, asked),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.destroy(actor, asked),
        }
    }

    pub fn event(
        &self,
        actor: &str,
        asked: &mut crate::script::Asked,
        event: &crate::script::ScriptEvent,
    ) {
        match self {
            ScriptBackend::Native(script) => script.event(actor, asked, event),
            #[cfg(not(target_arch = "wasm32"))]
            ScriptBackend::Wasm(script) => script.event(actor, asked, event),
        }
    }
}

/// The actor's custom components, live. Authored values seed it on every
/// rebuild; `set <field> of <component>` writes here, and the sensing
/// snapshot reads back out, so a run's changes last exactly as long as the
/// run does - like a position, and unlike a variable.
#[derive(Component, Debug, Clone, Default)]
pub struct CustomComponents(pub HashMap<String, HashMap<String, Evaluated>>);

/// A camera component on this actor. Mirrored onto the entity so
/// `set camera to first person` can change it without touching the document.
#[derive(Component, Debug, Clone, Copy)]
pub struct CameraRig(pub CameraAttach);

/// A `glide` in progress: the host interpolates while the script sleeps.
#[derive(Component, Debug, Clone)]
pub struct Gliding {
    pub from: Vec3,
    pub to: Vec3,
    pub elapsed: f32,
    pub duration: f32,
    pub easing: blockloom_core::animation::TweenEasing,
}

/// A `tween size` in progress: like a glide, but over the scale.
#[derive(Component, Debug, Clone)]
pub struct TweeningScale {
    pub from: f32,
    pub to: f32,
    pub elapsed: f32,
    pub duration: f32,
    pub easing: blockloom_core::animation::TweenEasing,
}

/// A `tween rotation` in progress: one axis, eased.
#[derive(Component, Debug, Clone)]
pub struct TweeningRotation {
    pub axis: blockloom_core::scene::Axis,
    pub from: f32,
    pub to: f32,
    pub elapsed: f32,
    pub duration: f32,
    pub easing: blockloom_core::animation::TweenEasing,
}

/// A `tween color` in progress: from tint to tint, eased. `from` fills in on
/// the first step from whatever is showing, so a retarget eases out of the
/// live color rather than snapping back to the authored one.
#[derive(Component, Debug, Clone)]
pub struct TweeningColor {
    pub from: Option<Color>,
    pub to: Color,
    pub elapsed: f32,
    pub duration: f32,
    pub easing: blockloom_core::animation::TweenEasing,
}

/// The animation player: which clip is showing, in which state, and how far
/// in. Clips are authored on the `Animation` component; this is the live
/// cursor over them, stepped on the fixed tick (`anim2d::step_animations`).
/// A `Once` clip at its end fires `when animation ends` once.
#[derive(Component, Debug, Clone, Default)]
pub struct AnimationPlayer {
    pub clip: String,
    /// The state playing the clip, or empty for a bare clip.
    pub state: String,
    pub elapsed: f32,
    pub speed: f32,
    pub playing: bool,
    pub ended_fired: bool,
    /// The last step markers were counted to; `None` right after a start.
    pub last_step: Option<u64>,
    /// The 1-based frame showing, which `current frame` reads.
    pub frame: usize,
    /// The state hands the clip's motion to the actor.
    pub root_motion: bool,
    /// The clip being crossfaded out of, if any.
    pub fade: Option<AnimationFade>,
    /// Triggers fired this tick, for the state machine.
    pub triggers: Vec<String>,
}

/// The previous clip, still advancing while it fades out.
#[derive(Debug, Clone)]
pub struct AnimationFade {
    pub clip: String,
    pub elapsed: f32,
    pub speed: f32,
    pub left: f32,
    pub total: f32,
}

/// The pose an actor settled at the end of a fixed step - physically, or from
/// that step's own effects. The renderer lerps between this and [`PrevPose`]
/// to smooth the gaps between fixed steps.
#[derive(Component, Debug, Clone)]
pub struct PhysicsPose(pub Transform);

/// The pose one fixed step older than [`PhysicsPose`].
#[derive(Component, Debug, Clone)]
pub struct PrevPose(pub Transform);

/// Effects produced by this frame's VM tick, waiting to be applied.
#[derive(Resource, Default)]
pub struct PendingEffects(pub Vec<blockloom_core::vm::Effect>);

/// Which dimension this process was started for. The editor restarts the
/// runtime when a project switches mode, so it never changes here.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimension(pub Mode);

pub struct Engine {
    pub lan: crate::lan::Session,
    pub incoming: Receiver<EditorMessage>,
    /// A built game's own end of that channel. There is no editor to send it
    /// Load and Start, so it sends them to itself and holds the sender for as
    /// long as it runs - dropping it would read as the editor hanging up.
    pub link: Option<Sender<EditorMessage>>,
    pub project: Project,
    pub vm: Vm,
    pub variables: Variables,
    pub lists: Lists,
    pub dicts: Dicts,
    /// Per-player values for this project's explicitly saved variables.
    pub save_data: SaveData,
    pub save_path: PathBuf,
    /// Which save slot this run writes to, by normalized name. A slot name
    /// doubles as a profile name, so "Slot 1" and a player name are the
    /// same file either way.
    pub save_slot: String,
    /// Every slot with a file on disk, refreshed when a slot is switched
    /// or deleted rather than listed every tick.
    pub save_slots: Vec<String>,
    /// The language this run speaks, lowercased. What `text for key`
    /// answers in; moved by `set language to`.
    pub language: String,
    /// A built game's native block program. Editor Play keeps using the VM so
    /// what is being edited always runs immediately.
    pub logic: Option<crate::logic::LoadedLogic>,
    /// The plugin code the editor asked this run to host.
    pub plugins: crate::plugins::PluginHost,
    /// How many actors the host itself has made this run. Only a compiled
    /// program's run needs them: it mints its own ids, so the host's carry a
    /// shape of their own and the two can never collide.
    made: usize,
    /// Actor id -> its entity, for as long as the world stands.
    pub entities: HashMap<String, Entity>,
    pub running: bool,
    pub paused: bool,
    /// Play was pressed and the run begins once the world is warm (see
    /// `streaming::warm_up`). Reads as running to the editor.
    pub starting: bool,
    /// Whether a Start waits for the world to warm up. Set by `add_world`,
    /// so a bare test world starts on the spot.
    pub prewarm: bool,
    /// Whether the game window actually holds OS focus, from `WindowFocused`
    /// events only. Bevy's `Window::focused` defaults to true and winit only
    /// reports changes, so a window opened behind the editor would read
    /// focused forever without this - and raw device input would drive a game
    /// whose cursor sits in another window.
    pub window_focused: bool,
    /// Whether a `lock mouse` block wants the pointer grabbed right now.
    /// Kept apart from the window's own options because the backend only
    /// attempts the grab on change: a request that lands before the window is
    /// focused degrades to confined-or-nothing, so the runtime re-asserts
    /// from this flag until the real lock sticks.
    pub wants_cursor_locked: bool,
    /// A look camera has already asked for the pointer this run.
    pub look_lock_offered: bool,
    /// `Time::elapsed_secs` when the current pause began, if paused. Used to
    /// keep the `timer` reporter frozen while paused.
    pub pause_began: Option<f64>,
    /// `Time::elapsed_secs_f64` when the green flag was pressed.
    pub started_at: f64,
    /// `Time<Real>` seconds when the green flag was pressed. The wall clock a
    /// UI strand sleeps against counts from here and never pauses or scales.
    pub wall_started_at: f64,
    /// Who is touching whom, from collision messages, by actor id.
    pub touching: HashMap<String, HashSet<String>>,
    /// Every collider pair that touches, with the events they made.
    pub contacts: blockloom_core::physics::ContactTracker,
    /// Fixed ticks the contact tracker has closed this run.
    pub contact_ticks: u64,
    /// Motor problems already logged this run.
    pub motor_warned: std::collections::HashSet<String>,
    /// Actors whose collision filter changed since the last tick closed, so an
    /// Exit it causes says why.
    pub filter_touched: HashSet<String>,
    /// The current speech bubble for each actor. A later `say` replaces the
    /// earlier one, and an empty `say` clears it.
    pub speech: HashMap<String, String>,
    /// When the next status report is due, in elapsed seconds.
    pub next_report: f64,
    /// Set when the world needs rebuilding from `project` before the next tick.
    pub rebuild: bool,
    /// The open project's folder, which is where its assets and its built
    /// script libraries are. `None` until the editor says.
    pub project_dir: Option<PathBuf>,
    /// Each actor's loaded scripts, by actor id, in component order. Reopened
    /// on every rebuild, so a script edited and rebuilt between runs takes
    /// effect on the next Play.
    pub scripts: HashMap<String, Vec<ActorScript>>,
    /// Smoothed milliseconds per script path, for the profiler's `script/*`
    /// rows. Timed around each entry's callbacks so one heavy file stands
    /// out from a busy project.
    pub script_times: HashMap<String, f64>,
    /// Which scripts have had their `start` called, by actor and path. Per
    /// script rather than one flag for the run, since a clone made half way
    /// through still needs its own.
    pub scripts_started: HashSet<(String, String)>,
    /// Events fired since scripts last ran, for their `event` entry points.
    pub script_events: Vec<blockloom_core::vm::Event>,
    /// Which components each actor is carrying right now. Seeded from the
    /// project on every rebuild and moved by `attach`/`detach`, so it - not
    /// the document - is what a mid-run question about a component answers.
    pub attached: HashMap<String, HashSet<String>>,
    /// Actors the run made that the document never had: clones, and actors a
    /// `create actor` block conjured. Looked up before the project, so the
    /// rest of the runtime asks one question to find any actor at all.
    pub spawned: HashMap<String, Actor>,
    /// Where `project.actors` last held each id looked up. Checked against the
    /// actor found there, so a stale slot is a miss, never a wrong answer.
    actor_slots: std::cell::RefCell<HashMap<String, usize>>,
    /// Clone id -> the authored actor it was copied from. Only clones are in
    /// here, which is what "am I a clone?" reads.
    pub clones: HashMap<String, String>,
    /// Child actor id -> the actor it hangs off. Seeded from every `Parent`
    /// component on a rebuild and moved by `set my parent to` after that.
    pub parents: HashMap<String, String>,
    /// Actor id -> the last actor or clone it made, for "the actor I made".
    pub last_created: HashMap<String, String>,
    /// Actor id -> lumens a block or script set its light to this run.
    pub light_intensity: HashMap<String, f32>,
    /// Actor id -> what `enable volume` and `set weight of volume` set this
    /// run, over the authored `Volume`.
    pub volume_enabled: HashMap<String, bool>,
    pub volume_weight: HashMap<String, f32>,
    /// `set time scale` this run, over 1. A cutscene's slow-motion keys
    /// only move `cine_scale` while this is unset.
    pub time_scale: Option<f32>,
    /// The playing cutscene's slow-motion keys, while one plays.
    pub cine_scale: Option<f32>,
    /// The cutscene playing right now, by name, and seconds into it. What
    /// `is cutscene playing?` and `cutscene time` read.
    pub cine_name: String,
    pub cine_time: f32,
    /// Actor id -> whether `turn my light's shadows` left its light casting.
    pub light_shadows: HashMap<String, bool>,
    /// Set once on Mali GPUs: their driver holds per-shadow-view memory to
    /// OOM, so point and spot lights never get shadow maps there. Latched
    /// by `lights::sync_lights`; the sun keeps its own maps.
    pub no_point_shadow_maps: bool,
    /// `set shadow distance` this run, over the project's.
    pub shadow_distance: Option<f32>,
    /// A `capture probes` waiting for the probes to pick it up.
    pub capture_probes: bool,
    /// `set HDR output` and `set peak brightness` this run, over the
    /// project's display settings.
    pub hdr_output: Option<bool>,
    pub peak_nits: Option<f32>,
    /// `enable ray tracing`, `set GI bounces` and `set GI samples` this run,
    /// over the project's ray tracing settings.
    pub ray_tracing: Option<bool>,
    pub gi_bounces: Option<u32>,
    pub gi_samples: Option<u32>,
    /// `set fog density`, `set aurora` and `set lightning storm` this run,
    /// over the project's fog, sky and lightning.
    pub fog_density: Option<f32>,
    pub aurora_kp: Option<f32>,
    pub lightning_rate: Option<f32>,
    /// `set wind`, `set storm` and `set cloud drift` this run, over the
    /// project's wind.
    pub wind: blockloom_core::wind::WindOverrides,
    /// `set precipitation` this run, over the director and the project.
    pub precipitation: blockloom_core::director::PrecipitationOverrides,
    /// The live weather blend: what the air is right now and where it is
    /// going. Stepped on the fixed tick; `blend weather to` moves it.
    pub weather: blockloom_core::director::WeatherState,
    /// The director clock's explicit time, 0-24, once `set time of day` or
    /// `advance time by` has moved it. `None` follows the project's clock.
    pub director_time: Option<f32>,
    /// `set clouds` this run, over the blended volumetric clouds.
    pub clouds: blockloom_core::clouds::CloudOverrides,
    /// `set snow cover to` and `set surface wetness to`, for the run.
    pub surface: SurfaceOverrides,
    /// Erosion previews the editor asked for, by terrain actor.
    pub terrain_previews: Vec<(String, Option<blockloom_core::terrain::sculpt::Erosion>)>,
    /// `set cloud layer` this run, over the project's layers.
    pub cloud_layers: blockloom_core::cloud_layers::CloudLayerOverrides,
    /// `set water level/chop/foam` this run, over each body's authored spec.
    pub water: blockloom_core::water::WaterOverrides,
    /// Dynamic actors a walk verb (`move`, `change position`) drove this
    /// tick. A walk sets an absolute velocity, so when a driven actor goes
    /// quiet the dimension pass brakes it - otherwise the last written
    /// speed glides on for seconds. Only ever walk-driven actors are in
    /// here, so solver-driven things (a ball off a collision, an impulse)
    /// keep their inertia.
    pub driven: HashSet<String>,
    /// Live collision filter per actor: (layer, mask, trigger). Seeded from
    /// the document on every rebuild and moved by the `set trigger` /
    /// `set collision` blocks, so queries read this run's values rather than
    /// the authored ones.
    pub physics_filter: HashMap<String, (u8, u8, bool)>,
    /// Run-scoped input remaps: lowercase action name -> bindings. Seeded
    /// empty on every rebuild; the `bind`/`clear` blocks move it from there,
    /// so a settings screen remaps for this run without touching the
    /// document.
    pub input_overrides: HashMap<String, Vec<blockloom_core::input::InputBinding>>,
    /// Whether each action was held last frame, by lowercase name. What
    /// turns a held action into a one-frame `pressed`/`released`.
    pub prev_action_held: HashMap<String, bool>,
    /// Input events forwarded from the embedded preview, drained once a
    /// frame by the preview systems.
    pub preview_inputs: Vec<PreviewInput>,
    /// A single fixed tick to run while paused, then re-pause. What the
    /// editor's step button asks for.
    pub pause_after_tick: bool,
    /// A `switch scene to` waiting to unload the current scene and load the
    /// next: `(scene name as the block spelled it, transition, ticks left)`.
    /// Set when the effect lands, drained after the outgoing scene's `when
    /// scene ends` strands have had a tick to run. A named transition covers
    /// the outgoing scene on the wall clock first (see `transition`); the
    /// swap waits for cover.
    pub pending_scene: Option<(String, String, u8)>,
    /// The wall-clock veil over a scene switch. Started by the fixed step
    /// alongside `pending_scene`, drawn by `transition::drive_veil`.
    pub veil: crate::transition::SceneVeil,
    /// Survivor ids carried across the last scene switch: authored or spawned
    /// actors with `Persist` that the next rebuild must spawn alongside the
    /// new scene instead of dropping with the old one. Set by the switch,
    /// consumed by the rebuild, then cleared - the actors themselves live in
    /// `spawned` from then on.
    pub survivor_keep: std::collections::HashSet<String>,
}

impl Engine {
    pub fn new(incoming: Receiver<EditorMessage>, mode: Mode) -> Self {
        let variables = Variables::default();
        let lists = Lists::default();
        let dicts = Dicts::default();
        Self {
            #[allow(clippy::default_constructed_unit_structs)]
            lan: crate::lan::Session::default(),
            incoming,
            link: None,
            project: Project::starter("Untitled", mode),
            vm: Vm::with_stores(variables.clone(), lists.clone(), dicts.clone()),
            variables,
            lists,
            dicts,
            save_data: SaveData::default(),
            save_path: PathBuf::new(),
            save_slot: blockloom_core::save::DEFAULT_SLOT.to_string(),
            save_slots: Vec::new(),
            language: blockloom_core::locale::DEFAULT_LANGUAGE.to_string(),
            logic: None,
            plugins: Default::default(),
            made: 0,
            entities: HashMap::new(),
            running: false,
            paused: false,
            window_focused: false,
            wants_cursor_locked: false,
            look_lock_offered: false,
            pause_began: None,
            started_at: 0.0,
            wall_started_at: 0.0,
            touching: HashMap::new(),
            contacts: Default::default(),
            contact_ticks: 0,
            motor_warned: Default::default(),
            filter_touched: HashSet::new(),
            speech: HashMap::new(),
            next_report: 0.0,
            rebuild: true,
            starting: false,
            prewarm: false,
            project_dir: None,
            scripts: HashMap::new(),
            scripts_started: HashSet::new(),
            script_events: Vec::new(),
            // Smoothed milliseconds per script path, for the profiler's
            // `script/*` rows. Timed around each entry's callbacks so one
            // heavy file stands out from a busy project.
            script_times: HashMap::new(),
            attached: HashMap::new(),
            spawned: HashMap::new(),
            clones: HashMap::new(),
            parents: HashMap::new(),
            light_intensity: HashMap::new(),
            volume_enabled: HashMap::new(),
            volume_weight: HashMap::new(),
            time_scale: None,
            cine_scale: None,
            cine_name: String::new(),
            cine_time: 0.0,
            light_shadows: HashMap::new(),
            no_point_shadow_maps: false,
            shadow_distance: None,
            capture_probes: false,
            hdr_output: None,
            peak_nits: None,
            ray_tracing: None,
            gi_bounces: None,
            gi_samples: None,
            fog_density: None,
            aurora_kp: None,
            lightning_rate: None,
            wind: Default::default(),
            precipitation: Default::default(),
            weather: Default::default(),
            director_time: None,
            clouds: Default::default(),
            surface: Default::default(),
            terrain_previews: Vec::new(),
            cloud_layers: Default::default(),
            water: Default::default(),
            last_created: HashMap::new(),
            driven: HashSet::new(),
            physics_filter: HashMap::new(),
            input_overrides: HashMap::new(),
            prev_action_held: HashMap::new(),
            preview_inputs: Vec::new(),
            pause_after_tick: false,
            pending_scene: None,
            veil: crate::transition::SceneVeil::default(),
            survivor_keep: Default::default(),
            actor_slots: Default::default(),
        }
    }

    /// Any actor in the running world: one the run made first, then one the
    /// document authored. Everything that needs an actor's authored shape,
    /// physics or components goes through here, so a clone answers the same
    /// questions the actor it was copied from does.
    pub fn actor(&self, id: &str) -> Option<&Actor> {
        self.spawned.get(id).or_else(|| self.authored(id))
    }

    /// An authored actor by id. The document keeps actors in a list, so each
    /// hit is remembered by position; every frame asks about every actor.
    fn authored(&self, id: &str) -> Option<&Actor> {
        let actors = &self.project.actors;
        let mut slots = self.actor_slots.borrow_mut();
        if let Some(actor) = slots.get(id).and_then(|&i| actors.get(i))
            && actor.id == id
        {
            return Some(actor);
        }
        let found = actors.iter().position(|actor| actor.id == id)?;
        // Ids that left the document would pile up over a long run.
        if slots.len() > actors.len() * 2 + 64 {
            slots.clear();
        }
        slots.insert(id.to_string(), found);
        actors.get(found)
    }

    /// The per-axis stretch an actor was authored with, which no block moves.
    pub fn stretch_of(&self, id: &str) -> [f32; 3] {
        self.actor(id)
            .map_or([1.0; 3], |actor| actor.placement().stretch)
    }

    /// Every actor in the world, authored and made, by id.
    pub fn actor_ids(&self) -> impl Iterator<Item = &String> {
        self.entities.keys()
    }

    /// Whether hanging `child` off `parent` would make a loop - following
    /// `parent` upwards eventually arrives back at `child`.
    pub fn would_loop(&self, child: &str, parent: &str) -> bool {
        blockloom_core::project::reaches(&self.parents, parent, child)
    }

    /// Whether `actor` is carrying `component` at this moment in the run.
    pub fn has_component(&self, actor: &str, component: &str) -> bool {
        self.attached
            .get(actor)
            .is_some_and(|held| held.contains(component))
    }

    /// This run's collision filter for `actor`: live overrides first, then
    /// what the document authored. Always clamped, so a stray value can't
    /// shift a bit out of the mask.
    pub fn filter_of(&self, id: &str) -> (u8, u8, bool) {
        if let Some(filter) = self.physics_filter.get(id) {
            return *filter;
        }
        match self.actor(id) {
            Some(actor) => {
                let physics = actor.physics();
                (physics.layer(), physics.collision_mask, physics.trigger)
            }
            None => (1, 0xFF, false),
        }
    }

    /// Records a mid-run filter change, keeping the two halves the effect
    /// didn't name.
    pub fn set_filter(
        &mut self,
        id: &str,
        layer: Option<u8>,
        mask: Option<u8>,
        trigger: Option<bool>,
    ) {
        let (old_layer, old_mask, old_trigger) = self.filter_of(id);
        self.filter_touched.insert(id.to_string());
        self.physics_filter.insert(
            id.to_string(),
            (
                layer.unwrap_or(old_layer).clamp(1, 8),
                mask.unwrap_or(old_mask),
                trigger.unwrap_or(old_trigger),
            ),
        );
    }

    /// This run's bindings for an action: a `bind`/`clear` override first,
    /// then what the document authored. `None` for an action nobody defined.
    pub fn effective_bindings(
        &self,
        action: &str,
    ) -> Option<Vec<blockloom_core::input::InputBinding>> {
        let key = action.trim().to_lowercase();
        if let Some(bindings) = self.input_overrides.get(&key) {
            return Some(bindings.clone());
        }
        self.project
            .world
            .input
            .find(action)
            .map(|found| found.bindings.clone())
    }

    /// Clears every run-scoped remap and edge, so a fresh Play starts from
    /// the document again.
    pub fn reset_input_run(&mut self) {
        self.input_overrides.clear();
        self.prev_action_held.clear();
    }

    /// Seconds since the green flag, which is what the `timer` reporter reads.
    /// Frozen while paused, so resuming doesn't jump the timer forward.
    pub fn run_time(&self, now: f64) -> f64 {
        let end = match self.pause_began {
            Some(began) => began.min(now),
            None => now,
        };
        (end - self.started_at).max(0.0)
    }

    /// Seconds of real time since the green flag: unaffected by pause, game
    /// speed and a stalled frame, and what a UI strand's `wait` counts.
    pub fn wall_time(&self, real_now: f64) -> f64 {
        (real_now - self.wall_started_at).max(0.0)
    }

    pub fn actor_id_of(&self, entity: Entity) -> Option<&str> {
        self.entities
            .iter()
            .find(|(_, candidate)| **candidate == entity)
            .map(|(id, _)| id.as_str())
    }

    pub fn note_say(&mut self, actor: &str, text: &str) {
        if text.is_empty() {
            self.speech.remove(actor);
        } else {
            self.speech.insert(actor.to_string(), text.to_string());
        }
    }

    /// An id for an actor the host is making itself, distinct from anything
    /// the VM or a compiled program mints.
    pub fn new_actor_id(&mut self) -> String {
        self.made += 1;
        format!("~h{}", self.made)
    }

    pub fn fire(&mut self, event: blockloom_core::vm::Event) {
        // Queued even while paused: the fixed `tick` skips paused steps, but
        // the per-frame `ui` entry point drains these so menus answer while
        // the world is frozen, the way UI strands do.
        if self.running && !self.scripts.is_empty() {
            self.script_events.push(event.clone());
        }
        if let Some(logic) = &mut self.logic {
            logic.fire(event, &self.project);
        } else {
            self.vm.fire(event);
        }
    }

    pub fn stop_program(&mut self) {
        self.vm.stop_all();
        self.made = 0;
        if let Some(logic) = &mut self.logic {
            logic.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn say_replaces_and_empty_say_clears_an_actors_bubble() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);

        engine.note_say("player", "Hello");
        engine.note_say("player", "Still here");
        assert_eq!(
            engine.speech.get("player").map(String::as_str),
            Some("Still here")
        );

        engine.note_say("player", "");
        assert!(!engine.speech.contains_key("player"));
    }

    /// A runnable wasm artifact with no behavior: the imports, memory and
    /// entry points are the contract, not what the module does.
    #[cfg(not(target_arch = "wasm32"))]
    fn quiet_wasm() -> Vec<u8> {
        let abi = blockloom_core::script::abi::ABI_VERSION;
        wat::parse_str(format!(
            r#"(module
  (import "blockloom" "read_number" (func (param i32 i32 i32) (result i32)))
  (import "blockloom" "read_text" (func (param i32 i32 i32) (result i32)))
  (import "blockloom" "act" (func (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "blockloom_script_abi") (result i32) i32.const {abi})
  (func (export "blockloom_script_start") (param i32 i32))
  (func (export "blockloom_script_tick") (param i32 i32 f32))
  (func (export "blockloom_script_event") (param i32 i32 i32 f64 f64 f64 f64))
)"#
        ))
        .expect("the fixture is valid")
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn script_backend_falls_back_to_wasm_when_native_is_missing() {
        let dir =
            std::env::temp_dir().join(format!("blockloom-script-backend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let relative = "assets/scripts/player.rs";
        assert!(!ScriptBackend::is_built(&dir, relative));
        assert!(
            ScriptBackend::load(&dir, relative)
                .err()
                .expect("missing scripts fail to load")
                .contains("hasn't been built")
        );
        // Only the web-built artifact exists: that counts as built, and
        // loading falls back to the sandbox instead of failing.
        let path = blockloom_core::script::library_path_for(
            &dir,
            relative,
            Some(blockloom_core::script::WEB_TARGET),
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, quiet_wasm()).unwrap();
        assert!(ScriptBackend::is_built(&dir, relative));
        assert!(matches!(
            ScriptBackend::load(&dir, relative),
            Ok(ScriptBackend::Wasm(_))
        ));
        // The opt-in sandbox order loads the same artifact first.
        let previous = std::env::var("BLOCKLOOM_SCRIPT_BACKEND").ok();
        unsafe { std::env::set_var("BLOCKLOOM_SCRIPT_BACKEND", "wasm") };
        let loaded = ScriptBackend::load(&dir, relative);
        unsafe {
            match previous {
                Some(value) => std::env::set_var("BLOCKLOOM_SCRIPT_BACKEND", value),
                None => std::env::remove_var("BLOCKLOOM_SCRIPT_BACKEND"),
            }
        }
        assert!(matches!(loaded, Ok(ScriptBackend::Wasm(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Surface weather blocks and scripts set for the run, laid over the
/// blended environment.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SurfaceOverrides {
    pub snow: Option<f32>,
    pub wetness: Option<f32>,
}
