//! Blockloom's view of the value system. The expression tree and its
//! evaluation live in `blockstitch-core`; the reporter blocks Blockloom adds
//! - the sensing half of a game engine - are here.
//!
//! An extension operator is a plain `fn` with no context, so these read the
//! frame snapshot in [`crate::sense`] instead of taking the world as an
//! argument.

pub use blockstitch_core::value::*;

use crate::physics::query::{self, QueryFilter, QueryHit, QueryRequest};
use crate::scene::Axis;
use crate::sense;
use crate::sound::{SoundBus, normalize_sound};

/// The one operator every plugin reporter is stored as: `args` are the
/// plugin's id, the block's type id, then the block's slots in schema order.
pub const PLUGIN_READ: &str = "PluginRead";

/// A slot's value as a plugin sees it. A whole number is an integer, so it
/// satisfies an integer field as well as a number one.
pub fn json_of(value: &Evaluated) -> serde_json::Value {
    match value {
        Evaluated::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => {
            serde_json::Value::from(*n as i64)
        }
        Evaluated::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Evaluated::Text(s) => serde_json::Value::String(s.clone()),
        Evaluated::Bool(b) => serde_json::Value::Bool(*b),
    }
}

/// What a plugin answered, as a value: lists and vectors come out as their
/// JSON text, and nothing at all is an error.
pub fn evaluated_from_json(value: &serde_json::Value) -> Result<Evaluated, String> {
    match value {
        serde_json::Value::Null => Err("answered nothing".to_string()),
        serde_json::Value::Bool(b) => Ok(Evaluated::Bool(*b)),
        serde_json::Value::Number(n) => Ok(Evaluated::Number(n.as_f64().unwrap_or(0.0))),
        serde_json::Value::String(s) => Ok(Evaluated::Text(s.clone())),
        other => Ok(Evaluated::Text(other.to_string())),
    }
}

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

