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
    property bool deferStart: false
    property bool deferBuild: false
    property bool deferFrame: false
    property bool failInstall: false
    property bool deferLog: false
    property var buildStatus: ({})
    property var avds: [{ name: "blockloom", serial: "emulator-5554", booted: true }]
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
            else if (command === "android_emulator_status") done({ available: true, avds: test.avds });
            else if (command === "android_start_emulator" && test.deferStart) test.deferred.start = done;
            else if (command === "start_build_game") {
                test.buildStatus = { id: 1, state: "running", step: "Compiling Android runtime", detail: "Compiling blockloom-runtime", device: args.device };
                if (test.deferBuild) {
                    test.deferred.build = built => { test.buildStatus = { id: 1, state: "complete", step: "Running on device", built: built, device: args.device }; };
                } else if (test.failInstall) test.buildStatus = { id: 1, state: "failed", error: "Device disconnected" };
                else test.buildStatus = { id: 1, state: "complete", built: { binary: "/tmp/Demo.apk" }, device: args.device };
                done(test.buildStatus);
            }
            else if (command === "build_job_status") done(test.buildStatus);
            else if (command === "cancel_build_job") { test.buildStatus = { id: 1, state: "cancelled", step: "Cancelled" }; done({}); }
            else if (command === "android_mirror_frame" && test.deferFrame) test.deferred.frame = done;
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
        calls = []; deferred = {}; deferStart = false; deferBuild = false; deferFrame = false; failInstall = false; deferLog = false;
        avds = [{ name: "blockloom", serial: "emulator-5554", booted: true }];
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
        compare(matching("start_build_game").length, 1);
        compare(matching("start_build_game")[0].args.target, "aarch64-linux-android");
        verify(panel.busy);
        verify(!findChild(panel, "deployButton").enabled);
        panel.device = "emulator-5554";
        deferred.build({ binary: "/tmp/Demo.apk", application_id: "com.blockloom.game.demo" });
        findChild(panel, "deviceBuildProgress").poll();
        compare(matching("start_build_game")[0].args.device, "phone-1");
        verify(!panel.busy);
    }
    function test_deploymentProgressAndCancel() {
        panel.device = "emulator-5554"; deferBuild = true;
        panel.buildAndRun();
        const progress = findChild(panel, "deviceBuildProgress");
        verify(progress.visible);
        verify(findChild(progress, "buildProgressBar").indeterminate);
        compare(progress.step, "Compiling Android runtime");
        grabImage(panel).save("/tmp/blockloom-deploy-progress.png");
        mouseClick(findChild(progress, "cancelBuildButton"));
        compare(matching("cancel_build_job").length, 1);
        verify(!panel.busy);
        compare(panel.buildState, "Deployment cancelled.");
        verify(!panel.polling);
    }
    function test_emulatorTargetAndInstallFailure() {
        panel.device = "emulator-5554"; failInstall = true;
        panel.buildAndRun();
        compare(matching("start_build_game")[0].args.target, "x86_64-linux-android");
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
        compare(matching("start_build_game").length, 0);
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
        compare(matching("android_start_emulator")[0].args.waitSecs, 0);
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
    function test_startIndicatorAndDuplicateGuard() {
        deferStart = true;
        avds = [{ name: "blockloom", serial: "", booted: false }];
        panel.refresh();
        panel.startAvd("blockloom");
        compare(panel.pendingAvd, "blockloom");
        verify(findChild(panel, "startup_blockloom").running);
        panel.startAvd("blockloom");
        compare(matching("android_start_emulator").length, 1);
        deferred.start({ avd: "blockloom", serial: "", booted: false });
        compare(panel.pendingAvd, "blockloom");
        verify(!panel.emuBusy);
        avds = [{ name: "blockloom", serial: "emulator-5554", booted: false }];
        panel.refresh();
        compare(panel.device, "emulator-5554");
        verify(panel.booting);
        verify(findChild(panel, "startup_blockloom").running);
        avds = [{ name: "blockloom", serial: "emulator-5554", booted: true }];
        panel.refresh();
        compare(panel.pendingAvd, "");
        verify(!findChild(panel, "startup_blockloom").running);
        verify(panel.ready);
    }
    function test_manageStoppedAvd() {
        verify(!findChild(panel, "manage_blockloom").enabled);
        avds = [{ name: "test-device", serial: "", booted: false }];
        panel.refresh();
        const menu = findChild(panel, "manage_test-device");
        verify(menu.enabled);
        mouseClick(menu);
        const rename = findChild(test.Window.window.contentItem, "rename_test-device");
        tryCompare(rename, "visible", true);
        mouseClick(rename);
        const dialog = findChild(panel, "manageAvdDialog");
        tryCompare(dialog, "opened", true);
        compare(panel.editAvd, "test-device");
        compare(panel.renamedAvd, "test-device");
        panel.renamedAvd = "renamed";
        panel.manageAvd();
        compare(matching("android_rename_avd")[0].args.newName, "renamed");
        tryCompare(dialog, "opened", false);
        mouseClick(menu);
        mouseClick(findChild(test.Window.window.contentItem, "delete_test-device"));
        tryCompare(dialog, "opened", true);
        verify(panel.deletingAvd);
        panel.manageAvd();
        compare(matching("android_delete_avd")[0].args.name, "test-device");
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
        const navigation = findChild(panel, "deviceNavigation");
        const midpoint = navigation.mapToItem(screen, navigation.width / 2, 0);
        fuzzyCompare(midpoint.x, screen.width / 2, 1);
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
