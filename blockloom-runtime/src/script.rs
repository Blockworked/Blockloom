//! Loading and running an actor's compiled Rust script.
//!
//! The editor builds `assets/scripts/*.rs` into shared libraries before Play
//! (see `blockloom_core::script`); this half opens them, checks they were
//! built against the ABI this binary speaks, and calls their entry points
//! once a frame.
//!
//! A script reads the world through the same frame snapshot the block
//! reporters read (`sense`), and everything it does comes back as a
//! [`Effect`] - the same ones the VM emits, applied by the same systems. So a
//! script and a canvas can drive one actor between them, and neither has to
//! know about the other.

use blockloom_core::components::CameraView;
use blockloom_core::scene::Axis;
use blockloom_core::script::abi;
#[cfg(not(target_arch = "wasm32"))]
use blockloom_core::script::abi::{HostApi, Str};
use blockloom_core::sense;
use blockloom_core::sound::{SoundBus, clamp_pitch, user_to_gain};
use blockloom_core::ui::{UiAnchor, UiElement, UiKind, UiProp, UiTheme};
use blockloom_core::value::Evaluated;
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::c_void;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
type StartFn = unsafe extern "C" fn(*mut c_void, *const HostApi);
#[cfg(not(target_arch = "wasm32"))]
type TickFn = unsafe extern "C" fn(*mut c_void, *const HostApi, f32);
#[cfg(not(target_arch = "wasm32"))]
type EventFn = unsafe extern "C" fn(*mut c_void, *const HostApi, u32, f64, f64, f64, f64);
#[cfg(not(target_arch = "wasm32"))]
type AbiFn = unsafe extern "C" fn() -> u32;

/// One actor's script, open and ready to call. The library is kept alive
/// alongside the pointers into it, and closing it is what dropping this does.
///
/// In a browser the script is a wasm module of its own instead, which the
/// page compiled and the player instantiates (see [`browser`]).
pub struct LoadedScript {
    #[cfg(target_arch = "wasm32")]
    instance: std::rc::Rc<browser::Instance>,
    /// Dropped last, after the pointers that live inside it.
    #[cfg(not(target_arch = "wasm32"))]
    library: libloading::Library,
    #[cfg(not(target_arch = "wasm32"))]
    start: StartFn,
    #[cfg(not(target_arch = "wasm32"))]
    tick: TickFn,
    #[cfg(not(target_arch = "wasm32"))]
    event: EventFn,
}

/// Where `relative`'s library opens from. Desktop joins the project dir;
/// Android resolves the file name beside this library in the app's lib dir.
#[cfg(target_os = "android")]
fn library_path_for(project_dir: &Path, relative: &str) -> std::path::PathBuf {
    let path = blockloom_core::script::library_path(project_dir, relative);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("libscript.so");
    crate::android::native_lib_path(name)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn library_path_for(project_dir: &Path, relative: &str) -> std::path::PathBuf {
    blockloom_core::script::library_path(project_dir, relative)
}

#[cfg(not(target_arch = "wasm32"))]
impl LoadedScript {
    /// Whether the editor has built this script yet. Before the first Play it
    /// hasn't, which is ordinary rather than a problem worth reporting. On
    /// Android the build fails when a script lib is missing, so by the time
    /// the APK exists every script rides beside the runtime: a missing one
    /// is a load error, never a silent skip.
    #[cfg(not(target_os = "android"))]
    pub fn is_built(project_dir: &Path, relative: &str) -> bool {
        blockloom_core::script::library_path(project_dir, relative).is_file()
    }

    #[cfg(target_os = "android")]
    pub fn is_built(_project_dir: &Path, _relative: &str) -> bool {
        true
    }

    /// Opens the library the editor built for `relative`, or says why it
    /// couldn't be used. On Android that library is a `lib/<abi>/` entry
    /// unpacked beside this one, found by file name.
    pub fn load(project_dir: &Path, relative: &str) -> Result<LoadedScript, String> {
        let path = library_path_for(project_dir, relative);
        #[cfg(not(target_os = "android"))]
        if !path.is_file() {
            return Err(format!("{relative} hasn't been built"));
        }
        // Safety: the file is one this build's editor produced with rustc,
        // and the ABI check below is what stands between us and an older one.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|e| format!("{}: {e}", path.display()))?;
        unsafe {
            let abi = library
                .get::<AbiFn>(abi::SYM_ABI)
                .map_err(|_| format!("{relative} isn't a Blockloom script"))?;
            let version = abi();
            if version != abi::ABI_VERSION {
                return Err(format!(
                    "{relative} was built against script ABI {version}, this runtime speaks {}. \
                     Delete .blockloom/build and press Play again.",
                    abi::ABI_VERSION
                ));
            }
            let start = *library
                .get::<StartFn>(abi::SYM_START)
                .map_err(|_| missing_export(relative))?;
            let tick = *library
                .get::<TickFn>(abi::SYM_TICK)
                .map_err(|_| missing_export(relative))?;
            let event = *library
                .get::<EventFn>(abi::SYM_EVENT)
                .map_err(|_| missing_export(relative))?;
            Ok(LoadedScript {
                library,
                start,
                tick,
                event,
            })
        }
    }

    pub fn start(&self, actor: &str, asked: &mut Asked) {
        self.call(actor, asked, |entry, ctx| unsafe {
            (self.start)(ctx, entry);
        });
    }

    pub fn tick(&self, actor: &str, asked: &mut Asked, dt: f32) {
        self.call(actor, asked, |entry, ctx| unsafe {
            (self.tick)(ctx, entry, dt);
        });
    }

    pub fn event(&self, actor: &str, asked: &mut Asked, event: &ScriptEvent) {
        let [n0, n1, n2, n3] = event.numbers;
        with_event_words(event, || {
            self.call(actor, asked, |entry, ctx| unsafe {
                (self.event)(ctx, entry, event.kind, n0, n1, n2, n3);
            })
        });
    }

    /// Builds the context the script calls back through, runs `f`, and leaves
    /// whatever it asked for in `asked`.
    fn call(&self, actor: &str, asked: &mut Asked, f: impl FnOnce(*const HostApi, *mut c_void)) {
        let mut ctx = Ctx { actor, asked };
        let pointer = (&raw mut ctx).cast::<c_void>();
        // The script runs inside `sense::with_actor` so anything it reaches
        // for through the snapshot means its own actor, as it does for a
        // reporter block.
        sense::with_actor(actor, || f(&raw const HOST_API, pointer));
        // Keeps the library - and so the code that just ran - alive across
        // the call, which is the whole reason it is held here.
        let _ = &self.library;
    }
}

/// A browser has no `dlopen`: the page hands the player each script's
/// compiled wasm module, keyed by its path, and loading one instantiates it.
#[cfg(target_arch = "wasm32")]
impl LoadedScript {
    pub fn is_built(_project_dir: &Path, relative: &str) -> bool {
        crate::web::script_module(relative).is_some()
    }

    pub fn load(_project_dir: &Path, relative: &str) -> Result<LoadedScript, String> {
        Ok(LoadedScript {
            instance: browser::open(relative)?,
        })
    }

    pub fn start(&self, actor: &str, asked: &mut Asked) {
        self.instance.call(actor, asked, browser::Entry::Start);
    }

    pub fn tick(&self, actor: &str, asked: &mut Asked, dt: f32) {
        self.instance.call(actor, asked, browser::Entry::Tick(dt));
    }

    pub fn event(&self, actor: &str, asked: &mut Asked, event: &ScriptEvent) {
        with_event_words(event, || {
            self.instance
                .call(actor, asked, browser::Entry::Event(event))
        });
    }
}

/// One event on its way to a script's `event` entry point.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEvent {
    /// One of `abi::EVENT_*`.
    pub kind: u32,
    /// What `TEXT_EVENT` answers.
    pub subject: String,
    pub detail: String,
    pub numbers: [f64; 4],
}

