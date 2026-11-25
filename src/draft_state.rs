//! Revision-aware local drafts. Unsaved edits and recovery never count as saved.
use std::time::{Duration, Instant};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Parked {
    pub id: u64,
    pub text: String,
}
#[derive(Default)]
pub struct State {
    pub resumed: Option<u64>,
    pub sent_removals: Vec<u64>,
    pub revision: u64,
    pub confirmed: Option<u64>,
    pub pending: Option<u64>,
    pub pending_shelf: Option<Vec<Parked>>,
    pub pending_recovery: Option<String>,
    pub staged_recovery: Option<String>,
    pub failed: bool,
    pub changed: Option<Instant>,
    pub text: String,
    pub shelf: Vec<Parked>,
    pub staged_shelf: Option<Vec<Parked>>,
    pub recovery: Option<String>,
    pub deletion_start: Option<String>,
    pub shelf_open: bool,
    pub loaded: bool,
    pub clear_after_save: Option<String>,
}
/// Unlocking later cannot turn a failed initial load into permission to replace
/// a shelf we have never read. Restart/reload is required first.
pub fn can_write_shelf(loaded: bool, store_available: bool) -> bool {
    loaded && store_available
}

impl State {
    pub fn resume(&mut self, id: u64) -> Option<String> {
        let text = self.shelf.iter().find(|d| d.id == id)?.text.clone();
        self.resumed = Some(id);
        self.text = text.clone();
        self.touch();
        Some(text)
    }
    pub fn sent(&mut self, id: u64) {
        if self.resumed == Some(id) {
            self.resumed = None;
        }
        if !self.sent_removals.contains(&id) {
            self.sent_removals.push(id);
        }
        self.stage_sent_removals();
    }
    fn stage_sent_removals(&mut self) {
        if self.pending.is_some() || self.sent_removals.is_empty() {
            return;
        }
        let mut shelf = self
            .staged_shelf
            .take()
            .unwrap_or_else(|| self.shelf.clone());
        shelf.retain(|d| !self.sent_removals.contains(&d.id));
        self.staged_shelf = Some(shelf);
        self.touch();
    }
    pub fn edit(&mut self, before: &str, after: &str, deleting: bool) {
        if before == after {
            return;
        }
        self.clear_after_save = None;
        if after.trim().is_empty() {
            self.resumed = None;
        }
        if deleting {
            if self.deletion_start.is_none() {
                self.deletion_start = Some(before.into());
            }
            if after.is_empty() {
                let original = self.deletion_start.take().unwrap_or_else(|| before.into());
                if original.chars().count() >= 80 || original.contains('\n') {
                    self.recovery = Some(original);
                }
            }
        } else {
            self.deletion_start = None;
        }
        self.text = after.into();
        self.touch();
    }
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.failed = false;
        self.changed = Some(Instant::now());
    }
    pub fn ready(&self) -> bool {
        self.loaded
            && self
                .changed
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(650))
            && self.pending.is_none()
            && !self.failed
    }
    pub fn ack(&mut self, revision: u64, ok: bool) {
        if self.pending != Some(revision) {
            return;
        }
        self.pending = None;
        if ok {
            if let Some(shelf) = self.pending_shelf.take() {
                self.shelf = shelf;
                self.staged_shelf = None;
                if self.pending_recovery.is_some() && self.recovery == self.pending_recovery {
                    self.recovery = None;
                }
                self.pending_recovery = None;
                self.staged_recovery = None;
            }
        } else {
            self.pending_shelf = None;
        }
        if revision != self.revision {
            self.stage_sent_removals();
            return;
        }
        self.failed = !ok;
        if ok {
            self.confirmed = Some(revision);
            self.changed = None;
            if let Some(shelf) = self.staged_shelf.take() {
                self.shelf = shelf;
                if self.pending_recovery.is_some() && self.recovery == self.pending_recovery {
                    self.recovery = None;
                }
                self.pending_recovery = None;
                self.staged_recovery = None;
            }
        }
        if ok
            && self
                .shelf
                .iter()
                .any(|d| self.sent_removals.contains(&d.id))
        {
            self.stage_sent_removals();
        } else if ok {
            self.sent_removals.clear();
        }
    }
    pub fn status(&self) -> &'static str {
        if !self.loaded {
            "Not saved here until restart"
        } else if self.failed {
            "Not saved"
        } else if self.confirmed == Some(self.revision) && self.changed.is_none() {
            "Saved locally"
        } else {
            "Saving"
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn resumed_pair() -> State {
        State {
            loaded: true,
            shelf: vec![
                Parked {
                    id: 1,
                    text: "same".into(),
                },
                Parked {
                    id: 2,
                    text: "same".into(),
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn successful_send_removes_only_captured_identity_after_a_new_resume() {
        let mut s = resumed_pair();
        assert_eq!(s.resume(1).as_deref(), Some("same"));
        let sent = s.resumed.unwrap();
        s.resume(2);
        s.sent(sent);
        assert_eq!(s.resumed, Some(2));
        assert_eq!(
            s.staged_shelf
                .as_ref()
                .unwrap()
                .iter()
                .map(|d| d.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(s.shelf.len(), 2, "not persisted until write ack");
        s.pending = Some(s.revision);
        s.pending_shelf = s.staged_shelf.clone();
        s.ack(s.revision, true);
        assert_eq!(s.shelf.iter().map(|d| d.id).collect::<Vec<_>>(), vec![2]);
    }
    #[test]
    fn no_confirmation_retains_resumed_draft_and_failed_write_retains_retry() {
        let mut s = resumed_pair();
        s.resume(1);
        assert_eq!(s.shelf.len(), 2);
        assert!(s.staged_shelf.is_none());
        s.sent(1);
        s.pending = Some(s.revision);
        s.pending_shelf = s.staged_shelf.clone();
        s.ack(s.revision, false);
        assert!(s.failed);
        assert_eq!(s.shelf.len(), 2);
        assert_eq!(s.staged_shelf.as_ref().unwrap().len(), 1);
    }
    #[test]
    fn earlier_write_ack_cannot_resurrect_sent_item() {
        let mut s = resumed_pair();
        s.pending = Some(0);
        s.pending_shelf = Some(s.shelf.clone());
        s.sent(1);
        s.ack(0, true);
        assert_eq!(
            s.staged_shelf
                .as_ref()
                .unwrap()
                .iter()
                .map(|d| d.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }
    #[test]
    fn clearing_resumed_text_detaches_identity() {
        let mut s = resumed_pair();
        s.resume(1);
        s.edit("same", "", true);
        s.edit("", "new message", false);
        assert_eq!(s.resumed, None);
        assert_eq!(s.shelf.len(), 2);
    }
    #[test]
    fn shelf_not_loaded_blocks_writes_and_success_commits_count() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        s.loaded = false;
        s.changed = Some(Instant::now() - Duration::from_secs(1));
        assert!(!s.ready());
        s.loaded = true;
        assert!(s.ready());
        s.staged_shelf = Some(vec![Parked {
            id: 1,
            text: "kept".into(),
        }]);
        s.pending = Some(s.revision);
        s.ack(s.revision, true);
        assert_eq!(s.shelf.len(), 1);
        assert_eq!(s.status(), "Saved locally");
    }
    #[test]
    fn old_keep_ack_does_not_clear_new_recovery() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        s.pending = Some(1);
        s.revision = 2;
        s.recovery = Some("newer text".into());
        s.pending_recovery = Some("old text".into());
        s.pending_shelf = Some(vec![Parked {
            id: 1,
            text: "old text".into(),
        }]);
        s.ack(1, true);
        assert_eq!(s.recovery.as_deref(), Some("newer text"));
        assert_eq!(s.shelf.len(), 1);
    }
    #[test]
    fn top_keep_does_not_consume_recovery_and_unavailable_is_honest() {
        let mut s = State::default();
        assert_eq!(s.status(), "Not saved here until restart");
        s.edit("old", "new", false);
        assert_eq!(s.status(), "Not saved here until restart");
        s.loaded = true;
        s.recovery = Some("unrelated".into());
        s.pending = Some(s.revision);
        s.pending_shelf = Some(vec![Parked {
            id: 2,
            text: "composer text".into(),
        }]);
        s.ack(s.revision, true);
        assert_eq!(s.recovery.as_deref(), Some("unrelated"));
    }
    #[test]
    fn unavailable_then_unlocked_still_refuses_destructive_write() {
        assert!(!can_write_shelf(false, false));
        assert!(!can_write_shelf(false, true));
        assert!(can_write_shelf(true, true));
    }
    #[test]
    fn stale_ack_never_claims_new_text_saved() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        s.edit("", "first", false);
        s.pending = Some(s.revision);
        let old = s.revision;
        s.edit("first", "second", false);
        s.ack(old, true);
        assert_eq!(s.status(), "Saving");
        assert_ne!(s.confirmed, Some(s.revision));
    }
    #[test]
    fn failed_keep_preserves_recovery_and_count() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        s.recovery = Some("text".into());
        s.staged_shelf = Some(vec![Parked {
            id: 1,
            text: "text".into(),
        }]);
        s.touch();
        s.pending = Some(s.revision);
        s.ack(s.revision, false);
        assert!(s.shelf.is_empty());
        assert!(s.recovery.is_some());
        assert_eq!(s.status(), "Not saved");
    }
    #[test]
    fn gradual_delete_keeps_original_not_last_character() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        let original = "x".repeat(85);
        s.edit(&original, "x", true);
        s.edit("x", "", true);
        assert_eq!(s.recovery, Some(original));
        s.edit("a", "", true);
        assert!(s.deletion_start.is_none());
    }
}
