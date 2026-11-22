import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// View controls for the viewport: what is shown, at what size and zoom, and how
// drags snap. Wraps instead of forcing the centre column wide.
Flow {
    id: root
    required property var designer
    spacing: 14
    readonly property var sizes: [[960,720],[1280,720],[1920,1080],[720,1280],[1170,2532],[2560,1440]]
    readonly property var sizeNames: ["960 x 720", "1280 x 720 (720p)", "1920 x 1080 (1080p)", "720 x 1280 (portrait)", "1170 x 2532 (phone)", "2560 x 1440 (1440p)"]

    component Group: RowLayout {
        property string label: ""
        spacing: 6
        Text { visible: parent.label !== ""; text: parent.label; color: Theme.textDim; font.pixelSize: 11; Layout.alignment: Qt.AlignVCenter }
    }

    Group {
        label: "Editing"
        StitchComboBox {
            objectName: "interfaceTarget"
            implicitWidth: 170; implicitHeight: 30; font.pixelSize: 12
            model: ["Scene interface"].concat(root.designer.targets.map(p => p.split("/").pop()))
            currentIndex: root.designer.target ? root.designer.targets.indexOf(root.designer.target) + 1 : 0
            enabled: !root.designer.gesture
            onActivated: root.designer.target = currentIndex > 0 ? root.designer.targets[currentIndex - 1] : ""
            Accessible.name: "Interface or screen file to edit"
        }
    }
    Group {
        label: "Screen"
        StitchComboBox {
            objectName: "interfaceScreen"
            implicitWidth: 150; implicitHeight: 30; font.pixelSize: 12
            model: ["All screens"].concat(root.designer.screens)
            currentIndex: root.designer.screenId ? root.designer.screens.indexOf(root.designer.screenId) + 1 : 0
            onActivated: root.designer.screenId = currentIndex > 0 ? root.designer.screens[currentIndex - 1] : ""
            Accessible.name: "Screen to show"
        }
    }
    Group {
        label: "Preview"
        StitchComboBox {
            objectName: "interfacePreviewSize"
            implicitWidth: 170; implicitHeight: 30; font.pixelSize: 12
            model: root.sizeNames
            currentIndex: {
                for (let i = 0; i < root.sizes.length; ++i)
                    if (root.sizes[i][0] === root.designer.previewWidth && root.sizes[i][1] === root.designer.previewHeight) return i;
                return -1;
            }
            displayText: currentIndex >= 0 ? currentText : root.designer.previewWidth + " x " + root.designer.previewHeight
            onActivated: root.designer.setPreviewSize(root.sizes[currentIndex][0], root.sizes[currentIndex][1])
            Accessible.name: "Preview size"
        }
    }
    Group {
        label: "Zoom"
        IconButton { objectName: "interfaceZoomOut"; iconName: "zoom-out"; tip: "Zoom out"; implicitWidth: 30; implicitHeight: 30; onClicked: root.designer.zoomBy(1 / 1.25) }
        Text { objectName: "interfaceZoomLabel"; text: Math.round(root.designer.zoom * 100) + "%"; color: Theme.text; font.pixelSize: 12; Layout.preferredWidth: 38; horizontalAlignment: Text.AlignHCenter }
        IconButton { objectName: "interfaceZoomIn"; iconName: "zoom-in"; tip: "Zoom in"; implicitWidth: 30; implicitHeight: 30; onClicked: root.designer.zoomBy(1.25) }
        StitchButton { objectName: "interfaceFit"; text: "Fit"; implicitHeight: 30; font.pixelSize: 12; onClicked: root.designer.fitView() }
        StitchButton { objectName: "interfaceActualSize"; text: "100%"; implicitHeight: 30; font.pixelSize: 12; onClicked: root.designer.actualSize() }
    }
    Group {
        label: "Show"
        StitchCheckBox { objectName: "interfaceSafeArea"; text: "Safe area"; checked: root.designer.showSafeArea; onToggled: root.designer.showSafeArea = checked }
        StitchCheckBox { objectName: "interfaceReference"; text: "Reference size"; checked: root.designer.showReference; onToggled: root.designer.showReference = checked }
        StitchCheckBox { objectName: "interfaceRulers"; text: "Rulers"; checked: root.designer.showRulers; onToggled: root.designer.showRulers = checked }
        StitchCheckBox { objectName: "interfaceWorld"; text: "World behind"; checked: root.designer.showWorld; onToggled: root.designer.showWorld = checked }
        StitchButton { objectName: "interfaceClearGuides"; text: "Clear guides"; visible: root.designer.guides.length > 0; implicitHeight: 30; font.pixelSize: 12; onClicked: root.designer.guides = [] }
    }
    Group {
        label: "Snap"
        StitchCheckBox { text: "Grid"; checked: root.designer.snapGrid; onToggled: root.designer.snapGrid = checked }
        NumberField {
            objectName: "interfaceSnapStep"
            Layout.fillWidth: false; Layout.preferredWidth: 52; Layout.minimumWidth: 52
            enabled: root.designer.snapGrid
            value: root.designer.snapStep; fallback: 8
            onCommitted: n => root.designer.snapStep = Math.max(1, Math.min(256, n))
        }
        StitchCheckBox { text: "Edges and centres"; checked: root.designer.snapAlign; onToggled: root.designer.snapAlign = checked }
    }
}
