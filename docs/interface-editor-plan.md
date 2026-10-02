# Interface editor overhaul

Status: implementation started. Phase 0 has a runtime design session, frame-matched selection and screen isolation in the Interface workspace; its preview gate is still open.

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

The next increment is a typed, undoable move/resize transaction. The current preview still shows the idle scene behind the interface.

Open phase 0 checks: embedded/process rendering and teardown on actual platforms; native presentation timing and process screenshot metadata on actual platforms; viewport resize/DPI/safe-area matrices; real image/font asset loading; screenshot baselines and gameplay input isolation with held inputs. Rotation, text editing/IME, nine-slice and animation capabilities remain unproven. The fixture image node currently has no asset.

To exercise the spike through an attached shell/MCP session, stop the game and send `preview-interface design={"revision":1,"generation":1,"document":{...}}`, then read `interface-layout`. Geometry is asynchronous and initially null. Increase revision for a new draft and generation for a new viewport. Omit `design` to return to the scene view. The Interface viewport shows the rendered UI over the idle scene, and both frame transports carry design revision metadata. Requests may include `viewport: [960,720]` for a physical-pixel resolution and `screen: "root-id"` to isolate a top-level widget tree. Omit `screen` or pass null to show all trees. A committed project edit cancels the draft rather than rebasing it.

Verification for this increment: `cargo check --workspace --offline`; runtime library suite (392 passed, 61 ignored); app/protocol suites (all passed); Qt 6 `qmllint` on the two edited QML files (passed with unqualified-access warnings); formatting and diff checks. Existing attach/audio tests require local socket access outside the sandbox. GPU/platform screenshot checks remain outstanding.

Verification for the second increment: `cargo build --workspace --offline` passed; runtime library suite passed (396 tests, 62 ignored); backend/protocol suites passed; Qt Quick suite passed (43 tests, including viewport interaction tests with a mock frame source); Qt 6 QML lint passed with existing access warnings. The Vulkan design smoke test passed for projects saved with resolution scales of 1.0 and 0.5, checking rendered pixels and matching slot metadata. Formatting and diff checks passed. Actual Qt GPU presentation and process previews on other desktop platforms remain open checks.

Verification for screen isolation: `cargo build --workspace --offline` passed; all four runtime design-session tests passed, including real Bevy geometry before/after isolation; backend/protocol suites passed (59 tests) with local socket access; Qt Quick suite passed (46 tests); Qt 6 QML lint completed with existing warnings. Formatting and diff checks passed. Actual platform presentation and the other phase 0 checks remain open.
