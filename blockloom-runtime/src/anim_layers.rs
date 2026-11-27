//! Animation layers: clips a model plays on a bone-masked layer over its
//! state machine (`play animation on layer`). The clocks, weights, markers
//! and ends run here on the fixed tick; `model::drive_animations` puts them
//! on the rig, each layer's bones taking `weight` of the layer's clip.

use bevy::prelude::*;
use blockloom_core::animation::{AnimationSpec, LoopMode};
use blockloom_core::vm::Event;
use blockloom_protocol::RuntimeMessage;
use serde_json::Value;

use crate::bridge;
use crate::engine::{ActorId, Engine, PendingEffects};
use crate::verbs::calls;

/// One layer playing on an actor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerPlay {
    pub layer: String,
    pub clip: String,
    pub elapsed: f32,
    /// How much of the layer's bones the clip has now, easing to `target`.
    pub weight: f32,
    pub target: f32,
    pub last_step: Option<u64>,
    pub ended: bool,
}

/// The layers an actor is playing, by layer.
#[derive(Component, Debug, Clone, Default)]
pub struct AnimationLayers(pub Vec<LayerPlay>);

impl AnimationLayers {
    fn entry(&mut self, layer: &str) -> &mut LayerPlay {
        let at = match self.0.iter().position(|play| play.layer == layer) {
            Some(at) => at,
            None => {
                self.0.push(LayerPlay {
                    layer: layer.to_string(),
                    ..Default::default()
                });
                self.0.len() - 1
            }
        };
        &mut self.0[at]
    }
}

