//! Local suggestions for chats that look like the same person on different
//! networks. Exact matches only: a full name that normalises the same, or an
//! identical handle. Nothing here touches the network or links anything by
//! itself; fuzzy matching stays in the manual picker.
use crate::merged::{Member, Person};
use crate::model::{Chat, ChatKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A linked person supports at most this many chats.
pub const MAX_MEMBERS: usize = 8;

/// A pair the user said is not the same person, stored in a fixed order so
/// either direction matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkPair {
    pub a: Member,
    pub b: Member,
}

impl LinkPair {
    pub fn new(x: &Member, y: &Member) -> Self {
        let (a, b) = if (&x.origin, &x.account, &x.chat) <= (&y.origin, &y.account, &y.chat) {
            (x, y)
        } else {
            (y, x)
        };
        Self {
            a: a.clone(),
            b: b.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The same full name, shown as the first chat spells it.
    Name(String),
    /// The same phone number or username.
    Handle(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub a: Member,
    pub b: Member,
    pub reason: Reason,
}

impl Suggestion {
    pub fn pair(&self) -> LinkPair {
        LinkPair::new(&self.a, &self.b)
    }

    pub fn involves(&self, chat: &str) -> bool {
        self.a.chat == chat || self.b.chat == chat
    }
}

fn fold(c: char) -> &'static str {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'ā' => "a",
        'ç' | 'ć' | 'č' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => "o",
        'š' | 'ś' => "s",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ý' | 'ÿ' => "y",
        'ž' | 'ź' | 'ż' => "z",
        'ß' => "ss",
        _ => "",
    }
}

/// A full name as comparable words: lower case, accents folded, punctuation
/// ignored. `None` for anything that is not plainly a person's full name:
/// one word, numbers, addresses, or a placeholder title.
pub fn normalized_name(name: &str) -> Option<String> {
    let lower = name.trim().to_lowercase();
    if lower.contains('@') || lower.contains(':') || lower.contains("name unavailable") {
        return None;
    }
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in lower.chars() {
        let folded = fold(c);
        if !folded.is_empty() {
            word.push_str(folded);
        } else if c.is_alphanumeric() {
            word.push(c);
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let letters: usize = words
        .iter()
        .map(|w| w.chars().filter(|c| c.is_alphabetic()).count())
        .sum();
    let total: usize = words.iter().map(|w| w.chars().count()).sum();
    if words.len() < 2 || letters * 2 < total || words.iter().any(|w| w.chars().count() < 2) {
        return None;
    }
    Some(words.join(" "))
}

/// A phone number as `+digits`. `None` unless it starts with `+` and has 7
/// to 15 digits.
pub fn normalized_phone(raw: &str) -> Option<String> {
    let text = raw.trim();
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let phone_like = text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'));
    (phone_like && text.starts_with('+') && (7..=15).contains(&digits.len()))
        .then(|| format!("+{digits}"))
}

/// A username without its `@`, lower case. `None` when too short to identify
/// anyone, not plain letters, digits, `.`, `_` or `-`, or phone-like.
pub fn normalized_username(raw: &str) -> Option<String> {
    let user = raw.trim().trim_start_matches('@').to_lowercase();
    let plain = user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    (plain && user.len() >= 4 && user.chars().any(|c| !c.is_ascii_digit())).then_some(user)
}

/// Either kind, for handles already stored on a chat.
pub fn normalized_handle(raw: &str) -> Option<String> {
    normalized_phone(raw).or_else(|| normalized_username(raw))
}

fn eligible(chat: &Chat) -> bool {
    chat.kind == ChatKind::Direct
        && !chat.locked
        && chat.backend_network.is_some()
        && Member::of(chat).is_some()
}

/// Pairs of direct chats on different networks of one server that share a
/// full name or a handle, minus dismissed pairs and chats already linked
/// together. A chat joining an existing person is suggested when the person
/// has room; two different people are never suggested for merging.
pub fn suggestions(chats: &[Chat], people: &[Person], dismissed: &[LinkPair]) -> Vec<Suggestion> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for chat in chats {
        *seen.entry(chat.id.as_str()).or_default() += 1;
    }
    let candidates: Vec<&Chat> = chats
        .iter()
        .filter(|c| eligible(c) && seen.get(c.id.as_str()) == Some(&1))
        .collect();
    let mut keys: HashMap<String, Vec<(usize, Reason)>> = HashMap::new();
    for (index, chat) in candidates.iter().enumerate() {
        if let Some(name) = normalized_name(&chat.name) {
            keys.entry(format!("n:{name}"))
                .or_default()
                .push((index, Reason::Name(chat.name.trim().to_owned())));
        }
        let mut handles: Vec<String> = chat
            .handles
            .iter()
            .filter_map(|h| normalized_handle(h))
            .collect();
        handles.sort();
        handles.dedup();
        for handle in handles {
            keys.entry(format!("h:{handle}"))
                .or_default()
                .push((index, Reason::Handle(handle)));
        }
    }
    let person_of = |chat: &Chat| {
        people
            .iter()
            .position(|p| p.members.iter().any(|m| m.matches(chat)))
    };
    let mut found: Vec<Suggestion> = Vec::new();
    let mut taken = std::collections::HashSet::new();
    let mut groups: Vec<_> = keys.into_values().collect();
    // Handles first: a shared number is stronger than a shared name.
    groups.sort_by_key(|g| !matches!(g[0].1, Reason::Handle(_)));
    for group in groups {
        for (i, (x, reason)) in group.iter().enumerate() {
            for (y, _) in &group[i + 1..] {
                let (a, b) = (candidates[*x], candidates[*y]);
                let (Some(ma), Some(mb)) = (Member::of(a), Member::of(b)) else {
                    continue;
                };
                let pair = LinkPair::new(&ma, &mb);
                let (pa, pb) = (person_of(a), person_of(b));
                let room = match (pa, pb) {
                    (Some(_), Some(_)) => false,
                    (Some(p), None) | (None, Some(p)) => people[p].members.len() < MAX_MEMBERS,
                    (None, None) => true,
                };
                if ma.origin == mb.origin
                    && a.backend_network != b.backend_network
                    && room
                    && !dismissed.contains(&pair)
                    && taken.insert(pair)
                {
                    found.push(Suggestion {
                        a: ma,
                        b: mb,
                        reason: reason.clone(),
                    });
                }
            }
        }
    }
    found.sort_by(|l, r| (&l.a.chat, &l.b.chat).cmp(&(&r.a.chat, &r.b.chat)));
    found
}

/// The people list after accepting `suggestion`: a new person, or the other
/// chat added to the person one of them already belongs to.
pub fn accepted(mut people: Vec<Person>, suggestion: &Suggestion, name: &str) -> Vec<Person> {
    let existing = people.iter().position(|p| {
        p.members
            .iter()
            .any(|m| *m == suggestion.a || *m == suggestion.b)
    });
    match existing {
        Some(index) => {
            for member in [&suggestion.a, &suggestion.b] {
                if !people[index].members.contains(member) {
                    people[index].members.push(member.clone());
                }
            }
        }
        None => people.push(Person {
            name: name.to_owned(),
            members: vec![suggestion.a.clone(), suggestion.b.clone()],
            primary: None,
        }),
    }
    people
}

/// The chats an accepted suggestion put into the people list: the ones that
/// were in no person before.
pub fn added_by(people: &[Person], suggestion: &Suggestion) -> Vec<Member> {
    [&suggestion.a, &suggestion.b]
        .into_iter()
        .filter(|m| !people.iter().any(|p| p.members.contains(m)))
        .cloned()
        .collect()
}

/// The people list with `added` taken back out, and nothing else changed. A
/// person that lost a chat and is left with fewer than two is no longer a link
/// and goes too; people this undo did not touch are kept as they are.
pub fn undone(mut people: Vec<Person>, added: &[Member]) -> Vec<Person> {
    let mut touched = vec![false; people.len()];
    for (person, touched) in people.iter_mut().zip(&mut touched) {
        let before = person.members.len();
        person.members.retain(|m| !added.contains(m));
        *touched = person.members.len() != before;
        if person.primary.as_ref().is_some_and(|m| added.contains(m)) {
            person.primary = None;
        }
    }
    let mut touched = touched.into_iter();
    people.retain(|p| !touched.next().unwrap_or(false) || p.members.len() >= 2);
    people
}

/// Whether an Undo for `added` depends on `person`: it took chats into that
/// person, so a later join onto the same person builds on it.
pub fn depends_on(person: &Person, added: &[Member]) -> bool {
    added.iter().any(|m| person.members.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, name: &str, network: &str, handles: &[&str]) -> Chat {
        Chat {
            backend_origin: Some("http://local".into()),
            backend_account: Some(format!("{network}-acct")),
            backend_network: Some(network.into()),
            handles: handles.iter().map(|h| (*h).to_owned()).collect(),
            ..Chat::new(id.into(), name.into())
        }
    }

    #[test]
    fn the_same_full_name_on_two_networks_is_suggested() {
        let chats = [
            chat("wa", "Åsa  Möller", "WhatsApp", &[]),
            chat("tg", "asa moller", "Telegram", &[]),
        ];
        let found = suggestions(&chats, &[], &[]);
        assert_eq!(found.len(), 1);
        assert!(matches!(found[0].reason, Reason::Name(_)));
    }

    #[test]
    fn single_words_placeholders_and_close_names_are_not_suggested() {
        let chats = [
            chat("a", "Anna", "WhatsApp", &[]),
            chat("b", "Anna", "Telegram", &[]),
            chat("c", "Anna Virtanen", "Signal", &[]),
            chat("d", "Anna Virtanen-Koski", "Instagram", &[]),
            chat("e", "Signal conversation (name unavailable)", "Signal", &[]),
            chat(
                "f",
                "Signal conversation (name unavailable)",
                "Telegram",
                &[],
            ),
        ];
        assert!(suggestions(&chats, &[], &[]).is_empty());
    }

    #[test]
    fn an_identical_phone_or_username_is_suggested_even_with_different_names() {
        let chats = [
            chat("wa", "Mikko", "WhatsApp", &["+358 40 123 4567"]),
            chat("sg", "M. Laine", "Signal", &["+358401234567"]),
            chat("ig", "Kati", "Instagram", &["@kati_k"]),
            chat("tg", "Kati K", "Telegram", &["kati_k"]),
        ];
        let found = suggestions(&chats, &[], &[]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|s| matches!(s.reason, Reason::Handle(_))));
    }

    #[test]
    fn the_same_network_or_server_never_matches() {
        let mut other = chat("b", "Ada Lovelace", "Telegram", &[]);
        other.backend_origin = Some("http://elsewhere".into());
        let chats = [
            chat("a", "Ada Lovelace", "WhatsApp", &[]),
            chat("c", "Ada Lovelace", "WhatsApp", &[]),
            other,
        ];
        assert!(suggestions(&chats, &[], &[]).is_empty());
    }

    #[test]
    fn groups_locked_chats_and_duplicate_ids_are_skipped() {
        let mut group = chat("g", "Ada Lovelace", "Telegram", &[]);
        group.kind = ChatKind::Group;
        let mut locked = chat("l", "Ada Lovelace", "Signal", &[]);
        locked.locked = true;
        let chats = [
            chat("a", "Ada Lovelace", "WhatsApp", &[]),
            group,
            locked,
            chat("d", "Ada Lovelace", "Instagram", &[]),
            chat("d", "Ada Lovelace", "Instagram", &[]),
        ];
        assert!(suggestions(&chats, &[], &[]).is_empty());
    }

    #[test]
    fn a_dismissed_pair_stays_gone_in_either_order() {
        let chats = [
            chat("wa", "Ada Lovelace", "WhatsApp", &[]),
            chat("tg", "Ada Lovelace", "Telegram", &[]),
        ];
        let found = suggestions(&chats, &[], &[]);
        let reversed = LinkPair::new(&found[0].b, &found[0].a);
        assert!(suggestions(&chats, &[], &[reversed]).is_empty());
    }

    #[test]
    fn linked_chats_are_not_suggested_but_a_third_chat_may_join() {
        let chats = [
            chat("wa", "Ada Lovelace", "WhatsApp", &[]),
            chat("tg", "Ada Lovelace", "Telegram", &[]),
            chat("sg", "Ada Lovelace", "Signal", &[]),
        ];
        let first = suggestions(&chats, &[], &[]);
        let both = accepted(Vec::new(), &first[0], "Ada");
        assert_eq!(both.len(), 1);
        let left_out = ["wa", "tg", "sg"]
            .into_iter()
            .find(|id| both[0].members.iter().all(|m| m.chat != *id))
            .unwrap();
        let rest = suggestions(&chats, &both, &[]);
        assert!(!rest.is_empty());
        assert!(rest.iter().all(|s| s.involves(left_out)));
        let joined = accepted(both, &rest[0], "Ada");
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].members.len(), 3);
    }

    #[test]
    fn two_existing_people_are_never_offered_for_merging() {
        let chats = [
            chat("wa", "Ada Lovelace", "WhatsApp", &[]),
            chat("tg", "Ada Lovelace", "Telegram", &[]),
        ];
        let one = Person {
            name: "Ada".into(),
            members: vec![Member::of(&chats[0]).unwrap()],
            primary: None,
        };
        let two = Person {
            name: "Ada T".into(),
            members: vec![Member::of(&chats[1]).unwrap()],
            primary: None,
        };
        assert!(suggestions(&chats, &[one, two], &[]).is_empty());
    }

    #[test]
    fn handles_reject_opaque_ids_and_short_numbers() {
        assert_eq!(normalized_handle("@opaque:beeper.local"), None);
        assert_eq!(normalized_handle("12345"), None);
        assert_eq!(
            normalized_handle("+1 (555) 010-9999").as_deref(),
            Some("+15550109999")
        );
        assert_eq!(normalized_handle("@Kati_K").as_deref(), Some("kati_k"));
    }

    #[test]
    fn undoing_one_link_leaves_the_others() {
        let chats = [
            chat("a-wa", "Alice Aalto", "WhatsApp", &[]),
            chat("a-tg", "Alice Aalto", "Telegram", &[]),
            chat("b-wa", "Bob Berg", "WhatsApp", &[]),
            chat("b-tg", "Bob Berg", "Telegram", &[]),
        ];
        let found = suggestions(&chats, &[], &[]);
        let alice = found.iter().find(|s| s.a.chat.starts_with("a-")).unwrap();
        let bob = found.iter().find(|s| s.a.chat.starts_with("b-")).unwrap();
        let mut people = Vec::new();
        let alice_added = added_by(&people, alice);
        people = accepted(people, alice, "Alice Aalto");
        let bob_added = added_by(&people, bob);
        people = accepted(people, bob, "Bob Berg");
        assert_eq!(people.len(), 2);
        let after = undone(people.clone(), &alice_added);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Bob Berg");
        let after = undone(people, &bob_added);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Alice Aalto");
    }

    #[test]
    fn undoing_a_join_removes_only_the_joined_chat() {
        let chats = [
            chat("wa", "Ada Lovelace", "WhatsApp", &[]),
            chat("tg", "Ada Lovelace", "Telegram", &[]),
            chat("sg", "Ada Lovelace", "Signal", &[]),
        ];
        let first = suggestions(&chats, &[], &[]);
        let mut people = accepted(Vec::new(), &first[0], "Ada");
        let rest = suggestions(&chats, &people, &[]);
        let added = added_by(&people, &rest[0]);
        assert_eq!(added.len(), 1);
        people = accepted(people, &rest[0], "Ada");
        assert_eq!(people[0].members.len(), 3);
        let after = undone(people, &added);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].members.len(), 2);
    }

    fn member(chat: &str) -> Member {
        Member {
            origin: "http://local".into(),
            account: "acct".into(),
            chat: chat.into(),
        }
    }

    #[test]
    fn undo_leaves_unrelated_one_chat_people_alone() {
        let people = vec![
            Person {
                name: "Solo".into(),
                members: vec![member("solo")],
                primary: None,
            },
            Person {
                name: "Pair".into(),
                members: vec![member("a"), member("b")],
                primary: None,
            },
        ];
        let after = undone(people, &[member("a"), member("b")]);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Solo");
    }

    #[test]
    fn a_join_depends_on_the_person_it_extends() {
        let person = Person {
            name: "P".into(),
            members: vec![member("a"), member("b")],
            primary: None,
        };
        assert!(depends_on(&person, &[member("a"), member("b")]));
        assert!(!depends_on(&person, &[member("z")]));
    }
}
