//! The voxel plugin through the real boundary: the C entry point in this
//! process, the world host that applies its effects, and the sealed package
//! with its WebAssembly module.
//!
//! The wasm tests build the crate with the toolchain running them. A missing
//! `wasm32-unknown-unknown` target skips them unless `BLOCKLOOM_REQUIRE_WASM`
//! is set, which CI does.

use blockloom_plugin_api::loadout::LoadoutBlock;
use blockloom_plugin_api::manifest::PortableEntry;
use blockloom_plugin_api::schema::{BlockKind, CommandAction, Contributions};
use blockloom_plugin_host::module::CodeModule;
use blockloom_plugin_host::native::{NativeModule, default_services};
use blockloom_plugin_host::package::{self, Package};
use blockloom_plugin_host::portable::PortableModule;
use blockloom_plugin_host::world::{Effect, Outcome, Preloaded, WorldPlugins};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const ID: &str = "com.blockworked.voxel";
mod common;
use common::build_wasm;

fn package_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("package")
}

fn contributions() -> Contributions {
    let text = std::fs::read_to_string(package_dir().join("schemas/voxel.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn blocks() -> Vec<LoadoutBlock> {
    let contributions = contributions();
    contributions
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Statement | BlockKind::Reporter))
        .map(|b| {
            let command = contributions
                .command(b.command.as_deref().unwrap())
                .unwrap();
            let CommandAction::Module { op } = &command.action else {
                panic!("the voxel blocks run module ops");
            };
            LoadoutBlock {
                type_id: b.type_id.clone(),
                op: op.clone(),
                slots: b.slots.clone(),
                wants_actor: false,
                returns: b.returns.clone(),
            }
        })
        .collect()
}

fn module() -> NativeModule {
    unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::new(),
            default_services("0.0.1".to_string()),
        )
    }
    .unwrap()
}

fn hosted(module: CodeModule) -> WorldPlugins {
    WorldPlugins::with_modules(vec![Preloaded {
        id: ID.to_string(),
        module,
        hooks: vec![],
        blocks: blocks(),
    }])
    .unwrap()
}

/// What a game hands the plugin when a run starts: the project's `world`
/// resource.
fn resources(payload: Value) -> impl Fn(&str) -> Value {
    move |_| {
        json!({"records": [], "resources": [{
            "type_id": "world", "schema_version": 1, "payload": payload.clone(),
        }]})
    }
}

/// The meshes a stream of outcomes leaves standing, by name.
#[derive(Default)]
struct Scene {
    meshes: BTreeMap<String, blockloom_plugin_api::mesh::MeshData>,
    said: Vec<String>,
}

impl Scene {
    fn apply(&mut self, outcomes: Vec<Outcome>) {
        for outcome in outcomes {
            match outcome {
                Outcome::Effect {
                    effect: Effect::Mesh(mesh),
                    ..
                } => {
                    self.meshes.insert(mesh.name.clone(), mesh);
                }
                Outcome::Effect {
                    effect: Effect::RemoveMesh { name },
                    ..
                } => {
                    self.meshes.remove(&name);
                }
                Outcome::Effect {
                    effect: Effect::Say { text },
                    ..
                } => self.said.push(text),
                Outcome::Log { .. } => {}
                Outcome::Effect {
                    effect: Effect::NavDirty { .. },
                    ..
                } => {}
                other => panic!("unexpected outcome {other:?}"),
            }
        }
    }

    fn triangles(&self) -> usize {
        self.meshes.values().map(|m| m.indices.len() / 3).sum()
    }
}

fn small_flat() -> Value {
    json!({"preset": "flat", "seed": 1, "size": [32, 16, 32], "voxel_size": 0.5, "origin": [10, 0, -4]})
}

#[test]
fn the_package_schema_is_valid_and_its_blocks_resolve() {
    let contributions = contributions();
    contributions.check_definition().unwrap();
    assert_eq!(contributions.resources.len(), 1);
    assert_eq!(contributions.tools.len(), 4);
    assert_eq!(blocks().len(), 19);
    // Every statement and reporter has the op it names.
    let ops: BTreeSet<_> = blocks().into_iter().map(|b| b.op).collect();
    for op in [
        "set", "fill", "sphere", "generate", "get", "height", "cast", "break", "place",
    ] {
        assert!(ops.contains(op), "{op}");
    }
}

