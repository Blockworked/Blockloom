//! What a plugin may ask the running world: physics queries and paths over the
//! navigation mesh.
//!
//! Both are read-only questions answered on the thread the world runs on. During
//! a run physics queries are the authoritative ones (the Rapier world's own
//! shapes and layer rules, see `blockloom_core::physics::query`), the same a
//! block or script asks. Where no world is running (a hosted preview) they
//! fall back to the frame's sensor snapshot, which is approximate. A module that
//! wants to change the world submits an effect instead.
//!
//! - `physics.ray {from, to}` or `{from, dir, max}`, `skip?`, `mask?` (layer
//!   bits) answers `{hit, actor?, body?, collider?, distance?, fraction?,
//!   point?, normal?, startedInside?, trigger?}`.
//! - `physics.overlap_point {point, mask?}` and `physics.overlap_sphere
//!   {center, radius, skip?, mask?}` answer `{actors: [...]}`, nearest first,
//!   and `hits` with each collider's identity.
//! - `nav.available {}` answers `{available}`; `nav.path {from, to, layer?}`
//!   answers `{found, points?}` in the navigation plane's coordinates.

use bevy::math::Vec3;
use blockloom_core::physics::query::{self, QueryFilter, QueryRequest, QueryShape, TriggerPolicy};
use blockloom_core::physics_query::{overlap_circle, overlap_point, ray_hit, segment_contact};
use blockloom_core::scene::Mode;
use blockloom_core::{nav, sense};
use blockloom_plugin_host::services::ServiceProvider;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

struct NavSnapshot {
    mesh: Arc<polyanya::Mesh>,
    mode: Mode,
    settings: nav::NavSettings,
}

thread_local! {
    static MESH_POSES: RefCell<std::collections::BTreeMap<(String,String),Value>> = const { RefCell::new(std::collections::BTreeMap::new()) };
    static NAV: RefCell<Option<Rc<NavSnapshot>>> = const { RefCell::new(None) };
}

/// Called whenever the world bakes its navigation mesh.
pub fn publish_nav(navmesh: &crate::world::NavMesh) {
    let snapshot = navmesh.mesh.as_ref().map(|mesh| {
        Rc::new(NavSnapshot {
            mesh: mesh.clone(),
            mode: navmesh.mode,
            settings: navmesh.settings.clone(),
        })
    });
    NAV.with(|slot| *slot.borrow_mut() = snapshot);
}

pub fn publish_mesh_poses(poses: std::collections::BTreeMap<(String, String), Value>) {
    MESH_POSES.with(|slot| *slot.borrow_mut() = poses);
}

pub struct WorldQueries;

fn vec3(input: &Value, name: &str) -> Result<[f32; 3], String> {
    let list = input
        .get(name)
        .and_then(Value::as_array)
        .filter(|l| l.len() == 3)
        .ok_or_else(|| format!("`{name}` is [x, y, z]"))?;
    let mut out = [0.0f32; 3];
    for (slot, v) in out.iter_mut().zip(list) {
        let n = v
            .as_f64()
            .ok_or_else(|| format!("`{name}` holds numbers"))?;
        if !n.is_finite() {
            return Err(format!("`{name}` holds a number that is not finite"));
        }
        *slot = n as f32;
    }
    Ok(out)
}

fn mask(input: &Value, skip: Option<&str>) -> u8 {
    match input.get("mask").and_then(Value::as_u64) {
        Some(m) => m.min(255) as u8,
        None => sense::read(|s| blockloom_core::physics_query::query_mask(s, skip)),
    }
}

/// The filter a service call asks with: `skip` is the asker and `mask` the
/// layers it may hit.
fn authoritative(input: &Value, skip: Option<&str>) -> QueryFilter {
    QueryFilter {
        layers: input
            .get("mask")
            .and_then(Value::as_u64)
            .map_or(u32::MAX, |m| m.min(u64::from(u32::MAX)) as u32),
        as_actor: skip.map(str::to_string),
        triggers: TriggerPolicy::UseGlobal,
        ..QueryFilter::default()
    }
}

fn hit_json(hit: &query::QueryHit) -> Value {
    json!({
        "actor": hit.actor,
        "body": hit.body,
        "collider": hit.collider,
        "part": hit.subshape,
        "distance": hit.distance,
        "fraction": hit.fraction,
        "point": hit.point,
        "normal": hit.normal,
        "startedInside": hit.started_inside,
        "trigger": hit.trigger,
    })
}

