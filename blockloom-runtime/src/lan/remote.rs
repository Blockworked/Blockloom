//! A LAN host as a guest's server: the guest runs the game's own world as a
//! client, fed by the host's effects and snapshots, and sends its seat's
//! actions back.

use crate::server::{ActorPose, ClientInput, Roster, ServerLink, ServerTick};
use bevy::prelude::*;
use blockloom_net::ClientOptions;
use blockloom_net::game::{ActionInput, Invite, LanClient, ViewInput};
use blockloom_protocol::EditorMessage;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The host this process joins instead of running its own server.
pub struct JoinTarget {
    pub invite: String,
    pub trusted: String,
}

static JOIN: OnceLock<JoinTarget> = OnceLock::new();

/// Makes this process's world a client of the host `invite` names.
pub fn set_join(target: JoinTarget) {
    let _ = JOIN.set(target);
}

pub fn join_target() -> Option<&'static JoinTarget> {
    JOIN.get()
}

pub struct RemoteServer {
    client: LanClient,
    failed: Option<String>,
    seen: u64,
    /// The newest tick whose effects arrived.
    effects_tick: u64,
}

impl RemoteServer {
    pub fn connect(target: &JoinTarget) -> Result<Self, String> {
        let invite = Invite::decode(&target.invite)?;
        let trusted = decode_hash(&target.trusted)?;
        let client = LanClient::connect(invite, trusted, ClientOptions::default())?;
        Ok(Self {
            client,
            failed: None,
            seen: 0,
            effects_tick: 0,
        })
    }
}

fn decode_hash(text: &str) -> Result<[u8; 32], String> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return Err("The trusted build hash must be 64 hex digits".into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| "The trusted build hash must be 64 hex digits")?;
    }
    Ok(out)
}

/// A guest's own actions by base name: player 1's, which its devices drive.
/// The host feeds them in as its seat's copies.
pub fn own_actions(sensors: &blockloom_core::sense::Sensors) -> BTreeMap<String, ActionInput> {
    sensors
        .actions
        .iter()
        .filter(|(name, _)| !is_player_copy(name))
        .take(blockloom_net::game::MAX_INPUTS)
        .filter(|(name, _)| !name.is_empty() && name.len() <= blockloom_net::game::MAX_INPUT_NAME)
        .map(|(name, a)| {
            let clamp = |v: f32| {
                if v.is_finite() {
                    v.clamp(
                        -blockloom_net::game::MAX_INPUT_VALUE,
                        blockloom_net::game::MAX_INPUT_VALUE,
                    )
                } else {
                    0.0
                }
            };
            (
                name.clone(),
                ActionInput {
                    held: a.held,
                    value: clamp(a.value),
                    vector: [clamp(a.vector[0]), clamp(a.vector[1])],
                },
            )
        })
        .collect()
}

