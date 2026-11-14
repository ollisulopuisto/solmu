//! Client for the solmu-scheduler companion service.
//!
//! The app uses this to schedule, list, edit, cancel and retry Send Later
//! messages on the sidecar. The sidecar binds localhost only and may be on
//! another machine reached through an SSH tunnel, exactly like the Server
//! API. An unreachable sidecar is a plain error, never a panic; the UI
//! shows Send Later as unavailable in that case.

use reqwest::blocking::Client;

use crate::scheduler::ScheduledMessage;

const REQUEST_TIMEOUT_SECS: u64 = 10;

pub struct SchedulerClient {
    base_url: String,
    http: Client,
}

impl SchedulerClient {
    /// `token` matches the daemon's SOLMU_CONTROL_TOKEN; empty means the
    /// daemon was started without one (still guarded against web-originated
    /// requests; see the daemon's header comment).
    pub fn new(base_url: &str, token: &str) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        if !token.is_empty() {
            let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| format!("invalid scheduler token characters: {e}"))?;
            auth.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, auth);
        }
        let http = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// True when the sidecar answers. One cheap probe; UI calls this before
    /// offering Send Later actions.
    pub fn available(&self) -> bool {
        self.http
            .get(format!("{}/v1/health", self.base_url))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub fn list(&self) -> Result<Vec<ScheduledMessage>, String> {
        let body = self.get("/v1/scheduled")?;
        serde_json::from_str(&body).map_err(|e| format!("queue unreadable: {e}"))
    }

    /// Schedule a message; returns the assigned id.
    pub fn add(
        &self,
        account_id: &str,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
        due_at: i64,
        replaces: Option<&str>,
        repeat: Option<&crate::recurrence::Repeat>,
    ) -> Result<String, String> {
        let mut payload = serde_json::json!({
            "account_id": account_id,
            "chat_id": chat_id,
            "text": text,
            "due_at": due_at,
        });
        if let Some(reply_to) = reply_to {
            payload["reply_to"] = reply_to.into();
        }
        if let Some(replaces) = replaces {
            payload["replaces"] = replaces.into();
        }
        if let Some(repeat) = repeat {
            payload["repeat"] = serde_json::to_value(repeat).map_err(|e| e.to_string())?;
        }
        let body = self.post("/v1/scheduled", payload)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("response unreadable: {e}"))?;
        parsed
            .get("id")
            .and_then(|id| id.as_str())
            .map(str::to_string)
            .ok_or_else(|| "response without id".into())
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        self.action(id, "cancel", serde_json::json!({}))
    }

    /// Retry a Failed or Missed item, or an Uncertain one after
    /// reconciliation found no copy.
    pub fn retry(&self, id: &str) -> Result<(), String> {
        self.action(id, "retry", serde_json::json!({}))
    }

    pub fn edit(&self, id: &str, text: Option<&str>, due_at: Option<i64>) -> Result<(), String> {
        let mut payload = serde_json::json!({});
        if let Some(text) = text {
            payload["text"] = text.into();
        }
        if let Some(due_at) = due_at {
            payload["due_at"] = due_at.into();
        }
        self.action(id, "edit", payload)
    }

    /// Skip the pending occurrence of a repeating series.
    pub fn skip(&self, id: &str) -> Result<(), String> {
        self.action(id, "skip", serde_json::json!({}))
    }

    /// Pause a repeating series; it is never sent while paused.
    pub fn pause(&self, id: &str) -> Result<(), String> {
        self.action(id, "pause", serde_json::json!({}))
    }

    pub fn resume(&self, id: &str) -> Result<(), String> {
        self.action(id, "resume", serde_json::json!({}))
    }

    /// Deliver now instead of at the scheduled time.
    pub fn send_now(&self, id: &str) -> Result<(), String> {
        self.action(id, "send-now", serde_json::json!({}))
    }

    fn action(&self, id: &str, action: &str, payload: serde_json::Value) -> Result<(), String> {
        self.post(&format!("/v1/scheduled/{id}/{action}"), payload)?;
        Ok(())
    }

    fn get(&self, path: &str) -> Result<String, String> {
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .send()
            .map_err(|e| format!("scheduler unreachable: {e}"))?;
        let status = response.status();
        let body = response.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        Ok(body)
    }

    fn post(&self, path: &str, payload: serde_json::Value) -> Result<String, String> {
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.to_string())
            .send()
            .map_err(|e| format!("scheduler unreachable: {e}"))?;
        let status = response.status();
        let body = response.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::State;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{Receiver, channel};
    use std::thread;

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
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap();
                tx.send(String::from_utf8_lossy(&buf[..n]).into_owned())
                    .unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Stub {
            base_url: format!("http://127.0.0.1:{port}"),
            requests: rx,
        }
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn available_reflects_health() {
        let stub = stub(vec![ok(r#"{"ok":true}"#)]);
        let client = SchedulerClient::new(&stub.base_url, "").unwrap();
        assert!(client.available());
        let down = SchedulerClient::new("http://127.0.0.1:1", "").unwrap();
        assert!(!down.available());
    }

    #[test]
    fn add_posts_fields_and_returns_id() {
        let stub = stub(vec![ok(r#"{"id":"sl-1"}"#)]);
        let client = SchedulerClient::new(&stub.base_url, "").unwrap();
        let id = client
            .add(
                "wa-main",
                "chat-1",
                "hello",
                Some("reply-1"),
                1_762_000_000,
                None,
                None,
            )
            .unwrap();
        assert_eq!(id, "sl-1");
        let request = stub.requests.recv().unwrap();
        assert!(request.starts_with("POST /v1/scheduled "));
        assert!(request.contains(r#""due_at":1762000000"#));
        assert!(request.contains(r#""reply_to":"reply-1""#));
    }

    #[test]
    fn list_parses_queue_items() {
        let body = r#"[{"id":"sl-1","account_id":"wa-main","chat_id":"chat-1","text":"hi","reply_to":null,"due_at":1762000000,"created_at":1761999000,"state":"Scheduled","attempts":0,"error":null,"sent_message_id":null}]"#;
        let stub = stub(vec![ok(body)]);
        let client = SchedulerClient::new(&stub.base_url, "").unwrap();
        let items = client.list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "sl-1");
        assert_eq!(items[0].state, State::Scheduled);
    }

    #[test]
    fn actions_hit_their_paths() {
        let stub = stub(vec![
            ok(r#"{"ok":true}"#),
            ok(r#"{"ok":true}"#),
            ok(r#"{"ok":true}"#),
        ]);
        let client = SchedulerClient::new(&stub.base_url, "").unwrap();
        client.cancel("sl-1").unwrap();
        client.edit("sl-1", None, Some(1_762_000_100)).unwrap();
        client.send_now("sl-1").unwrap();
        assert!(
            stub.requests
                .recv()
                .unwrap()
                .starts_with("POST /v1/scheduled/sl-1/cancel ")
        );
        assert!(
            stub.requests
                .recv()
                .unwrap()
                .starts_with("POST /v1/scheduled/sl-1/edit ")
        );
        assert!(
            stub.requests
                .recv()
                .unwrap()
                .starts_with("POST /v1/scheduled/sl-1/send-now ")
        );
    }

    #[test]
    fn conflict_is_an_error_not_a_panic() {
        let stub = stub(vec![
            "HTTP/1.1 409 Conflict\r\ncontent-length: 20\r\nconnection: close\r\n\r\n{\"error\":\"BadState\"}".to_string(),
        ]);
        let client = SchedulerClient::new(&stub.base_url, "").unwrap();
        let err = client.cancel("sl-1").unwrap_err();
        assert!(err.starts_with("HTTP 409"));
    }
}
