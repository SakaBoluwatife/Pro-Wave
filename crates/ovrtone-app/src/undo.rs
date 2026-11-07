//! Edit > Undo / Redo. After each finished gesture (a click, a drag let go, a key, a menu
//! entry) the app calls [`History::record`]; when the project differs from the last recorded
//! state (by the same comparison that tells whether there are unsaved changes), that state is
//! kept for Undo. Everything that is saved can be undone (tracks, regions, notes, mixer, tempo,
//! signature, locators); the playhead and the selection are not, as in Logic.
use crate::file_commands::content;
use crate::model::Project;
use ovrtone_project::ProjectDoc;

/// Undo steps kept; the oldest go first.
const LIMIT: usize = 100;

pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// What each step was, in words.
    undo_labels: Vec<String>,
    redo_labels: Vec<String>,
    /// The project as last recorded, and what it saves as.
    base: Project,
    base_doc: ProjectDoc,
}

/// What an edit did, in words, from how the project differs.
fn describe(before: &ProjectDoc, after: &ProjectDoc) -> String {
    let (a, b) = (&before.tracks, &after.tracks);
    if b.len() > a.len() {
        return "Add Track".to_string();
    }
    if b.len() < a.len() {
        return "Delete Track".to_string();
    }
    if before.tempo != after.tempo || before.tempo_changes != after.tempo_changes {
        return "Change Tempo".to_string();
    }
    if before.song_patterns != after.song_patterns {
        return "Edit Pattern".to_string();
    }
    if before.master_plugins != after.master_plugins {
        return "Change Output Plugins".to_string();
    }
    for (x, y) in a.iter().zip(b) {
        if x.regions.len() != y.regions.len() {
            let what = if y.regions.len() > x.regions.len() {
                "Add"
            } else {
                "Delete"
            };
            return format!("{what} Region on {}", y.name);
        }
        if x.name != y.name {
            return format!("Rename Track to {}", y.name);
        }
        if x.regions != y.regions {
            let notes = x
                .regions
                .iter()
                .zip(&y.regions)
                .any(|(r, s)| r.notes != s.notes);
            let what = if notes { "Edit Notes" } else { "Edit Region" };
            return format!("{what} on {}", y.name);
        }
        if x.plugins != y.plugins || x.instrument_plugin != y.instrument_plugin {
            return format!("Change Plugins on {}", y.name);
        }
        if x != y {
            return format!("Change {}", y.name);
        }
    }
    "Edit".to_string()
}

/// An Undo command picked in a menu or by key; run by `App` after the panels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UndoCmd {
    Undo,
    Redo,
    /// Edit > Delete Undo History.
    Clear,
}

impl History {
    pub fn new(p: &Project) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            undo_labels: Vec::new(),
            redo_labels: Vec::new(),
            base: p.clone(),
            base_doc: content(p),
        }
    }

    /// Forgets every step (a project was opened or created).
    pub fn reset(&mut self, p: &Project) {
        *self = Self::new(p);
    }

    /// What each step Undo can go back through was, oldest first.
    pub fn undo_labels(&self) -> &[String] {
        &self.undo_labels
    }

    /// What each undone step was, the next to redo last.
    pub fn redo_labels(&self) -> &[String] {
        &self.redo_labels
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Keeps the last recorded state for Undo if `p` has changed since. Returns whether it had.
    pub fn record(&mut self, p: &Project) -> bool {
        let doc = content(p);
        if doc == self.base_doc {
            return false;
        }
        let label = describe(&self.base_doc, &doc);
        let before = std::mem::replace(&mut self.base, p.clone());
        self.undo.push(before);
        self.undo_labels.push(label);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
            self.undo_labels.remove(0);
        }
        self.redo.clear();
        self.redo_labels.clear();
        self.base_doc = doc;
        true
    }

    /// Goes back one step. Returns false when there is nothing to undo.
    pub fn undo(&mut self, p: &mut Project) -> bool {
        // An edit not recorded yet is the step to undo first.
        self.record(p);
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(p.clone());
        if let Some(label) = self.undo_labels.pop() {
            self.redo_labels.push(label);
        }
        self.restore(p, prev);
        true
    }

    /// Goes forward one undone step. Returns false when there is nothing to redo.
    pub fn redo(&mut self, p: &mut Project) -> bool {
        if self.record(p) {
            // A new edit since the undo: the undone steps are gone, as in every editor.
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(p.clone());
        if let Some(label) = self.redo_labels.pop() {
            self.undo_labels.push(label);
        }
        self.restore(p, next);
        true
    }

    /// Gives up every step.
    pub fn clear(&mut self, p: &Project) {
        self.reset(p);
    }

    /// Puts `state` in place of `p`, keeping what is not part of an edit (playhead, transport)
    /// and making the engine rebuild its song.
    fn restore(&mut self, p: &mut Project, mut state: Project) {
        state.position_beats = p.position_beats;
        state.playing = p.playing;
        state.recording = p.recording;
        state.song_rev = p.song_rev.wrapping_add(1);
        state.selected_track = state
            .selected_track
            .min(state.tracks.len().saturating_sub(1));
        *p = state;
        self.base = p.clone();
        self.base_doc = content(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    #[test]
    fn an_added_track_is_undone_and_redone() {
        let mut p = Project::demo();
        let mut h = History::new(&p);
        let n = p.tracks.len();
        assert!(!h.can_undo());

        p.add_track(TrackKind::Instrument);
        assert!(h.record(&p));
        assert!(!h.record(&p), "nothing new to record");
        assert!(h.can_undo() && !h.can_redo());

        let rev = p.song_rev;
        assert!(h.undo(&mut p));
        assert_eq!(p.tracks.len(), n);
        assert_ne!(p.song_rev, rev, "the engine rebuilds the song");
        assert!(h.can_redo());

        assert!(h.redo(&mut p));
        assert_eq!(p.tracks.len(), n + 1);
        assert!(!h.redo(&mut p), "nothing left to redo");
    }

    #[test]
    fn an_unrecorded_edit_is_undone_first_and_the_playhead_stays() {
        let mut p = Project::demo();
        let mut h = History::new(&p);
        p.tempo = 90.0;
        p.position_beats = 33.0;
        assert!(h.undo(&mut p), "the tempo change was recorded, then undone");
        assert_eq!(p.tempo, 120.0);
        assert_eq!(p.position_beats, 33.0, "undo does not move the playhead");
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo_steps() {
        let mut p = Project::demo();
        let mut h = History::new(&p);
        p.tempo = 90.0;
        h.record(&p);
        h.undo(&mut p);
        p.tempo = 100.0;
        assert!(!h.redo(&mut p));
        assert_eq!(p.tempo, 100.0);
        assert!(!h.can_redo());
        h.clear(&p);
        assert!(!h.can_undo());
    }

    #[test]
    fn only_the_last_hundred_steps_are_kept() {
        let mut p = Project::demo();
        let mut h = History::new(&p);
        for i in 0..(LIMIT + 20) {
            p.tempo = 60.0 + i as f32;
            h.record(&p);
        }
        let mut steps = 0;
        while h.undo(&mut p) {
            steps += 1;
        }
        assert_eq!(steps, LIMIT);
    }
}