#[test]
fn a_run_draws_the_world_and_edits_redraw_only_what_they_touch() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&resources(small_flat())));
    // A flat 32x16x32 world is one section with four mesh tiles, the
    // ground in each, and a few triangles apiece.
    assert_eq!(scene.meshes.len(), 4, "{:?}", scene.meshes.keys());
    assert!(scene.said[0].contains("32x16x32"), "{:?}", scene.said);
    let ground = scene.triangles();
    assert!(ground < 100, "{ground} triangles for a flat floor");
    let first = &scene.meshes["chunk/0/0/0"];
    assert_eq!(first.origin, [10.0, 0.0, -4.0]);
    assert!(first.collider);
    // A mesh tile is 16 cells of half a unit: positions stay within 8.
    assert!(first.positions.iter().all(|&p| (0.0..=8.0).contains(&p)));

    // A flat world of height 16 is ground up to y 4, grass on top.
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(5), json!(5)], "me")
            .unwrap(),
        json!(4)
    );
    assert_eq!(
        world
            .read(ID, "voxel_at", &[json!(5), json!(4), json!(5)], "me")
            .unwrap(),
        json!(3)
    );

    // A tower redraws the four occupied tiles of its section.
    let before = scene.meshes.clone();
    let outcomes = world.run_block(
        ID,
        "fill_voxels",
        &[
            json!(2),
            json!(5),
            json!(2),
            json!(3),
            json!(8),
            json!(3),
            json!("wood"),
        ],
        "me",
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(
                o,
                Outcome::Effect {
                    effect: Effect::Mesh(_),
                    ..
                }
            ))
            .count(),
        4
    );
    assert!(outcomes.iter().any(|o| matches!(
        o,
        Outcome::Effect {
            effect: Effect::NavDirty { .. },
            ..
        }
    )));
    scene.apply(outcomes);
    assert_eq!(scene.meshes.len(), 4);
    assert_ne!(scene.meshes["chunk/0/0/0"], before["chunk/0/0/0"]);
    assert_eq!(scene.meshes["chunk/1/0/1"], before["chunk/1/0/1"]);

    // A cell on a tile edge redraws the covering section.
    let outcomes = world.run_block(
        ID,
        "set_voxel",
        &[json!(15), json!(5), json!(4), json!("stone")],
        "me",
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(
                o,
                Outcome::Effect {
                    effect: Effect::Mesh(_),
                    ..
                }
            ))
            .count(),
        4
    );

    // Digging a hole through the floor shows in the reporters.
    let outcomes = world.run_block(
        ID,
        "voxel_sphere",
        &[json!(20), json!(4), json!(20), json!(3), json!("air")],
        "me",
    );
    scene.apply(outcomes);
    assert_eq!(
        world
            .read(ID, "voxel_at", &[json!(20), json!(4), json!(20)], "me")
            .unwrap(),
        json!(0)
    );
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(20), json!(20)], "me")
            .unwrap(),
        json!(0)
    );

    // Glowing ore is its own mesh with emission.
    scene.apply(world.run_block(
        ID,
        "set_voxel",
        &[json!(1), json!(5), json!(1), json!("glow")],
        "me",
    ));
    let glow = &scene.meshes["chunk/0/0/0/glow7"];
    assert!(glow.emission.unwrap()[0] > 1.0);
    // Taking it away again retires the mesh.
    scene.apply(world.run_block(
        ID,
        "set_voxel",
        &[json!(1), json!(5), json!(1), json!("air")],
        "me",
    ));
    assert!(!scene.meshes.contains_key("chunk/0/0/0/glow7"));

    // Generating again replaces the world.
    scene.apply(world.run_block(ID, "generate_terrain", &[json!("empty"), json!(1)], "me"));
    assert!(scene.meshes.is_empty(), "{:?}", scene.meshes.keys());
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(5), json!(5)], "me")
            .unwrap(),
        json!(-1)
    );
}

