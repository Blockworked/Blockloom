//! A native spectator client for inspecting authoritative LAN replicas.
use blockloom_net::game::{Invite, LanClient};
use blockloom_net::{ClientOptions, Fingerprint};
use std::time::Duration;
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let invite = Invite::decode(
        &args
            .next()
            .ok_or("Usage: blockloom-lan INVITE TRUSTED_BUILD_HASH")?,
    )?;
    let trusted: Fingerprint = args
        .next()
        .ok_or("Supply the installed game's trusted build hash")?
        .parse()
        .map_err(|e: blockloom_net::NetError| e.to_string())?;
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let mut client = LanClient::connect(invite, trusted.0, ClientOptions::default())?;
    loop {
        if client.poll()?
            && let Some(s) = client.state()
        {
            println!(
                "epoch={} tick={} scene={:?} dimension={} paused={} game_ns={}",
                s.epoch, s.tick, s.scene, s.dimension, s.paused, s.game_ns
            );
            for a in s.actors.values() {
                println!(
                    "  {} template={} pos={:?} visible={}",
                    a.id,
                    a.template,
                    &a.pose[..3],
                    a.visible
                );
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
