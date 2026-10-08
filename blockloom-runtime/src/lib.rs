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

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]
// A Bevy system declares every query and resource it touches as an argument, so
// the usual argument-count limit doesn't apply here.
#![allow(clippy::too_many_arguments)]
// Bevy system queries and params are long by nature.
#![allow(clippy::type_complexity)]

mod ai;
#[cfg(target_os = "android")]
mod android;
mod anim2d;
mod atmosphere;
mod batching;
mod beams;
mod bridge;
mod capture;
mod cinematic;
mod cloud_layers;
mod clouds;
mod constraints;
mod contacts;
mod controller;
mod culling;
mod decals;
mod decals_deferred;
mod destruction;
mod dim2;
mod dim3;
mod director;
mod display;
mod edit;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod embed;
mod engine;
mod environment;
mod fog;
mod fx;
mod gpu;
mod hdr;
mod indirect;
mod lan;
mod light_probes;
mod lightning;
mod lights;
mod logic;
mod luminance;
mod materials;
mod model;
mod motor;
mod player_camera;
mod wind;
// Plumbing the Phase 5 passes build on; nothing reads most of it yet.
mod overlay;
#[allow(dead_code)]
mod passes;
mod pbr_patch;
mod performance;
mod physics_debug;
mod physics_install;
pub mod player;
#[cfg(feature = "plugins")]
mod plugin_compute;
#[cfg(feature = "plugins")]
mod plugin_lod;
mod plugin_meshes;
mod plugin_quads;
#[cfg(feature = "plugins")]
mod plugin_services;
mod plugins;
mod post;
mod preview;
mod probes;
mod quality;
mod queries;
mod ray_tracing;
mod script;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod script_wasm;
mod shadows;
mod simulation;
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
mod ui_design;
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
use blockloom_plugin_api::schema::Stage;
use blockloom_protocol::{GAME_SIZE, PROTOCOL_VERSION, RuntimeMessage};
use engine::Dimension;
use player::Launch;
use simulation::SimStep;

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
    // The binary never runs on a phone - the APK loads the library - but a
    // stray execution should still boot the game rather than wait on stdin.
    #[cfg(target_os = "android")]
    run_android();
    // Before the pack is read: a saved document names Blockloom's own
    // reporter blocks, which have to be registered to evaluate.
    #[cfg(not(target_os = "android"))]
    {
        #[cfg(all(
            feature = "multiplayer",
            not(target_arch = "wasm32"),
            not(target_os = "android")
        ))]
        {
            let args: Vec<_> = std::env::args().collect();
            if let Some(index) = args.iter().position(|a| a == "--join") {
                let result = (|| {
                    let invite = args.get(index + 1).ok_or("--join needs a LAN invite")?;
                    let trusted = args
                        .iter()
                        .position(|a| a == "--trusted-build")
                        .and_then(|i| args.get(i + 1))
                        .ok_or("--trusted-build needs the trusted content hash")?;
                    lan::guest::run(invite, trusted)
                })();
                if let Err(error) = result {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                return;
            }
        }
        blockloom_core::init();
        let launch = Launch::from_args(std::env::args().skip(1));
        run_launch(launch);
    }
}

/// Boots the shipped game on Android: the pack comes from the APK's assets,
/// and the world presses its own green flag like every other player build.
/// Called from the `bevy_main` entry, never from the binary.
#[cfg(target_os = "android")]
pub fn run_android() {
    // winit forbids building its event loop twice in one process, so a
    // second activity in a lingering process would panic with
    // RecreationAttempt. Die instead; the relaunch then forks fresh.
    static ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ENTERED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        std::process::exit(0);
    }
    crate::android::install_panic_hook();
    // Forwards `log` records (wgpu, driver notes) into Bevy's tracing so
    // they reach logcat. Without this they vanish on device, including
    // the descriptor-allocator diagnostics.
    let _ = tracing_log::LogTracer::init();
    blockloom_core::init();
    match crate::android::load_pack() {
        Ok(pack) => {
            run_launch(Launch::from_android(pack));
            // The activity is gone when the app returns (back button, quit).
            // Die with it so the next launch forks fresh rather than
            // hitting the guard above and flashing out.
            std::process::exit(0);
        }
        Err(message) => {
            eprintln!("blockloom: {message}");
            std::process::exit(1);
        }
    }
}

