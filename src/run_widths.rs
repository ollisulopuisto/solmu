//! Neighbour-aware widths for outgoing text bubbles.
//!
//! Plain text bubbles from the reader no longer settle on the shared 80pt
//! grid: each takes the natural width of its balanced layout, and a bubble
//! keeps its run neighbour's width when wrapping at that width costs at
//! most one line over the text's minimum. A bubble that would pay more
//! than that widens to its own natural width. Decisions freeze once made
//! at a given width cap, so an appended or prepended message never resizes
//! bubbles already on screen; a later slightly wider message grows alone
//! instead. Media, quotes, cards and forwarded messages keep the old grid.
//! This module is the pure decision logic; measuring and balancing text
//! lives in `crate::wrap` and `crate::ui::conversation` (private modules, so no doc links).

use std::collections::HashMap;

/// A bubble keeps its neighbour's width while wrapping there costs at
/// most this many lines over its minimum. From the line-wrap design
/// sheets: "keep the previous width if it costs <= +1 line vs the text's
/// minimum; else widen to the min-line layout."
pub const KEEP_EXTRA_LINES: usize = 1;

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

    /// Forget one message's width, e.g. after its text was edited or the
    /// message left the chat; it settles again when next measured.
    pub fn invalidate(&mut self, id: &str) {
        self.decided.remove(id);
    }

    /// Settles a message not yet decided: it keeps its run neighbour's
    /// settled width when balancing at that width costs at most
    /// [`KEEP_EXTRA_LINES`] over the text's minimum line count, and takes
    /// its own natural width (its min-line layout at the cap) otherwise.
    /// `min_lines` and `natural` come from balancing at the width cap;
    /// `lines_at` balances the text at the neighbour's width. The result
    /// freezes; later calls return it unchanged.
    pub fn settle(
        &mut self,
        id: &str,
        min_lines: usize,
        natural: f32,
        neighbour: Option<f32>,
        lines_at: impl FnOnce(f32) -> usize,
    ) -> f32 {
        if let Some(width) = self.decided.get(id) {
            return *width;
        }
        let width = match neighbour {
            Some(kept) if lines_at(kept) <= min_lines + KEEP_EXTRA_LINES => kept,
            _ => natural,
        };
        self.decided.insert(id.to_owned(), width);
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_bubble_takes_its_natural_width() {
        let mut book = WidthBook::default();
        let width = book.settle("a", 2, 359.0, None, |_| unreachable!());
        assert_eq!(width, 359.0);
    }

    #[test]
    fn keeps_the_neighbour_within_one_extra_line() {
        // Sheet 3 at the 440 column: the second bubble wraps to 4 lines at
        // the neighbour's 359pt, one over its 3-line minimum, and keeps.
        let mut book = WidthBook::default();
        book.settle("a", 1, 359.0, None, |_| unreachable!());
        let width = book.settle("b", 3, 472.0, Some(359.0), |_| 4);
        assert_eq!(width, 359.0);
    }

    #[test]
    fn widens_past_one_extra_line() {
        // Sheet 3 at the 560 column: the second bubble wraps to 4 lines at
        // 359pt against a 2-line minimum, and widens to its 591pt min-line
        // layout.
        let mut book = WidthBook::default();
        book.settle("a", 1, 359.0, None, |_| unreachable!());
        let width = book.settle("b", 2, 591.0, Some(359.0), |_| 4);
        assert_eq!(width, 591.0);
    }

    #[test]
    fn a_kept_width_passes_down_the_run() {
        // Sheet 3 at 440: the third bubble also holds the run's 359pt.
        let mut book = WidthBook::default();
        book.settle("a", 1, 359.0, None, |_| unreachable!());
        book.settle("b", 3, 472.0, Some(359.0), |_| 4);
        let width = book.settle("c", 2, 264.0, Some(359.0), |_| 3);
        assert_eq!(width, 359.0);
    }

    #[test]
    fn decisions_freeze() {
        let mut book = WidthBook::default();
        book.settle("a", 1, 359.0, None, |_| unreachable!());
        // A later call never re-measures, whatever the run looks like now.
        let width = book.settle("a", 5, 100.0, Some(200.0), |_| {
            panic!("a frozen width is not re-measured")
        });
        assert_eq!(width, 359.0);
    }

    #[test]
    fn a_cap_change_decides_again() {
        let mut book = WidthBook::default();
        book.retain_fingerprint(1);
        book.settle("a", 1, 359.0, None, |_| unreachable!());
        book.retain_fingerprint(2);
        let width = book.settle("a", 1, 200.0, None, |_| unreachable!());
        assert_eq!(width, 200.0);
    }
}
