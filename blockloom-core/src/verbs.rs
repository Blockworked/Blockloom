//! Block verbs declared once. A [`VerbSpec`] names a statement or reporter,
//! its typed arguments, its label and where the palette files it, and every
//! surface reads that one row: the VM and compiled logic (one generic
//! `Verb` instruction and the `Verb` reporter operator), scripts (the
//! `ACT_VERB`/`READ_VERB`/`TEXT_VERB` ABI verbs plus generated typed
//! methods), the shell and MCP (the vocabulary) and the palette (a
//! generated table in `Blocks.qml`). A new verb is one row here plus, for a
//! statement, the runtime system that acts on it.

use crate::sense;
use crate::value::{Evaluated, json_of};
use crate::vm::Effect;

/// What an argument holds. Coerced the same way by every scheduler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArgType {
    Number,
    Text,
    Bool,
    /// An actor by id or name; empty, `me` or `myself` is the running one.
    /// Resolved to an id against this frame's snapshot.
    Actor,
    /// One of the listed words, matched without case.
    Choice(&'static [&'static str]),
    /// Whatever the slot evaluated to.
    Any,
}

impl ArgType {
    /// The palette's slot type, as plugin block schemas spell it.
    pub fn schema_name(self) -> &'static str {
        match self {
            ArgType::Number => "number",
            ArgType::Text => "text",
            ArgType::Bool => "bool",
            ArgType::Actor => "actor",
            ArgType::Choice(_) => "choice",
            ArgType::Any => "any",
        }
    }
}

/// One argument of a verb.
#[derive(Debug, Clone, Copy)]
pub struct VerbArg {
    pub name: &'static str,
    pub ty: ArgType,
    /// The fresh block's value, as text (`"1"`, `"myself"`).
    pub default: &'static str,
}

/// What a reporter answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Returns {
    Number,
    Text,
    Bool,
    /// Whatever the read found: a number, text or a switch.
    Any,
}

/// A reporter's answer from its coerced arguments, read off the published
/// snapshot as the asking actor. Plain data in, so the VM, compiled logic
/// and scripts land on the same answer.
pub type ReadFn = fn(me: &str, args: &[Evaluated]) -> Result<Evaluated, String>;

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// A statement: queues `Effect::Verb`, which a runtime system acts on.
    Statement,
    Reporter {
        returns: Returns,
        read: ReadFn,
    },
}

/// One verb, declared once.
#[derive(Debug, Clone, Copy)]
pub struct VerbSpec {
    /// The stable id saved in documents and named by scripts (`set_property`).
    pub name: &'static str,
    /// The block's words with `{arg}` where each argument's slot goes.
    pub label: &'static str,
    /// The palette category id (`actors`, `interface`, ...).
    pub category: &'static str,
    /// The titled section inside it the block joins (made if missing).
    pub section: &'static str,
    /// One line for agents and the palette's details.
    pub purpose: &'static str,
    /// A Lucide icon name for the row.
    pub icon: &'static str,
    pub args: &'static [VerbArg],
    pub shape: Shape,
}

impl VerbSpec {
    pub fn is_reporter(&self) -> bool {
        matches!(self.shape, Shape::Reporter { .. })
    }
}

/// The wire `type` of the one statement every verb shares.
pub const INSTRUCTION: &str = "Verb";
/// The reporter operator every verb reporter shares; args are the verb's
/// name, then its slots.
pub const OPERATOR: &str = "Verb";

const ACTOR: VerbArg = VerbArg {
    name: "actor",
    ty: ArgType::Actor,
    default: "myself",
};

