//! The simulation half of the world's registration: the fixed-step chain, the
//! physics pipelines and what they need, with nothing that draws, plays or
//! reads a window. `add_world` calls this and then adds the presentation
//! systems around it, so a server can register the same simulation alone.
//!
//! The fixed step used to be one chain mixing both kinds. Each simulation
//! system now sits in a `SimStep`, the steps are chained in the old order, and
//! presentation systems order themselves against the steps they used to sit
//! between (`add_presentation` in lib.rs), so the order is unchanged.
use crate::engine::InsertEngine;

use crate::engine::{Dimension, Engine, PendingEffects};
use crate::{
    ai, atmosphere, constraints, controller, dim2, dim3, motor, physics_install, plugins, volumes,
    world,
};
use bevy::prelude::*;
use blockloom_core::scene::Mode;
use blockloom_plugin_api::schema::Stage;

/// One simulation system (or small chain) of the fixed step, in run order.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimStep {
    Pause,
    Timestep,
    Sample,
    PluginInput,
    PluginPreSimulation,
    Vm,
    Scripts,
    PluginFixed,
    SavedData,
    Verbs,
    Lifetimes,
    Common,
    VolumeEffects,
    ApplyEffects,
    ComponentEffects,
    PluginEffects,
    Joints,
    Animation,
    ClearEffects,
    FinishStep,
}

