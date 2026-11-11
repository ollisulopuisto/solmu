//! Local-only person links. Never send to a synthetic/automatic routing ID.
use crate::model::{Chat, Message};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Member {
    pub origin: String,
    pub account: String,
    pub chat: String,
}
impl Member {
    pub fn of(chat: &Chat) -> Option<Self> {
        Some(Self {
            origin: chat.backend_origin.clone()?,
            account: chat.backend_account.clone()?,
            chat: chat.id.clone(),
        })
    }
    pub fn matches(&self, chat: &Chat) -> bool {
        Self::of(chat).as_ref() == Some(self)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    pub members: Vec<Member>,
    #[serde(default)]
    pub primary: Option<Member>,
}
impl Person {
    pub fn default_destination<'a>(
        &'a self,
        available: &[Member],
    ) -> Result<Option<&'a Member>, &'static str> {
        match &self.primary {
            None => Ok(None),
            Some(primary) if self.members.contains(primary) && available.contains(primary) => {
                Ok(Some(primary))
            }
            Some(_) => Err(
                "Primary chat unavailable. Choose a destination explicitly; no fallback was selected.",
            ),
        }
    }
}
/// Exact same-member reply only. Cross-over is deliberately a new message.
pub fn reply_route(
    source: &Member,
    destination: &Member,
    id: &str,
) -> Option<crate::model::ReplyTarget> {
    (source == destination).then(|| crate::model::ReplyTarget {
        id: id.to_owned(),
        chat: None,
    })
}
/// Group only consecutive same-chat/sender/day runs, never different sources.
pub fn header_boundary(
    previous: Option<(&str, &str, &str, bool)>,
    current: (&str, &str, &str, bool),
) -> bool {
    previous != Some(current)
}
/// Whether a linked-timeline bubble earns its network mark: the first loaded
/// message always shows it; after that only a change from the previous
/// message's network does. A sender or direction change alone never repeats
/// the mark.
pub fn shows_network_mark(previous_network: Option<&str>, network: &str) -> bool {
    previous_network != Some(network)
}
/// Overlapping rows and a small overscan are measured; distant rows use cached heights.
pub fn row_near_view(
    top: f32,
    height: f32,
    view_top: f32,
    view_bottom: f32,
    overscan: f32,
) -> bool {
    top + height >= view_top - overscan && top <= view_bottom + overscan
}
/// Checks whether `text` contains `query_lower` (which is already lowercase),
/// avoiding string allocations when `is_ascii` is true.
fn case_contains(text: &str, query_lower: &str, is_ascii: bool) -> bool {
    if is_ascii {
        // Zero-allocation byte-level sliding window ASCII case-insensitive search.
        let q_bytes = query_lower.as_bytes();
        text.as_bytes()
            .windows(q_bytes.len())
            .any(|w| w.eq_ignore_ascii_case(q_bytes))
    } else {
        text.to_lowercase().contains(query_lower)
    }
}

/// Literal case-insensitive search of already-loaded data. No remote query.
pub fn search_matches(body: &str, query: &str, network: &str, sender: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    // Performance optimization:
    // 1. Return immediately when `query.trim()` is empty without lowercase conversion.
    // 2. When `query` is ASCII, match without allocating lowercased Strings for
    //    `body`, `network`, and `sender`.
    // 3. Short-circuit across `[body, network, sender]` sequentially to avoid
    //    redundant checks and allocations once a match is found.
    let is_ascii = query.is_ascii();
    let query_lower = query.to_lowercase();
    [body, network, sender]
        .into_iter()
        .any(|value| case_contains(value, &query_lower, is_ascii))
}
/// Keyboard-focusable fallback operates on the same filtered, ordered rows as the view.
pub fn latest_replyable<'a>(messages: &[&'a Message]) -> Option<&'a Message> {
    messages
        .iter()
        .rev()
        .copied()
        .find(|message| !matches!(message.content, crate::model::Content::Revoked))
}
/// One linked chat's paging, as the catch-up needs it.
pub struct Paging {
    pub chat: String,
    /// Timestamp of the oldest loaded message, if any is loaded.
    pub oldest: Option<i64>,
    /// No further history can be fetched for this chat.
    pub exhausted: bool,
    /// A request for older messages is in flight.
    pub busy: bool,
}

/// Why a chat stopped paging before reaching the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// A request went out and brought nothing older.
    NoProgress,
    /// No request could go out: the phone is not connected or is cooling
    /// down after an empty answer.
    NotNow,
}

