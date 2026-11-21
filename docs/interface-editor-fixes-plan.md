# Interface editor: fixes and follow-up

Status: implemented and verified; the evidence is under "Outcome" at the end.
This came from a code and visual review of the Interface tab on 2026-10-11
(debug build, default 1480x948 window and a 2300x1300 one). It extends
`interface-editor-plan.md`; it does not replace it.

What the review could not establish at the time, and how each was settled:

- Dragging, handles, snapping and the timeline had been read in source only.
  They are now exercised through the real editor by
  `blockloom-qt/tests/interface-harness/` (see Outcome).
- A stray blue block above a panel title and a pale bar below it were the
  project's own `Player` and `Ground` actors, now switchable with "World
  behind".

Scope of the verification: gestures are driven with Qt's synthetic events
inside the editor process, against the real backend and embedded runtime.
Driving a physical pointer on a live desktop is deliberately not part of it,
because it acts on whatever window is under the cursor.

## What is good (keep)

- Preview is the real runtime pipeline, with geometry replies matched on
  `revision` and `generation`. QML draws overlays only.
- Begin/update/commit/cancel edit transactions, one undo step per gesture,
  no-op commits deduped (`commands.rs` `commit_interface_edit`).
- Typed edits are atomic (`UiDocument::apply_edit` clones, applies,
  validates, swaps) and have Rust tests.
- Marquee, group move, align/distribute, guides, parent-frame snapping.
- 55 QML viewport tests against a mock backend.

## P0: the editor is not usable at the default window size

Seen in the running app, not inferred.

1. **Inspector is clipped off-screen.** At 1480x948 only a ~35 px sliver of
   the right column shows; at 2300 px wide its fields still run past the
   window edge. The centre column's toolbar rows have an implicit width of
   about 1590 px, so the centre refuses to shrink and pushes the fixed 245 px
   inspector out (`UiDesigner.qml` ~950-972 and the `RowLayout` at ~850).
   - Wrap the toolbar in a flow or a horizontal `ScrollView`, give the centre
     a small `Layout.minimumWidth`, and let it shrink.
   - Side panels get min/max widths and a splitter, like the other tabs
     (`panels.rightWidth` in `EditorPage.qml`).
   - Gate: at 1100x700 every control is reachable, nothing is cut, and there
     is no horizontal page scroll.
2. **Left column is clipped.** Hierarchy search and row buttons are cut;
   "Space H", "Space V", "Bottom" and "Delete" truncate to "Spa...".
   Same fix: real widths, buttons that size to content or use icons.
3. Add a QML test that loads `UiDesigner` at 1100x700 and asserts the
   inspector and tree are fully inside the window.

## P1: layout and visual consistency

4. **Restyle with the app's own controls.** `UiDesigner.qml` has 0
   `LucideIcon` and 12 `Theme.` references against `InspectorPanel`'s 80;
   it uses stock Basic `Button`/`ComboBox`/`TextField`/`CheckBox` and about
   15 hard-coded hex colours that are not in `docs/ui-tokens.md`.
   Use `InspectorRow`, `SectionLabel`, `IconButton`, `NumberField`,
   `ColorField` and the Theme tokens. Move the canvas, ruler and guide
   colours into Theme or ui-tokens.
5. **Palette.** Replace the 23 text buttons (7 visible) with a grouped,
   searchable list with icons (Layout, Controls, Display, Input). Keep drag
   to the canvas and click to add.
6. **Hierarchy rows.** Kind icon, id, and hide/lock as `IconButton`s with
   clear on/off states (the current `●`/`·`/emoji padlock is unreadable).
   Use the freed space: the tree currently has a large empty area under it
   while the palette is cramped.
7. **Move per-selection actions out of the tree panel.** Align/distribute and
   Duplicate/Delete live permanently under the hierarchy. Put them in a
   contextual bar over the viewport (shown with a selection) or the inspector
   header.
8. **Toolbar.** One row of labelled groups: Screen, Preview (size, DPI),
   View (zoom, fit, rulers, guides, safe area, reference), Snap (grid, align).
   Document settings (theme, scale mode) belong in a document inspector that
   shows when nothing is selected, not in the viewport toolbar.
9. **Inspector structure.** Foldable sections as the plan promised:
   Layout, Appearance, Content, Interaction, Bindings, Animation. Show only
   what applies to the selected kind (a Label should not open on gap, grow,
   columns and row height). Header shows kind icon and id, not a bare
   ComboBox. Remember fold state.
10. **Remove "All properties".** It instantiates a ComboBox, CheckBox and
    TextArea per property (one visible) and exposes `layout`, `style`,
    `bindings`, `actions`, `variants` and `fx` as raw JSON. Replace with
    typed controls; if a developer escape hatch is wanted, keep it behind a
    collapsed "Advanced (JSON)" section for the selected property only.
11. **Empty and error states.** "Select a widget" panel with a short hint;
    errors as a dismissible banner instead of one line of text at the bottom.

## P1: correctness

