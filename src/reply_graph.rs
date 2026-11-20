//! Exact loaded-history reply graph, independent of rendering and network access.
use std::collections::{HashMap, HashSet};
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub chat: String,
    pub id: String,
}
pub fn index(
    rows: impl IntoIterator<Item = (Key, Option<Key>)>,
) -> (HashMap<Key, Vec<Key>>, HashMap<Key, Key>) {
    let rows: Vec<_> = rows.into_iter().collect();
    let known: HashSet<_> = rows.iter().map(|(key, _)| key.clone()).collect();
    let mut replies: HashMap<Key, Vec<Key>> = HashMap::new();
    let mut parents = HashMap::new();
    for (key, parent) in rows {
        if let Some(parent) = parent
            && parent != key
            && known.contains(&parent)
            && !parents.contains_key(&key)
        {
            replies.entry(parent.clone()).or_default().push(key.clone());
            parents.insert(key, parent);
        }
    }
    (replies, parents)
}
pub fn chain(root: &Key, parents: &HashMap<Key, Key>) -> HashSet<Key> {
    let mut out = HashSet::from([root.clone()]);
    loop {
        let before = out.len();
        for (child, parent) in parents {
            if out.contains(parent) {
                out.insert(child.clone());
            }
        }
        if before == out.len() {
            return out;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn k(c: &str, i: &str) -> Key {
        Key {
            chat: c.into(),
            id: i.into(),
        }
    }
    #[test]
    fn exact_chat_not_id_alone() {
        let a = k("a", "1");
        let b = k("b", "1");
        let c = k("a", "2");
        let (r, p) = index([
            (a.clone(), None),
            (b.clone(), None),
            (c.clone(), Some(b.clone())),
            (c, Some(b.clone())),
        ]);
        assert!(!r.contains_key(&a));
        assert_eq!(r[&b].len(), 1);
        assert_eq!(p.len(), 1);
    }
    #[test]
    fn unloaded_and_self_parent_are_not_counted() {
        let a = k("a", "1");
        let (r, p) = index([(a.clone(), Some(a)), (k("a", "2"), Some(k("missing", "1")))]);
        assert!(r.is_empty());
        assert!(p.is_empty());
    }
    #[test]
    fn descendants_terminate_cycles_and_exclude_unrelated() {
        let a = k("a", "1");
        let b = k("a", "2");
        let c = k("a", "3");
        let d = k("a", "4");
        let (_, p) = index([
            (a.clone(), Some(b.clone())),
            (b.clone(), Some(a.clone())),
            (c, Some(b)),
            (d.clone(), None),
        ]);
        let set = chain(&a, &p);
        assert_eq!(set.len(), 3);
        assert!(!set.contains(&d));
    }
}
