//! Loading and running a built game's compiled block program.

use blockloom_core::codegen::{
    self, ABI_MISSING, ABI_OK, ABI_PANIC, ABI_TOO_LONG, ACT_APPLY_IMPULSE, ACT_ATTACH,
    ACT_BROADCAST, ACT_CHANGE_POSITION, ACT_CREATE_ACTOR, ACT_CREATE_CLONE, ACT_DELETE_ACTOR,
    ACT_DELETE_ELEMENT, ACT_DETACH, ACT_ERROR, ACT_GLIDE, ACT_GO_TO, ACT_HIDE_ELEMENT, ACT_MOVE,
    ACT_POINT_TOWARDS, ACT_SAVE_VARIABLE, ACT_SAY, ACT_SET_BODY, ACT_SET_CAMERA_PITCH,
    ACT_SET_CAMERA_VIEW, ACT_SET_COLOR, ACT_SET_DENSITY, ACT_SET_FIELD, ACT_SET_FOCUS,
    ACT_SET_GRAVITY, ACT_SET_MASS, ACT_SET_MOUSE_LOCKED, ACT_SET_PARENT, ACT_SET_PAUSED,
    ACT_SET_ROTATION, ACT_SET_SCALE, ACT_SET_UI_PROP, ACT_SET_UI_THEME, ACT_SET_VELOCITY,
    ACT_SET_VISIBLE, ACT_SHOW_ELEMENT, ACT_TURN, AbiStr, AbiValue, LOGIC_ABI_VERSION, LogicHostApi,
    READ_SENSE, READ_VARIABLE, SYM_LOGIC_ABI, SYM_LOGIC_FIRE, SYM_LOGIC_FREE, SYM_LOGIC_NEW,
    SYM_LOGIC_PAUSE, SYM_LOGIC_RESET, SYM_LOGIC_TICK, TICK_STOPPED, VALUE_BOOL, VALUE_ERROR,
    VALUE_NUMBER, VALUE_TEXT,
};
use blockloom_core::components::CameraView;
use blockloom_core::project::Project;
use blockloom_core::scene::{Axis, BodyKind};
use blockloom_core::sense;
use blockloom_core::ui::{UiAnchor, UiElement, UiKind, UiProp, UiTheme};
use blockloom_core::value::{Evaluated, ext_operator};
use blockloom_core::vm::{Effect, Event, Variables};
use std::ffi::c_void;
use std::path::Path;

type AbiFn = unsafe extern "C" fn() -> u32;
type NewFn = unsafe extern "C" fn() -> *mut c_void;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type ResetFn = unsafe extern "C" fn(*mut c_void);
type FireFn = unsafe extern "C" fn(*mut c_void, AbiStr, AbiStr, AbiStr, AbiStr);
type TickFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const LogicHostApi, f64, f64) -> u32;
type PauseFn = unsafe extern "C" fn(*mut c_void, u32);

/// One generated program and its suspended strands.
pub struct LoadedLogic {
    library: libloading::Library,
    state: *mut c_void,
    free: FreeFn,
    reset: ResetFn,
    fire: FireFn,
    tick: TickFn,
    /// A program built before this export existed simply has none, and the
    /// editor's Pause then reaches it the long way, through `SetPaused`.
    pause: Option<PauseFn>,
}

impl LoadedLogic {
    pub fn is_built(project_dir: &Path) -> bool {
        codegen::library_path(project_dir).is_file()
    }

    pub fn load(project_dir: &Path) -> Result<Self, String> {
        let path = codegen::library_path(project_dir);
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
            })
        }
    }

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
            Event::UiClicked { id } => self.fire_raw("UiClicked", "", &id, ""),
            Event::UiChanged { id, .. } => self.fire_raw("UiChanged", "", &id, ""),
            // The program makes its own clones and starts their strands
            // itself, so nothing outside it queues one. A script's clone
            // comes through `cloned` below instead.
            Event::Cloned { .. } => {}
        }
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
        effects: &mut Vec<Effect>,
        messages: &mut Vec<String>,
    ) {
        let mut context = Context {
            variables,
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
        let _ = &self.library;
    }
}

impl Drop for LoadedLogic {
    fn drop(&mut self) {
        unsafe { (self.free)(self.state) };
    }
}

fn missing_export() -> String {
    "the compiled block program is missing an entry point".to_string()
}

struct Context<'a> {
    variables: Variables,
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
        ACT_CHANGE_POSITION => Effect::ChangePosition {
            actor,
            axis: axis_of(n0),
            by: n1 as f32,
        },
        ACT_GLIDE => Effect::Glide {
            actor,
            seconds: n0 as f32,
            target: [n1 as f32, n2 as f32, value.number as f32],
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
        ACT_SET_MOUSE_LOCKED => Effect::SetMouseLocked { locked: n0 != 0.0 },
        ACT_SET_CAMERA_PITCH => Effect::SetCameraPitch {
            actor,
            degrees: n0 as f32,
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
            ],
        ));
        project.actors.push(actor);

        codegen::compile_for(&project, &root, None).unwrap();
        let variables = Variables::default();
        variables.load(&project);
        let mut logic = LoadedLogic::load(&root).unwrap();
        logic.fire(Event::Started, &project);
        let mut effects = Vec::new();
        let mut messages = Vec::new();
        logic.tick(0.0, 0.0, variables.clone(), &mut effects, &mut messages);

        assert_eq!(
            effects,
            vec![Effect::Move {
                actor: "a1".to_string(),
                steps: 9.0,
            }]
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
        let mut logic = LoadedLogic::load(&root).unwrap();
        logic.fire(Event::Started, &project);

        let mut effects = Vec::new();
        let mut messages = Vec::new();
        logic.tick(0.0, 0.0, variables.clone(), &mut effects, &mut messages);
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
