//! Shadow tuning, 3D only: the sun's cascades, normal bias, PCSS size and
//! cookie, and the world camera's shadow filter and contact shadows, all
//! from the project's `ShadowSettings` plus `set shadow distance` this run.
//! The fade at the far end is a `pbr_patch` constant.
//!
//! The sun's color, illuminance and depth bias stay with `environment`,
//! which volumes blend; nothing here does.

use crate::engine::Engine;
use crate::pbr_patch::PbrPatches;
use crate::world::{WorldCamera, WorldLight};
use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightTexture, ShadowFilteringMethod};
use bevy::pbr::ContactShadows;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::RenderAdapter;
use blockloom_core::scene::{ShadowFilter, ShadowSettings};
use blockloom_protocol::RuntimeMessage;

/// Steps a contact shadow ray takes: the 16-tap march.
const CONTACT_STEPS: u32 = 16;

/// What the sun and camera were last set up from, so a quiet frame does
/// nothing.
#[derive(Clone, PartialEq)]
pub struct Applied {
    settings: ShadowSettings,
    distance: f32,
    cookie: String,
    cookie_size: f32,
}

#[derive(Default)]
pub struct SunCookie {
    path: String,
    image: Option<Handle<Image>>,
}

#[allow(clippy::type_complexity)]
pub fn apply_shadows(
    mut commands: Commands,
    engine: NonSend<Engine>,
    scaling: Option<Res<crate::quality::Scaling>>,
    mut images: ResMut<Assets<Image>>,
    mut applied: Local<Option<Applied>>,
    patches: Option<ResMut<PbrPatches>>,
    mut cookie: Local<SunCookie>,
    adapter: Option<Res<RenderAdapter>>,
    mut warned_mali: Local<bool>,
    mut suns: Query<(
        Entity,
        Ref<WorldLight>,
        &mut DirectionalLight,
        &mut Transform,
    )>,
    cameras: Query<(Entity, Ref<WorldCamera>), With<Camera3d>>,
) {
    let lighting = &engine.project.world.lighting;
    let settings = lighting.shadows.clone().sanitized();
    let now = Applied {
        distance: engine.shadow_distance.unwrap_or(settings.distance)
            * scaling
                .as_ref()
                .map_or(1.0, |s| s.budget().distance.min(1.0)),
        settings,
        cookie: blockloom_core::assets::normalize(&lighting.sun_cookie).unwrap_or_default(),
        cookie_size: lighting.sun_cookie_size.max(0.01),
    };
    if let Some(mut patches) = patches
        && patches.shadow_fade != now.settings.fade
    {
        patches.shadow_fade = now.settings.fade;
    }
    let fresh = suns.iter().any(|(_, sun, ..)| sun.is_added())
        || cameras.iter().any(|(_, camera)| camera.is_added());
    let changed = applied.as_ref() != Some(&now);

    // The sun's transform is rewritten whenever the environment changes, so
    // its scale - the cookie's tile size - goes back on every frame.
    let tile = if now.cookie.is_empty() {
        1.0
    } else {
        now.cookie_size / 3.0f32.sqrt()
    };
    for (_, _, _, mut transform) in &mut suns {
        if transform.scale != Vec3::splat(tile) {
            transform.scale = Vec3::splat(tile);
        }
    }
    if !changed && !fresh {
        return;
    }

    if cookie.path != now.cookie {
        cookie.path = now.cookie.clone();
        cookie.image = load_cookie(&engine, &now.cookie).map(|image| images.add(image));
    }
    let s = &now.settings;
    let cascades = CascadeShadowConfigBuilder {
        num_cascades: s.cascades as usize,
        minimum_distance: 0.1,
        maximum_distance: now.distance,
        first_cascade_far_bound: s.first_cascade.min(now.distance),
        overlap_proportion: s.cascade_blend,
    }
    .build();
    for (entity, _, mut light, _) in &mut suns {
        light.shadow_normal_bias = s.normal_bias;
        light.contact_shadows_enabled = s.contact;
        // PCSS is off in a browser (see the runtime's Cargo.toml).
        #[cfg(not(target_arch = "wasm32"))]
        {
            light.soft_shadow_size = (s.sun_size > 0.0).then(|| s.sun_size.to_radians());
        }
        let mut sun = commands.entity(entity);
        sun.insert(cascades.clone());
        match &cookie.image {
            Some(image) => {
                sun.insert(DirectionalLightTexture {
                    image: image.clone(),
                    tiled: true,
                });
            }
            None => {
                sun.remove::<DirectionalLightTexture>();
            }
        }
    }
    for (entity, _) in &cameras {
        let mut camera = commands.entity(entity);
        camera.insert(filter_of(s.filter));
        // Contact shadows stay off on Mali: its shader compiler crashes
        // compiling Bevy's screen-space raymarch (scalarizer SEGV), so a
        // Mali game runs without that darkening rather than not at all.
        // Every other GPU keeps the setting. Warned once per run.
        let mali = mali_without_contact(adapter.as_deref());
        if mali && s.contact && !*warned_mali {
            *warned_mali = true;
            tracing::warn!(
                "Contact shadows are off on this Mali GPU: its driver crashes compiling them. The game runs without that darkening."
            );
        }
        if s.contact && !mali {
            camera.insert(ContactShadows {
                linear_steps: CONTACT_STEPS,
                thickness: s.contact_thickness,
                length: s.contact_length,
            });
        } else {
            camera.remove::<ContactShadows>();
        }
    }
    *applied = Some(now);
}

