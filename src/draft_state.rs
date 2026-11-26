//! Revision-aware local drafts. Unsaved edits and recovery never count as saved.
use std::time::{Duration, Instant};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Parked {
    pub id: u64,
    pub text: String,
}
#[derive(Default)]
pub struct State {
    pub recovery_min_chars: Option<usize>,
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
    pub recovery_left: Option<Duration>,
    pub recovery_started: Option<Instant>,
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
        if !after.trim().is_empty()
            && self.recovery.is_some()
            && self.recovery_left.is_none()
            && self.staged_recovery.is_none()
        {
            self.recovery_left = Some(Duration::from_secs(10));
            self.recovery_started = Some(Instant::now());
        }
        if after.trim().is_empty() {
            self.resumed = None;
        }
        if deleting {
            if self.deletion_start.is_none() {
                self.deletion_start = Some(before.into());
            }
            if after.is_empty() {
                let original = self.deletion_start.take().unwrap_or_else(|| before.into());
                if !original.trim().is_empty()
                    && (original.chars().count() >= self.recovery_min_chars.unwrap_or(30)
                        || original.contains('\n'))
                {
                    self.recovery = Some(original);
                    self.recovery_left = None;
                    self.recovery_started = None;
                }
            }
        } else {
            self.deletion_start = None;
        }
        self.text = after.into();
        self.touch();
    }
    pub fn pause_recovery(&mut self, now: Instant) {
        if let (Some(left), Some(start)) = (self.recovery_left, self.recovery_started.take()) {
            self.recovery_left = Some(left.saturating_sub(now.saturating_duration_since(start)));
        }
    }
    pub fn recovery_fraction(&mut self, now: Instant) -> Option<f32> {
        if self.recovery.is_none() {
            self.recovery_left = None;
            self.recovery_started = None;
            return None;
        }
        let left = self.recovery_left?;
        let start = *self.recovery_started.get_or_insert(now);
        let remaining = left.saturating_sub(now.saturating_duration_since(start));
        if remaining.is_zero() {
            self.recovery = None;
            self.recovery_left = None;
            self.recovery_started = None;
            None
        } else {
            Some(remaining.as_secs_f32() / 10.0)
        }
    }
    pub fn keep_text(&mut self, text: &str, clear_composer: bool) -> bool {
        if !self.loaded || self.staged_shelf.is_some() || text.trim().is_empty() {
            return false;
        }
        self.resumed = None;
        let mut shelf = self.shelf.clone();
        shelf.push(Parked {
            id: crate::util::now() as u64 * 1000 + self.revision,
            text: text.into(),
        });
        if clear_composer {
            self.text = text.into();
            self.clear_after_save = Some(text.into());
        } else {
            self.staged_recovery = Some(text.into());
            // An explicit save must not lose its retry copy to the expiry timer.
            self.recovery_left = None;
            self.recovery_started = None;
        }
        self.staged_shelf = Some(shelf);
        self.touch();
        true
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
    #[test]
    fn explicit_recovery_save_keeps_failure_retry_without_a_new_expiry() {
        let mut state = State {
            loaded: true,
            text: "new composer".into(),
            recovery: Some("deleted original".into()),
            recovery_left: Some(Duration::from_secs(4)),
            recovery_started: Some(Instant::now()),
            ..Default::default()
        };
        assert!(state.keep_text("deleted original", false));
        state.pending = Some(state.revision);
        state.ack(state.revision, false);
        state.edit("new composer", "new composer continues", false);
        assert!(state.recovery_left.is_none());
        assert!(
            state
                .recovery_fraction(Instant::now() + Duration::from_secs(30))
                .is_none()
        );
        assert_eq!(state.recovery.as_deref(), Some("deleted original"));
        assert_eq!(state.staged_recovery.as_deref(), Some("deleted original"));
        assert_eq!(state.staged_shelf.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn new_text_starts_one_countdown_and_navigation_pauses_it() {
        let mut s = State {
            loaded: true,
            recovery_min_chars: Some(1),
            ..Default::default()
        };
        s.edit("previous text", "", true);
        let now = Instant::now();
        assert!(
            s.recovery_fraction(now).is_none(),
            "deletion alone has no timer"
        );
        s.edit("", "new", false);
        let start = s.recovery_started.unwrap();
        s.edit("new", "new text", false);
        assert_eq!(
            s.recovery_started,
            Some(start),
            "do not reset on more typing"
        );
        s.pause_recovery(start + Duration::from_secs(4));
        let resumed = start + Duration::from_secs(100);
        assert!((s.recovery_fraction(resumed).unwrap() - 0.6).abs() < 0.01);
        assert!(
            s.recovery_fraction(resumed + Duration::from_secs(6))
                .is_none()
        );
        assert!(s.recovery.is_none());
        assert!(s.staged_shelf.is_none(), "expiry must not auto-save");
    }
    #[test]
    fn keep_text_stages_without_claiming_saved_before_ack() {
        let mut s = State {
            loaded: true,
            ..Default::default()
        };
        assert!(s.keep_text("draft", true));
        assert_eq!(s.clear_after_save.as_deref(), Some("draft"));
        assert_eq!(s.text, "draft", "WriteDraftState reads the current text");
        assert_ne!(s.status(), "Saved locally");
        assert!(s.shelf.is_empty());
        assert!(!s.keep_text("duplicate", true));
    }
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
    fn recovery_uses_thirty_unicode_characters_by_default_and_keeps_newline_rule() {
        for (text, recover) in [
            ("x".repeat(29), false),
            ("ä".repeat(30), true),
            ("x".repeat(31), true),
            ("a\nb".into(), true),
            (" ".repeat(40), false),
        ] {
            let mut s = State::default();
            s.edit(&text, "", true);
            assert_eq!(s.recovery.is_some(), recover, "{text:?}");
        }
        let mut s = State {
            recovery_min_chars: Some(50),
            ..Default::default()
        };
        s.edit(&"x".repeat(30), "", true);
        assert!(s.recovery.is_none());
        s.edit("two\nlines", "", true);
        assert!(
            s.recovery.is_some(),
            "newline rule is independent of preference"
        );
    }
    #[test]
    fn incremental_delete_uses_original_length_and_programmatic_clear_does_not_offer() {
        let original = "x".repeat(30);
        let mut s = State::default();
        s.edit(&original, "x", true);
        s.edit("x", "", true);
        assert_eq!(s.recovery.as_deref(), Some(original.as_str()));
        let mut s = State::default();
        s.edit(&original, "", false);
        assert!(s.recovery.is_none());
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
