//! The integrated server: a world's simulation half on its own thread, with
//! every player, the host included, as a client of it. A client sends its
//! seat's input; the server answers each fixed tick with that tick's effects
//! and the settled pose of every actor. Nothing here draws.

use crate::engine::{ActorId, Engine, PendingEffects, PhysicsPose};
use crate::simulation::SimStep;
use bevy::prelude::*;
use blockloom_core::project::Project;
use blockloom_core::scene::Mode;
use blockloom_core::sense::Sensors;
use blockloom_core::vm::{Effect, Event};
use blockloom_protocol::EditorMessage;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

/// What a client says about its own seat for the next tick: its input as a
/// sensor snapshot (actors are the server's and are ignored) and the edges
/// it saw, such as a key going down.
#[derive(Clone, Debug, Default)]
pub struct ClientInput {
    pub sensors: Sensors,
    pub events: Vec<Event>,
}

pub enum ToServer {
    Input(Box<ClientInput>),
}

/// One actor as a tick left it.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorPose {
    pub id: String,
    pub transform: Transform,
    pub visible: bool,
}

/// What one fixed tick did, in order.
#[derive(Clone, Debug, Default)]
pub struct ServerTick {
    pub tick: u64,
    pub effects: Vec<Effect>,
    pub poses: Vec<ActorPose>,
}

