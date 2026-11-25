//! Exact album member targeting across secondary-button transitions.
pub(super) fn update(
    target: &mut Option<String>,
    owned: bool,
    pressed: bool,
    clicked: bool,
    hit: Option<String>,
) {
    if owned && (pressed || clicked) {
        *target = hit;
    }
}
#[cfg(test)]
mod tests {
    use super::update;
    #[test]
    fn floating_menu_cannot_retarget_an_album_action() {
        let mut target = None;
        update(&mut target, true, true, false, Some("album-2".into()));
        update(&mut target, false, true, false, Some("album-0".into()));
        update(&mut target, false, false, true, Some("album-0".into()));
        assert_eq!(target.as_deref(), Some("album-2"));
    }
    #[test]
    fn a_no_hit_release_drops_the_old_member_before_the_next_action() {
        let mut target = Some("album-2".into());
        update(&mut target, true, false, true, None);
        assert_eq!(target, None);
        update(&mut target, true, true, false, Some("album-1".into()));
        assert_eq!(target.as_deref(), Some("album-1"));
    }
}
