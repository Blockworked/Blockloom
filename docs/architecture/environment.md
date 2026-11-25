# Environment and lighting (3D)

Environment blending and volumes, HDR, post, sky, fog, wind, water, VFX, decals, lights, ray tracing, clouds, texturing and terrain. Split out of `CLAUDE.md`; the index there lists every page.

## Environment

`blockloom-runtime/src/environment.rs`. `Environment` is the one resource
the background, sun, ambient, AO, exposure and post come from: each frame
`blend_environment` starts from the project's `World` settings, lays
`EnvironmentVolumes` over them by weight (numbers and colors lerp, switches
flip at half weight), and resolves exposure through `ExposureClaims`
(director beats auto-exposure beats the manual EV). `apply_environment`
writes it onto the world camera, the sun and `ClearColor` when it changes or
a rebuild spawns new ones, and it is extracted to the render world. Passes
read `Environment`, never `project.world.lighting`/`post`; the rebuild only
spawns a bare camera and sun.

## Environment volumes

`blockloom-core/src/volume.rs` is the model: a `Volume` component
(`VolumeSpec`) makes an actor a box, a sphere or a global layer with a
priority, a blend distance (outside the shape, in world units) and a weight.
Every property carries an HDRP-style override checkbox (`Override { on,
value }`, the value kept while unchecked), so a volume only changes what it
ticks. `coverage` is the shape maths, flat in 2D; `blend_order` sorts lowest
priority first so the highest wins.

