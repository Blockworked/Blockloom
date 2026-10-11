# Hub coherence plan

Follow-up to `docs/hub-plan.md`. Turns the Hub review (`~/hub`, also saved as
`~/hub.txt`) into buildable work. No application code was changed for the
review itself.

## Status

All items are built. Checks: `scripts/test-hub.py` (63), 
`blockloom-app/tests/hub_coherence.rs` (8), Hub UI smoke
(`scripts/test-hub-ui.py`, 20 scenarios including empty, unavailable,
missing editor, no match and busy), and `tst_Dashboard.qml`.

- 1: `blockloom_core::hub::conflict` runs in `open_project` (Dashboard, Open a
  folder, `--project`, shell, MCP, attach). A mismatch, or an editor the Hub
  does not manage, is refused with `wrong-editor:` unless `other_editor=true`;
  the editor offers "Open anyway". New and imported projects bind to the
  running installation.
- 2: Hub New project (name, location preview, dimension, sample, editor
  choice) starts the editor with `--new-project`, so its creation rules apply.
  The Hub never launches a bare editor.
- 3: busy comes from the owner lock; a launch record counts only for
  `STARTUP_GRACE` seconds. Launch confirms the editor holds the project.
  Switch to editor writes `.blockloom/focus.json`; the owning editor polls
  `take_focus_request` and raises its window.
- 4: one library, the editor's `projects.json`. Missing folders stay listed
  with a cause, Locate folder and Remove from list.
- 5: delete refuses a folder another editor owns, moves to the OS trash by
  default (`permanent=true` to erase), sits in a menu and needs the name typed.
- 6: close, open, create, import, forget, rename and quit stop on a failed
  save (`save-failed:`) with Retry or Discard / Quit without saving.
- 7: `docs/ui-tokens.md` holds the shared tokens and words; the Hub uses the
  editor's palette and terms.
- 8: primary Open, Change editor secondary, last opened, editor and reason on
  rows, empty-search message, staged progress (`Stage:` lines), More menu for
  advanced installs, module versions and approximate sizes with an "All
  Android build tools" choice and the default rule shown, Dashboard search and
  Enter to open.

Still open before release: Windows and macOS interactive passes (the Windows
Recycle Bin and macOS trash paths are untested), and exact module sizes
(they are approximate until a download starts).

## Follow-up review (2026-10-10)

A second review of the built result found gaps between this plan's "built"
and the running app. All of it is now implemented:

- The editor's picker re-reads the library every 3 seconds while the
  Dashboard shows and when its window is activated (`refresh_library`), so a
  Hub add, forget or open shows without a restart. Cards carry the editor's
  friendly name (`installation_label`) and `wrong_editor`, and the card says
  "Set to Blockloom X, not this editor" before the click.
- "Open anyway" is "Open once" or "Use this editor for the project"
  (`rebind`). Either copies `project.blockloom` to
  `.blockloom/backups/` first. Messages use friendly names
  (`hub::friendly_name`), never ids.
- Hub: background refresh runs on its own process (`HubService::poll`) and
  never sets busy. The operation lock records its pid and a dead owner's lock
  is cleared. `launch` and `new-project` release the lock before waiting for
  the editor; New project at the default location is confirmed through the
  library instead of waiting out the timeout.
- Open on a project with no editor binds the default one (newest release)
  and says so. Rows open on click and Enter, have a menu (Open, Show in
  folder, Backups, Remove from list) and the right-click menu. Add existing
  asks about the editor only when none is set.
- Switch to editor reports whether the editor heard the request
  (`acknowledged`); the request is withdrawn if nobody did.
- Backups: `backups` lists a project's snapshots and `restore-backup` puts
  one back after keeping the current state as a snapshot. The Hub's project
  menu has a Backups dialog; the global "Show backup" button is gone.
- Interplay contract: `blockloom-core/tests/fixtures/hub-contract/` holds the
  lock, binding, focus, library and presence/request/reply files, and both
  `blockloom-core/tests/hub_contract.rs` (editor side) and
  `blockloom-hub-state/tests/contract.rs` (Hub side) check against them.
