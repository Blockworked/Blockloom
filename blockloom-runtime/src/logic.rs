//! Loading and running a built game's compiled block program.

use blockloom_core::blocks::{
    DictItem, ListItem, dict_remove, dict_set, is_dict_reporter, is_list_reporter, list_index,
    parse_json_array, parse_json_object, resolve_dict_reporter, resolve_list_reporter,
};
use blockloom_core::codegen::{
    ABI_MISSING, ABI_OK, ABI_PANIC, ABI_TOO_LONG, ACT_ADVANCE_TIME, ACT_APPLY_IMPULSE, ACT_ATTACH,
    ACT_BIND_ACTION, ACT_BLEND_WEATHER, ACT_BROADCAST, ACT_BURST_PARTICLES, ACT_CAMERA_SHAKE,
    ACT_CAPTURE_PROBES, ACT_CHANGE_POSITION, ACT_CLEAR_ACTION_BINDINGS, ACT_CREATE_ACTOR,
    ACT_CREATE_CLONE, ACT_DELETE_ACTOR, ACT_DELETE_ELEMENT, ACT_DETACH, ACT_DICT_CLEAR,
    ACT_DICT_DELETE_KEY, ACT_DICT_SET, ACT_ENABLE_VOLUME, ACT_ERROR, ACT_FADE_DECALS,
    ACT_FADE_SCREEN, ACT_FIRE_ANIMATION_TRIGGER, ACT_FRACTURE, ACT_GLIDE, ACT_GO_TO,
    ACT_HIDE_ELEMENT, ACT_HITSTOP, ACT_JSON_TO_DICT, ACT_JSON_TO_LIST, ACT_LIST_ADD,
    ACT_LIST_CLEAR, ACT_LIST_DELETE, ACT_LIST_INSERT, ACT_LIST_REPLACE, ACT_LIST_REVERSE,
    ACT_LIST_SHIFT, ACT_MOVE, ACT_NAVIGATE_TO, ACT_PAINT_TILE, ACT_PLAY_ANIMATION,
    ACT_PLAY_CUTSCENE, ACT_PLAY_SOUND, ACT_POINT_TOWARDS, ACT_PUFF_SMOKE, ACT_RUMBLE_GAMEPAD,
    ACT_SAVE_VARIABLE, ACT_SAY, ACT_SET_ANIMATION_SPEED, ACT_SET_AURORA, ACT_SET_BODY,
    ACT_SET_BUS_VOLUME, ACT_SET_CAMERA_FOV, ACT_SET_CAMERA_PITCH, ACT_SET_CAMERA_VIEW,
    ACT_SET_CLOUD_DRIFT, ACT_SET_CLOUD_LAYER, ACT_SET_CLOUDS, ACT_SET_COLLISION_LAYER,
    ACT_SET_COLLISION_MASK, ACT_SET_COLOR, ACT_SET_DENSITY, ACT_SET_EMISSIVE_STRENGTH,
    ACT_SET_EMITTER_DIAL, ACT_SET_EMITTER_PLAYING, ACT_SET_EXPOSURE, ACT_SET_FIELD, ACT_SET_FOCUS,
    ACT_SET_FOG_DENSITY, ACT_SET_GI_BOUNCES, ACT_SET_GI_SAMPLES, ACT_SET_GRAVITY,
    ACT_SET_HDR_OUTPUT, ACT_SET_IK_TARGET, ACT_SET_LETTERBOX, ACT_SET_LIGHT_INTENSITY,
    ACT_SET_LIGHT_SHADOWS, ACT_SET_LIGHTNING_RATE, ACT_SET_MASS, ACT_SET_MOUSE_LOCKED,
    ACT_SET_PARALLAX, ACT_SET_PARENT, ACT_SET_PAUSED, ACT_SET_PEAK_BRIGHTNESS,
    ACT_SET_PRECIPITATION, ACT_SET_RAY_TRACING, ACT_SET_RENDER_SETTING, ACT_SET_RIG_SLOT,
    ACT_SET_ROTATION, ACT_SET_SCALE, ACT_SET_SHADOW_DISTANCE, ACT_SET_SLOT_TINT,
    ACT_SET_SOUND_PITCH, ACT_SET_SOUND_VOLUME, ACT_SET_SPRITE_DIAL, ACT_SET_TIME_OF_DAY,
    ACT_SET_TIME_SCALE, ACT_SET_TRAIL_ENABLED, ACT_SET_TRIGGER, ACT_SET_UI_PROP, ACT_SET_UI_THEME,
    ACT_SET_VELOCITY, ACT_SET_VISIBLE, ACT_SET_VOLUME_WEIGHT, ACT_SET_WATER, ACT_SET_WIND,
    ACT_SHOW_ELEMENT, ACT_SKIP_CUTSCENE, ACT_SPAWN_DECAL, ACT_SPLASH, ACT_STOP_ANIMATION,
    ACT_STOP_SOUND, ACT_STOP_TWEENS, ACT_STRIKE_LIGHTNING, ACT_SWITCH_SCENE, ACT_TURN,
    ACT_TWEEN_COLOR, ACT_TWEEN_ROTATION, ACT_TWEEN_SCALE, AbiStr, AbiValue, LOGIC_ABI_VERSION,
    LogicHostApi, READ_SENSE, READ_VARIABLE, TICK_STOPPED, VALUE_BOOL, VALUE_ERROR, VALUE_NUMBER,
    VALUE_TEXT,
};
// Symbol names for the native `dlopen` path; web builds link statically later.
#[cfg(not(target_arch = "wasm32"))]
use blockloom_core::codegen::{
    self, SYM_LOGIC_ABI, SYM_LOGIC_FIRE, SYM_LOGIC_FREE, SYM_LOGIC_NEW, SYM_LOGIC_PAUSE,
    SYM_LOGIC_RESET, SYM_LOGIC_SCENE, SYM_LOGIC_TICK,
};
use blockloom_core::components::CameraView;
use blockloom_core::project::Project;
use blockloom_core::scene::{Axis, BodyKind};
use blockloom_core::sense;
use blockloom_core::sound::SoundBus;
use blockloom_core::ui::{UiAnchor, UiElement, UiKind, UiProp, UiTheme};
use blockloom_core::value::{Evaluated, Op, Value, ext_operator};
use blockloom_core::vm::{Dicts, Effect, Event, Lists, Variables};
use std::ffi::c_void;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
type AbiFn = unsafe extern "C" fn() -> u32;
#[cfg(not(target_arch = "wasm32"))]
type NewFn = unsafe extern "C" fn() -> *mut c_void;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type ResetFn = unsafe extern "C" fn(*mut c_void);
type FireFn = unsafe extern "C" fn(*mut c_void, AbiStr, AbiStr, AbiStr, AbiStr);
type TickFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const LogicHostApi, f64, f64) -> u32;
type PauseFn = unsafe extern "C" fn(*mut c_void, u32);
type SceneFn = unsafe extern "C" fn(*mut c_void, AbiStr) -> u32;

