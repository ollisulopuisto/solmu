//! solmu-scheduler: the Send Later companion service.
//!
//! Runs beside the Beeper Server (same host, localhost-only) and delivers
//! scheduled messages through the documented Server API at their due time,
//! whether or not the Solmu app is open. The app talks to it over a
//! small local HTTP interface.
//!
//! Configuration (environment):
//! - BEEPER_BASE_URL       Server API base, e.g. http://127.0.0.1:23374
//! - BEEPER_ACCESS_TOKEN   Server API token (required)
//! - SOLMU_QUEUE_PATH    queue file (default: ~/.local/share/solmu/scheduled.json)
//! - SOLMU_LISTEN        listen address (default: 127.0.0.1:23376; ALWAYS
//!   keep this localhost - there is no auth on the control socket beyond
//!   the loopback binding)
//! - SOLMU_TICK_SECS     seconds between due checks (default: 15, min 1)
//! - SOLMU_CONTROL_TOKEN bearer token REQUIRED on every request; the
//!   daemon refuses to start without one. Prefer SOLMU_CONTROL_TOKEN_FILE
//!   (default: ~/.config/zapfast/scheduler-token, 0600) over the environment,
//!   so the secret stays out of the process list.
//!
//! Socket hardening, even without a token: POSTs require
//! Content-Type: application/json and any request carrying an Origin or
//! Referer header is refused, so a web page cannot drive the socket with a
//! form or fetch (both always send Origin on POST, and neither can set a
//! bearer token without a CORS preflight this server never answers).
//! Request bodies are capped at 64 KiB.
//!
//! Endpoints:
//!   GET  /v1/health
//!   GET  /v1/scheduled                      list the queue
//!   POST /v1/scheduled                      add: {account_id, chat_id, text,
//!                                           reply_to?, due_at} (unix seconds)
//!   POST /v1/scheduled/{id}/cancel
//!   POST /v1/scheduled/{id}/retry           Failed/Missed freely; Uncertain
//!                                           needs {"confirm_duplicate":true}
//!   POST /v1/scheduled/{id}/reconcile       check an Uncertain item against
//!                                           the chat history (no resend)
//!   POST /v1/scheduled/{id}/edit            {text?, due_at?}
//!   POST /v1/scheduled/{id}/send-now        deliver immediately
//!
//! Durability: every state change is persisted to the queue file before the
//! next action (claim-before-send). A crash mid-send recovers the item as
//! Uncertain on startup; Uncertain items are never resent automatically.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use solmu::scheduler::{Queue, Scheduler, Send};
use solmu::scheduler_api::ServerSender;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn queue_path() -> PathBuf {
    if let Ok(path) = std::env::var("SOLMU_QUEUE_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/solmu/scheduled.json")
}

fn new_id() -> String {
    // Unique enough for a single-writer local queue: nanos + urandom.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let random = std::fs::File::open("/dev/urandom")
        .ok()
        .and_then(|mut f| {
            let mut bytes = [0u8; 4];
            // Exactly 4 bytes: fs::read on /dev/urandom never sees EOF and
            // grows until the OOM killer stops it.
            f.read_exact(&mut bytes)
                .ok()
                .map(|_| u32::from_le_bytes(bytes))
        })
        .unwrap_or(0);
    format!("sl-{nanos:x}-{random:08x}")
}

fn json_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// The most a control request's body may carry.
const MAX_BODY: usize = 1 << 16;

fn read_request(stream: &mut TcpStream) -> Option<(String, String, String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..pos]).into_owned();
            let length = headers
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1)?.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if length > MAX_BODY {
                return None;
            }
            if buf.len() < pos + 4 + length {
                continue;
            }
            let body = String::from_utf8_lossy(&buf[pos + 4..pos + 4 + length]).into_owned();
            let mut parts = headers.lines().next()?.split_whitespace();
            let method = parts.next()?.to_string();
            let path = parts.next()?.to_string();
            return Some((method, path, headers, body));
        }
        if buf.len() > MAX_BODY {
            return None;
        }
    }
}

fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers
        .lines()
        .find(|l| {
            l.to_ascii_lowercase()
                .starts_with(&format!("{}:", name.to_ascii_lowercase()))
        })
        .and_then(|l| l.split(':').nth(1))
        .map(str::trim)
}

