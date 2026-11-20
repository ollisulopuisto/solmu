//! Socket side of solmu-reader: one connection per thread behind a
//! global cap, a shared upstream and rate limiter, and the checks that
//! must run before authentication (web-origin refusal, bounded head).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::merged::Person;
use crate::read_api::{
    PeopleSource, RateLimiter, Registry, Upstream, handle, hash_token, parse_people,
};
use crate::scheduler_api::ServerSender;

/// Most connections served at once; extra ones are refused with 429.
pub const MAX_CONNECTIONS: usize = 32;
const MAX_REQUEST: usize = 16 * 1024;

impl Upstream for ServerSender {
    fn get(&mut self, target: &str) -> Result<(u16, String), String> {
        self.get_bounded(target)
    }
    fn send_text(&mut self, chat: &str, text: &str) -> crate::write_journal::Outcome {
        self.send_api_text(chat, text)
    }
}

/// Hands each request its own upstream handle, so a slow Server call
/// delays only the connection that made it.
pub trait UpstreamSource: Send + Sync {
    fn open(&self) -> Box<dyn Upstream>;
}

impl UpstreamSource for ServerSender {
    fn open(&self) -> Box<dyn Upstream> {
        Box::new(self.clone())
    }
}

pub struct Shared {
    pub source: Box<dyn UpstreamSource>,
    pub limiter: Mutex<RateLimiter>,
    /// The app's settings file, read-only, for linked people. None leaves
    /// the people endpoints unavailable.
    pub settings: Option<PathBuf>,
    /// API base URLs this reader serves, as the app records them in a
    /// person's members.
    pub origins: Vec<String>,
    /// Parsed registry snapshots; held only while checking/loading the file.
    pub registry_cache: Mutex<RegistryCache>,
}

/// Cache only when the platform provides a file identity and change timestamp.
/// mtime alone misses chmod and replacements that preserve modification time.
#[derive(PartialEq, Eq)]
struct RegistryStamp {
    modified: SystemTime,
    len: u64,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

fn registry_stamp(path: &Path) -> Result<Option<RegistryStamp>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("registry unreadable: {e}")),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("registry file must be mode 0600".into());
        }
        Ok(meta.modified().ok().map(|modified| RegistryStamp {
            modified,
            len: meta.len(),
            identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
        }))
    }
    // Without a reliable replacement/permission-change signal, load each time.
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok(None)
    }
}

/// Shares immutable parsed snapshots, never a lock held over upstream I/O.
pub struct RegistryCache {
    path: PathBuf,
    stamp: Option<RegistryStamp>,
    registry: Arc<Registry>,
}

impl RegistryCache {
    pub fn new(path: &Path) -> Result<Self, String> {
        let mut cache = Self {
            path: PathBuf::new(),
            stamp: None,
            registry: Arc::new(Registry::default()),
        };
        cache.registry(path)?;
        Ok(cache)
    }

    /// Revalidate file permissions on every request. Missing files load an
    /// empty registry; unreadable/invalid files never return a stale snapshot.
    pub fn registry(&mut self, path: &Path) -> Result<Arc<Registry>, String> {
        let stamp = registry_stamp(path)?;
        if stamp.is_some() && stamp == self.stamp && self.path == path {
            return Ok(Arc::clone(&self.registry));
        }
        let registry = Registry::load(path)?;
        if registry_stamp(path)? != stamp {
            return Err("registry changed while loading; retry request".into());
        }
        self.path = path.to_path_buf();
        self.stamp = stamp;
        self.registry = Arc::new(registry);
        Ok(Arc::clone(&self.registry))
    }
}

const MAX_SETTINGS_BYTES: u64 = 4 << 20;

