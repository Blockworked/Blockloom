//! The physics playground: a small project that uses most of the physics model
//! with no custom code, as a starting point and a fixture. A player (preset),
//! crates, a hinged door or swinging pendulum, a rope, a motor wheel, a joint
//! that snaps and a trigger zone, all authored as components.

use super::joints::{ConstraintKind, ConstraintSpec, Limit, MotorMode};
use super::presets::PlayerPreset;
use super::spec::{BodyType, ColliderSpec, RigidbodySpec};
use crate::project::{Actor, Project};
use crate::scene::{Mode, Visual};

/// Which samples exist, for the project dialog and the shell.
pub const SAMPLES: [&str; 1] = ["physics-playground"];

/// Builds the sample named `sample` (see [`SAMPLES`]).
pub fn build(sample: &str, name: &str, mode: Mode) -> Result<Project, String> {
    match sample {
        "physics-playground" => physics_playground(name, mode),
        other => Err(format!(
            "No sample called \"{other}\". Samples: {}",
            SAMPLES.join(", ")
        )),
    }
}

struct Spawn {
    name: &'static str,
    visual: Visual,
    at: [f32; 3],
    /// `None` is static scenery (a collider and no body).
    body: Option<BodyType>,
    trigger: bool,
}

fn cube(color: &str, size: f32, mode: Mode) -> Visual {
    if mode.is_3d() {
        Visual::Cuboid {
            color: color.into(),
            size: [size; 3],
        }
    } else {
        Visual::Rect {
            color: color.into(),
            size: [size * 50.0, size * 50.0],
        }
    }
}

