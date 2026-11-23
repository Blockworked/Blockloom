//! A primitive spectator view; no VM, physics or scripts run here. With a
//! local copy of the game, its plugins rebuild what the host syncs (voxels).
use bevy::prelude::*;
use blockloom_core::scene::Visual;
use blockloom_net::game::{Invite, LanClient};
use blockloom_net::{ClientOptions, Fingerprint};
use std::collections::BTreeMap;

#[derive(Resource, Default)]
struct View {
    actors: BTreeMap<String, (Entity, Vec<u8>)>,
    epoch: Option<u64>,
    dimension: u8,
    camera: Option<Entity>,
    light: Option<Entity>,
    skipped: std::collections::BTreeSet<String>,
}
#[derive(serde::Deserialize)]
struct Appearance {
    visual: Option<Visual>,
}

/// The local game's plugins, hosted only to apply the host's synced regions.
/// Nothing executable arrives over the network: the code is the local build's.
#[cfg(feature = "plugins")]
pub(crate) struct GuestPlugins {
    world: blockloom_plugin_host::world::WorldPlugins,
    pending: Vec<blockloom_plugin_host::world::Outcome>,
    applied: Option<BTreeMap<String, blockloom_net::game::Region>>,
    meshes: BTreeMap<(String, String), Entity>,
    bounds: Option<(Vec3, Vec3)>,
    framed: bool,
}

#[cfg(feature = "plugins")]
impl GuestPlugins {
    /// Opens the plugins of the game at `path` (a build folder, its `game/`
    /// or the pack) and starts them as a preview, which writes no saves.
    pub(crate) fn open(path: &std::path::Path) -> Result<Self, String> {
        use blockloom_plugin_api::data::DATA_DIR;
        use blockloom_plugin_host::services::HostServices;
        use blockloom_plugin_host::storage::PackStore;
        let file = crate::player::locate_pack(path)
            .ok_or_else(|| format!("{} doesn't hold a Blockloom game", path.display()))?;
        let pack = blockloom_core::pack::GamePack::read(&file)?;
        let dir = file.parent().unwrap_or(std::path::Path::new("."));
        crate::api::link_engine_modules();
        let loadout =
            crate::plugins::shipped_loadout(dir, &pack.plugins).map_err(|e| e.join("; "))?;
        let host = HostServices::new(env!("CARGO_PKG_VERSION"))
            .with_project_store(std::sync::Arc::new(PackStore::open(dir.join(DATA_DIR))))
            .with_assets(dir.to_path_buf());
        Ok(Self::hosting(
            blockloom_plugin_host::world::WorldPlugins::load_with(&loadout, &host),
            &pack.project,
        ))
    }

    pub(crate) fn hosting(
        mut world: blockloom_plugin_host::world::WorldPlugins,
        project: &blockloom_core::project::Project,
    ) -> Self {
        let pending = world.start(&|plugin| crate::plugins::records_of(project, plugin, true));
        GuestPlugins {
            world,
            pending,
            applied: None,
            meshes: BTreeMap::new(),
            bounds: None,
            framed: false,
        }
    }
}

pub fn run(invite: &str, trusted: &str, game: Option<&std::path::Path>) -> Result<(), String> {
    #[cfg(feature = "plugins")]
    let plugins = game.map(GuestPlugins::open).transpose()?;
    #[cfg(not(feature = "plugins"))]
    if game.is_some() {
        return Err("This runtime was built without plugin support".into());
    }
    let invite = Invite::decode(invite)?;
    let trusted: Fingerprint = trusted
        .parse()
        .map_err(|e: blockloom_net::NetError| e.to_string())?;
    let client = LanClient::connect(invite, trusted.0, ClientOptions::default())?;
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Blockloom LAN spectator".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .set(crate::asset_plugin()),
    );
    app.insert_non_send(client).init_resource::<View>();
    #[cfg(feature = "plugins")]
    if let Some(plugins) = plugins {
        app.insert_non_send(plugins);
    }
    app.add_systems(Update, receive);
    app.run();
    Ok(())
}

