//! The rule the compiler has to keep: a compiled program asks the world for
//! exactly what the VM asks for, in the same order, with the same values, and
//! with the same complaints about the same bad slots.
//!
//! Each case is one project run twice - stepped by the VM, and compiled by
//! `codegen` into Rust that a `rustc` run turns into a program which prints
//! what it would have done. Both are driven tick by tick off the same clock,
//! and every line says which tick it landed on, so a loop that forgot to hand
//! the frame back fails here rather than merely running fast. The two
//! transcripts have to match line for line.
//!
//! Anything the two could disagree about that isn't the blocks' fault stays
//! out of these projects: `random` and `current time` read a world the two
//! halves don't share, and `broadcast` starts scripts the VM has a queue for
//! and this harness doesn't.
//!
//! No toolchain means no compiled half to compare against, so the whole file
//! skips rather than fails - the same bargain `blockloom-runtime`'s script
//! tests make.

#![allow(clippy::field_reassign_with_default)]

use blockloom_core::animation::TweenEasing;
use blockloom_core::blocks::{
    BlockDef, BlockPiece, BlockShape, DictDef, DictEntry, DictItem, EmitterDial, InputValueType,
    Instruction, InstructionKind as K, ListDef, ListItem, SpriteDial, Strand, VariableDef,
};
use blockloom_core::cloud_layers::CloudLayerProperty;
use blockloom_core::clouds::CloudProperty;
use blockloom_core::director::PrecipitationKind;
use blockloom_core::input::ActionSense;
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::{Axis, Mode, Visual};
use blockloom_core::sense::{ActorSense, Sensors, TouchSense, UiSense};
use blockloom_core::sound::SoundBus;
use blockloom_core::tilemap::ParallaxAxis;
use blockloom_core::ui::{UiAnchor, UiProp, UiTheme};
use blockloom_core::value::{Evaluated, Op, Value};
use blockloom_core::vm::{Effect, Event, Vm};
use blockloom_core::water::WaterProperty;
use blockloom_core::wind::WindProperty;
use std::process::Command;

// ─── The world both halves see ──────────────────────────────────────────────
// Small and fixed. The VM reads it from a published snapshot; the compiled
// program is handed the same numbers by the harness below.

const ACTOR: &str = "a1";
const TIMER: f64 = 2.5;
const MY_POSITION: [f32; 3] = [3.0, 7.0, 0.0];
const MY_LOCAL_POSITION: [f64; 3] = [1.0, 2.0, 0.0];
const OTHER_POSITION: [f32; 3] = [10.0, -2.0, 0.0];
const OTHER_LOCAL_POSITION: [f64; 3] = [4.0, -1.0, 0.0];
const CAMERA_POSITION: [f32; 3] = [9.0, 8.0, 7.0];
const CAMERA_DIRECTION: [f32; 3] = [0.0, 0.6, -0.8];
const MOUSE_DELTA: [f32; 2] = [24.0, -9.0];

fn publish_world() {
    let mut sensors = Sensors {
        time: TIMER,
        mouse_delta: MOUSE_DELTA,
        mouse_locked: true,
        ..Default::default()
    };
    sensors.keys.insert("space".to_string());
    sensors.actors.insert(
        ACTOR.to_string(),
        ActorSense {
            name: "Player".to_string(),
            position: MY_POSITION,
            local_position: [
                MY_LOCAL_POSITION[0] as f32,
                MY_LOCAL_POSITION[1] as f32,
                0.0,
            ],
            // The harness player idles: no tween running, holding the Walk
            // clip on its third frame, still playing.
            tweening: false,
            anim_clip: "Walk".to_string(),
            anim_frame: 3,
            anim_playing: true,
            ..Default::default()
        },
    );
    sensors.actors.insert(
        "a2".to_string(),
        ActorSense {
            name: "Friend".to_string(),
            position: OTHER_POSITION,
            local_position: [
                OTHER_LOCAL_POSITION[0] as f32,
                OTHER_LOCAL_POSITION[1] as f32,
                0.0,
            ],
            ..Default::default()
        },
    );
    // Two interface elements, so the reporters over them have something to
    // answer about, and one input holding the keyboard.
    sensors.ui.insert(
        "volume".to_string(),
        UiSense {
            value: Evaluated::Number(4.0),
            text: String::new(),
            shown: true,
        },
    );
    sensors.ui.insert(
        "hint".to_string(),
        UiSense {
            value: Evaluated::Text(String::new()),
            text: "Paused".to_string(),
            // Inside a panel somebody hid: made, remembered, not on screen.
            shown: false,
        },
    );
    sensors.ui_focus = "name".to_string();
    sensors.camera = blockloom_core::sense::CameraSense {
        position: CAMERA_POSITION,
        forward: CAMERA_DIRECTION,
    };
    sensors.mouse_buttons.insert("right".to_string());
    sensors.actions.insert(
        "Jump".to_string(),
        ActionSense {
            held: true,
            pressed: true,
            released: false,
            value: 1.0,
            vector: [1.0, 0.0],
        },
    );
    sensors.actions.insert(
        "Left".to_string(),
        ActionSense {
            held: true,
            pressed: false,
            released: false,
            value: 0.5,
            vector: [0.5, 0.0],
        },
    );
    sensors.touches.push(TouchSense {
        id: 7,
        position: [5.0, 6.0],
    });
    sensors.gamepad_connected = true;
    sensors.gamepad_axes.insert("leftstickx".to_string(), 0.5);
    sensors.gamepad_buttons.insert("south".to_string());
    sensors.atmosphere.wind_speed = 3.0;
    sensors.atmosphere.time_of_day = 6.5;
    sensors.atmosphere.weather = "Storm".to_string();
    sensors.atmosphere.sun_direction = [0.0, 0.5, -0.8660254];
    sensors.atmosphere.luminance = 42.0;
    sensors.atmosphere.hdr_display = true;
    sensors.atmosphere.peak_brightness = 600.0;
    sensors.atmosphere.ray_tracing = true;
    sensors.atmosphere.ray_tracing_available = true;
    sensors.atmosphere.volumes = vec!["Cave".to_string()];
    sensors.current_scene = "Scene 1".to_string();
    sensors.scene_names = vec!["Scene 1".to_string(), "Scene 2".to_string()];
    sensors.current_save_slot = "default".to_string();
    sensors.save_slots = vec!["default".to_string()];
    sensors.language = "en".to_string();
    sensors.cutscene_name = "Opener".to_string();
    sensors.cutscene_time = 4.25;
    blockloom_core::sense::publish(sensors);
}

/// The same answers, written as Rust for the compiled half to be handed.
const HARNESS: &str = r#"
use std::collections::HashMap;

struct Recorder {
    vars: HashMap<String, Val>,
    lists: HashMap<String, Vec<Val>>,
    dicts: HashMap<String, Vec<(String, Val)>>,
    /// Which tick is being run, so every line says when it happened and not
    /// just what order things came in.
    tick: usize,
    out: Vec<String>,
    /// The last physics query: where its one hit stood, how big the ball was
    /// (or the range), and whether triggers were asked for.
    query: Option<([f64; 3], f64, bool)>,
    /// The last controller move's flags, with 8 added when it ended grounded.
    controller: u32,
}

impl Host for Recorder {
    fn act(&mut self, actor: &str, act: Act) {
        // List writes are the program's own state, like variables: they
        // land silently rather than as transcript lines, and a name nothing
        // declared is a no-op.
        match &act {
            // The harness world answers every query with one wall, standing
            // at the point the query was aimed at.
            Act::Query { kind, triggers, numbers } => {
                let at = |i: usize| numbers.get(i).copied().unwrap_or(0.0);
                let (point, size) = match *kind {
                    "ray" | "rays" => ([at(3), at(4), at(5)], 0.0),
                    "ball cast" => ([at(4), at(5), at(6)], at(0)),
                    _ => ([at(1), at(2), at(3)], at(0)),
                };
                self.query = Some((point.map(|n| n as f32 as f64), size as f32 as f64, *triggers == "Include"));
            }
            // The harness controller stands one metre over a floor: a
            // plain move with any downward part lands on it, a simple move's
            // gravity is too small to reach it.
            Act::Controller { op, vector } => {
                if !op.starts_with("set ") {
                    self.controller = if *op == "move" && vector[1] < 0.0 { 12 } else { 0 };
                }
            }
            Act::AddToList { name, value } => {
                match list_item(value) {
                    Some(item) => {
                        if let Some(list) = self.lists.get_mut(*name) {
                            list.push(item);
                        }
                    }
                    None => self.error(actor, "list items must be number or text"),
                }
                return;
            }
            Act::DeleteOfList { name, index } => {
                if let Some(list) = self.lists.get_mut(*name) {
                    if let Some(at) = list_index_pos(*index, list.len(), false) {
                        list.remove(at);
                    }
                }
                return;
            }
            Act::DeleteAllOfList { name } => {
                if let Some(list) = self.lists.get_mut(*name) {
                    list.clear();
                }
                return;
            }
            Act::ShiftList { name, amount } => {
                if let Some(list) = self.lists.get_mut(*name) {
                    if !list.is_empty() {
                        let len = list.len();
                        let distance = amount.round() as isize;
                        if distance >= 0 {
                            list.rotate_right(distance as usize % len);
                        } else {
                            list.rotate_left(distance.unsigned_abs() % len);
                        }
                    }
                }
                return;
            }
            Act::InsertIntoList { name, index, value } => {
                match list_item(value) {
                    Some(item) => {
                        if let Some(list) = self.lists.get_mut(*name) {
                            if let Some(at) = list_index_pos(*index, list.len(), true) {
                                list.insert(at, item);
                            }
                        }
                    }
                    None => self.error(actor, "list items must be number or text"),
                }
                return;
            }
            Act::ReplaceItemOfList { name, index, value } => {
                match list_item(value) {
                    Some(item) => {
                        if let Some(list) = self.lists.get_mut(*name) {
                            if let Some(at) = list_index_pos(*index, list.len(), false) {
                                list[at] = item;
                            }
                        }
                    }
                    None => self.error(actor, "list items must be number or text"),
                }
                return;
            }
            Act::ReverseList { name } => {
                if let Some(list) = self.lists.get_mut(*name) {
                    list.reverse();
                }
                return;
            }
            Act::SetDictValue { name, key, value } => {
                match dict_item(value) {
                    Some(item) => {
                        if let Some(dict) = self.dicts.get_mut(*name) {
                            dict_set_entry(dict, key.clone(), item);
                        }
                    }
                    None => self.error(actor, "dict values must be number or text"),
                }
                return;
            }
            Act::DeleteDictKey { name, key } => {
                if let Some(dict) = self.dicts.get_mut(*name) {
                    dict.retain(|entry| &entry.0 != key);
                }
                return;
            }
            Act::DeleteAllOfDict { name } => {
                if let Some(dict) = self.dicts.get_mut(*name) {
                    dict.clear();
                }
                return;
            }
            Act::LoadJsonIntoDict { name, json } => {
                match parse_json_object(&json.as_text()) {
                    Ok(entries) => {
                        if let Some(dict) = self.dicts.get_mut(*name) {
                            *dict = entries;
                        }
                    }
                    Err(message) => self.error(actor, &message),
                }
                return;
            }
            Act::LoadJsonIntoList { name, json } => {
                match parse_json_array(&json.as_text()) {
                    Ok(items) => {
                        if let Some(list) = self.lists.get_mut(*name) {
                            *list = items;
                        }
                    }
                    Err(message) => self.error(actor, &message),
                }
                return;
            }
            _ => {}
        }
        // `delete` is recorded against the actor it takes out of the run
        // rather than the one that asked, because that is who the VM's own
        // effect is about.
        let actor = match &act {
            Act::DeleteActor { target } => target.clone(),
            // Window-global, or screen-space: against nobody in particular,
            // which is how the VM's own effects say it.
            Act::SetMouseLocked { .. }
            | Act::RumbleGamepad { .. }
            | Act::ShowElement { .. }
            | Act::SetUiProp { .. }
            | Act::HideElement { .. }
            | Act::DeleteElement { .. }
            | Act::SetFocus { .. }
            | Act::SetUiTheme { .. }
            | Act::SetBusVolume { .. }
            | Act::SetRenderSetting { .. }
            | Act::SetExposure { .. }
            | Act::SetHdrOutput { .. }
            | Act::SetPeakBrightness { .. }
            | Act::CaptureProbes
            | Act::SetShadowDistance { .. }
            | Act::SetRayTracing { .. }
            | Act::SetGiBounces { .. }
            | Act::SetGiSamples { .. }
            | Act::SetFogDensity { .. }
            | Act::SetAurora { .. }
            | Act::StrikeLightning { .. }
            | Act::Splash { .. }
            | Act::PuffSmoke { .. }
            | Act::SpawnDecal { .. }
            | Act::FadeDecals { .. }
            | Act::SetLightningRate { .. }
            | Act::SetWind { .. }
            | Act::SetCloudDrift { .. }
            | Act::SetClouds { .. }
            | Act::SetCloudLayer { .. }
            | Act::SetTimeOfDay { .. }
            | Act::AdvanceTime { .. }
            | Act::SetPrecipitation { .. }
            | Act::BlendWeather { .. }
            | Act::PlayCutscene { .. }
            | Act::SkipCutscene
            | Act::CameraShake { .. }
            | Act::SetTimeScale { .. }
            | Act::Hitstop { .. }
            | Act::SetLetterbox { .. }
            | Act::FadeScreen { .. }
            | Act::SetPaused { .. } => String::new(),
            _ => actor.to_string(),
        };
        let line = line_of(&act);
        self.out.push(format!("{} {actor}|{line}", self.tick));
    }

