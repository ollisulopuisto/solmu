//! Cached model presence. The settings view only reads this state.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Presence {
    value: Arc<Mutex<(u64, Option<bool>)>>,
}

impl Presence {
    #[cfg(any(test, feature = "demo"))]
    pub fn known(present: bool) -> Self {
        Self {
            value: Arc::new(Mutex::new((0, Some(present)))),
        }
    }
    pub fn get(&self) -> Option<bool> {
        self.value.lock().unwrap_or_else(|p| p.into_inner()).1
    }

    pub fn refresh(&self, path: PathBuf, wake: impl FnOnce() + Send + 'static) {
        self.refresh_with(move || path.is_file(), wake);
    }

    fn refresh_with(
        &self,
        probe: impl FnOnce() -> bool + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) {
        let generation = {
            let mut state = self.value.lock().unwrap_or_else(|p| p.into_inner());
            state.0 = state.0.wrapping_add(1);
            state.1 = None;
            state.0
        };
        let value = Arc::clone(&self.value);
        std::thread::spawn(move || {
            let present = probe();
            let mut state = value.lock().unwrap_or_else(|p| p.into_inner());
            if state.0 == generation {
                state.1 = Some(present);
            }
            drop(state);
            wake();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn blocked_probe_does_not_block_refresh_or_cached_reads() {
        let presence = Arc::new(Presence::default());
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (returned_tx, returned_rx) = channel();
        let (wake_tx, wake_rx) = channel();
        let worker = Arc::clone(&presence);
        let handle = std::thread::spawn(move || {
            worker.refresh_with(
                move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    true
                },
                move || {
                    wake_tx.send(()).unwrap();
                },
            );
            returned_tx.send(()).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let returned_without_disk = returned_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        if returned_without_disk {
            for _ in 0..1000 {
                assert_eq!(presence.get(), None);
            }
        }
        release_tx.send(()).unwrap();
        handle.join().unwrap();
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            returned_without_disk,
            "refresh waited for a blocked filesystem probe"
        );
        assert_eq!(presence.get(), Some(true));
    }
    #[test]
    fn newer_refresh_wins_over_an_older_blocked_probe() {
        let presence = Presence::default();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (old_wake_tx, old_wake_rx) = channel();
        let (new_wake_tx, new_wake_rx) = channel();
        presence.refresh_with(
            move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                true
            },
            move || {
                old_wake_tx.send(()).unwrap();
            },
        );
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        presence.refresh_with(
            || false,
            move || {
                new_wake_tx.send(()).unwrap();
            },
        );
        new_wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(presence.get(), Some(false));
        release_tx.send(()).unwrap();
        old_wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            presence.get(),
            Some(false),
            "stale existence probe must not resurrect removed model"
        );
    }

    #[test]
    fn install_and_removal_refresh_real_metadata_and_wake() {
        let path = std::env::temp_dir().join(format!("solmu-presence-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let presence = Presence::default();
        for exists in [false, true, false] {
            if exists {
                std::fs::write(&path, b"synthetic model marker").unwrap();
            } else {
                let _ = std::fs::remove_file(&path);
            }
            let (tx, rx) = channel();
            presence.refresh(path.clone(), move || {
                tx.send(()).unwrap();
            });
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(presence.get(), Some(exists));
        }
    }
}