/// Reads the running actor's snapshot in place, or errors with the reason
/// there isn't one - a reporter previewed in the editor has no actor context.
fn with_me<R>(f: impl FnOnce(&sense::ActorSense) -> R) -> Result<R, String> {
    sense::with_me(f)
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
        eval: |_| with_me(|me| Evaluated::Bool(me.tweening)),
    },
    ExtOperator {
        kind: "CurrentClip",
        op: "CurrentClip",
        arity: 0,
        default_args: Vec::new,
        // The clip the animation player is holding, or empty for none. The
        // state name in a clip-per-state project, which is what a state
        // machine transition switches on.
        eval: |_| with_me(|me| Evaluated::Text(me.anim_clip.clone())),
    },
    ExtOperator {
        kind: "CurrentFrame",
        op: "CurrentFrame",
        arity: 0,
        default_args: Vec::new,
        // The 1-based frame showing right now. Zero with no clip.
        eval: |_| with_me(|me| Evaluated::Number(me.anim_frame as f64)),
    },
    ExtOperator {
        kind: "AnimationPlaying",
        op: "AnimationPlaying",
        arity: 0,
        default_args: Vec::new,
        // Whether the player's clip is still advancing. A `Once` clip at
        // its end reads as false, which is when `when animation ends` fires.
        eval: |_| with_me(|me| Evaluated::Bool(me.anim_playing)),
    },
    ExtOperator {
        kind: "ParticleCount",
        op: "ParticleCount",
        arity: 0,
        default_args: Vec::new,
        // My emitter's live particles, as of the last frame drawn.
        eval: |_| with_me(|me| Evaluated::Number(me.particles.alive as f64)),
    },
    ExtOperator {
        kind: "ParticleEventCount",
        op: "ParticleEventCount",
        arity: 1,
        default_args: || vec![text("Die")],
        // How many of my particles spawned, died or collided this frame:
        // what a `when my particles` hat fired for.
        eval: |args| {
            let event = crate::vfx::ParticleEvent::parse(&args[0].as_text())
                .ok_or_else(|| format!("particles can't \"{}\"", args[0].as_text()))?;
            with_me(|me| Evaluated::Number(me.particles.count(event) as f64))
        },
    },
    ExtOperator {
        kind: "ParticleEventPosition",
        op: "ParticleEventPosition",
        arity: 2,
        default_args: || vec![text("Collide"), text("X")],
        // Where the last of my particles spawned, died or collided. Before
        // any has, where I stand: that is where they come from.
        eval: |args| {
            let event = crate::vfx::ParticleEvent::parse(&args[0].as_text())
                .ok_or_else(|| format!("particles can't \"{}\"", args[0].as_text()))?;
            let axis = axis_of(args.get(1));
            with_me(|me| {
                let at = me.particles.at(event).unwrap_or(me.position);
                Evaluated::Number(at[axis.index()] as f64)
            })
        },
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
            let axis = axis_of(args.first());
            with_me(|me| Evaluated::Number(me.position[axis.index()] as f64))
        },
    },
    ExtOperator {
        kind: "MyRotation",
        op: "MyRotation",
        arity: 1,
        default_args: || vec![text("Z")],
        eval: |args| {
            let axis = axis_of(args.first());
            with_me(|me| Evaluated::Number(me.rotation[axis.index()] as f64))
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
            let axis = axis_of(args.first());
            with_me(|me| Evaluated::Number(me.local_position[axis.index()] as f64))
        },
    },
    ExtOperator {
        kind: "CameraPosition",
        op: "CameraPosition",
        arity: 1,
        default_args: || vec![text("X")],
        // Where the world camera stands this frame. Needs no actor, so it
        // answers in the editor as zeros, and in a game straight from the
        // published snapshot - the same numbers the view renders from.
        eval: |args| {
            let axis = axis_of(args.first());
            Ok(Evaluated::Number(
                sense::read(|sensors| sensors.camera.position[axis.index()]) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "CameraDirection",
        op: "CameraDirection",
        arity: 1,
        default_args: || vec![text("X")],
        // Where the world camera looks this frame, unit length. What the
        // middle of the screen points at, read live - an aim block built
        // from these three numbers cannot drift from the view the way a
        // hand-tracked yaw can.
        eval: |args| {
            let axis = axis_of(args.first());
            Ok(Evaluated::Number(
                sense::read(|sensors| sensors.camera.forward[axis.index()]) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "Touching",
        op: "Touching",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| {
            let target = args[0].as_text();
            with_me(|me| {
                // An empty target asks "touching anything at all?".
                if target.trim().is_empty() {
                    return Evaluated::Bool(!me.touching.is_empty());
                }
                Evaluated::Bool(sense::read(|sensors| {
                    me.touching.iter().any(|id| {
                        id == &target
                            || sensors
                                .actors
                                .get(id)
                                .is_some_and(|other| other.name.eq_ignore_ascii_case(&target))
                    })
                }))
            })
        },
    },
    ExtOperator {
        kind: "DistanceTo",
        op: "DistanceTo",
        arity: 1,
        default_args: || vec![text("")],
        eval: |args| {
            let target = args[0].as_text();
            with_me(|me| {
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
            })?
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
            with_me(|me| {
                me.components
                    .get(component.trim())
                    .and_then(|fields| fields.get(field.trim()))
                    .cloned()
            })?
            .ok_or_else(|| format!("I have no \"{component}\" component with a \"{field}\""))
        },
    },
    ExtOperator {
        kind: "IsClone",
        op: "IsClone",
        arity: 0,
        default_args: Vec::new,
        eval: |_| with_me(|me| Evaluated::Bool(me.is_clone)),
    },
    ExtOperator {
        kind: "MyParent",
        op: "MyParent",
        arity: 0,
        default_args: Vec::new,
        // The id rather than the name: clones share a name, and this is what
        // `set my parent to` and `delete` want handed back to them.
        eval: |_| with_me(|me| Evaluated::Text(me.parent.clone())),
    },
    ExtOperator {
        kind: "NewActor",
        op: "NewActor",
        arity: 0,
        default_args: Vec::new,
        eval: |_| with_me(|me| Evaluated::Text(me.last_created.clone())),
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
        kind: "Velocity",
        op: "Velocity",
        arity: 2,
        default_args: || vec![text(""), text("X")],
        // Linear velocity in world units a second (pixels in 2D, metres in
        // 3D). Empty names the running actor itself; zero with no body.
        eval: |args| {
            let target = args[0].as_text();
            let axis = axis_of(args.get(1));
            if target.trim().is_empty() {
                return with_me(|me| Evaluated::Number(me.velocity[axis.index()] as f64));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Number(actor.velocity[axis.index()] as f64))
            })
        },
    },
    ExtOperator {
        kind: "AngularVelocity",
        op: "AngularVelocity",
        arity: 2,
        default_args: || vec![text(""), text("Z")],
        // Spin in radians a second about an axis. A 2D body turns about z
        // only. Empty names the running actor itself; zero with no body.
        eval: |args| {
            let target = args[0].as_text();
            let axis = axis_of(args.get(1));
            if target.trim().is_empty() {
                return with_me(|me| Evaluated::Number(me.angular_velocity[axis.index()] as f64));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Number(
                    actor.angular_velocity[axis.index()] as f64,
                ))
            })
        },
    },
    ExtOperator {
        kind: "Mass",
        op: "Mass",
        arity: 1,
        default_args: || vec![text("")],
        // A body's mass in kilograms, from its shapes. Empty names the
        // running actor itself; zero with no body.
        eval: |args| {
            let target = args[0].as_text();
            if target.trim().is_empty() {
                return with_me(|me| Evaluated::Number(me.mass as f64));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Number(actor.mass as f64))
            })
        },
    },
    ExtOperator {
        kind: "IsGrounded",
        op: "IsGrounded",
        arity: 1,
        default_args: || vec![text("")],
        // Whether a solid contact holds the actor up. Needs no controller
        // move, unlike the controller's own `grounded`. Empty names this
        // actor.
        eval: |args| {
            let target = args[0].as_text();
            if target.trim().is_empty() {
                return with_me(|me| Evaluated::Bool(me.grounded));
            }
            sense::read(|sensors| {
                let actor = sensors
                    .find(&target)
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?;
                Ok(Evaluated::Bool(actor.grounded))
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
        kind: "QueryNumber",
        op: "QueryNumber",
        arity: 2,
        default_args: || vec![number(1.0), text("count")],
        // A number from this actor's last physics query (the `cast`, `overlap`
        // and `find closest` blocks): the `hit`th result's point, normal,
        // distance and so on, or how many there were. A miss reads as zero.
        eval: |args| query_field(args, false),
    },
    ExtOperator {
        kind: "QueryText",
        op: "QueryText",
        arity: 2,
        default_args: || vec![number(1.0), text("actor")],
        // Words from this actor's last physics query: the `hit`th result's
        // actor, body or collider, or the error that stopped the query.
        eval: |args| query_field(args, true),
    },
    ExtOperator {
        kind: "ControllerNumber",
        op: "ControllerNumber",
        arity: 2,
        default_args: || vec![number(1.0), text("grounded")],
        // A number from this actor's last character controller move: whether
        // it stands on something, the collision flags, how far it really
        // moved, or the `hit`th obstacle's point and normal. Zero otherwise.
        eval: |args| controller_field(args, false),
    },
    ExtOperator {
        kind: "ControllerText",
        op: "ControllerText",
        arity: 2,
        default_args: || vec![number(1.0), text("actor")],
        // Words from this actor's last controller move: the `hit`th
        // obstacle's actor, body or collider, or the error that stopped it.
        eval: |args| controller_field(args, true),
    },
    ExtOperator {
        kind: "JointNumber",
        op: "JointNumber",
        arity: 2,
        default_args: || vec![text("angle"), text("")],
        // One reading of the named constraint on this actor: angle or position,
        // speed, force, torque, broken?, enabled?. Zero for one that isn't there.
        eval: |args| {
            let field = args[0].as_text();
            let name = args[1].as_text();
            Ok(Evaluated::Number(
                sense::current_actor().map_or(0.0, |actor| {
                    crate::physics::joints::read_number(&actor, &name, &field)
                }),
            ))
        },
    },
    ExtOperator {
        kind: "MotorNumber",
        op: "MotorNumber",
        arity: 1,
        default_args: || vec![text("grounded")],
        // One reading of this actor's character motor: grounded, speed,
        // vertical speed, jumps left and so on. Zero without a motor.
        eval: |args| motor_field(args, false),
    },
    ExtOperator {
        kind: "MotorText",
        op: "MotorText",
        arity: 1,
        default_args: || vec![text("state")],
        // Words from this actor's character motor: its state, the support
        // it stands on, its owner or a warning.
        eval: |args| motor_field(args, true),
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
        kind: PLUGIN_READ,
        op: PLUGIN_READ,
        arity: 2,
        default_args: || vec![text(""), text("")],
        // Asks the plugin's module while a game runs; there is nothing to
        // ask in the editor, so a preview says so rather than guessing.
        eval: |args| {
            let (plugin, block) = (args[0].as_text(), args[1].as_text());
            sense::plugin_read(&plugin, &block, &args[2..])
        },
    },
    ExtOperator {
        kind: "FrameTime",
        op: "FrameTime",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.performance.frame_ms))),
    },
    ExtOperator {
        kind: "DrawCalls",
        op: "DrawCalls",
        arity: 0,
        default_args: Vec::new,
        eval: |_| {
            Ok(Evaluated::Number(
                sense::read(|s| s.performance.draw_calls) as f64
            ))
        },
    },
    ExtOperator {
        kind: "CurrentQuality",
        op: "CurrentQuality",
        arity: 0,
        default_args: Vec::new,
        eval: |_| {
            Ok(Evaluated::Text(format!(
                "{:?}",
                sense::read(|s| s.performance.quality)
            )))
        },
    },
    ExtOperator {
        kind: "DlssAvailable",
        op: "DlssAvailable",
        arity: 0,
        default_args: Vec::new,
        eval: |_| {
            Ok(Evaluated::Bool(sense::read(|s| {
                s.performance.dlss_available
            })))
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
        kind: "IsRayTracing",
        op: "IsRayTracing",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.atmosphere.ray_tracing))),
    },
    ExtOperator {
        kind: "RayTracingAvailable",
        op: "RayTracingAvailable",
        arity: 0,
        default_args: Vec::new,
        eval: |_| {
            Ok(Evaluated::Bool(sense::read(|s| {
                s.atmosphere.ray_tracing_available
            })))
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
        kind: "LightLevel",
        op: "LightLevel",
        arity: 2,
        default_args: || vec![number(0.0), number(0.0)],
        // The 2D light at a point as of the last frame: ambient plus every
        // light, shadows included, 1 being the picture as drawn. A world
        // without 2D lighting reads 1 everywhere.
        eval: |args| {
            let (x, y) = (num(args.first()) as f32, num(args.get(1)) as f32);
            Ok(Evaluated::Number(
                sense::read(|sensors| sensors.light2d.level_at([x, y])) as f64,
            ))
        },
    },
    ExtOperator {
        kind: "CameraZoom",
        op: "CameraZoom",
        arity: 0,
        default_args: Vec::new,
        // The 2D camera's zoom in pixels per world unit as of the last frame
        // (0 before a game has drawn one).
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.camera2d.zoom) as f64)),
    },
    ExtOperator {
        kind: "CameraAtBounds",
        op: "CameraAtBounds",
        arity: 0,
        default_args: Vec::new,
        // Whether the 2D camera's bounds are holding the view back.
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.camera2d.at_bounds))),
    },
    ExtOperator {
        kind: "IsScreenShaking",
        op: "IsScreenShaking",
        arity: 0,
        default_args: Vec::new,
        // Whether camera trauma is still shaking the 2D view.
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.camera2d.shaking))),
    },
    ExtOperator {
        kind: "ScreenCover",
        op: "ScreenCover",
        arity: 0,
        default_args: Vec::new,
        // How much of the screen a cover hides, 0 clear to 1 opaque.
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.camera2d.cover) as f64)),
    },
    ExtOperator {
        kind: "IsNight",
        op: "IsNight",
        arity: 0,
        default_args: Vec::new,
        // Whether the time-of-day clock is outside the project's day hours.
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.light2d.night))),
    },
    ExtOperator {
        kind: "WaterHeight",
        op: "WaterHeight",
        arity: 2,
        default_args: || vec![number(0.0), number(0.0)],
        // The surface height over x and z (z means nothing in 2D) as of this
        // fixed tick, the highest where bodies overlap. Dry land is reported
        // rather than read as sea level.
        eval: |args| {
            let (x, z) = (num(args.first()) as f32, num(args.get(1)) as f32);
            sense::read(|sensors| sensors.water.height_at(x, z))
                .map(|height| Evaluated::Number(height as f64))
                .ok_or_else(|| format!("there's no water at {x}, {z}"))
        },
    },
    ExtOperator {
        kind: "Underwater",
        op: "Underwater",
        arity: 1,
        default_args: || vec![text("")],
        // Whether an actor stands below a water surface and above its
        // bottom. Empty names the running actor itself.
        eval: |args| {
            let target = args[0].as_text();
            let position = if target.trim().is_empty() {
                with_me(|me| me.position)?
            } else {
                sense::read(|sensors| sensors.find(&target).map(|actor| actor.position))
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?
            };
            Ok(Evaluated::Bool(sense::read(|sensors| {
                sensors.water.underwater(position)
            })))
        },
    },
    ExtOperator {
        kind: "TileAt",
        op: "TileAt",
        arity: 4,
        default_args: || vec![number(0.0), number(0.0), number(0.0), text("")],
        // The sheet index at a world point (x, y, z; z only matters in 3D)
        // in the live tilemaps, -1 for an empty cell or no map there. An
        // empty map name reads any map. Three arguments are (x, y, map).
        eval: |args| {
            let (z, map) = if args.len() >= 4 {
                (num(args.get(2)) as f32, args.get(3))
            } else {
                (0.0, args.get(2))
            };
            let point = [num(args.first()) as f32, num(args.get(1)) as f32, z];
            let map = map.map(|arg| arg.as_text()).unwrap_or_default();
            sense::read(|sensors| sensors.level.tile_at(point, &map))
                .map(|tile| Evaluated::Number(tile as f64))
        },
    },
    ExtOperator {
        kind: "RoomContaining",
        op: "RoomContaining",
        arity: 1,
        default_args: || vec![text("")],
        // The name of the smallest room an actor stands in, or empty for
        // none. Empty names the running actor.
        eval: |args| {
            let target = args[0].as_text();
            let position = if target.trim().is_empty() {
                with_me(|me| me.position)?
            } else {
                sense::read(|sensors| sensors.find(&target).map(|actor| actor.position))
                    .ok_or_else(|| format!("there's no actor named \"{target}\""))?
            };
            Ok(Evaluated::Text(sense::read(|sensors| {
                sensors
                    .level
                    .room_at(position)
                    .map(|room| room.name.clone())
                    .unwrap_or_default()
            })))
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
                return with_me(|me| Evaluated::Bool(me.trigger));
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
                return with_me(|me| Evaluated::Bool(me.casts_shadows));
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
                return with_me(|me| Evaluated::Number(me.layer as f64));
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
            Ok(Evaluated::Text(
                nearest_ray(from, to)?
                    .map(|hit| actor_name(&hit.actor))
                    .unwrap_or_default(),
            ))
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
            Ok(Evaluated::Number(
                nearest_ray(from, to)?
                    .map(|hit| hit.distance as f64)
                    .unwrap_or(-1.0),
            ))
        },
    },
    ExtOperator {
        kind: "CurrentScene",
        op: "CurrentScene",
        arity: 0,
        default_args: Vec::new,
        // The scene running right now, by name. Sampled on the fixed tick.
        eval: |_| Ok(Evaluated::Text(sense::read(|s| s.current_scene.clone()))),
    },
    ExtOperator {
        kind: "TimeOfDay",
        op: "TimeOfDay",
        arity: 0,
        default_args: Vec::new,
        // Hours, 0-24. Sampled with the air on the fixed tick, so every
        // scheduler agrees.
        eval: |_| {
            Ok(Evaluated::Number(
                sense::read(|s| s.atmosphere.time_of_day) as f64
            ))
        },
    },
    ExtOperator {
        kind: "SunElevation",
        op: "SunElevation",
        arity: 0,
        default_args: Vec::new,
        // Degrees above the horizon, from the tick's sun direction.
        eval: |_| {
            Ok(Evaluated::Number(sense::read(|s| {
                s.atmosphere.field("sun elevation").unwrap_or(0.0)
            })))
        },
    },
    ExtOperator {
        kind: "CurrentWeather",
        op: "CurrentWeather",
        arity: 0,
        default_args: Vec::new,
        // The weather preset the air is in, by name. Empty before the first
        // blend. Sampled on the fixed tick like `current scene`.
        eval: |_| {
            Ok(Evaluated::Text(sense::read(|s| {
                s.atmosphere.weather.clone()
            })))
        },
    },
    ExtOperator {
        kind: "IsCutscenePlaying",
        op: "IsCutscenePlaying",
        arity: 0,
        default_args: Vec::new,
        // Whether a cutscene reel is playing right now. Sampled with the
        // sensors each frame from the wall-clock player, so VM and compiled
        // logic agree.
        eval: |_| {
            Ok(Evaluated::Bool(sense::read(|s| {
                !s.cutscene_name.is_empty()
            })))
        },
    },
    ExtOperator {
        kind: "CutsceneTime",
        op: "CutsceneTime",
        arity: 0,
        default_args: Vec::new,
        // Seconds into the playing cutscene, or 0 with none playing.
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.cutscene_time) as f64)),
    },
    ExtOperator {
        kind: "SceneNames",
        op: "SceneNames",
        arity: 0,
        default_args: Vec::new,
        // Every scene's name as a JSON list, so `load json into list` takes
        // it. Sampled on the fixed tick like `active volumes`.
        eval: |_| {
            let names = sense::read(|s| s.scene_names.clone());
            Ok(Evaluated::Text(
                serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string()),
            ))
        },
    },
    ExtOperator {
        kind: "SaveSlot",
        op: "SaveSlot",
        arity: 0,
        default_args: Vec::new,
        // The save slot this run writes to, by name. Sampled on the fixed
        // tick like `current scene`.
        eval: |_| {
            Ok(Evaluated::Text(sense::read(|s| {
                if s.current_save_slot.is_empty() {
                    crate::save::DEFAULT_SLOT.to_string()
                } else {
                    s.current_save_slot.clone()
                }
            })))
        },
    },
    ExtOperator {
        kind: "SaveSlots",
        op: "SaveSlots",
        arity: 0,
        default_args: Vec::new,
        // Every slot with a file on disk as a JSON list, so `load json
        // into list` takes it. Sampled on the fixed tick.
        eval: |_| {
            let slots = sense::read(|s| s.save_slots.clone());
            Ok(Evaluated::Text(
                serde_json::to_string(&slots).unwrap_or_else(|_| "[]".to_string()),
            ))
        },
    },
    ExtOperator {
        kind: "Language",
        op: "Language",
        arity: 0,
        default_args: Vec::new,
        // The language this run speaks, lowercased. Moved by `set
        // language to`; the editor answers the default without a run.
        eval: |_| {
            Ok(Evaluated::Text(sense::read(|s| {
                if s.language.is_empty() {
                    crate::locale::DEFAULT_LANGUAGE.to_string()
                } else {
                    s.language.clone()
                }
            })))
        },
    },
    ExtOperator {
        kind: "LocalizedText",
        op: "LocalizedText",
        arity: 1,
        default_args: || vec![text("message")],
        // The text for a key in the run's language, falling back to the
        // default language and then to the key itself.
        eval: |args| Ok(Evaluated::Text(sense::locale_text(&args[0].as_text()))),
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
            Ok(Evaluated::Text(
                nearest_within(center, radius)?
                    .map(|hit| actor_name(&hit.actor))
                    .unwrap_or_default(),
            ))
        },
    },
];

