//! Per-item state for composed forwards. Acceptance is not delivery, and an
//! unknown send outcome is never safe for bulk retry.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Sending,
    Accepted { pending_id: String },
    FailedBeforeSend(String),
    Uncertain(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub origin: String,
    pub planned: bool,
    pub source_chat: String,
    pub source_message: String,
    pub source_account: String,
    pub destination_account: String,
    pub attachment: Option<usize>,
    pub destination: String,
    pub text: String,
    pub state: State,
}

/// Reset only failures known to have happened before a message POST.
pub fn retry_failed(items: &mut [Item]) {
    for item in items {
        if matches!(item.state, State::FailedBeforeSend(_)) {
            item.state = State::Waiting;
        }
    }
}

/// Header fields are metadata, not new lines, mentions or Markdown directives.
fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// API sends interpret Markdown. Escape literal source text rather than
/// accidentally pinging a destination's unrelated @mention identities.
pub fn literal_text(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '*' | '_' | '~' | '`' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            result.push('\\');
        }
        result.push(ch);
        // The send endpoint has no explicit empty-mentions override. Break
        // automatic @room/user scanning while preserving the visible text.
        if ch == '@' {
            result.push('\u{200b}');
        }
    }
    result
}

pub fn compose(sender: &str, network: &str, when: &str, group: Option<&str>, body: &str) -> String {
    let sender = one_line(sender);
    let network = one_line(network);
    let when = one_line(when);
    let group = group.map(one_line).filter(|name| !name.is_empty());
    let header = match group {
        Some(group) => format!("Forwarded from {sender} ({network}), {when}, in {group}:"),
        None => format!("Forwarded from {sender} ({network}), {when}:"),
    };
    if body.is_empty() {
        literal_text(&header)
    } else {
        format!("{}\n{}", literal_text(&header), literal_text(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(state: State) -> Item {
        Item {
            origin: "origin".into(),
            planned: true,
            source_chat: "source".into(),
            source_message: "same-id".into(),
            source_account: "src-account".into(),
            destination_account: "dst-account".into(),
            attachment: None,
            destination: "destination".into(),
            text: "forward".into(),
            state,
        }
    }
    #[test]
    fn header_has_sender_network_date_and_no_group_by_default() {
        assert_eq!(
            compose("Ada", "Signal", "11.11.2026 10:00 EET", None, "hello"),
            "Forwarded from Ada (Signal), 11.11.2026 10:00 EET:\nhello"
        );
        assert!(!compose("Ada", "Signal", "date", None, "hello").contains("Secret team"));
        assert!(
            compose("Ada", "Signal", "date", Some("Secret team"), "hello")
                .contains("in Secret team:")
        );
    }
    #[test]
    fn metadata_is_single_line_and_source_markup_is_literal() {
        assert_eq!(
            compose("Ada\nEve", "Signal", "date", None, "<script>*hello* _yes_"),
            "Forwarded from Ada Eve (Signal), date:\n\\<script\\>\\*hello\\* \\_yes\\_"
        );
    }
    #[test]
    fn source_mentions_are_visible_but_not_destination_mentions() {
        let escaped = literal_text("hello @room and @Ada");
        assert_eq!(escaped, "hello @\u{200b}room and @\u{200b}Ada");
        assert!(!escaped.contains("@room"));
        assert_eq!(escaped.replace('\u{200b}', ""), "hello @room and @Ada");
    }
    #[test]
    fn bulk_retry_never_repeats_accepted_sending_or_uncertain_items() {
        let states = vec![
            State::Accepted {
                pending_id: "p".into(),
            },
            State::Uncertain("timeout".into()),
            State::Sending,
            State::FailedBeforeSend("download".into()),
            State::Waiting,
        ];
        let mut items: Vec<_> = states.clone().into_iter().map(item).collect();
        retry_failed(&mut items);
        for index in 0..3 {
            assert_eq!(items[index].state, states[index]);
        }
        assert_eq!(items[3].state, State::Waiting);
        assert_eq!(items[4].state, State::Waiting);
    }
}

/// Exact local provenance and restart-safe pending resolution identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardedMessage {
    pub member: crate::merged::Member,
    pub message: String,
    #[serde(default = "pending_by_default")]
    pub pending: bool,
}
fn pending_by_default() -> bool {
    true
}

pub fn unresolved_marks(path: &std::path::Path, origin: &str) -> Vec<ForwardedMessage> {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 8 * 1024 * 1024
    {
        return Vec::new();
    }
    let Ok(settings) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    settings
        .pointer("/beeper/forwarded_messages")
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<ForwardedMessage>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|m| {
            m.pending
                && m.member.origin == origin
                && !m.member.account.is_empty()
                && !m.member.chat.is_empty()
                && !m.message.is_empty()
        })
        .collect()
}
