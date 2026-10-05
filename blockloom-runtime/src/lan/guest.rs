//! A primitive spectator view; no VM, physics, scripts or plugins run here.
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
}
#[derive(serde::Deserialize)]
struct Appearance {
    visual: Option<Visual>,
}

pub fn run(invite: &str, trusted: &str) -> Result<(), String> {
    let invite = Invite::decode(invite)?;
    let trusted: Fingerprint = trusted
        .parse()
        .map_err(|e: blockloom_net::NetError| e.to_string())?;
    let client = LanClient::connect(invite, trusted.0, ClientOptions::default())?;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Blockloom LAN spectator".into(),
            ..Default::default()
        }),
        ..Default::default()
    }));
    app.insert_non_send(client).init_resource::<View>();
    app.add_systems(Update, receive);
    app.run();
    Ok(())
}

fn supported(visual: Option<&Visual>, dimension: u8) -> Result<(), String> {
    let lengths:Vec<f32>=match visual {
        None=>vec![],
        Some(Visual::Rect{size,..}) if dimension==0=>size.to_vec(),
        Some(Visual::Circle{radius,..}) if dimension==0=>vec![*radius],
        Some(Visual::Cuboid{size,..}) if dimension==1=>size.to_vec(),
        Some(Visual::Sphere{radius,..}) if dimension==1=>vec![*radius],
        Some(Visual::Capsule{radius,height,..}) if dimension==1=>vec![*radius,*height],
        Some(Visual::Plane{size,..}) if dimension==1=>size.to_vec(),
        _=>return Err("The spectator viewer supports primitive looks only; use blockloom-lan for other replicas".into()),
    };
    if lengths
        .iter()
        .any(|n| !n.is_finite() || *n <= 0. || *n > 1e6)
    {
        return Err("Invalid primitive size".into());
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
        if let Err(e) = supported(appearance.visual.as_ref(), state.dimension) {
            error!("{e}");
            exit.write(AppExit::error());
            return;
        }
        appearances.insert(a.id.clone(), appearance);
    }
    if view.epoch != Some(state.epoch) {
        for (_, (entity, _)) in std::mem::take(&mut view.actors) {
            commands.entity(entity).despawn();
        }
        if let Some(camera) = view.camera.take() {
            commands.entity(camera).despawn();
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
        view.epoch = Some(state.epoch);
        view.dimension = state.dimension;
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
            let mut app = App::new();
            app.insert_non_send(client)
                .init_resource::<View>()
                .init_resource::<Assets<Mesh>>()
                .init_resource::<Assets<StandardMaterial>>()
                .init_resource::<Assets<ColorMaterial>>()
                .init_resource::<Messages<AppExit>>()
                .add_systems(Update, receive);
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
