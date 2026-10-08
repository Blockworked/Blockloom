//! Maps a command name plus its JSON arguments onto the matching function in
//! `commands.rs`. Argument names are camelCase, the spelling the QML
//! frontend uses with `invokeCommand`.

use crate::commands;
use crate::state::{InstrPath, ValueLocation};
use blockloom_core::blocks::{BlockPiece, BlockShape, Instruction};
use blockloom_core::components::ActorComponent;
use blockloom_core::nav::NavSettings;
use blockloom_core::scene::{
    Camera, DisplayOutput, Lighting, Mode, Physics, Placement, PostProcess, Visual,
};
use blockloom_core::sound::SoundMixer;
use blockloom_core::value::Value as BlockValue;
use blockloom_core::wire;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Backend;

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    let value = args.get(name).cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| format!("invalid argument '{name}': {e}"))
}

/// An instruction comes in flat (`{"id": _, "type": _, ...}`) and has to be
/// folded back into blockstitch's `{id, kind}` shape - see [`wire`].
fn instruction_arg(args: &Value, name: &str) -> Result<Instruction, String> {
    let value = args.get(name).cloned().unwrap_or(Value::Null);
    wire::from_wire(value).map_err(|e| format!("invalid argument '{name}': {e}"))
}

fn instructions_arg(args: &Value, name: &str) -> Result<Vec<Instruction>, String> {
    let value = args.get(name).cloned().unwrap_or(Value::Null);
    wire::from_wire(value).map_err(|e| format!("invalid argument '{name}': {e}"))
}