/// Every verb. Names are saved in documents and scripts: never rename one.
pub static VERBS: &[VerbSpec] = &[
    VerbSpec {
        name: "set_property",
        label: "set {property} of {actor} to {value}",
        category: "actors",
        section: "Components",
        purpose: "Writes one property of a component on an actor for the rest of the run, by its registry path (Light.intensity, Look.visual.color).",
        icon: "sliders-horizontal",
        args: &[
            VerbArg {
                name: "property",
                ty: ArgType::Text,
                default: "Light.intensity",
            },
            ACTOR,
            VerbArg {
                name: "value",
                ty: ArgType::Any,
                default: "800",
            },
        ],
        shape: Shape::Statement,
    },
    VerbSpec {
        name: "set_surface",
        label: "set surface {property} to {value}",
        category: "world",
        section: "Sky and weather",
        purpose: "Sets the world's snow cover or surface wetness (0 to 1) for the rest of the run, over the director and volumes.",
        icon: "cloud",
        args: &[
            VerbArg {
                name: "property",
                ty: ArgType::Choice(&["snow", "wetness"]),
                default: "snow",
            },
            VerbArg {
                name: "value",
                ty: ArgType::Number,
                default: "1",
            },
        ],
        shape: Shape::Statement,
    },
    VerbSpec {
        name: "property",
        label: "{property} of {actor}",
        category: "actors",
        section: "Components",
        purpose: "Reads one property of a component on an actor by its registry path, as the run has it now.",
        icon: "sliders-horizontal",
        args: &[
            VerbArg {
                name: "property",
                ty: ArgType::Text,
                default: "Light.intensity",
            },
            ACTOR,
        ],
        shape: Shape::Reporter {
            returns: Returns::Any,
            read: read_property,
        },
    },
    VerbSpec {
        name: "screen_open",
        label: "is screen {screen} open?",
        category: "interface",
        section: "Screens",
        purpose: "Whether a screen the scene or game lists is open, by its file name without the extension or its path.",
        icon: "app-window",
        args: &[VerbArg {
            name: "screen",
            ty: ArgType::Text,
            default: "pause",
        }],
        shape: Shape::Reporter {
            returns: Returns::Bool,
            read: read_screen_open,
        },
    },
];

/// The verb called `name`.
pub fn find(name: &str) -> Option<&'static VerbSpec> {
    VERBS.iter().find(|verb| verb.name == name)
}

/// The actor an `Actor` argument names, as an id: the running actor for an
/// empty name, `me` or `myself`, else an id or name from this frame.
pub fn resolve_actor(me: &str, named: &str) -> Result<String, String> {
    let named = named.trim();
    if named.is_empty() || named.eq_ignore_ascii_case("me") || named.eq_ignore_ascii_case("myself")
    {
        return Ok(me.to_string());
    }
    sense::read(|sensors| sensors.find_id(named).map(str::to_string))
        .ok_or_else(|| format!("there is no actor called \"{named}\""))
}

/// Checks and coerces a verb's arguments: one per declared argument, a
/// missing one taking its default. Actors come back as ids.
pub fn coerce(spec: &VerbSpec, me: &str, args: &[Evaluated]) -> Result<Vec<Evaluated>, String> {
    if args.len() > spec.args.len() {
        return Err(format!(
            "{} takes {} values, not {}",
            spec.name,
            spec.args.len(),
            args.len()
        ));
    }
    let mut out = Vec::with_capacity(spec.args.len());
    for (i, arg) in spec.args.iter().enumerate() {
        let given = args
            .get(i)
            .cloned()
            .unwrap_or_else(|| Evaluated::Text(arg.default.to_string()));
        let value = match arg.ty {
            ArgType::Number => Evaluated::Number(
                given
                    .as_number()
                    .map_err(|e| format!("{} of {}: {e}", arg.name, spec.name))?,
            ),
            ArgType::Text => Evaluated::Text(given.as_text()),
            ArgType::Bool => Evaluated::Bool(given.as_bool()),
            ArgType::Actor => Evaluated::Text(resolve_actor(me, &given.as_text())?),
            ArgType::Choice(options) => {
                let text = given.as_text();
                let found = options
                    .iter()
                    .find(|option| option.eq_ignore_ascii_case(text.trim()))
                    .ok_or_else(|| {
                        format!(
                            "{} of {} is one of {}, not \"{text}\"",
                            arg.name,
                            spec.name,
                            options.join(", ")
                        )
                    })?;
                Evaluated::Text(found.to_string())
            }
            ArgType::Any => given,
        };
        out.push(value);
    }
    Ok(out)
}

