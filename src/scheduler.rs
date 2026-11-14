//! Send Later: a durable queue of messages to send at a chosen time.
//!
//! The Beeper Server API has no scheduled send, and the Server itself is a
//! closed binary, so scheduling lives here: a persisted queue plus a tick
//! loop. The same core backs a companion scheduler running next to the
//! Server (delivery without this app open) and an in-app queue.
//!
//! Safety rules, in order:
//! - A send is claimed on disk (state `Sending`) BEFORE the network call, so
//!   a crash can never silently lose that a send was attempted.
//! - A crash between the network call and persisting its result leaves the
//!   item in `Sending`; startup recovery moves those to `Uncertain`.
//! - `Uncertain` items are NEVER resent automatically: the API has no
//!   idempotency, so a blind retry could duplicate the message. Reconcile
//!   first (search the chat for the message), then the user decides.
//! - Items whose due time passed long ago (scheduler was down) become
//!   `Missed` and wait for an explicit send-now, reschedule or cancel -
//!   no silent late sends.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// All times are Unix seconds. The UI converts with jiff at the boundary;
// the queue itself stays integer math so persistence needs no jiff/serde
// feature and ordering is trivial.

/// On-disk schema version. Unknown versions refuse to load rather than
/// silently dropping scheduled messages.
pub const SCHEMA: u32 = 1;

