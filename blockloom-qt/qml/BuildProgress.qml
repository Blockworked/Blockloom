import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

ColumnLayout {
    id: root
    required property var app
    property bool busy: false
    property bool cancelling: false
    property double jobId: 0
    property string step: ""
    property string detail: ""
    property bool polling: false
    property bool cancelSent: false
    signal finished(var result)
    signal failed(string message)
    signal cancelled()
    visible: busy
    spacing: 8

    function start(args) {
        if (busy) return;
        busy = true; cancelling = false; cancelSent = false; jobId = 0;
        step = "Preparing build..."; detail = "";
        app.invoke("start_build_game", args, status => {
            jobId = status.id;
            accept(status);
            if (cancelling) sendCancel();
        }, e => { busy = false; root.failed(String(e)); });
    }
    function accept(status) {
        if (status.id !== jobId || !busy) return;
        step = status.step || ""; detail = status.detail || "";
        if (status.state === "running") return;
        busy = false;
        if (status.state === "complete") root.finished(status);
        else if (status.state === "cancelled") root.cancelled();
        else root.failed(status.error || "Build failed.");
    }
    function poll() {
        if (!busy || !jobId || polling) return;
        const id = jobId;
        polling = true;
        app.invoke("build_job_status", { id: id }, status => {
            polling = false;
            if (id === jobId) accept(status);
        }, e => { polling = false; if (id === jobId && busy) detail = String(e); });
    }
    function cancel() {
        if (!busy || cancelling) return;
        cancelling = true; step = "Cancelling...";
        sendCancel();
    }
    function sendCancel() {
        if (!jobId || cancelSent || !busy) return;
        cancelSent = true;
        app.invoke("cancel_build_job", { id: jobId }, () => poll(), e => {
            cancelling = false; cancelSent = false; detail = String(e);
        });
    }

    Text { Layout.fillWidth: true; text: root.cancelling ? "Cancelling..." : root.step; color: Theme.text; font.pixelSize: 12; wrapMode: Text.WordWrap }
    ProgressBar { objectName: "buildProgressBar"; Layout.fillWidth: true; indeterminate: root.busy }
    Text { visible: !!root.detail; Layout.fillWidth: true; text: root.detail; color: Theme.textDim; font.pixelSize: 11; wrapMode: Text.WrapAnywhere; maximumLineCount: 3; elide: Text.ElideRight }
    BwButton { objectName: "cancelBuildButton"; text: root.cancelling ? "Cancelling..." : "Cancel"; enabled: root.busy && !root.cancelling; onClicked: root.cancel() }
    Timer { interval: 500; running: root.busy; repeat: true; onTriggered: root.poll() }
}
