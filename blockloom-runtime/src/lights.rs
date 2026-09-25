//! Actor lights: an actor's `Light` component as a Bevy point, spot or area
//! light, in lumens (or candela) with a range in metres. 3D only.
//!
//! The light rides a child entity rather than the actor itself, since
//! batching hides a merged actor through an empty `RenderLayers`, which
//! would switch its light off too.
//!
//! A cookie and an IES profile both end up as one mask texture the light
//! shines through: baked on the CPU when the light changes, a square over
//! the cone for a spot and a six-face strip for a point.
//!
//! Area lights lean on `pbr_patch`: a disk goes out with negative extents,
//! and a shadowed one gets a black point light twin whose cube shadow the
//! patched shader reads.

use crate::engine::{ActorId, Engine, PendingEffects};
use crate::world::parse_color;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::CubemapLayout;
use bevy::light::{PointLightTexture, RectLight, SpotLightTexture};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use blockloom_core::components::{LightKind, LightSpec};
use blockloom_core::pipeline::{self, ies::IesProfile};
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What an actor's light child was built from, so a quiet frame changes
/// nothing.
#[derive(Component)]
pub struct Lit {
    pub spec: LightSpec,
    child: Entity,
}

/// The child entity carrying an actor's light.
#[derive(Component)]
pub struct ActorLight;

/// Under a shadowed area light: the black point light casting its shadow.
#[derive(Component)]
pub struct ShadowTwin;

/// Baked masks by what they were baked from, so actors sharing a fixture
/// share a texture and a rebuild doesn't bake again.
#[derive(Resource, Default)]
pub struct LightMasks(HashMap<MaskKey, Handle<Image>>);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MaskKey {
    dir: PathBuf,
    point: bool,
    cookie: String,
    /// Tiling and cone as bits, so the key can hash.
    tiling: u32,
    cone: u32,
    ies: String,
}

/// Texels per side of a spot mask, and per face of a point one.
const MASK_SIZE: u32 = 256;
const POINT_MASK_SIZE: u32 = 64;

/// `set my light to`, `turn my light's shadows`, `set shadow distance` and
/// `capture probes` for the rest of the run. Cleared with everything else
/// live on a rebuild.
pub fn apply_light_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetLightIntensity { actor, intensity } => {
                engine
                    .light_intensity
                    .insert(actor.clone(), intensity.max(0.0));
            }
            Effect::SetLightShadows { actor, enabled } => {
                engine.light_shadows.insert(actor.clone(), *enabled);
            }
            Effect::SetShadowDistance { distance } if distance.is_finite() => {
                engine.shadow_distance = Some(distance.clamp(1.0, 10_000.0));
            }
            Effect::CaptureProbes => engine.capture_probes = true,
            _ => {}
        }
    }
}

/// The light each actor should be carrying right now, from what it holds
/// (`engine.attached`, so attach and detach work mid-run), what the editor
/// authored and what the blocks have set it to since.
fn wanted(engine: &Engine, actor: &str) -> Option<LightSpec> {
    if !engine.has_component(actor, "Light") {
        return None;
    }
    let mut spec = engine.actor(actor)?.components.light()?.clone();
    if let Some(intensity) = engine.light_intensity.get(actor) {
        // The block speaks the light's own unit.
        spec.intensity = *intensity;
    }
    if let Some(shadows) = engine.light_shadows.get(actor) {
        spec.shadows = *shadows;
    }
    Some(spec)
}

/// Whether an actor's light casts shadow maps right now. What `casts
/// shadows?` reads; a 2D world has no lights.
pub fn casts_shadows(engine: &Engine, actor: &str) -> bool {
    engine.project.world.mode.is_3d() && wanted(engine, actor).is_some_and(|spec| spec.shadows)
}

/// Builds, rebuilds or takes away each actor's light child to match.
pub fn sync_lights(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut masks: ResMut<LightMasks>,
    mut images: ResMut<Assets<Image>>,
    actors: Query<(Entity, &ActorId, Option<&Lit>)>,
) {
    for (entity, id, lit) in &actors {
        let spec = wanted(&engine, &id.0);
        if spec.as_ref() == lit.map(|lit| &lit.spec) {
            continue;
        }
        if let Some(lit) = lit {
            commands.entity(lit.child).despawn();
            commands.entity(entity).remove::<Lit>();
        }
        let Some(spec) = spec else {
            continue;
        };
        let mask = engine
            .project_dir
            .as_deref()
            .and_then(|dir| mask_for(dir, &spec, &mut masks, &mut images, &id.0));
        let mut child = commands.spawn((ActorLight, Transform::default(), ChildOf(entity)));
        insert_light(&mut child, &spec, mask);
        let child = child.id();
        commands.entity(entity).insert(Lit { spec, child });
    }
}

