//! The contact lifecycle: which collider pairs touch, what changed on each fixed
//! tick, and the events gameplay hears about it.
//!
//! The backend reports raw begin and end transitions; this tracker turns them
//! into Enter once, Stay at most once per eligible tick and Exit once, keeps the
//! actor-level "touching" answer as a count over active collider pairs, and makes
//! up an Exit (with a reason) for a pair whose collider went away. Events made in
//! tick N are handed out from tick N+1 on, whatever the render rate was.

use super::ColliderId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// One side of a pair: the collider, the actor that carries it and the actor
/// whose body owns it (none for scenery).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Endpoint {
    pub collider: ColliderId,
    pub actor: String,
    pub body: Option<String>,
}

/// Whether the pair pushes (a solid collision) or only overlaps (a trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ContactKind {
    Collision,
    Trigger,
}

impl ContactKind {
    pub fn name(self) -> &'static str {
        match self {
            ContactKind::Collision => "Collision",
            ContactKind::Trigger => "Trigger",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "collision" => Some(ContactKind::Collision),
            "trigger" => Some(ContactKind::Trigger),
            _ => None,
        }
    }
}

/// Which kinds of pair a `when I touch` hat listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactScope {
    /// Solid touches and trigger overlaps both: what the hat always did.
    #[default]
    Any,
    Collision,
    Trigger,
}

impl ContactScope {
    pub fn name(self) -> &'static str {
        match self {
            ContactScope::Any => "Any",
            ContactScope::Collision => "Collision",
            ContactScope::Trigger => "Trigger",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "any" => Some(ContactScope::Any),
            "collision" => Some(ContactScope::Collision),
            "trigger" => Some(ContactScope::Trigger),
            _ => None,
        }
    }

    pub fn accepts(self, kind: ContactKind) -> bool {
        match self {
            ContactScope::Any => true,
            ContactScope::Collision => kind == ContactKind::Collision,
            ContactScope::Trigger => kind == ContactKind::Trigger,
        }
    }
}

/// Where a pair is in its life. The order is the order events of one pair take
/// within a tick.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ContactPhase {
    #[default]
    Enter,
    Stay,
    Exit,
}

impl ContactPhase {
    pub fn name(self) -> &'static str {
        match self {
            ContactPhase::Enter => "Enter",
            ContactPhase::Stay => "Stay",
            ContactPhase::Exit => "Exit",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "enter" => Some(ContactPhase::Enter),
            "stay" => Some(ContactPhase::Stay),
            "exit" => Some(ContactPhase::Exit),
            _ => None,
        }
    }
}

/// Why a pair ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExitReason {
    /// The shapes moved apart.
    Separated,
    /// A collider was switched off.
    ColliderDisabled,
    /// A collider was deleted or its shape taken away.
    ColliderRemoved,
    /// An actor was deleted.
    ActorRemoved,
    /// A collider's owner changed (reparent, body added or removed).
    OwnershipChanged,
    /// A collider's layer or filter changed so the pair no longer collides.
    FilterChanged,
}

impl ExitReason {
    pub fn name(self) -> &'static str {
        match self {
            ExitReason::Separated => "Separated",
            ExitReason::ColliderDisabled => "ColliderDisabled",
            ExitReason::ColliderRemoved => "ColliderRemoved",
            ExitReason::ActorRemoved => "ActorRemoved",
            ExitReason::OwnershipChanged => "OwnershipChanged",
            ExitReason::FilterChanged => "FilterChanged",
        }
    }
}

/// One contact point, in world units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ContactPoint {
    pub point: [f32; 3],
    /// Negative while the shapes overlap.
    pub separation: f32,
}

/// What a collision event knows about the touch. A trigger has none of it: it
/// has identity and phase but no contact force to fabricate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContactPayload {
    /// Points from the first collider of the pair towards the second.
    pub normal: [f32; 3],
    pub points: Vec<ContactPoint>,
    /// The second body's velocity relative to the first at the contact.
    pub relative_velocity: [f32; 3],
    /// The impulse the solver spent on the pair this tick.
    pub impulse: f32,
}

/// What gameplay hears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContactEvent {
    /// The fixed tick the transition happened in.
    pub tick: u64,
    pub phase: ContactPhase,
    pub kind: ContactKind,
    /// The pair, ordered by collider id so the same pair always reads the same.
    pub a: Endpoint,
    pub b: Endpoint,
    /// Why an Exit happened; none for Enter and Stay.
    pub reason: Option<ExitReason>,
    pub payload: Option<ContactPayload>,
    /// Whether this is the actor-level edge too: the first pair of its kind to
    /// join two actors (Enter), the last to leave (Exit), or the one Stay a
    /// tick gives the actor pair. `when I touch` listens to these only.
    pub edge: bool,
}

