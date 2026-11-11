//! Local list groups, never bridge labels or remote folders.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Exact local account and bridge identity. Chat ids alone are not identities.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Member {
    pub account: String,
    pub origin: Option<String>,
    pub bridge_account: Option<String>,
    pub chat: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Groups {
    pub items: Vec<Group>,
    pub grouped_view: bool,
    next_id: u64,
}

impl Groups {
    pub fn create(&mut self, name: &str) -> Option<u64> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > 80
            || self.items.len() >= 100
            || self.items.iter().any(|g| g.name.eq_ignore_ascii_case(name))
        {
            return None;
        }
        self.next_id = self
            .next_id
            .max(self.items.iter().map(|g| g.id).max().unwrap_or(0))
            .checked_add(1)?;
        let id = self.next_id;
        self.items.push(Group {
            id,
            name: name.into(),
            members: Vec::new(),
        });
        Some(id)
    }

    /// Add only. Existing membership in other groups is never moved.
    pub fn add(&mut self, id: u64, member: Member) -> bool {
        let Some(group) = self.items.iter_mut().find(|g| g.id == id) else {
            return false;
        };
        if group.members.contains(&member) {
            return false;
        }
        group.members.push(member);
        true
    }

    /// Preserve source assignments when an unchanged linked row is applied.
    pub fn assign_linked(&mut self, target: &Member, sources: &[Member], selected: &[u64]) {
        for group in &mut self.items {
            if selected.contains(&group.id) {
                if !group.members.iter().any(|member| sources.contains(member)) {
                    group.members.push(target.clone());
                }
            } else {
                group.members.retain(|member| !sources.contains(member));
            }
        }
    }

    pub fn remove(&mut self, id: u64, member: &Member) {
        if let Some(group) = self.items.iter_mut().find(|g| g.id == id) {
            group.members.retain(|m| m != member);
        }
    }

    /// Only group metadata is removed. There is no chat/backend argument.
    pub fn delete(&mut self, id: u64) {
        self.items.retain(|g| g.id != id);
    }

    pub fn unread_total<'a>(&self, unread: impl IntoIterator<Item = &'a Member>) -> usize {
        unread.into_iter().collect::<HashSet<_>>().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member(account: &str) -> Member {
        Member {
            account: account.into(),
            origin: Some("server".into()),
            bridge_account: Some("signal".into()),
            chat: "same-id".into(),
        }
    }
    #[test]
    fn many_groups_do_not_copy_or_move_chats() {
        let mut groups = Groups::default();
        let a = groups.create("Work").unwrap();
        let b = groups.create("Family").unwrap();
        let chat = member("one");
        assert!(groups.add(a, chat.clone()));
        assert!(!groups.add(a, chat.clone()));
        assert!(groups.add(b, chat.clone()));
        assert_eq!(groups.unread_total([&chat, &chat]), 1);
        groups.delete(a);
        assert_eq!(groups.items[0].members, vec![chat]);
    }
    #[test]
    fn checked_linked_assignment_preserves_original_members_after_unlinking() {
        let mut groups = Groups::default();
        let id = groups.create("Work").unwrap();
        let a = member("one");
        let b = member("two");
        groups.add(id, a.clone());
        groups.assign_linked(&b, &[a.clone(), b.clone()], &[id]);
        assert_eq!(groups.items[0].members, vec![a.clone()]);
        groups.assign_linked(&b, &[a.clone(), b.clone()], &[]);
        assert!(groups.items[0].members.is_empty());
        groups.assign_linked(&b, &[a.clone(), b.clone()], &[id]);
        assert_eq!(groups.items[0].members, vec![b]);
    }

    #[test]
    fn identities_and_roundtrip_are_exact() {
        let mut groups = Groups::default();
        let id = groups.create("Work").unwrap();
        groups.add(id, member("one"));
        groups.add(id, member("two"));
        let restored: Groups =
            serde_json::from_str(&serde_json::to_string(&groups).unwrap()).unwrap();
        assert_eq!(groups, restored);
        assert_eq!(restored.items[0].members.len(), 2);
        assert!(groups.create("work").is_none());
        assert!(groups.create(" ").is_none());
    }
}