fn insert_light(entity: &mut EntityCommands, spec: &LightSpec, mask: Option<Handle<Image>>) {
    let color = parse_color(&spec.color);
    let intensity = spec.lumens();
    let range = spec.range.max(0.01);
    let radius = spec.radius.max(0.0);
    match spec.kind {
        LightKind::Point => {
            entity.insert(PointLight {
                color,
                intensity,
                range,
                radius,
                shadow_maps_enabled: spec.shadows,
                contact_shadows_enabled: spec.contact_shadows,
                soft_shadows_enabled: spec.soft_shadows,
                shadow_depth_bias: spec
                    .shadow_depth_bias
                    .unwrap_or(PointLight::DEFAULT_SHADOW_DEPTH_BIAS),
                shadow_normal_bias: spec
                    .shadow_normal_bias
                    .unwrap_or(PointLight::DEFAULT_SHADOW_NORMAL_BIAS),
                ..default()
            });
            if let Some(image) = mask {
                entity.insert(PointLightTexture {
                    image,
                    cubemap_layout: CubemapLayout::SequenceVertical,
                });
            }
        }
        LightKind::Spot => {
            let (inner_angle, outer_angle) = spec.cone();
            entity.insert(SpotLight {
                color,
                intensity,
                range,
                radius,
                shadow_maps_enabled: spec.shadows,
                contact_shadows_enabled: spec.contact_shadows,
                soft_shadows_enabled: spec.soft_shadows,
                shadow_depth_bias: spec
                    .shadow_depth_bias
                    .unwrap_or(SpotLight::DEFAULT_SHADOW_DEPTH_BIAS),
                shadow_normal_bias: spec
                    .shadow_normal_bias
                    .unwrap_or(SpotLight::DEFAULT_SHADOW_NORMAL_BIAS),
                inner_angle,
                outer_angle,
                ..default()
            });
            if let Some(image) = mask {
                entity.insert(SpotLightTexture { image });
            }
        }
        LightKind::Rect | LightKind::Disk => {
            let (width, height) = spec.area_size();
            // Both extents negative marks a disk for the patched shader;
            // Bevy's own draws the same square either way round.
            let sign = if spec.kind == LightKind::Disk {
                -1.0
            } else {
                1.0
            };
            entity.insert(RectLight {
                color,
                intensity,
                range,
                width: width * sign,
                height: height * sign,
            });
            if spec.shadows {
                // The shadow twin: black, so it lights nothing, at exactly
                // the light's position, which is how the shader finds it. Its
                // own entity, since Bevy clusters one light per entity.
                let parent = entity.id();
                entity.commands().spawn((
                    ShadowTwin,
                    Transform::default(),
                    ChildOf(parent),
                    PointLight {
                        color: Color::BLACK,
                        intensity: 0.0,
                        range,
                        radius: 0.5 * width.max(height),
                        shadow_maps_enabled: true,
                        soft_shadows_enabled: spec.soft_shadows,
                        shadow_depth_bias: spec
                            .shadow_depth_bias
                            .unwrap_or(PointLight::DEFAULT_SHADOW_DEPTH_BIAS),
                        shadow_normal_bias: spec
                            .shadow_normal_bias
                            .unwrap_or(PointLight::DEFAULT_SHADOW_NORMAL_BIAS),
                        ..default()
                    },
                ));
            }
        }
    }
}

/// The light's mask, baked and cached, or `None` when it has neither a
/// cookie nor a profile (or is an area light, which takes neither). A file
/// that won't load is reported and the light shines unmasked.
fn mask_for(
    dir: &Path,
    spec: &LightSpec,
    masks: &mut LightMasks,
    images: &mut Assets<Image>,
    actor: &str,
) -> Option<Handle<Image>> {
    let point = match spec.kind {
        LightKind::Point => true,
        LightKind::Spot => false,
        LightKind::Rect | LightKind::Disk => return None,
    };
    let cookie = blockloom_core::assets::normalize(&spec.cookie).unwrap_or_default();
    let ies = blockloom_core::assets::normalize(&spec.ies).unwrap_or_default();
    if cookie.is_empty() && ies.is_empty() {
        return None;
    }
    let key = MaskKey {
        dir: dir.to_path_buf(),
        point,
        cookie,
        tiling: spec.cookie_tiling.to_bits(),
        cone: if point { 0 } else { spec.cone().1.to_bits() },
        ies,
    };
    if let Some(handle) = masks.0.get(&key) {
        return Some(handle.clone());
    }
    match bake_mask(&key, spec) {
        Ok(image) => {
            let handle = images.add(image);
            masks.0.insert(key, handle.clone());
            Some(handle)
        }
        Err(message) => {
            crate::bridge::send(&RuntimeMessage::Error {
                actor: actor.to_string(),
                message: format!("The light's mask didn't load: {message}"),
            });
            None
        }
    }
}

