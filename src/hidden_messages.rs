//! Local display masks. Messages remain in the archive and on the network.
use crate::merged::Member;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HiddenMessage {
    pub member: Member,
    pub message: String,
}

pub fn ids_in<'a>(
    list: &'a [HiddenMessage],
    member: &Member,
) -> std::collections::HashSet<&'a str> {
    list.iter()
        .filter(|entry| entry.member == *member)
        .map(|entry| entry.message.as_str())
        .collect()
}

/// A bounded local preference list. Never evict a mask silently.
pub const MAX_HIDDEN: usize = 4096;

/// Returns false when adding a new mask would exceed the local limit.
/// Unhide and updates of existing masks always remain available.
pub fn set(list: &mut Vec<HiddenMessage>, member: Member, message: String, hidden: bool) -> bool {
    let existing = list
        .iter()
        .any(|entry| entry.member == member && entry.message == message);
    if hidden && !existing && list.len() >= MAX_HIDDEN {
        return false;
    }
    list.retain(|entry| entry.member != member || entry.message != message);
    if hidden {
        list.push(HiddenMessage { member, message });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_never_evicts_masks_or_blocks_unhide() {
        let member = Member::default();
        let mut list: Vec<_> = (0..MAX_HIDDEN)
            .map(|index| HiddenMessage {
                member: member.clone(),
                message: index.to_string(),
            })
            .collect();
        assert!(!set(&mut list, member.clone(), "extra".into(), true));
        assert_eq!(list.len(), MAX_HIDDEN);
        assert!(set(&mut list, member.clone(), "0".into(), true));
        assert!(set(&mut list, member.clone(), "0".into(), false));
        assert!(set(&mut list, member, "extra".into(), true));
        assert_eq!(list.len(), MAX_HIDDEN);
    }

    #[test]
    fn masks_round_trip_and_stay_inside_the_exact_source() {
        let member = Member {
            origin: "server".into(),
            account: "account".into(),
            chat: "chat".into(),
        };
        let mut list = Vec::new();
        set(&mut list, member.clone(), "message".into(), true);
        set(&mut list, member.clone(), "message".into(), true);
        assert_eq!(list.len(), 1);
        let json = serde_json::to_string(&list).unwrap();
        let mut list: Vec<HiddenMessage> = serde_json::from_str(&json).unwrap();
        assert!(ids_in(&list, &member).contains("message"));
        for other in [
            Member {
                origin: "other".into(),
                ..member.clone()
            },
            Member {
                account: "other".into(),
                ..member.clone()
            },
            Member {
                chat: "other".into(),
                ..member.clone()
            },
        ] {
            assert!(ids_in(&list, &other).is_empty());
        }
        set(&mut list, member.clone(), "message".into(), false);
        assert!(ids_in(&list, &member).is_empty());
    }
}
