//! Authoritative physics queries over the Rapier world the simulation runs.
//!
//! The service borrows the physics context for one system and is installed with
//! [`blockloom_core::physics::query::with_service`], so a block, a reporter, a
//! script or compiled logic asks the same world with the same shapes, layer
//! matrix and trigger rules a contact would use. Identity (actor, body,
//! collider id) comes from [`PlannedCollider`]; a legacy collider with none
//! reports its actor and a `<actor>#body` id.

use crate::engine::ActorId;
use crate::physics_install::{PhysicsLayers, PlannedCollider};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::physics::query::{
    QueryFilter, QueryHit, QueryOutcome, QueryRequest, QueryService, QueryShape,
};
use blockloom_core::physics::{ColliderFilter, ColliderId, layer_bit, pair_collides};

/// Who a collider entity is, as far as queries are concerned.
pub(crate) struct Ident {
    pub(crate) actor: String,
    pub(crate) body: Option<String>,
    pub(crate) collider: ColliderId,
    pub(crate) trigger: bool,
    pub(crate) filter: Option<ColliderFilter>,
}

/// What every dimension looks collider entities up with.
#[derive(SystemParam)]
pub struct Lookups<'w, 's> {
    planned: Query<'w, 's, &'static PlannedCollider>,
    actors: Query<'w, 's, &'static ActorId>,
    parents: Query<'w, 's, &'static ChildOf>,
    layers: Option<Res<'w, PhysicsLayers>>,
}

impl Lookups<'_, '_> {
    pub(crate) fn ident(&self, entity: Entity, sensor: bool, carried: bool) -> Ident {
        if let Ok(planned) = self.planned.get(entity) {
            return Ident {
                actor: planned.actor.clone(),
                body: planned.body.clone(),
                collider: planned.id.clone(),
                trigger: planned.trigger || sensor,
                filter: Some(planned.filter),
            };
        }
        // Not installed from a plan: the nearest actor above it owns it.
        let mut at = entity;
        let owner = loop {
            if let Ok(id) = self.actors.get(at) {
                break id.0.clone();
            }
            match self.parents.get(at) {
                Ok(parent) => at = parent.parent(),
                Err(_) => break String::new(),
            }
        };
        Ident {
            collider: ColliderId::from(format!("{owner}#body").as_str()),
            body: (carried && !owner.is_empty()).then(|| owner.clone()),
            actor: owner,
            trigger: sensor,
            filter: None,
        }
    }

    /// The asker's own filter: its first collider's, or the default layer's.
    fn asker(&self, filter: &QueryFilter) -> Option<ColliderFilter> {
        let actor = filter.as_actor.as_ref()?;
        Some(
            self.layers
                .as_ref()
                .and_then(|layers| layers.askers.get(actor).copied())
                .unwrap_or(ColliderFilter {
                    layer: 1,
                    overrides: Default::default(),
                    legacy_mask: None,
                }),
        )
    }

    /// Whether `filter` lets a collider be found.
    pub(crate) fn admits(&self, ident: &Ident, queryable: bool, filter: &QueryFilter) -> bool {
        if !queryable
            || (ident.trigger && !filter.triggers.includes_triggers())
            || filter.exclude_colliders.contains(&ident.collider)
        {
            return false;
        }
        let named =
            |actor: &String| ident.actor == *actor || ident.body.as_deref() == Some(actor.as_str());
        if filter.as_actor.as_ref().is_some_and(named) || filter.exclude_actors.iter().any(named) {
            return false;
        }
        let layer = ident.filter.map_or(1, |f| f.layer);
        if filter.layers & layer_bit(layer) == 0 {
            return false;
        }
        match (self.asker(filter), ident.filter, self.layers.as_ref()) {
            (Some(asker), Some(target), Some(layers)) => match layers.mode {
                Some(mode) => pair_collides(&asker, &target, &layers.settings, mode),
                None => true,
            },
            _ => true,
        }
    }
}

