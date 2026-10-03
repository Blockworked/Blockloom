//! Authoritative physics queries: what a ray, a swept shape, an overlap or a
//! closest-point question finds in the world the simulation runs.
//!
//! Core only owns the vocabulary and the contract. The runtime implements
//! [`QueryService`] over the same Rapier world, shapes and layer rules the
//! simulation uses, installs it for the span of a fixed step ([`with_service`])
//! and every adapter (blocks, reporters, scripts, compiled logic) asks through
//! [`dispatch`]. The answer to a block's query is kept per actor ([`store`]) so
//! reporters read it back without asking a second, different world.
//!
//! Semantics, all of them stated rather than inherited from a backend:
//! - A query sees the world as the previous fixed step left it, plus nothing
//!   queued this tick. It never blocks.
//! - A ray, cast or overlap that starts inside a solid collider reports it at
//!   distance zero with `started_inside` set (the normal opposes the direction).
//! - Hits are sorted by distance, then collider id, then sub-shape, so ties are
//!   stable. A query holds at most `limit` results; more candidates set
//!   `overflow` instead of vanishing.
//! - A zero-length ray asks which colliders contain the point.
//! - A collider with `queryable` off, a disabled one and the asker's own
//!   colliders (and its body's) are never reported.

use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::ColliderId;

/// The most results a query holds unless the caller asks for fewer.
pub const MAX_RESULTS: usize = 64;

/// Whether a query reports trigger colliders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, serde::Deserialize)]
pub enum TriggerPolicy {
    /// What the project says: triggers are reported, as in Unity.
    #[default]
    UseGlobal,
    Ignore,
    Include,
}

impl TriggerPolicy {
    /// Whether triggers are reported.
    pub fn includes_triggers(self) -> bool {
        !matches!(self, TriggerPolicy::Ignore)
    }

    /// The policy a block's dropdown names; anything unknown is the project's.
    pub fn parse(word: &str) -> Self {
        match word.trim().to_ascii_lowercase().as_str() {
            "ignore" | "ignore triggers" | "skip triggers" => Self::Ignore,
            "include" | "include triggers" | "hit triggers" => Self::Include,
            _ => Self::UseGlobal,
        }
    }
}

/// How many hits a ray block keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, serde::Deserialize)]
pub enum RayHits {
    /// Only the nearest.
    #[default]
    Nearest,
    /// Everything on the segment, nearest first.
    Every,
}

impl RayHits {
    pub fn kind(self) -> QueryKind {
        match self {
            RayHits::Nearest => QueryKind::Ray,
            RayHits::Every => QueryKind::Rays,
        }
    }
}

/// What a query may hit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryFilter {
    /// Layers (bit `n - 1` for layer `n`) the query may hit. All by default.
    pub layers: u32,
    /// Ask as this actor: its colliders and its body's are skipped, and the
    /// project's collision matrix (and its overrides) decides which layers it
    /// may hit, exactly as for a contact. `None` asks as nobody.
    pub as_actor: Option<String>,
    pub triggers: TriggerPolicy,
    /// Actors whose colliders (and their bodies') are skipped.
    pub exclude_actors: Vec<String>,
    pub exclude_colliders: Vec<ColliderId>,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self {
            layers: u32::MAX,
            as_actor: None,
            triggers: TriggerPolicy::UseGlobal,
            exclude_actors: Vec::new(),
            exclude_colliders: Vec::new(),
        }
    }
}

impl QueryFilter {
    /// A query made by an actor, with the actor's own rules.
    pub fn as_actor(actor: impl Into<String>) -> Self {
        Self {
            as_actor: Some(actor.into()),
            ..Self::default()
        }
    }
}

/// The shape swept or tested. Positions are world units; in 2D the z of a
/// position and rotation is ignored and `rotation` turns about z only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum QueryShape {
    /// A ball (a circle in 2D).
    Ball { radius: f32 },
    /// A box by half extents, turned by `rotation` (x, y, z, w).
    Box { half: [f32; 3], rotation: [f32; 4] },
    /// A capsule along y by the half length of its segment and its radius.
    Capsule {
        radius: f32,
        half_height: f32,
        rotation: [f32; 4],
    },
}

