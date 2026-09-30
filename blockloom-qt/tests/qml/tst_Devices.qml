import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Devices"
    visible: true
    width: 1000; height: 720
    when: windowShown
    property var panel: null
    property var calls: []
    property var deferred: ({})
    property bool deferBuild: false
    property bool deferFrame: false
    property bool failInstall: false
    property bool deferLog: false
    property var status: [
        { serial: "phone-1", state: "device", emulator: false },
        { serial: "emulator-5554", state: "device", emulator: true }
    ]
    QtObject {
        id: backend
        property var appState: ({ project: { name: "Demo" }, default_build_location: "/tmp/build" })
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "android_device_status") done(test.status);
            else if (command === "android_emulator_status") done({ available: true, avds: [
                { name: "blockloom", serial: "emulator-5554", booted: true }
            ] });
            else if (command === "build_game" && test.deferBuild) test.deferred.build = done;
            else if (command === "android_mirror_frame" && test.deferFrame) test.deferred.frame = done;
            else if (command === "build_game") done({ binary: "/tmp/Demo.apk", application_id: "com.blockloom.game.demo" });
            else if (command === "android_install" && test.failInstall) failed("Device disconnected");
            else if (command === "android_install") done({ component: "NativeActivity", device: args.device });
            else if (command === "android_logcat_tail" && test.deferLog) test.deferred.log = done;
            else if (command === "android_logcat_tail") done({ lines: [], panics: [] });
            else if (command === "android_mirror_frame") done({ image: "", width: 360, height: 800 });
            else done({});
        }
    }
    Component { id: factory; Editor.DevicesPanel { app: backend } }
    Component { id: imageFactory; Rectangle { width: 360; height: 800; color: "#219b76" } }
    function init() {
        test.Window.window.width = 1000; test.Window.window.height = 720;
        calls = []; deferred = {}; deferBuild = false; deferFrame = false; failInstall = false; deferLog = false;
        status = [
            { serial: "phone-1", state: "device", emulator: false },
            { serial: "emulator-5554", state: "device", emulator: true }
        ];
        panel = createTemporaryObject(factory, test, { width: 1000, height: 720 });
        verify(panel !== null);
    }
    function cleanup() { panel.destroy(); panel = null; }
    function matching(command) { return calls.filter(c => c.command === command); }
    function test_selectionAndReadiness() {
        compare(panel.device, "");
        verify(!findChild(panel, "deployButton").enabled);
        panel.device = "phone-1";
        verify(panel.ready);
        status = [{ serial: "phone-1", state: "unauthorized", emulator: false }];
        panel.refresh();
        verify(!panel.ready);
        verify(!findChild(panel, "deployButton").enabled);
        status = [];
        panel.refresh();
        compare(panel.device, "");
    }
    function test_buildCapturesTargetAndPreventsRepeat() {
        panel.device = "phone-1";
        deferBuild = true;
        const deploy = findChild(panel, "deployButton");
        mouseClick(deploy); mouseClick(deploy);
        compare(matching("build_game").length, 1);
        compare(matching("build_game")[0].args.target, "aarch64-linux-android");
        verify(panel.busy);
        verify(!findChild(panel, "deployButton").enabled);
        panel.device = "emulator-5554";
        deferred.build({ binary: "/tmp/Demo.apk", application_id: "com.blockloom.game.demo" });
        compare(matching("android_install")[0].args.device, "phone-1");
        verify(!panel.busy);
    }
    function test_emulatorTargetAndInstallFailure() {
        panel.device = "emulator-5554"; failInstall = true;
        panel.buildAndRun();
        compare(matching("build_game")[0].args.target, "x86_64-linux-android");
        verify(!panel.busy);
        compare(panel.buildError, "Device disconnected");
        verify(!panel.polling);
    }
    function test_bootingEmulatorCannotDeploy() {
        panel.device = "emulator-5554";
        panel.emulator = { available: true, avds: [
            { name: "blockloom", serial: "emulator-5554", booted: false }
        ] };
        verify(panel.booting);
        verify(!findChild(panel, "deployButton").enabled);
        panel.buildAndRun();
        compare(matching("build_game").length, 0);
    }
    function test_mirrorIgnoresStaleFrameAndNoOverlappingPolls() {
        panel.device = "phone-1"; deferFrame = true;
        const screen = findChild(panel, "deviceScreen");
        tryCompare(screen, "connected", true);
        tryCompare(screen, "device", "phone-1");
        screen.startWatching(); screen.pollFrame(); screen.pollFrame();
        compare(matching("android_mirror_frame").length, 1);
        panel.device = "emulator-5554";
        tryCompare(screen, "device", "emulator-5554");
        deferred.frame({ image: "stale", width: 360, height: 800 });
        compare(screen.frame, "");
        verify(!screen.watching);
        screen.startWatching();
        compare(matching("android_mirror_frame")[1].args.device, "emulator-5554");
        panel.visible = false;
        deferred.frame({ image: "hidden", width: 360, height: 800 });
        compare(screen.frame, "");
        verify(!screen.watching);
        panel.visible = true;
        tryCompare(screen, "watching", true);
        compare(matching("android_mirror_frame").length, 3);
    }
    function test_stopCannotKillPhone() {
        panel.stopEmu("phone-1");
        compare(matching("android_stop_emulator").length, 0);
    }
    function test_emulatorLifecycleAndDialog() {
        panel.startAvd("blockloom");
        compare(matching("android_start_emulator").length, 1);
        compare(matching("android_start_emulator")[0].args.wait_secs, 0);
        compare(matching("android_start_emulator")[0].args.headless, true);
        panel.stopEmu("emulator-5554");
        compare(matching("android_stop_emulator")[0].args.serial, "emulator-5554");
        const create = findChild(panel, "newAvdButton");
        verify(create.enabled);
        wait(50);
        mouseClick(create);
        const dialog = findChild(panel, "avdDialog");
        tryCompare(dialog, "opened", true);
        verify(dialog.height > 160);
        panel.newAvd = "test-device";
        panel.createAvd();
        compare(matching("android_create_avd")[0].args.name, "test-device");
        verify(!panel.emuBusy);
        tryCompare(dialog, "opened", false);
    }
    function test_staleLogsAndPollingGuard() {
        panel.device = "phone-1"; deferLog = true;
        panel.buildAndRun();
        panel.pollLogcat(); panel.pollLogcat();
        compare(matching("android_logcat_tail").length, 1);
        panel.device = "emulator-5554";
        deferred.log({ lines: ["old run"], panics: [] });
        compare(panel.logcat, "");
        verify(!panel.logBusy);
    }
    function test_previewControlsAndAspect() {
        panel.device = "phone-1";
        const screen = findChild(panel, "deviceScreen");
        const show = findChild(panel, "showScreenButton");
        const toggle = findChild(panel, "mirrorToggle");
        verify(show.visible); verify(!toggle.visible);
        deferFrame = true;
        mouseClick(show);
        verify(!show.visible); verify(toggle.visible);
        const sample = createTemporaryObject(imageFactory, test);
        grabImage(sample).save("/tmp/blockloom-qml-test-frame.png");
        sample.destroy();
        deferred.frame({ image: "file:///tmp/blockloom-qml-test-frame.png", width: 360, height: 800 });
        wait(50);
        const image = findChild(panel, "mirrorImage");
        verify(image.visible);
        fuzzyCompare(image.width / image.height, 360 / 800, 0.001);
        verify(image.height < screen.height);
        verify(image.width < screen.width);
        const captured = grabImage(panel);
        const point = image.mapToItem(panel, image.width / 2, image.height / 2);
        compare(captured.pixel(Math.floor(point.x), Math.floor(point.y)), "#219b76");
        captured.save("/tmp/blockloom-qml-preview.png");
        screen.frameWidth = 800; screen.frameHeight = 360;
        wait(50);
        fuzzyCompare(image.width / image.height, 800 / 360, 0.001);
        verify(image.width < screen.width);
        mouseClick(toggle);
        verify(!screen.watching);
    }
    function test_layout_data() {
        return [
            { tag: "compact", w: 640, h: 480 },
            { tag: "desktop", w: 1000, h: 720 },
            { tag: "wide", w: 1600, h: 1000 }
        ];
    }
    function test_layout(data) {
        test.Window.window.width = data.w; test.Window.window.height = data.h;
        panel.width = data.w; panel.height = data.h;
        panel.device = "phone-1";
        wait(50);
        const screen = findChild(panel, "deviceScreen");
        verify(screen.width > 0);
        verify(screen.height > 0);
        const deploy = findChild(panel, "deployButton");
        verify(deploy.width > 150);
        verify(deploy.mapToItem(panel, 0, 0).y < panel.height);
        const image = grabImage(panel);
        compare(image.width, data.w);
        compare(image.height, data.h);
        verify(image.red(data.w - 10, 10) < 60);
        verify(image.pixel(10, 10) !== image.pixel(data.w - 10, 10));
        image.save("/tmp/blockloom-devices-" + data.tag + ".png");
    }
}
