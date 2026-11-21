# Shared UI rules for the editor and the Hub

Both apps use the editor's `Theme` values (blockstitch `Theme.qml`). The Hub
copies them as literals because it does not link blockstitch.

| Role | Value |
| --- | --- |
| window | `#1d1e20` |
| panel / card | `#242527` |
| raised / button | `#303134` |
| field | `#292a2d` |
| border | `#4a4b50` (soft `#36373a`) |
| text / dim | `#dedee0` / `#96979c` |
| accent / hover | `#1597ff` / `#32a5ff` |
| danger | `#ff4848` (text `#ff7b7b`) |
| warning | `#f5b942` |
| radius | 6 (cards 10) |

One primary button per view (accent fill). Secondary actions are raised
buttons; destructive actions live in a menu and need confirmation.

## Words

| Action | Word |
| --- | --- |
| start a project in an editor | Open |
| pick another editor for a project | Change editor |
| make a new project | New project |
| add an existing folder to the list | Add existing project |
| drop from the list, folder kept | Remove from list |
| erase the folder | Delete project folder |
| add an editor | Install |
| remove an editor | Uninstall |
| point at a moved folder | Locate folder |

## Project rows

Both apps show: name, path, dimension, last opened, editor binding and a
busy/unavailable reason, in that order.

## Editors and projects

- An installation is always named for a person: "Blockloom 0.2.0", or
  "Development build" for a repository build. Ids (`release-0.2.0`,
  `dev-<hash>`) never appear in a label or message.
- A project with no editor set opens with the newest release and is set to
  it. "Change editor" is for choosing another.
- One primary button per view. A disabled button looks disabled (flat gray),
  never a dimmed accent.
- A project open in another editor: the Hub says so and offers Switch to
  editor; the editor's card says opening it joins that session.
- A row is a button: click or Enter opens it, its menu holds Show in folder,
  Backups and Remove from list.

## Interface designer

The designer's own chrome (palette, tree, toolbar, inspector) uses the Theme
values above and the `Bw*` controls, `InspectorRow`, `IconButton`,
`NumberField` and `ChoiceField`. Only the design surface draws with its own
colours, declared once at the top of `UiViewport.qml`:

| Role | Value |
| --- | --- |
| stage behind the frame | `#171a21` |
| rulers / ruler text | `#1d222c` / `#8892a6` |
| selection / marquee | `#70baff` |
| hover outline | `#b9dfff` |
| also selected | `#8fd0a0` |
| safe area | `#ffb454` |
| reference size | `#c792ea` |
| guides | `#4dd0e1` |
| snap lines | `#ffcc70` |
| canvas behind the frame (dark / light theme) | `#252d3a` / `#d9dfe8` |

Panels fit a 1100x700 window: the side panels have min and max widths and a
drag handle, and the toolbar wraps instead of widening the centre column.