/// Reads a field of the running actor's last physics query.
fn query_field(args: &[Evaluated], text: bool) -> Result<Evaluated, String> {
    let name = args[1].as_text();
    let field = query::HitField::parse(&name)
        .filter(|field| field.is_text() == text)
        .ok_or_else(|| {
            let kind = if text { "words" } else { "numbers" };
            format!("a query result has no {kind} called \"{name}\"")
        })?;
    let Some(actor) = sense::current_actor() else {
        // A reporter previewed in the editor has no query to read.
        return Ok(if text {
            Evaluated::Text(String::new())
        } else {
            Evaluated::Number(0.0)
        });
    };
    let index = num(args.first()).max(0.0) as usize;
    Ok(match query::read_field(&actor, index, field, actor_name) {
        query::HitValue::Number(n) => Evaluated::Number(n),
        query::HitValue::Text(t) => Evaluated::Text(t),
    })
}

/// Reads a field of the running actor's character motor.
fn motor_field(args: &[Evaluated], text: bool) -> Result<Evaluated, String> {
    let name = args[0].as_text();
    let actor = sense::current_actor();
    Ok(if text {
        Evaluated::Text(match actor {
            Some(actor) => {
                let words = crate::physics::motor::read_text(&actor, &name);
                if name.trim().eq_ignore_ascii_case("support") && !words.is_empty() {
                    actor_name(&words)
                } else {
                    words
                }
            }
            None => String::new(),
        })
    } else {
        Evaluated::Number(actor.map_or(0.0, |actor| {
            crate::physics::motor::read_number(&actor, &name)
        }))
    })
}

