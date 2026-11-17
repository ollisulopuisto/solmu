//! Sent-run thirds and retargetable, nearest-first width transitions.
use std::collections::{HashMap, HashSet};

pub const PAUSE: f64 = 0.0875;
pub const RESIZE: f64 = 0.210;

/// Round an outer bubble width up to a third of the run's widest bubble.
/// The viewport wins over the 120-point floor on very narrow panes.
pub fn band(natural: f32, widest: f32, cap: f32) -> f32 {
    let widest = widest.max(120.0).min(cap.max(0.0));
    if widest <= 0.0 {
        return 0.0;
    }
    let third = widest / 3.0;
    ((natural / third - 0.00001).ceil() * third)
        .max(120.0)
        .min(widest)
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

/// One timeline's natural-size cache and current width plan. Only appended
/// messages animate existing, visible rows. History loads and cap/font changes
/// snap. State is pruned to the current message set on every plan.
#[derive(Clone, Default)]
pub struct WidthBook {
    natural: HashMap<String, f32>,
    signatures: HashMap<String, u64>,
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
    pub fn remember(&mut self, id: &str, width: f32) {
        self.natural.insert(id.into(), width);
    }
    pub fn invalidate(&mut self, id: &str) {
        self.natural
            .retain(|key, _| key != id && !key.ends_with(&format!("\0{id}")));
    }
    pub fn check_signature(&mut self, id: &str, signature: u64) {
        if self.signatures.insert(id.into(), signature) != Some(signature) {
            self.natural.remove(id);
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
        let ids: HashSet<&str> = entries.iter().map(|(id, _, _)| id.as_str()).collect();
        self.natural.retain(|id, _| ids.contains(id.as_str()));
        self.signatures.retain(|id, _| ids.contains(id.as_str()));
        self.motion
            .retain(|id, transition| ids.contains(id.as_str()) && now < transition.start + RESIZE);
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
        self.order = entries.iter().map(|(id, _, _)| id.clone()).collect();
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
    fn sent_bands_round_up_and_respect_narrow_caps() {
        assert_eq!(
            [80., 250., 400.].map(|w| band(w, 450., 560.)),
            [150., 300., 450.]
        );
        for natural in [0., 20., 120., 300., 900.] {
            let width = band(natural, 900., 75.);
            assert_eq!(width, 75.);
        }
        assert_eq!(band(1., 70., 560.), 120.);
        assert_eq!(band(450., 450., 560.), 450.);
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
        b.remember("a", 123.);
        b.remember("b", 456.);
        b.plan(&rows(&[("a", 123.)]), 0., true);
        assert!(b.get("b").is_none());
        b.invalidate("a");
        assert!(b.get("a").is_none());
    }
}