/// What is asked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum QueryRequest {
    /// Every collider a segment from `from` to `to` crosses, nearest first.
    Ray {
        from: [f32; 3],
        to: [f32; 3],
        /// False asks for the nearest only.
        all: bool,
    },
    /// The first collider `shape` meets sweeping from `from` to `to`.
    Cast {
        shape: QueryShape,
        from: [f32; 3],
        to: [f32; 3],
    },
    /// Every collider `shape` overlaps standing at `at`.
    Overlap { shape: QueryShape, at: [f32; 3] },
    /// The collider nearest `point` within `max_distance`.
    Closest { point: [f32; 3], max_distance: f32 },
}

impl QueryRequest {
    /// The name reports and diagnostics give this request.
    pub fn name(&self) -> &'static str {
        match self {
            QueryRequest::Ray { .. } => "ray",
            QueryRequest::Cast { .. } => "cast",
            QueryRequest::Overlap { .. } => "overlap",
            QueryRequest::Closest { .. } => "closest point",
        }
    }
}

/// One thing a query found, with its full identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryHit {
    pub actor: String,
    /// The actor whose Rigidbody carries the collider; `None` for scenery.
    pub body: Option<String>,
    pub collider: ColliderId,
    /// Which part of a compound, hull set or mesh was hit; 0 for a plain shape.
    pub subshape: u32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// From the start of the query along its direction, in world units.
    pub distance: f32,
    /// `distance` over the query's length; 0 where there is none.
    pub fraction: f32,
    pub started_inside: bool,
    pub trigger: bool,
}

impl QueryHit {
    /// The stable order of results: distance, then collider id, then sub-shape.
    pub fn order(a: &QueryHit, b: &QueryHit) -> std::cmp::Ordering {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| a.collider.cmp(&b.collider))
            .then_with(|| a.subshape.cmp(&b.subshape))
    }
}

/// What a service answered.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct QueryOutcome {
    pub hits: Vec<QueryHit>,
    /// More matched than `limit` allowed; the nearest `limit` are kept.
    pub overflow: bool,
    /// Why nothing could be asked (no world, bad numbers), if so.
    pub error: Option<String>,
}

impl QueryOutcome {
    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            error: Some(why.into()),
            ..Self::default()
        }
    }

    /// Sorts, then keeps the nearest `limit`, setting `overflow` for the rest.
    pub fn finish(mut hits: Vec<QueryHit>, limit: usize) -> Self {
        hits.sort_by(QueryHit::order);
        let overflow = hits.len() > limit;
        hits.truncate(limit);
        Self {
            hits,
            overflow,
            error: None,
        }
    }
}

/// Answers requests against the simulation's world.
pub trait QueryService {
    fn run(&self, request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryOutcome;
}

/// The query kinds a block, script or compiled program can ask by name, with
/// the numbers each one reads. Positions are world units; box and capsule
/// rotations are Euler degrees (x, y, z) like a collider's.
///
/// | kind | numbers |
/// | --- | --- |
/// | `ray`, `rays` | from x y z, to x y z |
/// | `ball cast` | radius, from x y z, to x y z |
/// | `ball overlap` | radius, x y z |
/// | `box cast` | half x y z, rotation x y z, from x y z, to x y z |
/// | `box overlap` | half x y z, rotation x y z, x y z |
/// | `capsule cast` | radius, half height, rotation x y z, from x y z, to x y z |
/// | `capsule overlap` | radius, half height, rotation x y z, x y z |
/// | `closest` | range, x y z |
///
/// `ray` keeps the nearest hit, `rays` every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum QueryKind {
    Ray,
    Rays,
    BallCast,
    BallOverlap,
    BoxCast,
    BoxOverlap,
    CapsuleCast,
    CapsuleOverlap,
    Closest,
}

impl QueryKind {
    pub const ALL: [QueryKind; 9] = [
        QueryKind::Ray,
        QueryKind::Rays,
        QueryKind::BallCast,
        QueryKind::BallOverlap,
        QueryKind::BoxCast,
        QueryKind::BoxOverlap,
        QueryKind::CapsuleCast,
        QueryKind::CapsuleOverlap,
        QueryKind::Closest,
    ];

