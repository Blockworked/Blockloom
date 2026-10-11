import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.BlockloomHub 1.0
import com.blockworked.Blockstitch 1.0

ApplicationWindow {
    id: window
    width: 1080
    height: 740
    minimumWidth: 800
    minimumHeight: 580
    visible: true
    title: "Blockloom Hub"
    color: Theme.window
    palette.window: Theme.window
    palette.windowText: Theme.text
    palette.base: Theme.field
    palette.text: Theme.text
    palette.button: Theme.panelRaised
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: "white"
    palette.placeholderText: Theme.textDim
    font.pixelSize: 13
    font.family: regularFont.status === FontLoader.Ready ? regularFont.name : Qt.application.font.family
    FontLoader { id: regularFont; source: "qrc:/fonts/AtkinsonHyperlegibleNext-Regular.ttf" }
    FontLoader { source: "qrc:/fonts/AtkinsonHyperlegibleNext-Medium.ttf" }
    FontLoader { source: "qrc:/fonts/AtkinsonHyperlegibleNext-SemiBold.ttf" }
    FontLoader { source: "qrc:/fonts/AtkinsonHyperlegibleNext-Bold.ttf" }

    // The Hub's controls are blockstitch's, so it looks like the editor.
    component HubButton: StitchButton {}
    component HubField: StitchTextField { implicitHeight: 34 }
    component HubScrollBar: ScrollBar {}
    component HubTab: TabButton {
        id: control
        implicitHeight: 34
        padding: 8
        horizontalPadding: 14
        HoverHandler { cursorShape: Qt.PointingHandCursor }
        contentItem: Text {
            text: control.text
            font.pixelSize: 13
            font.weight: control.checked ? Font.DemiBold : Font.Normal
            color: control.checked ? Theme.text : Theme.textDim
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 6
            color: control.checked ? Theme.panelRaised : control.hovered ? Theme.hover : "transparent"
        }
    }
    component HubCheck: StitchCheckBox { font.pixelSize: 13; hoverEnabled: true }
    component HubCombo: StitchComboBox {
        displayText: currentIndex < 0 ? "Select a version" : currentText
    }
    component HubDialog: StitchDialog {}

    property var projects: []
    property var installations: []
    property string error: ""
    property string notice: ""
    property string lastBackup: ""
    property var backupRows: []
    property var backupsTarget: ({})
    property var restoreTarget: ({})
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
    property string stage: ""
    property var moduleInfo: ({})
    function moduleText(id, fallback) {
        const row = (moduleInfo.modules || []).find(m => m.id === id)
        return row ? row.label + " (about " + row.approx_mib + " MiB)" : fallback
    }
    readonly property var readyEditors: installations.filter(i => i.status === "ready")

    // Latest "Stage:" line of the running operation, for the status bar.
    function updateStage() {
        const lines = (service.log || "").split("\n")
        for (let n = lines.length - 1; n >= 0; --n)
            if (lines[n].startsWith("Stage: ")) { stage = lines[n].slice(7); return }
        stage = ""
    }
    function opened(entry) {
        if (!entry.opened_at) return "Never opened"
        const days = Math.floor((Date.now() / 1000 - entry.opened_at) / 86400)
        if (days <= 0) return "Opened today"
        if (days === 1) return "Opened yesterday"
        if (days < 30) return "Opened " + days + " days ago"
        return "Opened " + new Date(entry.opened_at * 1000).toLocaleDateString()
    }
    function installationMissing(entry) {
        return !!entry.installation && !readyEditors.some(i => i.id === entry.installation)
    }
    // Why Open is unavailable, or "" when it isn't.
    function openBlocked(entry) {
        if (!entry.available) return entry.error || "The project folder is unavailable."
        if (!entry.installation) return defaultEditor() ? "" : "Install an editor first."
        if (installationMissing(entry)) return "Its editor (" + friendly(entry.installation) + ") is not installed. Change editor to pick another."
        if (entry.active) {
            const owner = entry.owner || {}
            return "Open in " + (owner.installation ? window.selection(owner.installation) : "another editor") + "."
        }
        return ""
    }

    function perform(command, args) {
        error = ""
        notice = ""
        noticeTimer.restart()
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
    // "release-0.2.0" as a person says it; used when the installation is gone.
    function friendly(identity) {
        const text = String(identity || "")
        if (text.startsWith("release-")) return "Blockloom " + text.slice(8)
        if (text.startsWith("dev-")) return "Development build"
        return text
    }
    function selection(identity) {
        for (let installation of installations)
            if (installation.id === identity) return label(installation)
        return identity ? friendly(identity) + " (not installed)" : "No editor chosen yet"
    }
    // What a project with no editor will open with: the newest release, else any ready one.
    function defaultEditor() {
        const releases = readyEditors.filter(i => i.kind === "release")
        const pool = releases.length ? releases : readyEditors
        if (pool.length === 0) return null
        const key = i => String(i.version).split(/[.-]/).map(n => /^\d+$/.test(n) ? parseInt(n) : 0)
        return pool.slice().sort((a, b) => {
            const x = key(a), y = key(b)
            for (let n = 0; n < Math.max(x.length, y.length); ++n) {
                const d = (x[n] || 0) - (y[n] || 0)
                if (d !== 0) return d
            }
            return 0
        })[pool.length - 1]
    }
    function openable(entry) { return !!entry.available && !entry.active && openBlocked(entry) === "" }
    function showFolderOf(entry) {
        if (!service.showFolder(entry.path)) error = "The project folder could not be opened."
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
                } else if (command === "modules") {
                    window.moduleInfo = result
                } else if (command === "projects") {
                    window.projects = result
                    if (!service.smokeTest() && window.moduleInfo.modules === undefined)
                        Qt.callLater(() => { if (!service.busy) service.run("modules", []) })
                    if (service.smokeTest()) {
                        if ((service.smokePage() === "bind" || service.smokePage() === "upgrade-bind") && !window.smokeBindingDone && result.length > 0) {
                            window.smokeBindingDone = true
                            window.choose(result[0])
                            versionChooser.currentIndex = versionDialog.choices.findIndex(i => i.id === "release-0.1.0")
                            versionDialog.accept()
                            return
                        }
                        if (service.smokePage() === "installations") tabs.currentIndex = 1
                        if (service.smokePage() === "no-match") search.text = "zzz-nothing"
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
                        if (service.smokePage() === "dev-install") {
                            devJava.checked = false; devSdk.checked = false; devNdk.checked = false; devTargets.checked = false
                            devDialog.accept()
                        }
                        else if (!service.smokePage().startsWith("checkbox-"))
                            service.smokeReady(window.projects.length, window.installations.length)
                    }
                } else if (command === "remember") {
                    // Only ask about the editor when the project has none yet.
                    if (!result.installation) window.choose(result)
                    window.refresh()
                } else if (command === "open" || command === "new-project") {
                    window.notice = result.opened
                        ? (command === "open" ? "Project opened in " : "New project created in ") + window.selection(result.installation)
                            + (result.reused ? ", which was already running." : ".")
                            + (result.bound ? " It was set to this editor." : "")
                        : (result.reused ? "The running editor is still opening the project."
                                         : "The editor started and is still opening the project.")
                    window.refresh()
                } else if (command === "focus") {
                    window.notice = result.acknowledged
                        ? "The editor heard you. If its window didn't come forward, your desktop may be holding it back."
                        : "No editor window answered. Switch to it from your taskbar, or close it there first."
                } else if (command === "backups") {
                    window.backupRows = result
                } else if (command === "restore-backup") {
                    window.notice = "Project restored. What was there before is kept in its backups."
                    window.refresh()
                } else if (command === "forget" || command === "relocate") {
                    window.notice = command === "forget" ? "Removed from the list. The folder was not touched." : "Project folder located."
                    window.refresh()
                } else {
                    if (command === "bind" && result.backup) window.lastBackup = result.backup.path
                    window.notice = command === "open" ? "Editor launched."
                        : command === "bind" ? "Project editor saved." + (result.backup ? " A backup was kept (Backups in the project's menu)." : "")
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
        running: !service.busy && !service.smokeTest() && !versionDialog.visible && !newProjectDialog.visible && !devDialog.visible && !installDialog.visible && !settingsDialog.visible && !toolsDialog.visible && !deleteDialog.visible && !window.installMenuOpen
        onTriggered: service.poll("installations")
    }
    Connections {
        target: service
        // Background refreshes: no busy state, so buttons never flicker.
        function onPolled(command, ok, response) {
            if (!ok) return
            try {
                const result = JSON.parse(response)
                if (command === "installations") {
                    window.installations = result
                    service.poll("projects")
                } else if (command === "projects") {
                    window.projects = result
                }
            } catch (e) {}
        }
    }
    // A result line fades after a while; errors stay until the next action.
    Timer {
        id: noticeTimer
        interval: 8000
        running: window.notice.length > 0 && window.error.length === 0
        onTriggered: window.notice = ""
    }
    Connections { target: service; function onLogChanged() { window.updateStage() } }
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
                Label { text: "Blockloom Hub"; font.pixelSize: 24; font.bold: true }
            }
            Item { Layout.fillWidth: true }
            HubButton {
                text: "Show backup"
                visible: window.lastBackup.length > 0
                onClicked: if (!service.showBackup(window.lastBackup)) window.error = "The backup folder could not be opened."
            }
            HubButton { iconName: "settings"; text: "Settings"; enabled: !service.busy; onClicked: settingsDialog.open() }
            HubButton { iconName: "refresh-cw"; text: "Refresh"; enabled: !service.busy; onClicked: refresh() }
        }
        TabBar {
            id: tabs
            spacing: 4
            background: null
            HubTab { text: "Projects" }
            HubTab { text: "Installations" }
        }
        Label {
            visible: window.error.length > 0 || window.notice.length > 0
            text: window.error || window.notice
            color: window.error ? Theme.danger : Theme.running
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
                    HubButton { objectName: "newProject"; primary: window.projects.length > 0; text: "New project"; enabled: !service.busy; onClicked: newProjectDialog.open() }
                    HubButton { text: "Add existing project"; enabled: !service.busy; onClicked: projectFolder.open() }
                    HubField { id: search; objectName: "searchField"; visible: window.projects.length > 4; Layout.fillWidth: true; placeholderText: "Search projects" }
                    Item { visible: window.projects.length <= 4; Layout.fillWidth: true }
                }
                ColumnLayout {
                    visible: window.projects.length === 0
                    Layout.topMargin: 30
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 10
                    Label { Layout.alignment: Qt.AlignHCenter; text: "Make your first game"; font.pixelSize: 20; font.bold: true }
                    Label {
                        Layout.alignment: Qt.AlignHCenter
                        text: window.readyEditors.length === 0
                            ? "Install an editor first, then create a project."
                            : "Create a new project, or add one you already have."
                        color: Theme.textDim
                    }
                    HubButton {
                        Layout.alignment: Qt.AlignHCenter
                        primary: true
                        objectName: "emptyPrimary"
                        text: window.readyEditors.length === 0 ? "Get an editor" : "New project"
                        onClicked: {
                            if (window.readyEditors.length === 0) { releaseDialog.open(); perform("check-releases", []) }
                            else newProjectDialog.open()
                        }
                    }
                }
                Label {
                    objectName: "noSearchMatch"
                    visible: window.projects.length > 4 && search.text.length > 0
                        && window.projects.filter(e => window.matches(e)).length === 0
                    text: "No project matches “" + search.text + "”."
                    color: Theme.textDim
                    Layout.topMargin: 20
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 0
                    model: window.projects.filter(entry => window.matches(entry))
                    ScrollBar.vertical: HubScrollBar { id: projectsScrollBar }
                    delegate: Rectangle {
                        id: projectCard
                        required property var modelData
                        width: ListView.view.width - (projectsScrollBar.visible ? projectsScrollBar.width + 8 : 0)
                        height: projectRow.implicitHeight + 24
                        color: cardHover.hovered && window.openable(modelData) ? Theme.hover : "transparent"
                        Rectangle { anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; height: 1; color: Theme.borderSoft }
                        border.color: activeFocus ? Theme.accent : "transparent"
                        activeFocusOnTab: true
                        HoverHandler { id: cardHover }
                        Keys.onReturnPressed: if (window.openable(modelData) && !service.busy) perform("open", [modelData.path])
                        Keys.onEnterPressed: if (window.openable(modelData) && !service.busy) perform("open", [modelData.path])
                        TapHandler {
                            acceptedButtons: Qt.RightButton
                            onTapped: if (modelData.available) rowMenu.popup()
                        }
                        Menu {
                            id: rowMenu
                            onOpened: window.installMenuOpen = true
                            onClosed: window.installMenuOpen = false
                            MenuItem {
                                text: "Open"
                                enabled: !service.busy && window.openable(modelData)
                                onTriggered: perform("open", [modelData.path])
                            }
                            MenuItem { text: "Show in folder"; onTriggered: window.showFolderOf(modelData) }
                            MenuItem {
                                text: "Backups..."
                                enabled: !service.busy
                                onTriggered: { window.backupsTarget = modelData; window.backupRows = []; backupsDialog.open(); perform("backups", [modelData.path]) }
                            }
                            MenuSeparator {}
                            MenuItem { text: "Remove from list"; enabled: !service.busy; onTriggered: perform("forget", [modelData.path]) }
                        }
                        RowLayout {
                            id: projectRow
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 16
                            ColumnLayout {
                                Layout.fillWidth: true
                                // Clicking the words opens the project, the way an editor card does.
                                TapHandler {
                                    enabled: window.openable(modelData) && !service.busy
                                    onTapped: perform("open", [modelData.path])
                                }
                                Label { text: modelData.name || "Unavailable project"; font.bold: true; font.pixelSize: 17 }
                                Label { text: window.displayPath(modelData.path); color: Theme.textDim; elide: Text.ElideMiddle; Layout.fillWidth: true }
                                Label {
                                    text: modelData.available
                                        ? (modelData.mode === "ThreeD" ? "3D" : "2D") + "  ·  "
                                            + (modelData.installation ? window.selection(modelData.installation)
                                                : window.defaultEditor() ? "Will open with " + window.label(window.defaultEditor()) : "No editor chosen yet")
                                            + "  ·  " + window.opened(modelData)
                                        : window.opened(modelData)
                                    color: modelData.available && modelData.installation ? Theme.accent : Theme.textDim
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                                Label {
                                    objectName: "projectReason"
                                    visible: text.length > 0
                                    text: window.openBlocked(modelData)
                                    color: modelData.available ? Theme.warning : Theme.danger
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                            }
                            HubButton {
                                visible: !modelData.available
                                objectName: "locateFolder"
                                text: "Locate folder"
                                enabled: !service.busy
                                onClicked: { window.selectedProject = modelData; locateFolder.open() }
                            }
                            HubButton {
                                visible: !modelData.available
                                objectName: "removeFromList"
                                text: "Remove from list"
                                enabled: !service.busy
                                onClicked: perform("forget", [modelData.path])
                            }
                            HubButton {
                                visible: modelData.available
                                text: "Change editor"
                                enabled: !service.busy && !modelData.active
                                onClicked: window.choose(modelData)
                            }
                            HubButton {
                                visible: modelData.available && !modelData.active
                                objectName: "openProject"
                                primary: true
                                text: "Open"
                                enabled: !service.busy && window.openBlocked(modelData) === ""
                                onClicked: perform("open", [modelData.path])
                            }
                            HubButton {
                                visible: modelData.available && modelData.active
                                objectName: "switchToEditor"
                                primary: true
                                text: "Switch to editor"
                                enabled: !service.busy
                                onClicked: perform("focus", [modelData.path])
                            }
                            HubButton {
                                visible: modelData.available
                                objectName: "projectMenu"
                                text: "⋮"
                                font.pixelSize: 18
                                font.bold: true
                                implicitWidth: 44
                                Accessible.name: "Project options for " + modelData.name
                                onClicked: rowMenu.popup(x, y + height)
                            }
                        }
                    }
                }
            }
            ColumnLayout {
                spacing: 12
                RowLayout {
                    HubButton { primary: true; text: "Get a release"; enabled: !service.busy; onClicked: { releaseDialog.open(); perform("check-releases", []) } }
                    HubButton {
                        text: "More…"
                        enabled: !service.busy
                        onClicked: moreMenu.open()
                        Menu {
                            id: moreMenu
                            y: parent.height
                            onOpened: window.installMenuOpen = true
                            onClosed: window.installMenuOpen = false
                            MenuItem { text: "Import release bundle…"; onTriggered: { window.selectedRelease = {}; installDialog.open() } }
                            MenuItem { text: "Add local repository…"; onTriggered: repoFolder.open() }
                        }
                    }
                }
                Label {
                    visible: window.installations.length === 0
                    text: "No editors yet. Choose Get a release to download one."
                    color: Theme.textDim
                    Layout.topMargin: 30
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 0
                    model: window.installations
                    ScrollBar.vertical: HubScrollBar { id: installationsScrollBar }
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width - (installationsScrollBar.visible ? installationsScrollBar.width + 8 : 0)
                        height: installRow.implicitHeight + 24
                        color: "transparent"
                        Rectangle { anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; height: 1; color: Theme.borderSoft }
                        RowLayout {
                            id: installRow
                            anchors.fill: parent
                            anchors.margins: 12
                            ColumnLayout {
                                Layout.fillWidth: true
                                Label { text: window.label(modelData); font.weight: Font.DemiBold; font.pixelSize: 14 }
                                Label {
                                    text: window.displayPath(modelData.repo || modelData.target)
                                    color: Theme.textDim
                                    elide: Text.ElideMiddle
                                    Layout.fillWidth: true
                                }
                                Label {
                                    text: modelData.status === "ready"
                                        ? (modelData.running ? "In use" : "Ready")
                                            + (Object.keys(modelData.tools).filter(t => t !== "rust").length > 0
                                                ? "  ·  Android: " + Object.keys(modelData.tools).filter(t => t !== "rust").join(", ") : "")
                                        : "Build this repository to create its installation."
                                    color: Theme.accent
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                            }
                            RowLayout {
                                spacing: 8
                                HubButton {
                                    visible: !!modelData.prepared
                                    text: "Install new build"
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
                                    iconName: "ellipsis"
                                    implicitWidth: 38
                                    Accessible.name: "Installation options for " + window.label(modelData)
                                    Layout.alignment: Qt.AlignRight
                                    enabled: !service.busy
                                    onClicked: installMenu.open()
                                    StitchMenu {
                                        id: installMenu
                                        y: parent.height
                                        onOpened: window.installMenuOpen = true
                                        onClosed: window.installMenuOpen = false
                                        StitchMenuItem {
                                            text: "Add modules..."
                                            enabled: modelData.status === "ready" && !service.busy && !modelData.running
                                            onTriggered: {
                                                toolsDialog.target = modelData
                                                window.toolsTarget = modelData
                                                toolsDialog.open()
                                            }
                                        }
                                        StitchMenuItem {
                                            text: "Open file location"
                                            enabled: !!modelData.path
                                            onTriggered: { if (!service.showFolder(modelData.path)) window.error = "The installation folder could not be opened." }
                                        }
                                        MenuSeparator {}
                                        StitchMenuItem {
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
            Label { objectName: "statusText"; text: service.cancelling ? "Stopping..." : service.busy ? (window.stage || "Working...") : "Ready"; color: Theme.textDim; elide: Text.ElideRight; Layout.maximumWidth: 600 }
            Item { Layout.fillWidth: true }
            HubButton { text: service.cancelling ? "Stopping..." : "Cancel"; visible: service.canCancel; enabled: !service.cancelling; onClicked: service.cancel() }
            HubButton { flat: true; text: "Details"; onClicked: logDialog.open() }
        }
    }

    FolderDialog { id: projectFolder; title: "Select a Blockloom project"; onAccepted: perform("remember", [service.localPath(selectedFolder)]) }
    FolderDialog {
        id: locateFolder
        title: "Locate the project folder"
        onAccepted: perform("relocate", [window.selectedProject.path, service.localPath(selectedFolder)])
    }
    FolderDialog { id: newProjectLocation; title: "Where to keep the project"; onAccepted: npLocation.text = service.localPath(selectedFolder) }
    HubDialog {
        id: newProjectDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 560)
        title: "New project"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        property string mode: "TwoD"
        property string sample: ""
        readonly property string folder: {
            const cleaned = npName.text.replace(/[\/\\:*?"<>|]/g, "-").trim().replace(/^\.+|\.+$/g, "").trim()
            return cleaned === "" ? "Project" : cleaned
        }
        onOpened: {
            npName.text = "My Game"; mode = "TwoD"; sample = ""
            if (npLocation.text.length === 0) npLocation.text = ""
            npEditor.currentIndex = Math.max(0, npEditor.count - 1)
        }
        onAccepted: {
            const args = [npName.text.trim(), "--mode", mode]
            if (npEditor.currentIndex >= 0) args.push("--installation", window.readyEditors[npEditor.currentIndex].id)
            if (npLocation.text.trim().length > 0) args.push("--location", npLocation.text.trim())
            if (sample.length > 0) args.push("--sample", sample)
            perform("new-project", args)
        }
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Create"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => npName.text.trim().length > 0 && npEditor.count > 0 && !service.busy)
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 8
            Label { text: "Name"; color: Theme.textDim }
            HubField { id: npName; objectName: "npName"; Layout.fillWidth: true }
            Label { text: "Where to keep it (leave empty for the default)"; color: Theme.textDim }
            RowLayout {
                HubField { id: npLocation; Layout.fillWidth: true; placeholderText: "~/Blockloom/projects" }
                HubButton { text: "Browse"; onClicked: newProjectLocation.open() }
            }
            Label {
                Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; color: Theme.textDim
                text: "The project gets a folder of its own: " + (npLocation.text.trim() || "~/Blockloom/projects") + "/" + newProjectDialog.folder
            }
            Label { text: "Dimension"; color: Theme.textDim }
            RowLayout {
                HubButton { text: "2D"; primary: newProjectDialog.mode === "TwoD"; onClicked: newProjectDialog.mode = "TwoD" }
                HubButton { text: "3D"; primary: newProjectDialog.mode === "ThreeD"; onClicked: newProjectDialog.mode = "ThreeD" }
            }
            Label {
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim
                text: (newProjectDialog.mode === "TwoD" ? "Sprites and flat physics, measured in pixels." : "Meshes and 3D physics, measured in metres.") + " A project can switch later."
            }
            Label { text: "Start from"; color: Theme.textDim }
            RowLayout {
                HubButton { text: "Empty"; primary: newProjectDialog.sample === ""; onClicked: newProjectDialog.sample = "" }
                HubButton { text: "Physics playground"; primary: newProjectDialog.sample !== ""; onClicked: newProjectDialog.sample = "physics-playground" }
            }
            Label { text: "Editor"; color: Theme.textDim; visible: window.readyEditors.length > 1 }
            HubCombo {
                id: npEditor
                visible: window.readyEditors.length > 1
                Layout.fillWidth: true
                model: window.readyEditors.map(i => window.label(i))
            }
            Label {
                visible: window.readyEditors.length === 0
                color: Theme.warning; wrapMode: Text.Wrap; Layout.fillWidth: true
                text: "No editor is installed yet. Get a release on the Installations tab first."
            }
            Label {
                visible: window.readyEditors.length === 1
                color: Theme.textDim; text: "Editor: " + (window.readyEditors.length ? window.label(window.readyEditors[0]) : "")
            }
        }
    }
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
        title: (window.selectedProject.installation ? "Change editor for " : "Choose an editor for ") + (window.selectedProject.name || "project")
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
        }
        Component.onCompleted: standardButton(Dialog.Save).enabled = Qt.binding(() => versionChooser.currentIndex >= 0 && !service.busy)
        ColumnLayout {
            anchors.fill: parent
            Label {
                visible: !!window.selectedProject.installation
                text: "Current: " + window.selection(window.selectedProject.installation); wrapMode: Text.Wrap; Layout.fillWidth: true
            }
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
                    : !window.selectedProject.installation ? "This project opens with the editor you pick here."
                    : "Opening with a newer editor may update the project's files, so a backup is kept first. You can restore it from the project's Backups."
                color: Theme.textDim
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
            Label { text: window.error || window.notice; visible: text.length > 0; color: window.error ? Theme.danger : Theme.running; wrapMode: Text.Wrap; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: window.availableReleases
                spacing: 0
                ScrollBar.vertical: HubScrollBar { id: releasesScrollBar }
                delegate: Rectangle {
                    required property var modelData
                    width: ListView.view.width - (releasesScrollBar.visible ? releasesScrollBar.width + 8 : 0)
                    height: 64
                    color: "transparent"
                    Rectangle { anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; height: 1; color: Theme.borderSoft }
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        ColumnLayout {
                            Layout.fillWidth: true
                            Label { text: "Blockloom " + modelData.version; font.bold: true }
                            Label { text: (modelData.channel || "release") + "  ·  " + (modelData.size / 1024 / 1024).toFixed(0) + " MiB"; color: Theme.textDim }
                        }
                        HubButton {
                            text: modelData.installed ? "Installed" : "Install"
                            enabled: !service.busy && !modelData.installed
                            onClicked: { window.selectedRelease = modelData; releaseDialog.close(); installDialog.open() }
                        }
                    }
                }
            }
            Label { text: "Installing a release keeps existing editors and project selections."; color: Theme.textDim; wrapMode: Text.Wrap; Layout.fillWidth: true }
            Label {
                visible: window.error.indexOf("GitHub CLI") >= 0
                text: "Releases come from GitHub through its command-line tool. Install it and sign in, or open Settings to use a release catalog URL instead."
                color: Theme.warning; wrapMode: Text.Wrap; Layout.fillWidth: true
            }
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
            Label { text: "Uses your gh auth login session."; visible: settingsGithub.checked; color: Theme.textDim }
            HubField { id: settingsUrl; visible: !settingsGithub.checked; placeholderText: "HTTPS release catalog URL"; Layout.fillWidth: true }
            Label {
                text: "The catalog URL must serve the release catalog JSON, not a repository page."
                visible: !settingsGithub.checked
                color: Theme.textDim
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label { text: window.error || window.notice; visible: text.length > 0; color: window.error ? Theme.danger : Theme.running; wrapMode: Text.Wrap; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight }
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
            HubCheck {
                objectName: "allAndroidInstall"
                text: "All Android build tools"
                checked: javaCheck.checked && sdkCheck.checked && ndkCheck.checked && targetsCheck.checked
                onClicked: { javaCheck.checked = checked; sdkCheck.checked = checked; ndkCheck.checked = checked; targetsCheck.checked = checked }
            }
            HubCheck { id: javaCheck; text: window.moduleText("java", "Java"); enabled: true }
            HubCheck { id: sdkCheck; text: window.moduleText("android-sdk", "Android SDK"); enabled: true }
            HubCheck { id: ndkCheck; text: window.moduleText("android-ndk", "Android NDK"); enabled: true }
            HubCheck { id: targetsCheck; text: window.moduleText("android-rust-targets", "Android Rust targets (ARM64 and x86-64)"); enabled: true }
            Label {
                text: window.selectedRelease.version
                    ? "Release installs start with no optional modules; sizes are approximate. Selected tools are downloaded with this release. APK builds need Java and the SDK; native Android builds also need the NDK and Rust targets."
                    : "Selected tools must be included in the bundle. APK builds need Java and the SDK; native Android builds also need the NDK and Rust targets."
                color: Theme.textDim
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
            Label { text: "Installed modules are checked and cannot be changed. New modules (Java, Android SDK, Android NDK, Android Rust targets) are downloaded with the versions this editor pins."; color: Theme.textDim; wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubCheck { id: addJava; text: "Java" }
            HubCheck { id: addSdk; text: "Android SDK" }
            HubCheck { id: addNdk; text: "Android NDK" }
            HubCheck { id: addTargets; text: "Android Rust targets (ARM64 and x86-64)" }
            Label {
                text: "Everything is already installed."
                visible: !addJava.enabled && !addSdk.enabled && !addNdk.enabled && !addTargets.enabled
                color: Theme.running
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
                color: Theme.danger
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label {
                visible: deleteDialog.blockers.length === 0
                text: "Projects using it must be assigned another editor first."
                color: Theme.textDim
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
            if (service.smokeTest() && service.smokePage().startsWith("checkbox-")) {
                devSdk.checked = service.smokePage() === "checkbox-checked"
                service.smokeReady(window.projects.length, window.installations.length)
            }
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
            Label { text: "Build complete. Choose what to include in this installation."; color: Theme.text; wrapMode: Text.Wrap; Layout.fillWidth: true }
            HubCheck { text: "Rust toolchain (required)"; checked: true; enabled: false }
            HubCheck {
                text: "All Android build tools"
                checked: devJava.checked && devSdk.checked && devNdk.checked && devTargets.checked
                onClicked: { devJava.checked = checked; devSdk.checked = checked; devNdk.checked = checked; devTargets.checked = checked }
            }
            HubCheck { id: devJava; text: window.moduleText("java", "Java") }
            HubCheck { id: devSdk; objectName: "smokeCheckbox"; text: window.moduleText("android-sdk", "Android SDK") }
            HubCheck { id: devNdk; text: window.moduleText("android-ndk", "Android NDK") }
            HubCheck {
                id: devTargets
                text: window.moduleText("android-rust-targets", "Android Rust targets (ARM64 and x86-64)")
            }
            Label {
                text: "Development installs start with every module selected; release installs start with none. Sizes are approximate. Selected tools are downloaded and bundled with this editor. Versions follow the repository's Android requirements. Local tool installations are not used."
                color: Theme.textDim
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }
    HubDialog {
        id: backupsDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 640)
        height: Math.min(window.height - 80, 480)
        title: "Backups of " + (window.backupsTarget.name || "project")
        modal: true
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label {
                visible: window.backupRows.length === 0
                objectName: "noBackups"
                text: service.busy ? "Looking for backups..." : "No backups yet. One is kept each time this project's editor changes."
                color: Theme.textDim
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 8
                model: window.backupRows
                ScrollBar.vertical: HubScrollBar { id: backupsScrollBar }
                delegate: Rectangle {
                    required property var modelData
                    width: ListView.view.width - (backupsScrollBar.visible ? backupsScrollBar.width + 8 : 0)
                    height: 64
                    radius: 6
                    color: Theme.window
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        ColumnLayout {
                            Layout.fillWidth: true
                            Label { text: new Date(modelData.created_at * 1000).toLocaleString(); font.bold: true }
                            Label { text: (modelData.size / 1024 / 1024).toFixed(1) + " MiB"; color: Theme.textDim }
                        }
                        HubButton {
                            text: "Show"
                            onClicked: if (!service.showBackup(modelData.path)) window.error = "The backup folder could not be opened."
                        }
                        HubButton {
                            objectName: "restoreBackup"
                            text: "Restore"
                            enabled: !service.busy && !window.backupsTarget.active
                            onClicked: { window.restoreTarget = modelData; restoreDialog.open() }
                        }
                    }
                }
            }
            Label { text: window.error || window.notice; visible: text.length > 0; color: window.error ? Theme.danger : Theme.running; wrapMode: Text.Wrap; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight }
        }
    }
    HubDialog {
        id: restoreDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 60, 520)
        title: "Restore this backup?"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: perform("restore-backup", [window.backupsTarget.path, window.restoreTarget.name])
        Component.onCompleted: {
            standardButton(Dialog.Ok).text = "Restore"
            standardButton(Dialog.Ok).enabled = Qt.binding(() => !service.busy)
        }
        Label {
            width: parent.width
            wrapMode: Text.Wrap
            text: "The project goes back to how it was on "
                + (window.restoreTarget.created_at ? new Date(window.restoreTarget.created_at * 1000).toLocaleString() : "")
                + ", including which editor it uses. What is there now is kept as a backup first."
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
            background: Rectangle { color: Theme.field; radius: 6; border.color: Theme.border }
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
                    color: Theme.text
                    selectionColor: Theme.accent
                    selectedTextColor: "white"
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
                    color: Theme.textDim
                    font.pixelSize: 13
                    x: 14
                    y: 2
                    visible: logViewport.count === 0
                }
            }
        }
    }
}