/// Where the logic library opens from. Desktop joins the project dir;
/// Android resolves the file name beside this library in the app's lib dir.
#[cfg(target_os = "android")]
fn logic_path_for(project_dir: &Path) -> std::path::PathBuf {
    let path = codegen::library_path(project_dir);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("libblockloom_logic.so");
    crate::android::native_lib_path(name)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn logic_path_for(project_dir: &Path) -> std::path::PathBuf {
    codegen::library_path(project_dir)
}

/// One generated program and its suspended strands.
///
/// Web builds have no `dlopen`, so compiled logic stays unloaded there and
/// blocks run on the VM; static linking follows the same entry points later.
pub struct LoadedLogic {
    #[cfg(not(target_arch = "wasm32"))]
    library: libloading::Library,
    state: *mut c_void,
    free: FreeFn,
    reset: ResetFn,
    fire: FireFn,
    tick: TickFn,
    /// A program built before this export existed simply has none, and the
    /// editor's Pause then reaches it the long way, through `SetPaused`.
    pause: Option<PauseFn>,
    /// A program built before multi-scene logic simply has none, and a
    /// scene switch then falls back to the VM the way it used to.
    scene: Option<SceneFn>,
}

#[cfg(not(target_arch = "wasm32"))]
impl LoadedLogic {
    /// On Android the build fails when the logic library is missing, so by
    /// the time the APK exists it rides beside the runtime: a missing one is
    /// a load error, never a silent fallback to the VM.
    #[cfg(not(target_os = "android"))]
    pub fn is_built(project_dir: &Path) -> bool {
        codegen::library_path(project_dir).is_file()
    }

    #[cfg(target_os = "android")]
    pub fn is_built(_project_dir: &Path) -> bool {
        true
    }

    pub fn load(project_dir: &Path) -> Result<Self, String> {
        let path = logic_path_for(project_dir);
        #[cfg(not(target_os = "android"))]
        if !path.is_file() {
            return Err("this game has no compiled block program".to_string());
        }
        // Safety: this is the library Blockloom generated, and every symbol is
        // checked before any program state is allowed to escape it.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|error| format!("{}: {error}", path.display()))?;
        unsafe {
            let abi = library
                .get::<AbiFn>(SYM_LOGIC_ABI)
                .map_err(|_| "the compiled block program has no Blockloom ABI".to_string())?;
            let version = abi();
            if version != LOGIC_ABI_VERSION {
                return Err(format!(
                    "the compiled block program uses ABI {version}, this player uses {LOGIC_ABI_VERSION}"
                ));
            }
            let new = *library
                .get::<NewFn>(SYM_LOGIC_NEW)
                .map_err(|_| missing_export())?;
            let free = *library
                .get::<FreeFn>(SYM_LOGIC_FREE)
                .map_err(|_| missing_export())?;
            let reset = *library
                .get::<ResetFn>(SYM_LOGIC_RESET)
                .map_err(|_| missing_export())?;
            let fire = *library
                .get::<FireFn>(SYM_LOGIC_FIRE)
                .map_err(|_| missing_export())?;
            let tick = *library
                .get::<TickFn>(SYM_LOGIC_TICK)
                .map_err(|_| missing_export())?;
            let pause = library.get::<PauseFn>(SYM_LOGIC_PAUSE).ok().map(|f| *f);
            let scene = library.get::<SceneFn>(SYM_LOGIC_SCENE).ok().map(|f| *f);
            let state = new();
            if state.is_null() {
                return Err("the compiled block program could not start".to_string());
            }
            Ok(Self {
                library,
                state,
                free,
                reset,
                fire,
                tick,
                pause,
                scene,
            })
        }
    }
}

/// Web builds have no `dlopen`, so nothing is ever built to open: blocks
/// run on the VM instead.
#[cfg(target_arch = "wasm32")]
impl LoadedLogic {
    pub fn is_built(_project_dir: &Path) -> bool {
        false
    }

    pub fn load(_project_dir: &Path) -> Result<Self, String> {
        Err("this web build runs blocks on the VM".to_string())
    }
}

impl LoadedLogic {
    pub fn reset(&mut self) {
        unsafe { (self.reset)(self.state) };
    }

    /// Freezes or thaws the world from outside the program - the editor's
    /// own Pause button. A `pause game` block freezes the program's table
    /// itself, so this is only ever the host's word.
    pub fn set_paused(&mut self, paused: bool) {
        if let Some(pause) = self.pause {
            unsafe { pause(self.state, u32::from(paused)) };
        }
    }

    pub fn fire(&mut self, event: Event, project: &Project) {
        match event {
            Event::Started => self.fire_raw("Started", "", "", ""),
            Event::QualityDropped => self.fire_raw("QualityDropped", "", "", ""),
            Event::SceneStarted => self.fire_raw("SceneStarted", "", "", ""),
            Event::SceneEnded => self.fire_raw("SceneEnded", "", "", ""),
            Event::Key(key) => self.fire_raw("Key", "", &key, ""),
            Event::Click { actor } => self.fire_raw("Clicked", &actor, "", ""),
            Event::Message(message) => self.fire_raw("Message", "", &message, ""),
            Event::Collision { actor, with } => {
                let other_name = project
                    .actors
                    .iter()
                    .find(|candidate| candidate.id == with)
                    .map(|candidate| candidate.name.as_str())
                    .unwrap_or("");
                self.fire_raw("Collision", &actor, &with, other_name);
            }
            Event::UiEvent { id, event } => {
                self.fire_raw("UiEvent", "", &format!("{event}\n{id}"), "")
            }
            Event::UiClicked { id } => self.fire_raw("UiClicked", "", &id, ""),
            Event::UiChanged { id, .. } => self.fire_raw("UiChanged", "", &id, ""),
            Event::AnimationEnded { actor, clip } => {
                self.fire_raw("AnimationEnded", &actor, &clip, "")
            }
            Event::Particles { actor, event } => {
                self.fire_raw("Particles", &actor, event.name(), "")
            }
            Event::AnimationMarker { actor, marker } => {
                self.fire_raw("AnimationMarker", &actor, &marker, "")
            }
            Event::EnteredRoom { actor, room } => self.fire_raw("EnteredRoom", &actor, &room, ""),
            Event::Weather { weather } => self.fire_raw("Weather", "", &weather, ""),
            Event::CutsceneSignal { signal } => self.fire_raw("CutsceneSignal", "", &signal, ""),
            Event::CutsceneEnded { cutscene } => self.fire_raw("CutsceneEnded", "", &cutscene, ""),
            Event::Action(action) => self.fire_raw("Action", "", &action, ""),
            Event::Touched => self.fire_raw("Touched", "", "", ""),
            // The program makes its own clones and starts their strands
            // itself, so nothing outside it queues one. A script's clone
            // comes through `cloned` below instead.
            Event::Cloned { .. } => {}
        }
    }

    /// Switches the program to the scene `scene_id` names, the way
    /// `Vm::load_scene` does. Answers false when the program has no such
    /// scene - a stale build, or one from before multi-scene logic - and
    /// the host falls back to the VM.
    pub fn load_scene(&mut self, scene_id: &str) -> bool {
        let Some(scene) = self.scene else {
            return false;
        };
        unsafe { scene(self.state, AbiStr::borrow(scene_id)) == ABI_OK }
    }

    /// Hands over a clone something outside the program made - a script's -
    /// so the copy's `when I start as a clone` strands run too.
    pub fn cloned(&mut self, clone: &str, template: &str) {
        self.fire_raw("Cloned", clone, template, "");
    }

    /// The same for an actor the program never had, so a block can still name
    /// it. It has no blocks of its own, so nothing is scheduled for it.
    pub fn created(&mut self, actor: &str, name: &str) {
        self.fire_raw("Created", actor, name, "");
    }

    /// Takes an actor out of the run: its strands stop, as they do when a
    /// block deletes it.
    pub fn deleted(&mut self, actor: &str) {
        self.fire_raw("Deleted", actor, "", "");
    }

    fn fire_raw(&mut self, kind: &str, actor: &str, detail: &str, other_name: &str) {
        unsafe {
            (self.fire)(
                self.state,
                AbiStr::borrow(kind),
                AbiStr::borrow(actor),
                AbiStr::borrow(detail),
                AbiStr::borrow(other_name),
            )
        };
    }

    pub fn tick(
        &mut self,
        now: f64,
        wall: f64,
        variables: Variables,
        lists: Lists,
        dicts: Dicts,
        effects: &mut Vec<Effect>,
        messages: &mut Vec<String>,
    ) {
        let mut context = Context {
            variables,
            lists,
            dicts,
            effects,
            messages,
        };
        let status = unsafe {
            (self.tick)(
                self.state,
                (&raw mut context).cast(),
                &raw const HOST_API,
                now,
                wall,
            )
        };
        match status {
            ABI_OK => {}
            TICK_STOPPED => context.effects.push(Effect::Stopped),
            ABI_PANIC => context.effects.push(Effect::Error {
                actor: String::new(),
                message: "compiled block program panicked".to_string(),
            }),
            other => context.effects.push(Effect::Error {
                actor: String::new(),
                message: format!("compiled block program returned status {other}"),
            }),
        }
        // Keeps the library alive across the call, which is the whole
        // reason it is held here. Web builds never open one.
        #[cfg(not(target_arch = "wasm32"))]
        let _ = &self.library;
    }
}

impl Drop for LoadedLogic {
    fn drop(&mut self) {
        unsafe { (self.free)(self.state) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn missing_export() -> String {
    "the compiled block program is missing an entry point".to_string()
}

struct Context<'a> {
    variables: Variables,
    lists: Lists,
    dicts: Dicts,
    effects: &'a mut Vec<Effect>,
    messages: &'a mut Vec<String>,
}

unsafe fn context<'a>(pointer: *mut c_void) -> &'a mut Context<'a> {
    unsafe { &mut *pointer.cast::<Context>() }
}

static HOST_API: LogicHostApi = LogicHostApi {
    abi: LOGIC_ABI_VERSION,
    read,
    set_variable,
    act,
    set_clock,
};

/// Which clock the strand about to run keeps. The generated program says so
/// before every slice, so `timer` answers a strand the interface started
/// with the wall clock - exactly as it does under the VM.
extern "C" fn set_clock(_pointer: *mut c_void, ui: u32) {
    sense::set_ui_strand(ui != 0);
}

extern "C" fn read(
    pointer: *mut c_void,
    actor: AbiStr,
    what: u32,
    name: AbiStr,
    args: *const AbiValue,
    arg_count: usize,
    out: *mut AbiValue,
    text: *mut u8,
    capacity: usize,
    needed: *mut usize,
) -> u32 {
    let context = unsafe { context(pointer) };
    let actor = unsafe { actor.as_str() };
    let name = unsafe { name.as_str() };
    let args = if args.is_null() || arg_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args, arg_count) }
    };
    let answer = match what {
        READ_VARIABLE => Ok(context.variables.read(actor, name)),
        READ_SENSE => {
            // List reporters arrive here like any other `Op::Ext` sensing
            // reporter, but read the run's lists rather than the world.
            if is_list_reporter(&Op::from_name(name)) {
                let args: Vec<Value> = args
                    .iter()
                    .map(|arg| value_from_abi(arg).into_value())
                    .collect();
                let lists = context.lists.snapshot_for(actor);
                return write_answer(
                    resolve_list_reporter(name, args, &lists).and_then(|value| value.eval()),
                    out,
                    text,
                    capacity,
                    needed,
                );
            }
            // Dict reporters likewise, against the run's dicts.
            if is_dict_reporter(&Op::from_name(name)) {
                let args: Vec<Value> = args
                    .iter()
                    .map(|arg| value_from_abi(arg).into_value())
                    .collect();
                let dicts = context.dicts.snapshot_for(actor);
                return write_answer(
                    resolve_dict_reporter(name, args, &dicts).and_then(|value| value.eval()),
                    out,
                    text,
                    capacity,
                    needed,
                );
            }
            // `random` and `current time` are core operators, not extension
            // ones, but the emitter routes them through the host like any
            // other sensing reporter so one run has one source of each.
            // Without them every `rnd` pitch in a built game reads as
            // missing (silently zero) and clamps to 0.125, three octaves
            // down, while fixed-pitch music plays on untouched.
            if name == "Random" {
                let values: Vec<Evaluated> = args.iter().map(value_from_abi).collect();
                let get = |i: usize| {
                    values
                        .get(i)
                        .cloned()
                        .unwrap_or(Evaluated::Number(0.0))
                        .as_number()
                };
                let answer = match (get(0), get(1)) {
                    (Ok(l), Ok(r)) => {
                        Value::op(Op::Random, vec![Value::number(l), Value::number(r)]).eval()
                    }
                    (Err(message), _) | (_, Err(message)) => Err(message),
                };
                return write_answer(answer, out, text, capacity, needed);
            }
            if name == "CurrentTime" {
                let values: Vec<Evaluated> = args.iter().map(value_from_abi).collect();
                let arg = values
                    .first()
                    .cloned()
                    .unwrap_or(Evaluated::Text(String::new()));
                let answer = Value::op(Op::CurrentTime, vec![arg.into_value()]).eval();
                return write_answer(answer, out, text, capacity, needed);
            }
            let Some(operator) = ext_operator(name) else {
                return ABI_MISSING;
            };
            let values: Vec<Evaluated> = args.iter().map(value_from_abi).collect();
            sense::with_actor(actor, || (operator.eval)(&values))
        }
        _ => return ABI_MISSING,
    };
    write_answer(answer, out, text, capacity, needed)
}

