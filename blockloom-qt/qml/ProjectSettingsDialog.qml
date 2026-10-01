import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

BwDialog {
    id: root
    required property var app
    property string page: "project"
    title: "Project settings"
    standardButtons: Dialog.Close
    width: Math.min(parent ? parent.width - 80 : 820, 820)
    height: Math.min(parent ? parent.height - 80 : 720, 720)
    function showPage(next) { page = next; open(); }
    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 180; Layout.fillHeight: true
            color: Theme.panel
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 8; spacing: 4
                BwButton { objectName: "settings-general"; Layout.fillWidth: true; text: "General";
                    primary: root.page === "project"; onClicked: root.page = "project" }
                Text { text: "Publishing"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold;
                    Layout.topMargin: 12; Layout.leftMargin: 8 }
                BwButton { objectName: "settings-app-info"; Layout.fillWidth: true; Layout.leftMargin: 12; text: "App Info";
                    primary: root.page === "publishing"; onClicked: root.page = "publishing" }
                BwButton { objectName: "settings-android"; Layout.fillWidth: true; Layout.leftMargin: 12; text: "Android";
                    primary: root.page === "android"; onClicked: root.page = "android" }
                Item { Layout.fillHeight: true }
            }
        }
        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.borderSoft }
        SettingsFields { objectName: "project-settings-fields"; Layout.fillWidth: true; Layout.fillHeight: true; app: root.app; page: root.page }
    }
}