fn actors_of(outcome: query::QueryOutcome) -> Result<Value, String> {
    if let Some(why) = outcome.error {
        return Err(why);
    }
    let mut actors: Vec<&str> = Vec::new();
    for hit in &outcome.hits {
        if !actors.contains(&hit.actor.as_str()) {
            actors.push(&hit.actor);
        }
    }
    Ok(json!({
        "actors": actors,
        "hits": outcome.hits.iter().map(hit_json).collect::<Vec<_>>(),
        "overflow": outcome.overflow,
    }))
}

fn ray(input: &Value) -> Result<Value, String> {
    let from = vec3(input, "from")?;
    let to = if input.get("to").is_some() {
        vec3(input, "to")?
    } else {
        let dir = Vec3::from(vec3(input, "dir")?)
            .try_normalize()
            .ok_or("`dir` has no direction")?;
        let max = input.get("max").and_then(Value::as_f64).unwrap_or(64.0) as f32;
        (Vec3::from(from) + dir * max.clamp(0.0, 100_000.0)).to_array()
    };
    let skip = input.get("skip").and_then(Value::as_str);
    if query::available() {
        let request = QueryRequest::Ray {
            from,
            to,
            all: false,
        };
        let outcome = query::dispatch(&request, &authoritative(input, skip), 1);
        if let Some(why) = outcome.error {
            return Err(why);
        }
        return Ok(match outcome.hits.first() {
            None => json!({"hit": false}),
            Some(hit) => {
                let mut found = hit_json(hit);
                found["hit"] = json!(true);
                found
            }
        });
    }
    let mask = mask(input, skip);
    Ok(sense::read(|s| match ray_hit(s, from, to, skip, mask) {
        None => json!({"hit": false}),
        Some((actor, distance)) => {
            let span = Vec3::from(to) - Vec3::from(from);
            let point = (Vec3::from(from) + span.normalize_or_zero() * distance).to_array();
            // The contact normal comes from the nearest shape; use it only
            // when that is the body the ray reported.
            let normal = segment_contact(s, from, to, skip)
                .filter(|(p, _)| Vec3::from(*p).distance(Vec3::from(point)) < 1e-2)
                .map(|(_, n)| n);
            json!({"hit": true, "actor": actor, "distance": distance, "point": point, "normal": normal})
        }
    }))
}

fn path(input: &Value) -> Result<Value, String> {
    let from = vec3(input, "from")?;
    let to = vec3(input, "to")?;
    let layer = input.get("layer").and_then(Value::as_u64).unwrap_or(1) as u32;
    let snapshot = NAV.with(|slot| slot.borrow().clone());
    let Some(nav) = snapshot else {
        return Ok(json!({"found": false, "reason": "there is no navigation mesh"}));
    };
    let a = nav::plane_coords(nav.mode, from);
    let b = nav::plane_coords(nav.mode, to);
    Ok(
        match nav::find_route(&nav.mesh, a, b, &nav.settings, layer) {
            Some(points) => json!({"found": true, "points": points}),
            None => json!({"found": false, "reason": "no route"}),
        },
    )
}

