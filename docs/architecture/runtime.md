# Runtime

The Game and scene views, simulation registration, per-frame cost, sound and cinematics. Split out of `CLAUDE.md`; the index there lists every page.

## Game view

`Backend::start_embedded` takes an `EmbeddedRuntime` host, and
`RuntimeHandle` then starts the world on a `blockloom-world` thread instead of
spawning a child - same messages, over channels. `bridge::attach` routes the
world's `send` into that channel. A panic in the world is caught and reported
as `Fatal`, ending the run rather than the editor; a native crash (a script
library, a GPU fault) still takes the editor down.

Bevy runs headless there: no winit, one update per frame the view presents
(`embed::paced` waits on `FrameExchange::presented`, which the view calls on
every swap and keeps asking for while a world runs; 50 ms at most, so a hidden
view still hears the editor), and synchronous pipeline compilation, since async compile tasks outliving the
device crash NVIDIA at exit. Cameras render at the view's real pixel size
(`FrameExchange::resize`, debounced by the view, since each size is a new
ring; a fixed resolution picked in the Game tab is sent as is at scale 1),
with `UiScale` and the 2D projection following the view's pixel density, into a ring of three Vulkan images
exported as dma-bufs (`embed.rs`, raw `ash` under wgpu). `FrameExchange` hands
slots between the world and the view: the world never draws into the one
being shown or waiting to be. The C++ `GameView` item imports the ring through
EGL. This is why Qt is forced onto its OpenGL renderer, and onto EGL on X11.

The ring's layout is negotiated. The view offers (`FrameExchange::accept`)
the tiled DRM format modifiers its EGL samples as a plain texture; the world
turns on `VK_EXT_image_drm_format_modifier` through Bevy's `raw_vulkan_init`,
allocates the ring in one of those, and has the cameras draw straight into it
(`aim_cameras` swaps the claimed slot in as their output attachment) - no copy
on either side. With no common modifier, or once the view `refuse`s one it
couldn't import, it falls back to a linear system-memory ring the frame is
copied into, which NVIDIA only samples as an external texture, so the view
copies it again through a small shader into a plain texture.

Status (positions, variables) and the run log reach QML through their own
`statusJson` and `logJson`, not the whole state snapshot - re-evaluating every
binding on each status or `say` stuttered the view. `RunLog.qml` appends by
`log.total` rather than rebuilding its list.

Input is `PreviewInput`, as it is for the MJPEG preview: keys, buttons,
position, text, scroll, touch, plus `focus` (which, once sent, decides whether
the game is focused, and releases held keys and fingers when lost) and
`mouse_delta`. Keys travel by physical position: the view turns Qt's
`nativeScanCode` into a `KeyCode` name through `blockloom_protocol::keys`,
falling back to Qt's key name where there is no scan code (macOS). Scroll and
touch become Bevy's own `MouseWheel`/`TouchInput` messages in
`preview::drain_preview_inputs`. A windowless
world answers `lock mouse` with `RuntimeMessage::PointerLock`; the view then
locks the pointer while it has the keyboard - Wayland pointer constraints and
relative pointer (the generated glue is vendored in `blockloom-qt/src/wayland/`),
cursor warping on X11 - and forwards raw motion. Escape always releases it and
a click takes it back.

HDR frames can't go through the 8-bit ring or Qt's 8-bit window. On Wayland
the view makes a subsurface *under* its window (`makePlane` in
`game_view.cpp`, input passing straight through, sized by `wp_viewporter`)
and offers it through `FrameExchange::offer_hdr_surface`; the world makes a
swapchain on it (`embed::HdrPlane`), which is what `DisplayOffers` then
reports. While the frame resolves HDR, the cameras' view takes the
swapchain's format and `aim_plane` points them at its texture instead of a
ring slot, and `hdr_live` tells the view to draw a zero-alpha texture Qt
thinks is opaque, which shows through to the plane while overlays still
draw on top. That is why the window has an alpha buffer on Wayland.
`BLOCKLOOM_HDR_VIEW=0` turns the plane off.

