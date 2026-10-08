use blockloom_net::game::{ActorState, Invite, LanClient, LanHost, Snapshot, build_hash};
use blockloom_net::{ClientOptions, Impairment, ServerOptions};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn sample() -> Snapshot {
    let a = ActorState {
        id: "player".into(),
        template: "player".into(),
        pose: [0.; 16],
        visible: true,
        appearance: b"{}".to_vec(),
    };
    Snapshot {
        epoch: 1,
        tick: 60,
        scene: "First".into(),
        actors: BTreeMap::from([(a.id.clone(), a)]),
        ..Default::default()
    }
}
fn until(host: &mut LanHost, client: &mut LanClient, done: impl Fn(&LanClient) -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        host.poll();
        client.poll().unwrap();
        if done(client) {
            return;
        }
        assert!(Instant::now() < end, "LAN client stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn late_join_move_spawn_delete_scene_and_close_under_loss() {
    let initial = sample();
    let build = build_hash(b"trusted fixture");
    let impairment = Impairment {
        loss: 0.05,
        delay: Duration::from_millis(10),
        jitter: Duration::from_millis(5),
        ..Default::default()
    };
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        initial.clone(),
        ServerOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    let invite = Invite::decode(&host.invite().encode()).unwrap();
    let mut c = LanClient::connect(
        invite,
        build,
        ClientOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&initial));
    let mut next = initial.clone();
    next.tick += 1;
    next.actors.get_mut("player").unwrap().pose[0] = 12.;
    let mut clone = next.actors["player"].clone();
    clone.id = "clone".into();
    next.actors.insert(clone.id.clone(), clone);
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    next.tick += 1;
    next.actors.remove("player");
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    next.epoch = 2;
    next.tick = 0;
    next.dimension = 1;
    next.scene = "Next".into();
    next.actors.clear();
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    assert_eq!(host.guests().len(), 1);
    host.close();
    assert!(host.guests().is_empty());
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        host.poll();
        if c.poll().is_err() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn wrong_admission_and_build_cannot_receive_state() {
    let build = build_hash(b"fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        ServerOptions::default(),
    )
    .unwrap();
    assert!(LanClient::connect(host.invite().clone(), [0; 32], ClientOptions::default()).is_err());
    let mut invite = host.invite().clone();
    invite.token = [0; 32];
    let mut c = LanClient::connect(invite, build, ClientOptions::default()).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        host.poll();
        if c.poll().is_err() {
            break;
        }
        assert!(c.state().is_none());
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(host.guests().is_empty());
}
#[test]
fn bind_failure_and_interface_validation_are_explicit() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    assert!(
        LanHost::open(
            socket.local_addr().unwrap(),
            [0; 32],
            sample(),
            ServerOptions::default()
        )
        .is_err()
    );
    assert!(
        LanHost::open(
            "0.0.0.0:0".parse().unwrap(),
            [0; 32],
            sample(),
            ServerOptions::default()
        )
        .is_err()
    );
}
#[test]
fn slow_guest_coalesces_state_without_unbounded_tail() {
    let build = build_hash(b"fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        ServerOptions::default(),
    )
    .unwrap();
    let mut c = LanClient::connect(host.invite().clone(), build, ClientOptions::default()).unwrap();
    until(&mut host, &mut c, |c| c.state().is_some());
    let mut next = sample();
    for i in 61..500 {
        next.tick = i;
        next.actors.get_mut("player").unwrap().pose[0] = i as f32;
        host.publish(next.clone()).unwrap();
        host.poll();
    }
    until(&mut host, &mut c, |c| c.state() == Some(&next));
}
