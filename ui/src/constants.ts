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

export const VISIBLE_OPTIONS = [
  { value: 'true', label: 'show' },
  { value: 'false', label: 'hide' },
];

/** The palette offers "mouse" alongside the actor names, same as the blocks do. */
export const MOUSE_TARGET = 'mouse';
