import QtQuick
import com.blockworked.Blockloom 1.0

Window {
    id: root
    visible: true
    width: 640
    height: 360
    title: "Blockloom Game view smoke test"
    property string output: Qt.application.arguments[1]
    property int token: 0
    property var pending: ({})
    property string mode: "TwoD"
    property int phase: 0
    property int waiting: 0

    function command(name, args, done) {
        const id = ++token;
        pending[id] = done;
        bridge.invokeCommand(id, name, JSON.stringify(args || {}));
    }
    function fail(message) {
        console.error("SMOKE FAILED: " + message);
        bridge.shutdown();
        Qt.exit(1);
    }
    function startWorld() {
        command("create_project", { name: mode, mode: mode, location: output }, () => {
            command("set_background", { color: "#ff0000" }, () => {
                command("set_scene_view", { view: { enabled: false } }, () => {
                    loader.active = true;
                    command("open_world", {}, () => settle.start());
                });
            });
        });
    }
    function capture() {
        if (!loader.item.hasFrame && waiting++ < 60) {
            settle.start();
            return;
        }
        if (!loader.item.hasFrame || loader.item.error.length) {
            fail("no frame: " + loader.item.error);
            return;
        }
        loader.item.grabToImage(result => {
            const name = mode + "-" + phase + ".png";
            if (!result.saveToFile(output + "/" + name)) {
                fail("couldn't save " + name);
                return;
            }
            console.log("SMOKE captured " + name);
            if (phase === 0) {
                phase = 1;
                waiting = 0;
                loader.item.resolution = Qt.size(321, 193);
                settle.start();
            } else {
                // Destroy the item while its scene-graph node still exists.
                loader.active = false;
                command("close_project", {}, () => {
                    if (mode === "TwoD") {
                        mode = "ThreeD";
                        phase = 0;
                        waiting = 0;
                        startWorld();
                    } else {
                        bridge.shutdown();
                        console.log("SMOKE PASSED");
                        Qt.quit();
                    }
                });
            }
        });
    }
    AppBridge {
        id: bridge
        onReplied: (id, response) => {
            const reply = JSON.parse(response);
            const done = root.pending[id];
            delete root.pending[id];
            if (!reply.ok) root.fail(reply.error);
            else if (done) done(reply.result);
        }
    }
    Loader {
        id: loader
        anchors.fill: parent
        active: false
        sourceComponent: GameView { }
    }
    Timer { id: settle; interval: 10000; onTriggered: root.capture() }
    Timer { interval: 180000; running: true; onTriggered: root.fail("timed out") }
    Component.onCompleted: {
        bridge.start();
        startWorld();
    }
    onClosing: bridge.shutdown()
}