#[test]
fn rays_measure_break_and_build_in_world_units() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&resources(small_flat())));
    // Cells are half a unit from (10, 0, -4); the floor's top is y 2.5.
    let ray = |reach: f64| {
        [
            json!(12.2),
            json!(6),
            json!(0.2),
            json!(0),
            json!(-1),
            json!(0),
            json!(reach),
        ]
    };
    let distance = world
        .read(ID, "voxel_distance", &ray(10.0), "me")
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((distance - 3.5).abs() < 0.01, "{distance}");
    let short = world.read(ID, "voxel_distance", &ray(2.0), "me").unwrap();
    assert_eq!(short.as_f64().unwrap(), -1.0);

    // Breaking opens the cell the ray hit: x (12.2-10)/0.5 = 4, z 8.
    scene.apply(world.run_block(ID, "break_voxel", &ray(10.0), "me"));
    let at = |world: &mut WorldPlugins, y: i64| {
        world
            .read(ID, "voxel_at", &[json!(4), json!(y), json!(8)], "me")
            .unwrap()
    };
    assert_eq!(at(&mut world, 4), json!(0));
    assert_eq!(at(&mut world, 3), json!(2));
    // Building puts a cube on the cell now in front of the ray.
    let mut place = ray(10.0).to_vec();
    place.push(json!("wood"));
    scene.apply(world.run_block(ID, "place_voxel", &place, "me"));
    assert_eq!(at(&mut world, 4), json!(5));
    assert_eq!(at(&mut world, 3), json!(2));
    // A ray that meets nothing changes nothing.
    place[0] = json!(500);
    assert!(world.run_block(ID, "place_voxel", &place, "me").is_empty());
}

#[test]
fn saved_edits_are_laid_over_the_terrain_in_order() {
    let mut payload = small_flat();
    payload["edits"] = json!([
        "fill 0 0 0 3 3 3 wood",
        "set 1 1 1 air",
        "sphere 20 8 20 2 stone",
        "set 99 99 99 stone",
        "bounce 1 2 3",
        "fill 0 0 0 1 1 1 nonsense"
    ]);
    let mut world = hosted(CodeModule::Native(module()));
    let mut outcomes = world.start(&resources(payload));
    // Bad lines are reported by number and the good ones still apply; an edit
    // past the world's edge is just clipped.
    let errors: Vec<String> = outcomes
        .iter()
        .filter_map(|o| match o {
            Outcome::Effect {
                effect: Effect::Error { message },
                ..
            } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("edit 5"), "{errors:?}");
    assert!(errors[1].contains("edit 6"), "{errors:?}");
    outcomes.retain(|o| {
        !matches!(
            o,
            Outcome::Effect {
                effect: Effect::Error { .. },
                ..
            }
        )
    });
    let mut scene = Scene::default();
    scene.apply(outcomes);
    let at = |world: &mut WorldPlugins, c: [i64; 3]| {
        world
            .read(
                ID,
                "voxel_at",
                &[json!(c[0]), json!(c[1]), json!(c[2])],
                "me",
            )
            .unwrap()
    };
    assert_eq!(at(&mut world, [0, 0, 0]), json!(5), "wood");
    assert_eq!(at(&mut world, [1, 1, 1]), json!(0), "a later edit wins");
    assert_eq!(at(&mut world, [20, 8, 20]), json!(1), "stone");
    // The fill that named an unknown material changed nothing.
    assert_eq!(at(&mut world, [3, 3, 3]), json!(5));
}

#[test]
fn a_cell_can_be_a_slab_and_a_saved_shape_line_does_the_same() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&resources(small_flat())));
    let flat = scene.triangles();
    // The floor's top cell (y 4) becomes a slab: a dip in the surface.
    let at = [json!(4), json!(4), json!(8)];
    let mut args = at.to_vec();
    args.push(json!("slab"));
    scene.apply(world.run_block(ID, "shape_voxel", &args, "me"));
    assert!(scene.triangles() > flat, "{} vs {flat}", scene.triangles());
    assert_eq!(world.read(ID, "voxel_at", &at, "me").unwrap(), json!(3));
    // Back to a cube, and it merges flat again.
    args[3] = json!("cube");
    scene.apply(world.run_block(ID, "shape_voxel", &args, "me"));
    assert_eq!(scene.triangles(), flat);
    // Air has no shape, so shaping it does nothing.
    args[1] = json!(9);
    args[3] = json!("post");
    assert!(world.run_block(ID, "shape_voxel", &args, "me").is_empty());

    // The saved edit line gives the same dip when a world starts.
    let mut payload = small_flat();
    payload["edits"] = json!(["shape 4 4 8 top slab"]);
    let mut again = hosted(CodeModule::Native(module()));
    let mut saved = Scene::default();
    saved.apply(again.start(&resources(payload)));
    assert!(saved.triangles() > flat);
}

