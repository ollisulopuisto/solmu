//! Read-only API for other local apps, without the socket.
//!
//! Every client app gets its own named bearer token. Only the SHA-256 of a
//! token is stored in a 0600 registry file. File identity, change stamps
//! and permissions are rechecked on every request; changes reload the
//! parsed snapshot, so deleting an entry revokes it on the next request. A client holds scopes
//! (one per endpoint family) and may be limited to an account allowlist.
//!
//! The upstream Beeper token never reaches a client and no write endpoint
//! is routable: the route table below is the whole surface, and every
//! entry is a GET. Unknown scopes, unknown registry fields (for example a
//! chat allowlist this version cannot enforce) and unreadable registries
//! all fail closed.
//!
//! Not covered yet: linked-person and merged-timeline views, which live in
//! the app's settings and not on the Server.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::merged::Person;

pub const SCOPES: [&str; 6] = [
    "accounts:read",
    "chats:read",
    "messages:read",
    "search:read",
    "people:read",
    "messages:send",
];
const MAX_LIMIT: u32 = 100;
pub const RATE_PER_MINUTE: u32 = 120;
const STALE_LOCK_SECS: u64 = 120;
/// Most linked members one merged timeline will read.
const MAX_MEMBERS: usize = 16;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClientEntry {
    pub name: String,
    pub token_sha256: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub send_destinations: Vec<crate::write_grants::Destination>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default)]
    pub clients: Vec<ClientEntry>,
}

/// A lock is stale only when the process that wrote it is gone. Age alone
/// never breaks a lock, so a slow but live command keeps it. A lock with no
/// readable owner yet is given STALE_LOCK_SECS to be written.
fn lock_is_stale(lock: &Path) -> bool {
    let owner = std::fs::read_to_string(lock)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok());
    match owner {
        Some(pid) => !pid_alive(pid),
        None => std::fs::metadata(lock)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age.as_secs() > STALE_LOCK_SECS)
            .unwrap_or(false),
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

pub struct RegistryLock(std::path::PathBuf);

impl Drop for RegistryLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Lowercase hex of the SHA-256 digest, written straight into a stack buffer
/// instead of 32 per-byte `format!` allocations. Both sides of
/// [`Registry::authenticate`] are lowercase hex, so no re-lowercasing is needed
/// there either.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = [0u8; 64];
    for (i, byte) in digest.iter().enumerate() {
        out[i * 2] = HEX[(byte >> 4) as usize];
        out[i * 2 + 1] = HEX[(byte & 0x0F) as usize];
    }
    // Hex digits are always valid UTF-8.
    String::from_utf8(out.to_vec()).expect("hex digits are valid UTF-8")
}

/// Compares every byte, ignoring ASCII case; the hashes are fixed-length, so
/// a length difference is not secret. Both sides are validated ASCII hex, and
/// folding with `| 0x20` makes upper- and lowercase hex digits compare equal
/// without allocating a lowered copy.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .fold(0u8, |acc, (x, y)| acc | ((x | 0x20) ^ (y | 0x20)))
            == 0
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn valid_account(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@'))
}

