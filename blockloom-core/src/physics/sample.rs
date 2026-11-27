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
pub const SAMPLES: [&str; 6] = [
    "physics-playground",
    "third-person",
    "platformer",
    "top-down",
    "first-person",
    "ui-puzzle",
];

/// The dimensions a sample can be made in.
pub fn modes(sample: &str) -> &'static [Mode] {
    match sample {
        "third-person" | "first-person" => &[Mode::ThreeD],
        "platformer" | "top-down" => &[Mode::TwoD],
        _ => &[Mode::TwoD, Mode::ThreeD],
    }
}

/// Where the third-person sample keeps its figure.
pub const FIGURE: &str = "assets/models/figure.glb";

/// Builds the sample named `sample` (see [`SAMPLES`]).
pub fn build(sample: &str, name: &str, mode: Mode) -> Result<Project, String> {
    match sample {
        "physics-playground" => physics_playground(name, mode),
        "third-person" => third_person(name, mode),
        "platformer" => super::genres::platformer(name, mode),
        "top-down" => super::genres::top_down(name, mode),
        "first-person" => super::genres::first_person(name, mode),
        "ui-puzzle" => super::genres::ui_puzzle(name, mode),
        other => Err(format!(
            "No sample called \"{other}\". Samples: {}",
            SAMPLES.join(", ")
        )),
    }
}

/// The files a sample brings into its project folder, by relative path.
pub fn files(sample: &str) -> Vec<(&'static str, Vec<u8>)> {
    match sample {
        "third-person" => vec![(FIGURE, crate::figure::glb())],
        _ => Vec::new(),
    }
}

