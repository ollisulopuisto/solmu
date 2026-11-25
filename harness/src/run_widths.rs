//! Neighbour-aware widths for outgoing text bubbles.
//!
//! Plain text bubbles from the reader no longer settle on the shared 80pt
//! grid: each takes its natural content width, and neighbours within a
//! consecutive same-sender run join the larger width when they are close
//! enough. Decisions freeze once made at a given width cap, so an appended
//! or prepended message never resizes bubbles already on screen; a later
//! slightly wider message grows alone instead. Media, quotes, cards and
//! forwarded messages keep the old grid. This module is the pure decision
//! logic; measuring text lives in [`crate::ui::conversation`].

use std::collections::HashMap;

/// Neighbours join when their natural widths differ by at most this many
/// logical points... Tunable: the value was proposed, not measured.
pub const JOIN_POINTS: f32 = 16.0;
/// ...AND at most this share of the larger width. Both conditions must
/// hold, so small bubbles need near-exact matches and large ones stay
/// proportional. Tunable for the same reason.
pub const JOIN_RATIO: f32 = 0.15;

/// The width neighbours share when they join, or `None` when they differ
/// by more than the tolerance allows.
pub fn join(a: f32, b: f32) -> Option<f32> {
    let diff = (a - b).abs();
    let larger = a.max(b);
    (diff <= JOIN_POINTS && diff <= larger * JOIN_RATIO).then_some(larger)
}

/// Frozen width decisions for one conversation at one width cap.
#[derive(Default)]
pub struct WidthBook {
    /// Settled width per message id.
    decided: HashMap<String, f32>,
    /// The cap (`f32::to_bits`) the decisions were made at. A viewport or
    /// font change legitimately re-lays out: everything is decided again.
    fingerprint: Option<u32>,
}

impl WidthBook {
    /// Drops every decision when the cap changed since the last call.
    pub fn retain_fingerprint(&mut self, cap_bits: u32) {
        if self.fingerprint != Some(cap_bits) {
            self.decided.clear();
            self.fingerprint = Some(cap_bits);
        }
    }

    /// The frozen decision for a message already settled, if any.
    pub fn get(&self, id: &str) -> Option<f32> {
        self.decided.get(id).copied()
    }

    /// Settles a message not yet decided: it joins its run neighbour's
    /// settled width when both are close, keeping the larger natural width,
    /// and takes its own natural width otherwise. The result freezes; later
    /// calls return it unchanged.
    pub fn settle(&mut self, id: &str, natural: f32, neighbour: Option<f32>) -> f32 {
        if let Some(width) = self.decided.get(id) {
            return *width;
        }
        let width = neighbour
            .and_then(|other| join(other, natural))
            .unwrap_or(natural);
        self.decided.insert(id.to_owned(), width);
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_within_tolerance_share_the_larger_width() {
        let mut book = WidthBook::default();
        book.retain_fingerprint(1);
        assert_eq!(book.settle("a", 100.0, None), 100.0);
        // 14 <= 16 points and 14/114 <= 15%: the pair joins at 114.
        assert_eq!(book.settle("b", 114.0, Some(100.0)), 114.0);
        // Frozen: the earlier bubble never grows afterwards.
        assert_eq!(book.settle("a", 100.0, Some(114.0)), 100.0);
        // A later, slightly wider message joins at its own larger width...
        assert_eq!(book.settle("c", 128.0, Some(114.0)), 128.0);
        // ...without growing its neighbours.
        assert_eq!(book.settle("a", 100.0, Some(128.0)), 100.0);
        assert_eq!(book.settle("b", 114.0, Some(128.0)), 114.0);
    }

    #[test]
    fn just_outside_the_point_tolerance_does_not_join() {
        assert_eq!(join(100.0, 116.0), Some(116.0));
        assert_eq!(join(100.0, 116.5), None);
    }

    #[test]
    fn just_outside_the_ratio_tolerance_does_not_join() {
        // 16 points apart is fine for wide bubbles but too much for narrow.
        assert_eq!(join(200.0, 216.0), Some(216.0));
        assert_eq!(join(40.0, 56.0), None);
        assert_eq!(join(40.0, 55.9), None); // 15.9/55.9 > 15%
    }

    #[test]
    fn runs_break_without_a_neighbour() {
        let mut book = WidthBook::default();
        assert_eq!(book.settle("a", 100.0, None), 100.0);
        // A different sender, chat, day or an excluded row passes no
        // neighbour: the next message starts from its own natural width.
        assert_eq!(book.settle("b", 113.0, None), 113.0);
    }

    #[test]
    fn prepended_history_keeps_loaded_widths() {
        let mut book = WidthBook::default();
        book.retain_fingerprint(1);
        // The first page loaded: "b" joined "a" upwards.
        assert_eq!(book.settle("a", 100.0, None), 100.0);
        assert_eq!(book.settle("b", 114.0, Some(100.0)), 114.0);
        // Older history arrives before "a". It settles among itself...
        assert_eq!(book.settle("older", 110.0, None), 110.0);
        // ...and "a" keeps its frozen width rather than joining "older".
        assert_eq!(book.settle("a", 100.0, Some(110.0)), 100.0);
    }

    #[test]
    fn a_cap_change_redecides() {
        let mut book = WidthBook::default();
        book.retain_fingerprint(1);
        assert_eq!(book.settle("a", 100.0, None), 100.0);
        book.retain_fingerprint(2);
        assert_eq!(book.get("a"), None);
        assert_eq!(book.settle("a", 90.0, None), 90.0);
        book.retain_fingerprint(2);
        assert_eq!(book.get("a"), Some(90.0));
    }
}
