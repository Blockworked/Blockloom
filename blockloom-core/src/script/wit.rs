//! Frozen script WIT world (v1): the typed contract a non-Rust script
//! compiles against, equivalent to the numeric [`super::abi`] surface.
//!
//! The numeric ABI stays the portable core-module shape (three host calls,
//! `u32` verbs, `start/tick/event` entries): every verb below names its WIT
//! path, so moving a verb to a typed function is a visible table diff, not
//! silent drift. Hot paths already have typed homes (`lifecycle.pose`,
//! `state.*`, `vars.*`, `lists.*`, `physics.*`, `plugins.*`); the long tail
//! rides its interface under the kebab-case of its ABI name, which the
//! coverage test pins verb by verb.
//!
//! Deferred-effect semantics hold across the world: snapshot reads,
//! later-applied writes, read-after-write reads old - exactly as for blocks
//! and native scripts. Batch the hot reads (`lifecycle.pose`, one event
//! struct) to amortize canonical-lift overhead; the fixed step stays
//! deterministic (same tick, same answers).

use super::abi;

/// The frozen world version. Bump with a new WIT text and a migration
/// note; guests check it the way native scripts check [`abi::ABI_VERSION`].
pub const WIT_VERSION: u32 = 1;

/// The frozen WIT world, as `wit-bindgen` consumes it. Package and version
/// are part of the freeze: renaming either is a new world, not an edit.
pub const WIT: &str = r#"
package blockloom:script@0.1.0;

/// Snapshot reads. Hot verbs have typed functions; the rest ride
/// `read` under the kebab-case of their ABI name (see READS).
interface sensors {
    read: func(what: u32, a: string, b: string, arg: f64) -> result<f64, read-error>;
    enum read-error { missing }
}

/// Text and binary reads, same shape as `sensors`.
interface texts {
    read-text: func(what: u32, a: string, b: string) -> result<string, read-error>;
    enum read-error { missing, too-long }
}

/// World writes. Each becomes the same effect the blocks produce;
/// read-after-write still reads old.
interface acts {
    act: func(what: u32, a: string, b: string, c: string, numbers: list<f64>);
}

/// Batched records and the event a hat block would start on.
interface lifecycle {
    record vec3 { x: f32, y: f32, z: f32 }
    record pose { position: vec3, rotation: vec3, scale: f32 }
    pose: func(target: string) -> result<pose, read-error>;
    pose-bytes: func(target: string) -> result<list<u8>, read-error>;
    variant event {
        message(string), key(string), action(string), clicked, touched,
        collision(tuple<string, string, contact-phase, bool, f64, f64>),
        particles(tuple<string, f64, vec3>), animation-ended(string),
        animation-marker(string), ui-clicked(string), ui-changed(tuple<string, string>),
        ui-event(tuple<string, string>), entered-room(string), scene-started,
        scene-ended, quality-dropped, weather(string), cutscene-signal(string),
        cutscene-ended(string), plugin(tuple<string, string, list<string>>),
        contact(tuple<string, string, contact-phase, bool, f64, f64>),
    }
    enum contact-phase { enter, stay, exit }
    enum read-error { missing, too-long }
    start: func();
    tick: func(dt: f32);
    frame: func(dt: f32);
    ui: func(dt: f32);
    stop: func();
    destroy: func();
    on-event: func(kind: u32, n0: f64, n1: f64, n2: f64, n3: f64);
}

/// Host-owned per-actor storage: immediate writes, per clone, cleared
/// each run. Read-after-write sees the write.
interface state {
    get-number: func(key: string) -> result<f64, read-error>;
    get-text: func(key: string) -> result<string, read-error>;
    set-number: func(key: string, value: f64);
    set-text: func(key: string, value: string);
    clear: func(key: string);
    enum read-error { missing }
}

/// Block variables shared with canvases: own value first, then shared.
interface vars {
    get-number: func(name: string) -> f64;
    get-text: func(name: string) -> string;
    set-number: func(name: string, value: f64);
    set-text: func(name: string, value: string);
}

/// Block lists shared with canvases, 1-based like the blocks.
interface lists {
    len: func(name: string) -> u32;
    get-number: func(name: string, index: u32) -> result<f64, read-error>;
    get-text: func(name: string, index: u32) -> result<string, read-error>;
    add-number: func(name: string, value: f64);
    add-text: func(name: string, value: string);
    insert-number: func(name: string, index: u32, value: f64);
    insert-text: func(name: string, index: u32, value: string);
    replace-number: func(name: string, index: u32, value: f64);
    replace-text: func(name: string, index: u32, value: string);
    delete: func(name: string, index: u32);
    clear: func(name: string);
    enum read-error { missing }
}

/// Queries, the character controller and one-step forces.
interface physics {
    query: func(kind: string, trigger-policy: string, layer-mask: u32, numbers: list<f64>);
    query-number: func(field: string, hit: u32) -> f64;
    query-text: func(field: string, hit: u32) -> result<string, read-error>;
    controller-move: func(op: string, value: list<f64>);
    controller-number: func(field: string, obstacle: u32) -> f64;
    controller-text: func(field: string, obstacle: u32) -> result<string, read-error>;
    add-force: func(mode: string, torque: bool, target: string, vector: vec3);
    use lifecycle.{vec3};
    enum read-error { missing }
}

/// Plugin reporters and block calls, answered only while the game runs.
interface plugins {
    read-number: func(plugin: string, block: string, slots: string) -> result<f64, read-error>;
    read-text: func(plugin: string, block: string, slots: string) -> result<string, read-error>;
    call: func(plugin: string, block: string, slots: string);
    enum read-error { missing }
}

/// Hat events as a typed variant; `on-event` carries the numeric kind
/// beside it until every guest binds the variant (see EVENTS).
interface event {
    use lifecycle.{event, contact-phase};
}

world script {
    import sensors;
    import texts;
    import acts;
    import lifecycle;
    import state;
    import vars;
    import lists;
    import physics;
    import plugins;
    import event;
}
"#;

/// One verb and the WIT path that carries it (`interface.func`). A path
/// outside the typed functions above rides its interface under the
/// kebab-case of its ABI name - pinned here verb by verb, so a rename or
/// a move to a typed function shows as a table diff.
pub struct WitOp {
    /// The `abi::` constant value, so the table breaks when it moves.
    pub abi: u32,
    /// The ABI name, for the coverage test's error messages.
    pub name: &'static str,
    /// `interface.func` in [`WIT`].
    pub wit: &'static str,
}

