//! Session-only presentation of long message bodies. Message content stays whole.
pub const ROW_LIMIT: usize = 25;
pub fn visible_rows(total: usize, expanded: bool) -> usize {
    if expanded {
        total
    } else {
        total.min(ROW_LIMIT)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_bodies_hide_only_rows_after_twenty_five_and_expand_without_loss() {
        for total in [0, 1, 24, 25, 26, 34, 1000] {
            assert_eq!(visible_rows(total, false), total.min(ROW_LIMIT));
            assert_eq!(visible_rows(total, true), total);
            assert_eq!(total - visible_rows(total, false), total.saturating_sub(25));
        }
    }
}
