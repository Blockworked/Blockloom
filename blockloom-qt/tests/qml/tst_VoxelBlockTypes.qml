import QtQuick
import QtTest
import "../../../plugins/voxel/package/editor" as Voxel

TestCase {
    id: test
    name: "VoxelBlockTypes"
    visible: true
    width: 800; height: 600
    when: windowShown
    property var files: ({})
    property var writes: []
    property var world: ({ preset: "flat", block_set: "assets/blocks/set.blockset" })
    QtObject {
        id: mockApp
        function invoke(command, args, done, failed) {
            if (command === "read_text_asset") {
                if (test.files[args.path] === undefined) { if (failed) failed("missing"); return; }
                if (done) done(test.files[args.path]);
            } else if (command === "write_text_asset") {
                test.writes = test.writes.concat([args]);
                const next = Object.assign({}, test.files);
                next[args.path] = args.text;
                test.files = next;
                if (done) done(args.path);
            }
        }
    }
    QtObject {
        id: mockHost
        readonly property var app: mockApp
        function resource(name) { return name === "world" ? test.world : {}; }
        function setResource(name, payload, done) { test.world = payload; if (done) done({}); }
    }
    Component { id: factory; Voxel.BlockTypes { host: mockHost } }
    function init() {
        test.writes = [];
        test.world = { preset: "flat", block_set: "assets/blocks/set.blockset" };
        test.files = {
            "assets/blocks/set.blockset": JSON.stringify({ blocks: ["assets/blocks/oak_stairs.blockdef", { name: "marble", id: 900 }] }),
            "assets/blocks/oak_stairs.blockdef": JSON.stringify({ name: "oak_stairs", id: 400, textures: { all: "oak.png" },
                properties: { facing: ["north", "east", "south", "west"], half: ["bottom", "top"] } })
        };
    }

    function test_statesCountLastPropertyFastest() {
        const v = createTemporaryObject(factory, test);
        const props = v.parseProperties("facing: north, east\nhalf: bottom, top");
        compare(v.stateCount(props), 4);
        compare(v.stateNames("s", props, 3), ["s[facing=north,half=bottom]", "s[facing=north,half=top]", "s[facing=east,half=bottom]"]);
        compare(v.propertiesText(props), "facing: north, east\nhalf: bottom, top");
        let threw = false;
        try { v.parseProperties("facing"); } catch (e) { threw = true; }
        verify(threw);
    }
    function test_theSetAndItsFilesLoad() {
        const v = createTemporaryObject(factory, test);
        compare(v.entries.length, 2);
        compare(v.entries[0].def.name, "oak_stairs");
        compare(v.entries[1].file, "");
        compare(v.entries[1].def.id, 900);
    }
    function test_savingKeepsKeysTheFormDoesNotDraw() {
        const v = createTemporaryObject(factory, test);
        const def = v.buildDef(v.entries[0].def, { name: "oak_stairs", id: "401", color: "#8a6a3a", model: "", layer: "cutout",
            tags: "Stairs, wood", properties: "facing: north, south", variants: "" });
        compare(def.textures.all, "oak.png");
        compare(def.id, 401);
        compare(def.layer, "cutout");
        compare(def.tags, ["stairs", "wood"]);
        compare(def.properties.facing, ["north", "south"]);
        let threw = false;
        try { v.buildDef({}, { name: "Bad Name", id: "", color: "", model: "", layer: "opaque", tags: "", properties: "", variants: "" }); }
        catch (e) { threw = true; }
        verify(threw);
    }
    function test_addingATypeWritesItsFileAndListsIt() {
        const v = createTemporaryObject(factory, test);
        v.addType("glass");
        compare(test.writes[0].path, "assets/blocks/glass.blockdef");
        const set = JSON.parse(test.files["assets/blocks/set.blockset"]);
        compare(set.blocks.length, 3);
        compare(set.blocks[2], "assets/blocks/glass.blockdef");
        compare(set.blocks[1].name, "marble");
        compare(v.current, 2);
        v.removeCurrent();
        compare(JSON.parse(test.files["assets/blocks/set.blockset"]).blocks.length, 2);
    }
    function test_aWorldWithNoSetGetsOne() {
        test.world = { preset: "flat" };
        const v = createTemporaryObject(factory, test);
        compare(v.entries.length, 0);
        v.makeSet();
        compare(test.world.block_set, "assets/blocks/blocks.blockset");
        compare(test.world.preset, "flat");
        compare(JSON.parse(test.files["assets/blocks/blocks.blockset"]).blocks, []);
    }
}
