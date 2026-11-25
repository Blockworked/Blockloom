//! The runtime half of the declared verbs (`blockloom_core::verbs`). The
//! schedulers queue `Effect::Verb` with checked arguments; a system here
//! (or in the module that owns the verb) acts on it by name.

use bevy::prelude::*;
use blockloom_core::components::ActorComponent;
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
use serde_json::Value;

use crate::bridge;
use crate::engine::{Engine, PendingEffects};

/// Every `Effect::Verb` called `name` this step: who ran it and its args.
pub fn calls<'a>(
    effects: &'a PendingEffects,
    name: &'a str,
) -> impl Iterator<Item = (&'a str, &'a [Value])> + 'a {
    effects.0.iter().filter_map(move |effect| match effect {
        Effect::Verb { actor, verb, args } if verb == name => Some((actor.as_str(), &args[..])),
        _ => None,
    })
}

/// Components rebuilt from their spec only when attached: a property write
/// re-attaches them. Everything else reads its spec each frame.
const REATTACH: &[&str] = &["Look", "Body", "Sprite", "Camera", "Render"];

/// `set property`: writes the run's copy of one component of an actor and,
/// for components built only when attached, re-attaches it so the change
/// shows. The document is untouched; a rebuild forgets every write.
pub fn apply_property_writes(mut effects: ResMut<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running
        || !effects
            .0
            .iter()
            .any(|e| matches!(e, Effect::Verb { verb, .. } if verb == "set_property"))
    {
        return;
    }
    let mut reattach = Vec::new();
    for (by, args) in calls(&effects, "set_property") {
        let property = args.first().and_then(Value::as_str).unwrap_or_default();
        let target = args.get(1).and_then(Value::as_str).unwrap_or_default();
        let value = blockloom_core::verbs::args_of(&args[2.min(args.len())..])
            .into_iter()
            .next()
            .unwrap_or(blockloom_core::value::Evaluated::Text(String::new()));
        match write_property(&mut engine, target, property, &value) {
            Ok(component) => {
                if REATTACH.contains(&component.as_str())
                    && engine.has_component(target, &component)
                {
                    reattach.push((target.to_string(), component));
                }
            }
            Err(message) => bridge::send(&RuntimeMessage::Error {
                actor: by.to_string(),
                message,
            }),
        }
    }
    for (actor, component) in reattach {
        effects.0.push(Effect::DetachComponent {
            actor: actor.clone(),
            component: component.clone(),
        });
        effects.0.push(Effect::AttachComponent { actor, component });
    }
}

fn write_property(
    engine: &mut Engine,
    target: &str,
    property: &str,
    value: &blockloom_core::value::Evaluated,
) -> Result<String, String> {
    let (component, path) = blockloom_core::verbs::split_property(property)?;
    if matches!(component, "Place" | "Script" | "Parent") {
        return Err(format!(
            "{component} can't be set by property: use the motion, script or parent blocks"
        ));
    }
    let mut actor = engine
        .actor(target)
        .cloned()
        .ok_or_else(|| format!("there is no actor \"{target}\""))?;
    let current = actor
        .components
        .get(component)
        .ok_or_else(|| format!("{} has no {component}", actor.name))?;
    if path.is_empty() {
        return Err(format!(
            "name a property of {component}, like {component}.<field>"
        ));
    }
    let path = blockloom_core::registry::resolve_component_path(current, path);
    let json = blockloom_core::registry::coerce_component_value(current, &path, value);
    let next: ActorComponent =
        blockloom_core::registry::set_component_property(current, &path, json)?;
    if let Some(slot) = actor.components.get_mut(component) {
        *slot = next;
    }
    let name = component.to_string();
    engine.set_live(actor);
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};

    fn engine_with_light() -> (Engine, String) {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        let mut actor = Actor::new(
            "Lamp",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0],
            },
        );
        actor
            .components
            .insert(blockloom_core::registry::default_component("Light", Mode::ThreeD).unwrap());
        let id = actor.id.clone();
        engine.project.actors.push(actor);
        engine.running = true;
        (engine, id)
    }

    #[test]
    fn a_property_write_changes_the_runs_copy_only() {
        let (mut engine, id) = engine_with_light();
        let value = blockloom_core::value::Evaluated::Text("1234".into());
        write_property(&mut engine, &id, "Light.intensity", &value).unwrap();
        let live = engine.actor(&id).unwrap().components.get("Light").unwrap();
        let read = blockloom_core::registry::component_property(live, "light.intensity").unwrap();
        assert_eq!(read.as_f64(), Some(1234.0));
        let authored = engine
            .project
            .actors
            .iter()
            .find(|actor| actor.id == id)
            .and_then(|actor| actor.components.get("Light"))
            .unwrap();
        let before =
            blockloom_core::registry::component_property(authored, "light.intensity").unwrap();
        assert_ne!(before.as_f64(), Some(1234.0));
        engine.clear_live();
        let after = engine.actor(&id).unwrap().components.get("Light").unwrap();
        assert_eq!(
            blockloom_core::registry::component_property(after, "light.intensity").unwrap(),
            before
        );
    }

    #[test]
    fn a_bad_write_says_why() {
        let (mut engine, id) = engine_with_light();
        let text = blockloom_core::value::Evaluated::Text("x".into());
        assert!(write_property(&mut engine, &id, "Light.nonsense", &text).is_err());
        assert!(write_property(&mut engine, &id, "Water.level", &text).is_err());
        assert!(write_property(&mut engine, &id, "Place.position", &text).is_err());
        assert!(write_property(&mut engine, "nobody", "Light.intensity", &text).is_err());
    }
}
