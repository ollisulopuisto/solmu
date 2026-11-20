//! Explicit write grants. Read scopes and unrestricted read account access never imply send access.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub account: String,
    pub chat: String,
}
pub const SEND_SCOPE: &str = "messages:send";
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    pub account_id: String,
    pub text: String,
    pub request_id: String,
}
impl SendRequest {
    pub fn validate(&self) -> bool {
        !self.account_id.is_empty()
            && self.account_id.len() <= 120
            && !self.text.trim().is_empty()
            && self.text.len() <= MAX_TEXT_BYTES
            && !self.request_id.is_empty()
            && self.request_id.len() <= 80
            && self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}
pub fn permitted(
    scopes: &[String],
    destinations: &[Destination],
    account: &str,
    chat: &str,
) -> bool {
    scopes.iter().any(|s| s == SEND_SCOPE)
        && destinations
            .iter()
            .any(|d| d.account == account && d.chat == chat)
}
pub fn valid_destinations(destinations: &[Destination]) -> bool {
    !destinations.is_empty()
        && destinations.len() <= 50
        && destinations.iter().all(|d| {
            !d.account.is_empty()
                && d.account.len() <= 120
                && !d.chat.is_empty()
                && d.chat.len() <= 512
                && !d.account.chars().any(char::is_control)
                && !d.chat.chars().any(char::is_control)
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_read_tokens_and_empty_write_grants_do_not_send() {
        let d = vec![Destination {
            account: "a".into(),
            chat: "c".into(),
        }];
        assert!(!permitted(&["messages:read".into()], &d, "a", "c"));
        assert!(!permitted(&[SEND_SCOPE.into()], &[], "a", "c"));
        assert!(!permitted(&[SEND_SCOPE.into()], &d, "b", "c"));
        assert!(!permitted(&[SEND_SCOPE.into()], &d, "a", "d"));
        assert!(permitted(&[SEND_SCOPE.into()], &d, "a", "c"));
    }
    #[test]
    fn request_is_bounded_and_unknown_fields_rejected() {
        let mut r = SendRequest {
            account_id: "a".into(),
            text: "hello".into(),
            request_id: "request-1".into(),
        };
        assert!(r.validate());
        r.text = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(!r.validate());
        r.text = " ".into();
        assert!(!r.validate());
        assert!(
            serde_json::from_str::<SendRequest>(
                r#"{"account_id":"a","text":"hi","request_id":"x","files":["secret"]}"#
            )
            .is_err()
        );
    }
}