- One editor process per Open: an editor at its Dashboard publishes presence
  (`<data dir>/blockloom/editors/<pid>.json`, every 1.5 s through
  `hub_poll`). Open and New project in the Hub hand a request to a fresh idle
  editor of the right installation instead of starting another process, wait
  for its reply, and start a new editor only when there is none or it does
  not take the request within a few seconds. An editor with a project open
  is never reused, and a request it refuses comes back as an error, not a
  second editor.
- Project and editor logic is Rust: `blockloom-hub-state` holds the project
  list, owner checks, binding, snapshots and restore, focus, and starting or
  reusing editors. The Hub window runs it as itself (`blockloom-hub
  --hubctl <command>`), `scripts/hub.py` forwards those commands and asks it
  for the installation guards, and `blockloom-hubctl` is the same code as a
  standalone binary for scripts and tests. Python keeps only installing,
  downloading and building editors.

Decided, not built:

- A project open in another editor: the Hub blocks Open and offers Switch to
  editor (it never forks a copy); the editor's picker lets a person join that
  session on purpose, and says so on the card. Two apps, two jobs.
- An editor with a project open is not reused for another project: it holds
  one document, and silently swapping a person's work out is worse than a
  second window.
- Windows and macOS are untested for the state tool (process identity uses
  `sysinfo`; the fake-editor tests are Unix-only).

## Review basis

- Reviewed the implementation, fresh Hub screenshots and a rendered editor
  project picker.
- Ran the existing checks on Linux headless: 80 service tests and 15 Hub UI
  smoke scenarios passed.
- Scope was Linux headless only, not a full interactive assessment on Windows
  or macOS.
- Finding: the Hub has a solid installation foundation, but the overall
  experience is not yet coherent. It is usable for someone developing
  Blockloom, but much less convincing as the front door for someone who just
  wants to make a game.

## Product direction

Make the Hub the complete entry point for creating, finding, and opening
projects, with editor management alongside it. Keep convenient project
switching inside the editor, but make it use the same library, ownership
checks, and version-routing rules.

Largest improvement comes from making project opening predictable across both
apps. Cosmetic polish helps, but it cannot compensate for conflicting
behavior.

Build in this order:

1. Fix version routing, ownership checks, and failed-save navigation.
2. Unify project membership and current-project tracking.
3. Complete the first-run journey.
4. Improve recovery and progress feedback.
5. Align styling and simplify the controls.

## 1. Editor bypasses the Hub selected version

Problem: the Hub associates a project with an installation, but the editor
Dashboard opens any selected project in the editor process already running.
It does not consult that association.

Example: launch project A with editor version 1, choose "All projects", then
open project B assigned to version 2. B opens in version 1. That undermines
the reason to select an editor in the Hub and can expose projects to
unintended format changes.

Current code: `blockloom-app/src/commands.rs:284` (`open_project`). The
Dashboard cards and "Open a folder" both land there.

Plan:

- Define one version decision used by every project-opening route, including
  folder opening and imports: read `.blockloom/hub.json`, compare with the
  running editor installation identity, and refuse or reroute on mismatch.
- Decide the UX for mismatch: at minimum block a silent open in the wrong
  version. Options are relaunch via the Hub, prompt to switch, or open
  read-only. Pick one and apply it everywhere.
- Cover `open-project` from shell/MCP/attach the same way.
- Add regression tests: two fake installations, project B bound to version 2,
  attempt open from a version 1 backend, assert refusal or reroute, never a
  silent open.

Acceptance: no route opens a bound project in a non-matching editor without
an explicit user decision.

## 2. First-run onboarding dead end

Problem: the Hub empty state tells users to add an existing project. It
offers no project creation and no way to launch an installed editor without
a project. Installing an editor still leaves a new user without a clear next
action.

The editor has the better starting experience: "Create project", a location
preview, dimension explanations, and a sample. Those capabilities need to be
reachable from the Hub.

Current code: `blockloom-hub/qml/Main.qml:363` (Hub project screen).

Plan:

- Make New project a primary Hub action, with an editor choice included when
  necessary (unbound default, exactly one installation, or several).
- Reuse the editor creation semantics: name, location preview, dimension
  choice with explanations, sample option, default scene handling.
- Add a launch-editor-without-project path, or an explicit decision that the
  Hub never launches bare editors plus a New project button that always lets
  the user proceed.