fn bake_mask(key: &MaskKey, spec: &LightSpec) -> Result<Image, String> {
    let profile = if key.ies.is_empty() {
        None
    } else {
        Some(pipeline::load_ies(&key.dir, &key.ies)?)
    };
    let cookie = if key.cookie.is_empty() {
        None
    } else {
        let path = blockloom_core::assets::resolve(&key.dir, &key.cookie)
            .ok_or_else(|| format!("\"{}\" isn't a path in this project", key.cookie))?;
        let image = image::open(&path).map_err(|e| format!("{}: {e}", key.cookie))?;
        Some(image.to_luma8())
    };
    if key.point {
        let data = point_mask(
            profile.as_ref(),
            cookie.as_ref(),
            spec.cookie_tiling,
            POINT_MASK_SIZE,
        );
        return Ok(mask_image(POINT_MASK_SIZE, POINT_MASK_SIZE * 6, data));
    }
    let data = spot_mask(
        profile.as_ref(),
        cookie.as_ref(),
        spec.cookie_tiling,
        spec.cone().1.tan(),
        MASK_SIZE,
    );
    Ok(mask_image(MASK_SIZE, MASK_SIZE, data))
}

/// A linear one-channel mask; only red is read.
fn mask_image(width: u32, height: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.label = Some("light_mask");
    image
}