    pub fn name(self) -> &'static str {
        match self {
            QueryKind::Ray => "ray",
            QueryKind::Rays => "rays",
            QueryKind::BallCast => "ball cast",
            QueryKind::BallOverlap => "ball overlap",
            QueryKind::BoxCast => "box cast",
            QueryKind::BoxOverlap => "box overlap",
            QueryKind::CapsuleCast => "capsule cast",
            QueryKind::CapsuleOverlap => "capsule overlap",
            QueryKind::Closest => "closest",
        }
    }

    pub fn parse(word: &str) -> Option<QueryKind> {
        let word = word.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|kind| kind.name() == word)
    }

    /// How many numbers the kind reads.
    pub fn arity(self) -> usize {
        match self {
            QueryKind::Ray | QueryKind::Rays => 6,
            QueryKind::BallCast => 7,
            QueryKind::BallOverlap | QueryKind::Closest => 4,
            QueryKind::BoxCast => 12,
            QueryKind::BoxOverlap => 9,
            QueryKind::CapsuleCast => 11,
            QueryKind::CapsuleOverlap => 8,
        }
    }
}

fn euler_quat(x: f32, y: f32, z: f32) -> [f32; 4] {
    glam::Quat::from_euler(
        glam::EulerRot::XYZ,
        x.to_radians(),
        y.to_radians(),
        z.to_radians(),
    )
    .to_array()
}

/// The request `numbers` make for `kind`; a wrong count is an error naming the
/// kind, and a number that is not finite is refused by [`check`].
pub fn build_request(kind: QueryKind, numbers: &[f64]) -> Result<QueryRequest, String> {
    if numbers.len() != kind.arity() {
        return Err(format!(
            "a {} query takes {} numbers, not {}",
            kind.name(),
            kind.arity(),
            numbers.len()
        ));
    }
    let n: Vec<f32> = numbers.iter().map(|v| *v as f32).collect();
    let at = |i: usize| [n[i], n[i + 1], n[i + 2]];
    let request = match kind {
        QueryKind::Ray | QueryKind::Rays => QueryRequest::Ray {
            from: at(0),
            to: at(3),
            all: kind == QueryKind::Rays,
        },
        QueryKind::BallCast => QueryRequest::Cast {
            shape: QueryShape::Ball { radius: n[0] },
            from: at(1),
            to: at(4),
        },
        QueryKind::BallOverlap => QueryRequest::Overlap {
            shape: QueryShape::Ball { radius: n[0] },
            at: at(1),
        },
        QueryKind::BoxCast => QueryRequest::Cast {
            shape: QueryShape::Box {
                half: at(0),
                rotation: euler_quat(n[3], n[4], n[5]),
            },
            from: at(6),
            to: at(9),
        },
        QueryKind::BoxOverlap => QueryRequest::Overlap {
            shape: QueryShape::Box {
                half: at(0),
                rotation: euler_quat(n[3], n[4], n[5]),
            },
            at: at(6),
        },
        QueryKind::CapsuleCast => QueryRequest::Cast {
            shape: QueryShape::Capsule {
                radius: n[0],
                half_height: n[1],
                rotation: euler_quat(n[2], n[3], n[4]),
            },
            from: at(5),
            to: at(8),
        },
        QueryKind::CapsuleOverlap => QueryRequest::Overlap {
            shape: QueryShape::Capsule {
                radius: n[0],
                half_height: n[1],
                rotation: euler_quat(n[2], n[3], n[4]),
            },
            at: at(5),
        },
        QueryKind::Closest => QueryRequest::Closest {
            point: at(1),
            max_distance: n[0],
        },
    };
    check(&request)?;
    Ok(request)
}

