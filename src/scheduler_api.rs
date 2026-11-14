//! Server-API implementation of the Send Later scheduler's send channel.
//!
//! Talks to the local Beeper Server/Desktop API exactly like the app's own
//! backend: bearer token, POST /v1/chats/{chatID}/messages for sends and
//! GET /v1/messages/search for reconciliation. Error mapping preserves the
//! one distinction the scheduler's safety rules depend on: a definite
//! rejection (HTTP error status) versus an unknown outcome (timeout,
//! dropped connection, unreadable response), which may have been delivered
//! and must never be blindly retried.

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};

use crate::scheduler::{Reconcile, ScheduledMessage, Send, SendError};

/// Long enough for a slow bridge, short enough that a wedged connection
/// fails the tick instead of the whole scheduler.
const REQUEST_TIMEOUT_SECS: u64 = 30;
/// Search responses are bounded; reconciliation never pages.
const MAX_BODY_BYTES: usize = 1 << 20;

#[derive(Clone)]
pub struct ServerSender {
    base_url: String,
    http: Client,
}

#[derive(serde::Deserialize)]
struct SearchPage {
    #[serde(default)]
    items: Vec<SearchMessage>,
}

#[derive(serde::Deserialize)]
struct SearchMessage {
    #[serde(rename = "chatID", default)]
    chat_id: String,
    #[serde(rename = "accountID", default)]
    account_id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    is_sender: Option<bool>,
    #[serde(default)]
    timestamp: String,
}

impl ServerSender {
    pub fn new(base_url: String, token: &str) -> Result<Self, String> {
        if token.is_empty() {
            return Err("Beeper access token missing".into());
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| format!("invalid token characters: {e}"))?;
        auth.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        let http = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Reads at most MAX_BODY_BYTES so a misbehaving local server cannot
    /// exhaust memory; the cap applies while streaming, not after the whole
    /// body is already in memory.
    fn bounded_body(mut response: reqwest::blocking::Response) -> Result<String, SendError> {
        use std::io::Read;
        let mut buf = Vec::new();
        response
            .by_ref()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut buf)
            .map_err(|e| SendError::Unknown(format!("response unreadable: {e}")))?;
        if buf.len() > MAX_BODY_BYTES {
            return Err(SendError::Unknown("response too large".into()));
        }
        String::from_utf8(buf).map_err(|e| SendError::Unknown(format!("response not utf-8: {e}")))
    }

    /// One RFC3339 timestamp, as the API emits them, to Unix seconds.
    /// Unparseable timestamps never match reconciliation.
    fn unix_seconds(timestamp: &str) -> Option<i64> {
        timestamp
            .parse::<jiff::Timestamp>()
            .ok()
            .map(|t| t.as_second())
    }
}

impl ServerSender {
    /// One bounded GET for the read-only companion. Returns the status and
    /// body, or a transport error that carries no response content.
    pub fn get_bounded(&self, target: &str) -> Result<(u16, String), String> {
        let url = format!("{}{target}", self.base_url);
        let response = self
            .http
            .get(url)
            .send()
            .map_err(|_| "server unreachable".to_string())?;
        let status = response.status().as_u16();
        let body = Self::bounded_body(response).map_err(|_| "response unusable".to_string())?;
        Ok((status, body))
    }

    /// Enqueue-time validation: the chat must exist on the Server and
    /// belong to the stated account, so a mistargeted item fails at
    /// scheduling time rather than sending somewhere wrong at due time.
    pub fn validate_chat(&mut self, chat_id: &str, account_id: &str) -> Result<(), String> {
        #[derive(serde::Deserialize)]
        struct Chat {
            #[serde(rename = "accountID", default)]
            account_id: String,
        }
        let url = format!("{}/v1/chats/{}", self.base_url, encode_segment(chat_id));
        let response = self
            .http
            .get(url)
            .send()
            .map_err(|e| format!("validation unreachable: {e}"))?;
        let status = response.status();
        let body = Self::bounded_body(response).map_err(|e| match e {
            SendError::Rejected(m) | SendError::Unknown(m) => m,
        })?;
        if status.as_u16() == 404 {
            return Err("chat not found on the server".into());
        }
        if !status.is_success() {
            return Err(format!("validation failed: HTTP {status}"));
        }
        let chat: Chat = serde_json::from_str(&body)
            .map_err(|e| format!("validation response unreadable: {e}"))?;
        if chat.account_id != account_id {
            return Err(format!(
                "chat belongs to account {}, not {account_id}",
                chat.account_id
            ));
        }
        Ok(())
    }
}

