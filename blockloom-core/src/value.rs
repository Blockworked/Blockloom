//! Blockloom's view of the value system. The expression tree and its
//! evaluation live in `blockstitch-core`; the reporter blocks Blockloom adds
//! - the sensing half of a game engine - are here.
//!
//! An extension operator is a plain `fn` with no context, so these read the
//! frame snapshot in [`crate::sense`] instead of taking the world as an
//! argument.

pub use blockstitch_core::value::*;

use crate::scene::Axis;
use crate::sense;

fn text(value: &str) -> Value {
    Value::text(value)
}

fn axis_of(arg: Option<&Evaluated>) -> Axis {
    match arg.map(Evaluated::as_text).unwrap_or_default().as_str() {
        "Y" | "y" => Axis::Y,
        "Z" | "z" => Axis::Z,
        _ => Axis::X,
    }
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
    }
}
