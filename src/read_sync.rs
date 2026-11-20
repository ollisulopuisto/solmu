//! Per-connection suppression of terminal read-state endpoints. A rejected
//! source never prevents another linked source from being synchronized.
use std::collections::HashSet;

#[derive(Default)]
pub struct ReadSync {
    unavailable: HashSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Synced,
    Unavailable,
    Skipped,
    Failed(String),
}

impl ReadSync {
    pub fn mark(
        &mut self,
        chat: &str,
        read_only: bool,
        mut send: impl FnMut() -> Result<(), String>,
    ) -> Outcome {
        if read_only || self.unavailable.contains(chat) {
            return Outcome::Skipped;
        }
        match send() {
            Ok(()) => Outcome::Synced,
            Err(error) if error.contains("returned 404") || error.contains("refused: 404") => {
                self.unavailable.insert(chat.to_owned());
                Outcome::Unavailable
            }
            Err(error) => Outcome::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_source_is_attempted_once_and_never_claims_a_receipt() {
        let mut sync = ReadSync::default();
        let mut calls = 0;
        for expected in [Outcome::Unavailable, Outcome::Skipped] {
            assert_eq!(
                sync.mark("missing", false, || {
                    calls += 1;
                    Err("Beeper API POST returned 404 Not Found".into())
                }),
                expected
            );
        }
        assert_eq!(calls, 1);
        assert_eq!(sync.mark("other", false, || Ok(())), Outcome::Synced);
    }
    #[test]
    fn transient_and_permission_errors_stay_visible_and_retryable() {
        let mut sync = ReadSync::default();
        for error in [
            "Beeper API POST returned 403 Forbidden",
            "Beeper API POST returned 500 Internal Server Error",
            "connection failed",
        ] {
            assert_eq!(
                sync.mark("chat", false, || Err(error.into())),
                Outcome::Failed(error.into())
            );
        }
        assert_eq!(sync.mark("chat", false, || Ok(())), Outcome::Synced);
    }
    #[test]
    fn a_new_connection_can_retry_and_readonly_never_posts() {
        let mut sync = ReadSync::default();
        assert_eq!(
            sync.mark("chat", true, || panic!("readonly POST")),
            Outcome::Skipped
        );
        let mut fresh = ReadSync::default();
        assert_eq!(fresh.mark("chat", false, || Ok(())), Outcome::Synced);
    }
}