impl Send for ServerSender {
    fn send(&mut self, item: &ScheduledMessage) -> Result<String, SendError> {
        let mut payload = serde_json::json!({"text": item.text});
        if let Some(reply) = &item.reply_to {
            payload["replyToMessageID"] = reply.clone().into();
        }
        let url = format!(
            "{}/v1/chats/{}/messages",
            self.base_url,
            encode_segment(&item.chat_id)
        );
        let response = self
            .http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(payload.to_string())
            .send()
            .map_err(|e| SendError::Unknown(format!("send outcome unknown: {e}")))?;
        let status = response.status();
        let body = Self::bounded_body(response)?;
        if !status.is_success() {
            // An HTTP error status means the server rejected the request;
            // nothing went out. Truncate server detail for the error field.
            let detail: String = body.chars().take(200).collect();
            return Err(SendError::Rejected(format!("HTTP {status}: {detail}")));
        }
        let accepted: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
            SendError::Unknown("send accepted but response unreadable; reconcile".into())
        })?;
        accepted
            .get("pendingMessageID")
            .or_else(|| accepted.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_string)
            .ok_or_else(|| SendError::Unknown("send accepted without a message id".into()))
    }

    fn reconcile(&mut self, item: &ScheduledMessage) -> Reconcile {
        let url = format!("{}/v1/messages/search", self.base_url);
        let response = self
            .http
            .get(url)
            .query(&[("query", item.text.as_str())])
            .send();
        let Ok(response) = response else {
            return Reconcile::Unknown;
        };
        if !response.status().is_success() {
            return Reconcile::Unknown;
        }
        let Ok(body) = Self::bounded_body(response) else {
            return Reconcile::Unknown;
        };
        let Ok(page) = serde_json::from_str::<SearchPage>(&body) else {
            return Reconcile::Unknown;
        };
        let found = page.items.iter().any(|message| {
            message.chat_id == item.chat_id
                && message.account_id == item.account_id
                && message.text.as_deref() == Some(item.text.as_str())
                && message.is_sender == Some(true)
                && Self::unix_seconds(&message.timestamp)
                    .map(|at| at >= item.due_at - 60)
                    .unwrap_or(false)
        });
        if found {
            Reconcile::Found
        } else {
            Reconcile::NotFound
        }
    }
}

