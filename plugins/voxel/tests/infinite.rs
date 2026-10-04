//! Infinite worlds: noise, biomes, features, the block registry, chunk
//! stepped render distance, far LOD and budgeted concurrent generation.
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::{
    native::NativeModule,
    services::HostServices,
    storage::{BlobStore, MemoryStore},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

const ID: &str = "com.blockworked.voxel";

fn module(store: Arc<dyn BlobStore>) -> NativeModule {
    unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::from([Capability::ProjectStorage, Capability::GpuCompute]),
            HostServices::new("0.0.1")
                .with_save_store(store)
                .for_plugin(ID),
        )
    }
    .unwrap()
}

fn start(module: &NativeModule, payload: Value) -> Value {
    module
        .call_json(
            "world.start",
            &json!({"resources":[{"type_id":"world","payload":payload}]}),
        )
        .unwrap()
}

fn infinite_payload() -> Value {
    json!({
        "preset": "infinite",
        "streamed": true,
        "size": [512, 64, 512],
        "origin": [-256, 0, -256],
        "seed": 7,
        "stream_radius": 2,
        "vertical_radius": 1,
        "max_pages": 32,
        "pages_per_tick": 2,
        "chunk_workers": 4,
        "render_distance": 256,
        "distant_chunks": 128,
        "pregen_distant": true,
    })
}

#[test]
fn infinite_ground_is_canonical_and_named() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    let answer = start(&m, infinite_payload());
    assert!(
        answer["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["effect"] != "error"),
        "{answer}"
    );
    // The same column reads the same twice, whatever the paging order.
    let a = m
        .call_json("get", &json!({"x": 4, "y": 10, "z": -3}))
        .unwrap();
    let b = m
        .call_json("get", &json!({"x": 4, "y": 10, "z": -3}))
        .unwrap();
    assert_eq!(a, b);
    // Deep stone sits under the surface somewhere near the origin.
    let height = m.call_json("height", &json!({"x": 0, "z": 0})).unwrap();
    let top = height["value"].as_i64().unwrap() as i32;
    assert!(top >= 2, "{height}");
    let deep = m
        .call_json("get", &json!({"x": 0, "y": top - 5, "z": 0}))
        .unwrap();
    assert_eq!(deep["material"], 1, "{deep}");
    let count = m.call_json("count", &json!({})).unwrap();
    assert_eq!(count["preset"], "infinite");
    assert_eq!(count["render_distance"], 256);
    assert_eq!(count["distant_chunks"], 128);
    assert_eq!(count["pregen_distant"], true);
}

#[test]
fn render_distance_steps_by_chunk_and_infinite_needs_streaming() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    // Not streamed: refused.
    assert!(
        m.call_json(
            "world.start",
            &json!({"resources":[{"type_id":"world","payload":{"preset":"infinite","streamed":false}}]}),
        )
        .is_err()
    );
    // Not a multiple of 32: refused at start.
    assert!(
        m.call_json(
            "world.start",
            &json!({"resources":[{"type_id":"world","payload":{"preset":"infinite","streamed":true,"render_distance":100}}]}),
        )
        .is_err()
    );
    // Zero distant chunks is allowed (near field only).
    let store2: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m2 = module(store2);
    let mut payload = infinite_payload();
    payload["distant_chunks"] = json!(0);
    payload["pregen_distant"] = json!(false);
    let answer = start(&m2, payload);
    assert!(
        answer["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["effect"] != "error"),
        "{answer}"
    );
}

#[test]
fn block_registry_lists_ids_names_and_textures() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    let mut payload = infinite_payload();
    payload["blocks"] = json!([
        r##"{"name":"mossy_stone","id":8,"color":"#6a7a5a","textures":{"all":"assets/textures/mossy_stone.png"},"model":"cube"}"##,
    ]);
    start(&m, payload);
    let listed = m.call_json("blocks", &json!({})).unwrap();
    let blocks = listed["blocks"].as_array().unwrap();
    assert!(blocks.len() >= 8, "{listed}");
    let mossy = blocks.iter().find(|b| b["name"] == "mossy_stone").unwrap();
    assert_eq!(mossy["id"], 8);
    // Unknown block names are refused at start.
    let store2: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m2 = module(store2);
    let mut bad = infinite_payload();
    bad["biomes"] = json!([
        "biome nowhere temp 0.0 1.0 hum 0.0 1.0 surface unobtanium sub dirt hills 1 trees 0 rocks 0 ponds 0"
    ]);
    assert!(
        m2.call_json(
            "world.start",
            &json!({"resources":[{"type_id":"world","payload":bad}]}),
        )
        .is_err()
    );
}