impl ScriptEvent {
    fn new(kind: u32, subject: impl Into<String>) -> ScriptEvent {
        ScriptEvent {
            kind,
            subject: subject.into(),
            detail: String::new(),
            numbers: [0.0; 4],
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> ScriptEvent {
        self.detail = detail.into();
        self
    }

    /// What a script hears of a VM event, and whose script: `None` for every
    /// script, `Some(actor)` for that actor's alone. `name_of` turns an
    /// actor id into its name. Starts and clones are what `start` is for.
    pub fn of(
        event: &blockloom_core::vm::Event,
        name_of: impl Fn(&str) -> String,
    ) -> Option<(Option<String>, ScriptEvent)> {
        use blockloom_core::vm::Event;
        Some(match event {
            Event::Started | Event::Cloned { .. } | Event::Plugin { .. } => return None,
            Event::QualityDropped => (None, ScriptEvent::new(abi::EVENT_QUALITY_DROPPED, "")),
            Event::SceneStarted => (None, ScriptEvent::new(abi::EVENT_SCENE_STARTED, "")),
            Event::SceneEnded => (None, ScriptEvent::new(abi::EVENT_SCENE_ENDED, "")),
            Event::Message(message) => (None, ScriptEvent::new(abi::EVENT_MESSAGE, message)),
            Event::Key(key) => (None, ScriptEvent::new(abi::EVENT_KEY, key)),
            Event::Action(action) => (None, ScriptEvent::new(abi::EVENT_ACTION, action)),
            Event::Touched => (None, ScriptEvent::new(abi::EVENT_TOUCHED, "")),
            Event::Click { actor } => (
                Some(actor.clone()),
                ScriptEvent::new(abi::EVENT_CLICKED, ""),
            ),
            Event::Collision { actor, with } => (
                Some(actor.clone()),
                ScriptEvent::new(abi::EVENT_COLLISION, name_of(with)).detail(with.clone()),
            ),
            Event::Particles { actor, event } => {
                let particles = me(actor).map(|me| (me.particles, me.position));
                let (count, at) = particles.map_or((0, [0.0; 3]), |(particles, position)| {
                    (
                        particles.count(*event),
                        particles.at(*event).unwrap_or(position),
                    )
                });
                let mut out = ScriptEvent::new(abi::EVENT_PARTICLES, event.name().to_lowercase());
                out.numbers = [count as f64, at[0] as f64, at[1] as f64, at[2] as f64];
                (Some(actor.clone()), out)
            }
            Event::AnimationEnded { actor, clip } => (
                Some(actor.clone()),
                ScriptEvent::new(abi::EVENT_ANIMATION_ENDED, clip),
            ),
            Event::AnimationMarker { actor, marker } => (
                Some(actor.clone()),
                ScriptEvent::new(abi::EVENT_ANIMATION_MARKER, marker),
            ),
            Event::UiClicked { id } => (None, ScriptEvent::new(abi::EVENT_UI_CLICKED, id)),
            Event::UiChanged { id, value } => (
                None,
                ScriptEvent::new(abi::EVENT_UI_CHANGED, id).detail(value.as_text()),
            ),
            Event::UiEvent { id, event } => (
                None,
                ScriptEvent::new(abi::EVENT_UI, id).detail(event.clone()),
            ),
            Event::EnteredRoom { actor, room } => (
                Some(actor.clone()),
                ScriptEvent::new(abi::EVENT_ENTERED_ROOM, room),
            ),
            Event::Weather { weather } => (None, ScriptEvent::new(abi::EVENT_WEATHER, weather)),
            Event::CutsceneSignal { signal } => {
                (None, ScriptEvent::new(abi::EVENT_CUTSCENE_SIGNAL, signal))
            }
            Event::CutsceneEnded { cutscene } => {
                (None, ScriptEvent::new(abi::EVENT_CUTSCENE_ENDED, cutscene))
            }
        })
    }
}

thread_local! {
    /// The words of the event a script is being called with, for `TEXT_EVENT`.
    static EVENT_WORDS: std::cell::RefCell<Option<(String, String)>> =
        const { std::cell::RefCell::new(None) };
}

fn with_event_words(event: &ScriptEvent, f: impl FnOnce()) {
    EVENT_WORDS.with(|words| words.replace(Some((event.subject.clone(), event.detail.clone()))));
    f();
    EVENT_WORDS.with(|words| words.replace(None));
}

/// What one run of a script asked the world for. Effects are applied by the
/// same systems that apply a block's; a broadcast isn't an effect at all, so
/// it is carried out separately and fired at the VM by the caller.
#[derive(Default)]
pub struct Asked {
    pub effects: Vec<Effect>,
    pub messages: Vec<String>,
    /// `(who asked, what to copy)`. Making a clone means registering a
    /// scheduler slot for its strands, which is the VM's to do, so these are
    /// handed to it rather than turned into effects here.
    pub clones: Vec<(String, String)>,
    /// `(who asked, name, position)` for a brand-new actor, for the same
    /// reason: the VM mints its id.
    pub created: Vec<(String, String, [f32; 3])>,
    /// `(who asked, what to delete)`, so the VM stops its scripts too.
    pub deleted: Vec<(String, String)>,
}

fn missing_export(relative: &str) -> String {
    format!(
        "{relative} doesn't name its entry points - end the file with \
         `blockloom::export!(start = start, tick = tick);`"
    )
}

/// What a callback is handed: who is running, and somewhere to put what it
/// asks for.
struct Ctx<'a> {
    actor: &'a str,
    asked: &'a mut Asked,
}

/// # Safety
/// Only ever called from a script, with the pointer `LoadedScript::call`
/// handed it for the duration of that one call.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn ctx<'a>(pointer: *mut c_void) -> &'a mut Ctx<'a> {
    unsafe { &mut *pointer.cast::<Ctx>() }
}

/// The three entry points every script is given. A `static` rather than a
/// value built per call so its address is stable for as long as the process.
#[cfg(not(target_arch = "wasm32"))]
static HOST_API: HostApi = HostApi {
    abi: abi::ABI_VERSION,
    read_number,
    read_text,
    act,
};

fn axis_of(value: f64) -> Axis {
    match value as i32 {
        1 => Axis::Y,
        2 => Axis::Z,
        _ => Axis::X,
    }
}

/// A scene transition by the name a script spells it. Unknown spellings read
/// as `none`, the same rule the blocks keep.
fn normalize_scene_transition(name: &str) -> String {
    let key: String = name
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    match key.as_str() {
        "fade" => "fade".to_string(),
        "wipe" => "wipe".to_string(),
        "circle" => "circle".to_string(),
        _ => "none".to_string(),
    }
}

fn view_of(value: f64) -> CameraView {
    match value as i32 {
        1 => CameraView::FirstPerson,
        2 => CameraView::ThirdPerson,
        _ => CameraView::Follow,
    }
}

/// What a fresh element a script asked for starts at. A slider reads its
/// own number off the call; everything else takes the blank its kind means.
fn ui_start(kind: UiKind, flag: bool, value: f64) -> Evaluated {
    match kind {
        UiKind::Slider | UiKind::Progress | UiKind::RadialProgress | UiKind::Scrollbar => {
            Evaluated::Number(value)
        }
        other => UiElement::blank(other, flag),
    }
}