/// What a statement verb queues when `me` runs it: `Effect::Verb` with the
/// coerced arguments, or the reason it can't run as an error.
pub fn effect(me: &str, name: &str, args: &[Evaluated]) -> Effect {
    let checked = match find(name) {
        Some(spec) if !spec.is_reporter() => coerce(spec, me, args),
        Some(_) => Err(format!(
            "{name} is a reporter, not a block that does something"
        )),
        None => Err(format!("there is no block called \"{name}\"")),
    };
    match checked {
        Ok(args) => Effect::Verb {
            actor: me.to_string(),
            verb: name.to_string(),
            args: args.iter().map(json_of).collect(),
        },
        Err(message) => Effect::Error {
            actor: me.to_string(),
            message,
        },
    }
}

/// A reporter verb's answer for the running actor.
pub fn read(name: &str, args: &[Evaluated]) -> Result<Evaluated, String> {
    let spec = find(name).ok_or_else(|| format!("there is no reporter called \"{name}\""))?;
    let Shape::Reporter { read, .. } = spec.shape else {
        return Err(format!("{name} is a block, not a reporter"));
    };
    let me = sense::current_actor().unwrap_or_default();
    let args = coerce(spec, &me, args)?;
    read(&me, &args)
}

/// A statement's arguments as `Effect::Verb` carries them, read back.
pub fn args_of(args: &[serde_json::Value]) -> Vec<Evaluated> {
    args.iter()
        .map(|arg| crate::value::evaluated_from_json(arg).unwrap_or(Evaluated::Text(String::new())))
        .collect()
}

/// Splits `Light.intensity` into the component and the path inside it.
pub fn split_property(property: &str) -> Result<(&str, &str), String> {
    let property = property.trim();
    let (component, path) = property.split_once('.').unwrap_or((property, ""));
    if component.is_empty() {
        return Err("name a component and a property, like Light.intensity".into());
    }
    Ok((component, path))
}

fn read_property(_me: &str, args: &[Evaluated]) -> Result<Evaluated, String> {
    let property = args[0].as_text();
    let actor = args[1].as_text();
    let (component, path) = split_property(&property)?;
    sense::read(|sensors| {
        let spec = sensors
            .actors
            .get(&actor)
            .and_then(|a| a.spec.as_ref())
            .ok_or_else(|| format!("{actor}'s components aren't known yet"))?;
        let found = spec
            .json(component)
            .ok_or_else(|| format!("{actor} has no {component}"))?;
        let path = crate::registry::resolve_path_in(found, path);
        match crate::registry::value_at(found, &path)? {
            serde_json::Value::Null => Err(format!("{property} is empty")),
            other => crate::value::evaluated_from_json(other),
        }
    })
}

fn read_screen_open(_me: &str, args: &[Evaluated]) -> Result<Evaluated, String> {
    let screen = args[0].as_text();
    let screen = screen.trim();
    sense::read(|sensors| {
        sensors
            .screens
            .iter()
            .find(|(path, _)| {
                path == screen || crate::ui_layers::screen_name(path).eq_ignore_ascii_case(screen)
            })
            .map(|(_, open)| Evaluated::Bool(*open))
    })
    .ok_or_else(|| format!("no screen called \"{screen}\" is listed"))
}

// ─── Generated surfaces ─────────────────────────────────────────────────────

/// The verbs as the shell, MCP and the palette read them: plugin block
/// schemas' shape, so the palette draws both with the same code.
pub fn to_json() -> serde_json::Value {
    serde_json::Value::Array(VERBS.iter().map(verb_json).collect())
}