- Update the empty state copy: create, add existing, and launch are all
  visible. Creation is primary.

Acceptance: a fresh install with zero projects shows how to make a game in
one click, and never strands the user after installing an editor.

## 3. "In use" stays wrong after leaving a project

Problem: the Hub records the project passed to an editor at launch and
considers it active while that process lives. Returning to the Dashboard
releases the project lock, but the launch record still marks the original
project busy. That blocks opening or changing its editor after the user has
left it. The underlying detection behavior was reproduced with an isolated
fixture.

Current code: `scripts/hub.py:275` (`project_active`, `running_editors`,
`require_idle`).

Plan:

- Track installation usage and current project usage separately. Process
  alive means the installation is in use. Project busy means a live owner
  lock or heartbeat for that folder.
- On Dashboard return, the previous project must read idle once its lock is
  released, even while the editor process continues.
- Handle externally launched editors that never wrote `running.json`:
  owner locks are the source of truth for project busy, not launch records.
- Add Switch to editor for an already-open project instead of only a
  disabled button. Disabled Open with no path forward is not enough.
- Extend `scripts/test-hub.py` style coverage: launch, return to Dashboard,
  assert idle; open elsewhere, assert busy; stale PID records never count.

Acceptance: leaving a project clears its busy state; the Hub never blocks
an idle project because its editor process still runs.

## 4. Project lists disagree on membership and recovery

Problem: the Hub merges its own registry with the editor registry. The
editor reads only its registry. Consequences reproduced or observed:

- Adding a folder to the Hub does not immediately add it to the editor
  picker.
- Forgetting a Hub-added project in the editor leaves it in the Hub.
- The editor removes unavailable or unreadable projects from its registry;
  the Hub retains them while their entries remain available to it.
- Deleting a Hub-added project can leave an unavailable entry with no Hub
  removal action.

The forget inconsistency was reproduced with a fixture.

Current code: `scripts/hub.py:301` (Hub registry merge), 
`blockloom-core/src/library.rs:145` (`list` drops gone or unreadable
folders and rewrites the file).

Plan:

- Define one consistent library: same membership in Hub and editor picker,
  same treatment of missing folders, same forget and remove actions.
- Decide the missing-folder rule explicitly. Proposal from the review: keep
  remembered projects, show Locate folder and Remove from list for
  unavailable entries, and never let a disconnected drive silently erase a
  remembered project. That means changing `library::list` pruning behavior.
- Make Hub add visible in the editor picker without restart, and editor
  forget visible in the Hub. Decide which registry is canonical or how the
  two sync.
- Fix the dead-end case: a deleted project must not leave an unavailable
  entry with no Hub removal action.
- Tests: Hub add then editor list, editor forget then Hub list, missing
  folder stays with recovery actions, disconnected drive does not prune.

Acceptance: Hub and editor show the same projects; unavailable entries offer
Locate folder and Remove from list; missing storage never silently forgets.

## 5. Project deletion lacks active-project protection

Problem: the Dashboard exposes permanent folder deletion, but its backend
does not check whether another editor owns that folder. One editor can
delete a project another editor is using.

Current code: `blockloom-app/src/commands.rs:421` (`delete_project`).

Plan:

- Apply the same ownership protection as Hub operations (`require_idle`
  semantics, live owner check) to Dashboard deletion.
- Move permanent deletion into a clearly labeled project menu, away from
  one-click icons. Prefer OS trash where practical over immediate recursive
  delete.
- Confirm the dialog names the project, warns when it is open elsewhere,
  and refuses while another editor owns it.
- Tests: owned folder deletion refused; unowned deletion still works;
  forget does not touch the folder.

Acceptance: no editor can permanently delete a folder another live editor
owns.

## 6. Returning to project selection can hide a failed save

Problem: closing a project attempts autosave, but a failure is logged and
the project is still discarded from the current editor state. The user can
reach the Dashboard without being told the save failed. "All projects" looks
like ordinary navigation, which makes this a trust issue.

Current code: `blockloom-app/src/commands.rs:77` (`auto_save` logs instead
of surfacing), `blockloom-app/src/commands.rs:676`
(`close_open_project` discards after a best-effort save).

Plan:

