// The C boundary between the runtime and a compiled script. This file is
// compiled twice: once into `blockloom-core` (the host's view) and once, as
// text, into the `blockloom` crate a script links against. Editing it changes
// both halves at once, which is the point - and [`ABI_VERSION`] must go up
// whenever it does, since a script built against an older one is still sitting
// in somebody's project folder.

use std::ffi::c_void;

/// Bumped whenever anything in this file changes shape. The host refuses a
/// library that reports a different one rather than calling into it.
pub const ABI_VERSION: u32 = 11;

/// A borrowed string, as the boundary passes one. Not NUL-terminated: the
/// length is the length.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Str {
    pub ptr: *const u8,
    pub len: usize,
}

impl Str {
    pub const EMPTY: Str = Str {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn borrow(text: &str) -> Str {
        Str {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }

    /// # Safety
    /// `ptr` must be valid UTF-8 for `len` bytes, or null with a zero length.
    pub unsafe fn as_str<'a>(self) -> &'a str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        std::str::from_utf8(bytes).unwrap_or("")
    }
}

/// What a host call answered. Anything but [`OK`] leaves the out-parameter
/// untouched.
pub const OK: u32 = 0;
/// No such component, field, actor or reading.
pub const MISSING: u32 = 1;
/// The answer didn't fit in the buffer the caller offered.
pub const TOO_LONG: u32 = 2;

// ─── What a script can read ────────────────────────────────────────────────
// `a` and `b` name things; `arg` carries an axis or another plain number.

pub const READ_POSITION: u32 = 1;
pub const READ_ROTATION: u32 = 2;
pub const READ_SCALE: u32 = 3;
pub const READ_VISIBLE: u32 = 4;
pub const READ_TIMER: u32 = 5;
/// `a` = key name.
pub const READ_KEY_DOWN: u32 = 6;
/// `arg` = axis; only X and Y mean anything.
pub const READ_MOUSE: u32 = 7;
pub const READ_MOUSE_DOWN: u32 = 8;
/// `arg` = axis; raw pointer motion in pixels since the last frame.
pub const READ_MOUSE_DELTA: u32 = 16;
/// `a` = actor name, or empty for "anything at all".
pub const READ_TOUCHING: u32 = 9;
/// `a` = actor name, or "mouse".
pub const READ_DISTANCE_TO: u32 = 10;
/// `a` = component name.
pub const READ_HAS_COMPONENT: u32 = 11;
/// `a` = component, `b` = field.
pub const READ_FIELD: u32 = 12;
/// `a` = another actor's name, `arg` = axis.
pub const READ_POSITION_OF: u32 = 13;
/// Whether this actor is a clone rather than one the editor authored.
pub const READ_IS_CLONE: u32 = 14;
/// `a` = actor name, or empty for every actor; counts clones too.
pub const READ_ACTOR_COUNT: u32 = 15;
/// Whether the pointer is grabbed and hidden for first-person play.
pub const READ_MOUSE_LOCKED: u32 = 17;
/// `a` = an interface element's id. A slider's number, or a toggle as 0/1.
/// An id nothing answers to is [`MISSING`].
pub const READ_UI_VALUE: u32 = 18;
/// Whether `pause game` has the world frozen right now.
pub const READ_GAME_PAUSED: u32 = 19;
/// `a` = an interface element's id. Whether it and everything it flows
/// inside are visible. An id nothing answers to reads as 0, not [`MISSING`].
pub const READ_UI_SHOWN: u32 = 20;
/// `a` = an interface element's id. Whether the blocks have made one by that
/// name at all - a hidden one still counts.
pub const READ_UI_EXISTS: u32 = 21;
/// `a` = an asset path. Whether any voice is playing that file right now.
pub const READ_SOUND_PLAYING: u32 = 22;
/// `a` = a bus name. Its live gain in 0-100, as the play blocks speak it.
pub const READ_BUS_VOLUME: u32 = 23;

// ─── What a script can read as text ────────────────────────────────────────

pub const TEXT_ACTOR_NAME: u32 = 1;
/// `a` = component, `b` = field.
pub const TEXT_FIELD: u32 = 2;
/// This actor's own id, which is what the acts below take.
pub const TEXT_ACTOR_ID: u32 = 3;
/// The id of the actor this one hangs off, or [`MISSING`] for none.
pub const TEXT_PARENT: u32 = 4;
/// The id of the last actor or clone this one made.
pub const TEXT_NEW_ACTOR: u32 = 5;
/// `a` = an interface element's id; a text input's typed text.
pub const TEXT_UI_VALUE: u32 = 6;
/// `a` = an interface element's id; the words it is showing, which for an
/// empty input is its placeholder.
pub const TEXT_UI_TEXT: u32 = 7;
/// The id of the text input holding the keyboard, or [`MISSING`] for none.
pub const TEXT_UI_FOCUS: u32 = 8;

// ─── What a script can do ──────────────────────────────────────────────────
// Every one of these becomes the same `vm::Effect` the blocks produce, so a
// script and a canvas can drive the same actor without knowing about each
// other.

/// `n0` = steps.
pub const ACT_MOVE: u32 = 1;
/// `n0`, `n1`, `n2` = position.
pub const ACT_GO_TO: u32 = 2;
/// `n0` = axis, `n1` = amount.
pub const ACT_CHANGE_POSITION: u32 = 3;
/// `n0` = axis, `n1` = degrees.
pub const ACT_TURN: u32 = 4;
/// `n0` = axis, `n1` = degrees.
pub const ACT_SET_ROTATION: u32 = 5;
/// `a` = actor name, or "mouse".
pub const ACT_POINT_TOWARDS: u32 = 6;
/// `n0` = factor.
pub const ACT_SET_SCALE: u32 = 7;
/// `n0`, `n1`, `n2` = impulse.
pub const ACT_APPLY_IMPULSE: u32 = 8;
/// `n0`, `n1`, `n2` = velocity.
pub const ACT_SET_VELOCITY: u32 = 9;
/// `a` = text; empty clears the bubble.
pub const ACT_SAY: u32 = 10;
/// `n0` != 0 to show.
pub const ACT_SET_VISIBLE: u32 = 11;
/// `a` = `#RRGGBB`.
pub const ACT_SET_COLOR: u32 = 12;
/// `a` = message name.
pub const ACT_BROADCAST: u32 = 13;
/// `a` = component, `b` = field, `n0` = value.
pub const ACT_SET_FIELD: u32 = 14;
/// `a` = component, `b` = field, `c` = value.
pub const ACT_SET_FIELD_TEXT: u32 = 15;
/// `a` = component name.
pub const ACT_ATTACH: u32 = 16;
/// `a` = component name.
pub const ACT_DETACH: u32 = 17;
/// `n0` = 0 follow, 1 first person, 2 third person.
pub const ACT_SET_CAMERA_VIEW: u32 = 18;
/// `a` = message for the editor's run log.
pub const ACT_LOG: u32 = 19;
pub const ACT_STOP_ALL: u32 = 20;
/// `a` = the actor to hang off, by id or name; empty takes it off.
pub const ACT_SET_PARENT: u32 = 21;
/// `a` = the actor to copy, by id or name; empty clones this one.
pub const ACT_CREATE_CLONE: u32 = 22;
/// `a` = name, `n0`, `n1`, `n2` = position.
pub const ACT_CREATE_ACTOR: u32 = 23;
/// `a` = the actor to delete, by id or name; empty deletes this one.
pub const ACT_DELETE_ACTOR: u32 = 24;
/// `n0` != 0 grabs the pointer and hides it for first-person play.
pub const ACT_SET_MOUSE_LOCKED: u32 = 25;
/// `n0` = camera pitch in degrees; positive looks up.
pub const ACT_SET_CAMERA_PITCH: u32 = 26;
/// `n0` = vertical field of view in degrees.
pub const ACT_SET_CAMERA_FOV: u32 = 36;
/// Makes an interface element or updates the one that id already names.
/// `a` = id, `b` = content, `c` = parent id, and the numbers are kind,
/// anchor, x, y, width, height, flag, min, max, value - the one act that
/// needs more than three, which is why [`HostApi::act`] takes a run of them.
pub const ACT_UI_SHOW: u32 = 27;
/// `a` = id, `b` = the property's name, `n0` = the number to write.
pub const ACT_UI_SET: u32 = 28;
/// The same, writing text: `c` = the text.
pub const ACT_UI_SET_TEXT: u32 = 29;
/// `a` = id; `n0` != 0 hides every element instead.
pub const ACT_UI_HIDE: u32 = 30;
/// `a` = id.
pub const ACT_UI_DELETE: u32 = 31;
/// `n0` != 0 freezes the world.
pub const ACT_SET_PAUSED: u32 = 32;
/// `a` = a text input's id, which is handed the keyboard. An empty id, or
/// one naming anything else, takes it back instead.
pub const ACT_UI_FOCUS: u32 = 33;
/// `n0` = 0 dark, 1 light, 2 high contrast.
pub const ACT_UI_THEME: u32 = 34;
/// `a` = variable name; `n0` != 0 clears instead of saving.
pub const ACT_SAVE_VARIABLE: u32 = 35;
/// `n0`, `n1`, `n2` = target; `n3` = speed in units per second.
pub const ACT_NAVIGATE_TO: u32 = 37;
/// `a` = asset path, `b` = bus name, `c` = followed actor id or empty for a
/// global voice; `n0` = linear gain, `n1` = pitch, `n2` != 0 loops.
pub const ACT_PLAY_SOUND: u32 = 38;
/// `a` = asset path; empty stops every voice at once.
pub const ACT_STOP_SOUND: u32 = 39;
/// `a` = asset path; `n0` = linear gain.
pub const ACT_SET_SOUND_VOLUME: u32 = 40;
/// `a` = asset path; `n0` = pitch.
pub const ACT_SET_SOUND_PITCH: u32 = 41;
/// `a` = bus name; `n0` = linear gain.
pub const ACT_SET_BUS_VOLUME: u32 = 42;

/// The three calls a script makes back into the runtime, handed to it on
/// every entry point along with an opaque context. Three instead of one per
/// verb so that adding a verb is a new constant above, not a new field here -
/// which would be an ABI break for every script already built.
#[repr(C)]
pub struct HostApi {
    pub abi: u32,
    /// `(ctx, what, a, b, arg, out) -> OK | MISSING`
    pub read_number: extern "C" fn(*mut c_void, u32, Str, Str, f64, *mut f64) -> u32,
    /// `(ctx, what, a, b, out, out_cap, out_len) -> OK | MISSING | TOO_LONG`.
    /// `out_len` is always written with the length the answer needs, so a
    /// caller handed [`TOO_LONG`] knows how big a buffer to bring back.
    pub read_text: extern "C" fn(*mut c_void, u32, Str, Str, *mut u8, usize, *mut usize) -> u32,
    /// `(ctx, what, a, b, c, numbers, count)`. A run of numbers rather than a
    /// fixed three, because one interface element names ten at once; a
    /// shorter run reads as zeros from there on.
    pub act: extern "C" fn(*mut c_void, u32, Str, Str, Str, *const f64, usize),
}

/// What the host looks for in a compiled script. A library missing any of
/// these isn't one.
pub const SYM_ABI: &[u8] = b"blockloom_script_abi";
pub const SYM_START: &[u8] = b"blockloom_script_start";
pub const SYM_TICK: &[u8] = b"blockloom_script_tick";
