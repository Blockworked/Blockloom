//! Typed Rust binding for the frozen script world `blockloom:script@0.2.0`.
//!
//! A guest written against this crate compiles to the core wasm shape the
//! script sandbox already runs: three `blockloom` imports and the
//! `blockloom_script_*` entry points. Same source builds both ways:
//!
//! ```sh
//! cargo build --target wasm32-unknown-unknown
//! ```
//!
//! as a `cdylib` for the sandbox, or as an `rlib` for host-side checks. The
//! world itself lives in `wit/world.wit`, byte for byte the frozen
//! [`blockloom_core::script::wit::WIT`](https://blockloom) text; this crate is
//! that world lowered by hand to the numeric ABI, because the host is core
//! wasmi rather than the component model. When the `wasmi` vs `wasmtime` call
//! is made, the same file feeds `wit-bindgen` directly.
//!
//! Deferred-effect semantics hold, exactly as for blocks and prelude scripts:
//! reads see the snapshot, writes apply later, read-after-write reads old.
//! Hot paths have typed homes (`lifecycle`, `state`, `vars`, `lists`,
//! `physics`, `plugins`); everything else rides the generic
//! `sensors`/`texts`/`acts` escapes under its verb constant.
//!
//! ```ignore
//! use blockloom_script_guest::{export_script, lifecycle, Event};
//!
//! fn start() {
//!     lifecycle::say("Hello from a guest");
//! }
//!
//! fn tick(_dt: f32) {
//!     lifecycle::change_position(0, 1.0);
//! }
//!
//! fn event(_event: &Event) {
//!     lifecycle::say("event heard");
//! }
//!
//! export_script!(start = start, tick = tick, event = event);
//! ```

mod verbs;

pub use verbs::*;

/// The frozen world this crate binds. Matches
/// `blockloom_core::script::wit::WIT_VERSION`; the core drift test refuses a
/// mismatch.
pub const WIT_VERSION: u32 = 2;

/// `package` and version are part of the freeze: renaming either is a new
/// world, not an edit.
pub const WIT_PACKAGE: &str = "blockloom:script@0.2.0";

/// The numeric ABI this binding lowers to. Matches `abi::ABI_VERSION`; the
/// core drift test refuses a mismatch.
pub const ABI_VERSION: u32 = 47;

/// What a host call answered. Anything but `OK` leaves the out-parameter
/// untouched.
pub const OK: u32 = 0;
/// No such component, field, actor or reading.
pub const MISSING: u32 = 1;
/// The answer didn't fit in the buffer the caller offered.
pub const TOO_LONG: u32 = 2;

/// The `TEXT_*` verb carrying the fixed pose record.
pub const BYTES_POSE: u32 = 24;
/// Seven little-endian `f32`s: position, rotation, scale.
pub const POSE_BYTES: usize = 28;

/// The slot a run writes to before any switch. Mirrors
/// `save::DEFAULT_SLOT`; the core drift test refuses drift.
pub const DEFAULT_SAVE_SLOT: &str = "default";
/// The language a run speaks before any switch. Mirrors
/// `locale::DEFAULT_LANGUAGE`; the core drift test refuses drift.
pub const DEFAULT_LANGUAGE: &str = "en";