/// The third-person template: the blocky figure as the player, walking,
/// running and jumping on one blended animator with foot and head IK, and a
/// boulder that knocks it limp for two seconds. A footstep counter shows the
/// animator's markers reaching the blocks.
fn third_person(name: &str, mode: Mode) -> Result<Project, String> {
    use crate::animation::{AnimationLayer, AnimationSpec, FrameMarker, ModelIk, RagdollSpec};
    use crate::blocks::{Instruction, InstructionKind as K, Strand};
    use crate::components::ActorComponent;
    use crate::figure;
    use crate::value::Value;
    if !mode.is_3d() {
        return Err("the third-person sample is 3D only".to_string());
    }
    let mut project = Project::starter(name, mode);
    // Room to run.
    if let Some(ground) = project.actors.iter_mut().find(|a| a.name == "Ground") {
        ground.components.set_visual(Visual::Plane {
            color: "#3E4A5B".into(),
            size: [120.0, 120.0],
        });
    }
    let mut library = project.physics.materials.clone();
    let (scene, settings) = project.active_scene_and_settings();
    scene.migrate_physics(&mut library);
    let player = scene
        .actors
        .iter()
        .find(|a| a.name == "Player")
        .map(|a| a.id.clone())
        .ok_or("the starter project lost its player")?;
    scene.apply_player_preset(
        &mut settings.input,
        &player,
        PlayerPreset::ThirdPerson3d,
        true,
        &library,
    )?;
    let actor = scene.actor_mut(&player).ok_or("the player went missing")?;
    let height = actor
        .components
        .character_controller()
        .map_or(1.8, |controller| controller.height);
    let size = height / 1.8;
    actor.components.set_visual(Visual::Model {
        path: FIGURE.to_string(),
        tint: "#FFFFFF".to_string(),
        scale: [size; 3],
        animation: String::new(),
    });
    let motor = actor
        .components
        .character_motor()
        .cloned()
        .unwrap_or_default();
    let mut animation: AnimationSpec = AnimationSpec::default();
    let clips: Vec<crate::pipeline::ModelClip> = figure::clip_lengths()
        .into_iter()
        .map(|(name, seconds)| crate::pipeline::ModelClip {
            name: name.to_string(),
            seconds,
        })
        .collect();
    animation.add_model_clips(&clips);
    animation.add_locomotion(motor.walk_speed, motor.sprint_speed);
    // A footstep at each foot's plant.
    for clip in &mut animation.clips {
        let frames: &[usize] = match clip.name.as_str() {
            "Walk" => &[1, 16],
            "Run" => &[1, 10],
            _ => &[],
        };
        clip.markers = frames
            .iter()
            .map(|&frame| FrameMarker {
                frame,
                name: "step".to_string(),
            })
            .collect();
    }
    animation.ik = Some(ModelIk {
        feet: figure::FEET.iter().map(|b| b.to_string()).collect(),
        hips: figure::HIPS.to_string(),
        foot_reach: 0.4,
        foot_height: 0.05 * size,
        head: figure::HEAD.to_string(),
        look_limit: 70.0,
    });
    animation.ragdoll = Some(RagdollSpec {
        bones: figure::RAGDOLL.iter().map(|b| b.to_string()).collect(),
        radius: 0.08 * size,
        mass: 70.0,
        swing: 60.0,
        get_up: 0.5,
    });
    animation.layers = vec![AnimationLayer {
        name: "upper body".to_string(),
        bones: figure::UPPER_BODY.iter().map(|b| b.to_string()).collect(),
        weight: 1.0,
    }];
    animation.normalize();
    actor
        .components
        .insert(ActorComponent::Animation { animation });
    let verb = |name: &str, args: Vec<Value>| K::Verb {
        verb: name.to_string(),
        args,
    };
    let strand = |x: i32, kinds: Vec<K>| {
        Strand::with_instructions(x, 0, kinds.into_iter().map(Instruction::new).collect())
    };
    actor.graph.strands = vec![
        strand(
            0,
            vec![
                K::WhenAnimationMarker {
                    marker: "step".to_string(),
                },
                K::ChangeVariable {
                    name: "steps".to_string(),
                    value: Value::number(1.0),
                },
            ],
        ),
        strand(
            320,
            vec![
                K::WhenCollision {
                    with: "Boulder".to_string(),
                    phase: super::ContactPhase::Enter,
                    scope: Default::default(),
                },
                verb("go_limp", Vec::new()),
                K::Wait {
                    duration: Value::number(2.0),
                },
                verb("get_up", Vec::new()),
            ],
        ),
        strand(
            640,
            vec![
                K::WhenStarted,
                verb("look_at", vec![Value::text("Boulder")]),
            ],
        ),
    ];

    // A boulder that drops onto where the player starts.
    let boulder = scene.add_actor({
        let mut boulder = Actor::new(
            "Boulder",
            Visual::Sphere {
                color: "#8D8D8D".into(),
                radius: 0.5,
            },
        );
        boulder.components.placement_mut().position = [0.3, 12.0, 0.0];
        boulder
    });
    scene.set_rigidbody(
        &boulder,
        RigidbodySpec {
            body_type: BodyType::Dynamic,
            ..RigidbodySpec::default()
        },
        &library,
    )?;
    scene.add_collider(&boulder, ColliderSpec::from_look(), &library)?;
    project.physics.materials = library;
    Ok(project)
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
    let (scene, settings) = project.active_scene_and_settings();
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
    scene.apply_player_preset(&mut settings.input, &player, preset, true, &library)?;

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
    rope.anchor = [0.0, 0.0, 0.0];
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
    fn the_third_person_template_carries_its_figure_and_animator() {
        assert!(build("third-person", "T", Mode::TwoD).is_err());
        let project = build("third-person", "T", Mode::ThreeD).unwrap();
        let errors: Vec<_> = plan(&project).errors().map(|i| i.message.clone()).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let player = project.actors.iter().find(|a| a.name == "Player").unwrap();
        assert!(matches!(player.visual(), Some(Visual::Model { path, .. }) if path == FIGURE));
        let spec = player.components.animation().unwrap();
        assert_eq!(spec.initial, crate::animation::LOCOMOTION);
        assert!(spec.ik.is_some() && spec.ragdoll.is_some() && spec.layers.len() == 1);
        let walk = spec.find_clip("Walk").unwrap();
        assert_eq!(walk.markers.len(), 2);
        assert!(player.components.character_motor().is_some());
        let files = files("third-person");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, FIGURE);
        assert_eq!(&files[0].1[..4], b"glTF");
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
    fn every_template_plans_cleanly_in_its_dimensions() {
        for sample in SAMPLES {
            for &mode in modes(sample) {
                let project = build(sample, "T", mode).unwrap();
                let errors: Vec<_> = plan(&project).errors().map(|i| i.message.clone()).collect();
                assert!(errors.is_empty(), "{sample} {mode:?}: {errors:?}");
                let pack = crate::pack::GamePack::new(project.clone());
                let text = serde_json::to_string(&pack).unwrap();
                let back = crate::pack::GamePack::from_json(&text, "test").unwrap();
                assert_eq!(back.project.actors.len(), project.actors.len(), "{sample}");
            }
            for mode in [Mode::TwoD, Mode::ThreeD] {
                if !modes(sample).contains(&mode) {
                    assert!(build(sample, "T", mode).is_err(), "{sample} {mode:?}");
                }
            }
        }
    }

    #[test]
    fn an_unknown_sample_names_the_ones_there_are() {
        let error = build("nope", "x", Mode::ThreeD).unwrap_err();
        assert!(error.contains("physics-playground"), "{error}");
    }
}
