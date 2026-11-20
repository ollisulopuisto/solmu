//! Stable per-recipient summaries for the forward picker and retained results.
use crate::composed_forward::{Item, State};
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recipient {
    pub origin: String,
    pub account: String,
    pub chat: String,
    pub total: usize,
    pub waiting: usize,
    pub sending: usize,
    pub accepted: usize,
    pub confirmed: usize,
    pub unsent: usize,
    pub unknown: usize,
    pub indices: Vec<usize>,
}
pub fn recipients(items: &[Item]) -> Vec<Recipient> {
    let mut groups: Vec<Recipient> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let at = groups
            .iter()
            .position(|row| {
                row.origin == item.origin
                    && row.account == item.destination_account
                    && row.chat == item.destination
            })
            .unwrap_or_else(|| {
                groups.push(Recipient {
                    origin: item.origin.clone(),
                    account: item.destination_account.clone(),
                    chat: item.destination.clone(),
                    ..Default::default()
                });
                groups.len() - 1
            });
        let group = &mut groups[at];
        group.total += 1;
        group.indices.push(index);
        match item.state {
            State::Waiting => group.waiting += 1,
            State::Sending => group.sending += 1,
            State::Accepted { .. } => group.accepted += 1,
            State::Confirmed { .. } => group.confirmed += 1,
            State::FailedBeforeSend(_) => group.unsent += 1,
            State::Uncertain(_) => group.unknown += 1,
        }
    }
    groups
}

pub fn retry_recipient(
    items: &mut [Item],
    origin: &str,
    account: &str,
    chat: &str,
) -> Vec<(usize, Item)> {
    items
        .iter_mut()
        .enumerate()
        .filter_map(|(index, item)| {
            if item.origin != origin
                || item.destination_account != account
                || item.destination != chat
                || !matches!(item.state, State::FailedBeforeSend(_))
            {
                return None;
            }
            item.state = State::Waiting;
            Some((index, item.clone()))
        })
        .collect()
}

/// Feedback is scoped by full destination identity and its exact accepted token.
/// Failure and timeout stay uncertain, never pre-submit/retryable.
pub fn feedback(
    items: &mut [Item],
    origin: &str,
    account: &str,
    chat: &str,
    pending: &str,
    state: State,
) {
    for item in items {
        if item.origin == origin
            && item.destination_account == account
            && item.destination == chat
            && matches!(&item.state,State::Accepted{pending_id} if pending_id==pending)
        {
            item.state = state.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(account: &str, chat: &str, state: State) -> Item {
        Item {
            origin: "server".into(),
            planned: true,
            source_chat: "source".into(),
            source_message: "m".into(),
            source_account: "source-account".into(),
            destination_account: account.into(),
            attachment: None,
            destination: chat.into(),
            text: "text".into(),
            state,
        }
    }
    #[test]
    fn summary_keeps_accounts_separate_and_retries_only_one_known_unsent_recipient() {
        let mut items = vec![
            item(
                "a",
                "same",
                State::Confirmed {
                    message_id: "final".into(),
                },
            ),
            item("a", "same", State::FailedBeforeSend("download".into())),
            item("b", "same", State::FailedBeforeSend("download".into())),
            item("a", "unknown", State::Uncertain("timeout".into())),
        ];
        let groups = recipients(&items);
        assert_eq!(groups.len(), 3);
        assert_eq!(
            (groups[0].confirmed, groups[0].unsent, groups[0].total),
            (1, 1, 2)
        );
        let retry = retry_recipient(&mut items, "server", "a", "same");
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].0, 1);
        assert!(matches!(items[0].state, State::Confirmed { .. }));
        assert!(matches!(items[2].state, State::FailedBeforeSend(_)));
        assert!(matches!(items[3].state, State::Uncertain(_)));
    }
    #[test]
    fn feedback_never_confirms_failures_or_crosses_account_origin_or_pending_token() {
        let mut items = vec![
            item(
                "a",
                "chat",
                State::Accepted {
                    pending_id: "p".into(),
                },
            ),
            item(
                "b",
                "chat",
                State::Accepted {
                    pending_id: "p".into(),
                },
            ),
            item(
                "a",
                "chat",
                State::Accepted {
                    pending_id: "q".into(),
                },
            ),
        ];
        feedback(
            &mut items,
            "other",
            "a",
            "chat",
            "p",
            State::Confirmed {
                message_id: "final".into(),
            },
        );
        assert!(
            items
                .iter()
                .all(|i| matches!(i.state, State::Accepted { .. }))
        );
        feedback(
            &mut items,
            "server",
            "a",
            "chat",
            "p",
            State::Uncertain("timeout".into()),
        );
        assert!(matches!(items[0].state, State::Uncertain(_)));
        assert!(matches!(items[1].state, State::Accepted { .. }));
        assert!(matches!(items[2].state, State::Accepted { .. }));
        assert!(retry_recipient(&mut items, "server", "a", "chat").is_empty());
        feedback(
            &mut items,
            "server",
            "a",
            "chat",
            "q",
            State::Confirmed {
                message_id: "final".into(),
            },
        );
        assert!(matches!(items[2].state, State::Confirmed { .. }));
    }
}
