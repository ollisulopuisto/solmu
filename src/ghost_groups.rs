//! Ghost groups: short-lived, local-only recipient groups.
//!
//! A ghost group is a list of chats the user picked in the sidebar, kept on this
//! computer only. It is not a chat, not a bridge label and not visible to anyone
//! in it. A message "to" a ghost group is one ordinary message per member chat,
//! sent separately. A group disbands in one click and expires by itself after a
//! stretch of disuse, counted from the last time it was used.
use serde::{Deserialize, Serialize};

pub use crate::flat_groups::Member;

/// A group expires this long after its last use unless told otherwise.
pub const DEFAULT_EXPIRY_SECS: i64 = 7 * 24 * 60 * 60;
pub const MIN_MEMBERS: usize = 2;
pub const MAX_MEMBERS: usize = 50;
pub const MAX_GROUPS: usize = 20;
pub const MAX_NAME_CHARS: usize = 60;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ghost {
    pub id: u64,
    pub name: String,
    pub members: Vec<Member>,
    pub created_at: i64,
    /// Creation counts as the first use.
    pub last_used: i64,
    pub expires_after_secs: i64,
}

impl Ghost {
    pub fn expires_at(&self) -> i64 {
        self.last_used.saturating_add(self.expires_after_secs)
    }
    pub fn expired(&self, now: i64) -> bool {
        now >= self.expires_at()
    }
    /// Whole days left, rounded up, never below zero.
    pub fn days_left(&self, now: i64) -> i64 {
        (self.expires_at().saturating_sub(now).max(0) + 86_399) / 86_400
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ghosts {
    pub items: Vec<Ghost>,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    TooFewMembers,
    TooManyMembers,
    TooManyGroups,
    BadName,
}

impl Ghosts {
    /// Creates a group from exact identities. Duplicates are dropped, order kept.
    /// An empty name becomes "Group N" so a quick group needs no typing.
    pub fn create(&mut self, name: &str, members: Vec<Member>, now: i64) -> Result<u64, Refusal> {
        let mut unique: Vec<Member> = Vec::with_capacity(members.len());
        for member in members {
            if !unique.contains(&member) {
                unique.push(member);
            }
        }
        if unique.len() < MIN_MEMBERS {
            return Err(Refusal::TooFewMembers);
        }
        if unique.len() > MAX_MEMBERS {
            return Err(Refusal::TooManyMembers);
        }
        if self.items.len() >= MAX_GROUPS {
            return Err(Refusal::TooManyGroups);
        }
        let name = name.trim();
        if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
            return Err(Refusal::BadName);
        }
        let id = self
            .next_id
            .max(self.items.iter().map(|g| g.id).max().unwrap_or(0))
            .checked_add(1)
            .ok_or(Refusal::TooManyGroups)?;
        self.next_id = id;
        let name = if name.is_empty() {
            format!("Group {id}")
        } else {
            name.to_owned()
        };
        self.items.push(Ghost {
            id,
            name,
            members: unique,
            created_at: now,
            last_used: now,
            expires_after_secs: DEFAULT_EXPIRY_SECS,
        });
        Ok(id)
    }

    pub fn get(&self, id: u64) -> Option<&Ghost> {
        self.items.iter().find(|g| g.id == id)
    }

    /// Restarts the expiry clock. Returns false for an unknown or expired group.
    pub fn touch(&mut self, id: u64, now: i64) -> bool {
        match self
            .items
            .iter_mut()
            .find(|g| g.id == id && !g.expired(now))
        {
            Some(group) => {
                group.last_used = group.last_used.max(now);
                true
            }
            None => false,
        }
    }

    /// One click: the group is gone, the chats and their messages are untouched.
    pub fn disband(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|g| g.id != id);
        self.items.len() != before
    }

    /// Drops every expired group and returns how many went.
    pub fn prune(&mut self, now: i64) -> usize {
        let before = self.items.len();
        self.items.retain(|g| !g.expired(now));
        before - self.items.len()
    }