#[test]
fn a_scene_tool_casts_a_ray_and_its_command_saves_an_edit() {
    let contributions = contributions();
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&resources(small_flat())));
    // Straight down onto the floor's top cell (4, 4, 8).
    let ray = json!({"x": 12.25, "y": 6.0, "z": 0.25, "dx": 0, "dy": -1, "dz": 0, "reach": 200});
    let hit = world.query(ID, "cast", &ray).unwrap();
    assert_eq!(hit["cell"], json!([4, 4, 8]));
    assert_eq!(hit["before"], json!([4, 5, 8]));
    // A ray into the sky is a miss, and an op the module lacks an error.
    let sky = json!({"x": 12.25, "y": 6.0, "z": 0.25, "dx": 0, "dy": 1, "dz": 0, "reach": 20});
    assert_eq!(world.query(ID, "cast", &sky).unwrap()["hit"], json!(false));
    assert!(world.query(ID, "nonsense", &ray).is_err());

    let line = |tool: &str, options: Value| {
        let tool = contributions.tools.iter().find(|t| t.name == tool).unwrap();
        let args = tool.resolve_args(&hit, &options).unwrap();
        let command = contributions.command(&tool.command).unwrap();
        let CommandAction::SetResourceField {
            template: Some(template),
            ..
        } = &command.action
        else {
            panic!("a brush appends a templated edit line");
        };
        blockloom_plugin_api::schema::fill_template(template, &args).unwrap()
    };
    assert_eq!(line("paint", json!({})), "set 4 5 8 stone");
    assert_eq!(line("paint", json!({"material": "wood"})), "set 4 5 8 wood");
    assert_eq!(line("erase", json!({})), "set 4 4 8 air");
    assert_eq!(line("ball", json!({"radius": 2})), "sphere 4 5 8 2 stone");
    assert_eq!(
        line("shape", json!({"shape": "stair north"})),
        "shape 4 4 8 stair north"
    );

    // The saved lines make the same world a run would.
    let mut payload = small_flat();
    payload["edits"] = json!([
        line("paint", json!({"material": "wood"})),
        line("erase", json!({}))
    ]);
    let mut again = hosted(CodeModule::Native(module()));
    again.start(&resources(payload));
    let mut at = |cell: [i64; 3]| {
        again
            .read(ID, "voxel_at", &cell.map(|c| json!(c)), "me")
            .unwrap()
    };
    assert_eq!(at([4, 5, 8]), json!(5), "wood");
    assert_eq!(at([4, 4, 8]), json!(0), "erased");
}

#[test]
fn bad_settings_and_edits_are_refused_with_a_reason() {
    let mut world = hosted(CodeModule::Native(module()));
    // The reason is logged; the call itself reports that it failed.
    let outcomes = world.start(&resources(json!({"size": [4096, 64, 4096]})));
    let text = format!("{outcomes:?}");
    assert!(
        text.contains("1 to 1024") && text.contains("world.start"),
        "{text}"
    );

    let mut world = hosted(CodeModule::Native(module()));
    // Nothing is built until the game starts.
    let early = world.run_block(
        ID,
        "set_voxel",
        &[json!(0), json!(0), json!(0), json!("stone")],
        "me",
    );
    assert!(
        format!("{early:?}").contains("press Play first"),
        "{early:?}"
    );
    world.start(&resources(small_flat()));
    let unknown = world.run_block(
        ID,
        "set_voxel",
        &[json!(0), json!(0), json!(0), json!("lava")],
        "me",
    );
    let text = format!("{unknown:?}");
    assert!(text.contains("no material called lava"), "{text}");
    let preset = world.run_block(ID, "generate_terrain", &[json!("moon"), json!(1)], "me");
    assert!(format!("{preset:?}").contains("island"), "{preset:?}");
}