/// A request that cannot be asked, whatever the world holds.
pub fn check(request: &QueryRequest) -> Result<(), String> {
    let finite = |v: &[f32]| v.iter().all(|n| n.is_finite());
    let shape_ok = |shape: &QueryShape| match shape {
        QueryShape::Ball { radius } => radius.is_finite() && *radius >= 0.0,
        QueryShape::Box { half, rotation } => {
            finite(half) && half.iter().all(|h| *h >= 0.0) && finite(rotation)
        }
        QueryShape::Capsule {
            radius,
            half_height,
            rotation,
        } => {
            radius.is_finite()
                && *radius >= 0.0
                && half_height.is_finite()
                && *half_height >= 0.0
                && finite(rotation)
        }
    };
    let ok = match request {
        QueryRequest::Ray { from, to, .. } => finite(from) && finite(to),
        QueryRequest::Cast { shape, from, to } => shape_ok(shape) && finite(from) && finite(to),
        QueryRequest::Overlap { shape, at } => shape_ok(shape) && finite(at),
        QueryRequest::Closest {
            point,
            max_distance,
        } => finite(point) && max_distance.is_finite() && *max_distance >= 0.0,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "a {} query was given a number that is not finite or is negative where it must not be",
            request.name()
        ))
    }
}

thread_local! {
    /// The service the world installed for the fixed step in progress, and the
    /// tick it describes. A raw pointer only because the service borrows the
    /// ECS for the step; [`with_service`] clears it before the borrow ends.
    static SERVICE: Cell<Option<(*const dyn QueryService, u64)>> = const { Cell::new(None) };

    /// What each actor's last block query found.
    static RECORDS: RefCell<HashMap<String, QueryRecord>> = RefCell::new(HashMap::new());
}

/// Runs `f` with `service` answering every [`dispatch`] on this thread, as the
/// world stood at `tick`. Nested scopes restore the outer one on the way out.
pub fn with_service<R>(service: &dyn QueryService, tick: u64, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<(*const dyn QueryService, u64)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SERVICE.with(|slot| slot.set(self.0));
        }
    }
    // SAFETY: the pointer is only read while `f` runs, `Restore` puts the
    // previous value back even on unwind, and the service outlives this call.
    let erased: *const dyn QueryService =
        unsafe { std::mem::transmute::<&dyn QueryService, &'static dyn QueryService>(service) };
    let _restore = Restore(SERVICE.with(|slot| slot.replace(Some((erased, tick)))));
    f()
}

/// Whether a service is installed on this thread.
pub fn available() -> bool {
    SERVICE.with(|slot| slot.get().is_some())
}

/// The tick the installed service describes.
pub fn current_tick() -> Option<u64> {
    SERVICE.with(|slot| slot.get().map(|(_, tick)| tick))
}

/// Asks the installed service. Without one (a reporter previewed in the
/// editor, a world not yet built) the answer carries the reason.
pub fn dispatch(request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryOutcome {
    if let Err(why) = check(request) {
        return QueryOutcome::failed(why);
    }
    let limit = limit.clamp(1, MAX_RESULTS);
    SERVICE.with(|slot| match slot.get() {
        // SAFETY: see `with_service`; the scope that set it is still running.
        Some((service, _)) => unsafe { (*service).run(request, filter, limit) },
        None => QueryOutcome::failed("physics queries only answer while the game is running"),
    })
}

/// What one actor's last query found, as its reporters read it.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct QueryRecord {
    pub kind: String,
    /// The fixed tick the answer describes.
    pub tick: u64,
    pub hits: Vec<QueryHit>,
    pub overflow: bool,
    pub error: Option<String>,
}

/// Asks, files the answer under `actor` for its reporters and returns it.
pub fn ask(actor: &str, request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryRecord {
    let outcome = dispatch(request, filter, limit);
    let record = QueryRecord {
        kind: request.name().to_string(),
        tick: current_tick().unwrap_or(0),
        hits: outcome.hits,
        overflow: outcome.overflow,
        error: outcome.error,
    };
    store(actor, record.clone());
    record
}

/// The query a block, script or compiled program asked by name: builds the
/// request from `numbers`, asks as `actor` (so its own colliders are skipped
/// and its layer rules apply) and files the answer under it. `layers` of zero
/// means every layer. A request that cannot be built is filed as an error.
pub fn ask_call(
    actor: &str,
    kind: QueryKind,
    triggers: TriggerPolicy,
    layers: u32,
    numbers: &[f64],
) -> QueryRecord {
    let filter = QueryFilter {
        layers: if layers == 0 { u32::MAX } else { layers },
        triggers,
        ..QueryFilter::as_actor(actor)
    };
    match build_request(kind, numbers) {
        Ok(request) => ask(actor, &request, &filter, MAX_RESULTS),
        Err(why) => {
            let record = QueryRecord {
                kind: kind.name().to_string(),
                tick: current_tick().unwrap_or(0),
                error: Some(why),
                ..QueryRecord::default()
            };
            store(actor, record.clone());
            record
        }
    }
}

/// Files `record` as the actor's last query.
pub fn store(actor: &str, record: QueryRecord) {
    RECORDS.with(|records| {
        records.borrow_mut().insert(actor.to_string(), record);
    });
}

/// The actor's last query, if it has made one this run.
pub fn record_of<R>(actor: &str, f: impl FnOnce(Option<&QueryRecord>) -> R) -> R {
    RECORDS.with(|records| f(records.borrow().get(actor)))
}

/// Forgets every stored answer: a rebuilt or reset world starts clean.
pub fn reset() {
    RECORDS.with(|records| records.borrow_mut().clear());
}

/// A field of a stored query a reporter can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitField {
    Count,
    Overflow,
    Tick,
    Error,
    Actor,
    ActorId,
    Body,
    Collider,
    Subshape,
    X,
    Y,
    Z,
    NormalX,
    NormalY,
    NormalZ,
    Distance,
    Fraction,
    StartedInside,
    Trigger,
}

