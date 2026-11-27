import QtQuick
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Tour"
    visible: true
    width: 900; height: 600
    when: windowShown

    Item {
        id: host
        anchors.fill: parent
        Rectangle { objectName: "dock-left"; x: 0; y: 40; width: 200; height: 500 }
        Column {
            x: 220; y: 40
            // Inside a Repeater, as the real panels are.
            Repeater { model: 1; delegate: Rectangle { objectName: "code-pane"; width: 400; height: 300 } }
        }
        Rectangle { objectName: "dock-right"; x: 700; y: 40; width: 200; height: 500 }
    }
    Editor.Tour { id: tour; host: host; anchors.fill: parent; autoStart: false }

    function child(name) { return findChild(tour, name); }

    function test_walks_the_steps_and_rings_each_part() {
        verify(!tour.running);
        tour.start();
        compare(tour.index, 0);
        compare(child("tour-title").text, "Your actors");
        const ring = child("tour-ring");
        verify(ring.visible);
        compare(ring.x, -4);
        compare(ring.width, 208);
        // The card sits beside the actor list, not on it.
        verify(child("tour-card").x >= 200);

        // Blocks aren't in this window: the card goes to the middle.
        tour.next();
        verify(!ring.visible);
        compare(child("tour-card").x, (tour.width - child("tour-card").width) / 2);

        tour.next();
        compare(child("tour-title").text, "The canvas");
        verify(ring.visible);
        compare(ring.x, 216);

        tour.back();
        compare(tour.index, 1);
        child("tour-skip").clicked();
        verify(!tour.running);
        verify(!tour.visible);
    }

    function test_done_on_the_last_step_ends_it() {
        tour.start();
        for (let i = 0; i < tour.steps.length - 1; i++) tour.next();
        compare(child("tour-next").text, "Done");
        child("tour-next").clicked();
        verify(!tour.running);
    }
}