impl Registry {
    pub fn parse(text: &str) -> Result<Registry, String> {
        let registry: Registry =
            serde_json::from_str(text).map_err(|e| format!("registry unreadable: {e}"))?;
        let mut seen = Vec::new();
        for client in &registry.clients {
            if !valid_name(&client.name) || seen.contains(&client.name) {
                return Err("registry has an invalid or duplicate client name".into());
            }
            seen.push(client.name.clone());
            let hash = &client.token_sha256;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("registry has an invalid token hash".into());
            }
            if client.scopes.is_empty()
                || client.scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))
            {
                return Err("registry has an empty or unknown scope".into());
            }
            if client
                .scopes
                .iter()
                .any(|scope| scope == crate::write_grants::SEND_SCOPE)
                && (!crate::write_grants::valid_destinations(&client.send_destinations)
                    || client.send_destinations.iter().any(|d| {
                        !client.accounts.is_empty() && !client.accounts.contains(&d.account)
                    }))
            {
                return Err(
                    "send scope requires explicit valid destinations within the account grant"
                        .into(),
                );
            }
            if client.accounts.iter().any(|a| !valid_account(a)) {
                return Err("registry has an invalid account id".into());
            }
        }
        Ok(registry)
    }

    /// A missing file is an empty registry (nobody is authorized). An
    /// existing file must be private to its owner, or it is refused.
    pub fn load(path: &Path) -> Result<Registry, String> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
            Err(e) => return Err(format!("registry unreadable: {e}")),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err("registry file must be mode 0600".into());
            }
        }
        #[cfg(not(unix))]
        let _ = meta;
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("registry unreadable: {e}"))?;
        Registry::parse(&text)
    }

    /// Exclusive lock for a load-modify-save cycle across processes.
    pub fn lock(path: &Path) -> Result<RegistryLock, String> {
        use std::io::Write;
        let lock = path.with_extension("lock");
        for attempt in 0..2 {
            let created = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock);
            if let Ok(mut file) = created {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(RegistryLock(lock));
            }
            if attempt == 0 && lock_is_stale(&lock) {
                let _ = std::fs::remove_file(&lock);
            }
        }
        Err("another token command is running".to_string())
    }

    /// Written 0600 from creation, then renamed into place.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|e| format!("save failed: {e}"))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("save failed: {e}"))?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(|e| format!("save failed: {e}"))
    }

    /// Adds a client and returns its token, which exists nowhere else.
    pub fn add(
        &mut self,
        name: &str,
        scopes: Vec<String>,
        accounts: Vec<String>,
        expires_at: Option<i64>,
        now: i64,
    ) -> Result<String, String> {
        self.add_with_destinations(name, scopes, accounts, vec![], expires_at, now)
    }
    pub fn add_with_destinations(
        &mut self,
        name: &str,
        scopes: Vec<String>,
        accounts: Vec<String>,
        send_destinations: Vec<crate::write_grants::Destination>,
        expires_at: Option<i64>,
        now: i64,
    ) -> Result<String, String> {
        let mut bytes = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| "no secure randomness available".to_string())?;
        let mut token = String::from("zfr_");
        for byte in bytes {
            token.push_str(&format!("{byte:02x}"));
        }
        let entry = ClientEntry {
            name: name.to_string(),
            token_sha256: hash_token(&token),
            scopes,
            accounts,
            expires_at,
            created_at: now,
            send_destinations,
        };
        let mut next = Registry {
            clients: self.clients.clone(),
        };
        next.clients.push(entry);
        let text = serde_json::to_string(&next).map_err(|e| e.to_string())?;
        Registry::parse(&text)?;
        *self = next;
        Ok(token)
    }

    pub fn revoke(&mut self, name: &str) -> bool {
        let before = self.clients.len();
        self.clients.retain(|c| c.name != name);
        self.clients.len() != before
    }

    /// Constant-time over every entry: no early exit reveals which hash
    /// was close. `hash_token` emits lowercase hex and `ct_eq` ignores ASCII
    /// case, so stored hashes match whether they are upper- or lowercase, with
    /// no per-entry allocation or re-lowercasing.
    pub fn authenticate(&self, token: &str, now: i64) -> Option<&ClientEntry> {
        let presented = hash_token(token);
        let mut found = None;
        for client in &self.clients {
            let same = ct_eq(presented.as_bytes(), client.token_sha256.as_bytes());
            if same {
                found = Some(client);
            }
        }
        found.filter(|c| c.expires_at.map(|at| now < at).unwrap_or(true))
    }
}

#[derive(Debug, PartialEq)]
pub enum Route {
    Accounts,
    Chats,
    Messages(String),
    Search,
    People,
    PersonMessages { id: String, limit: u32 },
}

impl Route {
    pub fn scope(&self) -> &'static str {
        match self {
            Route::Accounts => "accounts:read",
            Route::Chats => "chats:read",
            Route::Messages(_) => "messages:read",
            Route::Search => "search:read",
            Route::People | Route::PersonMessages { .. } => "people:read",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Route::Accounts => "accounts",
            Route::Chats => "chats",
            Route::Messages(_) => "messages",
            Route::Search => "search",
            Route::People => "people",
            Route::PersonMessages { .. } => "people-messages",
        }
    }
}

