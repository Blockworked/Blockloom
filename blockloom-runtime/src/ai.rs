//! Behavior tree decisions, evaluated once per fixed simulation step.

use bevy::prelude::*;
use blockloom_core::ai::{BehaviorNode, BrainSpec};
use blockloom_core::physics_query;
use blockloom_core::scene::Mode;
use blockloom_core::sense::Sensors;
use blockloom_core::vm::Effect;

use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::world::forward_of;

struct Decision<'a> {
    actor: &'a str,
    target_id: Option<&'a str>,
    from: Vec3,
    to: Option<Vec3>,
    facing: Vec3,
    brain: &'a BrainSpec,
    sensors: &'a Sensors,
    mode: Mode,
    mask: u8,
}

impl Decision<'_> {
    fn delta(&self) -> Option<Vec3> {
        let mut delta = self.to? - self.from;
        if self.mode == Mode::TwoD {
            delta.z = 0.0;
        } else {
            delta.y = 0.0;
        }
        Some(delta)
    }

    fn visible(&self) -> bool {
        let Some(delta) = self.delta() else {
            return false;
        };
        let distance = delta.length();
        if distance > self.brain.sight.max(0.0) {
            return false;
        }
        if distance > 1e-4 && self.brain.fov < 360.0 {
            let facing = if self.mode == Mode::TwoD {
                Vec3::new(self.facing.x, self.facing.y, 0.0)
            } else {
                Vec3::new(self.facing.x, 0.0, self.facing.z)
            };
            if facing.length_squared() > 1e-6
                && facing.angle_between(delta).to_degrees() > self.brain.fov.max(0.0) * 0.5
            {
                return false;
            }
        }
        let Some(to) = self.to else {
            return false;
        };
        let hit = physics_query::ray_hit(
            self.sensors,
            self.from.to_array(),
            to.to_array(),
            Some(self.actor),
            self.mask,
        );
        hit.is_none_or(|(id, _)| Some(id.as_str()) == self.target_id)
    }

    fn eval(&self, node: &BehaviorNode, effects: &mut Vec<Effect>, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match node {
            BehaviorNode::Selector { children } => {
                for child in children {
                    let start = effects.len();
                    if self.eval(child, effects, depth + 1) {
                        return true;
                    }
                    effects.truncate(start);
                }
                false
            }
            BehaviorNode::Sequence { children } => {
                let start = effects.len();
                for child in children {
                    if !self.eval(child, effects, depth + 1) {
                        effects.truncate(start);
                        return false;
                    }
                }
                true
            }
            BehaviorNode::CanSeeTarget => self.visible(),
            BehaviorNode::TargetNear { distance } => self
                .delta()
                .is_some_and(|d| d.length() <= distance.max(0.0)),
            BehaviorNode::NavigateToTarget => {
                let Some(to) = self.to else {
                    return false;
                };
                effects.push(Effect::NavigateTo {
                    actor: self.actor.to_string(),
                    target: to.to_array(),
                    speed: self.brain.speed.max(0.0),
                });
                true
            }
            BehaviorNode::FleeTarget => {
                let Some(delta) = self.delta() else {
                    return false;
                };
                if delta.length_squared() < 1e-6 {
                    return false;
                }
                let to = self.from - delta.normalize() * self.brain.sight.max(1.0);
                effects.push(Effect::NavigateTo {
                    actor: self.actor.to_string(),
                    target: to.to_array(),
                    speed: self.brain.speed.max(0.0),
                });
                true
            }
            BehaviorNode::Idle => true,
        }
    }
}

pub fn tick(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    transforms: Query<&Transform, With<ActorId>>,
    mut effects: ResMut<PendingEffects>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mut ids: Vec<_> = engine.entities.keys().collect();
    ids.sort();
    blockloom_core::sense::read(|sensors| {
        for id in &ids {
            if !engine.has_component(id, "Brain") {
                continue;
            }
            let Some(brain) = engine.actor(id).and_then(|a| a.components.brain()) else {
                continue;
            };
            let Some(entity) = engine.entities.get(*id) else {
                continue;
            };
            let Ok(transform) = transforms.get(*entity) else {
                continue;
            };
            let target_id = ids
                .iter()
                .find(|candidate| {
                    candidate.as_str() != id.as_str()
                        && (candidate.as_str() == brain.target
                            || engine
                                .actor(candidate)
                                .is_some_and(|a| a.name.eq_ignore_ascii_case(&brain.target)))
                })
                .map(|s| s.as_str());
            let to = target_id
                .and_then(|id| engine.entities.get(id))
                .and_then(|e| transforms.get(*e).ok())
                .map(|t| t.translation);
            let decision = Decision {
                actor: id,
                target_id,
                from: transform.translation,
                to,
                facing: forward_of(transform, dimension.0),
                brain,
                sensors,
                mode: dimension.0,
                mask: engine.filter_of(id).1,
            };
            decision.eval(&brain.tree, &mut effects.0, 0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::sense::{ActorSense, ColliderShape};

    fn target(at: [f32; 3]) -> ActorSense {
        ActorSense {
            name: "Player".to_string(),
            position: at,
            has_body: true,
            shape: ColliderShape::Ball { radius: 0.2 },
            ..Default::default()
        }
    }

    #[test]
    fn sight_respects_range_cone_and_occlusion() {
        let mut sensors = Sensors::default();
        sensors
            .actors
            .insert("player".to_string(), target([5.0, 0.0, 0.0]));
        let brain = BrainSpec {
            target: "player".to_string(),
            sight: 10.0,
            fov: 90.0,
            ..Default::default()
        };
        let decision = Decision {
            actor: "enemy",
            target_id: Some("player"),
            from: Vec3::ZERO,
            to: Some(Vec3::X * 5.0),
            facing: Vec3::X,
            brain: &brain,
            sensors: &sensors,
            mode: Mode::TwoD,
            mask: 255,
        };
        assert!(decision.visible());
        assert!(
            !Decision {
                facing: -Vec3::X,
                ..decision
            }
            .visible()
        );
        sensors.actors.insert(
            "wall".to_string(),
            ActorSense {
                position: [2.0, 0.0, 0.0],
                has_body: true,
                shape: ColliderShape::Box {
                    half: [0.2, 0.2, 0.2],
                },
                ..Default::default()
            },
        );
        let blocked = Decision {
            actor: "enemy",
            target_id: Some("player"),
            from: Vec3::ZERO,
            to: Some(Vec3::X * 5.0),
            facing: Vec3::X,
            brain: &brain,
            sensors: &sensors,
            mode: Mode::TwoD,
            mask: 255,
        };
        assert!(!blocked.visible());
    }

    #[test]
    fn selector_discards_failed_branch_effects() {
        let sensors = Sensors::default();
        let brain = BrainSpec::default();
        let decision = Decision {
            actor: "enemy",
            target_id: None,
            from: Vec3::ZERO,
            to: None,
            facing: Vec3::X,
            brain: &brain,
            sensors: &sensors,
            mode: Mode::TwoD,
            mask: 255,
        };
        let tree = BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![BehaviorNode::Idle, BehaviorNode::NavigateToTarget],
                },
                BehaviorNode::Idle,
            ],
        };
        let mut effects = Vec::new();
        assert!(decision.eval(&tree, &mut effects, 0));
        assert!(effects.is_empty());
    }
}
