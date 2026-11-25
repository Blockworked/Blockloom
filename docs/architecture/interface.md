# Interface

The game's screen-space interface, UI v2 screens and styles, and the interface designer. Split out of `CLAUDE.md`; the index there lists every page.

## Interface

A game builds its HUD and its menus out of blocks, in screen space, over the
world. `blockloom-core/src/ui.rs` is the vocabulary: seven element kinds
(`Panel`, `Label`, `Button`, `Image`, `Input`, `Slider`, `Toggle`), a 9-point
anchor, and the properties `set [prop] of (id) to` can write. An element is
named by an id string the project invents, and `show` makes one *or updates
the one that id already names* - so a HUD strand can rebuild itself every
frame without piling up. `hide` takes one off the screen without forgetting
it, children and all; `delete` forgets it. A parented element flows after its
siblings inside its parent's vertical stack and its own placement is ignored,
which is what makes Resume / Settings / Quit a three-block menu.

Every `show` block spells its id, caption and placement the same way, so the
element kinds share one set of field ids (`UiId`, `UiContent`, `UiX`, ...). The
JSON field is `element` rather than `id`, because a flattened instruction
already carries its own `id` on the wire (see `wire.rs`).

Three properties have no `show` row and so live beside the style rather than
on the element: a slider's `step`, and a text input's `allow` and `max
length`. A HUD strand that re-shows its own slider every frame would
otherwise wipe them. `step` rounds a slider's number to a multiple of itself
measured from `min`, which is also applied to a number written straight in,
so a step that doesn't divide the span leaves the far end short. `allow`
names a character set (`any`, `numbers`, `digits`, `letters`) by a word
somebody types, and anything else reads as `any` - a typo shouldn't deaden a
field. A refused keystroke is simply not there: no error, since somebody
holding a key down means no harm by it.

Five reporters read the interface: `value of (id)` and `text of (id)`
(which is a label's words, or an empty input's placeholder), `is (id)
shown?`, `does (id) exist?` and `the focused element`. `hide` leaves an
element existing but not shown; only `delete` takes it out of both. An id
nothing answers to is an error for the first two, the way a missing actor
is, and plainly false for the other two.

`blockloom-runtime/src/ui.rs` is the id map and the rules over it - the
hit test, the subtree walk, the anchoring - Bevy-free but for the entity
handle, so they unit-test without a window. `overlay.rs` is the Bevy half:
`apply_ui_effects` folds the fixed step's effects into the map, and `draw_ui`
spawns, despawns and restyles. An element's anchor is a percentage plus a
`UiTransform` of its own size rather than a worked-out pixel offset, because
an auto-sized label isn't measured until Bevy has laid it out - and a window
resize then recomputes for free.

Clicks route through the interface first (`world::detect_clicks`): the
topmost visible element whose rectangle covers the pointer wins, and only
what nothing wanted reaches the world picks. A visible modal element swallows
the rest, so clicking beside a pause menu never fires the gun behind it. A
click on a text input hands it the keyboard; while it holds it, game strands
see no keys at all. `focus (id)` and `clear focus` move the keyboard without
a click, and the input holding it is drawn brighter, since otherwise nothing
would say where the typing is going.

`pause game` freezes the world: no world strand advances, no physics steps,
no key or collision event queues, so resuming never bursts. The one fork is
that strands a UI event started keep ticking - otherwise a pause menu's own
buttons would be dead - and they run on the wall clock, so a `wait 1` blink
on a frozen menu still blinks. Only the interface broadcasts while paused,
so a message then starts its receivers as UI strands too (escape and back
likewise). A `pause game` in a world strand stops that
strand where it stands, the way `delete myself` does; in a UI strand it
doesn't. `world::set_paused` is the one place that flips it, whether the
editor's Pause button or the block asked.

The wall clock is real time: `Engine::wall_time` counts `Time<Real>` seconds
from `begin_run`'s `wall_started_at`, so a pause, a game speed below 1 or a
stalled frame doesn't slow or rewind it (it used to be the fixed clock minus the
run start, which jumped back by the paused span on resume). Run clocks read
`elapsed_secs_f64`, since the `f32` form loses about a quarter millisecond an
hour in.

Pointer lock is fully manual: game code unlocks around a menu and re-locks on
close. In the Game view the editor holds it on the world's behalf (see Game
view above). The one safety net is that showing a modal while the pointer is locked
logs a warning, since a locked hidden cursor can't press anything.