fn run_launch(launch: Launch) {
    let mode = launch.mode();
    let title = launch.title();
    let hdr = hdr::HdrPolicy {
        allow: launch.allows_hdr(),
        windowed: true,
    };

    let mut app = App::new();
    // DLSS needs its project id before `DefaultPlugins` (which holds
    // `DlssInitPlugin` when the `dlss` feature is on). One id for all of
    // Blockloom; generate a fresh one per fork, never copy Bevy's example.
    #[cfg(all(feature = "dlss", not(target_arch = "wasm32")))]
    app.insert_resource(bevy::anti_alias::dlss::DlssProjectId(
        bevy::asset::uuid::uuid!("259f6fa9-7a86-42c3-a74a-91643c0b9c7b"),
    ));
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
    #[cfg(target_os = "android")]
    crate::android::register(&mut app);
    // Serialize render schedules on Android to avoid concurrent Mali driver
    // calls. Desktop keeps its threads.
    #[cfg(target_os = "android")]
    serial_render(&mut app);
    app.run();
}

/// Serializes the outer render schedule and its nested camera schedules.
#[cfg(target_os = "android")]
fn serial_render(app: &mut App) {
    use bevy::ecs::schedule::{Schedules, SingleThreadedExecutor};
    use bevy::render::RenderApp;
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    let mut schedules = render.world_mut().resource_mut::<Schedules>();
    for (_, schedule) in schedules.iter_mut() {
        schedule.set_executor(SingleThreadedExecutor::new());
    }
}

