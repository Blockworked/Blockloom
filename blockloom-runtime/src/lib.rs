//! Blockloom's game world.
//!
//! A Bevy app that renders one project and runs its blocks. It runs either as
//! its own process, talking to the editor over its pipes (see
//! `blockloom-protocol`), or on a thread inside the editor (see [`embed`]),
//! rendering offscreen into images the editor's Game view shows. Which
//! dimension to build is fixed at launch, since the plugin set differs: the
//! editor restarts the runtime when a project changes mode.
//!
//! The same binary is what a built game ships: with a pack beside it (see
//! `blockloom_core::pack`) it loads that instead of waiting for an editor,
//! takes its dimension from the document, and presses its own green flag.
//! `player::Launch` is the whole of the difference.
//!

// A Bevy system declares every query and resource it touches as an argument, so
// the usual argument-count limit doesn't apply here.
#![allow(clippy::too_many_arguments)]
// Bevy system queries and params are long by nature.
#![allow(clippy::type_complexity)]

mod ai;
mod atmosphere;
mod batching;
mod beams;
mod bridge;
mod capture;
mod clouds;
mod culling;
mod dim2;
mod dim3;
mod display;
mod edit;
#[cfg(target_os = "linux")]
pub mod embed;
mod engine;
mod environment;
mod fog;
mod fx;
mod gpu;
mod hdr;
mod light_probes;
mod lightning;
mod lights;
mod logic;
mod luminance;
mod materials;
mod model;
mod wind;
// Plumbing the Phase 5 passes build on; nothing reads most of it yet.
mod overlay;
#[allow(dead_code)]
mod passes;
mod pbr_patch;
mod performance;
pub mod player;
mod preview;
mod probes;
mod ray_tracing;
mod script;
mod shadows;
mod sky;
mod sound;
mod space;
mod streaming;
mod ui;
mod ui_systems;
mod volume_heat;
mod volumes;
#[cfg(target_arch = "wasm32")]
pub mod web;
mod world;

use bevy::asset::{AssetPlugin, UnapprovedPathMode};
use bevy::prelude::*;
use bevy::render::diagnostic::{MeshAllocatorDiagnosticPlugin, RenderDiagnosticsPlugin};
use bevy::window::WindowResolution;
use blockloom_core::scene::Mode;
use blockloom_protocol::{GAME_SIZE, PROTOCOL_VERSION, RuntimeMessage};
use engine::{Dimension, PendingEffects};
use player::Launch;

/// Runs the world as its own process: the editor's child, or a built game.
pub fn run_process() {
    // Before the pack is read: a saved document names Blockloom's own
    // reporter blocks, which have to be registered to evaluate.
    blockloom_core::init();
    let launch = Launch::from_args(std::env::args().skip(1));
    let mode = launch.mode();
    let title = launch.title();
    let hdr = hdr::HdrPolicy {
        allow: launch.allows_hdr(),
        windowed: true,
    };

    let mut app = App::new();
    #[cfg(target_os = "linux")]
    {
        let mut vulkan = bevy::render::renderer::raw_vulkan_init::RawVulkanInitSettings::default();
        display::add_vulkan_extensions(&mut vulkan);
        app.insert_resource(vulkan);
    }
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title,
                    resolution: WindowResolution::new(GAME_SIZE.0, GAME_SIZE.1),
                    ..default()
                }),
                ..default()
            })
            .set(asset_plugin()),
    );
    // Before the world, so `hdr::register` keeps it; the display half takes
    // the window's surface over when it offers HDR.
    app.insert_resource(hdr);
    add_world(&mut app, mode, launch.into_engine());
    if hdr.allow {
        display::register(&mut app);
    }
    app.run();
}

/// Project assets live in the project's own folder, anywhere on disk, and
/// are handed to the asset server as absolute paths. Those are unapproved
/// by default in Bevy 0.19 (`Forbid`), which fails the load and leaves a
/// white sprite - so allow them here. The files are the user's own.
pub(crate) fn asset_plugin() -> AssetPlugin {
    AssetPlugin {
        unapproved_path_mode: UnapprovedPathMode::Allow,
        ..default()
    }
}

