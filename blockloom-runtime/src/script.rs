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
use blockloom_core::script::abi::{self, HostApi, Str};
use blockloom_core::sense;
use blockloom_core::ui::{UiAnchor, UiElement, UiKind, UiProp, UiTheme};
use blockloom_core::value::Evaluated;
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
use std::ffi::c_void;
use std::path::Path;

type StartFn = unsafe extern "C" fn(*mut c_void, *const HostApi);
type TickFn = unsafe extern "C" fn(*mut c_void, *const HostApi, f32);
type AbiFn = unsafe extern "C" fn() -> u32;

/// One actor's script, open and ready to call. The library is kept alive
/// alongside the pointers into it, and closing it is what dropping this does.
pub struct LoadedScript {
    /// Dropped last, after the pointers that live inside it.
    library: libloading::Library,
    start: StartFn,
    tick: TickFn,
}

impl LoadedScript {
    /// Whether the editor has built this script yet. Before the first Play it
    /// hasn't, which is ordinary rather than a problem worth reporting.
    pub fn is_built(project_dir: &Path, relative: &str) -> bool {
        blockloom_core::script::library_path(project_dir, relative).is_file()
    }

    /// Opens the library the editor built for `relative`, or says why it
    /// couldn't be used.
    pub fn load(project_dir: &Path, relative: &str) -> Result<LoadedScript, String> {
        let path = blockloom_core::script::library_path(project_dir, relative);
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
            Ok(LoadedScript {
                library,
                start,
                tick,
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
unsafe fn ctx<'a>(pointer: *mut c_void) -> &'a mut Ctx<'a> {
    unsafe { &mut *pointer.cast::<Ctx>() }
}

/// The three entry points every script is given. A `static` rather than a
/// value built per call so its address is stable for as long as the process.
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
        UiKind::Slider => Evaluated::Number(value),
        other => UiElement::blank(other, flag),
    }
}

/// This actor as the frame's snapshot sees it.
fn me(actor: &str) -> Option<sense::ActorSense> {
    sense::read(|sensors| sensors.actors.get(actor).cloned())
}

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
        abi::READ_ACTOR_COUNT => Some(sense::read(|sensors| sensors.count_named(a)) as f64),
        abi::READ_POSITION_OF => {
            let axis = axis_of(arg).index();
            sense::read(|sensors| {
                sensors
                    .find(a.trim())
                    .map(|other| other.position[axis] as f64)
            })
        }
        _ => None,
    }
}

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
    let answer = match what {
        abi::TEXT_ACTOR_NAME => me(ctx.actor).map(|me| me.name),
        abi::TEXT_FIELD => me(ctx.actor)
            .and_then(|me| me.components.get(a.trim())?.get(b.trim()).cloned())
            .map(|value| value.as_text()),
        abi::TEXT_ACTOR_ID => Some(ctx.actor.to_string()),
        // An actor with no parent and one that made nothing both answer
        // `MISSING`, which the prelude turns into `None`.
        abi::TEXT_PARENT => me(ctx.actor)
            .map(|me| me.parent)
            .filter(|id| !id.is_empty()),
        abi::TEXT_NEW_ACTOR => me(ctx.actor)
            .map(|me| me.last_created)
            .filter(|id| !id.is_empty()),
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
        _ => None,
    };
    let Some(answer) = answer else {
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
    let actor = ctx.actor.to_string();
    // A run of numbers rather than a fixed three, because one interface
    // element names ten at once. A short run reads as zeros from there on.
    let numbers: &[f64] = if numbers.is_null() || count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(numbers, count) }
    };
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
        abi::ACT_SET_PARENT => Effect::SetParent {
            actor,
            parent: a.trim().to_string(),
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