/// Every `abi::READ_*` verb and its WIT path.
pub const READS: &[WitOp] = &[
    WitOp {
        abi: abi::READ_POSITION,
        name: "READ_POSITION",
        wit: "lifecycle.pose",
    },
    WitOp {
        abi: abi::READ_ROTATION,
        name: "READ_ROTATION",
        wit: "lifecycle.pose",
    },
    WitOp {
        abi: abi::READ_SCALE,
        name: "READ_SCALE",
        wit: "lifecycle.pose",
    },
    WitOp {
        abi: abi::READ_VISIBLE,
        name: "READ_VISIBLE",
        wit: "sensors.visible",
    },
    WitOp {
        abi: abi::READ_TIMER,
        name: "READ_TIMER",
        wit: "sensors.timer",
    },
    WitOp {
        abi: abi::READ_KEY_DOWN,
        name: "READ_KEY_DOWN",
        wit: "sensors.key-down",
    },
    WitOp {
        abi: abi::READ_MOUSE,
        name: "READ_MOUSE",
        wit: "sensors.mouse",
    },
    WitOp {
        abi: abi::READ_MOUSE_DOWN,
        name: "READ_MOUSE_DOWN",
        wit: "sensors.mouse-down",
    },
    WitOp {
        abi: abi::READ_TOUCHING,
        name: "READ_TOUCHING",
        wit: "sensors.touching",
    },
    WitOp {
        abi: abi::READ_DISTANCE_TO,
        name: "READ_DISTANCE_TO",
        wit: "sensors.distance-to",
    },
    WitOp {
        abi: abi::READ_HAS_COMPONENT,
        name: "READ_HAS_COMPONENT",
        wit: "sensors.has-component",
    },
    WitOp {
        abi: abi::READ_FIELD,
        name: "READ_FIELD",
        wit: "sensors.field",
    },
    WitOp {
        abi: abi::READ_POSITION_OF,
        name: "READ_POSITION_OF",
        wit: "sensors.position-of",
    },
    WitOp {
        abi: abi::READ_IS_CLONE,
        name: "READ_IS_CLONE",
        wit: "sensors.is-clone",
    },
    WitOp {
        abi: abi::READ_ACTOR_COUNT,
        name: "READ_ACTOR_COUNT",
        wit: "sensors.actor-count",
    },
    WitOp {
        abi: abi::READ_MOUSE_DELTA,
        name: "READ_MOUSE_DELTA",
        wit: "sensors.mouse-delta",
    },
    WitOp {
        abi: abi::READ_MOUSE_LOCKED,
        name: "READ_MOUSE_LOCKED",
        wit: "sensors.mouse-locked",
    },
    WitOp {
        abi: abi::READ_UI_VALUE,
        name: "READ_UI_VALUE",
        wit: "sensors.ui-value",
    },
    WitOp {
        abi: abi::READ_GAME_PAUSED,
        name: "READ_GAME_PAUSED",
        wit: "sensors.game-paused",
    },
    WitOp {
        abi: abi::READ_UI_SHOWN,
        name: "READ_UI_SHOWN",
        wit: "sensors.ui-shown",
    },
    WitOp {
        abi: abi::READ_UI_EXISTS,
        name: "READ_UI_EXISTS",
        wit: "sensors.ui-exists",
    },
    WitOp {
        abi: abi::READ_SOUND_PLAYING,
        name: "READ_SOUND_PLAYING",
        wit: "sensors.sound-playing",
    },
    WitOp {
        abi: abi::READ_BUS_VOLUME,
        name: "READ_BUS_VOLUME",
        wit: "sensors.bus-volume",
    },
    WitOp {
        abi: abi::READ_LOCAL_POSITION,
        name: "READ_LOCAL_POSITION",
        wit: "sensors.local-position",
    },
    WitOp {
        abi: abi::READ_LOCAL_POSITION_OF,
        name: "READ_LOCAL_POSITION_OF",
        wit: "sensors.local-position-of",
    },
    WitOp {
        abi: abi::READ_IS_TRIGGER,
        name: "READ_IS_TRIGGER",
        wit: "sensors.is-trigger",
    },
    WitOp {
        abi: abi::READ_COLLISION_LAYER,
        name: "READ_COLLISION_LAYER",
        wit: "sensors.collision-layer",
    },
    WitOp {
        abi: abi::READ_RAY_DISTANCE,
        name: "READ_RAY_DISTANCE",
        wit: "sensors.ray-distance",
    },
    WitOp {
        abi: abi::READ_CIRCLE_HIT_OF,
        name: "READ_CIRCLE_HIT_OF",
        wit: "sensors.circle-hit-of",
    },
    WitOp {
        abi: abi::READ_ACTION_DOWN,
        name: "READ_ACTION_DOWN",
        wit: "sensors.action-down",
    },
    WitOp {
        abi: abi::READ_ACTION_PRESSED,
        name: "READ_ACTION_PRESSED",
        wit: "sensors.action-pressed",
    },
    WitOp {
        abi: abi::READ_ACTION_RELEASED,
        name: "READ_ACTION_RELEASED",
        wit: "sensors.action-released",
    },
    WitOp {
        abi: abi::READ_ACTION_VALUE,
        name: "READ_ACTION_VALUE",
        wit: "sensors.action-value",
    },
    WitOp {
        abi: abi::READ_TOUCH_COUNT,
        name: "READ_TOUCH_COUNT",
        wit: "sensors.touch-count",
    },
    WitOp {
        abi: abi::READ_TOUCH,
        name: "READ_TOUCH",
        wit: "sensors.touch",
    },
    WitOp {
        abi: abi::READ_GAMEPAD_CONNECTED,
        name: "READ_GAMEPAD_CONNECTED",
        wit: "sensors.gamepad-connected",
    },
    WitOp {
        abi: abi::READ_GAMEPAD_AXIS,
        name: "READ_GAMEPAD_AXIS",
        wit: "sensors.gamepad-axis",
    },
    WitOp {
        abi: abi::READ_GAMEPAD_BUTTON,
        name: "READ_GAMEPAD_BUTTON",
        wit: "sensors.gamepad-button",
    },
    WitOp {
        abi: abi::READ_MOUSE_BUTTON,
        name: "READ_MOUSE_BUTTON",
        wit: "sensors.mouse-button",
    },
    WitOp {
        abi: abi::READ_ATMOSPHERE,
        name: "READ_ATMOSPHERE",
        wit: "sensors.atmosphere",
    },
    WitOp {
        abi: abi::READ_IS_TWEENING,
        name: "READ_IS_TWEENING",
        wit: "sensors.is-tweening",
    },
    WitOp {
        abi: abi::READ_ANIM_FRAME,
        name: "READ_ANIM_FRAME",
        wit: "sensors.anim-frame",
    },
    WitOp {
        abi: abi::READ_ANIM_PLAYING,
        name: "READ_ANIM_PLAYING",
        wit: "sensors.anim-playing",
    },
    WitOp {
        abi: abi::READ_CASTS_SHADOWS,
        name: "READ_CASTS_SHADOWS",
        wit: "sensors.casts-shadows",
    },
    WitOp {
        abi: abi::READ_WATER,
        name: "READ_WATER",
        wit: "sensors.water",
    },
    WitOp {
        abi: abi::READ_UNDERWATER,
        name: "READ_UNDERWATER",
        wit: "sensors.underwater",
    },
    WitOp {
        abi: abi::READ_PARTICLES,
        name: "READ_PARTICLES",
        wit: "sensors.particles",
    },
    WitOp {
        abi: abi::READ_TILE_AT,
        name: "READ_TILE_AT",
        wit: "sensors.tile-at",
    },
    WitOp {
        abi: abi::READ_FRAME_TIME,
        name: "READ_FRAME_TIME",
        wit: "sensors.frame-time",
    },
    WitOp {
        abi: abi::READ_DRAW_CALLS,
        name: "READ_DRAW_CALLS",
        wit: "sensors.draw-calls",
    },
    WitOp {
        abi: abi::READ_DLSS_AVAILABLE,
        name: "READ_DLSS_AVAILABLE",
        wit: "sensors.dlss-available",
    },
    WitOp {
        abi: abi::READ_CUTSCENE_TIME,
        name: "READ_CUTSCENE_TIME",
        wit: "sensors.cutscene-time",
    },
    WitOp {
        abi: abi::READ_PLUGIN,
        name: "READ_PLUGIN",
        wit: "plugins.read-number",
    },
    WitOp {
        abi: abi::READ_QUERY,
        name: "READ_QUERY",
        wit: "physics.query-number",
    },
    WitOp {
        abi: abi::READ_CONTROLLER,
        name: "READ_CONTROLLER",
        wit: "physics.controller-number",
    },
    WitOp {
        abi: abi::READ_VELOCITY,
        name: "READ_VELOCITY",
        wit: "sensors.velocity",
    },
    WitOp {
        abi: abi::READ_ANGULAR_VELOCITY,
        name: "READ_ANGULAR_VELOCITY",
        wit: "sensors.angular-velocity",
    },
    WitOp {
        abi: abi::READ_MASS,
        name: "READ_MASS",
        wit: "sensors.mass",
    },
    WitOp {
        abi: abi::READ_GROUNDED,
        name: "READ_GROUNDED",
        wit: "sensors.grounded",
    },
    WitOp {
        abi: abi::READ_DATA,
        name: "READ_DATA",
        wit: "state.get-number",
    },
    WitOp {
        abi: abi::READ_VARIABLE,
        name: "READ_VARIABLE",
        wit: "vars.get-number",
    },
    WitOp {
        abi: abi::READ_LIST_LENGTH,
        name: "READ_LIST_LENGTH",
        wit: "lists.len",
    },
    WitOp {
        abi: abi::READ_LIST_ITEM,
        name: "READ_LIST_ITEM",
        wit: "lists.get-number",
    },
];

