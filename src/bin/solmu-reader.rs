//! solmu-reader: a read-only API for other local apps.
//!
//! Runs beside the Beeper Server and serves a small allowlist of GET
//! endpoints to client apps that each hold their own revocable token. See
//! contrib/read-api/README.md for the model and its limits.
//!
//! Serve (environment): BEEPER_BASE_URL, BEEPER_ACCESS_TOKEN or
//! BEEPER_ACCESS_TOKEN_FILE (the upstream token, never shown to clients),
//! SOLMU_SETTINGS_FILE (optional: the app's settings.json, read-only, for
//! linked people), SOLMU_READER_ORIGINS (comma list of API base URLs the
//! app records for chats this reader can serve; default BEEPER_BASE_URL),
//! SOLMU_READER_LISTEN (default 127.0.0.1:23377; non-loopback addresses
//! are refused), SOLMU_READER_CLIENTS (default
//! ~/.config/zapfast/reader-clients.json, mode 0600).
//!
//! Check that the app's recorded origins are served:
//!   solmu-reader people-check   (needs SOLMU_SETTINGS_FILE; exit 1 if any
//!   linked member's origin is not served)
//!
//! Manage clients (never reads the upstream token):
//!   solmu-reader token add NAME --scopes a,b [--accounts x,y] [--days N]
//!   solmu-reader token list
//!   solmu-reader token revoke NAME
//! `token add` prints the new token once, on the terminal. It is not
//! stored anywhere in clear.
//!
//! Endpoints (all GET): /v1/accounts, /v1/chats,
//! /v1/chats/{id}/messages, /v1/messages/search?query=, /v1/people and
//! /v1/people/{id}/messages (needs SOLMU_SETTINGS_FILE)
//!
//! Requests carrying Origin or Referer are refused, so web pages cannot
//! drive it. The log holds client name, endpoint family and status only:
//! never query strings, chat ids or message content.

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use solmu::read_api::{RateLimiter, Registry, SCOPES, origin_report, parse_people};
use solmu::read_http::{MAX_CONNECTIONS, RegistryCache, Shared, run};
use solmu::scheduler_api::ServerSender;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn registry_path() -> PathBuf {
    if let Ok(path) = std::env::var("SOLMU_READER_CLIENTS") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/zapfast/reader-clients.json")
}

fn token_command(args: &[String]) -> i32 {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = match Registry::lock(&path) {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("solmu-reader: {e}");
            return 1;
        }
    };
    let mut registry = match Registry::load(&path) {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!("solmu-reader: {e}");
            return 1;
        }
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let list = |v: Option<String>| -> Vec<String> {
        v.map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_default()
    };
    match args.first().map(String::as_str) {
        Some("add") => {
            let Some(name) = args.get(1).filter(|n| !n.starts_with("--")) else {
                eprintln!("usage: token add NAME --scopes a,b [--accounts x,y] [--days N]");
                return 2;
            };
            let scopes = list(flag("--scopes"));
            let accounts = list(flag("--accounts"));
            let days = flag("--days").and_then(|d| d.parse::<i64>().ok());
            let expires = days.map(|d| now() + d.clamp(1, 3650) * 86400);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let destinations = match flag("--send-destinations-file") {
                Some(path) => match std::fs::read_to_string(path).ok().and_then(|s| {
                    serde_json::from_str::<Vec<solmu::write_grants::Destination>>(&s).ok()
                }) {
                    Some(d) => d,
                    None => {
                        eprintln!("invalid send destination file");
                        return 2;
                    }
                },
                None => vec![],
            };
            match registry.add_with_destinations(
                name,
                scopes,
                accounts,
                destinations,
                expires,
                now(),
            ) {
                Ok(token) => match registry.save(&path) {
                    Ok(()) => {
                        println!("client {name} created. Token (shown once):\n{token}");
                        0
                    }
                    Err(e) => {
                        eprintln!("solmu-reader: {e}");
                        1
                    }
                },
                Err(e) => {
                    eprintln!("solmu-reader: {e}. Scopes: {}", SCOPES.join(", "));
                    1
                }
            }
        }
        Some("list") => {
            for client in &registry.clients {
                let accounts = if client.accounts.is_empty() {
                    "all accounts".to_string()
                } else {
                    client.accounts.join(",")
                };
                println!(
                    "{}  scopes={}  {accounts}  expires={}",
                    client.name,
                    client.scopes.join(","),
                    client
                        .expires_at
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "never".into())
                );
            }
            0
        }
        Some("revoke") => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: token revoke NAME");
                return 2;
            };
            if !registry.revoke(name) {
                eprintln!("solmu-reader: no such client");
                return 1;
            }
            match registry.save(&path) {
                Ok(()) => {
                    println!("client {name} revoked");
                    0
                }
                Err(e) => {
                    eprintln!("solmu-reader: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: solmu-reader token add|list|revoke");
            2
        }
    }
}

