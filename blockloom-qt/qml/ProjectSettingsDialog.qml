import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

BwDialog {
    id: root
    required property var app
    property string page: "project"
    title: page === "publishing" ? "Publishing / App Info" : page === "android" ? "Publishing / Android" : "Project settings / General"
    standardButtons: Dialog.Close
    width: 560; height: Math.min(parent ? parent.height - 80 : 800, 640)
    function showPage(next) { page = next; open(); }
    SettingsFields { anchors.fill: parent; app: root.app; page: root.page }
}
