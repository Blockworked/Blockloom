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
use std::time::{Duration, SystemTime};

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

/// On a loaded model scene: every animation in its file as one graph, and
/// what the animator last put on the rig.
#[derive(Component)]
pub struct ModelRig {
    players: Vec<Entity>,
    /// Lowercased animation name (`animationN` by index too) to its node.
    nodes: HashMap<String, AnimationNodeIndex>,
    missing: Option<String>,
}

impl ModelRig {
    fn node(&self, name: &str) -> Option<AnimationNodeIndex> {
        self.nodes.get(&name.trim().to_lowercase()).copied()
    }
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
    // Every animation goes in one graph, so the animator can crossfade
    // between them; the look's own animation loops until it takes over.
    let mut graph = AnimationGraph::new();
    let mut nodes = HashMap::new();
    let named: HashMap<&Handle<AnimationClip>, &str> = gltf
        .named_animations
        .iter()
        .map(|(name, clip)| (clip, &**name))
        .collect();
    let mut order = Vec::new();
    for (index, clip) in gltf.animations.iter().enumerate() {
        let node = graph.add_clip(clip.clone(), 1.0, graph.root);
        nodes.insert(format!("animation{index}"), node);
        if let Some(name) = named.get(clip) {
            nodes.insert(name.to_lowercase(), node);
        }
        order.push(node);
    }
    let start = if scene.animation.is_empty() {
        order.first().copied()
    } else {
        let found = nodes.get(&scene.animation.to_lowercase()).copied();
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
    let graph = graphs.add(graph);
    let mut rigged = Vec::new();
    for entity in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            if let Some(node) = start {
                transitions.play(&mut player, node, Duration::ZERO).repeat();
            }
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(graph.clone()), transitions));
            rigged.push(entity);
        }
    }
    commands.entity(ready.entity).insert(ModelRig {
        players: rigged,
        nodes,
        missing: None,
    });
}

