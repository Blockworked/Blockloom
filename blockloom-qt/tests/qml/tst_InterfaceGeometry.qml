import QtQuick
import QtTest
import "../../qml" as Editor

TestCase {
    name: "InterfaceGeometry"
    Editor.InterfaceGeometry { id: geometry }
    function widget(id, transform, order, clips) {
        return {id: id, visible: true, size: [100, 60], transform: transform, paint_order: order, clips: clips || []};
    }
    function test_transforms_and_paint_order() {
        const back = widget("back", [1,0,0,1,100,100], 1);
        const rotated = widget("rotated", [0,2,-2,0,100,100], 2);
        compare(geometry.pick([rotated, back], 100, 140), "rotated");
        compare(geometry.pick([back, rotated], 100, 100), "rotated");
        compare(geometry.pick([back, rotated], 145, 100), "rotated");
        compare(geometry.pick([back, rotated], 300, 300), "");
        rotated.visible = false;
        compare(geometry.pick([rotated, back], 100, 100), "back");
        verify(!geometry.contains(widget("singular", [0,0,0,0,100,100], 3), 100, 100));
    }
    function test_nested_clipping_and_outline() {
        const w = widget("clipped", [1,0,0,1,100,100], 1,
            [{rect: [-20,-30,20,30], viewport_to_local: [1,0,0,1,-100,-100]},
             {rect: [0,-10,50,10], viewport_to_local: [1,0,0,1,-100,-100]}]);
        verify(geometry.contains(w, 110, 100));
        verify(!geometry.contains(w, 90, 100));
        verify(!geometry.contains(w, 110, 120));
        const polygon = geometry.polygon(w);
        compare(polygon.length, 4);
        polygon.forEach(p => { verify(p.x >= 100 && p.x <= 120); verify(p.y >= 90 && p.y <= 110); });
    }
    function test_reflected_clip() {
        const w = widget("reflected", [-1,0,0,1,100,100], 1,
            [{rect: [-20,-20,20,20], viewport_to_local: [-1,0,0,1,100,-100]}]);
        verify(geometry.contains(w, 100, 100));
        const polygon = geometry.polygon(w);
        compare(polygon.length, 4);
        polygon.forEach(p => { verify(p.x >= 80 && p.x <= 120); verify(p.y >= 80 && p.y <= 120); });
    }
}
