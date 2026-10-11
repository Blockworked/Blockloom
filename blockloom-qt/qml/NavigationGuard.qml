import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Asks what to do when leaving a project or opening one needs a decision: a
// save that failed, or a project the Hub set to another editor. Repeats the
// refused command with the user's answer.
StitchDialog {
    id: root
    required property var app
    property string command: ""
    property var args: ({})
    property string kind: ""
    property string message: ""
    title: kind === "wrong-editor" ? "Different editor" : "Couldn't save the project"
    standardButtons: Dialog.NoButton

    function ask(cmd, a, error) {
        command = cmd; args = a;
        kind = error.startsWith("save-failed:") ? "save-failed" : "wrong-editor";
        message = app.guardMessage(error);
        open();
    }
    function askQuit(error) {
        command = "__quit"; args = {}; kind = "quit-save-failed";
        message = error;
        open();
    }
    function again(extra) {
        if (command === "__quit") {
            close();
            if (extra.discard) { app.quitReady = true; app.close(); }
            else app.close();
            return;
        }
        const next = Object.assign({}, args, extra);
        close();
        app.invoke(command, next);
    }

    ColumnLayout {
        width: 460; spacing: 10
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 13; text: root.message }
        Text {
            visible: root.kind === "wrong-editor"
            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: "Opening may update the project file for this editor. A copy of the file is kept in the project's .blockloom/backups folder first. \"Open once\" leaves the project set to its current editor."
        }
        Text {
            visible: root.kind !== "wrong-editor"
            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: root.kind === "quit-save-failed"
                ? "The editor is still open. Fix the problem and retry, or quit and lose what couldn't be saved."
                : "The project is still open. Fix the problem and retry, or discard what couldn't be saved."
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight; spacing: 8
            StitchButton { text: "Cancel"; onClicked: root.close() }
            StitchButton { visible: root.kind !== "wrong-editor"; objectName: "discard-changes"; text: root.kind === "quit-save-failed" ? "Quit without saving" : "Discard changes"; onClicked: root.again({ discard: true }) }
            StitchButton { visible: root.kind !== "wrong-editor"; objectName: "retry-save"; primary: true; text: "Retry"; onClicked: root.again({}) }
            StitchButton { visible: root.kind === "wrong-editor"; objectName: "open-anyway"; text: "Open once"; onClicked: root.again({ other_editor: true }) }
            StitchButton { visible: root.kind === "wrong-editor"; objectName: "open-and-rebind"; primary: true; text: "Use this editor for the project"; onClicked: root.again({ other_editor: true, rebind: true }) }
        }
    }
}