/// Whether contact shadows must stay off: ARM Mali GPUs, whose shader
/// compiler crashes (MaliScalarizer SEGV) compiling the raymarch. Unknown
/// GPUs keep them; only a known-broken one loses the effect.
fn mali_without_contact(adapter: Option<&RenderAdapter>) -> bool {
    const ARM: u32 = 0x13B5;
    adapter.is_some_and(|adapter| adapter.get_info().vendor == ARM)
}

fn filter_of(filter: ShadowFilter) -> ShadowFilteringMethod {
    match filter {
        ShadowFilter::Hardware => ShadowFilteringMethod::Hardware2x2,
        ShadowFilter::Gaussian => ShadowFilteringMethod::Gaussian,
        ShadowFilter::Temporal => ShadowFilteringMethod::Temporal,
    }
}

/// The sun's cookie as a linear one-channel mask, or `None` for none (or one
/// that won't load, which is reported).
fn load_cookie(engine: &Engine, relative: &str) -> Option<Image> {
    if relative.is_empty() {
        return None;
    }
    let dir = engine.project_dir.as_deref()?;
    let loaded = blockloom_core::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))
        .and_then(|path| image::open(path).map_err(|e| format!("{relative}: {e}")));
    match loaded {
        Ok(image) => {
            let luma = image.to_luma8();
            let (width, height) = luma.dimensions();
            let mut image = Image::new(
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                luma.into_raw(),
                TextureFormat::R8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.label = Some("light_mask");
            Some(image)
        }
        Err(message) => {
            crate::bridge::send(&RuntimeMessage::Error {
                actor: "Blockloom".into(),
                message: format!("The sun's cookie didn't load: {message}"),
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::light::CascadeShadowConfig;
    use blockloom_core::scene::Mode;

    fn app() -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let engine = Engine::new(rx, Mode::ThreeD);
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Assets<Image>>()
            .add_systems(Update, apply_shadows);
        app.world_mut().spawn((
            DirectionalLight::default(),
            Transform::default(),
            WorldLight,
        ));
        app.world_mut().spawn((Camera3d::default(), WorldCamera));
        app
    }

    fn sun(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<WorldLight>>()
            .single(app.world())
            .unwrap()
    }

    fn camera(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<WorldCamera>>()
            .single(app.world())
            .unwrap()
    }

    #[test]
    fn the_project_tunes_the_sun_and_the_camera() {
        let mut app = app();
        {
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            let shadows = &mut engine.project.world.lighting.shadows;
            shadows.filter = ShadowFilter::Temporal;
            shadows.cascades = 2;
            shadows.distance = 60.0;
            shadows.normal_bias = 0.5;
            shadows.sun_size = 0.5;
            shadows.contact = true;
            shadows.fade = 0.25;
        }
        app.init_resource::<PbrPatches>();
        app.update();
        assert_eq!(app.world().resource::<PbrPatches>().shadow_fade, 0.25);
        let sun = sun(&mut app);
        let light = app.world().get::<DirectionalLight>(sun).unwrap();
        assert_eq!(light.shadow_normal_bias, 0.5);
        assert!(light.contact_shadows_enabled);
        assert!(light.soft_shadow_size.is_some());
        let cascades = app.world().get::<CascadeShadowConfig>(sun).unwrap();
        assert_eq!(cascades.bounds.len(), 2);
        assert_eq!(*cascades.bounds.last().unwrap(), 60.0);

        let camera = camera(&mut app);
        assert_eq!(
            app.world().get::<ShadowFilteringMethod>(camera),
            Some(&ShadowFilteringMethod::Temporal)
        );
        assert_eq!(
            app.world()
                .get::<ContactShadows>(camera)
                .unwrap()
                .linear_steps,
            16
        );
    }

    #[test]
    fn a_block_shortens_the_shadows_for_the_run() {
        let mut app = app();
        app.update();
        app.world_mut().non_send_mut::<Engine>().shadow_distance = Some(25.0);
        app.update();
        let sun = sun(&mut app);
        let cascades = app.world().get::<CascadeShadowConfig>(sun).unwrap();
        assert_eq!(*cascades.bounds.last().unwrap(), 25.0);
        // No contact shadows unless the project asks.
        let camera = camera(&mut app);
        assert!(app.world().get::<ContactShadows>(camera).is_none());
    }

    #[test]
    fn contact_shadows_stay_on_when_the_gpu_is_unknown() {
        // Fail open: without adapter info (headless tests, unrecognized
        // GPUs) the setting stands. Only a known Mali loses the effect.
        assert!(!mali_without_contact(None));
    }
}
