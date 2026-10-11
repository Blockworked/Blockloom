# Hub contract fixtures

The small files the editor and the Hub both read or write.
`blockloom-core/tests/hub_contract.rs` (the editor's side) and
`blockloom-hub-state/tests/contract.rs` (the Hub's side) load these same
files, and each checks that what it writes has exactly these keys, so neither
side can change a format without the other test failing.

| File | Lives at | Writer | Reader |
| --- | --- | --- | --- |
| `lock.json` | `<project>/.blockloom/` | editor | Hub |
| `hub.json` | `<project>/.blockloom/` | Hub, editor (new projects, rebind) | both |
| `focus.json` | `<project>/.blockloom/` | Hub | editor (deletes it to acknowledge) |
| `projects.json` | `<data dir>/blockloom/` | editor, Hub | both |
| `presence.json` | `<data dir>/blockloom/editors/<pid>.json` | editor (every poll) | Hub |
| `request.json` | `<data dir>/blockloom/editors/<pid>.request.json` | Hub | editor (deletes it to take it) |
| `reply.json` | `<data dir>/blockloom/editors/<pid>.reply.json` | editor | Hub |

An editor at its Dashboard is reusable: its presence names the installation
and no project, and is fresh (a few seconds old). The Hub drops a request, and
the editor answers with a reply carrying the same `id`.
