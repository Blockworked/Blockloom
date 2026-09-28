//! Blockloom's game world.
//!
//! A Bevy app that renders one project and runs its blocks. It runs either as
//! its own process, talking to the editor over its pipes (see
//! `blockloom-protocol`), or on a thread inside the editor (see [`embed`]),
//! rendering offscreen into images the editor's Game view shows. Both
//! dimensions' pipelines register at launch and the live `Dimension` follows
//! the active scene, so a project or scene switch across dimensions swaps
//! live under the next rebuild.
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
mod anim2d;
mod atmosphere;
mod batching;
mod beams;
mod bridge;
mod capture;
mod cloud_layers;
mod clouds;
mod culling;
mod decals;
mod decals_deferred;
mod destruction;
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
mod post;
mod preview;
mod probes;
mod quality;
mod ray_tracing;
mod script;
mod shadows;
mod sky;
#[cfg(feature = "ray_tracing")]
mod solari_patch;
mod sound;
mod space;
mod sprites;
mod ssr;
mod streaming;
mod terrain;
mod tiles;
#[cfg(feature = "ray_tracing")]
mod traced;
mod transition;
mod ui;
mod ui_systems;
mod vfx;
mod volume_heat;
mod volumes;
mod water;
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

/// True while the live world is 2D: what gates the 2D simulation chain when
/// both dimensions' pipelines are registered for live cross-dimension switches.
fn is_2d(dimension: Res<Dimension>) -> bool {
    dimension.0 == Mode::TwoD
}

/// True while the live world is 3D.
fn is_3d(dimension: Res<Dimension>) -> bool {
    dimension.0 == Mode::ThreeD
}