impl ContactEvent {
    /// The pair's stable identity.
    pub fn pair_id(&self) -> (&ColliderId, &ColliderId) {
        (&self.a.collider, &self.b.collider)
    }

    /// This event as the other side of the pair hears it: the endpoints swapped
    /// and the payload turned round, so "me" is always first.
    pub fn seen_by(&self, actor: &str) -> Option<ContactEvent> {
        if self.a.actor == actor {
            Some(self.clone())
        } else if self.b.actor == actor {
            let mut flipped = self.clone();
            std::mem::swap(&mut flipped.a, &mut flipped.b);
            if let Some(payload) = flipped.payload.as_mut() {
                payload.normal = payload.normal.map(|n| -n);
                payload.relative_velocity = payload.relative_velocity.map(|v| -v);
            }
            Some(flipped)
        } else {
            None
        }
    }
}

type PairKey = (ColliderId, ColliderId);

fn edge_key(a: &str, b: &str, kind: ContactKind) -> (String, String, ContactKind) {
    if a <= b {
        (a.to_string(), b.to_string(), kind)
    } else {
        (b.to_string(), a.to_string(), kind)
    }
}

fn key_of(a: &ColliderId, b: &ColliderId) -> PairKey {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

#[derive(Debug, Clone)]
struct Active {
    kind: ContactKind,
    a: Endpoint,
    b: Endpoint,
    payload: Option<ContactPayload>,
    entered: u64,
}

#[derive(Debug, Clone)]
enum Transition {
    Started {
        a: Endpoint,
        b: Endpoint,
        kind: ContactKind,
        payload: Option<ContactPayload>,
    },
    Stopped {
        a: ColliderId,
        b: ColliderId,
        reason: ExitReason,
    },
}

/// Follows every collider pair of a world.
#[derive(Debug, Clone, Default)]
pub struct ContactTracker {
    active: BTreeMap<PairKey, Active>,
    transitions: Vec<Transition>,
    outbox: VecDeque<ContactEvent>,
    /// Active collider pairs per ordered pair of actors, for `touching?`.
    touching: HashMap<String, BTreeMap<String, u32>>,
    /// Active collider pairs per unordered actor pair and kind, for edges.
    edges: HashMap<(String, String, ContactKind), u32>,
    /// Set whenever `touching` changed, until `take_changed`.
    changed: bool,
}

impl ContactTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The backend saw two shapes begin to touch.
    pub fn started(
        &mut self,
        a: Endpoint,
        b: Endpoint,
        kind: ContactKind,
        payload: Option<ContactPayload>,
    ) {
        self.transitions.push(Transition::Started {
            a,
            b,
            kind,
            payload,
        });
    }

    /// The backend saw two shapes stop touching.
    pub fn stopped(&mut self, a: &ColliderId, b: &ColliderId) {
        self.transitions.push(Transition::Stopped {
            a: a.clone(),
            b: b.clone(),
            reason: ExitReason::Separated,
        });
    }

    /// The backend reports a pair ended because a collider was removed or
    /// switched off.
    pub fn stopped_for(&mut self, a: &ColliderId, b: &ColliderId, reason: ExitReason) {
        self.transitions.push(Transition::Stopped {
            a: a.clone(),
            b: b.clone(),
            reason,
        });
    }

    /// A collider went away (or stopped taking part): every pair it was in ends,
    /// with `reason`, at the close of this tick.
    pub fn remove_collider(&mut self, id: &ColliderId, reason: ExitReason) {
        let ends: Vec<PairKey> = self
            .active
            .keys()
            .filter(|(a, b)| a == id || b == id)
            .cloned()
            .collect();
        for (a, b) in ends {
            self.transitions.push(Transition::Stopped { a, b, reason });
        }
    }

    /// An actor was deleted: all its colliders go.
    pub fn remove_actor(&mut self, actor: &str) {
        let ends: Vec<PairKey> = self
            .active
            .iter()
            .filter(|(_, pair)| pair.a.actor == actor || pair.b.actor == actor)
            .map(|(key, _)| key.clone())
            .collect();
        for (a, b) in ends {
            self.transitions.push(Transition::Stopped {
                a,
                b,
                reason: ExitReason::ActorRemoved,
            });
        }
    }

    /// The colliders still in the world after a shape replacement or an
    /// ownership change. A pair with both sides still there is kept as it is,
    /// so a geometry swap does not make a fake Exit and Enter; a pair that lost
    /// a side ends with `reason`.
    pub fn retain_colliders(&mut self, alive: &HashSet<ColliderId>, reason: ExitReason) {
        let ends: Vec<PairKey> = self
            .active
            .keys()
            .filter(|(a, b)| !alive.contains(a) || !alive.contains(b))
            .cloned()
            .collect();
        for (a, b) in ends {
            self.transitions.push(Transition::Stopped { a, b, reason });
        }
    }

    /// Closes `tick`: applies what the backend reported, queues the events and
    /// decides who gets a Stay. `awake` says whether an endpoint's body is moving
    /// or could be; a pair where neither side is awake is quiet.
    pub fn end_tick(&mut self, tick: u64, awake: impl Fn(&Endpoint) -> bool) {
        let before: HashSet<PairKey> = self.active.keys().cloned().collect();
        let mut made: Vec<(PairKey, ContactEvent)> = Vec::new();
        for transition in std::mem::take(&mut self.transitions) {
            match transition {
                Transition::Started {
                    a,
                    b,
                    kind,
                    payload,
                } => {
                    if a.actor == b.actor && a.body == b.body {
                        // Shapes of one body never touch each other.
                        continue;
                    }
                    let (a, b) = if a.collider <= b.collider {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let key = (a.collider.clone(), b.collider.clone());
                    if self.active.contains_key(&key) {
                        continue;
                    }
                    self.count(&a.actor, &b.actor, 1);
                    let edge = self.edge(&a.actor, &b.actor, kind, 1);
                    made.push((
                        key.clone(),
                        ContactEvent {
                            tick,
                            phase: ContactPhase::Enter,
                            kind,
                            a: a.clone(),
                            b: b.clone(),
                            reason: None,
                            payload: payload.clone(),
                            edge,
                        },
                    ));
                    self.active.insert(
                        key,
                        Active {
                            kind,
                            a,
                            b,
                            payload,
                            entered: tick,
                        },
                    );
                }
                Transition::Stopped { a, b, reason } => {
                    let key = key_of(&a, &b);
                    let Some(pair) = self.active.remove(&key) else {
                        continue;
                    };
                    self.count(&pair.a.actor, &pair.b.actor, -1);
                    let edge = self.edge(&pair.a.actor, &pair.b.actor, pair.kind, -1);
                    made.push((
                        key,
                        ContactEvent {
                            tick,
                            phase: ContactPhase::Exit,
                            kind: pair.kind,
                            a: pair.a,
                            b: pair.b,
                            reason: Some(reason),
                            payload: None,
                            edge,
                        },
                    ));
                }
            }
        }
        // A pair that was already touching and still is gets a Stay, once.
        let mut staying: HashSet<(String, String, ContactKind)> = HashSet::new();
        for (key, pair) in &self.active {
            if before.contains(key) && pair.entered < tick && (awake(&pair.a) || awake(&pair.b)) {
                let edge = staying.insert(edge_key(&pair.a.actor, &pair.b.actor, pair.kind));
                made.push((
                    key.clone(),
                    ContactEvent {
                        tick,
                        phase: ContactPhase::Stay,
                        kind: pair.kind,
                        a: pair.a.clone(),
                        b: pair.b.clone(),
                        reason: None,
                        payload: pair.payload.clone(),
                        edge,
                    },
                ));
            }
        }
        // Tick, pair id, phase: the same order however the backend listed them.
        // The sort is stable, so one pair's events within a phase keep arrival order.
        made.sort_by(|(ka, ea), (kb, eb)| ka.cmp(kb).then(ea.phase.cmp(&eb.phase)));
        self.outbox.extend(made.into_iter().map(|(_, event)| event));
    }

    /// Fresh numbers for a pair that is still touching, for the next Stay.
    pub fn refresh(&mut self, a: &ColliderId, b: &ColliderId, payload: ContactPayload) {
        if let Some(pair) = self.active.get_mut(&key_of(a, b))
            && pair.kind == ContactKind::Collision
        {
            pair.payload = Some(payload);
        }
    }

    /// Events made before `tick`, oldest first. Events made in `tick` itself wait
    /// for the next one.
    pub fn deliver(&mut self, tick: u64) -> Vec<ContactEvent> {
        let mut out = Vec::new();
        while self.outbox.front().is_some_and(|event| event.tick < tick) {
            out.extend(self.outbox.pop_front());
        }
        out
    }

    /// A reset or unload: nothing queued reaches the world that replaces this one.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Moves the (kind, actor pair) count and says whether it crossed zero.
    fn edge(&mut self, a: &str, b: &str, kind: ContactKind, by: i32) -> bool {
        let key = edge_key(a, b, kind);
        let slot = self.edges.entry(key.clone()).or_insert(0);
        *slot = slot.saturating_add_signed(by);
        let crossed = if by > 0 { *slot == 1 } else { *slot == 0 };
        if *slot == 0 {
            self.edges.remove(&key);
        }
        crossed
    }

    /// Whether `touching` changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn count(&mut self, a: &str, b: &str, by: i32) {
        self.changed = true;
        for (one, other) in [(a, b), (b, a)] {
            let row = self.touching.entry(one.to_string()).or_default();
            let slot = row.entry(other.to_string()).or_insert(0);
            *slot = slot.saturating_add_signed(by);
            if *slot == 0 {
                row.remove(other);
            }
            if row.is_empty() {
                self.touching.remove(one);
            }
        }
    }

    /// The actors `actor` is touching right now: one or more collider pairs.
    pub fn touching(&self, actor: &str) -> impl Iterator<Item = &str> {
        self.touching
            .get(actor)
            .into_iter()
            .flat_map(|row| row.keys().map(String::as_str))
    }

    pub fn is_touching(&self, actor: &str, other: &str) -> bool {
        self.touching
            .get(actor)
            .is_some_and(|row| row.contains_key(other))
    }

    /// How many collider pairs join two actors.
    pub fn pairs_between(&self, actor: &str, other: &str) -> u32 {
        self.touching
            .get(actor)
            .and_then(|row| row.get(other))
            .copied()
            .unwrap_or(0)
    }

    /// Every pair touching now, in pair order.
    pub fn active_pairs(&self) -> impl Iterator<Item = (&Endpoint, &Endpoint, ContactKind)> {
        self.active
            .values()
            .map(|pair| (&pair.a, &pair.b, pair.kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(collider: &str, actor: &str) -> Endpoint {
        Endpoint {
            collider: ColliderId::from(collider),
            actor: actor.to_string(),
            body: Some(actor.to_string()),
        }
    }

    fn scenery(collider: &str, actor: &str) -> Endpoint {
        Endpoint {
            collider: ColliderId::from(collider),
            actor: actor.to_string(),
            body: None,
        }
    }

    fn awake(_: &Endpoint) -> bool {
        true
    }

    fn phases(events: &[ContactEvent]) -> Vec<(u64, ContactPhase)> {
        events.iter().map(|e| (e.tick, e.phase)).collect()
    }

    #[test]
    fn enter_stay_exit_each_arrive_in_the_next_tick() {
        let mut t = ContactTracker::new();
        t.started(
            end("c1", "ball"),
            scenery("c2", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        assert!(t.deliver(1).is_empty(), "tick 1's events wait for tick 2");
        assert_eq!(phases(&t.deliver(2)), [(1, ContactPhase::Enter)]);
        t.end_tick(2, awake);
        t.end_tick(3, awake);
        let stays = t.deliver(4);
        assert_eq!(
            phases(&stays),
            [(2, ContactPhase::Stay), (3, ContactPhase::Stay)]
        );
        t.stopped(&ColliderId::from("c2"), &ColliderId::from("c1"));
        t.end_tick(4, awake);
        let exit = t.deliver(5);
        assert_eq!(phases(&exit), [(4, ContactPhase::Exit)]);
        assert_eq!(exit[0].reason, Some(ExitReason::Separated));
        assert!(!t.is_touching("ball", "floor"));
    }

    #[test]
    fn a_pair_is_the_same_whichever_way_the_backend_lists_it() {
        let mut t = ContactTracker::new();
        t.started(
            scenery("z", "floor"),
            end("a", "ball"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        let event = &t.deliver(2)[0];
        assert_eq!(event.a.collider.as_str(), "a");
        assert_eq!(event.b.collider.as_str(), "z");
        // Reported again the other way round: still one pair.
        t.started(
            end("a", "ball"),
            scenery("z", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(2, awake);
        assert_eq!(phases(&t.deliver(3)), [(2, ContactPhase::Stay)]);
    }

    #[test]
    fn a_touch_that_starts_and_ends_in_one_tick_keeps_both_events_in_order() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            scenery("b", "wall"),
            ContactKind::Collision,
            None,
        );
        t.stopped(&ColliderId::from("a"), &ColliderId::from("b"));
        t.end_tick(1, awake);
        assert_eq!(
            phases(&t.deliver(2)),
            [(1, ContactPhase::Enter), (1, ContactPhase::Exit)]
        );
        assert!(!t.is_touching("ball", "wall"));
    }

    #[test]
    fn actor_touching_stays_true_until_the_last_shape_pair_ends() {
        let mut t = ContactTracker::new();
        for (mine, theirs) in [("p1", "w"), ("p2", "w")] {
            t.started(
                end(mine, "car"),
                scenery(theirs, "wall"),
                ContactKind::Collision,
                None,
            );
        }
        t.end_tick(1, awake);
        assert_eq!(t.pairs_between("car", "wall"), 2);
        t.stopped(&ColliderId::from("p1"), &ColliderId::from("w"));
        t.end_tick(2, awake);
        assert!(t.is_touching("car", "wall"), "one shape still touches");
        assert_eq!(t.touching("wall").collect::<Vec<_>>(), ["car"]);
        t.stopped(&ColliderId::from("p2"), &ColliderId::from("w"));
        t.end_tick(3, awake);
        assert!(!t.is_touching("car", "wall"));
        assert_eq!(t.touching("car").count(), 0);
    }

    #[test]
    fn removing_a_collider_ends_its_pairs_with_the_reason() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            scenery("b", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        t.remove_collider(&ColliderId::from("a"), ExitReason::ColliderDisabled);
        t.end_tick(2, awake);
        let events = t.deliver(3);
        let exit = events.last().unwrap();
        assert_eq!(exit.phase, ContactPhase::Exit);
        assert_eq!(exit.reason, Some(ExitReason::ColliderDisabled));
        assert!(!t.is_touching("ball", "floor"));
        // The backend reporting the separation afterwards changes nothing.
        t.stopped(&ColliderId::from("a"), &ColliderId::from("b"));
        t.end_tick(3, awake);
        assert!(t.deliver(4).is_empty());
    }

    #[test]
    fn removing_an_actor_ends_everything_it_touched() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            end("b", "crate"),
            ContactKind::Collision,
            None,
        );
        t.started(
            end("a", "ball"),
            end("c", "other"),
            ContactKind::Trigger,
            None,
        );
        t.end_tick(1, awake);
        t.remove_actor("ball");
        t.end_tick(2, awake);
        let exits: Vec<_> = t
            .deliver(3)
            .into_iter()
            .filter(|e| e.phase == ContactPhase::Exit)
            .collect();
        assert_eq!(exits.len(), 2);
        assert!(
            exits
                .iter()
                .all(|e| e.reason == Some(ExitReason::ActorRemoved))
        );
    }

    #[test]
    fn replacing_a_shape_keeps_the_pair_without_a_fake_exit() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            scenery("b", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        let alive: HashSet<_> = ["a", "b"].into_iter().map(ColliderId::from).collect();
        t.retain_colliders(&alive, ExitReason::ColliderRemoved);
        t.end_tick(2, awake);
        let kinds: Vec<_> = t.deliver(3).into_iter().map(|e| e.phase).collect();
        assert_eq!(kinds, [ContactPhase::Enter, ContactPhase::Stay]);
        let alive: HashSet<_> = std::iter::once(ColliderId::from("b")).collect();
        t.retain_colliders(&alive, ExitReason::ColliderRemoved);
        t.end_tick(3, awake);
        assert_eq!(t.deliver(4)[0].reason, Some(ExitReason::ColliderRemoved));
    }

    #[test]
    fn a_reset_drops_queued_events_and_touching() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            scenery("b", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        t.clear();
        assert!(t.deliver(10).is_empty());
        assert!(!t.is_touching("ball", "floor"));
    }

    #[test]
    fn sleeping_pairs_send_no_stay_but_stay_touching() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "ball"),
            scenery("b", "floor"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        t.end_tick(2, |_| false);
        t.end_tick(3, awake);
        assert_eq!(
            phases(&t.deliver(4)),
            [(1, ContactPhase::Enter), (3, ContactPhase::Stay)]
        );
        assert!(t.is_touching("ball", "floor"));
    }

    #[test]
    fn events_within_a_tick_are_ordered_by_pair_then_phase() {
        let mut t = ContactTracker::new();
        t.started(
            end("m", "a1"),
            scenery("n", "f"),
            ContactKind::Collision,
            None,
        );
        t.started(
            end("b", "a2"),
            scenery("c", "f"),
            ContactKind::Trigger,
            None,
        );
        t.end_tick(1, awake);
        let ids: Vec<_> = t
            .deliver(2)
            .iter()
            .map(|e| e.a.collider.as_str().to_string())
            .collect();
        assert_eq!(ids, ["b", "m"]);
    }

    #[test]
    fn shapes_of_one_body_never_pair() {
        let mut t = ContactTracker::new();
        t.started(
            end("a", "car"),
            end("b", "car"),
            ContactKind::Collision,
            None,
        );
        t.end_tick(1, awake);
        assert!(t.deliver(2).is_empty());
        assert!(!t.is_touching("car", "car"));
    }

    #[test]
    fn the_far_side_hears_the_pair_turned_round() {
        let mut t = ContactTracker::new();
        let payload = ContactPayload {
            normal: [0.0, 1.0, 0.0],
            relative_velocity: [0.0, -2.0, 0.0],
            ..Default::default()
        };
        t.started(
            end("a", "ball"),
            scenery("b", "floor"),
            ContactKind::Collision,
            Some(payload),
        );
        t.end_tick(1, awake);
        let event = t.deliver(2).remove(0);
        let floor = event.seen_by("floor").unwrap();
        assert_eq!(floor.a.actor, "floor");
        assert_eq!(floor.payload.unwrap().normal, [0.0, -1.0, 0.0]);
        assert!(event.seen_by("nobody").is_none());
    }

    #[test]
    fn scopes_filter_by_kind() {
        assert!(ContactScope::Any.accepts(ContactKind::Trigger));
        assert!(ContactScope::Collision.accepts(ContactKind::Collision));
        assert!(!ContactScope::Collision.accepts(ContactKind::Trigger));
        assert!(!ContactScope::Trigger.accepts(ContactKind::Collision));
        assert_eq!(ContactScope::parse(""), Some(ContactScope::Any));
    }

    #[test]
    fn words_round_trip() {
        for phase in [ContactPhase::Enter, ContactPhase::Stay, ContactPhase::Exit] {
            assert_eq!(ContactPhase::parse(phase.name()), Some(phase));
        }
        for kind in [ContactKind::Collision, ContactKind::Trigger] {
            assert_eq!(ContactKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ContactPhase::parse("later"), None);
    }

    #[test]
    fn a_compound_wall_is_one_actor_level_enter_and_exit() {
        let mut t = ContactTracker::new();
        for (i, piece) in ["w1", "w2", "w3"].into_iter().enumerate() {
            t.started(
                end("b", "ball"),
                scenery(piece, "wall"),
                ContactKind::Collision,
                None,
            );
            t.end_tick(1 + i as u64, awake);
        }
        let all = t.deliver(10);
        let edges: Vec<_> = all
            .iter()
            .filter(|e| e.phase == ContactPhase::Enter)
            .map(|e| e.edge)
            .collect();
        assert_eq!(
            edges,
            [true, false, false],
            "only the first piece is the edge"
        );
        assert_eq!(t.pairs_between("ball", "wall"), 3);
        for (i, piece) in ["w1", "w2", "w3"].into_iter().enumerate() {
            t.stopped(&ColliderId::from("b"), &ColliderId::from(piece));
            t.end_tick(4 + i as u64, awake);
            assert_eq!(t.is_touching("ball", "wall"), i < 2);
        }
        let exits: Vec<_> = t
            .deliver(10)
            .iter()
            .filter(|e| e.phase == ContactPhase::Exit)
            .map(|e| e.edge)
            .collect();
        assert_eq!(exits, [false, false, true]);
    }

    #[test]
    fn a_stay_is_one_edge_per_actor_pair_each_tick() {
        let mut t = ContactTracker::new();
        for piece in ["w1", "w2"] {
            t.started(
                end("b", "ball"),
                scenery(piece, "wall"),
                ContactKind::Collision,
                None,
            );
        }
        t.end_tick(1, awake);
        t.end_tick(2, awake);
        let stays: Vec<_> = t
            .deliver(3)
            .into_iter()
            .filter(|e| e.phase == ContactPhase::Stay)
            .collect();
        assert_eq!(stays.len(), 2);
        assert_eq!(stays.iter().filter(|e| e.edge).count(), 1);
    }
}
