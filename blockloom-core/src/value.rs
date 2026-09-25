//! Blockloom's view of the value system. The expression tree and its
//! evaluation live in `blockstitch-core`; the reporter blocks Blockloom adds
//! - the sensing half of a game engine - are here.
//!
//! An extension operator is a plain `fn` with no context, so these read the
//! frame snapshot in [`crate::sense`] instead of taking the world as an
//! argument.

pub use blockstitch_core::value::*;

use crate::physics_query;
use crate::scene::Axis;
use crate::sense;
use crate::sound::{SoundBus, normalize_sound};

fn text(value: &str) -> Value {
    Value::text(value)
}

fn number(value: f64) -> Value {
    Value::number(value)
}

fn axis_of(arg: Option<&Evaluated>) -> Axis {
    match arg.map(Evaluated::as_text).unwrap_or_default().as_str() {
        "Y" | "y" => Axis::Y,
        "Z" | "z" => Axis::Z,
        _ => Axis::X,
    }
}

fn num(arg: Option<&Evaluated>) -> f64 {
    arg.and_then(|value| value.as_number().ok()).unwrap_or(0.0)
}

/// The running actor, or an error naming the reason there isn't one - a
/// reporter previewed in the editor has no actor context.
fn me() -> Result<sense::ActorSense, String> {
    let id = sense::current_actor().ok_or("no actor is running this script")?;
    sense::read(|sensors| {
        sensors
            .actors
            .get(&id)
            .cloned()
            .ok_or_else(|| "this actor isn't in the running world".to_string())
    })
}

