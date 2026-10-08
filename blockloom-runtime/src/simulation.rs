//! The simulation half of the world's registration: the fixed-step chain, the
//! physics pipelines and what they need, with nothing that draws, plays or
//! reads a window. `add_world` calls this and then adds the presentation
//! systems around it, so a server can register the same simulation alone.
//!
//! The fixed step used to be one chain mixing both kinds. Each simulation
//! system now sits in a `SimStep`, the steps are chained in the old order, and
//! presentation systems order themselves against the steps they used to sit
//! between (`add_presentation` in lib.rs), so the order is unchanged.

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
    app.insert_resource(Dimension(mode))
        .init_resource::<PendingEffects>()
        .init_resource::<world::NavMesh>()
        .init_resource::<crate::player_camera::BodyFacing>()
        .init_resource::<physics_install::PhysicsLayers>()
        .insert_non_send(engine);
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
            (world::apply_lifetimes, world::sync_navmesh)
                .chain()
                .in_set(SimStep::Lifetimes),
            (
                motor::drive_motors,
                controller::apply_motion,
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
        )
            .chain(),
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

    /// The simulation alone: no window or renderer, and the asset stores are
    /// empty CPU-side collections. One `app.update()` after the first is one
    /// fixed step at 60 Hz. Schedules the blocks through native logic compiled into `native` when
    /// given a folder, and through the VM otherwise.
    fn headless_with(project: Project, mode: Mode, native: Option<&std::path::Path>) -> App {
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
        let actors = engine.project.actors.clone();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<crate::materials::GraphMaterial2d>()
            .init_asset::<crate::materials::GraphMaterial3d>()
            .init_asset::<bevy::sprite_render::ColorMaterial>()
            .init_resource::<crate::performance::RenderCache>()
            .init_resource::<crate::ui::UiManager>()
            .init_resource::<crate::sound::SoundState>();
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        for actor in &actors {
            let entity = app.world_mut().spawn(world::actor_bundle(actor)).id();
            engine.entities.insert(actor.id.clone(), entity);
        }
        engine.rebuild = false;
        world::begin_run(&mut engine, 0.0, 0.0);
        add_simulation(&mut app, mode, engine);
        app
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
}