#[test]
fn a_default_world_is_what_an_absent_resource_means() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&|_| json!({"records": [], "resources": []})));
    assert!(scene.said[0].contains("64x32x64"), "{:?}", scene.said);
    // 4 x 2 x 4 chunks, the island's air above and below left out.
    assert!(!scene.meshes.is_empty() && scene.meshes.len() <= 32);
    let again = {
        let mut other = hosted(CodeModule::Native(module()));
        let mut scene = Scene::default();
        scene.apply(other.start(&|_| json!({"records": [], "resources": []})));
        scene
    };
    // The same seed, the same meshes.
    assert_eq!(scene.meshes, again.meshes);
}

fn portable(path: &Path, entry: &PortableEntry) -> PortableModule {
    PortableModule::load(
        &std::fs::read(path).unwrap(),
        entry,
        BTreeSet::new(),
        default_services("0.0.1".to_string()),
    )
    .unwrap()
}

/// The sealed package's module draws the same default world as the native
/// one, inside the manifest's own call budget.
#[test]
fn the_sealed_package_runs_in_the_portable_executor() {
    let Some(wasm) = build_wasm() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(ID);
    package::copy_dir(&package_dir(), &root).unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/voxel.wasm")).unwrap();
    package::seal(&root).unwrap();
    let package = Package::load(&root).unwrap();
    assert_eq!(package.manifest.id, ID);
    assert_eq!(package.contributions.blocks.len(), 19);

    let entry = package.manifest.runtime.portable.clone().unwrap();
    let wasm_module = portable(&root.join(&entry.module), &entry);
    let mut world = hosted(CodeModule::Portable(Box::new(wasm_module)));
    let mut scene = Scene::default();
    let began = std::time::Instant::now();
    scene.apply(world.start(&|_| json!({"records": [], "resources": []})));
    println!(
        "voxel.wasm is {} bytes; the default world took {:?}",
        std::fs::metadata(&wasm).unwrap().len(),
        began.elapsed()
    );
    assert!(scene.said[0].contains("64x32x64"), "{:?}", scene.said);

    let mut native = hosted(CodeModule::Native(module()));
    let mut expected = Scene::default();
    expected.apply(native.start(&|_| json!({"records": [], "resources": []})));
    assert_eq!(scene.meshes, expected.meshes);
}

