//! Model looks. A `Visual::Model` actor draws its glTF file's first scene as
//! a child entity, scaled by the look's `scale`, and loops one of the file's
//! animations on the rig. The authored box stands in while the file loads and
//! stays when it won't, and it is always what the actor collides as.

use crate::engine::Engine;
use crate::materials::GraphMaterial3d;
use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use blockloom_core::scene::Visual;
use blockloom_protocol::RuntimeMessage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The child entity a model's scene spawns under, and what it came from.
#[derive(Component)]
pub struct ModelScene {
    actor: Entity,
    id: String,
    source: String,
    gltf: Handle<Gltf>,
    animation: String,
    reported: bool,
}

/// On the actor: the child its model scene hangs off.
#[derive(Component)]
pub struct ModelChild(pub Entity);

/// Keeps every model the world has drawn loaded, so the rebuild after each
/// edit reuses the file instead of reading it again with a box in between.
/// A file whose modified time moved is reloaded.
#[derive(Resource, Default)]
pub struct ModelCache(HashMap<PathBuf, (SystemTime, Handle<Gltf>, Handle<WorldAsset>)>);

/// Whether the runtime can draw `path` as a scene. OBJ and FBX have no Bevy
/// loader, so they keep the box.
pub fn is_gltf(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".gltf") || lower.ends_with(".glb")
}

/// Start drawing `visual`'s file under `entity`. Answers whether the scene is
/// already loaded, in which case the box never needs to show.
pub fn attach(
    commands: &mut Commands,
    entity: Entity,
    actor: &str,
    visual: &Visual,
    dir: Option<&Path>,
    assets: &AssetServer,
) -> bool {
    let Visual::Model {
        path,
        scale,
        animation,
        ..
    } = visual
    else {
        return false;
    };
    let source = path.trim();
    if source.is_empty() {
        return false;
    }
    if !is_gltf(source) {
        tracing::warn!("{source}: only glTF and GLB models draw in the game; it shows as its box");
        return false;
    }
    let file = crate::world::asset_path(dir, source);
    let gltf: Handle<Gltf> = assets.load(file.clone());
    let scene: Handle<WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(file.clone()));
    let loaded = assets.is_loaded_with_dependencies(&scene);
    keep_loaded(commands, file, gltf.clone(), scene.clone());
    let child = commands
        .spawn((
            WorldAssetRoot(scene),
            Transform::from_scale(Vec3::from(*scale)),
            ModelScene {
                actor: entity,
                id: actor.to_string(),
                source: source.to_string(),
                gltf,
                animation: animation.trim().to_string(),
                reported: false,
            },
        ))
        .observe(on_ready)
        .id();
    commands
        .entity(entity)
        .add_child(child)
        .insert(ModelChild(child));
    if loaded {
        commands.entity(entity).remove::<Mesh3d>();
    }
    loaded
}

/// Take the model off an actor whose look is going away.
pub fn detach(commands: &mut Commands, entity: Entity, children: &Query<&ModelChild>) {
    if let Ok(child) = children.get(entity) {
        commands.entity(child.0).despawn();
        commands.entity(entity).remove::<ModelChild>();
    }
}

fn keep_loaded(
    commands: &mut Commands,
    file: PathBuf,
    gltf: Handle<Gltf>,
    scene: Handle<WorldAsset>,
) {
    commands.queue(move |world: &mut World| {
        let modified = std::fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let Some(server) = world.get_resource::<AssetServer>().cloned() else {
            return;
        };
        let Some(mut cache) = world.get_resource_mut::<ModelCache>() else {
            return;
        };
        let stale = cache
            .0
            .get(&file)
            .is_some_and(|(seen, ..)| *seen != modified);
        cache.0.insert(file.clone(), (modified, gltf, scene));
        if stale {
            server.reload(file);
        }
    });
}