/// Every `abi::TEXT_*` verb and its WIT path.
pub const TEXTS: &[WitOp] = &[
    WitOp {
        abi: abi::TEXT_ACTOR_NAME,
        name: "TEXT_ACTOR_NAME",
        wit: "texts.actor-name",
    },
    WitOp {
        abi: abi::TEXT_FIELD,
        name: "TEXT_FIELD",
        wit: "texts.field",
    },
    WitOp {
        abi: abi::TEXT_ACTOR_ID,
        name: "TEXT_ACTOR_ID",
        wit: "texts.actor-id",
    },
    WitOp {
        abi: abi::TEXT_PARENT,
        name: "TEXT_PARENT",
        wit: "texts.parent",
    },
    WitOp {
        abi: abi::TEXT_NEW_ACTOR,
        name: "TEXT_NEW_ACTOR",
        wit: "texts.new-actor",
    },
    WitOp {
        abi: abi::TEXT_UI_VALUE,
        name: "TEXT_UI_VALUE",
        wit: "texts.ui-value",
    },
    WitOp {
        abi: abi::TEXT_UI_TEXT,
        name: "TEXT_UI_TEXT",
        wit: "texts.ui-text",
    },
    WitOp {
        abi: abi::TEXT_UI_FOCUS,
        name: "TEXT_UI_FOCUS",
        wit: "texts.ui-focus",
    },
    WitOp {
        abi: abi::TEXT_RAY_HIT,
        name: "TEXT_RAY_HIT",
        wit: "texts.ray-hit",
    },
    WitOp {
        abi: abi::TEXT_CIRCLE_HIT,
        name: "TEXT_CIRCLE_HIT",
        wit: "texts.circle-hit",
    },
    WitOp {
        abi: abi::TEXT_CURRENT_CLIP,
        name: "TEXT_CURRENT_CLIP",
        wit: "texts.current-clip",
    },
    WitOp {
        abi: abi::TEXT_ACTIVE_VOLUMES,
        name: "TEXT_ACTIVE_VOLUMES",
        wit: "texts.active-volumes",
    },
    WitOp {
        abi: abi::TEXT_EVENT,
        name: "TEXT_EVENT",
        wit: "texts.event",
    },
    WitOp {
        abi: abi::TEXT_ROOM,
        name: "TEXT_ROOM",
        wit: "texts.room",
    },
    WitOp {
        abi: abi::TEXT_ENTERED_ROOM,
        name: "TEXT_ENTERED_ROOM",
        wit: "texts.entered-room",
    },
    WitOp {
        abi: abi::TEXT_CURRENT_SCENE,
        name: "TEXT_CURRENT_SCENE",
        wit: "texts.current-scene",
    },
    WitOp {
        abi: abi::TEXT_SCENE_NAMES,
        name: "TEXT_SCENE_NAMES",
        wit: "texts.scene-names",
    },
    WitOp {
        abi: abi::TEXT_CURRENT_QUALITY,
        name: "TEXT_CURRENT_QUALITY",
        wit: "texts.current-quality",
    },
    WitOp {
        abi: abi::TEXT_CURRENT_WEATHER,
        name: "TEXT_CURRENT_WEATHER",
        wit: "texts.current-weather",
    },
    WitOp {
        abi: abi::TEXT_CUTSCENE_NAME,
        name: "TEXT_CUTSCENE_NAME",
        wit: "texts.cutscene-name",
    },
    WitOp {
        abi: abi::TEXT_PLUGIN,
        name: "TEXT_PLUGIN",
        wit: "plugins.read-text",
    },
    WitOp {
        abi: abi::TEXT_QUERY,
        name: "TEXT_QUERY",
        wit: "physics.query-text",
    },
    WitOp {
        abi: abi::TEXT_CONTROLLER,
        name: "TEXT_CONTROLLER",
        wit: "physics.controller-text",
    },
    WitOp {
        abi: abi::TEXT_DATA,
        name: "TEXT_DATA",
        wit: "state.get-text",
    },
    WitOp {
        abi: abi::TEXT_VARIABLE,
        name: "TEXT_VARIABLE",
        wit: "vars.get-text",
    },
    WitOp {
        abi: abi::TEXT_LIST_ITEM,
        name: "TEXT_LIST_ITEM",
        wit: "lists.get-text",
    },
    WitOp {
        abi: abi::TEXT_SAVE_SLOT,
        name: "TEXT_SAVE_SLOT",
        wit: "texts.save-slot",
    },
    WitOp {
        abi: abi::TEXT_SAVE_SLOTS,
        name: "TEXT_SAVE_SLOTS",
        wit: "texts.save-slots",
    },
    WitOp {
        abi: abi::TEXT_LANGUAGE,
        name: "TEXT_LANGUAGE",
        wit: "texts.language",
    },
    WitOp {
        abi: abi::TEXT_LOCALE_TEXT,
        name: "TEXT_LOCALE_TEXT",
        wit: "texts.locale-text",
    },
];