/// This actor as the frame's snapshot sees it.
fn me(actor: &str) -> Option<sense::ActorSense> {
    sense::read(|sensors| sensors.actors.get(actor).cloned())
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn read_number(
    pointer: *mut c_void,
    what: u32,
    a: Str,
    b: Str,
    arg: f64,
    out: *mut f64,
) -> u32 {
    let ctx = unsafe { ctx(pointer) };
    let a = unsafe { a.as_str() };
    let b = unsafe { b.as_str() };
    let Some(value) = number_for(ctx.actor, what, a, b, arg) else {
        return abi::MISSING;
    };
    unsafe { *out = value };
    abi::OK
}

fn number_for(actor: &str, what: u32, a: &str, b: &str, arg: f64) -> Option<f64> {
    let bool_as = |value: bool| Some(if value { 1.0 } else { 0.0 });
    match what {
        abi::READ_POSITION => Some(me(actor)?.position[axis_of(arg).index()] as f64),
        abi::READ_ROTATION => Some(me(actor)?.rotation[axis_of(arg).index()] as f64),
        abi::READ_SCALE => Some(me(actor)?.scale as f64),
        abi::READ_VISIBLE => bool_as(me(actor)?.visible),
        abi::READ_TIMER => Some(sense::read(|sensors| sensors.time)),
        abi::READ_KEY_DOWN => {
            let key = sense::normalize_key(a);
            bool_as(sense::read(|sensors| sensors.keys.contains(&key)))
        }
        abi::READ_MOUSE => {
            let index = if arg as i32 == 1 { 1 } else { 0 };
            Some(sense::read(|sensors| sensors.mouse[index]) as f64)
        }
        abi::READ_MOUSE_DOWN => bool_as(sense::read(|sensors| sensors.mouse_down)),
        abi::READ_MOUSE_DELTA => {
            let index = if arg as i32 == 1 { 1 } else { 0 };
            Some(sense::read(|sensors| sensors.mouse_delta[index]) as f64)
        }
        abi::READ_MOUSE_LOCKED => bool_as(sense::read(|sensors| sensors.mouse_locked)),
        abi::READ_MOUSE_BUTTON => {
            let button = a.trim().to_lowercase();
            bool_as(sense::read(|sensors| {
                if button == "left" {
                    sensors.mouse_down
                } else {
                    sensors.mouse_buttons.contains(&button)
                }
            }))
        }
        abi::READ_ACTION_DOWN => bool_as(sense::read(|sensors| {
            sensors
                .actions
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(a.trim()))
                .is_some_and(|(_, action)| action.held)
        })),
        abi::READ_ACTION_PRESSED => bool_as(sense::read(|sensors| {
            sensors
                .actions
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(a.trim()))
                .is_some_and(|(_, action)| action.pressed)
        })),
        abi::READ_ACTION_RELEASED => bool_as(sense::read(|sensors| {
            sensors
                .actions
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(a.trim()))
                .is_some_and(|(_, action)| action.released)
        })),
        abi::READ_ACTION_VALUE => Some(sense::read(|sensors| {
            sensors
                .actions
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(a.trim()))
                .map(|(_, action)| action.value as f64)
                .unwrap_or(0.0)
        })),
        abi::READ_TOUCH_COUNT => Some(sense::read(|sensors| sensors.touches.len() as f64)),
        abi::READ_TOUCH => {
            // Even args are x, odd are y, of the 1-based index they halve to.
            let slot = arg as usize;
            let (index, axis) = (slot / 2, slot % 2);
            Some(sense::read(|sensors| {
                index
                    .checked_sub(1)
                    .and_then(|i| sensors.touches.get(i))
                    .map(|touch| touch.position[axis] as f64)
                    .unwrap_or(0.0)
            }))
        }
        abi::READ_GAMEPAD_CONNECTED => bool_as(sense::read(|sensors| sensors.gamepad_connected)),
        abi::READ_GAMEPAD_AXIS => {
            let axis = blockloom_core::input::normalize_pad_axis(a);
            Some(sense::read(|sensors| {
                sensors.gamepad_axes.get(&axis).copied().unwrap_or(0.0) as f64
            }))
        }
        abi::READ_GAMEPAD_BUTTON => {
            let button = blockloom_core::input::normalize_pad_button(a);
            bool_as(sense::read(|sensors| {
                sensors.gamepad_buttons.contains(&button)
            }))
        }
        abi::READ_GAME_PAUSED => bool_as(sense::read(|sensors| sensors.paused)),
        abi::READ_UI_SHOWN => bool_as(sense::read(|sensors| {
            sensors
                .ui
                .get(a.trim())
                .is_some_and(|element| element.shown)
        })),
        abi::READ_UI_EXISTS => bool_as(sense::read(|sensors| sensors.ui.contains_key(a.trim()))),
        abi::READ_UI_VALUE => match sense::read(|sensors| {
            sensors
                .ui
                .get(a.trim())
                .map(|element| element.value.clone())
        })? {
            Evaluated::Number(n) => Some(n),
            Evaluated::Bool(value) => bool_as(value),
            // A text input still answers if what was typed reads as a
            // number, the same way a text field does.
            Evaluated::Text(text) => text.trim().parse().ok(),
        },
        abi::READ_TOUCHING => {
            let me = me(actor)?;
            if a.trim().is_empty() {
                return bool_as(!me.touching.is_empty());
            }
            bool_as(sense::read(|sensors| {
                me.touching.iter().any(|id| {
                    id == a
                        || sensors
                            .actors
                            .get(id)
                            .is_some_and(|other| other.name.eq_ignore_ascii_case(a))
                })
            }))
        }
        abi::READ_DISTANCE_TO => {
            let me = me(actor)?;
            sense::read(|sensors| {
                let to = if a.eq_ignore_ascii_case("mouse") {
                    [sensors.mouse[0], sensors.mouse[1], me.position[2]]
                } else {
                    sensors.find(a)?.position
                };
                let d = |i: usize| (me.position[i] - to[i]) as f64;
                Some((d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt())
            })
        }
        abi::READ_HAS_COMPONENT => bool_as(me(actor)?.attached.contains(a.trim())),
        abi::READ_FIELD => match me(actor)?.components.get(a.trim())?.get(b.trim())? {
            Evaluated::Number(n) => Some(*n),
            Evaluated::Bool(value) => bool_as(*value),
            // A text field still answers if it reads as a number, the same
            // way a text variable does in an arithmetic block.
            Evaluated::Text(text) => text.trim().parse().ok(),
        },
        abi::READ_IS_CLONE => bool_as(me(actor)?.is_clone),
        abi::READ_SOUND_PLAYING => {
            bool_as(sense::read(|sensors| sensors.sounds.contains(a.trim())))
        }
        abi::READ_BUS_VOLUME => {
            let bus = SoundBus::parse(a)?;
            Some(
                sense::read(|sensors| sensors.bus_volumes.get(&bus).copied().unwrap_or(100.0))
                    as f64,
            )
        }
        abi::READ_FRAME_TIME => Some(sense::read(|s| s.performance.frame_ms)),
        abi::READ_DRAW_CALLS => Some(sense::read(|s| s.performance.draw_calls) as f64),
        abi::READ_DLSS_AVAILABLE => bool_as(sense::read(|s| s.performance.dlss_available)),
        abi::READ_CUTSCENE_TIME => Some(sense::read(|s| s.cutscene_time) as f64),
        abi::READ_ATMOSPHERE => sense::read(|sensors| sensors.atmosphere.field(a)),
        abi::READ_WATER => {
            let mut at = a.split_whitespace().map(|n| n.parse::<f32>().ok());
            let (x, z) = (at.next()??, at.next().flatten().unwrap_or(0.0));
            let sample = sense::read(|sensors| sensors.water.surface_at(x, z).map(|(_, s)| s))?;
            water_reading(&sample, b).map(f64::from)
        }
        abi::READ_UNDERWATER => {
            let position = if a.trim().is_empty() {
                me(actor)?.position
            } else {
                sense::read(|sensors| sensors.find(a.trim()).map(|found| found.position))?
            };
            bool_as(sense::read(|sensors| sensors.water.underwater(position)))
        }
        abi::READ_TILE_AT => {
            let mut at = a.split_whitespace().map(|n| n.parse::<f32>().ok());
            let (x, y) = (at.next()??, at.next()??);
            let z = at.next().flatten().unwrap_or(0.0);
            sense::read(|sensors| sensors.level.tile_at([x, y, z], b).ok()).map(f64::from)
        }
        abi::READ_PARTICLES => {
            let me = me(actor)?;
            let what = a.trim().to_ascii_lowercase();
            if what == "alive" {
                return Some(me.particles.alive as f64);
            }
            let (event, axis) = what.split_once(' ').unwrap_or((&what, ""));
            let event = blockloom_core::vfx::ParticleEvent::parse(event)?;
            if axis.is_empty() {
                return Some(me.particles.count(event) as f64);
            }
            let at = me.particles.at(event).unwrap_or(me.position);
            let index = match axis.trim() {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                _ => return None,
            };
            Some(at[index] as f64)
        }
        abi::READ_IS_TWEENING => bool_as(me(actor)?.tweening),
        abi::READ_ANIM_FRAME => Some(me(actor)?.anim_frame as f64),
        abi::READ_ANIM_PLAYING => bool_as(me(actor)?.anim_playing),
        abi::READ_ACTOR_COUNT => Some(sense::read(|sensors| sensors.count_named(a)) as f64),
        abi::READ_POSITION_OF => {
            let axis = axis_of(arg).index();
            sense::read(|sensors| {
                sensors
                    .find(a.trim())
                    .map(|other| other.position[axis] as f64)
            })
        }
        abi::READ_LOCAL_POSITION => Some(me(actor)?.local_position[axis_of(arg).index()] as f64),
        abi::READ_LOCAL_POSITION_OF => {
            let axis = axis_of(arg).index();
            sense::read(|sensors| {
                sensors
                    .find(a.trim())
                    .map(|other| other.local_position[axis] as f64)
            })
        }
        abi::READ_CASTS_SHADOWS => {
            let casts = if a.trim().is_empty() {
                me(actor)?.casts_shadows
            } else {
                sense::read(|sensors| sensors.find(a.trim()).map(|found| found.casts_shadows))?
            };
            Some(if casts { 1.0 } else { 0.0 })
        }
        abi::READ_IS_TRIGGER => {
            let target = trigger_target(actor, a)?;
            Some(if target { 1.0 } else { 0.0 })
        }
        abi::READ_COLLISION_LAYER => {
            let layer = if a.trim().is_empty() {
                me(actor)?.layer
            } else {
                sense::read(|sensors| sensors.find(a.trim()).map(|found| found.layer))?
            };
            Some(layer as f64)
        }
        abi::READ_RAY_DISTANCE => {
            let from = parse_triple(a)?;
            let to = parse_triple(b)?;
            sense::read(|sensors| {
                let mask = blockloom_core::physics_query::query_mask(sensors, Some(actor));
                blockloom_core::physics_query::ray_hit(sensors, from, to, Some(actor), mask)
                    .map(|(_, distance)| distance as f64)
            })
        }
        _ => None,
    }
}

