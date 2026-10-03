//! Checkpoints and streamed edits through the actual native plugin boundary.
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::{
    native::NativeModule,
    services::HostServices,
    storage::{BlobOp, BlobStore, MemoryStore},
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
fn material(module: &NativeModule, cell: [i32; 3]) -> Value {
    module
        .call_json("get", &json!({"x":cell[0],"y":cell[1],"z":cell[2]}))
        .unwrap()
}
#[test]
fn runtime_saves_survive_a_new_player_and_preview_cannot_touch_them() {
    let store = Arc::new(MemoryStore::new());
    let a = module(store.clone());
    let settings = json!({"size":[32,16,16],"preset":"empty","surface":"smooth","persistent":true});
    start(&a, settings.clone());
    a.call_json(
        "sphere",
        &json!({"x":15,"y":6,"z":6,"radius":2.2,"material":"glow"}),
    )
    .unwrap();
    a.call_json("shape", &json!({"x":15,"y":6,"z":6,"shape":"ramp west"}))
        .unwrap();
    let sample = material(&a, [17, 6, 6]);
    assert!(sample["density"].as_f64().unwrap() > -1.0);
    drop(a);
    let b = module(store.clone());
    let restored = start(&b, settings.clone());
    assert!(
        restored["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["effect"] != "error")
    );
    assert_eq!(material(&b, [17, 6, 6]), sample);
    assert_eq!(material(&b, [15, 6, 6])["shape"], "ramp west");
    let before = store.read(&format!("{ID}/voxel/world.json")).unwrap();
    b.call_json(
        "world.start",
        &json!({"preview":true,"resources":[{"type_id":"world","payload":settings}]}),
    )
    .unwrap();
    assert_eq!(material(&b, [15, 6, 6])["material"], 0);
    assert!(b.call_json("save", &json!({})).is_err());
    assert!(b.call_json("load", &json!({})).is_err());
    b.call_json("set", &json!({"x":0,"y":0,"z":0,"material":"stone"}))
        .unwrap();
    assert_eq!(
        store.read(&format!("{ID}/voxel/world.json")).unwrap(),
        before
    );
}
#[test]
fn damaged_or_incompatible_checkpoints_leave_live_terrain_unchanged() {
    let store = Arc::new(MemoryStore::new());
    let a = module(store.clone());
    start(&a, json!({"size":[16,16,16],"preset":"empty"}));
    a.call_json("set", &json!({"x":1,"y":2,"z":3,"material":"wood"}))
        .unwrap();
    a.call_json("save", &json!({"slot":"mine"})).unwrap();
    let mut checkpoint: Value = serde_json::from_slice(
        &store
            .read(&format!("{ID}/voxel/mine.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    checkpoint["grid"]["cells"] = json!([[[1, 2, 3], 0], [[16, 0, 0], 255]]);
    store
        .commit(&[BlobOp::Write {
            key: format!("{ID}/voxel/mine.json"),
            data: checkpoint.to_string().into_bytes(),
        }])
        .unwrap();
    assert!(a.call_json("load", &json!({"slot":"mine"})).is_err());
    assert_eq!(material(&a, [1, 2, 3])["material"], 5);
    assert!(a.call_json("save", &json!({"slot":"../unsafe"})).is_err());
    a.call_json("save", &json!({"slot":"mine"})).unwrap();
    start(&a, json!({"size":[16,16,16],"preset":"flat"}));
    assert!(a.call_json("load", &json!({"slot":"mine"})).is_err());
}
#[test]
fn streamed_pages_stay_bounded_and_unloaded_edits_survive_teleports_and_saves() {
    let store = Arc::new(MemoryStore::new());
    let a = module(store.clone());
    let settings = json!({"streamed":true,"size":[4096,32,4096],"preset":"flat","stream_radius":1,"max_pages":4,"pages_per_tick":1});
    let first = start(&a, settings.clone());
    assert!(
        first["effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["effect"] == "mesh")
            .count()
            <= 8
    );
    a.call_json("set", &json!({"x":2048,"y":2,"z":2048,"material":"glow"}))
        .unwrap();
    for centre in [
        [2048.0, 0.0, 2048.0],
        [0.0, 0.0, 0.0],
        [2048.0, 0.0, 2048.0],
    ] {
        for _ in 0..5 {
            let result = a
                .call_json(
                    "stream",
                    &json!({"x":centre[0],"y":centre[1],"z":centre[2]}),
                )
                .unwrap();
            assert!(result["resident"].as_u64().unwrap() <= 4);
        }
        assert_eq!(material(&a, [2048, 2, 2048])["material"], 7);
    }
    a.call_json("save", &json!({})).unwrap();
    let b = module(store);
    start(&b, settings);
    b.call_json("load", &json!({})).unwrap();
    assert_eq!(material(&b, [2048, 2, 2048])["material"], 7);
    // Unloaded canonical queries can still see generated terrain.
    assert_eq!(material(&b, [4000, 8, 4000])["material"], 3);
}
#[test]
fn fracture_preserves_supported_and_budgeted_material_and_reloads_fragments() {
    let store = Arc::new(MemoryStore::new());
    let a = module(store.clone());
    let settings = json!({"streamed":true,"size":[64,16,32],"preset":"empty","max_fragments":1,"palette_density":[2500.0]});
    start(&a, settings.clone());
    // A cross-page bridge anchored to the floor, plus two floating islands.
    a.call_json(
        "fill",
        &json!({"x1":14,"y1":0,"z1":4,"x2":14,"y2":4,"z2":4,"material":"stone"}),
    )
    .unwrap();
    a.call_json(
        "fill",
        &json!({"x1":14,"y1":4,"z1":4,"x2":18,"y2":4,"z2":4,"material":"stone"}),
    )
    .unwrap();
    a.call_json("set", &json!({"x":25,"y":6,"z":4,"material":"wood"}))
        .unwrap();
    a.call_json("set", &json!({"x":30,"y":6,"z":4,"material":"wood"}))
        .unwrap();
    let args = json!({"x1":0,"y1":0,"z1":0,"x2":47,"y2":15,"z2":15});
    let result = a.call_json("fracture", &args).unwrap();
    assert_eq!(result["detached_cells"], 1);
    assert_eq!(result["pending"], 1);
    assert_eq!(material(&a, [16, 4, 4])["material"], 1);
    assert_eq!(material(&a, [25, 6, 4])["material"], 0);
    assert_eq!(material(&a, [30, 6, 4])["material"], 5);
    let mesh = result["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "fragment/1")
        .unwrap();
    assert_eq!(mesh["body"]["mass"], 1000.0);
    assert_eq!(mesh["collider_kind"], "convex_hull");
    a.call_json("save", &json!({})).unwrap();
    let b = module(store);
    start(&b, settings);
    let loaded = b.call_json("load", &json!({})).unwrap();
    assert!(
        loaded["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "fragment/1")
    );
    let deleted = b
        .call_json(
            "fragment_set",
            &json!({"id":1,"x":0,"y":0,"z":0,"material":"air"}),
        )
        .unwrap();
    assert_eq!(deleted["effects"][0]["effect"], "remove_mesh");
    // The pending island may now become physical; no material disappeared.
    assert_eq!(b.call_json("fracture", &args).unwrap()["detached_cells"], 1);
}
#[test]
fn gpu_mesh_effects_are_bounded_and_ship_checked_kernels() {
    let a = module(Arc::new(MemoryStore::new()));
    let result = start(
        &a,
        json!({"preset":"flat","size":[16,16,16],"gpu_meshing":true}),
    );
    assert!(
        result["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "gpu_dispatch")
    );
    assert!(
        result["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["effect"] != "gpu_read")
    );
    let mesh = result["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["effect"] == "mesh")
        .unwrap();
    let data: blockloom_plugin_api::mesh::MeshData = serde_json::from_value(mesh.clone()).unwrap();
    data.check().unwrap();
    assert_eq!(data.gpu.unwrap().vertices, 147456);
    let contributions: blockloom_plugin_api::schema::Contributions =
        serde_json::from_str(include_str!("../package/schemas/voxel.json")).unwrap();
    blockloom_plugin_gpu::check::check_kernel(
        &contributions.kernels[0],
        include_str!("../package/kernels/mesh.wgsl"),
    )
    .unwrap();
}

#[test]
fn huge_streamed_bounds_and_rejected_brushes_do_not_allocate_or_mutate_the_world() {
    let a = module(Arc::new(MemoryStore::new()));
    start(
        &a,
        json!({"streamed":true,"size":[1048576,1048576,1048576],"preset":"empty","max_pages":1,"pages_per_tick":1,"stream_radius":0}),
    );
    let at = json!({"x":1048575,"y":1048575,"z":1048575,"material":"stone"});
    a.call_json("set", &at).unwrap();
    assert_eq!(material(&a, [1048575; 3])["material"], 1);
    assert!(
        a.call_json(
            "fill",
            &json!({"x1":0,"y1":0,"z1":0,"x2":1000,"y2":1000,"z2":1000,"material":"glow"})
        )
        .is_err()
    );
    assert_eq!(material(&a, [0; 3])["material"], 0);
    a.call_json("stream", &json!({"x":1048575,"y":1048575,"z":1048575}))
        .unwrap();
    assert_eq!(a.call_json("count", &json!({})).unwrap()["resident"], 1);
    let state = a.call_json("world.save", &json!({})).unwrap();
    a.call_json(
        "set",
        &json!({"x":1048575,"y":1048575,"z":1048575,"material":"air"}),
    )
    .unwrap();
    a.call_json("world.restore", &state).unwrap();
    assert_eq!(material(&a, [1048575; 3])["material"], 1);
}

#[test]
fn preview_pages_finish_in_slices_without_touching_player_storage() {
    let a = module(Arc::new(MemoryStore::new()));
    a.call_json("world.start",&json!({"preview":true,"resources":[{"type_id":"world","payload":{"streamed":true,"persistent":true,"size":[64,16,64],"preset":"flat","max_pages":4,"pages_per_tick":1}}]})).unwrap();
    assert!(
        a.call_json("count", &json!({})).unwrap()["pending"]
            .as_u64()
            .unwrap()
            > 0
    );
    for _ in 0..4 {
        a.call_json("world.preview", &json!({})).unwrap();
    }
    assert_eq!(a.call_json("count", &json!({})).unwrap()["pending"], 0);
    assert_eq!(a.call_json("count", &json!({})).unwrap()["resident"], 4);
}

#[test]
fn dense_checkpoint_pages_compress_and_runtime_options_can_change_between_runs() {
    let store = Arc::new(MemoryStore::new());
    let a = module(store.clone());
    start(&a, json!({"size":[64,64,64],"preset":"empty"}));
    a.call_json(
        "fill",
        &json!({"x1":0,"y1":0,"z1":0,"x2":63,"y2":63,"z2":63,"material":"stone"}),
    )
    .unwrap();
    a.call_json("save", &json!({})).unwrap();
    let bytes = store
        .read(&format!("{ID}/voxel/world.json"))
        .unwrap()
        .unwrap();
    assert!(
        bytes.len() < 10000,
        "{} bytes for 262144 cells",
        bytes.len()
    );
    let b = module(store);
    start(
        &b,
        json!({"size":[64,64,64],"preset":"empty","persistent":true,"gpu_meshing":true}),
    );
    assert_eq!(material(&b, [63; 3])["material"], 1);
}

#[test]
fn moving_fragment_edits_and_checkpoints_keep_pose_and_momentum() {
    struct Pose;
    impl blockloom_plugin_host::services::ServiceProvider for Pose {
        fn call(
            &self,
            _plugin: &str,
            service: &str,
            _input: &Value,
        ) -> Option<Result<Value, String>> {
            (service=="world.mesh_pose").then(||Ok(json!({"position":[20,10,30],"rotation":[0,std::f64::consts::FRAC_1_SQRT_2,0,std::f64::consts::FRAC_1_SQRT_2],"velocity":[2,0,1],"angular_velocity":[0,1,0]})))
        }
    }
    let store = Arc::new(MemoryStore::new());
    let services = HostServices::new("0.0.1")
        .with_save_store(store.clone())
        .with_provider(Arc::new(Pose));
    let a = unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::from([Capability::ProjectStorage]),
            services.for_plugin(ID),
        )
    }
    .unwrap();
    let settings = json!({"preset":"empty","size":[16,16,16]});
    start(&a, settings.clone());
    a.call_json("set", &json!({"x":4,"y":4,"z":4,"material":"stone"}))
        .unwrap();
    a.call_json(
        "fracture",
        &json!({"x1":0,"y1":0,"z1":0,"x2":15,"y2":15,"z2":15}),
    )
    .unwrap();
    let shaped = a
        .call_json(
            "fragment_shape",
            &json!({"id":1.0,"x":0,"y":0,"z":0,"shape":"slab"}),
        )
        .unwrap();
    assert_eq!(shaped["effects"][0]["origin"], json!([20.0, 10.0, 30.0]));
    assert_eq!(
        shaped["effects"][0]["body"]["angular_velocity"],
        json!([0.0, 1.0, 0.0])
    );
    a.call_json("save", &json!({})).unwrap();
    let b = module(store);
    start(&b, settings);
    let loaded = b.call_json("load", &json!({})).unwrap();
    let body = loaded["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "fragment/1")
        .unwrap();
    assert_eq!(body["origin"], json!([20.0, 10.0, 30.0]));
    assert_eq!(body["body"]["velocity"], json!([2.0, 0.0, 1.0]));
    assert_eq!(body["body"]["mass"], 500.0);
    // A cube-world ray uses occupied samples even for a shaped fragment.
    let hit = b
        .call_json(
            "fragment_cast",
            &json!({"id":1,"x":20.5,"y":20,"z":29.5,"dx":0,"dy":-1,"dz":0,"reach":30}),
        )
        .unwrap();
    assert_eq!(hit["hit"], true);
    assert_eq!(hit["cell"], json!([0, 0, 0]));
    assert!((hit["distance"].as_f64().unwrap() - 9.0).abs() < 1e-4);
}

#[test]
fn player_save_write_failures_are_reported_and_stop_still_retires_meshes() {
    struct Unwritable;
    impl BlobStore for Unwritable {
        fn read(&self, _: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
        fn list(&self, _: &str) -> Result<Vec<(String, u64)>, String> {
            Ok(Vec::new())
        }
        fn commit(&self, _: &[BlobOp]) -> Result<(), String> {
            Err("save volume unavailable".into())
        }
    }
    let a = module(Arc::new(Unwritable));
    start(
        &a,
        json!({"preset":"empty","size":[16,16,16],"persistent":true}),
    );
    let edited = a
        .call_json("set", &json!({"x":1,"y":2,"z":3,"material":"stone"}))
        .unwrap();
    assert!(
        edited["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "error")
    );
    assert_eq!(material(&a, [1, 2, 3])["material"], 1);
    assert!(a.call_json("save", &json!({})).is_err());
    let stopped = a.call_json("world.stop", &json!({})).unwrap();
    assert!(
        stopped["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["effect"] == "remove_mesh")
    );
    assert!(a.call_json("get", &json!({"x":1,"y":2,"z":3})).is_err());
}

mod common;

#[test]
fn streamed_gpu_fracture_checkpoints_match_in_the_portable_player() {
    use blockloom_plugin_api::manifest::PluginManifest;
    use blockloom_plugin_host::portable::PortableModule;
    let Some(wasm) = common::build_wasm() else {
        return;
    };
    let manifest: PluginManifest =
        serde_json::from_str(include_str!("../package/plugin.json")).unwrap();
    let store = Arc::new(MemoryStore::new());
    let mut portable = PortableModule::load(
        &std::fs::read(wasm).unwrap(),
        manifest.runtime.portable.as_ref().unwrap(),
        BTreeSet::from([Capability::ProjectStorage, Capability::GpuCompute]),
        HostServices::new("0.0.1")
            .with_save_store(store)
            .for_plugin(ID),
    )
    .unwrap();
    let native = module(Arc::new(MemoryStore::new()));
    let settings = json!({"preset":"empty","size":[4096,32,32],"streamed":true,"gpu_meshing":true,"stream_radius":0,"max_pages":1,"persistent":true});
    let mut calls = vec![(
        "world.start",
        json!({"resources":[{"type_id":"world","payload":settings}]}),
    )];
    calls.extend([
        ("set", json!({"x":20,"y":5,"z":6,"material":"glow"})),
        ("stream", json!({"x":20,"y":5,"z":6})),
        (
            "fracture",
            json!({"x1":16,"y1":1,"z1":0,"x2":31,"y2":15,"z2":15}),
        ),
        ("save", json!({})),
        ("load", json!({})),
        ("world.save", json!({})),
    ]);
    for (op, args) in calls {
        let expected = native.call_json(op, &args).unwrap();
        let actual = portable.call_json(op, &args).unwrap();
        assert_eq!(actual, expected, "portable {op}");
    }
    for surface in ["cubes", "smooth"] {
        let args = json!({"resources":[{"type_id":"world","payload":{
            "preset":"empty","size":[33,33,33],"surface":surface,"gpu_meshing":true}}]});
        assert_eq!(
            portable.call_json("world.start", &args).unwrap(),
            native.call_json("world.start", &args).unwrap()
        );
        let args = json!({"x":31,"y":31,"z":31,"radius":3.2,"material":"glow"});
        assert_eq!(
            portable.call_json("sphere", &args).unwrap(),
            native.call_json("sphere", &args).unwrap()
        );
        let state = native.call_json("world.save", &json!({})).unwrap();
        assert_eq!(
            portable.call_json("world.restore", &state).unwrap(),
            native.call_json("world.restore", &state).unwrap()
        );
        let mut legacy = native.call_json("world.save", &json!({})).unwrap();
        legacy["state"]["version"] = json!(1);
        legacy["state"]["grid"] = json!({"size":[48,48,48],"generator":null,
            "cells":[[[32,32,32],1]],"shapes":[],"densities":[]});
        assert_eq!(
            portable.call_json("world.restore", &legacy).unwrap(),
            native.call_json("world.restore", &legacy).unwrap()
        );
        assert_eq!(
            portable.call_json("world.save", &json!({})).unwrap(),
            native.call_json("world.save", &json!({})).unwrap()
        );
    }
}

#[test]
fn disk_player_save_survives_reopening_the_store() {
    use blockloom_plugin_host::storage::DiskStore;
    let dir = tempfile::tempdir().unwrap();
    let settings = json!({"preset":"empty","size":[16,16,16],"persistent":true});
    let a = module(Arc::new(DiskStore::new(dir.path(), false)));
    start(&a, settings.clone());
    a.call_json("set", &json!({"x":3,"y":4,"z":5,"material":"wood"}))
        .unwrap();
    drop(a);
    let b = module(Arc::new(DiskStore::new(dir.path(), false)));
    start(&b, settings);
    assert_eq!(material(&b, [3, 4, 5])["material"], 5);
}

#[test]
fn streamed_fracture_waits_for_source_collider_replacements() {
    let a = module(Arc::new(MemoryStore::new()));
    start(
        &a,
        json!({"preset":"empty","size":[64,16,16],"streamed":true,"stream_radius":1,"max_pages":2,"pages_per_tick":1}),
    );
    a.call_json(
        "fill",
        &json!({"x1":31,"y1":5,"z1":5,"x2":32,"y2":5,"z2":5,"material":"stone"}),
    )
    .unwrap();
    a.call_json("hook.stream", &json!({"tick":1,"dt":0.016}))
        .unwrap();
    let broken = a
        .call_json(
            "fracture",
            &json!({"x1":30,"y1":4,"z1":4,"x2":33,"y2":6,"z2":6}),
        )
        .unwrap();
    assert_eq!(broken["detached_cells"], 2);
    assert!(
        broken["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["body"].is_null())
    );
    let frame = a
        .call_json("hook.stream", &json!({"tick":2,"dt":0.016}))
        .unwrap();
    let effects = frame["effects"].as_array().unwrap();
    let body = effects
        .iter()
        .position(|e| e["name"] == "fragment/1")
        .unwrap();
    assert!(effects[..body].iter().any(|e| e["effect"] == "remove_mesh"));
}

#[test]
fn legacy_pages_migrate_by_cell_coordinate_and_keep_shapes_and_density() {
    let a = module(Arc::new(MemoryStore::new()));
    start(
        &a,
        json!({"preset":"empty","size":[65,100,33],"surface":"smooth"}),
    );
    let mut state = a.call_json("world.save", &json!({})).unwrap();
    state["state"]["version"] = json!(1);
    state["state"]["grid"] = json!({
        "size":[80,112,48], "generator":null,
        "pages":[{"chunk":[2,6,2],"runs":[[50,0],[1,2],[1,1],[14,0],[1,3],[4029,0]]}],
        "shapes":[[[35,99,32],"slab"]],
        "densities":[[[34,99,32],-64]]
    });
    state["state"]["fragments"] = json!([{
        "id":1,"grid":{"size":[16,16,16],"generator":null,
            "cells":[[[1,2,3],1]],"shapes":[],"densities":[]},
        "origin":[1,2,3],"rotation":[0,0,0,1],"velocity":[0,0,0]
    }]);
    a.call_json("world.restore", &state).unwrap();
    assert_eq!(material(&a, [35, 99, 32])["material"], 1);
    assert_eq!(material(&a, [35, 99, 32])["shape"], "slab");
    assert_eq!(material(&a, [34, 99, 32])["density"], -0.25);
    assert_eq!(material(&a, [34, 100, 32])["material"], 0);
    let upgraded = a.call_json("world.save", &json!({})).unwrap();
    assert_eq!(upgraded["state"]["version"], 2);
    assert_eq!(upgraded["state"]["fragments"][0]["grid"]["page_size"], 32);
    assert_eq!(upgraded["state"]["grid"]["page_size"], 32);
    assert_eq!(upgraded["state"]["grid"]["size"], json!([65, 100, 33]));
    a.call_json("world.restore", &upgraded).unwrap();
    assert_eq!(material(&a, [35, 99, 32])["material"], 1);
}

#[test]
fn column_streaming_has_independent_vertical_and_byte_budgets() {
    let a = module(Arc::new(MemoryStore::new()));
    start(
        &a,
        json!({"streamed":true,"preset":"empty","size":[96,100,96],
        "stream_radius":1,"vertical_radius":1,"max_pages":10,"pages_per_tick":16,
        "max_resident_bytes":65536}),
    );
    let count = a.call_json("count", &json!({})).unwrap();
    assert_eq!(count["column_counts"], json!([3, 3]));
    assert_eq!(count["resident_sections"], 2);
    assert_eq!(count["resident_columns"], 1);
    assert_eq!(count["allocated_bytes"], 0);
    a.call_json("set", &json!({"x":1,"y":33,"z":1,"material":"stone"}))
        .unwrap();
    assert_eq!(
        a.call_json("count", &json!({})).unwrap()["allocated_bytes"],
        32768
    );
    a.call_json("stream", &json!({"x":80,"y":99,"z":80}))
        .unwrap();
    assert_eq!(material(&a, [1, 33, 1])["material"], 1);
    let count = a.call_json("count", &json!({})).unwrap();
    assert!(count["allocated_bytes"].as_u64().unwrap() <= 65536);
    assert!(count["resident_sections"].as_u64().unwrap() <= 2);
}

#[test]
fn partial_sections_submit_bounded_cpu_and_gpu_mesh_tiles() {
    for surface in ["cubes", "smooth"] {
        for height in [1, 31, 33, 100] {
            let a = module(Arc::new(MemoryStore::new()));
            start(
                &a,
                json!({"preset":"empty","size":[33,height,33],"surface":surface,"gpu_meshing":true}),
            );
            let answer = a
                .call_json(
                    "fill",
                    &json!({"x1":0,"y1":0,"z1":0,
                "x2":32,"y2":height-1,"z2":32,"material":"stone"}),
                )
                .unwrap();
            let meshes: Vec<blockloom_plugin_api::mesh::MeshData> = answer["effects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["effect"] == "mesh")
                .map(|e| serde_json::from_value(e.clone()).unwrap())
                .collect();
            assert!(!meshes.is_empty());
            assert!(meshes.iter().any(|m| m.gpu.is_some()));
            for mesh in meshes {
                mesh.check().unwrap();
            }
            assert_eq!(material(&a, [32, height - 1, 32])["material"], 1);
            assert_eq!(material(&a, [32, height, 32])["material"], 0);
            let state = a.call_json("world.save", &json!({})).unwrap();
            a.call_json("world.restore", &state).unwrap();
            assert_eq!(material(&a, [32, height - 1, 32])["material"], 1);
        }
    }
}

#[test]
fn mip_queries_survive_residency_eviction_and_fracture_removals() {
    let a = module(Arc::new(MemoryStore::new()));
    start(
        &a,
        json!({"size":[128,32,32],"preset":"empty","streamed":true,
        "stream_radius":0,"vertical_radius":0,"max_pages":1,"pages_per_tick":1}),
    );
    for x in [63, 64] {
        a.call_json("set", &json!({"x":x,"y":5,"z":5,"material":"wood"}))
            .unwrap();
    }
    let query = json!({"level":4,"x":4,"y":0,"z":0});
    let before = a.call_json("lod_sample", &query).unwrap();
    assert_eq!(before["material"], 5);
    assert!(
        a.call_json("count", &json!({})).unwrap()["lod_nodes"]
            .as_u64()
            .unwrap()
            > 0
    );
    a.call_json("stream", &json!({"x":0,"y":0,"z":0})).unwrap();
    assert_eq!(a.call_json("lod_sample", &query).unwrap(), before);
    a.call_json(
        "fracture",
        &json!({"x1":63,"y1":5,"z1":5,"x2":64,"y2":5,"z2":5}),
    )
    .unwrap();
    let after = a.call_json("lod_sample", &query).unwrap();
    assert_eq!(after["material"], 0);
    assert!(after["revision"].as_u64().unwrap() > before["revision"].as_u64().unwrap());
    assert!(
        a.call_json("lod_sample", &json!({"level":-1,"x":0,"y":0,"z":0}))
            .is_err()
    );
}