fn encode(raw: &str) -> String {
    let mut out = String::new();
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn query_value_ok(value: &str) -> bool {
    value.len() <= 512
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'%' | b'+')
        })
}

/// Parses a request target into a route and the exact upstream target to
/// fetch. Only allowlisted query keys survive; anything else is refused,
/// not stripped, so a client learns its request was not honoured.
pub fn route(method: &str, target: &str) -> Result<(Route, String), u16> {
    if method != "GET" {
        return Err(405);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let (parsed, upstream, keys): (Route, String, &[&str]) = match segments.as_slice() {
        ["v1", "accounts"] => (Route::Accounts, "/v1/accounts".into(), &[]),
        ["v1", "chats"] => (
            Route::Chats,
            "/v1/chats".into(),
            &["cursor", "direction", "limit"],
        ),
        ["v1", "chats", id, "messages"] => {
            let chat = decode(id).ok_or(400u16)?;
            if chat.is_empty() || chat.len() > 256 || chat.contains('/') || chat.contains("..") {
                return Err(400);
            }
            let upstream = format!("/v1/chats/{}/messages", encode(&chat));
            (
                Route::Messages(chat),
                upstream,
                &["cursor", "direction", "limit"],
            )
        }
        ["v1", "messages", "search"] => (
            Route::Search,
            "/v1/messages/search".into(),
            &["query", "cursor", "direction", "limit"],
        ),
        ["v1", "people"] => (Route::People, String::new(), &[]),
        ["v1", "people", id, "messages"] => {
            if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(400);
            }
            let route = Route::PersonMessages {
                id: id.to_ascii_lowercase(),
                limit: MAX_LIMIT,
            };
            (route, String::new(), &["limit"])
        }
        _ => return Err(404),
    };
    let mut parsed = parsed;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').ok_or(400u16)?;
        if !keys.contains(&key) || pairs.iter().any(|(k, _)| k == key) || !query_value_ok(value) {
            return Err(400);
        }
        let value = match key {
            "limit" => value
                .parse::<u32>()
                .map_err(|_| 400u16)?
                .clamp(1, MAX_LIMIT)
                .to_string(),
            "direction" if value != "before" && value != "after" => return Err(400),
            _ => value.to_string(),
        };
        pairs.push((key.to_string(), value));
    }
    if parsed == Route::Search && !pairs.iter().any(|(k, _)| k == "query") {
        return Err(400);
    }
    if !pairs.iter().any(|(k, _)| k == "limit") && !keys.is_empty() {
        pairs.push(("limit".into(), MAX_LIMIT.to_string()));
    }
    if let Route::PersonMessages { limit, .. } = &mut parsed {
        let given = pairs.iter().find(|(k, _)| k == "limit");
        *limit = given.and_then(|(_, v)| v.parse().ok()).unwrap_or(MAX_LIMIT);
    }
    let mut full = upstream;
    for (i, (key, value)) in pairs.iter().enumerate() {
        full.push(if i == 0 { '?' } else { '&' });
        full.push_str(&format!("{key}={value}"));
    }
    Ok((parsed, full))
}

pub trait Upstream {
    /// A GET against the Server. Err is a transport failure.
    fn get(&mut self, target: &str) -> Result<(u16, String), String>;
    /// Default refusal keeps read-only test/adaptor implementations incapable of sending.
    fn send_text(&mut self, _chat: &str, _text: &str) -> crate::write_journal::Outcome {
        crate::write_journal::Outcome::Rejected
    }
}

fn account_of(value: &Value) -> Option<&str> {
    value.get("accountID").and_then(Value::as_str)
}