UI v2 (`blockloom-core/src/ui_tree.rs`) is the tree the saved interface
moves to: a `UiScreen` of `UiNode`s, each with a `Widget` carrying only its
kind's fields, a `LayoutItem` (flowing in its parent, or `Anchored` by
anchors, pivot and position) apart from the container its widget is, and
`Spacing` overrides over the kind's own (`kind_spacing`, which mirrors the
runtime's `node_for`). `UiScreen::from_document` reads a flat `UiDocument`
(prefabs become `components`; fields a kind can't use go to `Legacy`) and
`lower` writes one back for the runtime, which still draws flat documents;
stretched anchors and non-top-left pins inside a stack refuse to lower until
the runtime draws them. The gates are the round trip in `ui_tree_tests.rs`
(the golden projects too, plus `BLOCKLOOM_UI_FIXTURES=<folder>`) and
`a_document_read_as_a_tree_lays_out_where_it_was` in `ui_design.rs`, which
lays both out, plus the Phase 5 `gate_*` tests beside it: a HUD in its
corners, a centred modal, an inventory grid of component slots, a
scrolling menu and one menu file shared by two scenes, each laid out from
a v2 screen.

A `.blockui` file is one `UiScreen` (`blockloom-core/src/ui_layers.rs`). A
scene lists the files it draws in `World.screens` (a `Screens` scene
component, saved only when there are some) and the game in
`GameSettings.screens`; each is a `UiLayer { screen, open }`.
`ui_layers::scene_interface` is what the world draws at every rebuild: the
scene's own interface, then its screens, then the game's, each file once,
lowered and merged (a widget id another layer took, or a style, component or
animation of the same name that differs, skips that layer with an error; the
others still draw). A closed layer's roots start hidden. Play and every
`switch scene to` compose it again (`world::load_interface`, kept as
`Engine::interface`, which widget actions read). The files aren't in
the document, so `write-screen` marks the world stale to redraw it.
Commands: `create-screen` (`fromInterface` moves the scene's interface into
the file), `read-screen`, `write-screen`, `add-screen-layer`,
`remove-screen-layer` (`commands/screens.rs`); the Problems panel lists
layers that don't compose.

A `.blockstyle` (`UiStyleFile`: `base` theme, `tokens`, `classes`) is a
theme. Listed in a document's `stylesheets`, it sets the theme, lays its
`UiTokens` (panel, button, input, input_focused, text, accent, track,
toggle_off colors) under the document's own and adds its classes;
`GameSettings.style` names one every interface starts from (Project Settings
> Game, or `set-property target=settings path=style`). The runtime draws
those colors through `ui::Palette` (`UiManager::palette`), which falls back
to the built-in theme's for anything unset and for any other theme a `set
theme` switches to. `create-style` writes a starter one.

A tree node with `instance: Some(UiInstance { component, inputs, slots })`
uses a component by reference: `lower` (`UiScreen::expand`) writes the
component's nodes out renamed `<node>/<member>` (so the root is
`<node>/<root>`, the prefix form the v1 instance tools expect), puts the
node's own placement on the root, places `slots` nodes under the exported
slot (named where the instance is), and sets each input through the v1
`SetExposed` edit, so the designer sees them as overrides it can reset.
Components nest up to 16 deep, which also stops one that uses itself. A
screen's `inputs` are exports at screen level; `lower_with` sets them, and a
`UiLayer`'s `inputs` (`add-screen-layer inputs={...}`) are what a scene
passes. Old documents keep their expanded instances (`instance_of`), so
they still round-trip.

`open screen` and `close screen` (`OpenScreen`/`CloseScreen`, one
`Action::SetScreenOpen`, `Effect::SetScreenOpen`, `ACT_SET_SCREEN_OPEN`)
name a screen by its file name without the extension
(`ui_layers::screen_name`) or its path, and set its roots' visibility
(`UiManager::open_screen`; `Composed::screens` lists each screen's roots).
Per J6 the immediate-mode `show` blocks are retired: they still load and
run, but the palette lists Screens instead of Showing and their vocabulary
entries say so. Not yet: a script ABI verb for screens, an `is screen open?`
reporter, and setting a screen's inputs while it runs.

The designer edits either the scene's interface or one of the screens the
scene or game lists (the toolbar's Editing picker, `UiDesigner.target`).
For a screen, `begin_interface_edit screen=<path>` starts from the file
lowered (`screen-document` shows the same thing), and commit writes
`UiScreen::from_edit` back: the edited flat document read as a tree, with
each expanded instance folded back into a reference when lowering the fold
gives the same widgets, and the screen's inputs kept. A file edit is not a
document change, so it has no undo step yet.

## Interface editor

Plan and progress: `docs/interface-editor-plan.md`. The Interface workspace
(`UiDesigner.qml`) draws no layout of its own: `preview_interface` runs the
document through the player's retained widget renderer in an idle design
session and the reply carries each widget's computed transform, size, clips,
visibility and paint order, tagged with a document revision and viewport
generation. QML picks and draws handles from that geometry only.

Edits are `blockloom_core::ui_framework::UiEdit` (`apply_edit` validates and
swaps atomically): `Move`, `Resize`, `SetProperty` (canonical paths),
`Reparent` (free or flow), `Reorder`, `Create`, `Delete` and `Duplicate`. A
gesture is `begin_interface_edit` -> `update_interface_edit` (replaces the
draft, never saves) -> `commit_interface_edit` (one undo step) or
`cancel_interface_edit`; the same commands serve shell and MCP. `Create`
appends a widget (a Canvas parent forces it free, any other parent flow),
`Delete` removes the subtree and clears `scroll_target`s into it, `Duplicate`
copies the subtree after the original as `new_id` and `new_id.<old id>`,
remapping parents and `scroll_target`s inside the copy only. Blocks and
scripts that name the original ids are not rewritten. `set_interface` stays
the whole-document import path. The hierarchy is a tree built in QML from sibling order (`treeRows`) with search and fold state; Lock (`lockedIds`) is editor-only and never saved. `UiDocument.version` (`UI_SCHEMA_VERSION`) is stamped by `migrate()` on load and in `set_interface`; a newer document is refused by `validate`, not rewritten. Editor hide travels as `InterfaceDesign.hidden` and only affects the design preview. `Batch` applies several edits atomically; `property_metadata()` (`interface-properties`) is the table of `SetProperty` paths and is kept in step with `UiPropertyEdit` by a test. Selection is `selectedId` plus `extraIds` in QML (Shift-click, marquee) and Delete/Duplicate send one `Batch`. Group drag, Align and Space H/V (`arrange`) compute in QML from runtime geometry and send a `Batch` of `Move`s. Content and style fields (`style`, `class`, `bindings`, `items`, `tooltip`, `scroll_target`, `tab_index`, `transition`, `element.range`) plus `SetDocument` and `SetClass` are typed too, so `set_interface` is only the import path. Committing with a design session open sends no `Load`. A new edit shape needs the `update-interface-edit`
summary in `shell.rs` and a case in `tst_InterfaceViewport.qml`'s mock backend.

The designer loads `interface_properties` on start and `applies(path)` hides inspector rows that do not suit the selected widget kind (Modal, scroll target, Items).

The workspace is split by view, all driven by the `designer` root in `UiDesigner.qml` (selection, gestures, snapping, typed edit helpers): `UiPalette` (grouped, searchable kinds), `UiHierarchy` (tree, hide/lock, arrow keys), `UiToolbar` (a wrapping `Flow`), `UiViewport` (frame, rulers, overlay, handles; its colours are named tokens), `UiInspector` (foldable `UiSection`s, a document panel when nothing is selected, one "Advanced (JSON)" property) and `UiTimeline`. New files are listed in `build.rs`. Side panels have min/max widths and drag handles so the designer fits 1100x700 (`test_panels_fit_a_small_window`). Fold state and the preview size persist in QML `Settings` when the app has an identity. Every inspector write is a typed `SetProperty`/structural edit (`world_actor` is typed too); `set_interface` is import only. An edit attempted during a gesture reports through `error` rather than being dropped. `interface-overrides` (`interface_overrides`) lists what a component instance changed, `ResetField` puts one of those fields back, and `Rename` changes a widget id (parents, scroll targets and animation tracks follow). `update-interface-edit` takes `patch=true` and then answers with `UiPatch` (`document` without widgets, `order`, changed `widgets`); the designer applies it with `applyUpdate`. Move, resize, snap, align and reparent arithmetic is in `UiEditMath.qml`, a plain object over `InterfaceGeometry.qml` (tested in `tst_InterfaceMath.qml`). A transaction holds only the interface document it started from; `UiDocument::with_edit` is one clone and one validate, `validate` walks each parent chain once, and `scale_tests::timings` prints the 1,000 and 10,000 widget cost. `ui_framework.rs` keeps the model and `ui_framework/` holds `animation`, `edit`, `prefabs`, `rich_text` and the tests; everything still re-exports through `ui::`.

Components: `UiEdit::SavePrefab/InstantiatePrefab/DeletePrefab/DetachInstance` over `UiDocument.prefabs`; an instance root has `instance_of`, updating rebuilds instances but keeps widgets changed from the old source, and a missing prefab fails validation. Image widgets take `UiPaint.image_fit` (`Stretch`, `Sliced{border}`, `Tiled`), applied in `overlay.rs::image_node`.

## Interface effects and animation preview

`UiFx` (opacity, offset, rotation) is visual state laid over an element after
layout: `UiWidget.fx` holds the authored start (typed edit path `fx`), and the
`Opacity`/`OffsetX`/`OffsetY`/`Rotation` `UiProp`s write it at run time and are
animatable tracks. It composes with the anchor without touching its
`UiTransform` translation: offset adds to the margin (anchored), insets
(absolute) or relative insets (flow child) in `layout_node`, rotation sets the
same transform's rotation (`overlay::apply_fx_rotation`), and opacity
multiplies down the tree (`UiManager::opacity_of`) into background, border,
shadow, image and text colours. Not faded yet: slider tracks and fills.
`InterfaceDesign.world` (default true, `PROTOCOL_VERSION` 38; the toolbar's "World behind") false hides every actor for the preview only (`ui_design::hide_world`). `InterfaceDesign.animation` (`{name, time}`, `PROTOCOL_VERSION` 37) freezes a
document animation at a time in the design session through
`UiManager::sample_animation`; the editor's timeline Preview/scrub uses it and
pauses editing while it is on. Nothing is saved.