    fn sense(&mut self, _actor: &str, kind: &str, args: &[Val]) -> R {
        match kind {
            "KeyDown" => Ok(Val::Bool(args[0].as_text().to_lowercase() == "space")),
            "Timer" => Ok(Val::Num(2.5)),
            "MouseDeltaX" => Ok(Val::Num(24.0)),
            "MouseDeltaY" => Ok(Val::Num(-9.0)),
            "MouseLocked" => Ok(Val::Bool(true)),
            "MouseButtonDown" => Ok(Val::Bool(args[0].as_text().to_lowercase() == "right")),
            "ActionDown" => Ok(Val::Bool(args[0].as_text().eq_ignore_ascii_case("Jump"))),
            "ActionPressed" => Ok(Val::Bool(args[0].as_text().eq_ignore_ascii_case("Jump"))),
            "ActionReleased" => Ok(Val::Bool(false)),
            "ActionValue" => Ok(Val::Num(
                if args[0].as_text().eq_ignore_ascii_case("Jump") {
                    1.0
                } else if args[0].as_text().eq_ignore_ascii_case("Left") {
                    0.5
                } else {
                    0.0
                },
            )),
            "TouchCount" => Ok(Val::Num(1.0)),
            "TouchX" => Ok(Val::Num(if args[0].as_number().unwrap_or(0.0) as usize == 1 {
                5.0
            } else {
                0.0
            })),
            "TouchY" => Ok(Val::Num(if args[0].as_number().unwrap_or(0.0) as usize == 1 {
                6.0
            } else {
                0.0
            })),
            "GamepadConnected" => Ok(Val::Bool(true)),
            "GamepadAxis" => Ok(Val::Num(
                if args[0].as_text().to_lowercase().replace([' ', '_', '-'], "").as_str()
                    == "leftstickx"
                {
                    0.5
                } else {
                    0.0
                },
            )),
            "GamepadButtonDown" => Ok(Val::Bool(
                args[0].as_text().to_lowercase().replace([' ', '_', '-'], "") == "south",
            )),
            "UiValue" | "UiSelectedIndex" => match args[0].as_text().as_str() {
                "volume" => Ok(Val::Num(4.0)),
                "hint" => Ok(Val::Text(String::new())),
                other => Err(format!("there's no interface element called \"{other}\"")),
            },
            "UiText" => match args[0].as_text().as_str() {
                "volume" => Ok(Val::Text(String::new())),
                "hint" => Ok(Val::Text("Paused".to_string())),
                other => Err(format!("there's no interface element called \"{other}\"")),
            },
            "UiShown" => Ok(Val::Bool(args[0].as_text() == "volume")),
            "UiExists" => Ok(Val::Bool(matches!(
                args[0].as_text().as_str(),
                "volume" | "hint"
            ))),
            "UiFocus" => Ok(Val::Text("name".to_string())),
            "SceneLuminance" => Ok(Val::Num(42.0)),
            "TimeOfDay" => Ok(Val::Num(6.5)),
            // The same f32 maths the snapshot's own field does, so the two
            // halves print the same digits.
            "SunElevation" => Ok(Val::Num(f32::to_degrees(f32::asin(0.5)) as f64)),
            "CurrentWeather" => Ok(Val::Text("Storm".into())),
            "IsCutscenePlaying" => Ok(Val::Bool(true)),
            "CutsceneTime" => Ok(Val::Num(4.25)),
            "IsHdrDisplay" => Ok(Val::Bool(true)),
            "PeakBrightness" => Ok(Val::Num(600.0)),
            "IsRayTracing" => Ok(Val::Bool(true)),
            "RayTracingAvailable" => Ok(Val::Bool(true)),
            "ActiveVolumes" => Ok(Val::Text("[\"Cave\"]".into())),
            "CurrentScene" => Ok(Val::Text("Scene 1".into())),
            "SceneNames" => Ok(Val::Text("[\"Scene 1\",\"Scene 2\"]".into())),
            // The published snapshot the VM reads above answers the default
            // slot, one slot on disk and English: reporters read the tick's
            // snapshot on both sides, so a switch only shows from the next
            // tick, once the host applies it.
            "SaveSlot" => Ok(Val::Text("default".into())),
            "SaveSlots" => Ok(Val::Text("[\"default\"]".into())),
            "Language" => Ok(Val::Text("en".into())),
            "LocalizedText" => Ok(Val::Text(args[0].as_text())),
            "Atmosphere" => match args[0].as_text().as_str() {
                "wind speed" => Ok(Val::Num(3.0)),
                "time of day" => Ok(Val::Num(6.5)),
                "sun elevation" => Ok(Val::Num(f32::to_degrees(f32::asin(0.5)) as f64)),
                other => Err(format!("the atmosphere has no \"{other}\" reading")),
            },
            // The harness player idles holding Walk on frame 3, mirroring
            // the snapshot the VM reads above.
            "IsTweening" => Ok(Val::Bool(false)),
            "CurrentClip" => Ok(Val::Text("Walk".to_string())),
            "CurrentFrame" => Ok(Val::Num(3.0)),
            "AnimationPlaying" => Ok(Val::Bool(true)),
            "MyPosition" => Ok(Val::Num(axis_of(&args[0], [3.0, 7.0, 0.0]))),
            "CameraPosition" => Ok(Val::Num(axis_of(&args[0], [9.0, 8.0, 7.0]))),
            // The same f32 maths the snapshot's own field does, so the two
            // halves print the same digits.
            "CameraDirection" => Ok(Val::Num(axis_of(
                &args[0],
                [0.0, 0.6f32 as f64, -0.8f32 as f64],
            ))),
            "MyLocalPosition" => Ok(Val::Num(axis_of(&args[0], [1.0, 2.0, 0.0]))),
            "ActorPosition" => {
                let name = args[0].as_text();
                if name != "Friend" {
                    return Err(format!("there's no actor named \"{name}\""));
                }
                Ok(Val::Num(axis_of(&args[1], [10.0, -2.0, 0.0])))
            }
            "ActorLocalPosition" => {
                let name = args[0].as_text();
                if name != "Friend" {
                    return Err(format!("there's no actor named \"{name}\""));
                }
                Ok(Val::Num(axis_of(&args[1], [4.0, -1.0, 0.0])))
            }
            // Nobody is a trigger - mirroring what the
            // VM reads off the same published snapshot.
            "IsTrigger" => {
                let name = args[0].as_text();
                if name == "Player" || name == "Friend" {
                    Ok(Val::Bool(false))
                } else {
                    Err(format!("there's no actor named \"{name}\""))
                }
            }
            // Nobody carries a light either.
            "CastsShadows" => {
                let name = args[0].as_text();
                if name == "Player" || name == "Friend" {
                    Ok(Val::Bool(false))
                } else {
                    Err(format!("there's no actor named \"{name}\""))
                }
            }
            "CollisionLayer" => {
                let name = args[0].as_text();
                if name == "Player" || name == "Friend" {
                    Ok(Val::Num(1.0))
                } else {
                    Err(format!("there's no actor named \"{name}\""))
                }
            }
            "QueryNumber" => {
                let field = args[1].as_text();
                let index = args[0].as_number().unwrap_or(0.0).max(0.0) as usize;
                let hit = if index == 1 { self.query } else { None };
                let (point, size, trigger) = hit.unwrap_or(([0.0; 3], 0.0, false));
                let seen = hit.is_some();
                let flag = |on: bool| if seen && on { 1.0 } else { 0.0 };
                Ok(Val::Num(match field.as_str() {
                    "count" => if self.query.is_some() { 1.0 } else { 0.0 },
                    "overflowed" | "tick" | "part" => 0.0,
                    "x" => point[0],
                    "y" => point[1],
                    "z" => point[2],
                    "normal x" | "normal z" => 0.0,
                    "normal y" => if seen { 1.0 } else { 0.0 },
                    "distance" => point[0] as f32 as f64,
                    "fraction" => size,
                    "started inside" => flag(false),
                    "is trigger" => flag(trigger),
                    _ => return Err(format!("a query result has no numbers called \"{field}\"")),
                }))
            }
            "ControllerNumber" => {
                let field = args[1].as_text();
                Ok(Val::Num(match field.as_str() {
                    "grounded" => f64::from(self.controller >> 3 & 1),
                    "below" => f64::from(self.controller >> 2 & 1),
                    "sides" | "above" => 0.0,
                    "flags" => f64::from(self.controller & 7),
                    _ => 0.0,
                }))
            }
            "JointNumber" => {
                let field = args[0].as_text();
                let name = args[1].as_text();
                Ok(Val::Num(match (name.as_str(), field.as_str()) {
                    ("hinge", "position") => 12.5,
                    ("hinge", "broken") => 0.0,
                    (_, "enabled") => 1.0,
                    _ => 0.0,
                }))
            }
            "ControllerText" => Ok(Val::Text(String::new())),
            "QueryText" => {
                let field = args[1].as_text();
                let index = args[0].as_number().unwrap_or(0.0).max(0.0) as usize;
                let seen = index == 1 && self.query.is_some();
                Ok(Val::Text(match field.as_str() {
                    "actor" | "actor id" if seen => "wall".to_string(),
                    "collider" if seen => "wall:0".to_string(),
                    "actor" | "actor id" | "collider" | "body" | "error" => String::new(),
                    _ => return Err(format!("a query result has no words called \"{field}\"")),
                }))
            }
            // The harness world is one wall standing where each query aims.
            "RayHit" | "CircleHit" => Ok(Val::Text("wall".to_string())),
            "RayDistance" => Ok(Val::Num(args[3].as_number().unwrap_or(0.0) as f32 as f64)),
            // List reporters read the run's lists, unknown names included:
            // nothing declared reads as empty, exactly as it does on the VM.
            "ListItem" => {
                let list = self.lists.get(&args[1].as_text());
                let len = list.map_or(0, Vec::len);
                let index = args[0].as_number().unwrap_or(0.0);
                Ok(list_index_pos(index, len, false)
                    .and_then(|at| list.and_then(|list| list.get(at)).cloned())
                    .unwrap_or(Val::Text(String::new())))
            }
            "ListItemNumber" => {
                let needle = list_item(&args[0])
                    .ok_or_else(|| "list items must be number or text".to_string())?;
                Ok(Val::Num(
                    self.lists
                        .get(&args[1].as_text())
                        .map_or(0, |list| {
                            list.iter().position(|item| item == &needle).map_or(0, |at| at + 1)
                        }) as f64,
                ))
            }
            "ListAmount" => {
                let needle = list_item(&args[0])
                    .ok_or_else(|| "list items must be number or text".to_string())?;
                Ok(Val::Num(
                    self.lists
                        .get(&args[1].as_text())
                        .map_or(0, |list| list.iter().filter(|item| *item == &needle).count())
                        as f64,
                ))
            }
            "ListLength" => Ok(Val::Num(
                self.lists.get(&args[0].as_text()).map_or(0, Vec::len) as f64
            )),
            "ListContains" => {
                let needle = list_item(&args[1])
                    .ok_or_else(|| "list items must be number or text".to_string())?;
                Ok(Val::Bool(
                    self.lists
                        .get(&args[0].as_text())
                        .is_some_and(|list| list.contains(&needle)),
                ))
            }
            "ListItemExists" => {
                let len = self.lists.get(&args[1].as_text()).map_or(0, Vec::len);
                let index = args[0].as_number().unwrap_or(0.0);
                Ok(Val::Bool(list_index_pos(index, len, false).is_some()))
            }
            "ListIsEmpty" => Ok(Val::Bool(
                self.lists.get(&args[0].as_text()).map_or(true, Vec::is_empty),
            )),
            "ListAsJson" => Ok(Val::Text(list_as_json(
                self.lists.get(&args[0].as_text()).map_or(&[], Vec::as_slice),
            ))),
            // Dict reporters read the run's dicts, unknown names included:
            // nothing declared reads as empty, exactly as it does on the VM.
            "DictValue" => Ok(dict_find(
                self.dicts.get(&args[1].as_text()).map_or(&[], Vec::as_slice),
                &args[0].as_text(),
            )
            .cloned()
            .unwrap_or(Val::Text(String::new()))),
            "DictHasKey" => Ok(Val::Bool(
                self.dicts
                    .get(&args[0].as_text())
                    .is_some_and(|dict| dict.iter().any(|entry| entry.0 == args[1].as_text())),
            )),
            "DictSize" => Ok(Val::Num(
                dict_keys(self.dicts.get(&args[0].as_text()).map_or(&[], Vec::as_slice)).len()
                    as f64,
            )),
            "DictIsEmpty" => Ok(Val::Bool(
                self.dicts.get(&args[0].as_text()).map_or(true, Vec::is_empty),
            )),
            "DictKeys" => Ok(Val::Text(keys_as_json(&dict_keys(
                self.dicts.get(&args[0].as_text()).map_or(&[], Vec::as_slice),
            )))),
            "DictAsJson" => Ok(Val::Text(dict_as_json(
                self.dicts.get(&args[0].as_text()).map_or(&[], Vec::as_slice),
            ))),
            // A plugin's module, as the harness answers for one: the same
            // rule sits in `plugin_answer` for the VM's side.
            "PluginRead" => {
                let slots: Vec<String> = args[2..].iter().map(Val::as_text).collect();
                match args[1].as_text().as_str() {
                    "count" => Ok(Val::Num(slots.join("").len() as f64 * 2.0)),
                    "label" => Ok(Val::Text(format!("{}:{}", args[0].as_text(), slots.join("+")))),
                    other => Err(format!("{}/{other} has no reading", args[0].as_text())),
                }
            }
            other => Err(format!("unknown operator '{other}'")),
        }
    }

    fn variable(&mut self, _actor: &str, name: &str) -> Val {
        self.vars.get(name).cloned().unwrap_or(Val::Num(0.0))
    }

    fn set_variable(&mut self, _actor: &str, name: &str, value: Val) {
        self.vars.insert(name.to_string(), value);
    }

    fn error(&mut self, actor: &str, message: &str) {
        self.out.push(format!("{} {actor}|Error {message}", self.tick));
    }
}

fn axis_of(value: &Val, position: [f64; 3]) -> f64 {
    match value.as_text().as_str() {
        "Y" | "y" => position[1],
        "Z" | "z" => position[2],
        _ => position[0],
    }
}

/// A value a list can hold: numbers and text only, the way the VM's own
/// items reject a boolean.
fn list_item(value: &Val) -> Option<Val> {
    match value {
        Val::Num(_) | Val::Text(_) => Some(value.clone()),
        Val::Bool(_) => None,
    }
}

/// A value a dict can hold: the same literal-only rule as lists.
fn dict_item(value: &Val) -> Option<Val> {
    list_item(value)
}

/// Looks up `key`, last write wins when keys repeat.
fn dict_find<'a>(dict: &'a [(String, Val)], key: &str) -> Option<&'a Val> {
    dict.iter().rev().find(|entry| entry.0 == key).map(|entry| &entry.1)
}

/// Sets `key`, replacing the last entry with that key or pushing a new one.
fn dict_set_entry(dict: &mut Vec<(String, Val)>, key: String, value: Val) {
    if let Some(entry) = dict.iter_mut().rev().find(|entry| entry.0 == key) {
        entry.1 = value;
    } else {
        dict.push((key, value));
    }
}

/// Distinct keys in first-seen order, which is what `DictKeys` reports.
fn dict_keys(dict: &[(String, Val)]) -> Vec<String> {
    let mut keys = Vec::new();
    for entry in dict {
        if !keys.contains(&entry.0) {
            keys.push(entry.0.clone());
        }
    }
    keys
}

/// A number as JSON spells it: `3.0` stays `3.0`, never `3`.
fn num_json(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e17 {
        format!("{value:.1}")
    } else {
        format!("{value:?}")
    }
}

