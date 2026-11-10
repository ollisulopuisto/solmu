//! Automatic chat categories: People, Quiet people and Notifications.
//!
//! The rules are plain, deterministic and computed on this computer from what
//! the chat list already holds. Nothing is sent anywhere and no model is
//! involved. A category the user picked for a chat always wins over the
//! rules and is never re-sorted.

use crate::model::{Chat, ChatKind};
use serde::{Deserialize, Serialize};

/// A direct chat with no activity for this long counts as quiet.
pub const QUIET_AFTER_SECS: i64 = 60 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoCategory {
    /// One-to-one chats you are in touch with.
    People,
    /// One-to-one chats with nothing new for a long time.
    Quiet,
    /// Channels and read-only chats: senders that only write to you.
    Notifications,
}

impl AutoCategory {
    pub const EVERY: [Self; 3] = [Self::People, Self::Quiet, Self::Notifications];

    pub fn label(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        use crate::i18n::gettext;
        match self {
            Self::People => gettext(locale, "People"),
            Self::Quiet => gettext(locale, "Quiet people"),
            Self::Notifications => gettext(locale, "Notifications"),
        }
    }
}

/// The category the rules give a chat, or `None` for a group, which stays in
/// Groups. `now` is a Unix timestamp in seconds.
pub fn automatic(chat: &Chat, now: i64) -> Option<AutoCategory> {
    if chat.kind == ChatKind::Group {
        return None;
    }
    if chat.kind == ChatKind::Broadcast || chat.is_channel() || chat.read_only {
        return Some(AutoCategory::Notifications);
    }
    if chat.last_activity > 0 && now.saturating_sub(chat.last_activity) >= QUIET_AFTER_SECS {
        return Some(AutoCategory::Quiet);
    }
    Some(AutoCategory::People)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn chat(id: &str) -> Chat {
        let mut chat = Chat::new(id.into(), id.into());
        chat.last_activity = NOW - 60;
        chat
    }

    #[test]
    fn recent_direct_chats_are_people() {
        assert_eq!(
            automatic(&chat("a@s.whatsapp.net"), NOW),
            Some(AutoCategory::People)
        );
    }

    #[test]
    fn a_chat_with_no_recorded_activity_is_not_called_quiet() {
        let mut empty = chat("a@s.whatsapp.net");
        empty.last_activity = 0;
        assert_eq!(automatic(&empty, NOW), Some(AutoCategory::People));
    }

    #[test]
    fn quiet_starts_exactly_at_the_threshold() {
        let mut edge = chat("a@s.whatsapp.net");
        edge.last_activity = NOW - QUIET_AFTER_SECS + 1;
        assert_eq!(automatic(&edge, NOW), Some(AutoCategory::People));
        edge.last_activity = NOW - QUIET_AFTER_SECS;
        assert_eq!(automatic(&edge, NOW), Some(AutoCategory::Quiet));
    }

    #[test]
    fn channels_and_read_only_chats_are_notifications() {
        let mut channel = chat("news@newsletter");
        channel.kind = ChatKind::Broadcast;
        assert_eq!(automatic(&channel, NOW), Some(AutoCategory::Notifications));
        let mut announce = chat("bank@s.whatsapp.net");
        announce.read_only = true;
        assert_eq!(automatic(&announce, NOW), Some(AutoCategory::Notifications));
        // A quiet channel is still a channel.
        channel.last_activity = 1;
        assert_eq!(automatic(&channel, NOW), Some(AutoCategory::Notifications));
    }

    #[test]
    fn groups_stay_in_groups() {
        let mut group = chat("g@g.us");
        group.kind = ChatKind::Group;
        assert_eq!(automatic(&group, NOW), None);
    }
}