fn supported(visual: Option<&Visual>, dimension: u8) -> Result<(), String> {
    let lengths: Vec<f32> = match visual {
        None => vec![],
        Some(Visual::Rect { size, .. }) if dimension == 0 => size.to_vec(),
        Some(Visual::Circle { radius, .. }) if dimension == 0 => vec![*radius],
        Some(Visual::Cuboid { size, .. }) if dimension == 1 => size.to_vec(),
        Some(Visual::Sphere { radius, .. }) if dimension == 1 => vec![*radius],
        Some(Visual::Capsule { radius, height, .. }) if dimension == 1 => vec![*radius, *height],
        Some(Visual::Plane { size, .. }) if dimension == 1 => size.to_vec(),
        _ => return Err(
            "the spectator view draws primitive looks only; use blockloom-lan for other replicas"
                .into(),
        ),
    };
    if lengths
        .iter()
        .any(|n| !n.is_finite() || *n <= 0. || *n > 1e6)
    {
        return Err("invalid primitive size".into());
    }
    Ok(())
}
fn receive(
    mut client: NonSendMut<LanClient>,
    mut view: ResMut<View>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
    mut exit: MessageWriter<AppExit>,
    #[cfg(feature = "plugins")] mut guest: Option<NonSendMut<GuestPlugins>>,
    #[cfg(feature = "plugins")] assets: Option<Res<AssetServer>>,
) {
    let changed = match client.poll() {
        Ok(v) => v,
        Err(e) => {
            error!("LAN spectator disconnected: {e}");
            exit.write(AppExit::error());
            return;
        }
    };
    if !changed {
        return;
    }
    let Some(state) = client.state() else {
        return;
    };
    let mut appearances = BTreeMap::new();
    for a in state.actors.values() {
        let appearance: Appearance = match serde_json::from_slice(&a.appearance) {
            Ok(a) => a,
            Err(e) => {
                error!("Invalid actor appearance: {e}");
                exit.write(AppExit::error());
                return;
            }
        };
        let mut appearance = appearance;
        // A look this view can't draw leaves that actor out, not the world.
        if let Err(e) = supported(appearance.visual.as_ref(), state.dimension) {
            if view.skipped.insert(a.id.clone()) {
                warn!("{}: {e}", a.id);
            }
            appearance.visual = None;
        }
        appearances.insert(a.id.clone(), appearance);
    }
    if view.epoch != Some(state.epoch) {
        for (_, (entity, _)) in std::mem::take(&mut view.actors) {
            commands.entity(entity).despawn();
        }
        for entity in [view.camera.take(), view.light.take()]
            .into_iter()
            .flatten()
        {
            commands.entity(entity).despawn();
        }
        view.camera = Some(if state.dimension == 0 {
            commands.spawn(Camera2d).id()
        } else {
            commands
                .spawn((
                    Camera3d::default(),
                    Transform::from_xyz(0., 10., 20.).looking_at(Vec3::ZERO, Vec3::Y),
                ))
                .id()
        });
        if state.dimension == 1 {
            view.light = Some(
                commands
                    .spawn((
                        DirectionalLight::default(),
                        Transform::from_xyz(0.3, 1., 0.5).looking_at(Vec3::ZERO, Vec3::Y),
                    ))
                    .id(),
            );
        }
        view.epoch = Some(state.epoch);
        view.dimension = state.dimension;
        #[cfg(feature = "plugins")]
        if let Some(guest) = guest.as_mut() {
            guest.applied = None;
            guest.framed = false;
        }
    }
    #[cfg(feature = "plugins")]
    if let Some(guest) = guest.as_mut() {
        sync_plugins(
            guest,
            &state.regions,
            state.dimension,
            view.camera,
            &mut commands,
            &mut meshes,
            &mut materials,
            assets.as_deref(),
        );
    }
    let removed: Vec<_> = view
        .actors
        .keys()
        .filter(|id| !state.actors.contains_key(*id))
        .cloned()
        .collect();
    for id in removed {
        let (entity, _) = view.actors.remove(&id).unwrap();
        commands.entity(entity).despawn();
    }
    for a in state.actors.values() {
        if view
            .actors
            .get(&a.id)
            .is_some_and(|(_, bytes)| *bytes != a.appearance)
        {
            let (entity, _) = view.actors.remove(&a.id).unwrap();
            commands.entity(entity).despawn();
        }
        let entity = if let Some((entity, _)) = view.actors.get(&a.id) {
            *entity
        } else {
            let entity = commands.spawn_empty().id();
            let visual = appearances[&a.id].visual.as_ref();
            let color = visual
                .and_then(Visual::color)
                .and_then(|s| Srgba::hex(s).map(Color::from).ok())
                .unwrap_or(Color::WHITE);
            match visual {
                Some(Visual::Rect { size, .. }) => {
                    commands
                        .entity(entity)
                        .insert(Sprite::from_color(color, Vec2::from_array(*size)));
                }
                Some(Visual::Circle { radius, .. }) => {
                    commands.entity(entity).insert((
                        Mesh2d(meshes.add(Circle::new(*radius))),
                        MeshMaterial2d(colors.add(color)),
                    ));
                }
                Some(v) => {
                    let mesh: Mesh = match v {
                        Visual::Cuboid { size, .. } => {
                            Cuboid::new(size[0], size[1], size[2]).into()
                        }
                        Visual::Sphere { radius, .. } => Sphere::new(*radius).into(),
                        Visual::Capsule { radius, height, .. } => {
                            Capsule3d::new(*radius, (*height - 2. * radius).max(0.)).into()
                        }
                        Visual::Plane { size, .. } => {
                            Plane3d::default().mesh().size(size[0], size[1]).build()
                        }
                        _ => unreachable!("validated look"),
                    };
                    commands.entity(entity).insert((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: color,
                            unlit: true,
                            ..Default::default()
                        })),
                    ));
                }
                None => {}
            }
            view.actors
                .insert(a.id.clone(), (entity, a.appearance.clone()));
            entity
        };
        let pose = &a.pose;
        commands.entity(entity).insert((
            Transform {
                translation: Vec3::from_slice(&pose[..3]),
                rotation: Quat::from_xyzw(pose[3], pose[4], pose[5], pose[6]),
                scale: Vec3::from_slice(&pose[7..10]),
            },
            if a.visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
    }
}