/// "Load older" in progress. Each linked chat pages by count, so a busy chat
/// reaches back less far than a quiet one, and the merged view only shows a
/// gap-free stretch back to the newest of their starts. This keeps paging the
/// chats that start after `target` (the oldest message on screen when it was
/// pressed), so the timeline fills in below it, and records which chats could
/// not follow instead of dropping them silently.
#[derive(Clone, Debug)]
pub struct CatchUp {
    target: i64,
    /// The oldest message each chat had when its last request went out.
    asked: std::collections::HashMap<String, i64>,
    stopped: std::collections::BTreeMap<String, Stopped>,
}

impl CatchUp {
    pub fn new(target: i64) -> Self {
        Self {
            target,
            asked: Default::default(),
            stopped: Default::default(),
        }
    }

    /// The chats to ask for older messages now, and whether any request is
    /// still in flight. Chats that already gave up are left alone.
    pub fn plan(&mut self, members: &[Paging]) -> (Vec<String>, bool) {
        let mut requests = Vec::new();
        let mut waiting = false;
        for member in members {
            let Some(oldest) = member.oldest else {
                continue;
            };
            let reached = oldest <= self.target;
            let given_up = self.stopped.contains_key(&member.chat);
            if member.exhausted || reached || given_up {
                continue;
            }
            if member.busy {
                waiting = true;
            } else if self.asked.get(&member.chat) == Some(&oldest) {
                self.stopped
                    .insert(member.chat.clone(), Stopped::NoProgress);
            } else {
                requests.push(member.chat.clone());
            }
        }
        (requests, waiting)
    }

    /// The request for `chat` went out while its oldest message was `oldest`.
    pub fn asked(&mut self, chat: &str, oldest: i64) {
        self.asked.insert(chat.to_owned(), oldest);
    }

    /// No request could go out for `chat`.
    pub fn unavailable(&mut self, chat: &str) {
        self.stopped.insert(chat.to_owned(), Stopped::NotNow);
    }

    /// The chats that could not follow, with why.
    pub fn stopped(&self) -> impl Iterator<Item = (&str, Stopped)> {
        self.stopped.iter().map(pair)
    }
}

fn pair<'a>((chat, why): (&'a String, &'a Stopped)) -> (&'a str, Stopped) {
    (chat.as_str(), *why)
}