fn physics_playground(name: &str, mode: Mode) -> Result<Project, String> {
    let mut project = Project::starter(name, mode);
    let mut library = project.physics.materials.clone();
    // Start from components: the starter's Player and Ground become a body and a
    // collider, so nothing here leans on the legacy Body.
    let scene = project.active_scene_mut();
    scene.migrate_physics(&mut library);
    let player = scene
        .actors
        .iter()
        .find(|a| a.name == "Player")
        .map(|a| a.id.clone())
        .ok_or("the starter project lost its player")?;
    let preset = if mode.is_3d() {
        PlayerPreset::ThirdPerson3d
    } else {
        PlayerPreset::Platformer2d
    };
    scene.apply_player_preset(&player, preset, true, &library)?;

    // Metres in 3D, pixels in 2D: the 2D scale is 50 pixels to a metre.
    let k = if mode.is_3d() { 1.0 } else { 50.0 };
    let at = |x: f32, y: f32, z: f32| -> [f32; 3] {
        if mode.is_3d() {
            [x, y, z]
        } else {
            [x * k, y * k - 150.0, 0.0]
        }
    };
    let spawn = |scene: &mut crate::project::Scene, s: Spawn| -> Result<String, String> {
        let mut actor = Actor::new(s.name, s.visual);
        actor.components.placement_mut().position = s.at;
        let id = scene.add_actor(actor);
        if let Some(body_type) = s.body {
            scene.set_rigidbody(
                &id,
                RigidbodySpec {
                    body_type,
                    ..RigidbodySpec::default()
                },
                &library,
            )?;
        }
        let mut collider = ColliderSpec::from_look();
        collider.trigger = s.trigger;
        scene.add_collider(&id, collider, &library)?;
        Ok(id)
    };
    let constraint =
        |scene: &mut crate::project::Scene,
         actor: &str,
         spec: ConstraintSpec|
         -> Result<(), String> { scene.add_constraint(actor, spec, &library).map(|_| ()) };

    // A stack of crates to knock over.
    for (i, (x, y)) in [(3.0, 0.5), (3.0, 1.5), (3.0, 2.5)].into_iter().enumerate() {
        spawn(
            scene,
            Spawn {
                name: ["Crate A", "Crate B", "Crate C"][i],
                visual: cube("#C58B4B", 1.0, mode),
                at: at(x, y, 0.0),
                body: Some(BodyType::Dynamic),
                trigger: false,
            },
        )?;
    }

    // A pendulum: a weight on a rope that holds it at its length.
    let weight = spawn(
        scene,
        Spawn {
            name: "Pendulum",
            visual: cube("#E4572E", 0.6, mode),
            at: at(-3.0, 4.0, 0.0),
            body: Some(BodyType::Dynamic),
            trigger: false,
        },
    )?;
    let mut rope = ConstraintSpec::of(ConstraintKind::Distance, mode);
    rope.name = "rope".into();
    rope.anchor = [0.0, if mode.is_3d() { 0.0 } else { 0.0 }, 0.0];
    rope.max_distance = 2.5 * k;
    constraint(scene, &weight, rope)?;

    // A wheel a motor turns, which a block can speed up or stop.
    let wheel = spawn(
        scene,
        Spawn {
            name: "Wheel",
            visual: if mode.is_3d() {
                Visual::Cuboid {
                    color: "#3A86FF".into(),
                    size: [2.0, 0.2, 2.0],
                }
            } else {
                Visual::Rect {
                    color: "#3A86FF".into(),
                    size: [100.0, 10.0],
                }
            },
            at: at(-6.0, 2.0, 0.0),
            body: Some(BodyType::Dynamic),
            trigger: false,
        },
    )?;
    let mut spin = ConstraintSpec::of(ConstraintKind::Hinge, mode);
    spin.name = "motor".into();
    if mode.is_3d() {
        spin.axis = [0.0, 1.0, 0.0];
        spin.connected_axis = [0.0, 1.0, 0.0];
    }
    spin.motor.mode = MotorMode::Velocity;
    spin.motor.target = 90.0;
    spin.motor.max_force = 500.0;
    constraint(scene, &wheel, spin)?;

    // A door that swings a quarter turn each way, hinged at its edge.
    if mode.is_3d() {
        let door = spawn(
            scene,
            Spawn {
                name: "Door",
                visual: Visual::Cuboid {
                    color: "#8D6E63".into(),
                    size: [1.2, 2.0, 0.1],
                },
                at: [6.0, 1.0, -2.0],
                body: Some(BodyType::Dynamic),
                trigger: false,
            },
        )?;
        let mut hinge = ConstraintSpec::of(ConstraintKind::Hinge, mode);
        hinge.name = "hinge".into();
        hinge.anchor = [-0.6, 0.0, 0.0];
        hinge.axis = [0.0, 1.0, 0.0];
        hinge.connected_axis = [0.0, 1.0, 0.0];
        hinge.limit = Limit {
            enabled: true,
            min: -90.0,
            max: 90.0,
        };
        constraint(scene, &door, hinge)?;
    }

    // A plank held by a joint that snaps under a heavy hit.
    let plank = spawn(
        scene,
        Spawn {
            name: "Plank",
            visual: if mode.is_3d() {
                Visual::Cuboid {
                    color: "#6D6875".into(),
                    size: [3.0, 0.2, 1.0],
                }
            } else {
                Visual::Rect {
                    color: "#6D6875".into(),
                    size: [150.0, 10.0],
                }
            },
            at: at(0.0, 3.0, 4.0),
            body: Some(BodyType::Dynamic),
            trigger: false,
        },
    )?;
    let mut weld = ConstraintSpec::of(ConstraintKind::Fixed, mode);
    weld.name = "weld".into();
    weld.break_force = Some(400.0 * k);
    weld.break_message = "plank broke".into();
    constraint(scene, &plank, weld)?;

    // A trigger zone: a collider that reports enter and exit without blocking.
    spawn(
        scene,
        Spawn {
            name: "Zone",
            visual: cube("#2A9D8F", 3.0, mode),
            at: at(-3.0, 1.5, 6.0),
            body: None,
            trigger: true,
        },
    )?;

    project.physics.materials = library;
    project.physics.stamp();
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::plan::PhysicsPlan;

    fn plan(project: &Project) -> PhysicsPlan {
        let scene = project.active_scene();
        PhysicsPlan::build(&scene.actors, scene.world.mode, &project.physics)
    }

    #[test]
    fn the_playground_plans_cleanly_in_both_dimensions() {
        for mode in [Mode::ThreeD, Mode::TwoD] {
            let project = build("physics-playground", "Playground", mode).unwrap();
            let plan = plan(&project);
            let errors: Vec<_> = plan.errors().map(|i| i.message.clone()).collect();
            assert!(errors.is_empty(), "{mode:?}: {errors:?}");
            assert!(plan.bodies.len() >= 7, "{mode:?}: {}", plan.bodies.len());
            assert_eq!(plan.controllers.len(), 1);
            assert!(plan.constraints.len() >= 3, "{mode:?}");
            // Nothing leans on the legacy Body.
            let scene = project.active_scene();
            assert!(
                scene
                    .actors
                    .iter()
                    .all(|a| a.components.get("Body").is_none()),
                "{mode:?}: {:?}",
                scene
                    .actors
                    .iter()
                    .filter(|a| a.components.get("Body").is_some())
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_built_pack_carries_the_same_plan() {
        for mode in [Mode::ThreeD, Mode::TwoD] {
            let project = build("physics-playground", "Playground", mode).unwrap();
            let pack = crate::pack::GamePack::new(project.clone());
            let text = serde_json::to_string(&pack).unwrap();
            let back = crate::pack::GamePack::from_json(&text, "test").unwrap();
            let (a, b) = (plan(&project), plan(&back.project));
            assert_eq!(a.bodies.len(), b.bodies.len(), "{mode:?}");
            assert_eq!(a.colliders.len(), b.colliders.len(), "{mode:?}");
            assert_eq!(a.constraints.len(), b.constraints.len(), "{mode:?}");
            let names = |plan: &PhysicsPlan| {
                let mut v: Vec<_> = plan.constraints.iter().map(|c| c.handle.clone()).collect();
                v.sort();
                v
            };
            assert_eq!(names(&a), names(&b), "{mode:?}");
            assert!(b.errors().next().is_none(), "{mode:?}");
        }
    }

    #[test]
    fn an_unknown_sample_names_the_ones_there_are() {
        let error = build("nope", "x", Mode::ThreeD).unwrap_err();
        assert!(error.contains("physics-playground"), "{error}");
    }
}
