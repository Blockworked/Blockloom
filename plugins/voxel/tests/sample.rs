//! The mini-survival sample: its own assets, played through the native plugin.
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::{native::NativeModule, services::HostServices};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf, time::Instant};

const ID: &str = "com.blockworked.voxel";

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples/mini-survival")
}

/// The world resource the setup script writes, read from the sample.
fn settings() -> Value {
    let text = std::fs::read_to_string(sample().join("world.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn module() -> NativeModule {
    unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::from([Capability::ProjectStorage]),
            HostServices::new("0.0.1")
                .with_assets(sample())
                .for_plugin(ID),
        )
    }
    .unwrap()
}

fn call(m: &NativeModule, op: &str, args: Value) -> Value {
    m.call_json(op, &args)
        .unwrap_or_else(|e| panic!("{op}: {e:?}"))
}

fn at(m: &NativeModule, cell: [i32; 3]) -> i64 {
    call(m, "get", json!({"x":cell[0],"y":cell[1],"z":cell[2]}))["material"]
        .as_i64()
        .unwrap()
}

fn effects(answer: &Value) -> &Vec<Value> {
    answer["effects"].as_array().unwrap()
}

fn id_of(m: &NativeModule, name: &str) -> i64 {
    call(m, "blocks", json!({}))["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no block {name}"))["id"]
        .as_i64()
        .unwrap()
}

#[test]
fn the_mini_survival_sample_mines_builds_stores_flows_and_turns_to_night() {
    let m = module();
    let clock = Instant::now();
    let started = call(
        &m,
        "world.start",
        json!({"resources":[{"type_id":"world","payload":settings()}]}),
    );
    let start_ms = clock.elapsed().as_millis();
    let errors: Vec<_> = effects(&started)
        .iter()
        .filter(|e| e["effect"] == "error")
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let meshes = effects(&started)
        .iter()
        .filter(|e| e["effect"] == "mesh")
        .count();
    assert!(meshes > 0, "the world draws");

    let size = settings()["size"].clone();
    let [sx, sz] = [
        size[0].as_i64().unwrap() as i32,
        size[2].as_i64().unwrap() as i32,
    ];
    let water = id_of(&m, "water");
    let grass = id_of(&m, "grass");
    let wheat = id_of(&m, "wheat");
    let height = |x, z| {
        call(&m, "height", json!({"x":x,"z":z}))["value"]
            .as_i64()
            .unwrap() as i32
    };

    // The graph makes sea, meadow and wheat.
    let mut found_water = false;
    let mut land = None;
    let mut crops = Vec::new();
    for x in (0..sx).step_by(2) {
        for z in (0..sz).step_by(2) {
            let top = height(x, z);
            let block = at(&m, [x, top, z]);
            found_water |= block == water;
            if block == grass && land.is_none() && x > 4 && z > 4 && x < sx - 5 && z < sz - 5 {
                land = Some([x, top, z]);
            }
            if (wheat..wheat + 4).contains(&block) {
                crops.push([x, top, z]);
            }
        }
    }
    assert!(found_water, "the sea is there");
    let [x, top, z] = land.expect("some meadow");

    // Mine the grass under us, then build a plank on the stone below it.
    let eye = |dy: i32| json!({"x":x as f64 + 0.5,"y":(top + dy) as f64 + 0.5,"z":z as f64 + 0.5,"dx":0,"dy":-1,"dz":0,"reach":6});
    let mined = call(&m, "break", eye(3));
    assert_eq!(mined["cell"], json!([x, top, z]));
    assert_eq!(at(&m, [x, top, z]), 0);
    let mut args = eye(3);
    args["material"] = json!("planks");
    let built = call(&m, "place", args);
    assert_eq!(
        built["cell"],
        json!([x, top, z]),
        "built back where it was mined"
    );
    assert_eq!(at(&m, [x, top, z]), id_of(&m, "planks"));

    // A chest keeps what goes in and says what it held when broken.
    let chest = [x, top + 1, z];
    call(
        &m,
        "set",
        json!({"x":chest[0],"y":chest[1],"z":chest[2],"material":"chest"}),
    );
    let mut items = json!({"x":chest[0],"y":chest[1],"z":chest[2],"item":"wheat","count":5});
    assert_eq!(call(&m, "add_items", items.clone())["value"], 0);
    items["count"] = json!(2);
    assert_eq!(call(&m, "take_items", items.clone())["value"], 2);
    assert_eq!(call(&m, "item_count", items)["value"], 3);
    let broken = call(
        &m,
        "set",
        json!({"x":chest[0],"y":chest[1],"z":chest[2],"material":"air"}),
    );
    let event = effects(&broken)
        .iter()
        .find(|e| e["effect"] == "event")
        .expect("a broken chest says so");
    assert_eq!(
        event["args"],
        json!(["chest", chest[0], chest[1], chest[2], "wheat x3"])
    );

    // Water poured on land flows out over it.
    let pour = [x, top + 1, z];
    call(
        &m,
        "set",
        json!({"x":pour[0],"y":pour[1],"z":pour[2],"material":"water"}),
    );
    let clock = Instant::now();
    let ticks = 60;
    for tick in 0..ticks {
        call(&m, "hook.tick", json!({"tick":tick}));
    }
    let tick_ms = clock.elapsed().as_millis() as f64 / f64::from(ticks);
    let spread = (-3..=3)
        .flat_map(|dx| (-3..=3).map(move |dz| [x + dx, top + 1, z + dz]))
        .filter(|c| (water..water + 8).contains(&at(&m, *c)))
        .count();
    assert!(spread > 4, "water spread to {spread} cells");

    // Random ticks grow the wheat the graph planted.
    if let Some(crop) = crops.first() {
        let before = at(&m, *crop);
        for tick in 0..400 {
            call(&m, "hook.tick", json!({"tick":ticks + tick}));
            if at(&m, *crop) != before {
                break;
            }
        }
        assert_ne!(at(&m, *crop), before, "wheat at {crop:?} grew");
    }

    // Night relights the world; asking again for the same level does nothing.
    let clock = Instant::now();
    let night = call(&m, "daylight", json!({"level":0.1}));
    let mut night_ms = clock.elapsed().as_millis();
    assert!(effects(&night).iter().any(|e| e["effect"] == "mesh"));
    let mut left = night["relighting"].as_u64().unwrap();
    assert!(left > 0, "the rest relights a chunk a frame");
    while left > 0 {
        let clock = Instant::now();
        call(&m, "hook.stream", json!({}));
        night_ms = night_ms.max(clock.elapsed().as_millis());
        let now = call(&m, "count", json!({}))["relighting"].as_u64().unwrap();
        assert!(now < left);
        left = now;
    }
    let again = call(&m, "daylight", json!({"level":0.1}));
    assert!(!effects(&again).iter().any(|e| e["effect"] == "mesh"));

    eprintln!(
        "mini-survival: start {start_ms} ms, {meshes} meshes, {tick_ms:.2} ms a tick, nightfall {night_ms} ms a frame"
    );
    // The budget: a tick fits a 60 Hz frame and the world is up in 2 s.
    // Debug builds are far slower, so they only catch something gone badly wrong.
    let slack = if cfg!(debug_assertions) { 40 } else { 1 };
    assert!(tick_ms < 16.0 * slack as f64, "{tick_ms} ms a tick");
    assert!(start_ms < 2_000 * slack, "{start_ms} ms to start");
    assert!(night_ms < 16 * slack, "{night_ms} ms to relight a chunk");
}