/// One call's arguments, laid out in guest memory exactly as `abi::WasmCall`.
/// Every pointer is an offset into that memory; every field is a plain word
/// or a double, so both sides agree on the layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WasmCall {
    pub a_ptr: u32,
    pub a_len: u32,
    pub b_ptr: u32,
    pub b_len: u32,
    pub c_ptr: u32,
    pub c_len: u32,
    /// A run of `f64`s for `act`.
    pub numbers: u32,
    pub count: u32,
    /// `read_number`'s plain number.
    pub arg: f64,
    /// Where the answer goes: one `f64`, or `out_cap` bytes of text.
    pub out: u32,
    pub out_cap: u32,
    /// Where `read_text` writes the length its answer needs, as a `u32`.
    pub out_len: u32,
    pub _pad: u32,
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::WasmCall;
    use core::sync::atomic::{AtomicI32, Ordering};

    #[link(wasm_import_module = "blockloom")]
    unsafe extern "C" {
        #[link_name = "read_number"]
        fn import_read_number(ctx: i32, what: u32, call: *mut WasmCall) -> u32;
        #[link_name = "read_text"]
        fn import_read_text(ctx: i32, what: u32, call: *mut WasmCall) -> u32;
        #[link_name = "act"]
        fn import_act(ctx: i32, what: u32, call: *mut WasmCall) -> u32;
    }

    static CTX: AtomicI32 = AtomicI32::new(0);

    /// Notes the call now running and, once, hooks panics: a wasm build
    /// aborts on panic, so the message is logged before the host sees the
    /// trap and stops the script.
    pub fn enter(ctx: i32) {
        CTX.store(ctx, Ordering::Relaxed);
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                super::log(&format!("the script panicked: {info}"));
            }));
        });
    }

    fn ptr_of(bytes: &[u8]) -> u32 {
        bytes.as_ptr() as usize as u32
    }

    pub fn raw_read_number(what: u32, a: &str, b: &str, arg: f64) -> Option<f64> {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let mut out = 0.0f64;
        let mut call = WasmCall {
            a_ptr: ptr_of(a),
            a_len: a.len() as u32,
            b_ptr: ptr_of(b),
            b_len: b.len() as u32,
            arg,
            out: (&mut out as *mut f64) as usize as u32,
            ..WasmCall::default()
        };
        let code = unsafe { import_read_number(CTX.load(Ordering::Relaxed), what, &mut call) };
        (code == super::OK).then_some(out)
    }

    pub fn raw_read_text(what: u32, a: &str, b: &str) -> Option<Vec<u8>> {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let mut buffer = std::vec![0u8; 128];
        let mut needed: u32 = 0;
        let mut call = WasmCall {
            a_ptr: ptr_of(a),
            a_len: a.len() as u32,
            b_ptr: ptr_of(b),
            b_len: b.len() as u32,
            out: ptr_of(&buffer),
            out_cap: buffer.len() as u32,
            out_len: (&mut needed as *mut u32) as usize as u32,
            ..WasmCall::default()
        };
        let mut code = unsafe { import_read_text(CTX.load(Ordering::Relaxed), what, &mut call) };
        if code == super::TOO_LONG {
            buffer.resize(needed as usize, 0);
            call.out = ptr_of(&buffer);
            call.out_cap = buffer.len() as u32;
            code = unsafe { import_read_text(CTX.load(Ordering::Relaxed), what, &mut call) };
        }
        (code == super::OK).then(|| {
            buffer.truncate(needed as usize);
            buffer
        })
    }

    pub fn raw_act(what: u32, a: &str, b: &str, c: &str, numbers: &[f64]) {
        let (a, b, c) = (a.as_bytes(), b.as_bytes(), c.as_bytes());
        let mut call = WasmCall {
            a_ptr: ptr_of(a),
            a_len: a.len() as u32,
            b_ptr: ptr_of(b),
            b_len: b.len() as u32,
            c_ptr: ptr_of(c),
            c_len: c.len() as u32,
            numbers: numbers.as_ptr() as usize as u32,
            count: numbers.len() as u32,
            ..WasmCall::default()
        };
        unsafe { import_act(CTX.load(Ordering::Relaxed), what, &mut call) };
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    /// Host calls only run in the wasm sandbox; a native build of a guest is
    /// for checking and testing its pure logic.
    fn unsupported() -> ! {
        panic!("blockloom-script-guest host calls only run in the wasm sandbox")
    }

    pub fn enter(_ctx: i32) {}

    pub fn raw_read_number(_what: u32, _a: &str, _b: &str, _arg: f64) -> Option<f64> {
        unsupported()
    }

    pub fn raw_read_text(_what: u32, _a: &str, _b: &str) -> Option<Vec<u8>> {
        unsupported()
    }

    pub fn raw_act(_what: u32, _a: &str, _b: &str, _c: &str, _numbers: &[f64]) {
        unsupported()
    }
}

/// Notes the entry call now running. The `export_script!` entry points call
/// this; guest code never does.
#[doc(hidden)]
pub fn __private_enter(ctx: i32) {
    imp::enter(ctx);
}

/// One numeric read. `None` when the host has no such reading.
pub fn read_number(what: u32, a: &str, b: &str, arg: f64) -> Option<f64> {
    imp::raw_read_number(what, a, b, arg)
}

