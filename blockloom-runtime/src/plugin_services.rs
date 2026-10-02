//! What a plugin may ask the running world: physics queries over the frame's
//! sensor snapshot and paths over the navigation mesh.
//!
//! Both are read-only questions answered on the thread the world runs on, so
//! they see exactly what a reporter block sees: the state published for this
//! frame. A module that wants to change the world submits an effect instead.
//!
//! - `physics.ray {from, to}` or `{from, dir, max}`, `skip?`, `mask?`
//!   answers `{hit, actor?, distance?, point?, normal?}`.
//! - `physics.overlap_point {point, mask?}` and `physics.overlap_sphere
//!   {center, radius, skip?, mask?}` answer `{actors: [...]}`, nearest first.
//! - `nav.available {}` answers `{available}`; `nav.path {from, to, layer?}`
//!   answers `{found, points?}` in the navigation plane's coordinates.

use bevy::math::Vec3;
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
    fn call(&self, _plugin: &str, service: &str, input: &Value) -> Option<Result<Value, String>> {
        Some(match service {
            "physics.ray" => ray(input),
            "physics.overlap_point" => vec3(input, "point").map(|point| {
                let m = mask(input, None);
                json!({"actors": sense::read(|s| overlap_point(s, point, m))})
            }),
            "physics.overlap_sphere" => vec3(input, "center").map(|center| {
                let radius = input.get("radius").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let skip = input.get("skip").and_then(Value::as_str);
                let m = mask(input, skip);
                json!({"actors": sense::read(|s| overlap_circle(s, center, radius, skip, m))})
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
}
