import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "PluginInspector"
    visible: true
    width: 600; height: 600
    when: windowShown
    QtObject { id: stubApp; property var assetDrag: ({ kind: "" }) }
    property var healthType: ({
        name: "com.example.health/Health", displayName: "Health", version: 2,
        fields: [
            { name: "hp", type: "int", min: 0, max: 1000, default: 100 },
            { name: "bar_color", type: "color", description: "The bar's tint." },
            { name: "tags", type: "list", item: { type: "text" }, max_len: 2 }
        ],
        defaults: { hp: 100, bar_color: "#44CC55", tags: [] }
    })
    Component { id: formFactory; Editor.PluginRecordForm { app: stubApp } }
    Component { id: editorFactory; Editor.PluginValueEditor { app: stubApp } }
    SignalSpy { id: formSpy; signalName: "changed" }
    SignalSpy { id: editorSpy; signalName: "edited" }

    function test_aFormHasARowPerField() {
        const form = createTemporaryObject(formFactory, test, { type: healthType, payload: { hp: 40 } });
        verify(form !== null);
        verify(findChild(form, "plugin-row-hp") !== null);
        verify(findChild(form, "plugin-row-bar_color") !== null);
        verify(findChild(form, "plugin-row-tags") !== null);
    }
    function test_anAbsentValueReadsAsTheDefault() {
        const form = createTemporaryObject(formFactory, test, { type: healthType, payload: { hp: 40 } });
        compare(form.valueOf(healthType.fields[0]), 40);
        compare(form.valueOf(healthType.fields[1]), "#44CC55");
    }
    function test_anEditKeepsKeysTheSchemaDoesNotName() {
        const form = createTemporaryObject(formFactory, test, { type: healthType, payload: { hp: 40, from_the_future: true } });
        const next = form.with_(healthType.fields[0], 55);
        compare(next.hp, 55);
        compare(next.from_the_future, true);
        compare(form.payload.hp, 40);
    }
    function test_labelsReadAsWords() {
        const form = createTemporaryObject(formFactory, test, { type: healthType, payload: {} });
        compare(form.labelOf("bar_color"), "Bar color");
    }
    function test_numbersStayInsideTheirBounds() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "int", min: 0, max: 10 }, value: 3 });
        compare(e.clamp(12, 0, 10), 10);
        compare(e.clamp(-4, 0, 10), 0);
        compare(e.clamp(5, undefined, undefined), 5);
    }
    function test_aNewListItemStartsAsItsTypeZero() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "text" }, value: "" });
        compare(e.zeroOf({ type: "bool" }), false);
        compare(e.zeroOf({ type: "int", min: 5 }), 5);
        compare(e.zeroOf({ type: "vec3" }).length, 3);
        compare(e.zeroOf({ type: "choice", options: ["a", "b"] }), "a");
        compare(e.zeroOf({ type: "color" }), "#FFFFFF");
    }
    function test_aPickedColorKeepsItsAlpha() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "color" }, value: "#112233CC" });
        compare(e.recolor("#112233CC", "#445566"), "#445566CC");
        compare(e.recolor("#112233", "#445566"), "#445566");
    }
    function test_soundAssetsUseTheTraysAudioKind() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "asset", kind: "sound" }, value: "" });
        compare(e.acceptOf("sound")[0], "audio");
        compare(e.acceptOf("any").length, 0);
        compare(e.acceptOf("image")[0], "image");
    }
    function test_aVectorEditsOneComponentAtATime() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "vec3" }, value: [1, 2, 3] });
        compare(e.withComponent([1, 2, 3], 1, 9).join(","), "1,9,3");
        compare(e.withComponent(null, 0, 4).join(","), "4,0,0");
    }
    function test_aListReportsTheWholeNextList() {
        const e = createTemporaryObject(editorFactory, test, { ty: { type: "list", item: { type: "text" } }, value: ["a"] });
        editorSpy.target = e;
        editorSpy.clear();
        e.edited(e.items.concat([e.zeroOf(e.ty.item)]));
        compare(editorSpy.count, 1);
        compare(editorSpy.signalArguments[0][0].length, 2);
    }
}