#[test]
fn live_render_and_distant_changes_apply() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    start(&m, infinite_payload());
    let answer = m.call_json("render", &json!({"distance": 128.0})).unwrap();
    assert_eq!(answer["render_distance"], 128);
    assert!(m.call_json("render", &json!({"distance": 100.0})).is_err());
    let answer = m
        .call_json("distant", &json!({"chunks": 64.0, "pregen": 1.0}))
        .unwrap();
    assert_eq!(answer["distant_chunks"], 64);
    assert!(m.call_json("distant", &json!({"chunks": 999.0})).is_err());
}

#[test]
fn textured_faces_reach_their_meshes_with_uvs() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    let mut payload = infinite_payload();
    payload["size"] = json!([64, 32, 64]);
    payload["origin"] = json!([0, 0, 0]);
    payload["blocks"] = json!([
        r##"{"name":"sod","id":8,"textures":{"top":"top.png","side":"side.png","bottom":"bottom.png"}}"##,
    ]);
    start(&m, payload);
    let listed = m.call_json("blocks", &json!({})).unwrap();
    assert!(
        listed["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["name"] == "sod"),
        "{listed}"
    );
    // Clear air around a cell so every face of the block shows.
    m.call_json(
        "fill",
        &json!({"x1": 2, "y1": 28, "z1": 2, "x2": 6, "y2": 31, "z2": 6, "material": "air"}),
    )
    .unwrap();
    let answer = m
        .call_json("set", &json!({"x": 4, "y": 30, "z": 4, "material": "sod"}))
        .unwrap();
    let check = m
        .call_json("get", &json!({"x": 4, "y": 30, "z": 4}))
        .unwrap();
    assert_eq!(check["material"], 8, "{check}");
    let effects = answer["effects"].as_array().unwrap();
    let textured: Vec<_> = effects
        .iter()
        .filter(|e| e["effect"] == "mesh" && e.get("texture").and_then(|t| t.as_str()).is_some())
        .collect();
    assert!(!textured.is_empty(), "{answer}");
    for mesh in textured {
        let verts = mesh["positions"].as_array().unwrap().len() / 3;
        assert_eq!(mesh["uvs"].as_array().unwrap().len(), verts * 2);
        assert!(["top.png", "side.png", "bottom.png"].contains(&mesh["texture"].as_str().unwrap()));
    }
}

#[test]
fn concurrent_pages_stay_budgeted_and_nearest_first() {
    let store: Arc<dyn BlobStore> = Arc::new(MemoryStore::new());
    let m = module(store);
    let mut payload = infinite_payload();
    payload["pages_per_tick"] = json!(1);
    payload["chunk_workers"] = json!(2);
    payload["max_pages"] = json!(8);
    start(&m, payload);
    // Move the stream centre far away: one budgeted slice per flush.
    let first = m
        .call_json("stream", &json!({"x": 200.0, "y": 20.0, "z": 200.0}))
        .unwrap();
    assert!(first["pending"].as_u64().unwrap() <= 8, "{first}");
    assert!(first["resident"].as_u64().unwrap() <= 8, "{first}");
    let count = m.call_json("count", &json!({})).unwrap();
    assert!(count["pending"].as_u64().unwrap() <= 8, "{count}");
}

#[test]
fn persistent_infinite_restart_keeps_unloaded_terrain() {
    let store = Arc::new(MemoryStore::new());
    let payload = infinite_payload();
    let first = module(store.clone());
    start(&first, payload.clone());
    let cell = json!({"x":200, "y":2, "z":200});
    let before = first.call_json("get", &cell).unwrap();
    assert_ne!(before["material"], 0);
    first.call_json("save", &json!({"slot":"world"})).unwrap();
    drop(first);
    let second = module(store);
    let mut payload = payload;
    payload["persistent"] = json!(true);
    let answer = start(&second, payload);
    assert!(
        answer["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["effect"] != "error"),
        "{answer}"
    );
    assert_eq!(second.call_json("get", &cell).unwrap(), before);
}