/// Keeps only the items inside the allowlist. Counts, cursors, flags and
/// nested lookup maps are all dropped: they are computed over the
/// unfiltered result and could reveal accounts outside the allowlist, so
/// a restricted client sees one filtered page and no paging.
fn filter_items(value: &Value, allowed: &[String]) -> Option<Value> {
    let items = value.as_object()?.get("items")?.as_array()?;
    let kept: Vec<Value> = items
        .iter()
        .filter(|i| {
            account_of(i)
                .map(|a| allowed.iter().any(|x| x == a))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    Some(serde_json::json!({ "items": kept }))
}

/// Stable, content-derived id for a linked person (settings has none).
/// One line per distinct member origin in the app's people links: how many
/// members use it and whether this reader serves it. Origins are API base
/// URLs, not secrets.
pub fn origin_report(people: &[Person], origins: &[String]) -> Vec<(String, usize, bool)> {
    let mut rows: Vec<(String, usize, bool)> = Vec::new();
    for member in people.iter().flat_map(|p| p.members.iter()) {
        match rows.iter_mut().find(|r| r.0 == member.origin) {
            Some(row) => row.1 += 1,
            None => rows.push((member.origin.clone(), 1, readable(member, origins))),
        }
    }
    rows.sort();
    rows
}

/// What the people endpoints need besides the Server handle.
pub struct PeopleSource<'a> {
    pub people: Option<&'a [Person]>,
    pub origins: &'a [String],
}

pub fn person_id(person: &Person) -> String {
    let mut parts: Vec<String> = person
        .members
        .iter()
        .map(|m| format!("{}|{}|{}", m.origin, m.account, m.chat))
        .collect();
    parts.sort();
    hash_token(&format!("{}\0{}", person.name, parts.join("\0")))[..16].to_string()
}

/// A member's origin is the API base URL the app read the chat from. Only
/// origins this reader is configured to serve are readable; others belong
/// to a backend the reader cannot reach.
fn readable(member: &crate::merged::Member, origins: &[String]) -> bool {
    let origin = member.origin.trim_end_matches('/');
    origins.iter().any(|o| o.trim_end_matches('/') == origin)
}

/// A restricted client sees a person only if every linked member is in its
/// allowlist, so the existence of a link across accounts never leaks.
fn visible<'a>(people: &'a [Person], client: &ClientEntry) -> Vec<&'a Person> {
    people
        .iter()
        .filter(|p| {
            !p.members.is_empty()
                && (client.accounts.is_empty()
                    || p.members
                        .iter()
                        .all(|m| client.accounts.contains(&m.account)))
        })
        .collect()
}

fn member_json(member: &crate::merged::Member, origins: &[String]) -> Value {
    serde_json::json!({
        "origin": member.origin,
        "account": member.account,
        "chat": member.chat,
        "readable": readable(member, origins),
    })
}

fn people_list(people: &[Person], origins: &[String], client: &ClientEntry) -> Value {
    let items: Vec<Value> = visible(people, client)
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "id": person_id(p),
                "name": p.name,
                "primary": p.primary.as_ref().map(|m| member_json(m, origins)),
                "members": p
                    .members
                    .iter()
                    .map(|m| member_json(m, origins))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "items": items })
}

fn timestamp_of(item: &Value) -> i64 {
    item.get("timestamp")
        .and_then(Value::as_str)
        .and_then(|t| t.parse::<jiff::Timestamp>().ok())
        .map(|t| t.as_second())
        .unwrap_or(0)
}