- Make close return the save result. On failure, stop navigation and offer
  retry or explicit discard. Never silently drop unsaved work.
- Apply to all close paths: Dashboard return, open-other-project, forget,
  rename flows that save first, and quit.
- Surface which project failed and why, with the folder path in the message.
- Tests: unwritable folder plus close, assert the project stays open and an
  error is shown; explicit discard then closes.

Acceptance: a failed save always blocks navigation until the user chooses
retry or discard.

## 7. Visual coherence

Finding: both screens are readable, but they feel like separate products.
Hub blue-gray surfaces and purple accents differ from editor neutral gray
and bright blue. Controls, project presentation, terminology, and
information hierarchy also differ. Neither needs a dramatic redesign, but
they need shared visual rules.

Plan:

- Write down shared tokens first: surfaces, text colors, accent, danger,
  spacing, corner radius, button hierarchy. Then apply to both apps.
- Align terminology: project open, editor select, install, remove, delete,
  forget. One word per action everywhere.
- Align project rows: same name, recency, editor association, and active
  status in both lists.
- Do this after sections 1 to 6. Do not let styling land before behavior is
  predictable.

## 8. Specific UI and UX weaknesses

Each item below was observed in the review. Fix with the behavior work
above, not as isolated tweaks.

1. Hub lacks a clear primary action. Open, Select editor, Settings, and
   Refresh have similar emphasis. Fix: opening the project dominates;
   changing its editor is secondary.
2. Project information is uneven. Hub emphasizes full paths and
   installation identity but omits last-opened time, even though it already
   receives that data (`opened_at` in `scripts/hub.py:301`). Editor shows
   recency but omits editor association and active status. Fix: show name,
   recency, editor binding, and active state in both.
3. Installation page wastes vertical space. Two blank labels create a gap
   before the list. Developer rows are inflated by vertically stacked
   actions. Fix: remove placeholder labels, compact developer rows.
4. Developer concerns dominate installation discovery. "Import release
   bundle" and "Add local repository" sit beside "Get a release" with equal
   weight. Fix: one obvious installation route for general users; advanced
   routes in a menu.
5. Recovery states are weak. A missing installation can still leave Open
   enabled. "Folder unavailable" hides the actual cause and offers no
   repair. Search with no matches gives an unexplained blank list. Fix:
   disable Open with a reason when the installation is missing; show cause
   plus Locate and Remove actions for unavailable folders; add an explicit
   empty-search message.
6. Progress depends too much on logs. "Working..." and raw build output do
   not tell ordinary users which stage runs. The Hub reports "Editor
   launched" after spawning the process, without confirming the project
   opened. Fix: staged progress (download, verify, extract, install;
   prepare, build, install), and confirm open success rather than spawn
   success.
7. Android module selection asks users to understand the toolchain. Java,
   SDK, NDK, and Rust targets should show versions and sizes, plus a
   convenient Android-build-tools selection. Development installs default to
   all optional tools, release installs default to none. Keep that default
   rule and make it visible.
8. Editor picker needs better keyboard support and safer actions. Cards have
   mouse activation but no explicit keyboard opening behavior. Small remove
   and permanent-delete icons sit together on every card. Fix: keyboard open
   behavior, separated and labeled destructive actions, project menu for
   delete, search for large libraries.

## 9. Decisions worth preserving

Keep these. They were called out as sensible:

- Side-by-side installations.
- Explicit project bindings.
- Backups enabled when changing an existing binding.
- Preservation of working development installations during failed builds.
- Restrained decoration, readable contrast, understandable grouping.
- Editor creation dialog as the model for Hub onboarding.

## Test plan

- Service tests: version routing refusal, idle versus busy tracking, stale
  PID handling, ownership refusal for delete and bind, failed-save blocking,
  missing-folder retention with recovery actions, no silent pruning on
  disconnected storage.
- UI smoke: empty state with New project, primary Open emphasis, missing
  installation disables Open with reason, unavailable folder shows cause plus
  Locate and Remove, empty search message, staged progress text, keyboard
  open in the editor picker.
- Keep the current baseline green: service plus Hub UI smoke scenarios.
- Manual: Windows and macOS interactive passes were out of scope for the
  review and remain open before release.
