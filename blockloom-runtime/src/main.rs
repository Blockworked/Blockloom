//! Blockloom's game world.
//!
//! A Bevy app that renders one project and runs its blocks. It is a separate
//! process from the editor because Bevy needs its own window and event loop and
//! the editor's CEF runtime already owns one; the editor spawns this and talks
//! to it over its pipes (see `blockloom-protocol`). Which dimension to build is
//! a launch argument, since the plugin set differs: the editor restarts the
//! runtime when a project changes mode.
//!
//! Usage: `blockloom-runtime [--mode 2d|3d]`.

// A Bevy system declares every query and resource it touches as an argument, so
// the usual argument-count limit doesn't apply here.
#![allow(clippy::too_many_arguments)]

mod bridge;
mod dim2;
mod dim3;
mod engine;
mod overlay;
mod script;
mod world;

use bevy::asset::{AssetPlugin, UnapprovedPathMode};
use bevy::prelude::*;
use bevy::window::WindowResolution;
use blockloom_core::scene::Mode;
use blockloom_protocol::{PROTOCOL_VERSION, RuntimeMessage};
use engine::{Dimension, Engine, PendingEffects};

fn main() {
    let mode = mode_from_args(std::env::args().skip(1));
    blockloom_core::init();

    let incoming = bridge::listen();
    let mut app = App::new();
    // Project assets live in the project's own folder, anywhere on disk, and
    // are handed to the asset server as absolute paths. Those are unapproved
    // by default in Bevy 0.19 (`Forbid`), which fails the load and leaves a
    // white sprite - so allow them here. The files are the user's own.
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Blockloom".to_string(),
            resolution: WindowResolution::new(960, 720),
            ..default()
        }),
        ..default()
    }).set(AssetPlugin {
        unapproved_path_mode: UnapprovedPathMode::Allow,
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.11, 0.14, 0.19)))
    .insert_resource(Dimension(mode))
    .init_resource::<PendingEffects>()
    .insert_non_send(Engine::new(incoming, mode))
    .add_systems(Startup, (overlay::spawn, announce_ready));

    // Only the dimension in use gets a physics pipeline: two would simulate
    // the same actors twice. Simulation runs on Bevy's `FixedUpdate` - a
    // constant-rate step whatever the display does - while input, sensors and
    // rendering stay on the per-frame `Update`.
    match mode {
        Mode::TwoD => {
            // The plugin starts its timestep mode at a fixed 60Hz; the loaded
            // project's own rate takes over on the first step.
            app.insert_resource(bevy_rapier2d::prelude::TimestepMode::Fixed {
                dt: 1.0 / 60.0,
                substeps: 1,
            });
            app.add_plugins(bevy_rapier2d::prelude::RapierPhysicsPlugin::<
                bevy_rapier2d::prelude::NoUserData,
            >::pixels_per_meter(dim2::PIXELS_PER_METER)
            .in_fixed_schedule())
                .add_systems(
                    FixedUpdate,
                    (
                        dim2::sync_pause,
                        dim2::sync_timestep,
                        world::step_vm,
                        world::step_scripts,
                        world::apply_common,
                        dim2::apply_effects,
                        world::apply_component_effects,
                        world::step_glides,
                        world::clear_effects,
                    )
                        .chain()
                        .in_set(world::SimulationSet),
                )
                .add_systems(FixedPostUpdate, dim2::record_poses)
                .add_systems(
                    Update,
                    (
                        world::pump_editor,
                        world::rebuild_world,
                        dim2::relay_collisions,
                        world::publish_sensors,
                        world::detect_clicks,
                        world::interpolate_poses,
                        world::drive_camera,
                        overlay::update_speech_bubbles,
                        world::report_status,
                        overlay::update_status,
                    )
                        .chain(),
                )
                .configure_sets(
                    FixedUpdate,
                    world::SimulationSet
                        .before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend),
                );
        }
        Mode::ThreeD => {
            app.insert_resource(bevy_rapier3d::prelude::TimestepMode::Fixed {
                dt: 1.0 / 60.0,
                substeps: 1,
            });
            app.add_plugins(bevy_rapier3d::prelude::RapierPhysicsPlugin::<
                bevy_rapier3d::prelude::NoUserData,
            >::default()
            .in_fixed_schedule())
                .add_systems(
                    FixedUpdate,
                    (
                        dim3::sync_pause,
                        dim3::sync_timestep,
                        world::step_vm,
                        world::step_scripts,
                        world::apply_common,
                        dim3::apply_effects,
                        world::apply_component_effects,
                        world::step_glides,
                        world::clear_effects,
                    )
                        .chain()
                        .in_set(world::SimulationSet),
                )
                .add_systems(FixedPostUpdate, dim3::record_poses)
                .add_systems(
                    Update,
                    (
                        world::pump_editor,
                        world::rebuild_world,
                        dim3::relay_collisions,
                        world::publish_sensors,
                        world::detect_clicks,
                        world::interpolate_poses,
                        world::drive_camera,
                        overlay::update_speech_bubbles,
                        world::report_status,
                        overlay::update_status,
                    )
                        .chain(),
                )
                .configure_sets(
                    FixedUpdate,
                    world::SimulationSet
                        .before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend),
                );
        }
    }

    app.run();
}

fn announce_ready() {
    bridge::send(&RuntimeMessage::Ready {
        protocol: PROTOCOL_VERSION,
    });
}

/// `--mode 3d` (or `--mode=3d`) picks the 3D pipeline; 2D is the default.
fn mode_from_args(args: impl Iterator<Item = String>) -> Mode {
    let mut wants_value = false;
    for arg in args {
        if wants_value {
            return parse_mode(&arg);
        }
        match arg.split_once('=') {
            Some(("--mode", value)) => return parse_mode(value),
            _ if arg == "--mode" => wants_value = true,
            _ => {}
        }
    }
    Mode::TwoD
}

fn parse_mode(value: &str) -> Mode {
    match value.trim().to_lowercase().as_str() {
        "3d" | "threed" | "3" => Mode::ThreeD,
        _ => Mode::TwoD,
    }
}