/// One text read. `None` when the host has no such reading.
pub fn read_text(what: u32, a: &str, b: &str) -> Option<String> {
    imp::raw_read_text(what, a, b).and_then(|bytes| String::from_utf8(bytes).ok())
}

/// One world write. It applies later, as the same effect the blocks produce.
pub fn act(what: u32, a: &str, b: &str, c: &str, numbers: &[f64]) {
    imp::raw_act(what, a, b, c, numbers)
}

/// A line for the run log, without saying anything in the world.
pub fn log(text: &str) {
    act(ACT_LOG, text, "", "", &[]);
}

/// Shows a speech bubble over this actor; empty clears it.
pub fn say(text: &str) {
    act(ACT_SAY, text, "", "", &[]);
}

/// Moves this actor along one axis by world units: 0 is x, 1 is y, 2 is z.
pub fn change_position(axis: u32, by: f32) {
    act(ACT_CHANGE_POSITION, "", "", "", &[axis as f64, by as f64]);
}

/// Which part of a touch an event contact is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactPhase {
    Enter,
    Stay,
    Exit,
}

impl ContactPhase {
    /// The `n0` the host passes for a contact: 0 is enter, 1 stay, else exit.
    pub fn of(n0: f64) -> ContactPhase {
        match n0 as u32 {
            0 => ContactPhase::Enter,
            1 => ContactPhase::Stay,
            _ => ContactPhase::Exit,
        }
    }
}

/// What the host called `event` with: its numeric kind, its four numbers, and
/// its words. Decode by kind, the way the prelude's `Event::from_raw` does:
/// `subject` is the message, key, room or weather name; `detail` is the
/// collision id, the changed value, or the plugin's slot texts.
pub struct Event {
    pub kind: u32,
    pub numbers: [f64; 4],
    pub subject: String,
    pub detail: String,
}

impl Event {
    /// Reads the words the host holds for the event now running.
    pub fn current(kind: u32, numbers: [f64; 4]) -> Event {
        let word = |what: &str| read_text(TEXT_EVENT, what, "").unwrap_or_default();
        Event {
            kind,
            numbers,
            subject: word(""),
            detail: word("detail"),
        }
    }
}

/// A point in world units: pixels in 2D, metres in 3D.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// This actor's world transform in one host call, without allocating.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: f32,
}

/// Why a typed read failed. Numbers fall back (usually to zero); texts with
/// a fallback never surface this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadError {
    Missing,
    TooLong,
}

/// Snapshot reads and self motion: the `lifecycle` interface's typed home.
pub mod lifecycle {
    use super::{POSE_BYTES, Pose, ReadError, Vec3};

    /// This actor's pose, or another actor's by name or id. Empty names this
    /// actor. Seven little-endian `f32`s over `BYTES_POSE`.
    pub fn pose(target: &str) -> Result<Pose, ReadError> {
        let bytes =
            super::imp::raw_read_text(super::BYTES_POSE, target, "").ok_or(ReadError::Missing)?;
        if bytes.len() != POSE_BYTES {
            return Err(ReadError::TooLong);
        }
        let mut floats = [0.0f32; 7];
        for (chunk, out) in bytes.as_chunks::<4>().0.iter().zip(floats.iter_mut()) {
            *out = f32::from_le_bytes(*chunk);
        }
        Ok(Pose {
            position: Vec3 {
                x: floats[0],
                y: floats[1],
                z: floats[2],
            },
            rotation: Vec3 {
                x: floats[3],
                y: floats[4],
                z: floats[5],
            },
            scale: floats[6],
        })
    }

    fn axis_at(axis: u32) -> f32 {
        super::read_number(super::READ_POSITION, "", "", axis as f64).unwrap_or(0.0) as f32
    }

    /// This actor's position on one axis: 0 is x, 1 is y, 2 is z.
    pub fn position(axis: u32) -> f32 {
        axis_at(axis)
    }

    /// This actor's x, y and z.
    pub fn x() -> f32 {
        axis_at(0)
    }

    /// This actor's x, y and z.
    pub fn y() -> f32 {
        axis_at(1)
    }

    /// This actor's x, y and z.
    pub fn z() -> f32 {
        axis_at(2)
    }

