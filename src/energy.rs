//! Whether the OS may pause Solmu to save power.
//!
//! While a window is shown, Solmu holds an NSProcessInfo activity that opts
//! it out of App Nap, so messaging and the managed connection stay prompt for
//! someone who is looking at it. With the window hidden to the tray or
//! minimized, the activity is released and macOS may pause Solmu. The System
//! setting "Allow App Nap" lifts the activity in the foreground too.

use std::sync::atomic::{AtomicBool, Ordering};

static ALLOWED: AtomicBool = AtomicBool::new(false);
static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(true);

/// Lets the OS pause Solmu at any time (`true`), or only while no window is
/// shown (`false`).
pub fn set_app_nap_allowed(allowed: bool) {
    ALLOWED.store(allowed, Ordering::Relaxed);
    sync();
}

/// Tells the power policy whether a Solmu window is shown (not hidden to the
/// tray and not minimized). Cheap to call every frame.
pub fn set_window_visible(visible: bool) {
    if WINDOW_VISIBLE.swap(visible, Ordering::Relaxed) != visible {
        sync();
    }
}

fn sync() {
    imp::set_held(assertion_required(
        ALLOWED.load(Ordering::Relaxed),
        WINDOW_VISIBLE.load(Ordering::Relaxed),
    ));
}

/// Whether Solmu must hold the power assertion: only while a window is shown,
/// and never when the setting allows App Nap. Kept as its own function so the
/// policy is testable.
#[allow(dead_code)]
pub(crate) fn assertion_required(allow_app_nap: bool, window_visible: bool) -> bool {
    !allow_app_nap && window_visible
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::Mutex;

    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    /// The held activity, made sendable for the static: the token is only
    /// ever handed back to NSProcessInfo, never inspected.
    struct SendableActivity(
        objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>>,
    );
    unsafe impl Send for SendableActivity {}

    /// endActivity is always called before release: dropping the object
    /// alone does not end the assertion.
    static ACTIVITY: Mutex<Option<SendableActivity>> = Mutex::new(None);

    pub(super) fn set_held(held: bool) {
        let mut slot = ACTIVITY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match (held, slot.take()) {
            (false, Some(activity)) => unsafe {
                NSProcessInfo::processInfo().endActivity(&activity.0);
            },
            (true, None) => {
                let reason = NSString::from_str("Solmu is open in a window");
                let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                    &reason,
                );
                *slot = Some(SendableActivity(activity));
            }
            (_, activity) => *slot = activity,
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub(super) fn set_held(_held: bool) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_assertion_is_held_only_while_a_window_is_shown_and_not_allowed_to_nap() {
        assert!(super::assertion_required(false, true));
        assert!(!super::assertion_required(false, false));
        assert!(!super::assertion_required(true, true));
        assert!(!super::assertion_required(true, false));
    }
}