impl HitField {
    pub const ALL: [(HitField, &'static str); 19] = [
        (HitField::Count, "count"),
        (HitField::Overflow, "overflowed"),
        (HitField::Tick, "tick"),
        (HitField::Error, "error"),
        (HitField::Actor, "actor"),
        (HitField::ActorId, "actor id"),
        (HitField::Body, "body"),
        (HitField::Collider, "collider"),
        (HitField::Subshape, "part"),
        (HitField::X, "x"),
        (HitField::Y, "y"),
        (HitField::Z, "z"),
        (HitField::NormalX, "normal x"),
        (HitField::NormalY, "normal y"),
        (HitField::NormalZ, "normal z"),
        (HitField::Distance, "distance"),
        (HitField::Fraction, "fraction"),
        (HitField::StartedInside, "started inside"),
        (HitField::Trigger, "is trigger"),
    ];

    pub fn parse(word: &str) -> Option<HitField> {
        let word = word.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .find(|(_, name)| *name == word)
            .map(|(field, _)| *field)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(field, _)| *field == self)
            .map(|(_, name)| *name)
            .unwrap_or("count")
    }

    /// Whether the field is words (the rest are numbers, booleans as 0 or 1).
    pub fn is_text(self) -> bool {
        matches!(
            self,
            HitField::Error
                | HitField::Actor
                | HitField::ActorId
                | HitField::Body
                | HitField::Collider
        )
    }
}

/// A reporter's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum HitValue {
    Number(f64),
    Text(String),
}