fn write_answer(
    answer: Result<Evaluated, String>,
    out: *mut AbiValue,
    text: *mut u8,
    capacity: usize,
    needed: *mut usize,
) -> u32 {
    let (kind, number, body) = match answer {
        Ok(Evaluated::Number(number)) => (VALUE_NUMBER, number, None),
        Ok(Evaluated::Bool(value)) => (VALUE_BOOL, if value { 1.0 } else { 0.0 }, None),
        Ok(Evaluated::Text(value)) => (VALUE_TEXT, 0.0, Some(value)),
        Err(error) => (VALUE_ERROR, 0.0, Some(error)),
    };
    let length = body.as_ref().map_or(0, String::len);
    unsafe {
        *needed = length;
        (*out).kind = kind;
        (*out).number = number;
        (*out).text = AbiStr::EMPTY;
    }
    if length > capacity {
        return ABI_TOO_LONG;
    }
    if let Some(body) = body
        && !body.is_empty()
    {
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), text, body.len()) };
    }
    ABI_OK
}

extern "C" fn set_variable(pointer: *mut c_void, actor: AbiStr, name: AbiStr, value: AbiValue) {
    let context = unsafe { context(pointer) };
    context.variables.write(
        unsafe { actor.as_str() },
        unsafe { name.as_str() },
        value_from_abi(&value),
    );
}