fn load_people(path: &Option<PathBuf>) -> Option<Vec<Person>> {
    let path = path.as_ref()?;
    if std::fs::metadata(path).ok()?.len() > MAX_SETTINGS_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    parse_people(&text).ok()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let text = match status {
        200 => "200 OK",
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        409 => "409 Conflict",
        429 => "429 Too Many Requests",
        _ => "502 Bad Gateway",
    };
    let response = format!(
        "HTTP/1.1 {text}\r\ncontent-type: application/json\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    loop {
        if stream.read(&mut byte).ok()? != 1 {
            return None;
        }
        buf.push(byte[0]);
        if buf.len() > MAX_REQUEST {
            return None;
        }
        if buf.ends_with(b"\r\n\r\n") {
            return String::from_utf8(buf).ok();
        }
    }
}
fn write_allowed(key: &str, at: i64) -> bool {
    static LIMIT: std::sync::OnceLock<Mutex<std::collections::HashMap<String, (i64, u32)>>> =
        std::sync::OnceLock::new();
    let mut entries = LIMIT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    entries.retain(|_, (minute, _)| *minute == at / 60);
    let entry = entries.entry(key.into()).or_insert((at / 60, 0));
    if entry.1 >= 10 {
        return false;
    }
    entry.1 += 1;
    true
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

pub fn serve_connection(stream: &mut TcpStream, path: &Path, shared: &Shared) {
    let Some(head) = read_head(stream) else {
        return respond(stream, 400, r#"{"error":"bad request"}"#);
    };
    if header(&head, "origin").is_some() || header(&head, "referer").is_some() {
        return respond(stream, 403, r#"{"error":"web-originated request refused"}"#);
    }
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return respond(stream, 400, r#"{"error":"bad request"}"#);
    };
    let headers: Vec<_> = head
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .collect();
    let mut names = std::collections::HashSet::new();
    for line in headers {
        let Some((name, _)) = line.split_once(':') else {
            return respond(stream, 400, r#"{"error":"bad header"}"#);
        };
        if !names.insert(name.to_ascii_lowercase())
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            return respond(stream, 400, r#"{"error":"ambiguous request"}"#);
        }
    }
    let registry = match shared
        .registry_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .registry(path)
    {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!("solmu-reader: client registry unusable, refusing all requests: {e}");
            return respond(stream, 401, r#"{"error":"unauthorized"}"#);
        }
    };
    let bearer = header(&head, "authorization").and_then(|v| v.strip_prefix("Bearer "));
    let at = now();
    // Keyed by token hash, so unknown tokens cannot use a real client's
    // allowance or reach the Server.
    let key = bearer.map(hash_token).unwrap_or_default();
    let allowed = {
        let mut limiter = shared.limiter.lock().unwrap_or_else(|e| e.into_inner());
        limiter.allow(&key, at)
    };
    if !allowed {
        return respond(stream, 429, r#"{"error":"rate limited"}"#);
    }
    if method == "POST" {
        let Some(client) = bearer.and_then(|t| registry.authenticate(t, at)) else {
            return respond(stream, 401, r#"{"error":"unauthorized"}"#);
        };
        let chat = match crate::read_api::route("GET", target) {
            Ok((crate::read_api::Route::Messages(chat), _)) if !target.contains('?') => chat,
            _ => return respond(stream, 404, r#"{"error":"unknown write route"}"#),
        };
        if header(&head, "content-type")
            .and_then(|v| v.split(';').next())
            .map(str::trim)
            != Some("application/json")
        {
            return respond(stream, 400, r#"{"error":"JSON required"}"#);
        }
        let Some(length) = header(&head, "content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0 && *n <= 20 * 1024)
        else {
            return respond(
                stream,
                400,
                r#"{"error":"bounded content length required"}"#,
            );
        };
        if !write_allowed(&key, at) {
            return respond(stream, 429, r#"{"error":"send rate limited"}"#);
        }
        let mut body = vec![0; length];
        if stream.read_exact(&mut body).is_err() {
            return respond(stream, 400, r#"{"error":"incomplete body"}"#);
        }
        let mut upstream = shared.source.open();
        let journal = crate::write_journal::WriteJournal::new(path.with_extension("writes.json"));
        let (status, body) = crate::write_api::handle(
            &registry,
            &mut *upstream,
            &journal,
            bearer,
            &chat,
            &body,
            at,
            || {
                shared
                    .registry_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .registry(path)
                    .ok()
                    .and_then(|r| {
                        r.authenticate(bearer.unwrap_or(""), now())
                            .map(|c| c == client)
                    })
                    .unwrap_or(false)
            },
        );
        eprintln!(
            "solmu-reader: client={} route=send status={status}",
            client.name
        );
        return respond(stream, status, &body);
    }
    let people = if target.starts_with("/v1/people") {
        load_people(&shared.settings)
    } else {
        None
    };
    let mut upstream = shared.source.open();
    let (status, body, client, label) = handle(
        &registry,
        &mut *upstream,
        &PeopleSource {
            people: people.as_deref(),
            origins: &shared.origins,
        },
        bearer,
        method,
        target,
        at,
    );
    eprintln!(
        "solmu-reader: client={} route={label} status={status}",
        client.as_deref().unwrap_or("-")
    );
    respond(stream, status, &body);
}

/// Serves until the listener fails. Each connection gets a thread, so a
/// slow client holds only its own slot; the cap bounds the total.
pub fn run(listener: TcpListener, path: PathBuf, shared: Arc<Shared>, max: usize) {
    let active = Arc::new(AtomicUsize::new(0));
    for mut stream in listener.incoming().flatten() {
        if active.fetch_add(1, Ordering::SeqCst) >= max {
            active.fetch_sub(1, Ordering::SeqCst);
            respond(&mut stream, 429, r#"{"error":"too many connections"}"#);
            continue;
        }
        let path = path.clone();
        let shared = Arc::clone(&shared);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            serve_connection(&mut stream, &path, &shared);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}
