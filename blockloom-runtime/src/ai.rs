//! Behavior tree decisions, evaluated once per fixed simulation step.

use bevy::prelude::*;
use blockloom_core::ai::{BehaviorNode, BrainSpec};
use blockloom_core::physics::query::{self, QueryFilter, QueryRequest, TriggerPolicy};
use blockloom_core::scene::Mode;
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
    mode: Mode,
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
        // What stands between: the nearest solid thing along the sight line, as
        // the asker (so its own colliders and layer rules apply).
        let request = QueryRequest::Ray {
            from: self.from.to_array(),
            to: to.to_array(),
            all: false,
        };
        let filter = QueryFilter {
            triggers: TriggerPolicy::Ignore,
            ..QueryFilter::as_actor(self.actor)
        };
        let seen = query::dispatch(&request, &filter, 1);
        seen.hits
            .first()
            .is_none_or(|hit| Some(hit.actor.as_str()) == self.target_id)
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
    queries: crate::queries::QueryAccess,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mut ids: Vec<_> = engine.entities.keys().collect();
    ids.sort();
    queries.scope(engine.contact_ticks, || {
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
                mode: dimension.0,
            };
            decision.eval(&brain.tree, &mut effects.0, 0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::physics::query::{QueryHit, QueryOutcome, QueryService};

    /// A world with one thing on the sight line, at `distance`.
    struct Along(Option<(&'static str, f32)>);

    impl QueryService for Along {
        fn run(&self, request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryOutcome {
            // The enemy asks as itself and never for triggers.
            assert_eq!(filter.as_actor.as_deref(), Some("enemy"));
            assert_eq!(filter.triggers, TriggerPolicy::Ignore);
            assert!(matches!(request, QueryRequest::Ray { all: false, .. }));
            let hits = self
                .0
                .map(|(actor, distance)| QueryHit {
                    actor: actor.to_string(),
                    body: None,
                    collider: "c".into(),
                    subshape: 0,
                    point: [distance, 0.0, 0.0],
                    normal: [-1.0, 0.0, 0.0],
                    distance,
                    fraction: distance / 5.0,
                    started_inside: false,
                    trigger: false,
                })
                .into_iter()
                .collect();
            QueryOutcome::finish(hits, limit)
        }
    }

    fn sees(world: &Along, facing: Vec3) -> bool {
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
            facing,
            brain: &brain,
            mode: Mode::TwoD,
        };
        query::with_service(world, 1, || decision.visible())
    }

    #[test]
    fn sight_respects_range_cone_and_occlusion() {
        let clear = Along(Some(("player", 5.0)));
        assert!(sees(&clear, Vec3::X));
        // Behind the enemy, outside its cone.
        assert!(!sees(&clear, -Vec3::X));
        // Something solid stands in the way.
        assert!(!sees(&Along(Some(("wall", 2.0))), Vec3::X));
        // An empty line (the target has no collider) is a clear one.
        assert!(sees(&Along(None), Vec3::X));
    }

    #[test]
    fn selector_discards_failed_branch_effects() {
        let brain = BrainSpec::default();
        let decision = Decision {
            actor: "enemy",
            target_id: None,
            from: Vec3::ZERO,
            to: None,
            facing: Vec3::X,
            brain: &brain,
            mode: Mode::TwoD,
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
