//! Sent-run width sharing for near-equal text bubbles, plus retargetable,
//! nearest-first width transitions.
use std::collections::{HashMap, HashSet};

pub const PAUSE: f64 = 0.0875;
pub const RESIZE: f64 = 0.210;

/// Join a sent text bubble to the previous text bubble's width when the
/// two are near-equal: within 16 points AND 15% of the larger (#282's
/// tolerance). Otherwise the bubble keeps its natural width, so a run
/// holding one wide message never inflates the short replies around it.
pub fn join(natural: f32, widest: f32) -> f32 {
    let diff = (widest - natural).abs();
    if diff <= 16.0 && diff <= 0.15 * natural.max(widest) {
        // Near-equal bubbles share the LARGER width: joining never shrinks
        // a bubble under its own text.
        natural.max(widest)
    } else {
        natural
    }
}

fn bounce(t: f32) -> f32 {
    let keys = [(0.0, 0.0), (0.5, 1.28), (0.78, 0.895), (1.0, 1.0)];
    let t = t.clamp(0.0, 1.0);
    for pair in keys.windows(2) {
        let [(a, x), (b, y)] = pair else {
            unreachable!()
        };
        if t <= *b {
            return x + (y - x) * (t - a) / (b - a);
        }
    }
    1.0
}

#[derive(Clone, Copy)]
struct Transition {
    from: f32,
    to: f32,
    start: f64,
}
impl Transition {
    fn sample(self, now: f64) -> f32 {
        self.from + (self.to - self.from) * bounce(((now - self.start) / RESIZE) as f32)
    }
}

/// The inputs a natural width was measured from. `fresh` compares them
/// field by field, so a frame over an unchanged chat costs one body
/// byte-compare per message instead of re-hashing every body.
#[derive(Clone)]
struct Snapshot {
    body: Option<Box<str>>,
    flags: u64,
    mentions: u64,
}

/// One timeline's natural-size cache and current width plan. Only appended
/// messages animate existing, visible rows. History loads and cap/font changes
/// snap. State is pruned to the current message set on every plan.
#[derive(Clone, Default)]
pub struct WidthBook {
    natural: HashMap<String, f32>,
    snapshots: HashMap<String, Snapshot>,
    targets: HashMap<String, f32>,
    motion: HashMap<String, Transition>,
    order: Vec<String>,
    fingerprint: Option<u32>,
}
impl WidthBook {
    pub fn retain_fingerprint(&mut self, fingerprint: u32) {
        if self.fingerprint != Some(fingerprint) {
            *self = Self {
                fingerprint: Some(fingerprint),
                ..Self::default()
            };
        }
    }
    pub fn get(&self, id: &str) -> Option<f32> {
        self.natural.get(id).copied()
    }
    /// Whether the remembered width for `id` was measured from these exact
    /// inputs: the cheap flags and mention names hashed, the body compared
    /// byte for byte.
    pub fn fresh(&self, id: &str, body: Option<&str>, flags: u64, mentions: u64) -> bool {
        self.snapshots.get(id).is_some_and(|snapshot| {
            snapshot.flags == flags
                && snapshot.mentions == mentions
                && snapshot.body.as_deref() == body
        })
    }
    pub fn remember(
        &mut self,
        id: &str,
        width: f32,
        body: Option<&str>,
        flags: u64,
        mentions: u64,
    ) {
        self.natural.insert(id.into(), width);
        self.snapshots.insert(
            id.into(),
            Snapshot {
                body: body.map(Into::into),
                flags,
                mentions,
            },
        );
    }
    pub fn invalidate(&mut self, id: &str) {
        if id.contains('\0') {
            self.natural.remove(id);
            self.snapshots.remove(id);
        } else {
            let suffix = format!("\0{id}");
            self.natural
                .retain(|key, _| key != id && !key.ends_with(&suffix));
            self.snapshots
                .retain(|key, _| key != id && !key.ends_with(&suffix));
        }
    }

