# Blocks to scripts Rosetta

Every block below with the `Actor::` call that does the same thing. Reads
answer from the same snapshot blocks read, writes queue the same effects, and
read-after-write still reads old - so a canvas and a script can drive one
actor between them.

Three rules cover most of the table:

- **Control flow is Rust.** `if`/`repeat`/`while` are `if`/`for`/`while`.
  `forever` is the body of `tick` (it runs every fixed step). `wait 2` is a
  [`Timer`]/[`after`](script-api.md#waiting-without-blocking) fed `dt`;
  `wait until <cond>` is `if <cond>` checked each tick. `break`/`continue`
  are Rust's. `stop everything` is `stop_all`, `broadcast` is `broadcast`.
- **Hats are entry points.** `when the project starts` is `start`,
  everything continuous is `tick(dt)`/`frame(dt)`/`ui(dt)`, and every other
  `when ...` is one `Event` variant matched in `event` (table at the end).
- **Reporters are readers.** `x position` is `x()`, `Ball's x position` is
  `position_of("Ball")` or `pose_of("Ball")`. Cross-actor writes go through
  a handle: `me.world().actor("Ball").go_to(..)`. Actor names, input actions,
  custom fields and shared variables have checked constants under
  `blockloom::symbols` (see below), and dial strings have checked enums
  (`ForceMode`, `WindDial`, `WaterDial`, `CloudDial`, `Precipitation`,
  `Easing`, `Color`).

`dt` is seconds since the last step: multiply motion by it
(`me.move_forward(200.0 * dt)`), exactly like the blocks do internally.

## Motion

| Block | Script |
| --- | --- |
| `move forward (10) steps` | `me.move_forward(10.0)` |
| `go to x y z` | `me.go_to(x, y, z)` |
| `change x by (10)` | `me.change_position(Axis::X, 10.0)` |
| `turn (15) degrees` | `me.turn(Axis::Z, 15.0)` |
| `point in direction (90)` | `me.set_rotation(Axis::Z, 90.0)` |
| `point towards (mouse)` | `me.point_towards("mouse")` |
| `set size to (100)%` | `me.set_scale(1.0)` |
| `navigate to x y z at speed` | `me.navigate_to(x, y, z, speed)` |
| `glide to x y z in (1) secs` | No direct call: move a little each `tick`, or `tween` toward it and poll `is_tweening()` |
| `x/y/z position`, `direction`, `size` | `me.x()`, `me.y()`, `me.z()`, `me.rotation(Axis::Z)`, `me.scale()` - or `me.pose()` for all seven at once |
| `Ball's x position` | `me.position_of("Ball", Axis::X)` - or `me.pose_of("Ball")` |

## Tweens and animation

| Block | Script |
| --- | --- |
| `tween size`, `tween rotation`, `tween color` | `me.tween_scale(f, s, "EaseOut")` - or the checked `me.tween_scale_eased(f, s, Easing::EaseOut)`, `me.tween_rotation_eased(..)`, `me.tween_color_eased(Color::RED, ..)` |
| `stop my tweens` / `is tweening?` | `me.stop_tweens()` / `me.is_tweening()` |
| `play animation (walk)` | `me.play_animation("walk", 1.0)` |
| `stop my animation` / `set animation speed` | `me.stop_animation()` / `me.set_animation_speed(1.0)` |
| `fire trigger (jump)` | `me.fire_animation_trigger("jump")` |
| `swap a rig slot`, `tint a rig slot`, `point a rig IK chain` | `me.set_rig_slot(slot, att)`, `me.set_slot_tint(slot, "#FF0000")`, `me.set_ik_target(name, x, y)` |
| `set a sprite dial` | `me.set_sprite_dial("FlipX", 1.0)` |
| `current clip`, `current frame`, `is playing?` | `me.current_clip()`, `me.current_frame()`, `me.is_animation_playing()` |
| `when an animation ends` / `reaches a marker` | `Event::AnimationEnded(clip)` / `Event::AnimationMarker(marker)` in `event` |

## Looks and light

| Block | Script |
| --- | --- |
| `say (hello)` | `me.say("hello")` |
| `show` / `hide` | `me.set_visible(true)` / `me.set_visible(false)` |
| `set color to (#FF0000)` | `me.set_color("#FF0000")` - or `me.set_color_rgb(Color::RED)` |
| `set exposure`, `set my light brightness`, `set my glow` | `me.set_exposure(ev)`, `me.set_light_intensity(lm)`, `me.set_emissive_strength(s)` |
| `turn HDR output on/off`, `set peak brightness` | `me.set_hdr_output(on)`, `me.set_peak_brightness(nits)` |
| `enable volume`, `set a volume's weight` | `me.enable_volume("Cave", true)`, `me.set_volume_weight("Cave", 0.5)` |
| `capture light probes` | `me.capture_probes()` |
| `set shadow distance`, `turn my light's shadows on/off` | `me.set_shadow_distance(m)`, `me.set_light_shadows(on)` |
| `turn ray tracing on/off`, `set GI bounces/samples` | `me.set_ray_tracing(on)`, `me.set_gi_bounces(n)`, `me.set_gi_samples(n)` |
| `is ray tracing on?`, `ray tracing available?` | `me.ray_tracing_on()`, `me.ray_tracing_available()` |
| `casts shadows?` | `me.casts_shadows("")` (empty names this actor) |
| `set fog density`, `set aurora` | `me.set_fog_density(d)`, `me.set_aurora(kp)` |
| `strike lightning at`, `set lightning storm` | `me.strike_lightning(x, y, z)`, `me.set_lightning_rate(n)` |

## Physics

| Block | Script |
| --- | --- |
| `push` | `me.push(x, y, z)` |
| `add force (Impulse)` | `me.add_force("Impulse", x, y, z)` - or `me.add_force_mode(ForceMode::Impulse, x, y, z)` |
| `add torque (Force)` | `me.add_torque(..)` / `me.add_torque_mode(..)` |
| `set velocity` | `me.set_velocity(x, y, z)` |
| `velocity`, `angular velocity`, `mass`, `is grounded?` | `me.velocity(Axis::X)`, `me.angular_velocity(..)`, `me.mass()`, `me.grounded()` - plus `*_of("Ball")` for others |
| `move controller` / `simple move` | `me.move_controller(x, y, z)` / `me.simple_move_controller(x, y, z)` (answer is a `Moved`) |
| `set controller (radius)` | `me.set_controller("radius", 0.4)` |
| `motor (steer/jump/stop)`, `set motor (walk speed)` | `me.motor_steer(x, y, z)`, `me.motor_jump()`, `me.motor_stop()`, `me.set_motor("walk speed", v)` |
| `motor (speed)`, `motor (state)` | `me.motor_number("speed")`, `me.motor_text("state")` |
| `joint (motor speed)`, `joint angle` | `me.joint("motor speed", name, v)`, `me.joint_number(name, "angle")` |
| `cast ray/ball`, `find in ball`, `find closest` | `me.raycast(..)`, `me.cast_ball(..)`, `me.overlap_ball(..)`, `me.closest(..)` |
| `ray hit?`, `ray distance`, `circle hit?` | `me.ray_hit(..)`, `me.ray_distance(..)`, `me.circle_hit(..)` |
| `hit count`, `hit (field)` | `me.hit_count()`, `me.hit_number(..)` / `me.hit_text(..)` |
| `touching (Ball)?`, `touching anything?` | `me.touching("Ball")`, `me.touching_anything()` |
| `make me solid/a trigger`, `set my collision layer/mask` | `me.set_trigger(false)`, `me.set_collision_layer(1)`, `me.set_collision_mask(0xFF)` |
| `set body`, `set gravity`, `set density`, `set mass` | Blocks only: change the authored components instead |

## Actors and components

| Block | Script |
| --- | --- |
| `create a clone of (Ball)` / `of myself` | `me.create_clone("Ball")` / `me.clone_myself()` |
| `create actor (name) at x y z` | `me.create_actor("Spark", x, y, z)` |
| `delete (Ball)` / `delete myself` | `me.delete("Ball")` / `me.delete_myself()` |
| `how many (Ball) are there?` / `am I a clone?` | `me.actor_count("Ball")` / `me.is_clone()` |
| `attach (Emitter)` / `detach (Emitter)` | `me.attach("Emitter")` / `me.detach("Emitter")` |
| `carrying (Emitter)?` | `me.has("Emitter")` |
| `set (Health hp) to (3)` | `me.set_field("Health", "hp", 3.0)` / `me.field("Health", "hp")` |
| `set my parent to (Ball)` / `clear my parent` | `me.set_parent("Ball")` / `me.clear_parent()` |
| `set my camera view/pitch/fov` | `me.set_camera_view(CameraView::Follow)`, `me.set_camera_pitch(d)`, `me.set_camera_fov(f)` |

## Variables and lists

| Block | Script |
| --- | --- |
| `set (score) to`, `(score)`, `change (score) by` | `me.set_variable("score", v)`, `me.variable("score")`, `me.change_variable("score", 1.0)` |
| `save (score)` / `clear saved (score)` | `me.save_variable("score")` / `me.clear_saved_variable("score")` |
| `add (x) to [log]`, `[log] length`, `item (1) of [log]` | `me.list_add("log", 1.0)`, `me.list_len("log")`, `me.list_number("log", 1)` |
| `insert/text/replace/delete/clear list` | `me.list_insert[_text]`, `me.list_add_text`, `me.list_replace[_text]`, `me.list_delete`, `me.list_clear` |
| dict blocks (`set a dict value`, ...) | Blocks only: scripts share variables and lists, not dicts |

## Sensing and input

| Block | Script |
| --- | --- |
| `timer` | `me.timer()` |
| `key (right arrow) down?`, `mouse down?`, `mouse x/y` | `me.key_down("right arrow")`, `me.mouse_down()`, `me.mouse()` |
| `action (jump) pressed?/held?/value?` | `me.action_pressed("jump")`, `me.action_down(..)`, `me.action_value(..)` - or `symbols::actions::JUMP` for the name |
| `touch count`, `touch (1)` | `me.touch_count()`, `me.touch(1)` |
| `gamepad button down?`, `rumble` | `me.gamepad_button_down("a")`, `me.rumble_gamepad(s, d)` |
| `distance to (Ball)` | `me.distance_to("Ball")` |
| `lock/unlock the mouse`, `is the mouse locked?` | `me.set_mouse_locked(true)`, `me.mouse_locked()` |
| `bind (Space) to action (jump)`, `clear bindings` | `me.bind_action("jump", "Space")`, `me.clear_action_bindings("jump")` |
| `sun elevation`, `time of day`, `wind speed/direction`, `storm` | `me.sun_elevation()`, `me.time_of_day()`, `me.wind_speed()`, `me.wind_direction()`, `me.storm()` |
| `scene luminance`, `frame time`, `draw calls` | `me.frame_time()`, `me.draw_calls()` (luminance stays blocks-only) |

## World, weather and water

| Block | Script |
| --- | --- |
| `set the wind (speed)` | `me.set_wind("speed", v)` - or `me.set_wind_dial(WindDial::Speed, v)` |
| `set clouds (coverage)` | `me.set_clouds(..)` / `me.set_clouds_dial(CloudDial::Coverage, v)` |
| `set a cloud layer`, `set cloud drift` | `me.set_cloud_layer(1, "coverage", v)`, `me.set_cloud_drift(x, y, z)` |
| `set water level/chop/foam` | `me.set_water(..)` / `me.set_water_dial(WaterDial::Level, v)` |
| `water height at`, `is underwater?` | `me.water_at(x, z)`, `me.is_underwater("")` |
| `set time of day`, `advance time` | `me.set_time_of_day(h)`, `me.advance_time(h)` |
| `set rain/snow`, `blend weather` | `me.set_precipitation(..)` / `me.set_precipitation_kind(Precipitation::Rain, v)`, `me.blend_weather("Storm", 3.0)` |
| `set a layer's parallax` | `me.set_parallax("Hills", "x", 0.5)` |
| `paint a tile`, `tile at x y` | `me.paint_tile("", 3, x, y)`, `me.tile_at("", x, y)` |
| `room containing`, `entered room` | `me.room_containing("")`, `me.entered_room()` |
| `active volumes` | `me.active_volumes()` |

## Scenes, time and cutscenes

| Block | Script |
| --- | --- |
| `switch scene to (Cave)` | `me.switch_scene("Cave", "")` |
| `current scene`, `scene names` | `me.current_scene()`, `me.scene_names()` |
| `pause/resume the game`, `is paused?` | `me.set_paused(true/false)`, `me.is_paused()` |
| `set time scale`, `hitstop`, `set letterbox`, `fade screen` | `me.set_time_scale(s)`, `me.hitstop(n)`, `me.set_letterbox(1.0)`, `me.fade_screen("black")` |
| `play/skip cutscene`, `shake camera` | `me.play_cutscene("Intro")`, `me.skip_cutscene()`, `me.shake_camera(0.5)` |
| `cutscene name/time` | `me.cutscene_name()`, `me.cutscene_time()` |

## Sound

| Block | Script |
| --- | --- |
| `play sound (pop)` | `me.play_sound("pop", 100.0, 1.0, false, SoundBus::Sfx)` |
| `play sound at (Ball)` | `me.play_sound_at("pop", "Ball", 100.0, 1.0, false, SoundBus::Sfx)` |
| `stop sound (pop)` (empty stops all) | `me.stop_sound("pop")` |
| `set sound volume/pitch`, `bus volume`, `is playing?` | `me.set_sound_volume(..)`, `me.set_sound_pitch(..)`, `me.set_bus_volume(SoundBus::Music, v)` / `me.bus_volume(..)`, `me.is_sound_playing(..)` |

## Interface

| Block | Script |
| --- | --- |
| `show a panel/label/button/... (id)` | `me.show(&Ui::panel("menu", "Title").sized(300.0, 200.0))` (also `Ui::label/button/image/input/slider/toggle`, `.modal()`) |
| `set (width) of (menu) to`, `text of (menu)` | `me.set_ui("menu", "Width", 300.0)`, `me.set_ui_text("menu", "Text", "hi")` |
| `value/text/caption of (id)`, `is shown?`, `exists?`, `focused` | `me.ui_value(id)`, `me.ui_text(id)`, `me.ui_caption(id)`, `me.ui_shown(id)`, `me.ui_exists(id)`, `me.ui_focus()` |
| `hide (menu)`, `hide all ui`, `delete (menu)` | `me.hide_ui("menu")`, `me.hide_all_ui()`, `me.delete_ui("menu")` |
| `focus (name)`, `clear focus` | `me.focus_ui("name")`, `me.clear_ui_focus()` |
| `set the ui theme` | `me.set_ui_theme(UiTheme::Light)` |

## Hats to events

| Block | Script |
| --- | --- |
| `when the project starts` | `start` entry point |
| `when I start as a clone` | `start` runs again for the clone (branch on `is_clone()` if it differs) |
| `when I get (go)` | `Event::Message(message)` - sent by `broadcast("go")` |
| `when a key/action is pressed` | `Event::Key(key)` / `Event::Action(name)` - or poll `key_down`/`action_pressed` in `tick` |
| `when I am clicked` / `screen touched` | `Event::Clicked` / `Event::Touched` |
| `when I touch (Ball)` | `Event::Collision { with, .. }`, phases in `Event::Contact { phase, .. }` (`ContactPhase::Enter/Stay/Exit`) |
| `when my particles spawn/die/collide` | `Event::Particles { kind, count, at }` (`ParticleKind::Spawn/Die/Collide`) |
| `when an element is clicked` / `an input changes` | `Event::UiClicked(id)` / `Event::UiChanged { element, value }` |
| `when scene starts/ends` | `Event::SceneStarted` / `Event::SceneEnded` |
| `when I enter (Cave)` | `Event::EnteredRoom(name)` - or poll `entered_room()` |
| `when weather becomes (Storm)` | `Event::Weather(name)` |
| `when cutscene signal (door)` / `ends` | `Event::CutsceneSignal(name)` / `Event::CutsceneEnded` |
| `when quality drops` | blocks only |
| `when an interface event occurs` | `Event::Ui { .. }` |
| `plugin block` / `when a plugin event occurs` / `plugin reporter` | `me.plugin_call(..)`, `Event::Plugin { .. }`, `me.plugin_number(..)` / `me.plugin_text(..)` |

## Checked names

Stringly names have checked spellings. Prefer the right column; the left
still builds.

| Instead of | Write |
| --- | --- |
| `"Ball"` / `"jump"` / `"score"` | `symbols::actors::BALL`, `symbols::actions::JUMP`, `symbols::variables::SCORE` |
| `me.variable("score")` | `me.variable(symbols::variables::SCORE)` |
| `me.field("Health", "hp")` | `me.field(symbols::components::health::NAME, symbols::components::health::HP)` |
| `"Force"` / `"Impulse"` | `ForceMode::Force` / `ForceMode::Impulse` with `add_force_mode` / `add_torque_mode` |
| `"speed"` (wind) | `WindDial::Speed` with `set_wind_dial` |
| `"chop"` (water) | `WaterDial::Chop` with `set_water_dial` |
| `"coverage"` (clouds) | `CloudDial::Coverage` with `set_clouds_dial` |
| `"snow"` (precipitation) | `Precipitation::Snow` with `set_precipitation_kind` |
| `"EaseOut"` (tweens) | `Easing::EaseOut` with `tween_scale_eased` / `tween_rotation_eased` / `tween_color_eased` |
| `"#FF0000"` | `Color::RED` (or `Color::new(r, g, b)`) with `set_color_rgb` |

`symbols` comes from the project at build time: renaming an actor, action,
custom field, variable or list rebuilds every script, and a stale name fails
compile instead of silently reading zero. It is always available as
`blockloom::symbols` - no import, no setup.
