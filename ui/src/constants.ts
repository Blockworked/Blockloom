// Fixed option lists shared by the palette and the block rows.

/** Keys a `when key pressed` header or a `key down?` reporter can name. Kept in
 * step with `blockloom_core::sense::KEY_NAMES`, which the runtime matches on. */
export const KEY_NAMES = [
  'space',
  'up arrow',
  'down arrow',
  'left arrow',
  'right arrow',
  'enter',
  'escape',
  'shift',
  'control',
  'alt',
  'tab',
  'backspace',
  ...'abcdefghijklmnopqrstuvwxyz'.split(''),
  ...'0123456789'.split(''),
];

export const KEY_OPTIONS = KEY_NAMES.map(name => ({ value: name, label: name }));

export const AXIS_OPTIONS = [
  { value: 'X', label: 'x' },
  { value: 'Y', label: 'y' },
  { value: 'Z', label: 'z' },
];

export const BODY_OPTIONS = [
  { value: 'None', label: 'none (blocks only)' },
  { value: 'Static', label: 'static' },
  { value: 'Dynamic', label: 'dynamic' },
  { value: 'Kinematic', label: 'kinematic' },
];

/** How an attached camera frames its actor. In a 2D project all three mean
 * "centre on the actor", so the block still reads sensibly either way. */
export const CAMERA_VIEW_OPTIONS = [
  { value: 'Follow', label: 'follow' },
  { value: 'FirstPerson', label: 'first person' },
  { value: 'ThirdPerson', label: 'third person' },
];

export const VISIBLE_OPTIONS = [
  { value: 'true', label: 'show' },
  { value: 'false', label: 'hide' },
];

export const MOUSE_LOCK_OPTIONS = [
  { value: 'true', label: 'lock' },
  { value: 'false', label: 'unlock' },
];

/** Which corner, edge or centre of the window a top-level element hangs off.
 * Kept in step with `blockloom_core::ui::UiAnchor`. */
export const UI_ANCHOR_OPTIONS = [
  { value: 'TopLeft', label: 'top left' },
  { value: 'Top', label: 'top' },
  { value: 'TopRight', label: 'top right' },
  { value: 'Left', label: 'left' },
  { value: 'Center', label: 'centre' },
  { value: 'Right', label: 'right' },
  { value: 'BottomLeft', label: 'bottom left' },
  { value: 'Bottom', label: 'bottom' },
  { value: 'BottomRight', label: 'bottom right' },
];

/** What `set [prop] of (id) to` can write - `blockloom_core::ui::UiProp`. */
export const UI_PROP_OPTIONS = [
  { value: 'Text', label: 'text' },
  { value: 'TextColor', label: 'text color' },
  { value: 'TextSize', label: 'text size' },
  { value: 'Background', label: 'background' },
  { value: 'Width', label: 'width' },
  { value: 'Height', label: 'height' },
  { value: 'Visible', label: 'visible' },
  { value: 'CornerRadius', label: 'corner radius' },
  { value: 'Padding', label: 'padding' },
  { value: 'Modal', label: 'modal' },
  { value: 'Min', label: 'min' },
  { value: 'Max', label: 'max' },
  { value: 'Value', label: 'value' },
];

/** A panel either swallows the clicks behind it or it doesn't. */
export const MODAL_OPTIONS = [
  { value: 'false', label: 'floating' },
  { value: 'true', label: 'modal' },
];

/** A toggle starts one way or the other. */
export const ON_OFF_OPTIONS = [
  { value: 'true', label: 'on' },
  { value: 'false', label: 'off' },
];

/** The palette offers "mouse" alongside the actor names, same as the blocks do. */
export const MOUSE_TARGET = 'mouse';