fn handle(
    stream: &mut TcpStream,
    queue: &mut Queue,
    path: &std::path::Path,
    token: &str,
    validator: &mut ServerSender,
    reconciler: &mut ServerSender,
) {
    let Some((method, req_path, headers, body)) = read_request(stream) else {
        return json_response(stream, "400 Bad Request", r#"{"error":"bad request"}"#);
    };
    // Web pages must not drive the socket: browsers always send Origin on
    // POST, and a preflight would never be answered here.
    if header(&headers, "origin").is_some() || header(&headers, "referer").is_some() {
        return json_response(
            stream,
            "403 Forbidden",
            r#"{"error":"web-originated request refused"}"#,
        );
    }
    if method == "POST" {
        let json = header(&headers, "content-type")
            .map(|v| v.to_ascii_lowercase().starts_with("application/json"))
            .unwrap_or(false);
        if !json {
            return json_response(
                stream,
                "403 Forbidden",
                r#"{"error":"json content-type required"}"#,
            );
        }
    }
    if !token.is_empty() {
        let ok = header(&headers, "authorization")
            .map(|v| v == format!("Bearer {token}"))
            .unwrap_or(false);
        if !ok {
            return json_response(stream, "403 Forbidden", r#"{"error":"bad control token"}"#);
        }
    }
    let (status, response, changed) = solmu::scheduler_api::control(
        queue,
        &method,
        &req_path,
        &body,
        now(),
        &new_id,
        &mut |chat, account| validator.validate_chat(chat, account),
        &mut |item| reconciler.reconcile(item),
    );
    if changed {
        persist(queue, path);
    }
    let status_text = match status {
        200 => "200 OK",
        400 => "400 Bad Request",
        404 => "404 Not Found",
        409 => "409 Conflict",
        _ => "500 Internal Server Error",
    };
    json_response(stream, status_text, &response);
}

/// The shared secret guarding the control socket. From the environment,
/// or better, a 0600 file the installer creates.
fn control_token() -> Option<String> {
    if let Ok(token) = std::env::var("SOLMU_CONTROL_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    let path = std::env::var("SOLMU_CONTROL_TOKEN_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config/zapfast/scheduler-token")
        });
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn persist(queue: &Queue, path: &std::path::Path) {
    if let Err(e) = queue.save(path) {
        eprintln!("solmu-scheduler: queue save failed: {e}");
    }
}

fn main() {
    let base_url =
        std::env::var("BEEPER_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:23374".into());
    let token = std::env::var("BEEPER_ACCESS_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            std::env::var("BEEPER_ACCESS_TOKEN_FILE")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
        .unwrap_or_default();
    let control_token = match control_token() {
        Some(token) => token,
        None => {
            eprintln!(
                "solmu-scheduler: no control token. Set SOLMU_CONTROL_TOKEN or run the installer, which writes the token file."
            );
            std::process::exit(1);
        }
    };
    let listen = std::env::var("SOLMU_LISTEN").unwrap_or_else(|_| "127.0.0.1:23376".into());
    let tick_secs = std::env::var("SOLMU_TICK_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(15)
        .max(1);
    let path = queue_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let sender = match ServerSender::new(base_url.clone(), &token) {
        Ok(sender) => sender,
        Err(e) => {
            eprintln!("solmu-scheduler: {e}");
            std::process::exit(1);
        }
    };
    let mut reconciler = match ServerSender::new(base_url.clone(), &token) {
        Ok(sender) => sender,
        Err(e) => {
            eprintln!("solmu-scheduler: {e}");
            std::process::exit(1);
        }
    };
    let mut validator = match ServerSender::new(base_url, &token) {
        Ok(sender) => sender,
        Err(e) => {
            eprintln!("solmu-scheduler: {e}");
            std::process::exit(1);
        }
    };
    // The listening port is the ownership guard: bind before the queue is
    // read or written, so a second process exits without touching the file.
    let listener = match TcpListener::bind(&listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("solmu-scheduler: cannot bind {listen}: {e}");
            std::process::exit(1);
        }
    };
    listener.set_nonblocking(true).ok();
    let queue = match Queue::load(&path) {
        Ok(queue) => queue,
        Err(e) => {
            eprintln!("solmu-scheduler: cannot load {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let mut scheduler = Scheduler::new(queue, sender);
    let recovered = scheduler.recover();
    if !recovered.is_empty() {
        eprintln!(
            "solmu-scheduler: {} item(s) crashed mid-send, now Uncertain: {}",
            recovered.len(),
            recovered.join(", ")
        );
    }
    // Recovery may also have written missing successors of repeating
    // series; save them even when nothing was mid-send.
    persist(&scheduler.queue, &path);

    eprintln!(
        "solmu-scheduler: listening on {listen}, queue {}",
        path.display()
    );

    let tick = Duration::from_secs(tick_secs);
    loop {
        let started = Instant::now();
        // Drain control requests first so user actions land before sends.
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => handle(
                    &mut stream,
                    &mut scheduler.queue,
                    &path,
                    &control_token,
                    &mut validator,
                    &mut reconciler,
                ),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        let outcomes = scheduler.tick(now(), &mut |queue| persist(queue, &path));
        for outcome in outcomes {
            eprintln!("solmu-scheduler: {outcome:?}");
        }
        std::thread::sleep(tick.saturating_sub(started.elapsed()));
    }
}
