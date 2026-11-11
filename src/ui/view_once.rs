//! Composer visibility only. The Beeper send API cannot express view-once.
pub(super) fn visible(editing: bool, beeper: bool, eligible: bool, selected: bool) -> bool {
    !editing && !beeper && (eligible || selected)
}

#[cfg(test)]
mod tests {
    use super::visible;

    #[test]
    fn beeper_never_offers_view_once_even_for_restored_drafts() {
        for eligible in [false, true] {
            for selected in [false, true] {
                assert!(!visible(false, true, eligible, selected));
            }
        }
    }

    #[test]
    fn legacy_photo_drafts_and_editing_keep_their_existing_behavior() {
        assert!(visible(false, false, true, false));
        assert!(visible(false, false, false, true));
        assert!(!visible(false, false, false, false));
        assert!(!visible(true, false, true, true));
    }
}