#[test]
fn smooth_sculpting_replays_saved_edits_and_uses_exact_collision_meshes() {
    let native = module();
    let start = json!({"resources": [{"type_id": "world", "payload": {
        "preset": "empty", "surface": "smooth", "size": [32,16,16],
        "voxel_size": 2, "origin": [-10,3,-4]
    }}]});
    native.call_json("world.start", &start).unwrap();
    let args = json!({"x": 16,"y": 6,"z": 6,"radius": 3.2,"material": "glow"});
    let edited = native.call_json("sphere", &args).unwrap();
    assert!(edited["changed"].as_u64().unwrap() > 0);
    let mut meshes = BTreeMap::new();
    for effect in edited["effects"].as_array().unwrap() {
        if effect["effect"] != "mesh" {
            continue;
        }
        let mesh: blockloom_plugin_api::mesh::MeshData =
            serde_json::from_value(effect.clone()).unwrap();
        mesh.check().unwrap();
        assert!(mesh.collider && mesh.emission.is_some());
        meshes.insert(mesh.name.clone(), mesh);
    }
    assert_eq!(meshes.len(), 2, "both sides of the chunk boundary");
    assert_eq!(native.call_json("sphere", &args).unwrap()["changed"], 0);
    // The ray hits the interpolated sphere surface, beyond the cell boundary.
    let hit = native
        .call_json(
            "cast",
            &json!({"x":23,"y":30,"z":9,"dx":0,"dy":-1,"dz":0,"reach":40}),
        )
        .unwrap();
    assert_eq!(hit["hit"], true);
    let distance = hit["distance"].as_f64().unwrap();
    assert!((distance - 7.6).abs() < 0.03, "{hit}");
    let replay = module();
    let mut saved = start.clone();
    saved["resources"][0]["payload"]["edits"] = json!(["sphere 16 6 6 3.2 glow"]);
    let replayed = replay.call_json("world.start", &saved).unwrap();
    let replay_meshes: BTreeMap<_, _> = replayed["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["effect"] == "mesh")
        .map(|e| {
            let m: blockloom_plugin_api::mesh::MeshData =
                serde_json::from_value(e.clone()).unwrap();
            (m.name.clone(), m)
        })
        .collect();
    assert_eq!(meshes, replay_meshes);
    let carved = native
        .call_json(
            "sphere",
            &json!({"x":16,"y":6,"z":6,"radius":2.2,"material":"air"}),
        )
        .unwrap();
    assert!(carved["changed"].as_u64().unwrap() > 0);
    assert_eq!(
        native
            .call_json("get", &json!({"x":16,"y":6,"z":6}))
            .unwrap()["material"],
        0
    );
    // A different paint stays within the sphere rather than its bounding box.
    native
        .call_json("set", &json!({"x":18,"y":8,"z":8,"material":"wood"}))
        .unwrap();
    native
        .call_json(
            "sphere",
            &json!({"x":16,"y":6,"z":6,"radius":2.2,"material":"stone"}),
        )
        .unwrap();
    assert_eq!(
        native
            .call_json("get", &json!({"x":18,"y":8,"z":8}))
            .unwrap()["material"],
        5
    );
}

#[test]
fn smooth_worlds_match_native_and_portable_execution() {
    let Some(wasm) = build_wasm() else {
        return;
    };
    let entry = PortableEntry {
        module: "portable/voxel.wasm".into(),
        memory_limit_mib: 256,
        call_limit_ms: 10000,
    };
    let mut portable = portable(&wasm, &entry);
    let native = module();
    let start = json!({"resources": [{"type_id":"world","payload": {
        "surface":"smooth","preset":"island"
    }}]});
    let began = std::time::Instant::now();
    let expected = native.call_json("world.start", &start).unwrap();
    println!(
        "smooth payload {} bytes, {} vertices",
        serde_json::to_vec(&expected).unwrap().len(),
        expected["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["positions"].as_array().map_or(0, |p| p.len() / 3))
            .sum::<usize>()
    );
    let actual = portable.call_json("world.start", &start).unwrap();
    println!(
        "default smooth native and wasm worlds took {:?}",
        began.elapsed()
    );
    assert_eq!(expected, actual);
    for (op, args) in [
        (
            "sphere",
            json!({"x":16,"y":8,"z":16,"radius":3.2,"material":"air"}),
        ),
        ("shape", json!({"x":8,"y":1,"z":8,"shape":"ramp north"})),
        (
            "cast",
            json!({"x":16.5,"y":30,"z":16.5,"dx":0,"dy":-1,"dz":0,"reach":100}),
        ),
        ("lod_sample", json!({"level":4,"x":1,"y":0,"z":1})),
        ("set", json!({"x":31,"y":1,"z":31,"material":"glow"})),
        ("lod_sample", json!({"level":4,"x":1,"y":0,"z":1})),
        ("lod_sample", json!({"level":1,"x":15,"y":0,"z":15})),
        ("lod_mesh", json!({"level":1,"x":1,"y":0,"z":1})),
        ("count", json!({})),
        ("generate", json!({"preset":"empty","seed":3})),
        ("lod_sample", json!({"level":4,"x":1,"y":0,"z":1})),
        ("lod_mesh", json!({"level":4,"x":0,"y":0,"z":0})),
        ("set", json!({"x":31,"y":1,"z":31,"material":"glow"})),
        ("shape", json!({"x":31,"y":1,"z":31,"shape":"post"})),
        ("lod_mesh", json!({"level":4,"x":0,"y":0,"z":0})),
        ("lod_mesh", json!({"level":4,"x":0,"y":0,"z":0})),
        ("set", json!({"x":31,"y":1,"z":31,"material":"air"})),
        ("lod_mesh", json!({"level":4,"x":0,"y":0,"z":0})),
        ("lod_mesh", json!({"level":4,"x":0,"y":0,"z":0})),
        ("count", json!({})),
    ] {
        assert_eq!(
            native.call_json(op, &args).unwrap(),
            portable.call_json(op, &args).unwrap(),
            "{op}"
        );
    }
}
