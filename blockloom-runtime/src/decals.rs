//! Pooled projectors over Bevy's clustered material path, forward and deferred.

use bevy::asset::RenderAssetUsages;
use bevy::light::ClusteredDecal;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::{RenderAdapter, RenderDevice};
use bevy::render::settings::WgpuFeatures;
use blockloom_core::decals::{ATLAS_CELL, Pool, atlas_pages};
use blockloom_core::scene::Mode;
use blockloom_core::vm::Effect;

use crate::engine::{Dimension, Engine, PendingEffects};

pub fn register(app: &mut App) {
    app.init_resource::<Decals>()
        .add_systems(
            FixedUpdate,
            apply_effects
                .in_set(crate::world::SimulationSet)
                .after(crate::world::apply_common)
                .before(crate::world::clear_effects),
        )
        .add_systems(
            Update,
            clear
                .after(crate::world::pump_editor)
                .before(crate::world::rebuild_world),
        );
    if app.world().resource::<Dimension>().0 == Mode::ThreeD {
        app.add_systems(Update, draw.after(crate::world::rebuild_world));
    }
}

#[derive(Resource, Default)]
pub struct Decals {
    pub pool: Pool,
    atlas: Option<[Handle<Image>; 4]>,
    warned: bool,
}

#[derive(Component)]
struct Projector(u64);

fn clear(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut decals: ResMut<Decals>,
    projectors: Query<Entity, With<Projector>>,
) {
    if engine.rebuild || !engine.running {
        decals.pool = Pool::default();
        for entity in &projectors {
            commands.entity(entity).despawn();
        }
    }
}

fn apply_effects(
    engine: NonSend<Engine>,
    time: Res<Time<Fixed>>,
    effects: Res<PendingEffects>,
    mut decals: ResMut<Decals>,
) {
    if !engine.running || engine.paused {
        return;
    }
    decals.pool.step(time.delta_secs());
    for effect in &effects.0 {
        match effect {
            Effect::SpawnDecal(spawn) => decals.pool.spawn(spawn.clone()),
            Effect::FadeDecals {
                at,
                radius,
                seconds,
            } => decals.pool.fade_in_radius(*at, *radius, *seconds),
            Effect::Stopped => decals.pool = Pool::default(),
            _ => {}
        }
    }
}

fn tag(mark: &blockloom_core::decals::Mark) -> u32 {
    0x8000_0000 | mark.spawn.preset.cell() | ((mark.opacity() * 65535.0).round() as u32) << 8
}