/// Sort pointer-sized references, not message text/media payloads, on every frame.
pub fn ordered_refs(mut messages: Vec<&Message>) -> Vec<&Message> {
    messages.sort_unstable_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then(a.chat.cmp(&b.chat))
            .then(a.history_order.cmp(&b.history_order))
            .then(a.id.cmp(&b.id))
    });
    messages.dedup_by(|a, b| a.chat == b.chat && a.id == b.id);
    messages
}
pub fn ordered(mut messages: Vec<Message>) -> Vec<Message> {
    messages.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then(a.chat.cmp(&b.chat))
            .then(a.history_order.cmp(&b.history_order))
            .then(a.id.cmp(&b.id))
    });
    messages.dedup_by(|a, b| a.chat == b.chat && a.id == b.id);
    messages
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_mark_shows_on_first_and_on_network_change_only() {
        // First loaded message always carries the mark.
        assert!(shows_network_mark(None, "WhatsApp"));
        // A change of network earns the mark again...
        assert!(shows_network_mark(Some("WhatsApp"), "Instagram"));
        assert!(shows_network_mark(Some("Instagram"), "WhatsApp"));
        // ...but the same network never repeats it, whether the sender or
        // the direction changed or not.
        assert!(!shows_network_mark(Some("WhatsApp"), "WhatsApp"));
    }
    #[test]
    fn keyboard_reply_uses_last_matching_exact_source_and_skips_revoked() {
        let mut chat = Chat::new("work".into(), "Work".into());
        chat.backend_origin = Some("server".into());
        chat.backend_account = Some("work-account".into());
        let first = Message {
            id: "same".into(),
            chat: "signal".into(),
            sender: "x".into(),
            sender_name: None,
            from_me: false,
            timestamp: 1,
            history_order: None,
            content: crate::model::Content::text("signal"),
            status: crate::model::Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: vec![],
            edited: false,
            mentions: vec![],
            forwarded: false,
            thumbnail: None,
        };
        let mut work = first.clone();
        work.chat = chat.id.clone();
        work.timestamp = 2;
        let mut revoked = work.clone();
        revoked.id = "revoked".into();
        revoked.timestamp = 3;
        revoked.content = crate::model::Content::Revoked;
        let rows = ordered_refs(vec![&revoked, &work, &first]);
        assert!(std::ptr::eq(latest_replyable(&rows).unwrap(), &work));
        assert_eq!(
            Member::of(&chat).unwrap().chat,
            latest_replyable(&rows).unwrap().chat
        );
        assert!(latest_replyable(&[]).is_none());
        assert!(latest_replyable(&[&revoked]).is_none());
    }
    #[test]
    fn consecutive_headers_never_hide_a_source_change() {
        assert!(header_boundary(None, ("signal", "friend", "one", false)));
        assert!(!header_boundary(
            Some(("signal", "friend", "one", false)),
            ("signal", "friend", "one", false)
        ));
        assert!(header_boundary(
            Some(("signal", "friend", "one", false)),
            ("work", "friend", "one", false)
        ));
        assert!(header_boundary(
            Some(("signal", "friend", "one", false)),
            ("signal", "friend", "two", false)
        ));
        assert!(header_boundary(
            Some(("signal", "friend", "one", false)),
            ("signal", "me", "one", true)
        ));
    }
    #[test]
    fn virtual_rows_include_edges_and_overscan_only() {
        assert!(row_near_view(0.0, 80.0, 80.0, 700.0, 0.0));
        assert!(row_near_view(800.0, 80.0, 80.0, 700.0, 120.0));
        assert!(!row_near_view(1000.0, 80.0, 80.0, 700.0, 120.0));
        assert!(!row_near_view(0.0, 80.0, 300.0, 700.0, 120.0));
    }
    #[test]
    fn loaded_search_is_literal_unicode_and_source_scoped() {
        assert!(search_matches(
            "Hyvää yötä, Alice",
            "Hyvää",
            "SIGNAL",
            "friend"
        ));
        assert!(search_matches("plain text", "signal", "Signal", "friend"));
        assert!(search_matches("plain text", "friend", "Work", "Friend"));
        assert!(!search_matches("plain text", "signal", "Work", "friend"));
        assert!(!search_matches("plain text", ".*", "Work", "friend"));
        assert!(search_matches("plain text", "  ", "Work", "friend"));
    }
    #[test]
    fn borrowed_timeline_preserves_sources_without_copying_payloads() {
        let make = |chat: &str, time| Message {
            id: "same".into(),
            chat: chat.into(),
            sender: "x".into(),
            sender_name: None,
            from_me: false,
            timestamp: time,
            history_order: None,
            content: crate::model::Content::text("x".repeat(1024 * 1024)),
            status: crate::model::Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: vec![],
            edited: false,
            mentions: vec![],
            forwarded: false,
            thumbnail: None,
        };
        let a = make("work", 2);
        let b = make("signal", 1);
        let refs = ordered_refs(vec![&a, &b]);
        assert!(std::ptr::eq(refs[0], &b));
        assert!(std::ptr::eq(refs[1], &a));
    }
    #[test]
    fn same_id_in_distinct_chats_is_not_deduplicated() {
        fn msg(chat: &str, time: i64) -> Message {
            Message {
                id: "collision".into(),
                chat: chat.into(),
                sender: "x".into(),
                sender_name: None,
                from_me: false,
                timestamp: time,
                history_order: None,
                content: crate::model::Content::text(chat),
                status: crate::model::Delivery::Sent,
                delivered_at: None,
                read_at: None,
                quoted: None,
                reactions: vec![],
                edited: false,
                mentions: vec![],
                forwarded: false,
                thumbnail: None,
            }
        }
        let rows = ordered(vec![msg("b", 2), msg("a", 1), msg("b", 2)]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].chat, "a");
        assert_eq!(rows[1].chat, "b");
    }
    #[test]
    fn primary_never_falls_back() {
        let first = Member {
            origin: "server".into(),
            account: "work".into(),
            chat: "one".into(),
        };
        let second = Member {
            origin: "server".into(),
            account: "personal".into(),
            chat: "two".into(),
        };
        let mut person = Person {
            name: "A".into(),
            members: vec![first.clone(), second.clone()],
            primary: None,
        };
        assert_eq!(
            person
                .default_destination(&[first.clone(), second.clone()])
                .unwrap(),
            None
        );
        person.primary = Some(first.clone());
        assert_eq!(
            person
                .default_destination(std::slice::from_ref(&first))
                .unwrap(),
            Some(&first)
        );
        assert!(person.default_destination(&[second]).is_err());
    }
    #[test]
    fn crossover_never_quotes() {
        let a = Member {
            origin: "server".into(),
            account: "work".into(),
            chat: "one".into(),
        };
        let mut b = a.clone();
        b.account = "personal".into();
        assert!(reply_route(&a, &b, "same-id").is_none());
        b = a.clone();
        b.chat = "two".into();
        assert!(reply_route(&a, &b, "same-id").is_none());
        assert_eq!(reply_route(&a, &a, "same-id").unwrap().id, "same-id");
    }
    #[test]
    fn exact_origin_and_account_are_required() {
        let mut chat = Chat::new("same".into(), "A".into());
        chat.backend_origin = Some("one".into());
        chat.backend_account = Some("work".into());
        let member = Member::of(&chat).unwrap();
        chat.backend_origin = Some("two".into());
        assert!(!member.matches(&chat));
        chat.backend_origin = Some("one".into());
        chat.backend_account = Some("personal".into());
        assert!(!member.matches(&chat));
    }
}

