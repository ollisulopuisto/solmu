//! Local media submission state. A message POST is the retry boundary.
use crate::merged::Member;
use crate::model::ReplyTarget;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Payload {
    File {
        path: PathBuf,
        document: bool,
    },
    Picture {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Sending,
    /// Delivery confirmed; retained only to keep a sparse echo's local preview.
    Confirmed,
    Accepted {
        pending: String,
    },
    NotSubmitted(String),
    Unknown(String),
}
impl State {
    /// Failure-only presentation and the safe retry boundary. Ambiguous
    /// submissions must be checked in history, never blindly posted twice.
    pub fn failure(&self) -> Option<(&str, bool)> {
        match self {
            Self::NotSubmitted(error) => Some((error, true)),
            Self::Unknown(error) => Some((error, false)),
            Self::Sending | Self::Accepted { .. } | Self::Confirmed => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Job {
    pub temporary: bool,
    pub local: String,
    pub member: Member,
    pub payload: Payload,
    pub caption: Option<String>,
    pub quoting: Option<ReplyTarget>,
}
#[derive(Clone, Debug)]
pub struct Record {
    pub visible: String,
    pub job: Job,
    pub state: State,
}
impl Record {
    pub fn retryable(&self) -> bool {
        matches!(self.state, State::NotSubmitted(_))
    }
}

#[cfg(test)]
mod failure_ui_tests {
    use super::*;
    #[test]
    fn only_failures_have_a_status_indicator() {
        assert_eq!(State::Sending.failure(), None);
        assert_eq!(
            State::Accepted {
                pending: "id".into()
            }
            .failure(),
            None
        );
        assert_eq!(State::Confirmed.failure(), None);
        assert_eq!(
            State::NotSubmitted("offline".into()).failure(),
            Some(("offline", true))
        );
        assert_eq!(
            State::Unknown("timed out".into()).failure(),
            Some(("timed out", false))
        );
    }
}
