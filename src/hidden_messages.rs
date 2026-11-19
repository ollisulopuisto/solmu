//! Local display masks. Messages remain in the archive and on the network.
use crate::merged::Member;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HiddenMessage {
    pub member: Member,
    pub message: String,
}

pub fn ids_in<'a>(list: &'a [HiddenMessage], member: &Member) -> std::collections::HashSet<&'a str> {
    list.iter()
        .filter(|entry| entry.member == *member)
        .map(|entry| entry.message.as_str())
        .collect()
}

pub fn set(list: &mut Vec<HiddenMessage>, member: Member, message: String, hidden: bool) {
    list.retain(|entry| entry.member != member || entry.message != message);
    if hidden {
        list.push(HiddenMessage { member, message });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