/// Rebuild gating for the unified world: 3D rebuilds at once, 2D waits for
/// the sprite shader bindings, mirroring the old per-dimension behavior.
fn rebuild_ready(
    dimension: Res<Dimension>,
    ready: Local<bool>,
    shaders: Res<Assets<bevy::shader::Shader>>,
) -> bool {
    if dimension.0 == Mode::ThreeD {
        return true;
    }
    dim2::sprite_shaders_ready(ready, shaders)
}

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
        // A browser reads game files by their path in the game folder (see
        // `web`), and a build ships no `.meta` files to ask for.
        #[cfg(target_arch = "wasm32")]
        file_path: String::new(),
        #[cfg(target_arch = "wasm32")]
        meta_check: bevy::asset::AssetMetaCheck::Never,
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
    quality::register(app);
    // Custom shader materials plus the tilemap material. Every dimension
    // registers all three, so systems can take their asset stores
    // unconditionally; an unused plugin costs nothing at runtime.
    materials::register(app);
    sprites::register(app);
    app.init_resource::<anim2d::RigCache>();
    passes::register(app);
    hdr::register(app);
    luminance::register(app);
    post::register(app);
    ssr::register(app);
    capture::register(app);
    // Ray tracing is 3D only, but registers always so a live switch into
    // 3D finds it; its systems no-op in 2D.
    ray_tracing::register(app, Mode::ThreeD);
    lightning::register(app);
    decals::register(app);
    destruction::register(app);
    wind::register(app);
    vfx::register(app, mode);
    water::register(app, mode);
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

    // Both dimensions' physics pipelines live side by side for live
    // cross-dimension scene switches. Each only simulates its own bodies, so
    // the idle one no-ops; `sync_pause`/`sync_timestep` keep both configs in
    // step. Simulation runs on Bevy's `FixedUpdate` - a constant-rate step
    // whatever the display does - while input, sensors and rendering stay on
    // the per-frame `Update`.
    //
    // Both dimensions' systems are registered below too. Shared systems appear
    // exactly once - Bevy cannot order against a system type with two
    // instances in one schedule, so a duplicated whole chain gated by `run_if`
    // still panics at schedule init. Only dimension-specific pairs run side
    // by side (each no-ops on the other side); the rest is gated per system
    // with `is_2d`/`is_3d` where it must not run out of dimension.
    app.insert_resource(bevy_rapier2d::prelude::TimestepMode::Fixed {
        dt: 1.0 / 60.0,
        substeps: 1,
    });
    app.insert_resource(bevy_rapier3d::prelude::TimestepMode::Fixed {
        dt: 1.0 / 60.0,
        substeps: 1,
    });
    app.insert_resource(bevy::audio::DefaultSpatialScale(if mode.is_3d() {
        bevy::audio::SpatialScale::default()
    } else {
        bevy::audio::SpatialScale::new_2d(1.0 / 500.0)
    }));
    app.init_resource::<model::ModelCache>();
    app.init_resource::<lights::LightMasks>();
    app.add_plugins(
        bevy_rapier2d::prelude::RapierPhysicsPlugin::<dim2::OneWayHooks>::pixels_per_meter(
            dim2::PIXELS_PER_METER,
        )
        .in_fixed_schedule(),
    );
    app.add_plugins(
        bevy_rapier3d::prelude::RapierPhysicsPlugin::<bevy_rapier3d::prelude::NoUserData>::default(
        )
        .in_fixed_schedule(),
    );
    // The veil and the audio scale follow the live scene, not the launch
    // mode, so they run once outside the gated dimension blocks.
    app.add_systems(Update, transition::drive_veil);
    app.add_systems(Update, world::sync_audio_scale.after(world::rebuild_world));
    // Both dimensions register always for live cross-dimension switches;
    // each side's chains run only while its Dimension is live.
    {
        // Both dimensions register for live cross-dimension switches.
        // Shared systems appear exactly once: Bevy cannot order against a
        // system type with two instances in one schedule, so gating a whole
        // duplicated chain with `run_if` is not enough. Dimension-specific
        // pairs (dim2/dim3) run side by side; each no-ops on the other side.
        use bevy::camera::visibility::VisibilitySystems;
        // 3D-only rendering registrations (no-op with no 3D content).
        batching::register(app);
        culling::register(app);
        probes::register(app);
        light_probes::register(app);
        pbr_patch::register(app);
        sky::register(app);
        space::register(app);
        fog::register(app);
        clouds::register(app);
        cloud_layers::register(app);
        beams::register(app);
        terrain::register(app);
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
            )
                .run_if(is_3d),
        );
        #[cfg(feature = "ray_tracing")]
        app.add_systems(
            PostUpdate,
            (
                ray_tracing::sync_traced_scene,
                ray_tracing::drive_path_tracer,
            )
                .chain()
                .after(bevy::transform::TransformSystems::Propagate)
                .run_if(is_3d),
        );
        // ── Simulation (FixedUpdate), shared once ──
        app.add_systems(
            FixedUpdate,
            (
                (dim2::sync_pause, dim3::sync_pause).chain(),
                (dim2::sync_timestep, dim3::sync_timestep).chain(),
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
                (dim2::apply_effects, dim3::apply_effects).chain(),
                (
                    world::apply_component_effects,
                    lights::apply_light_effects.run_if(is_3d),
                    ray_tracing::apply_ray_tracing_effects.run_if(is_3d),
                )
                    .chain(),
                (dim2::sync_joints, dim3::sync_joints).chain(),
                fx::apply_fx_effects,
                sound::apply_sound_effects,
                (
                    world::step_glides,
                    world::step_tweens,
                    anim2d::apply_animation_effects,
                    anim2d::step_animations,
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
            (
                world::apply_parenting,
                dim2::record_poses,
                dim3::record_poses,
            )
                .chain(),
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
                    world::rebuild_world.run_if(rebuild_ready),
                    volumes::gather_volumes,
                    post::auto_expose,
                    environment::blend_environment,
                    hdr::resolve_frame,
                    environment::apply_environment,
                    (post::apply_post, post::focus_depth_of_field.run_if(is_3d)),
                    (
                        lights::sync_lights,
                        shadows::apply_shadows,
                        ray_tracing::apply_ray_tracing,
                        light_probes::load_baked,
                        light_probes::sync_probes,
                    )
                        .chain()
                        .run_if(is_3d),
                )
                    .chain(),
                (dim2::relay_collisions, dim3::relay_collisions).chain(),
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
                streaming::update_streaming_cells.run_if(is_3d),
                (
                    overlay::update_speech_bubbles,
                    preview::capture_preview_frame,
                )
                    .chain(),
                (
                    world::report_status.run_if(bridge::editor_attached),
                    overlay::update_status.run_if(bridge::editor_attached),
                    ray_tracing::report_ray_tracing
                        .run_if(bridge::editor_attached)
                        .run_if(is_3d),
                )
                    .chain(),
            )
                .chain(),
        )
        .configure_sets(
            FixedUpdate,
            world::SimulationSet.before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend),
        )
        .configure_sets(
            FixedUpdate,
            world::SimulationSet.before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend),
        )
        // Rigs and sprite dials draw after the camera settles, so Y-sort
        // measures from where it ended up.
        .add_systems(
            Update,
            (
                anim2d::ensure_rigs,
                anim2d::draw_rigs,
                sprites::sync_sprites,
                sprites::sync_part_effects,
            )
                .chain()
                .after(world::drive_camera)
                .before(overlay::update_speech_bubbles)
                .run_if(is_2d),
        )
        // The level: painted tiles, regions and rooms on the fixed tick;
        // parallax, room cameras, streaming and the Tiles tool per frame.
        // Shared level systems run once; per-dimension variants are gated.
        .init_resource::<tiles::Level>()
        .add_systems(
            FixedUpdate,
            (
                tiles::apply_level_effects,
                tiles::redraw_maps.run_if(is_2d),
                tiles::redraw_maps_3d.run_if(is_3d),
                tiles::apply_regions.run_if(is_2d),
                tiles::apply_regions_3d.run_if(is_3d),
                tiles::track_rooms,
            )
                .chain()
                .in_set(world::SimulationSet)
                .after(dim2::apply_effects)
                .after(dim3::apply_effects)
                .before(world::clear_effects),
        )
        .add_systems(
            Update,
            (
                tiles::paint_tiles
                    .after(edit::interact)
                    .before(edit::report),
                tiles::redraw_maps
                    .after(tiles::paint_tiles)
                    .after(world::rebuild_world)
                    .run_if(is_2d),
                tiles::redraw_maps_3d
                    .after(tiles::paint_tiles)
                    .after(world::rebuild_world)
                    .run_if(is_3d),
                tiles::publish_level.after(world::publish_sensors),
                tiles::confine_camera
                    .after(world::drive_camera)
                    .before(edit::apply_view),
                (streaming::update_streaming_cells_2d, tiles::stream_rooms)
                    .chain()
                    .after(world::drive_camera)
                    .after(world::rebuild_world)
                    .run_if(is_2d),
                tiles::stream_rooms_3d
                    .after(streaming::update_streaming_cells)
                    .after(world::rebuild_world)
                    .run_if(is_3d),
                tiles::draw_overlays.after(edit::draw),
            ),
        )
        .add_systems(
            First,
            (
                sprites::clear_sort_depth.run_if(is_2d),
                tiles::clear_parallax,
                tiles::clear_parallax_3d.run_if(is_3d),
            ),
        )
        .add_systems(
            PostUpdate,
            (tiles::apply_parallax, tiles::sync_parallax_copies)
                .chain()
                .after(sprites::apply_sort_depth)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(is_2d),
        )
        .add_systems(
            PostUpdate,
            sprites::apply_sort_depth
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(is_2d),
        )
        .add_systems(
            PostUpdate,
            (tiles::apply_parallax_3d, tiles::sync_parallax_copies_3d)
                .chain()
                .before(batching::upload_instances)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(is_3d),
        )
        .add_systems(
            FixedUpdate,
            water::float_bodies_2d
                .in_set(world::SimulationSet)
                .after(dim2::apply_effects)
                .before(fx::apply_fx_effects)
                .run_if(is_2d),
        )
        .add_systems(
            FixedUpdate,
            water::float_bodies_3d
                .in_set(world::SimulationSet)
                .after(dim3::apply_effects)
                .before(fx::apply_fx_effects)
                .run_if(is_3d),
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
                    .before(dim2::relay_collisions)
                    .run_if(is_2d),
                performance::mark_update_segment
                    .after(light_probes::sync_probes)
                    .before(dim3::relay_collisions)
                    .run_if(is_3d),
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
                fx::step_particles,
                fx::snapshot_trails,
                fx::step_ghosts,
                materials::tick_graph_time,
                materials::animate_tiles,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                model::watch_models,
                model::pause_rigs,
                (
                    light_probes::start_bakes,
                    probes::run_captures,
                    light_probes::collect_bakes,
                )
                    .chain(),
            )
                .chain()
                .run_if(is_3d),
        );
    }
}

fn announce_ready() {
    bridge::send(&RuntimeMessage::Ready {
        protocol: PROTOCOL_VERSION,
    });
}
