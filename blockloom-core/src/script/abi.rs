// The C boundary between the runtime and a compiled script. This file is
// compiled twice: once into `blockloom-core` (the host's view) and once, as
// text, into the `blockloom` crate a script links against. Editing it changes
// both halves at once, which is the point - and [`ABI_VERSION`] must go up
// whenever it does, since a script built against an older one is still sitting
// in somebody's project folder.

use std::ffi::c_void;

/// Bumped whenever anything in this file changes shape. The host refuses a
/// library that reports a different one rather than calling into it.
pub const ABI_VERSION: u32 = 31;

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
/// `arg` = axis. Where this actor stands in its parent's frame - the world
/// position itself when it hangs off nothing.
pub const READ_LOCAL_POSITION: u32 = 24;
/// `a` = another actor's name, `arg` = axis. Its place in its own parent's
/// frame, likewise the world position when it hangs off nothing.
pub const READ_LOCAL_POSITION_OF: u32 = 25;
/// `a` = actor name, empty for this actor. Whether its collider only senses.
pub const READ_IS_TRIGGER: u32 = 26;
/// `a` = actor name, empty for this actor. Which layer it lives on, 1-8.
pub const READ_COLLISION_LAYER: u32 = 27;
/// `a` = "x1 y1 z1", `b` = "x2 y2 z2". How far along the segment the first
/// body sits, or [`MISSING`] for nothing. Z is ignored in a 2D project.
pub const READ_RAY_DISTANCE: u32 = 28;
/// `a` = "x y z", `b` = radius. Nearest body a ball overlaps, by name.
pub const READ_CIRCLE_HIT_OF: u32 = 29;
/// `a` = action name. Whether it is held right now.
pub const READ_ACTION_DOWN: u32 = 30;
/// `a` = action name. True only on the frame it went down.
pub const READ_ACTION_PRESSED: u32 = 31;
/// `a` = action name. True only on the frame it went up.
pub const READ_ACTION_RELEASED: u32 = 32;
/// `a` = action name. The strongest binding's analog value.
pub const READ_ACTION_VALUE: u32 = 33;
/// How many fingers are down.
pub const READ_TOUCH_COUNT: u32 = 34;
/// `arg` = 1-based touch index. Its world position on one axis.
pub const READ_TOUCH: u32 = 35;
/// Whether at least one gamepad is connected.
pub const READ_GAMEPAD_CONNECTED: u32 = 36;
/// `a` = axis name. A live stick or trigger value, -1..1.
pub const READ_GAMEPAD_AXIS: u32 = 37;
/// `a` = button name. Whether that pad button is held.
pub const READ_GAMEPAD_BUTTON: u32 = 38;
/// `a` = `left`, `right` or `middle`. Whether that mouse button is held.
pub const READ_MOUSE_BUTTON: u32 = 39;
/// `a` = a reading's name (`wind speed`, `rain`, ...). The air as of this
/// fixed tick, the same slot the atmosphere reporter reads.
pub const READ_ATMOSPHERE: u32 = 40;
/// Whether any tween (a glide or a `tween ...` block) is still moving this
/// actor. What `is tweening?` answers.
pub const READ_IS_TWEENING: u32 = 41;
/// The 1-based flipbook frame showing right now. Zero with no clip.
pub const READ_ANIM_FRAME: u32 = 42;
/// Whether the animation player's clip is still advancing.
pub const READ_ANIM_PLAYING: u32 = 43;
/// `a` = actor name, empty for this actor. Whether it carries a light that
/// casts shadows right now.
pub const READ_CASTS_SHADOWS: u32 = 44;
/// `a` = "x z", `b` = what to read there: `height`, `normal x|y|z`,
/// `velocity x|y|z` or `foam` (how pinched the crest is, 0-1). The water as
/// of this fixed tick; [`MISSING`] over dry land.
pub const READ_WATER: u32 = 45;
/// `a` = actor name, empty for this actor. Whether it is below a water
/// surface and above that body's bottom.
pub const READ_UNDERWATER: u32 = 46;
/// `a` = what to read of this actor's particles: `alive`, a count of this
/// frame's `spawn`, `die` or `collide` events, or where the last one was
/// (`collide x`, `spawn z`, ...; this actor's position before any).
pub const READ_PARTICLES: u32 = 47;
/// `a` = "x y" or "x y z" (world), `b` = tilemap name, empty for any. The
/// sheet index there, -1 for an empty cell or no map; [`MISSING`] for an
/// unknown map.
pub const READ_TILE_AT: u32 = 48;

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
/// `a` = "x1 y1 z1", `b` = "x2 y2 z2". The first body a segment hits, by
/// name, or [`MISSING`] for nothing.
pub const TEXT_RAY_HIT: u32 = 9;
/// `a` = "x y z", `b` = radius. The nearest body a ball overlaps, by name.
pub const TEXT_CIRCLE_HIT: u32 = 10;
/// The clip the animation player is holding, or [`MISSING`] for none.
pub const TEXT_CURRENT_CLIP: u32 = 11;
/// The environment volumes showing at the camera, as a JSON list of names
/// in blend order. What `active volumes` reports.
pub const TEXT_ACTIVE_VOLUMES: u32 = 12;
/// Only answers inside the event entry point. `a` empty = the event's
/// subject (a message, key, actor, clip, element ...), `a` = `detail` = its
/// second word where it has one (the other actor's id, a changed value).
pub const TEXT_EVENT: u32 = 13;
/// `a` = actor name, empty for this actor. The smallest room it stands in,
/// by name, or [`MISSING`] for none.
pub const TEXT_ROOM: u32 = 14;
/// The room this actor entered on the last fixed tick, by name, or
/// [`MISSING`]. What `when I enter room` would have started on.
pub const TEXT_ENTERED_ROOM: u32 = 15;

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
/// `n0` != 0 senses overlap without pushing back.
pub const ACT_SET_TRIGGER: u32 = 43;
/// `n0` = layer 1-8.
pub const ACT_SET_COLLISION_LAYER: u32 = 44;
/// `n0` = bitmask of the layers the actor pairs with.
pub const ACT_SET_COLLISION_MASK: u32 = 45;
/// `n0` = strength 0-100, `n1` = seconds. Rumbles every connected gamepad;
/// zero of either stops instead.
pub const ACT_RUMBLE_GAMEPAD: u32 = 46;
/// `a` = action, `b` = binding text. Adds one binding for the rest of the run.
pub const ACT_BIND_ACTION: u32 = 47;
/// `a` = action. Forgets every binding for the rest of the run.
pub const ACT_CLEAR_ACTION_BINDINGS: u32 = 48;
/// `n0` = EV100, lower is brighter. Holds for the rest of the run and
/// outranks auto-exposure.
pub const ACT_SET_EXPOSURE: u32 = 49;
/// `n0` = lumens. Sets this actor's Light component.
pub const ACT_SET_LIGHT_INTENSITY: u32 = 50;
/// `n0` = factor, `n1` = seconds; `b` = easing name. Eased like the block.
pub const ACT_TWEEN_SCALE: u32 = 51;
/// `n0` = axis, `n1` = degrees, `n2` = seconds; `b` = easing name.
pub const ACT_TWEEN_ROTATION: u32 = 52;
/// `a` = `#RRGGBB`, `b` = easing name; `n0` = seconds.
pub const ACT_TWEEN_COLOR: u32 = 53;
/// No numbers: stops every tween on the actor where it stands.
pub const ACT_STOP_TWEENS: u32 = 54;
/// `a` = clip name; `n0` = speed.
pub const ACT_PLAY_ANIMATION: u32 = 55;
/// No numbers: stops the animation player, keeping the frame.
pub const ACT_STOP_ANIMATION: u32 = 56;
/// `n0` = speed. 1 is as authored, 0 freezes.
pub const ACT_SET_ANIMATION_SPEED: u32 = 57;
/// `n0` = multiple of this actor's emissive tint.
pub const ACT_SET_EMISSIVE_STRENGTH: u32 = 58;
/// `n0` != 0 turns HDR output on where the display offers it.
pub const ACT_SET_HDR_OUTPUT: u32 = 59;
/// `n0` = the display's peak brightness in nits.
pub const ACT_SET_PEAK_BRIGHTNESS: u32 = 60;
/// `a` = volume by id or name, empty for this actor; `n0` != 0 turns it on.
pub const ACT_ENABLE_VOLUME: u32 = 61;
/// `a` = volume by id or name, empty for this actor; `n0` = weight 0-1.
pub const ACT_SET_VOLUME_WEIGHT: u32 = 62;
/// Re-captures every light probe where it stands. Window-global.
pub const ACT_CAPTURE_PROBES: u32 = 63;
/// `n0` = metres the sun's shadows reach. Window-global.
pub const ACT_SET_SHADOW_DISTANCE: u32 = 64;
/// `n0` != 0 turns this actor's light's shadows on.
pub const ACT_SET_LIGHT_SHADOWS: u32 = 65;
/// `n0` != 0 turns ray-traced lighting on where the GPU can trace rays.
/// Window-global.
pub const ACT_SET_RAY_TRACING: u32 = 66;
/// `n0` = most bounces a traced light path takes. Window-global.
pub const ACT_SET_GI_BOUNCES: u32 = 67;
/// `n0` = light samples per pixel when tracing. Window-global.
pub const ACT_SET_GI_SAMPLES: u32 = 68;
/// `n0` = height fog's extinction per metre; 0 clears it. Window-global.
pub const ACT_SET_FOG_DENSITY: u32 = 69;
/// `n0` = the aurora's KP index, 0-9. Window-global.
pub const ACT_SET_AURORA: u32 = 70;
/// `n0..n2` = where a lightning strike lands. Window-global.
pub const ACT_STRIKE_LIGHTNING: u32 = 71;
/// `n0` = strikes a minute the storm throws. Window-global.
pub const ACT_SET_LIGHTNING_RATE: u32 = 72;
/// `a` = wind dial (`direction`, `speed`, `gust` or `storm`), `n0` = value.
/// Window-global.
pub const ACT_SET_WIND: u32 = 73;
/// `n0..n2` = extra cloud drift per second. Window-global.
pub const ACT_SET_CLOUD_DRIFT: u32 = 74;
/// `a` = cloud dial (`coverage`, `density` or `type`), `n0` = value.
/// Window-global.
pub const ACT_SET_CLOUDS: u32 = 75;
/// `a` = cloud layer dial (`coverage`, `opacity`, `contrast`, `altitude` or
/// `spin`), `n0` = layer from 1, `n1` = value. Window-global.
pub const ACT_SET_CLOUD_LAYER: u32 = 76;
/// `a` = water dial (`level`, `chop` or `foam`), `n0` = value. This actor's
/// own water when it has some, every body's otherwise.
pub const ACT_SET_WATER: u32 = 77;
/// `a` = trigger name for the animation state machine, this tick only.
pub const ACT_FIRE_ANIMATION_TRIGGER: u32 = 78;
/// `a` = rig slot, `b` = attachment; empty hides the slot.
pub const ACT_SET_RIG_SLOT: u32 = 79;
/// `a` = rig slot, `b` = `#RRGGBB`.
pub const ACT_SET_SLOT_TINT: u32 = 80;
/// `a` = IK constraint; `n0`, `n1` = where, relative to the actor.
pub const ACT_SET_IK_TARGET: u32 = 81;
/// `a` = sprite dial (`FlipX`, `FlipY`, `Order`, `YSort`, `Palette`,
/// `OutlineWidth`); `n0` = value.
pub const ACT_SET_SPRITE_DIAL: u32 = 82;
/// `n0` = how many particles this actor's emitter bursts out now.
pub const ACT_BURST_PARTICLES: u32 = 83;
/// `a` = emitter dial (`rate`, `lifetime`, `speed`, `spread`, `gravity`,
/// `size start`, `size end` or `max`), `n0` = value, for the run.
pub const ACT_SET_EMITTER_DIAL: u32 = 84;
/// `n0` = nonzero to keep emitting, zero to stop (live particles finish).
pub const ACT_SET_EMITTER_PLAYING: u32 = 85;
/// `a` = tilemap actor (empty for this actor, or whichever map covers the
/// point); `n0` = tile (-1 erases), `n1`, `n2`, `n3` = world x, y, z.
pub const ACT_PAINT_TILE: u32 = 86;
/// `a` = parallax layer actor, `b` = axis (`both`, `x` or `y`); `n0` = scroll
/// factor 0-2.
pub const ACT_SET_PARALLAX: u32 = 87;

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
/// `(ctx, api, kind, n0, n1, n2, n3)`: one [`EVENT_MESSAGE`]-style kind, its
/// numbers, and its words through [`TEXT_EVENT`].
pub const SYM_EVENT: &[u8] = b"blockloom_script_event";