/// One interface element, or an error naming the id nothing answers to -
/// a missing element is a mistake worth reporting, the way a missing actor
/// is, rather than a silent empty string.
fn element(id: &str, f: impl FnOnce(&sense::UiSense) -> Evaluated) -> Result<Evaluated, String> {
    sense::read(|sensors| {
        sensors
            .ui
            .get(id.trim())
            .map(f)
            .ok_or_else(|| format!("there's no interface element called \"{id}\""))
    })
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let dx = (a[0] - b[0]) as f64;
    let dy = (a[1] - b[1]) as f64;
    let dz = (a[2] - b[2]) as f64;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Blockloom's own operators, in blockstitch's extension shape. The wire
/// names double as the palette `kind` strings the frontend registers.
static OPERATORS: &[ExtOperator] = &[
    ExtOperator {
        kind: "KeyDown",
        op: "KeyDown",
        arity: 1,
        default_args: || vec![text("space")],
        eval: |args| {
            let key = sense::normalize_key(&args[0].as_text());
            Ok(Evaluated::Bool(sense::read(|s| s.keys.contains(&key))))
        },
    },
    ExtOperator {
        kind: "MouseX",
        op: "MouseX",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.mouse[0]) as f64)),
    },
    ExtOperator {
        kind: "MouseY",
        op: "MouseY",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.mouse[1]) as f64)),
    },
    ExtOperator {
        kind: "MouseDeltaX",
        op: "MouseDeltaX",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.mouse_delta[0]) as f64)),
    },
    ExtOperator {
        kind: "MouseDeltaY",
        op: "MouseDeltaY",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.mouse_delta[1]) as f64)),
    },
    ExtOperator {
        kind: "MouseDown",
        op: "MouseDown",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.mouse_down))),
    },
    ExtOperator {
        kind: "MouseLocked",
        op: "MouseLocked",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.mouse_locked))),
    },
    ExtOperator {
        kind: "MouseButtonDown",
        op: "MouseButtonDown",
        arity: 1,
        default_args: || vec![text("right")],
        // Which mouse button is down: left, right or middle. Left is what
        // `mouse down?` has always meant.
        eval: |args| {
            let button = args[0].as_text().trim().to_lowercase();
            Ok(Evaluated::Bool(sense::read(|s| {
                if button == "left" {
                    s.mouse_down
                } else {
                    s.mouse_buttons.contains(&button)
                }
            })))
        },
    },
    ExtOperator {
        kind: "ActionDown",
        op: "ActionDown",
        arity: 1,
        default_args: || vec![text("Jump")],
        // Whether the named input action is held right now, however its
        // bindings say it: keys, mouse buttons, pad buttons or sticks.
        eval: |args| {
            let name = args[0].as_text();
            Ok(Evaluated::Bool(sense::read(|s| {
                s.actions
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
                    .is_some_and(|(_, action)| action.held)
            })))
        },
    },
    ExtOperator {
        kind: "ActionPressed",
        op: "ActionPressed",
        arity: 1,
        default_args: || vec![text("Jump")],
        // True only on the frame the action went down.
        eval: |args| {
            let name = args[0].as_text();
            Ok(Evaluated::Bool(sense::read(|s| {
                s.actions
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
                    .is_some_and(|(_, action)| action.pressed)
            })))
        },
    },
    ExtOperator {
        kind: "ActionReleased",
        op: "ActionReleased",
        arity: 1,
        default_args: || vec![text("Jump")],
        // True only on the frame the action went up.
        eval: |args| {
            let name = args[0].as_text();
            Ok(Evaluated::Bool(sense::read(|s| {
                s.actions
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
                    .is_some_and(|(_, action)| action.released)
            })))
        },
    },
    ExtOperator {
        kind: "ActionValue",
        op: "ActionValue",
        arity: 1,
        default_args: || vec![text("Left")],
        // The strongest binding's analog value: 0/1 for buttons, -1..1 for
        // a whole stick axis, 0..1 for a directed half.
        eval: |args| {
            let name = args[0].as_text();
            Ok(Evaluated::Number(sense::read(|s| {
                s.actions
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
                    .map(|(_, action)| action.value as f64)
                    .unwrap_or(0.0)
            })))
        },
    },
    ExtOperator {
        kind: "TouchCount",
        op: "TouchCount",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.touches.len() as f64))),
    },
    ExtOperator {
        kind: "TouchX",
        op: "TouchX",
        arity: 1,
        default_args: || vec![text("1")],
        // The 1-based touch point's world x. Out of range reads as zero
        // rather than an error, since fingers come and go every frame.
        eval: |args| {
            let index = args[0].as_number().unwrap_or(0.0) as usize;
            Ok(Evaluated::Number(sense::read(|s| {
                index
                    .checked_sub(1)
                    .and_then(|i| s.touches.get(i))
                    .map(|touch| touch.position[0] as f64)
                    .unwrap_or(0.0)
            })))
        },
    },
    ExtOperator {
        kind: "TouchY",
        op: "TouchY",
        arity: 1,
        default_args: || vec![text("1")],
        eval: |args| {
            let index = args[0].as_number().unwrap_or(0.0) as usize;
            Ok(Evaluated::Number(sense::read(|s| {
                index
                    .checked_sub(1)
                    .and_then(|i| s.touches.get(i))
                    .map(|touch| touch.position[1] as f64)
                    .unwrap_or(0.0)
            })))
        },
    },
    ExtOperator {
        kind: "GamepadConnected",
        op: "GamepadConnected",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.gamepad_connected))),
    },
    ExtOperator {
        kind: "GamepadAxis",
        op: "GamepadAxis",
        arity: 1,
        default_args: || vec![text("LeftStickX")],
        // A live stick or trigger value, -1..1. Unknown names read as zero.
        eval: |args| {
            let axis = crate::input::normalize_pad_axis(&args[0].as_text());
            Ok(Evaluated::Number(
                sense::read(|s| s.gamepad_axes.get(&axis).copied().unwrap_or(0.0)) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "GamepadButtonDown",
        op: "GamepadButtonDown",
        arity: 1,
        default_args: || vec![text("South")],
        // Whether the named pad button is held. Most games read this
        // through an action instead, so remapping keeps working.
        eval: |args| {
            let button = crate::input::normalize_pad_button(&args[0].as_text());
            Ok(Evaluated::Bool(sense::read(|s| {
                s.gamepad_buttons.contains(&button)
            })))
        },
    },
    ExtOperator {
        kind: "IsTweening",
        op: "IsTweening",
        arity: 0,
        default_args: Vec::new,
        // Whether any tween (a glide or a `tween ...` block) is still moving
        // me. What a strand waits on before starting the next hop.
        eval: |_| Ok(Evaluated::Bool(me()?.tweening)),
    },
    ExtOperator {
        kind: "CurrentClip",
        op: "CurrentClip",
        arity: 0,
        default_args: Vec::new,
        // The clip the animation player is holding, or empty for none. The
        // state name in a clip-per-state project, which is what a state
        // machine transition switches on.
        eval: |_| Ok(Evaluated::Text(me()?.anim_clip)),
    },
    ExtOperator {
        kind: "CurrentFrame",
        op: "CurrentFrame",
        arity: 0,
        default_args: Vec::new,
        // The 1-based frame showing right now. Zero with no clip.
        eval: |_| Ok(Evaluated::Number(me()?.anim_frame as f64)),
    },
    ExtOperator {
        kind: "AnimationPlaying",
        op: "AnimationPlaying",
        arity: 0,
        default_args: Vec::new,
        // Whether the player's clip is still advancing. A `Once` clip at
        // its end reads as false, which is when `when animation ends` fires.
        eval: |_| Ok(Evaluated::Bool(me()?.anim_playing)),
    },
    ExtOperator {
        kind: "Timer",
        op: "Timer",
        arity: 0,
        default_args: Vec::new,
        // A strand the interface started reads the wall clock: a menu is up
        // precisely when the world's own clock is frozen.
        eval: |_| {
            let ui = sense::in_ui_strand();
            Ok(Evaluated::Number(sense::read(|s| s.clock(ui))))
        },
    },
    ExtOperator {
        kind: "GamePaused",
        op: "GamePaused",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.paused))),
    },
    ExtOperator {
        kind: "UiSelectedIndex",
        op: "UiSelectedIndex",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| element(&args[0].as_text(), |e| e.value.clone()),
    },
    ExtOperator {
        kind: "UiValue",
        op: "UiValue",
        arity: 1,
        default_args: || vec![text("")],
        // A slider's number, a toggle's on/off, an input's text - whatever
        // that kind of element has to report. An id nothing answers to is an
        // error rather than a silent zero, as a missing actor is.
        eval: |args| {
            let id = args[0].as_text();
            element(&id, |element| element.value.clone())
        },
    },
    ExtOperator {
        kind: "UiText",
        op: "UiText",
        arity: 1,
        default_args: || vec![text("")],
        // The words an element is showing: a label's text, a button's
        // caption, what has been typed into an input or, while nothing has,
        // its placeholder.
        eval: |args| {
            let id = args[0].as_text();
            element(&id, |element| Evaluated::Text(element.text.clone()))
        },
    },
    ExtOperator {
        kind: "UiShown",
        op: "UiShown",
        arity: 1,
        default_args: || vec![text("")],
        // Whether it is on the screen right now, which a hidden parent
        // decides as surely as its own `hide` does. An id nothing answers to
        // is not shown - unlike `value of`, there is a sensible answer here.
        eval: |args| {
            let id = args[0].as_text();
            Ok(Evaluated::Bool(sense::read(|sensors| {
                sensors
                    .ui
                    .get(id.trim())
                    .is_some_and(|element| element.shown)
            })))
        },
    },
    ExtOperator {
        kind: "UiExists",
        op: "UiExists",
        arity: 1,
        default_args: || vec![text("")],
        // Whether the blocks have made one by that name at all - hidden
        // still counts, since `hide` doesn't forget an element.
        eval: |args| {
            let id = args[0].as_text();
            Ok(Evaluated::Bool(sense::read(|sensors| {
                sensors.ui.contains_key(id.trim())
            })))
        },
    },
    ExtOperator {
        kind: "UiFocus",
        op: "UiFocus",
        arity: 0,
        default_args: Vec::new,
        // Which text input holds the keyboard, or an empty string. What a
        // strand checks before reading a key itself.
        eval: |_| Ok(Evaluated::Text(sense::read(|s| s.ui_focus.clone()))),
    },
    ExtOperator {
        kind: "MyPosition",
        op: "MyPosition",
        arity: 1,
        default_args: || vec![text("X")],
        eval: |args| {
            let me = me()?;
            Ok(Evaluated::Number(
                me.position[axis_of(args.first()).index()] as f64,
            ))
        },
    },
    ExtOperator {
        kind: "MyRotation",
        op: "MyRotation",
        arity: 1,
        default_args: || vec![text("Z")],
        eval: |args| {
            let me = me()?;
            Ok(Evaluated::Number(
                me.rotation[axis_of(args.first()).index()] as f64,
            ))
        },
    },
    ExtOperator {
        kind: "MyLocalPosition",
        op: "MyLocalPosition",
        arity: 1,
        default_args: || vec![text("X")],
        // Where I stand in my parent's frame - the world position itself
        // when I hang off nothing, so this never needs a parent to answer.
        eval: |args| {
            let me = me()?;
            Ok(Evaluated::Number(
                me.local_position[axis_of(args.first()).index()] as f64,
            ))
        },
    },
    ExtOperator {
        kind: "Touching",
        op: "Touching",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| {
            let me = me()?;
            let target = args[0].as_text();
            // An empty target asks "touching anything at all?".
            if target.trim().is_empty() {
                return Ok(Evaluated::Bool(!me.touching.is_empty()));
            }
            Ok(Evaluated::Bool(sense::read(|sensors| {
                me.touching.iter().any(|id| {
                    id == &target
                        || sensors
                            .actors
                            .get(id)
                            .is_some_and(|other| other.name.eq_ignore_ascii_case(&target))
                })
            })))
        },
    },
    ExtOperator {
        kind: "DistanceTo",
        op: "DistanceTo",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| {
            let me = me()?;
            let target = args[0].as_text();
            sense::read(|sensors| {
                // "mouse" is a valid target here, same as for `point towards`.
                if target.eq_ignore_ascii_case("mouse") {
                    let mouse = [sensors.mouse[0], sensors.mouse[1], me.position[2]];
                    return Ok(Evaluated::Number(distance(me.position, mouse)));
                }
                let other = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Number(distance(me.position, other.position)))
            })
        },
    },
    ExtOperator {
        kind: "ComponentField",
        op: "ComponentField",
        arity: 2,
        default_args: || vec![text(""), text("")],
        eval: |args| {
            let component = args[0].as_text();
            let field = args[1].as_text();
            let me = me()?;
            me.components
                .get(component.trim())
                .and_then(|fields| fields.get(field.trim()))
                .cloned()
                .ok_or_else(|| format!("I have no \"{component}\" component with a \"{field}\""))
        },
    },
    ExtOperator {
        kind: "IsClone",
        op: "IsClone",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(me()?.is_clone)),
    },
    ExtOperator {
        kind: "MyParent",
        op: "MyParent",
        arity: 0,
        default_args: Vec::new,
        // The id rather than the name: clones share a name, and this is what
        // `set my parent to` and `delete` want handed back to them.
        eval: |_| Ok(Evaluated::Text(me()?.parent)),
    },
    ExtOperator {
        kind: "NewActor",
        op: "NewActor",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Text(me()?.last_created)),
    },
    ExtOperator {
        kind: "ActorCount",
        op: "ActorCount",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| {
            let name = args[0].as_text();
            Ok(Evaluated::Number(
                sense::read(|sensors| sensors.count_named(&name)) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "ActorPosition",
        op: "ActorPosition",
        arity: 2,
        default_args: || vec![text(""), text("X")],
        eval: |args| {
            let name = args[0].as_text();
            let axis = axis_of(args.get(1));
            sense::read(|sensors| {
                let actor = sensors
                    .find(&name)
                    .ok_or_else(|| format!("there's no actor named \"{name}\""))?;
                Ok(Evaluated::Number(actor.position[axis.index()] as f64))
            })
        },
    },
    ExtOperator {
        kind: "ActorLocalPosition",
        op: "ActorLocalPosition",
        arity: 2,
        default_args: || vec![text(""), text("X")],
        // Another actor's place in its own parent's frame - its world
        // position when it hangs off nothing, mirroring `MyLocalPosition`.
        eval: |args| {
            let name = args[0].as_text();
            let axis = axis_of(args.get(1));
            sense::read(|sensors| {
                let actor = sensors
                    .find(&name)
                    .ok_or_else(|| format!("there's no actor named \"{name}\""))?;
                Ok(Evaluated::Number(actor.local_position[axis.index()] as f64))
            })
        },
    },
    ExtOperator {
        kind: "SoundPlaying",
        op: "SoundPlaying",
        arity: 1,
        default_args: || vec![text("")],
        // Whether any voice is playing that file right now. An empty slot
        // asks about nothing in particular, so it answers no.
        eval: |args| {
            let sound = normalize_sound(&args[0].as_text());
            Ok(Evaluated::Bool(sense::read(|sensors| {
                sensors.sounds.contains(&sound)
            })))
        },
    },
    ExtOperator {
        kind: "BusVolume",
        op: "BusVolume",
        arity: 1,
        default_args: || vec![text("Sfx")],
        // The live gain of a mixing bus in 0-100, as the play blocks speak
        // it. An unknown bus is a mistake worth reporting, not a silent zero.
        eval: |args| {
            let bus = args[0].as_text();
            match SoundBus::parse(&bus) {
                Some(bus) => Ok(Evaluated::Number(sense::read(|sensors| {
                    sensors.bus_volumes.get(&bus).copied().unwrap_or(100.0)
                }) as f64)),
                None => Err(format!("there's no \"{bus}\" mixing bus")),
            }
        },
    },
    ExtOperator {
        kind: "Atmosphere",
        op: "Atmosphere",
        arity: 1,
        default_args: || vec![text("wind speed")],
        // One reading of the air as of this fixed tick. An unknown name is
        // reported rather than read as calm.
        eval: |args| {
            let name = args[0].as_text();
            sense::read(|sensors| sensors.atmosphere.field(&name))
                .map(Evaluated::Number)
                .ok_or_else(|| format!("the atmosphere has no \"{name}\" reading"))
        },
    },
    ExtOperator {
        kind: "SceneLuminance",
        op: "SceneLuminance",
        arity: 0,
        default_args: Vec::new,
        // Sampled with the air on the fixed tick, so every scheduler agrees.
        eval: |_| {
            Ok(Evaluated::Number(
                sense::read(|s| s.atmosphere.luminance) as f64
            ))
        },
    },
    ExtOperator {
        kind: "IsHdrDisplay",
        op: "IsHdrDisplay",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.atmosphere.hdr_display))),
    },
    ExtOperator {
        kind: "PeakBrightness",
        op: "PeakBrightness",
        arity: 0,
        default_args: Vec::new,
        eval: |_| {
            Ok(Evaluated::Number(
                sense::read(|s| s.atmosphere.peak_brightness) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "ActiveVolumes",
        op: "ActiveVolumes",
        arity: 0,
        default_args: Vec::new,
        // A JSON list of names in blend order, so `load json into list`
        // takes it. Sampled with the air on the fixed tick.
        eval: |_| {
            let names = sense::read(|s| s.atmosphere.volumes.clone());
            Ok(Evaluated::Text(
                serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string()),
            ))
        },
    },
    ExtOperator {
        kind: "IsTrigger",
        op: "IsTrigger",
        arity: 1,
        default_args: || vec![text("")],
        // Whether an actor's collider is a trigger: it senses without
        // pushing. Empty names the running actor itself.
        eval: |args| {
            let target = args[0].as_text();
            if target.trim().is_empty() {
                return Ok(Evaluated::Bool(me()?.trigger));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Bool(actor.trigger))
            })
        },
    },
    ExtOperator {
        kind: "CastsShadows",
        op: "CastsShadows",
        arity: 1,
        default_args: || vec![text("")],
        // Whether an actor carries a light casting shadows right now, after
        // any `turn my light's shadows` this run. Empty names this actor.
        eval: |args| {
            let target = args[0].as_text();
            if target.trim().is_empty() {
                return Ok(Evaluated::Bool(me()?.casts_shadows));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Bool(actor.casts_shadows))
            })
        },
    },
    ExtOperator {
        kind: "CollisionLayer",
        op: "CollisionLayer",
        arity: 1,
        default_args: || vec![text("")],
        // Which layer an actor lives on, 1-8. Empty names the running actor.
        eval: |args| {
            let target = args[0].as_text();
            if target.trim().is_empty() {
                return Ok(Evaluated::Number(me()?.layer as f64));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Number(actor.layer as f64))
            })
        },
    },
    ExtOperator {
        kind: "RayHit",
        op: "RayHit",
        arity: 6,
        default_args: || {
            vec![
                number(0.0),
                number(0.0),
                number(0.0),
                number(100.0),
                number(0.0),
                number(0.0),
            ]
        },
        // The first body a segment hits, by name, or empty for nothing.
        // Bodies only; the querier itself is skipped; layers filter by the
        // running actor's own mask, or not at all outside a run.
        eval: |args| {
            let from = [
                num(args.first()) as f32,
                num(args.get(1)) as f32,
                num(args.get(2)) as f32,
            ];
            let to = [
                num(args.get(3)) as f32,
                num(args.get(4)) as f32,
                num(args.get(5)) as f32,
            ];
            Ok(Evaluated::Text(sense::read(|sensors| {
                let skip = sense::current_actor();
                let mask = physics_query::query_mask(sensors, skip.as_deref());
                physics_query::ray_hit(sensors, from, to, skip.as_deref(), mask)
                    .and_then(|(id, _)| sensors.actors.get(&id).map(|actor| actor.name.clone()))
                    .unwrap_or_default()
            })))
        },
    },
    ExtOperator {
        kind: "RayDistance",
        op: "RayDistance",
        arity: 6,
        default_args: || {
            vec![
                number(0.0),
                number(0.0),
                number(0.0),
                number(100.0),
                number(0.0),
                number(0.0),
            ]
        },
        // How far along the segment the first hit sits, or -1 for nothing.
        eval: |args| {
            let from = [
                num(args.first()) as f32,
                num(args.get(1)) as f32,
                num(args.get(2)) as f32,
            ];
            let to = [
                num(args.get(3)) as f32,
                num(args.get(4)) as f32,
                num(args.get(5)) as f32,
            ];
            Ok(Evaluated::Number(sense::read(|sensors| {
                let skip = sense::current_actor();
                let mask = physics_query::query_mask(sensors, skip.as_deref());
                physics_query::ray_hit(sensors, from, to, skip.as_deref(), mask)
                    .map(|(_, distance)| distance as f64)
                    .unwrap_or(-1.0)
            })))
        },
    },
    ExtOperator {
        kind: "CircleHit",
        op: "CircleHit",
        arity: 4,
        default_args: || vec![number(0.0), number(0.0), number(0.0), number(10.0)],
        // The nearest body a ball overlaps, by name, or empty. The shape-cast
        // half of the query pair: a ground check is a small ball underfoot.
        eval: |args| {
            let center = [
                num(args.first()) as f32,
                num(args.get(1)) as f32,
                num(args.get(2)) as f32,
            ];
            let radius = num(args.get(3)) as f32;
            Ok(Evaluated::Text(sense::read(|sensors| {
                let skip = sense::current_actor();
                let mask = physics_query::query_mask(sensors, skip.as_deref());
                physics_query::overlap_circle(sensors, center, radius, skip.as_deref(), mask)
                    .into_iter()
                    .next()
                    .and_then(|id| sensors.actors.get(&id).map(|actor| actor.name.clone()))
                    .unwrap_or_default()
            })))
        },
    },
];