/// Percent-encode one path segment, as the backend does for chat ids.
fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let ok = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if ok {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The daemon's control protocol, without the socket: one request in,
/// (HTTP status, response body, queue changed) out. The daemon persists
/// when `changed` is true. Testable without a listener.
// Flat on purpose: this is the daemon's protocol seam, exercised directly
// by the tests; bundling the closures would obscure it.
#[allow(clippy::too_many_arguments)]
pub fn control(
    queue: &mut crate::scheduler::Queue,
    method: &str,
    path: &str,
    body: &str,
    now: i64,
    new_id: &dyn Fn() -> String,
    validate: &mut dyn FnMut(&str, &str) -> Result<(), String>,
    reconcile: &mut dyn FnMut(&ScheduledMessage) -> Reconcile,
) -> (u16, String, bool) {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (method, segments.as_slice()) {
        ("GET", ["v1", "health"]) => (200, r#"{"ok":true}"#.to_string(), false),
        ("GET", ["v1", "scheduled"]) => {
            let body = serde_json::to_string(&queue.items).unwrap_or_else(|_| "[]".into());
            (200, body, false)
        }
        ("POST", ["v1", "scheduled"]) => {
            let parsed: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return (400, format!(r#"{{"error":"invalid json: {e}"}}"#), false);
                }
            };
            let get_str = |key: &str| parsed.get(key).and_then(|v| v.as_str()).map(str::to_string);
            let (Some(account_id), Some(chat_id), Some(text), Some(due_at)) = (
                get_str("account_id"),
                get_str("chat_id"),
                get_str("text"),
                parsed.get("due_at").and_then(|v| v.as_i64()),
            ) else {
                return (
                    400,
                    r#"{"error":"need account_id, chat_id, text, due_at"}"#.to_string(),
                    false,
                );
            };
            // The chat must exist and belong to the account BEFORE the
            // item is queued; a mismatch must never reach the send path.
            if let Err(e) = validate(&chat_id, &account_id) {
                return (400, format!(r#"{{"error":"invalid target: {e}"}}"#), false);
            }
            let mut item = crate::scheduler::ScheduledMessage::new(
                new_id(),
                account_id,
                chat_id,
                text,
                get_str("reply_to"),
                due_at,
                now,
            );
            // A repeating series: one chat, one rule, validated before it
            // is queued. The first occurrence's id names the series.
            if let Some(raw) = parsed.get("repeat").filter(|v| !v.is_null()) {
                let mut repeat: crate::recurrence::Repeat =
                    match serde_json::from_value(raw.clone()) {
                        Ok(repeat) => repeat,
                        Err(e) => {
                            return (400, format!(r#"{{"error":"invalid repeat: {e}"}}"#), false);
                        }
                    };
                repeat.series = item.id.clone();
                repeat.sent = 0;
                repeat.paused = false;
                if let Err(e) = repeat.validate() {
                    return (400, format!(r#"{{"error":"invalid repeat: {e}"}}"#), false);
                }
                item.repeat = Some(repeat);
            }
            let id = item.id.clone();
            let replaces = parsed.get("replaces").and_then(|v| v.as_str());
            let result = match replaces {
                Some(replaces) => queue.add_replacing(item, replaces),
                None => queue.add(item).map(|_| id),
            };
            match result {
                Ok(id) => (200, format!(r#"{{"id":"{id}"}}"#), true),
                Err(e) => (409, format!(r#"{{"error":"{e:?}"}}"#), false),
            }
        }
        ("POST", ["v1", "scheduled", id, action]) => {
            let result = match *action {
                "cancel" => queue.cancel(id),
                "retry" => {
                    // An Uncertain item may already have reached the chat, so
                    // a blind retry can duplicate it. Reconcile first, or make
                    // the caller confirm it accepts the possible duplicate.
                    let uncertain = matches!(
                        queue.get(id).map(|item| item.state),
                        Some(crate::scheduler::State::Uncertain)
                    );
                    if uncertain {
                        let parsed: serde_json::Value =
                            serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
                        let confirmed = parsed
                            .get("confirm_duplicate")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !confirmed {
                            return (
                                409,
                                r#"{"error":"item is Uncertain: a copy may already have reached the chat; run reconcile first or retry with {\"confirm_duplicate\":true} accepting a possible duplicate"}"#.to_string(),
                                false,
                            );
                        }
                    }
                    queue.retry(id, now)
                }
                "reconcile" => {
                    let Some(item) = queue.get(id) else {
                        let e = crate::scheduler::QueueError::NotFound(id.to_string());
                        return (409, format!(r#"{{"error":"{e:?}"}}"#), false);
                    };
                    if item.state != crate::scheduler::State::Uncertain {
                        let e = crate::scheduler::QueueError::BadState {
                            id: id.to_string(),
                            state: item.state,
                        };
                        return (409, format!(r#"{{"error":"{e:?}"}}"#), false);
                    }
                    return match reconcile(item) {
                        Reconcile::Found => {
                            queue.get_mut(id).expect("item checked above").state =
                                crate::scheduler::State::Sent;
                            queue.spawn_next(id, now);
                            (200, r#"{"reconcile":"found"}"#.to_string(), true)
                        }
                        Reconcile::NotFound => {
                            (200, r#"{"reconcile":"not_found"}"#.to_string(), false)
                        }
                        Reconcile::Unknown => {
                            (200, r#"{"reconcile":"unknown"}"#.to_string(), false)
                        }
                    };
                }
                "edit" => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
                    let text = parsed
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let due_at = parsed.get("due_at").and_then(|v| v.as_i64());
                    queue.edit(id, text, due_at)
                }
                "send-now" => queue.edit(id, None, Some(now)),
                "skip" => queue.skip(id, now),
                "pause" => queue.set_paused(id, true),
                "resume" => queue.set_paused(id, false),
                _ => return (404, r#"{"error":"unknown action"}"#.to_string(), false),
            };
            match result {
                Ok(()) => (200, r#"{"ok":true}"#.to_string(), true),
                Err(e) => (409, format!(r#"{{"error":"{e:?}"}}"#), false),
            }
        }
        _ => (404, r#"{"error":"not found"}"#.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::Queue;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{Receiver, channel};
    use std::thread;

    /// One-shot HTTP stub: serves each queued response in order and records
    /// the raw requests it received.
    struct Stub {
        base_url: String,
        requests: Receiver<String>,
    }

    fn stub(responses: Vec<String>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read until headers end; test bodies are small and sent with
                // the headers by reqwest's blocking client.
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_header_end(&buf) {
                        if let Some(len) = content_length(&buf[..pos]) {
                            if buf.len() >= pos + 4 + len {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Stub {
            base_url: format!("http://127.0.0.1:{port}"),
            requests: rx,
        }
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn content_length(headers: &[u8]) -> Option<usize> {
        let text = String::from_utf8_lossy(headers);
        text.lines()
            .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
            .and_then(|line| line.split(':').nth(1)?.trim().parse().ok())
    }

    fn ok_json(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn item() -> ScheduledMessage {
        ScheduledMessage::new(
            "sched-1".into(),
            "wa-main".into(),
            "chat 9/4".into(),
            "hello later".into(),
            Some("reply-1".into()),
            1_762_000_000,
            1_761_999_000,
        )
    }

    fn sender(base_url: &str) -> ServerSender {
        ServerSender::new(base_url.to_string(), "secret-token").unwrap()
    }

    mod protocol {
        use super::super::control;
        use crate::scheduler::{Queue, Reconcile, ScheduledMessage, State};

        fn no_reconcile() -> impl FnMut(&ScheduledMessage) -> Reconcile {
            |_| Reconcile::Unknown
        }

        fn queue() -> Queue {
            Queue::default()
        }

        fn ids() -> impl Fn() -> String {
            let n = std::cell::Cell::new(0u32);
            move || {
                n.set(n.get() + 1);
                format!("test-{}", n.get())
            }
        }

        #[test]
        fn health_and_empty_list() {
            let mut q = queue();
            assert_eq!(
                control(
                    &mut q,
                    "GET",
                    "/v1/health",
                    "",
                    0,
                    &|| "x".into(),
                    &mut |_, _| Ok(()),
                    &mut no_reconcile()
                )
                .0,
                200
            );
            let (status, body, changed) = control(
                &mut q,
                "GET",
                "/v1/scheduled",
                "",
                0,
                &|| "x".into(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!((status, body.as_str(), changed), (200, "[]", false));
        }

        #[test]
        fn add_then_list_then_cancel() {
            let mut q = queue();
            let (status, body, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled",
                r#"{"account_id":"wa","chat_id":"c1","text":"hi","due_at":100}"#,
                50,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!((status, changed), (200, true));
            assert!(body.contains("test-1"));
            assert_eq!(q.items.len(), 1);
            let (status, _, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/test-1/cancel",
                "",
                50,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!((status, changed), (200, true));
            assert_eq!(q.items[0].state, State::Cancelled);
            // A cancelled item cannot be cancelled again.
            let (status, _, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/test-1/cancel",
                "",
                50,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!((status, changed), (409, false));
        }

        #[test]
        fn missing_fields_and_bad_json_are_400() {
            let mut q = queue();
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled",
                "{not json",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 400);
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled",
                r#"{"text":"hi"}"#,
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 400);
            assert!(q.items.is_empty());
        }

        #[test]
        fn edit_and_send_now() {
            let mut q = queue();
            q.add(ScheduledMessage::new(
                "a".into(),
                "wa".into(),
                "c1".into(),
                "hi".into(),
                None,
                1_000,
                0,
            ))
            .unwrap();
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/edit",
                r#"{"text":"changed","due_at":2000}"#,
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 200);
            assert_eq!(q.items[0].text, "changed");
            assert_eq!(q.items[0].due_at, 2000);
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/send-now",
                "",
                555,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 200);
            assert_eq!(q.items[0].due_at, 555);
            assert_eq!(q.items[0].state, State::Scheduled);
        }

        #[test]
        fn enqueue_validates_chat_and_account() {
            let mut q = queue();
            let add = r#"{"account_id":"wa","chat_id":"c1","text":"hi","due_at":100}"#;
            // Validation failure: nothing queued, clear 400.
            let (status, body, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled",
                add,
                50,
                &ids(),
                &mut |_, _| Err("chat belongs to account other".into()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 400);
            assert!(body.contains("invalid target"));
            assert!(!changed);
            assert!(q.items.is_empty());
            // The validator sees the exact chat and account.
            let mut seen = Vec::new();
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled",
                add,
                50,
                &ids(),
                &mut |chat, account| {
                    seen.push((chat.to_string(), account.to_string()));
                    Ok(())
                },
                &mut no_reconcile(),
            );
            assert_eq!(status, 200);
            assert_eq!(seen, vec![("c1".to_string(), "wa".to_string())]);
        }

        #[test]
        fn unknown_paths_and_actions_are_404() {
            let mut q = queue();
            assert_eq!(
                control(
                    &mut q,
                    "GET",
                    "/v1/nope",
                    "",
                    0,
                    &ids(),
                    &mut |_, _| Ok(()),
                    &mut no_reconcile()
                )
                .0,
                404
            );
            assert_eq!(
                control(
                    &mut q,
                    "POST",
                    "/v1/scheduled/a/explode",
                    "",
                    0,
                    &ids(),
                    &mut |_, _| Ok(()),
                    &mut no_reconcile()
                )
                .0,
                404
            );
        }

        #[test]
        fn retry_of_failed_item() {
            let mut q = queue();
            let mut item = ScheduledMessage::new(
                "a".into(),
                "wa".into(),
                "c1".into(),
                "hi".into(),
                None,
                100,
                0,
            );
            item.state = State::Failed;
            q.add(item).unwrap();
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/retry",
                "",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 200);
            assert_eq!(q.items[0].state, State::Scheduled);
        }

        fn uncertain_queue() -> Queue {
            let mut q = queue();
            let mut item = ScheduledMessage::new(
                "a".into(),
                "wa".into(),
                "c1".into(),
                "hi".into(),
                None,
                100,
                0,
            );
            item.state = State::Uncertain;
            q.add(item).unwrap();
            q
        }

        #[test]
        fn retry_of_uncertain_requires_confirm_duplicate() {
            let mut q = uncertain_queue();
            let (status, body, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/retry",
                "{}",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 409);
            assert!(body.contains("confirm_duplicate"));
            assert!(!changed);
            assert_eq!(q.items[0].state, State::Uncertain);
            let (status, _, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/retry",
                r#"{"confirm_duplicate":true}"#,
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 200);
            assert!(changed);
            assert_eq!(q.items[0].state, State::Scheduled);
        }

        #[test]
        fn reconcile_found_marks_sent() {
            let mut q = uncertain_queue();
            let (status, body, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/reconcile",
                "",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut |_| Reconcile::Found,
            );
            assert_eq!(
                (status, body.as_str(), changed),
                (200, r#"{"reconcile":"found"}"#, true)
            );
            assert_eq!(q.items[0].state, State::Sent);
        }

        #[test]
        fn repeat_is_validated_then_skip_pause_and_resume_work() {
            let mut q = queue();
            let create = |q: &mut Queue, repeat: &str| {
                let body = format!(
                    r#"{{"account_id":"acc","chat_id":"c","text":"hi","due_at":100,"repeat":{repeat}}}"#
                );
                control(
                    q,
                    "POST",
                    "/v1/scheduled",
                    &body,
                    0,
                    &ids(),
                    &mut |_, _| Ok(()),
                    &mut no_reconcile(),
                )
            };
            // Interval 0 is refused and nothing is queued.
            let bad = r#"{"series":"","every":0,"unit":"Day","hour":9,"minute":0,"tz":"UTC"}"#;
            assert_eq!(create(&mut q, bad).0, 400);
            assert!(q.items.is_empty());
            let good = r#"{"series":"ignored","every":1,"unit":"Day","hour":9,"minute":0,"tz":"UTC","sent":7,"paused":true}"#;
            let (status, _, changed) = create(&mut q, good);
            assert_eq!((status, changed), (200, true));
            let repeat = q.items[0].repeat.clone().unwrap();
            // The daemon names the series and resets what a client could forge.
            assert_eq!(repeat.series, q.items[0].id);
            assert_eq!((repeat.sent, repeat.paused), (0, false));
            let id = q.items[0].id.clone();
            let act = |q: &mut Queue, action: &str| {
                control(
                    q,
                    "POST",
                    &format!("/v1/scheduled/{id}/{action}"),
                    "",
                    0,
                    &ids(),
                    &mut |_, _| Ok(()),
                    &mut no_reconcile(),
                )
            };
            assert_eq!(act(&mut q, "pause").0, 200);
            assert!(q.items[0].repeat.as_ref().unwrap().paused);
            assert_eq!(act(&mut q, "resume").0, 200);
            assert!(!q.items[0].repeat.as_ref().unwrap().paused);
            let before = q.items[0].due_at;
            assert_eq!(act(&mut q, "skip").0, 200);
            assert!(q.items[0].due_at > before);
            assert_eq!(q.items.len(), 1);
        }

        #[test]
        fn reconcile_not_found_keeps_uncertain() {
            let mut q = uncertain_queue();
            let (status, body, changed) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/reconcile",
                "",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut |_| Reconcile::NotFound,
            );
            assert_eq!(
                (status, body.as_str(), changed),
                (200, r#"{"reconcile":"not_found"}"#, false)
            );
            assert_eq!(q.items[0].state, State::Uncertain);
        }

        #[test]
        fn reconcile_rejects_non_uncertain_and_unknown_ids() {
            let mut q = queue();
            q.add(ScheduledMessage::new(
                "a".into(),
                "wa".into(),
                "c1".into(),
                "hi".into(),
                None,
                100,
                0,
            ))
            .unwrap();
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled/a/reconcile",
                "",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 409);
            let (status, _, _) = control(
                &mut q,
                "POST",
                "/v1/scheduled/ghost/reconcile",
                "",
                0,
                &ids(),
                &mut |_, _| Ok(()),
                &mut no_reconcile(),
            );
            assert_eq!(status, 409);
        }
    }

    #[test]
    fn send_posts_text_and_returns_pending_id() {
        let stub = stub(vec![ok_json(r#"{"pendingMessageID":"pm-1"}"#)]);
        let mut sender = sender(&stub.base_url);
        let id = sender.send(&item()).unwrap();
        assert_eq!(id, "pm-1");
        let request = stub.requests.recv().unwrap();
        assert!(request.starts_with("POST /v1/chats/chat%209%2F4/messages "));
        assert!(request.contains("authorization: Bearer secret-token"));
        assert!(request.contains(r#""text":"hello later""#));
        assert!(request.contains(r#""replyToMessageID":"reply-1""#));
    }

    #[test]
    fn http_error_is_a_rejection() {
        let stub = stub(vec![
            "HTTP/1.1 404 Not Found\r\ncontent-length: 11\r\nconnection: close\r\n\r\nunknown cha"
                .to_string(),
        ]);
        let mut sender = sender(&stub.base_url);
        match sender.send(&item()) {
            Err(SendError::Rejected(detail)) => assert!(detail.starts_with("HTTP 404")),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn connection_failure_is_unknown() {
        let mut sender = sender("http://127.0.0.1:1");
        match sender.send(&item()) {
            Err(SendError::Unknown(_)) => {}
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_matches_own_message_in_chat_after_due() {
        let body = r#"{"items":[
            {"chatID":"chat 9/4","accountID":"wa-main","text":"hello later","is_sender":true,"timestamp":"2025-11-01T12:26:41Z"},
            {"chatID":"chat 9/4","accountID":"wa-main","text":"hello later","is_sender":false,"timestamp":"2025-11-01T12:27:00Z"}
        ]}"#;
        let stub = stub(vec![ok_json(body)]);
        let mut sender = sender(&stub.base_url);
        // due_at 1_762_000_000 is 2025-11-01T12:26:40Z; the own message at +1s matches.
        assert_eq!(sender.reconcile(&item()), Reconcile::Found);
        let request = stub.requests.recv().unwrap();
        assert!(
            request.starts_with("GET /v1/messages/search?query=hello+later")
                || request.starts_with("GET /v1/messages/search?query=hello%20later")
        );
    }

    #[test]
    fn reconcile_ignores_other_chats_and_old_copies() {
        let body = r#"{"items":[
            {"chatID":"other","accountID":"wa-main","text":"hello later","is_sender":true,"timestamp":"2025-11-01T12:27:00Z"},
            {"chatID":"chat 9/4","accountID":"wa-main","text":"hello later","is_sender":true,"timestamp":"2025-11-01T11:00:00Z"}
        ]}"#;
        let stub = stub(vec![ok_json(body)]);
        let mut sender = sender(&stub.base_url);
        assert_eq!(sender.reconcile(&item()), Reconcile::NotFound);
    }

    #[test]
    fn reconcile_search_failure_is_unknown() {
        let stub = stub(vec![
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                .to_string(),
        ]);
        let mut sender = sender(&stub.base_url);
        assert_eq!(sender.reconcile(&item()), Reconcile::Unknown);
    }

    #[test]
    fn missing_token_is_an_error() {
        assert!(ServerSender::new("http://localhost".into(), "").is_err());
    }

    #[test]
    fn queue_type_stays_usable_with_sender() {
        // The adapter composes with the queue the same way the mock does.
        let mut queue = Queue::default();
        queue.add(item()).unwrap();
        let _sender = sender("http://127.0.0.1:1");
        assert_eq!(queue.items[0].state, crate::scheduler::State::Scheduled);
    }
}
