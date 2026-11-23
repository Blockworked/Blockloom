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
        ? [{ value: "", label: "nothing" }].concat((app.appState.project.actors || []).map(a => ({ value: a.id, label: a.name })))
        : []
    // A plugin's record: a `choice` with a `source` lists what its preview answered.
    property string plugin: ""
    Layout.fillWidth: true
    host: root
    editors: ({ color: colorEditor, asset: assetEditor, actor: actorEditor, choice: choiceEditor })

    function choices(field) {
        const plugins = app.appState ? app.appState.plugins : null;
        const live = field && field.source && root.plugin && plugins && plugins.choices && plugins.choices[root.plugin]
            ? plugins.choices[root.plugin][field.source] : null;
        return live && live.length ? live : (field && field.options ? field.options : []);
    }

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
    Component {
        id: choiceEditor
        StitchComboBox {
            property var field
            property var value
            property var host
            signal edited(var next)
            readonly property var options: root.choices(field)
            objectName: "registry-choice"
            implicitHeight: 30; font.pixelSize: 12
            model: options
            currentIndex: options.indexOf(value)
            onActivated: i => { const v = options[i]; if (v !== value) edited(v); }
        }
    }
}
