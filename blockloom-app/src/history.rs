//! The editor's document tracking: undo over [`Change`]s rather than
//! whole-project copies, and the record of what changed since the last
//! save and the last snapshot the frontend got.
//!
//! Commands keep editing `open.project` directly. [`Tracker::commit`] diffs
//! it against the last committed copy, so every edit becomes one change
//! however it was made.

use blockloom_core::change::{self, Change};
use blockloom_core::project::Project;
use blockstitch_core::editor::EditSession;
use std::collections::BTreeSet;

/// A bounded undo/redo stack of changes, coalescing a run of keystrokes in
/// one session into one step.
#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
    session: Option<EditSession>,
}

impl History {
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn end_session(&mut self) {
        self.session = None;
    }

    /// Records an undoable change; one in the session already in progress
    /// joins the step it started.
    fn record(&mut self, change: Change, session: Option<EditSession>) {
        let joins = session.is_some() && self.session == session;
        self.redo.clear();
        match self.undo.pop() {
            Some(top) if joins => self.undo.push(top.then(change)),
            top => {
                self.undo.extend(top);
                if self.undo.len() >= crate::state::UNDO_STACK_LIMIT {
                    self.undo.remove(0);
                }
                self.undo.push(change);
            }
        }
        self.session = session;
    }
}

/// Which files a save still owes the disk.
#[derive(Default, Debug)]
pub(crate) struct Unsaved {
    pub(crate) everything: bool,
    pub(crate) index: bool,
    pub(crate) scenes: BTreeSet<String>,
}

impl Unsaved {
    pub(crate) fn is_empty(&self) -> bool {
        !self.everything && !self.index && self.scenes.is_empty()
    }
    fn add(&mut self, change: &Change) {
        self.index |= change.touches_index();
        match change.touched_scenes() {
            Some(scenes) => self.scenes.extend(scenes),
            None => self.everything = true,
        }
    }
}

/// What the frontend hasn't been sent yet, per scene: actor ids, or all.
#[derive(Default, Debug)]
pub(crate) struct Unpublished {
    pub(crate) everything: bool,
    pub(crate) project: bool,
    pub(crate) actors: std::collections::BTreeMap<String, Option<BTreeSet<String>>>,
}

impl Unpublished {
    fn add(&mut self, change: &Change) {
        if change.whole.is_some() {
            self.everything = true;
            return;
        }
        self.project |= change.header.is_some()
            || change.scene_order.is_some()
            || change
                .scenes
                .iter()
                .any(|s| s.settings.is_some() || s.order.is_some());
        for scene in &change.scenes {
            let entry = self
                .actors
                .entry(scene.id.clone())
                .or_insert_with(|| Some(BTreeSet::new()));
            match (change.touched_actors(&scene.id), entry.as_mut()) {
                (Some(ids), Some(set)) => set.extend(ids),
                _ => *entry = None,
            }
        }
    }
}

/// The committed copy and everything that keys off it.
#[derive(Default)]
pub(crate) struct Tracker {
    /// The document as of the last commit. `None` until the first one,
    /// which then counts as changing everything.
    shadow: Option<Project>,
    /// The next commit's change is undoable, in this session.
    mark: Option<Option<EditSession>>,
    pub(crate) history: History,
    pub(crate) unsaved: Unsaved,
    pub(crate) unpublished: Unpublished,
}

impl Tracker {
    /// Takes `project` as committed without recording anything: what a
    /// fresh open or a load from disk does.
    pub(crate) fn reset(&mut self, project: &Project) {
        self.shadow = Some(project.clone());
        self.mark = None;
        self.history.clear();
        self.unsaved = Unsaved::default();
        self.unpublished = Unpublished {
            everything: true,
            ..Unpublished::default()
        };
    }

    /// Marks the next commit undoable. Whatever changed before this is
    /// committed first, so it isn't folded into the step.
    pub(crate) fn checkpoint(&mut self, project: &Project, session: Option<EditSession>) {
        self.commit(project);
        self.mark = Some(session);
    }

    /// Diffs `project` against the committed copy and files the result.
    /// The last committed copy of the project.
    pub(crate) fn shadow(&self) -> Option<&Project> {
        self.shadow.as_ref()
    }

    pub(crate) fn commit(&mut self, project: &Project) -> Option<Change> {
        let mark = self.mark.take();
        let Some(shadow) = &mut self.shadow else {
            self.shadow = Some(project.clone());
            self.unsaved.everything = true;
            self.unpublished.everything = true;
            return None;
        };
        let change = change::diff(shadow, project);
        if change.is_empty() {
            return None;
        }
        change.apply(shadow, true);
        self.unsaved.add(&change);
        self.unpublished.add(&change);
        if let Some(session) = mark {
            self.history.record(change.clone(), session);
        } else if self.history.session.is_some() {
            // An unrecorded change ends any run of keystrokes.
            self.history.session = None;
        }
        Some(change)
    }

    /// Undoes (`back`) or redoes one step on `project`. Answers whether
    /// there was one.
    pub(crate) fn step(&mut self, project: &mut Project, back: bool) -> bool {
        self.commit(project);
        let (from, to) = if back {
            (&mut self.history.undo, &mut self.history.redo)
        } else {
            (&mut self.history.redo, &mut self.history.undo)
        };
        let Some(change) = from.pop() else {
            return false;
        };
        change.apply(project, !back);
        to.push(change);
        self.history.session = None;
        self.mark = None;
        // Files and the frontend follow; history already moved.
        let shadow_change = self
            .shadow
            .as_ref()
            .map(|shadow| change::diff(shadow, project));
        if let (Some(shadow), Some(applied)) = (&mut self.shadow, shadow_change) {
            applied.apply(shadow, true);
            self.unsaved.add(&applied);
            self.unpublished.add(&applied);
        }
        true
    }

    /// Takes what the frontend is owed, leaving nothing owed.
    pub(crate) fn take_unpublished(&mut self) -> Unpublished {
        std::mem::take(&mut self.unpublished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};

    fn project() -> Project {
        let mut p = Project::starter("History", Mode::TwoD);
        p.add_actor(Actor::new(
            "A",
            Visual::Circle {
                color: "#fff".into(),
                radius: 4.0,
            },
        ));
        p
    }

    #[test]
    fn undo_and_redo_walk_the_changes() {
        let mut p = project();
        let mut t = Tracker::default();
        t.reset(&p);
        let start = p.clone();
        t.checkpoint(&p, None);
        p.actors[0].name = "B".into();
        t.commit(&p);
        let renamed = p.clone();
        assert!(t.history.can_undo());
        assert!(t.step(&mut p, true));
        assert_eq!(p, start);
        assert!(t.step(&mut p, false));
        assert_eq!(p, renamed);
        assert!(!t.step(&mut p, false));
    }

    #[test]
    fn a_session_coalesces_and_unmarked_changes_stay_out_of_history() {
        let mut p = project();
        let mut t = Tracker::default();
        t.reset(&p);
        let start = p.clone();
        let session = Some(EditSession::Comment {
            comment_id: "c".into(),
        });
        for name in ["x", "xy", "xyz"] {
            t.checkpoint(&p, session.clone());
            p.actors[0].name = name.into();
            t.commit(&p);
        }
        assert_eq!(t.history.undo.len(), 1);
        t.step(&mut p, true);
        assert_eq!(p, start);
        // Not marked: committed, saved and published, but not undoable.
        p.name = "Elsewhere".into();
        t.commit(&p);
        assert!(t.unsaved.index);
        assert_eq!(t.history.undo.len(), 0);
    }
}