/// A string as JSON spells it, with the same escapes `serde_json` writes.
fn str_json(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn val_json(value: &Val) -> String {
    match value {
        Val::Num(n) => num_json(*n),
        Val::Text(s) => str_json(s),
        Val::Bool(_) => "null".to_string(),
    }
}

fn list_as_json(items: &[Val]) -> String {
    let parts: Vec<String> = items.iter().map(val_json).collect();
    format!("[{}]", parts.join(","))
}

fn dict_as_json(dict: &[(String, Val)]) -> String {
    let parts: Vec<String> = dict
        .iter()
        .map(|entry| format!("{}:{}", str_json(&entry.0), val_json(&entry.1)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

fn keys_as_json(keys: &[String]) -> String {
    let parts: Vec<String> = keys.iter().map(|key| str_json(key)).collect();
    format!("[{}]", parts.join(","))
}

/// A flat JSON value: only numbers and strings survive, and anything else
/// (booleans, null, objects, arrays) is `Other` for the caller to refuse
/// with the key or position attached.
enum Flat {
    Num(f64),
    Str(String),
    Other,
}

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.pos < self.text.len() && matches!(self.text[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn lit(&mut self, word: &str) -> bool {
        if self.text[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.text.get(self.pos) != Some(&b'"') {
            return None;
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = *self.text.get(self.pos)?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.pos += 1;
                    match *self.text.get(self.pos)? {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(self.text.get(self.pos + 1..self.pos + 5)?).ok()?;
                            let unit = u32::from_str_radix(hex, 16).ok()?;
                            self.pos += 4;
                            let ch = if (0xD800..0xDC00).contains(&unit) {
                                if self.text.get(self.pos + 1) == Some(&b'\\')
                                    && self.text.get(self.pos + 2) == Some(&b'u')
                                {
                                    let low_hex = std::str::from_utf8(
                                        self.text.get(self.pos + 3..self.pos + 7)?,
                                    )
                                    .ok()?;
                                    let low = u32::from_str_radix(low_hex, 16).ok()?;
                                    if !(0xDC00..0xE000).contains(&low) {
                                        return None;
                                    }
                                    self.pos += 6;
                                    char::from_u32(
                                        0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00),
                                    )?
                                } else {
                                    return None;
                                }
                            } else {
                                char::from_u32(unit)?
                            };
                            out.push(ch);
                        }
                        _ => return None,
                    }
                    self.pos += 1;
                }
                0x00..=0x1F => return None,
                _ => {
                    let rest = std::str::from_utf8(&self.text[self.pos..]).ok()?;
                    let ch = rest.chars().next()?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        if self.text.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.text.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.text.get(self.pos), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return None,
        }
        if self.text.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !matches!(self.text.get(self.pos), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.text.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.text.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.text.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.text.get(self.pos), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.text.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(self.text.get(start..self.pos)?).ok()?;
        text.parse::<f64>().ok()
    }

    fn value(&mut self) -> Option<Flat> {
        self.ws();
        let flat = match self.text.get(self.pos)? {
            b'"' => Flat::Str(self.string()?),
            b'-' | b'0'..=b'9' => Flat::Num(self.number()?),
            b't' => {
                if !self.lit("true") {
                    return None;
                }
                Flat::Other
            }
            b'f' => {
                if !self.lit("false") {
                    return None;
                }
                Flat::Other
            }
            b'n' => {
                if !self.lit("null") {
                    return None;
                }
                Flat::Other
            }
            b'{' | b'[' => Flat::Other,
            _ => return None,
        };
        // Nested structures still have to scan cleanly, or trailing bytes
        // would read as a second value.
        self.ws();
        Some(flat)
    }

    fn skip_nested(&mut self) -> bool {
        let open = match self.text.get(self.pos) {
            Some(b'{') => (b'{', b'}'),
            Some(b'[') => (b'[', b']'),
            _ => return false,
        };
        self.pos += 1;
        let mut depth = 1;
        let mut in_string = false;
        while self.pos < self.text.len() {
            let byte = self.text[self.pos];
            if in_string {
                if byte == b'\\' {
                    self.pos += 1;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == open.0 {
                depth += 1;
            } else if byte == open.1 {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    return true;
                }
            }
            self.pos += 1;
        }
        false
    }

    fn flat_value(&mut self) -> Option<Flat> {
        self.ws();
        match self.text.get(self.pos) {
            Some(b'{') | Some(b'[') => {
                if self.skip_nested() {
                    Some(Flat::Other)
                } else {
                    None
                }
            }
            _ => self.value(),
        }
    }

    fn end(&mut self) -> bool {
        self.ws();
        self.pos == self.text.len()
    }
}

/// Parses a JSON object into dict entries, in document order. Only numbers
/// and strings are valid values - anything else names its key, and anything
/// that is not an object at all is refused whole.
fn parse_json_object(text: &str) -> Result<Vec<(String, Val)>, String> {
    let mut parser = Parser { text: text.as_bytes(), pos: 0 };
    parser.ws();
    if parser.text.get(parser.pos) != Some(&b'{') {
        return Err("that text isn't a JSON object".to_string());
    }
    parser.pos += 1;
    let mut entries = Vec::new();
    parser.ws();
    if parser.text.get(parser.pos) == Some(&b'}') {
        parser.pos += 1;
        return if parser.end() { Ok(entries) } else { Err("that text isn't a JSON object".to_string()) };
    }
    loop {
        parser.ws();
        let Some(key) = parser.string() else {
            return Err("that text isn't a JSON object".to_string());
        };
        parser.ws();
        if parser.text.get(parser.pos) != Some(&b':') {
            return Err("that text isn't a JSON object".to_string());
        }
        parser.pos += 1;
        let Some(flat) = parser.flat_value() else {
            return Err("that text isn't a JSON object".to_string());
        };
        match flat {
            Flat::Num(n) => entries.push((key, Val::Num(n))),
            Flat::Str(s) => entries.push((key, Val::Text(s))),
            Flat::Other => return Err(format!("{key:?} isn't a number or text")),
        }
        parser.ws();
        match parser.text.get(parser.pos) {
            Some(b',') => parser.pos += 1,
            Some(b'}') => {
                parser.pos += 1;
                return if parser.end() { Ok(entries) } else { Err("that text isn't a JSON object".to_string()) };
            }
            _ => return Err("that text isn't a JSON object".to_string()),
        }
    }
}

/// Parses a JSON array into list items, in order. Only numbers and strings
/// are valid elements - anything else names its 1-based position, and
/// anything that is not an array at all is refused whole.
fn parse_json_array(text: &str) -> Result<Vec<Val>, String> {
    let mut parser = Parser { text: text.as_bytes(), pos: 0 };
    parser.ws();
    if parser.text.get(parser.pos) != Some(&b'[') {
        return Err("that text isn't a JSON array".to_string());
    }
    parser.pos += 1;
    let mut items = Vec::new();
    parser.ws();
    if parser.text.get(parser.pos) == Some(&b']') {
        parser.pos += 1;
        return if parser.end() { Ok(items) } else { Err("that text isn't a JSON array".to_string()) };
    }
    loop {
        let Some(flat) = parser.flat_value() else {
            return Err("that text isn't a JSON array".to_string());
        };
        match flat {
            Flat::Num(n) => items.push(Val::Num(n)),
            Flat::Str(s) => items.push(Val::Text(s)),
            Flat::Other => {
                return Err(format!("item {} isn't a number or text", items.len() + 1));
            }
        }
        parser.ws();
        match parser.text.get(parser.pos) {
            Some(b',') => parser.pos += 1,
            Some(b']') => {
                parser.pos += 1;
                return if parser.end() { Ok(items) } else { Err("that text isn't a JSON array".to_string()) };
            }
            _ => return Err("that text isn't a JSON array".to_string()),
        }
    }
}

/// One line per thing done, in the same words the test builds from an effect.
fn line_of(act: &Act) -> String {
    match act {
        Act::Move { steps } => format!("Move {steps:?}"),
        Act::GoTo { position } => format!("GoTo {position:?}"),
        Act::NavigateTo { target, speed } => format!("NavigateTo {target:?} {speed:?}"),
        Act::BurstParticles { count } => format!("BurstParticles {count}"),
        Act::SetEmitterDial { dial, value } => format!("SetEmitterDial {dial} {value:?}"),
        Act::SetTrailEnabled { enabled } => format!("SetTrailEnabled {enabled}"),
        Act::SetEmitterPlaying { playing } => format!("SetEmitterPlaying {playing}"),
        Act::ChangePosition { axis, by } => format!("ChangePosition {axis} {by:?}"),
        Act::Glide {
            seconds,
            target,
            easing,
        } => format!("Glide {seconds:?} {target:?} {easing}"),
        Act::TweenScale {
            factor,
            seconds,
            easing,
        } => format!("TweenScale {factor:?} {seconds:?} {easing}"),
        Act::TweenRotation {
            axis,
            degrees,
            seconds,
            easing,
        } => format!("TweenRotation {axis} {degrees:?} {seconds:?} {easing}"),
        Act::TweenColor {
            color,
            seconds,
            easing,
        } => format!("TweenColor {color} {seconds:?} {easing}"),
        Act::StopTweens => "StopTweens".to_string(),
        Act::PlayAnimation { clip, speed } => format!("PlayAnimation {clip} {speed:?}"),
        Act::StopAnimation => "StopAnimation".to_string(),
        Act::SetAnimationSpeed { speed } => format!("SetAnimationSpeed {speed:?}"),
        Act::FireAnimationTrigger { name } => format!("FireAnimationTrigger {name}"),
        Act::SetRigSlot { slot, attachment } => format!("SetRigSlot {slot} {attachment}"),
        Act::SetSlotTint { slot, color } => format!("SetSlotTint {slot} {color}"),
        Act::SetIkTarget { constraint, x, y } => format!("SetIkTarget {constraint} {x:?} {y:?}"),
        Act::SetSpriteDial { dial, value } => format!("SetSpriteDial {dial} {value:?}"),
        Act::Turn { axis, degrees } => format!("Turn {axis} {degrees:?}"),
        Act::SetScale { factor } => format!("SetScale {factor:?}"),
        Act::SetRenderSetting { setting, value } => format!("SetRenderSetting {setting} {value:?}"),
        Act::SetExposure { ev } => format!("SetExposure {ev:?}"),
        Act::SetLightIntensity { intensity } => format!("SetLightIntensity {intensity:?}"),
        Act::SetEmissiveStrength { strength } => format!("SetEmissiveStrength {strength:?}"),
        Act::SetHdrOutput { enabled } => format!("SetHdrOutput {enabled}"),
        Act::SetPeakBrightness { nits } => format!("SetPeakBrightness {nits:?}"),
        Act::EnableVolume { volume, enabled } => format!("SetVolumeEnabled {volume} {enabled}"),
        Act::SetVolumeWeight { volume, weight } => format!("SetVolumeWeight {volume} {weight:?}"),
        Act::CaptureProbes => "CaptureProbes".to_string(),
        Act::SetShadowDistance { distance } => format!("SetShadowDistance {distance:?}"),
        Act::SetLightShadows { enabled } => format!("SetLightShadows {enabled}"),
        Act::SetRayTracing { enabled } => format!("SetRayTracing {enabled}"),
        Act::SetGiBounces { bounces } => format!("SetGiBounces {bounces:?}"),
        Act::SetGiSamples { samples } => format!("SetGiSamples {samples:?}"),
        Act::SetFogDensity { density } => format!("SetFogDensity {density:?}"),
        Act::SetAurora { kp } => format!("SetAurora {kp:?}"),
        Act::StrikeLightning { at } => format!("StrikeLightning {at:?}"),
        Act::Fracture { target } => format!("Fracture {target}"),
        Act::Splash { values } => format!("Splash {values:?}"),
        Act::PuffSmoke { values } => format!("PuffSmoke {values:?}"),
        Act::SpawnDecal { preset, values } => format!("SpawnDecal {preset} {values:?}"),
        Act::FadeDecals { values } => format!("FadeDecals {values:?}"),
        Act::SetLightningRate { rate } => format!("SetLightningRate {rate:?}"),
        Act::SetWind { property, value } => format!("SetWind {property} {value:?}"),
        Act::SetCloudDrift { drift } => format!("SetCloudDrift {drift:?}"),
        Act::SetClouds { property, value } => format!("SetClouds {property} {value:?}"),
        Act::SetTimeOfDay { time } => format!("SetTimeOfDay {time:?}"),
        Act::AdvanceTime { hours } => format!("AdvanceTime {hours:?}"),
        Act::SetPrecipitation { property, value } => {
            format!("SetPrecipitation {property} {value:?}")
        }
        Act::BlendWeather { weather, seconds } => {
            format!("BlendWeather {weather} {seconds:?}")
        }
        Act::PlayCutscene { cutscene } => format!("PlayCutscene {cutscene}"),
        Act::SkipCutscene => "SkipCutscene".to_string(),
        Act::CameraShake { amount } => format!("CameraShake {amount:?}"),
        Act::SetTimeScale { scale } => format!("SetTimeScale {scale:?}"),
        Act::Hitstop { frames } => format!("Hitstop {frames:?}"),
        Act::SetLetterbox { on } => format!("SetLetterbox {on:?}"),
        Act::FadeScreen { color } => format!("FadeScreen {color}"),
        Act::SetWater { property, value } => format!("SetWater {property} {value:?}"),
        Act::PaintTile { map, tile, x, y, z } => format!(
            "PaintTile {map} {} {x:?} {y:?} {z:?}",
            (tile.floor() as i32).max(-1)
        ),
        Act::SetParallax { layer, axis, value } => format!("SetParallax {layer} {axis} {value:?}"),
        Act::SetCloudLayer {
            layer,
            property,
            value,
        } => format!("SetCloudLayer {layer:?} {property} {value:?}"),
        Act::Say { text } => format!("Say {text}"),
        Act::SetColor { color } => format!("SetColor {color}"),
        Act::SetVisible { visible } => format!("SetVisible {visible}"),
        Act::SetMouseLocked { locked } => format!("SetMouseLocked {locked}"),
        Act::SetCameraPitch { degrees } => format!("SetCameraPitch {degrees:?}"),
        Act::SetCameraFov { fov } => format!("SetCameraFov {fov:?}"),
        Act::SetComponentField { component, field, value } => {
            format!("SetComponentField {component} {field} {}", shown(value))
        }
        Act::AttachComponent { component } => format!("AttachComponent {component}"),
        Act::SetParent { target } => format!("SetParent {target}"),
        Act::CreateClone { of, clone } => format!("CreateClone {clone} {of}"),
        Act::CreateActor { id, name, position } => {
            format!("CreateActor {id} {name} {position:?}")
        }
        Act::DeleteActor { .. } => "DeleteActor".to_string(),
        Act::SwitchScene { scene, transition } => format!("SwitchScene {scene} {transition}"),
        Act::PluginCall {
            plugin,
            block,
            args,
        } => format!("PluginCall {plugin}/{block} {}", plugin_args_json(args)),
        Act::Query { kind, .. } => format!("Query {kind} 1"),
        Act::Controller { op, vector } => format!("Controller {op} {vector:?}"),
        Act::SetBody { body } => format!("SetBody {body}"),
        Act::AddForce {
            mode,
            torque,
            vector,
        } => format!("AddForce {mode} {torque} {vector:?}"),
        Act::SetTrigger { trigger } => format!("SetTrigger {trigger}"),
        Act::SetCollisionLayer { layer } => format!("SetCollisionLayer {layer}"),
        Act::SetCollisionMask { mask } => format!("SetCollisionMask {mask}"),
        Act::ShowElement {
            id,
            kind,
            content,
            anchor,
            offset,
            size,
            parent,
            flag,
            range,
            value,
        } => format!(
            "ShowElement {id} {kind} {content} {anchor} {offset:?} {size:?} \
             {parent} {flag} {range:?} {}",
            shown(value)
        ),
        Act::SetUiProp { id, prop, value } => {
            format!("SetUiProp {id} {prop} {}", shown(value))
        }
        Act::HideElement { id, all } => format!("HideElement {id} {all}"),
        Act::DeleteElement { id } => format!("DeleteElement {id}"),
        Act::SetFocus { id } => format!("SetFocus {id}"),
        Act::SetUiTheme { theme } => format!("SetUiTheme {theme}"),
        Act::SetPaused { paused } => format!("SetPaused {paused}"),
        Act::SaveVariable { name, clear } => format!("SaveVariable {name} {clear}"),
        Act::SwitchSaveSlot { slot } => format!("SwitchSaveSlot {slot}"),
        Act::DeleteSaveSlot { slot } => format!("DeleteSaveSlot {slot}"),
        Act::SetLanguage { language } => format!("SetLanguage {language}"),
        // Buses travel as text on the wire, so both halves spell them the
        // same way: the enum's own name, which is what `SoundBus::name` is.
        Act::PlaySound {
            sound,
            volume,
            pitch,
            loop_,
            bus,
            at,
        } => format!("PlaySound {sound} {volume:?} {pitch:?} {loop_} {bus} {at:?}"),
        Act::StopSound { sound } => format!("StopSound {sound}"),
        Act::SetSoundVolume { sound, volume } => {
            format!("SetSoundVolume {sound} {volume:?}")
        }
        Act::SetSoundPitch { sound, pitch } => format!("SetSoundPitch {sound} {pitch:?}"),
        Act::SetBusVolume { bus, volume } => format!("SetBusVolume {bus} {volume:?}"),
        Act::RumbleGamepad {
            strength,
            duration,
        } => format!("RumbleGamepad {strength:?} {duration:?}"),
        Act::BindAction { action, binding } => format!("BindAction {action} {binding}"),
        Act::ClearActionBindings { action } => format!("ClearActionBindings {action}"),
        other => format!("{other:?}"),
    }
}

fn shown(value: &Val) -> String {
    match value {
        Val::Num(n) => format!("n{n:?}"),
        Val::Text(s) => format!("t{s}"),
        Val::Bool(b) => format!("b{b}"),
    }
}

/// The same schedule the VM runs on: every live strand gets one slice per
/// fixed tick, in the order they started, and the run ends when they are all
/// done or one of them says `stop all`.
fn main() {
    let mut recorder = Recorder { vars: HashMap::new(), lists: HashMap::new(), dicts: HashMap::new(), tick: 0, out: Vec::new(), query: None, controller: 0 };
    for (name, value) in seeded() {
        recorder.vars.insert(name.to_string(), value);
    }
    for (name, items) in seeded_lists() {
        recorder.lists.insert(name.to_string(), items);
    }
    for (name, entries) in seeded_dicts() {
        recorder.dicts.insert(name.to_string(), entries);
    }

    // The program's own scheduler, which is what a built game runs on: one
    // slice per strand per fixed tick, clones started at the top of the tick
    // after they were made, and deleted actors dropped at the end of one.
    let mut runner = Runner::new(SCENES, ACTIVE_SCENE);
    runner.fire("Started", "", "", "");
    runner.fire("UiClicked", "", "resume", "");
    runner.fire("UiEvent", "", "hover\nresume", "");
    // A Walk clip ending on the harness player, beside the green flag: a
    // case with a `when animation ends` strand gets one, and nothing else
    // sees it.
    runner.fire("AnimationEnded", "a1", "Walk", "");
    // The harness player's particles dying, likewise.
    runner.fire("Particles", "a1", "Die", "");
    runner.fire("AnimationMarker", "a1", "Step", "");
    runner.fire("EnteredRoom", "a1", "Cave", "");
    // A storm arriving, beside the green flag: a case with a `when weather
    // becomes` strand gets one, and nothing else sees it.
    runner.fire("Weather", "", "Storm", "");
    // A cutscene signal and end, likewise.
    runner.fire("CutsceneSignal", "", "beat", "");
    runner.fire("CutsceneEnded", "", "Opener", "");
    // A plugin's events, likewise: one for everybody, one for the harness
    // player only, and one whose slot is the number a hat spells another way.
    runner.fire("Plugin", "", "com.example.tally\u{1f}changed\u{1f}coins", "");
    runner.fire("Plugin", "a1", "com.example.tally\u{1f}hit\u{1f}5", "");
    runner.fire("Plugin", "", "com.example.tally\u{1f}spent\u{1f}5.0", "");

    for tick in 0..TICKS {
        recorder.tick = tick;
        // Escape mid-run, matching the VM side tick for tick.
        if tick == 3 {
            runner.fire("Key", "", "escape", "");
        }
        if runner.tick(&mut recorder, tick as f64 * DT) {
            // Everything after this one in the tick is gone too, as the VM
            // has it: `stop all` empties the list where it stands.
            recorder.out.push(format!("{tick} |Stopped"));
            break;
        }
        if !runner.is_running() {
            break;
        }
    }

    for line in &recorder.out {
        println!("{line}");
    }
}
"#;

/// The VM's side of the same transcript.
fn line_of(effect: &Effect) -> Option<String> {
    let line = match effect {
        Effect::Move { actor, steps } => format!("{actor}|Move {steps:?}"),
        Effect::GoTo { actor, position } => format!("{actor}|GoTo {position:?}"),
        Effect::NavigateTo {
            actor,
            target,
            speed,
        } => format!("{actor}|NavigateTo {target:?} {speed:?}"),
        Effect::BurstParticles { actor, count } => format!("{actor}|BurstParticles {count}"),
        Effect::SetEmitterDial { actor, dial, value } => {
            format!("{actor}|SetEmitterDial {dial:?} {value:?}")
        }
        Effect::SetTrailEnabled { actor, enabled } => format!("{actor}|SetTrailEnabled {enabled}"),
        Effect::SetEmitterPlaying { actor, playing } => {
            format!("{actor}|SetEmitterPlaying {playing}")
        }
        Effect::ChangePosition { actor, axis, by } => {
            format!("{actor}|ChangePosition {} {by:?}", axis.index())
        }
        Effect::Glide {
            actor,
            seconds,
            target,
            easing,
        } => format!("{actor}|Glide {seconds:?} {target:?} {easing:?}"),
        Effect::TweenScale {
            actor,
            factor,
            seconds,
            easing,
        } => format!("{actor}|TweenScale {factor:?} {seconds:?} {easing:?}"),
        Effect::TweenRotation {
            actor,
            axis,
            degrees,
            seconds,
            easing,
        } => format!(
            "{actor}|TweenRotation {} {degrees:?} {seconds:?} {easing:?}",
            axis.index()
        ),
        Effect::TweenColor {
            actor,
            color,
            seconds,
            easing,
        } => format!("{actor}|TweenColor {color} {seconds:?} {easing:?}"),
        Effect::StopTweens { actor } => format!("{actor}|StopTweens"),
        Effect::PlayAnimation { actor, clip, speed } => {
            format!("{actor}|PlayAnimation {clip} {speed:?}")
        }
        Effect::StopAnimation { actor } => format!("{actor}|StopAnimation"),
        Effect::SetAnimationSpeed { actor, speed } => {
            format!("{actor}|SetAnimationSpeed {speed:?}")
        }
        Effect::FireAnimationTrigger { actor, name } => {
            format!("{actor}|FireAnimationTrigger {name}")
        }
        Effect::SetRigSlot {
            actor,
            slot,
            attachment,
        } => format!("{actor}|SetRigSlot {slot} {attachment}"),
        Effect::SetSlotTint { actor, slot, color } => {
            format!("{actor}|SetSlotTint {slot} {color}")
        }
        Effect::SetIkTarget {
            actor,
            constraint,
            x,
            y,
        } => format!("{actor}|SetIkTarget {constraint} {x:?} {y:?}"),
        Effect::SetSpriteDial { actor, dial, value } => {
            format!("{actor}|SetSpriteDial {dial:?} {value:?}")
        }
        // Nobody's effect in particular: the run itself ending.
        Effect::Stopped => "|Stopped".to_string(),
        Effect::Turn {
            actor,
            axis,
            degrees,
        } => format!("{actor}|Turn {} {degrees:?}", axis.index()),
        Effect::SetScale { actor, factor } => format!("{actor}|SetScale {factor:?}"),
        Effect::SetRenderSetting { setting, value } => {
            format!("|SetRenderSetting {setting:?} {value:?}")
        }
        Effect::SetExposure { ev } => format!("|SetExposure {ev:?}"),
        Effect::SetLightIntensity { actor, intensity } => {
            format!("{actor}|SetLightIntensity {intensity:?}")
        }
        Effect::SetEmissiveStrength { actor, strength } => {
            format!("{actor}|SetEmissiveStrength {strength:?}")
        }
        Effect::SetHdrOutput { enabled } => format!("|SetHdrOutput {enabled}"),
        Effect::SetPeakBrightness { nits } => format!("|SetPeakBrightness {nits:?}"),
        Effect::SetVolumeEnabled {
            actor,
            volume,
            enabled,
        } => format!("{actor}|SetVolumeEnabled {volume} {enabled}"),
        Effect::SetVolumeWeight {
            actor,
            volume,
            weight,
        } => format!("{actor}|SetVolumeWeight {volume} {weight:?}"),
        Effect::CaptureProbes => "|CaptureProbes".to_string(),
        Effect::SetShadowDistance { distance } => format!("|SetShadowDistance {distance:?}"),
        Effect::SetLightShadows { actor, enabled } => {
            format!("{actor}|SetLightShadows {enabled}")
        }
        Effect::SetRayTracing { enabled } => format!("|SetRayTracing {enabled}"),
        Effect::SetGiBounces { bounces } => format!("|SetGiBounces {bounces:?}"),
        Effect::SetGiSamples { samples } => format!("|SetGiSamples {samples:?}"),
        Effect::SetFogDensity { density } => format!("|SetFogDensity {density:?}"),
        Effect::SetAurora { kp } => format!("|SetAurora {kp:?}"),
        Effect::StrikeLightning { at } => format!("|StrikeLightning {at:?}"),
        Effect::Fracture { actor } => format!("{ACTOR}|Fracture {actor}"),
        Effect::Splash {
            at,
            radius,
            strength,
        } => format!("|Splash {:?}", [at[0], at[1], at[2], *radius, *strength]),
        Effect::PuffSmoke {
            at,
            radius,
            strength,
        } => format!("|PuffSmoke {:?}", [at[0], at[1], at[2], *radius, *strength]),
        Effect::SpawnDecal(s) => format!(
            "|SpawnDecal {:?} {:?}",
            s.preset,
            [
                s.at[0],
                s.at[1],
                s.at[2],
                s.normal[0],
                s.normal[1],
                s.normal[2],
                s.size,
                s.lifetime,
                s.fade
            ]
        ),
        Effect::FadeDecals {
            at,
            radius,
            seconds,
        } => format!("|FadeDecals {:?}", [at[0], at[1], at[2], *radius, *seconds]),
        Effect::SetLightningRate { rate } => format!("|SetLightningRate {rate:?}"),
        Effect::SetWind { property, value } => {
            format!("|SetWind {} {value:?}", property.name())
        }
        Effect::SetCloudDrift { drift } => format!("|SetCloudDrift {drift:?}"),
        Effect::SetClouds { property, value } => {
            format!("|SetClouds {} {value:?}", property.name())
        }
        Effect::SetTimeOfDay { time } => format!("|SetTimeOfDay {time:?}"),
        Effect::AdvanceTime { hours } => format!("|AdvanceTime {hours:?}"),
        Effect::SetPrecipitation { property, value } => {
            format!("|SetPrecipitation {} {value:?}", property.name())
        }
        Effect::BlendWeather { weather, seconds } => {
            format!("|BlendWeather {weather} {seconds:?}")
        }
        Effect::PlayCutscene { cutscene } => format!("|PlayCutscene {cutscene}"),
        Effect::SkipCutscene => "|SkipCutscene".to_string(),
        Effect::CameraShake { amount } => format!("|CameraShake {amount:?}"),
        Effect::SetTimeScale { scale } => format!("|SetTimeScale {scale:?}"),
        Effect::Hitstop { frames } => format!("|Hitstop {frames:?}"),
        Effect::SetLetterbox { on } => format!("|SetLetterbox {on:?}"),
        Effect::FadeScreen { color } => format!("|FadeScreen {color}"),
        Effect::SetWater {
            actor,
            property,
            value,
        } => format!("{actor}|SetWater {} {value:?}", property.name()),
        Effect::PaintTile {
            actor,
            map,
            tile,
            x,
            y,
            z,
        } => format!("{actor}|PaintTile {map} {tile} {x:?} {y:?} {z:?}"),
        Effect::SetParallax {
            actor,
            layer,
            axis,
            value,
        } => format!("{actor}|SetParallax {layer} {} {value:?}", axis.name()),
        Effect::SetCloudLayer {
            layer,
            property,
            value,
        } => format!("|SetCloudLayer {layer:?} {} {value:?}", property.name()),
        Effect::Say { actor, text } => format!("{actor}|Say {text}"),
        Effect::SetColor { actor, color } => format!("{actor}|SetColor {color}"),
        Effect::SetVisible { actor, visible } => format!("{actor}|SetVisible {visible}"),
        Effect::SetComponentField {
            actor,
            component,
            field,
            value,
        } => format!(
            "{actor}|SetComponentField {component} {field} {}",
            shown(value)
        ),
        Effect::AttachComponent { actor, component } => {
            format!("{actor}|AttachComponent {component}")
        }
        Effect::SetParent { actor, parent } => format!("{actor}|SetParent {parent}"),
        Effect::CreateClone { actor, clone, of } => format!("{actor}|CreateClone {clone} {of}"),
        Effect::CreateActor {
            actor,
            id,
            name,
            position,
        } => format!("{actor}|CreateActor {id} {name} {position:?}"),
        // Against the actor it takes out of the run, which is the one thing
        // both halves say about it.
        Effect::DeleteActor { actor } => format!("{actor}|DeleteActor"),
        Effect::SwitchScene {
            actor,
            scene,
            transition,
        } => format!("{actor}|SwitchScene {scene} {transition}"),
        Effect::PluginCall {
            actor,
            plugin,
            block,
            args,
        } => format!(
            "{actor}|PluginCall {plugin}/{block} {}",
            serde_json::Value::Array(args.clone())
        ),
        Effect::PhysicsQuery { actor, kind, hits } => format!("{actor}|Query {kind} {hits}"),
        Effect::Controller {
            actor, op, vector, ..
        } => format!("{actor}|Controller {op} {vector:?}"),
        Effect::SetBody { actor, body } => format!("{actor}|SetBody {body:?}"),
        Effect::AddForce {
            actor,
            mode,
            torque,
            vector,
        } => format!("{actor}|AddForce {} {torque} {vector:?}", mode.name()),
        Effect::SetTrigger { actor, trigger } => format!("{actor}|SetTrigger {trigger}"),
        Effect::SetCollisionLayer { actor, layer } => format!("{actor}|SetCollisionLayer {layer}"),
        Effect::SetCollisionMask { actor, mask } => format!("{actor}|SetCollisionMask {mask}"),
        Effect::Error { actor, message } => format!("{actor}|Error {message}"),
        // The world's own doing rather than the program's, and nothing the
        // compiled half is asked to produce.
        Effect::SetGravity { .. } => return None,
        // Window-global, so against nobody: the harness blanks the actor
        // for this act the same way.
        Effect::SetMouseLocked { locked } => format!("|SetMouseLocked {locked}"),
        Effect::SetCameraPitch { actor, degrees } => {
            format!("{actor}|SetCameraPitch {degrees:?}")
        }
        Effect::SetCameraFov { actor, fov } => {
            format!("{actor}|SetCameraFov {fov:?}")
        }
        // Screen-space, so against nobody: the harness blanks the actor for
        // these acts the same way.
        Effect::ShowElement { element } => format!(
            "|ShowElement {} {} {} {} {:?} {:?} {} {} {:?} {}",
            element.id,
            element.kind.index(),
            element.content,
            element.anchor.index(),
            element.offset,
            element.size,
            element.parent,
            element.modal,
            element.range,
            shown(&element.value)
        ),
        Effect::SetUiProp { id, prop, value } => {
            format!("|SetUiProp {id} {} {}", prop.name(), shown(value))
        }
        Effect::HideElement { id, all } => format!("|HideElement {id} {all}"),
        Effect::DeleteElement { id } => format!("|DeleteElement {id}"),
        Effect::SetFocus { id } => format!("|SetFocus {id}"),
        Effect::SetUiTheme { theme } => format!("|SetUiTheme {}", theme.index()),
        Effect::SetPaused { paused } => format!("|SetPaused {paused}"),
        Effect::SetBusVolume { bus, volume } => {
            format!("|SetBusVolume {} {volume:?}", bus.name())
        }
        Effect::RumbleGamepad { strength, duration } => {
            format!("|RumbleGamepad {strength:?} {duration:?}")
        }
        Effect::BindAction {
            actor,
            action,
            binding,
        } => format!("{actor}|BindAction {action} {binding}"),
        Effect::ClearActionBindings { actor, action } => {
            format!("{actor}|ClearActionBindings {action}")
        }
        Effect::PlaySound {
            actor,
            sound,
            volume,
            pitch,
            loop_,
            bus,
            at,
        } => format!(
            "{actor}|PlaySound {sound} {volume:?} {pitch:?} {loop_} {} {at:?}",
            bus.name()
        ),
        Effect::StopSound { actor, sound } => format!("{actor}|StopSound {sound}"),
        Effect::SetSoundVolume {
            actor,
            sound,
            volume,
        } => format!("{actor}|SetSoundVolume {sound} {volume:?}"),
        Effect::SetSoundPitch {
            actor,
            sound,
            pitch,
        } => {
            format!("{actor}|SetSoundPitch {sound} {pitch:?}")
        }
        Effect::SaveVariable { actor, name, clear } => {
            format!("{actor}|SaveVariable {name} {clear}")
        }
        Effect::SwitchSaveSlot { actor, slot } => {
            format!("{actor}|SwitchSaveSlot {slot}")
        }
        Effect::DeleteSaveSlot { actor, slot } => {
            format!("{actor}|DeleteSaveSlot {slot}")
        }
        Effect::SetLanguage { actor, language } => {
            format!("{actor}|SetLanguage {language}")
        }
        other => panic!("this test has no line for {other:?}"),
    };
    Some(line)
}

fn shown(value: &Evaluated) -> String {
    match value {
        Evaluated::Number(n) => format!("n{n:?}"),
        Evaluated::Text(s) => format!("t{s}"),
        Evaluated::Bool(b) => format!("b{b}"),
    }
}

// ─── Building the two halves ────────────────────────────────────────────────

/// The clock both halves step on, and how long a case is given to finish.
/// Neither number matters in itself - what matters is that the two sides
/// compute the very same `now` from the very same tick.
const DT: f64 = 1.0 / 60.0;
const TICKS: usize = 200;

/// A custom block: what a call names it, the inputs it declares, and the
/// body those inputs are read in.
struct Block {
    id: &'static str,
    inputs: &'static [&'static str],
    body: Vec<K>,
}

fn block(id: &'static str, inputs: &'static [&'static str], body: Vec<K>) -> Block {
    Block { id, inputs, body }
}

/// A reporter-shaped call, which resolves to whatever the body returns.
fn call(id: &str, args: Vec<Value>) -> Value {
    Value::Call {
        block_id: id.to_string(),
        args,
        branches: Vec::new(),
        // What the editor shows while the call isn't being run; never read.
        saved: Box::new(Value::number(0.0)),
    }
}

/// One actor: a green-flag strand per body, and a custom block per
/// definition. Several strands on one actor start in the order they're
/// written on both sides, which is what makes them worth comparing; several
/// actors wouldn't, since the VM finds those through a hash map.
fn project_with_blocks(
    bodies: Vec<Vec<K>>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) -> Project {
    let headed = bodies
        .into_iter()
        .map(|body| (K::WhenStarted, body))
        .collect();
    project_with_headers(headed, blocks, globals)
}

/// The same, with each strand's own header: `when I start as a clone` is a
/// header like any other, and a clone's strands are the point of these cases.
fn project_with_headers(
    strands: Vec<(K, Vec<K>)>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) -> Project {
    let mut actor = Actor::new(
        "Player",
        Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [10.0, 10.0],
        },
    );
    actor.id = ACTOR.to_string();
    actor.graph.strands = strands
        .into_iter()
        .enumerate()
        .map(|(index, (header, body))| {
            let mut instructions = vec![Instruction::new(header)];
            instructions.extend(body.into_iter().map(Instruction::new));
            Strand::with_instructions(0, index as i32 * 400, instructions)
        })
        .collect();

    // A custom block is a prototype plus a strand headed by it: that header
    // is how the VM finds the body, and where a call jumps to.
    for (index, block) in blocks.into_iter().enumerate() {
        let mut instructions = vec![Instruction::new(K::BlockHeader {
            block_id: block.id.to_string(),
        })];
        instructions.extend(block.body.into_iter().map(Instruction::new));
        actor.graph.strands.push(Strand::with_instructions(
            600,
            index as i32 * 400,
            instructions,
        ));
        let mut pieces = vec![BlockPiece::Label {
            id: format!("{}-label", block.id),
            text: block.id.to_string(),
        }];
        pieces.extend(block.inputs.iter().map(|name| BlockPiece::Input {
            id: format!("{}-{name}", block.id),
            name: name.to_string(),
            value_type: InputValueType::Any,
        }));
        actor.graph.block_defs.push(BlockDef {
            id: block.id.to_string(),
            pieces,
            shape: BlockShape::Normal,
            color: blockloom_core::blocks::default_block_color(),
        });
    }

    let scene = blockloom_core::project::Scene {
        id: "s1".to_string(),
        name: "Scene 1".to_string(),
        path: "assets/scenes/Scene 1.blockscene".to_string(),
        world: blockloom_core::scene::World {
            mode: Mode::TwoD,
            ..Default::default()
        },
        actors: vec![actor],
    };
    Project {
        id: "p".to_string(),
        name: "differential".to_string(),
        icon: String::new(),
        android: Default::default(),
        scenes: vec![scene],
        active_scene: "s1".to_string(),
        default_scene: "s1".to_string(),
        globals: globals
            .iter()
            .map(|(name, value)| VariableDef {
                name: name.to_string(),
                value: value.clone(),
            })
            .collect(),
        global_lists: Vec::new(),
        global_dicts: Vec::new(),
        plugin_resources: Vec::new(),
        physics: Default::default(),
        multiplayer: Default::default(),
        localization: Default::default(),
    }
}

/// What the VM does with it, as transcript lines.
/// Answers every physics query with one wall at the point it was aimed at, as
/// the compiled harness does.
struct WallWorld;

impl blockloom_core::physics::query::QueryService for WallWorld {
    fn run(
        &self,
        request: &blockloom_core::physics::query::QueryRequest,
        filter: &blockloom_core::physics::query::QueryFilter,
        limit: usize,
    ) -> blockloom_core::physics::query::QueryOutcome {
        use blockloom_core::physics::query::{QueryHit, QueryOutcome, QueryRequest, QueryShape};
        let size = |shape: &QueryShape| match shape {
            QueryShape::Ball { radius } => *radius,
            _ => 0.0,
        };
        let (point, size) = match request {
            QueryRequest::Ray { to, .. } => (*to, 0.0),
            QueryRequest::Cast { shape, to, .. } => (*to, size(shape)),
            QueryRequest::Overlap { shape, at } => (*at, size(shape)),
            QueryRequest::Closest {
                point,
                max_distance,
            } => (*point, *max_distance),
        };
        let hit = QueryHit {
            actor: "wall".into(),
            body: None,
            collider: "wall:0".into(),
            subshape: 0,
            point,
            normal: [0.0, 1.0, 0.0],
            distance: point[0],
            fraction: size,
            started_inside: false,
            trigger: filter.triggers == blockloom_core::physics::query::TriggerPolicy::Include,
        };
        QueryOutcome::finish(vec![hit], limit)
    }
}

/// A controller standing a metre over flat ground, as the compiled harness
/// models it.
struct FloorWorld;

impl blockloom_core::physics::controller::ControllerService for FloorWorld {
    fn step(
        &self,
        r: &blockloom_core::physics::controller::StepRequest,
    ) -> Result<blockloom_core::physics::controller::StepOutcome, String> {
        use blockloom_core::physics::controller::{RawHit, StepOutcome};
        let foot = r.pending[1] + r.spec.center[1] - r.spec.height * 0.5;
        let mut effective = r.displacement;
        let mut hits = Vec::new();
        if foot + effective[1] < 0.0 {
            effective[1] = -foot;
            hits.push(RawHit {
                actor: "ground".into(),
                body: None,
                collider: "ground".into(),
                point: [0.0; 3],
                normal: [0.0, 1.0, 0.0],
                applied: effective,
            });
        }
        Ok(StepOutcome {
            grounded: foot + effective[1] <= 1e-4,
            effective,
            hits,
            ..StepOutcome::default()
        })
    }

    fn gravity(&self) -> [f32; 3] {
        [0.0, -10.0, 0.0]
    }

    fn timestep(&self) -> f32 {
        0.1
    }

    fn mode(&self) -> blockloom_core::scene::Mode {
        blockloom_core::scene::Mode::ThreeD
    }
}

fn by_vm(project: &Project) -> Vec<String> {
    use blockloom_core::physics::controller;
    blockloom_core::physics::query::reset();
    controller::reset();
    let mut spec = controller::CharacterControllerSpec::default();
    spec.center = [0.0, 2.0, 0.0];
    controller::register(ACTOR, spec);
    blockloom_core::physics::query::with_service(&WallWorld, 0, || {
        controller::with_service(&FloorWorld, 0, || by_vm_ticks(project))
    })
}

fn by_vm_ticks(project: &Project) -> Vec<String> {
    blockloom_core::init();
    publish_world();
    // The module the harness answers for, so a plugin reporter reads the
    // same on both sides.
    blockloom_core::sense::set_plugin_reader(Some(Box::new(plugin_answer)));
    let mut vm = Vm::new();
    vm.load(project);
    vm.fire(Event::Started);
    // The interface's own click, beside the green flag: a case with a
    // `when (resume) clicked` strand gets one that keeps running while the
    // world is frozen. No other case has that hat, so nothing else sees it.
    vm.fire(Event::UiClicked {
        id: "resume".to_string(),
    });
    vm.fire(Event::UiEvent {
        id: "resume".into(),
        event: "hover".into(),
    });
    // A Walk clip ending on the harness player, beside the green flag: a
    // case with a `when animation ends` strand gets one, like above.
    vm.fire(Event::AnimationEnded {
        actor: ACTOR.to_string(),
        clip: "Walk".to_string(),
    });
    vm.fire(Event::Particles {
        actor: ACTOR.to_string(),
        event: blockloom_core::vfx::ParticleEvent::Die,
    });
    vm.fire(Event::AnimationMarker {
        actor: ACTOR.to_string(),
        marker: "Step".to_string(),
    });
    vm.fire(Event::EnteredRoom {
        actor: ACTOR.to_string(),
        room: "Cave".to_string(),
    });
    // A storm arriving, beside the green flag: a case with a `when weather
    // becomes` strand gets one, like above.
    vm.fire(Event::Weather {
        weather: "Storm".to_string(),
    });
    // A cutscene signal and end, likewise: a case with a `when cutscene`
    // strand gets them, and nothing else sees them.
    vm.fire(Event::CutsceneSignal {
        signal: "beat".to_string(),
    });
    vm.fire(Event::CutsceneEnded {
        cutscene: "Opener".to_string(),
    });
    // A plugin's events: the harness's other half fires the same three.
    for (actor, event, arg) in [
        (None, "changed", "coins"),
        (Some(ACTOR), "hit", "5"),
        (None, "spent", "5.0"),
    ] {
        vm.fire(Event::Plugin {
            plugin: "com.example.tally".to_string(),
            event: event.to_string(),
            args: vec![arg.to_string()],
            actor: actor.map(str::to_string),
        });
    }
    let mut lines = Vec::new();
    for tick in 0..TICKS {
        // Escape mid-run, so a case with a pause menu can toggle on it. No
        // other case has that hat, so nothing else sees it.
        if tick == 3 {
            vm.fire(Event::Key("escape".to_string()));
        }
        let mut effects = Vec::new();
        vm.tick(tick as f64 * DT, &mut effects);
        // The tick each line landed on, not just the order they came in: a
        // loop that forgot to yield gets everything right but the when.
        lines.extend(
            effects
                .iter()
                .filter_map(line_of)
                .map(|line| format!("{tick} {line}")),
        );
        if !vm.is_running() {
            break;
        }
    }
    lines
}

/// The harness's plugin: `count` is twice the slots' length, `label` joins
/// them, and anything else has no reading. Mirrors `Recorder::sense`.
fn plugin_answer(plugin: &str, block: &str, args: &[Evaluated]) -> Result<Evaluated, String> {
    let slots: Vec<String> = args.iter().map(Evaluated::as_text).collect();
    match block {
        "count" => Ok(Evaluated::Number(slots.join("").len() as f64 * 2.0)),
        "label" => Ok(Evaluated::Text(format!("{plugin}:{}", slots.join("+")))),
        other => Err(format!("{plugin}/{other} has no reading")),
    }
}

/// What the compiled program does with it, as the same lines.
fn by_compiler(project: &Project, globals: &[(&str, Evaluated)], case: &str) -> Vec<String> {
    let source = blockloom_core::codegen::compile(project).expect("this project compiles");
    let seed: Vec<String> = globals
        .iter()
        .map(|(name, value)| format!("({name:?}, {})", as_val(value)))
        .collect();
    // The harness's lists start where the document says, which is what the
    // VM loads out of the same document.
    let list_seed: Vec<String> = project
        .actors
        .iter()
        .flat_map(|actor| actor.graph.lists.iter())
        .chain(project.global_lists.iter())
        .map(|list| {
            let items = list
                .items
                .iter()
                .map(|item| match item {
                    ListItem::Number(n) => format!("Val::Num({n:?}f64)"),
                    ListItem::Text(s) => format!("Val::Text({s:?}.to_string())"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({:?}, vec![{items}])", list.name)
        })
        .collect();
    // The harness's dicts start where the document says too.
    let dict_seed: Vec<String> = project
        .actors
        .iter()
        .flat_map(|actor| actor.graph.dicts.iter())
        .chain(project.global_dicts.iter())
        .map(|dict| {
            let entries = dict
                .entries
                .iter()
                .map(|entry| {
                    let value = match &entry.value {
                        DictItem::Number(n) => format!("Val::Num({n:?}f64)"),
                        DictItem::Text(s) => format!("Val::Text({s:?}.to_string())"),
                    };
                    format!("({:?}.to_string(), {value})", entry.key)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({:?}, vec![{entries}])", dict.name)
        })
        .collect();
    // The clock, and what the project's variables start at - which is what
    // the VM loads out of `globals`. The harness's `main` reads both.
    let source = format!(
        "{source}\n{HARNESS}\nfn seeded() -> Vec<(&'static str, Val)> {{ vec![{}] }}\n\
         fn seeded_lists() -> Vec<(&'static str, Vec<Val>)> {{ vec![{}] }}\n\
         fn seeded_dicts() -> Vec<(&'static str, Vec<(String, Val)>)> {{ vec![{}] }}\n\
         const DT: f64 = {DT:?};\nconst TICKS: usize = {TICKS};\n",
        seed.join(", "),
        list_seed.join(", "),
        dict_seed.join(", ")
    );

    let dir = std::env::temp_dir().join(format!("blockloom-codegen-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a build folder");
    let path = dir.join("program.rs");
    std::fs::write(&path, &source).expect("the generated source");

    let binary = dir.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    let built = Command::new("rustc")
        .arg("--edition")
        .arg("2024")
        .arg("-C")
        .arg("opt-level=0")
        .arg("-o")
        .arg(&binary)
        .arg(&path)
        .output()
        .expect("rustc runs");
    assert!(
        built.status.success(),
        "the generated program didn't compile:\n{}\n\n{source}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Through files rather than pipes, so waiting on it can't deadlock on a
    // full pipe buffer while the deadline below is what we want to hit.
    let printed = dir.join("out.txt");
    let complained = dir.join("err.txt");
    let mut child = Command::new(&binary)
        .stdout(std::fs::File::create(&printed).expect("somewhere to print"))
        .stderr(std::fs::File::create(&complained).expect("somewhere to complain"))
        .spawn()
        .expect("the program runs");

    // A strand that never hands the frame back would spin here instead of
    // failing, and a hung test says far less than a failed one. Every case is
    // a few hundred blocks across `TICKS` slices, so this is wildly generous.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("the program can be waited on") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("{case}: the generated program never finished - a loop that doesn't yield?");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    assert!(
        status.success(),
        "{case}: the generated program failed:\n{}",
        std::fs::read_to_string(&complained).unwrap_or_default()
    );

    let lines = std::fs::read_to_string(&printed)
        .expect("the transcript")
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    lines
}

fn as_val(value: &Evaluated) -> String {
    match value {
        Evaluated::Number(n) => format!("Val::Num({n:?}f64)"),
        Evaluated::Text(s) => format!("Val::Text({s:?}.to_string())"),
        Evaluated::Bool(b) => format!("Val::Bool({b})"),
    }
}

fn toolchain() -> bool {
    blockloom_core::script::toolchain_version().is_ok()
}

fn assert_same(case: &str, body: Vec<K>, globals: &[(&str, Evaluated)]) {
    assert_same_strands(case, vec![body], globals);
}

fn assert_same_strands(case: &str, bodies: Vec<Vec<K>>, globals: &[(&str, Evaluated)]) {
    assert_same_blocks(case, bodies, Vec::new(), globals);
}

fn assert_same_headed(case: &str, strands: Vec<(K, Vec<K>)>) {
    if !toolchain() {
        return;
    }
    assert_project(case, project_with_headers(strands, Vec::new(), &[]), &[]);
}

fn assert_same_blocks(
    case: &str,
    bodies: Vec<Vec<K>>,
    blocks: Vec<Block>,
    globals: &[(&str, Evaluated)],
) {
    if !toolchain() {
        return;
    }
    assert_project(case, project_with_blocks(bodies, blocks, globals), globals);
}

fn assert_project(case: &str, project: Project, globals: &[(&str, Evaluated)]) {
    let interpreted = by_vm(&project);
    let compiled = by_compiler(&project, globals, case);
    assert_eq!(
        interpreted, compiled,
        "{case}: the VM and the compiled program disagreed"
    );
    assert!(!interpreted.is_empty(), "{case}: nothing was compared");
}

// ─── The cases ──────────────────────────────────────────────────────────────

fn number(n: f64) -> Value {
    Value::number(n)
}

fn op(name: &str, args: Vec<Value>) -> Value {
    Value::op(Op::from_name(name), args)
}

#[test]
fn arithmetic_lands_on_the_same_numbers() {
    assert_same(
        "arithmetic",
        vec![
            K::Move {
                steps: op(
                    "Add",
                    vec![number(2.0), op("Mul", vec![number(3.0), number(4.0)])],
                ),
            },
            K::ChangePosition {
                axis: Axis::Y,
                by: op("Div", vec![number(7.0), number(2.0)]),
            },
            K::Turn {
                axis: Axis::Z,
                degrees: op("Mod", vec![number(-90.0), number(360.0)]),
            },
            K::SetScale {
                factor: op("Round", vec![number(1.5)]),
            },
            K::SetExposure {
                ev: op("Sub", vec![number(9.7), number(2.0)]),
            },
            K::SetLightIntensity {
                intensity: op("Mul", vec![number(800.0), number(3.0)]),
            },
            // A slot that isn't a number stands a zero, the same both ways.
            K::SetLightIntensity {
                intensity: Value::text("bright"),
            },
            K::SetEmissiveStrength {
                strength: op("Div", vec![number(5.0), number(2.0)]),
            },
            K::SetHdrOutput { enabled: true },
            K::SetPeakBrightness {
                nits: op("Add", vec![number(600.0), number(400.0)]),
            },
            K::SetHdrOutput { enabled: false },
            K::EnableVolume {
                enabled: false,
                volume: op("Join", vec![Value::text(" Ca"), Value::text("ve ")]),
            },
            K::EnableVolume {
                enabled: true,
                volume: Value::text(""),
            },
            K::SetVolumeWeight {
                volume: Value::text("Cave"),
                weight: op("Div", vec![number(1.0), number(4.0)]),
            },
            // A weight that isn't a number stands a zero, the same both ways.
            K::SetVolumeWeight {
                volume: Value::text("Cave"),
                weight: Value::text("heavy"),
            },
            K::CaptureProbes,
            K::SetShadowDistance {
                distance: op("Mul", vec![number(20.0), number(2.5)]),
            },
            K::SetLightShadows { enabled: false },
            K::SetLightShadows { enabled: true },
            K::SetRayTracing { enabled: true },
            K::SetGiBounces {
                bounces: op("Add", vec![number(2.0), number(2.0)]),
            },
            // A count that isn't a number stands a zero, the same both ways.
            K::SetGiSamples {
                samples: Value::text("lots"),
            },
            K::SetRayTracing { enabled: false },
            K::SetFogDensity {
                density: op("Div", vec![number(3.0), number(300.0)]),
            },
            K::SetAurora {
                kp: op("Add", vec![number(4.0), number(3.0)]),
            },
            // A KP that isn't a number stands a zero, the same both ways.
            K::SetAurora {
                kp: Value::text("bright"),
            },
            K::StrikeLightning {
                x: number(12.0),
                y: op("Sub", vec![number(0.0), number(1.0)]),
                z: op("Mul", vec![number(-4.0), number(2.5)]),
            },
            K::Fracture {
                target: Value::text("myself"),
            },
            K::Fracture {
                target: Value::text("missing"),
            },
            K::Splash {
                x: number(1.0),
                y: number(2.0),
                z: number(3.0),
                radius: number(4.0),
                strength: Value::text("strong"),
            },
            K::PuffSmoke {
                x: number(4.0),
                y: number(5.0),
                z: number(6.0),
                radius: number(2.0),
                strength: number(0.8),
            },
            K::SpawnDecal {
                preset: blockloom_core::decals::DecalPreset::FreshScorch,
                x: number(1.0),
                y: number(2.0),
                z: op("Mul", vec![number(-4.0), number(2.5)]),
                nx: number(0.0),
                ny: number(1.0),
                nz: number(0.0),
                size: op("Add", vec![number(1.0), number(0.5)]),
                lifetime: number(12.0),
                fade: number(3.0),
            },
            K::FadeDecals {
                x: number(1.0),
                y: number(2.0),
                z: number(-10.0),
                radius: number(5.0),
                seconds: Value::text("now"),
            },
            K::SetLightningRate {
                rate: op("Mul", vec![number(3.0), number(4.0)]),
            },
            K::SetWind {
                property: WindProperty::Direction,
                value: op("Add", vec![number(200.0), number(70.0)]),
            },
            K::SetWind {
                property: WindProperty::Speed,
                value: op("Mul", vec![number(2.5), number(4.0)]),
            },
            // A dial that isn't a number stands a zero, the same both ways.
            K::SetWind {
                property: WindProperty::Storm,
                value: Value::text("wild"),
            },
            K::SetCloudDrift {
                x: number(3.0),
                y: op("Sub", vec![number(0.0), number(0.5)]),
                z: op("Mul", vec![number(-2.0), number(1.5)]),
            },
            K::SetCloudLayer {
                layer: number(2.0),
                property: CloudLayerProperty::Opacity,
                value: op("Div", vec![number(1.0), number(4.0)]),
            },
            // A layer that isn't a number stands a zero too.
            K::SetCloudLayer {
                layer: Value::text("top"),
                property: CloudLayerProperty::Spin,
                value: number(15.0),
            },
            K::SetClouds {
                property: CloudProperty::Coverage,
                value: op("Div", vec![number(3.0), number(4.0)]),
            },
            // A dial that isn't a number stands a zero, the same both ways.
            K::SetClouds {
                property: CloudProperty::Type,
                value: Value::text("fluffy"),
            },
            K::SetWater {
                property: WaterProperty::Level,
                value: op("Sub", vec![number(1.0), number(0.25)]),
            },
            K::SetWater {
                property: WaterProperty::Chop,
                value: Value::text("choppy"),
            },
            K::Move {
                steps: op("Math", vec![Value::text("Sqrt"), number(2.0)]),
            },
        ],
        &[],
    );
}

#[test]
fn text_and_comparison_land_on_the_same_answers() {
    assert_same(
        "text",
        vec![
            K::Say {
                text: op("Join", vec![Value::text("a"), Value::text("b")]),
            },
            K::Say {
                text: op("Case", vec![Value::text("Mixed"), Value::text("Upper")]),
            },
            K::Say {
                text: op("LetterOf", vec![number(2.0), Value::text("abc")]),
            },
            K::Say {
                text: op("Length", vec![Value::text("hello")]),
            },
            K::Say {
                text: op("Eq", vec![Value::text("5"), number(5.0)]),
            },
            K::Say {
                text: op("Gt", vec![Value::text("10"), number(9.0)]),
            },
            K::Say {
                text: op("IndexOf", vec![Value::text("l"), Value::text("hello")]),
            },
        ],
        &[],
    );
}

#[test]
fn a_bad_slot_is_reported_once_and_stands_in_as_zero() {
    assert_same(
        "errors",
        vec![
            // Reported, and the move is zero.
            K::Move {
                steps: op("Div", vec![number(1.0), number(0.0)]),
            },
            // Evaluated fine, but text that isn't a number: zero, and
            // nothing said about it.
            K::Move {
                steps: Value::text("not a number"),
            },
            // One error for the whole tree, not one per bad branch.
            K::ChangePosition {
                axis: Axis::X,
                by: op(
                    "Add",
                    vec![
                        op("Div", vec![number(1.0), number(0.0)]),
                        op("Div", vec![number(2.0), number(0.0)]),
                    ],
                ),
            },
            K::Say {
                text: op("LetterOf", vec![number(9.0), Value::text("abc")]),
            },
        ],
        &[],
    );
}

#[test]
fn a_short_circuit_keeps_the_bad_slot_on_the_right_quiet() {
    assert_same(
        "shortcircuit",
        vec![
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("False", vec![]),
                        op("Div", vec![number(1.0), number(0.0)]),
                    ],
                ),
            },
            K::Say {
                text: op(
                    "Or",
                    vec![
                        op("True", vec![]),
                        op("Div", vec![number(1.0), number(0.0)]),
                    ],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn sensing_reads_the_same_world() {
    assert_same(
        "sensing",
        vec![
            K::Say {
                text: op("KeyDown", vec![Value::text("Space")]),
            },
            K::Move {
                steps: op("Timer", vec![]),
            },
            K::Say {
                text: op("MouseDeltaX", vec![]),
            },
            K::Say {
                text: op("MouseDeltaY", vec![]),
            },
            K::Say {
                text: op("MouseLocked", vec![]),
            },
            K::Say {
                text: op("MouseButtonDown", vec![Value::text("right")]),
            },
            K::Say {
                text: op("ActionDown", vec![Value::text("Jump")]),
            },
            K::Say {
                text: op("ActionPressed", vec![Value::text("Jump")]),
            },
            K::Say {
                text: op("ActionReleased", vec![Value::text("Jump")]),
            },
            K::Say {
                text: op("ActionValue", vec![Value::text("Left")]),
            },
            K::Say {
                text: op("TouchCount", vec![]),
            },
            K::Say {
                text: op("TouchX", vec![number(1.0)]),
            },
            K::Say {
                text: op("TouchY", vec![number(1.0)]),
            },
            K::Say {
                text: op("GamepadConnected", vec![]),
            },
            K::Say {
                text: op("GamepadAxis", vec![Value::text("LeftStickX")]),
            },
            K::Say {
                text: op("GamepadButtonDown", vec![Value::text("South")]),
            },
            K::Say {
                text: op("Atmosphere", vec![Value::text("wind speed")]),
            },
            K::Say {
                text: op("Atmosphere", vec![Value::text("humidity")]),
            },
            K::Say {
                text: op("SceneLuminance", vec![]),
            },
            K::Say {
                text: op("IsHdrDisplay", vec![]),
            },
            K::Say {
                text: op("PeakBrightness", vec![]),
            },
            K::Say {
                text: op("IsRayTracing", vec![]),
            },
            K::Say {
                text: op("RayTracingAvailable", vec![]),
            },
            K::Say {
                text: op("ActiveVolumes", vec![]),
            },
            K::Say {
                text: op("CurrentScene", vec![]),
            },
            K::Say {
                text: op("SceneNames", vec![]),
            },
            K::Say {
                text: op("CastsShadows", vec![Value::text("Player")]),
            },
            K::Say {
                text: op("CastsShadows", vec![Value::text("Nobody")]),
            },
            K::ChangePosition {
                axis: Axis::X,
                by: op("MyPosition", vec![Value::text("Y")]),
            },
            K::ChangePosition {
                axis: Axis::Y,
                by: op("MyLocalPosition", vec![Value::text("Y")]),
            },
            K::Say {
                text: op(
                    "ActorPosition",
                    vec![Value::text("Friend"), Value::text("X")],
                ),
            },
            K::Say {
                text: op(
                    "ActorLocalPosition",
                    vec![Value::text("Friend"), Value::text("X")],
                ),
            },
            // The same complaint, in the same words, when it isn't there.
            K::Say {
                text: op(
                    "ActorPosition",
                    vec![Value::text("Nobody"), Value::text("X")],
                ),
            },
            K::Say {
                text: op("CameraPosition", vec![Value::text("Y")]),
            },
            K::Say {
                text: op("CameraDirection", vec![Value::text("Z")]),
            },
        ],
        &[],
    );
}

#[test]
fn variables_read_and_write_the_same_way() {
    assert_same(
        "variables",
        vec![
            K::Say {
                text: Value::Var {
                    name: "score".to_string(),
                },
            },
            K::SetVariable {
                name: "score".to_string(),
                value: number(10.0),
            },
            K::ChangeVariable {
                name: "score".to_string(),
                value: number(2.5),
            },
            K::Say {
                text: Value::Var {
                    name: "score".to_string(),
                },
            },
            // A variable nothing ever set reads as zero on both sides.
            K::Say {
                text: Value::Var {
                    name: "never".to_string(),
                },
            },
        ],
        &[("score", Evaluated::Number(4.0))],
    );
}

/// A project with declared lists seeded with items: the VM loads them out
/// of the document, and `by_compiler` seeds the harness's own copy from the
/// same document, so the two halves start holding the same things.
fn project_with_lists(bodies: Vec<Vec<K>>, lists: Vec<(String, Vec<ListItem>)>) -> Project {
    let mut project = project_with_blocks(bodies, Vec::new(), &[]);
    let actor = project.actors.first_mut().expect("one actor");
    for (name, items) in lists {
        actor.graph.lists.push(ListDef {
            name,
            items,
            editor_visible: false,
            editor_x: 0,
            editor_y: 0,
        });
    }
    project
}

fn assert_lists(case: &str, bodies: Vec<Vec<K>>, lists: Vec<(String, Vec<ListItem>)>) {
    if !toolchain() {
        return;
    }
    let project = project_with_lists(bodies, lists);
    assert_project(case, project, &[]);
}

/// A project with declared dicts seeded with entries: the VM loads them out
/// of the document, and `by_compiler` seeds the harness's own copy from the
/// same document, so the two halves start holding the same things.
fn project_with_dicts(
    bodies: Vec<Vec<K>>,
    lists: Vec<(String, Vec<ListItem>)>,
    dicts: Vec<(String, Vec<DictEntry>)>,
) -> Project {
    let mut project = project_with_lists(bodies, lists);
    let actor = project.actors.first_mut().expect("one actor");
    for (name, entries) in dicts {
        actor.graph.dicts.push(DictDef {
            name,
            entries,
            editor_visible: false,
            editor_x: 0,
            editor_y: 0,
        });
    }
    project
}

fn assert_dicts(
    case: &str,
    bodies: Vec<Vec<K>>,
    lists: Vec<(String, Vec<ListItem>)>,
    dicts: Vec<(String, Vec<DictEntry>)>,
) {
    if !toolchain() {
        return;
    }
    let project = project_with_dicts(bodies, lists, dicts);
    assert_project(case, project, &[]);
}

fn dict_entry(key: &str, value: DictItem) -> DictEntry {
    DictEntry {
        key: key.to_string(),
        value,
    }
}

#[test]
fn lists_hold_and_report_their_items_the_same_way() {
    assert_lists(
        "list-items",
        vec![vec![
            K::AddToList {
                value: Value::text("first"),
                name: "items".to_string(),
            },
            K::AddToList {
                value: number(7.0),
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListItem", vec![number(1.0), Value::text("items")]),
            },
            K::Say {
                text: op("ListLength", vec![Value::text("items")]),
            },
            K::DeleteOfList {
                index: number(1.0),
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListItem", vec![number(1.0), Value::text("items")]),
            },
            K::InsertIntoList {
                value: Value::text("zero"),
                index: number(1.0),
                name: "items".to_string(),
            },
            K::ReplaceItemOfList {
                index: number(2.0),
                name: "items".to_string(),
                value: number(8.0),
            },
            K::ShiftList {
                name: "items".to_string(),
                amount: number(1.0),
            },
            K::ReverseList {
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListItem", vec![number(1.0), Value::text("items")]),
            },
            K::Say {
                text: op("ListLength", vec![Value::text("items")]),
            },
            K::DeleteAllOfList {
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListIsEmpty", vec![Value::text("items")]),
            },
        ]],
        vec![("items".to_string(), Vec::new())],
    );
}

#[test]
fn list_reporters_answer_the_same_including_the_edges() {
    assert_lists(
        "list-reporters",
        vec![vec![
            K::Say {
                text: op(
                    "ListItemNumber",
                    vec![Value::text("b"), Value::text("letters")],
                ),
            },
            K::Say {
                text: op("ListAmount", vec![Value::text("a"), Value::text("letters")]),
            },
            K::Say {
                text: op(
                    "ListContains",
                    vec![Value::text("letters"), Value::text("c")],
                ),
            },
            K::Say {
                text: op("ListItemExists", vec![number(2.0), Value::text("letters")]),
            },
            K::Say {
                text: op("ListItemExists", vec![number(9.0), Value::text("letters")]),
            },
            // A list nothing declared reads empty, and so does a missing item.
            K::Say {
                text: op("ListItem", vec![number(1.0), Value::text("nobody")]),
            },
            K::Say {
                text: op("ListLength", vec![Value::text("nobody")]),
            },
            K::Say {
                text: op("ListIsEmpty", vec![Value::text("nobody")]),
            },
            // An out-of-range change is nothing at all, and a boolean is not
            // an item: reported, and dropped.
            K::DeleteOfList {
                index: number(9.0),
                name: "letters".to_string(),
            },
            K::AddToList {
                value: Value::Bool,
                name: "letters".to_string(),
            },
            K::Say {
                text: op("ListLength", vec![Value::text("letters")]),
            },
        ]],
        vec![(
            "letters".to_string(),
            vec![
                ListItem::Text("a".to_string()),
                ListItem::Text("b".to_string()),
                ListItem::Text("a".to_string()),
            ],
        )],
    );
}

#[test]
fn dicts_hold_and_report_their_values_the_same_way() {
    assert_dicts(
        "dict-values",
        vec![vec![
            K::SetDictValue {
                key: Value::text("hp"),
                name: "save".to_string(),
                value: number(3.0),
            },
            K::SetDictValue {
                key: Value::text("name"),
                name: "save".to_string(),
                value: Value::text("fox"),
            },
            K::Say {
                text: op("DictValue", vec![Value::text("hp"), Value::text("save")]),
            },
            K::Say {
                text: op("DictSize", vec![Value::text("save")]),
            },
            K::SetDictValue {
                key: Value::text("hp"),
                name: "save".to_string(),
                value: number(4.0),
            },
            K::Say {
                text: op("DictValue", vec![Value::text("hp"), Value::text("save")]),
            },
            K::Say {
                text: op("DictKeys", vec![Value::text("save")]),
            },
            K::DeleteDictKey {
                key: Value::text("name"),
                name: "save".to_string(),
            },
            K::Say {
                text: op("DictHasKey", vec![Value::text("save"), Value::text("name")]),
            },
            K::Say {
                text: op("DictIsEmpty", vec![Value::text("save")]),
            },
            // A dict nothing declared reads empty, and so does a missing
            // key. A boolean is not a value: reported, and dropped.
            K::Say {
                text: op("DictValue", vec![Value::text("mp"), Value::text("nobody")]),
            },
            K::Say {
                text: op("DictSize", vec![Value::text("nobody")]),
            },
            K::SetDictValue {
                key: Value::text("ok"),
                name: "save".to_string(),
                value: Value::Bool,
            },
            K::DeleteAllOfDict {
                name: "save".to_string(),
            },
            K::Say {
                text: op("DictSize", vec![Value::text("save")]),
            },
        ]],
        Vec::new(),
        // A seeded entry is replaced by the first write, so both halves
        // still report exactly what the blocks wrote.
        vec![(
            "save".to_string(),
            vec![dict_entry("hp", DictItem::Number(1.0))],
        )],
    );
}

#[test]
fn dicts_and_lists_bridge_to_json_the_same_way() {
    assert_dicts(
        "json-bridge",
        vec![vec![
            K::SetDictValue {
                key: Value::text("hp"),
                name: "save".to_string(),
                value: number(3.0),
            },
            K::SetDictValue {
                key: Value::text("name"),
                name: "save".to_string(),
                value: Value::text("fox"),
            },
            K::Say {
                text: op("DictAsJson", vec![Value::text("save")]),
            },
            K::AddToList {
                value: number(7.0),
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListAsJson", vec![Value::text("items")]),
            },
            K::LoadJsonIntoDict {
                json: Value::text(r#"{"hp": 9, "title": "mage"}"#),
                name: "save".to_string(),
            },
            K::Say {
                text: op("DictValue", vec![Value::text("title"), Value::text("save")]),
            },
            K::Say {
                text: op("DictAsJson", vec![Value::text("save")]),
            },
            K::LoadJsonIntoList {
                json: Value::text(r#"[1, "two"]"#),
                name: "items".to_string(),
            },
            K::Say {
                text: op("ListAsJson", vec![Value::text("items")]),
            },
            // What isn't JSON reports and leaves the collection: an array
            // for a dict, a non-array for a list, and a boolean hiding in
            // an object for a dict.
            K::LoadJsonIntoDict {
                json: Value::text("[1, 2]"),
                name: "save".to_string(),
            },
            K::LoadJsonIntoList {
                json: Value::text("nope"),
                name: "items".to_string(),
            },
            K::LoadJsonIntoDict {
                json: Value::text(r#"{"ok": true}"#),
                name: "save".to_string(),
            },
            K::Say {
                text: op("DictAsJson", vec![Value::text("save")]),
            },
            K::Say {
                text: op("ListAsJson", vec![Value::text("items")]),
            },
        ]],
        vec![("items".to_string(), Vec::new())],
        vec![("save".to_string(), Vec::new())],
    );
}

#[test]
fn the_rest_of_the_leaf_blocks_land_the_same() {
    assert_same(
        "leaves",
        vec![
            K::GoTo {
                x: number(1.0),
                y: op("Add", vec![number(2.0), number(3.0)]),
                z: number(0.0),
            },
            K::NavigateTo {
                x: number(1.0),
                y: number(2.0),
                z: number(0.0),
                speed: op("Add", vec![number(2.0), number(3.0)]),
            },
            K::BurstParticles {
                count: number(17.0),
            },
            K::SetEmitterDial {
                dial: EmitterDial::Speed,
                value: number(42.0),
            },
            K::SetTrailEnabled { enabled: false },
            K::SetEmitterPlaying { playing: false },
            K::SetVisible { visible: false },
            K::SetMouseLocked { locked: true },
            K::RumbleGamepad {
                strength: number(80.0),
                duration: number(0.5),
            },
            K::BindAction {
                action: Value::text("Jump"),
                binding: Value::text("space"),
            },
            K::ClearActionBindings {
                action: Value::text("Jump"),
            },
            K::SetCameraPitch {
                degrees: number(12.5),
            },
            K::SetCameraFov { fov: number(90.0) },
            K::SetColor {
                color: Value::text("#ff0000"),
            },
            K::SetComponentField {
                component: "Stats".to_string(),
                field: "hp".to_string(),
                value: op("Sub", vec![number(10.0), number(4.0)]),
            },
            K::AttachComponent {
                component: "Body".to_string(),
            },
            // The VM resolves the name to an id before the effect leaves it,
            // so the compiled half has to ask the host the same thing.
            K::SetParent {
                parent: Value::text("Friend"),
            },
        ],
        &[],
    );
}

#[test]
fn sounds_ask_for_the_same_things() {
    assert_same(
        "sounds",
        vec![
            K::PlaySound {
                sound: Value::text("assets/sounds/jump.wav"),
                volume: number(50.0),
                pitch: number(2.0),
                loop_: false,
                bus: SoundBus::Sfx,
            },
            // An expression in a slot, a bus that isn't the default, and a
            // target the VM resolves to an id before the effect leaves it.
            K::PlaySoundAt {
                sound: Value::text("assets/sounds/hum.wav"),
                volume: op("Add", vec![number(40.0), number(60.0)]),
                pitch: number(1.0),
                loop_: true,
                bus: SoundBus::Music,
                target: Value::text("Player"),
            },
            // An empty sound reports on both sides rather than playing.
            K::PlaySound {
                sound: Value::text(""),
                volume: number(100.0),
                pitch: number(1.0),
                loop_: false,
                bus: SoundBus::Master,
            },
            K::SetSoundVolume {
                sound: Value::text("assets/sounds/hum.wav"),
                volume: number(25.0),
            },
            K::SetSoundPitch {
                sound: Value::text("assets/sounds/hum.wav"),
                pitch: op("Add", vec![number(1.0), number(0.5)]),
            },
            K::SetBusVolume {
                bus: SoundBus::Music,
                volume: number(80.0),
            },
            K::StopSound {
                sound: Value::text(""),
            },
        ],
        &[],
    );
}

#[test]
fn every_actor_is_in_the_name_table_whether_it_has_blocks_or_not() {
    // `delete` and `create a clone of` name an actor the way a block does, so
    // one with an empty canvas still has to be findable by name.
    let mut project = project_with_blocks(
        vec![vec![K::DeleteActor {
            target: Value::text("Scenery"),
        }]],
        Vec::new(),
        &[],
    );
    let mut quiet = Actor::new(
        "Scenery",
        Visual::Rect {
            color: "#000000".to_string(),
            size: [1.0, 1.0],
        },
    );
    quiet.id = "a2".to_string();
    project.actors.push(quiet);

    let source = blockloom_core::codegen::compile(&project).expect("this project compiles");
    assert!(source.contains("(\"a2\", \"Scenery\")"), "{source}");
    // And nothing was emitted for it: an actor with no steps has no strands.
    assert!(!source.contains("fn actor_1("), "{source}");
}

#[test]
fn the_weather_director_asks_for_the_same_things() {
    // Both halves read each slot first, trim the weather's name, and ask in
    // row order - so what follows never runs on either side.
    assert_same(
        "weather-director",
        vec![
            K::SetTimeOfDay {
                time: op("Add", vec![number(6.0), number(12.5)]),
            },
            // A clock that isn't a number stands a zero, the same both ways.
            K::SetTimeOfDay {
                time: Value::text("noon"),
            },
            K::AdvanceTime {
                hours: op("Sub", vec![number(2.0), number(0.5)]),
            },
            K::SetPrecipitation {
                property: PrecipitationKind::Rain,
                value: op("Div", vec![number(7.0), number(10.0)]),
            },
            // Snow that isn't a number stands a zero too.
            K::SetPrecipitation {
                property: PrecipitationKind::Snow,
                value: Value::text("flurry"),
            },
            K::BlendWeather {
                weather: Value::text(" Storm "),
                seconds: op("Add", vec![number(2.0), number(3.0)]),
            },
            K::BlendWeather {
                weather: op("Join", vec![Value::text("Sto"), Value::text("rm")]),
                seconds: number(0.0),
            },
            K::Say {
                text: op("TimeOfDay", vec![]),
            },
            K::Say {
                text: op("SunElevation", vec![]),
            },
            K::Say {
                text: op("CurrentWeather", vec![]),
            },
            K::Say {
                text: op("Atmosphere", vec![Value::text("time of day")]),
            },
            K::Say {
                text: op("Atmosphere", vec![Value::text("sun elevation")]),
            },
        ],
        &[],
    );
}

#[test]
fn when_weather_becomes_starts_only_for_its_weather() {
    assert_same_headed(
        "weather-arrives",
        vec![
            (
                K::WhenWeather {
                    weather: "storm".to_string(),
                },
                vec![K::Say {
                    text: Value::text("storm's here"),
                }],
            ),
            (
                K::WhenWeather {
                    weather: "Clear".to_string(),
                },
                vec![K::Say {
                    text: Value::text("never"),
                }],
            ),
            (
                K::WhenWeather {
                    weather: "".to_string(),
                },
                vec![K::Say {
                    text: Value::text("any weather"),
                }],
            ),
        ],
    );
}

#[test]
fn the_cutscene_blocks_ask_for_the_same_things_in_the_same_order() {
    // Both halves read each slot in row order, trim names, normalize the
    // fade, and keep the strand running - a cutscene plays beside the
    // blocks, not instead of them.
    assert_same(
        "cutscene-director",
        vec![
            K::PlayCutscene {
                cutscene: Value::text(" Opener "),
            },
            K::SkipCutscene,
            K::CameraShake {
                amount: op("Add", vec![number(0.2), number(0.5)]),
            },
            // A shake that isn't a number stands a zero, the same both ways.
            K::CameraShake {
                amount: Value::text("heavy"),
            },
            K::SetTimeScale {
                scale: Value::text("blurred"),
            },
            K::SetTimeScale {
                scale: op("Div", vec![number(1.0), number(2.0)]),
            },
            K::Hitstop {
                frames: Value::number(3.0),
            },
            K::SetLetterbox {
                on: Value::number(1.0),
            },
            K::FadeScreen {
                color: Value::text("BLACK"),
            },
            K::FadeScreen {
                color: Value::text("curtain"),
            },
            K::Say {
                text: op("IsCutscenePlaying", vec![]),
            },
            K::Say {
                text: op("CutsceneTime", vec![]),
            },
        ],
        &[],
    );
}

#[test]
fn when_cutscene_markers_arrive_they_start_their_strands() {
    assert_same_headed(
        "cutscene-arrives",
        vec![
            (
                K::WhenCutsceneSignal {
                    signal: "beat".to_string(),
                },
                vec![K::Say {
                    text: Value::text("the beat drops"),
                }],
            ),
            (
                K::WhenCutsceneSignal {
                    signal: "sting".to_string(),
                },
                vec![K::Say {
                    text: Value::text("never"),
                }],
            ),
            (
                K::WhenCutsceneSignal {
                    signal: "".to_string(),
                },
                vec![K::Say {
                    text: Value::text("any signal"),
                }],
            ),
            (
                K::WhenCutsceneEnds,
                vec![K::Say {
                    text: Value::text("curtain down"),
                }],
            ),
        ],
    );
}

#[test]
fn save_slots_and_language_ask_the_same_things_in_order() {
    // Both halves read the slot first, trim it, and carry on (unlike
    // `switch scene to`, nothing ends the strand) - so what follows runs on
    // either side, in order.
    assert_same_strands(
        "save-slots-and-language",
        vec![vec![
            K::SwitchSaveSlot {
                slot: Value::text("Slot 1"),
            },
            K::Say {
                text: op("SaveSlot", vec![]),
            },
            K::DeleteSaveSlot {
                slot: op("Join", vec![Value::text("old "), Value::text("slot")]),
            },
            K::Say {
                text: op("SaveSlots", vec![]),
            },
            K::SetLanguage {
                language: Value::text("fr"),
            },
            K::Say {
                text: op("Language", vec![]),
            },
            K::Say {
                text: op("LocalizedText", vec![Value::text("menu.play")]),
            },
        ]],
        &[],
    );
}

#[test]
fn switching_scenes_asks_for_the_same_scene_in_the_same_order() {
    // Both halves read the scene slot first and the transition second, trim
    // the scene, normalize the transition, and end the strand where it
    // stands - so what follows never runs on either side.
    assert_same_strands(
        "switch-scene",
        vec![
            vec![
                K::SwitchScene {
                    scene: Value::text("Scene 2"),
                    transition: Value::text("fade"),
                },
                K::Say {
                    text: Value::text("unreached"),
                },
            ],
            vec![K::SwitchScene {
                scene: op("Join", vec![Value::text("Scene "), Value::number(2.0)]),
                transition: Value::text("WIPE"),
            }],
            vec![K::SwitchScene {
                scene: Value::text("Scene 2"),
                transition: Value::text("curtain"),
            }],
        ],
        &[],
    );
}

#[test]
fn plugin_blocks_ask_the_same_things_in_order() {
    // Slots are read left to right before the call is queued, a whole number
    // goes over as an integer, and a bad slot is reported once and stands in
    // as zero on both sides.
    let call = |args: Vec<Value>| K::PluginBlock {
        plugin: "com.example.health".to_string(),
        block: "heal".to_string(),
        args,
    };
    assert_same_strands(
        "plugin-call",
        vec![vec![
            call(vec![Value::text("Hero"), Value::number(5.0)]),
            call(vec![
                op("Join", vec![Value::text("a\"b\\"), Value::text("\n")]),
                number(2.5),
                op("True", vec![]),
            ]),
            call(vec![op("Div", vec![number(1.0), Value::text("x")])]),
            call(vec![]),
        ]],
        &[],
    );
}

#[test]
fn plugin_reporters_answer_the_same_including_the_errors() {
    let read = |block: &str, slots: Vec<Value>| {
        let mut args = vec![Value::text("com.example.tally"), Value::text(block)];
        args.extend(slots);
        op(blockloom_core::value::PLUGIN_READ, args)
    };
    assert_same(
        "plugin-read",
        vec![
            K::Say {
                text: read("count", vec![Value::text("coins")]),
            },
            K::Say {
                text: read("label", vec![Value::text("a"), number(3.0)]),
            },
            K::Say {
                text: read("nothing", vec![]),
            },
            K::Say {
                text: op(
                    "Add",
                    vec![number(1.0), read("count", vec![Value::text("ab")])],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn when_a_plugin_fires_only_its_hats_start() {
    let hat = |event: &str, arg: &str| K::WhenPlugin {
        plugin: "com.example.tally".to_string(),
        block: event.to_string(),
        event: event.to_string(),
        args: vec![arg.to_string()],
    };
    let said = |text: &str| K::Say {
        text: Value::text(text),
    };
    assert_same_headed(
        "plugin-hats",
        vec![
            (hat("changed", "coins"), vec![said("changed coins")]),
            (hat("changed", ""), vec![said("changed anything")]),
            (hat("changed", "gems"), vec![said("changed gems")]),
            (hat("hit", "5"), vec![said("hit five")]),
            (hat("spent", "5"), vec![said("spent five")]),
            (hat("other", ""), vec![said("never fired")]),
        ],
    );
}

fn body(kinds: Vec<K>) -> Vec<Instruction> {
    kinds.into_iter().map(Instruction::new).collect()
}

#[test]
fn a_branch_takes_the_same_side() {
    assert_same(
        "branches",
        vec![
            K::If {
                condition: op("Gt", vec![number(2.0), number(1.0)]),
                body: body(vec![K::Say {
                    text: Value::text("taken"),
                }]),
            },
            K::If {
                condition: op("Gt", vec![number(1.0), number(2.0)]),
                body: body(vec![K::Say {
                    text: Value::text("skipped"),
                }]),
            },
            K::IfElse {
                condition: op("KeyDown", vec![Value::text("space")]),
                then_body: body(vec![K::Say {
                    text: Value::text("then"),
                }]),
                else_body: body(vec![K::Say {
                    text: Value::text("else"),
                }]),
            },
            // A condition that won't evaluate reports itself once and reads
            // as false, so the else side runs on both.
            K::IfElse {
                condition: op("Div", vec![number(1.0), number(0.0)]),
                then_body: body(vec![K::Say {
                    text: Value::text("bad then"),
                }]),
                else_body: body(vec![K::Say {
                    text: Value::text("bad else"),
                }]),
            },
        ],
        &[],
    );
}

/// A loop hands the frame back every time round, so the count of iterations
/// and the ticks they land on both have to agree.
#[test]
fn a_repeat_goes_round_the_same_number_of_times() {
    assert_same(
        "repeat",
        vec![
            K::Repeat {
                count: number(3.0),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                ]),
            },
            // Counted once on the way in: changing it mid-loop changes
            // nothing, and a count of zero skips the body outright.
            K::Repeat {
                count: number(0.0),
                body: body(vec![K::Say {
                    text: Value::text("never"),
                }]),
            },
            K::Repeat {
                count: number(-1.0),
                body: body(vec![K::Say {
                    text: Value::text("never either"),
                }]),
            },
            K::Say {
                text: Value::text("after"),
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

#[test]
fn a_while_rechecks_its_condition_every_time_round() {
    assert_same(
        "while",
        vec![
            K::While {
                condition: op(
                    "Lt",
                    vec![
                        Value::Var {
                            name: "n".to_string(),
                        },
                        number(4.0),
                    ],
                ),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::Move {
                        steps: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                ]),
            },
            K::Say {
                text: Value::Var {
                    name: "n".to_string(),
                },
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

/// `escape` leaves the loop it is in and nothing further out; `continue`
/// lands on the back edge, so it still costs the frame an iteration does.
#[test]
fn escaping_and_continuing_leave_from_the_same_places() {
    assert_same(
        "escape",
        vec![
            K::Repeat {
                count: number(5.0),
                body: body(vec![
                    K::ChangeVariable {
                        name: "n".to_string(),
                        value: number(1.0),
                    },
                    K::If {
                        condition: op(
                            "Eq",
                            vec![
                                Value::Var {
                                    name: "n".to_string(),
                                },
                                number(2.0),
                            ],
                        ),
                        body: body(vec![K::ContinueLoop]),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "n".to_string(),
                        },
                    },
                    K::If {
                        condition: op(
                            "Gte",
                            vec![
                                Value::Var {
                                    name: "n".to_string(),
                                },
                                number(3.0),
                            ],
                        ),
                        body: body(vec![K::EscapeLoop]),
                    },
                ]),
            },
            K::Say {
                text: Value::text("out"),
            },
        ],
        &[("n", Evaluated::Number(0.0))],
    );
}

/// The inner loop's own `escape` must not take the outer one with it, and
/// the two counters must not tread on each other.
#[test]
fn nested_loops_keep_their_own_tallies() {
    assert_same(
        "nested",
        vec![
            K::Repeat {
                count: number(3.0),
                body: body(vec![
                    K::Say {
                        text: Value::text("outer"),
                    },
                    K::Repeat {
                        count: number(2.0),
                        body: body(vec![
                            K::Say {
                                text: Value::text("inner"),
                            },
                            K::EscapeLoop,
                            K::Say {
                                text: Value::text("unreachable"),
                            },
                        ]),
                    },
                ]),
            },
            K::Say {
                text: Value::text("done"),
            },
        ],
        &[],
    );
}

/// The timing rule, not just the ordering one: a `wait` sleeps for as long on
/// both sides, so what happens on which tick has to line up as well.
#[test]
fn waiting_wakes_on_the_same_tick() {
    assert_same(
        "wait",
        vec![
            K::Say {
                text: Value::text("before"),
            },
            K::Wait {
                duration: number(0.25),
            },
            K::Say {
                text: Value::text("after"),
            },
            // A zero-second wait isn't a yield at all.
            K::Wait {
                duration: number(0.0),
            },
            K::Say {
                text: Value::text("straight on"),
            },
            // Nor is a negative one, which clamps to nothing.
            K::Wait {
                duration: number(-3.0),
            },
            K::Say {
                text: Value::text("still going"),
            },
            K::Repeat {
                count: number(2.0),
                body: body(vec![
                    K::Wait {
                        duration: number(0.1),
                    },
                    K::Move { steps: number(1.0) },
                ]),
            },
        ],
        &[],
    );
}

#[test]
fn a_wait_until_holds_the_strand_where_it_is() {
    assert_same(
        "waituntil",
        vec![
            // The world these tests publish never changes, so a condition
            // that holds passes straight through and one that doesn't holds
            // the strand until the run gives up - on both sides alike.
            K::WaitUntil {
                condition: op("KeyDown", vec![Value::text("space")]),
            },
            K::Say {
                text: Value::text("through"),
            },
            K::WaitUntil {
                condition: op("KeyDown", vec![Value::text("escape")]),
            },
            K::Say {
                text: Value::text("never reached"),
            },
        ],
        &[],
    );
}

#[test]
fn a_glide_starts_the_same_slide_and_sleeps_as_long() {
    assert_same(
        "glide",
        vec![
            K::Glide {
                seconds: number(0.2),
                x: number(5.0),
                y: op("Add", vec![number(1.0), number(2.0)]),
                z: number(0.0),
                easing: TweenEasing::EaseOut,
            },
            K::Say {
                text: Value::text("landed"),
            },
            // No time to slide over: the effect still goes out, and nothing
            // is suspended.
            K::Glide {
                seconds: number(0.0),
                x: number(1.0),
                y: number(1.0),
                z: number(1.0),
                easing: TweenEasing::Linear,
            },
            K::Say {
                text: Value::text("instant"),
            },
        ],
        &[],
    );
}

#[test]
fn tweens_ease_towards_scale_rotation_and_color_then_stop() {
    assert_same(
        "tweens",
        vec![
            K::TweenScale {
                factor: number(2.0),
                seconds: number(0.2),
                easing: TweenEasing::EaseInOut,
            },
            K::TweenRotation {
                axis: Axis::Z,
                degrees: op("Add", vec![number(45.0), number(45.0)]),
                seconds: number(0.2),
                easing: TweenEasing::Bounce,
            },
            K::TweenColor {
                color: Value::text("#FF0000"),
                seconds: number(0.2),
                easing: TweenEasing::Elastic,
            },
            K::StopTweens,
            // No time to tween over: the effect still goes out, and nothing
            // is suspended.
            K::TweenScale {
                factor: number(1.0),
                seconds: number(0.0),
                easing: TweenEasing::Linear,
            },
            K::Say {
                text: Value::text("settled"),
            },
        ],
        &[],
    );
}

#[test]
fn animation_blocks_play_retune_read_back_and_stop_together() {
    assert_same(
        "animation",
        vec![
            K::PlayAnimation {
                clip: Value::text("Walk"),
                speed: number(1.5),
            },
            K::SetAnimationSpeed {
                speed: op("CurrentFrame", vec![]),
            },
            K::Say {
                text: op("CurrentClip", vec![]),
            },
            K::Say {
                text: op("AnimationPlaying", vec![]),
            },
            K::Say {
                text: op("IsTweening", vec![]),
            },
            K::StopAnimation,
        ],
        &[],
    );
}

#[test]
fn when_my_particles_starts_only_for_its_event() {
    use blockloom_core::vfx::ParticleEvent;
    assert_same_headed(
        "particles",
        vec![
            (
                K::WhenParticles {
                    event: ParticleEvent::Die,
                },
                vec![
                    K::Say {
                        text: Value::text("died"),
                    },
                    K::SetEmitterPlaying { playing: false },
                ],
            ),
            (
                K::WhenParticles {
                    event: ParticleEvent::Collide,
                },
                vec![K::Say {
                    text: Value::text("hit"),
                }],
            ),
        ],
    );
}

#[test]
fn when_animation_ends_starts_only_for_its_clip() {
    assert_same_headed(
        "animation-ends",
        vec![
            (
                K::WhenAnimationEnds {
                    clip: "Walk".to_string(),
                },
                vec![K::Say {
                    text: Value::text("walk done"),
                }],
            ),
            (
                K::WhenAnimationEnds {
                    clip: "".to_string(),
                },
                vec![K::Say {
                    text: Value::text("any done"),
                }],
            ),
        ],
    );
}

#[test]
fn when_animation_marker_starts_only_for_its_marker() {
    assert_same_headed(
        "animation-marker",
        vec![
            (
                K::WhenAnimationMarker {
                    marker: "step".to_string(),
                },
                vec![K::Say {
                    text: Value::text("footstep"),
                }],
            ),
            (
                K::WhenAnimationMarker {
                    marker: "land".to_string(),
                },
                vec![K::Say {
                    text: Value::text("never"),
                }],
            ),
            (
                K::WhenAnimationMarker {
                    marker: "".to_string(),
                },
                vec![K::Say {
                    text: Value::text("any marker"),
                }],
            ),
        ],
    );
}

#[test]
fn when_enter_room_starts_only_for_its_room() {
    assert_same_headed(
        "enter-room",
        vec![
            (
                K::WhenEnterRoom {
                    room: "cave".to_string(),
                },
                vec![K::Say {
                    text: Value::text("dark in here"),
                }],
            ),
            (
                K::WhenEnterRoom {
                    room: "Hall".to_string(),
                },
                vec![K::Say {
                    text: Value::text("never"),
                }],
            ),
            (
                K::WhenEnterRoom {
                    room: "".to_string(),
                },
                vec![K::Say {
                    text: Value::text("any room"),
                }],
            ),
        ],
    );
}

#[test]
fn level_blocks_ask_the_same_things_in_order() {
    assert_same(
        "level",
        vec![
            K::PaintTile {
                map: Value::text(" Ground "),
                tile: op("Add", vec![number(2.0), number(1.5)]),
                x: op("Mul", vec![number(16.0), number(3.0)]),
                y: number(-8.0),
                z: op("Sub", vec![number(1.0), number(3.0)]),
            },
            K::PaintTile {
                map: Value::text(""),
                tile: number(-7.0),
                x: Value::text("left"),
                y: number(0.5),
                z: number(0.0),
            },
            K::SetParallax {
                layer: Value::text("Hills"),
                axis: ParallaxAxis::X,
                value: op("Div", vec![number(1.0), number(4.0)]),
            },
            K::SetParallax {
                layer: op("Join", vec![Value::text("Sk"), Value::text("y")]),
                axis: ParallaxAxis::Both,
                value: number(0.0),
            },
        ],
        &[],
    );
}

#[test]
fn query_blocks_ask_the_same_things_and_read_the_same_answers() {
    use blockloom_core::physics::query::{RayHits, TriggerPolicy};
    assert_same(
        "queries",
        vec![
            K::CastRay {
                hits: RayHits::Nearest,
                triggers: TriggerPolicy::UseGlobal,
                from_x: number(0.0),
                from_y: number(1.0),
                from_z: number(2.0),
                to_x: op("Add", vec![number(3.0), number(0.5)]),
                to_y: number(-4.0),
                to_z: number(5.0),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("x")]),
            },
            K::Say {
                text: op("QueryText", vec![number(1.0), Value::text("actor")]),
            },
            K::CastRay {
                hits: RayHits::Every,
                triggers: TriggerPolicy::Include,
                from_x: number(0.0),
                from_y: number(0.0),
                from_z: number(0.0),
                to_x: number(9.0),
                to_y: number(0.0),
                to_z: number(0.0),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("is trigger")]),
            },
            K::CastBall {
                triggers: TriggerPolicy::Ignore,
                radius: number(0.25),
                from_x: number(1.0),
                from_y: number(2.0),
                from_z: number(3.0),
                to_x: number(4.0),
                to_y: number(5.0),
                to_z: number(6.0),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("fraction")]),
            },
            K::OverlapBall {
                triggers: TriggerPolicy::UseGlobal,
                radius: number(2.0),
                x: number(7.0),
                y: number(8.0),
                z: number(9.0),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("distance")]),
            },
            K::Say {
                text: op("QueryNumber", vec![number(2.0), Value::text("distance")]),
            },
            K::FindClosest {
                triggers: TriggerPolicy::UseGlobal,
                range: number(12.0),
                x: number(-1.0),
                y: number(-2.0),
                z: number(-3.0),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("fraction")]),
            },
            K::Say {
                text: op("QueryText", vec![number(1.0), Value::text("collider")]),
            },
            K::Say {
                text: op("QueryNumber", vec![number(1.0), Value::text("count")]),
            },
        ],
        &[],
    );
}

#[test]
fn force_blocks_ask_the_same_things_in_order() {
    assert_same(
        "forces",
        vec![
            K::AddForce {
                mode: blockloom_core::physics::ForceMode::Force,
                x: number(1.5),
                y: op("Mul", vec![number(2.0), number(3.0)]),
                z: number(-4.0),
            },
            K::AddTorque {
                mode: blockloom_core::physics::ForceMode::VelocityChange,
                x: Value::text("2"),
                y: number(0.0),
                z: op("Sub", vec![number(1.0), number(3.0)]),
            },
            K::AddForce {
                mode: blockloom_core::physics::ForceMode::Impulse,
                x: number(0.0),
                y: number(0.0),
                z: number(0.0),
            },
        ],
        &[],
    );
}

#[test]
fn controller_blocks_ask_the_same_things_in_order() {
    use blockloom_core::physics::controller::{ControllerProperty, MoveMode};
    let say = |field: &str| K::Say {
        text: op("ControllerNumber", vec![number(0.0), Value::text(field)]),
    };
    assert_same(
        "controller",
        vec![
            K::ControllerMove {
                mode: MoveMode::Move,
                x: number(1.0),
                y: number(0.0),
                z: op("Sub", vec![number(1.0), number(3.0)]),
            },
            say("grounded"),
            K::ControllerMove {
                mode: MoveMode::Move,
                x: number(0.0),
                y: op("Mul", vec![number(-1.0), number(3.0)]),
                z: number(0.0),
            },
            say("grounded"),
            say("below"),
            say("flags"),
            K::ControllerMove {
                mode: MoveMode::Move,
                x: number(0.0),
                y: number(2.0),
                z: number(0.0),
            },
            say("grounded"),
            K::ControllerMove {
                mode: MoveMode::Simple,
                x: number(2.0),
                y: number(0.0),
                z: number(0.0),
            },
            say("below"),
            K::SetController {
                property: ControllerProperty::SlopeLimit,
                value: op("Add", vec![number(30.0), number(15.0)]),
            },
            K::Say {
                text: op("ControllerText", vec![number(1.0), Value::text("actor")]),
            },
        ],
        &[],
    );
}

#[test]
fn joint_blocks_ask_the_same_things_in_order() {
    use blockloom_core::physics::joints::{self, JointStatus, JointVerb};
    // The VM reads the shared registry the runtime publishes into; the compiled
    // host below answers the same numbers.
    joints::reset();
    joints::register_handles("a1", &["hinge"]);
    joints::publish(
        "a1",
        "hinge",
        JointStatus {
            enabled: true,
            position: 12.5,
            ..Default::default()
        },
    );
    let read = |field: &str, joint: &str| K::Say {
        text: op("JointNumber", vec![Value::text(field), Value::text(joint)]),
    };
    assert_same(
        "joints",
        vec![
            K::JointAct {
                action: JointVerb::MotorSpeed,
                joint: "hinge".into(),
                value: op("Add", vec![number(30.0), number(60.0)]),
            },
            read("position", "hinge"),
            read("broken", "hinge"),
            K::JointAct {
                action: JointVerb::Disable,
                joint: "hinge".into(),
                value: number(0.0),
            },
            read("enabled", "hinge"),
            K::JointAct {
                action: JointVerb::Break,
                joint: "hinge".into(),
                value: number(0.0),
            },
        ],
        &[],
    );
}

#[test]
fn rig_and_sprite_blocks_ask_the_same_things_in_order() {
    assert_same(
        "rig-sprite",
        vec![
            K::FireAnimationTrigger {
                name: op("Join", vec![Value::text("ju"), Value::text("mp")]),
            },
            K::SetRigSlot {
                slot: Value::text("hand"),
                attachment: op("CurrentClip", vec![]),
            },
            K::SetSlotTint {
                slot: Value::text("cape"),
                color: Value::text("#FF0000"),
            },
            K::SetIkTarget {
                constraint: Value::text("reach"),
                x: op("Add", vec![number(1.0), number(2.0)]),
                y: op("CurrentFrame", vec![]),
            },
            K::SetSpriteDial {
                dial: SpriteDial::FlipX,
                value: number(1.0),
            },
            K::SetSpriteDial {
                dial: SpriteDial::OutlineWidth,
                value: op("Add", vec![number(2.0), number(1.5)]),
            },
        ],
        &[],
    );
}

/// `stop all` ends the run where it stands, and everything queued behind it
/// in that same tick goes with it.
#[test]
fn stopping_ends_the_run_at_the_same_point() {
    assert_same_strands(
        "stopall",
        vec![
            vec![
                K::Say {
                    text: Value::text("first"),
                },
                K::Wait {
                    duration: number(0.1),
                },
                K::StopAll,
                K::Say {
                    text: Value::text("after the stop"),
                },
            ],
            vec![K::Forever {
                body: body(vec![K::Move { steps: number(1.0) }]),
            }],
        ],
        &[],
    );
}

/// Two strands on one actor share the ticks between them, so which one gets
/// to act first on each is part of what has to match.
#[test]
fn strands_take_their_turns_in_the_same_order() {
    assert_same_strands(
        "turns",
        vec![
            vec![K::Repeat {
                count: number(3.0),
                body: body(vec![K::Say {
                    text: Value::text("a"),
                }]),
            }],
            vec![K::Repeat {
                count: number(3.0),
                body: body(vec![K::Say {
                    text: Value::text("b"),
                }]),
            }],
            vec![
                K::Wait {
                    duration: number(0.05),
                },
                K::Say {
                    text: Value::text("late"),
                },
            ],
        ],
        &[],
    );
}

// ─── Custom blocks ──────────────────────────────────────────────────────────

fn calling(id: &str, args: Vec<Value>) -> K {
    K::CallBlock {
        block_id: id.to_string(),
        args,
    }
}

fn param(name: &str) -> Value {
    Value::Param {
        name: name.to_string(),
    }
}

#[test]
fn a_statement_call_runs_the_body_and_comes_back() {
    assert_same_blocks(
        "statement",
        vec![vec![
            K::Say {
                text: Value::text("before"),
            },
            calling("b1", vec![number(3.0)]),
            K::Say {
                text: Value::text("after"),
            },
            // The same block twice over: the second call has to find its own
            // way home rather than the first one's.
            calling("b1", vec![number(4.0)]),
            K::Say {
                text: Value::text("done"),
            },
        ]],
        vec![block(
            "b1",
            &["distance"],
            vec![
                K::Move {
                    steps: param("distance"),
                },
                // A name the block never declared reads as zero on both sides.
                K::Say {
                    text: param("nobody"),
                },
            ],
        )],
        &[],
    );
}

/// Arguments bind by position, and only as far as the two lists overlap: a
/// surplus one is never worked out at all, so its complaints never happen.
#[test]
fn arguments_bind_as_far_as_the_two_lists_overlap() {
    assert_same_blocks(
        "binding",
        vec![vec![
            calling("b1", vec![number(1.0), number(2.0)]),
            // One short: the second input has nothing to read.
            calling("b1", vec![number(9.0)]),
            // One over, and the surplus is a slot that would have complained.
            calling(
                "b1",
                vec![
                    number(5.0),
                    number(6.0),
                    op("Div", vec![number(1.0), number(0.0)]),
                ],
            ),
        ]],
        vec![block(
            "b1",
            &["a", "b"],
            vec![K::Move { steps: param("a") }, K::Say { text: param("b") }],
        )],
        &[],
    );
}

#[test]
fn a_reporter_runs_in_place_and_hands_its_value_back() {
    assert_same_blocks(
        "reporter",
        vec![vec![
            K::Say {
                text: call("b1", vec![number(4.0)]),
            },
            // In the middle of an expression, and twice over in one.
            K::Move {
                steps: op(
                    "Add",
                    vec![call("b1", vec![number(1.0)]), call("b1", vec![number(2.0)])],
                ),
            },
            // A body that falls off the end rather than returning is zero.
            K::Say {
                text: call("b2", vec![]),
            },
        ]],
        vec![
            block(
                "b1",
                &["n"],
                vec![
                    // A reporter may act on the world on its way to a value.
                    K::Say {
                        text: Value::text("asked"),
                    },
                    K::Return {
                        value: op("Mul", vec![param("n"), number(10.0)]),
                    },
                ],
            ),
            block(
                "b2",
                &[],
                vec![K::Say {
                    text: Value::text("no return"),
                }],
            ),
        ],
        &[],
    );
}

/// The order that makes hoisting necessary. A whole tree is resolved - every
/// variable and every reporter call - before one operator runs, so a reporter
/// on the side `and` never reads runs anyway, and says what it says.
#[test]
fn a_reporter_runs_even_on_the_side_a_short_circuit_never_reads() {
    assert_same_blocks(
        "resolve",
        vec![vec![
            K::Say {
                text: op("And", vec![op("False", vec![]), call("b1", vec![])]),
            },
            K::Say {
                text: op("Or", vec![op("True", vec![]), call("b1", vec![])]),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Say {
                    text: Value::text("ran anyway"),
                },
                K::Return {
                    value: op("True", vec![]),
                },
            ],
        )],
        &[],
    );
}

/// A `wait` inside a reporter suspends its caller: the strand sleeps as long,
/// and a loop in one costs its ticks the way any other loop does.
#[test]
fn a_wait_inside_a_reporter_suspends_its_caller() {
    assert_same_blocks(
        "suspendable",
        vec![vec![
            K::Say {
                text: call("b1", vec![]),
            },
            K::Say {
                text: Value::text("after"),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Say {
                    text: Value::text("round one"),
                },
                K::Wait {
                    duration: number(0.25),
                },
                K::Say {
                    text: Value::text("round two"),
                },
                K::Repeat {
                    count: number(2.0),
                    body: vec![Instruction::new(K::Say {
                        text: Value::text("loop"),
                    })],
                },
                K::WaitUntil {
                    condition: op("KeyDown", vec![Value::text("space")]),
                },
                K::Glide {
                    seconds: number(0.2),
                    x: number(1.0),
                    y: number(2.0),
                    z: number(3.0),
                    easing: TweenEasing::Linear,
                },
                K::Return {
                    value: Value::text("done"),
                },
            ],
        )],
        &[],
    );
}

/// A statement call is part of the strand, so a loop inside one yields the
/// way any other loop does - the block boundary changes nothing about when.
#[test]
fn a_loop_inside_a_statement_call_still_costs_its_ticks() {
    assert_same_blocks(
        "callloop",
        vec![vec![
            calling("b1", vec![]),
            K::Say {
                text: Value::text("after"),
            },
        ]],
        vec![block(
            "b1",
            &[],
            vec![
                K::Repeat {
                    count: number(3.0),
                    body: vec![Instruction::new(K::Say {
                        text: Value::text("round"),
                    })],
                },
                K::Wait {
                    duration: number(0.1),
                },
                K::Say {
                    text: Value::text("slept"),
                },
            ],
        )],
        &[],
    );
}

/// Each call runs on a state of its own, so a reporter may call itself. This
/// one counts down, which only works if the inner call's input doesn't
/// trample the outer one's.
#[test]
fn a_recursive_reporter_keeps_each_calls_own_inputs() {
    assert_same_blocks(
        "recursion",
        vec![vec![K::Say {
            text: call("fact", vec![number(5.0)]),
        }]],
        vec![block(
            "fact",
            &["n"],
            vec![
                K::If {
                    condition: op("Lte", vec![param("n"), number(1.0)]),
                    body: vec![Instruction::new(K::Return { value: number(1.0) })],
                },
                K::Return {
                    value: op(
                        "Mul",
                        vec![
                            param("n"),
                            call("fact", vec![op("Sub", vec![param("n"), number(1.0)])]),
                        ],
                    ),
                },
            ],
        )],
        &[],
    );
}

/// And when it never stops, both sides give up at the same depth and say so
/// in the same words.
#[test]
fn a_reporter_that_never_stops_gives_up_at_the_same_depth() {
    assert_same_blocks(
        "toodeep",
        vec![vec![K::Say {
            text: call("b1", vec![]),
        }]],
        vec![block(
            "b1",
            &[],
            vec![K::Return {
                value: call("b1", vec![]),
            }],
        )],
        &[],
    );
}

/// A `return` from inside a loop leaves the whole block, not just the loop.
#[test]
fn a_return_from_inside_a_loop_leaves_the_block() {
    assert_same_blocks(
        "returnout",
        vec![vec![
            calling("b1", vec![]),
            K::Say {
                text: Value::text("back"),
            },
            K::Say {
                text: call("b2", vec![]),
            },
        ]],
        vec![
            block(
                "b1",
                &[],
                vec![
                    K::Forever {
                        body: vec![
                            Instruction::new(K::Say {
                                text: Value::text("once"),
                            }),
                            Instruction::new(K::Return { value: number(0.0) }),
                        ],
                    },
                    K::Say {
                        text: Value::text("never"),
                    },
                ],
            ),
            block(
                "b2",
                &[],
                vec![K::Repeat {
                    count: number(5.0),
                    body: vec![Instruction::new(K::Return { value: number(7.0) })],
                }],
            ),
        ],
        &[],
    );
}

/// A call to a block that isn't there is stepped over, arguments and all.
#[test]
fn a_call_to_nothing_is_skipped_the_same_way() {
    assert_same_blocks(
        "missing",
        vec![vec![
            calling("gone", vec![op("Div", vec![number(1.0), number(0.0)])]),
            K::Say {
                text: call("gone", vec![op("Div", vec![number(1.0), number(0.0)])]),
            },
            K::Say {
                text: Value::text("carried on"),
            },
        ]],
        Vec::new(),
        &[],
    );
}

/// Sensing on both sides of an `and`: the left is asked, the right is not,
/// and both halves have to agree about which.
#[test]
fn sensing_on_both_sides_of_a_short_circuit_agrees() {
    assert_same(
        "bothsides",
        vec![
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("KeyDown", vec![Value::text("escape")]),
                        op(
                            "ActorPosition",
                            vec![Value::text("Nobody"), Value::text("X")],
                        ),
                    ],
                ),
            },
            K::Say {
                text: op(
                    "And",
                    vec![
                        op("KeyDown", vec![Value::text("space")]),
                        op(
                            "ActorPosition",
                            vec![Value::text("Friend"), Value::text("X")],
                        ),
                    ],
                ),
            },
        ],
        &[],
    );
}

#[test]
fn recursive_statement_blocks_keep_their_own_loop_tallies() {
    // Two blocks that call each other as statements. Each invocation keeps its
    // own counters now, the way the VM gives every call a frame.
    assert_same_blocks(
        "recursivestatement",
        vec![vec![
            calling("countdown", vec![number(3.0)]),
            K::Say {
                text: Value::text("done"),
            },
        ]],
        vec![block(
            "countdown",
            &["n"],
            vec![
                K::Say { text: param("n") },
                K::If {
                    condition: op("Gt", vec![param("n"), number(0.0)]),
                    body: vec![Instruction::new(calling(
                        "countdown",
                        vec![op("Sub", vec![param("n"), number(1.0)])],
                    ))],
                },
                K::Repeat {
                    count: number(2.0),
                    body: vec![Instruction::new(K::Say {
                        text: op("Join", vec![Value::text("round "), param("n")]),
                    })],
                },
            ],
        )],
        &[],
    );
}

/// A statement call inside a reporter's body. It runs on the reporter's own
/// state, so the frame it pushes is the reporter's and not the strand's.
#[test]
fn a_statement_call_inside_a_reporter_stays_inside_it() {
    assert_same_blocks(
        "nestedcall",
        vec![vec![
            K::Say {
                text: call("outer", vec![number(3.0)]),
            },
            K::Say {
                text: Value::text("after"),
            },
        ]],
        vec![
            block(
                "outer",
                &["n"],
                vec![
                    calling("inner", vec![param("n")]),
                    // The inner call has been and gone, so this reads the
                    // outer block's own input again.
                    K::Return {
                        value: op("Add", vec![param("n"), number(100.0)]),
                    },
                ],
            ),
            block(
                "inner",
                &["m"],
                vec![K::Move {
                    steps: op("Mul", vec![param("m"), number(2.0)]),
                }],
            ),
        ],
        &[],
    );
}

/// One body, called both ways. Its inputs are read back the same however it
/// was entered, since both bind them by position.
#[test]
fn one_block_serves_as_a_statement_and_as_a_reporter() {
    assert_same_blocks(
        "bothways",
        vec![vec![
            calling("b1", vec![number(2.0)]),
            K::Say {
                text: call("b1", vec![number(5.0)]),
            },
        ]],
        vec![block(
            "b1",
            &["n"],
            vec![
                K::Move { steps: param("n") },
                K::Return {
                    value: op("Add", vec![param("n"), number(1.0)]),
                },
            ],
        )],
        &[],
    );
}

/// A `stop all` inside a reporter ends the run - but not before the strand
/// that asked for the value finishes the slice it was in.
#[test]
fn stopping_from_inside_a_reporter_ends_the_run_the_same_way() {
    assert_same_blocks(
        "stopinside",
        vec![
            vec![
                K::Say {
                    text: call("b1", vec![]),
                },
                K::Say {
                    text: Value::text("still this tick"),
                },
            ],
            vec![K::Forever {
                body: vec![Instruction::new(K::Move { steps: number(1.0) })],
            }],
        ],
        vec![block(
            "b1",
            &[],
            vec![
                K::Say {
                    text: Value::text("about to stop"),
                },
                K::StopAll,
            ],
        )],
        &[],
    );
}

/// A call frame has to survive a suspension: the block waits, and the strand
/// has to come back into it and then find its way home - twice, from two
/// different call sites.
#[test]
fn a_call_frame_survives_the_wait_inside_it() {
    assert_same_blocks(
        "suspendedcall",
        vec![vec![
            calling("b1", vec![number(1.0)]),
            K::Say {
                text: Value::text("first back"),
            },
            calling("b1", vec![number(2.0)]),
            K::Say {
                text: Value::text("second back"),
            },
        ]],
        vec![block(
            "b1",
            &["n"],
            vec![
                K::Move { steps: param("n") },
                K::Wait {
                    duration: number(0.05),
                },
                // Read after the wait: the frame, and its inputs, are still
                // there when the strand picks the block back up.
                K::Say { text: param("n") },
            ],
        )],
        &[],
    );
}

// ─── Actors that come and go ────────────────────────────────────────────────
// A clone is the one place a compiled program runs one emitted function under
// an id the document never had, so what matters here is that both halves make
// the same actors, in the same order, and give them the same slices.

fn say(text: &str) -> K {
    K::Say {
        text: Value::text(text),
    }
}

fn clone_of(name: &str) -> K {
    K::CreateClone {
        of: name.to_string(),
    }
}

fn delete(target: &str) -> K {
    K::DeleteActor {
        target: Value::text(target),
    }
}

#[test]
fn a_clone_runs_its_own_strand_under_its_own_id() {
    assert_same_headed(
        "clones",
        vec![
            (
                K::WhenStarted,
                vec![clone_of(""), clone_of(""), say("made them")],
            ),
            (
                K::WhenCloned,
                vec![say("I am new"), K::Move { steps: number(1.0) }],
            ),
        ],
    );
}

#[test]
fn a_clone_of_a_clone_is_a_clone_of_the_same_authored_actor() {
    assert_same_headed(
        "clones-of-clones",
        vec![
            (K::WhenStarted, vec![clone_of("Player")]),
            (
                K::WhenCloned,
                vec![
                    say("copy"),
                    K::Wait {
                        duration: number(0.1),
                    },
                    K::StopAll,
                ],
            ),
        ],
    );
}

#[test]
fn a_clone_keeps_the_variables_its_template_had_and_counts_its_own() {
    assert_same_headed(
        "clone-variables",
        vec![
            (
                K::WhenStarted,
                vec![
                    K::SetVariable {
                        name: "hits".to_string(),
                        value: number(7.0),
                    },
                    clone_of(""),
                    K::ChangeVariable {
                        name: "hits".to_string(),
                        value: number(100.0),
                    },
                    say("template done"),
                ],
            ),
            (
                K::WhenCloned,
                vec![
                    K::ChangeVariable {
                        name: "hits".to_string(),
                        value: number(1.0),
                    },
                    K::Say {
                        text: Value::Var {
                            name: "hits".to_string(),
                        },
                    },
                ],
            ),
        ],
    );
}

#[test]
fn an_actor_made_mid_run_is_named_and_placed_the_same_way_by_both() {
    assert_same(
        "create-actor",
        vec![
            K::CreateActor {
                name: Value::text("Bullet"),
                x: number(3.0),
                y: op("Add", vec![number(2.0), number(2.0)]),
                z: number(0.0),
            },
            say("made one"),
            delete("Bullet"),
            say("and unmade it"),
        ],
        &[],
    );
}

#[test]
fn deleting_myself_ends_the_strand_where_it_stands() {
    assert_same_strands(
        "delete-myself",
        vec![
            vec![say("bye"), delete(""), say("never said")],
            vec![
                K::Wait {
                    duration: number(0.1),
                },
                say("the other strand went with it"),
            ],
        ],
        &[],
    );
}

#[test]
fn naming_nobody_is_reported_by_both_and_kills_neither() {
    assert_same(
        "no-such-actor",
        vec![
            delete("Nobody"),
            clone_of("Nobody"),
            say("carried on regardless"),
        ],
        &[],
    );
}

// ─── The interface ──────────────────────────────────────────────────────────
// Every `show` block reads a run of slots left to right, so what matters here
// is not only that the element comes out the same but that the two halves
// asked the world for its pieces in the same order - which is why the cases
// below put reporters in the slots rather than plain numbers.

fn panel(id: &str, title: Value, modal: bool, parent: &str) -> K {
    K::ShowPanel {
        element: Value::text(id),
        title,
        modal,
        anchor: UiAnchor::Center,
        x: number(0.0),
        y: number(0.0),
        width: number(240.0),
        height: number(0.0),
        parent: Value::text(parent),
    }
}

/// Says whatever a reporter answers, so the transcript carries the answer
/// itself rather than only the fact that something was asked.
fn say_value(value: Value) -> K {
    K::Say { text: value }
}

fn label(id: &str, text: Value, parent: &str) -> K {
    K::ShowLabel {
        element: Value::text(id),
        text,
        anchor: UiAnchor::TopLeft,
        x: number(12.0),
        y: number(12.0),
        width: number(0.0),
        height: number(0.0),
        parent: Value::text(parent),
    }
}

#[test]
fn every_show_block_asks_for_its_slots_in_the_same_order() {
    assert_same(
        "interface-show",
        vec![
            panel("menu", Value::text("Paused"), true, ""),
            K::ShowButton {
                element: Value::text("resume"),
                label: op("Join", vec![Value::text("Res"), Value::text("ume")]),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(0.0),
                width: number(0.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            K::ShowImage {
                element: Value::text("logo"),
                asset: Value::text("assets/logo.png"),
                anchor: UiAnchor::Top,
                x: number(0.0),
                y: number(8.0),
                width: number(64.0),
                height: number(64.0),
                parent: Value::text(""),
            },
            K::ShowInput {
                element: Value::text("name"),
                placeholder: Value::text("your name"),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(40.0),
                width: number(180.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            // The one row with three slots of its own, and the one whose
            // starting value is asked for rather than fixed.
            K::ShowSlider {
                element: Value::text("volume"),
                min: number(0.0),
                max: op("Add", vec![number(5.0), number(5.0)]),
                value: op("MyPosition", vec![Value::text("X")]),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(80.0),
                width: number(180.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
            K::ShowToggle {
                element: Value::text("shadows"),
                label: Value::text("Shadows"),
                on: true,
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(120.0),
                width: number(0.0),
                height: number(0.0),
                parent: Value::text("menu"),
            },
        ],
        &[],
    );
}

#[test]
fn writing_hiding_and_deleting_an_element_land_the_same_way() {
    assert_same(
        "interface-change",
        vec![
            label("score", Value::text("score: 0"), ""),
            K::SetUiProp {
                prop: UiProp::Text,
                element: Value::text("score"),
                value: op("Join", vec![Value::text("score: "), number(3.0)]),
            },
            K::SetUiProp {
                prop: UiProp::TextSize,
                element: Value::text("score"),
                value: number(22.0),
            },
            // A bad slot is reported once and stands a zero in its place,
            // on both sides and in the same place.
            K::SetUiProp {
                prop: UiProp::Width,
                element: Value::text("score"),
                value: op("Div", vec![number(1.0), number(0.0)]),
            },
            K::HideElement {
                element: Value::text("score"),
            },
            K::DeleteElement {
                element: Value::text("score"),
            },
            K::HideAllUi,
        ],
        &[],
    );
}

#[test]
fn an_inputs_rules_and_a_sliders_step_are_written_the_same_way() {
    assert_same(
        "interface-polish",
        vec![
            K::ShowInput {
                element: Value::text("name"),
                placeholder: Value::text("your name"),
                anchor: UiAnchor::Center,
                x: number(0.0),
                y: number(0.0),
                width: number(180.0),
                height: number(0.0),
                parent: Value::text(""),
            },
            K::SetUiProp {
                prop: UiProp::Allow,
                element: Value::text("name"),
                value: Value::text("letters"),
            },
            K::SetUiProp {
                prop: UiProp::MaxLength,
                element: Value::text("name"),
                value: op("Add", vec![number(8.0), number(4.0)]),
            },
            K::SetUiProp {
                prop: UiProp::Step,
                element: Value::text("volume"),
                // A bad slot is reported once and stands a zero in its
                // place, on both sides and in the same place.
                value: op("Div", vec![number(1.0), number(0.0)]),
            },
            // The keyboard given and taken back, by slot and by block.
            K::FocusElement {
                element: op("Join", vec![Value::text("na"), Value::text("me")]),
            },
            K::ClearFocus,
        ],
        &[],
    );
}

#[test]
fn lists_themes_and_saved_variables_land_the_same_way() {
    assert_same(
        "interface-v3",
        vec![
            K::ShowList {
                element: Value::text("scores"),
                anchor: UiAnchor::Right,
                x: number(-12.0),
                y: number(0.0),
                width: number(260.0),
                height: number(320.0),
                parent: Value::text(""),
            },
            K::SetUiTheme {
                theme: UiTheme::HighContrast,
            },
            K::SaveVariable {
                name: "score".to_string(),
            },
            K::ClearSavedVariable {
                name: "score".to_string(),
            },
        ],
        &[("score", Evaluated::Number(12.0))],
    );
}

#[test]
fn every_reporter_over_an_element_answers_the_same_on_both_sides() {
    assert_same(
        "interface-reporters",
        vec![
            say_value(op("UiValue", vec![Value::text("volume")])),
            say_value(op("UiSelectedIndex", vec![Value::text("volume")])),
            say_value(op("UiText", vec![Value::text("hint")])),
            say_value(op("UiShown", vec![Value::text("volume")])),
            say_value(op("UiShown", vec![Value::text("hint")])),
            say_value(op("UiExists", vec![Value::text("hint")])),
            say_value(op("UiExists", vec![Value::text("nothing")])),
            say_value(op("UiFocus", vec![])),
            // An id nothing answers to is reported once and stands a blank
            // in its place, the same way a missing actor is.
            say_value(op("UiText", vec![Value::text("nothing")])),
        ],
        &[],
    );
}

#[test]
fn physics_filters_and_queries_land_the_same_way() {
    assert_same(
        "physics-queries",
        vec![
            K::SetTrigger { trigger: true },
            K::SetTrigger { trigger: false },
            K::SetCollisionLayer { layer: number(3.0) },
            K::SetCollisionMask {
                mask: number(255.0),
            },
            // A bad layer is clamped, not refused, on both sides.
            K::SetCollisionLayer {
                layer: number(99.0),
            },
            // Reporters over the harness world: no bodies there, so every
            // ray and ball finds nothing, and nobody is a trigger.
            say_value(op("IsTrigger", vec![Value::text("Player")])),
            say_value(op("IsTrigger", vec![Value::text("Nobody")])),
            say_value(op("CollisionLayer", vec![Value::text("Player")])),
            say_value(op(
                "RayHit",
                vec![
                    number(0.0),
                    number(0.0),
                    number(0.0),
                    number(20.0),
                    number(0.0),
                    number(0.0),
                ],
            )),
            say_value(op(
                "RayDistance",
                vec![
                    number(0.0),
                    number(0.0),
                    number(0.0),
                    number(20.0),
                    number(0.0),
                    number(0.0),
                ],
            )),
            say_value(op(
                "CircleHit",
                vec![number(3.0), number(7.0), number(0.0), number(5.0)],
            )),
        ],
        &[],
    );
}

#[test]
fn a_paused_world_stops_every_strand_the_interface_did_not_start() {
    assert_same_strands(
        "interface-pause",
        vec![
            vec![
                say("before"),
                K::PauseGame,
                // The pause takes hold where it stands, the strand that ran
                // it included, so neither of these ever runs.
                say("never said"),
                K::Wait {
                    duration: number(0.1),
                },
                say("nor this"),
            ],
            // A second strand, later in the same tick: frozen too, wherever
            // in the tick the block landed.
            vec![
                K::Wait {
                    duration: number(0.05),
                },
                say("nor this either"),
            ],
        ],
        &[],
    );
}

#[test]
fn a_strand_the_interface_started_runs_through_a_pause_and_ends_it() {
    assert_same_headed(
        "interface-resume",
        vec![
            (
                K::WhenStarted,
                vec![say("before"), K::PauseGame, say("after")],
            ),
            // The interface's own strand: it pauses nothing by running, and
            // a `pause game` inside it doesn't stop it either.
            (
                K::WhenUiClicked {
                    element: "resume".to_string(),
                },
                vec![
                    K::PauseGame,
                    say("the menu is alive"),
                    K::ResumeGame,
                    say("and the world is back"),
                ],
            ),
            // Frozen at its `wait` until that resume lands, then it finishes.
            (
                K::WhenStarted,
                vec![
                    K::Wait {
                        duration: number(0.05),
                    },
                    say("the world moved again"),
                ],
            ),
        ],
    );
}

#[test]
fn escape_toggles_a_pause_menu_on_both_sides() {
    // Both harnesses fire escape at tick 3, while the world stands still.
    // The strand has to run there rather than staying frozen, on each side.
    let case = "interface-escape";
    if !toolchain() {
        return;
    }
    let project = project_with_headers(
        vec![
            (K::WhenStarted, vec![say("down"), K::PauseGame]),
            (
                K::WhenKeyPressed {
                    key: "escape".to_string(),
                },
                vec![say("up"), K::ResumeGame],
            ),
        ],
        Vec::new(),
        &[],
    );
    let interpreted = by_vm(&project);
    let compiled = by_compiler(&project, &[], case);
    assert_eq!(
        interpreted, compiled,
        "{case}: the VM and the compiled program disagreed"
    );
    assert!(
        interpreted
            .iter()
            .any(|line| line.contains("|Say") && line.contains("up")),
        "{case}: escape while paused never ran its strand"
    );
}

#[test]
fn framework_widgets_and_property_blocks_match_native_logic() {
    let mut blocks = Vec::new();
    for i in 8..23 {
        blocks.push(K::ShowWidget {
            kind: blockloom_core::ui::UiKind::from_index(i),
            element: Value::text(format!("widget{i}")),
            text: Value::text("caption"),
            anchor: UiAnchor::TopLeft,
            x: number(10.),
            y: number(20.),
            width: number(180.),
            height: number(40.),
            parent: Value::text(""),
        });
    }
    blocks.extend([
        K::BindUi {
            element: Value::text("widget15"),
            value: Value::text(
                r#"[{"property":"Value","source":{"Variable":{"actor":"","name":"health"}}}]"#,
            ),
        },
        K::SetUiItems {
            element: Value::text("widget17"),
            value: Value::text(r#"["Sword","Shield"]"#),
        },
        K::ScrollUi {
            element: Value::text("widget17"),
            value: number(96.),
        },
        K::SetElementTheme {
            element: Value::text("widget17"),
            value: Value::text("Light"),
        },
        K::SetUiProp {
            element: Value::text("widget8"),
            prop: UiProp::Layout,
            value: Value::text(r#"{"columns":3,"gap":12}"#),
        },
    ]);
    assert_same("interface-framework", blocks, &[]);
}

#[test]
fn bubbled_ui_event_strands_run_while_the_world_is_paused() {
    assert_same_headed(
        "interface-hover",
        vec![
            (K::WhenStarted, vec![K::PauseGame]),
            (
                K::WhenUiEvent {
                    element: "resume".into(),
                    event: "hover".into(),
                },
                vec![
                    say("hover"),
                    K::Wait {
                        duration: number(0.05),
                    },
                    say("still alive"),
                ],
            ),
            (
                K::WhenUiEvent {
                    element: "other".into(),
                    event: "hover".into(),
                },
                vec![say("wrong widget")],
            ),
        ],
    );
}

#[test]
fn render_settings_match_compiled_logic() {
    use blockloom_core::quality::Setting;
    assert_same(
        "render-settings",
        vec![
            K::SetRenderSetting {
                setting: Setting::Quality,
                value: Value::text("Medium"),
            },
            K::SetRenderSetting {
                setting: Setting::ResolutionScale,
                value: number(0.75),
            },
            K::SetRenderSetting {
                setting: Setting::Upscaler,
                value: Value::text("Taa"),
            },
            K::SetRenderSetting {
                setting: Setting::DlssMode,
                value: Value::text("Balanced"),
            },
        ],
        &[],
    );
}