extern "C" fn act(
    pointer: *mut c_void,
    actor: AbiStr,
    what: u32,
    a: AbiStr,
    b: AbiStr,
    c: AbiStr,
    numbers: *const f64,
    count: usize,
    value: AbiValue,
) {
    let context = unsafe { context(pointer) };
    let actor = unsafe { actor.as_str() }.to_string();
    let a = unsafe { a.as_str() };
    let b = unsafe { b.as_str() };
    let c = unsafe { c.as_str() };
    // A run of numbers rather than a fixed three, because one interface
    // element names nine at once. A short one reads as zeros from there on.
    let numbers: &[f64] = if numbers.is_null() || count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(numbers, count) }
    };
    let at = |index: usize| numbers.get(index).copied().unwrap_or(0.0);
    let (n0, n1, n2) = (at(0), at(1), at(2));
    let vector = [n0 as f32, n1 as f32, n2 as f32];
    let effect = match what {
        ACT_MOVE => Effect::Move {
            actor,
            steps: n0 as f32,
        },
        ACT_GO_TO => Effect::GoTo {
            actor,
            position: vector,
        },
        ACT_NAVIGATE_TO => Effect::NavigateTo {
            actor,
            target: vector,
            speed: at(3) as f32,
        },
        ACT_BURST_PARTICLES => Effect::BurstParticles {
            actor,
            count: (n0 as i64).clamp(0, 512) as u32,
        },
        ACT_SET_EMITTER_DIAL => Effect::SetEmitterDial {
            actor,
            dial: match a {
                "Rate" => blockloom_core::blocks::EmitterDial::Rate,
                "Lifetime" => blockloom_core::blocks::EmitterDial::Lifetime,
                "Speed" => blockloom_core::blocks::EmitterDial::Speed,
                "Spread" => blockloom_core::blocks::EmitterDial::Spread,
                "Gravity" => blockloom_core::blocks::EmitterDial::Gravity,
                "SizeStart" => blockloom_core::blocks::EmitterDial::SizeStart,
                "SizeEnd" => blockloom_core::blocks::EmitterDial::SizeEnd,
                "Max" => blockloom_core::blocks::EmitterDial::Max,
                _ => return,
            },
            value: n0 as f32,
        },
        ACT_SET_TRAIL_ENABLED => Effect::SetTrailEnabled {
            actor,
            enabled: n0 != 0.0,
        },
        ACT_SET_EMITTER_PLAYING => Effect::SetEmitterPlaying {
            actor,
            playing: n0 != 0.0,
        },
        ACT_CHANGE_POSITION => Effect::ChangePosition {
            actor,
            axis: axis_of(n0),
            by: n1 as f32,
        },
        ACT_GLIDE => Effect::Glide {
            actor,
            seconds: n0 as f32,
            target: [n1 as f32, n2 as f32, value.number as f32],
            easing: blockloom_core::animation::TweenEasing::parse(a)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        ACT_TWEEN_SCALE => Effect::TweenScale {
            actor,
            factor: n0 as f32,
            seconds: n1 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(a)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        ACT_TWEEN_ROTATION => Effect::TweenRotation {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
            seconds: n2 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(a)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        ACT_TWEEN_COLOR => Effect::TweenColor {
            actor,
            color: a.to_string(),
            seconds: n0 as f32,
            easing: blockloom_core::animation::TweenEasing::parse(b)
                .unwrap_or(blockloom_core::animation::TweenEasing::Linear),
        },
        ACT_STOP_TWEENS => Effect::StopTweens { actor },
        ACT_PLAY_ANIMATION => Effect::PlayAnimation {
            actor,
            clip: a.trim().to_string(),
            speed: n0 as f32,
        },
        ACT_STOP_ANIMATION => Effect::StopAnimation { actor },
        ACT_SET_ANIMATION_SPEED => Effect::SetAnimationSpeed {
            actor,
            speed: n0 as f32,
        },
        ACT_FIRE_ANIMATION_TRIGGER => Effect::FireAnimationTrigger {
            actor,
            name: a.trim().to_string(),
        },
        ACT_SET_RIG_SLOT => Effect::SetRigSlot {
            actor,
            slot: a.trim().to_string(),
            attachment: b.trim().to_string(),
        },
        ACT_SET_SLOT_TINT => Effect::SetSlotTint {
            actor,
            slot: a.trim().to_string(),
            color: b.to_string(),
        },
        ACT_SET_IK_TARGET => Effect::SetIkTarget {
            actor,
            constraint: a.trim().to_string(),
            x: n0 as f32,
            y: n1 as f32,
        },
        ACT_SET_SPRITE_DIAL => Effect::SetSpriteDial {
            actor,
            dial: match blockloom_core::blocks::SpriteDial::parse(a) {
                Some(dial) => dial,
                None => return,
            },
            value: n0 as f32,
        },
        ACT_TURN => Effect::Turn {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
        },
        ACT_SET_ROTATION => Effect::SetRotation {
            actor,
            axis: axis_of(n0),
            degrees: n1 as f32,
        },
        ACT_POINT_TOWARDS => Effect::PointTowards {
            actor,
            target: a.to_string(),
        },
        ACT_SET_SCALE => Effect::SetScale {
            actor,
            factor: n0 as f32,
        },
        ACT_SET_RENDER_SETTING => Effect::SetRenderSetting {
            setting: match a {
                "Quality" => blockloom_core::quality::Setting::Quality,
                "ResolutionScale" => blockloom_core::quality::Setting::ResolutionScale,
                "Upscaler" => blockloom_core::quality::Setting::Upscaler,
                "DlssMode" => blockloom_core::quality::Setting::DlssMode,
                _ => return,
            },
            value: b.to_string(),
        },
        ACT_SET_EXPOSURE => Effect::SetExposure { ev: n0 as f32 },
        ACT_SET_LIGHT_INTENSITY => Effect::SetLightIntensity {
            actor,
            intensity: n0 as f32,
        },
        ACT_SET_EMISSIVE_STRENGTH => Effect::SetEmissiveStrength {
            actor,
            strength: n0 as f32,
        },
        ACT_SET_HDR_OUTPUT => Effect::SetHdrOutput { enabled: n0 != 0.0 },
        ACT_SET_PEAK_BRIGHTNESS => Effect::SetPeakBrightness { nits: n0 as f32 },
        ACT_ENABLE_VOLUME => Effect::SetVolumeEnabled {
            actor,
            volume: a.trim().to_string(),
            enabled: n0 != 0.0,
        },
        ACT_SET_VOLUME_WEIGHT => Effect::SetVolumeWeight {
            actor,
            volume: a.trim().to_string(),
            weight: n0 as f32,
        },
        ACT_CAPTURE_PROBES => Effect::CaptureProbes,
        ACT_SET_SHADOW_DISTANCE => Effect::SetShadowDistance {
            distance: n0 as f32,
        },
        ACT_SET_LIGHT_SHADOWS => Effect::SetLightShadows {
            actor,
            enabled: n0 != 0.0,
        },
        ACT_SET_RAY_TRACING => Effect::SetRayTracing { enabled: n0 != 0.0 },
        ACT_SET_GI_BOUNCES => Effect::SetGiBounces { bounces: n0 as f32 },
        ACT_SET_GI_SAMPLES => Effect::SetGiSamples { samples: n0 as f32 },
        ACT_SET_FOG_DENSITY => Effect::SetFogDensity { density: n0 as f32 },
        ACT_SET_AURORA => Effect::SetAurora { kp: n0 as f32 },
        ACT_STRIKE_LIGHTNING => Effect::StrikeLightning { at: vector },
        ACT_FRACTURE => Effect::Fracture {
            actor: a.to_string(),
        },
        ACT_SPLASH => Effect::Splash {
            at: vector,
            radius: at(3) as f32,
            strength: at(4) as f32,
        },
        ACT_PUFF_SMOKE => Effect::PuffSmoke {
            at: vector,
            radius: at(3) as f32,
            strength: at(4) as f32,
        },
        ACT_SPAWN_DECAL => {
            let Some(preset) = blockloom_core::decals::DecalPreset::parse(a) else {
                return;
            };
            Effect::SpawnDecal(blockloom_core::decals::Spawn {
                preset,
                at: vector,
                normal: [at(3) as f32, at(4) as f32, at(5) as f32],
                size: at(6) as f32,
                lifetime: at(7) as f32,
                fade: at(8) as f32,
            })
        }
        ACT_FADE_DECALS => Effect::FadeDecals {
            at: vector,
            radius: at(3) as f32,
            seconds: at(4) as f32,
        },
        ACT_SET_LIGHTNING_RATE => Effect::SetLightningRate { rate: n0 as f32 },
        ACT_SET_WIND => match blockloom_core::wind::WindProperty::parse(a) {
            Some(property) => Effect::SetWind {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no wind dial called \"{a}\""),
            },
        },
        ACT_SET_CLOUD_DRIFT => Effect::SetCloudDrift { drift: vector },
        ACT_SET_CLOUD_LAYER => match blockloom_core::cloud_layers::CloudLayerProperty::parse(a) {
            Some(property) => Effect::SetCloudLayer {
                layer: n0 as f32,
                property,
                value: n1 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no cloud layer dial called \"{a}\""),
            },
        },
        ACT_SET_CLOUDS => match blockloom_core::clouds::CloudProperty::parse(a) {
            Some(property) => Effect::SetClouds {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no cloud dial called \"{a}\""),
            },
        },
        ACT_SET_TIME_OF_DAY => Effect::SetTimeOfDay { time: n0 as f32 },
        ACT_ADVANCE_TIME => Effect::AdvanceTime { hours: n0 as f32 },
        ACT_SET_PRECIPITATION => match blockloom_core::director::PrecipitationKind::parse(a) {
            Some(property) => Effect::SetPrecipitation {
                property,
                value: n0 as f32,
            },
            None => Effect::Error {
                actor,
                message: format!("there's no precipitation called \"{a}\""),
            },
        },
        ACT_BLEND_WEATHER => Effect::BlendWeather {
            weather: a.trim().to_string(),
            seconds: n0 as f32,
        },
        ACT_PLAY_CUTSCENE => Effect::PlayCutscene {
            cutscene: a.trim().to_string(),
        },
        ACT_SKIP_CUTSCENE => Effect::SkipCutscene,
        ACT_CAMERA_SHAKE => Effect::CameraShake { amount: n0 as f32 },
        ACT_SET_TIME_SCALE => Effect::SetTimeScale { scale: n0 as f32 },
        ACT_HITSTOP => Effect::Hitstop { frames: n0 as f32 },
        ACT_SET_LETTERBOX => Effect::SetLetterbox { on: n0 as f32 },
        ACT_FADE_SCREEN => Effect::FadeScreen {
            color: blockloom_core::cinematic::normalize_fade(a),
        },
        ACT_PAINT_TILE => Effect::PaintTile {
            actor,
            map: a.trim().to_string(),
            tile: (n0.floor() as i32).max(-1),
            x: n1 as f32,
            y: n2 as f32,
            z: at(3) as f32,
        },
        ACT_SET_PARALLAX => match blockloom_core::tilemap::ParallaxAxis::parse(b) {
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
        ACT_SET_WATER => match blockloom_core::water::WaterProperty::parse(a) {
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
        ACT_SET_BODY => Effect::SetBody {
            actor,
            body: body_of(a),
        },
        ACT_APPLY_IMPULSE => Effect::ApplyImpulse {
            actor,
            impulse: vector,
        },
        ACT_SET_VELOCITY => Effect::SetVelocity {
            actor,
            velocity: vector,
        },
        ACT_SET_GRAVITY => Effect::SetGravity { gravity: vector },
        ACT_SET_DENSITY => Effect::SetDensity {
            actor,
            density: n0 as f32,
        },
        ACT_SET_MASS => Effect::SetMass {
            actor,
            mass: n0 as f32,
        },
        ACT_SET_TRIGGER => Effect::SetTrigger {
            actor,
            trigger: n0 != 0.0,
        },
        ACT_SET_COLLISION_LAYER => Effect::SetCollisionLayer {
            actor,
            layer: (n0.round() as i32).clamp(1, 8) as u8,
        },
        ACT_SET_COLLISION_MASK => Effect::SetCollisionMask {
            actor,
            mask: (n0.round() as i32).clamp(0, 255) as u8,
        },
        ACT_RUMBLE_GAMEPAD => Effect::RumbleGamepad {
            strength: (n0 as f32).clamp(0.0, 100.0),
            duration: (n1 as f32).max(0.0),
        },
        ACT_BIND_ACTION => Effect::BindAction {
            actor,
            action: a.trim().to_string(),
            binding: b.trim().to_string(),
        },
        ACT_CLEAR_ACTION_BINDINGS => Effect::ClearActionBindings {
            actor,
            action: a.trim().to_string(),
        },
        ACT_SAY => Effect::Say {
            actor,
            text: a.to_string(),
        },
        ACT_SET_VISIBLE => Effect::SetVisible {
            actor,
            visible: n0 != 0.0,
        },
        ACT_SET_COLOR => Effect::SetColor {
            actor,
            color: a.to_string(),
        },
        ACT_PLAY_SOUND => Effect::PlaySound {
            actor,
            sound: a.trim().to_string(),
            volume: n0 as f32,
            pitch: n1 as f32,
            loop_: n2 != 0.0,
            bus: bus_of(b),
            at: (!c.is_empty()).then(|| c.to_string()),
        },
        ACT_STOP_SOUND => Effect::StopSound {
            actor,
            sound: a.trim().to_string(),
        },
        ACT_SET_SOUND_VOLUME => Effect::SetSoundVolume {
            actor,
            sound: a.trim().to_string(),
            volume: n0 as f32,
        },
        ACT_SET_SOUND_PITCH => Effect::SetSoundPitch {
            actor,
            sound: a.trim().to_string(),
            pitch: n0 as f32,
        },
        ACT_SET_BUS_VOLUME => Effect::SetBusVolume {
            bus: bus_of(b),
            volume: n0 as f32,
        },
        ACT_SET_FIELD => Effect::SetComponentField {
            actor,
            component: a.to_string(),
            field: b.to_string(),
            value: value_from_abi(&value),
        },
        ACT_SET_CAMERA_VIEW => Effect::SetCameraView {
            actor,
            view: view_of(a),
        },
        ACT_ATTACH => Effect::AttachComponent {
            actor,
            component: a.to_string(),
        },
        ACT_DETACH => Effect::DetachComponent {
            actor,
            component: a.to_string(),
        },
        ACT_SET_PARENT => Effect::SetParent {
            actor,
            parent: a.trim().to_string(),
        },
        // The program has already given the copy an id and its own strands;
        // the entity is all that is left, and that is the host's. The
        // variables are copied here rather than when the effect lands,
        // because the VM copies them the moment the block runs.
        ACT_CREATE_CLONE => {
            context.variables.copy_actor(a, b);
            Effect::CreateClone {
                actor,
                clone: b.to_string(),
                of: a.to_string(),
            }
        }
        ACT_CREATE_ACTOR => Effect::CreateActor {
            actor,
            id: a.to_string(),
            name: b.to_string(),
            position: vector,
        },
        // About the actor it takes out of the run, not the one that asked.
        ACT_DELETE_ACTOR => {
            context.variables.forget_actor(a);
            Effect::DeleteActor {
                actor: a.to_string(),
            }
        }
        ACT_BROADCAST => {
            context.messages.push(a.to_string());
            return;
        }
        ACT_SWITCH_SCENE => Effect::SwitchScene {
            actor,
            scene: a.trim().to_string(),
            transition: b.trim().to_string(),
        },
        ACT_SET_MOUSE_LOCKED => Effect::SetMouseLocked { locked: n0 != 0.0 },
        ACT_SET_CAMERA_PITCH => Effect::SetCameraPitch {
            actor,
            degrees: n0 as f32,
        },
        ACT_SET_CAMERA_FOV => Effect::SetCameraFov {
            actor,
            fov: n0 as f32,
        },
        ACT_SHOW_ELEMENT => Effect::ShowElement {
            element: UiElement {
                id: a.to_string(),
                kind: UiKind::from_index(at(0) as usize),
                content: b.to_string(),
                anchor: UiAnchor::from_index(at(1) as usize),
                offset: [at(2) as f32, at(3) as f32],
                size: [at(4) as f32, at(5) as f32],
                parent: c.to_string(),
                modal: at(6) != 0.0,
                range: [at(7) as f32, at(8) as f32],
                value: value_from_abi(&value),
            },
        },
        ACT_SET_UI_PROP => match UiProp::from_name(b) {
            Some(prop) => Effect::SetUiProp {
                id: a.to_string(),
                prop,
                value: value_from_abi(&value),
            },
            // A property the program knows and this player doesn't: nothing
            // to write, and nothing worth stopping the run for.
            None => return,
        },
        ACT_HIDE_ELEMENT => Effect::HideElement {
            id: a.to_string(),
            all: n0 != 0.0,
        },
        ACT_DELETE_ELEMENT => Effect::DeleteElement { id: a.to_string() },
        ACT_SET_FOCUS => Effect::SetFocus { id: a.to_string() },
        ACT_SET_UI_THEME => Effect::SetUiTheme {
            theme: UiTheme::from_index(n0 as usize),
        },
        ACT_SET_PAUSED => Effect::SetPaused { paused: n0 != 0.0 },
        ACT_SAVE_VARIABLE => Effect::SaveVariable {
            actor,
            name: a.to_string(),
            clear: n0 != 0.0,
        },
        // List writes are the program's own state, like variables: they land
        // directly and report nothing, so no effect leaves this match.
        ACT_LIST_ADD => match ListItem::from_evaluated(value_from_abi(&value)) {
            Some(item) => {
                context
                    .lists
                    .with_list_mut(&actor, a, |list| list.push(item));
                return;
            }
            None => Effect::Error {
                actor,
                message: "list items must be number or text".to_string(),
            },
        },
        ACT_LIST_DELETE => {
            context.lists.with_list_mut(&actor, a, |list| {
                if let Some(at) = list_index(n0, list.len(), false) {
                    list.remove(at);
                }
            });
            return;
        }
        ACT_LIST_CLEAR => {
            context.lists.with_list_mut(&actor, a, |list| list.clear());
            return;
        }
        ACT_LIST_SHIFT => {
            context.lists.with_list_mut(&actor, a, |list| {
                if !list.is_empty() {
                    let len = list.len();
                    let distance = n0.round() as isize;
                    if distance >= 0 {
                        list.rotate_right(distance as usize % len);
                    } else {
                        list.rotate_left(distance.unsigned_abs() % len);
                    }
                }
            });
            return;
        }
        ACT_LIST_INSERT => match ListItem::from_evaluated(value_from_abi(&value)) {
            Some(item) => {
                context.lists.with_list_mut(&actor, a, |list| {
                    if let Some(at) = list_index(n0, list.len(), true) {
                        list.insert(at, item);
                    }
                });
                return;
            }
            None => Effect::Error {
                actor,
                message: "list items must be number or text".to_string(),
            },
        },
        ACT_LIST_REPLACE => match ListItem::from_evaluated(value_from_abi(&value)) {
            Some(item) => {
                context.lists.with_list_mut(&actor, a, |list| {
                    if let Some(at) = list_index(n0, list.len(), false) {
                        list[at] = item;
                    }
                });
                return;
            }
            None => Effect::Error {
                actor,
                message: "list items must be number or text".to_string(),
            },
        },
        ACT_LIST_REVERSE => {
            context
                .lists
                .with_list_mut(&actor, a, |list| list.reverse());
            return;
        }
        // Dict writes are the program's own state, like lists: they land
        // directly and report nothing, so no effect leaves this match.
        // `b` carries the key, `value` the item or JSON text.
        ACT_DICT_SET => match DictItem::from_evaluated(value_from_abi(&value)) {
            Some(item) => {
                let key = b.to_string();
                context
                    .dicts
                    .with_dict_mut(&actor, a, |dict| dict_set(dict, key, item));
                return;
            }
            None => Effect::Error {
                actor,
                message: "dict values must be number or text".to_string(),
            },
        },
        ACT_DICT_DELETE_KEY => {
            context.dicts.with_dict_mut(&actor, a, |dict| {
                dict_remove(dict, b);
            });
            return;
        }
        ACT_DICT_CLEAR => {
            context.dicts.with_dict_mut(&actor, a, |dict| dict.clear());
            return;
        }
        ACT_JSON_TO_DICT => match parse_json_object(&value_from_abi(&value).as_text()) {
            Ok(entries) => {
                context
                    .dicts
                    .with_dict_mut(&actor, a, |dict| *dict = entries);
                return;
            }
            Err(message) => Effect::Error { actor, message },
        },
        ACT_JSON_TO_LIST => match parse_json_array(&value_from_abi(&value).as_text()) {
            Ok(items) => {
                context.lists.with_list_mut(&actor, a, |list| *list = items);
                return;
            }
            Err(message) => Effect::Error { actor, message },
        },
        ACT_ERROR => Effect::Error {
            actor,
            message: a.to_string(),
        },
        _ => return,
    };
    context.effects.push(effect);
}

fn value_from_abi(value: &AbiValue) -> Evaluated {
    match value.kind {
        VALUE_TEXT => Evaluated::Text(unsafe { value.text.as_str() }.to_string()),
        VALUE_BOOL => Evaluated::Bool(value.number != 0.0),
        _ => Evaluated::Number(value.number),
    }
}

fn axis_of(value: f64) -> Axis {
    match value as usize {
        1 => Axis::Y,
        2 => Axis::Z,
        _ => Axis::X,
    }
}

fn body_of(value: &str) -> BodyKind {
    match value {
        "Static" => BodyKind::Static,
        "Dynamic" => BodyKind::Dynamic,
        "Kinematic" => BodyKind::Kinematic,
        _ => BodyKind::None,
    }
}

fn view_of(value: &str) -> CameraView {
    match value {
        "FirstPerson" => CameraView::FirstPerson,
        "ThirdPerson" => CameraView::ThirdPerson,
        _ => CameraView::Follow,
    }
}

/// A bus name off the wire into the bus it names. Unknown spellings read as
/// the effects bus rather than refusing the play: a compiled program only
/// ever sends what the emitter wrote, which is always one of the three.
fn bus_of(value: &str) -> SoundBus {
    SoundBus::parse(value).unwrap_or(SoundBus::Sfx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};
    use blockloom_core::value::Value;

    #[test]
    fn a_generated_library_runs_through_the_player_boundary() {
        if blockloom_core::script::toolchain_version().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "blockloom-native-logic-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut project = Project::starter("Native logic", Mode::TwoD);
        project.actors.clear();
        let mut actor = Actor::new(
            "Player",
            Visual::Circle {
                color: "#fff".to_string(),
                radius: 10.0,
            },
        );
        actor.id = "a1".to_string();
        actor.graph.strands.push(Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenStarted),
                Instruction::new(K::SetVariable {
                    name: "distance".to_string(),
                    value: Value::number(9.0),
                }),
                Instruction::new(K::Move {
                    steps: Value::Var {
                        name: "distance".to_string(),
                    },
                }),
                Instruction::new(K::SpawnDecal {
                    preset: blockloom_core::decals::DecalPreset::Footprint,
                    x: Value::number(1.0),
                    y: Value::number(2.0),
                    z: Value::number(3.0),
                    nx: Value::number(0.0),
                    ny: Value::number(1.0),
                    nz: Value::number(0.0),
                    size: Value::number(0.5),
                    lifetime: Value::number(12.0),
                    fade: Value::number(2.0),
                }),
                Instruction::new(K::FadeDecals {
                    x: Value::number(1.0),
                    y: Value::number(2.0),
                    z: Value::number(3.0),
                    radius: Value::number(5.0),
                    seconds: Value::number(1.0),
                }),
            ],
        ));
        project.actors.push(actor);

        codegen::compile_for(&project, &root, None).unwrap();
        let variables = Variables::default();
        variables.load(&project);
        let lists = Lists::default();
        lists.load(&project);
        let dicts = Dicts::default();
        dicts.load(&project);
        let mut logic = LoadedLogic::load(&root).unwrap();
        logic.fire(Event::Started, &project);
        let mut effects = Vec::new();
        let mut messages = Vec::new();
        logic.tick(
            0.0,
            0.0,
            variables.clone(),
            lists.clone(),
            dicts.clone(),
            &mut effects,
            &mut messages,
        );

        assert_eq!(
            effects,
            vec![
                Effect::Move {
                    actor: "a1".to_string(),
                    steps: 9.0,
                },
                Effect::SpawnDecal(blockloom_core::decals::Spawn {
                    preset: blockloom_core::decals::DecalPreset::Footprint,
                    at: [1.0, 2.0, 3.0],
                    normal: [0.0, 1.0, 0.0],
                    size: 0.5,
                    lifetime: 12.0,
                    fade: 2.0,
                }),
                Effect::FadeDecals {
                    at: [1.0, 2.0, 3.0],
                    radius: 5.0,
                    seconds: 1.0
                }
            ]
        );
        assert_eq!(variables.read("a1", "distance"), Evaluated::Number(9.0));
        drop(logic);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_clone_runs_its_own_strand_under_its_own_id_through_the_boundary() {
        if blockloom_core::script::toolchain_version().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "blockloom-native-clone-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut project = Project::starter("Native clones", Mode::TwoD);
        project.actors.clear();
        let mut actor = Actor::new(
            "Player",
            Visual::Circle {
                color: "#fff".to_string(),
                radius: 10.0,
            },
        );
        actor.id = "a1".to_string();
        // The template counts to 5, clones itself, and the copy counts one
        // more onto the number it inherited.
        actor.graph.strands.push(Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(K::WhenStarted),
                Instruction::new(K::SetVariable {
                    name: "hits".to_string(),
                    value: Value::number(5.0),
                }),
                Instruction::new(K::CreateClone { of: String::new() }),
            ],
        ));
        actor.graph.strands.push(Strand::with_instructions(
            0,
            400,
            vec![
                Instruction::new(K::WhenCloned),
                Instruction::new(K::ChangeVariable {
                    name: "hits".to_string(),
                    value: Value::number(1.0),
                }),
                Instruction::new(K::Move {
                    steps: Value::Var {
                        name: "hits".to_string(),
                    },
                }),
            ],
        ));
        project.actors.push(actor);

        codegen::compile_for(&project, &root, None).unwrap();
        let variables = Variables::default();
        variables.load(&project);
        let lists = Lists::default();
        lists.load(&project);
        let dicts = Dicts::default();
        dicts.load(&project);
        let mut logic = LoadedLogic::load(&root).unwrap();
        logic.fire(Event::Started, &project);

        let mut effects = Vec::new();
        let mut messages = Vec::new();
        logic.tick(
            0.0,
            0.0,
            variables.clone(),
            lists.clone(),
            dicts.clone(),
            &mut effects,
            &mut messages,
        );
        let clone = match effects.as_slice() {
            [Effect::CreateClone { actor, clone, of }] => {
                assert_eq!(actor, "a1");
                assert_eq!(of, "a1");
                clone.clone()
            }
            other => panic!("expected one clone, got {other:?}"),
        };

        // The copy's own strand runs on the next tick, not this one.
        effects.clear();
        logic.tick(
            1.0 / 60.0,
            1.0 / 60.0,
            variables.clone(),
            lists.clone(),
            dicts.clone(),
            &mut effects,
            &mut messages,
        );
        assert_eq!(
            effects,
            vec![Effect::Move {
                actor: clone.clone(),
                steps: 6.0,
            }],
            "the clone started from its template's 5 and counted its own one on"
        );
        assert_eq!(variables.read("a1", "hits"), Evaluated::Number(5.0));
        assert_eq!(variables.read(&clone, "hits"), Evaluated::Number(6.0));

        drop(logic);
        let _ = std::fs::remove_dir_all(root);
    }
}