fn report(actor: &str, message: String) {
    bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

fn text(args: &[Value], at: usize) -> &str {
    args.get(at).and_then(Value::as_str).unwrap_or_default()
}

fn spec_of(engine: &Engine, actor: &str) -> Option<AnimationSpec> {
    engine
        .actor(actor)
        .and_then(|a| a.components.animation())
        .filter(|_| engine.has_component(actor, "Animation"))
        .cloned()
}

/// `play animation on layer`, `stop animation layer` and `set animation
/// layer weight`, on the fixed tick before the layers step.
pub fn apply_layer_verbs(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    mut actors: Query<&mut AnimationLayers>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mut fresh: std::collections::HashMap<Entity, AnimationLayers> = Default::default();
    for verb in ["play_layer", "stop_layer", "set_layer_weight"] {
        for (actor, args) in calls(&effects, verb) {
            let Some(&entity) = engine.entities.get(actor) else {
                continue;
            };
            let Some(spec) = spec_of(&engine, actor) else {
                report(actor, "this actor has no Animation component".to_string());
                continue;
            };
            let named = if verb == "play_layer" {
                text(args, 1)
            } else {
                text(args, 0)
            };
            let Some(layer) = spec.find_layer(named) else {
                report(
                    actor,
                    format!("my Animation component has no layer called \"{named}\""),
                );
                continue;
            };
            let mut held = actors.get_mut(entity).ok();
            let layers = match held.as_deref_mut() {
                Some(layers) => layers,
                None => fresh.entry(entity).or_default(),
            };
            match verb {
                "play_layer" => {
                    let wanted = text(args, 0);
                    let Some(clip) = spec.find_clip(wanted) else {
                        report(actor, format!("there is no clip called \"{wanted}\""));
                        continue;
                    };
                    let play = layers.entry(&layer.name);
                    play.clip = clip.name.clone();
                    play.elapsed = 0.0;
                    play.target = layer.weight;
                    play.last_step = None;
                    play.ended = false;
                }
                "stop_layer" => layers.entry(&layer.name).target = 0.0,
                _ => {
                    let weight = args.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    layers.entry(&layer.name).target = weight.clamp(0.0, 1.0);
                }
            }
        }
    }
    for (entity, layers) in fresh {
        commands.entity(entity).insert(layers);
    }
}

/// Advances every layer: its clip's clock and markers, its weight toward
/// its target at the spec's crossfade, and a `Once` clip's end, which fires
/// `when animation ends` and fades the layer out. A layer faded to nothing
/// with nothing to fade to is dropped.
pub fn step_layers(
    time: Res<Time>,
    mut engine: NonSendMut<Engine>,
    mut actors: Query<(&ActorId, &mut AnimationLayers)>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    let mut events = Vec::new();
    for (id, mut layers) in &mut actors {
        let Some(spec) = spec_of(&engine, &id.0) else {
            layers.0.clear();
            continue;
        };
        let rate = if spec.crossfade > 0.0 {
            dt / spec.crossfade
        } else {
            f32::INFINITY
        };
        for play in &mut layers.0 {
            let gap = play.target - play.weight;
            play.weight += gap.clamp(-rate, rate);
            let Some(clip) = spec.find_clip(&play.clip) else {
                play.target = 0.0;
                play.weight = 0.0;
                continue;
            };
            if play.ended {
                continue;
            }
            play.elapsed += dt;
            let cursor = clip.cursor(play.elapsed);
            for marker in clip.markers_reached(play.last_step, cursor) {
                events.push(Event::AnimationMarker {
                    actor: id.0.clone(),
                    marker: marker.to_string(),
                });
            }
            play.last_step = Some(cursor.step);
            if clip.loop_mode == LoopMode::Once && play.elapsed >= clip.duration() {
                play.ended = true;
                play.target = 0.0;
                events.push(Event::AnimationEnded {
                    actor: id.0.clone(),
                    clip: play.clip.clone(),
                });
            }
        }
        layers
            .0
            .retain(|play| play.weight > 0.0 || play.target > 0.0);
    }
    for event in events {
        engine.fire(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::InsertEngine;
    use bevy::time::{TimePlugin, TimeUpdateStrategy};
    use blockloom_core::components::ActorComponent;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};

    #[test]
    fn a_layer_fades_in_plays_once_and_fades_back_out() {
        let spec: AnimationSpec = serde_json::from_str(
            r#"{
                "crossfade": 0.25,
                "clips": [{"name":"Wave","seconds":0.5,"fps":30,"loop_mode":"Once",
                           "markers":[{"frame":3,"name":"hand up"}]}],
                "layers": [{"name":"Upper body","bones":["Spine"],"weight":0.8}]
            }"#,
        )
        .unwrap();
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut actor = Actor::new(
            "Hero",
            Visual::Cuboid {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        actor
            .components
            .insert(ActorComponent::Animation { animation: spec });
        let id = actor.id.clone();
        engine
            .attached
            .insert(id.clone(), ["Animation".to_string()].into_iter().collect());
        engine.project.actors.push(actor);
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), TimePlugin));
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        app.init_resource::<PendingEffects>();
        let entity = app.world_mut().spawn(ActorId(id.clone())).id();
        engine.entities.insert(id.clone(), entity);
        app.insert_engine(engine);
        app.add_systems(FixedUpdate, (apply_layer_verbs, step_layers).chain());
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![blockloom_core::verbs::effect(
            &id,
            "play_layer",
            &[
                blockloom_core::value::Evaluated::Text("wave".into()),
                blockloom_core::value::Evaluated::Text("upper BODY".into()),
            ],
        )];
        app.update();
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        let play = |app: &App| {
            app.world()
                .get::<AnimationLayers>(entity)
                .and_then(|layers| layers.0.first().cloned())
        };
        let first = play(&app).expect("the layer plays");
        assert_eq!(
            (first.layer.as_str(), first.clip.as_str()),
            ("Upper body", "Wave")
        );
        assert!(
            first.weight > 0.0 && first.weight < 0.8,
            "easing in: {first:?}"
        );
        for _ in 0..20 {
            app.update();
        }
        assert!((play(&app).unwrap().weight - 0.8).abs() < 1e-4);
        for _ in 0..30 {
            app.update();
        }
        let heard = app
            .world()
            .non_send::<Engine>()
            .vm
            .pending_events()
            .to_vec();
        assert!(
            heard
                .iter()
                .any(|e| matches!(e, Event::AnimationEnded { clip, .. } if clip == "Wave"))
        );
        assert!(
            heard
                .iter()
                .any(|e| matches!(e, Event::AnimationMarker { marker, .. } if marker == "hand up"))
        );
        // Ended, it fades out and is dropped.
        for _ in 0..20 {
            app.update();
        }
        assert!(play(&app).is_none());
    }
}
