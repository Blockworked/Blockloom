# In-game interfaces

Open the editor's **Interface** tab to add widgets, move them on the canvas,
choose parents in the hierarchy, and edit their styles. Resolution presets
preview placement at landscape and portrait sizes. Play runs the actual Bevy
layout and input systems. Designer edits participate in undo and project saves.

An interface is saved in `world.interface`. Save an interface asset to
`assets/ui/menu.json` to reuse it in another scene or project. Loading an asset
replaces the current interface in one undoable edit. Prefabs stored in the
interface can be inserted with fresh widget IDs.

## Layout and widgets

The original Panel, Label, Button, Image, Input, Slider, Toggle and List widgets
remain available. The generic `show widget` block and script `Ui::new` support
VerticalBox, HorizontalBox, Grid, Canvas, WrapBox, SizeBox, Spacer, Progress,
RadialProgress, ListView, Tabs, Select, Scrollbar, RichText and Tooltip.

Bevy's retained UI layout measures text and child content, then arranges flex
and grid containers. A widget keeps its entity until its kind or parent changes.
Property writes dirty the existing node; repeating an identical show does not
rebuild it. This avoids repeated layout for static HUDs, although input and
binding systems still inspect the interface each frame or fixed tick.

`layout` accepts `width`/`height` as `"Auto"`, `{"Px":180}` or
`{"Percent":100}`, plus `min_size`, `max_size`, `padding`, `margin`, `gap`,
`columns`, `grow`, `align`, `absolute` and `row_height`. Four-edge arrays are
left, top, right, bottom. A zero maximum means unconstrained. Element `size`
provides the legacy pixel dimensions; use zero for content sizing. Canvas
children use nine-point anchors and offsets. Other children flow unless their
layout is absolute. Alignment values are Stretch, Start, Center and End.

ListView uses a recycled pool covering the viewport plus one row of overscan on
each side. `items` is an array of strings. Selection is one-based; zero means
no selection. Tabs and Select use the same selection convention. Give a child
of Tabs a `tab_index` to show it only for that page; use an absolute layout with
an offset below the tab headers for page content. A Scrollbar's `scroll_target`
links it to a ListView, synchronizing pixel scroll offset in both directions.

RichText supports nested `[b]`, `[i]` and `[color=#RRGGBB]` tags, with matching
closing tags. Text wraps within constrained dimensions and measures to its
content when unconstrained. Radial progress uses 64 segments. A widget with
`world_actor` set to an actor ID follows that actor's projected position, while
its explicit visibility is preserved. This is a screen-facing overlay, not a
3D mesh that participates in scene depth.

## Styles and bindings

The document's theme is Dark, Light or HighContrast. Each widget can select a
`class` from document `styles`, then override it with `style`. Both use the same
state keys: normal, hover, pressed, disabled and focused. A state inherits
unspecified properties from normal. Existing per-element block overrides take
precedence. Paint properties are background, text_color, text_size,
border_color, border_width, radius, shadow and fonts. Fonts are an ordered list
of project font asset paths or font family names.

`stylesheets` lists project-relative JSON assets containing a map of class
names to styles. Later sheets override earlier classes; inline document styles
win last. Small loaded UI images share a runtime atlas (up to 64 images of at
most 512 pixels per side), with larger images retaining their own textures.

Bindings are sampled before the fixed-tick scheduler so VM and compiled blocks
observe the same UI values. For example, a health label can use:

```json
{
  "property": "Text",
  "source": {"Component": {"actor": "player", "component": "Health", "field": "hp"}},
  "converter": {"prefix": "HP: ", "decimals": 0}
}
```

A two-way input can use:

```json
{
  "property": "Value",
  "source": {"Variable": {"actor": "", "name": "playerName"}},
  "two_way": true
}
```

Supported properties are Text, Value, Visible and SelectedIndex. Empty variable
actor IDs address global variables. Converters support prefix, suffix, scale,
decimals and boolean invert. Two-way converters remove their prefix/suffix and
reverse the scale; malformed input leaves the source unchanged.

The blocks `bind`, `set items`, `scroll to` and `set element theme` use the same
property path as `set UI property`. Bindings and items are JSON text values.
Scripts expose `bind_ui`, `set_ui_items`, `scroll_ui_to`, `set_widget_theme` and
`ui_selected_index`. Existing show/hide/delete and value/shown reporters remain.

## Interaction and canvas

The UI event hat accepts press, release, hover, leave, drag, scroll and focus.
Pointer events bubble through ancestors up to the nearest modal. The existing
clicked hat also bubbles. Modals block world clicks and focus navigation outside
the modal subtree. Arrow keys and gamepad D-pad move focus; Enter, Space or the
south gamepad button activate it. Text inputs retain their existing keyboard
capture rules. Touch scrolls lists and activates controls; wheel input routes
to the nearest scrollable ancestor.

Set the Transition property to a duration in seconds for background and pixel
size interpolation plus scale-based show/hide animations. The document's
`safe_area` insets the canvas. ConstantPixel preserves logical pixel sizes;
ScaleWithSize scales against `reference_size`, using the smaller available-axis
ratio. Percent sizes follow their containing box. The runtime applies display
DPI scaling in addition to canvas scaling.

The designer is a placement and style preview. Text metrics, content-dependent
layout, animations, bindings and interactions are authoritative in Play.
The existing input widget remains append/backspace based, without selection or
IME editing.

## Shell and MCP

The same backend commands are available through the shell and generated MCP
registry: `set-interface document={...}`, `save-interface-asset name=menu.json`
and `load-interface-asset path=assets/ui/menu.json`. Invalid hierarchy cycles,
missing parents and duplicate IDs are rejected before saving.