/// Every `abi::ACT_*` verb and its WIT path.
pub const ACTS: &[WitOp] = &[
    WitOp {
        abi: abi::ACT_MOVE,
        name: "ACT_MOVE",
        wit: "acts.move",
    },
    WitOp {
        abi: abi::ACT_GO_TO,
        name: "ACT_GO_TO",
        wit: "acts.go-to",
    },
    WitOp {
        abi: abi::ACT_CHANGE_POSITION,
        name: "ACT_CHANGE_POSITION",
        wit: "acts.change-position",
    },
    WitOp {
        abi: abi::ACT_TURN,
        name: "ACT_TURN",
        wit: "acts.turn",
    },
    WitOp {
        abi: abi::ACT_SET_ROTATION,
        name: "ACT_SET_ROTATION",
        wit: "acts.set-rotation",
    },
    WitOp {
        abi: abi::ACT_POINT_TOWARDS,
        name: "ACT_POINT_TOWARDS",
        wit: "acts.point-towards",
    },
    WitOp {
        abi: abi::ACT_SET_SCALE,
        name: "ACT_SET_SCALE",
        wit: "acts.set-scale",
    },
    WitOp {
        abi: abi::ACT_APPLY_IMPULSE,
        name: "ACT_APPLY_IMPULSE",
        wit: "acts.apply-impulse",
    },
    WitOp {
        abi: abi::ACT_SET_VELOCITY,
        name: "ACT_SET_VELOCITY",
        wit: "acts.set-velocity",
    },
    WitOp {
        abi: abi::ACT_SAY,
        name: "ACT_SAY",
        wit: "acts.say",
    },
    WitOp {
        abi: abi::ACT_SET_VISIBLE,
        name: "ACT_SET_VISIBLE",
        wit: "acts.set-visible",
    },
    WitOp {
        abi: abi::ACT_SET_COLOR,
        name: "ACT_SET_COLOR",
        wit: "acts.set-color",
    },
    WitOp {
        abi: abi::ACT_BROADCAST,
        name: "ACT_BROADCAST",
        wit: "acts.broadcast",
    },
    WitOp {
        abi: abi::ACT_SET_FIELD,
        name: "ACT_SET_FIELD",
        wit: "acts.set-field",
    },
    WitOp {
        abi: abi::ACT_SET_FIELD_TEXT,
        name: "ACT_SET_FIELD_TEXT",
        wit: "acts.set-field-text",
    },
    WitOp {
        abi: abi::ACT_ATTACH,
        name: "ACT_ATTACH",
        wit: "acts.attach",
    },
    WitOp {
        abi: abi::ACT_DETACH,
        name: "ACT_DETACH",
        wit: "acts.detach",
    },
    WitOp {
        abi: abi::ACT_SET_CAMERA_VIEW,
        name: "ACT_SET_CAMERA_VIEW",
        wit: "acts.set-camera-view",
    },
    WitOp {
        abi: abi::ACT_LOG,
        name: "ACT_LOG",
        wit: "acts.log",
    },
    WitOp {
        abi: abi::ACT_STOP_ALL,
        name: "ACT_STOP_ALL",
        wit: "acts.stop-all",
    },
    WitOp {
        abi: abi::ACT_SET_PARENT,
        name: "ACT_SET_PARENT",
        wit: "acts.set-parent",
    },
    WitOp {
        abi: abi::ACT_CREATE_CLONE,
        name: "ACT_CREATE_CLONE",
        wit: "acts.create-clone",
    },
    WitOp {
        abi: abi::ACT_CREATE_ACTOR,
        name: "ACT_CREATE_ACTOR",
        wit: "acts.create-actor",
    },
    WitOp {
        abi: abi::ACT_DELETE_ACTOR,
        name: "ACT_DELETE_ACTOR",
        wit: "acts.delete-actor",
    },
    WitOp {
        abi: abi::ACT_SET_MOUSE_LOCKED,
        name: "ACT_SET_MOUSE_LOCKED",
        wit: "acts.set-mouse-locked",
    },
    WitOp {
        abi: abi::ACT_SET_CAMERA_PITCH,
        name: "ACT_SET_CAMERA_PITCH",
        wit: "acts.set-camera-pitch",
    },
    WitOp {
        abi: abi::ACT_UI_SHOW,
        name: "ACT_UI_SHOW",
        wit: "acts.ui-show",
    },
    WitOp {
        abi: abi::ACT_UI_SET,
        name: "ACT_UI_SET",
        wit: "acts.ui-set",
    },
    WitOp {
        abi: abi::ACT_UI_SET_TEXT,
        name: "ACT_UI_SET_TEXT",
        wit: "acts.ui-set-text",
    },
    WitOp {
        abi: abi::ACT_UI_HIDE,
        name: "ACT_UI_HIDE",
        wit: "acts.ui-hide",
    },
    WitOp {
        abi: abi::ACT_UI_DELETE,
        name: "ACT_UI_DELETE",
        wit: "acts.ui-delete",
    },
    WitOp {
        abi: abi::ACT_SET_PAUSED,
        name: "ACT_SET_PAUSED",
        wit: "acts.set-paused",
    },
    WitOp {
        abi: abi::ACT_UI_FOCUS,
        name: "ACT_UI_FOCUS",
        wit: "acts.ui-focus",
    },
    WitOp {
        abi: abi::ACT_UI_THEME,
        name: "ACT_UI_THEME",
        wit: "acts.ui-theme",
    },
    WitOp {
        abi: abi::ACT_SAVE_VARIABLE,
        name: "ACT_SAVE_VARIABLE",
        wit: "acts.save-variable",
    },
    WitOp {
        abi: abi::ACT_SET_CAMERA_FOV,
        name: "ACT_SET_CAMERA_FOV",
        wit: "acts.set-camera-fov",
    },
    WitOp {
        abi: abi::ACT_NAVIGATE_TO,
        name: "ACT_NAVIGATE_TO",
        wit: "acts.navigate-to",
    },
    WitOp {
        abi: abi::ACT_PLAY_SOUND,
        name: "ACT_PLAY_SOUND",
        wit: "acts.play-sound",
    },
    WitOp {
        abi: abi::ACT_STOP_SOUND,
        name: "ACT_STOP_SOUND",
        wit: "acts.stop-sound",
    },
    WitOp {
        abi: abi::ACT_SET_SOUND_VOLUME,
        name: "ACT_SET_SOUND_VOLUME",
        wit: "acts.set-sound-volume",
    },
    WitOp {
        abi: abi::ACT_SET_SOUND_PITCH,
        name: "ACT_SET_SOUND_PITCH",
        wit: "acts.set-sound-pitch",
    },
    WitOp {
        abi: abi::ACT_SET_BUS_VOLUME,
        name: "ACT_SET_BUS_VOLUME",
        wit: "acts.set-bus-volume",
    },
    WitOp {
        abi: abi::ACT_SET_TRIGGER,
        name: "ACT_SET_TRIGGER",
        wit: "acts.set-trigger",
    },
    WitOp {
        abi: abi::ACT_SET_COLLISION_LAYER,
        name: "ACT_SET_COLLISION_LAYER",
        wit: "acts.set-collision-layer",
    },
    WitOp {
        abi: abi::ACT_SET_COLLISION_MASK,
        name: "ACT_SET_COLLISION_MASK",
        wit: "acts.set-collision-mask",
    },
    WitOp {
        abi: abi::ACT_RUMBLE_GAMEPAD,
        name: "ACT_RUMBLE_GAMEPAD",
        wit: "acts.rumble-gamepad",
    },
    WitOp {
        abi: abi::ACT_BIND_ACTION,
        name: "ACT_BIND_ACTION",
        wit: "acts.bind-action",
    },
    WitOp {
        abi: abi::ACT_CLEAR_ACTION_BINDINGS,
        name: "ACT_CLEAR_ACTION_BINDINGS",
        wit: "acts.clear-action-bindings",
    },
    WitOp {
        abi: abi::ACT_SET_EXPOSURE,
        name: "ACT_SET_EXPOSURE",
        wit: "acts.set-exposure",
    },
    WitOp {
        abi: abi::ACT_SET_LIGHT_INTENSITY,
        name: "ACT_SET_LIGHT_INTENSITY",
        wit: "acts.set-light-intensity",
    },
    WitOp {
        abi: abi::ACT_TWEEN_SCALE,
        name: "ACT_TWEEN_SCALE",
        wit: "acts.tween-scale",
    },
    WitOp {
        abi: abi::ACT_TWEEN_ROTATION,
        name: "ACT_TWEEN_ROTATION",
        wit: "acts.tween-rotation",
    },
    WitOp {
        abi: abi::ACT_TWEEN_COLOR,
        name: "ACT_TWEEN_COLOR",
        wit: "acts.tween-color",
    },
    WitOp {
        abi: abi::ACT_STOP_TWEENS,
        name: "ACT_STOP_TWEENS",
        wit: "acts.stop-tweens",
    },
    WitOp {
        abi: abi::ACT_PLAY_ANIMATION,
        name: "ACT_PLAY_ANIMATION",
        wit: "acts.play-animation",
    },
    WitOp {
        abi: abi::ACT_STOP_ANIMATION,
        name: "ACT_STOP_ANIMATION",
        wit: "acts.stop-animation",
    },
    WitOp {
        abi: abi::ACT_SET_ANIMATION_SPEED,
        name: "ACT_SET_ANIMATION_SPEED",
        wit: "acts.set-animation-speed",
    },
    WitOp {
        abi: abi::ACT_SET_EMISSIVE_STRENGTH,
        name: "ACT_SET_EMISSIVE_STRENGTH",
        wit: "acts.set-emissive-strength",
    },
    WitOp {
        abi: abi::ACT_SET_HDR_OUTPUT,
        name: "ACT_SET_HDR_OUTPUT",
        wit: "acts.set-hdr-output",
    },
    WitOp {
        abi: abi::ACT_SET_PEAK_BRIGHTNESS,
        name: "ACT_SET_PEAK_BRIGHTNESS",
        wit: "acts.set-peak-brightness",
    },
    WitOp {
        abi: abi::ACT_ENABLE_VOLUME,
        name: "ACT_ENABLE_VOLUME",
        wit: "acts.enable-volume",
    },
    WitOp {
        abi: abi::ACT_SET_VOLUME_WEIGHT,
        name: "ACT_SET_VOLUME_WEIGHT",
        wit: "acts.set-volume-weight",
    },
    WitOp {
        abi: abi::ACT_CAPTURE_PROBES,
        name: "ACT_CAPTURE_PROBES",
        wit: "acts.capture-probes",
    },
    WitOp {
        abi: abi::ACT_SET_SHADOW_DISTANCE,
        name: "ACT_SET_SHADOW_DISTANCE",
        wit: "acts.set-shadow-distance",
    },
    WitOp {
        abi: abi::ACT_SET_LIGHT_SHADOWS,
        name: "ACT_SET_LIGHT_SHADOWS",
        wit: "acts.set-light-shadows",
    },
    WitOp {
        abi: abi::ACT_SET_RAY_TRACING,
        name: "ACT_SET_RAY_TRACING",
        wit: "acts.set-ray-tracing",
    },
    WitOp {
        abi: abi::ACT_SET_GI_BOUNCES,
        name: "ACT_SET_GI_BOUNCES",
        wit: "acts.set-gi-bounces",
    },
    WitOp {
        abi: abi::ACT_SET_GI_SAMPLES,
        name: "ACT_SET_GI_SAMPLES",
        wit: "acts.set-gi-samples",
    },
    WitOp {
        abi: abi::ACT_SET_FOG_DENSITY,
        name: "ACT_SET_FOG_DENSITY",
        wit: "acts.set-fog-density",
    },
    WitOp {
        abi: abi::ACT_SET_AURORA,
        name: "ACT_SET_AURORA",
        wit: "acts.set-aurora",
    },
    WitOp {
        abi: abi::ACT_STRIKE_LIGHTNING,
        name: "ACT_STRIKE_LIGHTNING",
        wit: "acts.strike-lightning",
    },
    WitOp {
        abi: abi::ACT_SET_LIGHTNING_RATE,
        name: "ACT_SET_LIGHTNING_RATE",
        wit: "acts.set-lightning-rate",
    },
    WitOp {
        abi: abi::ACT_SET_WIND,
        name: "ACT_SET_WIND",
        wit: "acts.set-wind",
    },
    WitOp {
        abi: abi::ACT_SET_CLOUD_DRIFT,
        name: "ACT_SET_CLOUD_DRIFT",
        wit: "acts.set-cloud-drift",
    },
    WitOp {
        abi: abi::ACT_SET_CLOUDS,
        name: "ACT_SET_CLOUDS",
        wit: "acts.set-clouds",
    },
    WitOp {
        abi: abi::ACT_SET_CLOUD_LAYER,
        name: "ACT_SET_CLOUD_LAYER",
        wit: "acts.set-cloud-layer",
    },
    WitOp {
        abi: abi::ACT_SET_WATER,
        name: "ACT_SET_WATER",
        wit: "acts.set-water",
    },
    WitOp {
        abi: abi::ACT_FIRE_ANIMATION_TRIGGER,
        name: "ACT_FIRE_ANIMATION_TRIGGER",
        wit: "acts.fire-animation-trigger",
    },
    WitOp {
        abi: abi::ACT_SET_RIG_SLOT,
        name: "ACT_SET_RIG_SLOT",
        wit: "acts.set-rig-slot",
    },
    WitOp {
        abi: abi::ACT_SET_SLOT_TINT,
        name: "ACT_SET_SLOT_TINT",
        wit: "acts.set-slot-tint",
    },
    WitOp {
        abi: abi::ACT_SET_IK_TARGET,
        name: "ACT_SET_IK_TARGET",
        wit: "acts.set-ik-target",
    },
    WitOp {
        abi: abi::ACT_SET_SPRITE_DIAL,
        name: "ACT_SET_SPRITE_DIAL",
        wit: "acts.set-sprite-dial",
    },
    WitOp {
        abi: abi::ACT_BURST_PARTICLES,
        name: "ACT_BURST_PARTICLES",
        wit: "acts.burst-particles",
    },
    WitOp {
        abi: abi::ACT_SET_EMITTER_DIAL,
        name: "ACT_SET_EMITTER_DIAL",
        wit: "acts.set-emitter-dial",
    },
    WitOp {
        abi: abi::ACT_SET_EMITTER_PLAYING,
        name: "ACT_SET_EMITTER_PLAYING",
        wit: "acts.set-emitter-playing",
    },
    WitOp {
        abi: abi::ACT_PAINT_TILE,
        name: "ACT_PAINT_TILE",
        wit: "acts.paint-tile",
    },
    WitOp {
        abi: abi::ACT_SET_PARALLAX,
        name: "ACT_SET_PARALLAX",
        wit: "acts.set-parallax",
    },
    WitOp {
        abi: abi::ACT_SWITCH_SCENE,
        name: "ACT_SWITCH_SCENE",
        wit: "acts.switch-scene",
    },
    WitOp {
        abi: abi::ACT_SET_TIME_OF_DAY,
        name: "ACT_SET_TIME_OF_DAY",
        wit: "acts.set-time-of-day",
    },
    WitOp {
        abi: abi::ACT_ADVANCE_TIME,
        name: "ACT_ADVANCE_TIME",
        wit: "acts.advance-time",
    },
    WitOp {
        abi: abi::ACT_SET_PRECIPITATION,
        name: "ACT_SET_PRECIPITATION",
        wit: "acts.set-precipitation",
    },
    WitOp {
        abi: abi::ACT_BLEND_WEATHER,
        name: "ACT_BLEND_WEATHER",
        wit: "acts.blend-weather",
    },
    WitOp {
        abi: abi::ACT_PLAY_CUTSCENE,
        name: "ACT_PLAY_CUTSCENE",
        wit: "acts.play-cutscene",
    },
    WitOp {
        abi: abi::ACT_SKIP_CUTSCENE,
        name: "ACT_SKIP_CUTSCENE",
        wit: "acts.skip-cutscene",
    },
    WitOp {
        abi: abi::ACT_CAMERA_SHAKE,
        name: "ACT_CAMERA_SHAKE",
        wit: "acts.camera-shake",
    },
    WitOp {
        abi: abi::ACT_SET_TIME_SCALE,
        name: "ACT_SET_TIME_SCALE",
        wit: "acts.set-time-scale",
    },
    WitOp {
        abi: abi::ACT_HITSTOP,
        name: "ACT_HITSTOP",
        wit: "acts.hitstop",
    },
    WitOp {
        abi: abi::ACT_SET_LETTERBOX,
        name: "ACT_SET_LETTERBOX",
        wit: "acts.set-letterbox",
    },
    WitOp {
        abi: abi::ACT_FADE_SCREEN,
        name: "ACT_FADE_SCREEN",
        wit: "acts.fade-screen",
    },
    WitOp {
        abi: abi::ACT_PLUGIN_CALL,
        name: "ACT_PLUGIN_CALL",
        wit: "plugins.call",
    },
    WitOp {
        abi: abi::ACT_ADD_FORCE,
        name: "ACT_ADD_FORCE",
        wit: "physics.add-force",
    },
    WitOp {
        abi: abi::ACT_PHYSICS_QUERY,
        name: "ACT_PHYSICS_QUERY",
        wit: "physics.query",
    },
    WitOp {
        abi: abi::ACT_CONTROLLER,
        name: "ACT_CONTROLLER",
        wit: "physics.controller-move",
    },
    WitOp {
        abi: abi::ACT_SET_DATA,
        name: "ACT_SET_DATA",
        wit: "state.set-number",
    },
    WitOp {
        abi: abi::ACT_SET_DATA_TEXT,
        name: "ACT_SET_DATA_TEXT",
        wit: "state.set-text",
    },
    WitOp {
        abi: abi::ACT_CLEAR_DATA,
        name: "ACT_CLEAR_DATA",
        wit: "state.clear",
    },
    WitOp {
        abi: abi::ACT_SET_RENDER_SETTING,
        name: "ACT_SET_RENDER_SETTING",
        wit: "acts.set-render-setting",
    },
    WitOp {
        abi: abi::ACT_GO_TO_OTHER,
        name: "ACT_GO_TO_OTHER",
        wit: "acts.go-to-other",
    },
    WitOp {
        abi: abi::ACT_CHANGE_POSITION_OTHER,
        name: "ACT_CHANGE_POSITION_OTHER",
        wit: "acts.change-position-other",
    },
    WitOp {
        abi: abi::ACT_MOVE_OTHER,
        name: "ACT_MOVE_OTHER",
        wit: "acts.move-other",
    },
    WitOp {
        abi: abi::ACT_TURN_OTHER,
        name: "ACT_TURN_OTHER",
        wit: "acts.turn-other",
    },
    WitOp {
        abi: abi::ACT_SET_ROTATION_OTHER,
        name: "ACT_SET_ROTATION_OTHER",
        wit: "acts.set-rotation-other",
    },
    WitOp {
        abi: abi::ACT_SET_SCALE_OTHER,
        name: "ACT_SET_SCALE_OTHER",
        wit: "acts.set-scale-other",
    },
    WitOp {
        abi: abi::ACT_POINT_TOWARDS_OTHER,
        name: "ACT_POINT_TOWARDS_OTHER",
        wit: "acts.point-towards-other",
    },
    WitOp {
        abi: abi::ACT_SET_VISIBLE_OTHER,
        name: "ACT_SET_VISIBLE_OTHER",
        wit: "acts.set-visible-other",
    },
    WitOp {
        abi: abi::ACT_SET_COLOR_OTHER,
        name: "ACT_SET_COLOR_OTHER",
        wit: "acts.set-color-other",
    },
    WitOp {
        abi: abi::ACT_SAY_OTHER,
        name: "ACT_SAY_OTHER",
        wit: "acts.say-other",
    },
    WitOp {
        abi: abi::ACT_APPLY_IMPULSE_OTHER,
        name: "ACT_APPLY_IMPULSE_OTHER",
        wit: "acts.apply-impulse-other",
    },
    WitOp {
        abi: abi::ACT_SET_VELOCITY_OTHER,
        name: "ACT_SET_VELOCITY_OTHER",
        wit: "acts.set-velocity-other",
    },
    WitOp {
        abi: abi::ACT_ADD_FORCE_OTHER,
        name: "ACT_ADD_FORCE_OTHER",
        wit: "physics.add-force",
    },
    WitOp {
        abi: abi::ACT_SET_VARIABLE,
        name: "ACT_SET_VARIABLE",
        wit: "vars.set-number",
    },
    WitOp {
        abi: abi::ACT_SET_VARIABLE_TEXT,
        name: "ACT_SET_VARIABLE_TEXT",
        wit: "vars.set-text",
    },
    WitOp {
        abi: abi::ACT_LIST_ADD,
        name: "ACT_LIST_ADD",
        wit: "lists.add-number",
    },
    WitOp {
        abi: abi::ACT_LIST_ADD_TEXT,
        name: "ACT_LIST_ADD_TEXT",
        wit: "lists.add-text",
    },
    WitOp {
        abi: abi::ACT_LIST_INSERT,
        name: "ACT_LIST_INSERT",
        wit: "lists.insert-number",
    },
    WitOp {
        abi: abi::ACT_LIST_INSERT_TEXT,
        name: "ACT_LIST_INSERT_TEXT",
        wit: "lists.insert-text",
    },
    WitOp {
        abi: abi::ACT_LIST_REPLACE,
        name: "ACT_LIST_REPLACE",
        wit: "lists.replace-number",
    },
    WitOp {
        abi: abi::ACT_LIST_REPLACE_TEXT,
        name: "ACT_LIST_REPLACE_TEXT",
        wit: "lists.replace-text",
    },
    WitOp {
        abi: abi::ACT_LIST_DELETE,
        name: "ACT_LIST_DELETE",
        wit: "lists.delete",
    },
    WitOp {
        abi: abi::ACT_LIST_CLEAR,
        name: "ACT_LIST_CLEAR",
        wit: "lists.clear",
    },
    WitOp {
        abi: abi::ACT_SWITCH_SAVE_SLOT,
        name: "ACT_SWITCH_SAVE_SLOT",
        wit: "acts.switch-save-slot",
    },
    WitOp {
        abi: abi::ACT_DELETE_SAVE_SLOT,
        name: "ACT_DELETE_SAVE_SLOT",
        wit: "acts.delete-save-slot",
    },
    WitOp {
        abi: abi::ACT_SET_LANGUAGE,
        name: "ACT_SET_LANGUAGE",
        wit: "acts.set-language",
    },
];