/// Resources, physics and the fixed-step simulation. The engine moves in
/// because every simulation system reaches it as a `NonSend`.
pub(crate) fn add_simulation(app: &mut App, mode: Mode, engine: Engine) {
    if let Some(recorder) = crate::record::Recorder::from_env() {
        app.insert_resource(recorder);
    }
    crate::catchup::register(app);
    app.insert_resource(Dimension(mode))
        .init_resource::<PendingEffects>()
        .init_resource::<crate::volumes::VolumeRun>()
        .init_resource::<world::InputRun>()
        .add_message::<crate::module::WorldRebuilt>()
        .add_systems(Update, world::rebuild_world.in_set(world::RebuildSet))
        .add_systems(
            Update,
            crate::module::forget_on_rebuild::<world::InputRun>.after(world::rebuild_world),
        )
        .init_resource::<world::NavMesh>()
        .init_resource::<crate::player_camera::BodyFacing>()
        .init_resource::<physics_install::PhysicsLayers>()
        .insert_engine(engine);
    crate::module::register(app);
    atmosphere::register(app);
    crate::lan::register(app);
    app.init_resource::<crate::environment::Environment>();
    // Both dimensions' physics pipelines live side by side for live
    // cross-dimension scene switches; each only simulates its own bodies.
    app.insert_resource(bevy_rapier2d::prelude::TimestepMode::Fixed {
        dt: 1.0 / 60.0,
        substeps: 1,
    });
    app.insert_resource(bevy_rapier3d::prelude::TimestepMode::Fixed {
        dt: 1.0 / 60.0,
        substeps: 1,
    });
    app.add_plugins(
        bevy_rapier2d::prelude::RapierPhysicsPlugin::<dim2::OneWayHooks>::pixels_per_meter(
            dim2::PIXELS_PER_METER,
        )
        .in_fixed_schedule(),
    );
    app.add_plugins(
        bevy_rapier3d::prelude::RapierPhysicsPlugin::<physics_install::d3::Hooks3>::default()
            .in_fixed_schedule(),
    );
    app.configure_sets(
        FixedUpdate,
        (
            SimStep::Pause,
            SimStep::Timestep,
            SimStep::Sample,
            SimStep::PluginInput,
            SimStep::PluginPreSimulation,
            SimStep::Vm,
            SimStep::Scripts,
            SimStep::PluginFixed,
            SimStep::SavedData,
            SimStep::Verbs,
            SimStep::Lifetimes,
            SimStep::Common,
            SimStep::VolumeEffects,
            SimStep::ApplyEffects,
            SimStep::ComponentEffects,
            SimStep::PluginEffects,
            SimStep::Joints,
            SimStep::Animation,
            SimStep::ClearEffects,
            SimStep::FinishStep,
        )
            .chain()
            .in_set(world::SimulationSet),
    );
    // A client of an integrated server applies the server's effects but
    // never runs the blocks, scripts, plugins' gameplay or saves itself.
    app.configure_sets(
        FixedUpdate,
        (
            SimStep::PluginInput,
            SimStep::PluginPreSimulation,
            SimStep::Vm,
            SimStep::Scripts,
            SimStep::PluginFixed,
            SimStep::SavedData,
        )
            .run_if(is_authority),
    );
    app.configure_sets(
        FixedUpdate,
        world::SimulationSet.before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend),
    )
    .configure_sets(
        FixedUpdate,
        world::SimulationSet.before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend),
    );
    app.add_systems(
        FixedUpdate,
        (
            (dim2::sync_pause, dim3::sync_pause)
                .chain()
                .in_set(SimStep::Pause),
            (dim2::sync_timestep, dim3::sync_timestep)
                .chain()
                .in_set(SimStep::Timestep),
            (
                world::restore_poses,
                world::publish_simulation_actors,
                crate::tiles::publish_level.run_if(resource_exists::<crate::tiles::Level>),
                atmosphere::sample_atmosphere,
            )
                .chain()
                .in_set(SimStep::Sample),
            plugins::stage(Stage::Input).in_set(SimStep::PluginInput),
            plugins::stage(Stage::PreSimulation).in_set(SimStep::PluginPreSimulation),
            world::step_vm.in_set(SimStep::Vm),
            (world::step_scripts, ai::tick)
                .chain()
                .in_set(SimStep::Scripts),
            plugins::stage(Stage::FixedSimulation).in_set(SimStep::PluginFixed),
            world::apply_saved_data.in_set(SimStep::SavedData),
            crate::verbs::apply_property_writes.in_set(SimStep::Verbs),
            (world::apply_lifetimes, world::sync_navmesh)
                .chain()
                .in_set(SimStep::Lifetimes),
            (
                motor::drive_motors.run_if(is_authority),
                controller::apply_motion.run_if(is_authority),
                world::apply_common,
            )
                .chain()
                .in_set(SimStep::Common),
            volumes::apply_volume_effects.in_set(SimStep::VolumeEffects),
            (
                dim2::apply_effects,
                dim3::apply_effects,
                physics_install::d2::apply_forces.run_if(crate::is_2d),
                physics_install::d3::apply_forces.run_if(crate::is_3d),
            )
                .chain()
                .in_set(SimStep::ApplyEffects),
            world::apply_component_effects.in_set(SimStep::ComponentEffects),
            plugins::stage(Stage::EffectApplication).in_set(SimStep::PluginEffects),
            (dim2::sync_joints, dim3::sync_joints)
                .chain()
                .in_set(SimStep::Joints),
            (
                world::step_glides,
                world::step_tweens,
                crate::anim2d::apply_animation_effects,
                crate::anim2d::step_animations,
                crate::anim_layers::apply_layer_verbs,
                crate::anim_layers::step_layers,
                crate::ik3d::apply_look_verbs,
                crate::ragdoll::apply_ragdoll_verbs.run_if(crate::is_3d),
                crate::ragdoll::follow_ragdolls.run_if(crate::is_3d),
            )
                .chain()
                .in_set(SimStep::Animation),
            world::clear_effects.in_set(SimStep::ClearEffects),
            world::finish_step.in_set(SimStep::FinishStep),
        ),
    )
    .add_systems(
        FixedPostUpdate,
        (
            plugins::stage(Stage::PostPhysics),
            world::apply_parenting,
            dim2::record_poses,
            dim3::record_poses,
            dim2::track_contacts.run_if(crate::is_2d),
            dim3::track_contacts.run_if(crate::is_3d),
            crate::record::record,
        )
            .chain()
            // Unordered against the writeback, a pose could be read before or
            // after physics wrote it, which made replays drift.
            .after(bevy_rapier2d::prelude::PhysicsSet::Writeback)
            .after(bevy_rapier3d::prelude::PhysicsSet::Writeback),
    )
    // Rapier propagates transforms before it reads kinematic poses, but only
    // trees marked dirty, and Bevy marks them once a frame. Mark every step,
    // or a moved parent's second step in a frame syncs last frame's pose.
    .add_systems(
        FixedPostUpdate,
        bevy::transform::systems::mark_dirty_trees
            .before(bevy_rapier2d::plugin::RapierTransformPropagateSet)
            .before(bevy_rapier3d::plugin::RapierTransformPropagateSet),
    )
    .add_systems(
        FixedUpdate,
        (
            physics_install::d2::clamp_velocities.run_if(crate::is_2d),
            physics_install::d3::clamp_velocities.run_if(crate::is_3d),
            physics_install::d2::refresh_masses.run_if(crate::is_2d),
            physics_install::d3::refresh_masses.run_if(crate::is_3d),
        )
            .before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend)
            .before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend),
    )
    .add_systems(
        FixedUpdate,
        (
            constraints::d2::drive.run_if(crate::is_2d),
            constraints::d3::drive.run_if(crate::is_3d),
        )
            .after(bevy_rapier2d::prelude::PhysicsSet::Writeback)
            .after(bevy_rapier3d::prelude::PhysicsSet::Writeback),
    );
}

/// Whether this world runs its own gameplay, rather than following a server.
pub(crate) fn is_authority(following: Option<Res<crate::server::Following>>) -> bool {
    following.is_none()
}