/// Everything past the platform plugins: the world's resources, physics and
/// schedules, the same whether it has a window or an embedded view.
pub(crate) fn add_world(app: &mut App, mode: Mode, mut engine: engine::Engine) {
    engine.prewarm = true;
    app.add_plugins((RenderDiagnosticsPlugin, MeshAllocatorDiagnosticPlugin));
    app.insert_resource(ClearColor(Color::srgb(0.11, 0.14, 0.19)))
        .insert_resource(Dimension(mode))
        .init_resource::<PendingEffects>()
        .init_resource::<world::NavMesh>()
        .init_resource::<ui::UiManager>()
        .init_resource::<sound::SoundState>()
        .init_resource::<fx::FxCache>()
        .init_resource::<performance::RenderCache>()
        .init_resource::<performance::GameViewTargetBytes>()
        .init_resource::<preview::PreviewState>()
        .init_resource::<preview::PreviewPointer>()
        .init_resource::<preview::PreviewButtons>()
        .init_resource::<preview::PreviewKeys>()
        .init_resource::<preview::PreviewTouches>()
        .insert_non_send(engine);
    environment::register(app);
    atmosphere::register(app);
    volumes::register(app);
    volume_heat::register(app);
    streaming::register(app);
    gpu::register(app);
    // Custom shader materials plus the tilemap material. Every dimension
    // registers all three, so systems can take their asset stores
    // unconditionally; an unused plugin costs nothing at runtime.
    materials::register(app);
    passes::register(app);
    hdr::register(app);
    luminance::register(app);
    capture::register(app);
    ray_tracing::register(app, mode);
    lightning::register(app);
    wind::register(app);
    edit::configure(app);
    // Both of these only exist to talk to an editor, and a built game has
    // none: no corner status, no handshake.
    app.add_systems(
        Startup,
        (overlay::spawn, announce_ready).run_if(bridge::editor_attached),
    );
    // Split each frame into fixed-step sim versus the rest, for the profiler.
    app.init_resource::<performance::SimSplit>()
        .add_systems(FixedFirst, performance::mark_step_start)
        .add_systems(FixedLast, performance::mark_step_end);
    app.init_resource::<performance::LoopPace>()
        .init_resource::<performance::UpdateSplit>()
        .add_systems(First, performance::mark_main_start)
        .add_systems(
            PostUpdate,
            performance::mark_post_start.before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            Last,
            (
                performance::publish_sim_split,
                performance::mark_main_end,
                performance::publish_update_split,
            )
                .chain(),
        );

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
            // Positions are pixels in 2D, so the listener hears in hundreds:
            // scale the world down to speaking distance for the pan to mean
            // anything.
            app.insert_resource(bevy::audio::DefaultSpatialScale(
                bevy::audio::SpatialScale::new_2d(1.0 / 500.0),
            ));
            app.add_plugins(
                bevy_rapier2d::prelude::RapierPhysicsPlugin::<dim2::OneWayHooks>::pixels_per_meter(
                    dim2::PIXELS_PER_METER,
                )
                .in_fixed_schedule(),
            )
            .add_systems(
                FixedUpdate,
                (
                    dim2::sync_pause,
                    dim2::sync_timestep,
                    (world::restore_poses, atmosphere::sample_atmosphere).chain(),
                    (ui_systems::bindings, world::step_vm).chain(),
                    (world::step_scripts, ai::tick).chain(),
                    overlay::apply_ui_effects,
                    world::apply_saved_data,
                    (world::apply_lifetimes, world::sync_navmesh).chain(),
                    (
                        world::apply_common,
                        environment::apply_exposure_effects,
                        hdr::apply_hdr_effects,
                        volumes::apply_volume_effects,
                    )
                        .chain(),
                    dim2::apply_effects,
                    world::apply_component_effects,
                    dim2::sync_joints,
                    fx::apply_fx_effects,
                    sound::apply_sound_effects,
                    (
                        world::step_glides,
                        world::step_tweens,
                        world::step_animations,
                    )
                        .chain(),
                    world::apply_input_effects,
                    world::apply_rumble,
                    world::apply_cursor_lock,
                    world::clear_effects,
                    world::finish_step,
                )
                    .chain()
                    .in_set(world::SimulationSet),
            )
            .add_systems(
                FixedPostUpdate,
                (world::apply_parenting, dim2::record_poses).chain(),
            )
            .add_systems(
                Update,
                (
                    world::pump_editor,
                    (edit::interact, edit::report).chain(),
                    preview::apply_preview_visibility,
                    preview::drain_preview_inputs,
                    fx::despawn_fx,
                    (
                        world::rebuild_world.run_if(dim2::sprite_shaders_ready),
                        volumes::gather_volumes,
                        environment::blend_environment,
                        hdr::resolve_frame,
                        environment::apply_environment,
                    )
                        .chain(),
                    dim2::relay_collisions,
                    (
                        ui_systems::canvas,
                        ui_systems::collections,
                        ui_systems::hover,
                        ui_systems::navigation,
                        ui_systems::touch,
                        ui_systems::project_widgets,
                        overlay::draw_ui,
                        ui_systems::animate,
                        ui_systems::atlas,
                    )
                        .chain(),
                    world::type_into_focused_input,
                    world::scroll_ui_lists,
                    world::detect_clicks,
                    world::publish_sensors,
                    sound::maintain_voices,
                    world::interpolate_poses,
                    (
                        world::drive_camera,
                        edit::apply_view,
                        edit::draw,
                        volumes::draw_volumes,
                        volume_heat::collect_heat,
                    )
                        .chain(),
                    overlay::update_speech_bubbles,
                    preview::capture_preview_frame,
                    world::report_status.run_if(bridge::editor_attached),
                    overlay::update_status.run_if(bridge::editor_attached),
                )
                    .chain(),
            )
            .configure_sets(
                FixedUpdate,
                world::SimulationSet.before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend),
            )
            // Profiler segment marks, as explicit edges: the chained tuple
            // above is already at Bevy's 20-system cap, and restructuring it
            // would move its ApplyDeferred points.
            .add_systems(
                Update,
                (
                    performance::mark_update_segment.before(world::pump_editor),
                    performance::mark_update_segment
                        .after(fx::despawn_fx)
                        .before(volumes::gather_volumes),
                    performance::mark_update_segment
                        .after(environment::apply_environment)
                        .before(dim2::relay_collisions),
                    performance::mark_update_segment
                        .after(world::detect_clicks)
                        .before(world::publish_sensors),
                    performance::mark_update_segment
                        .after(world::interpolate_poses)
                        .before(world::drive_camera),
                    performance::mark_update_segment
                        .after(volume_heat::collect_heat)
                        .before(overlay::update_speech_bubbles),
                    performance::mark_update_segment.after(overlay::update_status),
                ),
            )
            .add_systems(
                Update,
                (
                    fx::emit_particles,
                    fx::step_particles,
                    fx::snapshot_trails,
                    fx::step_ghosts,
                    materials::tick_graph_time,
                    materials::animate_tiles,
                )
                    .chain(),
            );
        }
        Mode::ThreeD => {
            app.init_resource::<model::ModelCache>();
            batching::register(app);
            culling::register(app);
            probes::register(app);
            light_probes::register(app);
            pbr_patch::register(app);
            app.init_resource::<lights::LightMasks>();
            sky::register(app);
            space::register(app);
            fog::register(app);
            clouds::register(app);
            beams::register(app);
            use bevy::camera::visibility::VisibilitySystems;
            app.add_systems(
                PostUpdate,
                (
                    (culling::select_lod, batching::batch_meshes)
                        .chain()
                        .after(bevy::transform::TransformSystems::Propagate)
                        .after(VisibilitySystems::VisibilityPropagate)
                        .before(VisibilitySystems::CalculateBounds)
                        .before(VisibilitySystems::CheckVisibility),
                    batching::upload_instances,
                    culling::configure_cameras,
                    culling::cull_views
                        .after(VisibilitySystems::CheckVisibility)
                        .before(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible),
                    probes::hide_from_captures
                        .after(VisibilitySystems::CheckVisibility)
                        .before(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible),
                ),
            );
            #[cfg(feature = "ray_tracing")]
            app.add_systems(
                PostUpdate,
                (
                    ray_tracing::sync_traced_scene,
                    ray_tracing::drive_path_tracer,
                )
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate),
            );
            app.insert_resource(bevy_rapier3d::prelude::TimestepMode::Fixed {
                dt: 1.0 / 60.0,
                substeps: 1,
            });
            // `ScreenSpaceAmbientOcclusion` on the camera is driven by
            // `bevy_pbr`'s `PbrPlugin` (inside `DefaultPlugins`), so no extra
            // plugin is needed here.
            app.add_plugins(bevy_rapier3d::prelude::RapierPhysicsPlugin::<
                bevy_rapier3d::prelude::NoUserData,
            >::default()
            .in_fixed_schedule())
                .add_systems(
                    FixedUpdate,
                    (
                        dim3::sync_pause,
                        dim3::sync_timestep,
                        (world::restore_poses, atmosphere::sample_atmosphere).chain(),
                        (ui_systems::bindings, world::step_vm).chain(),
                        (world::step_scripts, ai::tick).chain(),
                        overlay::apply_ui_effects,
                        world::apply_saved_data,
                        (world::apply_lifetimes, world::sync_navmesh).chain(),
                        (
                        world::apply_common,
                        environment::apply_exposure_effects,
                        hdr::apply_hdr_effects,
                        volumes::apply_volume_effects,
                    )
                        .chain(),
                        dim3::apply_effects,
                        (
                            world::apply_component_effects,
                            lights::apply_light_effects,
                            ray_tracing::apply_ray_tracing_effects,
                        )
                            .chain(),
                        dim3::sync_joints,
                        fx::apply_fx_effects,
                        sound::apply_sound_effects,
                        (world::step_glides, world::step_tweens, world::step_animations).chain(),
                        world::apply_input_effects,
                        world::apply_rumble,
                        world::apply_cursor_lock,
                        world::clear_effects,
                        world::finish_step,
                    )
                        .chain()
                        .in_set(world::SimulationSet),
                )
                .add_systems(
                    FixedPostUpdate,
                    (world::apply_parenting, dim3::record_poses).chain(),
                )
                .add_systems(
                    Update,
                    (
                        world::pump_editor,
                        (edit::interact, edit::report).chain(),
                        preview::apply_preview_visibility,
                        preview::drain_preview_inputs,
                        fx::despawn_fx,
                        (
                            world::rebuild_world,
                            volumes::gather_volumes,
                            environment::blend_environment,
                            hdr::resolve_frame,
                            environment::apply_environment,
                            lights::sync_lights,
                            shadows::apply_shadows,
                            ray_tracing::apply_ray_tracing,
                            light_probes::load_baked,
                            light_probes::sync_probes,
                        )
                            .chain(),
                        dim3::relay_collisions,
                        (ui_systems::canvas, ui_systems::collections, ui_systems::hover, ui_systems::navigation, ui_systems::touch, ui_systems::project_widgets, overlay::draw_ui, ui_systems::animate, ui_systems::atlas).chain(),
                        world::type_into_focused_input,
                        world::scroll_ui_lists,
                        world::detect_clicks,
                        world::publish_sensors,
                        sound::maintain_voices,
                        world::interpolate_poses,
                        (world::drive_camera, edit::apply_view, edit::draw, volumes::draw_volumes, volume_heat::collect_heat).chain(),
                        streaming::update_streaming_cells,
                        overlay::update_speech_bubbles,
                        preview::capture_preview_frame,
                        world::report_status.run_if(bridge::editor_attached),
                        (
                            overlay::update_status,
                            ray_tracing::report_ray_tracing,
                        )
                            .run_if(bridge::editor_attached),
                    )
                        .chain(),
                )
                .configure_sets(
                    FixedUpdate,
                    world::SimulationSet
                        .before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend),
                )
                // Profiler segment marks, as explicit edges: the chained tuple
                // above is already at Bevy's 20-system cap, and restructuring
                // it would move its ApplyDeferred points.
                .add_systems(
                    Update,
                    (
                        performance::mark_update_segment.before(world::pump_editor),
                        performance::mark_update_segment
                            .after(fx::despawn_fx)
                            .before(volumes::gather_volumes),
                        performance::mark_update_segment
                            .after(light_probes::sync_probes)
                            .before(dim3::relay_collisions),
                        performance::mark_update_segment
                            .after(world::detect_clicks)
                            .before(world::publish_sensors),
                        performance::mark_update_segment
                            .after(world::interpolate_poses)
                            .before(world::drive_camera),
                        performance::mark_update_segment
                            .after(streaming::update_streaming_cells)
                            .before(overlay::update_speech_bubbles),
                        performance::mark_update_segment
                            .after(overlay::update_status)
                            .after(ray_tracing::report_ray_tracing),
                    ),
                )
                .add_systems(
                    Update,
                    (
                        fx::emit_particles,
                        fx::step_particles,
                        fx::snapshot_trails,
                        fx::step_ghosts,
                        materials::tick_graph_time,
                        materials::animate_tiles,
                        model::watch_models,
                        model::pause_rigs,
                        (
                            light_probes::start_bakes,
                            probes::run_captures,
                            light_probes::collect_bakes,
                        )
                            .chain(),
                    )
                        .chain(),
                );
        }
    }
}

fn announce_ready() {
    bridge::send(&RuntimeMessage::Ready {
        protocol: PROTOCOL_VERSION,
    });
}
