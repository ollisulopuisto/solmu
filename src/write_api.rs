//! One bounded text write. All entry points share authorization, validation, audit and no-retry semantics.
use crate::{
    read_api::{Registry, Upstream, hash_token},
    write_grants::{SendRequest, permitted},
    write_journal::{Claim, Outcome, WriteJournal},
};
fn response(status: u16, state: &str, id: Option<&str>) -> (u16, String) {
    (status,serde_json::json!({"state":state,"message_id":id,"delivery_confirmed":false,"automatic_retry":false}).to_string())
}
#[allow(clippy::too_many_arguments)]
pub fn handle(
    registry: &Registry,
    upstream: &mut dyn Upstream,
    journal: &WriteJournal,
    bearer: Option<&str>,
    chat: &str,
    body: &[u8],
    now: i64,
    revalidate: impl FnMut() -> bool,
) -> (u16, String) {
    if journal
        .audit_attempt(&bearer.map(hash_token).unwrap_or_default(), now)
        .is_err()
    {
        return response(503, "audit_unavailable", None);
    }
    handle_recorded(
        registry, upstream, journal, bearer, chat, body, now, revalidate,
    )
}
/// HTTP recorded the receipt before its early refusals. Direct callers use handle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_recorded(
    registry: &Registry,
    upstream: &mut dyn Upstream,
    journal: &WriteJournal,
    bearer: Option<&str>,
    chat: &str,
    body: &[u8],
    now: i64,
    mut revalidate: impl FnMut() -> bool,
) -> (u16, String) {
    let Some(token) = bearer else {
        return response(401, "unauthorized", None);
    };
    let Some(client) = registry.authenticate(token, now) else {
        return response(401, "unauthorized", None);
    };
    let Ok(request) = serde_json::from_slice::<SendRequest>(body) else {
        return response(400, "invalid_request", None);
    };
    if !request.validate()
        || chat.is_empty()
        || chat.len() > 512
        || chat.chars().any(char::is_control)
    {
        return response(400, "invalid_request", None);
    }
    if !permitted(
        &client.scopes,
        &client.send_destinations,
        &request.account_id,
        chat,
    ) {
        return response(403, "destination_not_granted", None);
    }
    let encoded: String = chat
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let Ok((status, metadata)) = upstream.get(&format!("/v1/chats/{encoded}")) else {
        return response(502, "destination_unverified", None);
    };
    if status != 200
        || serde_json::from_str::<serde_json::Value>(&metadata)
            .ok()
            .and_then(|v| {
                v.get("accountID")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .as_deref()
            != Some(request.account_id.as_str())
    {
        return response(403, "destination_unverified", None);
    }
    // Registry is reloaded after potentially blocking upstream identity lookup.
    if !revalidate() {
        return response(401, "authorization_changed", None);
    }
    let digest =
        hash_token(&serde_json::json!([request.account_id, chat, request.text]).to_string());
    let token_hash = hash_token(token);
    match journal.claim(&token_hash, &request.request_id, &digest, now) {
        Ok(Claim::Existing(outcome)) => return existing(outcome),
        Err(_) => return response(409, "request_conflict_or_audit_unavailable", None),
        Ok(Claim::New) => {}
    }
    // Disk claim may block too. Recheck once more at the last boundary before sending.
    if !revalidate() {
        let _ = journal.finish(&token_hash, &request.request_id, Outcome::Rejected);
        return response(401, "authorization_changed", None);
    }
    let outcome = upstream.send_text(chat, &request.text);
    if journal
        .finish(&token_hash, &request.request_id, outcome.clone())
        .is_err()
    {
        return response(502, "outcome_unknown_audit_failed_do_not_retry", None);
    }
    existing(outcome)
}
fn existing(outcome: Outcome) -> (u16, String) {
    match outcome {
        Outcome::Accepted { message_id } => response(200, "accepted", Some(&message_id)),
        Outcome::Rejected => response(400, "rejected", None),
        Outcome::Claimed | Outcome::Unknown => response(409, "outcome_unknown_do_not_retry", None),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_grants::Destination;
    struct Mock {
        reads: usize,
        writes: usize,
    }
    impl Upstream for Mock {
        fn get(&mut self, _: &str) -> Result<(u16, String), String> {
            self.reads += 1;
            Ok((200, r#"{"accountID":"a"}"#.into()))
        }
        fn send_text(&mut self, _: &str, _: &str) -> Outcome {
            self.writes += 1;
            Outcome::Accepted {
                message_id: "m".into(),
            }
        }
    }
    fn setup() -> (Registry, String, tempfile::TempDir, WriteJournal) {
        let mut r = Registry::default();
        let token = r
            .add_with_destinations(
                "app",
                vec!["messages:send".into()],
                vec!["a".into()],
                vec![Destination {
                    account: "a".into(),
                    chat: "c".into(),
                }],
                None,
                1,
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let j = WriteJournal::new(dir.path().join("audit.json"));
        (r, token, dir, j)
    }
    const BODY: &[u8] = br#"{"account_id":"a","text":"hi","request_id":"request-1"}"#;
    #[test]
    fn same_id_is_not_sent_twice() {
        let (r, t, _d, j) = setup();
        let mut u = Mock {
            reads: 0,
            writes: 0,
        };
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || true).0,
            200
        );
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || true).0,
            200
        );
        assert_eq!(u.writes, 1);
    }
    #[test]
    fn revocation_after_lookup_and_wrong_destination_never_post() {
        let (r, t, _d, j) = setup();
        let mut u = Mock {
            reads: 0,
            writes: 0,
        };
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "wrong", BODY, 2, || true).0,
            403
        );
        assert_eq!(u.reads, 0);
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || false).0,
            401
        );
        assert_eq!(u.writes, 0);
    }
    #[test]
    fn expiry_audit_failure_and_late_revocation_never_post() {
        let (mut r, t, d, j) = setup();
        let mut u = Mock {
            reads: 0,
            writes: 0,
        };
        r.clients[0].expires_at = Some(2);
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || true).0,
            401
        );
        r.clients[0].expires_at = None;
        let unavailable = WriteJournal::new(d.path().join("absent/audit.json"));
        assert_eq!(
            handle(&r, &mut u, &unavailable, Some(&t), "c", BODY, 2, || true).0,
            503
        );
        assert_eq!(u.reads, 0);
        let mut checks = 0;
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || {
                checks += 1;
                checks == 1
            })
            .0,
            401
        );
        assert_eq!(checks, 2);
        assert_eq!(u.writes, 0);
    }
    #[test]
    fn unknown_outcomes_are_durable_and_never_resent() {
        struct Unknown(usize);
        impl Upstream for Unknown {
            fn get(&mut self, _: &str) -> Result<(u16, String), String> {
                Ok((200, r#"{"accountID":"a"}"#.into()))
            }
            fn send_text(&mut self, _: &str, _: &str) -> Outcome {
                self.0 += 1;
                Outcome::Unknown
            }
        }
        let (r, t, d, j) = setup();
        let mut u = Unknown(0);
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || true).0,
            409
        );
        let j = WriteJournal::new(d.path().join("audit.json"));
        assert_eq!(
            handle(&r, &mut u, &j, Some(&t), "c", BODY, 2, || true).0,
            409
        );
        assert_eq!(u.0, 1);
    }
}
