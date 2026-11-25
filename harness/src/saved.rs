//! Local per-message bookmarks ("Save for later").
//!
//! A saved entry is an identity - server, account, chat and message - plus
//! a display copy taken when the message was saved, so the Saved list can
//! show and search it even when the original chat is not loaded or the
//! message is gone. Everything is local: nothing is starred or pinned on
//! the account, and removing a bookmark never deletes the message. This
//! module is the pure data and matching logic; the panel and the mark live
//! in [`crate::ui`].

use crate::merged::Member;
use crate::model::Message;

/// One saved message: who it is, and what it said when it was saved.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedMessage {
    /// Server, account and chat the message belongs to. Never the text:
    /// the same words in another chat are a different entry.
    #[serde(flatten)]
    pub member: Member,
    /// The message id inside that chat.
    pub message: String,
    /// The chat's title when the message was saved.
    #[serde(default)]
    pub chat_title: String,
    /// The sender's display name when the message was saved.
    #[serde(default)]
    pub sender: String,
    /// The message's plain-text summary when it was saved.
    #[serde(default)]
    pub preview: String,
    /// The message's own timestamp.
    #[serde(default)]
    pub timestamp: i64,
    /// Whether the reader sent the message, for the bubble's side.
    #[serde(default)]
    pub from_me: bool,
}

impl SavedMessage {
    /// Saves the message under the chat's member identity, remembering how
    /// it read at the time.
    pub fn new(
        member: Member,
        message: &Message,
        chat_title: String,
        sender: String,
        preview: String,
    ) -> Self {
        Self {
            member,
            message: message.id.clone(),
            chat_title,
            sender,
            preview,
            timestamp: message.timestamp,
            from_me: message.from_me,
        }
    }

    /// Whether this entry is that message in that chat.
    pub fn is(&self, member: &Member, message: &str) -> bool {
        self.member == *member && self.message == message
    }

    /// Case-insensitive substring match over the saved text, sender and
    /// chat title. An empty query matches everything.
    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        [&self.preview, &self.sender, &self.chat_title]
            .iter()
            .any(|haystack| haystack.to_lowercase().contains(&query))
    }
}

/// Whether the list holds this message in this chat.
pub fn contains(list: &[SavedMessage], member: &Member, message: &str) -> bool {
    list.iter().any(|saved| saved.is(member, message))
}

/// Adds the entry unless the message is already saved. Returns whether it
/// was added.
pub fn add(list: &mut Vec<SavedMessage>, entry: SavedMessage) -> bool {
    if contains(list, &entry.member, &entry.message) {
        return false;
    }
    list.push(entry);
    true
}

/// Removes this message's bookmark. The message itself is untouched.
pub fn remove(list: &mut Vec<SavedMessage>, member: &Member, message: &str) -> bool {
    let before = list.len();
    list.retain(|saved| !saved.is(member, message));
    list.len() != before
}

/// Ids of the saved messages in one chat, for the mark in the conversation.
pub fn ids_in<'a>(list: &'a [SavedMessage], member: &Member) -> std::collections::HashSet<&'a str> {
    list.iter()
        .filter(|saved| saved.member == *member)
        .map(|saved| saved.message.as_str())
        .collect()
}

/// The entries matching the query, newest saved first.
pub fn filtered<'a>(list: &'a [SavedMessage], query: &str) -> Vec<&'a SavedMessage> {
    list.iter()
        .rev()
        .filter(|entry| entry.matches_query(query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(chat: &str) -> Member {
        Member {
            origin: "local".to_owned(),
            account: "account".to_owned(),
            chat: chat.to_owned(),
        }
    }

    fn msg(id: &str, chat: &str) -> Message {
        Message {
            id: id.to_owned(),
            chat: chat.to_owned(),
            sender: "ada".to_owned(),
            sender_name: None,
            from_me: false,
            timestamp: 1_700_000_000,
            history_order: None,
            content: crate::model::Content::Text {
                text: "hello".to_owned(),
                preview: None,
            },
            status: crate::model::Delivery::Read,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    fn entry(chat: &str, message: &Message, preview: &str) -> SavedMessage {
        SavedMessage::new(
            member(chat),
            message,
            format!("Chat {chat}"),
            "Ada".to_owned(),
            preview.to_owned(),
        )
    }

    #[test]
    fn adding_twice_keeps_one_entry() {
        let mut list = Vec::new();
        let message = msg("m1", "c1");
        assert!(add(&mut list, entry("c1", &message, "hello")));
        assert!(!add(&mut list, entry("c1", &message, "hello")));
        assert_eq!(list.len(), 1);
        assert!(contains(&list, &member("c1"), "m1"));
    }

    #[test]
    fn identity_is_member_and_message_never_the_text() {
        let mut list = Vec::new();
        let message = msg("m1", "c1");
        assert!(add(&mut list, entry("c1", &message, "hello")));
        // The same id in another chat, and the same words under another id,
        // are not this entry.
        assert!(!contains(&list, &member("c2"), "m1"));
        assert!(!contains(&list, &member("c1"), "m2"));
        let other = msg("m2", "c1");
        assert!(add(&mut list, entry("c1", &other, "hello")));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn removing_unmarks_without_touching_the_rest() {
        let mut list = Vec::new();
        let first = msg("m1", "c1");
        let second = msg("m2", "c1");
        add(&mut list, entry("c1", &first, "one"));
        add(&mut list, entry("c1", &second, "two"));
        assert!(remove(&mut list, &member("c1"), "m1"));
        assert!(!contains(&list, &member("c1"), "m1"));
        assert!(contains(&list, &member("c1"), "m2"));
        assert!(!remove(&mut list, &member("c1"), "m1"));
    }

    #[test]
    fn the_query_matches_text_sender_and_chat() {
        let list = vec![entry("c1", &msg("m1", "c1"), "Lunch tomorrow?")];
        assert!(list[0].matches_query("lunch"));
        assert!(list[0].matches_query("ADA"));
        assert!(list[0].matches_query("chat c1"));
        assert!(list[0].matches_query("  "));
        assert!(!list[0].matches_query("dinner"));
    }

    #[test]
    fn ids_in_scopes_to_the_chat_and_filtered_is_newest_first() {
        let mut list = Vec::new();
        let one = msg("m1", "c1");
        let two = msg("m2", "c1");
        let three = msg("m3", "c2");
        add(&mut list, entry("c1", &one, "one"));
        add(&mut list, entry("c1", &two, "two"));
        add(&mut list, entry("c2", &three, "three"));
        let ids = ids_in(&list, &member("c1"));
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("m1") && ids.contains("m2"));
        let found = filtered(&list, "");
        assert_eq!(found[0].message, "m3");
        assert_eq!(found[2].message, "m1");
        assert_eq!(filtered(&list, "two").len(), 1);
    }
}