A project runtime (engine plugins linked in) is a child, but on Linux it
still draws into the Game view: the editor passes it one end of a socket pair
as `--frames-fd`, and `embed::remote` (`embed_remote.rs`) carries its
exchange's ring calls (install, claim, finished, clear, shm) to the editor's
`FrameExchange`, the ring's dma-bufs crossing as `SCM_RIGHTS` descriptors,
while the view's size, modifier offer and presents go back. Input stays on
stdin. `RuntimeHandle::in_view` says which runtime shows there, and
`runtime_embedded` follows it, so a host that can't serve frames falls back
to the preview stream. The ignored `a_child_world_draws_into_the_view_about_as_fast`
times key press to frame both ways (build `blockloom-runtime` first).

On Linux a project with a `Script` on any actor runs as a child by default
(`commands::isolate_runtime`): a script is native code loaded into the world,
and a child costs nothing there because it still draws into the view. The
child is the ordinary runtime, not a project runtime, so nothing has to be
built first. `BLOCKLOOM_RUNTIME=embedded` asks for the in-process world
anyway.

`BLOCKLOOM_RUNTIME=process` forces the child process and MJPEG preview on
Linux and Windows too. macOS has no embedded world yet.

## Scene view

While nothing runs, the Game view is a scene view over the same world:
loaded, never started, and seen through an editor camera instead of the
game's. A run ends the same way whether the Stop button or a `stop all`
ended it (`world::end_run`): the world is rebuilt from the document, so the
scene view never keeps where the run left things. `commands::open_world` brings a world up for it when the tab is shown
(embedded, or with the preview on), so editing never waits for Play.
`blockloom-runtime/src/edit.rs` is all of it: `SceneEditor` holds the camera
(a flying one in 3D, pan and zoom in 2D), the selection and any drag.

Input is the same forwarded `PreviewInput` a game reads, but `edit::interact`
takes it in order ahead of the game's systems, so a press and its release in
one frame still make a click. A click picks the actor under the pointer
(nearest along the ray in 3D, topmost in 2D) and reports `Picked`; a drag on
it or on the gizmo's handles moves the entity's pose live, and the release
reports `Placed`. `commands::place_from_view` writes that into the document
as one undo step, which reloads the world like any other edit. A child placed
in its parent's frame reports its new `offset` too, since that, not `Place`,
is what the world builds it from, and such children follow a dragged parent
live.