/// The scene is in the world: drop the stand-in box, drop any camera the
/// file brought (the world has one), and start the rig.
#[allow(clippy::too_many_arguments)]
fn on_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    scenes: Query<&ModelScene>,
    children: Query<&Children>,
    cameras: Query<(), With<Camera>>,
    mut players: Query<&mut AnimationPlayer>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Ok(scene) = scenes.get(ready.entity) else {
        return;
    };
    if let Ok(mut actor) = commands.get_entity(scene.actor) {
        actor.remove::<(
            Mesh3d,
            MeshMaterial3d<StandardMaterial>,
            MeshMaterial3d<GraphMaterial3d>,
            MeshMaterial3d<crate::materials::BoxMaterial>,
            MeshMaterial3d<crate::batching::InstancedMaterial>,
            bevy::mesh::MeshTag,
            crate::batching::InstanceSlot,
        )>();
    }
    for entity in children.iter_descendants(ready.entity) {
        if cameras.contains(entity) {
            commands.entity(entity).despawn();
        }
    }
    let Some(gltf) = gltfs.get(&scene.gltf) else {
        return;
    };
    let clip = if scene.animation.is_empty() {
        gltf.animations.first().cloned()
    } else {
        let found = gltf.named_animations.get(scene.animation.as_str()).cloned();
        if found.is_none() {
            let mut names: Vec<&str> = gltf.named_animations.keys().map(|name| &**name).collect();
            names.sort_unstable();
            report(
                &scene.id,
                format!(
                    "{} has no animation called \"{}\" (it has: {})",
                    scene.source,
                    scene.animation,
                    if names.is_empty() {
                        "none".to_string()
                    } else {
                        names.join(", ")
                    }
                ),
            );
        }
        found
    };
    let Some(clip) = clip else {
        return;
    };
    let (graph, index) = AnimationGraph::from_clip(clip);
    let graph = graphs.add(graph);
    for entity in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(entity) {
            player.play(index).repeat();
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(graph.clone()));
        }
    }
}

/// Say so once when a model file won't load; the box keeps standing in.
pub fn watch_models(mut scenes: Query<&mut ModelScene>, assets: Res<AssetServer>) {
    for mut scene in &mut scenes {
        if scene.reported {
            continue;
        }
        if let bevy::asset::LoadState::Failed(error) = assets.load_state(&scene.gltf) {
            scene.reported = true;
            report(
                &scene.id,
                format!("couldn't load the model {}: {error}", scene.source),
            );
        }
    }
}

/// Rigs hold still while the game is paused and move otherwise, including in
/// the scene view, so an idle loop shows while editing.
pub fn pause_rigs(engine: NonSend<Engine>, mut players: Query<&mut AnimationPlayer>) {
    let paused = engine.running && engine.paused;
    for mut player in &mut players {
        if paused && !player.all_paused() {
            player.pause_all();
        } else if !paused && player.all_paused() {
            player.resume_all();
        }
    }
}

/// A LOD-culled placeholder hides nothing by itself: the loaded glTF scene
/// hangs off `ModelChild` as its own entities with their own visibility.
/// Propagate the group's verdict onto the child, so a distant model sheds
/// its draws exactly like a culled box does. Loaded scenes carry no
/// decimated meshes, so this is a cull-only level: the whole scene shows
/// or none of it does.
pub fn sync_model_lod(
    actors: Query<(&crate::culling::LodGroup, &ModelChild)>,
    mut visibility: Query<&mut Visibility>,
) {
    for (group, child) in &actors {
        let Ok(mut shown) = visibility.get_mut(child.0) else {
            continue;
        };
        let hidden = group.is_culled();
        let want = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *shown != want {
            *shown = want;
        }
    }
}

fn report(actor: &str, message: String) {
    crate::bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective_camera(app: &mut App) {
        app.world_mut().spawn((
            crate::world::WorldCamera,
            Camera::default(),
            GlobalTransform::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: std::f32::consts::FRAC_PI_2,
                ..default()
            }),
        ));
    }

    #[test]
    fn culled_placeholders_hide_the_loaded_scene() {
        let mut app = App::new();
        app.init_resource::<crate::culling::LodPolicy>()
            .init_resource::<crate::culling::Culling>()
            .init_resource::<crate::quality::Scaling>()
            .add_systems(Update, (crate::culling::select_lod, sync_model_lod).chain());
        perspective_camera(&mut app);
        let child = app.world_mut().spawn(Visibility::Inherited).id();
        // A 2 m box bounds a 1.73 m sphere: visible at 5 m, gone at 200 m.
        let group = crate::culling::LodGroup::new(1.732).level(0.01, None);
        let actor = app
            .world_mut()
            .spawn((
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
                group,
                ModelChild(child),
            ))
            .id();
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -200.0));
        app.update();
        assert!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .is_culled()
        );
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Hidden
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0));
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
    }
}