#[cfg(test)]
mod catch_up_tests {
    use super::*;

    /// A bridge with `history` messages older than what is loaded, paged
    /// `page` at a time; `offline` bridges accept no request at all.
    struct Bridge {
        chat: &'static str,
        loaded_from: i64,
        remaining: Vec<i64>,
        page: usize,
        offline: bool,
        pending: bool,
    }

    impl Bridge {
        fn new(chat: &'static str, loaded_from: i64, remaining: Vec<i64>, page: usize) -> Self {
            Self {
                chat,
                loaded_from,
                remaining,
                page,
                offline: false,
                pending: false,
            }
        }

        fn paging(&self) -> Paging {
            Paging {
                chat: self.chat.to_owned(),
                oldest: Some(self.loaded_from),
                exhausted: self.remaining.is_empty() && !self.offline,
                busy: self.pending,
            }
        }

        /// A request arrives; it is answered on the next tick.
        fn request(&mut self) -> bool {
            if self.offline || self.pending {
                return false;
            }
            self.pending = true;
            true
        }

        fn tick(&mut self) {
            if !self.pending {
                return;
            }
            self.pending = false;
            for _ in 0..self.page {
                if let Some(next) = self.remaining.pop() {
                    self.loaded_from = next;
                }
            }
        }
    }

    /// The UI driver's loop, against fake bridges.
    fn run(bridges: &mut [Bridge], target: i64) -> CatchUp {
        let mut catch_up = CatchUp::new(target);
        for _ in 0..200 {
            let paging: Vec<_> = bridges.iter().map(Bridge::paging).collect();
            let (requests, waiting) = catch_up.plan(&paging);
            for chat in &requests {
                let bridge = bridges.iter_mut().find(|b| b.chat == *chat).unwrap();
                let oldest = bridge.loaded_from;
                if bridge.request() {
                    catch_up.asked(chat, oldest);
                } else {
                    catch_up.unavailable(chat);
                }
            }
            if requests.is_empty() && !waiting {
                break;
            }
            bridges.iter_mut().for_each(Bridge::tick);
        }
        catch_up
    }

    #[test]
    fn a_busy_bridge_pages_until_it_reaches_the_quiet_ones_start() {
        // Quiet chat loaded back to 100; the busy one only back to 900.
        let busy: Vec<i64> = (0..900).rev().collect();
        let mut bridges = [
            Bridge::new("quiet", 100, vec![], 10),
            Bridge::new("busy", 900, busy, 50),
        ];
        let catch_up = run(&mut bridges, 100);
        let reached = bridges[1].loaded_from;
        assert!(reached <= 100, "{reached}");
        assert_eq!(catch_up.stopped().count(), 0);
    }

    #[test]
    fn an_offline_bridge_is_reported_not_retried() {
        let mut bridges = [
            Bridge::new("quiet", 100, vec![], 10),
            Bridge::new("busy", 900, (0..900).rev().collect(), 50),
        ];
        bridges[1].offline = true;
        let catch_up = run(&mut bridges, 100);
        assert_eq!(bridges[1].loaded_from, 900);
        let stopped: Vec<_> = catch_up.stopped().collect();
        assert_eq!(stopped, [("busy", Stopped::NotNow)]);
    }

    #[test]
    fn a_page_that_brings_nothing_older_stops_the_chat() {
        // Pages arrive empty, as a phone that answers with nothing does.
        let mut bridges = [
            Bridge::new("quiet", 100, vec![], 10),
            Bridge::new("stuck", 900, vec![], 50),
        ];
        // Not exhausted by its own flag: the phone just keeps answering empty.
        let mut catch_up = CatchUp::new(100);
        for _ in 0..5 {
            let mut paging: Vec<_> = bridges.iter().map(Bridge::paging).collect();
            paging[1].exhausted = false;
            let (requests, _) = catch_up.plan(&paging);
            for chat in requests {
                let oldest = bridges[1].loaded_from;
                assert!(bridges[1].request());
                catch_up.asked(&chat, oldest);
            }
            bridges[1].tick();
        }
        let stopped: Vec<_> = catch_up.stopped().collect();
        assert_eq!(stopped, [("stuck", Stopped::NoProgress)]);
    }

    #[test]
    fn exhausted_and_already_reaching_chats_are_left_alone() {
        let bridges = [
            Bridge::new("quiet", 100, vec![], 10),
            Bridge::new("done", 500, vec![], 10),
        ];
        let paging: Vec<_> = bridges.iter().map(Bridge::paging).collect();
        let mut catch_up = CatchUp::new(100);
        assert_eq!(catch_up.plan(&paging), (vec![], false));
    }
}