    /// Shows a speech bubble over this actor; empty clears it.
    pub fn say(text: &str) {
        super::say(text)
    }

    /// Moves this actor along one axis by world units: 0 is x, 1 is y, 2 is z.
    pub fn change_position(axis: u32, by: f32) {
        super::change_position(axis, by)
    }
}

/// Host-owned per-actor storage: immediate writes, per clone, cleared each
/// run. Read-after-write sees the write.
pub mod state {
    /// The number under `key`, or 0 when it is unset or holds text.
    pub fn get_number(key: &str) -> f64 {
        super::read_number(super::READ_DATA, key, "", 0.0).unwrap_or(0.0)
    }

    /// The text under `key`, or empty when it is unset or holds a number.
    pub fn get_text(key: &str) -> String {
        super::read_text(super::TEXT_DATA, key, "").unwrap_or_default()
    }

    /// Whether `key` holds anything at all, of either kind.
    pub fn has(key: &str) -> bool {
        super::read_number(super::READ_DATA, key, "", 1.0).unwrap_or(0.0) != 0.0
    }

    /// Stores `value` under `key`, replacing whatever it held.
    pub fn set_number(key: &str, value: f64) {
        super::act(super::ACT_SET_DATA, key, "", "", &[value]);
    }

    /// Stores `value` under `key`, replacing whatever it held.
    pub fn set_text(key: &str, value: &str) {
        super::act(super::ACT_SET_DATA_TEXT, key, "", value, &[]);
    }

    /// Forgets `key`.
    pub fn clear(key: &str) {
        super::act(super::ACT_CLEAR_DATA, key, "", "", &[]);
    }

    /// Forgets every key.
    pub fn clear_all() {
        super::act(super::ACT_CLEAR_DATA, "", "", "", &[]);
    }
}

/// Block variables shared with canvases: this actor's slot first, then the
/// shared one. Immediate writes, so read-after-write sees them.
pub mod vars {
    /// The variable as a number. Unknown names read as 0.
    pub fn get_number(name: &str) -> f64 {
        super::read_number(super::READ_VARIABLE, name, "", 0.0).unwrap_or(0.0)
    }

    /// The variable as text. Unknown names read as `"0"`, the way a canvas
    /// reporter shows one.
    pub fn get_text(name: &str) -> String {
        super::read_text(super::TEXT_VARIABLE, name, "").unwrap_or_else(|| "0".to_string())
    }

    /// Writes the block variable `name`.
    pub fn set_number(name: &str, value: f64) {
        super::act(super::ACT_SET_VARIABLE, name, "", "", &[value]);
    }

    /// Writes the block variable `name` as text.
    pub fn set_text(name: &str, value: &str) {
        super::act(super::ACT_SET_VARIABLE_TEXT, name, "", value, &[]);
    }

    /// Adds `by` to the block variable `name`.
    pub fn change(name: &str, by: f64) {
        set_number(name, get_number(name) + by);
    }
}

/// Block lists shared with canvases, 1-based like the blocks. Unknown names
/// read empty and writes to them change nothing.
pub mod lists {
    /// How many items the list holds.
    pub fn len(name: &str) -> u32 {
        super::read_number(super::READ_LIST_LENGTH, name, "", 0.0)
            .unwrap_or(0.0)
            .max(0.0) as u32
    }

    /// The 1-based item `index` as a number. Out of range answers 0.
    pub fn get_number(name: &str, index: u32) -> f64 {
        super::read_number(super::READ_LIST_ITEM, name, "", f64::from(index)).unwrap_or(0.0)
    }

    /// The 1-based item `index` as text. Out of range answers empty.
    pub fn get_text(name: &str, index: u32) -> String {
        super::read_text(super::TEXT_LIST_ITEM, name, &index.to_string()).unwrap_or_default()
    }

    /// Appends a number.
    pub fn add_number(name: &str, value: f64) {
        super::act(super::ACT_LIST_ADD, name, "", "", &[value]);
    }

    /// Appends text.
    pub fn add_text(name: &str, value: &str) {
        super::act(super::ACT_LIST_ADD_TEXT, name, "", value, &[]);
    }