// ─── What the event entry point is called with ─────────────────────────────
// Global events reach every script; the rest only the actor they name.

/// A broadcast. Subject: the message.
pub const EVENT_MESSAGE: u32 = 1;
/// A key went down. Subject: the key, as `key down?` spells it.
pub const EVENT_KEY: u32 = 2;
/// An input action went down. Subject: the action.
pub const EVENT_ACTION: u32 = 3;
/// This actor was clicked.
pub const EVENT_CLICKED: u32 = 4;
/// A finger touched the screen.
pub const EVENT_TOUCHED: u32 = 5;
/// This actor started touching another. Subject: its name; detail: its id.
pub const EVENT_COLLISION: u32 = 6;
/// This actor's particles spawned, died or collided this frame. Subject:
/// `spawn`, `die` or `collide`; `n0` = how many, `n1..n3` = where the last was.
pub const EVENT_PARTICLES: u32 = 7;
/// A `Once` clip ended on this actor. Subject: the clip.
pub const EVENT_ANIMATION_ENDED: u32 = 8;
/// This actor's clip reached a marker. Subject: the marker.
pub const EVENT_ANIMATION_MARKER: u32 = 9;
/// An interface element was clicked. Subject: its id.
pub const EVENT_UI_CLICKED: u32 = 10;
/// An input element changed. Subject: its id; detail: its value as text.
pub const EVENT_UI_CHANGED: u32 = 11;
/// Any other interface event. Subject: the element's id; detail: the event.
pub const EVENT_UI: u32 = 12;
/// This actor walked into a room. Subject: the room's name.
pub const EVENT_ENTERED_ROOM: u32 = 13;

// ─── The same three calls in a browser ─────────────────────────────────────
// A web build loads each script as its own wasm module, and one module can't
// call another through a function pointer. So there the script imports the
// three calls from the [`WASM_MODULE`] module instead, each taking the
// context, the verb and a pointer to a [`WasmCall`] in the script's own
// memory, which the host reads and answers into. The entry points are
// called with a null `HostApi`.

/// The import module a web script's three calls come from.
pub const WASM_MODULE: &str = "blockloom";
pub const WASM_READ_NUMBER: &str = "read_number";
pub const WASM_READ_TEXT: &str = "read_text";
pub const WASM_ACT: &str = "act";

/// One call's arguments, as a web script lays them out in its own memory.
/// Every pointer is an offset into that memory, and every field is a plain
/// 32-bit word or a double, so both sides agree on the layout.
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
