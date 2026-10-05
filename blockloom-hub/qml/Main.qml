import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Templates as T
import com.blockworked.BlockloomHub 1.0

ApplicationWindow {
    id: window
    width: 1080
    height: 740
    minimumWidth: 800
    minimumHeight: 580
    visible: true
    title: "Blockloom Hub"
    color: "#171b24"
    palette.window: "#171b24"
    palette.windowText: "#ecedf3"
    palette.base: "#202633"
    palette.text: "#ecedf3"
    palette.button: "#30394b"
    palette.buttonText: "#ecedf3"
    palette.highlight: "#8171eb"
    palette.highlightedText: "#ffffff"
    font.pixelSize: 14

    component HubButton: Button {
        id: control
        padding: 10
        horizontalPadding: 16
        contentItem: Text {
            text: control.text
            font: control.font
            color: control.enabled ? "#ecedf3" : "#798298"
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 6
            color: control.down ? "#4a4375" : control.hovered ? "#414b62" : "#30394b"
            border.color: control.activeFocus ? "#b4a7ff" : "#46516a"
            opacity: control.enabled ? 1 : 0.55
        }
    }
    component HubField: TextField {
        id: control
        color: "#ecedf3"
        placeholderTextColor: "#a7b0c4"
        padding: 10
        verticalAlignment: TextInput.AlignVCenter
        selectByMouse: true
        background: Rectangle {
            implicitHeight: 40
            radius: 6
            color: "#202633"
            border.color: control.activeFocus ? "#b4a7ff" : "#46516a"
        }
    }
    component HubScrollBar: T.ScrollBar {
        id: control
        implicitWidth: 12
        implicitHeight: 12
        padding: 2
        minimumSize: 0.08
        hoverEnabled: true
        visible: policy === T.ScrollBar.AlwaysOn || (policy === T.ScrollBar.AsNeeded && size < 1)
        contentItem: Rectangle {
            radius: 4
            color: control.pressed ? "#8171eb" : control.hovered ? "#a79bdf" : "#626d85"
            opacity: control.active || control.hovered || control.pressed ? 1 : 0.65
        }
        background: Rectangle { radius: 6; color: "#222938" }
    }
    component HubTab: TabButton {
        id: control
        padding: 12
        contentItem: Text {
            text: control.text
            font: control.font
            color: control.checked ? "#ecedf3" : "#a7b0c4"
            horizontalAlignment: Text.AlignHCenter
        }
        background: Rectangle {
            radius: 6
            color: control.checked ? "#3f385d" : "#222938"
            border.color: control.checked ? "#8171eb" : "#30394b"
        }
    }
    component HubCheck: T.CheckBox {
        id: control
        hoverEnabled: true
        spacing: 10
        padding: 4
        implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
        implicitHeight: Math.max(contentItem.implicitHeight, 20) + topPadding + bottomPadding
        opacity: enabled ? 1 : 0.55
        background: null
        indicator: Rectangle {
            width: 20
            height: 20
            x: control.leftPadding
            y: (control.height - height) / 2
            radius: 4
            color: control.checked ? (control.down ? "#6959c9" : "#8171eb") : control.hovered ? "#30394b" : "#202633"
            border.color: control.activeFocus || control.hovered ? "#b4a7ff" : "#66738c"
            Canvas {
                visible: control.checked
                anchors.fill: parent
                onVisibleChanged: if (visible) requestPaint()
                onPaint: {
                    const context = getContext("2d")
                    context.clearRect(0, 0, width, height)
                    context.strokeStyle = "white"
                    context.lineWidth = 2
                    context.lineCap = "round"
                    context.lineJoin = "round"
                    context.beginPath()
                    context.moveTo(5, 10)
                    context.lineTo(9, 14)
                    context.lineTo(15, 6)
                    context.stroke()
                }
            }
        }
        contentItem: Text {
            text: control.text
            font: control.font
            color: "#ecedf3"
            leftPadding: control.indicator.width + control.spacing
            verticalAlignment: Text.AlignVCenter
        }
    }
    component HubCombo: ComboBox {
        id: control
        padding: 10
        contentItem: Text {
            text: control.currentIndex < 0 ? "Select a version" : control.displayText
            font: control.font
            color: "#ecedf3"
            rightPadding: 24
            elide: Text.ElideRight
        }
        indicator: Text {
            text: "▾"
            color: "#ecedf3"
            x: control.width - width - 12
            y: (control.height - height) / 2
        }
        background: Rectangle {
            implicitHeight: 40
            radius: 6
            color: "#202633"
            border.color: control.activeFocus ? "#b4a7ff" : "#46516a"
        }
        delegate: ItemDelegate {
            required property int index
            required property var modelData
            width: control.width
            contentItem: Text { text: modelData; color: "#ecedf3"; font: control.font }
            background: Rectangle { color: control.highlightedIndex === index ? "#4a4375" : "#222938" }
        }
    }
    component HubDialog: Dialog {
        id: control
        padding: 18
        background: Rectangle { color: "#222938"; border.color: "#46516a"; radius: 10 }
        header: Label {
            text: control.title
            padding: 18
            color: "#ecedf3"
            font.pixelSize: 19
            font.bold: true
            wrapMode: Text.Wrap
        }
        footer: DialogButtonBox {
            visible: count > 0
            padding: 18
            background: null
            delegate: HubButton {}
        }
    }

    property var projects: []
    property var installations: []
    property string error: ""
    property string notice: ""
    property string lastBackup: ""
    property var selectedProject: ({})
    property bool smokeBindingDone: false
    property bool smokeDevStarted: false
    property bool checkAfterSettings: false
    property var preparedDev: ({})
    property var availableReleases: []
    property var selectedRelease: ({})
    property var toolsTarget: ({})
    property var deleteTarget: ({})
    property bool installMenuOpen: false

    function perform(command, args) {
        error = ""
        notice = ""
        service.run(command, args || [])
    }
    function refresh() { service.run("installations", []) }
    function displayPath(path) {
        const text = String(path || "")
        if (text.startsWith("\\\\?\\UNC\\")) return "\\\\" + text.slice(8)
        if (text.startsWith("\\\\?\\")) return text.slice(4)
        return text
    }
    function label(installation) {
        if (!installation || !installation.id) return "this installation"
        return installation.kind === "dev"
            ? "Development: " + installation.name + " (" + installation.version + ")"
            : "Blockloom " + installation.version
    }
    function selection(identity) {
        for (let installation of installations)
            if (installation.id === identity) return label(installation)
        return identity ? "Missing installation: " + identity : "Choose an editor"
    }
    function matches(entry) {
        return ((entry.name || "") + " " + entry.path + " " + selection(entry.installation))
            .toLowerCase().includes(search.text.toLowerCase())
    }
    function choose(project) {
        selectedProject = project
        versionChooser.currentIndex = -1
        versionDialog.open()
    }

    HubService {
        id: service
        onCompleted: function(command, ok, response) {
            if (!ok) {
                if (command === "release-settings") window.checkAfterSettings = false
                if (command === "check-releases") window.availableReleases = []
                if (response.startsWith("Operation cancelled")) window.notice = response
                else window.error = response
                return
            }
            try {
                const result = JSON.parse(response)
                if (command === "release-settings") {
                    settingsGithub.checked = result.source === "github-cli"
                    settingsRepo.text = result.repo
                    settingsUrl.text = result.url
                    if (window.checkAfterSettings) {
                        window.checkAfterSettings = false
                        perform("check-releases", [])
                    }
                } else if (command === "check-releases") {
                    window.availableReleases = result.releases
                } else if (command === "installations") {
                    window.installations = result
                    service.run("projects", [])
                } else if (command === "projects") {
                    window.projects = result
                    if (service.smokeTest()) {
                        if ((service.smokePage() === "bind" || service.smokePage() === "upgrade-bind") && !window.smokeBindingDone && result.length > 0) {
                            window.smokeBindingDone = true
                            window.choose(result[0])
                            versionChooser.currentIndex = versionDialog.choices.findIndex(i => i.id === "release-0.1.0")
                            versionDialog.accept()
                            return
                        }
                        if (service.smokePage() === "installations") tabs.currentIndex = 1
                        if (service.smokePage() === "install") installDialog.open()
                        if (service.smokePage() === "version" && result.length > 0) window.choose(result[0])
                        if (service.smokePage() === "upgrade" && result.length > 0) {
                            window.choose(result[0])
                            versionChooser.currentIndex = versionDialog.choices.findIndex(i => i.id === "release-0.1.0")
                        }
                        if (service.smokePage().startsWith("log")) logDialog.open()
                        if (service.smokePage() === "releases") {
                            settingsDialog.open()
                            settingsGithub.checked = true
                        }
                        if ((service.smokePage().startsWith("dev-") || service.smokePage() === "checkbox-hover" || service.smokePage() === "checkbox-checked") && !window.smokeDevStarted) {
                            window.smokeDevStarted = true
                            perform("dev-options", ["dev-0123456789abcdef"])
                            return
                        }
                    }
                    service.smokeReady(window.projects.length, window.installations.length)
                } else if (command === "prepare-dev" || command === "dev-options") {
                    window.preparedDev = result
                    logDialog.close()
                    devDialog.open()
                    if (service.smokeTest()) {
                        if (service.smokePage() === "checkbox-checked") devSdk.checked = true
                        if (service.smokePage() === "dev-install") {
                            devJava.checked = false; devSdk.checked = false; devNdk.checked = false; devTargets.checked = false
                            devDialog.accept()
                        }
                        else service.smokeReady(window.projects.length, window.installations.length)
                    }
                } else if (command === "remember") {
                    window.choose(result)
                    window.refresh()
                } else {
                    if (command === "bind" && result.backup) window.lastBackup = result.backup.path
                    if (command === "bind") logDialog.close()
                    window.notice = command === "open" ? "Editor launched."
                        : command === "bind" ? "Project editor selection saved." + (result.backup ? " Backup: " + result.backup.path : "")
                        : command === "uninstall" ? "Installation uninstalled."
                        : command === "add-tools" ? "Modules added."
                        : "Installation saved."
                    window.refresh()
                }
            } catch (e) { window.error = "Invalid service response: " + e }
        }
    }
    Timer {
        interval: 5000
        repeat: true
        running: !service.busy && !service.smokeTest() && !versionDialog.visible && !devDialog.visible && !installDialog.visible && !settingsDialog.visible && !toolsDialog.visible && !deleteDialog.visible && !window.installMenuOpen
        onTriggered: service.run("installations", [])
    }
    Component.onCompleted: refresh()
    onClosing: function(close) {
        if (service.busy) {
            close.accepted = false
            window.notice = "Wait for the current operation to finish before closing the Hub."
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 28
        spacing: 16
        RowLayout {
            Layout.fillWidth: true
            ColumnLayout {
                spacing: 4
                Label { text: "Blockloom Hub"; font.pixelSize: 30; font.bold: true }
            }
            Item { Layout.fillWidth: true }
            HubButton {
                text: "Show backup"
                visible: window.lastBackup.length > 0
                onClicked: if (!service.showBackup(window.lastBackup)) window.error = "The backup folder could not be opened."
            }
            HubButton { text: "Settings"; enabled: !service.busy; onClicked: settingsDialog.open() }
            HubButton { text: "Refresh"; enabled: !service.busy; onClicked: refresh() }
        }
        TabBar {
            id: tabs
            Layout.fillWidth: true
            HubTab { text: "Projects" }
            HubTab { text: "Installations" }
        }
        Label {
            visible: window.error.length > 0 || window.notice.length > 0
            text: window.error || window.notice
            color: window.error ? "#ffaaaa" : "#b7e4c5"
            wrapMode: Text.Wrap
            maximumLineCount: 4
            elide: Text.ElideRight
            Layout.fillWidth: true
        }
        StackLayout {
            currentIndex: tabs.currentIndex
            Layout.fillWidth: true
            Layout.fillHeight: true
            ColumnLayout {
                spacing: 12
                RowLayout {
                    HubField { id: search; Layout.fillWidth: true; placeholderText: "Search projects" }
                    HubButton { text: "Add project folder"; enabled: !service.busy; onClicked: projectFolder.open() }
                }
                Label {
                    visible: window.projects.length === 0
                    text: "No projects yet. Add an existing project folder to get started."
                    color: "#a7b0c4"
                    Layout.topMargin: 30
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 8
                    model: window.projects.filter(entry => window.matches(entry))
                    ScrollBar.vertical: HubScrollBar { id: projectsScrollBar }
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width - (projectsScrollBar.visible ? projectsScrollBar.width + 8 : 0)
                        height: projectRow.implicitHeight + 28
                        radius: 10
                        color: "#222938"
                        RowLayout {
                            id: projectRow
                            anchors.fill: parent
                            anchors.margins: 14
                            spacing: 16
                            ColumnLayout {
                                Layout.fillWidth: true
                                Label { text: modelData.name || "Unavailable project"; font.bold: true; font.pixelSize: 17 }
                                Label { text: window.displayPath(modelData.path); color: "#a7b0c4"; elide: Text.ElideMiddle; Layout.fillWidth: true }
                                Label {
                                    text: modelData.error ? "Folder unavailable" : (modelData.mode === "ThreeD" ? "3D" : "2D")
                                        + "  ·  " + window.selection(modelData.installation)
                                    color: modelData.error ? "#ffaaaa" : "#c1b8ff"
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                            }
                            HubButton {
                                text: "Select editor"
                                enabled: !service.busy && !modelData.error && !modelData.active
                                onClicked: window.choose(modelData)
                            }
                            HubButton {
                                text: modelData.active ? "In use" : "Open"
                                enabled: !service.busy && !modelData.error && !modelData.active && !!modelData.installation
                                onClicked: perform("open", [modelData.path])
                            }
                        }
                    }
                }
            }
            ColumnLayout {
                spacing: 12
                RowLayout {
                    HubButton { text: "Get a release"; enabled: !service.busy; onClicked: { releaseDialog.open(); perform("check-releases", []) } }
                    HubButton { text: "Import release bundle"; enabled: !service.busy; onClicked: { window.selectedRelease = {}; installDialog.open() } }
                    HubButton { text: "Add local repository"; enabled: !service.busy; onClicked: repoFolder.open() }
                }
                Label {
                    text: " "
                    color: "#a7b0c4"
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
                Label {
                    text: " "
                    color: "#a7b0c4"
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
                Label {
                    visible: window.installations.length === 0
                    text: "Install a prepared release bundle or build from a local repository."
                    color: "#a7b0c4"
                    Layout.topMargin: 30
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 8
                    model: window.installations
                    ScrollBar.vertical: HubScrollBar { id: installationsScrollBar }
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width - (installationsScrollBar.visible ? installationsScrollBar.width + 8 : 0)
                        height: installRow.implicitHeight + 28
                        radius: 10
                        color: "#222938"
                        RowLayout {
                            id: installRow
                            anchors.fill: parent
                            anchors.margins: 14
                            ColumnLayout {
                                Layout.fillWidth: true
                                Label { text: window.label(modelData); font.bold: true; font.pixelSize: 17 }
                                Label {
                                    text: window.displayPath(modelData.repo || modelData.target)
                                    color: "#a7b0c4"
                                    elide: Text.ElideMiddle
                                    Layout.fillWidth: true
                                }
                                Label {
                                    text: modelData.status === "ready"
                                        ? (modelData.running ? "In use" : "Ready") + "  ·  Rust " + modelData.tools.rust.channel
                                            + (Object.keys(modelData.tools).length > 1 ? "  ·  " + Object.keys(modelData.tools).filter(t => t !== "rust").join(", ") : "")
                                        : "Build this repository to create its installation."
                                    color: "#c1b8ff"
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                            }
                            ColumnLayout {
                                HubButton {
                                    visible: !!modelData.prepared
                                    text: "Install prepared build"
                                    enabled: !service.busy && !modelData.running
                                    onClicked: perform("dev-options", [modelData.id])
                                }
                                HubButton {
                                    visible: modelData.kind === "dev"
                                    text: modelData.status === "ready" ? "Rebuild" : "Build"
                                    enabled: !service.busy && !modelData.running
                                    onClicked: {
                                        perform("prepare-dev", [modelData.id])
                                        logDialog.open()
                                    }
                                }
                                HubButton {
                                    text: "⋮"
                                    font.pixelSize: 18
                                    font.bold: true
                                    implicitWidth: 44
                                    Accessible.name: "Installation options for " + window.label(modelData)
                                    Layout.alignment: Qt.AlignRight
                                    enabled: !service.busy
                                    onClicked: installMenu.open()
                                    Menu {
                                        id: installMenu
                                        y: parent.height
                                        onOpened: window.installMenuOpen = true
                                        onClosed: window.installMenuOpen = false
                                        MenuItem {
                                            text: "Add modules..."
                                            enabled: modelData.status === "ready" && !service.busy && !modelData.running
                                            onTriggered: {
                                                toolsDialog.target = modelData
                                                window.toolsTarget = modelData
                                                toolsDialog.open()
                                            }
                                        }
                                        MenuItem {
                                            text: "Open File Location"
                                            enabled: !!modelData.path
                                            onTriggered: { if (!service.showFolder(modelData.path)) window.error = "The installation folder could not be opened." }
                                        }
                                        MenuSeparator {}
                                        MenuItem {
                                            text: "Uninstall..."
                                            enabled: !service.busy && !modelData.running
                                            onTriggered: { window.deleteTarget = modelData; deleteDialog.open() }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        RowLayout {
            BusyIndicator { running: service.busy; visible: running; implicitWidth: 28; implicitHeight: 28 }
            Label { text: service.cancelling ? "Stopping..." : service.busy ? "Working..." : "Ready"; color: "#a7b0c4" }
            Item { Layout.fillWidth: true }
            HubButton { text: service.cancelling ? "Stopping..." : "Cancel"; visible: service.canCancel; enabled: !service.cancelling; onClicked: service.cancel() }
            HubButton { text: "Operation log"; onClicked: logDialog.open() }
        }
    }

    FolderDialog { id: projectFolder; title: "Select a Blockloom project"; onAccepted: perform("remember", [service.localPath(selectedFolder)]) }
    FolderDialog {
        id: repoFolder
        title: "Select a Blockloom repository"
        onAccepted: perform("add-dev", [service.localPath(selectedFolder)])
    }
    FolderDialog {
        id: bundleFolder
        title: "Select an unpacked Blockloom release bundle"
        onAccepted: bundlePath.text = service.localPath(selectedFolder)
    }
    HubDialog {
        id: versionDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 550)
        title: "Select editor for " + (window.selectedProject.name || "project")
        modal: true
        standardButtons: Dialog.Save | Dialog.Cancel
        property var choices: window.installations.filter(i => i.status === "ready")
        onOpened: {
            backupBeforeChange.checked = true
            for (let n = 0; n < choices.length; ++n)
                if (choices[n].id === window.selectedProject.installation) versionChooser.currentIndex = n
        }
        onAccepted: {
            let args = [window.selectedProject.path, choices[versionChooser.currentIndex].id]
            if (backupBeforeChange.checked && backupBeforeChange.visible) args.push("--backup")
            perform("bind", args)
            if (args.includes("--backup")) logDialog.open()
        }
        Component.onCompleted: standardButton(Dialog.Save).enabled = Qt.binding(() => versionChooser.currentIndex >= 0 && !service.busy)
        ColumnLayout {
            anchors.fill: parent
            Label { text: "Current: " + window.selection(window.selectedProject.installation); wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubCombo {
                id: versionChooser
                Layout.fillWidth: true
                model: versionDialog.choices.map(i => window.label(i))
            }
            HubCheck {
                id: backupBeforeChange
                text: "Back up project before changing editor"
                checked: true
                visible: !!window.selectedProject.installation && versionChooser.currentIndex >= 0
                    && versionDialog.choices[versionChooser.currentIndex].id !== window.selectedProject.installation
            }
            Label {
                text: versionDialog.choices.length === 0 ? "Install or build an editor first."
                    : "Selecting an editor saves this project's choice. Opening with a newer editor may migrate project files. Back up your project before upgrading."
                color: "#a7b0c4"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: releaseDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 700)
        height: Math.min(window.height - 60, 560)
        title: "Blockloom releases"
        modal: true
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            spacing: 12
            RowLayout {
                HubButton {
                    text: "Check for releases"
                    enabled: !service.busy
                    onClicked: perform("check-releases", [])
                }
                HubButton {
                    text: "Settings"
                    enabled: !service.busy
                    onClicked: settingsDialog.open()
                }
            }
            Label { text: window.error || window.notice; visible: text.length > 0; color: window.error ? "#ffaaaa" : "#b7e4c5"; wrapMode: Text.Wrap; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: window.availableReleases
                spacing: 8
                ScrollBar.vertical: HubScrollBar { id: releasesScrollBar }
                delegate: Rectangle {
                    required property var modelData
                    width: ListView.view.width - (releasesScrollBar.visible ? releasesScrollBar.width + 8 : 0)
                    height: 72
                    radius: 6
                    color: "#171b24"
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        ColumnLayout {
                            Layout.fillWidth: true
                            Label { text: "Blockloom " + modelData.version; font.bold: true }
                            Label { text: (modelData.channel || "release") + "  ·  " + (modelData.size / 1024 / 1024).toFixed(0) + " MiB"; color: "#a7b0c4" }
                        }
                        HubButton {
                            text: modelData.installed ? "Installed" : "Install"
                            enabled: !service.busy && !modelData.installed
                            onClicked: { window.selectedRelease = modelData; releaseDialog.close(); installDialog.open() }
                        }
                    }
                }
            }
            Label { text: "Installing a release keeps existing editors and project selections."; color: "#a7b0c4"; wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubButton { visible: service.canCancel; text: service.cancelling ? "Stopping..." : "Cancel check"; enabled: !service.cancelling; onClicked: service.cancel() }
        }
    }
    HubDialog {
        id: settingsDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 550)
        title: "Hub settings"
        modal: true
        standardButtons: Dialog.Save | Dialog.Cancel
        onOpened: if (!service.busy) service.run("release-settings", [])
        onAccepted: {
            window.checkAfterSettings = true
            service.run("release-settings", ["--source", settingsGithub.checked ? "github-cli" : "https",
                "--repo", settingsRepo.text.trim(), "--url", settingsUrl.text.trim()])
        }
        Component.onCompleted: {
            standardButton(Dialog.Save).text = "Save"
            standardButton(Dialog.Save).enabled = Qt.binding(() => !service.busy)
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 12
            Label { text: "Blockloom releases"; font.bold: true }
            HubCheck { id: settingsGithub; text: "Use GitHub CLI for private releases"; checked: true; enabled: !service.busy }
            HubField { id: settingsRepo; visible: settingsGithub.checked; placeholderText: "GitHub owner/repo"; Layout.fillWidth: true }
            Label { text: "Uses your gh auth login session."; visible: settingsGithub.checked; color: "#a7b0c4" }
            HubField { id: settingsUrl; visible: !settingsGithub.checked; placeholderText: "HTTPS release catalog URL"; Layout.fillWidth: true }
            Label {
                text: "The catalog URL must serve the release catalog JSON, not a repository page."
                visible: !settingsGithub.checked
                color: "#a7b0c4"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label { text: window.error || window.notice; visible: text.length > 0; color: window.error ? "#ffaaaa" : "#b7e4c5"; wrapMode: Text.Wrap; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight }
        }
    }
    HubDialog {
        id: installDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 550)
        title: window.selectedRelease.version ? "Install Blockloom " + window.selectedRelease.version : "Import a release bundle"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Install"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => (!!window.selectedRelease.version || bundlePath.text.trim().length > 0) && !service.busy)
        }
        onOpened: { javaCheck.checked = false; sdkCheck.checked = false; ndkCheck.checked = false; targetsCheck.checked = false }
        onAccepted: {
            let args = window.selectedRelease.version ? [window.selectedRelease.version, "--sha256", window.selectedRelease.sha256] : [bundlePath.text]
            if (javaCheck.checked) args.push("--java")
            if (sdkCheck.checked) args.push("--android-sdk")
            if (ndkCheck.checked) args.push("--android-ndk")
            if (targetsCheck.checked) args.push("--android-rust-targets")
            perform(window.selectedRelease.version ? "download-release" : "install", args)
            logDialog.open()
        }
        ColumnLayout {
            anchors.fill: parent
            Label { text: window.selectedRelease.version ? "Download size: " + (window.selectedRelease.size / 1024 / 1024).toFixed(0) + " MiB. Choose the tools to include." : "Choose an unpacked release containing its installation manifest and tools."; wrapMode: Text.Wrap; Layout.fillWidth: true }
            RowLayout {
                visible: !window.selectedRelease.version
                HubField { id: bundlePath; Layout.fillWidth: true; placeholderText: "Release bundle folder" }
                HubButton { text: "Browse"; onClicked: bundleFolder.open() }
            }
            HubCheck { text: "Rust toolchain (required)"; checked: true; enabled: false }
            HubCheck { id: javaCheck; text: "Java"; enabled: true }
            HubCheck { id: sdkCheck; text: "Android SDK"; enabled: true }
            HubCheck { id: ndkCheck; text: "Android NDK"; enabled: true }
            HubCheck { id: targetsCheck; text: "Android Rust targets (ARM64 and x86-64)"; enabled: true }
            Label {
                text: window.selectedRelease.version
                    ? "Selected tools are downloaded with this release. APK builds need Java and the SDK; native Android builds also need the NDK and Rust targets."
                    : "Selected tools must be included in the bundle. APK builds need Java and the SDK; native Android builds also need the NDK and Rust targets."
                color: "#a7b0c4"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: toolsDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 550)
        title: "Add modules"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        property var target: ({})
        function has(tool) { return !!(((toolsDialog.target || {}).tools || {})[tool]) }
        onOpened: {
            addJava.checked = has("java"); addJava.enabled = !has("java")
            addSdk.checked = has("android-sdk"); addSdk.enabled = !has("android-sdk")
            addNdk.checked = has("android-ndk"); addNdk.enabled = !has("android-ndk")
            addTargets.checked = has("android-rust-targets"); addTargets.enabled = !has("android-rust-targets")
        }
        onAccepted: {
            let args = [toolsDialog.target.id]
            if (addJava.checked && addJava.enabled) args.push("--java")
            if (addSdk.checked && addSdk.enabled) args.push("--android-sdk")
            if (addNdk.checked && addNdk.enabled) args.push("--android-ndk")
            if (addTargets.checked && addTargets.enabled) args.push("--android-rust-targets")
            perform("add-tools", args)
            logDialog.open()
        }
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Download and add"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => !service.busy && !!toolsDialog.target.id
                && ((addJava.checked && addJava.enabled) || (addSdk.checked && addSdk.enabled)
                    || (addNdk.checked && addNdk.enabled) || (addTargets.checked && addTargets.enabled)))
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: window.label(toolsDialog.target); font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
            Label { text: "Installed modules are checked and cannot be changed. New modules (Java, Android SDK, Android NDK, Android Rust targets) are downloaded with the versions this editor pins."; color: "#a7b0c4"; wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubCheck { id: addJava; text: "Java" }
            HubCheck { id: addSdk; text: "Android SDK" }
            HubCheck { id: addNdk; text: "Android NDK" }
            HubCheck { id: addTargets; text: "Android Rust targets (ARM64 and x86-64)" }
            Label {
                text: "Everything is already installed."
                visible: !addJava.enabled && !addSdk.enabled && !addNdk.enabled && !addTargets.enabled
                color: "#b7e4c5"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: deleteDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 550)
        title: "Uninstall installation"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        property var blockers: window.projects.filter(p => !p.error && p.installation === (window.deleteTarget || {}).id)
        onAccepted: perform("uninstall", [window.deleteTarget.id])
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Uninstall"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => !!window.deleteTarget.id && !service.busy)
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label {
                text: "Uninstall " + window.label(window.deleteTarget) + "? This cannot be undone."
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label {
                visible: deleteDialog.blockers.length > 0
                text: "In use by: " + deleteDialog.blockers.map(p => p.name || p.path).join(", ") + ". Assign those projects another editor first."
                color: "#ffaaaa"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label {
                visible: deleteDialog.blockers.length === 0
                text: "Projects using it must be assigned another editor first."
                color: "#a7b0c4"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: devDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 600)
        title: "Install development build"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: {
            devJava.checked = true; devSdk.checked = true; devNdk.checked = true; devTargets.checked = true
        }
        onRejected: window.notice = "Build prepared. The current installation was kept."
        onAccepted: {
            let args = [window.preparedDev.id]
            if (window.preparedDev.build_id) args.push("--build-id", window.preparedDev.build_id)
            if (devJava.checked) args.push("--java")
            if (devSdk.checked) args.push("--android-sdk")
            if (devNdk.checked) args.push("--android-ndk")
            if (devTargets.checked) args.push("--android-rust-targets")
            perform("install-dev", args)
            logDialog.open()
        }
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Install"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => !service.busy)
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: "Build complete. Choose what to include in this installation."; color: "#ecedf3"; wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubCheck { text: "Rust toolchain (required)"; checked: true; enabled: false }
            HubCheck { id: devJava; text: "Java" }
            HubCheck { id: devSdk; objectName: "smokeCheckbox"; text: "Android SDK" }
            HubCheck { id: devNdk; text: "Android NDK" }
            HubCheck {
                id: devTargets
                text: "Android Rust targets (ARM64 and x86-64)"
            }
            Label {
                text: "Selected tools are downloaded and bundled with this editor. Versions follow the repository's Android requirements. Local tool installations are not used."
                color: "#a7b0c4"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: logDialog
        anchors.centerIn: parent
        width: window.width - 80
        height: window.height - 100
        title: "Operation log"
        modal: true
        function atBottom() {
            const bottom = logViewport.contentHeight - logViewport.height
            if (bottom <= 0) return false
            return logViewport.contentY >= bottom - 4
        }
        function followLog() {
            if (!followOutput.checked) return
            if (logViewport.count > 0) logViewport.positionViewAtEnd()
            logViewport.contentY = Math.max(0, logViewport.contentHeight - logViewport.height)
        }
        property string syncedLog: ""
        function syncLog() {
            const full = service.log || ""
            if (full === syncedLog) return
            if (syncedLog.length === 0 || full.length < syncedLog.length || !full.startsWith(syncedLog)) {
                logModel.clear()
                if (full.length > 0) {
                    const parts = full.split("\n")
                    for (let i = 0; i < parts.length; ++i) {
                        if (i === parts.length - 1 && parts[i] === "") break
                        logModel.append({ line: parts[i] })
                    }
                }
            } else {
                const diff = full.slice(syncedLog.length)
                if (diff.length > 0) {
                    const parts = diff.split("\n")
                    let start = 0
                    if (!syncedLog.endsWith("\n") && logModel.count > 0) {
                        const last = logModel.count - 1
                        logModel.set(last, { line: logModel.get(last).line + parts[0] })
                        start = 1
                    }
                    for (let i = start; i < parts.length; ++i) {
                        if (i === parts.length - 1 && parts[i] === "") break
                        logModel.append({ line: parts[i] })
                    }
                }
            }
            syncedLog = full
            if (followOutput.checked) Qt.callLater(logDialog.followLog)
        }
        ListModel { id: logModel }
        Connections { target: service; function onLogChanged() { logDialog.syncLog() } }
        Component.onCompleted: syncLog()
        onOpened: syncLog()
        footer: Item {
            implicitHeight: logFooter.implicitHeight + 24
            RowLayout {
                id: logFooter
                anchors.fill: parent
                anchors.leftMargin: 18
                anchors.rightMargin: 18
                anchors.topMargin: 12
                anchors.bottomMargin: 12
                spacing: 10
                HubCheck {
                    id: followOutput
                    objectName: "smokeLogFollow"
                    text: "Follow output"
                    checked: true
                    onCheckedChanged: if (checked) Qt.callLater(logDialog.followLog)
                }
                Item { Layout.fillWidth: true }
                HubButton { objectName: "smokeLogCancel"; text: service.cancelling ? "Stopping..." : "Cancel operation"; visible: service.canCancel; enabled: !service.cancelling; onClicked: service.cancel() }
                HubButton { objectName: "smokeLogClose"; text: "Close"; onClicked: logDialog.close() }
            }
        }
        ScrollView {
            id: logScroll
            anchors.fill: parent
            clip: true
            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
            ScrollBar.vertical: HubScrollBar {
                objectName: "smokeLogScrollBar"
                parent: logScroll
                orientation: Qt.Vertical
                x: logScroll.width - width - 2
                y: logViewport.y + 2
                width: 14
                height: logViewport.height - 4
            }
            rightPadding: 16
            // Breathing room as viewport padding, not scrollable content:
            // the first/last lines sit 14px inside the frame but the scroll
            // range stays exactly the lines, so follow math is exact and
            // there is no gap to overscroll past either end.
            topPadding: 14
            bottomPadding: 14
            background: Rectangle { color: "#141922"; radius: 6; border.color: "#46516a" }
            ListView {
                id: logViewport
                objectName: "smokeLogViewport"
                model: logModel
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                // Wheel scrolls become fling velocity, which Flickable clamps
                // at maximumFlickVelocity: the default cap saturates fast/big
                // scrolls so they travel no farther than small ones. A higher
                // cap plus gentler deceleration keeps big scrolls proportional
                // and moves a few more lines per notch.
                maximumFlickVelocity: 6000
                flickDeceleration: 1200
                onContentHeightChanged: if (followOutput.checked) Qt.callLater(logDialog.followLog)
                onHeightChanged: if (followOutput.checked) Qt.callLater(logDialog.followLog)
                onCountChanged: if (followOutput.checked) Qt.callLater(logDialog.followLog)
                onContentYChanged: {
                    // Mirror the scroll position in the checkbox. This only
                    // flips the checkbox, never snaps the view, so an active
                    // drag cannot fight followLog the way the old press/move
                    // handlers did. The scrollable guard keeps a cleared or
                    // short log from unchecking follow under new output.
                    if (logViewport.contentHeight > logViewport.height)
                        followOutput.checked = logDialog.atBottom()
                }
                WheelHandler {
                    id: logWheel
                    // One notch (120 angle units) moves this many pixels, about
                    // four log lines. Trackpad pixel deltas pass through 1:1.
                    property real notchStep: 76
                    acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                    onWheel: event => {
                        // Move contentY directly instead of letting Flickable
                        // turn the wheel into a fling: fling velocity is
                        // clamped, so one big scroll travels no farther than
                        // a small one. Direct moves stay proportional no
                        // matter how large the gesture is.
                        let dy = 0
                        if (event.pixelDelta.y !== 0)
                            dy = -event.pixelDelta.y
                        else if (event.angleDelta.y !== 0)
                            dy = -(event.angleDelta.y / 120) * logWheel.notchStep
                        if (dy === 0) return
                        const bottom = Math.max(0, logViewport.contentHeight - logViewport.height)
                        logViewport.cancelFlick()
                        logViewport.contentY = Math.max(0, Math.min(bottom, logViewport.contentY + dy))
                        if (dy < 0) followOutput.checked = false
                        event.accepted = true
                    }
                }
                delegate: TextEdit {
                    required property string line
                    width: ListView.view.width - 28
                    x: 14
                    text: line
                    color: "#dbe2f2"
                    selectionColor: "#5b4d94"
                    selectedTextColor: "#ffffff"
                    readOnly: true
                    wrapMode: TextEdit.Wrap
                    textFormat: TextEdit.PlainText
                    font.family: Qt.platform.os === "windows" ? "Consolas" : "monospace"
                    font.pixelSize: 13
                    selectByMouse: true
                    selectByKeyboard: true
                    persistentSelection: true
                }
                Text {
                    text: "No output for this operation."
                    color: "#a7b0c4"
                    font.pixelSize: 13
                    x: 14
                    y: 2
                    visible: logViewport.count === 0
                }
            }
        }
    }
}
