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