/// Whether `target` is a trigger: empty names the running actor itself.
fn trigger_target(running: &str, target: &str) -> Option<bool> {
    if target.trim().is_empty() {
        return Some(me(running)?.trigger);
    }
    sense::read(|sensors| sensors.find(target.trim()).map(|found| found.trigger))
}

/// Three space-separated numbers, as the prelude sends a point across.
fn parse_triple(text: &str) -> Option<[f32; 3]> {
    let mut numbers = text.split_whitespace().map(|part| part.parse::<f32>());
    let x = numbers.next()?.ok()?;
    let y = numbers.next()?.ok()?;
    let z = numbers.next()?.ok()?;
    Some([x, y, z])
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn read_text(
    pointer: *mut c_void,
    what: u32,
    a: Str,
    b: Str,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> u32 {
    let ctx = unsafe { ctx(pointer) };
    let a = unsafe { a.as_str() };
    let b = unsafe { b.as_str() };
    let Some(answer) = text_for(ctx.actor, what, a, b) else {
        return abi::MISSING;
    };
    // Always report the length, so a caller told the buffer was too small
    // knows exactly how big to make the next one.
    unsafe { *length = answer.len() };
    if answer.len() > capacity {
        return abi::TOO_LONG;
    }
    unsafe { std::ptr::copy_nonoverlapping(answer.as_ptr(), out, answer.len()) };
    abi::OK
}

fn text_for(actor: &str, what: u32, a: &str, b: &str) -> Option<String> {
    match what {
        abi::TEXT_ACTOR_NAME => me(actor).map(|me| me.name),
        abi::TEXT_FIELD => me(actor)
            .and_then(|me| me.components.get(a.trim())?.get(b.trim()).cloned())
            .map(|value| value.as_text()),
        abi::TEXT_ACTOR_ID => Some(actor.to_string()),
        // An actor with no parent and one that made nothing both answer
        // `MISSING`, which the prelude turns into `None`.
        abi::TEXT_PARENT => me(actor).map(|me| me.parent).filter(|id| !id.is_empty()),
        abi::TEXT_NEW_ACTOR => me(actor)
            .map(|me| me.last_created)
            .filter(|id| !id.is_empty()),
        abi::TEXT_ENTERED_ROOM => sense::read(|sensors| sensors.level.entered.get(actor).cloned()),
        abi::TEXT_ROOM => {
            let position = if a.trim().is_empty() {
                me(actor)?.position
            } else {
                sense::read(|sensors| sensors.find(a.trim()).map(|found| found.position))?
            };
            sense::read(|sensors| {
                sensors
                    .level
                    .room_at(position)
                    .map(|room| room.name.clone())
            })
        }
        abi::TEXT_UI_VALUE => sense::read(|sensors| {
            sensors
                .ui
                .get(a.trim())
                .map(|element| element.value.as_text())
        }),
        abi::TEXT_UI_TEXT => {
            sense::read(|sensors| sensors.ui.get(a.trim()).map(|element| element.text.clone()))
        }
        // An empty answer is [`MISSING`], which a script reads as "nobody
        // holds it" - the same shape `the parent` uses for none.
        abi::TEXT_UI_FOCUS => {
            sense::read(|sensors| Some(sensors.ui_focus.clone()).filter(|id| !id.is_empty()))
        }
        abi::TEXT_RAY_HIT => (|| {
            let from = parse_triple(a)?;
            let to = parse_triple(b)?;
            sense::read(|sensors| {
                let mask = blockloom_core::physics_query::query_mask(sensors, Some(actor));
                blockloom_core::physics_query::ray_hit(sensors, from, to, Some(actor), mask)
                    .and_then(|(id, _)| sensors.actors.get(&id))
                    .map(|actor| actor.name.clone())
                    .filter(|name| !name.is_empty())
            })
        })(),
        abi::TEXT_CIRCLE_HIT => (|| {
            let at = parse_triple(a)?;
            let radius = b.trim().parse::<f32>().ok()?;
            sense::read(|sensors| {
                let mask = blockloom_core::physics_query::query_mask(sensors, Some(actor));
                blockloom_core::physics_query::overlap_circle(
                    sensors,
                    at,
                    radius,
                    Some(actor),
                    mask,
                )
                .into_iter()
                .next()
                .and_then(|id| sensors.actors.get(&id))
                .map(|actor| actor.name.clone())
                .filter(|name| !name.is_empty())
            })
        })(),
        abi::TEXT_CURRENT_CLIP => me(actor)
            .map(|me| me.anim_clip)
            .filter(|clip| !clip.is_empty()),
        abi::TEXT_CURRENT_SCENE => sense::read(|sensors| {
            Some(sensors.current_scene.clone()).filter(|name| !name.is_empty())
        }),
        abi::TEXT_CURRENT_WEATHER => sense::read(|sensors| {
            Some(sensors.atmosphere.weather.clone()).filter(|name| !name.is_empty())
        }),
        abi::TEXT_CUTSCENE_NAME => sense::read(|sensors| {
            Some(sensors.cutscene_name.clone()).filter(|name| !name.is_empty())
        }),
        abi::TEXT_SCENE_NAMES => {
            serde_json::to_string(&sense::read(|s| s.scene_names.clone())).ok()
        }
        abi::TEXT_ACTIVE_VOLUMES => {
            serde_json::to_string(&sense::read(|s| s.atmosphere.volumes.clone())).ok()
        }
        abi::TEXT_CURRENT_QUALITY => Some(format!("{:?}", sense::read(|s| s.performance.quality))),
        abi::TEXT_EVENT => EVENT_WORDS.with(|words| {
            let words = words.borrow();
            let (subject, detail) = words.as_ref()?;
            Some(
                if a.trim() == "detail" {
                    detail
                } else {
                    subject
                }
                .clone(),
            )
        }),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn act(
    pointer: *mut c_void,
    what: u32,
    a: Str,
    b: Str,
    c: Str,
    numbers: *const f64,
    count: usize,
) {
    let ctx = unsafe { ctx(pointer) };
    let a = unsafe { a.as_str() };
    let b = unsafe { b.as_str() };
    let c = unsafe { c.as_str() };
    // A run of numbers rather than a fixed three, because one interface
    // element names ten at once. A short run reads as zeros from there on.
    let numbers: &[f64] = if numbers.is_null() || count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(numbers, count) }
    };
    act_for(ctx, what, a, b, c, numbers);
}

fn act_for(ctx: &mut Ctx, what: u32, a: &str, b: &str, c: &str, numbers: &[f64]) {
    let actor = ctx.actor.to_string();
    let at = |index: usize| numbers.get(index).copied().unwrap_or(0.0);
    let (n0, n1, n2) = (at(0), at(1), at(2));
    let vector = [n0 as f32, n1 as f32, n2 as f32];
    let effect = match what {
        abi::ACT_MOVE => Effect::Move {
            actor,
            steps: n0 as f32,
        },
        abi::ACT_GO_TO => Effect::GoTo {
            actor,
            position: vector,
        },
        abi::ACT_NAVIGATE_TO => Effect::NavigateTo {
            actor,
            target: vector,
            speed: at(3) as f32,
        },
        abi::ACT_CHANGE_POSITION => Effect::ChangePosition {
            actor,
            axis: axis_of(n0),
            by: n1 as f32,
        },
        abi::ACT_TURN => Effect::Turn {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
        },
        abi::ACT_SET_ROTATION => Effect::SetRotation {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
        },
        abi::ACT_POINT_TOWARDS => Effect::PointTowards {
            actor,
            target: a.to_string(),
        },
        abi::ACT_SET_SCALE => Effect::SetScale {
            actor,
            factor: n0 as f32,
        },
        abi::ACT_APPLY_IMPULSE => Effect::ApplyImpulse {
            actor,
            impulse: vector,
        },
        abi::ACT_SET_VELOCITY => Effect::SetVelocity {
            actor,
            velocity: vector,
        },
        abi::ACT_SET_TRIGGER => Effect::SetTrigger {
            actor,
            trigger: n0 != 0.0,
        },
        abi::ACT_SET_COLLISION_LAYER => Effect::SetCollisionLayer {
            actor,
            layer: (n0.round() as i32).clamp(1, 8) as u8,
        },
        abi::ACT_SET_COLLISION_MASK => Effect::SetCollisionMask {
            actor,
            mask: (n0.round() as i32).clamp(0, 255) as u8,
        },
        abi::ACT_SAY => Effect::Say {
            actor,
            text: a.to_string(),
        },
        abi::ACT_SET_VISIBLE => Effect::SetVisible {
            actor,
            visible: n0 != 0.0,
        },
        abi::ACT_SET_COLOR => Effect::SetColor {
            actor,
            color: a.to_string(),
        },
        abi::ACT_SET_FIELD => Effect::SetComponentField {
            actor,
            component: a.trim().to_string(),
            field: b.trim().to_string(),
            value: Evaluated::Number(n0),
        },
        abi::ACT_SET_FIELD_TEXT => Effect::SetComponentField {
            actor,
            component: a.trim().to_string(),
            field: b.trim().to_string(),
            value: Evaluated::Text(c.to_string()),
        },
        abi::ACT_ATTACH => Effect::AttachComponent {
            actor,
            component: a.trim().to_string(),
        },
        abi::ACT_DETACH => Effect::DetachComponent {
            actor,
            component: a.trim().to_string(),
        },
        abi::ACT_SET_CAMERA_VIEW => Effect::SetCameraView {
            actor,
            view: view_of(n0),
        },
        abi::ACT_SET_CAMERA_PITCH => Effect::SetCameraPitch {
            actor,
            degrees: n0 as f32,
        },
        abi::ACT_SET_CAMERA_FOV => Effect::SetCameraFov {
            actor,
            fov: n0 as f32,
        },
        abi::ACT_UI_SHOW => Effect::ShowElement {
            element: UiElement {
                id: a.trim().to_string(),
                kind: UiKind::from_index(at(0) as usize),
                content: b.to_string(),
                anchor: UiAnchor::from_index(at(1) as usize),
                offset: [at(2) as f32, at(3) as f32],
                size: [at(4) as f32, at(5) as f32],
                parent: c.trim().to_string(),
                modal: UiKind::from_index(at(0) as usize) == UiKind::Panel && at(6) != 0.0,
                range: [at(7) as f32, at(8) as f32],
                value: ui_start(UiKind::from_index(at(0) as usize), at(6) != 0.0, at(9)),
            },
        },
        abi::ACT_UI_SET | abi::ACT_UI_SET_TEXT => {
            let Some(prop) = UiProp::from_name(b) else {
                return;
            };
            Effect::SetUiProp {
                id: a.trim().to_string(),
                prop,
                value: if what == abi::ACT_UI_SET_TEXT {
                    Evaluated::Text(c.to_string())
                } else {
                    Evaluated::Number(n0)
                },
            }
        }
        abi::ACT_UI_HIDE => Effect::HideElement {
            id: a.trim().to_string(),
            all: n0 != 0.0,
        },
        abi::ACT_UI_DELETE => Effect::DeleteElement {
            id: a.trim().to_string(),
        },
        abi::ACT_UI_FOCUS => Effect::SetFocus {
            id: a.trim().to_string(),
        },
        abi::ACT_UI_THEME => Effect::SetUiTheme {
            theme: UiTheme::from_index(n0 as usize),
        },
        abi::ACT_SAVE_VARIABLE => Effect::SaveVariable {
            actor,
            name: a.trim().to_string(),
            clear: n0 != 0.0,
        },
        abi::ACT_SET_PAUSED => Effect::SetPaused { paused: n0 != 0.0 },
        abi::ACT_STOP_ALL => Effect::Stopped,
        abi::ACT_SET_MOUSE_LOCKED => Effect::SetMouseLocked { locked: n0 != 0.0 },
        abi::ACT_RUMBLE_GAMEPAD => Effect::RumbleGamepad {
            strength: (n0 as f32).clamp(0.0, 100.0),
            duration: (n1 as f32).max(0.0),
        },
        abi::ACT_BIND_ACTION => Effect::BindAction {
            actor,
            action: a.trim().to_string(),
            binding: b.trim().to_string(),
        },
        abi::ACT_CLEAR_ACTION_BINDINGS => Effect::ClearActionBindings {
            actor,
            action: a.trim().to_string(),
        },
        abi::ACT_SET_PARENT => Effect::SetParent {
            actor,
            parent: a.trim().to_string(),
        },
        abi::ACT_PLAY_SOUND => {
            let sound = a.trim().to_string();
            if sound.is_empty() {
                ctx.asked.effects.push(Effect::Error {
                    actor: actor.clone(),
                    message: "which sound should I play?".to_string(),
                });
                return;
            }
            let at = if c.trim().is_empty() {
                Some(actor.clone())
            } else {
                // Id first, then name, the same order the blocks resolve in.
                let wanted = c.trim();
                let id = sense::read(|sensors| {
                    if sensors.actors.contains_key(wanted) {
                        Some(wanted.to_string())
                    } else {
                        sensors
                            .actors
                            .iter()
                            .find(|(_, other)| other.name.eq_ignore_ascii_case(wanted))
                            .map(|(id, _)| id.clone())
                    }
                });
                match id {
                    Some(id) => Some(id),
                    None => {
                        ctx.asked.effects.push(Effect::Error {
                            actor: actor.clone(),
                            message: format!("there's no actor named \"{c}\" to play at"),
                        });
                        return;
                    }
                }
            };
            Effect::PlaySound {
                actor,
                sound,
                volume: user_to_gain(n0),
                pitch: clamp_pitch(n1 as f32),
                loop_: n2 != 0.0,
                bus: SoundBus::parse(b).unwrap_or(SoundBus::Sfx),
                at,
            }
        }
        abi::ACT_STOP_SOUND => Effect::StopSound {
            actor,
            sound: a.trim().to_string(),
        },
        abi::ACT_SET_SOUND_VOLUME => {
            let sound = a.trim().to_string();
            if sound.is_empty() {
                ctx.asked.effects.push(Effect::Error {
                    actor: actor.clone(),
                    message: "which sound's volume should I set?".to_string(),
                });
                return;
            }
            Effect::SetSoundVolume {
                actor,
                sound,
                volume: user_to_gain(n0),
            }
        }
        abi::ACT_SET_SOUND_PITCH => {
            let sound = a.trim().to_string();
            if sound.is_empty() {
                ctx.asked.effects.push(Effect::Error {
                    actor: actor.clone(),
                    message: "which sound's pitch should I set?".to_string(),
                });
                return;
            }
            Effect::SetSoundPitch {
                actor,
                sound,
                pitch: clamp_pitch(n0 as f32),
            }
        }
        abi::ACT_SET_RENDER_SETTING => Effect::SetRenderSetting {
            setting: match a {
                "Quality" => blockloom_core::quality::Setting::Quality,
                "ResolutionScale" => blockloom_core::quality::Setting::ResolutionScale,
                "Upscaler" => blockloom_core::quality::Setting::Upscaler,
                "DlssMode" => blockloom_core::quality::Setting::DlssMode,
                _ => return,
            },
            value: b.to_string(),
        },
        abi::ACT_SET_EXPOSURE => Effect::SetExposure { ev: n0 as f32 },
        abi::ACT_SET_LIGHT_INTENSITY => Effect::SetLightIntensity {
            actor,
            intensity: n0 as f32,
        },
        abi::ACT_TWEEN_SCALE => Effect::TweenScale {
            actor,
            factor: n0 as f32,
            seconds: n1 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(b)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        abi::ACT_TWEEN_ROTATION => Effect::TweenRotation {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
            seconds: n2 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(b)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        abi::ACT_TWEEN_COLOR => Effect::TweenColor {
            actor,
            color: a.to_string(),
            seconds: n0 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(b)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        abi::ACT_STOP_TWEENS => Effect::StopTweens { actor },
        abi::ACT_PLAY_ANIMATION => Effect::PlayAnimation {
            actor,
            clip: a.trim().to_string(),
            speed: n0 as f32,
        },
        abi::ACT_STOP_ANIMATION => Effect::StopAnimation { actor },
        abi::ACT_SET_ANIMATION_SPEED => Effect::SetAnimationSpeed {
            actor,
            speed: n0 as f32,
        },
        abi::ACT_FIRE_ANIMATION_TRIGGER => Effect::FireAnimationTrigger {
            actor,
            name: a.trim().to_string(),
        },
        abi::ACT_SET_RIG_SLOT => Effect::SetRigSlot {
            actor,
            slot: a.trim().to_string(),
            attachment: b.trim().to_string(),
        },
        abi::ACT_SET_SLOT_TINT => Effect::SetSlotTint {
            actor,
            slot: a.trim().to_string(),
            color: b.to_string(),
        },
        abi::ACT_SET_IK_TARGET => Effect::SetIkTarget {
            actor,
            constraint: a.trim().to_string(),
            x: n0 as f32,
            y: n1 as f32,
        },
        abi::ACT_BURST_PARTICLES => Effect::BurstParticles {
            actor,
            count: (n0 as i64).clamp(0, 512) as u32,
        },
        abi::ACT_SET_EMITTER_DIAL => match blockloom_core::blocks::EmitterDial::parse(a) {
            Some(dial) => Effect::SetEmitterDial {
                actor,
                dial,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no emitter dial called \"{a}\""),
            },
        },
        abi::ACT_SET_EMITTER_PLAYING => Effect::SetEmitterPlaying {
            actor,
            playing: n0 != 0.0,
        },
        abi::ACT_SET_SPRITE_DIAL => match blockloom_core::blocks::SpriteDial::parse(a) {
            Some(dial) => Effect::SetSpriteDial {
                actor,
                dial,
                value: n0 as f32,
            },
            None => return,
        },
        abi::ACT_SET_EMISSIVE_STRENGTH => Effect::SetEmissiveStrength {
            actor,
            strength: n0 as f32,
        },
        abi::ACT_SET_HDR_OUTPUT => Effect::SetHdrOutput { enabled: n0 != 0.0 },
        abi::ACT_SET_PEAK_BRIGHTNESS => Effect::SetPeakBrightness { nits: n0 as f32 },
        abi::ACT_ENABLE_VOLUME => Effect::SetVolumeEnabled {
            actor,
            volume: a.trim().to_string(),
            enabled: n0 != 0.0,
        },
        abi::ACT_SET_VOLUME_WEIGHT => Effect::SetVolumeWeight {
            actor,
            volume: a.trim().to_string(),
            weight: n0 as f32,
        },
        abi::ACT_CAPTURE_PROBES => Effect::CaptureProbes,
        abi::ACT_SET_SHADOW_DISTANCE => Effect::SetShadowDistance {
            distance: n0 as f32,
        },
        abi::ACT_SET_LIGHT_SHADOWS => Effect::SetLightShadows {
            actor,
            enabled: n0 != 0.0,
        },
        abi::ACT_SET_RAY_TRACING => Effect::SetRayTracing { enabled: n0 != 0.0 },
        abi::ACT_SET_GI_BOUNCES => Effect::SetGiBounces { bounces: n0 as f32 },
        abi::ACT_SET_GI_SAMPLES => Effect::SetGiSamples { samples: n0 as f32 },
        abi::ACT_SET_FOG_DENSITY => Effect::SetFogDensity { density: n0 as f32 },
        abi::ACT_SET_AURORA => Effect::SetAurora { kp: n0 as f32 },
        abi::ACT_STRIKE_LIGHTNING => Effect::StrikeLightning { at: vector },
        abi::ACT_SET_LIGHTNING_RATE => Effect::SetLightningRate { rate: n0 as f32 },
        abi::ACT_SET_WIND => match blockloom_core::wind::WindProperty::parse(a) {
            Some(property) => Effect::SetWind {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no wind dial called \"{a}\""),
            },
        },
        abi::ACT_SET_CLOUD_DRIFT => Effect::SetCloudDrift { drift: vector },
        abi::ACT_SET_CLOUD_LAYER => {
            match blockloom_core::cloud_layers::CloudLayerProperty::parse(a) {
                Some(property) => Effect::SetCloudLayer {
                    layer: n0 as f32,
                    property,
                    value: n1 as f32,
                },
                None => Effect::Error {
                    actor,
                    message: format!("there's no cloud layer dial called \"{a}\""),
                },
            }
        }
        abi::ACT_SET_CLOUDS => match blockloom_core::clouds::CloudProperty::parse(a) {
            Some(property) => Effect::SetClouds {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no cloud dial called \"{a}\""),
            },
        },
        abi::ACT_SET_TIME_OF_DAY => Effect::SetTimeOfDay { time: n0 as f32 },
        abi::ACT_ADVANCE_TIME => Effect::AdvanceTime { hours: n0 as f32 },
        abi::ACT_SET_PRECIPITATION => match blockloom_core::director::PrecipitationKind::parse(a) {
            Some(property) => Effect::SetPrecipitation {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no precipitation called \"{a}\""),
            },
        },
        abi::ACT_BLEND_WEATHER => Effect::BlendWeather {
            weather: a.trim().to_string(),
            seconds: n0 as f32,
        },
        abi::ACT_PLAY_CUTSCENE => Effect::PlayCutscene {
            cutscene: a.trim().to_string(),
        },
        abi::ACT_SKIP_CUTSCENE => Effect::SkipCutscene,
        abi::ACT_CAMERA_SHAKE => Effect::CameraShake { amount: n0 as f32 },
        abi::ACT_SET_TIME_SCALE => Effect::SetTimeScale { scale: n0 as f32 },
        abi::ACT_HITSTOP => Effect::Hitstop { frames: n0 as f32 },
        abi::ACT_SET_LETTERBOX => Effect::SetLetterbox { on: n0 as f32 },
        abi::ACT_FADE_SCREEN => Effect::FadeScreen {
            color: blockloom_core::cinematic::normalize_fade(a),
        },
        abi::ACT_PAINT_TILE => Effect::PaintTile {
            actor,
            map: a.trim().to_string(),
            tile: (n0.floor() as i32).max(-1),
            x: n1 as f32,
            y: n2 as f32,
            z: at(3) as f32,
        },
        abi::ACT_SET_PARALLAX => match blockloom_core::tilemap::ParallaxAxis::parse(b) {
            Some(axis) => Effect::SetParallax {
                actor,
                layer: a.trim().to_string(),
                axis,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no parallax axis called \"{b}\""),
            },
        },
        abi::ACT_SWITCH_SCENE => Effect::SwitchScene {
            actor,
            scene: a.trim().to_string(),
            transition: normalize_scene_transition(b),
        },
        abi::ACT_SET_WATER => match blockloom_core::water::WaterProperty::parse(a) {
            Some(property) => Effect::SetWater {
                actor,
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no water dial called \"{a}\""),
            },
        },
        abi::ACT_SET_BUS_VOLUME => Effect::SetBusVolume {
            bus: SoundBus::parse(a).unwrap_or(SoundBus::Sfx),
            volume: user_to_gain(n0),
        },
        // The clone's own id isn't minted here: the VM registers it so the
        // copy's `when I start as a clone` strands have a scheduler slot,
        // and `the actor I made` answers with it next frame.
        abi::ACT_CREATE_CLONE => {
            ctx.asked.clones.push((actor, a.trim().to_string()));
            return;
        }
        abi::ACT_CREATE_ACTOR => {
            ctx.asked
                .created
                .push((actor, a.trim().to_string(), vector));
            return;
        }
        abi::ACT_DELETE_ACTOR => {
            ctx.asked.deleted.push((actor, a.trim().to_string()));
            return;
        }
        // A broadcast isn't a change to the world, so it isn't an effect:
        // the caller fires it at the VM once this run is over.
        abi::ACT_BROADCAST => {
            ctx.asked.messages.push(a.trim().to_string());
            return;
        }
        // A log line is for the editor, not the world, so it goes straight
        // out rather than through the effect list.
        abi::ACT_LOG => {
            crate::bridge::send(&RuntimeMessage::Say {
                actor,
                text: a.to_string(),
            });
            return;
        }
        // A verb this runtime doesn't know is a script built against a newer
        // ABI, which the load-time check should already have caught.
        _ => return,
    };
    ctx.asked.effects.push(effect);
}