fn verb_json(verb: &VerbSpec) -> serde_json::Value {
    let slots: Vec<_> = verb
        .args
        .iter()
        .map(|arg| {
            let mut slot = serde_json::json!({
                "name": arg.name,
                "type": arg.ty.schema_name(),
                "default": default_json(arg),
            });
            if let ArgType::Choice(options) = arg.ty {
                slot["options"] = serde_json::json!(options);
            }
            slot
        })
        .collect();
    let mut out = serde_json::json!({
        "type_id": verb.name,
        "kind": if verb.is_reporter() { "reporter" } else { "statement" },
        "label": verb.label,
        "category": verb.category,
        "section": verb.section,
        "purpose": verb.purpose,
        "icon": verb.icon,
        "slots": slots,
    });
    if let Shape::Reporter { returns, .. } = verb.shape {
        out["returns"] = serde_json::json!({ "type": match returns {
            Returns::Number => "number",
            Returns::Text => "text",
            Returns::Bool => "bool",
            Returns::Any => "any",
        }});
    }
    out
}

fn default_json(arg: &VerbArg) -> serde_json::Value {
    match arg.ty {
        ArgType::Number => arg
            .default
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        ArgType::Bool => serde_json::Value::Bool(arg.default == "true"),
        _ => serde_json::Value::String(arg.default.to_string()),
    }
}

/// The table `Blocks.qml` carries between its generated markers.
pub fn render_qml(indent: &str) -> String {
    let mut out = String::from("[\n");
    for verb in VERBS {
        out.push_str(indent);
        out.push_str("    ");
        out.push_str(&verb_json(verb).to_string());
        out.push_str(",\n");
    }
    out.push_str(indent);
    out.push(']');
    out
}