`blockloom-runtime/src/volumes.rs` is the rest. `gather_volumes` weighs every
carried volume at the world camera (the editor camera in the scene view),
fills `EnvironmentVolumes` for `blend_environment`, and keeps the list in
`VolumeBlend`, which the atmosphere sample turns into `active volumes` and the
status reports as `Status.volumes`. `enable volume` and `set weight of volume`
(and a script's `enable_volume`/`set_volume_weight`) name a volume the way
`set my parent to` names a parent and last for the run
(`volumes::VolumeRun`). The debug views are
`SceneView.volumes`: bounds with their blend feather, a heat map, and freeze,
which holds the blend and sends each property's lerp as
`Status.volume_trace`. The heat map is `volume_heat.rs`, a pass after
tonemapping that puts each pixel back in the world (the depth prepass in 3D,
the screen plane in 2D) and runs the same coverage maths in
`shaders/volume_heat.wesl` over at most `MAX_VOLUMES`; change the two
together. A selected volume shows grips in the scene view (`edit.rs`): a box
face moves with the opposite face held, a sphere's edge sets the radius, and
the outer ring the blend distance. The drag updates the world's copy of the
spec live, and lands as `Placed` with `volume` set, one undo step. A
property the Phase 5 systems add is one field on `VolumeOverrides`,
`EnvironmentOverride` and `Environment`, plus its row in the inspector's
`volumeProperties`.

## HDR frame and lights

Every world camera carries `Hdr` (`environment::apply_environment`), bloom or
not: the scene renders linear FP16 and only the end of the chain makes display
values, so lights, sky and emissives can pass 1.0. A build made with HDR off
(`GamePack.hdr`, the Build dialog's switch) renders 8-bit instead. Exposure
stays the one EV on `Environment`; `set exposure to` (and a script's
`set_exposure`) takes `ExposureClaims::director` for the rest of the run, and
the atmosphere slot's `exposure` reading reports the resolved value.

`GameSettings.display` is the project's output: SDR, HDR10 (PQ) or scRGB, peak
brightness and paper white in nits. `hdr::HdrFrame` resolves it, with `set HDR
output`/`set peak brightness` laid over it for the run (`HdrRun`), against what the
window's display offers (`DisplayOffers`), and is extracted so both worlds
switch on the same frame. Only the windowed player can leave SDR:
`display.rs` makes the window's surface itself in the render world, keeps it
when the display offers an HDR color space, and hands Bevy's views its
texture each frame; otherwise Bevy keeps the window. On an HDR frame the
tonemapper stands aside for `HdrTone*` (a curve to the display's headroom)
and `HdrEncode*` (scRGB or PQ, after the UI so the HUD sits at paper white).
An HDR swapchain also gets HDR10 static metadata
(`display::send_metadata`: `VK_EXT_hdr_metadata`, turned on by
`add_vulkan_extensions`, or DXGI), resent whenever the swapchain is remade.
The Game view's ring is 8-bit, so on Wayland an HDR frame bypasses it (see
Game view).

`hdr.rs` also holds the Game view's debug views, a `FullscreenMaterial` per
dimension over the exposed image before tonemapping (`shaders/hdr.wesl`): false
color, a clipping zebra past paper white (past the headroom when the project
wants HDR), a histogram, a waveform, calibration patches and an HDR preview.
The choice is `SceneView::debug_view`, an editor preference that applies while
a game runs too. `luminance.rs` meters the world camera's exposed image with a
compute pass and reads it back; the atmosphere sample turns it into the
`scene luminance` reporter's nits on the fixed tick, beside `is HDR display?`
and `peak brightness`. The meter, the post stack's HDR pass, the debug views and the tone
curve run after everything Bevy's tonemapper runs after: `hdr::ToneInputs`, a schedule build
pass, adds those edges, since passes that flip the main texture must be
ordered or they submit out of the order they flipped in. `capture.rs` answers `EditorMessage::CaptureExr`: a
second camera renders the same view untonemapped into FP16, read back and
written as OpenEXR (`capture_exr`, the Game view's camera button).

The sky (below) is drawn and lights the world in the same FP16 frame. An
HDR asset's exposure bias (`set_exposure_bias`) is applied wherever it is
loaded.
`set my glow to` gives an actor its own material with the emissive scaled
(`dim3::set_glow`).

A `Light` component is a point or spot light in lumens with a range in
metres, 3D only. `lights::sync_lights` reconciles each actor's light against
`engine.attached`, the authored spec and `LightsRun::intensity` (what
`set my light to` wrote this run), and hangs it on a child entity, since
batching hides a merged actor through an empty `RenderLayers`.

The GPU half is checked by the ignored tests in `embed.rs` (`cargo test -p
blockloom-runtime -- --ignored embed`), which read pixels back from a real
world: false color in both dimensions, a lamp that still lights the floor
after batching, and an EXR capture that stays linear.

## Post stack

`World.post` (`scene::PostProcess`) is the whole camera finish, both
dimensions unless noted, in a fixed HDR-first order: motion blur, depth of
field, chromatic aberration and vignette (Bevy's own, set in
`environment::apply_environment`), then Blockloom's HDR pass (bloom, white
balance, grading, the toe/shoulder `ToneShape`), the tonemapper (AgX and
Khronos Neutral included), FXAA/SMAA and CAS sharpening, then Blockloom's LDR
pass (LUT and film grain). Every property volumes can blend has a field on
`VolumeOverrides` and `EnvironmentOverride`, and `Environment.post` holds the
blended chain; the LUT and dirt files, bokeh, metering mode and AO intensity
stay the project's.

`post.rs` is Blockloom's half. `apply_post` resolves each world camera's
`PostStack` from the blended environment, loading the LUT through
`pipeline::load_volume` into a 3D image and the lens dirt as an asset.
`post_hdr` (in `hdr::ToneInputs`, after `luminance::MeterPass`) runs a
five-level bloom chain at half resolution (Karis-weighted soft-threshold
prefilter, 13-tap downsamples, tent upsamples scattering each level into the
one above) and a composite that adds bloom times dirt, then grades.
`post_ldr` (`PostLdrPass`, after the tonemapper and `cas`) works on display
values; the LUT is skipped under HDR output. `shaders/post.wesl` holds every
entry point with unique binding numbers, and `PostUniforms` matches it field
for field. Bevy's `Bloom` is never used. The debug views `BloomMip`
(`SceneView::bloom_mip`), `CircleOfConfusion` and `AmbientOcclusion` are
drawn by `post_hdr` and bypass the tonemapper.

Auto-exposure (3D) reads the meter's average, center-weighted or spot
log-average (`luminance.wesl`), aims for middle grey plus compensation inside
`min_ev..max_ev` and moves at `speed_up`/`speed_down` stops a second
(`AutoExposure::target`/`step`); `post::auto_expose` writes that into
`ExposureClaims::auto`, so the director track still wins. Depth of field
focuses on the named actor's depth along the view (`focus_depth_of_field`),
maps Hexagonal/Circular bokeh to Bevy's Bokeh/Gaussian, and a negative
`max_depth` tells the `pbr_patch` edit of Bevy's `dof.wesl` to keep the near
field sharp. AO intensity is a power on SSAO visibility, baked into Bevy's
`ssao.wesl` by `pbr_patch` in twentieths (each patch re-applies only when its
own `key` changes). SSR turns opaque surfaces deferred through
`ray_tracing::apply_ray_tracing`, and it and SSAO turn MSAA off
(`Environment::wants_msaa_off`). EXR captures copy only the bloom
(`PostStack::linear_only`). The GPU half is the ignored `embed` tests
(grading, every bloom level, grain, AO and blur size debug views).

## Sky

`World::sky` (`blockloom-core/src/sky.rs`) is one of four kinds: `Flat` (the
background color and flat ambient, the default), `Physical` (Rayleigh, Mie
and ozone single scattering, a limb-darkened sun and a phased moon, a night
tint ramped on the sun), `Gradient` (three stops that warm near a low sun)
and `Hdri` (a panorama or strip, turned and tilted, tinted, optionally
blurred). `SunPlacement` decides where the sun stands for every kind - the
lighting's own direction, azimuth and elevation, or NOAA's solar position
from latitude, longitude, day and hour - and `Environment::from_world`
takes it from there, so volumes still override it. A physical sky's air
then reddens and dims the light (`Environment::through_air`, the CPU half of
the same model, kept as `Sun::above_air` for the sky to scatter). Old
documents' `lighting.sky` path becomes an HDRI sky in `Project::normalize`.

`blockloom-runtime/src/sky.rs` renders it, 3D only. `SkyParams` (the WESL
struct in core's `shaders/sky.wesl`, the `blockloom::sky` library module,
field for field) is resolved each frame from the sky and the blended
`Environment`. When it changes, `shaders/sky_cube.wesl` writes the sky's
light, disks left out, into a 256 cube and a 32 cube scaled by the ambient
dimmer; Bevy's `GeneratedEnvironmentMapLight` filters them on two helper
entities (`SkyProbe`), and the world camera's `EnvironmentMapLight` takes
reflections from one and diffuse from the other, or a black cube for
whichever the sky's toggles leave out. The filter stops a few frames after
the render world reports the cubes written (`SkyRender::written`) and the
pipeline backlog is empty, so a still sky is filtered once. Cubes store
nits over `SKY_UNIT` to stay inside FP16. The background is
`shaders/sky_background.wesl`, drawn per pixel in Bevy's opaque pass
through the skybox slot (`SkyView` supplies `SkyboxPipelineId`/
`SkyboxBindGroup`, never a `Skybox`): gradients per pixel with dither, the
physical sky from its cube plus analytic disks, an HDRI sharp from its image
or blurred from the filtered mips. Probe faces and EXR captures copy
`SkyView` (probe faces without the disks). A build bakes an HDRI to a BC6H
cube with its mip chain (`build::bake_sky`, seam fix applied) and drops the
source. `sky_exposure` and `ambient_dimmer` are volume properties too.

## Fog, space and lightning

`World::fog` (`blockloom-core/src/fog.rs`) is three media, 3D only: an
analytic exponential height fog, a froxel volumetric fog and aerial haze.
`blockloom-runtime/src/fog.rs` resolves them from the blended `Environment`
into `FogRender` each frame (so volumes, `set fog density to` and the time
of day reach them the way they reach the sun) and draws them in one pass
after the main passes and before any post. `shaders/fog_froxels.wesl`
injects each froxel's medium (height-falling density, FBM noise drifting
with `noise_wind`, and up to `MAX_LOCAL_FOG` local fog volumes) and its
in-scattered light: Bevy's directional lights read straight from its own
`GpuLights` and cascade shadow maps (the sun and moon carry
`VolumetricLight` while volumetric fog asks for them, which is what makes
light shafts), plus up to `MAX_FOG_LIGHTS` unshadowed point and spot
`Light`s whose `volumetric` switch is on. It blends with last frame's grid
reprojected, and a second entry point integrates each column front to back.
`shaders/fog_composite.wesl` then reads each pixel's distance from the depth
prepass: haze on surfaces, height fog along every ray (the sky's too, so
the horizon melts into it, glowing with the sky's horizon light from the
diffuse cube), then one froxel fetch. The shared WESL is the
`blockloom::fog` library module; `fog::FogUniforms` matches it field for
field. Fog properties on `Environment` (`fog_density`, `fog_colors`,
`fog_height`, `volumetric_density`, `volumetric_albedo`, `haze`) are volume
properties too, and a `Volume` can also add local fog in its own shape
(`VolumeSpec::fog`).

A light's `beam` (`fog::Beam`) is extra medium only that light scatters:
density, its own g, a falloff curve over the range and near/far fades,
mirrored by `beam_fade` in `blockloom::fog`. In the froxels it rides
`FogLight.beam`, and a beam alone runs the pass. `fog::pick_fog_lights` keeps
beams first, then the nearest, up to `MAX_FOG_LIGHTS`. `Environment.beams`
multiplies every beam (a volume property too), and `set fog density`
(`Environment::set_fog_density`) scales it and `volumetric_density` by the
asked density over the project's own, so a clear day has no beams.
`blockloom-runtime/src/beams.rs` is the geometry: a `BeamMode::Auto` beam
becomes an additive fresnel-faded shaft cone (`shaders/beam_shaft.wesl`,
spots only) while volumetric fog is off or Low, and `Motes` are GPU-placed
billboards (`shaders/beam_motes.wesl`) in a beam or, as
`VolumetricFog::dust`, in a box wrapped round the camera.

Stars, the Milky Way and aurora (`Sky::stars`, `Sky::aurora`) are drawn by
the sky's background pass in the main view only, never in probe faces or
the light cubes, from `space::SpaceRender` (`blockloom::space`). They sit in
their own uniform rather than `SkyParams` because they move every frame,
and a `SkyParams` change rewrites and refilters the cubes. So there are no
stars over a flat sky. `space.rs` also hangs the moon's own
`DirectionalLight` (`MoonLight`) where the physical sky puts it, dimmed by
its phase, and applies `set aurora to KP`.

`World::lightning` (`blockloom-core/src/lightning.rs`) is both dimensions.
`StormDirector` is plain arithmetic on the fixed tick from a seed, so one
storm replays the same. `blockloom-runtime/src/lightning.rs` turns strikes
(the director's and `strike lightning at`'s) into a decaying `PointLight`
above the ground (3D), a `LightningFlash` that pulses the environment's
ambient and background and the sky pass, and thunder late by distance at
the speed of sound: the project's sound, or `thunder_wav`, a rumble made in
code so no asset is needed. The fog, the aurora and the flash fill their
`AtmosphereSources` readings (`fog density`, `aurora`, `lightning`).

The GPU half is the ignored `embed` tests: height fog, volumetric glow,
sunlit fog and a roof's shadow in it, a volume's local fog, aurora, stars
and a lightning block.

## Wind

`World::wind` (`blockloom-core/src/wind.rs`) is the one wind, both
dimensions: a direction (degrees clockwise from north, -Z in 3D and up the
screen in 2D), a speed at the reference height, gusts on seeded 1D gradient
noise that also veer the direction, a log-law profile towards the ground (3D
only), and a storm dial that scales the rest (`StormScale`). A `Volume` can
carry a `LocalWind` zone (override, add or swirl round the actor's up axis,
plus turbulence), blended in `blend_order` like the rest of a volume.
`CloudDrift` is how the clouds ride it: the wind at their altitude times
`follow`, their own drift, an erosion drift and a time-lapse; `CloudOffsets`
integrates it. The public API speaks arrays because core's glam isn't
Bevy's.

`blockloom-runtime/src/wind.rs` steps it on the fixed tick's own clock
(`step_wind`, before `sample_atmosphere`), so a replay gusts the same and
the atmosphere slot's `wind speed`, `wind direction` and `storm` are the
wind at the camera that tick. Everything that moves with the air reads the
resulting `WindField`: particles ease into `field.at(position)` scaled by
their emitter's `wind`, the fog's noise scrolls by `field.drift`, and the
cloud passes are meant to take `field.clouds`. `set wind [dial] to` and
`set cloud drift to` (and a script's `set_wind`/`set_cloud_drift`) land in
`wind::WindRun` for the run.

## Water

A `Water` component (`blockloom-core/src/water.rs`, `WaterSpec`) makes an
actor an ocean, a lake or a river: the actor's `Place` is the surface's
centre at rest and its yaw turns the rectangle and a river's `flow`. An ocean
has no edge and reaches `OCEAN_REACH` from the camera. The surface is 8-12
Gerstner waves (`WaveSet::build`, seeded, lengths falling from the authored
wavelength, heading the project's wind or the spec's own), resolved each
tick with the sea state and chop laid on (`WaveSet::resolve`, horizontal
amplitudes shrunk before a crest folds). The sea state follows the wind at
the body through a fetch-limited JONSWAP fit (`sea_target`, eased by
`settle_sea`). Phillips-spectrum detail waves only bend the shading normal.
`WaterBody` is one body resolved for a tick, and `sample` inverts the
Gerstner displacement to answer height, normal, velocity and pinch
(`jacobian`, where crest foam starts) over any point. `shaders/water.wesl`
(`blockloom::water`) sums the same waves from the same numbers, so change the
two together. A 2D body's waves run along x only (`WaveSet::flattened`).
Past `CALM_FROM` wavelengths from the camera the drawn waves fade flat, and
`WaterBody::calm` makes the CPU sample fade the same way, so buoyancy far out
rides what is drawn. `RippleField` is a height field over the waves, stepped
by the wave equation (`RippleSim`): `sample` adds its height, slope and rate,
and `ripple` in `water.wesl` reads the same grid from a texture.

`blockloom-runtime/src/water/` is the rest. `sample_water` runs at the head
of each fixed tick after `sample_atmosphere`, fills `WaterState` (what the
surfaces draw) and `WaterSample`, publishes the latter with
`sense::publish_water`, and `publish_sensors` copies it into each frame's
snapshot, so `water height at`, `is _ underwater?` and a script's
`water_at`/`is_underwater` read what buoyancy read. `set water level/chop/
foam to` (and a script's `set_water`) lands in `water::WaterRun`
(`WaterOverrides`): on the water actor that ran it, or every body from
anyone else. The swell follows the live wind: `Swell` eases a heading
towards it and, once the waves are `TURN_THRESHOLD` off, builds the new
heading's set and fades between the two over `waves.turn` seconds
(`blend_waves`), since rotating a wave in place slides its phase. Each body
with `ripples.enabled` keeps a `RippleSim` covering it whole, or following the
camera when it is bigger than `ripples.extent`, and hands a snapshot to its
`WaterBody` each tick. A `Buoyancy` component (`BuoyancySpec`) makes a dynamic body
float: `float_bodies_2d`/`3d` push up at 1, 4 or 8 sample points by what each
displaces (`buoyant_force`, so density 0.5 rests half under), drag each
towards the water's own velocity and damp spin, all through rapier's
`ExternalImpulse` with the mass read back through `ReadMassProperties`. The
same system splashes any rigid body that crosses the surface faster than the
body's `splash.min_speed`: droplets (`fx::spawn_splash`), a dip in the ripple
field and the body's splash sound, and a floating body's points that cut the
surface stir the field as it moves (its wake). The surfaces read the field as
an `Rg32Float` texture of height and last tick's height, lerped between
ticks.

3D (`surface.rs`): each body is its own entity (`WaterSurface`, left out of
ray-traced proxies), a grid over its rectangle or rings round the camera for
an ocean, drawn with `WaterMaterial` (`shaders/water_surface.wesl`). Its
base `StandardMaterial` has transmission, which puts it in Bevy's
transmissive pass and so after the depth prepass; `sync_surfaces` gives the
world camera `ScreenSpaceTransmission` while any water exists. The fragment
refracts the opaque frame (`view_transmission_texture`), absorbs it by the
distance to the bed (Beer), lays caustics on the bed, foams along the shore
and on pinched crests, reflects by screen-space march, then the body's probe
(`ProbeRequest::water`, following the camera over an ocean through
`ProbeService::relocate`), then the background color, and lets Bevy's
lighting add the lit water color and the sun's GGX glint. `Planar`
reflections instead come from a mirror camera (`mirror.rs`): the world camera
mirrored under the surface and turned upside down, which keeps triangle
winding, so the surface samples it with v flipped. Its `MirrorProjection` is
an oblique reverse-z perspective whose near plane is the water (`oblique`),
so nothing under the surface blocks it; roughness blurs it with a disc of
taps. Each body also has an opaque floor at its depth, which is what the
depth prepass sees where the water is. While water exists the world camera's
main depth is bindable, and the fog pass reads that instead of the prepass,
so fog is measured to the surface rather than the floor under it. `under.rs` is a
pass after the fog that absorbs every ray over its underwater part and lays
caustics while the camera is under a surface. 2D (`flat.rs`): a strip from
the surface to the depth, its top row riding the waves (`water_2d.wesl`).
The GPU half is the ignored `embed` tests `a_lake_tints_the_floor_under_it`
and `a_planar_mirror_reflects_what_stands_over_the_water`; on lavapipe the
water's pipelines take a few hundred frames to compile.

## Particles and VFX

An `Emitter` component (`ParticleSpec`, `blockloom-core/src/material.rs`)
carries the whole VFX graph (`blockloom-core/src/vfx.rs`): a spawn
`SpawnShape` and `LaunchDirection`, `rate` plus timed `Burst`s on the
emitter's own clock (`vfx::due`), an ordered `UpdateModule` stack,
`ParticleRender` (blend, facing, flipbook, `Curve`s and a `Gradient` over
life, baked to `LUT` entries for the GPU) and a `RibbonSpec`. `Particle` is
one 64-byte slot, byte for byte `Particle` in `shaders/vfx.wesl` (the
`blockloom::vfx` library module), whose hash and noise match core's, so the
two sims spray the same way.

Two sims fill one buffer. `vfx::Pool` is the CPU one: 2D, `SimMode::Cpu`,
`VfxSettings::cpu_only`, devices without compute and `RibbonSource::Actor`
(slot 0 pinned to the actor) run on it, up to `CPU_MAX`, colliding with
actors' sensor shapes through `physics_query::segment_contact`. Everything
else in 3D runs `blockloom-runtime/src/shaders/vfx_sim.wesl`: `begin` resets
one `EmitterState`, `simulate` steps every slot, spawning claims slots
through an atomic budget, and `Collide` tries the frame's actor shapes first
(`collider_shapes`, the nearest `GPU_COLLIDERS` bodies' sensed boxes and
balls, the emitter's own left out), then the depth prepass (reverse Z), so
bodies off screen or hidden still stop particles.
`SimParams` in `vfx.rs` matches the WGSL field for field; change the two
together, and `ParticleLook` with `vfx_particles.wesl`.

`blockloom-runtime/src/vfx.rs` is the main-world half: `step_emitters`
(Update, after `interpolate_poses`) keeps a `Draws` entry per emitter (the
buffers, one or two slot meshes, the `ParticleMaterial`s and the sim),
decides what spawns within the project's budget, steps the pool or pushes a
`GpuStep`, and fires `Event::Particles` once a frame per kind while a run
is live. A mesh surface spawns on the actor's own mesh and every mesh
under it (a model's parts), gathered in the actor's frame. `vfx/gpu.rs` is
the render-world half: pool bytes land through
`VfxFrame` uploads, and the compute pass runs once a frame in `Core3d`
between the prepass and the main pass. GPU counts come back through a
`Readback` of the state buffer, tagged with a step sequence so each set is
used once and tied to its pool's reader so resizing cannot reuse old counts;
each event kind carries where its last one happened, and
`ParticleSenses` hands counts and positions to `publish_sensors` for the
particle reporters and a script's `particles()`. Overdraw is counted, not
estimated: every particle fragment adds to one `OverdrawMeter` counter,
zeroed each frame and read back as fragments over the view's pixels.
`vfx/render.rs` is the draw: the mesh's positions only name a
slot, a corner and a ribbon segment, and `vfx_particles.wesl` (3D) or
`vfx_particles_2d.wesl` reads the particle. A lit 3D particle receives
shadows and, with `translucency`, adds the light from behind it. While nothing runs, the actor
the scene view has selected plays its emitter on a loop of `duration`.
Draws go on a rebuild (`fx::despawn_fx`). `fx.rs` is now only splash
droplets and trails. Counts reach the profiler as `vfx/*`. The emitter is a module stack in the order VFX Graph runs its contexts
(spawn, initialize, update, output), not a node graph. The GPU half is the
ignored `embed` tests: GPU and CPU particles draw, a GPU spark lands on a
hidden body, ribbons draw without heads, and the overdraw meter counts.

## Decals

`blockloom-core/src/decals.rs` owns transient marks: a 256-entry LRU pool,
lifetimes and radius fades, plus four matching atlas pages for blood,
footprints and fresh scorch. `SpawnDecal` takes a position, outward normal,
size, lifetime and final fade; `FadeDecals` fades centres within a sphere.
Both run through the VM and compiled logic. Marks are never saved.

`blockloom-runtime/src/decals.rs` projects the atlas with Bevy's clustered
decals. `pbr_patch` appends `shaders/decals.wesl` to Bevy's decal module:
the tag packs atlas cell and opacity, and the shader fades by surface angle
and projector depth before blending albedo, normal, roughness and emission.
Standard, instanced, box-projected and terrain PBR surfaces receive them.
`decals_deferred.rs` applies the same shader to the G-buffer between prepass
and lighting, using a scratch target to avoid reading an attachment being
written. This also feeds Solari and the traced game lighting. The pool
touches visible projectors, pauses with the run and clears before rebuilds.
Profiler counts are `decals/active`, `decals/budget` and `decals/stolen`.
This path needs native texture binding arrays and reports an error once on
unsupported renderers, including WebGPU. Unlit graph surfaces and the
reference path tracer do not receive decals.

## Lighting rig

`Light` (`LightSpec`) is a point, spot, rect or disk light. Rect and disk are
Bevy `RectLight`s with LTC speculars. `intensity` is lumens or, with `unit:
Candela`, the brightest direction's candela (x 4 pi to lumens). A cookie
(tiled over a spot's beam, or on each face of a point light) and an IES
profile are baked on the CPU into one R8 mask in `lights.rs`
(`spot_mask`/`point_mask`, which invert Bevy's own texture lookups) and
handed over as `SpotLightTexture`/`PointLightTexture`. Masks are cached in
`LightMasks` by file, tiling and cone.

`pbr_patch.rs` edits Bevy's own PBR shaders as they load, for what Bevy has
no switch for. A disk is sent with both extents negative (the side of its
equal-area square) and integrates as a 12-gon of the same area. A shadowed
area light gets a `ShadowTwin` child, a black point light at exactly its
position, whose cube shadow the rect loop looks up in the cluster; black
point lights are skipped in the point loop. The sun's shadows fade over the
last `ShadowSettings::fade` of the distance (a shader constant, so changing
it recompiles). Each patch is exact text against the pinned Bevy; one that
no longer matches reports an error and leaves Bevy's shader alone. Per-light contact shadows,
PCSS and biases ride the same spec; `turn my light's shadows` lands in
`LightsRun::shadows`, which `wanted` lays over the authored spec, and `casts
shadows?` reads `ActorSense::casts_shadows`.

`Lighting::shadows` (`ShadowSettings`) is the sun and camera half, applied by
`shadows.rs`: cascades (count, first split, blend, distance - which `set
shadow distance` overrides as `LightsRun::shadow_distance`), normal bias, a PCSS
sun size, the fade, the camera's `ShadowFilteringMethod` and `ContactShadows`
(16 steps).
`Lighting::sun_cookie` is a tiled `DirectionalLightTexture` whose tile size is
the sun's scale, which is safe because cascades read only its rotation. The
depth bias and everything volumes blend stay in `environment.rs`.

A `Probe` component (`blockloom-core/src/probe.rs`) is a reflection probe (a
box-projected cubemap, `ParallaxCorrection::Auto`) or an irradiance volume (a
brick grid of ambient cubes in Bevy's `(Rx, 2Ry, 3Rz)` atlas). Its box turns
with the actor but isn't scaled by it. `light_probes.rs` bakes through the
capture service: a reflection capture is rotated with the probe, since Bevy
samples a probe's cube in its own frame, while bricks are world-aligned, since
the irradiance lookup uses the world normal. Captures are divided back out of
the camera's exposure into radiance. `BakeProbes` (the inspector's Bake,
`bake-probes`) writes `.blockloom/probes/<actor>.{dds,irr,json}`; the json
holds `probe::stamp`, a hash of the probe and every lit component around it,
so `probe-status` and the scene view know a bake is stale, and an auto-bake
probe rebakes itself after the next rebuild once warm-up closes. A bake
leaves out its own actor (`ProbeRequest::hide`, applied by
`probes::hide_from_captures`), so a probe actor (`ProbeOwner`) is never
batched. Volumes scale probe light through `reflections` (reflection probes
and the sky's light) and `indirect` (irradiance volumes); probes blend with
each other by their own falloff. `capture
probes` does the same capture mid-run and keeps it in memory. Builds copy the
bakes. A reflection bake loads as BC6H where the GPU samples BC, and is
decoded to FP16 elsewhere, as the sky is. `pipeline::bc6h` encodes all 14
modes, and its decoder matches bcdec bit for bit. The GPU half is the ignored `embed` tests (rect and disk lights, area
light shadows, cookie, both probe kinds, a bake leaving out its own actor).
Those tests start their worlds one at a time: concurrent Vulkan instance
creation crashes in the loader.

## Ray-traced lighting

`blockloom-runtime/src/ray_tracing.rs`, 3D only, behind the runtime's default
`ray_tracing` cargo feature (Bevy Solari, pinned with Bevy).
`Lighting::ray_tracing` (`RayTracingSettings`) turns it on; `turn ray
tracing`, `set GI bounces to` and `set GI samples to` (and a script's
`set_ray_tracing`, `set_gi_bounces` and `set_gi_samples`) override it for the
run as
`RayTracingRun`. `RayTracingState` probes the
device once for `SolariPlugins::required_wgpu_features`; without them the
raster rig carries on and the editor hears why once. The atmosphere sample
copies `active`/`available` on the fixed tick, so `is ray tracing on?` and
`ray tracing available?` agree between the VM and compiled logic.

`RayTracingSettings::mode` picks the realtime tracer. `Hybrid` puts
`SolariLighting` on the world camera (ReSTIR direct light plus a world cache
for GI). `PathTraced` puts `traced::TracedPaths` there instead: fresh paths
from every G-buffer pixel each frame (`shaders/traced_paths.wesl`, `paths`
per pixel, light sampling MIS'd against the BRDF), with Bevy's deferred
lighting skipped. Either way the camera also gets `Msaa::Off`, `Hdr` and a
storage-capable main texture, and sun shadow maps turn off. Tracing lights
the G-buffer, so while it is on `DefaultOpaqueRendererMethod` is deferred and
every standard and instanced material is touched to re-prepare; off, both go
back to forward.

`Denoiser` picks the cleanup: ReSTIR's reuse (`reuses`, Hybrid only) and
`traced::TracedDenoiser` (`filters`), an SVGF-style filter
(`shaders/denoise.wesl`) that runs after the opaque pass: demodulate by the
G-buffer albedo, reproject along motion vectors, five a-trous passes steered
by the variance, remodulate. Glossy surfaces keep a shorter history and a
tighter blur. It only touches pixels with a G-buffer (forward surfaces and
the background are left alone) and drops NaNs and clips fireflies first,
since its moments are half floats.

Solari ignores the environment map, so `solari_patch.rs` edits its shaders
as they load (the `pbr_patch` rules): a realtime bounce, a world cache GI ray
past its reach and a reference path tracer bounce that escape all see the
camera's `EnvironmentMapLight`. Under a flat sky `TracedAmbient` hangs a
one-texel cube of the ambient there instead, and a sky that lights but
doesn't reflect shows traced rays its diffuse cube. `instanced_pbr.wesl` is therefore
the instanced material's deferred shader too. Box-projected and graph
surfaces stay forward and keep the raster lights.

Solari only traces `RaytracingMesh3d` + `StandardMaterial` in one vertex
layout, and only the sun and emissive meshes light. So `sync_traced_scene`
(PostUpdate, after propagation) keeps a copy: one proxy per visible drawn
mesh (no `Mesh3d`, `traceable` converting the mesh, a standard material
standing in for instanced, box and graph surfaces) and one emissive stand-in
per `Light` whose `ray_traced` is on (a sphere for a point, a disk down a
spot's beam, the rect or disk itself), glowing with the light's power. A
spot narrower than `HOOD_WIDEST` also gets a black flared hood
(`TracedHood`), so its disk only lights the cone. Merged batches,
placeholders and particles are left out.

The path tracer is `SceneView::path_tracer`, an editor preference: the world
camera gets `Pathtracer` instead, which traces the same copy and starts over
when it or the camera moves (`drive_path_tracer`); progress and availability
reach the editor as `RuntimeMessage::RayTracing` and `state.ray_tracing`. An
EXR capture copies the camera's tracing (`trace_like`) and, under the path
tracer, waits for the sample or time budget. The GPU half is the ignored
`embed` tests (traced vs. untraced lamps, switching mid-run, the path tracer
and its EXR, sky and ambient on escaped rays, realtime path tracing, the
denoiser's grain, a spot's cone). They run on lavapipe too, which has ray
queries but no dma-bufs: `BLOCKLOOM_TEST_OPAQUE_FD=1
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`, and
`BLOCKLOOM_TEST_DUMP=<dir>` saves the frames some of them read as PNGs, and
`BLOCKLOOM_TEST_DEADLINE=<secs>` (default 20) gives a slow software device longer
to settle. `docs/gpu-test-triage.md` lists which fail on lavapipe and why.

## Volumetric clouds

`World.clouds` (`blockloom-core/src/clouds.rs`) persists shape, altitude,
lighting and quality, edited by `set_clouds` and Project Settings. The blended
`Environment.clouds` carries volume coverage, density and type overrides, and
`set clouds [coverage/density/type] to` (a script's `set_clouds`) lays
`space::SkyRun::clouds` over them for the run.
`blockloom-runtime/src/clouds.rs` bakes seeded, repeating 128³ shape and 32³
erosion volumes on the GPU, raymarches at half resolution before fog, and
uses the shared bilateral upsampler over the scene. WindField supplies drift.
`shape_volume`/`detail_volume` swap either bake for an authored volume asset,
read through `pipeline::load_volume`; `clouds::bake_noise` is the CPU twin of
`cloud_bake.wesl` (hash included), which `bake_cloud_noise` writes out as
strips. Change the two together.
Each view keeps two color/depth histories, invalidated on edits, resize,
skipped frames and fast motion. The march writes a raw frame and a second pass
(`cloud_resolve.wesl`) folds it into the history: the history is clipped to the
mean and spread of the pixel's depth-matching 3x3 neighbours, so a jittered
noisy march converges rather than snapping back to its latest sample, and far
clouds also take a spatial blur. Far rays cross slabs thousands of metres long,
so the march gets up to double its steps there and `density` takes a footprint
(the step or the pixel's width at that distance, whichever is wider): past
what it resolves, the shape's edges soften and erosion settles to its mean
(`lost`), rather than aliasing into per-pixel speckle. The ground shadow map
passes a footprint of 0. Sun and moon have separate shadow switches.
The 2048² ground transmittance map is a screen-space approximation over the
combined surface lighting; it does not isolate the PBR direct-light term.
Profiler readback reports primary steps and contributing samples per pixel;
render diagnostics time the march. GPU tests cover sky, terrain preservation
and ground shadows (`cargo test -p blockloom-runtime volumetric_clouds --lib
-- --ignored`); `distant_clouds_neither_crawl_nor_alias` holds the horizon band's
speckle down.

## World-space texturing

`Material::detail` (`SurfaceDetail`, `blockloom-core/src/material.rs`) is the
advanced pass over box projection, shared by materials and terrain (a
terrain's `texturing`): stochastic texture bombing (three hex tiles, randomly
offset and turned, blended by `stochastic_contrast`), macro variation (an
image or noise multiplied over albedo), a detail normal map faded by camera
distance, and a `MaskStack` of slope, height and cavity rule masks plus snow
and wetness. Cavity is baked per terrain sample; other surfaces read none.
The shader half is `blockloom::texturing` (core's `shaders/texturing.wesl`),
which `box_pbr.wesl`, `terrain.wesl` and `grass.wesl` import. Snow and
wetness also follow `World.surface`, volume overrides (`snow`, `wetness`) and
the `set_surface` verb (`environment::SurfaceRun`); `materials::SurfaceGlobals` is one storage buffer of weather
and wind every surface reads, written only on change. `DebugView::SurfaceBlend`
draws layer weights and masks as flat colors.

## Terrain and vegetation

A `Terrain` component (`blockloom-core/src/terrain/`, `TerrainSpec`) is a
heightfield of 2ⁿ+1 samples (129 to 4097) over `size` metres, `height` metres
tall, with up to four paint layers (each with slope, height and curvature
rules), optional holes, grass layers and scatter layers. Grids never live in
the document: `store.rs` keeps them content-addressed under
`.blockloom/terrain` (64² tiles plus a manifest), the spec names the manifest,
and `open_project` prunes what no terrain names. `store::name_of` predicts a
save's name without writing it.

`sculpt.rs` is every edit: a `Brush` (raise, lower, smooth, flatten, noise,
terrace, paint, erase) on a `BrushTarget` (heights, a layer, holes, a grass
or scatter density map), a `Stroke` of stamps applied to an `Editable`, and
`Erosion` (thermal or droplet hydraulic). The editor applies strokes
(`paint_terrain`, or the scene view's `TerrainStroke`), imports heightmaps
through `pipeline::load_heightmap` and erodes, each as one undo step.

`blockloom-runtime/src/terrain/` draws it, 3D only. `Geometry` cuts the
field into chunks (`mesh::ChunkLayout`), each a `LodGroup` of levels picked
by `pixel_error`; skirts hide the cracks between levels. Coarse levels stay
resident and finer ones stream as `CellTasks` payloads on `StreamingCells`.
Builds run off the main thread and are cached by spec (`TerrainCache`), so a
rebuild that didn't touch a terrain reuses it. Collision is one rapier
heightfield with holes removed. The material (`material.rs`) samples baked
layer weights and a normal/cavity surface map at the grid UV, and the
layers' maps triplanar from texture arrays.

`vegetation.rs` streams grass in 32 m cells within its cull distance: seeded
blades (`scatter::grass_blades`) merged into one mesh per cell, bent by the
`WindField` and thinned by distance in `grass.wesl`. Scatter layers place
trees and rocks once per build (`scatter::scatter_instances`: density,
clumping noise, slope, altitude and actor-avoidance filters, tint and scale
jitter); each instance is a holder `LodGroup` with a child per level (LOD0,
LOD1, crossed-quad billboard), procedural shapes or glTF parts drawn
instanced, and an optional trunk collider.

The scene view's Brush tool (`SceneTool::Brush`, `brush.rs`) stamps onto a
working copy and redraws touched chunks, weights and the surface map in
place; on release it sends the stroke and files the predicted result in the
cache under the spec the editor will send back, so the reload doesn't flash.
`EditorMessage::PreviewErosion` shows an erosion filter without saving it.
Triangle, chunk and instance counts reach the profiler as `terrain/*`.

## Cloud layers

`World.cloud_layers` (`blockloom-core/src/cloud_layers.rs`) is up to four
flat layers, 3D only. Coverage is an image's luma or tileable FBM baked from
the layer's seed (`bake_coverage`), always at `COVERAGE_SIZE`, and `remap`
turns it into cloud by coverage and contrast; `cloud_layers.wesl` mirrors
`remap`, so change the two together. `paint-cloud-layer` paints strokes
(`paint`, wrapping at the tile's edges) into `assets/clouds/layer-N.png` and
bumps the layer's `revision`, which is how the runtime knows to reread a file
whose path didn't change. Each stroke keeps a snapshot per revision under
`.blockloom/cloud-paint`, and undo/redo call `restore_painted` to put the
file back. `set cloud layer` lands in `SkyRun::cloud_layers`
(`CloudLayerOverrides`), laid over the project's layers each frame. `blockloom-runtime/src/cloud_layers.rs` loads or
bakes every layer into one texture array when their keys change, and draws
them in `CloudPass`: layers beyond the volumetric slab before `CloudMarch`,
layers between the camera and it after, so the fog pass lays height fog over
all of them. The same shader's `shadow` entry point multiplies the ground by
what the layers let through towards the sun, drawn first. Scroll, flow and
spin run on `CloudOffsets::time`, cloud time.