12. **Silent drops.** `save()` returns without feedback while a gesture is
    active (`UiDesigner.qml` ~542), and `extra("world_actor", ...)` still
    goes through it (~554). Queue or reject visibly. Prefer a typed
    `SetProperty` path for `world_actor` so `save()` can be deleted.
13. **Magic sentinel.** `add()` treats `x === 20 && y === 20` as "clicked,
    not dropped" (~617). Pass an explicit `dropped` flag.
14. **Legacy prefab and asset paths.** Delete "Reusable menus" / "Save
    interface as prefab" / "Insert prefab" (~1568-1576). It uses
    `copy<Date.now()>_` ids, remaps only `scroll_target` (misses animation
    tracks, `instance_of`, exports) and bypasses typed edits. The typed
    Component section replaces it. Revisit "Interface assets" the same way.
15. **Preview size combo** is not bound to `previewWidth/Height` and is not
    remembered. Bind `currentIndex`, persist per project in `Settings`.
16. **Marquee** selects by widget centre point (`finishMarquee`), not
    intersection. Make it intersect (or document and add a modifier).
17. **Backspace deletes the selection.** Keep Delete; make Backspace not
    delete (or ask), since it is easy to hit with focus on the canvas.

## P2: performance (plan gate: 1,000 widgets, 10,000 list items)

18. `begin_interface_edit` clones the whole project per gesture
    (`commands.rs` ~6004). Clone only the interface document.
19. Each pointer move runs `apply_edit` (clone + validate), then
    `document.validate()` again, then ships the whole document back to QML.
    Validate once, and return a patch or a revision plus the changed widgets.
20. `UiDocument::validate` walks parents with a linear `find` per hop:
    O(n^2 * depth), so the 10,000 widget cap is not usable on the drag path.
    Build an id to index map once per validate.
21. QML side: `screenWidgets`/`inScreen`/`treeRows`/`descendant` use repeated
    `find` over `document.widgets`. Build an id map once per document change.
    `refresh()` compares `JSON.stringify` of the whole document on every
    state snapshot; compare the revision instead.
22. Add the benchmark the plan promises: a generated 1,000 and 10,000 widget
    document, time per `apply_edit`, per validate, and drag latency end to
    end. Record numbers in `docs/performance.md`.

## P2: structure

23. **Split `UiDesigner.qml`** (1580 lines, about 70 root functions) into the
    plan's phase-2 pieces: `UiWorkspace`, `UiViewport` (canvas, overlay,
    rulers, handles), `UiHierarchy`, `UiPalette`, `UiInspector` (one file per
    section), `UiTimeline`. Keep a small controller object for the gesture
    state machine and snapping so it can be tested without the view. Register
    each file in `build.rs`.
24. **Split `ui_framework.rs`** (3056 lines): model, edits, migration,
    instancing and prefabs, animation sampling, rich text. Keep the public
    `ui::` paths stable.
25. Update `interface-editor-plan.md`'s status line, which still says "nine
    increments... preview gate still open".

## P3: authoring features the plan promised and the UI lacks

26. **Binding and event pickers.** Replace free-text "Actor id" and "Variable
    name" with pickers over real actors, variables and animations, validated
    against the project (plan phase 5).
27. **Animation timeline.** Current UI is an 18 px strip of diamonds where a
    click deletes a key, with typed time and value text. Needs draggable
    keys, a playhead synced with the existing scrub preview, multi-track
    rows labelled by widget and property, add-key from the current value,
    and ease selection per key.
28. **Responsive variants** UI: preview-size chips that match breakpoints and
    show which variant is active, instead of four free number fields.
29. **Instance overrides** surfaced in the inspector (which fields differ
    from the component, per-field reset), as the core already lists them.
30. **Keyboard navigation** of the tree (arrows, rename, delete) and a11y
    names on icon-only buttons.

## Suggested order

1. P0 (items 1-3) in one change; it is small and unblocks everything else.
2. Items 12-14 and 4-5 together (delete the legacy paths, restyle).
3. Items 6-11 (hierarchy, toolbar, inspector structure).
4. Items 18-22 (performance, with the benchmark first).
5. Items 23-25 (structure) alongside, since steps 2-3 touch the same code.
6. P3 as separate features.

## Verification

- Screenshot pass at 1100x700, 1480x948 and 1920x1080 for each of the P0 and
  P1 changes; an automated test where it can be (item 3).
- Existing `tst_InterfaceViewport.qml` and `tst_InterfaceGeometry.qml` keep
  passing; add cases for items 12, 13, 15, 16 and 17.
- Rust: tests for any `validate`/`apply_edit` change stay in
  `ui_framework.rs`; the id-map change needs a test with a deep chain and a
  cycle.
- Run the whole workspace build before merging, since the editor and runtime
  both consume the document.

## Outcome

Everything above is implemented. Numbers refer to the sections above.