/// Both dimensions' query access in one system parameter, so a system that
/// runs blocks, scripts or plugin hooks can open a query scope without being
/// generic over the live dimension.
#[derive(SystemParam)]
pub struct QueryAccess<'w, 's> {
    two: d2::World2<'w, 's>,
    three: d3::World3<'w, 's>,
    layers: Option<Res<'w, PhysicsLayers>>,
    controllers: crate::controller::ControllerAccess<'w, 's>,
}

impl QueryAccess<'_, '_> {
    /// Runs `f` with queries answering for the world as it stood after `tick`.
    /// With no physics world built (a project that has not started) queries
    /// report that instead of an empty world.
    pub fn scope<R>(&self, tick: u64, f: impl FnOnce() -> R) -> R {
        use blockloom_core::physics::query::with_service;
        use blockloom_core::scene::Mode;
        match self.layers.as_ref().and_then(|layers| layers.mode) {
            Some(Mode::TwoD) => with_service(&self.two.service(), tick, || {
                self.controllers.scope(tick, f)
            }),
            Some(Mode::ThreeD) => with_service(&self.three.service(), tick, || {
                self.controllers.scope(tick, f)
            }),
            None => f(),
        }
    }
}

/// Splits a segment into its unit direction and length; `None` when it is a
/// point.
fn segment(from: [f32; 3], to: [f32; 3]) -> Option<([f32; 3], f32)> {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (length > 1e-6).then(|| ([d[0] / length, d[1] / length, d[2] / length], length))
}

pub mod d3 {
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use bevy_rapier3d::rapier::parry::query::ShapeCastOptions;
    use bevy_rapier3d::rapier::parry::shape::{Ball, Capsule, Cuboid};

    /// The 3D world's query access, as a system parameter.
    #[derive(SystemParam)]
    pub struct World3<'w, 's> {
        context: rp::ReadRapierContext<'w, 's>,
        lookups: Lookups<'w, 's>,
    }