/// Every `abi::EVENT_*` verb and its WIT path.
pub const EVENTS: &[WitOp] = &[
    WitOp {
        abi: abi::EVENT_MESSAGE,
        name: "EVENT_MESSAGE",
        wit: "event.message",
    },
    WitOp {
        abi: abi::EVENT_KEY,
        name: "EVENT_KEY",
        wit: "event.key",
    },
    WitOp {
        abi: abi::EVENT_ACTION,
        name: "EVENT_ACTION",
        wit: "event.action",
    },
    WitOp {
        abi: abi::EVENT_CLICKED,
        name: "EVENT_CLICKED",
        wit: "event.clicked",
    },
    WitOp {
        abi: abi::EVENT_TOUCHED,
        name: "EVENT_TOUCHED",
        wit: "event.touched",
    },
    WitOp {
        abi: abi::EVENT_COLLISION,
        name: "EVENT_COLLISION",
        wit: "event.collision",
    },
    WitOp {
        abi: abi::EVENT_PARTICLES,
        name: "EVENT_PARTICLES",
        wit: "event.particles",
    },
    WitOp {
        abi: abi::EVENT_ANIMATION_ENDED,
        name: "EVENT_ANIMATION_ENDED",
        wit: "event.animation-ended",
    },
    WitOp {
        abi: abi::EVENT_ANIMATION_MARKER,
        name: "EVENT_ANIMATION_MARKER",
        wit: "event.animation-marker",
    },
    WitOp {
        abi: abi::EVENT_UI_CLICKED,
        name: "EVENT_UI_CLICKED",
        wit: "event.ui-clicked",
    },
    WitOp {
        abi: abi::EVENT_UI_CHANGED,
        name: "EVENT_UI_CHANGED",
        wit: "event.ui-changed",
    },
    WitOp {
        abi: abi::EVENT_UI,
        name: "EVENT_UI",
        wit: "event.ui",
    },
    WitOp {
        abi: abi::EVENT_ENTERED_ROOM,
        name: "EVENT_ENTERED_ROOM",
        wit: "event.entered-room",
    },
    WitOp {
        abi: abi::EVENT_SCENE_STARTED,
        name: "EVENT_SCENE_STARTED",
        wit: "event.scene-started",
    },
    WitOp {
        abi: abi::EVENT_SCENE_ENDED,
        name: "EVENT_SCENE_ENDED",
        wit: "event.scene-ended",
    },
    WitOp {
        abi: abi::EVENT_QUALITY_DROPPED,
        name: "EVENT_QUALITY_DROPPED",
        wit: "event.quality-dropped",
    },
    WitOp {
        abi: abi::EVENT_WEATHER,
        name: "EVENT_WEATHER",
        wit: "event.weather",
    },
    WitOp {
        abi: abi::EVENT_CUTSCENE_SIGNAL,
        name: "EVENT_CUTSCENE_SIGNAL",
        wit: "event.cutscene-signal",
    },
    WitOp {
        abi: abi::EVENT_CUTSCENE_ENDED,
        name: "EVENT_CUTSCENE_ENDED",
        wit: "event.cutscene-ended",
    },
    WitOp {
        abi: abi::EVENT_PLUGIN,
        name: "EVENT_PLUGIN",
        wit: "event.plugin",
    },
    WitOp {
        abi: abi::EVENT_CONTACT,
        name: "EVENT_CONTACT",
        wit: "event.contact",
    },
];