/// Reads a field of the running actor's last character controller move.
fn controller_field(args: &[Evaluated], text: bool) -> Result<Evaluated, String> {
    let name = args[1].as_text();
    let Some(actor) = sense::current_actor() else {
        return Ok(if text {
            Evaluated::Text(String::new())
        } else {
            Evaluated::Number(0.0)
        });
    };
    let index = num(args.first()).max(0.0) as usize;
    Ok(if text {
        Evaluated::Text(
            match crate::physics::controller::read_text(&actor, &name, index) {
                id if matches!(name.trim(), "actor") && !id.is_empty() => actor_name(&id),
                other => other,
            },
        )
    } else {
        Evaluated::Number(crate::physics::controller::read_number(
            &actor, &name, index,
        ))
    })
}

/// The name blocks use for an actor id, or the id itself when it has none.
fn actor_name(id: &str) -> String {
    sense::read(|sensors| {
        sensors
            .actors
            .get(id)
            .map(|actor| actor.name.clone())
            .unwrap_or_else(|| id.to_string())
    })
}

/// What a legacy ray reporter asks: the nearest collider on the segment, as
/// the running actor. Outside a run there is no world to ask, which reads as
/// nothing found; a world that fails to answer is an error.
fn nearest_ray(from: [f32; 3], to: [f32; 3]) -> Result<Option<QueryHit>, String> {
    if !query::available() {
        return Ok(None);
    }
    let request = QueryRequest::Ray {
        from,
        to,
        all: false,
    };
    let outcome = query::dispatch(&request, &asker(), 1);
    match outcome.error {
        Some(why) => Err(why),
        None => Ok(outcome.hits.into_iter().next()),
    }
}