/// Scripts in a browser. Each one is its own wasm module with its own memory,
/// so a string it passes is an offset into that memory, not a pointer the
/// player can follow: the imports here copy arguments out of it and answers
/// back in, around the same host calls a native script reaches through
/// [`HostApi`]. One instance per file, shared by every actor running it, the
/// way one library is natively.
#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use js_sys::{Function, Object, Reflect, Uint8Array, WebAssembly};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::{Rc, Weak};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    type Import = Closure<dyn FnMut(u32, u32, u32) -> u32>;

    pub struct Instance {
        relative: String,
        start: Function,
        tick: Function,
        event: Function,
        /// Set once the script traps: a wasm trap leaves its stack and heap
        /// wherever they were, so calling back in isn't safe.
        stopped: Cell<bool>,
        /// The last line it logged, which is where its panic hook put the
        /// panic's message before the trap.
        last_log: Rc<RefCell<String>>,
        /// The imports, alive for as long as the instance can call them.
        _imports: [Import; 3],
    }

    thread_local! {
        static OPEN: RefCell<HashMap<String, Weak<Instance>>> = RefCell::default();
    }

    /// The instance for `relative`, made on first use. Once every actor
    /// running it has gone, the next run gets a fresh one, as a library
    /// closed and reopened natively does.
    pub fn open(relative: &str) -> Result<Rc<Instance>, String> {
        if let Some(open) = OPEN.with_borrow(|open| open.get(relative).and_then(Weak::upgrade)) {
            return Ok(open);
        }
        let instance = Rc::new(instantiate(relative)?);
        OPEN.with_borrow_mut(|open| {
            open.insert(relative.to_string(), Rc::downgrade(&instance));
        });
        Ok(instance)
    }

    fn js_error(error: JsValue) -> String {
        error
            .dyn_ref::<js_sys::Error>()
            .map(|error| String::from(error.message()))
            .or_else(|| error.as_string())
            .unwrap_or_else(|| format!("{error:?}"))
    }

    fn instantiate(relative: &str) -> Result<Instance, String> {
        let module = crate::web::script_module(relative)
            .ok_or_else(|| format!("{relative} wasn't built for the web"))?;
        let memory: Rc<RefCell<Option<WebAssembly::Memory>>> = Rc::default();
        let last_log: Rc<RefCell<String>> = Rc::default();

        let imports = [
            import(&memory, |memory, ctx, what, call| {
                let a = text(memory, call.a_ptr, call.a_len);
                let b = text(memory, call.b_ptr, call.b_len);
                match number_for(ctx.actor, what, &a, &b, call.arg) {
                    Some(value) => {
                        write(memory, call.out, &value.to_le_bytes());
                        abi::OK
                    }
                    None => abi::MISSING,
                }
            }),
            import(&memory, |memory, ctx, what, call| {
                let a = text(memory, call.a_ptr, call.a_len);
                let b = text(memory, call.b_ptr, call.b_len);
                let Some(answer) = text_for(ctx.actor, what, &a, &b) else {
                    return abi::MISSING;
                };
                write(memory, call.out_len, &(answer.len() as u32).to_le_bytes());
                if answer.len() > call.out_cap as usize {
                    return abi::TOO_LONG;
                }
                write(memory, call.out, answer.as_bytes());
                abi::OK
            }),
            {
                let last_log = last_log.clone();
                import(&memory, move |memory, ctx, what, call| {
                    let a = text(memory, call.a_ptr, call.a_len);
                    let b = text(memory, call.b_ptr, call.b_len);
                    let c = text(memory, call.c_ptr, call.c_len);
                    let raw = bytes(memory, call.numbers, call.count.saturating_mul(8));
                    let numbers: Vec<f64> = raw
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|chunk| f64::from_le_bytes(*chunk))
                        .collect();
                    if what == abi::ACT_LOG {
                        last_log.replace(a.clone());
                    }
                    act_for(ctx, what, &a, &b, &c, &numbers);
                    abi::OK
                })
            },
        ];

        let calls = Object::new();
        for (name, import) in [abi::WASM_READ_NUMBER, abi::WASM_READ_TEXT, abi::WASM_ACT]
            .iter()
            .zip(&imports)
        {
            Reflect::set(&calls, &JsValue::from_str(name), import.as_ref()).map_err(js_error)?;
        }
        let wanted = Object::new();
        Reflect::set(&wanted, &JsValue::from_str(abi::WASM_MODULE), &calls).map_err(js_error)?;
        let instance = WebAssembly::Instance::new(&module, &wanted)
            .map_err(|error| format!("{relative} couldn't start: {}", js_error(error)))?;
        let exports = instance.exports();
        let export = |name: &[u8]| -> Result<Function, String> {
            let name = String::from_utf8_lossy(name);
            Reflect::get(&exports, &JsValue::from_str(&name))
                .ok()
                .and_then(|value| value.dyn_into::<Function>().ok())
                .ok_or_else(|| missing_export(relative))
        };
        let version = export(abi::SYM_ABI)
            .map_err(|_| format!("{relative} isn't a Blockloom script"))?
            .call0(&JsValue::NULL)
            .map_err(js_error)?
            .as_f64()
            .unwrap_or(0.0) as u32;
        if version != abi::ABI_VERSION {
            return Err(format!(
                "{relative} was built against script ABI {version}, this player speaks {}. \
                 Build the game again.",
                abi::ABI_VERSION
            ));
        }
        let start = export(abi::SYM_START)?;
        let tick = export(abi::SYM_TICK)?;
        let event = export(abi::SYM_EVENT)?;
        let own_memory = Reflect::get(&exports, &JsValue::from_str("memory"))
            .ok()
            .and_then(|value| value.dyn_into::<WebAssembly::Memory>().ok())
            .ok_or_else(|| format!("{relative} exports no memory"))?;
        memory.replace(Some(own_memory));
        Ok(Instance {
            relative: relative.to_string(),
            start,
            tick,
            event,
            stopped: Cell::new(false),
            last_log,
            _imports: imports,
        })
    }

    /// One import: `(ctx, what, call)`, where `ctx` is the player's own
    /// pointer handed through untouched and `call` a [`abi::WasmCall`] in
    /// the script's memory.
    fn import(
        memory: &Rc<RefCell<Option<WebAssembly::Memory>>>,
        answer: impl Fn(&WebAssembly::Memory, &mut Ctx, u32, abi::WasmCall) -> u32 + 'static,
    ) -> Import {
        let memory = memory.clone();
        Closure::new(move |ctx: u32, what: u32, call: u32| -> u32 {
            let memory = memory.borrow();
            let Some(memory) = memory.as_ref() else {
                return abi::MISSING;
            };
            let record = bytes(memory, call, std::mem::size_of::<abi::WasmCall>() as u32);
            if record.len() < std::mem::size_of::<abi::WasmCall>() {
                return abi::MISSING;
            }
            // Safety: every field is a plain number, so any bytes are one.
            let call = unsafe { std::ptr::read_unaligned(record.as_ptr().cast::<abi::WasmCall>()) };
            // Safety: `ctx` is the pointer `call` below handed the script
            // for the duration of this one call, coming back unchanged.
            let ctx = unsafe { &mut *(ctx as usize as *mut Ctx) };
            answer(memory, ctx, what, call)
        })
    }

    fn bytes(memory: &WebAssembly::Memory, at: u32, len: u32) -> Vec<u8> {
        if len == 0 {
            return Vec::new();
        }
        let view = Uint8Array::new(&memory.buffer());
        let end = at.saturating_add(len).min(view.length());
        view.subarray(at.min(end), end).to_vec()
    }

    fn text(memory: &WebAssembly::Memory, at: u32, len: u32) -> String {
        String::from_utf8(bytes(memory, at, len)).unwrap_or_default()
    }

    fn write(memory: &WebAssembly::Memory, at: u32, data: &[u8]) {
        let view = Uint8Array::new(&memory.buffer());
        let end = at as u64 + data.len() as u64;
        if at == 0 || end > view.length() as u64 {
            return;
        }
        view.subarray(at, end as u32).copy_from(data);
    }

    /// Which entry point a call runs.
    pub enum Entry<'a> {
        Start,
        Tick(f32),
        Event(&'a super::ScriptEvent),
    }

    impl Instance {
        /// Runs `start`, `tick` or `event`, inside the actor's sensing
        /// scope as natively. A trap stops the script for the rest of the
        /// game, with the panic's own message where it left one.
        pub fn call(&self, actor: &str, asked: &mut Asked, entry: Entry) {
            if self.stopped.get() {
                return;
            }
            let mut ctx = Ctx { actor, asked };
            let pointer = JsValue::from((&raw mut ctx) as usize as u32);
            // The script sees a null `HostApi` and uses its imports instead.
            let host = JsValue::from(0u32);
            let result = sense::with_actor(actor, || match entry {
                Entry::Start => self.start.call2(&JsValue::NULL, &pointer, &host),
                Entry::Tick(dt) => {
                    self.tick
                        .call3(&JsValue::NULL, &pointer, &host, &JsValue::from(dt))
                }
                Entry::Event(event) => {
                    let args = js_sys::Array::of3(&pointer, &host, &JsValue::from(event.kind));
                    for n in event.numbers {
                        args.push(&JsValue::from(n));
                    }
                    self.event.apply(&JsValue::NULL, &args)
                }
            });
            if let Err(error) = result {
                self.stopped.set(true);
                let logged = self.last_log.borrow();
                let why = if logged.starts_with("the script panicked") {
                    logged.clone()
                } else {
                    format!("the script trapped: {}", js_error(error))
                };
                ctx.asked.effects.push(Effect::Error {
                    actor: actor.to_string(),
                    message: format!("{}: {why}. It won't run again this game.", self.relative),
                });
            }
        }
    }
}