    /// Inserts a number at the 1-based `index`, allowing one past the end.
    pub fn insert_number(name: &str, index: u32, value: f64) {
        super::act(
            super::ACT_LIST_INSERT,
            name,
            "",
            "",
            &[f64::from(index), value],
        );
    }

    /// Inserts text at the 1-based `index`, allowing one past the end.
    pub fn insert_text(name: &str, index: u32, value: &str) {
        super::act(
            super::ACT_LIST_INSERT_TEXT,
            name,
            "",
            value,
            &[f64::from(index)],
        );
    }

    /// Replaces the 1-based item `index` with a number.
    pub fn replace_number(name: &str, index: u32, value: f64) {
        super::act(
            super::ACT_LIST_REPLACE,
            name,
            "",
            "",
            &[f64::from(index), value],
        );
    }

    /// Replaces the 1-based item `index` with text.
    pub fn replace_text(name: &str, index: u32, value: &str) {
        super::act(
            super::ACT_LIST_REPLACE_TEXT,
            name,
            "",
            value,
            &[f64::from(index)],
        );
    }

    /// Deletes the 1-based item `index`.
    pub fn delete(name: &str, index: u32) {
        super::act(super::ACT_LIST_DELETE, name, "", "", &[f64::from(index)]);
    }

    /// Empties the list.
    pub fn clear(name: &str) {
        super::act(super::ACT_LIST_CLEAR, name, "", "", &[]);
    }
}

/// Queries, the character controller and one-step forces.
pub mod physics {
    /// Asks the live world on the spot as this actor and files the answer as
    /// its query result. Returns how many hits the query found.
    pub fn query(kind: &str, triggers: &str, layers: u32, numbers: &[f64]) -> u32 {
        let mut all = Vec::with_capacity(numbers.len() + 1);
        all.push(f64::from(layers));
        all.extend_from_slice(numbers);
        super::act(super::ACT_PHYSICS_QUERY, kind, triggers, "", &all);
        super::read_number(super::READ_QUERY, "count", "", 1.0).unwrap_or(0.0) as u32
    }

    /// A number from the `hit`th (from 1) hit of the last query: `x`, `y`,
    /// `z`, `distance`, `fraction`, `count`, `overflowed` or `tick`.
    pub fn query_number(field: &str, hit: u32) -> f64 {
        super::read_number(super::READ_QUERY, field, "", f64::from(hit)).unwrap_or(0.0)
    }

    /// Words from a hit: `actor`, `actor id`, `body`, `collider`, or `error`.
    pub fn query_text(field: &str, hit: u32) -> Option<String> {
        super::read_text(super::TEXT_QUERY, field, &hit.to_string())
    }

    /// Runs one controller op (`move`, `simple move`, `motor ...`, `joint ...`
    /// or `set ...`) with its numbers.
    pub fn controller(op: &str, numbers: &[f64]) {
        super::act(super::ACT_CONTROLLER, op, "", "", numbers);
    }

    /// Numbers from the last controller move: `grounded`, `flags`, `moved x`,
    /// `normal y`, joint `position`, `speed`, `force`, `torque`, ...
    pub fn controller_number(field: &str, index: u32) -> f64 {
        super::read_number(super::READ_CONTROLLER, field, "", f64::from(index)).unwrap_or(0.0)
    }

    /// Words from the last controller move: the `index`th obstacle's `actor`,
    /// `body` or `collider`, or the `error` that stopped it.
    pub fn controller_text(field: &str, index: u32) -> Option<String> {
        super::read_text(super::TEXT_CONTROLLER, field, &index.to_string())
    }

    /// Pushes this actor for one fixed step: `mode` names the force mode,
    /// `torque` turns instead of pushing, `vector` is the force.
    pub fn add_force(mode: &str, torque: bool, x: f32, y: f32, z: f32) {
        let b = if torque { "torque" } else { "" };
        super::act(
            super::ACT_ADD_FORCE,
            mode,
            b,
            "",
            &[f64::from(x), f64::from(y), f64::from(z)],
        );
    }
}

/// One slot value of a plugin block: what `plugins::call` and the plugin
/// readers take.
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
        PluginArg::Number(f64::from(value))
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