/// Default grace after which an overdue item is Missed, not sent: 15 min.
pub const DEFAULT_MISSED_GRACE_SECS: i64 = 15 * 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum State {
    /// Waiting for its due time.
    Scheduled,
    /// Claimed on disk; a send attempt is (or was) in flight.
    Sending,
    /// The API accepted the message.
    Sent,
    /// The API rejected the send before it could go out (validation, auth,
    /// unknown chat). Safe to retry after the cause is fixed.
    Failed,
    /// Due time passed while no scheduler was running; needs a decision.
    Missed,
    /// A send attempt ended without knowing whether it went out (timeout,
    /// connection drop). Never auto-resent; reconcile, then the user decides.
    Uncertain,
    /// Cancelled by the user. Kept for history.
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledMessage {
    /// Unique id, assigned by the creator; never reused.
    pub id: String,
    pub account_id: String,
    pub chat_id: String,
    pub text: String,
    pub reply_to: Option<String>,
    pub due_at: i64,
    pub created_at: i64,
    pub state: State,
    pub attempts: u32,
    pub error: Option<String>,
    /// Pending message id returned by the API on a successful send.
    pub sent_message_id: Option<String>,
    /// Set on an occurrence of a repeating series. Absent on one-shot
    /// items and on queues written before repeats existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<crate::recurrence::Repeat>,
}

impl ScheduledMessage {
    pub fn new(
        id: String,
        account_id: String,
        chat_id: String,
        text: String,
        reply_to: Option<String>,
        due_at: i64,
        created_at: i64,
    ) -> Self {
        Self {
            id,
            account_id,
            chat_id,
            text,
            reply_to,
            due_at,
            created_at,
            state: State::Scheduled,
            attempts: 0,
            error: None,
            sent_message_id: None,
            repeat: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Queue {
    pub schema: u32,
    pub items: Vec<ScheduledMessage>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            items: Vec::new(),
        }
    }
}

impl Queue {
    /// A missing file is an empty queue; a corrupt or wrong-version file is
    /// an error and is left untouched, never overwritten.
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let queue: Self = serde_json::from_slice(&bytes).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("queue corrupt: {e}"))
                })?;
                if queue.schema != SCHEMA {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("queue schema {} unsupported", queue.schema),
                    ));
                }
                Ok(queue)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Atomic save: write a sibling temp file, flush, rename over the old
    /// one. A crash mid-save leaves either the old or the new queue, never
    /// a torn one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = tmp_path(path);
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&tmp, &bytes)?;
        // Flush directory metadata too where possible; rename is the part
        // that makes the swap atomic on every supported platform.
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ScheduledMessage> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ScheduledMessage> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    /// Add a message. Ids must be unique; a duplicate id is an error.
    pub fn add(&mut self, message: ScheduledMessage) -> Result<(), QueueError> {
        if self.get(&message.id).is_some() {
            return Err(QueueError::DuplicateId(message.id));
        }
        self.items.push(message);
        Ok(())
    }

    /// Queue a replacement for an existing item: the new item is added
    /// and the old one cancelled in one step. If the old item cannot be
    /// cancelled (a send is already in flight), the new item is NOT kept:
    /// queuing it would risk sending both. Returns the new id.
    pub fn add_replacing(
        &mut self,
        message: ScheduledMessage,
        replaces: &str,
    ) -> Result<String, QueueError> {
        let new_id = message.id.clone();
        self.add(message)?;
        if let Err(e) = self.cancel(replaces) {
            self.items.retain(|item| item.id != new_id);
            return Err(e);
        }
        Ok(new_id)
    }

    pub fn cancel(&mut self, id: &str) -> Result<(), QueueError> {
        let item = self
            .get_mut(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        match item.state {
            State::Scheduled | State::Missed | State::Failed | State::Uncertain => {
                item.state = State::Cancelled;
                Ok(())
            }
            // A send may already be in flight; cancelling now could duplicate.
            state => Err(QueueError::BadState {
                id: id.into(),
                state,
            }),
        }
    }

    /// Change text and/or due time. Only pending items can be edited; the
    /// item returns to a clean Scheduled state.
    pub fn edit(
        &mut self,
        id: &str,
        text: Option<String>,
        due_at: Option<i64>,
    ) -> Result<(), QueueError> {
        let item = self
            .get_mut(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        match item.state {
            State::Scheduled | State::Missed | State::Failed => {
                if let Some(text) = text {
                    item.text = text;
                }
                if let Some(due_at) = due_at {
                    item.due_at = due_at;
                }
                item.state = State::Scheduled;
                item.error = None;
                Ok(())
            }
            state => Err(QueueError::BadState {
                id: id.into(),
                state,
            }),
        }
    }

    /// After reconciliation found no copy, the user may resend an Uncertain
    /// or Failed item; it goes through the normal claim-before-send path.
    /// An explicit retry re-due's the item for now; without this a Missed
    /// item past the grace window would be missed again on the next tick.
    pub fn retry(&mut self, id: &str, now: i64) -> Result<(), QueueError> {
        let item = self
            .get_mut(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        match item.state {
            State::Failed | State::Uncertain | State::Missed => {
                remember_plan(item);
                item.state = State::Scheduled;
                item.error = None;
                item.due_at = now;
                Ok(())
            }
            state => Err(QueueError::BadState {
                id: id.into(),
                state,
            }),
        }
    }
}

/// Keep the planned date of a repeating occurrence before it is moved.
fn remember_plan(item: &mut ScheduledMessage) {
    let due_at = item.due_at;
    if let Some(repeat) = item.repeat.as_mut()
        && repeat.planned.is_none()
    {
        repeat.planned = Some(due_at);
    }
}

impl Queue {
    /// Send an occurrence now without moving its series: the next
    /// occurrence still follows the planned date.
    pub fn send_now(&mut self, id: &str, now: i64) -> Result<(), QueueError> {
        if let Some(item) = self.get_mut(id)
            && matches!(item.state, State::Scheduled | State::Missed | State::Failed)
        {
            remember_plan(item);
        }
        self.edit(id, None, Some(now))
    }

    /// Startup repair: an occurrence persisted as `Sent` whose successor
    /// was never written (a crash between the two) gets its successor now.
    /// Safe to repeat: the successor id is derived from the series and the
    /// count, so an existing one (even a cancelled one) blocks a duplicate.
    pub fn repair_series(&mut self, now: i64) -> Vec<String> {
        let sent: Vec<String> = self
            .items
            .iter()
            .filter(|item| item.state == State::Sent && item.repeat.is_some())
            .map(|item| item.id.clone())
            .collect();
        sent.iter()
            .filter_map(|id| self.spawn_next(id, now))
            .collect()
    }

    /// Pending occurrences of repeating series (not yet sent or cancelled).
    pub fn active_series(&self) -> usize {
        self.items
            .iter()
            .filter(|item| {
                item.repeat.is_some()
                    && matches!(
                        item.state,
                        State::Scheduled
                            | State::Sending
                            | State::Failed
                            | State::Missed
                            | State::Uncertain
                    )
            })
            .count()
    }

    /// After an occurrence is `Sent`, queue the next one. Only the daemon's
    /// send path and a successful reconcile call this, so an occurrence
    /// that failed, is uncertain or was cancelled never advances its
    /// series. Returns the new id, or `None` when the series has ended.
    pub fn spawn_next(&mut self, id: &str, now: i64) -> Option<String> {
        let item = self.get(id)?;
        if item.state != State::Sent {
            return None;
        }
        let mut repeat = item.repeat.clone()?;
        repeat.sent += 1;
        if repeat.ends_after.is_some_and(|limit| repeat.sent >= limit) {
            return None;
        }
        // Step from the planned date, so sending early (send now) or late
        // (retry) does not shift the series.
        let base = repeat.planned.unwrap_or(item.due_at);
        repeat.planned = None;
        let due_at = repeat.next_after(base, now)?;
        let mut next = ScheduledMessage::new(
            format!("{}~{}", repeat.series, repeat.sent),
            item.account_id.clone(),
            item.chat_id.clone(),
            item.text.clone(),
            item.reply_to.clone(),
            due_at,
            now,
        );
        next.repeat = Some(repeat);
        let next_id = next.id.clone();
        self.add(next).ok()?;
        Some(next_id)
    }

    /// Skip the pending occurrence of a series: move it to the next date
    /// after now without sending. A series with no next date ends
    /// (the item is cancelled).
    pub fn skip(&mut self, id: &str, now: i64) -> Result<(), QueueError> {
        let item = self
            .get_mut(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        let bad = |state| QueueError::BadState {
            id: id.into(),
            state,
        };
        let Some(repeat) = item.repeat.clone() else {
            return Err(bad(item.state));
        };
        match item.state {
            State::Scheduled | State::Missed | State::Failed => {
                match repeat.next_after(item.due_at, now) {
                    Some(due_at) => {
                        item.due_at = due_at;
                        item.state = State::Scheduled;
                        item.error = None;
                    }
                    None => item.state = State::Cancelled,
                }
                Ok(())
            }
            state => Err(bad(state)),
        }
    }

    /// Pause or resume a series. A paused occurrence is never sent; on
    /// resume an occurrence already past its grace window becomes Missed
    /// and waits for a decision, like any other overdue item.
    pub fn set_paused(&mut self, id: &str, paused: bool) -> Result<(), QueueError> {
        let item = self
            .get_mut(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        let state = item.state;
        match (&mut item.repeat, state) {
            (Some(repeat), State::Scheduled | State::Missed | State::Failed) => {
                repeat.paused = paused;
                Ok(())
            }
            _ => Err(QueueError::BadState {
                id: id.into(),
                state,
            }),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    DuplicateId(String),
    NotFound(String),
    BadState { id: String, state: State },
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// The send channel the scheduler writes through. The companion service
/// implements this against the Server API; tests use a mock.
pub trait Send {
    /// Attempt the send. Ok carries the API's pending message id.
    fn send(&mut self, item: &ScheduledMessage) -> Result<String, SendError>;
    /// Look for the message already in the chat, used before any retry of
    /// an Uncertain item.
    fn reconcile(&mut self, item: &ScheduledMessage) -> Reconcile;
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// The API rejected the request before sending (4xx, validation). The
    /// message definitely did not go out.
    Rejected(String),
    /// The attempt ended without knowing the outcome (timeout, dropped
    /// connection). The message may have gone out.
    Unknown(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Reconcile {
    /// A matching outgoing message exists in the chat after the due time.
    Found,
    /// No matching message; a retry will not duplicate.
    NotFound,
    /// The check itself failed; stay Uncertain.
    #[default]
    Unknown,
}

pub struct Scheduler<S> {
    pub queue: Queue,
    pub sender: S,
    pub missed_grace_secs: i64,
}

/// One send attempt's outcome, returned for reporting.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent { id: String },
    Failed { id: String },
    Uncertain { id: String },
    Missed { id: String },
}

impl<S: Send> Scheduler<S> {
    pub fn new(queue: Queue, sender: S) -> Self {
        Self {
            queue,
            sender,
            missed_grace_secs: DEFAULT_MISSED_GRACE_SECS,
        }
    }

    /// Startup recovery: a `Sending` item means a crash hit between the
    /// claim and the result. Its delivery is unknown: mark Uncertain.
    /// `Sent`/`Failed` results were persisted before any crash, so only
    /// `Sending` moves. Returns the ids now needing reconciliation.
    pub fn recover(&mut self) -> Vec<String> {
        let mut recovered = Vec::new();
        for item in &mut self.queue.items {
            if item.state == State::Sending {
                item.state = State::Uncertain;
                recovered.push(item.id.clone());
            }
        }
        // A series whose next occurrence was lost to a crash is repaired.
        self.queue.repair_series(jiff::Timestamp::now().as_second());
        recovered
    }

    /// Send everything due at `now`. `persist` is invoked after every state
    /// change and MUST durably save the queue; the claim-before-send order
    /// (persist `Sending` before calling `send`) is what makes a crash
    /// recoverable instead of a silent duplicate.
    pub fn tick(&mut self, now: i64, persist: &mut dyn FnMut(&Queue)) -> Vec<Outcome> {
        let mut outcomes = Vec::new();
        // Indices first: due order, earliest first, independent of queue order.
        let mut due: Vec<usize> = self
            .queue
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.state == State::Scheduled
                    && item.due_at <= now
                    && !item.repeat.as_ref().is_some_and(|r| r.paused)
            })
            .map(|(i, _)| i)
            .collect();
        due.sort_by_key(|&i| self.queue.items[i].due_at);
        for i in due {
            let item = &mut self.queue.items[i];
            let overdue = now - item.due_at;
            if overdue > self.missed_grace_secs {
                item.state = State::Missed;
                let id = item.id.clone();
                persist(&self.queue);
                outcomes.push(Outcome::Missed { id });
                continue;
            }
            // Claim on disk BEFORE the network call.
            item.state = State::Sending;
            item.attempts += 1;
            let id = item.id.clone();
            persist(&self.queue);
            let result = self.sender.send(&self.queue.items[i]);
            let item = &mut self.queue.items[i];
            let outcome = match result {
                Ok(message_id) => {
                    item.state = State::Sent;
                    item.sent_message_id = Some(message_id);
                    Outcome::Sent { id }
                }
                Err(SendError::Rejected(error)) => {
                    item.state = State::Failed;
                    item.error = Some(error);
                    Outcome::Failed { id }
                }
                Err(SendError::Unknown(error)) => {
                    item.state = State::Uncertain;
                    item.error = Some(error);
                    Outcome::Uncertain { id }
                }
            };
            if matches!(outcome, Outcome::Sent { .. }) {
                // The next occurrence is queued in the same write as the
                // Sent state, so a crash cannot leave a series without one.
                let sent_id = self.queue.items[i].id.clone();
                self.queue.spawn_next(&sent_id, now);
            }
            persist(&self.queue);
            outcomes.push(outcome);
        }
        outcomes
    }

    /// Reconcile an Uncertain item: `Found` settles it as Sent (with the
    /// duplicate risk gone), anything else leaves the state untouched for
    /// the user to retry (via `retry`, after `NotFound`) or cancel.
    pub fn reconcile(
        &mut self,
        id: &str,
        persist: &mut dyn FnMut(&Queue),
    ) -> Result<Reconcile, QueueError> {
        let item = self
            .queue
            .get(id)
            .ok_or_else(|| QueueError::NotFound(id.into()))?;
        if item.state != State::Uncertain {
            return Err(QueueError::BadState {
                id: id.into(),
                state: item.state,
            });
        }
        let verdict = self.sender.reconcile(item);
        if verdict == Reconcile::Found {
            let item = self.queue.get_mut(id).expect("item checked above");
            item.state = State::Sent;
            persist(&self.queue);
            let now = jiff::Timestamp::now().as_second();
            if self.queue.spawn_next(id, now).is_some() {
                persist(&self.queue);
            }
        }
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed reference instant; only relative offsets matter.
    fn ts(seconds: i64) -> i64 {
        1_762_000_000 + seconds
    }

    fn item(id: &str, due: i64) -> ScheduledMessage {
        ScheduledMessage::new(
            id.into(),
            "wa-main".into(),
            "chat-1".into(),
            format!("hello {id}"),
            None,
            due,
            ts(0),
        )
    }

    #[derive(Default)]
    struct Mock {
        results: std::collections::VecDeque<Result<String, SendError>>,
        reconcile_verdict: Reconcile,
        /// States observed by the persist callback, in order.
        persisted: Vec<Vec<State>>,
        /// Items handed to send(), in order.
        sent: Vec<String>,
    }

    impl Send for Mock {
        fn send(&mut self, item: &ScheduledMessage) -> Result<String, SendError> {
            self.sent.push(item.id.clone());
            self.results.pop_front().unwrap_or(Ok("msg-1".into()))
        }

        fn reconcile(&mut self, _item: &ScheduledMessage) -> Reconcile {
            self.reconcile_verdict
        }
    }

    fn scheduler(queue: Queue, mock: Mock) -> Scheduler<Mock> {
        Scheduler::new(queue, mock)
    }

    fn persist_into(mock: *mut Vec<Vec<State>>) -> impl FnMut(&Queue) {
        move |queue: &Queue| unsafe {
            (*mock).push(queue.items.iter().map(|item| item.state).collect());
        }
    }

    #[test]
    fn sends_due_items_earliest_first() {
        let mut queue = Queue::default();
        queue.add(item("later", ts(60))).unwrap();
        queue.add(item("first", ts(-5))).unwrap();
        queue.add(item("second", ts(30))).unwrap();
        let mut sched = scheduler(queue, Mock::default());
        let outcomes = sched.tick(
            ts(45),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        assert_eq!(sched.sender.sent, vec!["first", "second"]);
        assert!(matches!(outcomes[0], Outcome::Sent { .. }));
        assert_eq!(sched.queue.get("later").unwrap().state, State::Scheduled);
        assert_eq!(sched.queue.get("first").unwrap().state, State::Sent);
    }

    #[test]
    fn claim_is_persisted_before_the_network_call() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(-1))).unwrap();
        let mut sched = scheduler(queue, Mock::default());
        sched.tick(
            ts(0),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        let persisted = &sched.sender.persisted;
        // First persisted state is the Sending claim, last is the result.
        assert_eq!(persisted[0], vec![State::Sending]);
        assert_eq!(persisted.last().unwrap(), &vec![State::Sent]);
        assert_eq!(sched.queue.get("a").unwrap().attempts, 1);
    }

    #[test]
    fn rejection_fails_without_retry() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(-1))).unwrap();
        let mut mock = Mock::default();
        mock.results
            .push_back(Err(SendError::Rejected("unknown chat".into())));
        let mut sched = scheduler(queue, mock);
        sched.tick(
            ts(0),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        let item = sched.queue.get("a").unwrap();
        assert_eq!(item.state, State::Failed);
        assert_eq!(item.error.as_deref(), Some("unknown chat"));
        assert_eq!(sched.sender.sent.len(), 1);
    }

    #[test]
    fn unknown_outcome_becomes_uncertain_and_is_never_resent() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(-1))).unwrap();
        let mut mock = Mock::default();
        mock.results
            .push_back(Err(SendError::Unknown("timeout".into())));
        let mut sched = scheduler(queue, mock);
        sched.tick(
            ts(0),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        sched.tick(
            ts(120),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        assert_eq!(sched.queue.get("a").unwrap().state, State::Uncertain);
        // Exactly one network attempt across both ticks.
        assert_eq!(sched.sender.sent.len(), 1);
    }

    #[test]
    fn crash_mid_send_recovers_as_uncertain() {
        let mut queue = Queue::default();
        let mut crashed = item("a", ts(-1));
        crashed.state = State::Sending;
        crashed.attempts = 1;
        queue.add(crashed).unwrap();
        let mut sched = scheduler(queue, Mock::default());
        let recovered = sched.recover();
        assert_eq!(recovered, vec!["a"]);
        assert_eq!(sched.queue.get("a").unwrap().state, State::Uncertain);
    }

    #[test]
    fn reconcile_found_settles_sent_without_another_send() {
        let mut queue = Queue::default();
        let mut uncertain = item("a", ts(-1));
        uncertain.state = State::Uncertain;
        queue.add(uncertain).unwrap();
        let mock = Mock {
            reconcile_verdict: Reconcile::Found,
            ..Default::default()
        };
        let mut sched = scheduler(queue, mock);
        let verdict = sched
            .reconcile(
                "a",
                &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
            )
            .unwrap();
        assert_eq!(verdict, Reconcile::Found);
        assert_eq!(sched.queue.get("a").unwrap().state, State::Sent);
        assert!(sched.sender.sent.is_empty());
    }

    #[test]
    fn reconcile_not_found_leaves_uncertain_for_user_retry() {
        let mut queue = Queue::default();
        let mut uncertain = item("a", ts(-1));
        uncertain.state = State::Uncertain;
        queue.add(uncertain).unwrap();
        let mock = Mock {
            reconcile_verdict: Reconcile::NotFound,
            ..Default::default()
        };
        let mut sched = scheduler(queue, mock);
        sched
            .reconcile(
                "a",
                &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
            )
            .unwrap();
        assert_eq!(sched.queue.get("a").unwrap().state, State::Uncertain);
        sched.queue.retry("a", ts(1)).unwrap();
        assert_eq!(sched.queue.get("a").unwrap().state, State::Scheduled);
    }

    #[test]
    fn long_overdue_is_missed_not_sent() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(-3600))).unwrap();
        let mut sched = scheduler(queue, Mock::default());
        let outcomes = sched.tick(
            ts(0),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        assert_eq!(outcomes, vec![Outcome::Missed { id: "a".into() }]);
        assert_eq!(sched.queue.get("a").unwrap().state, State::Missed);
        assert!(sched.sender.sent.is_empty());
        // A missed item only goes out through an explicit retry.
        sched.queue.retry("a", ts(1)).unwrap();
        sched.tick(
            ts(1),
            &mut persist_into(&sched.sender.persisted as *const _ as *mut _),
        );
        assert_eq!(sched.queue.get("a").unwrap().state, State::Sent);
    }

    #[test]
    fn cancel_and_edit_rules() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(60))).unwrap();
        // Edit moves due time and keeps Scheduled.
        queue
            .edit("a", Some("changed".into()), Some(ts(120)))
            .unwrap();
        let edited = queue.get("a").unwrap();
        assert_eq!(edited.text, "changed");
        assert_eq!(edited.due_at, ts(120));
        queue.cancel("a").unwrap();
        assert_eq!(queue.get("a").unwrap().state, State::Cancelled);
        // Cancelled items cannot be edited or sent.
        assert!(queue.edit("a", None, None).is_err());
        // A Sending item cannot be cancelled mid-flight.
        queue.add(item("b", ts(60))).unwrap();
        queue.get_mut("b").unwrap().state = State::Sending;
        assert!(queue.cancel("b").is_err());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut queue = Queue::default();
        queue.add(item("a", ts(60))).unwrap();
        assert_eq!(
            queue.add(item("a", ts(61))),
            Err(QueueError::DuplicateId("a".into()))
        );
    }

    #[test]
    fn add_replacing_cancels_the_original() {
        let mut q = Queue::default();
        let old = ScheduledMessage::new(
            "a".into(),
            "acc".into(),
            "c".into(),
            "old".into(),
            None,
            100,
            0,
        );
        q.add(old).unwrap();
        let new = ScheduledMessage::new(
            "b".into(),
            "acc".into(),
            "c".into(),
            "new".into(),
            None,
            200,
            0,
        );
        let id = q.add_replacing(new, "a").unwrap();
        assert_eq!(id, "b");
        assert_eq!(q.get("a").unwrap().state, State::Cancelled);
        assert_eq!(q.get("b").unwrap().state, State::Scheduled);
    }

    #[test]
    fn add_replacing_a_sending_item_keeps_neither() {
        // A send is in flight for the original; queuing the replacement
        // would risk sending both, so the whole operation fails.
        let mut q = Queue::default();
        let old = ScheduledMessage::new(
            "a".into(),
            "acc".into(),
            "c".into(),
            "old".into(),
            None,
            100,
            0,
        );
        q.add(old).unwrap();
        q.get_mut("a").unwrap().state = State::Sending;
        let new = ScheduledMessage::new(
            "b".into(),
            "acc".into(),
            "c".into(),
            "new".into(),
            None,
            200,
            0,
        );
        assert!(q.add_replacing(new, "a").is_err());
        assert!(q.get("b").is_none());
        assert_eq!(q.get("a").unwrap().state, State::Sending);
    }

    #[test]
    fn cancel_rejects_a_claimed_send() {
        let mut q = Queue::default();
        q.add(ScheduledMessage::new(
            "a".into(),
            "acc".into(),
            "c".into(),
            "hi".into(),
            None,
            100,
            0,
        ))
        .unwrap();
        q.get_mut("a").unwrap().state = State::Sending;
        assert!(q.cancel("a").is_err());
        assert!(q.retry("a", ts(0)).is_err());
        assert!(q.edit("a", Some("x".into()), None).is_err());
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("scheduler-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queue.json");
        let mut queue = Queue::default();
        queue.add(item("a", ts(60))).unwrap();
        queue.save(&path).unwrap();
        assert!(!tmp_path(&path).exists());
        let loaded = Queue::load(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, "a");
        // A corrupt file is an error, never silently replaced.
        fs::write(&path, b"{not json").unwrap();
        assert!(Queue::load(&path).is_err());
        fs::remove_dir_all(&dir).ok();
        // A missing file is an empty queue.
        let missing = dir.join("nope.json");
        assert_eq!(Queue::load(&missing).unwrap().items.len(), 0);
    }

    fn series(id: &str, due: i64) -> ScheduledMessage {
        let mut message = item(id, due);
        message.repeat = Some(crate::recurrence::Repeat {
            series: id.into(),
            every: 1,
            unit: crate::recurrence::Unit::Day,
            weekdays: 0,
            hour: 12,
            minute: 26,
            day: 1,
            tz: "UTC".into(),
            ends_after: None,
            until: None,
            sent: 0,
            paused: false,
            planned: None,
        });
        message
    }

    #[test]
    fn a_sent_occurrence_queues_exactly_the_next_one() {
        let mut queue = Queue::default();
        queue.add(series("s", ts(0))).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        let outcomes = sched.tick(ts(1), &mut |_| {});
        assert_eq!(outcomes, vec![Outcome::Sent { id: "s".into() }]);
        assert_eq!(sched.queue.items.len(), 2);
        let next = sched.queue.get("s~1").unwrap();
        assert_eq!(next.state, State::Scheduled);
        assert!(next.due_at > ts(1));
        assert_eq!(next.repeat.as_ref().unwrap().sent, 1);
        // A second tick at the same instant sends nothing and queues nothing.
        assert!(sched.tick(ts(2), &mut |_| {}).is_empty());
        assert_eq!(sched.queue.items.len(), 2);
    }

    #[test]
    fn a_failed_or_uncertain_occurrence_does_not_advance_the_series() {
        let mut queue = Queue::default();
        queue.add(series("f", ts(0))).unwrap();
        queue.add(series("u", ts(0))).unwrap();
        let mut mock = Mock::default();
        mock.results.push_back(Err(SendError::Rejected("no".into())));
        mock.results.push_back(Err(SendError::Unknown("timeout".into())));
        let mut sched = Scheduler::new(queue, mock);
        sched.tick(ts(1), &mut |_| {});
        assert_eq!(sched.queue.items.len(), 2);
        assert_eq!(sched.queue.get("f").unwrap().state, State::Failed);
        assert_eq!(sched.queue.get("u").unwrap().state, State::Uncertain);
    }

    #[test]
    fn a_missed_occurrence_waits_and_does_not_catch_up() {
        let mut queue = Queue::default();
        queue.add(series("m", ts(0))).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        // Three days of downtime.
        let outcomes = sched.tick(ts(3 * 86_400), &mut |_| {});
        assert_eq!(outcomes, vec![Outcome::Missed { id: "m".into() }]);
        assert_eq!(sched.queue.items.len(), 1);
        assert!(sched.sender.sent.is_empty());
        // Skip moves it to the next date after now, still one item.
        sched.queue.skip("m", ts(3 * 86_400)).unwrap();
        let skipped = sched.queue.get("m").unwrap();
        assert_eq!(skipped.state, State::Scheduled);
        assert!(skipped.due_at > ts(3 * 86_400));
        assert_eq!(sched.queue.items.len(), 1);
    }

    #[test]
    fn a_paused_series_is_not_sent_until_resumed() {
        let mut queue = Queue::default();
        queue.add(series("p", ts(0))).unwrap();
        queue.set_paused("p", true).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        assert!(sched.tick(ts(1), &mut |_| {}).is_empty());
        assert!(sched.sender.sent.is_empty());
        sched.queue.set_paused("p", false).unwrap();
        assert_eq!(sched.tick(ts(2), &mut |_| {}).len(), 1);
    }

    #[test]
    fn a_series_stops_after_its_count() {
        let mut queue = Queue::default();
        let mut last = series("c", ts(0));
        last.repeat.as_mut().unwrap().ends_after = Some(1);
        queue.add(last).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        sched.tick(ts(1), &mut |_| {});
        assert_eq!(sched.queue.items.len(), 1);
    }

    #[test]
    fn stopping_a_series_cancels_its_pending_occurrence_for_good() {
        let mut queue = Queue::default();
        queue.add(series("x", ts(0))).unwrap();
        queue.cancel("x").unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        assert!(sched.tick(ts(1), &mut |_| {}).is_empty());
        assert_eq!(sched.queue.items.len(), 1);
        assert_eq!(sched.queue.active_series(), 0);
    }

    #[test]
    fn old_queue_files_without_a_repeat_still_load() {
        let json = r#"{"schema":1,"items":[{"id":"a","account_id":"acc","chat_id":"c","text":"hi","reply_to":null,"due_at":5,"created_at":1,"state":"Scheduled","attempts":0,"error":null,"sent_message_id":null}]}"#;
        let queue: Queue = serde_json::from_str(json).unwrap();
        assert!(queue.items[0].repeat.is_none());
    }

    #[test]
    fn a_sent_occurrence_without_a_successor_is_repaired_once() {
        // The state a crash leaves between persisting Sent and queuing the next.
        let mut queue = Queue::default();
        let mut sent = series("r", ts(0));
        sent.state = State::Sent;
        queue.add(sent).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        sched.recover();
        assert_eq!(sched.queue.items.len(), 2);
        assert!(sched.queue.get("r~1").is_some());
        sched.recover();
        assert_eq!(sched.queue.items.len(), 2);
    }

    #[test]
    fn the_successor_is_persisted_with_the_sent_state() {
        let mut queue = Queue::default();
        queue.add(series("w", ts(0))).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        let mut snapshots: Vec<Vec<State>> = Vec::new();
        sched.tick(ts(1), &mut |q| {
            snapshots.push(q.items.iter().map(|item| item.state).collect());
        });
        // Claim, then one write holding Sent and the queued successor.
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[1], vec![State::Sent, State::Scheduled]);
    }

    #[test]
    fn sending_now_does_not_shift_the_series() {
        let mut queue = Queue::default();
        queue.add(series("n", ts(86_400))).unwrap();
        let planned_next = {
            let mut probe = Queue::default();
            probe.add(series("n", ts(86_400))).unwrap();
            probe.get_mut("n").unwrap().state = State::Sent;
            probe.spawn_next("n", ts(0));
            probe.get("n~1").unwrap().due_at
        };
        queue.send_now("n", ts(10)).unwrap();
        let mut sched = Scheduler::new(queue, Mock::default());
        sched.tick(ts(11), &mut |_| {});
        assert_eq!(sched.queue.get("n~1").unwrap().due_at, planned_next);
    }
}