/// `Jump P2` and the like: another local player's copy.
fn is_player_copy(name: &str) -> bool {
    name.rsplit_once(" P")
        .is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn pose_of(state: &blockloom_net::game::ActorState) -> ActorPose {
    let p = state.pose;
    ActorPose {
        id: state.id.clone(),
        transform: Transform {
            translation: Vec3::new(p[0], p[1], p[2]),
            rotation: Quat::from_xyzw(p[3], p[4], p[5], p[6]).normalize(),
            scale: Vec3::new(p[7], p[8], p[9]),
        },
        visible: state.visible,
    }
}

impl ServerLink for RemoteServer {
    fn ticks(&mut self) -> Vec<ServerTick> {
        if self.failed.is_some() {
            return Vec::new();
        }
        if let Err(error) = self.client.poll() {
            self.failed = Some(format!("Lost the LAN host: {error}"));
            return Vec::new();
        }
        let mut ticks = Vec::new();
        for (tick, bytes) in self.client.take_effects() {
            match serde_json::from_slice(&bytes) {
                Ok(effects) => {
                    self.effects_tick = self.effects_tick.max(tick);
                    ticks.push(ServerTick {
                        tick,
                        effects,
                        ..Default::default()
                    })
                }
                Err(error) => {
                    let message = format!(
                        "The host sent effects this build can't read: {error}: run the same build as the host"
                    );
                    self.failed = Some(message);
                    return Vec::new();
                }
            }
        }
        if self.client.sequence() != self.seen
            && let Some(state) = self.client.state()
        {
            self.seen = self.client.sequence();
            let mut poses: Vec<ActorPose> = state.actors.values().map(pose_of).collect();
            poses.sort_by(|a, b| a.id.cmp(&b.id));
            // A snapshot older than effects already taken would undo them.
            let roster = (state.tick >= self.effects_tick).then(|| Roster {
                scene: state.scene.clone(),
                actors: state
                    .actors
                    .values()
                    .map(|a| (a.id.clone(), a.template.clone()))
                    .collect(),
            });
            ticks.push(ServerTick {
                tick: state.tick,
                effects: Vec::new(),
                poses,
                roster,
            });
        }
        ticks
    }

    fn send(&mut self, input: ClientInput) {
        if self.failed.is_none()
            && let Err(error) = self.client.send_input(
                &own_actions(&input.sensors),
                ViewInput {
                    yaw: input.view.yaw,
                    facing: input.view.facing.map(|(_, yaw)| yaw),
                },
            )
        {
            self.failed = Some(error);
        }
    }

    fn editor(&mut self, _: EditorMessage) {}

    fn is_running(&self) -> bool {
        self.failed.is_none()
    }

    fn seat(&self) -> u8 {
        self.client.seat().unwrap_or(0)
    }

    fn remote(&self) -> bool {
        true
    }

    fn failure(&self) -> Option<String> {
        self.failed.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_sends_only_its_own_actions_and_clamps_them() {
        let mut sensors = blockloom_core::sense::Sensors::default();
        let action = |held, value| blockloom_core::input::ActionSense {
            held,
            value,
            ..Default::default()
        };
        sensors.actions.insert("Jump".into(), action(true, 1.0));
        sensors.actions.insert("Jump P2".into(), action(true, 1.0));
        sensors
            .actions
            .insert("Look".into(), action(false, f32::NAN));
        let got = own_actions(&sensors);
        assert_eq!(got.len(), 2);
        assert!(got["Jump"].held);
        assert_eq!(got["Look"].value, 0.0);
        assert!(!got.contains_key("Jump P2"));
        assert!(decode_hash("zz").is_err());
        assert_eq!(decode_hash(&"ab".repeat(32)).unwrap(), [0xab; 32]);
    }

    /// A host's server with LAN open and a guest's world following it: the
    /// guest holds its own `Go`, the host sees seat 1's `Go P2`, and the
    /// guest's copy of the actor moves where the host moved it.
    #[test]
    fn a_guest_world_plays_through_the_host() {
        use crate::engine::{ActorId, Engine};
        use blockloom_core::blocks::{Instruction, InstructionKind, Strand};
        use blockloom_core::input::{InputAction, InputBinding};
        use blockloom_core::project::{Actor, Project};
        use blockloom_core::scene::{Mode, Visual};
        use blockloom_core::value::{Op, Value};
        use std::time::{Duration, Instant};

        let mut actor = Actor::new(
            "Player 2",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(InstructionKind::WhenStarted),
                Instruction::new(InstructionKind::Forever {
                    body: vec![Instruction::new(InstructionKind::If {
                        condition: Value::op(
                            Op::from_name("ActionDown"),
                            vec![Value::text("Go P2")],
                        ),
                        body: vec![Instruction::new(InstructionKind::Move {
                            steps: Value::number(5.0),
                        })],
                    })],
                }),
            ],
        )];
        let mut project = Project::starter("lan", Mode::TwoD);
        project.actors = vec![actor];
        project.multiplayer.enabled = true;
        let go = InputAction::new("Go", vec![InputBinding::key("g")]);
        let copy = {
            let mut c = go.clone();
            c.name = "Go P2".into();
            c.player = 1;
            c.bindings.clear();
            c
        };
        project.settings.input.actions = vec![go, copy];

        let (mut host, remote) = host_with_lan(&project);
        let end = Instant::now() + Duration::from_secs(20);
        let (_e, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project = project;
        let mut guest = crate::simulation::headless(engine, Mode::TwoD, true);
        crate::server::follow(&mut guest, remote);
        let x = |app: &mut App| {
            let mut q = app.world_mut().query::<(&ActorId, &Transform)>();
            q.iter(app.world())
                .next()
                .map_or(0.0, |(_, t)| t.translation.x)
        };
        loop {
            let mut sensors = blockloom_core::sense::Sensors::default();
            sensors.actions.insert(
                "Go".into(),
                blockloom_core::input::ActionSense {
                    held: true,
                    value: 1.0,
                    ..Default::default()
                },
            );
            blockloom_core::sense::publish(sensors);
            guest.update();
            host.update();
            std::thread::sleep(Duration::from_millis(2));
            if x(&mut guest) > 30.0 {
                break;
            }
            assert!(
                Instant::now() < end,
                "the guest's actor never moved: {}",
                x(&mut guest)
            );
        }
        assert!(x(&mut host) > 30.0, "the host moved it first");
        assert_eq!(
            crate::lan::guest_players(host.world().non_send::<Engine>()),
            vec![2]
        );
    }

    /// A host's server running `project` with LAN open, and a guest link to it.
    fn host_with_lan(project: &blockloom_core::project::Project) -> (App, RemoteServer) {
        let (host, _, target) = open_host(project);
        (host, RemoteServer::connect(&target).unwrap())
    }

    /// A host's server running `project` with LAN open, its editor's side,
    /// and what joins it.
    fn open_host(
        project: &blockloom_core::project::Project,
    ) -> (App, std::sync::mpsc::Sender<EditorMessage>, JoinTarget) {
        use crate::engine::Engine;
        use blockloom_core::scene::Mode;
        use std::time::{Duration, Instant};
        blockloom_core::value::register_blockloom_operators();
        let (editor, incoming) = std::sync::mpsc::channel();
        let mut host =
            crate::simulation::headless_server(Engine::new(incoming, Mode::TwoD), Mode::TwoD, 60.0);
        editor
            .send(EditorMessage::Load {
                project: Box::new(project.clone()),
                dir: None,
            })
            .unwrap();
        editor.send(EditorMessage::Start).unwrap();
        host.update();
        host.update();
        editor
            .send(EditorMessage::OpenLan {
                bind: "127.0.0.1:0".into(),
                max_guests: 2,
            })
            .unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        let invite = loop {
            host.update();
            let status = crate::lan::status(host.world().non_send::<Engine>());
            if let Some(invite) = status.invite {
                break invite;
            }
            assert!(Instant::now() < end, "LAN never opened: {:?}", status.error);
        };
        let trusted = invite.rsplit('|').next().unwrap().to_string();
        // Keep the editor's side open for the host's lifetime.
        let keep = editor.clone();
        std::mem::forget(editor);
        (host, keep, JoinTarget { invite, trusted })
    }

    /// A world playing its own game joins a host mid-run, follows it, and
    /// comes back to its own game when the host closes LAN.
    #[test]
    fn a_running_game_joins_a_host_and_comes_back() {
        use crate::engine::Engine;
        use crate::server::{Following, LinkChange, LinkRequest};
        use blockloom_core::project::{Actor, Project};
        use blockloom_core::scene::{Mode, Visual};
        use std::time::{Duration, Instant};

        let mut project = Project::starter("drop-in", Mode::TwoD);
        project.actors = vec![Actor::new(
            "Box",
            Visual::Rect {
                color: "#FFFFFF".to_string(),
                size: [10.0, 10.0],
            },
        )];
        project.multiplayer.enabled = true;
        let (mut host, host_editor, target) = open_host(&project);

        // The guest plays its own copy of the same game first.
        let (editor, incoming) = std::sync::mpsc::channel();
        let mut guest =
            crate::simulation::headless_server(Engine::new(incoming, Mode::TwoD), Mode::TwoD, 60.0);
        crate::server::register_client(&mut guest);
        editor
            .send(EditorMessage::Load {
                project: Box::new(project.clone()),
                dir: None,
            })
            .unwrap();
        editor.send(EditorMessage::Start).unwrap();
        for _ in 0..3 {
            guest.update();
        }
        assert!(guest.world().non_send::<Engine>().authority);
        guest.world_mut().resource_mut::<LinkRequest>().0 = Some(LinkChange::Join(target.invite));
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            guest.update();
            host.update();
            std::thread::sleep(Duration::from_millis(2));
            let seated = guest
                .world()
                .get_resource::<Following>()
                .is_some_and(|f| f.seat() == 1);
            if seated {
                break;
            }
            assert!(Instant::now() < end, "the guest never took a seat");
        }
        assert!(!guest.world().non_send::<Engine>().authority);
        assert!(guest.world().non_send::<Engine>().running);

        host_editor.send(EditorMessage::CloseLan).unwrap();
        loop {
            guest.update();
            host.update();
            std::thread::sleep(Duration::from_millis(2));
            if guest.world().get_resource::<Following>().is_none() {
                break;
            }
            assert!(Instant::now() < end, "the guest never came back");
        }
        guest.update();
        let engine = guest.world().non_send::<Engine>();
        assert!(engine.authority && engine.running);
        drop(editor);
    }

    /// Clones the host made before the guest joined show up on the guest.
    #[test]
    fn a_late_guest_gets_the_clones_it_missed() {
        use crate::engine::{ActorId, Engine};
        use blockloom_core::blocks::{Instruction, InstructionKind, Strand};
        use blockloom_core::project::{Actor, Project};
        use blockloom_core::scene::{Mode, Visual};
        use std::time::{Duration, Instant};

        let mut actor = Actor::new(
            "Coin",
            Visual::Rect {
                color: "#FFFF00".to_string(),
                size: [10.0, 10.0],
            },
        );
        actor.graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(InstructionKind::WhenStarted),
                Instruction::new(InstructionKind::CreateClone {
                    of: "myself".into(),
                }),
            ],
        )];
        let mut project = Project::starter("late", Mode::TwoD);
        project.actors = vec![actor];
        project.multiplayer.enabled = true;
        let (mut host, remote) = host_with_lan(&project);

        let (_e, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project = project;
        let mut guest = crate::simulation::headless(engine, Mode::TwoD, true);
        crate::server::follow(&mut guest, remote);
        let count = |app: &mut App| {
            let mut q = app.world_mut().query::<&ActorId>();
            q.iter(app.world()).count()
        };
        let end = Instant::now() + Duration::from_secs(20);
        while count(&mut guest) < 2 {
            guest.update();
            host.update();
            std::thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < end, "the clone never reached the guest");
        }
        assert_eq!(count(&mut host), 2);
        for _ in 0..30 {
            guest.update();
            host.update();
        }
        assert_eq!(count(&mut guest), 2, "made once");
    }

    /// The LAN co-op template: a guest's Move drives Player 2 on the host,
    /// who appears when the guest joins.
    #[test]
    fn the_co_op_template_gives_a_guest_player_two() {
        use crate::engine::{ActorId, Engine};
        use blockloom_core::scene::Mode;
        use std::time::{Duration, Instant};

        let project =
            blockloom_core::physics::sample::build("lan-co-op", "Co-op", Mode::TwoD).unwrap();
        let two = project
            .actors
            .iter()
            .find(|a| a.name == "Player 2")
            .unwrap()
            .id
            .clone();
        let (mut host, remote) = host_with_lan(&project);
        let (_e, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project = project;
        let mut guest = crate::simulation::headless(engine, Mode::TwoD, true);
        crate::server::follow(&mut guest, remote);
        let pose = |app: &mut App| {
            let mut q = app
                .world_mut()
                .query::<(&ActorId, &Transform, &Visibility)>();
            q.iter(app.world())
                .find(|(id, _, _)| id.0 == two)
                .map(|(_, t, v)| (t.translation.x, *v != Visibility::Hidden))
                .unwrap_or((f32::NAN, false))
        };
        let start = pose(&mut host).0;
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let mut sensors = blockloom_core::sense::Sensors::default();
            sensors.actions.insert(
                "Move".into(),
                blockloom_core::input::ActionSense {
                    held: true,
                    value: 1.0,
                    vector: [1.0, 0.0],
                    ..Default::default()
                },
            );
            blockloom_core::sense::publish(sensors);
            guest.update();
            host.update();
            std::thread::sleep(Duration::from_millis(2));
            let (x, shown) = pose(&mut guest);
            if shown && x > start + 30.0 {
                break;
            }
            assert!(
                Instant::now() < end,
                "player 2 never moved for the guest: {:?} from {start}",
                pose(&mut guest)
            );
        }
        let (x, shown) = pose(&mut host);
        assert!(shown && x > start + 30.0, "the host saw it too: {x}");
    }
}