/// Slot values as a JSON array, a whole number as an integer: the shape the
/// plugin readers and calls carry them in.
pub fn slots_json(args: &[PluginArg]) -> String {
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
                        ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
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

/// Plugin reporters and block calls, answered only while the game runs.
/// The separator between the block id and its slots is U+001F.
pub mod plugins {
    /// The `slots`th answer of the `block` reporter as a number.
    pub fn read_number(plugin: &str, block: &str, slots: &str) -> Option<f64> {
        let asked = format!("{block}\u{1f}{slots}");
        super::read_number(super::READ_PLUGIN, plugin, &asked, 0.0)
    }

    /// The `slots`th answer of the `block` reporter as text.
    pub fn read_text(plugin: &str, block: &str, slots: &str) -> Option<String> {
        let asked = format!("{block}\u{1f}{slots}");
        super::read_text(super::TEXT_PLUGIN, plugin, &asked)
    }

    /// Runs the `block` command with `slots` as its JSON array.
    pub fn call(plugin: &str, block: &str, slots: &str) {
        super::act(super::ACT_PLUGIN_CALL, plugin, block, slots, &[]);
    }

    /// Runs the `block` command with typed slot values.
    pub fn call_args(plugin: &str, block: &str, args: &[super::PluginArg]) {
        call(plugin, block, &super::slots_json(args));
    }
}

/// The long tail of numeric reads, under the kebab-case of their ABI names.
pub mod sensors {
    /// One numeric read: `what` is a `READ_*` verb, `a` and `b` name things,
    /// `arg` carries an axis or another plain number.
    pub fn read(what: u32, a: &str, b: &str, arg: f64) -> Option<f64> {
        super::read_number(what, a, b, arg)
    }
}

/// The long tail of text reads, under the kebab-case of their ABI names.
pub mod texts {
    /// One text read: `what` is a `TEXT_*` verb.
    pub fn read_text(what: u32, a: &str, b: &str) -> Option<String> {
        super::read_text(what, a, b)
    }
}

/// Save slots: which file this run's variables persist to. Rides `acts.*`
/// and `texts.*`, like the rest of the long tail.
pub mod saves {
    /// The save slot this run writes to, by name. What `save slot` reports.
    pub fn save_slot() -> String {
        super::read_text(super::TEXT_SAVE_SLOT, "", "")
            .unwrap_or_else(|| super::DEFAULT_SAVE_SLOT.to_string())
    }

    /// Every slot with a file on disk as a JSON list, default first.
    pub fn save_slots() -> String {
        super::read_text(super::TEXT_SAVE_SLOTS, "", "").unwrap_or_else(|| "[]".to_string())
    }

    /// Switches which save slot this run writes to and loads that slot's
    /// saved variables into the run; a name with no file yet starts fresh.
    pub fn switch_save_slot(slot: &str) {
        super::act(super::ACT_SWITCH_SAVE_SLOT, slot, "", "", &[]);
    }

    /// Deletes one save slot's file without touching the live run.
    pub fn delete_save_slot(slot: &str) {
        super::act(super::ACT_DELETE_SAVE_SLOT, slot, "", "", &[]);
    }
}

/// Interface strings: what the run says, in the language it speaks. Rides
/// `acts.*` and `texts.*`, like the rest of the long tail.
pub mod locale {
    /// The language this run speaks, lowercased. What `language` reports.
    pub fn language() -> String {
        super::read_text(super::TEXT_LANGUAGE, "", "")
            .unwrap_or_else(|| super::DEFAULT_LANGUAGE.to_string())
    }

    /// The text for `key` in the run's language, falling back to the
    /// default language and then to the key itself.
    pub fn text_for(key: &str) -> String {
        super::read_text(super::TEXT_LOCALE_TEXT, key, "").unwrap_or_else(|| key.to_string())
    }

    /// Speaks the run's language for the rest of the run: what `text for`
    /// answers in. Empty reads as the project's default language.
    pub fn set_language(language: &str) {
        super::act(super::ACT_SET_LANGUAGE, language, "", "", &[]);
    }
}

/// The long tail of world writes, under the kebab-case of their ABI names.
pub mod acts {
    /// One world write: `what` is an `ACT_*` verb.
    pub fn act(what: u32, a: &str, b: &str, c: &str, numbers: &[f64]) {
        super::act(what, a, b, c, numbers)
    }
}

/// Names the entry points: `start`, `tick`, `frame`, `ui`, `stop`, `destroy`
/// and `event`, each optional. Missing ones behave as absent, as for prelude
/// scripts.
///
/// ```ignore
/// export_script!(start = start, tick = tick, event = event);
/// ```
#[macro_export]
macro_rules! export_script {
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path]) => {
        $crate::export_script!(@emit $start, $tick, $frame, $ui, $stop, $destroy, $event);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] start = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$value, $tick, $frame, $ui, $stop, $destroy, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] tick = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $value, $frame, $ui, $stop, $destroy, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] frame = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $tick, $value, $ui, $stop, $destroy, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] ui = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $tick, $frame, $value, $stop, $destroy, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] stop = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $tick, $frame, $ui, $value, $destroy, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] destroy = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $tick, $frame, $ui, $stop, $value, $event] $($($rest)*)?);
    };
    (@take [$start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path] event = $value:path $(, $($rest:tt)*)?) => {
        $crate::export_script!(@take [$start, $tick, $frame, $ui, $stop, $destroy, $value] $($($rest)*)?);
    };
    (@emit $start:path, $tick:path, $frame:path, $ui:path, $stop:path, $destroy:path, $event:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_abi() -> u32 {
            $crate::ABI_VERSION
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_start(ctx: i32, _api: i32) {
            $crate::__private_enter(ctx);
            $start();
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_tick(ctx: i32, _api: i32, dt: f32) {
            $crate::__private_enter(ctx);
            $tick(dt);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_frame(ctx: i32, _api: i32, dt: f32) {
            $crate::__private_enter(ctx);
            $frame(dt);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_ui(ctx: i32, _api: i32, dt: f32) {
            $crate::__private_enter(ctx);
            $ui(dt);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_stop(ctx: i32, _api: i32) {
            $crate::__private_enter(ctx);
            $stop();
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_destroy(ctx: i32, _api: i32) {
            $crate::__private_enter(ctx);
            $destroy();
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn blockloom_script_event(
            ctx: i32,
            _api: i32,
            kind: u32,
            n0: f64,
            n1: f64,
            n2: f64,
            n3: f64,
        ) {
            $crate::__private_enter(ctx);
            $event(&$crate::Event::current(kind, [n0, n1, n2, n3]));
        }
    };
    ($($rest:tt)*) => {
        $crate::export_script!(@take [$crate::__no_start, $crate::__no_tick, $crate::__no_frame, $crate::__no_ui, $crate::__no_stop, $crate::__no_destroy, $crate::__no_event] $($rest)*);
    };
}

/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_start() {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_tick(_dt: f32) {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_frame(_dt: f32) {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_ui(_dt: f32) {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_stop() {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_destroy() {}
/// Defaults for entry points the guest leaves out.
#[doc(hidden)]
pub fn __no_event(_event: &Event) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_struct_matches_the_host_layout() {
        assert_eq!(std::mem::size_of::<WasmCall>(), 56);
        assert_eq!(std::mem::offset_of!(WasmCall, a_ptr), 0);
        assert_eq!(std::mem::offset_of!(WasmCall, numbers), 24);
        assert_eq!(std::mem::offset_of!(WasmCall, arg), 32);
        assert_eq!(std::mem::offset_of!(WasmCall, out), 40);
        assert_eq!(std::mem::offset_of!(WasmCall, out_len), 48);
    }

    #[test]
    fn slot_values_encode_the_way_plugin_calls_carry_them() {
        assert_eq!(slots_json(&[]), "[]");
        assert_eq!(
            slots_json(&[1.0.into(), "hi".into(), true.into()]),
            "[1,\"hi\",true]"
        );
        assert_eq!(slots_json(&[1.5.into()]), "[1.5]");
        assert_eq!(slots_json(&[f64::NAN.into()]), "[null]");
        assert_eq!(slots_json(&["a\"b\\c".into()]), "[\"a\\\"b\\\\c\"]");
    }

    #[test]
    fn contact_phases_follow_the_host_numbers() {
        assert_eq!(ContactPhase::of(0.0), ContactPhase::Enter);
        assert_eq!(ContactPhase::of(1.0), ContactPhase::Stay);
        assert_eq!(ContactPhase::of(2.0), ContactPhase::Exit);
    }
}