/// Project assets live in the project's own folder, anywhere on disk, and
/// are handed to the asset server as absolute paths. Those are unapproved
/// by default in Bevy 0.19 (`Forbid`), which fails the load and leaves a
/// white sprite - so allow them here. The files are the user's own. On
/// Android the default reader is already the APK asset manager, which serves
/// the staged game folder without any override here.
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
    // Before Bevy's own: a refused HDR surface configure degrades to SDR
    // in the surface code, so it must not quit the run like other errors.
    display::install_render_error_handler(app);
    app.add_plugins((RenderDiagnosticsPlugin, MeshAllocatorDiagnosticPlugin));
    // The simulation first: its resources and the engine, which the
    // presentation systems below reach as a `NonSend` too.
    simulation::add_simulation(app, mode, engine);
    app.insert_resource(ClearColor(Color::srgb(0.11, 0.14, 0.19)))
        .init_resource::<ui::UiManager>()
        .init_resource::<ui_design::DesignSession>()
        .init_resource::<ui::DeviceInsets>()
        .init_resource::<sound::SoundState>()
        .init_resource::<fx::FxCache>()
        .init_resource::<performance::RenderCache>()
        .init_resource::<performance::GameViewTargetBytes>()
        .init_resource::<preview::PreviewState>()
        .init_resource::<preview::PreviewPointer>()
        .init_resource::<preview::PreviewButtons>()
        .init_resource::<preview::PreviewKeys>()
        .init_resource::<preview::PreviewTouches>();
    environment::register(app);
    volumes::register(app);
    volume_heat::register(app);
    streaming::register(app);
    gpu::register(app);
    indirect::register(app);
    quality::register(app);
    // Custom shader materials plus the tilemap material. Every dimension
    // registers all three, so systems can take their asset stores
    // unconditionally; an unused plugin costs nothing at runtime.
    materials::register(app);
    sprites::register(app);
    app.init_resource::<anim2d::RigCache>()
        .init_resource::<plugin_meshes::PluginMeshes>()
        .add_systems(Update, plugin_meshes::sync.run_if(is_3d))
        .add_systems(
            PostUpdate,
            plugin_meshes::crossfade
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility)
                .run_if(is_3d),
        );
    #[cfg(feature = "plugins")]
    app.add_systems(PostUpdate, plugin_meshes::publish_poses.run_if(is_3d));
    #[cfg(feature = "plugins")]
    plugin_compute::register(app);
    #[cfg(feature = "plugins")]
    plugin_lod::register(app);
    plugin_quads::register(app);
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
    director::register(app);
    cinematic::register(app);
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
    // Plugin hooks that follow a frame rather than a fixed step.
    app.add_systems(
        PostUpdate,
        plugins::stage(Stage::RenderExtraction)
            .before(bevy::transform::TransformSystems::Propagate),
    );
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
                plugins::stage(Stage::Presentation),
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
    app.insert_resource(bevy::audio::DefaultSpatialScale(if mode.is_3d() {
        bevy::audio::SpatialScale::default()
    } else {
        bevy::audio::SpatialScale::new_2d(1.0 / 500.0)
    }));
    app.init_resource::<model::ModelCache>();
    app.init_resource::<lights::LightMasks>();
    // Physics Debug view: Rapier's gizmo renderer, off until the scene view asks.
    app.add_plugins(bevy_rapier2d::render::RapierDebugRenderPlugin::default().disabled());
    app.add_plugins(bevy_rapier3d::render::RapierDebugRenderPlugin::default().disabled());
    app.add_systems(Update, physics_debug::sync);
    app.add_systems(
        FixedUpdate,
        (
            physics_debug::d3::begin
                .before(bevy_rapier3d::prelude::PhysicsSet::SyncBackend)
                .run_if(is_3d),
            physics_debug::d3::end
                .after(bevy_rapier3d::prelude::PhysicsSet::Writeback)
                .run_if(is_3d),
            physics_debug::d2::begin
                .before(bevy_rapier2d::prelude::PhysicsSet::SyncBackend)
                .run_if(is_2d),
            physics_debug::d2::end
                .after(bevy_rapier2d::prelude::PhysicsSet::Writeback)
                .run_if(is_2d),
        ),
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
                // A culled placeholder's loaded glTF scene hangs off
                // `ModelChild` with its own visibility: at mid range it draws
                // decimated meshes, and far out it hides with the group so
                // distant models shed draws like culled boxes.
                model::sync_model_lod
                    .after(culling::select_lod)
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
        app.add_systems(Update, ui_design::resize.after(world::pump_editor));
        app.add_systems(Last, preview::capture_preview_frame);
        app.add_systems(
            PostUpdate,
            ui_design::report
                .after(bevy::ui::UiSystems::PostLayout)
                .after(bevy::ui::UiSystems::Stack),
        );
        // The fixed step's simulation is `simulation::add_simulation`; these
        // are the presentation systems that ran inside it, each ordered
        // between the `SimStep`s it used to sit between.
        app.add_systems(
            FixedUpdate,
            (
                ui_systems::bindings
                    .run_if(ui_design::inactive)
                    .after(SimStep::PluginPreSimulation)
                    .before(SimStep::Vm),
                overlay::apply_ui_effects
                    .after(SimStep::PluginFixed)
                    .before(SimStep::SavedData),
                (environment::apply_exposure_effects, hdr::apply_hdr_effects)
                    .chain()
                    .after(SimStep::Common)
                    .before(SimStep::VolumeEffects),
                (
                    lights::apply_light_effects.run_if(is_3d),
                    ray_tracing::apply_ray_tracing_effects.run_if(is_3d),
                )
                    .chain()
                    .after(SimStep::ComponentEffects)
                    .before(SimStep::PluginEffects),
                (fx::apply_fx_effects, sound::apply_sound_effects)
                    .chain()
                    .after(SimStep::Joints)
                    .before(SimStep::Animation),
                (
                    world::apply_input_effects,
                    world::apply_rumble,
                    world::apply_cursor_lock,
                )
                    .chain()
                    .after(SimStep::Animation)
                    .before(SimStep::ClearEffects),
            )
                .in_set(world::SimulationSet),
        )
        .add_systems(
            Update,
            (
                world::pump_editor,
                (edit::interact, edit::report)
                    .chain()
                    .run_if(ui_design::inactive),
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
                (
                    ui_systems::canvas,
                    ui_systems::collections,
                    ui_systems::hover,
                    ui_systems::navigation,
                    ui_systems::touch,
                    ui_systems::project_widgets.run_if(ui_design::inactive),
                    overlay::draw_ui,
                    ui_systems::animate,
                    ui_systems::atlas,
                )
                    .chain(),
                world::type_into_focused_input,
                world::scroll_ui_lists,
                world::detect_clicks,
                world::publish_sensors,
                world::step_scripts_frame_ui,
                sound::maintain_voices,
                world::interpolate_poses,
                (
                    world::drive_camera,
                    edit::apply_view,
                    edit::draw.run_if(ui_design::inactive),
                    volumes::draw_volumes,
                    volume_heat::collect_heat,
                )
                    .chain(),
                streaming::update_streaming_cells.run_if(is_3d),
                overlay::update_speech_bubbles,
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
                motor::latch_player_input.after(world::publish_sensors),
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
                    .before(ui_systems::canvas)
                    .run_if(is_2d),
                performance::mark_update_segment
                    .after(light_probes::sync_probes)
                    .before(ui_systems::canvas)
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
        // Its own call, not part of the chained tuple above (which sits at
        // Bevy's tuple cap): the back button fires before text inputs see
        // it, so a focused field releases rather than double-firing.
        .add_systems(
            Update,
            world::back_button.before(world::type_into_focused_input),
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
