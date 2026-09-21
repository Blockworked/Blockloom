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
        kind: "MouseDown",
        op: "MouseDown",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Bool(sense::read(|s| s.mouse_down))),
    },
    ExtOperator {
        kind: "Timer",
        op: "Timer",
        arity: 0,
        default_args: Vec::new,
        eval: |_| Ok(Evaluated::Number(sense::read(|s| s.time))),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sense::{ActorSense, Sensors};

    #[test]
    fn sensing_operators_read_the_published_snapshot() {
        register_blockloom_operators();
        let mut sensors = Sensors {
            time: 4.5,
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