/// The nearest collider within `radius` of `center`: the ball a ground check
/// holds underfoot touches exactly what lies within its radius.
fn nearest_within(center: [f32; 3], radius: f32) -> Result<Option<QueryHit>, String> {
    if !query::available() {
        return Ok(None);
    }
    let request = QueryRequest::Closest {
        point: center,
        max_distance: radius.max(0.0),
    };
    let outcome = query::dispatch(&request, &asker(), 1);
    match outcome.error {
        Some(why) => Err(why),
        None => Ok(outcome.hits.into_iter().next()),
    }
}

fn asker() -> QueryFilter {
    match sense::current_actor() {
        Some(actor) => QueryFilter::as_actor(actor),
        None => QueryFilter::default(),
    }
}

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
            camera: crate::sense::CameraSense {
                position: [1.0, 2.0, 3.0],
                forward: [0.0, 0.0, -1.0],
            },
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

        // The camera reporters need no actor: they read the world camera's
        // published pose, so they answer outside a script too.
        let cam_x = Value::op(Op::from_name("CameraPosition"), vec![Value::text("X")]);
        assert_eq!(cam_x.eval(), Ok(Evaluated::Number(1.0)));
        let cam_dz = Value::op(Op::from_name("CameraDirection"), vec![Value::text("Z")]);
        assert_eq!(cam_dz.eval(), Ok(Evaluated::Number(-1.0)));
        sense::with_actor("a1", || {
            assert_eq!(cam_x.eval(), Ok(Evaluated::Number(1.0)));
        });

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
    fn motion_reporters_read_velocity_mass_and_support() {
        register_blockloom_operators();
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "a1".to_string(),
            ActorSense {
                name: "Player".to_string(),
                velocity: [30.0, -9.0, 0.0],
                angular_velocity: [0.0, 0.0, 1.5],
                mass: 2.5,
                grounded: true,
                ..Default::default()
            },
        );
        sensors.actors.insert(
            "b2".to_string(),
            ActorSense {
                name: "Crate".to_string(),
                ..Default::default()
            },
        );
        sense::publish(sensors);

        // Empty names the running actor; a missing body reads zero, and a
        // missing actor is an error rather than a silent zero.
        let my_vx = Value::op(
            Op::from_name("Velocity"),
            vec![Value::text(""), Value::text("X")],
        );
        sense::with_actor("a1", || {
            assert_eq!(my_vx.eval(), Ok(Evaluated::Number(30.0)));
        });
        let others_vy = Value::op(
            Op::from_name("Velocity"),
            vec![Value::text("Player"), Value::text("Y")],
        );
        assert_eq!(others_vy.eval(), Ok(Evaluated::Number(-9.0)));
        let still = Value::op(
            Op::from_name("Velocity"),
            vec![Value::text("Crate"), Value::text("X")],
        );
        assert_eq!(still.eval(), Ok(Evaluated::Number(0.0)));
        let missing = Value::op(
            Op::from_name("Velocity"),
            vec![Value::text("Nobody"), Value::text("X")],
        );
        assert!(missing.eval().is_err());

        let spin = Value::op(
            Op::from_name("AngularVelocity"),
            vec![Value::text(""), Value::text("Z")],
        );
        sense::with_actor("a1", || {
            assert_eq!(spin.eval(), Ok(Evaluated::Number(1.5)));
        });

        let my_mass = Value::op(Op::from_name("Mass"), vec![Value::text("")]);
        sense::with_actor("a1", || {
            assert_eq!(my_mass.eval(), Ok(Evaluated::Number(2.5)));
        });
        let crate_mass = Value::op(Op::from_name("Mass"), vec![Value::text("Crate")]);
        assert_eq!(crate_mass.eval(), Ok(Evaluated::Number(0.0)));

        let grounded = Value::op(Op::from_name("IsGrounded"), vec![Value::text("")]);
        sense::with_actor("a1", || {
            assert_eq!(grounded.eval(), Ok(Evaluated::Bool(true)));
        });
        let crate_grounded = Value::op(Op::from_name("IsGrounded"), vec![Value::text("Crate")]);
        assert_eq!(crate_grounded.eval(), Ok(Evaluated::Bool(false)));
    }

    /// A wall whose near face is at x = 4, found by rays at y within 1 and by a
    /// ball within reach of x = 5.
    struct WallAtFive;

    impl query::QueryService for WallAtFive {
        fn run(
            &self,
            request: &QueryRequest,
            filter: &QueryFilter,
            limit: usize,
        ) -> query::QueryOutcome {
            // The asker is the running actor.
            assert_eq!(filter.as_actor.as_deref(), Some("me"));
            let wall = |distance: f32| QueryHit {
                actor: "wall".into(),
                body: None,
                collider: "wall#body".into(),
                subshape: 0,
                point: [4.0, 0.0, 0.0],
                normal: [-1.0, 0.0, 0.0],
                distance,
                fraction: 0.2,
                started_inside: false,
                trigger: false,
            };
            let hits = match request {
                QueryRequest::Ray { from, .. } if from[1].abs() <= 1.0 => vec![wall(4.0)],
                QueryRequest::Closest {
                    point,
                    max_distance,
                } if (point[0] - 5.0).abs() <= *max_distance + 1.0 => vec![wall(0.0)],
                _ => vec![],
            };
            query::QueryOutcome::finish(hits, limit)
        }
    }

    #[test]
    fn legacy_ray_and_ball_reporters_ask_the_installed_query_service() {
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
        // Nothing answers outside a run: no world, nothing found.
        sense::with_actor("me", || {
            assert_eq!(hit.eval(), Ok(Evaluated::Text(String::new())));
        });
        let world = WallAtFive;
        query::with_service(&world, 1, || {
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
            sense::with_actor("me", || {
                assert_eq!(sky.eval(), Ok(Evaluated::Text(String::new())));
            });

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
            sense::with_actor("me", || {
                assert_eq!(circle.eval(), Ok(Evaluated::Text("Wall".to_string())));
            });
        });
    }

    #[test]
    fn level_reporters_read_live_tiles_and_rooms() {
        use crate::tilemap::{LevelSense, RoomSense, RoomSpec, TilemapSense};
        register_blockloom_operators();
        let mut sensors = Sensors::default();
        sensors.actors.insert(
            "p".to_string(),
            ActorSense {
                name: "Player".to_string(),
                position: [10.0, 10.0, 0.0],
                ..Default::default()
            },
        );
        let mut map = crate::material::Tilemap::default();
        map.set_tile(4, 3, 2);
        sensors.level = LevelSense {
            tilemaps: vec![TilemapSense {
                id: "m".into(),
                name: "Ground".into(),
                center: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                flat: true,
                map: std::sync::Arc::new(map),
            }],
            rooms: vec![RoomSense {
                id: "r".into(),
                name: "Cave".into(),
                bounds: RoomSpec::default().bounds([0.0; 3], [1.0; 3], true),
            }],
            ..Default::default()
        };
        sense::publish(sensors);
        let tile = |x: f64, y: f64, map: &str| {
            Value::op(
                Op::from_name("TileAt"),
                vec![
                    Value::number(x),
                    Value::number(y),
                    Value::number(0.0),
                    Value::text(map),
                ],
            )
            .eval()
        };
        // An older three-slot reporter still reads (x, y, map).
        let old = Value::op(
            Op::from_name("TileAt"),
            vec![
                Value::number(10.0),
                Value::number(10.0),
                Value::text("Ground"),
            ],
        );
        assert_eq!(old.eval(), Ok(Evaluated::Number(2.0)));
        // Cell (4, 3) of an 8x8 map of 32s spans x 0..32, y 0..32.
        assert_eq!(tile(10.0, 10.0, ""), Ok(Evaluated::Number(2.0)));
        assert_eq!(tile(-10.0, -10.0, "Ground"), Ok(Evaluated::Number(-1.0)));
        assert!(tile(0.0, 0.0, "Sky").is_err());
        let room = |who: &str| Value::op(Op::from_name("RoomContaining"), vec![Value::text(who)]);
        assert_eq!(room("Player").eval(), Ok(Evaluated::Text("Cave".into())));
        sense::with_actor("p", || {
            assert_eq!(room("").eval(), Ok(Evaluated::Text("Cave".into())));
        });
    }

    #[test]
    fn water_reporters_read_the_ticks_sample() {
        use crate::water::{WaterBody, WaterKind, WaterSense};
        register_blockloom_operators();
        let mut sensors = Sensors::default();
        for (id, y) in [("fish", -1.0), ("gull", 3.0)] {
            sensors.actors.insert(
                id.to_string(),
                ActorSense {
                    name: id.to_string(),
                    position: [0.0, y, 0.0],
                    ..Default::default()
                },
            );
        }
        sensors.water = WaterSense {
            bodies: vec![WaterBody {
                id: "lake".to_string(),
                kind: WaterKind::Lake,
                center: [0.0, 0.5, 0.0],
                axis: [1.0, 0.0],
                half: [10.0, 10.0],
                depth: 4.0,
                flow: [0.0, 0.0],
                waves: Vec::new(),
                flat: false,
                calm: [0.0; 3],
                ripples: None,
            }],
            time: 0.0,
        };
        sense::publish(sensors);
        let height = |x: f64| {
            Value::op(
                Op::from_name("WaterHeight"),
                vec![Value::number(x), Value::number(0.0)],
            )
            .eval()
        };
        assert_eq!(height(1.0), Ok(Evaluated::Number(0.5)));
        assert!(height(50.0).is_err());
        let under = |who: &str| Value::op(Op::from_name("Underwater"), vec![Value::text(who)]);
        assert_eq!(under("gull").eval(), Ok(Evaluated::Bool(false)));
        sense::with_actor("fish", || {
            assert_eq!(under("").eval(), Ok(Evaluated::Bool(true)));
        });
        assert!(under("whale").eval().is_err());
    }
}