- **1-3** The centre column shrinks and the toolbar wraps. Side panels have
  min/max widths and drag handles. `test_panels_fit_a_small_window` loads the
  designer at 1100x700 and checks the tree, viewport and inspector sit inside.
- **4** Palette, tree, toolbar and inspector use `Bw*` controls, `InspectorRow`,
  `IconButton`, `NumberField`, `ChoiceField` and Theme. The design surface's own
  colours are named tokens at the top of `UiViewport.qml`, listed in
  `ui-tokens.md`.
- **5-8** Grouped, searchable palette with icons; tree with kind icons and
  clear hide/lock buttons; a contextual selection bar (duplicate, delete,
  order, align, distribute); labelled toolbar groups with document settings
  moved to the inspector.
- **9-11** Foldable inspector sections with per-widget relevance, remembered
  fold state, an empty-selection document panel, a dismissible error banner.
  "All properties" is now one property under "Advanced (JSON)".
- **12-17** No silent drops (an edit during a gesture reports why), explicit
  `dropped` flag, typed `world_actor` edit (`save()` and the fallback are
  gone), legacy prefab/asset-prefab paths removed, bound and persisted preview
  size, marquee selects what it touches, Backspace no longer deletes.
- **18-20** The edit transaction keeps only the interface document; one clone
  and one validate per edit (`UiDocument::with_edit`); `validate` is linear.
- **19** `update-interface-edit` takes `patch=true` and answers with
  `{document (no widgets), order, widgets (new or changed)}`; the designer asks
  for it, rebuilds the document from its own widgets, and unchanged widgets
  keep their objects. A reply naming an unknown widget fails the gesture.
  Without `patch` the reply is the whole document, as before.
- **21** Widget and geometry lookups in the designer are id maps built once per
  document or layout (`widgetsById`, `boundsById`).
- **22** `scale_tests::timings` and the numbers in `performance.md`.
- **23** The arithmetic of move, resize, snap targets, align, distribute and
  reparent placement is in `UiEditMath.qml` (over `InterfaceGeometry.qml`), pure
  functions with their own tests (`tst_InterfaceMath.qml`). The root keeps the
  transaction state machine and selection, and calls into it.
- **24** `ui_framework.rs` is split into `animation`, `edit`, `prefabs`,
  `rich_text` and test files under `ui_framework/`; public paths are unchanged.
- **25** The older plan's status line is current.
- **26-28** Actor and variable pickers for bindings and events, animation
  pickers for animation events; a timeline with draggable keys, a playhead
  that scrubs the preview, a selected-key editor and add-at-playhead;
  responsive variants show whether they match the preview size and have
  Phone/Tablet/Desktop presets.
- **29** `interface-overrides` lists what an instance changed, the Component
  section shows it with a reset button per field, backed by the new
  `ResetField` edit (one field back to the component's value).
- **30** Tree arrow keys move selection and fold; F2 or double-click renames a
  widget through the new `Rename` edit (parents, scroll targets and animation
  tracks follow; blocks and scripts are not rewritten); the tree and its rows
  expose accessible roles, names and level/expanded state.

Checked in the running system (not only in mock-backed tests): a real
`blockloom-runtime` process computing geometry for the demo interface, a
scrubbed animation moving a widget by its keyed offset (860 to 740), a
1920x1080 viewport change, begin, patch-reply update, batch update with
Rename, commit and undo through the shell's real transaction, and the
pointer-driven gestures below.

Explained, made switchable, or corrected:

- The blue block and pale bar seen behind the pause panel are the project's own
  `Player` (a 60 px blue rectangle) and `Ground` (800x40) actors. The design
  preview draws the project's world behind the interface, and the runtime's
  widget list for that document names neither. The toolbar's "World behind"
  switch (`InterfaceDesign.world`, `PROTOCOL_VERSION` 38) hides every actor for
  the preview only and puts each one's own visibility back afterwards
  (`ui_design::hide_world`); the sky and environment still draw. Checked on
  screen: grabbing the design surface from inside the editor with the switch on
  and off, the ground-coloured pixels go from about 130,000 to under 2,000 (what
  is left is interface), and switching back is pixel-identical to the first
  frame.
- `tst_PhysicsInspector.qml::test_aConflictNeedsTheConversionBeforeApplying`
  failed on a clean `HEAD` as well. Its assertion took the last call as the
  apply call, but `apply()` refreshes the preview and profile list afterwards,
  so with the synchronous mock the last call was `list_player_profiles`. The
  test now finds `apply_player_preset` by name; no product code changed.

Pointer-driven gestures against the real editor: the harness in
`blockloom-qt/tests/interface-harness/` (Qt's synthetic events inside the
editor process, real backend and embedded runtime) ran clean: a click picked
`hp_label`, an eight-step drag moved it from [20,20] to about [60.8,44.3] and
the runtime's geometry followed, the bottom-right handle grew it from 120x28 to
about 149x40, Escape mid-drag left the document alone, an arrow key nudged it
one unit, scrubbing the keyed animation moved `score` from 860 to 740, and a
rename landed.