/// Merged timeline of one person's Beeper-origin chats: the newest `limit`
/// messages across members, no paging. Members that cannot be read are
/// listed, never silently dropped.
fn person_messages(
    upstream: &mut dyn Upstream,
    person: &Person,
    origins: &[String],
    limit: u32,
) -> Value {
    let mut items: Vec<Value> = Vec::new();
    let mut unavailable: Vec<Value> = Vec::new();
    for (index, member) in person.members.iter().enumerate() {
        let mut skip = |reason: &str| {
            let mut entry = member_json(member, origins);
            entry["reason"] = reason.into();
            unavailable.push(entry);
        };
        if index >= MAX_MEMBERS {
            skip("member limit reached");
            continue;
        }
        if !readable(member, origins) {
            skip("origin not served by this reader");
            continue;
        }
        let chat = encode(&member.chat);
        let owner = match upstream.get(&format!("/v1/chats/{chat}")) {
            Ok((200, body)) => serde_json::from_str::<Value>(&body).ok(),
            _ => None,
        };
        if owner.as_ref().and_then(account_of) != Some(member.account.as_str()) {
            skip("chat not found on that account");
            continue;
        }
        let target = format!("/v1/chats/{chat}/messages?limit={limit}");
        let page = match upstream.get(&target) {
            Ok((200, body)) => serde_json::from_str::<Value>(&body).ok(),
            _ => None,
        };
        let Some(Value::Array(list)) = page.and_then(|mut v| v.get_mut("items").map(Value::take))
        else {
            skip("messages unreadable");
            continue;
        };
        for mut item in list {
            if let Some(object) = item.as_object_mut() {
                object.insert("chatID".into(), member.chat.clone().into());
                object.insert("accountID".into(), member.account.clone().into());
                items.push(item);
            }
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(timestamp_of(i)));
    items.truncate(limit as usize);
    serde_json::json!({
        "items": items,
        "partial": !unavailable.is_empty(),
        "unavailable": unavailable,
    })
}

/// The `beeper.people` list from the app's settings file, nothing else.
pub fn parse_people(settings_json: &str) -> Result<Vec<Person>, String> {
    let value: Value =
        serde_json::from_str(settings_json).map_err(|e| format!("settings unreadable: {e}"))?;
    match value.get("beeper").and_then(|b| b.get("people")) {
        None => Ok(Vec::new()),
        Some(people) => {
            serde_json::from_value(people.clone()).map_err(|e| format!("people unreadable: {e}"))
        }
    }
}

fn error(status: u16, message: &str) -> (u16, String) {
    (status, serde_json::json!({ "error": message }).to_string())
}

/// One authenticated request in, (status, body, client name, route label)
/// out. Upstream bodies are never forwarded on failure.
pub fn handle(
    registry: &Registry,
    upstream: &mut dyn Upstream,
    source: &PeopleSource,
    bearer: Option<&str>,
    method: &str,
    target: &str,
    now: i64,
) -> (u16, String, Option<String>, &'static str) {
    let Some(client) = bearer.and_then(|t| registry.authenticate(t, now)) else {
        let (s, b) = error(401, "unauthorized");
        return (s, b, None, "-");
    };
    let name = Some(client.name.clone());
    let (parsed, upstream_target) = match route(method, target) {
        Ok(ok) => ok,
        Err(status) => {
            let (s, b) = error(status, "request refused");
            return (s, b, name, "-");
        }
    };
    let label = parsed.label();
    if !client.scopes.iter().any(|s| s == parsed.scope()) {
        let (s, b) = error(403, "scope not granted");
        return (s, b, name, label);
    }
    let restricted = !client.accounts.is_empty();
    if matches!(parsed, Route::People | Route::PersonMessages { .. }) {
        let Some(people) = source.people else {
            let (s, b) = error(502, "people source unavailable");
            return (s, b, name, label);
        };
        if let Route::PersonMessages { id, limit } = &parsed {
            let person = visible(people, client)
                .into_iter()
                .find(|p| person_id(p) == *id);
            let Some(person) = person else {
                let (s, b) = error(404, "not found");
                return (s, b, name, label);
            };
            let body = person_messages(upstream, person, source.origins, *limit);
            return (200, body.to_string(), name, label);
        }
        return (
            200,
            people_list(people, source.origins, client).to_string(),
            name,
            label,
        );
    }
    if let (true, Route::Messages(chat)) = (restricted, &parsed) {
        let probe = format!("/v1/chats/{}", encode(chat));
        let owner = match upstream.get(&probe) {
            Ok((200, body)) => serde_json::from_str::<Value>(&body).ok(),
            Ok(_) | Err(_) => None,
        };
        let allowed = owner
            .as_ref()
            .and_then(account_of)
            .map(|a| client.accounts.iter().any(|x| x == a))
            .unwrap_or(false);
        if !allowed {
            let (s, b) = error(404, "not found");
            return (s, b, name, label);
        }
    }
    let (status, body) = match upstream.get(&upstream_target) {
        Ok((200, body)) => (200, body),
        Ok((404, _)) => {
            let (s, b) = error(404, "not found");
            return (s, b, name, label);
        }
        Ok(_) | Err(_) => {
            let (s, b) = error(502, "upstream unavailable");
            return (s, b, name, label);
        }
    };
    if !restricted || matches!(parsed, Route::Messages(_)) {
        return (status, body, name, label);
    }
    let filtered = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| match &v {
            Value::Array(list) => Some(Value::Array(
                list.iter()
                    .filter(|i| {
                        account_of(i)
                            .map(|a| client.accounts.iter().any(|x| x == a))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect(),
            )),
            _ => filter_items(&v, &client.accounts),
        });
    match filtered {
        Some(value) => (200, value.to_string(), name, label),
        None => {
            let (s, b) = error(502, "upstream response not understood");
            (s, b, name, label)
        }
    }
}

/// Fixed-window per-client request counter.
#[derive(Default)]
pub struct RateLimiter {
    windows: HashMap<String, (i64, u32)>,
}

impl RateLimiter {
    pub fn allow(&mut self, client: &str, now: i64) -> bool {
        let minute = now / 60;
        if self.windows.len() > 1024 {
            self.windows.retain(|_, slot| slot.0 == minute);
        }
        let slot = self
            .windows
            .entry(client.to_string())
            .or_insert((minute, 0));
        if slot.0 != minute {
            *slot = (minute, 0);
        }
        slot.1 += 1;
        slot.1 <= RATE_PER_MINUTE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: PeopleSource = PeopleSource {
        people: None,
        origins: &[],
    };

    struct Fake {
        calls: Vec<String>,
    }

    impl Upstream for Fake {
        fn get(&mut self, target: &str) -> Result<(u16, String), String> {
            self.calls.push(target.to_string());
            if target.starts_with("/v1/chats/missing") {
                return Ok((404, "internal detail".into()));
            }
            if target.starts_with("/v1/chats/other") {
                return Err("down".into());
            }
            let body = if target.starts_with("/v1/accounts") {
                r#"[{"accountID":"a1"},{"accountID":"b2"}]"#
            } else if target.starts_with("/v1/chats/c-a1") && !target.contains("messages") {
                r#"{"accountID":"a1"}"#
            } else if target.starts_with("/v1/chats/c-b2") && !target.contains("messages") {
                r#"{"accountID":"b2"}"#
            } else if target.contains("/messages?") {
                r#"{"items":[{"text":"hi"}]}"#
            } else if target.starts_with("/v1/chats?") {
                r#"{"items":[{"accountID":"a1","id":"x"},{"accountID":"b2","id":"y"}],"hasMore":true,"users":{"u":{"accountID":"b2"}}}"#
            } else if target.starts_with("/v1/messages/search") {
                r#"{"items":[{"accountID":"b2","text":"secret"},{"accountID":"a1","text":"ok"},{"text":"no account"}],"chats":{"y":{}}}"#
            } else {
                return Err("down".into());
            };
            Ok((200, body.into()))
        }
    }

    fn setup(scopes: &[&str], accounts: &[&str]) -> (Registry, String) {
        let mut registry = Registry::default();
        let token = registry
            .add(
                "app",
                scopes.iter().map(|s| s.to_string()).collect(),
                accounts.iter().map(|s| s.to_string()).collect(),
                None,
                100,
            )
            .unwrap();
        (registry, token)
    }

    fn run(registry: &Registry, token: &str, method: &str, target: &str) -> (u16, String) {
        let mut fake = Fake { calls: vec![] };
        let (status, body, _, _) =
            handle(registry, &mut fake, &NONE, Some(token), method, target, 200);
        (status, body)
    }

    #[test]
    fn token_is_stored_only_as_hash() {
        let (registry, token) = setup(&["chats:read"], &[]);
        let text = serde_json::to_string(&registry).unwrap();
        assert!(!text.contains(&token));
        assert!(text.contains(&hash_token(&token)));
        assert!(token.starts_with("zfr_") && token.len() == 68);
    }

    #[test]
    fn unknown_missing_wrong_and_revoked_tokens_are_refused() {
        let (mut registry, token) = setup(&["chats:read"], &[]);
        assert_eq!(run(&registry, "zfr_nope", "GET", "/v1/chats").0, 401);
        let mut fake = Fake { calls: vec![] };
        assert_eq!(
            handle(&registry, &mut fake, &NONE, None, "GET", "/v1/chats", 200).0,
            401
        );
        assert_eq!(run(&registry, &token, "GET", "/v1/chats").0, 200);
        assert!(registry.revoke("app"));
        assert_eq!(run(&registry, &token, "GET", "/v1/chats").0, 401);
    }

    #[test]
    fn expired_token_is_refused() {
        let mut registry = Registry::default();
        let token = registry
            .add("app", vec!["chats:read".into()], vec![], Some(150), 100)
            .unwrap();
        assert!(registry.authenticate(&token, 149).is_some());
        assert!(registry.authenticate(&token, 150).is_none());
    }

    #[test]
    fn scope_is_per_endpoint_family() {
        let (registry, token) = setup(&["chats:read"], &[]);
        assert_eq!(run(&registry, &token, "GET", "/v1/chats").0, 200);
        assert_eq!(run(&registry, &token, "GET", "/v1/accounts").0, 403);
        assert_eq!(
            run(&registry, &token, "GET", "/v1/messages/search?query=x").0,
            403
        );
        assert_eq!(
            run(&registry, &token, "GET", "/v1/chats/c-a1/messages").0,
            403
        );
    }

    #[test]
    fn nothing_but_get_on_the_allowlist_is_routable() {
        let (registry, token) = setup(
            &[
                "accounts:read",
                "chats:read",
                "messages:read",
                "search:read",
                "people:read",
            ],
            &[],
        );
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert_eq!(run(&registry, &token, method, "/v1/chats").0, 405);
        }
        for path in [
            "/v1/chats/c-a1",
            "/v1/chats/c-a1/read",
            "/v1/chats/c-a1/messages/m1",
            "/v1/focus",
            "/v0/mcp",
            "/oauth/introspect",
            "/v1/chats/a%2Fb/messages",
            "/v1/chats/%2e%2e/messages",
        ] {
            assert!(run(&registry, &token, "GET", path).0 >= 400, "{path}");
        }
    }

    #[test]
    fn query_keys_values_and_limit_are_constrained() {
        assert_eq!(
            route("GET", "/v1/chats?limit=5000").unwrap().1,
            "/v1/chats?limit=100"
        );
        assert_eq!(route("GET", "/v1/chats").unwrap().1, "/v1/chats?limit=100");
        assert!(route("GET", "/v1/chats?accountIDs=a1").is_err());
        assert!(route("GET", "/v1/chats?limit=1&limit=2").is_err());
        assert!(route("GET", "/v1/chats?limit=abc").is_err());
        assert!(route("GET", "/v1/chats?direction=sideways").is_err());
        assert!(route("GET", "/v1/chats?cursor=a/b").is_err());
        assert!(route("GET", "/v1/chats?cursor=a&b=1").is_err());
        assert!(route("GET", "/v1/messages/search").is_err());
        assert!(route("GET", "/v1/accounts?x=1").is_err());
    }

    #[test]
    fn chat_ids_are_re_encoded_for_upstream() {
        let (parsed, target) = route("GET", "/v1/chats/%21abc%3Abeeper.com/messages").unwrap();
        assert_eq!(parsed, Route::Messages("!abc:beeper.com".into()));
        assert_eq!(target, "/v1/chats/%21abc%3Abeeper.com/messages?limit=100");
    }

    #[test]
    fn account_allowlist_filters_every_listing() {
        let (registry, token) = setup(
            &[
                "accounts:read",
                "chats:read",
                "messages:read",
                "search:read",
                "people:read",
            ],
            &["a1"],
        );
        let accounts = run(&registry, &token, "GET", "/v1/accounts");
        assert_eq!(accounts.1, r#"[{"accountID":"a1"}]"#);
        let chats: Value =
            serde_json::from_str(&run(&registry, &token, "GET", "/v1/chats").1).unwrap();
        assert_eq!(chats["items"].as_array().unwrap().len(), 1);
        assert!(chats.get("hasMore").is_none());
        assert_eq!(chats.as_object().unwrap().len(), 1);
        assert!(chats.get("users").is_none());
        let found = run(&registry, &token, "GET", "/v1/messages/search?query=x").1;
        assert!(!found.contains("secret") && !found.contains("no account"));
        assert!(found.contains("ok") && !found.contains("\"chats\""));
    }

    #[test]
    fn account_allowlist_guards_messages_by_chat_owner() {
        let (registry, token) = setup(
            &[
                "accounts:read",
                "chats:read",
                "messages:read",
                "search:read",
                "people:read",
            ],
            &["a1"],
        );
        assert_eq!(
            run(&registry, &token, "GET", "/v1/chats/c-a1/messages").0,
            200
        );
        assert_eq!(
            run(&registry, &token, "GET", "/v1/chats/c-b2/messages").0,
            404
        );
        assert_eq!(
            run(&registry, &token, "GET", "/v1/chats/unknown/messages").0,
            404
        );
    }

    #[test]
    fn upstream_failures_never_leak_detail() {
        let (registry, token) = setup(
            &[
                "accounts:read",
                "chats:read",
                "messages:read",
                "search:read",
                "people:read",
            ],
            &[],
        );
        let missing = run(&registry, &token, "GET", "/v1/chats/missing/messages");
        assert_eq!(missing.0, 404);
        assert!(!missing.1.contains("internal"));
        let (status, body) = run(&registry, &token, "GET", "/v1/chats/other/messages");
        assert_eq!(status, 502);
        assert!(!body.contains("down"));
    }

    #[test]
    fn registry_fails_closed_on_unsupported_or_malformed_entries() {
        let hash = "a".repeat(64);
        let ok = format!(
            r#"{{"clients":[{{"name":"app","token_sha256":"{hash}","scopes":["chats:read"]}}]}}"#
        );
        assert!(Registry::parse(&ok).is_ok());
        let chats = ok.replace("\"scopes\"", "\"chats\":[\"c1\"],\"scopes\"");
        assert!(Registry::parse(&chats).is_err());
        assert!(Registry::parse(&ok.replace("chats:read", "chats:write")).is_err());
        assert!(Registry::parse(&ok.replace("[\"chats:read\"]", "[]")).is_err());
        assert!(Registry::parse(&ok.replace(&hash, "short")).is_err());
        assert!(Registry::parse(&ok.replace("app", "Bad Name")).is_err());
        assert!(Registry::parse("not json").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn registry_file_is_private_on_create_and_checked_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("zf-reader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clients.json");
        assert!(Registry::load(&path).unwrap().clients.is_empty());
        let (registry, token) = setup(&["chats:read"], &[]);
        registry.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&token));
        assert!(
            Registry::load(&path)
                .unwrap()
                .authenticate(&token, 200)
                .is_some()
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Registry::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn lock_is_kept_for_a_live_owner_and_recovered_from_a_dead_one() {
        let dir = std::env::temp_dir().join(format!("zf-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clients.json");
        let held = Registry::lock(&path).unwrap();
        assert!(Registry::lock(&path).is_err(), "live owner keeps it");
        drop(held);
        // A pid that cannot exist stands for a crashed command.
        std::fs::write(path.with_extension("lock"), "4194999\n").unwrap();
        assert!(Registry::lock(&path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn origin_report_counts_members_and_marks_unserved_origins() {
        let member = |origin: &str, chat: &str| crate::merged::Member {
            origin: origin.into(),
            account: "acct".into(),
            chat: chat.into(),
        };
        let people = vec![Person {
            name: "Ada".into(),
            members: vec![
                member("http://127.0.0.1:23374", "!a:beeper.local"),
                member("http://127.0.0.1:23374/", "!b:beeper.local"),
                member("http://127.0.0.1:23373", "!c:beeper.local"),
            ],
            primary: None,
        }];
        let report = origin_report(&people, &["http://127.0.0.1:23374".to_string()]);
        assert_eq!(report.len(), 3);
        assert!(report.iter().any(|r| r.0.ends_with("23373") && !r.2));
        assert!(report.iter().filter(|r| r.2).count() == 2);
    }

    #[test]
    fn duplicate_client_names_are_refused() {
        let (mut registry, _) = setup(&["chats:read"], &[]);
        assert!(
            registry
                .add("app", vec!["chats:read".into()], vec![], None, 1)
                .is_err()
        );
        assert_eq!(registry.clients.len(), 1);
    }

    #[test]
    fn rate_limit_is_per_client_and_per_minute() {
        let mut limiter = RateLimiter::default();
        for _ in 0..RATE_PER_MINUTE {
            assert!(limiter.allow("a", 600));
        }
        assert!(!limiter.allow("a", 601));
        assert!(limiter.allow("b", 601));
        assert!(limiter.allow("a", 660));
    }
}
