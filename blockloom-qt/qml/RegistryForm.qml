import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Blockstitch's PropertyForm with the editor's own color swatch, asset box
// and actor picker: the one form every schema-built card is drawn with.
PropertyForm {
    id: root
    required property var app
    // Actors an `actor` value can name, as [{value, label}].
    property var actors: app.appState && app.appState.project
        ? [{ value: "", label: "nothing" }].concat(app.appState.project.actors.map(a => ({ value: a.id, label: a.name })))
        : []
    Layout.fillWidth: true
    host: root
    editors: ({ color: colorEditor, asset: assetEditor, actor: actorEditor })

    Component {
        id: colorEditor
        RowLayout {
            property var field
            property var value
            signal edited(var next)
            ColorField {
                objectName: "registry-color"
                value: String(parent.value || "#FFFFFF").slice(0, 7)
                // Keep an alpha pair when only the swatch is picked.
                onPicked: c => parent.edited(String(parent.value).length === 9 ? c + String(parent.value).slice(7) : c)
            }
            Item { Layout.fillWidth: true }
        }
    }
    Component {
        id: assetEditor
        AssetField {
            property var field
            property var host
            signal edited(var next)
            objectName: "registry-asset"
            app: root.app
            accept: !field || !field.kind || field.kind === "any" ? [] : [field.kind === "sound" ? "audio" : field.kind]
            onCommitted: p => edited(p)
        }
    }
    Component {
        id: actorEditor
        ChoiceField {
            property var field
            signal edited(var next)
            objectName: "registry-actor"
            options: root.actors
            placeholder: "nothing"
            onChosen: v => edited(v)
        }
    }
}
