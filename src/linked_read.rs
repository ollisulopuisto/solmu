//! Transient read authorization for one restored linked source. Never persisted.
use crate::merged::Member;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Restoration {
    pub member: Member,
    pub token: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum State {
    #[default]
    Absent,
    Pending(Restoration),
    Requested(Restoration),
    Loaded(Restoration),
    ReadSent(Restoration),
    Failed(Restoration),
}
impl State {
    pub fn restore(&mut self, member: Member) {
        *self = Self::Pending(Restoration {
            member,
            token: NEXT.fetch_add(1, Ordering::Relaxed),
        });
    }
    pub fn member(&self) -> Option<&Member> {
        match self {
            Self::Absent => None,
            Self::Pending(r)
            | Self::Requested(r)
            | Self::Loaded(r)
            | Self::ReadSent(r)
            | Self::Failed(r) => Some(&r.member),
        }
    }
    pub fn pending(&self) -> bool {
        matches!(
            self,
            Self::Pending(_) | Self::Requested(_) | Self::Loaded(_) | Self::Failed(_)
        )
    }
    pub fn request(&mut self) -> Option<u64> {
        let Self::Pending(r) = self else {
            return None;
        };
        let r = r.clone();
        let token = r.token;
        *self = Self::Requested(r);
        Some(token)
    }
    pub fn complete(&mut self, token: u64, success: bool) {
        if let Self::Requested(r) = self
            && r.token == token
        {
            *self = if success {
                Self::Loaded(r.clone())
            } else {
                Self::Failed(r.clone())
            };
        }
    }
    pub fn retry(&mut self) {
        if self.pending() {
            let member = self
                .member()
                .expect("pending restoration has an identity")
                .clone();
            self.restore(member);
        }
    }
    pub fn ready(&self, member: &Member) -> bool {
        matches!(self, Self::Loaded(r) if &r.member == member)
    }
    pub fn read_sent(&mut self) {
        match self {
            Self::Absent => {}
            Self::Pending(r)
            | Self::Requested(r)
            | Self::Loaded(r)
            | Self::ReadSent(r)
            | Self::Failed(r) => *self = Self::ReadSent(r.clone()),
        }
    }
    pub fn retain_identity(&mut self, member: Option<&Member>) {
        if self.member().is_some() && self.member() != member {
            *self = Self::Absent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member() -> Member {
        Member {
            origin: "local".into(),
            account: "test".into(),
            chat: "source".into(),
        }
    }
    #[test]
    fn every_operation_is_defined_for_every_state() {
        let r = Restoration {
            member: member(),
            token: 17,
        };
        let states = [
            State::Absent,
            State::Pending(r.clone()),
            State::Requested(r.clone()),
            State::Loaded(r.clone()),
            State::Failed(r.clone()),
            State::ReadSent(r.clone()),
        ];
        for initial in states {
            let mut state = initial.clone();
            let token = state.request();
            if matches!(initial, State::Pending(_)) {
                assert_eq!(token, Some(17));
                assert_eq!(state, State::Requested(r.clone()));
            } else {
                assert_eq!(token, None);
                assert_eq!(state, initial);
            }
            for success in [false, true] {
                for token in [16, 17, 18] {
                    let mut state = initial.clone();
                    state.complete(token, success);
                    let expected = if matches!(initial, State::Requested(_)) && token == 17 {
                        if success {
                            State::Loaded(r.clone())
                        } else {
                            State::Failed(r.clone())
                        }
                    } else {
                        initial.clone()
                    };
                    assert_eq!(state, expected);
                }
            }
            let mut state = initial.clone();
            state.retry();
            if initial.pending() {
                assert!(matches!(state, State::Pending(_)));
                assert_ne!(state.request(), Some(17));
            } else {
                assert_eq!(state, initial);
            }
            let mut state = initial.clone();
            state.read_sent();
            assert_eq!(
                state,
                if initial == State::Absent {
                    State::Absent
                } else {
                    State::ReadSent(r.clone())
                }
            );
            let mut state = initial.clone();
            state.retain_identity(Some(&r.member));
            assert_eq!(state, initial);
            state.retain_identity(None);
            assert_eq!(state, State::Absent);
            assert_eq!(
                initial.ready(&r.member),
                matches!(initial, State::Loaded(_))
            );
            assert_eq!(
                initial.pending(),
                matches!(
                    initial,
                    State::Pending(_) | State::Requested(_) | State::Loaded(_) | State::Failed(_)
                )
            );
            let mut state = initial;
            state.restore(r.member.clone());
            assert!(matches!(state, State::Pending(_)));
            assert_ne!(state.request(), Some(17));
        }
    }

    #[test]
    fn transition_table_and_stale_completions() {
        let member = member();
        let mut state = State::default();
        assert_eq!(state, State::Absent);
        state.complete(1, true);
        assert_eq!(state, State::Absent);
        state.restore(member.clone());
        assert!(matches!(state, State::Pending(_)));
        let first = state.request().unwrap();
        assert!(state.request().is_none());
        state.complete(first + 1, true);
        assert!(matches!(state, State::Requested(_)));
        state.complete(first, false);
        assert!(matches!(state, State::Failed(_)));
        state.complete(first, true);
        assert!(matches!(state, State::Failed(_)));
        state.retry();
        let second = state.request().unwrap();
        assert_ne!(first, second);
        state.complete(first, true);
        assert!(!state.ready(&member));
        state.complete(second, true);
        assert!(state.ready(&member));
        state.retain_identity(Some(&member));
        assert!(state.ready(&member));
        state.read_sent();
        assert!(!state.pending());
        state.complete(second, true);
        assert!(matches!(state, State::ReadSent(_)));
        // A new restoration can never reuse a cached success.
        state.restore(member.clone());
        let third = state.request().unwrap();
        state.complete(second, true);
        assert!(!state.ready(&member));
        state.retain_identity(None);
        state.complete(third, true);
        assert_eq!(state, State::Absent);
    }
    #[test]
    fn explicit_read_ends_every_pending_phase_and_identity_changes_invalidate() {
        for phase in [0, 1, 2, 3] {
            let mut state = State::default();
            let member = member();
            state.restore(member.clone());
            if phase > 0 {
                let token = state.request().unwrap();
                if phase > 1 {
                    state.complete(token, phase == 2);
                }
            }
            assert!(state.pending());
            state.read_sent();
            assert!(matches!(state, State::ReadSent(_)));
            assert!(!state.pending());
            let mut changed = member;
            changed.account = "other".into();
            state.retain_identity(Some(&changed));
            assert_eq!(state, State::Absent);
        }
    }
}
