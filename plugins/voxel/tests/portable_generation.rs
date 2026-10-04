mod common;
use blockloom_plugin_api::manifest::{Capability, PluginManifest};
use blockloom_plugin_host::{
    portable::PortableModule, services::HostServices, storage::MemoryStore,
};
use std::{collections::BTreeSet, sync::Arc};
#[test]
fn large_infinite_world_fits_portable_call_budget() {
    let manifest: PluginManifest =
        serde_json::from_str(include_str!("../package/plugin.json")).unwrap();
    let Some(wasm) = common::build_wasm() else {
        return;
    };
    let mut module = PortableModule::load(
        &std::fs::read(wasm).unwrap(),
        manifest.runtime.portable.as_ref().unwrap(),
        BTreeSet::from([Capability::ProjectStorage, Capability::GpuCompute]),
        HostServices::new("0.0.1")
            .with_save_store(Arc::new(MemoryStore::new()))
            .for_plugin("com.blockworked.voxel"),
    )
    .unwrap();
    let mut args = serde_json::json!({"resources":[{"type_id":"world", "payload":{
        "preset":"infinite", "size":[8192,128,8192], "seed":20261002,
        "streamed":true, "visual_lod":true, "gpu_meshing":true,
        "pregen_distant":true, "render_distance":256, "distant_chunks":128,
        "pages_per_tick":4, "chunk_workers":4, "max_pages":256,
        "stream_radius":8, "vertical_radius":2, "max_resident_bytes":16777216
    }}],"records":[{"type_id":"invoker","actor":"player","position":[42.0,21.0,32.0],"payload":{}}]});
    args["mode"] = serde_json::json!("TwoD");
    let menu = module.call_json("world.start", &args).unwrap();
    assert!(
        !menu["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "mesh")
    );
    for _ in 0..24 {
        assert_eq!(
            module
                .call_json("hook.stream", &serde_json::json!({}))
                .unwrap()["effects"],
            serde_json::json!([])
        );
    }
    module
        .call_json("world.stop", &serde_json::json!({}))
        .unwrap();
    args["mode"] = serde_json::json!("ThreeD");
    let answer = module.call_json("world.start", &args).unwrap();
    let effects = answer["effects"].as_array().unwrap();
    assert!(effects.iter().all(|e| e["effect"] != "error"), "{answer}");

    let mut meshes = effects.iter().filter(|e| e["effect"] == "mesh").count();
    let view = serde_json::json!({"position":[42.0,21.7,32.0],"forward":[0.0,0.0,-1.0],"up":[0.0,1.0,0.0],"fov_y":60.0,"viewport_width":1080.0,"viewport_height":720.0,"render_distance":256.0,"split_pixels":160.0,"merge_pixels":120.0});
    for _ in 0..64 {
        let answer = module
            .call_json("hook.stream", &serde_json::json!({"view":view}))
            .unwrap();
        meshes += answer["effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["effect"] == "mesh")
            .count();
        assert!(
            answer["effects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["effect"] != "error"),
            "{answer}"
        );
    }
    assert!(meshes > 0, "streaming must publish terrain meshes");
    module
        .call_json("world.stop", &serde_json::json!({}))
        .unwrap();
    args["resources"][0]["payload"]["size"] = serde_json::json!([33, 1, 33]);
    args["resources"][0]["payload"]["visual_lod"] = serde_json::json!(false);
    args["resources"][0]["payload"]["distant_chunks"] = serde_json::json!(0);
    args["resources"][0]["payload"]["pregen_distant"] = serde_json::json!(false);
    module.call_json("world.start", &args).unwrap();
    let collision_region = serde_json::json!({"actor":"player", "x0":30.0,"y0":0.0,"z0":30.0,"x1":33.0,"y1":2.0,"z1":33.0});
    assert_eq!(
        module
            .call_json("collision_ready", &collision_region)
            .unwrap()["value"],
        false
    );
    for _ in 0..64 {
        module
            .call_json("hook.stream", &serde_json::json!({"view":view}))
            .unwrap();
    }
    assert_eq!(
        module
            .call_json("collision_ready", &collision_region)
            .unwrap()["value"],
        true,
        "collision releases after all four boundary pages are installed"
    );
    let stats = module.call_json("count", &serde_json::json!({})).unwrap();
    assert_eq!(
        stats["pending"], 0,
        "partial sections must finish after their empty padding"
    );
}
