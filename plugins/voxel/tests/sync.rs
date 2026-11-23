//! Multiplayer section sync: a guest's copy laid from the host's sections
//! ends up cell for cell what the host has.
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::{native::NativeModule, services::HostServices};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn module() -> NativeModule {
    unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::from([Capability::ProjectStorage]),
            HostServices::new("0.0.1").for_plugin("com.blockworked.voxel"),
        )
    }
    .unwrap()
}

fn call(m: &NativeModule, op: &str, args: Value) -> Value {
    m.call_json(op, &args)
        .unwrap_or_else(|e| panic!("{op}: {e:?}"))
}

fn start(settings: &Value) -> NativeModule {
    let m = module();
    call(
        &m,
        "world.start",
        json!({"resources":[{"type_id":"world","payload":settings}]}),
    );
    m
}

fn get(m: &NativeModule, c: [i32; 3]) -> (i64, Value) {
    let cell = json!({"x":c[0],"y":c[1],"z":c[2]});
    let answer = call(m, "get", cell);
    (
        answer["material"].as_i64().unwrap(),
        answer["shape"].clone(),
    )
}

/// Every cell of both worlds agrees.
fn same(host: &NativeModule, guest: &NativeModule, size: [i32; 3]) {
    for x in 0..size[0] {
        for y in 0..size[1] {
            for z in 0..size[2] {
                assert_eq!(
                    get(host, [x, y, z]),
                    get(guest, [x, y, z]),
                    "at {x} {y} {z}"
                );
            }
        }
    }
}

fn sync(host: &NativeModule, guest: &NativeModule, since: u64, generation: Option<u64>) -> Value {
    let mut args = json!({"since":since});
    if let Some(g) = generation {
        args["generation"] = json!(g);
    }
    let sent = call(host, "sync.sections", args);
    let applied = call(
        guest,
        "sync.apply",
        json!({"identity":sent["identity"],"sections":sent["sections"]}),
    );
    assert!(
        !applied["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "error"),
        "{applied}"
    );
    sent
}

#[test]
fn a_guest_copy_follows_the_hosts_edits_and_regeneration() {
    let settings = json!({"preset":"island","seed":3,"size":[48,24,40],"solid":false});
    let host = start(&settings);
    let guest = start(&settings);
    // Nothing edited yet: the guest's own generation is already the host's.
    let first = sync(&host, &guest, 0, None);
    assert!(first["sections"].as_array().unwrap().is_empty());

    call(
        &host,
        "sphere",
        json!({"x":20,"y":10,"z":20,"radius":5,"material":"air"}),
    );
    call(
        &host,
        "fill",
        json!({"x1":0,"y1":20,"z1":0,"x2":40,"y2":20,"z2":3,"material":"stone"}),
    );
    call(
        &host,
        "shape",
        json!({"x":2,"y":20,"z":1,"shape":"stair east"}),
    );
    let second = sync(&host, &guest, 0, first["generation"].as_u64());
    let sent = second["sections"].as_array().unwrap().len();
    assert!(sent > 0 && sent < 4, "only edited sections travel ({sent})");
    same(&host, &guest, [48, 24, 40]);

    // A later sync sends only what changed since.
    call(&host, "set", json!({"x":45,"y":1,"z":38,"material":"glow"}));
    let rev = second["revision"].as_u64().unwrap();
    let third = sync(&host, &guest, rev, second["generation"].as_u64());
    assert_eq!(third["sections"].as_array().unwrap().len(), 1);
    same(&host, &guest, [48, 24, 40]);

    // Regenerating drops the edits on both.
    call(&host, "generate", json!({"preset":"caves","seed":9}));
    call(&host, "set", json!({"x":1,"y":1,"z":1,"material":"sand"}));
    let fourth = sync(
        &host,
        &guest,
        third["revision"].as_u64().unwrap(),
        third["generation"].as_u64(),
    );
    assert_ne!(fourth["generation"], third["generation"]);
    same(&host, &guest, [48, 24, 40]);

    // A copy made for other settings refuses the host's sections.
    let other = start(&json!({"preset":"island","seed":4,"size":[48,24,40]}));
    let sent = call(&host, "sync.sections", json!({}));
    assert!(
        other
            .call_json(
                "sync.apply",
                &json!({"identity":sent["identity"],"sections":[]})
            )
            .is_err()
    );
}

#[test]
fn a_streamed_world_sends_only_its_edits() {
    let settings =
        json!({"preset":"island","seed":5,"size":[64,32,64],"streamed":true,"solid":false});
    let host = start(&settings);
    let guest = start(&settings);
    call(
        &host,
        "fill",
        json!({"x1":3,"y1":3,"z1":3,"x2":6,"y2":6,"z2":6,"material":"air"}),
    );
    call(
        &host,
        "set",
        json!({"x":40,"y":30,"z":40,"material":"stone"}),
    );
    let sent = sync(&host, &guest, 0, None);
    assert_eq!(sent["sections"].as_array().unwrap().len(), 2);
    // A generated cell travels as "keep", not as its block.
    let runs = &sent["sections"][0]["data"]["runs"];
    assert!(runs.as_array().unwrap().iter().any(|r| r[1] == 65535));
    same(&host, &guest, [64, 32, 64]);
}