/// Every entry point a script exports, without the `blockloom_script_`
/// prefix. Optional ones (`frame`, `ui`, `stop`, `destroy`) behave as
/// absent when unexported, as native scripts do.
pub const LIFECYCLE: &[&str] = &["start", "tick", "frame", "ui", "stop", "destroy", "event"];

/// One guest language's componentize toolchain: how its source becomes a
/// module for this world. Rust ships first; one GC language (Python or
/// TypeScript) is the gate before Go, C#, Kotlin or C are promised.
pub struct GuestToolchain {
    /// `Rust`, `Python`, ...
    pub language: &'static str,
    /// The compiler that emits the component, pinned per release.
    pub toolchain: &'static str,
    /// The exact build command from a source file to a module.
    pub build: &'static str,
    /// What to know before promising it for per-tick scripts.
    pub notes: &'static str,
}

pub const GUEST_TOOLCHAINS: &[GuestToolchain] = &[
    GuestToolchain {
        language: "Rust",
        toolchain: "rustc + wasm32-unknown-unknown std, wit-bindgen guest crate",
        build: "cargo build --target wasm32-unknown-unknown (cdylib) over the generated bindings",
        notes: "Ships first as blockloom-script-guest: the frozen world hand-lowered to the core-module shape, so sandbox tests hold its effects against the fixture's line for line.",
    },
    GuestToolchain {
        language: "Python",
        toolchain: "componentize-py",
        build: "componentize-py -d world.wit -w script app -o script.wasm",
        notes: "Gate candidate GC language: bundled interpreter size and per-call lift cost are the known risk; needs the overhead number before per-tick scripts are promised.",
    },
    GuestToolchain {
        language: "TypeScript",
        toolchain: "componentize-js (StarlingMonkey)",
        build: "componentize-js app.js -d world.wit -w script -o script.wasm",
        notes: "Alternate gate candidate GC language; same interpreter-size and lift-cost caveat as Python.",
    },
    GuestToolchain {
        language: "Go",
        toolchain: "TinyGo (standard Go cannot emit components)",
        build: "tinygo build -target=wasi -o script.wasm",
        notes: "Not promised until the Rust + GC-language gate lands; GC lifts per tick are unmeasured.",
    },
    GuestToolchain {
        language: "C#",
        toolchain: ".NET 10 wasi-wasm RID + Componentize.DotNet SDK",
        build: "dotnet build (wasi-wasm) with the componentize SDK",
        notes: "Not promised until the gate lands.",
    },
    GuestToolchain {
        language: "Kotlin",
        toolchain: "Gradle wasmWasi",
        build: "./gradlew wasmWasiBinary",
        notes: "Not promised until the gate lands.",
    },
    GuestToolchain {
        language: "C",
        toolchain: "wasi-sdk clang -mexec-model=reactor",
        build: "clang --target=wasm32-wasi reactor.c -o script.wasm",
        notes: "Manual memory management fits the fuel model; still needs bindings and the overhead number.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `pub const` verb in `abi.rs`, by namespace.
    fn abi_verbs(prefix: &str) -> Vec<(String, u32)> {
        let mut verbs = Vec::new();
        for line in include_str!("abi.rs").lines() {
            let line = line.trim();
            let head = format!("pub const {prefix}_");
            let Some(rest) = line.strip_prefix(&head) else {
                continue;
            };
            let Some((name, value)) = rest.split_once(": u32 = ") else {
                continue;
            };
            let value: u32 = value
                .trim()
                .trim_end_matches(';')
                .parse()
                .expect("a numeric verb");
            verbs.push((format!("{prefix}_{name}"), value));
        }
        verbs
    }

    fn check_table(prefix: &str, table: &[WitOp]) {
        let verbs = abi_verbs(prefix);
        assert!(!verbs.is_empty(), "no {prefix} verbs parsed");
        assert_eq!(
            table.len(),
            verbs.len(),
            "{prefix} table drifted from abi.rs"
        );
        for (name, value) in &verbs {
            let found = table.iter().find(|op| op.name == name);
            let Some(found) = found else {
                panic!("{name} has no WIT path")
            };
            assert_eq!(found.abi, *value, "{name} moved");
            let (iface, func) = found.wit.split_once('.').expect("interface.func");
            assert!(
                WIT.contains(&format!("interface {iface}")),
                "{name} names no WIT interface"
            );
            // Typed homes are written out; the long tail rides the
            // kebab-case of its ABI name, which the WIT documents.
            let default = name[prefix.len() + 1..].to_lowercase().replace('_', "-");
            assert!(
                WIT.contains(func) || *func == default,
                "{name} names nothing in WIT"
            );
        }
    }

    #[test]
    fn every_abi_verb_names_its_wit_path() {
        check_table("READ", READS);
        check_table("TEXT", TEXTS);
        check_table("ACT", ACTS);
        check_table("EVENT", EVENTS);
    }

    #[test]
    fn lifecycle_matches_the_exported_symbols() {
        for line in include_str!("abi.rs").lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("pub const SYM_") else {
                continue;
            };
            let Some((name, _)) = rest.split_once(": &[u8]") else {
                continue;
            };
            if name == "ABI" {
                continue;
            }
            let exported = name.to_lowercase();
            assert!(
                LIFECYCLE.contains(&exported.as_str()),
                "SYM_{name} has no lifecycle entry"
            );
        }
        assert!(WIT.contains("package blockloom:script@"));
        assert!(WIT.contains(&"world script".to_string()));
        assert!(
            WIT.contains("read-after-write reads old")
                || WIT.contains("read-after-write still reads old")
        );
    }

    #[test]
    fn guest_bindings_start_with_rust_and_one_gc_language() {
        let has = |lang: &str| {
            GUEST_TOOLCHAINS
                .iter()
                .find(|t| t.language == lang)
                .expect(lang)
        };
        let rust = has("Rust");
        assert!(!rust.toolchain.is_empty() && !rust.build.is_empty());
        let gate = has("Python");
        assert!(!gate.toolchain.is_empty() && !gate.build.is_empty());
        for entry in GUEST_TOOLCHAINS {
            assert!(
                !entry.language.is_empty() && !entry.notes.is_empty(),
                "toolchain entry is blank"
            );
        }
    }

    /// The WIT file the guest crate (and `wit-bindgen`) consumes is the frozen
    /// world, byte for byte: the file starts at `package`, without the raw
    /// string's leading newline.
    #[test]
    fn guest_world_file_matches_the_frozen_world() {
        let file = include_str!("../../../blockloom-script-guest/wit/world.wit");
        assert_eq!(file, WIT.strip_prefix('\n').unwrap_or(WIT));
    }

    /// Every `pub const` verb in the guest crate, by namespace.
    fn guest_verbs(source: &str, prefix: &str) -> Vec<(String, u32)> {
        let mut verbs = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            let head = format!("pub const {prefix}_");
            let Some(rest) = line.strip_prefix(&head) else {
                continue;
            };
            let Some((name, value)) = rest.split_once(": u32 = ") else {
                continue;
            };
            let value: u32 = value
                .trim()
                .trim_end_matches(';')
                .parse()
                .expect("a numeric verb");
            verbs.push((format!("{prefix}_{name}"), value));
        }
        verbs
    }

    /// The guest crate binds the whole frozen world: every ABI verb has a
    /// constant with the same value there, and the versions agree. A new verb
    /// without a binding fails here, not silently in a guest.
    #[test]
    fn guest_crate_covers_every_abi_verb() {
        let guest = include_str!("../../../blockloom-script-guest/src/lib.rs");
        let verbs = include_str!("../../../blockloom-script-guest/src/verbs.rs");
        for prefix in ["READ", "TEXT", "ACT", "EVENT"] {
            let mine = abi_verbs(prefix);
            let theirs = guest_verbs(verbs, prefix);
            assert_eq!(theirs.len(), mine.len(), "{prefix} guest drifted");
            for (name, value) in &mine {
                let found = theirs.iter().find(|(n, _)| n == name);
                let Some((_, theirs)) = found else {
                    panic!("{name} has no guest binding")
                };
                assert_eq!(*theirs, *value, "{name} moved");
            }
        }
        let version = |source: &str, name: &str| -> u32 {
            source
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("pub const {name}: u32 = "))?
                        .trim_end_matches(';')
                        .parse()
                        .ok()
                })
                .expect("a version constant")
        };
        assert_eq!(version(guest, "WIT_VERSION"), WIT_VERSION);
        assert_eq!(version(guest, "ABI_VERSION"), abi::ABI_VERSION);
    }
}