/// Teaches blockstitch about [`OPERATORS`]. Idempotent; called from
/// [`crate::init`], which every entry point runs before touching a project.
pub fn register_blockloom_operators() {
    register_operators(OPERATORS);
}

/// The sensing reporters as JSON-ready specs for the block vocabulary: the
/// wire `op` name, how many args it takes, and its default args as a usage
/// example. A reporter slots into any value slot as a `Value::Op`.
pub fn reporter_specs() -> Vec<serde_json::Value> {
    OPERATORS
        .iter()
        .map(|op| {
            serde_json::json!({
                "name": op.kind,
                "arity": op.arity,
                "exampleArgs": (op.default_args)(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sense::{ActorSense, Sensors};

    #[test]
    fn sensing_operators_read_the_published_snapshot() {
        register_blockloom_operators();
        let mut sensors = Sensors {
            time: 4.5,
            mouse_delta: [12.0, -7.0],
            mouse_locked: true,
            ..Default::default()
        };
        sensors.keys.insert("space".to_string());
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Player".to_string(),
                position: [3.0, 7.0, 0.0],
                local_position: [1.0, 2.0, 0.0],
                ..Default::default()
            },
        );
        sense::publish(sensors);

        let key_down = Value::op(Op::from_name("KeyDown"), vec![Value::text("Space")]);
        assert_eq!(key_down.eval(), Ok(Evaluated::Bool(true)));
        let timer = Value::op(Op::from_name("Timer"), vec![]);
        assert_eq!(timer.eval(), Ok(Evaluated::Number(4.5)));

        let delta_x = Value::op(Op::from_name("MouseDeltaX"), vec![]);
        assert_eq!(delta_x.eval(), Ok(Evaluated::Number(12.0)));
        let delta_y = Value::op(Op::from_name("MouseDeltaY"), vec![]);
        assert_eq!(delta_y.eval(), Ok(Evaluated::Number(-7.0)));
        let locked = Value::op(Op::from_name("MouseLocked"), vec![]);
        assert_eq!(locked.eval(), Ok(Evaluated::Bool(true)));

        let my_y = Value::op(Op::from_name("MyPosition"), vec![Value::text("Y")]);
        // Outside a script there's no actor, so "my y position" is an error,
        // not a silent zero.
        assert!(my_y.eval().is_err());
        sense::with_actor("a1", || {
            assert_eq!(my_y.eval(), Ok(Evaluated::Number(7.0)));
        });

        let other_x = Value::op(
            Op::from_name("ActorPosition"),
            vec![Value::text("Player"), Value::text("X")],
        );
        assert_eq!(other_x.eval(), Ok(Evaluated::Number(3.0)));

        // The parent-frame twins answer the local position instead, and
        // complain about a missing actor in the same words.
        let my_local = Value::op(Op::from_name("MyLocalPosition"), vec![Value::text("Y")]);
        sense::with_actor("a1", || {
            assert_eq!(my_local.eval(), Ok(Evaluated::Number(2.0)));
        });
        let other_local = Value::op(
            Op::from_name("ActorLocalPosition"),
            vec![Value::text("Player"), Value::text("X")],
        );
        assert_eq!(other_local.eval(), Ok(Evaluated::Number(1.0)));
        let missing_local = Value::op(
            Op::from_name("ActorLocalPosition"),
            vec![Value::text("Nobody"), Value::text("X")],
        );
        assert!(missing_local.eval().is_err());
    }

    #[test]
    fn physics_queries_read_bodies_through_the_snapshot() {
        use crate::sense::ColliderShape;
        register_blockloom_operators();
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "me".to_string(),
            ActorSense {
                name: "Hero".to_string(),
                position: [0.0, 0.0, 0.0],
                has_body: true,
                trigger: true,
                layer: 2,
                shape: ColliderShape::Box {
                    half: [1.0, 1.0, 1.0],
                },
                ..Default::default()
            },
        );
        sensors.actors.insert(
            "wall".to_string(),
            ActorSense {
                name: "Wall".to_string(),
                position: [5.0, 0.0, 0.0],
                has_body: true,
                shape: ColliderShape::Box {
                    half: [1.0, 1.0, 1.0],
                },
                ..Default::default()
            },
        );
        sense::publish(sensors);

        // Empty names the running actor: a trigger coin says so about
        // itself, and a wall says otherwise.
        let trigger = Value::op(Op::from_name("IsTrigger"), vec![Value::text("")]);
        sense::with_actor("me", || {
            assert_eq!(trigger.eval(), Ok(Evaluated::Bool(true)));
        });
        sense::with_actor("wall", || {
            assert_eq!(trigger.eval(), Ok(Evaluated::Bool(false)));
        });
        let wall_trigger = Value::op(Op::from_name("IsTrigger"), vec![Value::text("Wall")]);
        assert_eq!(wall_trigger.eval(), Ok(Evaluated::Bool(false)));
        let missing = Value::op(Op::from_name("IsTrigger"), vec![Value::text("Nobody")]);
        assert!(missing.eval().is_err());

        let layer = Value::op(Op::from_name("CollisionLayer"), vec![Value::text("")]);
        sense::with_actor("me", || {
            assert_eq!(layer.eval(), Ok(Evaluated::Number(2.0)));
        });

        // A ray past the wall names it and measures to its near face; a
        // ray at the sky names nothing and measures -1.
        let hit = Value::op(
            Op::from_name("RayHit"),
            vec![
                Value::number(0.0),
                Value::number(0.0),
                Value::number(0.0),
                Value::number(20.0),
                Value::number(0.0),
                Value::number(0.0),
            ],
        );
        sense::with_actor("me", || {
            assert_eq!(hit.eval(), Ok(Evaluated::Text("Wall".to_string())));
        });
        let distance = Value::op(
            Op::from_name("RayDistance"),
            vec![
                Value::number(0.0),
                Value::number(0.0),
                Value::number(0.0),
                Value::number(20.0),
                Value::number(0.0),
                Value::number(0.0),
            ],
        );
        sense::with_actor("me", || {
            assert_eq!(distance.eval(), Ok(Evaluated::Number(4.0)));
        });
        let sky = Value::op(
            Op::from_name("RayHit"),
            vec![
                Value::number(0.0),
                Value::number(50.0),
                Value::number(0.0),
                Value::number(20.0),
                Value::number(50.0),
                Value::number(0.0),
            ],
        );
        assert_eq!(sky.eval(), Ok(Evaluated::Text(String::new())));

        // A ball over the wall names it; a ball in the sky names nothing.
        let circle = Value::op(
            Op::from_name("CircleHit"),
            vec![
                Value::number(5.0),
                Value::number(0.0),
                Value::number(0.0),
                Value::number(2.0),
            ],
        );
        assert_eq!(circle.eval(), Ok(Evaluated::Text("Wall".to_string())));
    }
}