    /// Entries are (stable row key, final content width, may animate). New
    /// rows always take their final width immediately. Existing rows retarget
    /// from their currently painted width, never an old endpoint or a queue.
    pub fn plan(&mut self, entries: &[(String, f32, bool)], now: f64, animate: bool) {
        let appended = self.order.last().is_some_and(|last| {
            entries
                .iter()
                .position(|(id, _, _)| id == last)
                .is_some_and(|i| i + 1 < entries.len())
        });
        let same_order = self.order.len() == entries.len()
            && self
                .order
                .iter()
                .zip(entries)
                .all(|(old, (id, _, _))| old == id);
        let same_targets = same_order
            && entries
                .iter()
                .all(|(id, width, _)| self.targets.get(id) == Some(width));
        if same_targets {
            self.motion
                .retain(|_, transition| animate && now < transition.start + RESIZE);
            return;
        }
        if !same_order {
            let ids: HashSet<&str> = entries.iter().map(|(id, _, _)| id.as_str()).collect();
            self.natural.retain(|id, _| ids.contains(id.as_str()));
            self.snapshots.retain(|id, _| ids.contains(id.as_str()));
            self.motion.retain(|id, transition| {
                ids.contains(id.as_str()) && now < transition.start + RESIZE
            });
        } else {
            self.motion
                .retain(|_, transition| now < transition.start + RESIZE);
        }
        if !animate {
            self.motion.clear();
        }
        let mut rank = 0;
        for (id, target, visible) in entries.iter().rev() {
            let old = self.targets.get(id).copied();
            if old.is_some_and(|old| (old - target).abs() > 0.1)
                || (appended && self.motion.contains_key(id))
            {
                let from = self
                    .motion
                    .get(id)
                    .map_or(old.unwrap(), |motion| motion.sample(now));
                if animate && appended && *visible {
                    self.motion.insert(
                        id.clone(),
                        Transition {
                            from,
                            to: *target,
                            start: now + PAUSE + f64::from(rank) * RESIZE,
                        },
                    );
                    rank += 1;
                } else {
                    self.motion.remove(id);
                }
            }
        }
        self.targets = entries
            .iter()
            .map(|(id, width, _)| (id.clone(), *width))
            .collect();
        if !same_order {
            self.order = entries.iter().map(|(id, _, _)| id.clone()).collect();
        }
    }
    pub fn width(&self, id: &str, now: f64) -> Option<f32> {
        self.motion
            .get(id)
            .map(|motion| motion.sample(now))
            .or_else(|| self.targets.get(id).copied())
    }
    pub fn animating(&self, now: f64) -> bool {
        self.motion
            .values()
            .any(|motion| now < motion.start + RESIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(items: &[(&str, f32)]) -> Vec<(String, f32, bool)> {
        items
            .iter()
            .map(|(id, w)| (id.to_string(), *w, true))
            .collect()
    }
    #[test]
    fn unchanged_plans_reuse_collections_and_keep_motion_running() {
        let mut book = WidthBook::default();
        let initial = rows(&[("a", 120.)]);
        book.plan(&initial, 0., true);
        let changed = rows(&[("a", 220.), ("b", 220.)]);
        book.plan(&changed, 1., true);
        let order = book.order.as_ptr();
        let targets = book.targets.capacity();
        book.plan(&changed, 1. + PAUSE + RESIZE * 0.5, true);
        assert_eq!(book.order.as_ptr(), order);
        assert_eq!(book.targets.capacity(), targets);
        assert!(book.width("a", 1. + PAUSE + RESIZE * 0.5).unwrap() > 220.);
        book.plan(&changed, 2., false);
        assert_eq!(book.width("a", 2.), Some(220.));
        assert!(!book.animating(2.));
    }

    #[test]
    fn near_equal_text_widths_join_and_distant_ones_keep_their_own() {
        // Within both tolerances: joins, including the 16-point edge.
        assert_eq!(join(100., 116.), 116.);
        assert_eq!(join(90., 105.), 105.);
        // A wider bubble joining a narrower predecessor never shrinks:
        // both end at the larger width.
        assert_eq!(join(116., 100.), 116.);
        assert_eq!(join(105., 90.), 105.);
        // Past 16 points or past 15% of the larger: keeps its natural width.
        assert_eq!(join(100., 117.), 100.);
        assert_eq!(join(80., 95.9), 80.);
        // A run with one wide message leaves the short ones alone.
        assert_eq!(join(120., 540.), 120.);
        // A bubble as wide as the run joins itself: natural in, natural out.
        assert_eq!(join(540., 540.), 540.);
    }
    #[test]
    fn arrival_is_final_first_then_older_rows_resize_one_at_a_time() {
        let mut b = WidthBook::default();
        b.plan(&rows(&[("a", 100.), ("b", 100.), ("c", 100.)]), 0., true);
        b.plan(
            &rows(&[("a", 200.), ("b", 200.), ("c", 200.), ("d", 500.)]),
            1.,
            true,
        );
        assert_eq!(b.width("d", 1.), Some(500.));
        assert_eq!(b.width("c", 1. + PAUSE / 2.), Some(100.));
        assert!(b.width("c", 1. + PAUSE + RESIZE * 0.5).unwrap() > 200.);
        assert_eq!(b.width("b", 1. + PAUSE + RESIZE * 0.5), Some(100.));
        assert_eq!(b.width("a", 1. + PAUSE + RESIZE * 1.5), Some(100.));
        assert!((b.width("a", 1. + PAUSE + RESIZE * 3.).unwrap() - 200.).abs() < 0.01);
        assert!(!b.animating(2.));
    }
    #[test]
    fn rapid_arrival_retargets_without_a_jump_or_queue() {
        let mut b = WidthBook::default();
        b.plan(&rows(&[("a", 100.)]), 0., true);
        b.plan(&rows(&[("a", 200.), ("b", 300.)]), 1., true);
        let now = 1. + PAUSE + RESIZE * 0.3;
        let current = b.width("a", now).unwrap();
        b.plan(&rows(&[("a", 250.), ("b", 400.), ("c", 600.)]), now, true);
        assert!((b.width("a", now).unwrap() - current).abs() < 0.01);
        assert_eq!(b.width("c", now), Some(600.));
        assert_eq!(b.width("a", 3.), Some(250.));
    }
    #[test]
    fn reduced_motion_history_resize_and_hidden_rows_snap() {
        let mut b = WidthBook::default();
        b.plan(&rows(&[("a", 100.)]), 0., true);
        b.plan(&rows(&[("a", 200.), ("b", 400.)]), 1., false);
        assert_eq!(b.width("a", 1.), Some(200.));
        assert!(!b.animating(1.));
        b.plan(
            &rows(&[("older", 500.), ("a", 300.), ("b", 400.)]),
            2.,
            true,
        );
        assert_eq!(b.width("a", 2.), Some(300.));
        b.retain_fingerprint(2);
        assert!(b.width("a", 2.).is_none());
        b.plan(&rows(&[("a", 100.)]), 3., true);
        b.plan(
            &[("a".into(), 250., false), ("b".into(), 500., true)],
            4.,
            true,
        );
        assert_eq!(b.width("a", 4.), Some(250.));
    }
    #[test]
    fn deletion_prunes_and_edit_invalidates_measurements() {
        let mut b = WidthBook::default();
        b.remember("a", 123., Some("hello"), 1, 0);
        b.remember("b", 456., Some("world"), 1, 0);
        b.plan(&rows(&[("a", 123.)]), 0., true);
        assert!(b.get("b").is_none());
        assert!(b.fresh("a", Some("hello"), 1, 0));
        b.invalidate("a");
        assert!(b.get("a").is_none());
        assert!(!b.fresh("a", Some("hello"), 1, 0));
    }
    #[test]
    fn an_edit_makes_the_remembered_width_stale() {
        let mut b = WidthBook::default();
        b.remember("a", 123., Some("hello world"), 1, 0);
        assert!(b.fresh("a", Some("hello world"), 1, 0));
        // Same length, one byte different: a hash-only key could collide,
        // the byte-compare cannot.
        assert!(!b.fresh("a", Some("hello_world"), 1, 0));
        assert!(!b.fresh("a", Some("hello world!"), 1, 0));
        assert!(!b.fresh("a", Some("hello world"), 2, 0));
        assert!(!b.fresh("a", Some("hello world"), 1, 7));
        assert!(!b.fresh("a", None, 1, 0));
    }
}