/// Whether `regions` moved on from what the guest's plugins last applied.
#[cfg(feature = "plugins")]
fn regions_moved(
    old: Option<&BTreeMap<String, blockloom_net::game::Region>>,
    new: &BTreeMap<String, blockloom_net::game::Region>,
) -> bool {
    old.is_none_or(|old| {
        old.len() != new.len()
            || old
                .iter()
                .zip(new)
                .any(|((a, x), (b, y))| a != b || x.revision != y.revision)
    })
}

/// Hands newly synced regions to the local plugins and draws what they answer.
#[cfg(feature = "plugins")]
#[allow(clippy::too_many_arguments)]
fn sync_plugins(
    guest: &mut GuestPlugins,
    regions: &BTreeMap<String, blockloom_net::game::Region>,
    dimension: u8,
    camera: Option<Entity>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    assets: Option<&AssetServer>,
) {
    use blockloom_plugin_host::world::{Effect, Outcome};
    let mut out = std::mem::take(&mut guest.pending);
    if regions_moved(guest.applied.as_ref(), regions) {
        for (plugin, args) in super::sync_apply_calls(guest.applied.as_ref(), regions) {
            out.extend(guest.world.act(&plugin, "sync.apply", &args));
        }
        guest.applied = Some(regions.clone());
    }
    out.extend(guest.world.drain());
    for outcome in out {
        let (plugin, effect) = match outcome {
            Outcome::Effect { plugin, effect } => (plugin, effect),
            Outcome::Error { plugin, message } => {
                warn!("{plugin}: {message}");
                continue;
            }
            Outcome::Log { plugin, text, .. } => {
                info!("{plugin}: {text}");
                continue;
            }
        };
        match effect {
            // Plugin meshes are 3D only, as in the game.
            Effect::Mesh(mesh) if dimension == 1 => {
                if let Err(e) = mesh.check() {
                    warn!("{plugin}: {e}");
                    continue;
                }
                let key = (plugin, mesh.name.clone());
                if let Some(old) = guest.meshes.remove(&key) {
                    commands.entity(old).despawn();
                }
                let origin = Vec3::from_array(mesh.origin);
                for p in mesh.positions.as_chunks::<3>().0 {
                    let p = origin + Vec3::from_array(*p);
                    let (lo, hi) = guest.bounds.get_or_insert((p, p));
                    *lo = lo.min(p);
                    *hi = hi.max(p);
                }
                let emission = mesh.emission.unwrap_or([0.; 3]);
                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: mesh
                        .texture
                        .clone()
                        .zip(assets)
                        .map(|(path, server)| server.load::<Image>(path)),
                    perceptual_roughness: mesh.roughness.clamp(0.05, 1.),
                    emissive: LinearRgba::rgb(emission[0], emission[1], emission[2]),
                    alpha_mode: crate::plugin_meshes::alpha_mode(mesh.blend),
                    ..Default::default()
                });
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(crate::plugin_meshes::bevy_mesh(&mesh))),
                        MeshMaterial3d(material),
                        Transform::from_translation(origin),
                    ))
                    .id();
                guest.meshes.insert(key, entity);
            }
            Effect::RemoveMesh { name } => {
                if let Some(old) = guest.meshes.remove(&(plugin, name)) {
                    commands.entity(old).despawn();
                }
            }
            Effect::MeshVisibility { name, visible } => {
                if let Some(entity) = guest.meshes.get(&(plugin, name)) {
                    commands.entity(*entity).insert(if visible {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    });
                }
            }
            _ => {}
        }
    }
    // The fixed camera can't know where a world is, so it frames the first one.
    if let (false, Some((lo, hi)), Some(camera), 1) =
        (guest.framed, guest.bounds, camera, dimension)
    {
        let centre = (lo + hi) / 2.;
        let reach = (hi - lo).length().max(1.);
        commands.entity(camera).insert(
            Transform::from_translation(centre + Vec3::new(0., 0.45, 0.8) * reach)
                .looking_at(centre, Vec3::Y),
        );
        guest.framed = true;
    }
}