impl ServiceProvider for WorldQueries {
    fn call(&self, plugin: &str, service: &str, input: &Value) -> Option<Result<Value, String>> {
        Some(match service {
            "world.mesh_pose" => Ok(MESH_POSES.with(|slot| {
                input["name"]
                    .as_str()
                    .and_then(|name| slot.borrow().get(&(plugin.into(), name.into())).cloned())
                    .unwrap_or(Value::Null)
            })),
            "world.actor" => Ok(sense::read(|s| {
                input["actor"]
                    .as_str()
                    .and_then(|id| s.actors.get(id))
                    .map_or(
                        json!({"found":false}),
                        |actor| json!({"found":true,"position":actor.position}),
                    )
            })),
            "physics.ray" => ray(input),
            "physics.overlap_point" => vec3(input, "point").and_then(|point| {
                if query::available() {
                    let request = QueryRequest::Ray {
                        from: point,
                        to: point,
                        all: true,
                    };
                    return actors_of(query::dispatch(
                        &request,
                        &authoritative(input, None),
                        query::MAX_RESULTS,
                    ));
                }
                let m = mask(input, None);
                Ok(json!({"actors": sense::read(|s| overlap_point(s, point, m))}))
            }),
            "physics.overlap_sphere" => vec3(input, "center").and_then(|center| {
                let radius = input.get("radius").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let skip = input.get("skip").and_then(Value::as_str);
                if query::available() {
                    let request = QueryRequest::Overlap {
                        shape: QueryShape::Ball { radius },
                        at: center,
                    };
                    return actors_of(query::dispatch(
                        &request,
                        &authoritative(input, skip),
                        query::MAX_RESULTS,
                    ));
                }
                let m = mask(input, skip);
                Ok(json!({"actors": sense::read(|s| overlap_circle(s, center, radius, skip, m))}))
            }),
            "nav.available" => Ok(json!({"available": NAV.with(|s| s.borrow().is_some())})),
            "nav.path" => path(input),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::sense::{ActorSense, Sensors};

    fn wall() -> ActorSense {
        ActorSense {
            has_body: true,
            layer: 1,
            mask: 0xFF,
            position: [5.0, 0.0, 0.0],
            ..ActorSense::default()
        }
    }

    #[test]
    fn a_ray_and_an_overlap_read_the_published_snapshot() {
        let mut sensors = Sensors::default();
        sensors.actors.insert("wall".into(), wall());
        sense::publish(sensors);
        let q = WorldQueries;
        let miss = q.call(
            "p",
            "physics.ray",
            &json!({"from": [0, 50, 0], "to": [10, 50, 0]}),
        );
        assert_eq!(miss.unwrap().unwrap()["hit"], false);
        let bad = q.call("p", "physics.ray", &json!({"from": [0, 0]}));
        assert!(bad.unwrap().is_err());
        assert!(q.call("p", "physics.nonsense", &json!({})).is_none());
        let none = q.call(
            "p",
            "nav.path",
            &json!({"from": [0, 0, 0], "to": [1, 0, 1]}),
        );
        assert_eq!(none.unwrap().unwrap()["found"], false);
        let overlap = q.call(
            "p",
            "physics.overlap_sphere",
            &json!({"center": [100, 0, 0], "radius": 1.0}),
        );
        assert_eq!(overlap.unwrap().unwrap()["actors"], json!([]));
    }

    struct OneWall;

    impl query::QueryService for OneWall {
        fn run(
            &self,
            request: &QueryRequest,
            filter: &QueryFilter,
            limit: usize,
        ) -> query::QueryOutcome {
            let hit = query::QueryHit {
                actor: "wall".into(),
                body: None,
                collider: "c1".into(),
                subshape: 2,
                point: [5.0, 0.0, 0.0],
                normal: [-1.0, 0.0, 0.0],
                distance: 5.0,
                fraction: 0.5,
                started_inside: false,
                trigger: false,
            };
            // The asker and its layers reach the service unchanged.
            assert_eq!(filter.as_actor.as_deref(), Some("me"));
            assert_eq!(filter.layers, 0b100);
            let hits = match request {
                QueryRequest::Closest { .. } => vec![],
                _ => vec![hit],
            };
            query::QueryOutcome::finish(hits, limit)
        }
    }

    #[test]
    fn during_a_run_the_services_ask_the_authoritative_queries() {
        let q = WorldQueries;
        let ray = json!({"from": [0, 0, 0], "to": [10, 0, 0], "skip": "me", "mask": 4});
        let (hit, overlap) = query::with_service(&OneWall, 3, || {
            (
                q.call("p", "physics.ray", &ray).unwrap().unwrap(),
                q.call(
                    "p",
                    "physics.overlap_sphere",
                    &json!({"center": [5, 0, 0], "radius": 1.0, "skip": "me", "mask": 4}),
                )
                .unwrap()
                .unwrap(),
            )
        });
        assert_eq!(hit["hit"], true);
        assert_eq!(hit["actor"], "wall");
        assert_eq!(hit["collider"], "c1");
        assert_eq!(hit["part"], 2);
        assert_eq!(overlap["actors"], json!(["wall"]));
        assert_eq!(overlap["hits"][0]["fraction"], 0.5);
    }
}