/// Reads one field of the `index`th (from 1) hit of the actor's last query.
/// `name_of` turns an actor id into the name blocks use. A missing hit reads
/// as zero or empty, never as an error: a miss is an ordinary answer.
pub fn read_field(
    actor: &str,
    index: usize,
    field: HitField,
    name_of: impl Fn(&str) -> String,
) -> HitValue {
    record_of(actor, |record| {
        let Some(record) = record else {
            return if field.is_text() {
                HitValue::Text(String::new())
            } else {
                HitValue::Number(0.0)
            };
        };
        let flag = |on: bool| HitValue::Number(if on { 1.0 } else { 0.0 });
        match field {
            HitField::Count => return HitValue::Number(record.hits.len() as f64),
            HitField::Overflow => return flag(record.overflow),
            HitField::Tick => return HitValue::Number(record.tick as f64),
            HitField::Error => return HitValue::Text(record.error.clone().unwrap_or_default()),
            _ => {}
        }
        let hit = index.checked_sub(1).and_then(|i| record.hits.get(i));
        let Some(hit) = hit else {
            return if field.is_text() {
                HitValue::Text(String::new())
            } else {
                HitValue::Number(0.0)
            };
        };
        match field {
            HitField::Actor => HitValue::Text(name_of(&hit.actor)),
            HitField::ActorId => HitValue::Text(hit.actor.clone()),
            HitField::Body => HitValue::Text(hit.body.clone().unwrap_or_default()),
            HitField::Collider => HitValue::Text(hit.collider.to_string()),
            HitField::Subshape => HitValue::Number(hit.subshape as f64),
            HitField::X => HitValue::Number(hit.point[0] as f64),
            HitField::Y => HitValue::Number(hit.point[1] as f64),
            HitField::Z => HitValue::Number(hit.point[2] as f64),
            HitField::NormalX => HitValue::Number(hit.normal[0] as f64),
            HitField::NormalY => HitValue::Number(hit.normal[1] as f64),
            HitField::NormalZ => HitValue::Number(hit.normal[2] as f64),
            HitField::Distance => HitValue::Number(hit.distance as f64),
            HitField::Fraction => HitValue::Number(hit.fraction as f64),
            HitField::StartedInside => flag(hit.started_inside),
            HitField::Trigger => flag(hit.trigger),
            _ => unreachable!("handled above"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<QueryHit>);

    impl QueryService for Fixed {
        fn run(&self, _: &QueryRequest, _: &QueryFilter, limit: usize) -> QueryOutcome {
            QueryOutcome::finish(self.0.clone(), limit)
        }
    }

    fn hit(collider: &str, distance: f32) -> QueryHit {
        QueryHit {
            actor: format!("actor-{collider}"),
            body: None,
            collider: collider.into(),
            subshape: 0,
            point: [distance, 0.0, 0.0],
            normal: [-1.0, 0.0, 0.0],
            distance,
            fraction: distance / 10.0,
            started_inside: false,
            trigger: false,
        }
    }

    fn ray() -> QueryRequest {
        QueryRequest::Ray {
            from: [0.0; 3],
            to: [10.0, 0.0, 0.0],
            all: true,
        }
    }

    #[test]
    fn results_sort_by_distance_then_collider_and_overflow_is_reported() {
        let service = Fixed(vec![
            hit("c", 3.0),
            hit("b", 3.0),
            hit("a", 5.0),
            hit("z", 1.0),
        ]);
        let found = with_service(&service, 7, || dispatch(&ray(), &QueryFilter::default(), 3));
        let ids: Vec<_> = found.hits.iter().map(|h| h.collider.as_str()).collect();
        assert_eq!(ids, ["z", "b", "c"]);
        assert!(found.overflow);
        assert!(found.error.is_none());
    }

    #[test]
    fn nothing_answers_outside_a_scope_and_the_scope_restores_on_exit() {
        let outcome = dispatch(&ray(), &QueryFilter::default(), 4);
        assert!(outcome.error.unwrap().contains("running"));
        assert!(!available());
        let service = Fixed(vec![hit("a", 1.0)]);
        with_service(&service, 3, || {
            assert_eq!(current_tick(), Some(3));
            let inner = Fixed(vec![]);
            with_service(&inner, 4, || {
                assert!(dispatch(&ray(), &QueryFilter::default(), 4).hits.is_empty());
            });
            assert_eq!(current_tick(), Some(3));
            assert_eq!(dispatch(&ray(), &QueryFilter::default(), 4).hits.len(), 1);
        });
        assert!(!available());
    }

    #[test]
    fn a_panic_inside_a_scope_still_clears_it() {
        let service = Fixed(vec![]);
        let _ = std::panic::catch_unwind(|| with_service(&service, 1, || panic!("boom")));
        assert!(!available());
    }

    #[test]
    fn requests_with_bad_numbers_are_refused_before_the_world_sees_them() {
        let service = Fixed(vec![hit("a", 1.0)]);
        let bad = QueryRequest::Overlap {
            shape: QueryShape::Ball { radius: -1.0 },
            at: [0.0; 3],
        };
        let outcome = with_service(&service, 1, || dispatch(&bad, &QueryFilter::default(), 4));
        assert!(outcome.hits.is_empty());
        assert!(outcome.error.unwrap().contains("overlap"));
        let nan = QueryRequest::Ray {
            from: [f32::NAN, 0.0, 0.0],
            to: [1.0, 0.0, 0.0],
            all: false,
        };
        assert!(check(&nan).is_err());
    }

    #[test]
    fn an_actors_last_answer_is_read_back_field_by_field() {
        reset();
        let service = Fixed(vec![hit("a", 2.0), hit("b", 4.0)]);
        let record = with_service(&service, 9, || {
            ask("me", &ray(), &QueryFilter::as_actor("me"), 8)
        });
        assert_eq!(record.hits.len(), 2);
        let name = |id: &str| id.replace("actor-", "Name ");
        let read = |index, field| read_field("me", index, field, name);
        assert_eq!(read(0, HitField::Count), HitValue::Number(2.0));
        assert_eq!(read(1, HitField::Actor), HitValue::Text("Name a".into()));
        assert_eq!(read(2, HitField::Distance), HitValue::Number(4.0));
        assert_eq!(read(2, HitField::Tick), HitValue::Number(9.0));
        assert_eq!(read(3, HitField::Distance), HitValue::Number(0.0));
        assert_eq!(read(3, HitField::Collider), HitValue::Text(String::new()));
        assert_eq!(
            read_field("nobody", 1, HitField::Count, name),
            HitValue::Number(0.0)
        );
        reset();
        assert_eq!(read(1, HitField::Count), HitValue::Number(0.0));
    }

    #[test]
    fn fields_and_trigger_words_parse() {
        assert_eq!(HitField::parse("Normal X"), Some(HitField::NormalX));
        assert_eq!(HitField::parse("nope"), None);
        for (field, name) in HitField::ALL {
            assert_eq!(field.name(), name);
        }
        assert_eq!(TriggerPolicy::parse("ignore"), TriggerPolicy::Ignore);
        assert_eq!(TriggerPolicy::parse("include"), TriggerPolicy::Include);
        assert_eq!(TriggerPolicy::parse(""), TriggerPolicy::UseGlobal);
    }

    #[test]
    fn named_kinds_build_requests_from_their_numbers() {
        for kind in QueryKind::ALL {
            assert_eq!(QueryKind::parse(kind.name()), Some(kind));
            let zeros = vec![0.0; kind.arity()];
            assert!(build_request(kind, &zeros).is_ok(), "{}", kind.name());
            assert!(build_request(kind, &zeros[1..]).is_err());
        }
        let request = build_request(QueryKind::BallCast, &[0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(
            request,
            Ok(QueryRequest::Cast {
                shape: QueryShape::Ball { radius: 0.5 },
                from: [1.0, 2.0, 3.0],
                to: [4.0, 5.0, 6.0],
            })
        );
        let Ok(QueryRequest::Overlap {
            shape: QueryShape::Box { half, rotation },
            at,
        }) = build_request(
            QueryKind::BoxOverlap,
            &[1.0, 2.0, 3.0, 0.0, 90.0, 0.0, 7.0, 8.0, 9.0],
        )
        else {
            panic!("a box overlap");
        };
        assert_eq!((half, at), ([1.0, 2.0, 3.0], [7.0, 8.0, 9.0]));
        // A quarter turn about y is (0, sin 45, 0, cos 45).
        assert!((rotation[1] - 0.70710677).abs() < 1e-5 && (rotation[3] - 0.70710677).abs() < 1e-5);
        assert!(build_request(QueryKind::Ray, &[0.0, 0.0, 0.0, f64::NAN, 0.0, 0.0]).is_err());
    }

    #[test]
    fn a_named_query_asks_as_the_actor_and_files_even_its_failures() {
        reset();
        struct Echo;
        impl QueryService for Echo {
            fn run(&self, _: &QueryRequest, filter: &QueryFilter, _: usize) -> QueryOutcome {
                assert_eq!(filter.as_actor.as_deref(), Some("me"));
                assert_eq!(filter.layers, 0b10);
                assert_eq!(filter.triggers, TriggerPolicy::Ignore);
                QueryOutcome::finish(vec![hit("a", 1.0)], 8)
            }
        }
        let found = with_service(&Echo, 5, || {
            ask_call(
                "me",
                QueryKind::Ray,
                TriggerPolicy::Ignore,
                0b10,
                &[0.0, 0.0, 0.0, 10.0, 0.0, 0.0],
            )
        });
        assert_eq!((found.hits.len(), found.tick), (1, 5));
        let bad = ask_call("me", QueryKind::Ray, TriggerPolicy::UseGlobal, 0, &[1.0]);
        assert!(bad.error.unwrap().contains("6 numbers"));
        assert!(record_of("me", |r| r.unwrap().hits.is_empty()));
        reset();
    }
}
