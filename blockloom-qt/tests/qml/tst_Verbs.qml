import QtQuick
import QtTest
import com.blockworked.Blockstitch 1.0
import BlockloomBlocks

// Declared verbs (blockloom-core's verbs.rs) reach the palette from the
// generated table alone: a row, a section entry and the real type on drop.
TestCase {
    name: "Verbs"

    function verb(name) {
        for (const v of Blocks.builtinVerbs) if (v.type_id === name) return v;
        return null;
    }

    function test_everyVerbIsFiledInItsSection() {
        verify(Blocks.builtinVerbs.length > 0);
        for (const v of Blocks.builtinVerbs) {
            const name = Blocks.verbPaletteName(v);
            const section = Blocks.paletteSections.find(s => s.category === v.category && s.title === v.section);
            verify(section, v.type_id + " has no section");
            verify(section.items.indexOf(name) >= 0, v.type_id + " is missing from " + v.section);
        }
    }

    function test_aStatementDropsAsTheGenericVerb() {
        const set = verb("set_property");
        verify(set);
        const fresh = Blocks.verbFresh(set);
        compare(fresh.type, "Verb");
        compare(fresh.verb, "set_property");
        compare(fresh.args.length, set.slots.length);
        compare(Blocks.realType(Blocks.verbPaletteName(set)), "Verb");
        const head = Blocks.verbHead(fresh);
        compare(head.filter(p => p.kind === "value").map(p => p.field), ["PluginArg:0", "PluginArg:1", "PluginArg:2"]);
        compare(head[0].text, "set");
    }

    function test_aReporterIsTheVerbOperator() {
        const open = verb("screen_open");
        const value = Blocks.verbValue(open);
        compare(value.op, "Verb");
        compare(value.args[0].value, "screen_open");
        compare(Blocks.verbResult(value), "bool");
        compare(Blocks.realValue({ kind: "Op", op: Blocks.verbPaletteName(open), args: [] }).op, "Verb");
        const op = Blocks.extraOperators.find(o => o[0] === Blocks.verbPaletteName(open));
        verify(op, "the reporter is offered in the palette");
        compare(op[1], "bool");
        const layout = Blocks.verbLayout(value);
        compare(layout.filter(p => p.arg >= 0).map(p => p.arg), [1]);
    }

    function test_detailsSayWhatAVerbDoes() {
        const set = verb("set_property");
        compare(Blocks.labelFor(Blocks.verbPaletteName(set)), set.purpose);
    }
}