/// A spot's mask over its cone's bounding square, the way Bevy reads a spot
/// texture: `uv = (local.xy / (local.z * tan(outer))) * (-0.5, 0.5) + 0.5`,
/// looking down local -Z. The profile's vertical angle runs from the beam
/// axis; the cookie repeats `tiling` times across the square.
fn spot_mask(
    profile: Option<&IesProfile>,
    cookie: Option<&image::GrayImage>,
    tiling: f32,
    tan_outer: f32,
    size: u32,
) -> Vec<u8> {
    let peak = profile.map(|p| p.max_candela().max(f32::EPSILON));
    let tiling = clamp_tiling(tiling);
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let v = (y as f32 + 0.5) / size as f32;
            let mut value = 1.0;
            if let (Some(profile), Some(peak)) = (profile, peak) {
                // Invert the uv mapping at local z = -1.
                let lx = -(0.5 - u) * 2.0 * tan_outer;
                let ly = -(v - 0.5) * 2.0 * tan_outer;
                let dir = Vec3::new(lx, ly, -1.0).normalize();
                let vertical = dir.dot(Vec3::NEG_Z).clamp(-1.0, 1.0).acos().to_degrees();
                let horizontal = dir.y.atan2(dir.x).to_degrees();
                value *= profile.sample(vertical, horizontal) / peak;
            }
            if let Some(cookie) = cookie {
                value *= sample_tiled(cookie, u * tiling, v * tiling);
            }
            out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

fn clamp_tiling(tiling: f32) -> f32 {
    if tiling.is_finite() {
        tiling.clamp(0.01, 64.0)
    } else {
        1.0
    }
}

/// A point's mask as six faces stacked in Bevy's face order (+X, -X, +Y,
/// -Y, -Z, +Z), each texel the direction Bevy's `cubemap_uv` maps there. The
/// profile's vertical angle runs from straight down, local -Y; the cookie
/// repeats `tiling` times across every face, like a lantern's panes.
fn point_mask(
    profile: Option<&IesProfile>,
    cookie: Option<&image::GrayImage>,
    tiling: f32,
    size: u32,
) -> Vec<u8> {
    let peak = profile.map(|p| p.max_candela().max(f32::EPSILON));
    let tiling = clamp_tiling(tiling);
    let mut out = Vec::with_capacity((size * size * 6) as usize);
    for face in 0..6 {
        for y in 0..size {
            for x in 0..size {
                let a = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
                let b = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
                let dir = point_face_direction(face, a, b).normalize();
                let mut value = match (profile, peak) {
                    (Some(profile), Some(peak)) => {
                        let vertical = (-dir.y).clamp(-1.0, 1.0).acos().to_degrees();
                        let horizontal = dir.z.atan2(dir.x).to_degrees();
                        profile.sample(vertical, horizontal) / peak
                    }
                    _ => 1.0,
                };
                if let Some(cookie) = cookie {
                    let u = (x as f32 + 0.5) / size as f32;
                    let v = (y as f32 + 0.5) / size as f32;
                    value *= sample_tiled(cookie, u * tiling, v * tiling);
                }
                out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    out
}

/// Undoes `cubemap_uv`: face `face`, face coordinates `a`, `b` in -1..1.
fn point_face_direction(face: u32, a: f32, b: f32) -> Vec3 {
    match face {
        0 => Vec3::new(1.0, -b, a),
        1 => Vec3::new(-1.0, -b, -a),
        2 => Vec3::new(a, 1.0, -b),
        3 => Vec3::new(a, -1.0, b),
        4 => Vec3::new(a, -b, -1.0),
        _ => Vec3::new(a, b, 1.0),
    }
}

fn sample_tiled(image: &image::GrayImage, u: f32, v: f32) -> f32 {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return 1.0;
    }
    let x = (u.rem_euclid(1.0) * w as f32) as u32;
    let y = (v.rem_euclid(1.0) * h as f32) as u32;
    image.get_pixel(x.min(w - 1), y.min(h - 1)).0[0] as f32 / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::{ActorComponent, LightUnit};
    use blockloom_core::pipeline::ies::parse_ies;
    use blockloom_core::scene::Mode;

    fn app_with_lamp(spec: LightSpec) -> (App, Entity) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        let lamp = &mut engine.project.actors[0];
        lamp.components
            .insert(ActorComponent::Light { light: spec });
        let id = lamp.id.clone();
        engine
            .attached
            .entry(id.clone())
            .or_default()
            .insert("Light".to_string());
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<PendingEffects>()
            .init_resource::<LightMasks>()
            .init_resource::<Assets<Image>>()
            .add_systems(Update, (apply_light_effects, sync_lights).chain());
        let actor = app.world_mut().spawn(ActorId(id)).id();
        (app, actor)
    }

    fn lamp(kind: LightKind) -> LightSpec {
        LightSpec {
            kind,
            ..LightSpec::default()
        }
    }

    fn light_child(app: &mut App) -> Option<Entity> {
        app.world_mut()
            .query_filtered::<Entity, With<ActorLight>>()
            .iter(app.world())
            .next()
    }

    fn push(app: &mut App, effect: Effect) {
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(effect);
    }

    #[test]
    fn a_light_component_hangs_a_light_off_its_actor() {
        let (mut app, actor) = app_with_lamp(lamp(LightKind::Spot));
        app.update();
        let child = light_child(&mut app).expect("a light child");
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), actor);
        assert_eq!(
            app.world().get::<SpotLight>(child).unwrap().intensity,
            800.0
        );

        // A quiet frame leaves the same child in place.
        app.update();
        assert_eq!(light_child(&mut app), Some(child));
    }

    #[test]
    fn a_block_brightens_the_light_and_detaching_takes_it_away() {
        let (mut app, actor) = app_with_lamp(lamp(LightKind::Point));
        let id = app.world().get::<ActorId>(actor).unwrap().0.clone();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.update();
        push(
            &mut app,
            Effect::SetLightIntensity {
                actor: id.clone(),
                intensity: 2400.0,
            },
        );
        app.update();
        let child = light_child(&mut app).unwrap();
        assert_eq!(
            app.world().get::<PointLight>(child).unwrap().intensity,
            2400.0
        );

        app.world_mut()
            .non_send_mut::<Engine>()
            .attached
            .get_mut(&id)
            .unwrap()
            .remove("Light");
        app.update();
        assert!(light_child(&mut app).is_none());
        assert!(app.world().get::<Lit>(actor).is_none());
    }

    #[test]
    fn candela_turn_into_lumens_over_the_sphere() {
        let (mut app, _) = app_with_lamp(LightSpec {
            unit: LightUnit::Candela,
            intensity: 100.0,
            ..lamp(LightKind::Point)
        });
        app.update();
        let child = light_child(&mut app).unwrap();
        let lumens = app.world().get::<PointLight>(child).unwrap().intensity;
        assert!((lumens - 400.0 * std::f32::consts::PI).abs() < 1e-2);
    }

    #[test]
    fn rect_and_disk_lights_are_area_lights() {
        let (mut app, _) = app_with_lamp(LightSpec {
            width: 2.0,
            height: 0.5,
            shadows: true,
            ..lamp(LightKind::Rect)
        });
        app.update();
        let child = light_child(&mut app).unwrap();
        let rect = app.world().get::<RectLight>(child).unwrap();
        assert_eq!((rect.width, rect.height), (2.0, 0.5));
        // A shadowed area light casts through a black twin at its centre.
        let twin = app.world().get::<Children>(child).unwrap()[0];
        assert!(app.world().get::<ShadowTwin>(twin).is_some());
        let twin = app.world().get::<PointLight>(twin).unwrap();
        assert!(twin.shadow_maps_enabled);
        assert_eq!(twin.intensity, 0.0);
        assert_eq!(twin.range, rect.range);
        let engine = app.world().non_send::<Engine>();
        let id = engine.project.actors[0].id.clone();
        assert!(casts_shadows(engine, &id));

        let (mut app, _) = app_with_lamp(LightSpec {
            width: 2.0,
            ..lamp(LightKind::Disk)
        });
        app.update();
        let child = light_child(&mut app).unwrap();
        let disk = app.world().get::<RectLight>(child).unwrap();
        // Negative sides mark a disk, the square of its area.
        assert!(disk.width < 0.0 && disk.height < 0.0);
        assert!((disk.width * disk.height - std::f32::consts::PI).abs() < 1e-4);
        // No shadows, no twin.
        assert!(app.world().get::<Children>(child).is_none());
    }

    #[test]
    fn a_block_turns_the_shadows_off_and_casts_shadows_follows() {
        let (mut app, actor) = app_with_lamp(LightSpec {
            shadows: true,
            ..lamp(LightKind::Spot)
        });
        let id = app.world().get::<ActorId>(actor).unwrap().0.clone();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.update();
        assert!(casts_shadows(app.world().non_send::<Engine>(), &id));
        push(
            &mut app,
            Effect::SetLightShadows {
                actor: id.clone(),
                enabled: false,
            },
        );
        push(&mut app, Effect::SetShadowDistance { distance: 40.0 });
        push(&mut app, Effect::CaptureProbes);
        app.update();
        let child = light_child(&mut app).unwrap();
        assert!(
            !app.world()
                .get::<SpotLight>(child)
                .unwrap()
                .shadow_maps_enabled
        );
        let engine = app.world().non_send::<Engine>();
        assert!(!casts_shadows(engine, &id));
        assert_eq!(engine.shadow_distance, Some(40.0));
        assert!(engine.capture_probes);
    }

    /// A downlight: full straight down, half at 45 degrees, none sideways.
    const DOWNLIGHT: &str = "TILT=NONE\n1 1000 1 3 1 1 2 0.1 0.1 0\n1 1 10\n0 45 90\n0\n100 50 0\n";

    #[test]
    fn a_spot_mask_follows_the_profile_from_the_beam_axis() {
        let profile = parse_ies("down.ies", DOWNLIGHT).unwrap();
        // A 90 degree half-angle cone: the square's edge midpoints sit at 45.
        let mask = spot_mask(Some(&profile), None, 1.0, 1.0, 64);
        let at = |x: u32, y: u32| mask[(y * 64 + x) as usize];
        assert!(at(32, 32) > 250, "axis {}", at(32, 32));
        let edge = at(0, 32);
        assert!((120..=135).contains(&edge), "45 degrees {edge}");
    }

    #[test]
    fn a_cookie_tiles_across_the_beam() {
        // Left half dark, right half lit.
        let cookie =
            image::GrayImage::from_fn(2, 1, |x, _| image::Luma([if x == 0 { 0 } else { 255 }]));
        let mask = spot_mask(None, Some(&cookie), 2.0, 1.0, 8);
        let row: Vec<u8> = mask[..8].to_vec();
        assert_eq!(row, vec![0, 0, 255, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn a_point_mask_points_its_profile_down() {
        let profile = parse_ies("down.ies", DOWNLIGHT).unwrap();
        let size = 16;
        let mask = point_mask(Some(&profile), None, 1.0, size);
        let centre = |face: u32| mask[((face * size + size / 2) * size + size / 2) as usize];
        // -Y is face 3: straight down, full. +Y is dark.
        assert!(centre(3) > 230);
        assert_eq!(centre(2), 0);
        // Sideways faces look at 90 degrees: dark.
        assert!(centre(0) < 15);
        for face in 0..6 {
            let dir = point_face_direction(face, 0.0, 0.0);
            assert_eq!(dir.abs().max_element(), 1.0);
        }
    }

    #[test]
    fn a_point_light_repeats_its_cookie_on_every_face() {
        let cookie =
            image::GrayImage::from_fn(2, 1, |x, _| image::Luma([if x == 0 { 0 } else { 255 }]));
        let size = 4;
        let mask = point_mask(None, Some(&cookie), 2.0, size);
        for face in 0..6 {
            let row = &mask[(face * size * size) as usize..][..size as usize];
            assert_eq!(row, [0, 255, 0, 255], "face {face}");
        }
    }
}
