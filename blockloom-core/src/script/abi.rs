// The C boundary between the runtime and a compiled script. This file is
// compiled twice: once into `blockloom-core` (the host's view) and once, as
// text, into the `blockloom` crate a script links against. Editing it changes
// both halves at once, which is the point - and [`ABI_VERSION`] must go up
// whenever it does, since a script built against an older one is still sitting
// in somebody's project folder.

use std::ffi::c_void;

/// Bumped whenever anything in this file changes shape. The host refuses a
/// library that reports a different one rather than calling into it.
pub const ABI_VERSION: u32 = 1;

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
/// `a` = actor name, or empty for "anything at all".
pub const READ_TOUCHING: u32 = 9;
/// `a` = actor name, or "mouse".
pub const READ_DISTANCE_TO: u32 = 10;
/// `a` = component name.
pub const READ_HAS_COMPONENT: u32 = 11;
/// `a` = component, `b` = field.
pub const READ_FIELD: u32 = 12;

// ─── What a script can read as text ────────────────────────────────────────

pub const TEXT_ACTOR_NAME: u32 = 1;
/// `a` = component, `b` = field.
pub const TEXT_FIELD: u32 = 2;

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
    /// `(ctx, what, a, b, c, n0, n1, n2)`
    pub act: extern "C" fn(*mut c_void, u32, Str, Str, Str, f64, f64, f64),
}

/// What the host looks for in a compiled script. A library missing any of
/// these isn't one.
pub const SYM_ABI: &[u8] = b"blockloom_script_abi";
pub const SYM_START: &[u8] = b"blockloom_script_start";
pub const SYM_TICK: &[u8] = b"blockloom_script_tick";