An edit reaches an idle world as `EditorMessage::Patch` (`PROTOCOL_VERSION`
39): the `Change`s committed since the world's last copy (`Tracker::
take_runtime`; a run, a new world or a header change sends `Load` instead).
The world applies it to its copy, and when only open-scene actors changed
and none of them reaches past itself (`world::respawnable`: no parent or
children, camera, controller, constraint, terrain, water, room, parallax,
probe, tilemap or mesh collider) `world::respawn_actors` replaces just those
entities and rebakes the navmesh; anything else loads the patched copy, a
full rebuild.

The tool, snapping and steps are `blockloom_protocol::SceneView`, an editor
preference kept in QML `Settings` and re-sent to every world that comes up,
as is the selection (`EditorMessage::Select`). `Place` has a uniform
`scale` (the size blocks read and write) and a per-axis `stretch` in the
actor's own frame; the scale tool's axis handles move the stretch and its
centre handle the size. Handles are gizmo lines in their own `HandleGizmos`
group, drawn over the world. In 3D the editor holds the view's pointer lock
while the right button is down, so looking around reads raw motion.

## Simulation and presentation registration

`blockloom-runtime/src/simulation.rs` is the half of `add_world` that a server
could register alone: `add_simulation` inserts the engine, `Dimension`,
`PendingEffects`, the nav mesh, both Rapier pipelines (in the fixed schedule,
with the collision hooks), motors, controllers, constraints and the whole
fixed-step simulation chain, plus `FixedPostUpdate`'s parenting, pose recording
and contact tracking. The physics debug renderer stays with presentation. Each fixed-step simulation
system sits in a `SimStep` set and the sets are chained in the order the old
single chain ran. The systems that used to sit inside that chain but present
something (interface bindings and effects, exposure, HDR, lights, ray tracing,
fx, sound, input effects) are registered by `add_world` in lib.rs and order
themselves `.after`/`.before` the steps they used to sit between, so the order
is unchanged. A new simulation system is a `SimStep` member; a new
presentation system that reads an effect goes between two steps in lib.rs.

`world/` is split by job: `run` (the editor link, Play and Stop, saves,
pause), `rebuild`, `scripts`, `sensors`, `ui_input`, `geometry`, `vm`,
`effects`, `lifetimes`, `status` and `helpers`, each re-exported from
`world/mod.rs` so `crate::world::name` paths don't change.

`catchup.rs` caps fixed-step catch-up by the measured step cost: after a
hitch a frame runs as many steps as fit in `FRAME_BUDGET` (100 ms), never
more than Bevy's 250 ms. A cheap sim keeps 250 ms; an expensive one turns a
hitch into slow motion instead of a spiral. A test stepping time by hand is
left alone.

Still interleaved, and the next extraction: the `Update` chain (`pump_editor`,
`rebuild_world`, `gather_volumes`/`blend_environment`, `publish_sensors`) and
`rebuild_world`/`apply_lifetimes` themselves, which spawn meshes and materials
next to the entities. The sim systems therefore still take the asset stores
(`AssetServer`, `Assets<Mesh>`, `Assets<StandardMaterial>`, ...) as required
parameters. The tests in `simulation.rs` supply them as plain CPU-side asset
collections over `MinimalPlugins`, which is the headless footing: a 2D and a 3D
project step on the fixed clock with no window or device, the same project steps
to the same place on every run, and a project compiled to native logic steps to
the same place as the VM.

`engine::SceneDocument` is the open scene's `World`, the game settings and
the project folder as a plain resource, republished by `rebuild_world`, so a
system that only reads settings needn't take the main-thread `Engine`. The
sky, fog views, moon, clouds, cloud layers, beams, post, camera quality,
timestep and the asset-folder readers (sprites, rigs, terrain, water, the
interface) read it; a test app gets it through `InsertEngine::insert_engine`.
What's left on `Engine` is run state (the VM, variables, entities, touches,
parents, spawned actors, the live weather blend); no run override lives there.

`blockloom-runtime/src/module.rs` is the module contract: a
`BlockloomModule` (`name`, `build`) added with `AddModule::add_module`
registers its own resources and systems, keeps run state in its own
resources and drops it on the `WorldRebuilt` message (written by
`rebuild_world`), and reads the run through `RunState` (synced from `Engine`
at the head of every tick and frame) and the document through
`SceneDocument`; `module::forget_on_rebuild::<T>` resets a run-override
resource on that message (`RayTracingRun`, `HdrRun`, `space::SkyRun` for `set aurora`, `set fog density`, `set clouds` and `set cloud layer`, `water::WaterRun`, `lightning::StormRun`, `lights::LightsRun` for light intensity, light shadows, shadow distance and `capture probes`, `light2d::Look2d` for every `set [look] to` dial, `volumes::VolumeRun` for `enable volume` and volume weights, `director::DirectorRun` for `set time of day` and `set precipitation`, `environment::SurfaceRun` for the `set_surface` verb, `world::InputRun` for `bind`/`clear`). `cinematic::CinePlayer` holds `set time scale` and the playing reel and clears when the run stops. `wind::WindModule` is the first: `WindRun` holds `set wind`,
and it reads the weather blend through `director::WeatherNow` and volume
specs through `volumes::LiveVolumes`, both published on the fixed tick
before it, so it takes no `Engine` at all.

`volumes::VolumeEye` says where volumes are weighed: the world camera (the
default), a named actor or a point, so a world with no camera blends the same
atmosphere for `sample_atmosphere`.

## Per-frame cost

`publish_sensors` rebuilds the snapshot every frame, so it is the first thing
a crowded scene pays for. It refreshes last frame's `ActorSense` entries in
place (`sense::take_actors`, then `clone_from`) rather than building new ones,
and `Engine::actor` remembers each authored actor's slot in the document list
so per-actor lookups stay O(1). Own-actor reporters read through
`sense::with_me`, which borrows the entry; never clone an `ActorSense` to read
one field. By-name lookups (`Sensors::find`, `count_named`) scan twice and
then use a lazily built `NameIndex` for the rest of the snapshot, and a name
shared by clones answers with the smallest id. Writes through `&mut Transform`
mark it changed, so `interpolate_poses` and `restore_poses` assign only when the
value differs. `docs/performance.md` has the measurements, how to rerun them
(`cargo bench -p blockloom-core --bench vm`, the ignored `sensor_publish_cost`
test) and the open list; GPU numbers there are inferred until profiled.

## Sound

`blockloom-core/src/sound.rs` is the model: a `SoundBus` (`Master`, `Music`,
`Sfx`), a `SoundMixer` of one linear gain per bus saved in the project's `settings`, and
the 0-100 block scale both ways (`user_to_gain`/`gain_to_user`). Blocks are
`play sound` (global), `play sound at` (positional: follows an actor, panned
by Bevy spatial audio with per-frame distance falloff), `stop sound` (empty
stops all), `set sound volume`/`pitch` (live voices of one file), and `set
bus volume` (window-global like gravity). `blockloom-runtime/src/sound.rs`
owns the voices: one entity per play with an `AudioPlayer`, handles cached by
path, oldest stolen past 8 of a file or 128 total. Sounds ignore the pause
freeze (a menu click still clicks), and `stop all` silences them. Reporters
are `is playing?` and `bus volume`, read off the published snapshot.

`blockloom-core/src/soundscape.rs` (`World.soundscape`) layers rules over the
mix: weather loops follow the atmosphere's wind, rain, snow or storm, ducking
dips a bus while paused or while a named sound plays, and the underwater
muffle pulls buses down and dulls them while the camera is under water.
`drive_soundscape` steps the levels on the real clock, blends the listener's
`SoundZone`s over the project reverb (`blend_zones`) and spawns the weather
voices; `SoundState::bus_factor` multiplies the gains into `effective`.
`set-soundscape` writes it, and `set reverb to` / `muffle _ while underwater?`
override it for one run.

Voices don't play the file directly: `maintain_voices` waits for the
`AudioSource` to load, then gives the voice its own `FxSource`
(`sound_fx.rs`), a decoder wrapper with a lowpass and a Freeverb-style reverb
whose parameters are atomics on an `Arc<VoiceFx>` (so a parameter move never
needs the sink). Looping is done in that decoder; Bevy's `repeat_infinite`
would replay processed samples. Per frame each voice's cutoff is the
combination of underwater, zone and occlusion muffle, and its reverb is the
listener's blend for its bus. A WAV impulse response (project or zone) swaps the comb tail for per-channel partitioned FFT convolution (`RawIr`, `Convolver`). `probe_occlusion` (outside the big `Update`
chain, which is at the tuple limit) rays positional voices to the listener
through `QueryAccess` against static scenery only.

## Cinematics

A cutscene is a named reel on the `World` (`blockloom-core/src/cinematic.rs`,
a `Cutscenes` scene component): shots of a camera actor, an FOV and a
length, each with an optional dolly path of look-at/roll/FOV keys eased
between, plus signal, slow-motion and volume-weight keys.
`blockloom-runtime/src/cinematic.rs` plays it on the real clock ahead of the
sensor publish, so it runs through `pause game` like UI strands do. Shots
drive the world camera after the rig and the room confine; signals fire
`when cutscene signal` strands; slow-motion keys scale the virtual clock
the fixed clock follows, so blocks and physics keep their tick; volume keys
borrow weights and restore them at the end; `skip cutscene` fires the
remaining signals before `when cutscene ends`. Trauma shakes the camera on
seeded gradient noise and decays; letterbox bars (z 45) and a black/white
fade wash (z 47) ease in over the interface but under the scene veil.
Blocks are `play cutscene`, `skip cutscene`, `shake camera by`,
`set time scale to`, `hitstop`, `set letterbox to` and `fade screen to`,
with `is cutscene playing?` and `cutscene time` reporters. Reels are
authored as JSON through `set-scene-component`; there is no visual
timeline editor.
