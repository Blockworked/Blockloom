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
            (world::restore_poses, atmosphere::sample_atmosphere)
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
    use blockloom_core::value::Value;
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

    /// TEMPORARY repro for the Voxelvale freeze. DELETE AFTER USE.
    #[test]
    fn voxelvale_scene1_ticks_pace_and_pause_answers() {
        use std::time::{Duration, Instant};
        let dir =
            std::path::Path::new("/home/treetrain1/Blockloom/projects/Voxelvale");
        let mut project =
            blockloom_core::project::read_project_dir(dir).expect("read project");
        project.active_scene = "d1d9f2c015b844f0a407adf97afb0e2d".to_string();
        let mode = Mode::ThreeD;
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, mode);
        engine.project = project;
        let loaded = engine.project.clone();
        engine.vm.load(&loaded);
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
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f64(1.0 / 60.0),
        ));
        for actor in &actors {
            let entity = app
                .world_mut()
                .spawn(world::actor_bundle(actor))
                .id();
            engine.entities.insert(actor.id.clone(), entity);
        }
        world::begin_run(&mut engine, 0.0, 0.0);
        engine.fire(blockloom_core::vm::Event::SceneStarted);
        add_simulation(&mut app, mode, engine);
        let start = Instant::now();
        let mut last = start;
        for i in 0..900 {
            app.update();
            if i % 100 == 99 {
                let now = Instant::now();
                eprintln!("tick {} at {:?} (+{:?})", i + 1, now - start, now - last);
                last = now;
            }
            if i == 300 {
                app.world_mut()
                    .non_send_mut::<Engine>()
                    .fire(blockloom_core::vm::Event::Key("escape".to_string()));
            }
        }
        let total = start.elapsed();
        let paused = app.world().non_send::<Engine>().paused;
        eprintln!("900 ticks in {total:?}, paused={paused}");
        assert!(
            total < Duration::from_secs(120),
            "ticks wedged/spiralling: {total:?}"
        );
        assert!(paused, "escape did not pause: input routing dead");
    }
}
