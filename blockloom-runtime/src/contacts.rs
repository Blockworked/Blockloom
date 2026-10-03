//! The runtime half of the contact lifecycle: rapier's begin and end reports go
//! into the core `ContactTracker` once per fixed tick, and what a tick made is
//! handed to the blocks and scripts on the next one.

use crate::engine::Engine;
use crate::physics_install::PlannedCollider;
use bevy::prelude::*;
use blockloom_core::physics::{ColliderId, ContactKind, ContactPayload, Endpoint, ExitReason};

/// One begin or end the backend reported.
pub struct Seen {
    pub a: Entity,
    pub b: Entity,
    pub started: bool,
    pub sensor: bool,
    pub removed: bool,
}

/// What a dimension's systems know about an entity that took part in a pair.
pub struct Who {
    /// The actor the entity belongs to.
    pub owner: Entity,
    pub planned: Option<PlannedCollider>,
    /// Whether the actor has a body that is not fixed.
    pub moving: bool,
    pub disabled: bool,
}

/// The endpoint for a collider entity, if the world knows its actor.
pub fn endpoint(engine: &Engine, who: &Who) -> Option<Endpoint> {
    let actor = engine.actor_id_of(who.owner)?.to_string();
    let collider = match &who.planned {
        Some(planned) => planned.id.clone(),
        None => ColliderId::from(format!("{actor}#body").as_str()),
    };
    Some(Endpoint {
        collider,
        body: who.moving.then(|| actor.clone()),
        actor,
    })
}

/// Feeds one tick's reports to the tracker and closes the tick.
pub fn track(
    engine: &mut Engine,
    seen: &[Seen],
    who: impl Fn(&Engine, Entity) -> Option<Who>,
    payload: impl Fn(Entity, Entity) -> Option<ContactPayload>,
    refresh: Vec<(Entity, Entity, ContactPayload)>,
    awake: impl Fn(&Engine, &Endpoint) -> bool,
) {
    let mut tracker = std::mem::take(&mut engine.contacts);
    let touched = std::mem::take(&mut engine.filter_touched);
    for report in seen {
        let (Some(first), Some(second)) = (who(engine, report.a), who(engine, report.b)) else {
            continue;
        };
        let (Some(a), Some(b)) = (endpoint(engine, &first), endpoint(engine, &second)) else {
            continue;
        };
        if report.started {
            let kind = if report.sensor {
                ContactKind::Trigger
            } else {
                ContactKind::Collision
            };
            let body = match kind {
                ContactKind::Collision => payload(report.a, report.b),
                ContactKind::Trigger => None,
            };
            tracker.started(a, b, kind, body);
        } else {
            let reason = if report.removed {
                if first.disabled || second.disabled {
                    ExitReason::ColliderDisabled
                } else {
                    ExitReason::ColliderRemoved
                }
            } else if touched.contains(&a.actor) || touched.contains(&b.actor) {
                ExitReason::FilterChanged
            } else {
                ExitReason::Separated
            };
            tracker.stopped_for(&a.collider, &b.collider, reason);
        }
    }
    for (e1, e2, body) in refresh {
        let (Some(first), Some(second)) = (who(engine, e1), who(engine, e2)) else {
            continue;
        };
        if let (Some(x), Some(y)) = (&first.planned, &second.planned) {
            tracker.refresh(&x.id, &y.id, body);
        }
    }
    engine.contact_ticks += 1;
    let tick = engine.contact_ticks;
    tracker.end_tick(tick, |endpoint| awake(engine, endpoint));
    engine.contacts = tracker;
    crate::world::sync_touching(engine);
}