/// A running integrated server. Dropping it stops the thread.
pub struct IntegratedServer {
    to: Sender<ToServer>,
    editor: Sender<EditorMessage>,
    from: Receiver<ServerTick>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl IntegratedServer {
    /// A server for a `mode` world, waiting for the editor's messages.
    pub fn spawn(mode: Mode, rate: f32) -> Self {
        let (to, inbox) = channel();
        let (outbox, from) = channel();
        let (editor, incoming) = channel();
        let thread = std::thread::Builder::new()
            .name("blockloom-server".into())
            .spawn(move || run(mode, rate, incoming, inbox, outbox))
            .expect("spawn the integrated server");
        Self {
            to,
            editor,
            from,
            thread: Some(thread),
        }
    }

    /// A server already running `project`, past its green flag.
    #[allow(dead_code)]
    pub fn start(project: Project, mode: Mode) -> Self {
        let server = Self::spawn(mode, project.world.fixed_rate);
        server.editor(EditorMessage::Load {
            project: Box::new(project),
            dir: None,
        });
        server.editor(EditorMessage::Start);
        server
    }

    /// Hands the server what the editor told this world.
    pub fn editor(&self, message: EditorMessage) {
        let _ = self.editor.send(message);
    }

    pub fn send(&self, input: ClientInput) {
        let _ = self.to.send(ToServer::Input(Box::new(input)));
    }

    /// Every tick that finished since the last call, oldest first.
    pub fn ticks(&self) -> Vec<ServerTick> {
        self.from.try_iter().collect()
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for IntegratedServer {
    fn drop(&mut self) {
        // The server stops when its inbox closes.
        let (dead, _) = channel();
        self.to = dead;
        let (dead, _) = channel();
        self.editor = dead;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Resource)]
struct Link {
    inbox: std::sync::Mutex<Receiver<ToServer>>,
    outbox: Sender<ServerTick>,
    closed: bool,
    tick: u64,
    effects: Vec<Effect>,
}

fn run(
    mode: Mode,
    rate: f32,
    incoming: Receiver<EditorMessage>,
    inbox: Receiver<ToServer>,
    outbox: Sender<ServerTick>,
) {
    blockloom_core::value::register_blockloom_operators();
    crate::bridge::mark_server_thread();
    let engine = Engine::new(incoming, mode);
    let rate = if rate.is_finite() {
        f64::from(rate).clamp(1.0, 1000.0)
    } else {
        60.0
    };
    let mut app = crate::simulation::headless_server(engine, mode, rate);
    app.insert_resource(Link {
        inbox: std::sync::Mutex::new(inbox),
        outbox,
        closed: false,
        tick: 0,
        effects: Vec::new(),
    });
    register(&mut app);
    let step = Duration::from_secs_f64(1.0 / rate);
    let mut next = Instant::now();
    loop {
        app.update();
        if app.world().resource::<Link>().closed || app.should_exit().is_some() {
            return;
        }
        next += step;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > step * 10 {
            // Far behind (a debugger, a stall): drop the debt rather than race.
            next = now;
        }
    }
}

fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (
            receive.before(SimStep::Sample),
            keep_effects
                .after(SimStep::Animation)
                .before(SimStep::ClearEffects),
        )
            .in_set(crate::world::SimulationSet),
    )
    .add_systems(
        FixedPostUpdate,
        send_tick
            .after(crate::dim2::record_poses)
            .after(crate::dim3::record_poses),
    );
}

/// Drains clients' input: the newest snapshot stands for the seat, and every
/// edge is fired once.
fn receive(mut link: ResMut<Link>, mut engine: NonSendMut<Engine>) {
    let mut newest = None;
    loop {
        let got = link.inbox.lock().expect("server inbox").try_recv();
        match got {
            Ok(ToServer::Input(input)) => {
                for event in input.events.iter().cloned() {
                    engine.fire(event);
                }
                newest = Some(input.sensors);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                link.closed = true;
                break;
            }
        }
    }
    if let Some(mut sensors) = newest {
        // Actors, time and the run's own state stay the server's.
        blockloom_core::sense::read(|s| {
            sensors.time = s.time;
            sensors.wall_time = s.wall_time;
            sensors.paused = s.paused;
            sensors.current_scene = s.current_scene.clone();
            sensors.scene_names = s.scene_names.clone();
            sensors.current_save_slot = s.current_save_slot.clone();
            sensors.save_slots = s.save_slots.clone();
            sensors.language = s.language.clone();
        });
        sensors.actors = blockloom_core::sense::take_actors();
        blockloom_core::sense::publish(sensors);
    }
}

fn keep_effects(mut link: ResMut<Link>, effects: Res<PendingEffects>) {
    link.effects.extend(effects.0.iter().cloned());
}

fn send_tick(
    mut link: ResMut<Link>,
    actors: Query<(&ActorId, &Transform, Option<&PhysicsPose>, &Visibility)>,
) {
    link.tick += 1;
    let mut poses: Vec<ActorPose> = actors
        .iter()
        .map(|(id, t, pose, visible)| ActorPose {
            id: id.0.clone(),
            transform: pose.map_or(*t, |p| p.0),
            visible: *visible != Visibility::Hidden,
        })
        .collect();
    poses.sort_by(|a, b| a.id.cmp(&b.id));
    let tick = ServerTick {
        tick: link.tick,
        effects: std::mem::take(&mut link.effects),
        poses,
    };
    if link.outbox.send(tick).is_err() {
        link.closed = true;
    }
}

/// A world following an integrated server: it applies each tick's effects
/// the way the server did, poses every actor where the server left it, and
/// sends its own input back.
#[derive(Resource)]
pub struct Following {
    server: std::sync::Mutex<IntegratedServer>,
    /// The newest tick's poses, applied after this world's own step.
    poses: Vec<ActorPose>,
    pub last_tick: u64,
}

impl Following {
    pub fn editor(&self, message: EditorMessage) {
        self.server.lock().expect("server link").editor(message);
    }