/// One named reading of a water sample, as `READ_WATER` spells them.
fn water_reading(sample: &blockloom_core::water::WaterSample, what: &str) -> Option<f32> {
    let key: String = what
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    Some(match key.as_str() {
        "height" | "level" => sample.height,
        "normalx" => sample.normal[0],
        "normaly" => sample.normal[1],
        "normalz" => sample.normal[2],
        "velocityx" => sample.velocity[0],
        "velocityy" => sample.velocity[1],
        "velocityz" => sample.velocity[2],
        "foam" => (1.0 - sample.jacobian).clamp(0.0, 1.0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::sense::{ActorSense, Sensors};
    use std::collections::HashMap;

    /// A project folder of its own, removed when the test ends.
    struct TempProject(std::path::PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("blockloom-script-{name}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("assets/scripts")).expect("a temp project");
            Self(path)
        }

        /// Writes a script and builds it, or `None` when this machine has no
        /// toolchain - which is a reason to skip, not to fail.
        fn build(&self, source: &str) -> Option<LoadedScript> {
            if blockloom_core::script::toolchain_version().is_err() {
                return None;
            }
            let relative = "assets/scripts/test.rs";
            std::fs::write(self.0.join(relative), source).expect("the script");
            let built = blockloom_core::script::compile(&self.0, relative)
                .unwrap_or_else(|e| panic!("the test script didn't compile:\n{e}"));
            assert!(built.is_file());
            Some(LoadedScript::load(&self.0, relative).expect("a loadable script"))
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Publishes one actor the script can read, as the runtime would.
    fn publish_one(actor: &str) {
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            actor.to_string(),
            ActorSense {
                name: "Player".to_string(),
                position: [3.0, 7.0, 0.0],
                attached: ["Place", "Body"].iter().map(|s| s.to_string()).collect(),
                components: HashMap::from([(
                    "Stats".to_string(),
                    HashMap::from([("hp".to_string(), Evaluated::Number(5.0))]),
                )]),
                ..Default::default()
            },
        );
        sense::publish(sensors);
    }

    #[test]
    fn a_script_hears_events_through_its_event_entry_point() {
        use blockloom_core::vfx::{ParticleEvent, ParticleSense};
        use blockloom_core::vm::Event;
        let project = TempProject::new("events");
        // Only `event`: the other two entry points are filled in empty.
        let Some(script) = project.build(
            r#"
use blockloom::*;

fn event(me: &Actor, event: &Event) {
    match event {
        Event::Message(message) => me.say(&format!("heard {message}")),
        Event::Particles { kind: ParticleKind::Collide, count, at } => {
            me.say(&format!("{count} hit at {} {}", at.0, at.1))
        }
        Event::Collision { with, id } => me.say(&format!("touched {with} ({id})")),
        Event::EnteredRoom(room) => me.say(&format!("entered {room}")),
        _ => {}
    }
}

blockloom::export!(event = event);
"#,
        ) else {
            return;
        };

        let mut sensors = Sensors::default();
        let particles = ParticleSense {
            collided: 3,
            collide_at: Some([1.5, 2.0, 0.0]),
            ..Default::default()
        };
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Hose".to_string(),
                particles,
                ..Default::default()
            },
        );
        sense::publish(sensors);

        let names = |id: &str| {
            if id == "b2" {
                "Wall".to_string()
            } else {
                String::new()
            }
        };
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        script.tick("a1", &mut asked, 0.1);
        for event in [
            Event::Message("go".to_string()),
            Event::Particles {
                actor: "a1".to_string(),
                event: ParticleEvent::Collide,
            },
            Event::Collision {
                actor: "a1".to_string(),
                with: "b2".to_string(),
            },
            Event::EnteredRoom {
                actor: "a1".to_string(),
                room: "Cave".to_string(),
            },
        ] {
            let (to, heard) = ScriptEvent::of(&event, names).expect("a script event");
            assert!(to.is_none_or(|to| to == "a1"));
            script.event("a1", &mut asked, &heard);
        }
        let said: Vec<_> = asked
            .effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Say { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            said,
            [
                "heard go",
                "3 hit at 1.5 2",
                "touched Wall (b2)",
                "entered Cave"
            ]
        );
    }

    #[test]
    fn only_the_actor_an_event_names_hears_it() {
        use blockloom_core::vm::Event;
        let names = |_: &str| String::new();
        let (to, _) = ScriptEvent::of(&Event::Key("space".to_string()), names).unwrap();
        assert_eq!(to, None);
        let click = Event::Click {
            actor: "a1".to_string(),
        };
        let (to, heard) = ScriptEvent::of(&click, names).unwrap();
        assert_eq!(to.as_deref(), Some("a1"));
        assert_eq!(heard.kind, abi::EVENT_CLICKED);
        // `start` already covers these.
        assert!(ScriptEvent::of(&Event::Started, names).is_none());
    }

    #[test]
    fn a_compiled_script_reads_the_snapshot_and_asks_for_effects() {
        let project = TempProject::new("effects");
        let Some(script) = project.build(
            r#"
use blockloom::*;

fn start(me: &Actor) {
    me.say(&format!("{} at {}", me.name(), me.x()));
    me.set_field("Stats", "hp", me.field_or("Stats", "hp", 0.0) - 1.0);
}

fn tick(me: &Actor, dt: f32) {
    me.change_position(Axis::Y, 10.0 * dt);
    if me.has("Body") {
        me.detach("Body");
    }
}

blockloom::export!(start = start, tick = tick);
"#,
        ) else {
            return;
        };

        publish_one("a1");
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        script.tick("a1", &mut asked, 0.5);

        assert_eq!(
            asked.effects,
            vec![
                // Reads answered from the published snapshot, not from a
                // document the script never sees.
                Effect::Say {
                    actor: "a1".to_string(),
                    text: "Player at 3".to_string(),
                },
                Effect::SetComponentField {
                    actor: "a1".to_string(),
                    component: "Stats".to_string(),
                    field: "hp".to_string(),
                    value: Evaluated::Number(4.0),
                },
                Effect::ChangePosition {
                    actor: "a1".to_string(),
                    axis: Axis::Y,
                    by: 5.0,
                },
                Effect::DetachComponent {
                    actor: "a1".to_string(),
                    component: "Body".to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_broadcast_is_carried_out_separately_and_a_panic_stays_inside() {
        let project = TempProject::new("broadcast");
        let Some(script) = project.build(
            r#"
use blockloom::*;

fn start(me: &Actor) {
    me.broadcast("go");
}

fn tick(me: &Actor, _dt: f32) {
    // Crossing the C boundary with this would abort the whole game window,
    // so `export!` catches it.
    let empty: Vec<i32> = Vec::new();
    me.say(&format!("{}", empty[1]));
}

blockloom::export!(start = start, tick = tick);
"#,
        ) else {
            return;
        };

        publish_one("a1");
        let mut asked = Asked::default();
        script.start("a1", &mut asked);
        // A broadcast isn't a change to the world, so it isn't an effect.
        assert_eq!(asked.messages, vec!["go".to_string()]);
        assert!(asked.effects.is_empty());

        // The panic is swallowed and the process is still here to assert it.
        script.tick("a1", &mut asked, 0.1);
        assert!(asked.effects.is_empty());
    }

    #[test]
    fn a_script_can_read_another_actor() {
        let project = TempProject::new("crossactor");
        let Some(script) = project.build(
            r#"
use blockloom::*;

fn start(me: &Actor) {}

fn tick(me: &Actor, _dt: f32) {
    me.say(&format!(
        "{} is at {},{}",
        "Friend",
        me.position_of("Friend", Axis::X),
        me.position_of("Friend", Axis::Y)
    ));
}

blockloom::export!(start = start, tick = tick);
"#,
        ) else {
            return;
        };

        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Me".to_string(),
                ..Default::default()
            },
        );
        sensors.actors.insert(
            "b1".to_string(),
            ActorSense {
                name: "Friend".to_string(),
                position: [3.0, 7.0, 0.0],
                ..Default::default()
            },
        );
        sense::publish(sensors);

        let mut asked = Asked::default();
        script.tick("a1", &mut asked, 0.1);
        assert_eq!(
            asked.effects,
            vec![Effect::Say {
                actor: "a1".to_string(),
                text: "Friend is at 3,7".to_string(),
            }]
        );
    }

    #[test]
    fn a_library_that_never_called_export_is_refused() {
        let project = TempProject::new("notascript");
        // A real cdylib, but one that never called `export!`.
        let Some(_) = (blockloom_core::script::toolchain_version().is_ok()).then(|| {
            std::fs::write(
                project.0.join("assets/scripts/test.rs"),
                "pub fn unused() {}\n",
            )
            .expect("the script");
            blockloom_core::script::compile(&project.0, "assets/scripts/test.rs")
                .expect("an empty script still compiles");
        }) else {
            return;
        };

        let Err(error) = LoadedScript::load(&project.0, "assets/scripts/test.rs") else {
            panic!("a library with no entry points isn't a script");
        };
        // It fails at the ABI stamp, which `export!` is what writes - so the
        // version check is also the "is this ours at all" check.
        assert!(error.contains("isn't a Blockloom script"), "{error}");
    }
}