fn draw(
    mut commands: Commands,
    mut decals: ResMut<Decals>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
    adapter: Option<Res<RenderAdapter>>,
    mut projectors: Query<(Entity, &Projector, &mut ClusteredDecal, &ViewVisibility)>,
) {
    let mut present = std::collections::HashSet::new();
    for (entity, projector, mut decal, visible) in &mut projectors {
        if visible.get() {
            decals.pool.touch(projector.0);
        }
        if let Some(mark) = decals.pool.marks.iter().find(|m| m.id == projector.0) {
            let next = tag(mark);
            if decal.tag != next {
                decal.tag = next;
            }
            present.insert(mark.id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    if decals.pool.marks.is_empty() {
        return;
    }
    if !supported(device.as_deref(), adapter.as_deref()) {
        if !decals.warned {
            decals.warned = true;
            crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
                actor: "Blockloom".into(),
                message: "Projected decals need native texture binding arrays; this renderer cannot draw them.".into(),
            });
        }
        return;
    }
    let atlas = decals
        .atlas
        .get_or_insert_with(|| {
            let mut index = 0;
            atlas_pages().map(|data| {
                let format = if index == 0 {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                };
                index += 1;
                images.add(Image::new(
                    Extent3d {
                        width: ATLAS_CELL * 2,
                        height: ATLAS_CELL * 2,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    data,
                    format,
                    RenderAssetUsages::RENDER_WORLD,
                ))
            })
        })
        .clone();
    for mark in &decals.pool.marks {
        if present.contains(&mark.id) {
            continue;
        }
        let spawn = &mark.spawn;
        commands.spawn((
            Projector(mark.id),
            ClusteredDecal {
                base_color_texture: Some(atlas[0].clone()),
                normal_map_texture: Some(atlas[1].clone()),
                metallic_roughness_texture: Some(atlas[2].clone()),
                emissive_texture: Some(atlas[3].clone()),
                tag: tag(mark),
            },
            Transform {
                translation: Vec3::from(spawn.at),
                rotation: Quat::from_rotation_arc(Vec3::Z, Vec3::from(spawn.normal)),
                scale: Vec3::new(
                    spawn.size,
                    spawn.size,
                    (spawn.size * 0.15).clamp(0.02, 0.25),
                ),
            },
        ));
    }
}

fn supported(device: Option<&RenderDevice>, adapter: Option<&RenderAdapter>) -> bool {
    let (Some(device), Some(adapter)) = (device, adapter) else {
        return false;
    };
    let info = bevy::render::renderer::RenderAdapterInfo::new(adapter.get_info());
    // Match the pinned Bevy's clustered texture-array requirements.
    bevy::render::get_adreno_model(&info).is_none_or(|model| model > 610)
        && device.limits().max_binding_array_elements_per_shader_stage >= 24
        && device
            .limits()
            .max_binding_array_sampler_elements_per_shader_stage
            >= 24
        && device.features().contains(
            WgpuFeatures::TEXTURE_BINDING_ARRAY
                | WgpuFeatures::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        )
}

pub fn patch_shader(
    source: &str,
    _: &crate::pbr_patch::PbrPatches,
) -> Result<String, &'static str> {
    let source = crate::pbr_patch::replace_once(
        source,
        "\n    while (clustered_decal_iterator_next(&iterator)) {",
        "\n    while (clustered_decal_iterator_next(&iterator)) {\n        if ((iterator.tag & 0x80000000u) != 0u) {\n            blockloom_decal(iterator, world_normal, &base_color, &Nt, &perceptual_roughness, &emissive);\n            continue;\n        }",
        "the decal loop",
    )?;
    Ok(source + include_str!("shaders/decals.wesl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::decals::{DecalPreset, Spawn};

    #[test]
    fn the_shader_patch_skips_the_documented_iterator_example() {
        let source = "//!      while (clustered_decal_iterator_next(&iterator)) {\nfn apply_decals() {\n    while (clustered_decal_iterator_next(&iterator)) {\n    }\n}";
        let patched = patch_shader(source, &crate::pbr_patch::PbrPatches::default()).unwrap();
        assert!(patched.starts_with(
            "//!      while (clustered_decal_iterator_next(&iterator)) {\nfn apply_decals()"
        ));
        assert_eq!(patched.matches("blockloom_decal(iterator,").count(), 1);
    }

    #[test]
    fn effects_pause_and_rebuild_clear_the_pool() {
        let (_, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.running = true;
        engine.rebuild = false;
        app.insert_non_send(engine)
            .init_resource::<Decals>()
            .init_resource::<Time<Fixed>>()
            .init_resource::<PendingEffects>()
            .add_systems(Update, (apply_effects, clear).chain());
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::SpawnDecal(Spawn {
                preset: DecalPreset::Blood,
                at: [0.0; 3],
                normal: [0.0, 1.0, 0.0],
                size: 1.0,
                lifetime: 3.0,
                fade: 1.0,
            }));
        app.update();
        assert_eq!(app.world().resource::<Decals>().pool.marks.len(), 1);
        app.world_mut().non_send_mut::<Engine>().paused = true;
        app.update();
        assert_eq!(app.world().resource::<Decals>().pool.marks.len(), 1);
        app.world_mut().non_send_mut::<Engine>().rebuild = true;
        app.update();
        assert!(app.world().resource::<Decals>().pool.marks.is_empty());
    }
}