/// The drawing half of the 3D animator: puts the clips each actor's animator
/// plays (`anim2d::step_animations` keeps its clock, blend shares, markers and
/// states on the tick) on its model, each at its weight and held to the
/// animator's time, so the rig shows exactly where the simulation stands.
/// The weights are set here, not by Bevy's transitions, so a crossfade out of
/// a blend fades every clip of it.
pub fn drive_animations(
    mut commands: Commands,
    engine: NonSend<Engine>,
    fixed: Res<Time<Fixed>>,
    actors: Query<(
        &crate::engine::ActorId,
        &crate::engine::AnimationPlayer,
        &ModelChild,
    )>,
    mut rigs: Query<&mut ModelRig>,
    mut players: Query<(&mut AnimationPlayer, Has<AnimationTransitions>)>,
) {
    for (id, logical, child) in &actors {
        let Ok(mut rig) = rigs.get_mut(child.0) else {
            continue;
        };
        let Some(spec) = engine.actor(&id.0).and_then(|a| a.components.animation()) else {
            continue;
        };
        if spec.find_clip(&logical.clip).is_none() {
            continue;
        }
        // Between ticks the rig carries on at the animator's speed.
        let ahead = |speed: f32, playing: bool| {
            if playing {
                fixed.overstep().as_secs_f32() * speed.max(0.0)
            } else {
                0.0
            }
        };
        let fade_in = logical
            .fade
            .as_ref()
            .filter(|fade| fade.total > 0.0)
            .map_or(1.0, |fade| (1.0 - fade.left / fade.total).clamp(0.0, 1.0));
        // Every clip showing, its weight and its time.
        let mut wanted: Vec<(String, f32, f32)> = Vec::new();
        let mut put = |clip_name: &str, weight: f32, elapsed: f32, phase: Option<f32>| {
            let Some(clip) = spec.find_clip(clip_name) else {
                return;
            };
            let at = match phase {
                Some(phase) => clip.local_time(phase * clip.duration()),
                None => clip.local_time(elapsed),
            };
            wanted.push((clip.rig_name().to_string(), weight, at));
        };
        let now = ahead(logical.speed, logical.playing);
        if logical.blend.is_empty() {
            put(&logical.clip, fade_in, logical.elapsed + now, None);
        } else {
            let phase = logical.phase + now / logical.cycle.max(1e-4);
            for (name, w) in &logical.blend {
                put(name, w * fade_in, 0.0, Some(phase));
            }
        }
        if let Some(fade) = &logical.fade {
            let out = 1.0 - fade_in;
            let then = ahead(fade.speed, logical.playing);
            if fade.blend.is_empty() {
                put(&fade.clip, out, fade.elapsed + then, None);
            } else {
                let phase = fade.phase + then / fade.cycle.max(1e-4);
                for (name, w) in &fade.blend {
                    put(name, w * out, 0.0, Some(phase));
                }
            }
        }
        let mut weights: HashMap<AnimationNodeIndex, (f32, f32)> = HashMap::new();
        for (name, weight, at) in &wanted {
            match rig.node(name) {
                Some(node) => {
                    let entry = weights.entry(node).or_insert((0.0, *at));
                    entry.0 += weight;
                }
                None if rig.missing.as_deref() != Some(name.as_str()) => {
                    rig.missing = Some(name.clone());
                    report(
                        &id.0,
                        format!("the model has no animation called \"{name}\" for its animator"),
                    );
                }
                None => {}
            }
        }
        for &entity in &rig.players {
            let Ok((mut player, transitions)) = players.get_mut(entity) else {
                continue;
            };
            if transitions {
                // The animator takes over from the look's own looping clip.
                commands.entity(entity).remove::<AnimationTransitions>();
            }
            let stale: Vec<AnimationNodeIndex> = player
                .playing_animations()
                .map(|(node, _)| *node)
                .filter(|node| !weights.contains_key(node))
                .collect();
            for node in stale {
                player.stop(node);
            }
            for (&node, &(weight, at)) in &weights {
                if player.animation(node).is_none() {
                    player.play(node);
                }
                if let Some(active) = player.animation_mut(node) {
                    active.set_speed(0.0).set_weight(weight).seek_to(at);
                }
            }
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
/// Visible models keep their authored meshes; distance culling hides the scene.
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
    use bevy::mesh::{Indices, PrimitiveTopology};

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

    /// The drawing half follows the animator: the clip it plays goes on the
    /// rig held at the animator's time, and a state change crossfades.
    #[test]
    fn the_rig_follows_the_animators_clip_time_and_crossfade() {
        use crate::engine::InsertEngine;
        use blockloom_core::animation::{AnimationClip as Clip, AnimationSpec, LoopMode};
        use blockloom_core::components::ActorComponent;
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), bevy::time::TimePlugin));
        app.init_resource::<Assets<AnimationClip>>()
            .init_resource::<Assets<AnimationGraph>>();
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        let mut actor = blockloom_core::project::Actor::new(
            "Hero",
            Visual::Cuboid {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        let spec = AnimationSpec {
            clips: vec![
                Clip::timed("Walk", 0.8, LoopMode::Loop),
                Clip {
                    rig_animation: "jump_up".to_string(),
                    ..Clip::timed("Jump", 0.5, LoopMode::Once)
                },
            ],
            ..serde_json::from_str("{}").unwrap()
        };
        actor
            .components
            .insert(ActorComponent::Animation { animation: spec });
        let id = actor.id.clone();
        engine.project.actors.push(actor);
        app.insert_engine(engine);

        let mut graph = AnimationGraph::new();
        let mut clips = app.world_mut().resource_mut::<Assets<AnimationClip>>();
        let walk_clip = clips.add(AnimationClip::default());
        let jump_clip = clips.add(AnimationClip::default());
        let walk = graph.add_clip(walk_clip, 1.0, graph.root);
        let jump = graph.add_clip(jump_clip, 1.0, graph.root);
        let graph = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(graph);
        let mut player = AnimationPlayer::default();
        let mut transitions = AnimationTransitions::new();
        transitions.play(&mut player, walk, Duration::ZERO);
        let rigged = app
            .world_mut()
            .spawn((player, transitions, AnimationGraphHandle(graph)))
            .id();
        let scene = app
            .world_mut()
            .spawn(ModelRig {
                players: vec![rigged],
                nodes: [("walk".to_string(), walk), ("jump_up".to_string(), jump)]
                    .into_iter()
                    .collect(),
                missing: None,
            })
            .id();
        let actor = app
            .world_mut()
            .spawn((
                crate::engine::ActorId(id),
                ModelChild(scene),
                crate::engine::AnimationPlayer {
                    clip: "Walk".to_string(),
                    elapsed: 1.0,
                    speed: 1.0,
                    ..Default::default()
                },
            ))
            .id();
        app.add_systems(Update, drive_animations);
        app.update();
        let at = |app: &App, node| {
            app.world()
                .get::<AnimationPlayer>(rigged)
                .unwrap()
                .animation(node)
                .map(|active| active.seek_time())
        };
        let weight = |app: &App, node| {
            app.world()
                .get::<AnimationPlayer>(rigged)
                .unwrap()
                .animation(node)
                .map_or(0.0, |active| active.weight())
        };
        // 1 s into a 0.8 s loop is 0.2 s in.
        assert!((at(&app, walk).unwrap() - 0.2).abs() < 1e-4);

        let mut logical = app
            .world_mut()
            .get_mut::<crate::engine::AnimationPlayer>(actor)
            .unwrap();
        logical.clip = "Jump".to_string();
        logical.elapsed = 0.1;
        logical.fade = Some(crate::engine::AnimationFade {
            clip: "Walk".to_string(),
            elapsed: 1.1,
            speed: 1.0,
            left: 0.05,
            total: 0.2,
            ..Default::default()
        });
        app.update();
        assert!((at(&app, jump).unwrap() - 0.1).abs() < 1e-4);
        // Walk fades out from where it was, still held to the animator.
        assert!((at(&app, walk).unwrap() - 0.3).abs() < 1e-4);
        assert!((weight(&app, jump) - 0.75).abs() < 1e-4);
        assert!((weight(&app, walk) - 0.25).abs() < 1e-4);
        assert!(app.world().get::<AnimationTransitions>(rigged).is_none());

        // A blend plays every clip at its share, in step through one cycle.
        let mut logical = app
            .world_mut()
            .get_mut::<crate::engine::AnimationPlayer>(actor)
            .unwrap();
        logical.fade = None;
        logical.clip = "Walk".to_string();
        logical.blend = vec![("Walk".to_string(), 0.75), ("Jump".to_string(), 0.25)];
        logical.phase = 2.5;
        logical.cycle = 0.725;
        app.update();
        assert!((at(&app, walk).unwrap() - 0.4).abs() < 1e-4);
        assert!(
            (at(&app, jump).unwrap() - 0.5).abs() < 1e-4,
            "a Once clip holds its end"
        );
        assert!((weight(&app, walk) - 0.75).abs() < 1e-4);
        assert!((weight(&app, jump) - 0.25).abs() < 1e-4);
        assert!(app.world().get::<ModelRig>(scene).is_some());
    }

    #[test]
    fn culled_placeholders_hide_the_loaded_scene() {
        let mut app = App::new();
        app.init_resource::<crate::culling::LodPolicy>()
            .init_resource::<crate::culling::Culling>()
            .init_resource::<crate::quality::Scaling>()
            .init_resource::<Assets<Mesh>>()
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

    /// A `quads x quads` grid on the XZ plane, with normals and UVs.
    fn grid_mesh(quads: u32) -> Mesh {
        let n = quads + 1;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        for z in 0..n {
            for x in 0..n {
                positions.push([x as f32, 0.0, z as f32]);
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([x as f32 / quads as f32, z as f32 / quads as f32]);
            }
        }
        let mut indices = Vec::new();
        for z in 0..quads {
            for x in 0..quads {
                let a = z * n + x;
                indices.extend([a, a + 1, a + n, a + 1, a + n + 1, a + n]);
            }
        }
        Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
    }

    #[test]
    fn model_meshes_stay_intact_until_culled_and_return_up_close() {
        let mut app = App::new();
        app.init_resource::<crate::culling::LodPolicy>()
            .init_resource::<crate::culling::Culling>()
            .init_resource::<crate::quality::Scaling>()
            .init_resource::<Assets<Mesh>>()
            .add_systems(Update, (crate::culling::select_lod, sync_model_lod).chain());
        perspective_camera(&mut app);
        let original = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(grid_mesh(8));
        let part = app
            .world_mut()
            .spawn((Mesh3d(original.clone()), Visibility::Inherited))
            .id();
        let mut scene = app.world_mut().spawn(Visibility::Inherited);
        scene.add_child(part);
        let child = scene.id();
        let group = crate::performance::RenderCache::default()
            .lod(
                &Visual::Model {
                    path: "test.glb".into(),
                    tint: "#fff".into(),
                    scale: [2.0; 3],
                    animation: String::new(),
                },
                &original,
                &mut app.world_mut().resource_mut::<Assets<Mesh>>(),
            )
            .unwrap();
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
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(0)
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -100.0));
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(0)
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        let meshes = app.world().resource::<Assets<Mesh>>();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes.get(&original).unwrap().indices().unwrap().len(), 384);
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0));
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(0)
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        // Budget pressure changes culling distance without punching mesh holes.
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .geometry
            .factors[2] = 0.5;
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -50.0));
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
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
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0));
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
    }
}