/// A windowless guest view over `client`, for tests.
#[cfg(test)]
pub(crate) fn test_app(
    client: LanClient,
    #[cfg(feature = "plugins")] plugins: Option<GuestPlugins>,
) -> App {
    let mut app = App::new();
    app.insert_non_send(client)
        .init_resource::<View>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<ColorMaterial>>()
        .init_resource::<Messages<AppExit>>()
        .add_systems(Update, receive);
    #[cfg(feature = "plugins")]
    if let Some(plugins) = plugins {
        app.insert_non_send(plugins);
    }
    app
}

#[cfg(all(test, feature = "plugins"))]
impl GuestPlugins {
    pub(crate) fn query(
        &mut self,
        plugin: &str,
        op: &str,
        args: &serde_json::Value,
    ) -> serde_json::Value {
        self.world.query(plugin, op, args).unwrap()
    }

    pub(crate) fn mesh_names(&self) -> Vec<&str> {
        self.meshes.keys().map(|(_, name)| name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primitive_replicas_spawn_move_and_disappear_without_a_renderer() {
        use blockloom_net::ServerOptions;
        use blockloom_net::game::{ActorState, LanHost, Snapshot, build_hash};
        use std::time::{Duration, Instant};
        for dimension in [0, 1] {
            let visual = if dimension == 0 {
                Visual::Rect {
                    color: "#ffffff".into(),
                    size: [10.; 2],
                }
            } else {
                Visual::Cuboid {
                    color: "#ffffff".into(),
                    size: [1.; 3],
                }
            };
            let mut pose = [0.; 16];
            pose[6] = 1.;
            pose[7..10].fill(1.);
            let actor = ActorState {
                id: "actor".into(),
                template: "actor".into(),
                pose,
                visible: true,
                appearance: serde_json::to_vec(&serde_json::json!({"visual":visual})).unwrap(),
            };
            let mut state = Snapshot {
                epoch: 1,
                tick: 1,
                dimension,
                actors: BTreeMap::from([("actor".into(), actor)]),
                ..Default::default()
            };
            let build = build_hash(b"view fixture");
            let mut host = LanHost::open(
                "127.0.0.1:0".parse().unwrap(),
                build,
                state.clone(),
                ServerOptions::default(),
            )
            .unwrap();
            let client =
                LanClient::connect(host.invite().clone(), build, ClientOptions::default()).unwrap();
            let mut app = test_app(
                client,
                #[cfg(feature = "plugins")]
                None,
            );
            for phase in 0..3 {
                if phase == 1 {
                    state.tick += 1;
                    state.actors.get_mut("actor").unwrap().pose[0] = 15.;
                    host.publish(state.clone()).unwrap();
                }
                if phase == 2 {
                    state.tick += 1;
                    state.actors.clear();
                    host.publish(state.clone()).unwrap();
                }
                let end = Instant::now() + Duration::from_secs(5);
                loop {
                    host.poll();
                    app.update();
                    if app.world().non_send::<LanClient>().state() == Some(&state) {
                        break;
                    }
                    assert!(Instant::now() < end);
                    std::thread::sleep(Duration::from_millis(1));
                }
                let view = app.world().resource::<View>();
                assert_eq!(view.actors.len(), state.actors.len());
                if phase < 2 {
                    let entity = view.actors["actor"].0;
                    assert_eq!(
                        app.world().get::<Transform>(entity).unwrap().translation.x,
                        if phase == 0 { 0. } else { 15. }
                    );
                }
            }
        }
    }

    #[test]
    fn unsupported_assets_and_non_finite_geometry_are_refused() {
        assert!(
            supported(
                Some(&Visual::Image {
                    path: "outside.png".into(),
                    size: [32.; 2]
                }),
                0
            )
            .is_err()
        );
        assert!(
            supported(
                Some(&Visual::Circle {
                    color: "#ffffff".into(),
                    radius: f32::NAN
                }),
                0
            )
            .is_err()
        );
        assert!(
            supported(
                Some(&Visual::Cuboid {
                    color: "#ffffff".into(),
                    size: [1.; 3]
                }),
                1
            )
            .is_ok()
        );
    }
}
