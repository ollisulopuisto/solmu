//! Runtime-only separate sends. Never persisted or automatically replayed.
use crate::{
    merged::Member,
    pending_media::{Job, Payload, State},
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug)]
pub struct Row {
    pub member: Member,
    pub states: Vec<Option<State>>,
    pub started: bool,
    tokens: Vec<Option<String>>,
}
pub struct Queue {
    pub id: u64,
    pub name: String,
    pub rows: Vec<Row>,
    pub paused: bool,
    pub visible: bool,
    payloads: Vec<Payload>,
    text: String,
    next_at: Instant,
}
impl Queue {
    pub fn new(
        name: String,
        members: Vec<Member>,
        text: String,
        files: Vec<Payload>,
        now: Instant,
    ) -> Result<Self, &'static str> {
        let mut unique = Vec::new();
        for member in members {
            if !unique.contains(&member) {
                unique.push(member);
            }
        }
        if unique.is_empty() || unique.len() > 50 {
            return Err("Choose between 1 and 50 available chats");
        }
        if text.trim().is_empty() && files.is_empty() {
            return Err("Write a message or choose an attachment");
        }
        let payloads = if files.is_empty() {
            vec![Payload::Text(text.clone())]
        } else {
            files
        };
        Ok(Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            name,
            rows: unique
                .into_iter()
                .map(|member| Row {
                    member,
                    states: vec![None; payloads.len()],
                    started: false,
                    tokens: vec![None; payloads.len()],
                })
                .collect(),
            paused: false,
            visible: true,
            payloads,
            text,
            next_at: now,
        })
    }
    fn token(id: u64, row: usize, part: usize) -> String {
        format!(
            "audience-{id}-{row}-{part}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
    pub fn pending(&self) -> bool {
        self.rows.iter().any(|row| {
            row.states
                .iter()
                .any(|state| state.is_none() || matches!(state, Some(State::Sending)))
        })
    }
    pub fn awaiting_confirmation(&self) -> bool {
        self.rows.iter().any(|row| {
            row.states
                .iter()
                .any(|s| matches!(s, Some(State::Accepted { .. })))
        })
    }
    /// Exactly one submission may be in flight. Pause blocks only new rows.
    pub fn next(
        &mut self,
        now: Instant,
        mut available: impl FnMut(&Member) -> bool,
    ) -> Option<Job> {
        if now < self.next_at
            || self
                .rows
                .iter()
                .any(|row| row.states.iter().any(|s| matches!(s, Some(State::Sending))))
        {
            return None;
        }
        for (r, row) in self.rows.iter_mut().enumerate() {
            let Some(part) = row.states.iter().position(Option::is_none) else {
                continue;
            };
            if self.paused && !row.started {
                return None;
            }
            row.started = true;
            if !available(&row.member) {
                for state in &mut row.states {
                    if state.is_none() {
                        *state = Some(State::NotSubmitted(
                            "Destination unavailable or identity changed; nothing submitted".into(),
                        ));
                    }
                }
                continue;
            }
            row.states[part] = Some(State::Sending);
            let token = Self::token(self.id, r, part);
            row.tokens[part] = Some(token.clone());
            return Some(Job {
                temporary: false,
                local: token,
                member: row.member.clone(),
                payload: self.payloads[part].clone(),
                caption: (part == 0 && !self.text.is_empty()).then(|| self.text.clone()),
                quoting: None,
            });
        }
        None
    }
    /// Exact identity and job token, never chat ID alone. Terminal feedback
    /// cannot regress to a late acceptance or be made retryable by a timeout.
    pub fn feedback(&mut self, local: &str, member: &Member, state: State, now: Instant) -> bool {
        for row in &mut self.rows {
            if &row.member != member {
                continue;
            }
            for (p, current) in row.states.iter_mut().enumerate() {
                if row.tokens[p].as_deref() != Some(local) {
                    continue;
                }
                let allowed = matches!(current, Some(State::Sending))
                    || (matches!(current, Some(State::Accepted { .. }))
                        && matches!(state, State::Confirmed | State::Unknown(_)));
                if allowed {
                    *current = Some(state);
                    self.next_at = now + Duration::from_millis(500);
                }
                return true;
            }
        }
        false
    }
    /// Explicit retry means only proven pre-submit failures. Success, pending
    /// acceptance and unknown outcomes are never sent again.
    pub fn retry_unsent(&mut self, row: usize) -> bool {
        if self.pending() {
            return false;
        }
        let Some(row) = self.rows.get_mut(row) else {
            return false;
        };
        let mut changed = false;
        for state in &mut row.states {
            if matches!(state, Some(State::NotSubmitted(_))) {
                *state = None;
                changed = true;
            }
        }
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn member(account: &str) -> Member {
        Member {
            origin: "server".into(),
            account: account.into(),
            chat: "same".into(),
        }
    }
    #[test]
    fn retry_ignores_feedback_from_an_older_attempt() {
        let now = Instant::now();
        let mut q = Queue::new(
            "test".into(),
            vec![member("a")],
            "hello".into(),
            vec![],
            now,
        )
        .unwrap();
        let first = q.next(now, |_| true).unwrap();
        q.feedback(
            &first.local,
            &first.member,
            State::NotSubmitted("offline".into()),
            now,
        );
        assert!(q.retry_unsent(0));
        let retry = q.next(now + Duration::from_secs(1), |_| true).unwrap();
        assert_ne!(first.local, retry.local);
        assert!(!q.feedback(
            &first.local,
            &first.member,
            State::Unknown("late".into()),
            now
        ));
        assert!(matches!(q.rows[0].states[0], Some(State::Sending)));
        assert!(q.feedback(&retry.local, &retry.member, State::Confirmed, now));
    }
    #[test]
    fn paced_queue_pause_and_exact_feedback_never_duplicate_unknown() {
        let now = Instant::now();
        let mut q = Queue::new(
            "test".into(),
            vec![member("a"), member("b")],
            "hello".into(),
            vec![],
            now,
        )
        .unwrap();
        let a = q.next(now, |_| true).unwrap();
        assert!(q.next(now, |_| true).is_none());
        q.paused = true;
        assert!(!q.feedback(
            &a.local,
            &member("b"),
            State::Unknown("timeout".into()),
            now
        ));
        assert!(q.feedback(&a.local, &a.member, State::Unknown("timeout".into()), now));
        assert!(q.next(now + Duration::from_secs(1), |_| true).is_none());
        q.paused = false;
        let b = q.next(now + Duration::from_secs(1), |_| true).unwrap();
        assert_eq!(b.member, member("b"));
        q.feedback(
            &b.local,
            &b.member,
            State::NotSubmitted("offline".into()),
            now,
        );
        assert!(!q.retry_unsent(0));
        assert!(q.retry_unsent(1));
        assert!(matches!(q.rows[0].states[0], Some(State::Unknown(_))));
    }
    #[test]
    fn pause_finishes_started_recipient_parts_but_not_next_recipient() {
        let now = Instant::now();
        let files = vec![
            Payload::File {
                path: "one".into(),
                document: false,
            },
            Payload::File {
                path: "two".into(),
                document: true,
            },
        ];
        let mut q = Queue::new(
            "test".into(),
            vec![member("a"), member("b")],
            "caption".into(),
            files,
            now,
        )
        .unwrap();
        let first = q.next(now, |_| true).unwrap();
        q.paused = true;
        q.feedback(
            &first.local,
            &first.member,
            State::Accepted {
                pending: "p".into(),
            },
            now,
        );
        let next = now + Duration::from_secs(1);
        let second = q.next(next, |_| true).unwrap();
        assert_eq!(second.member, member("a"));
        assert!(second.caption.is_none());
        q.feedback(&second.local, &second.member, State::Confirmed, next);
        assert!(q.next(next + Duration::from_secs(1), |_| true).is_none());
    }
    #[test]
    fn stale_queue_token_and_foreign_origin_cannot_update_results() {
        let now = Instant::now();
        let mut q = Queue::new(
            "test".into(),
            vec![member("a")],
            "hello".into(),
            vec![],
            now,
        )
        .unwrap();
        let job = q.next(now, |_| true).unwrap();
        let mut wrong = job.member.clone();
        wrong.origin = "other".into();
        assert!(!q.feedback(&job.local, &wrong, State::Confirmed, now));
        assert!(!q.feedback("audience-old-0-0", &job.member, State::Confirmed, now));
        q.feedback(&job.local, &job.member, State::Confirmed, now);
        q.feedback(
            &job.local,
            &job.member,
            State::Accepted {
                pending: "late".into(),
            },
            now,
        );
        assert_eq!(q.rows[0].states[0], Some(State::Confirmed));
    }
    #[test]
    fn identity_change_prevents_dispatch() {
        let now = Instant::now();
        let mut q = Queue::new(
            "test".into(),
            vec![member("a")],
            "hello".into(),
            vec![],
            now,
        )
        .unwrap();
        assert!(q.next(now, |_| false).is_none());
        assert!(matches!(q.rows[0].states[0], Some(State::NotSubmitted(_))));
    }
}

#[derive(Default)]
pub struct Draft {
    pub members: Vec<crate::flat_groups::Member>,
    pub text: String,
    pub files: Vec<std::path::PathBuf>,
    pub name: String,
    pub keep: bool,
    pub picker: Option<std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>>,
    pub notice: Option<String>,
}