/// The simulation alone, drawing nothing, stepped one fixed tick per
/// `update` and already running: the playmode test runner and the tests.
/// `rebuild` builds the world from the document the way Play does; without
/// it, actors are spawned bare for a test to dress.
pub(crate) fn headless(mut engine: Engine, mode: Mode, rebuild: bool) -> App {
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    let actors = engine.project.actors.clone();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TransformPlugin);
    // The simulation draws nothing, so the only asset store is the mesh
    // one Rapier's own async colliders read.
    app.add_plugins(AssetPlugin::default());
    app.init_asset::<Mesh>()
        .init_resource::<crate::performance::RenderCache>()
        .init_resource::<crate::ui::UiManager>()
        .init_resource::<crate::sound::SoundState>();
    let rate = engine.project.world.fixed_rate;
    let rate = if rate.is_finite() {
        rate.clamp(1.0, 1000.0) as f64
    } else {
        60.0
    };
    app.insert_resource(Time::<Fixed>::from_hz(rate));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / rate,
    )));
    if !rebuild {
        for actor in &actors {
            let entity = app.world_mut().spawn(world::actor_bundle(actor)).id();
            engine.entities.insert(actor.id.clone(), entity);
        }
    }
    engine.rebuild = rebuild;
    world::begin_run(&mut engine, 0.0, 0.0);
    add_simulation(&mut app, mode, engine);
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use blockloom_core::blocks::{Instruction, InstructionKind, Strand};
    use blockloom_core::project::{Actor, Project};
    use blockloom_core::scene::Visual;
    use blockloom_core::value::{Op, Value};
    use std::time::Duration;

    /// A project with one actor whose canvas moves it a step every tick.
    fn mover(mode: Mode, steps: f64) -> Project {
        let mut actor = Actor::new(
            "Player",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(InstructionKind::WhenStarted),
                Instruction::new(InstructionKind::Forever {
                    body: vec![Instruction::new(InstructionKind::Move {
                        steps: Value::number(steps),
                    })],
                }),
            ],
        )];
        let mut project = Project::starter("headless", mode);
        project.actors = vec![actor];
        project
    }

    /// The simulation alone: no window, renderer, asset server or asset
    /// stores. One `app.update()` after the first is one
    /// fixed step at 60 Hz. Schedules the blocks through native logic compiled into `native` when
    /// given a folder, and through the VM otherwise.
    fn headless_with(project: Project, mode: Mode, native: Option<&std::path::Path>) -> App {
        headless_app(project, mode, native, false)
    }

    /// `headless_with`, but the world builds itself from the document the way
    /// a run does (components attached, bodies and motors installed).
    fn headless_built(project: Project, mode: Mode, native: Option<&std::path::Path>) -> App {
        headless_app(project, mode, native, true)
    }

    fn headless_app(
        project: Project,
        mode: Mode,
        native: Option<&std::path::Path>,
        rebuild: bool,
    ) -> App {
        blockloom_core::value::register_blockloom_operators();
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, mode);
        engine.project = project;
        let loaded = engine.project.clone();
        engine.vm.load(&loaded);
        if let Some(root) = native {
            blockloom_core::codegen::compile_for(&loaded, root, None).expect("compile the blocks");
            engine.variables.load(&loaded);
            engine.lists.load(&loaded);
            engine.dicts.load(&loaded);
            engine.logic = Some(crate::logic::LoadedLogic::load(root).expect("load the library"));
        }
        headless(engine, mode, rebuild)
    }

    /// The simulation building its own world from the document, the way a
    /// headless server would: no presentation registered, so every actor is
    /// an entity with a pose and a body, and a look nobody draws.
    #[test]
    fn a_headless_world_rebuilds_itself_without_drawing() {
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let mut project = mover(mode, 1.0);
            if mode.is_3d() {
                project.actors[0].components.set_visual(Visual::Cuboid {
                    color: "#FFFFFF".to_string(),
                    size: [1.0, 1.0, 1.0],
                });
            }
            blockloom_core::value::register_blockloom_operators();
            let (_sender, incoming) = std::sync::mpsc::channel();
            let mut engine = Engine::new(incoming, mode);
            engine.project = project;
            engine.rebuild = true;
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            app.add_plugins(TransformPlugin);
            app.add_plugins(AssetPlugin::default());
            app.init_asset::<Mesh>()
                .init_resource::<crate::ui::UiManager>()
                .init_resource::<crate::sound::SoundState>();
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )));
            add_simulation(&mut app, mode, engine);
            app.update();
            let mut actors = app
                .world_mut()
                .query::<(&crate::engine::ActorId, Option<&world::PendingLook>)>();
            let found: Vec<_> = actors
                .iter(app.world())
                .map(|(id, look)| (id.0.clone(), look.is_some()))
                .collect();
            assert_eq!(found.len(), 1, "{mode:?}: {found:?}");
            assert!(found[0].1, "{mode:?}: the look waits for presentation");
            let mut drawn = app
                .world_mut()
                .query_filtered::<Entity, Or<(With<Sprite>, With<Mesh3d>)>>();
            assert_eq!(drawn.iter(app.world()).count(), 0, "{mode:?}");
        }
    }

    fn position_after(mode: Mode, steps: usize) -> Vec3 {
        position_with(mode, steps, None)
    }

    fn position_with(mode: Mode, steps: usize, native: Option<&std::path::Path>) -> Vec3 {
        let mut app = headless_with(mover(mode, 2.0), mode, native);
        // The first update only starts the clock.
        for _ in 0..=steps {
            app.update();
        }
        let mut poses = app
            .world_mut()
            .query::<(&crate::engine::ActorId, &Transform)>();
        let (_, pose) = poses.single(app.world()).unwrap();
        pose.translation
    }

    #[test]
    fn a_patch_respawns_only_the_actor_it_names() {
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let mut project = mover(mode, 0.0);
            let other = Actor::new(
                "Other",
                Visual::Rect {
                    color: "#000000".to_string(),
                    size: [5.0, 5.0],
                },
            );
            project.actors.push(other.clone());
            let mut app = headless_with(project, mode, None);
            app.add_systems(Update, world::respawn_actors);
            app.update();
            let (player, kept) = {
                let engine = app.world().non_send::<Engine>();
                (
                    engine.project.actors[0].id.clone(),
                    engine.entities[&other.id],
                )
            };
            {
                let mut engine = app.world_mut().non_send_mut::<Engine>();
                engine.running = false;
                let before = engine.project.clone();
                let mut after = before.clone();
                after.actors[0].components.placement_mut().position = [40.0, 0.0, 0.0];
                let change = blockloom_core::change::diff(&before, &after);
                change.apply(&mut engine.project, true);
                let ids = world::respawnable(&engine, &change).expect("a plain edit");
                assert_eq!(ids, std::slice::from_ref(&player));
                engine.respawn = ids;
            }
            app.update();
            let engine = app.world().non_send::<Engine>();
            assert_eq!(
                engine.entities[&other.id], kept,
                "the other actor kept its entity"
            );
            let moved = engine.entities[&player];
            let at = app.world().get::<Transform>(moved).unwrap().translation;
            assert_eq!(at.x, 40.0, "{mode:?}");
            let actors = app
                .world_mut()
                .query::<&crate::engine::ActorId>()
                .iter(app.world())
                .count();
            assert_eq!(actors, 2);
        }
    }

    #[test]
    fn a_headless_2d_world_runs_the_blocks_and_moves_the_actor() {
        let at = position_after(Mode::TwoD, 30);
        assert!(at.x > 20.0, "moved to {at}");
    }

    #[test]
    fn a_headless_3d_world_runs_the_blocks_and_moves_the_actor() {
        let at = position_after(Mode::ThreeD, 30);
        assert!(at.length() > 0.1, "moved to {at}");
    }

    #[test]
    fn the_same_project_steps_to_the_same_place_every_run() {
        for mode in [Mode::TwoD, Mode::ThreeD] {
            assert_eq!(position_after(mode, 90), position_after(mode, 90));
        }
    }

    #[derive(Resource, Default)]
    struct Samples(Vec<f32>);

    fn capture_actor_sample(_engine: NonSend<Engine>, mut samples: ResMut<Samples>) {
        samples.0.push(blockloom_core::sense::read(|s| {
            s.actors.values().next().unwrap().position[0]
        }));
    }

    fn reporter_samples(frame: Duration, frames: usize) -> Vec<f32> {
        let mut project = mover(Mode::TwoD, 0.0);
        let body = Instruction::new(InstructionKind::Move {
            steps: Value::op(
                Op::from_name("Add"),
                vec![
                    Value::op(Op::from_name("MyPosition"), vec![Value::text("X")]),
                    Value::number(1.0),
                ],
            ),
        });
        project.actors[0].graph.strands[0].instructions[1] =
            Instruction::new(InstructionKind::Forever { body: vec![body] });
        let mut app = headless_with(project, Mode::TwoD, None);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(frame));
        app.init_resource::<Samples>();
        app.add_systems(
            FixedUpdate,
            capture_actor_sample
                .after(SimStep::Sample)
                .before(SimStep::Vm),
        );
        for _ in 0..=frames {
            app.update();
        }
        app.world().resource::<Samples>().0.clone()
    }

    #[test]
    fn actor_reporters_observe_each_tick_even_when_ticks_are_batched() {
        let step = Duration::from_secs_f64(1.0 / 60.0);
        let individual = reporter_samples(step, 6);
        let batched = reporter_samples(step * 3, 2);
        assert_eq!(individual, vec![0.0, 1.0, 3.0, 7.0, 15.0, 31.0]);
        assert_eq!(batched, individual);
    }

    #[derive(Resource, Default)]
    struct RunSamples(Vec<(f64, f64, bool, String, Vec<String>)>);

    fn capture_run_sample(_engine: NonSend<Engine>, mut samples: ResMut<RunSamples>) {
        samples.0.push(blockloom_core::sense::read(|s| {
            (
                s.time,
                s.wall_time,
                s.paused,
                s.current_scene.clone(),
                s.scene_names.clone(),
            )
        }));
    }

    fn run_samples(frame: Duration, frames: usize, paused: bool) -> RunSamples {
        let mut project = mover(Mode::TwoD, 1.0);
        project.actors[0].graph.strands[0].instructions[1] =
            Instruction::new(InstructionKind::Forever {
                body: vec![Instruction::new(InstructionKind::Move {
                    steps: Value::op(Op::from_name("Timer"), vec![]),
                })],
            });
        let mut app = headless_with(project, Mode::TwoD, None);
        if paused {
            world::set_paused(&mut app.world_mut().non_send_mut::<Engine>(), true, 0.0);
        }
        app.insert_resource(TimeUpdateStrategy::ManualDuration(frame));
        app.init_resource::<RunSamples>();
        app.add_systems(
            FixedUpdate,
            capture_run_sample
                .after(SimStep::Vm)
                .before(SimStep::Scripts),
        );
        for _ in 0..=frames {
            app.update();
        }
        let expected: f64 = app
            .world()
            .resource::<RunSamples>()
            .0
            .iter()
            .map(|s| s.0)
            .sum();
        let mut poses = app
            .world_mut()
            .query_filtered::<&Transform, With<crate::engine::ActorId>>();
        let moved = poses.single(app.world()).unwrap().translation.x as f64;
        assert!(
            (moved - expected).abs() < 1e-6,
            "timer-driven movement: {moved} vs {expected}"
        );
        app.world_mut().remove_resource::<RunSamples>().unwrap()
    }

    #[test]
    fn headless_run_context_advances_on_every_logical_tick() {
        let step = Duration::from_secs_f64(1.0 / 60.0);
        let individual = run_samples(step, 6, false).0;
        let batched = run_samples(step * 3, 2, false).0;
        assert_eq!(individual.len(), 6);
        assert_eq!(batched.len(), 6);
        let scene = Project::starter("headless", Mode::TwoD)
            .active_scene()
            .name
            .clone();
        for (index, (single, batch)) in individual.iter().zip(&batched).enumerate() {
            let expected = step.as_secs_f64() * (index + 1) as f64;
            assert!((single.0 - expected).abs() < 1e-8);
            assert_eq!(single.0, batch.0);
            assert!(!single.2);
            assert_eq!(single.3, scene);
            assert_eq!(single.4, vec![scene.clone()]);
            assert_eq!(single.3, batch.3);
            assert_eq!(single.4, batch.4);
            assert!(batch.1 >= batch.0);
        }
    }

    #[test]
    fn headless_pause_freezes_game_reporters_but_keeps_real_time() {
        let step = Duration::from_secs_f64(1.0 / 60.0);
        let samples = run_samples(step * 3, 2, true).0;
        assert_eq!(samples.len(), 6);
        for sample in &samples {
            assert_eq!(sample.0, 0.0);
            assert!(sample.2);
        }
        assert!(samples[0].1 > 0.0);
        assert!(samples[3].1 > samples[0].1);
        assert_eq!(samples[0].1, samples[2].1);
    }

    #[cfg(all(
        feature = "multiplayer",
        not(target_arch = "wasm32"),
        not(target_os = "android")
    ))]
    #[test]
    fn lan_capability_and_limit_are_fixed_for_the_run() {
        let mut app = headless_with(mover(Mode::TwoD, 2.0), Mode::TwoD, None);
        let mut e = app.world_mut().non_send_mut::<Engine>();
        assert!(!crate::lan::status(&e).enabled);
        e.project.multiplayer.enabled = true;
        e.project.multiplayer.max_guests = 2;
        crate::lan::open(&mut e, "127.0.0.1:0", 1);
        app.update();
        assert!(!crate::lan::status(app.world().non_send::<Engine>()).open);
        let mut e = app.world_mut().non_send_mut::<Engine>();
        world::begin_run(&mut e, 0.0, 0.0);
        assert!(crate::lan::status(&e).enabled);
        assert_eq!(crate::lan::status(&e).max_guests, 2);
        e.project.multiplayer.enabled = false;
        e.project.multiplayer.max_guests = 16;
        crate::lan::open(&mut e, "127.0.0.1:0", 3);
        app.update();
        assert!(!crate::lan::status(app.world().non_send::<Engine>()).open);
        crate::lan::open(
            &mut app.world_mut().non_send_mut::<Engine>(),
            "127.0.0.1:0",
            2,
        );
        app.update();
        assert!(crate::lan::status(app.world().non_send::<Engine>()).open);
        crate::lan::close(&mut app.world_mut().non_send_mut::<Engine>());
        assert!(crate::lan::status(app.world().non_send::<Engine>()).enabled);
    }

    #[cfg(all(
        feature = "multiplayer",
        not(target_arch = "wasm32"),
        not(target_os = "android")
    ))]
    #[test]
    fn lan_attaches_replicates_and_closes_without_restarting_either_dimension() {
        use blockloom_net::ClientOptions;
        use blockloom_net::game::{Invite, LanClient};
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let mut project = mover(mode, 2.0);
            project.multiplayer.enabled = true;
            let mut app = headless_with(project, mode, None);
            for _ in 0..31 {
                app.update();
            }
            let started = app.world().non_send::<Engine>().started_at;
            let before = app.world().non_send::<Engine>().contact_ticks;
            crate::lan::open(
                &mut app.world_mut().non_send_mut::<Engine>(),
                "127.0.0.1:0",
                4,
            );
            app.update();
            let status = crate::lan::status(app.world().non_send::<Engine>());
            assert!(status.open);
            let invite = Invite::decode(status.invite.as_ref().unwrap()).unwrap();
            let mut guest =
                LanClient::connect(invite.clone(), invite.build, ClientOptions::default()).unwrap();
            let end = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                app.update();
                guest.poll().unwrap();
                if guest.state().is_some() {
                    break;
                }
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(Duration::from_millis(1));
            }
            let state = guest.state().unwrap();
            assert_eq!(state.actors.len(), 1);
            assert_eq!(state.dimension, u8::from(mode == Mode::ThreeD));
            let pose = state.actors.values().next().unwrap().pose;
            assert!(Vec3::from_slice(&pose[..3]).length() > 20.0);
            assert!(app.world().non_send::<Engine>().contact_ticks > before);
            assert_eq!(app.world().non_send::<Engine>().started_at, started);
            let tick = app.world().non_send::<Engine>().contact_ticks;
            crate::lan::close(&mut app.world_mut().non_send_mut::<Engine>());
            assert!(!crate::lan::status(app.world().non_send::<Engine>()).open);
            app.update();
            assert!(app.world().non_send::<Engine>().running);
            assert_eq!(app.world().non_send::<Engine>().started_at, started);
            assert!(app.world().non_send::<Engine>().contact_ticks > tick);
        }
    }

    #[cfg(all(
        feature = "multiplayer",
        feature = "plugins",
        not(target_arch = "wasm32"),
        not(target_os = "android")
    ))]
    #[test]
    fn lan_guests_get_the_hosts_voxel_edits_and_rebuild_the_world_from_them() {
        use blockloom_net::ClientOptions;
        use blockloom_net::game::{Invite, LanClient};
        use blockloom_plugin_api::record::PluginRecord;
        use blockloom_plugin_host::{module::CodeModule, native::NativeModule, world::Preloaded};
        use serde_json::json;
        const ID: &str = "com.blockworked.voxel";
        let voxel = || {
            unsafe {
                NativeModule::from_entry(
                    blockloom_voxel::blockloom_plugin_entry_v1,
                    Default::default(),
                    blockloom_plugin_host::native::default_services("0.0.1".into()),
                )
            }
            .unwrap()
        };
        let settings = json!({"preset":"island","seed":2,"size":[64,24,64],"solid":false});
        let mut project = mover(Mode::ThreeD, 2.0);
        project.multiplayer.enabled = true;
        project
            .plugin_resources
            .push(PluginRecord::new(ID, "world", 1, settings));
        let mut app = headless_with(project, Mode::ThreeD, None);
        app.update();
        let world = blockloom_plugin_host::world::WorldPlugins::with_modules(vec![Preloaded {
            id: ID.to_string(),
            module: CodeModule::Native(voxel()),
            hooks: vec![],
            blocks: vec![],
        }])
        .unwrap();
        crate::plugins::install(&mut app.world_mut().non_send_mut::<Engine>(), world);
        let edit = |app: &App, op: &str, args: serde_json::Value| {
            crate::plugins::query_for_test(app.world().non_send::<Engine>(), ID, op, &args);
        };
        edit(
            &app,
            "sphere",
            json!({"x":30,"y":12,"z":30,"radius":6,"material":"air"}),
        );
        crate::lan::open(
            &mut app.world_mut().non_send_mut::<Engine>(),
            "127.0.0.1:0",
            2,
        );
        app.update();
        let status = crate::lan::status(app.world().non_send::<Engine>());
        assert!(status.open, "{status:?}");
        let invite = Invite::decode(status.invite.as_ref().unwrap()).unwrap();
        // The guest view hosts its own copy of the plugin, as a local build would.
        let client =
            LanClient::connect(invite.clone(), invite.build, ClientOptions::default()).unwrap();
        let copy = blockloom_plugin_host::world::WorldPlugins::with_modules(vec![Preloaded {
            id: ID.to_string(),
            module: CodeModule::Native(voxel()),
            hooks: vec![],
            blocks: vec![],
        }])
        .unwrap();
        let project = app.world().non_send::<Engine>().project.clone();
        let mut view = crate::lan::guest::test_app(
            client,
            Some(crate::lan::guest::GuestPlugins::hosting(copy, &project)),
        );
        let guest_get = |view: &mut App, at: &serde_json::Value| {
            view.world_mut()
                .non_send_mut::<crate::lan::guest::GuestPlugins>()
                .query(ID, "get", at)
        };
        let follow = |app: &mut App, view: &mut App, cell: [i32; 3]| {
            let end = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                app.update();
                view.update();
                let at = json!({"x":cell[0],"y":cell[1],"z":cell[2]});
                let host = crate::plugins::query_for_test(
                    app.world().non_send::<Engine>(),
                    ID,
                    "get",
                    &at,
                );
                let mine = guest_get(view, &at);
                if host["material"] == mine["material"] && host["shape"] == mine["shape"] {
                    return;
                }
                assert!(
                    std::time::Instant::now() < end,
                    "the guest's copy never caught up at {cell:?}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        // The carved sphere reaches the guest's copy, which draws its chunks.
        follow(&mut app, &mut view, [30, 12, 30]);
        let names = |view: &App| -> Vec<String> {
            view.world()
                .non_send::<crate::lan::guest::GuestPlugins>()
                .mesh_names()
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        assert!(names(&view).iter().any(|n| n.starts_with("chunk/")));
        // So does an edit made while the guest watches, glowing faces included.
        edit(
            &app,
            "fill",
            json!({"x1":0,"y1":22,"z1":0,"x2":63,"y2":22,"z2":63,"material":"glow"}),
        );
        follow(&mut app, &mut view, [63, 22, 63]);
        let end = std::time::Instant::now() + Duration::from_secs(2);
        while !names(&view).iter().any(|n| n.contains("glow")) {
            assert!(std::time::Instant::now() < end, "no glow mesh drawn");
            app.update();
            view.update();
        }
        for x in (0..64).step_by(3) {
            for y in 0..24 {
                for z in (0..64).step_by(3) {
                    let at = json!({"x":x,"y":y,"z":z});
                    let host = crate::plugins::query_for_test(
                        app.world().non_send::<Engine>(),
                        ID,
                        "get",
                        &at,
                    );
                    assert_eq!(
                        host["material"],
                        guest_get(&mut view, &at)["material"],
                        "{at}"
                    );
                }
            }
        }
        crate::lan::close(&mut app.world_mut().non_send_mut::<Engine>());
    }

    #[test]
    fn compiled_logic_and_the_vm_step_a_world_to_the_same_place() {
        if blockloom_core::script::toolchain_version().is_err() {
            return;
        }
        for mode in [Mode::TwoD, Mode::ThreeD] {
            let root = std::env::temp_dir().join(format!(
                "blockloom-headless-logic-{}-{mode:?}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let compiled = position_with(mode, 60, Some(&root));
            assert_eq!(compiled, position_after(mode, 60), "{mode:?}");
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// The third-person template driven by its own blocks: what its animator
    /// went through each tick, and its footstep count.
    fn template_run(native: Option<&std::path::Path>) -> (Vec<String>, Vec<Vec<String>>, f64) {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::components::ActorComponent;
        use blockloom_core::physics::motor::{MotorAction, MotorOwner, MoveSpace};
        use blockloom_core::value::Value;
        let mut project =
            blockloom_core::physics::sample::build("third-person", "T", Mode::ThreeD).unwrap();
        let player = project
            .actors
            .iter_mut()
            .find(|a| a.name == "Player")
            .unwrap();
        // Blocks steer it: forward, a jump, then a sprint.
        let mut motor = player.components.character_motor().cloned().unwrap();
        motor.owner = MotorOwner::Script;
        motor.space = MoveSpace::World;
        player
            .components
            .insert(ActorComponent::CharacterMotor { motor });
        let act = |action: MotorAction, z: f64| K::MotorAct {
            action,
            x: Value::number(0.0),
            y: Value::number(0.0),
            z: Value::number(z),
        };
        let wait = |seconds: f64| K::Wait {
            duration: Value::number(seconds),
        };
        player.graph.strands.push(Strand::with_instructions(
            960,
            0,
            [
                K::WhenStarted,
                act(MotorAction::Intent, -1.0),
                wait(1.5),
                act(MotorAction::Jump, 0.0),
                wait(1.5),
                act(MotorAction::SprintOn, 0.0),
            ]
            .into_iter()
            .map(Instruction::new)
            .collect(),
        ));
        let id = player.id.clone();
        let mut app = headless_built(project, Mode::ThreeD, native);
        let mut states: Vec<String> = Vec::new();
        let mut blends: Vec<Vec<String>> = Vec::new();
        for _ in 0..=300 {
            app.update();
            let mut players = app
                .world_mut()
                .query::<(&crate::engine::ActorId, &crate::engine::AnimationPlayer)>();
            let Some((_, playing)) = players.iter(app.world()).find(|(a, _)| a.0 == id) else {
                continue;
            };
            if states.last() != Some(&playing.state) {
                states.push(playing.state.clone());
            }
            let mut mixed: Vec<String> = playing
                .blend
                .iter()
                .filter(|(_, w)| *w > 0.05)
                .map(|(clip, _)| clip.clone())
                .collect();
            mixed.sort();
            if blends.last() != Some(&mixed) {
                blends.push(mixed);
            }
        }
        let steps = app
            .world()
            .non_send::<Engine>()
            .variables
            .read(&id, "steps")
            .as_number()
            .unwrap_or(0.0);
        (states, blends, steps)
    }

    /// Phase D's check: the template walks, runs, jumps and lands on the
    /// blended animator, and its footsteps reach the blocks; the VM and the
    /// compiled logic agree on all of it.
    #[test]
    fn the_third_person_template_walks_jumps_lands_and_runs() {
        let (states, blends, steps) = template_run(None);
        let locomotion = blockloom_core::animation::LOCOMOTION.to_string();
        let jump = states.iter().position(|s| s == "Jump").expect("it jumped");
        assert_eq!(states[0], locomotion, "{states:?}");
        assert!(
            states[jump + 1..].contains(&locomotion),
            "it landed: {states:?}"
        );
        // It walks, then the sprint blends walk into run on the way to a run.
        let seen = |clip: &str| blends.iter().any(|mix| mix.iter().any(|c| c == clip));
        assert!(seen("Walk") && seen("Run"), "{blends:?}");
        assert!(blends.iter().any(|mix| mix.len() == 2), "{blends:?}");
        assert!(steps >= 4.0, "{steps} footsteps");
        if blockloom_core::script::toolchain_version().is_err() {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("blockloom-template-logic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let compiled = template_run(Some(&root));
        let _ = std::fs::remove_dir_all(root);
        assert_eq!(compiled, (states, blends, steps));
    }

    /// Plays `sample` headless with the player's motor driven by blocks in
    /// world space along `intent` from the start, and reads a global after
    /// `ticks` fixed steps.
    fn play_template(sample: &str, mode: Mode, intent: [f64; 3], ticks: usize, read: &str) -> f64 {
        use blockloom_core::blocks::{Instruction, InstructionKind as K, Strand};
        use blockloom_core::components::ActorComponent;
        use blockloom_core::physics::motor::{MotorAction, MotorOwner, MoveSpace};
        let mut project = blockloom_core::physics::sample::build(sample, "T", mode).unwrap();
        let player = project
            .actors
            .iter_mut()
            .find(|a| a.name == "Player")
            .unwrap();
        let mut motor = player.components.character_motor().cloned().unwrap();
        motor.owner = MotorOwner::Script;
        motor.space = MoveSpace::World;
        player
            .components
            .insert(ActorComponent::CharacterMotor { motor });
        player.graph.strands.push(Strand::with_instructions(
            960,
            0,
            [
                K::WhenStarted,
                K::MotorAct {
                    action: MotorAction::Intent,
                    x: Value::number(intent[0]),
                    y: Value::number(intent[1]),
                    z: Value::number(intent[2]),
                },
            ]
            .into_iter()
            .map(Instruction::new)
            .collect(),
        ));
        let mut app = headless_built(project, mode, None);
        for _ in 0..ticks {
            app.update();
        }
        app.world()
            .non_send::<Engine>()
            .variables
            .read("", read)
            .as_number()
            .unwrap_or(0.0)
    }

    /// Phase E's genre templates play: walking into the first pickup counts it.
    #[test]
    fn the_genre_templates_count_their_first_pickup() {
        assert_eq!(
            play_template("platformer", Mode::TwoD, [1.0, 0.0, 0.0], 90, "coins"),
            1.0
        );
        assert_eq!(
            play_template("top-down", Mode::TwoD, [1.0, 0.0, 0.0], 60, "gems"),
            1.0
        );
        assert_eq!(
            play_template("first-person", Mode::ThreeD, [0.0, 0.0, -1.0], 90, "orbs"),
            1.0
        );
        // Nothing to pick up standing still.
        assert_eq!(
            play_template("platformer", Mode::TwoD, [0.0; 3], 90, "coins"),
            0.0
        );
    }

    /// The interface puzzle answers clicks and knows when it is solved.
    #[test]
    fn the_ui_puzzle_is_solved_by_its_solution() {
        use blockloom_core::physics::genres::LIGHTS_SOLUTION;
        use blockloom_core::vm::Event;
        let project = blockloom_core::physics::sample::build("ui-puzzle", "T", Mode::TwoD).unwrap();
        let mut app = headless_built(project, Mode::TwoD, None);
        let read = |app: &App, name: &str| {
            app.world()
                .non_send::<Engine>()
                .variables
                .read("", name)
                .as_number()
                .unwrap_or(-1.0)
        };
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(read(&app, "solved"), 0.0);
        for cell in LIGHTS_SOLUTION {
            app.world_mut()
                .non_send_mut::<Engine>()
                .fire(Event::UiClicked {
                    id: format!("cell {cell}"),
                });
            for _ in 0..5 {
                app.update();
            }
        }
        assert_eq!(read(&app, "moves"), 3.0);
        assert_eq!(read(&app, "solved"), 1.0);
    }
}