    pub fn any_expired(&self, now: i64) -> bool {
        self.items.iter().any(|g| g.expired(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn member(chat: &str) -> Member {
        Member {
            account: "acct".into(),
            origin: None,
            bridge_account: None,
            chat: chat.into(),
        }
    }

    fn three() -> Vec<Member> {
        vec![member("a"), member("b"), member("c")]
    }

    #[test]
    fn creation_dedupes_names_the_group_and_counts_as_first_use() {
        let mut ghosts = Ghosts::default();
        let id = ghosts
            .create("", vec![member("a"), member("b"), member("a")], 1_000)
            .unwrap();
        let group = ghosts.get(id).unwrap();
        assert_eq!(group.members.len(), 2);
        assert_eq!(group.name, format!("Group {id}"));
        assert_eq!(group.last_used, 1_000);
        assert_eq!(group.expires_after_secs, DEFAULT_EXPIRY_SECS);
    }

    #[test]
    fn size_and_name_limits_refuse_instead_of_trimming() {
        let mut ghosts = Ghosts::default();
        assert_eq!(
            ghosts.create("x", vec![member("a")], 0),
            Err(Refusal::TooFewMembers)
        );
        let many: Vec<_> = (0..=MAX_MEMBERS).map(|n| member(&n.to_string())).collect();
        assert_eq!(ghosts.create("x", many, 0), Err(Refusal::TooManyMembers));
        assert_eq!(
            ghosts.create(&"n".repeat(61), three(), 0),
            Err(Refusal::BadName)
        );
        assert_eq!(ghosts.create("a\nb", three(), 0), Err(Refusal::BadName));
        for n in 0..MAX_GROUPS {
            ghosts.create(&format!("g{n}"), three(), 0).unwrap();
        }
        assert_eq!(
            ghosts.create("one more", three(), 0),
            Err(Refusal::TooManyGroups)
        );
    }

    #[test]
    fn expiry_counts_from_last_use_not_from_creation() {
        let mut ghosts = Ghosts::default();
        let id = ghosts.create("Guests", three(), 0).unwrap();
        // Used on day 6: the 7-day clock restarts, so day 8 is still alive.
        assert!(ghosts.touch(id, 6 * DAY));
        assert_eq!(ghosts.prune(8 * DAY), 0);
        assert_eq!(ghosts.get(id).unwrap().days_left(8 * DAY), 5);
        // Day 13 is exactly 7 days after the last use.
        assert!(ghosts.get(id).unwrap().expired(13 * DAY));
        assert!(!ghosts.get(id).unwrap().expired(13 * DAY - 1));
        assert_eq!(ghosts.prune(13 * DAY), 1);
        assert!(ghosts.items.is_empty());
    }

    #[test]
    fn an_expired_group_cannot_be_revived_by_a_late_touch() {
        let mut ghosts = Ghosts::default();
        let id = ghosts.create("Guests", three(), 0).unwrap();
        assert!(!ghosts.touch(id, 8 * DAY));
        assert!(ghosts.any_expired(8 * DAY));
    }

    #[test]
    fn touch_never_moves_the_clock_backwards() {
        let mut ghosts = Ghosts::default();
        let id = ghosts.create("Guests", three(), 100).unwrap();
        assert!(ghosts.touch(id, 50));
        assert_eq!(ghosts.get(id).unwrap().last_used, 100);
    }

    #[test]
    fn disband_removes_only_that_group_and_ids_are_not_reused() {
        let mut ghosts = Ghosts::default();
        let first = ghosts.create("One", three(), 0).unwrap();
        let second = ghosts.create("Two", three(), 0).unwrap();
        assert!(ghosts.disband(first));
        assert!(!ghosts.disband(first));
        assert_eq!(ghosts.items.len(), 1);
        let third = ghosts.create("Three", three(), 0).unwrap();
        assert!(third > second);
    }

    #[test]
    fn days_left_rounds_up_and_stops_at_zero() {
        let mut ghosts = Ghosts::default();
        let id = ghosts.create("Guests", three(), 0).unwrap();
        let group = ghosts.get(id).unwrap();
        assert_eq!(group.days_left(0), 7);
        assert_eq!(group.days_left(DAY + 1), 6);
        assert_eq!(group.days_left(30 * DAY), 0);
    }

    #[test]
    fn groups_survive_a_settings_round_trip() {
        let mut ghosts = Ghosts::default();
        ghosts.create("Guests", three(), 5).unwrap();
        let json = serde_json::to_string(&ghosts).unwrap();
        assert_eq!(serde_json::from_str::<Ghosts>(&json).unwrap(), ghosts);
        assert_eq!(
            serde_json::from_str::<Ghosts>("{}").unwrap(),
            Ghosts::default()
        );
    }
}