    impl World3<'_, '_> {
        pub fn service(&self) -> View3<'_, '_, '_> {
            View3(self)
        }
    }

    pub struct View3<'a, 'w, 's>(&'a World3<'w, 's>);

    fn vec(v: [f32; 3]) -> Vec3 {
        Vec3::from(v)
    }

    fn quat(q: [f32; 4]) -> Quat {
        let q = Quat::from_array(q);
        if q.is_finite() && q.length_squared() > 1e-12 {
            q.normalize()
        } else {
            Quat::IDENTITY
        }
    }

    enum Cast {
        Ball(Ball),
        Box(Cuboid),
        Capsule(Capsule),
    }

    fn shape_of(shape: &QueryShape) -> (Cast, Quat) {
        match shape {
            QueryShape::Ball { radius } => (Cast::Ball(Ball::new(*radius)), Quat::IDENTITY),
            QueryShape::Box { half, rotation } => {
                (Cast::Box(Cuboid::new(vec(*half))), quat(*rotation))
            }
            QueryShape::Capsule {
                radius,
                half_height,
                rotation,
            } => (
                Cast::Capsule(Capsule::new_y(*half_height, *radius)),
                quat(*rotation),
            ),
        }
    }

    impl QueryService for View3<'_, '_, '_> {
        fn run(&self, request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryOutcome {
            let world = self.0;
            let Ok(context) = world.context.single() else {
                return QueryOutcome::failed("the physics world is not built yet");
            };
            let lookups = &world.lookups;
            let predicate =
                |entity: Entity, raw: &bevy_rapier3d::rapier::geometry::Collider| -> bool {
                    raw.is_enabled() && {
                        let ident = lookups.ident(entity, raw.is_sensor(), raw.parent().is_some());
                        let queryable = lookups
                            .planned
                            .get(entity)
                            .map_or(true, |planned| planned.queryable);
                        lookups.admits(&ident, queryable, filter)
                    }
                };
            let query_filter = rp::QueryFilter::default().predicate(&predicate);
            let hit_of = |entity: Entity,
                          sensor: bool,
                          carried: bool,
                          subshape: u32,
                          point: Vec3,
                          normal: Vec3,
                          distance: f32,
                          length: f32,
                          inside: bool| {
                let ident = lookups.ident(entity, sensor, carried);
                QueryHit {
                    actor: ident.actor,
                    body: ident.body,
                    collider: ident.collider,
                    subshape,
                    point: point.to_array(),
                    normal: normal.to_array(),
                    distance,
                    fraction: if length > 0.0 { distance / length } else { 0.0 },
                    started_inside: inside,
                    trigger: ident.trigger,
                }
            };
            let collider_flags = |entity: Entity| -> (bool, bool) {
                context
                    .colliders
                    .entity2collider()
                    .get(&entity)
                    .and_then(|handle| context.colliders.colliders.get(*handle))
                    .map_or((false, false), |c| (c.is_sensor(), c.parent().is_some()))
            };
            context.with_query_pipeline(query_filter, |pipeline| match request {
                QueryRequest::Ray { from, to, all } => {
                    let origin = vec(*from);
                    let Some((dir, length)) = segment(*from, *to) else {
                        let hits = pipeline
                            .intersect_point(origin)
                            .map(|(entity, collider)| {
                                hit_of(
                                    entity,
                                    collider.is_sensor(),
                                    collider.parent().is_some(),
                                    0,
                                    origin,
                                    Vec3::ZERO,
                                    0.0,
                                    0.0,
                                    true,
                                )
                            })
                            .collect();
                        return QueryOutcome::finish(hits, limit);
                    };
                    let dir_v = vec(dir);
                    let make = |entity: Entity,
                                collider: Option<&bevy_rapier3d::rapier::geometry::Collider>,
                                hit: &rp::RayIntersection| {
                        let (sensor, carried) = match collider {
                            Some(c) => (c.is_sensor(), c.parent().is_some()),
                            None => collider_flags(entity),
                        };
                        let inside = hit.time_of_impact <= 1e-6;
                        let normal = if inside || hit.normal.length_squared() < 1e-12 {
                            -dir_v
                        } else {
                            hit.normal
                        };
                        hit_of(
                            entity,
                            sensor,
                            carried,
                            hit.subshape,
                            hit.point,
                            normal,
                            hit.time_of_impact,
                            length,
                            inside,
                        )
                    };
                    if *all {
                        let hits = pipeline
                            .intersect_ray(origin, dir_v, length, true)
                            .map(|(entity, collider, hit)| make(entity, Some(collider), &hit))
                            .collect();
                        QueryOutcome::finish(hits, limit)
                    } else {
                        let hits = pipeline
                            .cast_ray_and_get_normal(origin, dir_v, length, true)
                            .map(|(entity, hit)| make(entity, None, &hit))
                            .into_iter()
                            .collect();
                        QueryOutcome::finish(hits, limit)
                    }
                }
                QueryRequest::Cast { shape, from, to } => {
                    let (cast, rotation) = shape_of(shape);
                    let origin = vec(*from);
                    let Some((dir, length)) = segment(*from, *to) else {
                        // No travel: it is the overlap at the start, nearest first.
                        return overlap_at(&pipeline, &cast, origin, rotation, limit, &|e, c| {
                            hit_of(
                                e,
                                c.is_sensor(),
                                c.parent().is_some(),
                                0,
                                c.position().translation,
                                Vec3::ZERO,
                                0.0,
                                0.0,
                                true,
                            )
                        });
                    };
                    let options = ShapeCastOptions {
                        max_time_of_impact: length,
                        target_distance: 0.0,
                        stop_at_penetration: true,
                        compute_impact_geometry_on_penetration: true,
                    };
                    let dir_v = vec(dir);
                    let found = match &cast {
                        Cast::Ball(s) => pipeline.cast_shape(origin, rotation, dir_v, s, options),
                        Cast::Box(s) => pipeline.cast_shape(origin, rotation, dir_v, s, options),
                        Cast::Capsule(s) => {
                            pipeline.cast_shape(origin, rotation, dir_v, s, options)
                        }
                    };
                    let hits = found
                        .map(|(entity, hit)| {
                            let (sensor, carried) = collider_flags(entity);
                            let inside = hit.time_of_impact <= 1e-6;
                            let details = hit.details;
                            let point = details.map_or(origin, |d| d.witness1);
                            let normal = match details {
                                Some(d) if !inside => d.normal1,
                                _ => -dir_v,
                            };
                            hit_of(
                                entity,
                                sensor,
                                carried,
                                hit.subshape1,
                                point,
                                normal,
                                hit.time_of_impact,
                                length,
                                inside,
                            )
                        })
                        .into_iter()
                        .collect();
                    QueryOutcome::finish(hits, limit)
                }
                QueryRequest::Overlap { shape, at } => {
                    let (cast, rotation) = shape_of(shape);
                    overlap_at(&pipeline, &cast, vec(*at), rotation, limit, &|e, c| {
                        hit_of(
                            e,
                            c.is_sensor(),
                            c.parent().is_some(),
                            0,
                            c.position().translation,
                            Vec3::ZERO,
                            0.0,
                            0.0,
                            true,
                        )
                    })
                }
                QueryRequest::Closest {
                    point,
                    max_distance,
                } => {
                    let origin = vec(*point);
                    let hits = pipeline
                        .project_point(origin, *max_distance, true)
                        .map(|(entity, projection)| {
                            let (sensor, carried) = collider_flags(entity);
                            let distance = if projection.is_inside {
                                0.0
                            } else {
                                origin.distance(projection.point)
                            };
                            let normal = if projection.is_inside || distance <= 1e-9 {
                                Vec3::ZERO
                            } else {
                                (origin - projection.point) / distance
                            };
                            hit_of(
                                entity,
                                sensor,
                                carried,
                                projection.subshape,
                                projection.point,
                                normal,
                                distance,
                                *max_distance,
                                projection.is_inside,
                            )
                        })
                        .into_iter()
                        .filter(|hit| hit.distance <= *max_distance)
                        .collect();
                    QueryOutcome::finish(hits, limit)
                }
            })
        }
    }

    fn overlap_at(
        pipeline: &rp::RapierQueryPipeline<'_>,
        cast: &Cast,
        at: Vec3,
        rotation: Quat,
        limit: usize,
        make: &dyn Fn(Entity, &bevy_rapier3d::rapier::geometry::Collider) -> QueryHit,
    ) -> QueryOutcome {
        let hits: Vec<QueryHit> = match cast {
            Cast::Ball(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
            Cast::Box(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
            Cast::Capsule(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
        };
        QueryOutcome::finish(hits, limit)
    }
}

pub mod d2 {
    use super::*;
    use bevy_rapier2d::prelude as rp;
    use bevy_rapier2d::rapier::parry::query::ShapeCastOptions;
    use bevy_rapier2d::rapier::parry::shape::{Ball, Capsule, Cuboid};

    /// The 3D world's query access, as a system parameter.
    #[derive(SystemParam)]
    pub struct World2<'w, 's> {
        context: rp::ReadRapierContext<'w, 's>,
        lookups: Lookups<'w, 's>,
    }

    impl World2<'_, '_> {
        pub fn service(&self) -> View2<'_, '_, '_> {
            View2(self)
        }
    }

    pub struct View2<'a, 'w, 's>(&'a World2<'w, 's>);

    fn vec(v: [f32; 3]) -> Vec2 {
        Vec2::new(v[0], v[1])
    }

    fn planar(v: [f32; 3]) -> [f32; 3] {
        [v[0], v[1], 0.0]
    }

    /// The turn about z a quaternion makes, in radians.
    fn quat(q: [f32; 4]) -> f32 {
        let q = Quat::from_array(q);
        if q.is_finite() && q.length_squared() > 1e-12 {
            let q = q.normalize();
            2.0 * q.z.atan2(q.w)
        } else {
            0.0
        }
    }

    enum Cast {
        Ball(Ball),
        Box(Cuboid),
        Capsule(Capsule),
    }

    fn shape_of(shape: &QueryShape) -> (Cast, f32) {
        match shape {
            QueryShape::Ball { radius } => (Cast::Ball(Ball::new(*radius)), 0.0),
            QueryShape::Box { half, rotation } => {
                (Cast::Box(Cuboid::new(vec(*half))), quat(*rotation))
            }
            QueryShape::Capsule {
                radius,
                half_height,
                rotation,
            } => (
                Cast::Capsule(Capsule::new_y(*half_height, *radius)),
                quat(*rotation),
            ),
        }
    }

    impl QueryService for View2<'_, '_, '_> {
        fn run(&self, request: &QueryRequest, filter: &QueryFilter, limit: usize) -> QueryOutcome {
            let world = self.0;
            let Ok(context) = world.context.single() else {
                return QueryOutcome::failed("the physics world is not built yet");
            };
            let lookups = &world.lookups;
            let predicate =
                |entity: Entity, raw: &bevy_rapier2d::rapier::geometry::Collider| -> bool {
                    raw.is_enabled() && {
                        let ident = lookups.ident(entity, raw.is_sensor(), raw.parent().is_some());
                        let queryable = lookups
                            .planned
                            .get(entity)
                            .map_or(true, |planned| planned.queryable);
                        lookups.admits(&ident, queryable, filter)
                    }
                };
            let query_filter = rp::QueryFilter::default().predicate(&predicate);
            let hit_of = |entity: Entity,
                          sensor: bool,
                          carried: bool,
                          subshape: u32,
                          point: Vec2,
                          normal: Vec2,
                          distance: f32,
                          length: f32,
                          inside: bool| {
                let ident = lookups.ident(entity, sensor, carried);
                QueryHit {
                    actor: ident.actor,
                    body: ident.body,
                    collider: ident.collider,
                    subshape,
                    point: [point.x, point.y, 0.0],
                    normal: [normal.x, normal.y, 0.0],
                    distance,
                    fraction: if length > 0.0 { distance / length } else { 0.0 },
                    started_inside: inside,
                    trigger: ident.trigger,
                }
            };
            let collider_flags = |entity: Entity| -> (bool, bool) {
                context
                    .colliders
                    .entity2collider()
                    .get(&entity)
                    .and_then(|handle| context.colliders.colliders.get(*handle))
                    .map_or((false, false), |c| (c.is_sensor(), c.parent().is_some()))
            };
            context.with_query_pipeline(query_filter, |pipeline| match request {
                QueryRequest::Ray { from, to, all } => {
                    let origin = vec(*from);
                    let Some((dir, length)) = segment(planar(*from), planar(*to)) else {
                        let hits = pipeline
                            .intersect_point(origin)
                            .map(|(entity, collider)| {
                                hit_of(
                                    entity,
                                    collider.is_sensor(),
                                    collider.parent().is_some(),
                                    0,
                                    origin,
                                    Vec2::ZERO,
                                    0.0,
                                    0.0,
                                    true,
                                )
                            })
                            .collect();
                        return QueryOutcome::finish(hits, limit);
                    };
                    let dir_v = vec(dir);
                    let make = |entity: Entity,
                                collider: Option<&bevy_rapier2d::rapier::geometry::Collider>,
                                hit: &rp::RayIntersection| {
                        let (sensor, carried) = match collider {
                            Some(c) => (c.is_sensor(), c.parent().is_some()),
                            None => collider_flags(entity),
                        };
                        let inside = hit.time_of_impact <= 1e-6;
                        let normal = if inside || hit.normal.length_squared() < 1e-12 {
                            -dir_v
                        } else {
                            hit.normal
                        };
                        hit_of(
                            entity,
                            sensor,
                            carried,
                            hit.subshape,
                            hit.point,
                            normal,
                            hit.time_of_impact,
                            length,
                            inside,
                        )
                    };
                    if *all {
                        let hits = pipeline
                            .intersect_ray(origin, dir_v, length, true)
                            .map(|(entity, collider, hit)| make(entity, Some(collider), &hit))
                            .collect();
                        QueryOutcome::finish(hits, limit)
                    } else {
                        let hits = pipeline
                            .cast_ray_and_get_normal(origin, dir_v, length, true)
                            .map(|(entity, hit)| make(entity, None, &hit))
                            .into_iter()
                            .collect();
                        QueryOutcome::finish(hits, limit)
                    }
                }
                QueryRequest::Cast { shape, from, to } => {
                    let (cast, rotation) = shape_of(shape);
                    let origin = vec(*from);
                    let Some((dir, length)) = segment(planar(*from), planar(*to)) else {
                        // No travel: it is the overlap at the start, nearest first.
                        return overlap_at(&pipeline, &cast, origin, rotation, limit, &|e, c| {
                            hit_of(
                                e,
                                c.is_sensor(),
                                c.parent().is_some(),
                                0,
                                c.position().translation,
                                Vec2::ZERO,
                                0.0,
                                0.0,
                                true,
                            )
                        });
                    };
                    let options = ShapeCastOptions {
                        max_time_of_impact: length,
                        target_distance: 0.0,
                        stop_at_penetration: true,
                        compute_impact_geometry_on_penetration: true,
                    };
                    let dir_v = vec(dir);
                    let found = match &cast {
                        Cast::Ball(s) => pipeline.cast_shape(origin, rotation, dir_v, s, options),
                        Cast::Box(s) => pipeline.cast_shape(origin, rotation, dir_v, s, options),
                        Cast::Capsule(s) => {
                            pipeline.cast_shape(origin, rotation, dir_v, s, options)
                        }
                    };
                    let hits = found
                        .map(|(entity, hit)| {
                            let (sensor, carried) = collider_flags(entity);
                            let inside = hit.time_of_impact <= 1e-6;
                            let details = hit.details;
                            let point = details.map_or(origin, |d| d.witness1);
                            let normal = match details {
                                Some(d) if !inside => d.normal1,
                                _ => -dir_v,
                            };
                            hit_of(
                                entity,
                                sensor,
                                carried,
                                hit.subshape1,
                                point,
                                normal,
                                hit.time_of_impact,
                                length,
                                inside,
                            )
                        })
                        .into_iter()
                        .collect();
                    QueryOutcome::finish(hits, limit)
                }
                QueryRequest::Overlap { shape, at } => {
                    let (cast, rotation) = shape_of(shape);
                    overlap_at(&pipeline, &cast, vec(*at), rotation, limit, &|e, c| {
                        hit_of(
                            e,
                            c.is_sensor(),
                            c.parent().is_some(),
                            0,
                            c.position().translation,
                            Vec2::ZERO,
                            0.0,
                            0.0,
                            true,
                        )
                    })
                }
                QueryRequest::Closest {
                    point,
                    max_distance,
                } => {
                    let origin = vec(*point);
                    let hits = pipeline
                        .project_point(origin, *max_distance, true)
                        .map(|(entity, projection)| {
                            let (sensor, carried) = collider_flags(entity);
                            let distance = if projection.is_inside {
                                0.0
                            } else {
                                origin.distance(projection.point)
                            };
                            let normal = if projection.is_inside || distance <= 1e-9 {
                                Vec2::ZERO
                            } else {
                                (origin - projection.point) / distance
                            };
                            hit_of(
                                entity,
                                sensor,
                                carried,
                                projection.subshape,
                                projection.point,
                                normal,
                                distance,
                                *max_distance,
                                projection.is_inside,
                            )
                        })
                        .into_iter()
                        .filter(|hit| hit.distance <= *max_distance)
                        .collect();
                    QueryOutcome::finish(hits, limit)
                }
            })
        }
    }

    fn overlap_at(
        pipeline: &rp::RapierQueryPipeline<'_>,
        cast: &Cast,
        at: Vec2,
        rotation: f32,
        limit: usize,
        make: &dyn Fn(Entity, &bevy_rapier2d::rapier::geometry::Collider) -> QueryHit,
    ) -> QueryOutcome {
        let hits: Vec<QueryHit> = match cast {
            Cast::Ball(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
            Cast::Box(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
            Cast::Capsule(s) => pipeline
                .intersect_shape(at, rotation, s)
                .map(|(entity, collider)| make(entity, collider))
                .collect(),
        };
        QueryOutcome::finish(hits, limit)
    }
}