    pub fn new(server: IntegratedServer) -> Self {
        Self {
            server: std::sync::Mutex::new(server),
            poses: Vec::new(),
            last_tick: 0,
        }
    }
}

/// Whether this world runs its game on an integrated server. Opt-in with
/// `BLOCKLOOM_SERVER=integrated` until single-player is proven on it.
pub fn integrated() -> bool {
    std::env::var("BLOCKLOOM_SERVER").is_ok_and(|v| v == "integrated")
}

/// Makes `app`'s world a client of `server`.
pub fn follow(app: &mut App, server: IntegratedServer) {
    app.world_mut().non_send_mut::<Engine>().authority = false;
    // The server records the run; the client's poses only follow it.
    app.world_mut().remove_resource::<crate::record::Recorder>();
    app.insert_resource(Following::new(server))
        .add_systems(
            FixedUpdate,
            take_ticks
                .after(SimStep::Sample)
                .before(SimStep::PluginInput),
        )
        .add_systems(
            FixedPostUpdate,
            pose_from_server
                .after(crate::dim2::record_poses)
                .after(crate::dim3::record_poses),
        )
        .add_systems(Last, send_input);
}

/// Every tick that arrived since the last step: their effects in order, and
/// the newest poses.
fn take_ticks(
    mut following: ResMut<Following>,
    mut effects: ResMut<PendingEffects>,
    engine: NonSend<Engine>,
) {
    let ticks = following.server.lock().expect("server link").ticks();
    // A world that isn't running draws the document, not the server's run.
    if !engine.running {
        following.poses.clear();
        return;
    }
    for tick in ticks {
        effects.0.extend(tick.effects);
        following.last_tick = tick.tick;
        following.poses = tick.poses;
    }
}

fn pose_from_server(
    following: Res<Following>,
    engine: NonSend<Engine>,
    mut actors: Query<(&mut Transform, Option<&mut PhysicsPose>, &mut Visibility)>,
) {
    if !engine.running {
        return;
    }
    for pose in &following.poses {
        let Some(entity) = engine.entities.get(&pose.id) else {
            continue;
        };
        let Ok((mut transform, physics, mut visible)) = actors.get_mut(*entity) else {
            continue;
        };
        *transform = pose.transform;
        if let Some(mut physics) = physics {
            physics.0 = pose.transform;
        }
        let want = if pose.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visible != want {
            *visible = want;
        }
    }
}

/// Once a frame: this world's input and the events it saw.
fn send_input(
    following: Res<Following>,
    mut engine: NonSendMut<Engine>,
    mut exit: MessageWriter<AppExit>,
) {
    // A client has nothing to show once its server is gone (a recorder or
    // profiler finished there, or it stopped).
    if !following.server.lock().expect("server link").is_running() {
        exit.write(AppExit::Success);
        return;
    }
    if std::mem::take(&mut engine.start_server) {
        following.editor(EditorMessage::Start);
    }
    let events = std::mem::take(&mut engine.forwarded);
    let mut sensors = blockloom_core::sense::read(|s| s.clone());
    sensors.actors.clear();
    following
        .server
        .lock()
        .expect("server link")
        .send(ClientInput { sensors, events });
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::{Instruction, InstructionKind, Strand};
    use blockloom_core::project::Actor;
    use blockloom_core::scene::Visual;
    use blockloom_core::value::{Op, Value};

    /// One actor that moves only while space is held.
    fn walker(mode: Mode) -> Project {
        let mut actor = Actor::new(
            "Player",
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
                        condition: Value::op(Op::from_name("KeyDown"), vec![Value::text("space")]),
                        body: vec![Instruction::new(InstructionKind::Move {
                            steps: Value::number(5.0),
                        })],
                    })],
                }),
            ],
        )];
        let mut project = Project::starter("server", mode);
        project.actors = vec![actor];
        project
    }

    fn x_after(server: &IntegratedServer, ticks: usize, held: bool) -> (f32, Vec<ServerTick>) {
        let mut got = Vec::new();
        let end = Instant::now() + Duration::from_secs(20);
        while got.len() < ticks {
            let mut sensors = Sensors::default();
            if held {
                sensors.keys.insert("space".to_string());
            }
            server.send(ClientInput {
                sensors,
                events: Vec::new(),
            });
            got.extend(server.ticks());
            assert!(Instant::now() < end, "the server stopped ticking");
            std::thread::sleep(Duration::from_millis(2));
        }
        let x = got
            .last()
            .and_then(|t| t.poses.first())
            .map_or(f32::NAN, |p| p.transform.translation.x);
        (x, got)
    }

    #[test]
    fn the_server_moves_an_actor_only_while_its_client_holds_the_key() {
        let server = IntegratedServer::start(walker(Mode::TwoD), Mode::TwoD);
        let (still, _) = x_after(&server, 10, false);
        let (moved, ticks) = x_after(&server, 20, true);
        assert!(moved > still + 20.0, "{still} -> {moved}");
        assert!(
            ticks
                .iter()
                .flat_map(|t| &t.effects)
                .any(|e| matches!(e, Effect::Move { .. })),
            "the moves reach the client as effects"
        );
        let numbers: Vec<u64> = ticks.iter().map(|t| t.tick).collect();
        assert!(numbers.windows(2).all(|w| w[1] == w[0] + 1), "{numbers:?}");
        drop(server);
    }

    /// A client world built from the same document, running nothing itself:
    /// its actor stands where the server put it, and its key press is what
    /// moved it there.
    #[test]
    fn a_client_world_follows_the_server_it_sends_input_to() {
        let project = walker(Mode::TwoD);
        let server = IntegratedServer::start(project.clone(), Mode::TwoD);
        blockloom_core::value::register_blockloom_operators();
        let (_editor, incoming) = channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project = project;
        let mut client = crate::simulation::headless(engine, Mode::TwoD, true);
        follow(&mut client, server);
        let x = |app: &mut App| {
            let mut q = app.world_mut().query::<(&ActorId, &Transform)>();
            q.iter(app.world()).next().map(|(_, t)| t.translation.x)
        };
        let end = Instant::now() + Duration::from_secs(20);
        let mut start = None;
        loop {
            let mut sensors = Sensors::default();
            sensors.keys.insert("space".to_string());
            blockloom_core::sense::publish(sensors);
            client.update();
            std::thread::sleep(Duration::from_millis(5));
            let now = x(&mut client);
            if client.world().resource::<Following>().last_tick > 0 {
                let first = *start.get_or_insert(now.unwrap());
                if now.unwrap() > first + 30.0 {
                    break;
                }
            }
            assert!(Instant::now() < end, "the client never moved: {now:?}");
        }
        // The client's own VM never ran a step.
        let engine = client.world().non_send::<Engine>();
        assert!(!engine.authority);
    }

    /// A key the client saw go down starts the server's `when key pressed`
    /// strand once, and the client never runs it itself.
    #[test]
    fn a_key_the_client_saw_starts_the_servers_strand_once() {
        let mut project = walker(Mode::TwoD);
        project.actors[0].graph.strands = vec![Strand::with_instructions(
            0,
            0,
            vec![
                Instruction::new(InstructionKind::WhenKeyPressed {
                    key: "space".into(),
                }),
                Instruction::new(InstructionKind::Move {
                    steps: Value::number(40.0),
                }),
            ],
        )];
        let server = IntegratedServer::start(project.clone(), Mode::TwoD);
        blockloom_core::value::register_blockloom_operators();
        let (_editor, incoming) = channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.project = project;
        let mut client = crate::simulation::headless(engine, Mode::TwoD, true);
        follow(&mut client, server);
        client
            .world_mut()
            .non_send_mut::<Engine>()
            .fire(Event::Key("space".into()));
        let x = |app: &mut App| {
            let mut q = app.world_mut().query::<(&ActorId, &Transform)>();
            q.iter(app.world())
                .next()
                .map_or(0.0, |(_, t)| t.translation.x)
        };
        let end = Instant::now() + Duration::from_secs(20);
        while x(&mut client) < 39.0 {
            client.update();
            std::thread::sleep(Duration::from_millis(5));
            assert!(Instant::now() < end, "the press never moved the actor");
        }
        for _ in 0..30 {
            client.update();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            (x(&mut client) - 40.0).abs() < 0.01,
            "moved once: {}",
            x(&mut client)
        );
    }
}