/// A tool run's hits: a list as `hits`, else the one `hit`.
fn tool_hits(args: &Value) -> Result<Vec<Value>, String> {
    match args.get("hits").filter(|v| !v.is_null()) {
        Some(_) => arg(args, "hits"),
        None => Ok(vec![arg(args, "hit")?]),
    }
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Backend {
    /// Runs the command named `cmd` with `args` (a JSON object keyed by
    /// camelCase argument name) and returns its result as JSON.
    pub fn dispatch(&self, cmd: &str, args: Value) -> Result<Value, String> {
        // Job controls must stay responsive while any other command holds state.
        match cmd {
            "build_job_status" => return to_json(self.builds.status(arg(&args, "id")?)?),
            "cancel_build_job" => return to_json(self.builds.cancel(arg(&args, "id")?)?),
            _ => {}
        }
        let state = &self.state;
        let app = &self.app;
        // Heartbeat first, then live reload: an idle backend follows the
        // folder's saves before running whatever was actually asked.
        commands::note_activity(state);
        commands::poll_live_reload(self);
        match cmd {
            "get_state" => to_json(commands::get_state(state)?),
            "block_vocabulary" => to_json(commands::block_vocabulary()?),
            "sync_status" => to_json(commands::sync_status(state)?),
            "reload_project" => to_json(commands::reload_project(
                state,
                app,
                arg(&args, "take_theirs").unwrap_or_default(),
            )?),
            "take_over_lock" => to_json(commands::take_over_lock(state, app)?),

            // ── Projects ───────────────────────────────────────────────────
            "open_project" => to_json(commands::open_project(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "force").unwrap_or_default(),
            )?),
            "create_project" => to_json(commands::create_project(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "location").ok().flatten(),
                arg(&args, "mode").unwrap_or_default(),
                arg(&args, "sample").ok(),
            )?),
            "close_project" => to_json(commands::close_project(state, app)?),
            "forget_project" => to_json(commands::forget_project(state, app, arg(&args, "path")?)?),
            "delete_project" => to_json(commands::delete_project(state, app, arg(&args, "path")?)?),
            "set_project_name" => {
                to_json(commands::set_project_name(state, app, arg(&args, "name")?)?)
            }
            "set_multiplayer" => to_json(commands::set_multiplayer(
                state,
                app,
                arg(&args, "settings")?,
            )?),
            "set_project_icon" => {
                to_json(commands::set_project_icon(state, app, arg(&args, "path")?)?)
            }
            "save_project" => to_json(commands::save_open_project(state, app)?),
            "export_file_name" => to_json(commands::export_file_name(state)?),
            "export_project" => to_json(commands::export_project(state, arg(&args, "path")?)?),
            "import_project" => to_json(commands::import_project(state, app, arg(&args, "path")?)?),
            "list_build_targets" => to_json(commands::list_build_targets(state)?),
            "start_build_game" => to_json(self.builds.start(
                self,
                serde_json::from_value(args).map_err(|e| e.to_string())?,
            )?),
            "build_game" => {
                let mut params: crate::build_jobs::BuildParams =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                params.device = None;
                let job = self.builds.start(self, params)?;
                to_json(self.builds.wait(job.id)?)
            }
            "android_status" => to_json(commands::android_status()?),
            "android_device_status" => to_json(commands::android_device_status()?),
            "android_install_sdk" => to_json(commands::android_install_sdk()?),
            "android_accept_licenses" => to_json(commands::android_accept_licenses(
                arg(&args, "accept").unwrap_or_default(),
            )?),
            "android_install" => to_json(commands::android_install(
                arg(&args, "apk")?,
                arg(&args, "app")?,
                arg(&args, "device").ok().flatten(),
            )?),
            "android_logcat" => to_json(commands::android_logcat(
                arg(&args, "device").ok().flatten(),
            )?),
            "android_logcat_tail" => to_json(commands::android_logcat_tail(
                state,
                app,
                arg(&args, "device").ok().flatten(),
            )?),
            "android_keyring_status" => to_json(commands::android_keyring_status(state)?),
            "android_forget_passwords" => to_json(commands::android_forget_passwords(state)?),
            "android_emulator_status" => to_json(commands::android_emulator_status()?),
            "android_create_avd" => to_json(commands::android_create_avd(
                arg(&args, "name").ok().flatten(),
            )?),
            "android_rename_avd" => to_json(commands::android_rename_avd(
                arg(&args, "name")?,
                arg(&args, "newName")?,
            )?),
            "android_delete_avd" => to_json(commands::android_delete_avd(arg(&args, "name")?)?),
            "android_start_emulator" => to_json(commands::android_start_emulator(
                arg(&args, "avd").ok().flatten(),
                arg(&args, "waitSecs").ok().flatten(),
                arg(&args, "headless").ok().flatten().unwrap_or(false),
            )?),
            "android_stop_emulator" => to_json(commands::android_stop_emulator(
                arg(&args, "serial").ok().flatten(),
            )?),
            "android_mirror_frame" => to_json(commands::android_mirror_frame(
                arg(&args, "device").ok().flatten(),
                arg(&args, "maxWidth").ok().flatten(),
            )?),
            "android_mirror_tap" => to_json(commands::android_mirror_tap(
                arg(&args, "device").ok().flatten(),
                arg(&args, "x").unwrap_or(0.5),
                arg(&args, "y").unwrap_or(0.5),
            )?),
            "android_mirror_swipe" => to_json(commands::android_mirror_swipe(
                arg(&args, "device").ok().flatten(),
                arg(&args, "x1").unwrap_or(0.5),
                arg(&args, "y1").unwrap_or(0.5),
                arg(&args, "x2").unwrap_or(0.5),
                arg(&args, "y2").unwrap_or(0.5),
                arg(&args, "durationMs").ok().flatten(),
            )?),
            "android_mirror_key" => to_json(commands::android_mirror_key(
                arg(&args, "device").ok().flatten(),
                arg(&args, "code").unwrap_or_else(|_| "back".to_string()),
            )?),
            "android_set_sdk_path" => to_json(commands::android_set_sdk_path(
                arg(&args, "path").unwrap_or_default(),
            )?),
            "android_set_ndk_path" => to_json(commands::android_set_ndk_path(
                arg(&args, "path").unwrap_or_default(),
            )?),
            "set_android_settings" => to_json(commands::set_android_settings(
                state,
                app,
                arg(&args, "applicationId").ok().flatten(),
                arg(&args, "versionCode").ok().flatten(),
                arg(&args, "versionName").ok().flatten(),
                arg(&args, "keystore").ok().flatten(),
                arg(&args, "keyAlias").ok().flatten(),
            )?),
            "android_create_keystore" => to_json(commands::android_create_keystore(
                arg(&args, "path")?,
                arg(&args, "alias")?,
                arg(&args, "storePass").ok().flatten(),
                arg(&args, "keyPass").ok().flatten(),
                arg(&args, "rememberPasswords")
                    .ok()
                    .flatten()
                    .unwrap_or(false),
            )?),
            // ── The world ──────────────────────────────────────────────────
            "set_mode" => {
                let mode: Mode = arg(&args, "mode")?;
                to_json(commands::set_mode(self, state, app, mode)?)
            }
            "add_scene" => to_json(commands::add_scene(
                state,
                app,
                arg(&args, "name").unwrap_or_default(),
                arg(&args, "mode").ok(),
            )?),
            "duplicate_scene" => to_json(commands::duplicate_scene(
                state,
                app,
                arg(&args, "sceneId")?,
            )?),
            "rename_scene" => to_json(commands::rename_scene(
                state,
                app,
                arg(&args, "sceneId")?,
                arg(&args, "name")?,
            )?),
            "remove_scene" => to_json(commands::remove_scene(state, app, arg(&args, "sceneId")?)?),
            "set_active_scene" => to_json(commands::set_active_scene(
                self,
                state,
                app,
                arg(&args, "sceneId")?,
            )?),
            "set_default_scene" => to_json(commands::set_default_scene(
                state,
                app,
                arg(&args, "sceneId")?,
            )?),
            "scene_components" => to_json(commands::scene_components(
                state,
                arg(&args, "sceneId").ok(),
            )?),
            "set_scene_component" => to_json(commands::set_scene_component(
                self,
                state,
                app,
                arg(&args, "sceneId").ok(),
                arg(&args, "component")?,
            )?),
            "remove_scene_component" => to_json(commands::remove_scene_component(
                state,
                app,
                arg(&args, "sceneId").ok(),
                arg(&args, "name")?,
            )?),
            "import_scene" => to_json(commands::import_scene(state, app, arg(&args, "path")?)?),
            "set_background" => {
                to_json(commands::set_background(state, app, arg(&args, "color")?)?)
            }
            "set_gravity" => {
                let gravity: [f32; 3] = arg(&args, "gravity")?;
                to_json(commands::set_gravity(state, app, gravity)?)
            }
            "set_fixed_rate" => {
                let fixed_rate: f32 = arg(&args, "fixedRate")?;
                to_json(commands::set_fixed_rate(state, app, fixed_rate)?)
            }
            "set_camera" => {
                let camera: Camera = arg(&args, "camera")?;
                to_json(commands::set_camera(state, app, camera)?)
            }
            "read_lighting_asset" => {
                to_json(commands::read_lighting_asset(state, arg(&args, "path")?)?)
            }
            "write_lighting_asset" => to_json(commands::write_lighting_asset(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "lighting")?,
            )?),
            "set_scene_lighting_asset" => to_json(commands::set_scene_lighting_asset(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "sceneId").ok(),
            )?),
            "set_lighting" => {
                let lighting: Lighting = arg(&args, "lighting")?;
                to_json(commands::set_lighting(state, app, lighting)?)
            }
            "save_interface_asset" => {
                to_json(commands::save_interface_asset(state, arg(&args, "name")?)?)
            }
            "load_interface_asset" => to_json(commands::load_interface_asset(
                state,
                app,
                arg(&args, "path")?,
            )?),
            "preview_interface" => to_json(commands::preview_interface(
                self,
                state,
                app,
                arg(&args, "design")?,
            )?),
            "begin_interface_edit" => to_json(commands::begin_interface_edit(
                state,
                arg(&args, "revision")?,
            )?),
            "update_interface_edit" => to_json(commands::update_interface_edit(
                state,
                arg(&args, "token")?,
                arg(&args, "edit")?,
            )?),
            "commit_interface_edit" => to_json(commands::commit_interface_edit(
                state,
                app,
                arg(&args, "token")?,
            )?),
            "cancel_interface_edit" => to_json(commands::cancel_interface_edit(
                state,
                arg(&args, "token")?,
            )?),
            "interface_layout" => to_json(commands::interface_layout(state)?),
            "set_interface" => to_json(commands::set_interface(
                state,
                app,
                arg(&args, "document")?,
            )?),
            "set_sound_mixer" => {
                let mixer: SoundMixer = arg(&args, "mixer")?;
                to_json(commands::set_sound_mixer(state, app, mixer)?)
            }
            "set_display_output" => {
                let display: DisplayOutput = arg(&args, "display")?;
                to_json(commands::set_display_output(state, app, display)?)
            }
            "set_sky" => {
                let sky: blockloom_core::sky::Sky = arg(&args, "sky")?;
                to_json(commands::set_sky(state, app, sky)?)
            }
            "set_fog" => {
                let fog: blockloom_core::fog::Fog = arg(&args, "fog")?;
                to_json(commands::set_fog(state, app, fog)?)
            }
            "set_clouds" => {
                let clouds: blockloom_core::clouds::Clouds = arg(&args, "clouds")?;
                to_json(commands::set_clouds(state, app, clouds)?)
            }
            "set_lighting_2d" => {
                let lighting: blockloom_core::light2d::Lighting2d = arg(&args, "lighting")?;
                to_json(commands::set_lighting_2d(state, app, lighting)?)
            }
            "set_post_2d" => {
                let post: blockloom_core::post2d::Post2d = arg(&args, "post")?;
                to_json(commands::set_post_2d(state, app, post)?)
            }
            "set_cloud_layers" => {
                let layers: Vec<blockloom_core::cloud_layers::CloudLayer> = arg(&args, "layers")?;
                to_json(commands::set_cloud_layers(state, app, layers)?)
            }
            "paint_cloud_layer" => {
                let layer: usize = arg(&args, "layer")?;
                let brush: blockloom_core::cloud_layers::Brush = arg(&args, "brush")?;
                let points: Vec<[f32; 2]> = arg(&args, "points")?;
                to_json(commands::paint_cloud_layer(
                    state, app, layer, brush, points,
                )?)
            }
            "bake_cloud_noise" => to_json(commands::bake_cloud_noise(state, app)?),
            "set_lightning" => {
                let lightning: blockloom_core::lightning::Lightning = arg(&args, "lightning")?;
                to_json(commands::set_lightning(state, app, lightning)?)
            }
            "set_wind" => {
                let wind: blockloom_core::wind::Wind = arg(&args, "wind")?;
                to_json(commands::set_wind(state, app, wind)?)
            }
            "set_director" => {
                let director: blockloom_core::director::Director = arg(&args, "director")?;
                to_json(commands::set_director(state, app, director)?)
            }
            "apply_director_preset" => to_json(commands::apply_director_preset(
                state,
                app,
                arg(&args, "preset")?,
            )?),
            "save_director_preset" => to_json(commands::save_director_preset(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "from")?,
            )?),
            "set_locale" => to_json(commands::set_locale(
                state,
                app,
                arg(&args, "key")?,
                arg(&args, "language")?,
                arg(&args, "text")?,
            )?),
            "remove_locale" => to_json(commands::remove_locale(state, app, arg(&args, "key")?)?),
            "remove_language" => to_json(commands::remove_language(
                state,
                app,
                arg(&args, "language")?,
            )?),
            "set_default_language" => to_json(commands::set_default_language(
                state,
                app,
                arg(&args, "language")?,
            )?),
            "list_locales" => to_json(commands::list_locales(state)?),
            "save_slots" => to_json(commands::save_slots(state)?),
            "delete_save_slot" => to_json(commands::delete_save_slot(state, arg(&args, "slot")?)?),
            "set_vfx" => {
                let vfx: blockloom_core::vfx::VfxSettings = arg(&args, "vfx")?;
                to_json(commands::set_vfx(state, app, vfx)?)
            }
            "set_quality" => to_json(commands::set_quality(state, app, arg(&args, "quality")?)?),
            "set_post_process" => {
                let post: PostProcess = arg(&args, "post")?;
                to_json(commands::set_post_process(state, app, post)?)
            }
            "set_navigation" => {
                let navigation: NavSettings = arg(&args, "navigation")?;
                to_json(commands::set_navigation(state, app, navigation)?)
            }

            // ── Actors ─────────────────────────────────────────────────────
            "select_actor" => to_json(commands::select_actor(state, app, arg(&args, "actorId")?)?),
            "add_actor" => to_json(commands::add_actor(
                state,
                app,
                arg(&args, "shape")?,
                arg(&args, "name").ok(),
            )?),
            "duplicate_actor" => to_json(commands::duplicate_actor(
                state,
                app,
                arg(&args, "actorId")?,
            )?),
            "remove_actor" => to_json(commands::remove_actor(state, app, arg(&args, "actorId")?)?),
            "move_actor" => to_json(commands::move_actor(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "parent").unwrap_or_default(),
                arg(&args, "before").unwrap_or_default(),
            )?),
            "rename_actor" => to_json(commands::rename_actor(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "name")?,
            )?),
            "add_actor_component" => {
                let component: ActorComponent = arg(&args, "component")?;
                to_json(commands::add_actor_component(
                    state,
                    app,
                    arg(&args, "actorId")?,
                    component,
                )?)
            }
            "set_actor_component" => {
                let component: ActorComponent = arg(&args, "component")?;
                to_json(commands::set_actor_component(
                    state,
                    app,
                    arg(&args, "actorId")?,
                    arg(&args, "name")?,
                    component,
                )?)
            }
            "remove_actor_component" => to_json(commands::remove_actor_component(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "name")?,
            )?),
            // ── Physics ────────────────────────────────────────────────────
            "add_collider" => to_json(commands::physics::add_collider(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "collider")?,
            )?),
            "set_collider" => to_json(commands::physics::set_collider(
                state,
                app,
                arg(&args, "collider")?,
            )?),
            "remove_collider" => to_json(commands::physics::remove_collider(
                state,
                app,
                arg(&args, "colliderId")?,
            )?),
            "add_constraint" => to_json(commands::physics::add_constraint(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "constraint")?,
            )?),
            "set_constraint" => to_json(commands::physics::set_constraint(
                state,
                app,
                arg(&args, "constraint")?,
            )?),
            "remove_constraint" => to_json(commands::physics::remove_constraint(
                state,
                app,
                arg(&args, "constraintId")?,
            )?),
            "list_constraints" => to_json(commands::physics::list_constraints(
                state,
                arg(&args, "actorId").ok(),
            )?),
            "fit_collider_to_look" => to_json(commands::physics::fit_collider_to_look(
                state,
                app,
                arg(&args, "colliderId")?,
            )?),
            "set_rigidbody" => to_json(commands::physics::set_rigidbody(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "rigidbody")?,
            )?),
            "remove_rigidbody" => to_json(commands::physics::remove_rigidbody(
                state,
                app,
                arg(&args, "actorId")?,
            )?),
            "set_character_controller" => to_json(commands::physics::set_character_controller(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "controller")?,
            )?),
            "remove_character_controller" => to_json(
                commands::physics::remove_character_controller(state, app, arg(&args, "actorId")?)?,
            ),
            "set_character_motor" => to_json(commands::physics::set_character_motor(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "motor")?,
            )?),
            "remove_character_motor" => to_json(commands::physics::remove_character_motor(
                state,
                app,
                arg(&args, "actorId")?,
            )?),
            "preview_player_preset" => to_json(commands::physics::preview_player_preset(
                state,
                arg(&args, "actorId")?,
                arg(&args, "preset")?,
            )?),
            "apply_player_preset" => to_json(commands::physics::apply_player_preset(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "preset")?,
                arg(&args, "convert").unwrap_or_default(),
            )?),
            "save_player_profile" => to_json(commands::physics::save_player_profile(
                state,
                arg(&args, "actorId")?,
                arg(&args, "name")?,
            )?),
            "list_player_profiles" => to_json(commands::physics::list_player_profiles(state)?),
            "apply_player_profile" => to_json(commands::physics::apply_player_profile(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "name")?,
                arg(&args, "convert").unwrap_or_default(),
            )?),
            "import_player_profile" => to_json(commands::physics::import_player_profile(
                state,
                arg(&args, "path")?,
            )?),
            "set_physics_profile" => to_json(commands::physics::set_physics_profile(
                state,
                app,
                arg(&args, "profile")?,
            )?),
            "set_physics_layer_name" => to_json(commands::physics::set_physics_layer_name(
                state,
                app,
                arg(&args, "layer")?,
                arg(&args, "name")?,
            )?),
            "set_layer_collision" => to_json(commands::physics::set_layer_collision(
                state,
                app,
                arg(&args, "mode")?,
                arg(&args, "a")?,
                arg(&args, "b")?,
                arg(&args, "collides")?,
            )?),
            "physics_plan" => to_json(commands::physics::physics_plan(state)?),
            "add_physics_material" => to_json(commands::physics::add_physics_material(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "material")?,
            )?),
            "set_physics_material" => to_json(commands::physics::set_physics_material(
                state,
                app,
                arg(&args, "id")?,
                arg(&args, "name").ok(),
                arg(&args, "material")?,
            )?),
            "remove_physics_material" => to_json(commands::physics::remove_physics_material(
                state,
                app,
                arg(&args, "id")?,
            )?),
            "physics_check" => to_json(commands::physics::physics_check(state)?),
            "physics_cook" => to_json(commands::physics::physics_cook(state)?),
            "set_physics_cooking" => to_json(commands::physics::set_physics_cooking(
                state,
                app,
                arg(&args, "weld").ok(),
                arg(&args, "maxHullVertices").ok(),
                arg(&args, "mesh").ok(),
                args.get("decompose").cloned(),
            )?),
            "physics_ownership" => to_json(commands::physics::physics_ownership(state)?),
            "physics_properties" => to_json(commands::physics::physics_properties()),
            "physics_migration_preview" => to_json(commands::physics::physics_migration_preview(
                state,
                arg(&args, "actorId").ok(),
            )?),
            "migrate_physics" => to_json(commands::physics::migrate_physics(
                state,
                app,
                arg(&args, "actorId").ok(),
            )?),
            // ── Plugins ────────────────────────────────────────────────────
            "plugin_list" => to_json(commands::plugins::plugin_list(state)?),
            "plugin_check" => to_json(commands::plugins::plugin_check(state)?),
            "plugin_diagnostics" => to_json(commands::plugins::plugin_diagnostics(state)?),
            "plugin_commands" => to_json(commands::plugins::plugin_commands(state)?),
            "plugin_install" => to_json(commands::plugins::plugin_install(
                state,
                app,
                commands::plugins::InstallRequest {
                    id: arg(&args, "id")?,
                    version: arg(&args, "version")?,
                    source: arg(&args, "source")?,
                    features: arg(&args, "features").unwrap_or_default(),
                    dry_run: arg(&args, "dryRun").unwrap_or_default(),
                    offline: arg(&args, "offline").unwrap_or_default(),
                },
            )?),
            "plugin_remove" => to_json(commands::plugins::plugin_remove(
                state,
                app,
                arg(&args, "id")?,
                arg(&args, "dryRun").unwrap_or_default(),
            )?),
            "plugin_update" => to_json(commands::plugins::plugin_update(
                state,
                app,
                arg(&args, "ids").unwrap_or_default(),
                arg(&args, "dryRun").unwrap_or_default(),
                arg(&args, "offline").unwrap_or_default(),
            )?),
            "plugin_pin" => to_json(commands::plugins::plugin_pin(
                state,
                app,
                arg(&args, "id")?,
                arg(&args, "version")?,
                arg(&args, "offline").unwrap_or_default(),
            )?),
            "plugin_sync" => to_json(commands::plugins::plugin_sync(
                state,
                app,
                arg(&args, "dryRun").unwrap_or_default(),
                arg(&args, "offline").unwrap_or_default(),
            )?),
            "plugin_rollback" => to_json(commands::plugins::plugin_rollback(state, app)?),
            "plugin_registry" => to_json(commands::plugins::plugin_registry(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "path")?,
            )?),
            "plugin_data_gc" => to_json(commands::plugins::plugin_data_gc(
                state,
                arg(&args, "dryRun").unwrap_or_default(),
            )?),
            "plugin_gc" => to_json(commands::plugins::plugin_gc(state)?),
            "plugin_inspect" => to_json(commands::plugins::plugin_inspect(arg(&args, "path")?)?),
            "plugin_new" => to_json(commands::plugins::plugin_new(
                arg(&args, "path")?,
                arg(&args, "id")?,
                arg(&args, "name")?,
                arg(&args, "template")?,
                arg(&args, "sdk")?,
            )?),
            "plugin_add_native" => to_json(commands::plugins::plugin_add_native(
                arg(&args, "path")?,
                arg(&args, "target")?,
                arg(&args, "library")?,
            )?),
            "plugin_seal" => to_json(commands::plugins::plugin_seal(arg(&args, "path")?)?),
            "plugin_publish" => to_json(commands::plugins::plugin_publish(
                arg(&args, "path")?,
                arg(&args, "registry")?,
            )?),
            "plugin_migrate" => to_json(commands::plugins::plugin_migrate(
                state,
                app,
                arg(&args, "id")?,
                arg(&args, "dryRun").unwrap_or_default(),
            )?),
            "plugin_importers" => to_json(commands::plugins::plugin_importers(state)?),
            "plugin_imports" => to_json(commands::plugins::plugin_imports(state)?),
            "plugin_import" => to_json(commands::plugins::plugin_import(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "importer").ok(),
            )?),
            "plugin_reimport" => to_json(commands::plugins::plugin_reimport(
                state,
                app,
                arg(&args, "path").ok(),
            )?),
            "plugin_call" => to_json(commands::plugins::plugin_call(
                state,
                app,
                arg(&args, "command")?,
                arg(&args, "args").unwrap_or(Value::Null),
            )?),
            "plugin_run_block" => to_json(commands::plugins::run_block(
                state,
                app,
                &arg::<String>(&args, "actor").unwrap_or_default(),
                &arg::<String>(&args, "plugin")?,
                &arg::<String>(&args, "block")?,
                arg(&args, "args").unwrap_or_default(),
            )?),
            "plugin_trust" => to_json(commands::plugins::plugin_trust(
                state,
                app,
                &arg::<String>(&args, "id")?,
            )?),
            "plugin_untrust" => to_json(commands::plugins::plugin_untrust(
                state,
                app,
                &arg::<String>(&args, "id")?,
            )?),
            "plugin_run_tool" => to_json(commands::plugins::run_tool(
                state,
                app,
                &arg::<String>(&args, "plugin")?,
                &arg::<String>(&args, "tool")?,
                &tool_hits(&args)?,
                &arg(&args, "options").unwrap_or(Value::Null),
            )?),
            "add_plugin_component" => to_json(commands::plugins::add_plugin_component(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "component")?,
                arg(&args, "payload")?,
            )?),
            "set_plugin_component" => to_json(commands::plugins::set_plugin_component(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "component")?,
                arg(&args, "payload")?,
            )?),
            "set_plugin_resource" => to_json(commands::plugins::set_plugin_resource(
                state,
                app,
                arg(&args, "resource")?,
                arg(&args, "payload")?,
            )?),
            "remove_plugin_resource" => to_json(commands::plugins::remove_plugin_resource(
                state,
                app,
                arg(&args, "resource")?,
            )?),
            // ── Assets ─────────────────────────────────────────────────────
            "list_assets" => to_json(commands::list_assets(
                state,
                arg(&args, "path").unwrap_or_default(),
            )?),
            "create_asset_folder" => to_json(commands::create_asset_folder(
                state,
                arg(&args, "parent").unwrap_or_default(),
                arg(&args, "name")?,
            )?),
            "create_asset" => to_json(commands::create_asset(
                state,
                app,
                arg(&args, "parent").unwrap_or_default(),
                arg(&args, "name")?,
            )?),
            "import_assets" => to_json(commands::import_assets(
                state,
                app,
                arg(&args, "parent").unwrap_or_default(),
                arg(&args, "paths")?,
            )?),
            "rename_asset" => to_json(commands::rename_asset(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "name")?,
            )?),
            "move_asset" => to_json(commands::move_asset(
                state,
                app,
                arg(&args, "path")?,
                arg(&args, "parent").unwrap_or_default(),
            )?),
            "delete_asset" => to_json(commands::delete_asset(state, app, arg(&args, "path")?)?),
            "read_asset" => to_json(commands::read_asset(state, arg(&args, "path")?)?),
            "open_asset_location" => {
                to_json(commands::open_asset_location(state, arg(&args, "path")?)?)
            }
            "inspect_asset" => to_json(commands::inspect_asset(state, arg(&args, "path")?)?),
            "pipeline_status" => to_json(commands::pipeline_status(state)?),
            "set_import_role" => to_json(commands::set_import_role(
                state,
                arg(&args, "path")?,
                arg(&args, "role")?,
            )?),
            "bake_normal_map" => to_json(commands::bake_normal_map(
                state,
                arg(&args, "path")?,
                arg(&args, "strength")?,
            )?),
            "preview_normal_map" => to_json(commands::preview_normal_map(
                state,
                arg(&args, "path")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
                arg(&args, "height")?,
                arg(&args, "size")?,
            )?),
            "set_exposure_bias" => to_json(commands::set_exposure_bias(
                state,
                arg(&args, "path")?,
                arg(&args, "ev")?,
            )?),
            "reimport_assets" => to_json(commands::reimport_assets(
                state,
                arg(&args, "paths").unwrap_or_default(),
            )?),
            "pack_atlas" => to_json(commands::pack_atlas(
                state,
                arg(&args, "paths")?,
                arg(&args, "maxSize").ok().flatten(),
                arg(&args, "padding").ok().flatten(),
                arg(&args, "output").ok().flatten(),
            )?),
            "export_shader" => to_json(commands::export_shader(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "path").ok().flatten(),
            )?),
            "check_shader" => to_json(commands::check_shader(state, app, arg(&args, "actorId")?)?),

            // ── Scripts ────────────────────────────────────────────────────
            "create_script" => {
                to_json(commands::create_script(state, app, arg(&args, "actorId")?)?)
            }
            "add_script" => to_json(commands::add_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "remove_script" => to_json(commands::remove_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "path")?,
            )?),
            "move_script" => to_json(commands::move_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "path")?,
                arg(&args, "index")?,
            )?),
            "check_script" => to_json(commands::check_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "read_script" => to_json(commands::read_script(
                state,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "script_toolchain" => to_json(commands::script_toolchain(state)?),
            "script_diagnostics" => to_json(commands::script_diagnostics(
                state,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "format_script" => to_json(commands::format_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "clippy_script" => to_json(commands::clippy_script(
                state,
                arg(&args, "actorId")?,
                arg::<Option<String>>(&args, "path")?,
            )?),
            "script_completions" => to_json(commands::script_completions(
                state,
                arg(&args, "prefix").unwrap_or_default(),
            )?),
            "script_hover" => to_json(commands::script_hover(state, arg(&args, "symbol")?)?),
            "sync_script_ide" => to_json(commands::sync_script_ide(state)?),
            "open_script_ide" => to_json(commands::open_script_ide(state)?),
            "write_script" => to_json(commands::write_script(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "source")?,
                arg::<Option<String>>(&args, "path")?,
            )?),

            "set_actor_visual" => {
                let visual: Visual = arg(&args, "visual")?;
                to_json(commands::set_actor_visual(
                    state,
                    app,
                    arg(&args, "actorId")?,
                    visual,
                )?)
            }
            "set_actor_placement" => {
                let placement: Placement = arg(&args, "placement")?;
                to_json(commands::set_actor_placement(
                    state,
                    app,
                    arg(&args, "actorId")?,
                    placement,
                )?)
            }
            "set_actor_physics" => {
                let physics: Physics = arg(&args, "physics")?;
                to_json(commands::set_actor_physics(
                    state,
                    app,
                    arg(&args, "actorId")?,
                    physics,
                )?)
            }
            "set_actor_visible" => to_json(commands::set_actor_visible(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "visible")?,
            )?),

            // ── Running ────────────────────────────────────────────────────
            "run_project" => to_json(commands::run_project(self, state, app)?),
            "stop_project" => to_json(commands::stop_project(state, app)?),
            "open_lan" => {
                let bind: String = arg(&args, "bind")?;
                let max_guests: usize = arg(&args, "max_guests")?;
                let address: std::net::SocketAddr =
                    bind.parse().map_err(|_| "Use a numeric IP:port")?;
                if address.ip().is_unspecified() || address.ip().is_multicast() {
                    return Err("Select a specific LAN interface".into());
                }
                if !(1..=16).contains(&max_guests) {
                    return Err("Guest limit must be 1 to 16".into());
                }
                to_json(commands::lan_command(
                    state,
                    blockloom_protocol::EditorMessage::OpenLan { bind, max_guests },
                )?)
            }
            "close_lan" => to_json(commands::lan_command(
                state,
                blockloom_protocol::EditorMessage::CloseLan,
            )?),
            "session_status" => to_json(commands::session_status(state)?),
            "kick_guest" => to_json(commands::lan_command(
                state,
                blockloom_protocol::EditorMessage::KickGuest {
                    id: arg(&args, "id")?,
                },
            )?),
            "pause_project" => to_json(commands::pause_project(state, app, arg(&args, "paused")?)?),
            "step_project" => to_json(commands::step_project(state, app)?),
            "close_runtime" => to_json(commands::close_runtime(state, app)?),
            "open_world" => to_json(commands::open_world(self, state, app)?),
            "set_scene_view" => to_json(commands::set_scene_view(
                state,
                arg::<blockloom_protocol::SceneView>(&args, "view")?,
            )?),
            "frame_selected" => to_json(commands::frame_selected(state)?),
            "capture_exr" => to_json(commands::capture_exr(state)?),
            "bake_probes" => to_json(commands::bake_probes(
                state,
                arg(&args, "actors").unwrap_or_default(),
            )?),
            "probe_status" => to_json(commands::probe_status(state)?),
            "paint_tiles" => to_json(commands::paint_tiles(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "brush")?,
                arg(&args, "segments")?,
            )?),
            "import_tileset" => to_json(commands::import_tileset(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "path")?,
            )?),
            "tilemap_stats" => to_json(commands::tilemap_stats(state, arg(&args, "actorId")?)?),
            "add_autotile" => to_json(commands::add_autotile(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "name")?,
                arg(&args, "mode")?,
                arg(&args, "first")?,
            )?),
            "paint_terrain" => to_json(commands::paint_terrain(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "stroke")?,
            )?),
            "import_terrain_heightmap" => to_json(commands::import_terrain_heightmap(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "path")?,
            )?),
            "erode_terrain" => to_json(commands::erode_terrain(
                state,
                app,
                arg(&args, "actorId")?,
                arg(&args, "erosion")?,
            )?),
            "preview_terrain_erosion" => to_json(commands::preview_terrain_erosion(
                state,
                arg(&args, "actorId")?,
                arg(&args, "erosion").unwrap_or_default(),
            )?),
            "set_preview_enabled" => to_json(commands::set_preview_enabled(
                state,
                app,
                arg(&args, "enabled")?,
            )?),
            "set_preview_headless" => to_json(commands::set_preview_headless(
                state,
                app,
                arg(&args, "headless")?,
            )?),
            "preview_input" => to_json(commands::preview_input(
                state,
                arg::<blockloom_protocol::PreviewInput>(&args, "input")?,
            )?),

            // ── Instructions ───────────────────────────────────────────────
            "add_instruction" => to_json(commands::add_instruction(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
                instruction_arg(&args, "instruction")?,
            )?),
            "edit_instruction" => to_json(commands::edit_instruction(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
                instruction_arg(&args, "instruction")?,
            )?),
            "remove_instruction" => to_json(commands::remove_instruction(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
            )?),
            "delete_instruction" => to_json(commands::delete_instruction(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "paste_instructions" => to_json(commands::paste_instructions(
                state,
                app,
                arg(&args, "x")?,
                arg(&args, "y")?,
                instructions_arg(&args, "instructions")?,
            )?),

            // ── Strands ────────────────────────────────────────────────────
            "add_strand" => {
                let instruction = match args.get("instruction") {
                    Some(Value::Null) | None => None,
                    Some(_) => Some(instruction_arg(&args, "instruction")?),
                };
                to_json(commands::add_strand(
                    state,
                    app,
                    arg(&args, "x").ok(),
                    arg(&args, "y").ok(),
                    instruction,
                )?)
            }
            "remove_strand" => to_json(commands::remove_strand(
                state,
                app,
                arg(&args, "strandId")?,
            )?),
            "move_strand" => to_json(commands::move_strand(
                state,
                app,
                arg(&args, "strandId")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "split_strand" => to_json(commands::split_strand(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "merge_strand" => to_json(commands::merge_strand(
                state,
                app,
                arg(&args, "draggedId")?,
                arg(&args, "targetId")?,
                arg::<InstrPath>(&args, "path")?,
            )?),
            "merge_tail" => to_json(commands::merge_tail(
                state,
                app,
                arg(&args, "strandId")?,
                arg::<InstrPath>(&args, "path")?,
                arg(&args, "targetId")?,
                arg::<InstrPath>(&args, "targetPath")?,
            )?),

            // ── Values ─────────────────────────────────────────────────────
            "edit_value_field" => to_json(commands::edit_value_field(
                state,
                app,
                arg::<ValueLocation>(&args, "location")?,
                arg(&args, "text")?,
            )?),
            "set_value_kind" => to_json(commands::set_value_kind(
                state,
                app,
                arg::<ValueLocation>(&args, "location")?,
                arg(&args, "kind")?,
            )?),
            "take_value" => to_json(commands::take_value(
                state,
                app,
                arg::<ValueLocation>(&args, "location")?,
            )?),
            "put_value" => to_json(commands::put_value(
                state,
                app,
                arg::<ValueLocation>(&args, "location")?,
                arg::<BlockValue>(&args, "value")?,
            )?),
            "preview_value" => to_json(commands::preview_value(
                state,
                arg::<BlockValue>(&args, "value")?,
            )?),
            "create_floating_value" => to_json(commands::create_floating_value(
                state,
                app,
                arg(&args, "x")?,
                arg(&args, "y")?,
                arg::<BlockValue>(&args, "value")?,
                arg(&args, "originBlockId").ok().flatten(),
            )?),
            "move_floating_value" => to_json(commands::move_floating_value(
                state,
                app,
                arg(&args, "floatingId")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "remove_floating_value" => to_json(commands::remove_floating_value(
                state,
                app,
                arg(&args, "floatingId")?,
            )?),

            // ── Comments ───────────────────────────────────────────────────
            "create_comment" => to_json(commands::create_comment(
                state,
                app,
                arg(&args, "x")?,
                arg(&args, "y")?,
                arg(&args, "text").unwrap_or_default(),
                arg(&args, "attachedTo").ok().flatten(),
            )?),
            "move_comment" => to_json(commands::move_comment(
                state,
                app,
                arg(&args, "commentId")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "edit_comment_text" => to_json(commands::edit_comment_text(
                state,
                app,
                arg(&args, "commentId")?,
                arg(&args, "text")?,
            )?),
            "set_comment_collapsed" => to_json(commands::set_comment_collapsed(
                state,
                app,
                arg(&args, "commentId")?,
                arg(&args, "collapsed")?,
            )?),
            "remove_comment" => to_json(commands::remove_comment(
                state,
                app,
                arg(&args, "commentId")?,
            )?),

            // ── Variables and custom blocks ────────────────────────────────
            "create_variable" => to_json(commands::create_variable(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "scope").unwrap_or_else(|_| "actor".to_string()),
            )?),
            "rename_variable" => to_json(commands::rename_variable(
                state,
                app,
                arg(&args, "oldName")?,
                arg(&args, "newName")?,
            )?),
            "delete_variable" => {
                to_json(commands::delete_variable(state, app, arg(&args, "name")?)?)
            }
            "create_input_action" => to_json(commands::create_input_action(
                state,
                app,
                arg(&args, "name")?,
            )?),
            "rename_input_action" => to_json(commands::rename_input_action(
                state,
                app,
                arg(&args, "oldName")?,
                arg(&args, "newName")?,
            )?),
            "delete_input_action" => to_json(commands::delete_input_action(
                state,
                app,
                arg(&args, "name")?,
            )?),
            "add_input_binding" => to_json(commands::add_input_binding(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "binding")?,
            )?),
            "remove_input_binding" => to_json(commands::remove_input_binding(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "binding")?,
            )?),
            "clear_input_bindings" => to_json(commands::clear_input_bindings(
                state,
                app,
                arg(&args, "name")?,
            )?),
            "create_list" => to_json(commands::create_list(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "scope").unwrap_or_else(|_| "actor".to_string()),
            )?),
            "rename_list" => to_json(commands::rename_list(
                state,
                app,
                arg(&args, "oldName")?,
                arg(&args, "newName")?,
            )?),
            "delete_list" => to_json(commands::delete_list(state, app, arg(&args, "name")?)?),
            "set_list_items" => {
                let items: Vec<blockloom_core::blocks::ListItem> = arg(&args, "items")?;
                to_json(commands::set_list_items(
                    state,
                    app,
                    arg(&args, "name")?,
                    items,
                )?)
            }
            "set_list_editor_state" => to_json(commands::set_list_editor_state(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "visible")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "create_dict" => to_json(commands::create_dict(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "scope").unwrap_or_else(|_| "actor".to_string()),
            )?),
            "rename_dict" => to_json(commands::rename_dict(
                state,
                app,
                arg(&args, "oldName")?,
                arg(&args, "newName")?,
            )?),
            "delete_dict" => to_json(commands::delete_dict(state, app, arg(&args, "name")?)?),
            "set_dict_entries" => {
                let entries: Vec<blockloom_core::blocks::DictEntry> = arg(&args, "entries")?;
                to_json(commands::set_dict_entries(
                    state,
                    app,
                    arg(&args, "name")?,
                    entries,
                )?)
            }
            "set_dict_editor_state" => to_json(commands::set_dict_editor_state(
                state,
                app,
                arg(&args, "name")?,
                arg(&args, "visible")?,
                arg(&args, "x")?,
                arg(&args, "y")?,
            )?),
            "create_block" => {
                let pieces: Vec<BlockPiece> = arg(&args, "pieces")?;
                let shape: BlockShape = arg(&args, "shape")?;
                to_json(commands::create_block(
                    state,
                    app,
                    pieces,
                    shape,
                    arg(&args, "color")?,
                )?)
            }
            "edit_block" => {
                let pieces: Vec<BlockPiece> = arg(&args, "pieces")?;
                let shape: BlockShape = arg(&args, "shape")?;
                to_json(commands::edit_block(
                    state,
                    app,
                    arg(&args, "blockId")?,
                    pieces,
                    shape,
                    arg(&args, "color")?,
                )?)
            }
            "delete_block" => to_json(commands::delete_block(state, app, arg(&args, "blockId")?)?),

            // ── Undo, log ──────────────────────────────────────────────────
            "undo" => to_json(commands::undo(state, app)?),
            "redo" => to_json(commands::redo(state, app)?),
            "value_kind_exists" => to_json(commands::value_kind_exists(arg(&args, "kind")?)?),
            "push_log" => to_json(commands::push_log(
                state,
                app,
                arg(&args, "kind")?,
                arg(&args, "text")?,
            )?),
            "clear_log" => to_json(commands::clear_log(state, app)?),

            other => Err(format!("Unknown command: {other}")),
        }
    }
}