/// Origins this reader serves: SOLMU_READER_ORIGINS, else the Server URL.
fn served_origins(base_url: &str) -> Vec<String> {
    match std::env::var("SOLMU_READER_ORIGINS") {
        Ok(list) => list.split(',').map(|o| o.trim().to_string()).collect(),
        Err(_) => vec![base_url.to_string()],
    }
}

/// Prints, for the app's linked people, each recorded member origin and
/// whether this reader serves it. Returns 0 only when nothing is unserved.
fn people_check(base_url: &str) -> i32 {
    let Some(path) = std::env::var("SOLMU_SETTINGS_FILE").ok() else {
        eprintln!("solmu-reader: set SOLMU_SETTINGS_FILE to the app's settings.json");
        return 2;
    };
    let people = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| parse_people(&text))
    {
        Ok(people) => people,
        Err(e) => {
            eprintln!("solmu-reader: cannot read people: {e}");
            return 1;
        }
    };
    let origins = served_origins(base_url);
    println!("serving origins: {}", origins.join(", "));
    let report = origin_report(&people, &origins);
    if report.is_empty() {
        println!("no linked people with members");
    }
    let mut unserved = 0;
    for (origin, members, served) in &report {
        let state = if *served { "served" } else { "NOT served" };
        println!("recorded origin {origin}: {members} member(s), {state}");
        if !served {
            unserved += 1;
        }
    }
    if unserved > 0 {
        println!(
            "Members under an unserved origin are listed as unreadable. If one of the recorded origins above is this Server reached another way, add it to SOLMU_READER_ORIGINS."
        );
    }
    i32::from(unserved > 0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("token") {
        std::process::exit(token_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("people-check") {
        let base_url =
            std::env::var("BEEPER_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:23374".into());
        std::process::exit(people_check(&base_url));
    }
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
    let listen = std::env::var("SOLMU_READER_LISTEN").unwrap_or_else(|_| "127.0.0.1:23377".into());
    let addr: SocketAddr = match listen.parse() {
        Ok(addr) => addr,
        Err(_) => {
            eprintln!("solmu-reader: listen address must be a literal IP:port");
            std::process::exit(1);
        }
    };
    if !addr.ip().is_loopback() {
        eprintln!("solmu-reader: refusing a non-loopback listen address");
        std::process::exit(1);
    }
    let path = registry_path();
    let registry_cache = Mutex::new(match RegistryCache::new(&path) {
        Ok(cache) => cache,
        Err(e) => {
            eprintln!("solmu-reader: {e}");
            std::process::exit(1);
        }
    });
    // The app records the API base URL it read each chat from; people whose
    // members carry other origins stay unreadable here.
    let origins = served_origins(&base_url);
    let sender = match ServerSender::new(base_url, &token) {
        Ok(sender) => sender,
        Err(e) => {
            eprintln!("solmu-reader: {e}");
            std::process::exit(1);
        }
    };
    if let Ok(settings) = std::env::var("SOLMU_SETTINGS_FILE") {
        let people = std::fs::read_to_string(settings)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_people(&text));
        for (origin, members, served) in origin_report(&people.unwrap_or_default(), &origins) {
            if !served {
                eprintln!(
                    "solmu-reader: {members} linked member(s) recorded under origin {origin}, which this reader does not serve (run `solmu-reader people-check`)"
                );
            }
        }
    }
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("solmu-reader: cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("solmu-reader: listening on {addr}");
    let shared = Arc::new(Shared {
        source: Box::new(sender),
        limiter: Mutex::new(RateLimiter::default()),
        settings: std::env::var("SOLMU_SETTINGS_FILE").ok().map(PathBuf::from),
        origins,
        registry_cache,
    });
    run(listener, path, shared, MAX_CONNECTIONS);
}
