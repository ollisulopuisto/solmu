//! End-to-end loopback writes, synthetic credentials and text only.
use solmu::{
    read_api::{RateLimiter, Registry, Upstream},
    read_http::{RegistryCache, Shared, UpstreamSource},
    write_grants::Destination,
    write_journal::Outcome,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Source(Arc<AtomicUsize>);
struct Mock(Arc<AtomicUsize>);
impl UpstreamSource for Source {
    fn open(&self) -> Box<dyn Upstream> {
        Box::new(Mock(self.0.clone()))
    }
}
impl Upstream for Mock {
    fn get(&mut self, _: &str) -> Result<(u16, String), String> {
        Ok((200, r#"{"accountID":"a"}"#.into()))
    }
    fn send_text(&mut self, _: &str, _: &str) -> Outcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        Outcome::Accepted {
            message_id: "synthetic-id".into(),
        }
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    path: std::path::PathBuf,
    shared: Arc<Shared>,
    token: String,
    read: String,
    count: Arc<AtomicUsize>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clients.json");
        let mut r = Registry::default();
        let token = r
            .add_with_destinations(
                "write",
                vec!["messages:send".into()],
                vec!["a".into()],
                vec![Destination {
                    account: "a".into(),
                    chat: "c".into(),
                }],
                None,
                0,
            )
            .unwrap();
        let read = r
            .add("read", vec!["messages:read".into()], vec![], None, 0)
            .unwrap();
        r.save(&path).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(Shared {
            source: Box::new(Source(count.clone())),
            limiter: Mutex::new(RateLimiter::default()),
            settings: None,
            origins: vec![],
            registry_cache: Mutex::new(RegistryCache::new(&path).unwrap()),
        });
        Self {
            dir,
            path,
            shared,
            token,
            read,
            count,
        }
    }
    fn request(&self, text: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let path = self.path.clone();
        let shared = self.shared.clone();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            solmu::read_http::serve_connection(&mut stream, &path, &shared)
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(text.as_bytes()).unwrap();
        let mut response = String::new();
        let read = stream.read_to_string(&mut response);
        assert!(
            read.is_ok()
                || (!response.is_empty()
                    && read.unwrap_err().kind() == std::io::ErrorKind::ConnectionReset)
        );
        task.join().unwrap();
        response
    }
    fn post(&self, token: &str, extra: &str, text: &str) -> String {
        let body =
            serde_json::json!({"account_id":"a","text":text,"request_id":"request-1"}).to_string();
        self.request(format!("POST /v1/chats/c/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",body.len()))
    }
}
#[test]
fn socket_send_is_exactly_once_and_body_private() {
    let f = Fixture::new();
    assert!(
        f.post(&f.token, "", "private synthetic text")
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        f.post(&f.token, "", "private synthetic text")
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        f.post(&f.token, "", "substitute")
            .starts_with("HTTP/1.1 409")
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
    let audit = std::fs::read_to_string(f.path.with_extension("writes.json")).unwrap();
    assert!(!audit.contains("private synthetic text"));
    assert!(!audit.contains(&f.token));
    assert!(f.dir.path().exists());
}
#[test]
fn socket_read_grant_web_and_duplicate_headers_never_send() {
    let f = Fixture::new();
    assert!(f.post(&f.read, "", "hi").starts_with("HTTP/1.1 403"));
    assert!(
        f.post(&f.token, "Origin: https://example.invalid\r\n", "hi")
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        f.post(&f.token, "Content-Length: 5\r\n", "hi")
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        f.post(&f.token, "Transfer-Encoding: chunked\r\n", "hi")
            .starts_with("HTTP/1.1 400")
    );
    let mut r = Registry::load(&f.path).unwrap();
    r.revoke("write");
    r.save(&f.path).unwrap();
    assert!(f.post(&f.token, "", "hi").starts_with("HTTP/1.1 401"));
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
}
#[test]
fn client_post_keeps_exact_path_without_read_pagination() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("token");
    std::fs::write(&path, "synthetic-only").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            stream.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        assert!(
            String::from_utf8(head)
                .unwrap()
                .starts_with("POST /v1/chats/c/messages HTTP/1.1\r\n")
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    let c = solmu::automation_client::Client::new(&base, path).unwrap();
    assert_eq!(
        c.call("POST", "/v1/chats/c/messages", Some(&serde_json::json!({})))
            .unwrap()
            .0,
        200
    );
    task.join().unwrap();
}
#[test]
fn actual_cli_sends_through_scoped_loopback_server() {
    let f = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let shared = f.shared.clone();
    let path = f.path.clone();
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        solmu::read_http::serve_connection(&mut stream, &path, &shared)
    });
    let token_file = f.dir.path().join("token");
    std::fs::write(&token_file, &f.token).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let text_file = f.dir.path().join("text");
    std::fs::write(&text_file, "synthetic cli send").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_solmu-control"))
        .args(["send", "--token-file"])
        .arg(token_file)
        .args(["--text-file"])
        .arg(text_file)
        .args([
            "--account",
            "a",
            "--chat",
            "c",
            "--request-id",
            "cli-stable",
            "--url",
            &base,
        ])
        .output()
        .unwrap();
    task.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["state"], "accepted");
    assert_eq!(value["delivery_confirmed"], false);
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
}
