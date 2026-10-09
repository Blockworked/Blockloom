# Interface editor overhaul

Status: implementation started (nine increments). Phase 0 has a runtime design session, frame-matched selection and screen isolation. Interface supports typed move/resize, property, reparent and reorder transactions, snapping, eight resize handles and keyboard nudging; the preview gate is still open.

## Goal

Make Blockloom's Interface workspace a complete visual editor for production game HUDs, menus, inventories, dialogs and touch controls. An author should be able to build, style, reuse, animate and wire an interface without editing JSON or repeatedly pressing Play to discover its layout.

Unity-level means a measurable authoring workflow, not just matching its panel arrangement. Combine the strengths of Unity's UI Builder (hierarchy, reusable styles, layout inspection) and Unreal's UMG (direct canvas manipulation, anchors, reusable widgets, animation tracks).

References:

- [Unity UI Builder interface](https://docs.unity.com/en-us/engine/6000.7/manual/uitoolkits/uielements/uibuilder/uib-interface-overview)
- [Unreal Widget Blueprints](https://dev.epicgames.com/documentation/unreal-engine/widget-blueprints-in-umg-for-unreal-engine?lang=en-US)
- [Unreal anchors](https://dev.epicgames.com/documentation/en-us/unreal-engine/umg-anchors-in-unreal-engine-ui)

## What is wrong today

The screenshot and current implementation reveal structural problems:

1. `UiDesigner.qml::rect()` implements a second, approximate layout algorithm. It substitutes arbitrary dimensions for auto-sized widgets and estimates sibling positions from an index and the current child's dimensions. It does not reproduce Bevy's text measurement, flex/grid layout or clipping.
2. Every widget is drawn as a generic rectangle and text. Images, rich text, controls and containers do not look like their runtime counterparts. Permanent borders and fallback kind names obscure the interface itself.
3. The hierarchy is a flat list with one indentation level. Selection uses array indices, which are unstable across deletion and reordering. There is no real tree, screen isolation, sibling reordering or useful visibility control.
4. Interface sits inside the actor workspace. `EditorPage.qml` retains the actor inspector and asset tray while the designer adds another inspector. The actual editing area gets squeezed.
5. Common layout and binding tasks expose JSON and unlabeled fields. The editor lacks resize handles, anchor editing, reliable reparenting, alignment, multiselection and proper zoom/pan.
6. `set_interface` validates and saves the entire document, adds an undo entry and syncs the runtime. It is useful as an import operation, but insufficient as the interaction API for a large editor with live gestures and concurrent updates.
7. Documentation explicitly makes Play authoritative for text, layout and interactions. An author cannot trust the designer. Multiple menus occupy the same canvas without a practical way to isolate the state being edited.

The backend is further along than the editor: it already has retained Bevy widgets, flex/grid layouts, state styles, bindings, safe areas, virtualized lists, UI assets and copied prefab trees. Preserve working behavior while replacing the authoring architecture.

## Recommended architecture

Keep Qt/QML for editor chrome and initially keep Bevy for game UI rendering. Replace the approximate QML preview with the actual game UI pipeline running in an isolated design session. Do an early capability spike before committing to Bevy for every advanced feature.

### One document and one rendering path

- `blockloom-core`: versioned UI document, validation, migration, property metadata, asset references and edit operations.
- `blockloom-app`: authoritative transactions, undo, autosave, revisions and shell/MCP access.
- `blockloom-runtime`: actual layout, text, control rendering, input and animation; a design mode runs these without starting gameplay, actor scripts or physics.
- `blockloom-qt`: dedicated Interface workspace, viewport overlays, tree, inspectors, asset pickers and timeline.
- `blockloom-protocol`: revision-tagged preview requests, draft edits and computed layout replies. Update protocol versions when messages change; update script ABI versions only when that boundary changes.

The design session returns computed bounds, transforms, clip regions, effective visibility and paint order keyed by stable widget IDs. QML draws selection handles and guides over the rendered frame. It does not compute a competing layout. Bind frame and geometry to the same document revision and viewport generation so stale replies cannot misplace selection handles.

Reuse existing frame transport on Linux and the process preview path elsewhere. Do not assume the existing Game view supports two simultaneous sessions: prove ownership, teardown, input routing and frame transport in the spike. Reusing one view while Interface is active is acceptable if it preserves the Game workspace's state. Isolate preview resources so asset loading and fonts match a shipped game.

### Editing contracts

- Select by immutable IDs; keep display names independent. Legacy IDs remain addressable by blocks and scripts.
- Define canonical property paths for inspector edits, bindings, animations, blocks, scripts and MCP.
- Replace routine whole-document writes with validated, revision-aware operations: insert, delete subtree, duplicate, move/reparent, reorder, set properties and edit component overrides.
- A gesture previews temporary edits, commits one undo step on release, and discards the draft on Escape. Preview never saves authored data. Autosave follows successful commits.
- Older replies cannot replace a newer draft. External project changes either rebase a safe operation or explicitly cancel a conflicting gesture.
- Provide computed property values and provenance: default, style, instance override, binding or animation. Do not silently let competing writers fight.

### Layout model

Give authors two explicit modes: free placement inside a Canvas, and flow inside stack/grid/wrap containers. A flow child is reordered by dragging; its size is controlled by layout. An absolute child has transform handles. The inspector explains which container controls a property.

Add paired normalized anchors, pivot, edge offsets, stretch, rotation, scale, aspect constraints, explicit sizing rules and draw order. Keep visual transforms separate from measured layout dimensions. Define local logical coordinates, display DPI, reference resolution and viewport zoom separately. Use one transform conversion for rendering, hit testing and gizmos.

Migrate existing nine-point anchors and offsets without moving the UI. Preserve legacy sizing and container behavior through an explicit compatibility representation where necessary. Introduce improved behavior for newly authored documents deliberately, rather than reinterpreting old saves.

## Workspace design

The Interface tab replaces actor panels with UI panels and remembers its own layout.

| Region | Contents |
| --- | --- |
| Left | Screens/components selector, searchable hierarchy, collapsible widget library |
| Center | Large viewport with pan/zoom, rulers, guides, safe-area overlay and selection handles |
| Right | Contextual widget, multiselection, screen or document inspector |
| Bottom, collapsible | UI assets, animation timeline, binding/event tools and diagnostics |
| Toolbar | Select/move/resize/rotate, snapping, preview size/DPI, screen/state, Design/Interact, frame selection |

Use the app's existing styled fields and asset pickers. Name every property. Group inspectors into Layout, Appearance, Content, Interaction, Bindings and Animation. Show only relevant sections. No selection shows screen/document settings. Container outlines and widget IDs appear only as optional overlays.

Editor hidden/locked flags are distinct from runtime visibility and enabled state. Screen isolation changes the preview, not the saved game's visible state. Interactive preview uses temporary fixture data; an explicit full-game preview remains available for gameplay-dependent behavior.

## Delivery sequence and gates

### 0. Establish fixtures and prove the preview architecture

Capture existing Emberwatch interfaces and representative legacy documents. Record runtime screenshots, computed geometry and interactions at several sizes. Verify documents generated by blocks as well as authored assets.

Build a small design-session spike with a nested panel, wrapped rich text, an image, a slider and a scroll list. Render with real runtime systems without running gameplay. Return revision-matched geometry and demonstrate selection overlays, asset loading and input routing on embedded and process paths.

**Gate:** displayed widgets and reported geometry match the game pipeline; changing reference size, DPI and safe area stays correct. Document capabilities and gaps for text editing, rotation, clipping, nine-slice images and animation. Any renderer replacement decision happens here, before expanding the editor.

### 1. Versioned document and transactional editing

Add schema versions, deterministic migration, stable identity, explicit sibling order and shared property metadata. Add draft/commit/cancel transactions and patch commands to dispatch, shell and MCP. Keep whole-document import/export.

Define layout compatibility, style inheritance and precedence among authored values, bindings, animations and runtime writes. Migrate inline prefab copies into the new representation without pretending they are already linked instances.

**Gate:** legacy interfaces keep their runtime geometry and behavior. Save/reload, duplication, reference remapping and undo/redo preserve IDs and ordering. A drag produces one saved edit; canceled or stale drafts never write files.

### 2. Dedicated workspace and trustworthy preview

Replace the monolithic `UiDesigner.qml` with workspace, viewport, hierarchy, library, inspector and selection-overlay components. Register new QML files in `build.rs`.

Remove actor panels from Interface. Add resizable panels, real hierarchy expansion/search, selection synchronization, editor hide/lock, screen isolation, preview backgrounds, zoom/pan, fit/frame selection and a useful empty state. Integrate the runtime preview from phase 0.

**Gate:** Emberwatch's HUD, title, pause, settings and forge views can each be inspected clearly. Images and rich text render correctly. No generic rectangle stand-ins or compulsory container outlines obscure the canvas.

### 3. Complete layout authoring

Add drag creation into a chosen parent, marquee/multiselection, move/resize/rotate handles, anchor and pivot presets, visible anchor editing, keyboard nudging, copy/paste, duplicate, group, delete, reparent and sibling reorder. Add alignment, distribution, grid snapping and contextual smart guides.

Expose flow, grid, wrapping, padding, gaps, min/max, grow/shrink, alignment, auto/content/percent dimensions and aspect ratios through typed controls. Reparenting preserves visual placement when mathematically possible; entering a flow container explicitly changes to flow behavior. Disable or explain handles controlled by layout.

**Gate:** an author can build an anchored HUD, centered modal, inventory grid and scroll menu entirely through visual controls. Nested anchors, transforms, clipping and multiselection work through zoom, DPI and safe-area changes.

### 4. Production styling and controls

Add visual style editing, shared tokens/classes, effective-value inspection, state preview and per-widget overrides. Add font and image pickers, text alignment/wrapping, nine-slice/stretch/fit image modes, opacity, borders, shadows, clipping and masks supported by the selected renderer.

Bring controls to production quality: range/step and orientation controls, scroll behavior, dropdowns, list row templates and focus navigation. Replace append-only text input with caret, selection, clipboard, Unicode editing and IME composition. Design composition transport for Qt, native players and web players rather than fixing only the editor.

**Gate:** a styled menu can be reproduced faithfully in the editor and shipped game, including hover, pressed, focused and disabled states. Lists virtualize real row templates, and text entry works with keyboard, touch and IME.

### 5. Reusable components and behavior wiring

Introduce separate screen assets and linked UI components with instance identity, named slots, exposed properties, overrides, variants and apply/revert operations. Updating a component propagates while preserving instance overrides. Validate dependency cycles and missing references.

Provide visual binding and event editors with actor/variable/component pickers, converters, sample values and errors. Authors can select a button and connect its click to existing blocks without manually copying IDs. Preserve VM, compiled-code and script behavior; define namespaced references for reusable instances and remap all internal references on duplication.

**Gate:** build a reusable settings row and inventory slot; update their source and verify multiple instances. A HUD health binding and pause/resume menu work in VM and compiled native execution, plus web execution where supported.

### 6. Animation, responsive authoring and shipping quality

Add a timeline with named clips, keyframes, easing, scrubbing, looping and event triggers. Start with opacity, translation, scale, rotation and color, then add layout properties where the cost is understood. Scrubbing never persists animated values into authored properties. Define interruption, blending and binding interaction.

Add responsive variants/breakpoints, preview matrices, localization samples, text expansion, safe-area/device profiles, navigation diagnostics and missing-asset reports. Build a fixture library of HUDs, title/pause/settings screens, inventory, dialog and touch controls.

**Gate:** a non-programmer can build, reuse, bind, animate and ship those interfaces without JSON or custom Rust. Desktop, web and applicable mobile players preserve the same UI behavior and scaling.

## Verification and performance

- Migration fixtures: geometry and behavior before/after, including block-created legacy widgets and saved assets.
- Transaction tests: atomicity, reference remapping, undo boundaries, cancellation, stale revisions and external saves.
- Geometry tests: nested anchors, flow children of unequal size, text wrapping, rotation, clipping, DPI, safe areas and responsive layouts.
- Visual comparisons: designer and player use identical assets, viewport and fixture data. Compare bounds exactly within a defined subpixel tolerance and screenshots with font/GPU-aware tolerances.
- Interaction tests: modality, focus, bubbling, controller navigation, scrolling, bindings and text composition. Verify no designer clicks reach gameplay.
- End-to-end QML tests for consequential workflows, plus manual native/web/device checks for input and rendering integration.
- Profile a 1,000-widget document and a 10,000-item virtualized collection on named reference hardware. Initial targets: responsive 60 Hz manipulation, under 50 ms typical edit-to-preview latency, no full-document save per pointer move, no full-world restart for a UI property edit, and no work proportional to every list item while scrolling. Treat these as measured targets, not existing guarantees.

Do focused checks per phase. Run workspace tests and relevant QML checks before merging; prepare patched dependencies before direct Cargo on a fresh checkout. Build the entire workspace when validating editor/runtime integration.

## Implementation boundaries

Primary changes span `blockloom-core/src/ui.rs`, `ui_framework.rs`, project serialization, `blockloom-app` commands/dispatch/history, `blockloom-protocol`, runtime `ui.rs`, `ui_systems.rs`, `overlay.rs`, design/input systems, and Qt `EditorPage.qml`, `UiDesigner.qml`, bridge and frame handling. Extend blocks, codegen and script APIs where new properties or events require them. Keep shell metadata and `mcp/src/registry.ts` synchronized when argument types change.

First implementation slice: phase 0 plus the Interface workspace shell. The first substantial vertical slice is an isolated Emberwatch screen rendered by the real pipeline, with ID-based selection, a typed layout inspector and one undoable move/resize gesture. This validates the critical architecture before committing to components and animation.

This is a major subsystem project. Do not claim engine-level parity after a cosmetic pass or phase 3. Estimate delivery after the preview and text/input spike establishes platform costs; use the gates above to keep each increment independently reviewable.

## Implementation progress

### Phase 0 foundation (first increment)

- Added temporary `preview_interface` requests and `interface_layout` replies through the existing embedded/process runtime message transport. Requests carry a document revision and viewport generation. Drafts validate before replacing the previous preview and never enter project history or autosave.
- The idle runtime uses the player's retained widget renderer, asset directory and stylesheet loader. Design mode bypasses gameplay bindings, world-projected widgets and scene-edit input. Play, Stop, document reload and closing the draft end the session.
- Computed replies report authored IDs, physical-pixel sizes, affine transforms, inherited clipping, effective visibility and Bevy paint order after layout. Replies from superseded requests are discarded by the backend.
- Added a legacy document fixture with nested panels, wrapped rich text, an image node, slider and virtualized scroll list, plus tests for draft isolation, invalid/stale requests and real Bevy geometry.
- Interface now hides the actor list, actor inspector and global asset tray. Widget selection follows IDs across document snapshots.

### Runtime viewport and selection (second increment)

- Replaced the approximate designer tiles with the embedded GPU viewport on Linux and the runtime MJPEG stream for process mode. Game creates its viewport only while its tab is visible; Interface creates its native viewport only while designing. The application owns the process stream watcher across tab switches.
- GPU slots carry the geometry extracted with their rendered UI. Acquiring a frame holds its slot and snapshots its geometry together. Process screenshot requests freeze post-layout geometry and deliver it in the same MJPEG part as the image. Identical images still deliver changed metadata.
- Interface rejects geometry from older revisions, viewport generations or pixel dimensions. Resolution presets request physical-pixel sizes at scale 1; process mode restores its original window resolution on cancellation. Selection and picking stay disabled until matching geometry arrives.
- Design sessions temporarily use full resolution and spatial output, independent of saved game quality or dynamic resolution. Cancellation restores the original runtime quality settings without editing the project. A Vulkan smoke test verifies visible UI and matching slot metadata for projects saved at full and half world resolution.
- Added ID-based click selection and hover/selection outlines from runtime affine transforms, effective visibility, paint order and inherited clips. Picking consumes editor input locally. The approximate rectangle layout and direct tile drag implementation are removed; property edits remain in the inspector.
- Added Qt Quick tests for transformed/clipped picking, stale metadata, viewport changes, selection without saves/gameplay input and cancellation, plus runtime tests for slot metadata ownership and process viewport restoration. Added an ignored GPU smoke test for a windowless design session.

### Screen isolation (third increment)

- Added a Screen selector for top-level widget trees, with an All screens option. The hierarchy follows the isolated tree and preserves ID-based selection for its descendants. Removing or reparenting the active root returns to All screens. Adding a new top-level widget also returns to All screens so it remains visible.
- Design requests accept an optional `screen` root ID. Backend and runtime share validation, reject missing or non-root IDs atomically, and retain the previous preview on invalid requests. Protocol version is now 23; requests without `screen` keep the existing all-widget behavior.
- Isolation hides other roots only in the temporary runtime manager. The authored document, gameplay visibility, project files, save revisions and undo history stay unchanged. Geometry reports retain all IDs with effective visibility for hidden trees; the retained player layout remains authoritative.
- Switching screens or documents invalidates the preview revision immediately, including the debounce interval before sending a new request. Picking and overlays wait for matching frame geometry.
- Added runtime checks for nested visibility, atomic validation, restoration and unchanged Bevy geometry of the selected tree; backend checks for no saves or world restart; and Qt Quick checks for hierarchy membership, selection and stale frames.

### Typed move/resize transactions (fourth increment)

- Added shared typed `Move` and `Resize` edits, addressed by widget ID with absolute authored offset/size values. Validation rejects unknown IDs, non-finite offsets, non-positive sizes, projected widgets and children positioned by a flow layout. Resizing sets existing layout width/height to pixels while preserving its other settings.
- Added `begin_interface_edit`, `update_interface_edit`, `commit_interface_edit` and `cancel_interface_edit` to dispatch and shell/MCP metadata. Begin captures the saved revision and project snapshot; updates replace a private draft from that starting document. Commit checks the token, project snapshot and loaded/disk revisions, then saves one undo step. Empty commits and cancellations do not save. Cancellation closes the temporary runtime preview.
- The viewport supports dragging a widget and its bottom-right resize handle. Gestures use frozen runtime transforms; root moves account for canvas scaling and child moves use their parent's frame. Resize preserves the opposite corner for anchored widgets. Escape, screen/viewport changes, tab exit and external edits discard active gestures. Pointer grabs survive pending preview frames.
- The X/Y/Width/Height inspector uses the same transactions. Only one update is in flight; later pointer positions coalesce, and release waits for the final validated update before committing. Delayed begin/update responses cannot resurrect canceled drafts. Whole-document import and the remaining inspector fields retain `set_interface`.
- Backend checks cover unchanged files/revisions during drafts, invalid-update atomicity, one-save commits, undo/redo, cancellation, stale undo/external saves, project reopening and empty commits. Core checks cover layout preservation and invalid dimensions. Qt Quick checks cover move/resize grabs, transformed geometry, Escape, typed inspector edits and delayed replies.

### Typed properties and reparenting (fifth increment)

- Added typed `SetProperty` edits with canonical paths `element.kind`, `element.content`, `element.anchor`, `element.modal` and `layout`. Each path has a fixed value type; IDs and parent links cannot be changed through property writes. Layout edits reject unknown fields, non-finite or negative dimensions/spacing, invalid column counts and contradictory min/max sizes. Saved legacy layouts retain their existing loading and rendering behavior.
- Added labeled layout controls for Auto/Px/Percent width and height, min/max sizes, margin, padding, gap, grow, columns, row height, child alignment and absolute placement. Authors can enable an explicit layout or remove it to return to legacy sizing. Flow children can edit layout even though direct move/resize handles remain disabled. Kind, content, anchor and modal inspector controls now use transactions too.
- Added typed `Reparent` with explicit `Free` or `Flow` placement. Free placement requires a Canvas parent or the document root, sets measured pixel dimensions and absolute offsets, and clears margins to preserve the measured box. Flow placement requires a non-Canvas parent, clears offsets and disables absolute positioning while retaining sizing. Both preserve IDs, descendants, reference targets and document order. Invalid parents, cycles and projected widgets reject atomically.
- The parent picker filters descendants and projected targets. Reparenting into a Canvas or the root uses matching runtime geometry and converts the measured box into the new parent's frame. Non-invertible, rotated or reflected relative transforms reject rather than moving the box unexpectedly. Flow parents explicitly take over positioning; the parent label explains this behavior. Geometry must be ready for free placement.
- Fixed the player renderer's legacy anchor shift on absolute root widgets, including clearing an existing shift when switching to absolute layout. Real Bevy checks verify that reparenting into a padded Canvas and back to the root preserves the measured box.
- The existing begin/update/commit/cancel commands expose these edits through dispatch, shell and MCP without new commands or protocol changes. Each update still replaces one edit relative to the starting document. Other inspector fields, creation, deletion, prefab operations and import retain whole-document writes.

### Snapping and resize handles (sixth increment)

- Added eight corner/edge resize handles with cursors derived from runtime transforms. Each gesture freezes its starting frame, handle direction and parent transform so preview invalidation does not drop its pointer grab. Edge handles retain the other dimension. Resize offsets account for transformed widget axes and anchors, preserving the opposite edge/corner for left/top drags as well as right/bottom drags. Authored min/max sizes and the one-pixel minimum constrain the result.
- Added opt-in Grid snapping with a 1-256 logical-pixel step (8 by default). Moves snap authored offsets; resize drags snap active dimensions. Constraints take priority when a limit does not fall on the grid. Inspector values remain exact. Hold Shift during a drag to bypass both grid and alignment snapping.
- Added opt-in edge/center alignment to the parent box and visible authored siblings in the same parent. Targets come from matching runtime geometry and stay frozen throughout the gesture. Hidden/projected widgets, generated collection rows and other parent trees do not contribute targets. The capture tolerance is six pixels on the editor viewport, converted through zoom and the parent transform.
- Orange guide lines show aligned axes during move/resize previews and clear on bypass, commit, cancellation or failure. Move alignment supports transformed parents; resize edge alignment requires widget axes parallel to its parent. Rotated relative widgets still resize and grid-snap, with the opposite corner preserved.
- All handles and snapping use the existing typed move/resize transactions. Release commits one undo step; Escape discards it. No new backend, shell/MCP commands or protocol messages are needed. Fully clipped widgets do not expose viewport handles.

### Keyboard nudging and sibling order (seventh increment)

- Arrow keys move an editable widget by one logical pixel in its parent's authored coordinates; Shift+arrows use ten pixels. Nudging uses exact offsets independently of drag snapping. Repeated presses and simultaneously held arrow keys share a draft and commit once after the final key release. A new press can extend a released draft while its final update is pending. Escape, focus loss, selection changes and the existing external-edit/tab/viewport cancellation paths discard the draft. Inspector/control focus, gameplay, projected widgets and flow positioning do not start nudges.
- Added typed `Reorder {id,index}` with a zero-based final index among widgets with the same parent. It permutes sibling slots in the document array, preserving all other slots, IDs, descendants, parent links and reference targets. Missing IDs and invalid indices reject atomically; the existing transaction commands provide draft isolation, one-save commits and undo/redo. Shell/MCP edit metadata documents the new shape.
- Added Earlier/Later hierarchy controls for both free and flow widgets. Boundary controls disable, selection follows the ID and the UI explains that sibling order controls flow layout and drawing. The hierarchy still needs the full tree/search overhaul in phase 2.
- Runtime spawning now walks parents before children, preserving sibling order even when a root reorder leaves a child earlier in the serialized array than its parent. Real Bevy checks cover flow placement, paint order and root reordering across child slots.

### Creation, deletion and duplication (eighth increment)

- Added typed `Create {widget}`, `Delete {id}` and `Duplicate {id,new_id,offset?}` edits. Create appends and normalizes placement from the parent (Canvas means free, anything else flow) and rejects projected widgets, unknown parents and duplicate or empty IDs. Delete removes the whole subtree and clears `scroll_target`s that pointed into it. Duplicate copies the subtree directly after the original under `new_id` and `new_id.<old id>`, remapping `parent` and `scroll_target` inside the copy only; an optional `offset` moves a free-placed root and a flow child rejects it.
- The Interface palette, Duplicate and Delete buttons now use the transaction commands (no `set_interface`), select the created or copied widget and clear selection on delete. A palette click adds inside the selected container; a drag still adds to the root.
- Block and script references to the original IDs are deliberately not rewritten; duplicated widgets are new names.

### Review against recent changes (2026-10-08)

Master since the seventh increment (Phase 6 2D work, script guests, bundled rustc, clippy/let-chain cleanups) left the interface pipeline intact: the only UI-file changes were formatting-level edits and the wall-clock pause fix in `overlay.rs`. Adjustments to the plan:

- Protocol version is 35 (the screen isolation note's 23 was stale). Any phase 1 message change bumps from there.
- 2D now has screen feedback (`screenfx.rs`: cover z 52, flash z 54, floaters) and lighting (`unlit_above`) drawn around the interface. The design session must stay free of both, and phase 2's "Interact" preview should show cover/flash z-order rather than ignore it.
- Interface strings now come from the project's string table (`text for key`, `set language`). Phase 4 text editing and phase 6 localization samples should preview through the table instead of adding a second mechanism.
- Scripts gained more guest languages and ABI 47 slots (save slots, locale). Phase 5's binding and event wiring should use the script ABI as it stands and add verbs only through `abi.rs`.
- The remaining order is unchanged: finish phase 0/1 (schema versioning, property metadata, no world reload on commit), then the phase 2 hierarchy overhaul, then marquee/multiselection and alignment tools.

### Hierarchy tree, search and lock (ninth increment)

- The hierarchy is a real tree in sibling order: indentation by depth, fold arrows, and a search box matching ID or content (matches keep their ancestors and ignore folds). It follows screen isolation.
- Per-widget Lock is editor-only state (never saved, never sent to the runtime): locked widgets are skipped by viewport picking and lose move/resize/nudge handles, but stay selectable from the tree and editable in the inspector.
- Editor hide is a per-row toggle. `InterfaceDesign` gained `hidden` (protocol 36): the design session hides those widgets and their subtrees in its temporary manager, like screen isolation, so the document, game visibility and saves are untouched. Unknown IDs reject the request atomically. Hidden widgets stay in the reported geometry with effective visibility false, so they cannot be picked.

### Schema versioning (tenth increment)

- `UiDocument` has a `version` (`UI_SCHEMA_VERSION` = 1). A document with no version loads as 0 and `migrate()` stamps it as 1 without touching IDs, order or geometry; migration is idempotent. It runs in `Project::normalize` (per scene) and `set_interface`.
- A document newer than this build is not rewritten: `validate()` refuses it with an upgrade message, so edits, previews and `set_interface` fail instead of silently dropping fields. Opening still works. Run-time loading is unaffected.
- Later steps (style inheritance, component instances) add a version and a `migrate` arm each; the stored shape stays serde-defaulted so older builds keep ignoring nothing they cannot read.

### Property metadata and multiselection (eleventh increment)

- `ui::property_metadata()` lists every `SetProperty` path with its label, group, type, choices and the widget kinds it applies to; a test keeps it in step with `UiPropertyEdit` (`path()` is an exhaustive match). It is exposed as `interface-properties` in dispatch, shell and MCP so inspectors and agents read one source. The inspector still draws its own controls from QML; moving it onto this table is the next step, together with the remaining styling paths.
- Added `Batch {edits}`: edits applied in order as one, all or nothing, one undo step. It cannot nest and holds at most 1000 edits.
- The viewport supports Shift-click and a marquee (drag on empty canvas; widgets whose centres fall inside, skipping locked and projected ones). The tree supports Ctrl/Shift-click. Delete (button or key) and Duplicate act on the whole selection as one Batch, skipping widgets whose ancestor is also selected. Dragging, handles and nudging still act on the primary widget only; group move and alignment/distribution are next.

### Group move, align and distribute (twelfth increment)

- Dragging a widget that belongs to a multiselection moves the whole selection when every member is free-placed under the same parent: the draft is one `Batch` of `Move`s sharing the primary's snapped delta. Mixed selections drag only the primary widget. Handles and keyboard nudging remain primary-only.
- Align (left, middle, right, top, middle, bottom) and Space H/V (equal gaps between the outer widgets) are QML computations over the runtime geometry, in the shared parent's frame, committed as one `Batch` of `Move`s. They require free-placed, unrotated widgets with one parent and report why otherwise. They write exact authored offsets, so no new backend command is needed.

### Reload-free commit and typed content/style edits (thirteenth increment)

- Committing an edit while a design session is open no longer reloads the world: the session already shows the committed document and Play sends the whole project. Without a session (or a runtime) the old sync still runs. A backend test asserts one `Load` across a preview and a commit.
- `SetProperty` now also covers `element.range`, `style` (all five states, sizes validated), `class`, `bindings` (validated binding properties), `items`, `tooltip`, `scroll_target` (must name a widget), `tab_index` and `transition`, all listed by `interface-properties`. New `SetDocument {theme?, scale?}` and `SetClass {name, styles|null}` cover document options and shared named styles. The inspector's tooltip, scroll target, tab page, items, transition, bindings, style and advanced JSON fields, and the theme/scale pickers, send these edits instead of `set_interface`. `world_actor`, prefab save/load and whole-document import still use it.

### Visual style editing (fourteenth increment)

- The Style section has colour swatches (`ColorField`) beside the hex fields, a font picker (`AssetField` for fonts, written to `fonts`), a Class dropdown over the document's named styles, Save style as class and Delete class. A class that a state does not override shows as a dimmed swatch and a `(class: value)` hint, so inherited values are visible. Saving and deleting a class are one `Batch` (`SetClass` plus the `class` property on affected widgets), so they undo together.

### Viewport overlays (fifteenth increment)

- Safe area and Reference size toggles draw dashed frames over the viewport from the document's `safe_area` and `reference_size` (scaled like the runtime does), as editor-only overlays. Two phone/desktop presets (1170 x 2532, 2560 x 1440) join the preview sizes. Rulers, draggable guides and zoom/pan polish remain open.

Remaining plan work, in order: draw the inspector from `interface-properties`, image pickers and nine-slice/fit modes, text-input caret/selection/IME, rulers/guides/safe-area overlay and zoom/pan polish (phase 2), reusable components with instances and overrides (phase 5), visual binding/event editors, animation timeline and responsive variants (phase 6), and the platform/DPI checks listed under phase 0. These are each a large increment of their own.

The next increment is moving the inspector onto the metadata, then the world-reload-free commit path. Schema versioning and property metadata (phase 1). Schema versioning, complete property metadata and the broader phase 1 contracts remain open. Committed edits still use the existing world synchronization path; avoiding a world reload on commit remains open. The current preview still shows the idle scene behind the interface.

Open phase 0 checks: embedded/process rendering and teardown on actual platforms; native presentation timing and process screenshot metadata on actual platforms; viewport resize/DPI/safe-area matrices; real image/font asset loading; screenshot baselines and gameplay input isolation with held inputs. Rotation, text editing/IME, nine-slice and animation capabilities remain unproven. The fixture image node currently has no asset.

To exercise the spike through an attached shell/MCP session, stop the game and send `preview-interface design={"revision":1,"generation":1,"document":{...}}`, then read `interface-layout`. Geometry is asynchronous and initially null. Increase revision for a new draft and generation for a new viewport. Omit `design` to return to the scene view. The Interface viewport shows the rendered UI over the idle scene, and both frame transports carry design revision metadata. Requests may include `viewport: [960,720]` for a physical-pixel resolution and `screen: "root-id"` to isolate a top-level widget tree. Omit `screen` or pass null to show all trees. A committed project edit invalidates an active transaction rather than rebasing it.

Verification for this increment: `cargo check --workspace --offline`; runtime library suite (392 passed, 61 ignored); app/protocol suites (all passed); Qt 6 `qmllint` on the two edited QML files (passed with unqualified-access warnings); formatting and diff checks. Existing attach/audio tests require local socket access outside the sandbox. GPU/platform screenshot checks remain outstanding.

Verification for the second increment: `cargo build --workspace --offline` passed; runtime library suite passed (396 tests, 62 ignored); backend/protocol suites passed; Qt Quick suite passed (43 tests, including viewport interaction tests with a mock frame source); Qt 6 QML lint passed with existing access warnings. The Vulkan design smoke test passed for projects saved with resolution scales of 1.0 and 0.5, checking rendered pixels and matching slot metadata. Formatting and diff checks passed. Actual Qt GPU presentation and process previews on other desktop platforms remain open checks.

Verification for screen isolation: `cargo build --workspace --offline` passed; all four runtime design-session tests passed, including real Bevy geometry before/after isolation; backend/protocol suites passed (59 tests) with local socket access; Qt Quick suite passed (46 tests); Qt 6 QML lint completed with existing warnings. Formatting and diff checks passed. Actual platform presentation and the other phase 0 checks remain open.

For a typed transaction, read `sync-status`, then call `begin-interface-edit revision=<revision>` and keep its returned token. Send `update-interface-edit token="<token>" edit={"kind":"Move","id":"button","offset":[40,60]}` or `{"kind":"Resize","id":"button","size":[180,48],"offset":[40,60]}`. Updates return the temporary document for `preview-interface`. Release with `commit-interface-edit token="<token>"`; discard with `cancel-interface-edit token="<token>"`. Each update replaces the preceding edit relative to the starting document.

Verification for typed move/resize transactions: the full workspace build passed; backend/protocol suites passed (60 tests); the targeted core edit test passed; Qt Quick suite passed (55 tests); MCP suite passed (7 tests), including schema generation from all 197 real shell commands. Qt 6 QML lint completed with existing warnings. Formatting and diff checks passed. Backend and MCP integration tests used local IPC outside the sandbox. Actual platform presentation, full DPI matrices and commit synchronization without a world reload remain open.

Examples: send `update-interface-edit token="<token>" edit={"kind":"SetProperty","id":"button","property":{"path":"layout","value":{"width":{"Percent":50},"height":{"Px":48},"gap":12}}}` to set an explicit layout; omitted layout fields take their shared defaults. Pass `value:null` to remove the explicit layout. For reparenting, send `edit={"kind":"Reparent","id":"button","parent":"menu","placement":{"mode":"Flow"}}` or `placement:{"mode":"Free","offset":[40,60],"size":[180,48]}` for a Canvas parent. Free placement deliberately materializes the measured size in pixels; flow retains authored sizing. Commit or cancel with the same token.

Verification for typed properties and reparenting: the final full workspace build passed; backend/protocol suites passed (61 tests); core interface tests passed (18 tests); runtime interface tests passed (23 tests) and all four design-session tests passed, including real Bevy geometry for Canvas/root reparenting. Qt Quick suite passed (59 tests), including safe-area/scaled-canvas conversions, flow layout edits and rejected transforms. MCP suite passed (7 tests). Qt 6 QML lint, formatting and diff checks passed. Backend/MCP integration tests used local IPC outside the sandbox. Platform presentation, full DPI matrices, complete property coverage and commit synchronization without a world reload remain open.

Verification for snapping and resize handles: the final full workspace build passed; Qt Quick suite passed (75 tests), including every handle's pointer grab and single commit, rotated/anchored resizing, min/max constraints, grid snapping, Shift bypass, guide cleanup, sibling filtering, transformed parent coordinates and zoom-dependent tolerance. Qt 6 QML lint completed with unresolved QQuickItem/GameView import warnings; the workspace build compiled the QML successfully. Formatting and diff checks passed. Actual platform presentation and the broader preview/DPI gates remain open.

For sibling order, use `update-interface-edit token="<token>" edit={"kind":"Reorder","id":"settings-row","index":0}` to move that widget to the first position among its siblings. The index is the final position, including the moved widget. Reordering does not reparent; use `Reparent` for that. Same-position commits do not create a save or undo entry.

Verification for keyboard nudging and sibling order: the final full workspace build passed; backend/protocol suites passed (61 tests), including reorder draft isolation, save/reload and undo/redo; core interface tests passed (19 tests); runtime interface tests passed (23 tests) and all four design-session tests passed, including flow/paint order after reordering a root across its child slots. Qt Quick suite passed (81 tests), including keyboard batching, delayed replies, Escape/focus cancellation, inspector focus isolation and typed flow/root order controls. MCP suite passed (7 tests). QML lint completed with existing QQuickItem/GameView import warnings; formatting and diff checks passed. Backend/MCP integration tests used local IPC outside the sandbox. Actual platform presentation, full input/DPI matrices and commit synchronization without a world reload remain open.