/// A typed method per verb on a script's `Actor`, generated into
/// `script/verbs.rs` (`BLESS=1 cargo test -p blockloom-core --test verbs`).
pub fn render_prelude() -> String {
    let mut out = String::from(
        "// Generated from blockloom-core/src/verbs.rs; BLESS=1 cargo test -p blockloom-core --test verbs rewrites it.\n\
         // One typed method per block verb, over the generic `verb` calls.\n\n\
         impl Actor {\n",
    );
    for (i, verb) in VERBS.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let params: Vec<String> = verb
            .args
            .iter()
            .map(|arg| {
                let ty = match arg.ty {
                    ArgType::Number => "f64",
                    ArgType::Bool => "bool",
                    ArgType::Any => "impl Into<PluginArg>",
                    _ => "&str",
                };
                format!("{}: {ty}", arg.name)
            })
            .collect();
        let values: Vec<String> = verb
            .args
            .iter()
            .map(|arg| match arg.ty {
                ArgType::Any => format!("{}.into()", arg.name),
                _ => format!("PluginArg::from({})", arg.name),
            })
            .collect();
        let call = format!("&[{}]", values.join(", "));
        out.push_str(&format!("    /// {}\n", verb.purpose));
        let params = params.join(", ");
        let comma = if params.is_empty() { "" } else { ", " };
        match verb.shape {
            Shape::Statement => out.push_str(&format!(
                "    pub fn {}(&self{comma}{params}) {{\n        self.verb(\"{}\", {call});\n    }}\n",
                verb.name, verb.name
            )),
            Shape::Reporter { returns, .. } => {
                let (ty, body) = match returns {
                    Returns::Number => ("Option<f64>", format!("self.verb_number(\"{}\", {call})", verb.name)),
                    Returns::Bool => (
                        "Option<bool>",
                        format!("self.verb_number(\"{}\", {call}).map(|n| n != 0.0)", verb.name),
                    ),
                    Returns::Text | Returns::Any => {
                        ("Option<String>", format!("self.verb_text(\"{}\", {call})", verb.name))
                    }
                };
                out.push_str(&format!(
                    "    pub fn {}(&self{comma}{params}) -> {ty} {{\n        {body}\n    }}\n",
                    verb.name
                ));
            }
        }
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_labels_name_every_argument() {
        let mut seen = std::collections::HashSet::new();
        for verb in VERBS {
            assert!(seen.insert(verb.name), "{} is declared twice", verb.name);
            assert!(
                verb.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{} must be a snake_case name scripts can call",
                verb.name
            );
            for arg in verb.args {
                assert!(
                    verb.label.contains(&format!("{{{}}}", arg.name)),
                    "{}'s label doesn't place {}",
                    verb.name,
                    arg.name
                );
            }
        }
    }

    #[test]
    fn missing_arguments_take_their_defaults_and_choices_are_checked() {
        static PICK: VerbSpec = VerbSpec {
            name: "pick",
            label: "pick {mode} {n}",
            category: "control",
            section: "",
            purpose: "test",
            icon: "",
            args: &[
                VerbArg {
                    name: "mode",
                    ty: ArgType::Choice(&["Fast", "Slow"]),
                    default: "Fast",
                },
                VerbArg {
                    name: "n",
                    ty: ArgType::Number,
                    default: "3",
                },
            ],
            shape: Shape::Statement,
        };
        let got = coerce(&PICK, "me", &[Evaluated::Text("slow".into())]).unwrap();
        assert_eq!(
            got,
            vec![Evaluated::Text("Slow".into()), Evaluated::Number(3.0)]
        );
        assert!(coerce(&PICK, "me", &[Evaluated::Text("medium".into())]).is_err());
        assert!(
            coerce(
                &PICK,
                "me",
                &[Evaluated::Text("Fast".into()), Evaluated::Text("x".into())]
            )
            .is_err()
        );
    }

    #[test]
    fn an_actor_argument_names_the_runner_by_default() {
        assert_eq!(resolve_actor("a1", "myself").unwrap(), "a1");
        assert_eq!(resolve_actor("a1", "").unwrap(), "a1");
        assert!(resolve_actor("a1", "Nobody").is_err());
    }

    #[test]
    fn a_reporter_can_not_be_queued_and_an_unknown_verb_errs() {
        assert!(matches!(effect("a", "property", &[]), Effect::Error { .. }));
        assert!(matches!(effect("a", "nope", &[]), Effect::Error { .. }));
        assert!(matches!(
            effect("a", "set_property", &[]),
            Effect::Verb { ref verb, ref args, .. } if verb == "set_property" && args.len() == 3
        ));
    }
}

#[cfg(test)]
mod cost {
    use super::*;

    /// What a generic `property of` read costs against a typed reporter
    /// (`my x position`). `cargo test -p blockloom-core --release --lib
    /// verbs::cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn property_read_cost() {
        crate::init();
        let mut components = crate::components::Components::default();
        components.insert(
            crate::registry::default_component("Light", crate::scene::Mode::ThreeD).unwrap(),
        );
        let mut actors = std::collections::HashMap::new();
        actors.insert(
            "a1".to_string(),
            sense::ActorSense {
                name: "Lamp".into(),
                spec: Some(std::sync::Arc::new(sense::ActorSpec::new(components))),
                ..Default::default()
            },
        );
        sense::publish(sense::Sensors {
            actors,
            ..Default::default()
        });
        let args = [
            Evaluated::Text("Light.intensity".into()),
            Evaluated::Text("myself".into()),
        ];
        let n = 20_000;
        let generic = sense::with_actor("a1", || {
            let start = std::time::Instant::now();
            for _ in 0..n {
                std::hint::black_box(read("property", &args).unwrap());
            }
            start.elapsed()
        });
        let typed = sense::with_actor("a1", || {
            let start = std::time::Instant::now();
            for _ in 0..n {
                std::hint::black_box(sense::with_me(|me| me.position[0]).unwrap());
            }
            start.elapsed()
        });
        println!(
            "property of: {:.0} ns, my x position: {:.0} ns",
            generic.as_nanos() as f64 / n as f64,
            typed.as_nanos() as f64 / n as f64
        );
    }
}
