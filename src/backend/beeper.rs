//! Beeper backend: an adapter that speaks the window's existing
//! [`Command`]/[`Event`] protocol, but serves chats from a Beeper Client API
//! endpoint - a headless Beeper Server or a running Beeper Desktop - instead
//! of talking the WhatsApp protocol directly.
//!
//! This is the "swap the backend from WhatsApp to Beeper/Matrix" demo: the
//! whole egui UI (chat list, conversation pane, composer, settings) is
//! unchanged Solmu code; only the source of chats, messages and sends is
//! different. Selected at startup with `SOLMU_BACKEND=beeper`; see
//! README-BEEPER.md.
//!
//! Multi-account Client API adapter. Current coverage and validation gaps are
//! recorded in PARITY.md. Transport contract tests use local generic fixtures;
//! they do not prove real-network or macOS compatibility.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::mpsc::UnboundedReceiver;

use super::{Command, Event, LinkStatus, Waker};
use crate::model::{Chat, ChatId, ChatKind, Content, Delivery, LastMessage, Message};
use crate::paths::AccountDirs;

mod gallery;

const CHAT_PAGE_LIMIT: u32 = 100;
/// How often the chat list (ordering, previews, unread counts) is refreshed.
const CHAT_REFRESH: Duration = Duration::from_secs(60);
/// Receipt refresh for the open chat. The forward-only live poll never
/// re-returns already loaded messages, so their delivery/read receipt
/// fields would stall without an occasional full-page refetch.
const RECEIPT_REFRESH: Duration = Duration::from_secs(30);
/// How often the open conversation is polled for new messages.
const MSG_POLL: Duration = Duration::from_secs(3);
/// Command-handling latency budget; the loop sleeps this long between drains.
const LOOP_TICK: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct BeeperConfig {
    pub base_url: String,
    pub token: String,
}

static SESSION: std::sync::Mutex<Option<BeeperConfig>> = std::sync::Mutex::new(None);

impl BeeperConfig {
    /// Solmu is a Beeper client; kept as a function so call sites read
    /// the same as before the native WhatsApp backend was removed.
    pub fn is_beeper_mode() -> bool {
        true
    }

    /// Retain the explicitly selected, ready Server connection for this process.
    pub fn set_session(cfg: &Self) {
        *SESSION.lock().unwrap_or_else(|p| p.into_inner()) = Some(cfg.clone());
    }

    /// The explicit in-app session wins. Otherwise preserve the launch configuration.
    pub fn from_env() -> Option<Self> {
        if let Some(cfg) = SESSION.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            return Some(cfg.clone());
        }
        let base_url = std::env::var("BEEPER_DESKTOP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:23373".to_string());
        let token = std::env::var("BEEPER_ACCESS_TOKEN").unwrap_or_default();
        Some(Self { base_url, token })
    }
}

// ---------- Beeper Client API wire types ----------
// Shapes follow the official TypeScript SDK (beeper/desktop-api-js, `next`),
// which is generated from Beeper's OpenAPI spec.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
// serde's derive would otherwise add a bogus `T: Default` bound for the
// `#[serde(default)]` fields (serde issue #1541).
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct Page<T> {
    #[serde(default)]
    items: Vec<T>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    oldest_cursor: Option<String>,
    #[serde(default)]
    newest_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperChat {
    id: String,
    #[serde(default)]
    #[serde(rename = "accountID")]
    account_id: String,
    #[serde(default)]
    network: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "imgURL", default)]
    image_url: Option<String>,
    #[serde(rename = "type", default)]
    chat_type: String,
    #[serde(default)]
    unread_count: i64,
    #[serde(default)]
    last_activity: Option<String>,
    #[serde(default)]
    is_archived: bool,
    #[serde(default)]
    is_muted: bool,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    is_read_only: bool,
    #[serde(default)]
    participants: Option<BeeperParticipants>,
    #[serde(default)]
    preview: Option<BeeperMessage>,
    #[serde(default)]
    capabilities: Option<BeeperCapabilities>,
}

/// The chat capabilities the Server reports. Only `poll` is read today:
/// polls are the one advertised capability the API gives no endpoint for.
#[derive(Debug, Deserialize)]
struct BeeperCapabilities {
    /// Network poll support: -2 rejected, -1 dropped, 0 unsupported,
    /// 1 partial, 2 full (per the Desktop API reference).
    #[serde(default)]
    poll: Option<i8>,
}

#[derive(Debug, Deserialize)]
struct BeeperParticipants {
    #[serde(default)]
    items: Vec<BeeperUser>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperUser {
    id: String,
    #[serde(rename = "imgURL", default)]
    image_url: Option<String>,
    #[serde(default)]
    full_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    phone_number: Option<String>,
    #[serde(default)]
    is_self: Option<bool>,
}

type PreviewFallbacks =
    std::sync::Arc<std::sync::Mutex<HashMap<(String, String), Option<BeeperMessage>>>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperMessage {
    id: String,
    #[serde(rename = "chatID", default)]
    chat_id: String,
    #[serde(rename = "accountID", default)]
    account_id: String,
    #[serde(rename = "senderID", default)]
    sender_id: String,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    sort_key: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "type", default)]
    message_type: Option<String>,
    #[serde(default)]
    is_sender: Option<bool>,
    #[serde(default)]
    is_hidden: bool,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    edited_timestamp: Option<String>,
    #[serde(default)]
    attachments: Vec<BeeperAttachment>,
    #[serde(default)]
    send_status: Option<BeeperSendStatus>,
    #[serde(default)]
    seen: Option<serde_json::Value>,
    #[serde(rename = "linkedMessageID", default)]
    linked_message_id: Option<String>,
    #[serde(default)]
    reactions: Vec<BeeperReaction>,
    #[serde(default)]
    links: Vec<BeeperLinkPreview>,
}

/// Supplied metadata only. Image URLs are deliberately not downloaded here.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperLinkPreview {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(rename = "originalURL", default)]
    original_url: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BeeperReaction {
    #[serde(rename = "participantID")]
    participant_id: String,
    #[serde(rename = "reactionKey")]
    reaction_key: String,
    #[serde(default)]
    emoji: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperSendStatus {
    status: String,
    #[serde(default)]
    delivered_to_users: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct SendAccepted {
    #[serde(rename = "pendingMessageID")]
    pending_id: String,
    #[serde(rename = "chatID", default)]
    chat: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperAttachment {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "srcURL", default)]
    src_url: Option<String>,
    #[serde(default)]
    file_size: u64,
    #[serde(default)]
    size: Option<AttachmentSize>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    is_gif: bool,
    #[serde(default)]
    is_sticker: bool,
    #[serde(default)]
    is_voice_note: bool,
}
#[derive(Debug, Clone, Deserialize)]
struct AttachmentSize {
    width: u32,
    height: u32,
}

/// Canonical navigation identity only. Account IDs, origins and send targets
/// are never changed. Unknown network names remain intact.
fn canonical_network(network: &str) -> String {
    match network.trim().to_ascii_lowercase().as_str() {
        "facebook" | "facebook/messenger" | "facebook messenger" | "messenger" | "facebookgo" => {
            "Facebook/Messenger".into()
        }
        "signal" | "signalgo" => "Signal".into(),
        "matrix" | "beeper" => "Beeper".into(),
        "linkedin" => "LinkedIn".into(),
        "whatsapp" => "WhatsApp".into(),
        "slack" | "slackgo" => "Slack".into(),
        "discord" | "discordgo" => "Discord".into(),
        "telegram" => "Telegram".into(),
        "instagram" => "Instagram".into(),
        "googlemessages" | "google messages" | "sms" => "Google Messages".into(),
        "imessage" => "iMessage".into(),
        _ => network.to_owned(),
    }
}

#[derive(Debug, Deserialize)]
struct BeeperBridge {
    #[serde(rename = "type", default)]
    bridge_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeeperAccount {
    #[serde(default)]
    bridge: Option<BeeperBridge>,
    #[serde(rename = "accountID")]
    account_id: String,
    #[serde(default)]
    user: Option<BeeperUser>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

impl BeeperAccount {
    fn navigation_network(&self) -> String {
        if let Some(network) = self.network.as_deref().filter(|n| !n.trim().is_empty()) {
            return canonical_network(network);
        }
        // Fall back only to a known protocol type supplied by this API, never
        // infer a network from an account ID or invent a connected account.
        self.bridge
            .as_ref()
            .map(|b| match b.bridge_type.as_str() {
                "facebook" | "facebookgo" | "messenger" | "signal" | "signalgo" | "matrix"
                | "linkedin" | "whatsapp" | "slack" | "slackgo" | "discord" | "discordgo"
                | "telegram" | "instagram" | "googlemessages" | "imessage" => {
                    canonical_network(&b.bridge_type)
                }
                _ => String::new(),
            })
            .unwrap_or_default()
    }
}

fn account_inventory(accounts: &[BeeperAccount]) -> Vec<super::BeeperAccountInfo> {
    accounts
        .iter()
        .map(|a| super::BeeperAccountInfo {
            id: a.account_id.clone(),
            name: a
                .user
                .as_ref()
                .and_then(|u| u.full_name.clone().or(u.username.clone()))
                .unwrap_or_else(|| a.account_id.clone()),
            network: a.navigation_network(),
            status: a.status.clone().unwrap_or_else(|| "Not supplied".into()),
            own_user_id: a.user.as_ref().map(|u| u.id.clone()),
        })
        .collect()
}

// ---------- HTTP ----------

#[derive(Clone)]
struct Api {
    base_url: String,
    http: reqwest::blocking::Client,
    read_only: bool,
    cache_lock: std::sync::Arc<std::sync::Mutex<()>>,
    own_ids: std::sync::Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Real last message for chats whose Beeper preview carries no content
    /// (a reaction event, say), keyed by chat and the placeholder preview id.
    preview_fallbacks: PreviewFallbacks,
    avatar_sources: std::sync::Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Chat ids whose avatar source is a participant's picture borrowed for a
    /// direct chat, not the chat's own. Only these may be dropped when the
    /// participant's picture goes away.
    borrowed_avatar_chats: std::sync::Arc<std::sync::Mutex<HashSet<String>>>,
}

fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Whether a Beeper API error is the Server reporting it has no such
/// resource (HTTP 404): permanent for this id, not worth retrying.
fn not_found(error: &str) -> bool {
    error.contains("returned 404") || error.contains("refused: 404")
}

/// Live-poll copy and whether the failure is terminal for this chat. A
/// chat the Server does not list 404s forever, so the poll stops instead
/// of "retrying automatically".
fn live_error_copy(error: &str) -> (String, bool) {
    if not_found(error) {
        (
            "Live updates ended: this chat does not exist on the Beeper Server".into(),
            true,
        )
    } else {
        (format!("Live updates unavailable: {error}"), false)
    }
}

/// History copy: a 404 means the chat id is not a Server chat at all
/// (a local placeholder, say), not a temporary failure.
fn history_error_copy(error: &str) -> String {
    if not_found(error) {
        "This chat does not exist on the Beeper Server; history cannot be loaded".into()
    } else {
        format!("Beeper history failed: {error}")
    }
}

/// Send copy: a 404 means the text went nowhere and retrying the same
/// chat cannot help.
fn send_error_copy(error: &str) -> String {
    if not_found(error) {
        format!(
            "{error}. The chat does not exist on the Beeper Server. Text restored; the message was not sent."
        )
    } else {
        format!("{error}. Text restored; check history before retrying.")
    }
}

/// Only our disposable files are eligible. Never follow links or recurse into
/// user folders, and never inspect account databases or saved downloads.
fn cache_files(
    dirs: &AccountDirs,
) -> std::io::Result<Vec<(std::time::SystemTime, std::path::PathBuf, u64)>> {
    let mut files = Vec::new();
    if std::fs::symlink_metadata(dirs.media_cache_dir()).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other(
            "Cache directory must not be a symlink",
        ));
    }
    let entries = match std::fs::read_dir(dirs.media_cache_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("beeper-")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let meta = entry.metadata()?;
        files.push((
            meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            entry.path(),
            meta.len(),
        ));
    }
    files.sort_by_key(|(time, _, _)| *time);
    Ok(files)
}

/// File extension for one ISO base media `ftyp` brand, if it is known.
fn iso_brand_extension(brand: &[u8]) -> Option<&'static str> {
    Some(match brand {
        b"qt  " => "mov",
        b"M4A " | b"M4B " => "m4a",
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => "heic",
        b"avif" | b"avis" => "avif",
        [b'3', b'g', ..] => "3gp",
        b"isom" | b"iso2" | b"iso3" | b"iso4" | b"iso5" | b"iso6" | b"iso7" | b"iso8" | b"iso9"
        | b"mp41" | b"mp42" | b"avc1" | b"dash" | b"M4V " | b"M4VH" | b"M4VP" | b"f4v " => "mp4",
        _ => return None,
    })
}

/// File extension for a downloaded attachment: the message type first, then
/// the original file name, then the server's content type, then the bytes.
fn media_extension(
    mime: Option<&str>,
    file_name: Option<&str>,
    content_type: Option<&str>,
    head: &[u8],
) -> &'static str {
    fn from_mime(mime: &str) -> Option<&'static str> {
        let base = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        Some(match base.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            "video/quicktime" => "mov",
            "audio/ogg" => "ogg",
            "audio/mpeg" => "mp3",
            "audio/mp4" | "audio/x-m4a" => "m4a",
            "application/pdf" => "pdf",
            _ => return None,
        })
    }
    if let Some(ext) = mime.and_then(from_mime) {
        return ext;
    }
    if let Some(ext) = file_name
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .and_then(|ext| match ext.as_str() {
            "png" => Some("png"),
            "jpg" | "jpeg" => Some("jpg"),
            "webp" => Some("webp"),
            "gif" => Some("gif"),
            "mp4" | "m4v" => Some("mp4"),
            "mov" => Some("mov"),
            "webm" => Some("webm"),
            "ogg" => Some("ogg"),
            "mp3" => Some("mp3"),
            "m4a" => Some("m4a"),
            "pdf" => Some("pdf"),
            _ => None,
        })
    {
        return ext;
    }
    if let Some(ext) = content_type.and_then(from_mime) {
        return ext;
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        // The major brand says what the ISO media file is; ftyp alone also
        // covers audio, HEIC and AVIF. When the major brand is unknown (for
        // example iso8), the compatible brands that follow it decide.
        let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let end = if size >= 16 {
            size.min(head.len())
        } else {
            head.len()
        };
        let compatible = (16..end).step_by(4).filter_map(|at| head.get(at..at + 4));
        return std::iter::once(&head[8..12])
            .chain(compatible)
            .find_map(iso_brand_extension)
            .unwrap_or("bin");
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        // EBML is Matroska and WebM alike; the DocType sits in the header.
        let window = &head[..head.len().min(64)];
        let has = |needle: &[u8]| window.windows(needle.len()).any(|w| w == needle);
        return if has(b"webm") {
            "webm"
        } else if has(b"matroska") {
            "mkv"
        } else {
            "bin"
        };
    }
    if head.starts_with(b"\x89PNG") {
        return "png";
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "jpg";
    }
    if head.starts_with(b"GIF8") {
        return "gif";
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return "webp";
    }
    if head.starts_with(b"OggS") {
        return "ogg";
    }
    if head.starts_with(b"%PDF") {
        return "pdf";
    }
    "bin"
}

pub fn cache_budget(dirs: &AccountDirs) -> u64 {
    std::fs::read_to_string(dirs.state.join("beeper-cache-limit-mib"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(256)
        .clamp(16, 4096)
        * 1024
        * 1024
}

pub fn set_cache_budget(dirs: &AccountDirs, mib: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(&dirs.state)?;
    std::fs::write(
        dirs.state.join("beeper-cache-limit-mib"),
        mib.clamp(16, 4096).to_string(),
    )
}

fn trim_cache(dirs: &AccountDirs, budget: u64) -> std::io::Result<()> {
    let files = cache_files(dirs)?;
    let mut size: u64 = files.iter().map(|(_, _, bytes)| bytes).sum();
    for (_, path, bytes) in files {
        if size <= budget {
            break;
        }
        std::fs::remove_file(path)?;
        size = size.saturating_sub(bytes);
    }
    Ok(())
}

fn measure_cache(dirs: &AccountDirs) -> std::io::Result<crate::model::StorageStats> {
    let mut stats = crate::model::StorageStats::default();
    for (_, path, bytes) in cache_files(dirs)? {
        stats.add(&path, bytes);
    }
    Ok(stats)
}

pub(crate) fn draft_namespace(origin: &str, chat: &str) -> String {
    serde_json::to_string(&(origin, chat)).expect("string pair serializes")
}

pub(crate) fn stored_drafts(
    archive: &crate::archive::Archive,
    origin: &str,
) -> Result<Vec<(String, String)>, String> {
    archive
        .drafts()
        .map_err(|_| "Could not load encrypted Beeper drafts".to_owned())
        .map(|drafts| {
            drafts
                .into_iter()
                .filter_map(|(key, text)| {
                    let (stored_origin, chat): (String, String) =
                        serde_json::from_str(&key).ok()?;
                    (stored_origin == origin).then_some((chat, text))
                })
                .collect()
        })
}

fn transport_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "connection timed out. Check the existing API endpoint; no backend was started"
    } else if error.is_connect() {
        "could not connect to the API. For Desktop, keep Beeper running and check Settings > Integrations; for an existing Server, check its actual address/running state. No backend was started"
    } else {
        "transport failed before an HTTP response. Check the existing API endpoint and local network/TLS settings"
    }
}

fn refuse_unsupported_payload(command: Command) -> Result<Command, Box<Event>> {
    use super::{Refusal, Unsent};
    let (chat, quoting, unsent) = match command {
        // A pasted image takes the upload pipeline like a picked file.
        // What that pipeline cannot carry - mentions or view-once - is
        // refused here, exactly as picked files are. A quote rides along.
        Command::SendImage {
            chat,
            width,
            height,
            rgba,
            view_once,
            caption,
            mentions,
            quoting,
            ..
        } if view_once || !mentions.is_empty() => (
            chat,
            quoting,
            Unsent::Image {
                width,
                height,
                rgba,
                view_once,
                caption,
            },
        ),
        Command::SendLocation {
            chat,
            latitude,
            longitude,
            quoting,
        } => (
            chat,
            quoting,
            Unsent::Location {
                latitude,
                longitude,
            },
        ),
        other => return Ok(other),
    };
    Err(Box::new(Event::SendRefused {
        chat,
        quoting,
        unsent,
        reason: Refusal::UnsupportedBackend,
    }))
}

/// How an uploaded file is sent: the type Beeper picks, a plain document, or
/// a voice note of this many seconds.
#[derive(Clone, Copy)]
enum Attachment {
    Auto,
    Document,
    Voice(u32),
    Gif,
    Sticker,
}

struct StopUploads(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for StopUploads {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
/// A pasted image's send encoding: lossless PNG, so the clipboard's
/// detail and alpha survive the send. Kept local so the portable adapter
/// check, which stages this module without the native worker, covers the
/// same bytes.
fn encode_png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(bytes.into_inner())
}

/// Temporary files this process created for the upload pipeline, so a kept
/// file that the composer sends again is still recognised as ours: a
/// successful retry deletes it instead of leaking it.
static TEMP_UPLOADS: std::sync::Mutex<Option<std::collections::HashSet<std::path::PathBuf>>> =
    std::sync::Mutex::new(None);

pub(crate) fn register_temp_upload(path: &std::path::Path) {
    TEMP_UPLOADS
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(path.to_path_buf());
}

// A single deletion worker keeps filesystem stalls off the UI thread.
enum TempCleanup {
    Remove(std::path::PathBuf),
    Flush(std::sync::mpsc::Sender<()>),
}
fn temp_cleanup() -> &'static std::sync::mpsc::Sender<TempCleanup> {
    static WORKER: std::sync::OnceLock<std::sync::mpsc::Sender<TempCleanup>> =
        std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("solmu-media-cleanup".into())
            .spawn(move || {
                for job in receiver {
                    match job {
                        TempCleanup::Remove(path) => {
                            let _ = std::fs::remove_file(path);
                        }
                        TempCleanup::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("media cleanup worker");
        sender
    })
}

/// Claims a temporary file at most once; never deletes user originals.
/// Only the worker touches the filesystem.
pub(crate) fn discard_temp_upload(path: &std::path::Path) {
    if take_temp_upload(path) {
        let _ = temp_cleanup().send(TempCleanup::Remove(path.to_path_buf()));
    }
}

/// Used at process shutdown, after producers stop, and by deterministic tests.
pub(crate) fn flush_temp_cleanup() {
    let (done, finished) = std::sync::mpsc::channel();
    if temp_cleanup().send(TempCleanup::Flush(done)).is_ok() {
        let _ = finished.recv();
    }
}

fn take_temp_upload(path: &std::path::Path) -> bool {
    TEMP_UPLOADS
        .lock()
        .unwrap()
        .as_mut()
        .is_some_and(|set| set.remove(path))
}

/// Encodes pasted pixels into a temporary PNG for the upload pipeline.
fn encode_pasted_image(width: u32, height: u32, rgba: &[u8]) -> Result<std::path::PathBuf, String> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or("Clipboard image data is invalid")?;
    let bytes = encode_png(&image::DynamicImage::ImageRgba8(image))?;
    let path = std::env::temp_dir().join(format!("pasted-image-{}.png", crate::util::now()));
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    register_temp_upload(&path);
    Ok(path)
}

/// Removes an app-created temporary file - a voice recording or a pasted
/// image - once its message was accepted. Other attachments are the user's
/// own files and are never deleted.
/// Caption and reply quote ride on a batch's first file only. Once any file
/// was accepted they are already sent, so the unsent suffix a failed upload
/// returns must not repeat them; an untouched batch gets both back.
fn first_unsent<T: Clone>(value: &Option<T>, index: usize) -> Option<T> {
    if index == 0 { value.clone() } else { None }
}

fn discard_sent_recording(temporary: bool, path: &std::path::Path) {
    if temporary {
        let _ = std::fs::remove_file(path);
    }
}

fn upload_stopped(cancel: Option<&std::sync::atomic::AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
}

fn read_attachment_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|_| "Attachment could not be opened")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Attachment could not be inspected")?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("Attachment must be a regular file within the size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "Attachment could not be read")?;
    if bytes.len() as u64 > limit {
        return Err("Attachment grew beyond the size limit".into());
    }
    Ok(bytes)
}

fn bounded_json_body(response: reqwest::blocking::Response) -> Result<String, String> {
    const MAX: u64 = 16 * 1024 * 1024;
    if response.content_length().is_some_and(|n| n > MAX) {
        return Err("Beeper JSON response exceeds 16 MB".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Beeper response read failed")?;
    if bytes.len() as u64 > MAX {
        return Err("Beeper JSON response exceeds 16 MB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Beeper JSON response was not UTF-8".into())
}

fn safe_send_error(response: reqwest::blocking::Response) -> String {
    let status = response.status();
    let body = bounded_json_body(response)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok());
    let mut hints = Vec::new();
    fn visit(value: &serde_json::Value, hints: &mut Vec<String>, depth: usize) {
        if depth > 6 || hints.len() >= 8 {
            return;
        }
        match value {
            serde_json::Value::Object(map) => {
                if let Some(path) = map.get("path").and_then(|v| v.as_array()) {
                    let fields: Vec<_> = path
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter(|s| {
                            matches!(
                                *s,
                                "text" | "replyToMessageID" | "attachment" | "uploadID" | "chatID"
                            )
                        })
                        .collect();
                    if !fields.is_empty() {
                        hints.push(format!("invalid field {}", fields.join(".")));
                    }
                }
                for (key, value) in map {
                    if key != "message" && key != "input" && key != "received" {
                        visit(value, hints, depth + 1);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values.iter().take(16) {
                    visit(value, hints, depth + 1);
                }
            }
            _ => {}
        }
    }
    if let Some(body) = body {
        visit(&body, &mut hints, 0);
    }
    hints.sort();
    hints.dedup();
    format!(
        "Beeper send returned {status}{}. Server response values are withheld for privacy",
        if hints.is_empty() {
            String::new()
        } else {
            format!(" ({})", hints.join(", "))
        }
    )
}

fn check_chat_inventory_bounds(rows: usize, pages: usize) -> Result<(), String> {
    if rows > 100_000 {
        return Err("Beeper chat inventory exceeds 100000 rows".into());
    }
    if pages > 1000 {
        return Err("Beeper chat inventory exceeds 1000 pages".into());
    }
    Ok(())
}

impl Api {
    fn new(cfg: &BeeperConfig) -> Result<Self, String> {
        if cfg.token.is_empty() {
            return Err("BEEPER_ACCESS_TOKEN is empty".to_string());
        }
        let endpoint =
            reqwest::Url::parse(&cfg.base_url).map_err(|_| "Invalid Beeper endpoint URL")?;
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Err("Do not put credentials in the endpoint URL".into());
        }
        if endpoint.host_str().is_none()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !matches!(endpoint.path(), "" | "/")
        {
            return Err("Use the Beeper API root URL without a path, query or fragment".into());
        }
        let loopback = matches!(
            endpoint.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
            return Err("Beeper endpoint must use HTTPS, except loopback HTTP".into());
        }
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", cfg.token))
            .map_err(|e| format!("invalid token characters: {e}"))?;
        auth.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth);
        let builder = reqwest::blocking::Client::builder();
        // Keep local API traffic and tokens off configured system proxies.
        let builder = if loopback {
            builder.no_proxy()
        } else {
            builder
        };
        let http = builder
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: endpoint.as_str().trim_end_matches('/').to_string(),
            http,
            cache_lock: Default::default(),
            own_ids: Default::default(),
            preview_fallbacks: Default::default(),
            avatar_sources: Default::default(),
            borrowed_avatar_chats: Default::default(),
            read_only: std::env::var("BEEPER_READONLY")
                .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes")),
        })
    }

    fn get_text(&self, path: &str, query: &[(&str, String)]) -> Result<String, String> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url))
            .query(query)
            .send()
            .map_err(|e| format!("Beeper read failed: {}", transport_error(&e)))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("Beeper API GET returned {status}"));
        }
        bounded_json_body(resp)
    }

    fn get<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, String> {
        let body = self.get_text(path, query)?;
        serde_json::from_str(&body).map_err(|_| {
            "Beeper API response had an unexpected format; no private payload was printed".into()
        })
    }

    fn post(&self, path: &str, body: serde_json::Value) -> Result<(), String> {
        if self.read_only {
            return Err("Beeper read-only mode: no changes were sent".into());
        }
        let resp = self
            .http
            .post(format!("{}{path}", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .map_err(|e| {
                format!(
                    "Beeper change could not be confirmed: {}. Check current state before retrying",
                    transport_error(&e)
                )
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("Beeper API POST returned {status}"));
        }
        Ok(())
    }

    fn mutate(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(), String> {
        if self.read_only {
            return Err("Beeper read-only mode: no changes were sent".into());
        }
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request
            .send()
            .map_err(|_| "Beeper change could not be confirmed; refresh before retrying")?;
        if !response.status().is_success() {
            return Err(format!("Beeper change refused: {}", response.status()));
        }
        Ok(())
    }

    /// Patches one boolean chat field. A successful PATCH answers with the
    /// chat itself, and the echoed field is the proof the change held: it
    /// tells a slow or refusing bridge apart from a confirmed change.
    /// `Ok(None)` means the answer carried no proof either way.
    fn change_chat(&self, chat: &str, field: &str, value: bool) -> Result<Option<bool>, String> {
        if self.read_only {
            return Err("Beeper read-only mode: no changes were sent".into());
        }
        let response = self
            .http
            .request(
                reqwest::Method::PATCH,
                format!("{}/v1/chats/{}", self.base_url, encode_segment(chat)),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::json!({field: value}).to_string())
            .send()
            .map_err(|_| "Beeper change could not be confirmed; refresh before retrying")?;
        if !response.status().is_success() {
            return Err(format!("Beeper change refused: {}", response.status()));
        }
        let body = response.text().unwrap_or_default();
        if body.trim().is_empty() {
            return Ok(None);
        }
        #[derive(Deserialize)]
        struct Echo {
            #[serde(rename = "isMuted")]
            is_muted: Option<bool>,
            #[serde(rename = "isArchived")]
            is_archived: Option<bool>,
            #[serde(rename = "isPinned")]
            is_pinned: Option<bool>,
        }
        let echo: Echo = serde_json::from_str(&body).map_err(|_| {
            "Beeper's answer to the change was unreadable; refresh before retrying".to_string()
        })?;
        Ok(match field {
            "isMuted" => echo.is_muted,
            "isArchived" => echo.is_archived,
            "isPinned" => echo.is_pinned,
            _ => None,
        })
    }

    /// The change may still be applying, or the bridge may not support it:
    /// say what is known and where the user's intent still holds.
    fn unconfirmed_change(change: &str, held_locally: bool) -> String {
        if held_locally {
            format!(
                "Beeper hasn't confirmed the {change} on its side; it still holds in Solmu, but other apps may not follow"
            )
        } else {
            format!(
                "Beeper hasn't confirmed the {change} on its side; check the chat before relying on it"
            )
        }
    }

    /// One chat's own record: the decisive check after a change the server
    /// did not echo. Listing absence is never treated as proof either way -
    /// pagination and filters make it unknowable.
    fn get_chat(&self, chat: &str) -> Result<BeeperChat, String> {
        self.get(&format!("/v1/chats/{}", encode_segment(chat)), &[])
    }

    /// One honest report when the server's own chat state contradicts the
    /// change just sent; None when they agree.
    fn unconfirmed_state(field: &str, expected: bool, shown: bool, held: bool) -> Option<String> {
        if shown == expected {
            return None;
        }
        let label = match (field, expected) {
            ("isMuted", true) => "mute",
            ("isMuted", false) => "unmute",
            ("isArchived", true) => "archive",
            ("isArchived", false) => "unarchive",
            ("isPinned", true) => "pin",
            _ => "unpin",
        };
        Some(Self::unconfirmed_change(label, held))
    }

    fn refresh_chat(&self, chat: &str, accounts: &[BeeperAccount], emit: &dyn Fn(Event)) {
        let _ = chat;
        if let Ok(page) = self.list_chats() {
            let label = network_labeller(&page.items, accounts);
            emit(Event::Chats(
                page.items
                    .iter()
                    .map(|c| self.map_chat(c, &label))
                    .collect(),
            ));
        } else {
            emit(Event::Info(
                "Could not refresh chat state after change; list may be stale".into(),
            ));
        }
    }

    fn refresh_message(&self, chat: &str, id: &str, emit: &dyn Fn(Event)) {
        if let Ok(wire) = self.get_message(chat, id) {
            emit(Event::MessageUpdated(Box::new(
                self.map_message(&wire, chat),
            )));
        } else {
            emit(Event::Info("Beeper accepted the change but the updated message could not be fetched; reopen the chat to confirm".into()));
        }
    }

    fn accounts(&self) -> Result<Vec<BeeperAccount>, String> {
        let accounts: Vec<BeeperAccount> = self.get("/v1/accounts", &[])?;
        *self
            .own_ids
            .lock()
            .map_err(|_| "Account identity lock unavailable")? = accounts
            .iter()
            .filter_map(|a| {
                a.user
                    .as_ref()
                    .map(|u| (a.account_id.clone(), u.id.clone()))
            })
            .collect();
        if let Ok(mut sources) = self.avatar_sources.lock() {
            for account in &accounts {
                if let Some(user) = &account.user
                    && let Some(url) = &user.image_url
                {
                    remember_avatar(&mut sources, &user.id, url);
                }
            }
        }
        Ok(accounts)
    }

    fn map_chat(&self, wire: &BeeperChat, label: &dyn Fn(&BeeperChat) -> String) -> Chat {
        if let Ok(mut sources) = self.avatar_sources.lock() {
            if let Some(url) = &wire.image_url {
                remember_avatar(&mut sources, &wire.id, url);
                // The chat now owns its picture; it is no longer borrowed.
                if let Ok(mut borrowed) = self.borrowed_avatar_chats.lock() {
                    borrowed.remove(&wire.id);
                }
            }
            if let Some(participants) = &wire.participants {
                for user in &participants.items {
                    if let Some(url) = &user.image_url {
                        remember_avatar(&mut sources, &user.id, url);
                    }
                }
            }
        }
        let own = self
            .own_ids
            .lock()
            .ok()
            .and_then(|ids| ids.get(&wire.account_id).cloned());
        // A direct chat often carries no avatar of its own; the other person's
        // picture is on their participant record, keyed by their user id, which
        // the chat id never reaches.
        if let (Ok(mut sources), Ok(mut borrowed)) = (
            self.avatar_sources.lock(),
            self.borrowed_avatar_chats.lock(),
        ) {
            match direct_chat_avatar(wire, own.as_deref()) {
                Some(url) => {
                    remember_avatar(&mut sources, &wire.id, url);
                    borrowed.insert(wire.id.clone());
                }
                // The contact's photo is gone and the chat has none of its
                // own: forget the borrowed source so it stops resolving. A
                // chat-owned source and participant-id sources stay.
                // An inventory that leaves `participants` out says nothing about
                // the contact's photo, so only a present list can retire it.
                None if wire.participants.is_some() && borrowed.remove(&wire.id) => {
                    sources.remove(&wire.id);
                }
                None => {}
            }
        }
        let mut chat = map_chat_for(wire, label, own.as_deref());
        chat.backend_origin = Some(self.base_url.clone());
        if chat.last.is_none()
            && let Some(preview) = wire.preview.as_ref().filter(|p| is_empty_preview(p))
            && let Some(real) = self.real_last_message(&wire.id, &preview.id)
        {
            let (body, labelled) = body_of(&real);
            chat.last = Some(Chat::last_preview(&real, body, labelled));
        }
        chat
    }

    /// Beeper's chat preview can be a content-free event (a reaction shows
    /// up this way). Show the newest message that has content instead, found
    /// once per placeholder and remembered so refreshes stay cheap.
    fn real_last_message(&self, chat: &str, preview_id: &str) -> Option<BeeperMessage> {
        let key = (chat.to_string(), preview_id.to_string());
        if let Ok(cache) = self.preview_fallbacks.lock()
            && let Some(known) = cache.get(&key)
        {
            return known.clone();
        }
        let page = self.list_messages(chat, None, None).ok()?;
        let found = newest_content_message(page.items);
        if let Ok(mut cache) = self.preview_fallbacks.lock() {
            if cache.len() > 512 {
                cache.clear();
            }
            cache.insert(key, found.clone());
        }
        found
    }

    fn map_message(&self, wire: &BeeperMessage, chat: &str) -> Message {
        let mut message = map_message(wire, chat);
        if let Ok(ids) = self.own_ids.lock()
            && let Some(own) = ids.get(&wire.account_id)
        {
            for reaction in &mut message.reactions {
                reaction.from_me = reaction.sender == *own;
            }
        }
        message
    }

    fn list_chats(&self) -> Result<Page<BeeperChat>, String> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut pages = 0;
        loop {
            pages += 1;
            check_chat_inventory_bounds(items.len(), pages)?;
            let mut query = vec![("limit", CHAT_PAGE_LIMIT.to_string())];
            if let Some(c) = &cursor {
                query.push(("cursor", c.clone()));
                query.push(("direction", "before".into()));
            }
            let page: Page<BeeperChat> = self.get("/v1/chats", &query)?;
            let more = page.has_more;
            let next = page.oldest_cursor.clone();
            check_chat_inventory_bounds(items.len().saturating_add(page.items.len()), pages)?;
            items.extend(page.items);
            if !more {
                break;
            }
            let next = next.ok_or("chat pagination: server omitted oldestCursor")?;
            if !seen.insert(next.clone()) {
                return Err("chat pagination: repeated cursor".into());
            }
            cursor = Some(next);
        }
        let mut ids = HashSet::new();
        items.retain(|c| ids.insert(c.id.clone()));
        Ok(Page {
            items,
            has_more: false,
            oldest_cursor: None,
            newest_cursor: None,
        })
    }

    fn list_messages(
        &self,
        chat: &str,
        cursor: Option<&str>,
        direction: Option<&str>,
    ) -> Result<Page<BeeperMessage>, String> {
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(c) = cursor {
            q.push(("cursor", c.to_string()));
        }
        if let Some(d) = direction {
            q.push(("direction", d.to_string()));
        }
        let page: Page<BeeperMessage> =
            self.get(&format!("/v1/chats/{}/messages", encode_segment(chat)), &q)?;
        if page
            .items
            .iter()
            .any(|message| !message.chat_id.is_empty() && message.chat_id != chat)
        {
            return Err(
                "History response did not match the requested conversation; no page was applied"
                    .into(),
            );
        }
        Ok(page)
    }

    fn get_message(&self, chat: &str, id: &str) -> Result<BeeperMessage, String> {
        let message: BeeperMessage = self.get(
            &format!(
                "/v1/chats/{}/messages/{}",
                encode_segment(chat),
                encode_segment(id)
            ),
            &[],
        )?;
        if !message.chat_id.is_empty() && message.chat_id != chat {
            return Err("Message response did not match the requested conversation".into());
        }
        Ok(message)
    }

    fn download_avatar(&self, id: &str, dirs: &AccountDirs) -> Result<std::path::PathBuf, String> {
        let source = self
            .avatar_sources
            .lock()
            .map_err(|_| "Avatar identity lock unavailable")?
            .get(id)
            .cloned()
            .ok_or("No avatar supplied")?;
        let source = avatar_source(&source)?;
        let _guard = self.cache_lock.lock().map_err(|_| "Cache busy")?;
        let budget = cache_budget(dirs);
        let limit = (2 * 1024 * 1024u64).min(budget);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&self.base_url, id, &source).hash(&mut hasher);
        let directory = dirs.media_cache_dir();
        std::fs::create_dir_all(&directory).map_err(|_| "Avatar cache unavailable")?;
        let path = directory.join(format!("beeper-avatar-{:016x}.png", hasher.finish()));
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
            return Err("Avatar cache must be a regular file".into());
        }
        if path.is_file() {
            return Ok(path);
        }
        let mut response = self
            .http
            .get(format!("{}/v1/assets/serve", self.base_url))
            .query(&[("url", source)])
            .send()
            .map_err(|_| "Avatar request failed")?;
        if !response.status().is_success() {
            return Err("Avatar unavailable from Beeper asset service".into());
        }
        if response.content_length().is_some_and(|n| n > limit) {
            return Err("Avatar exceeds 2 MiB limit".into());
        }
        let mut bytes = Vec::new();
        (&mut response)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Avatar transfer failed")?;
        if bytes.len() as u64 > limit {
            return Err("Avatar exceeds cache/transfer limit".into());
        }
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| "Avatar format unavailable")?;
        let format = reader.format().ok_or("Unknown avatar format")?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
        ) {
            return Err("Avatar must be PNG, JPEG or WebP".into());
        }
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| "Avatar dimensions unavailable")?;
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4_000_000 {
            return Err("Avatar exceeds 4 megapixel limit".into());
        }
        let decoded = image::load_from_memory_with_format(&bytes, format)
            .map_err(|_| "Avatar decode failed")?
            .thumbnail(512, 512);
        let mut normalized = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut normalized, image::ImageFormat::Png)
            .map_err(|_| "Avatar conversion failed")?;
        let bytes = normalized.into_inner();
        if bytes.len() as u64 > limit {
            return Err("Converted avatar exceeds cache limit".into());
        }
        trim_cache(dirs, budget.saturating_sub(bytes.len() as u64))
            .map_err(|_| "Avatar cache budget unavailable")?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&path)
            .map_err(|_| "Avatar cache write refused")?;
        if output.write_all(&bytes).is_err() {
            let _ = std::fs::remove_file(&path);
            return Err("Avatar cache write failed".into());
        }
        Ok(path)
    }

    fn download_media(
        &self,
        chat: &str,
        message: &str,
        dirs: &AccountDirs,
        manual: bool,
    ) -> Result<std::path::PathBuf, String> {
        self.download_media_at(chat, message, 0, dirs, manual)
    }

    fn download_media_at(
        &self,
        chat: &str,
        message: &str,
        index: usize,
        dirs: &AccountDirs,
        manual: bool,
    ) -> Result<std::path::PathBuf, String> {
        let _guard = self.cache_lock.lock().map_err(|_| "Cache is busy")?;
        let budget = cache_budget(dirs);
        let m: BeeperMessage = self.get_message(chat, message)?;
        let attachment = m
            .attachments
            .get(index)
            .ok_or("Message attachment is no longer available")?;
        if attachment
            .size
            .as_ref()
            .is_some_and(|size| u64::from(size.width) * u64::from(size.height) > 16_000_000)
        {
            return Err("Image dimensions exceed the 16 megapixel decode budget".into());
        }
        let source = attachment
            .id
            .as_deref()
            .or(attachment.src_url.as_deref())
            .ok_or("Attachment has no media URL")?;
        if !["mxc://", "localmxc://", "file://"]
            .iter()
            .any(|prefix| source.starts_with(prefix))
        {
            return Err(
                "Unsupported media source scheme; only Beeper media URLs are served".into(),
            );
        }
        // Always fetch through the authenticated API. Never send its token to an attachment host.
        let mut response = self
            .http
            .get(format!("{}/v1/assets/serve", self.base_url))
            .query(&[("url", source)])
            .send()
            .map_err(|_| "Media request failed")?;
        if !response.status().is_success() {
            return Err(format!("Media server returned {}", response.status()));
        }
        let limit = (if manual {
            500 * 1024 * 1024
        } else {
            64 * 1024 * 1024
        })
        .min(budget);
        if attachment.file_size > limit || response.content_length().is_some_and(|n| n > limit) {
            return Err("Attachment exceeds download size limit".into());
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (chat, message, source).hash(&mut hasher);
        let mut head = Vec::with_capacity(64);
        let mut ext = media_extension(
            attachment.mime_type.as_deref(),
            attachment.file_name.as_deref(),
            None,
            &[],
        );
        if ext == "bin" {
            // Telegram often sends no usable type. Read the content type and the
            // first bytes before choosing the name so the file opens normally.
            let header = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            (&mut response)
                .take(64)
                .read_to_end(&mut head)
                .map_err(|_| "Media transfer failed")?;
            ext = media_extension(
                attachment.mime_type.as_deref(),
                attachment.file_name.as_deref(),
                header.as_deref(),
                &head,
            );
        }
        let directory = dirs.media_cache_dir();
        std::fs::create_dir_all(&directory).map_err(|_| "Could not create media cache")?;
        let path = directory.join(format!("beeper-{:016x}.{ext}", hasher.finish()));
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err("Cache entry must not be a symlink".into());
        }
        if path.is_file() {
            return Ok(path);
        }
        // Reserve the maximum transfer before writing. Complete files plus the
        // in-flight file stay within the budget even without Content-Length.
        trim_cache(dirs, budget.saturating_sub(limit))
            .map_err(|_| "Could not enforce cache budget")?;
        let temporary = path.with_extension("part");
        if std::fs::symlink_metadata(&temporary).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Temporary cache entry must not be a symlink".into());
        }
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            // A stale partial is disposable; remove it rather than following
            // an attacker-controlled existing file or symlink.
            if temporary.exists() {
                std::fs::remove_file(&temporary).map_err(|_| "Could not replace stale partial")?;
            }
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options
                .open(&temporary)
                .map_err(|_| "Could not create media file")?;
            output.write_all(&head).map_err(|_| "Media write failed")?;
            let copied = head.len() as u64
                + std::io::copy(
                    &mut (&mut response).take(limit.saturating_sub(head.len() as u64)),
                    &mut output,
                )
                .map_err(|_| "Media transfer failed")?;
            let mut excess = [0u8; 1];
            if copied == limit
                && response
                    .read(&mut excess)
                    .map_err(|_| "Media transfer failed")?
                    != 0
            {
                return Err("Attachment exceeds download size limit");
            }
            output.flush().map_err(|_| "Media write failed")?;
            std::fs::rename(&temporary, &path).map_err(|_| "Could not finish media file")?;
            Ok(path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result.map_err(str::to_string)
    }

    fn plan_forwards(
        &self,
        items: Vec<crate::composed_forward::Item>,
    ) -> Vec<crate::composed_forward::Item> {
        use crate::composed_forward::State;
        let mut expanded = Vec::new();
        for mut item in items {
            if item.origin != self.base_url {
                item.state = State::FailedBeforeSend(
                    "Source belongs to another server; nothing submitted".into(),
                );
                expanded.push(item);
                continue;
            }
            if item.planned {
                expanded.push(item);
                continue;
            }
            match self.get_message(&item.source_chat, &item.source_message) {
                Ok(message) if !message.attachments.is_empty() => {
                    for attachment in 0..message.attachments.len() {
                        let mut copy = item.clone();
                        copy.attachment = Some(attachment);
                        copy.planned = true;
                        if attachment > 0 {
                            copy.text = item.text.lines().next().unwrap_or("").to_owned();
                        }
                        expanded.push(copy);
                    }
                }
                Ok(message) => {
                    if !forwardable_text(&message) {
                        item.state = State::FailedBeforeSend(
                            "This source type has no supported text or attachment to forward"
                                .into(),
                        );
                    }
                    item.planned = true;
                    expanded.push(item);
                }
                Err(error) => {
                    let mut copy = item;
                    copy.state = State::FailedBeforeSend(error);
                    expanded.push(copy);
                }
            }
        }
        expanded
    }

    /// Only failures before the message POST are safely retryable. Once that
    /// POST begins, even a malformed response must not invite a duplicate send.
    fn send_forward(
        &self,
        item: &crate::composed_forward::Item,
        dirs: &AccountDirs,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<SendAccepted, crate::composed_forward::State> {
        use crate::composed_forward::State;
        let prepare = || -> Result<serde_json::Value, String> {
            if item.origin != self.base_url {
                return Err("Source belongs to another server; nothing submitted".into());
            }
            if self.read_only || upload_stopped(Some(cancel)) {
                return Err("Forward was not submitted: read-only or session closed".into());
            }
            let source = self.get_chat(&item.source_chat)?;
            let destination = self.get_chat(&item.destination)?;
            if source.account_id != item.source_account
                || destination.account_id != item.destination_account
                || destination.id != item.destination
                || source.id != item.source_chat
                || destination.is_read_only
            {
                return Err(
                    "Source or destination identity changed; forward was not submitted".into(),
                );
            }
            let message = self.get_message(&item.source_chat, &item.source_message)?;
            if message.is_hidden
                || message.is_deleted
                || message.id != item.source_message
                || message.chat_id != item.source_chat
                || message.account_id != item.source_account
            {
                return Err("Source message is no longer available; nothing submitted".into());
            }
            if item.attachment.is_none()
                && (!forwardable_text(&message) || item.text.lines().count() < 2)
            {
                return Err("This message has no supported text or attachment to forward".into());
            }
            let mut payload = serde_json::json!({"text": item.text});
            if let Some(index) = item.attachment {
                let path = self.download_media_at(
                    &item.source_chat,
                    &item.source_message,
                    index,
                    dirs,
                    true,
                )?;
                use base64::Engine;
                let bytes = read_attachment_bounded(&path, 32 * 1024 * 1024)?;
                let filename = message
                    .attachments
                    .get(index)
                    .and_then(|a| a.file_name.as_deref())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("attachment");
                let response = self.http.post(format!("{}/v1/assets/upload/base64", self.base_url))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(serde_json::json!({"content": base64::engine::general_purpose::STANDARD.encode(bytes), "fileName": filename}).to_string())
                    .send().map_err(|_| "Asset upload failed; message was not submitted")?;
                if !response.status().is_success() {
                    return Err(format!(
                        "Asset upload returned {}; message was not submitted",
                        response.status()
                    ));
                }
                let uploaded: serde_json::Value =
                    serde_json::from_str(&bounded_json_body(response)?)
                        .map_err(|_| "Asset upload response invalid; message was not submitted")?;
                if uploaded
                    .get("error")
                    .and_then(|error| error.as_str())
                    .is_some_and(|error| !error.is_empty())
                {
                    return Err("Asset upload was rejected; message was not submitted".into());
                }
                let id = uploaded
                    .get("uploadID")
                    .and_then(|id| id.as_str())
                    .ok_or("Asset upload ID missing; message was not submitted")?;
                payload["attachment"] = serde_json::json!({"uploadID": id, "fileName": filename});
            }
            if upload_stopped(Some(cancel)) {
                return Err("Session closed; message was not submitted".into());
            }
            Ok(payload)
        };
        let payload = prepare().map_err(State::FailedBeforeSend)?;
        let response = self
            .http
            .post(format!(
                "{}/v1/chats/{}/messages",
                self.base_url,
                encode_segment(&item.destination)
            ))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.to_string())
            .send()
            .map_err(|_| {
                State::Uncertain(
                    "Send outcome unknown; check destination history before retrying".into(),
                )
            })?;
        if !response.status().is_success() {
            return Err(State::Uncertain(safe_send_error(response)));
        }
        let body = bounded_json_body(response).map_err(State::Uncertain)?;
        let accepted: SendAccepted = serde_json::from_str(&body).map_err(|_| {
            State::Uncertain(
                "Send response unreadable; check destination history before retrying".into(),
            )
        })?;
        if accepted.pending_id.is_empty() || accepted.chat != item.destination {
            return Err(State::Uncertain(
                "Unexpected send identity; check destination history before retrying".into(),
            ));
        }
        Ok(accepted)
    }

    fn send_text(
        &self,
        chat: &str,
        text: &str,
        reply: Option<&str>,
        mentions: &[String],
    ) -> Result<SendAccepted, String> {
        if self.read_only {
            return Err("Beeper read-only mode: no changes were sent".into());
        }
        let mut payload = serde_json::json!({"text":text});
        if let Some(reply) = reply {
            payload["replyToMessageID"] = reply.into();
        }
        if !mentions.is_empty() {
            payload["mentions"] = mentions.into();
        }
        let response = self
            .http
            .post(format!(
                "{}/v1/chats/{}/messages",
                self.base_url,
                encode_segment(chat)
            ))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.to_string())
            .send()
            .map_err(|_| "Send request outcome unknown; check history before retrying")?;
        if !response.status().is_success() {
            return Err(safe_send_error(response));
        }
        let body = bounded_json_body(response)
            .map_err(|e| format!("{e}; send outcome unknown, check history before retrying"))?;
        serde_json::from_str(&body).map_err(|_| {
            "Send accepted but response could not be read; check history before retrying".into()
        })
    }

    #[cfg(test)]
    fn send_file(
        &self,
        chat: &str,
        path: &std::path::Path,
        documents: bool,
        caption: Option<&str>,
    ) -> Result<SendAccepted, String> {
        let kind = if documents {
            Attachment::Document
        } else {
            Attachment::Auto
        };
        self.send_attachment(chat, path, kind, caption, None, None)
    }

    #[cfg(test)]
    fn send_file_cancellable(
        &self,
        chat: &str,
        path: &std::path::Path,
        documents: bool,
        caption: Option<&str>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<SendAccepted, String> {
        let kind = if documents {
            Attachment::Document
        } else {
            Attachment::Auto
        };
        self.send_attachment(chat, path, kind, caption, None, cancel)
    }

    fn send_attachment(
        &self,
        chat: &str,
        path: &std::path::Path,
        kind: Attachment,
        caption: Option<&str>,
        reply: Option<&str>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<SendAccepted, String> {
        self.send_attachment_tracked(chat, path, kind, caption, reply, cancel, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_attachment_tracked(
        &self,
        chat: &str,
        path: &std::path::Path,
        kind: Attachment,
        caption: Option<&str>,
        reply: Option<&str>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        post_started: Option<&std::sync::atomic::AtomicBool>,
        expected: Option<&crate::merged::Member>,
    ) -> Result<SendAccepted, String> {
        if upload_stopped(cancel) {
            return Err("Upload worker stopped; message was not submitted".into());
        }
        if self.read_only {
            return Err("Beeper read-only mode: no changes were sent".into());
        }
        use base64::Engine;
        // Read the opened file through a bound, including if it grows during
        // reading. Checking its earlier path metadata does not bound allocation.
        let bytes = read_attachment_bounded(path, 32 * 1024 * 1024)?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("attachment");
        let mut body = serde_json::json!({ "content": base64::engine::general_purpose::STANDARD.encode(bytes), "fileName": filename });
        // Our recorded clips are known Ogg/Opus, not arbitrary Ogg audio.
        // WhatsApp voice capabilities distinguish this from generic audio/ogg.
        if matches!(kind, Attachment::Voice(_)) {
            body["mimeType"] = "audio/ogg; codecs=opus".into();
        }
        if upload_stopped(cancel) {
            return Err("Upload worker stopped; message was not submitted".into());
        }
        let response = self
            .http
            .post(format!("{}/v1/assets/upload/base64", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .map_err(|_| "Attachment upload failed; message was not submitted")?;
        if !response.status().is_success() {
            return Err(format!(
                "Attachment upload returned {}; message was not submitted",
                response.status()
            ));
        }
        let upload: serde_json::Value = serde_json::from_str(
            &bounded_json_body(response)
                .map_err(|e| format!("{e}; attachment message was not submitted"))?,
        )
        .map_err(|_| "Upload response invalid; attachment message was not submitted")?;
        if upload
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| !e.is_empty())
        {
            return Err("Asset upload rejected; message was not submitted".into());
        }
        let id = upload
            .get("uploadID")
            .and_then(|s| s.as_str())
            .ok_or("Upload did not return an ID; message was not submitted")?;
        if upload_stopped(cancel) {
            return Err(
                "Upload worker stopped after asset upload; message was not submitted".into(),
            );
        }
        let mut attachment = serde_json::json!({"uploadID": id, "fileName": filename});
        match kind {
            Attachment::Auto => {}
            Attachment::Document => attachment["type"] = "file".into(),
            Attachment::Gif => attachment["type"] = "gif".into(),
            Attachment::Sticker => attachment["type"] = "sticker".into(),
            Attachment::Voice(seconds) => {
                attachment["type"] = "voice-note".into();
                attachment["mimeType"] = "audio/ogg; codecs=opus".into();
                attachment["duration"] = seconds.into();
            }
        }
        // The contract's text is an optional string: with no caption the
        // field goes away. An explicit null is not a string, and Beeper
        // answers the message send with 400 Bad Request.
        let mut message = serde_json::json!({ "attachment": attachment });
        if let Some(caption) = caption {
            message["text"] = caption.into();
        }
        if let Some(reply) = reply {
            message["replyToMessageID"] = reply.into();
        }
        if let Some(expected) = expected {
            let current = self.get_chat(chat)?;
            if current.id != expected.chat
                || current.account_id != expected.account
                || current.is_read_only
            {
                return Err("Destination changed before submission; nothing submitted".into());
            }
            if upload_stopped(cancel) {
                return Err("Session closed; nothing submitted".into());
            }
        }
        if let Some(started) = post_started {
            started.store(true, std::sync::atomic::Ordering::Release);
        }
        let response = self
            .http
            .post(format!(
                "{}/v1/chats/{}/messages",
                self.base_url,
                encode_segment(chat)
            ))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(message.to_string())
            .send()
            .map_err(|_| "Attachment send outcome unknown; check history before retrying")?;
        if !response.status().is_success() {
            // Same whitelisted-hint error as text sends: a validation 400
            // names the rejected field without echoing message content.
            return Err(safe_send_error(response));
        }
        serde_json::from_str(&bounded_json_body(response).map_err(|e| {
            format!("{e}; attachment send outcome unknown, check history before retrying")
        })?)
        .map_err(|_| "Attachment confirmation invalid; check history before retrying".into())
    }

    fn send_pending_media(
        &self,
        job: &crate::pending_media::Job,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<SendAccepted, crate::pending_media::State> {
        use crate::pending_media::{Payload, State};
        let prepare = || -> Result<(), String> {
            if job.member.origin != self.base_url || self.read_only || upload_stopped(Some(cancel))
            {
                return Err("Destination unavailable or session closed; nothing submitted".into());
            }
            let chat = self.get_chat(&job.member.chat)?;
            if chat.id != job.member.chat
                || chat.account_id != job.member.account
                || chat.is_read_only
            {
                return Err(
                    "Destination identity changed or is read-only; nothing submitted".into(),
                );
            }
            if job
                .quoting
                .as_ref()
                .is_some_and(|q| q.source_chat(&job.member.chat) != job.member.chat)
            {
                return Err("Cross-chat quote unsupported; nothing submitted".into());
            }
            Ok(())
        };
        prepare().map_err(State::NotSubmitted)?;
        let temporary;
        let (path, kind) = match &job.payload {
            Payload::File { path, document } => (
                path.as_path(),
                if *document {
                    Attachment::Document
                } else {
                    Attachment::Auto
                },
            ),
            Payload::Picture {
                width,
                height,
                rgba,
            } => {
                temporary =
                    encode_pasted_image(*width, *height, rgba).map_err(State::NotSubmitted)?;
                (temporary.as_path(), Attachment::Auto)
            }
        };
        let started = std::sync::atomic::AtomicBool::new(false);
        let result = self.send_attachment_tracked(
            &job.member.chat,
            path,
            kind,
            job.caption.as_deref(),
            job.quoting.as_ref().map(|q| q.id.as_str()),
            Some(cancel),
            Some(&started),
            Some(&job.member),
        );
        // The job retains pasted pixels for safe retry; never remove user files.
        if matches!(job.payload, Payload::Picture { .. }) {
            discard_temp_upload(path);
        }
        match result {
            Ok(mut accepted)
                if (accepted.chat.is_empty() || accepted.chat == job.member.chat)
                    && !accepted.pending_id.is_empty() =>
            {
                accepted.chat = job.member.chat.clone();
                Ok(accepted)
            }
            Ok(_) => Err(State::Unknown(
                "Unexpected acceptance identity; check chat history".into(),
            )),
            Err(error) if started.load(std::sync::atomic::Ordering::Acquire) => {
                Err(State::Unknown(error))
            }
            Err(error) => Err(State::NotSubmitted(error)),
        }
    }

    fn mark_read(&self, chat: &str) -> Result<(), String> {
        self.post(
            &format!("/v1/chats/{}/read", encode_segment(chat)),
            serde_json::json!({}),
        )
    }

    fn mark_unread(&self, chat: &str) -> Result<(), String> {
        self.post(
            &format!("/v1/chats/{}/unread", encode_segment(chat)),
            serde_json::json!({}),
        )
    }

    fn typing(&self, chat: &str, composing: bool) -> Result<(), String> {
        self.post(
            &format!("/v1/chats/{}/typing", encode_segment(chat)),
            serde_json::json!({ "state": if composing { "typing" } else { "paused" } }),
        )
    }
}

// ---------- Mapping: Beeper -> solmu model ----------

fn unix(ts: &Option<String>) -> i64 {
    ts.as_deref()
        .and_then(|s| jiff::Timestamp::from_str(s).ok())
        .map(|t| t.as_second())
        .unwrap_or(0)
}

/// The picture a direct chat should show when the chat has none of its own:
/// the other participant's. Groups never borrow a member's picture, and a
/// direct chat with several other candidates is left alone.
fn direct_chat_avatar<'a>(wire: &'a BeeperChat, own: Option<&str>) -> Option<&'a str> {
    if wire.chat_type != "single" || wire.image_url.as_deref().is_some_and(|u| !u.is_empty()) {
        return None;
    }
    let mut others = wire
        .participants
        .as_ref()?
        .items
        .iter()
        .filter(|user| Some(user.id.as_str()) != own && user.is_self != Some(true));
    let other = others.next()?;
    if others.next().is_some() {
        return None;
    }
    other.image_url.as_deref().filter(|url| !url.is_empty())
}

// Source hints are disposable, not an unlimited participant directory.
fn remember_avatar(sources: &mut HashMap<String, String>, id: &str, url: &str) {
    if id.len() > 4096 || url.len() > 16384 {
        return;
    }
    if sources.len() >= 4096
        && !sources.contains_key(id)
        && let Some(key) = sources.keys().next().cloned()
    {
        sources.remove(&key);
    }
    sources.insert(id.to_owned(), url.to_owned());
}
fn avatar_source(raw: &str) -> Result<String, String> {
    if ["mxc://", "localmxc://", "file://"]
        .iter()
        .any(|p| raw.starts_with(p))
    {
        return Ok(raw.to_owned());
    }
    if std::path::Path::new(raw).is_absolute() {
        return reqwest::Url::from_file_path(raw)
            .map(|u| u.to_string())
            .map_err(|_| "Invalid local avatar path".into());
    }
    // Never forward a Beeper bearer token to an avatar host, fetch an arbitrary
    // HTTP URL, interpret data/SVG markup, or read server paths on this client.
    Err("Avatar source not supported by Beeper asset service".into())
}

fn opaque_identity(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('@') && value.contains(':')
}

fn readable_name(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .filter(|v| !v.trim().is_empty() && !opaque_identity(v))
        .cloned()
}

/// Mention links in a rich body, as (id, label) pairs the mapper records:
/// the bridge marks a mention up as a matrix.to anchor whose text is the
/// person's name. Labels carry the same basic entity decoding the text gets.
fn rich_text_mentions(raw: &str) -> Vec<crate::model::MentionRef> {
    const MARK: &str = "https://matrix.to/#/@";
    let mut mentions = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find(MARK) {
        let after = &rest[start + MARK.len()..];
        let Some(id_end) = after.find(['"', '\'']) else {
            break;
        };
        let id = format!("@{}", &after[..id_end]);
        let Some(label_start) = after[id_end..]
            .find('>')
            .map(|i| start + MARK.len() + id_end + i + 1)
        else {
            break;
        };
        let Some(label_end) = rest[label_start..].find("</a>").map(|i| label_start + i) else {
            break;
        };
        let label = decode_entities(plain_rich_text(&rest[label_start..label_end]).trim());
        if !label.is_empty()
            && !mentions
                .iter()
                .any(|m: &crate::model::MentionRef| m.id == id)
        {
            mentions.push(crate::model::MentionRef { user: label, id });
        }
        rest = &rest[label_end + 4..];
    }
    mentions
}

/// Conservative rich-text fallback. Never executes markup, loads assets or
/// interprets CSS. Only an explicitly HTML-looking body is converted; ordinary
/// angle-bracket text is left alone. Unknown entities remain literal.
fn safe_html_href(tag: &str) -> Option<String> {
    let mut attrs = tag
        .split_once(char::is_whitespace)
        .map(|(_, attrs)| attrs)
        .unwrap_or("")
        .trim();
    while !attrs.is_empty() {
        let key_end = attrs
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(attrs.len());
        let key = &attrs[..key_end];
        attrs = attrs[key_end..].trim_start();
        if !attrs.starts_with('=') {
            if key_end == 0 {
                attrs = &attrs[1..];
            }
            continue;
        }
        attrs = attrs[1..].trim_start();
        let (value, remaining) =
            if let Some(q) = attrs.chars().next().filter(|ch| *ch == '\'' || *ch == '"') {
                let tail = &attrs[q.len_utf8()..];
                let end = tail.find(q)?;
                (&tail[..end], &tail[end + q.len_utf8()..])
            } else {
                let end = attrs.find(char::is_whitespace).unwrap_or(attrs.len());
                (&attrs[..end], &attrs[end..])
            };
        attrs = remaining.trim_start();
        if key.eq_ignore_ascii_case("href") {
            let value = value.replace("&amp;", "&");
            let url = reqwest::Url::parse(&value).ok()?;
            if matches!(url.scheme(), "https" | "http" | "mailto")
                && url.username().is_empty()
                && url.password().is_none()
            {
                return Some(value);
            }
            return None;
        }
    }
    None
}

/// The HTML entity decoding both halves of mention handling share, so the
/// body's @token and the mention's label always agree, HTML-looking or not.
fn decode_entities(text: &str) -> String {
    let mut decoded = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find(';').filter(|n| *n <= 16) {
            let entity = &rest[1..end];
            let replacement = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|n| u32::from_str_radix(n, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(ch) = replacement {
                decoded.push(ch);
                rest = &rest[end + 1..];
                continue;
            }
        }
        decoded.push('&');
        rest = &rest[1..];
    }
    decoded.push_str(rest);
    decoded
}

fn plain_rich_text(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let known = [
        "p",
        "div",
        "br",
        "html",
        "span",
        "a",
        "strong",
        "b",
        "ul",
        "ol",
        "li",
        "blockquote",
        "script",
        "style",
        "iframe",
        "object",
        "em",
        "i",
        "img",
        "h1",
        "h2",
        "h3",
        "tr",
        "del",
        "s",
        "strike",
        "code",
        "pre",
    ];
    let looks_html = lower.match_indices('<').any(|(start, _)| {
        let tail = lower[start + 1..].trim_start_matches('/');
        known.iter().any(|name| {
            tail.strip_prefix(name).is_some_and(|rest| {
                rest.starts_with('>')
                    || rest.starts_with('/')
                    || rest.starts_with(char::is_whitespace)
            })
        })
    });
    if !looks_html {
        // The API's contract for this field is rich text, not raw user
        // input: developers.beeper.com documents `text` as the "Rich-text
        // message body", and sending converts plain text and Markdown into
        // the same normalized form. The 2026-11-03 live probe (#136) showed
        // the escaping on the wire: the reported Dropbox link's separator
        // arrived as hex 26 61 6d 70 3b ("&amp;"), with zero bare ampersands
        // in the whole text; the reported WhatsApp quotes arrived as &quot;.
        // So entities are the transport encoding of this field and decode
        // one level: text the sender typed literally arrived double-escaped
        // ("&amp;quot;") and survives one level as the literal. A bridge
        // violating the contract with raw, unescaped text containing
        // entity-looking literals would misdecode - the accepted residual
        // risk #136 documented, now for the whole field.
        return decode_entities(raw);
    }
    let mut out = String::new();
    let mut rest = raw;
    let mut blocked: Option<String> = None;
    let mut link: Option<String> = None;
    let mut link_label: Option<usize> = None;
    // Semantic styles survive as the WhatsApp-style markup the renderer
    // already parses: markers are emitted where the tags sit, so properly
    // nested HTML styles correctly, and overlapping tags style by their
    // spans. Raw HTML is never emitted.
    let mut in_pre = false;
    let mut quote_depth: usize = 0;
    // Styles covering the current text position, oldest first. Markers
    // materialize lazily, at the first text char they cover, and close at
    // line ends because the renderer parses styles per line. A line or
    // pair without styled text never sees a marker at all.
    let mut styles: Vec<char> = Vec::new();
    // The prefix of `styles` whose markers are emitted on this line.
    let mut emitted: Vec<char> = Vec::new();
    // Inside inline code, backticks and backslashes are escaped so code
    // content cannot close its own span.
    let mut in_code = false;
    // Fenced content buffers so a content line that would collide with
    // the fence gets the backslash escape the renderer resolves.
    let mut pre_content = String::new();
    // Set when a style marker just closed: a zero-width space before a
    // following alphanumeric keeps the marker a closer at the renderer's
    // word-boundary rules without a visible character. The hint fires
    // only while the closed marker is still the last emitted char, so
    // neutral tags in between break nothing.
    let mut pending_boundary: Option<char> = None;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            // A quoted > is an attribute value, not the end of a tag.
            let mut quote = None;
            let end = rest.char_indices().skip(1).find_map(|(index, ch)| {
                if let Some(q) = quote {
                    if ch == q {
                        quote = None;
                    }
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                } else if ch == '>' {
                    return Some(index);
                }
                None
            });
            if let Some(end) = end {
                let tag = rest[1..end].trim();
                let name = tag
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let closing = tag.starts_with('/');
                if !known.contains(&name.as_str()) && blocked.is_none() {
                    out.push('<');
                    rest = &rest[1..];
                    continue;
                }
                if matches!(name.as_str(), "script" | "style" | "iframe" | "object") {
                    if closing {
                        if blocked.as_deref() == Some(&name) {
                            blocked = None;
                        }
                    } else {
                        blocked = Some(name);
                    }
                } else if blocked.is_none() {
                    if !in_pre {
                        let marker = match name.as_str() {
                            "strong" | "b" => Some('*'),
                            "em" | "i" => Some('_'),
                            "del" | "s" | "strike" => Some('~'),
                            "code" => Some('`'),
                            _ => None,
                        };
                        if let Some(marker) = marker {
                            if closing {
                                match styles.iter().rposition(|open| *open == marker) {
                                    // The style never covered text: no
                                    // marker pair is emitted at all.
                                    Some(at) if at >= emitted.len() => {
                                        styles.remove(at);
                                    }
                                    Some(at) => {
                                        styles.remove(at);
                                        emitted.remove(at);
                                        out.push(marker);
                                        pending_boundary = Some(marker);
                                    }
                                    // A close for a marker that never
                                    // opened stays a literal character.
                                    None => out.push(marker),
                                }
                                if marker == '`' {
                                    in_code = false;
                                }
                            } else {
                                styles.push(marker);
                                if marker == '`' {
                                    in_code = true;
                                }
                            }
                        }
                    }
                    if name == "pre" {
                        if closing {
                            if in_pre {
                                in_pre = false;
                                // Escape a content line that would collide
                                // with the fence: the backslash goes right
                                // before the backticks, so the renderer
                                // resolves it and any indentation survives.
                                let trimmed = pre_content.trim_end_matches('\n');
                                for (n, line) in trimmed.split('\n').enumerate() {
                                    if n > 0 {
                                        out.push('\n');
                                    }
                                    if line.trim() == "```" {
                                        let at = line.find("```").unwrap_or(0);
                                        out.push_str(&line[..at]);
                                        out.push('\\');
                                        out.push_str(&line[at..]);
                                    } else {
                                        out.push_str(line);
                                    }
                                }
                                pre_content.clear();
                                if !out.ends_with('\n') {
                                    out.push('\n');
                                }
                                out.push_str("```\n");
                                // Styles suspended for the block re-emit
                                // lazily at the next text char.
                            }
                        } else if !in_pre {
                            in_pre = true;
                            pre_content.clear();
                            // Styles emitted on this line close before the
                            // fence line; they re-emit lazily after it.
                            for marker in emitted.iter().rev() {
                                out.push(*marker);
                            }
                            emitted.clear();
                            if !out.ends_with('\n') {
                                out.push('\n');
                            }
                            out.push_str("```\n");
                        }
                    }
                    if name == "blockquote" && !in_pre {
                        if closing {
                            quote_depth = quote_depth.saturating_sub(1);
                            // An empty trailing quote line adds nothing.
                            if out.ends_with("> ") {
                                out.truncate(out.len() - 2);
                            }
                            let broke = !out.ends_with('\n');
                            if broke {
                                for marker in emitted.iter().rev() {
                                    out.push(*marker);
                                }
                                emitted.clear();
                                out.push('\n');
                            }
                            // Text after a nested quote still belongs to
                            // the outer one: its bar stays.
                            if quote_depth > 0 && !out.ends_with("> ") {
                                out.push_str("> ");
                            }
                        } else {
                            let broke = !out.ends_with('\n') && !out.ends_with("> ");
                            if broke {
                                for marker in emitted.iter().rev() {
                                    out.push(*marker);
                                }
                                emitted.clear();
                                out.push('\n');
                            }
                            quote_depth += 1;
                            out.push_str("> ");
                        }
                    }
                    if name == "a" {
                        if closing {
                            let label_start = link_label.take();
                            if let Some(url) = link.take() {
                                // A matrix.to mention link already carries the
                                // person's name as its text; the raw URL is noise.
                                if !url.starts_with("https://matrix.to/#/@") {
                                    // Entities in the attribute decode here,
                                    // where the URL is emitted.
                                    let url = decode_entities(&url);
                                    // A link whose text already is the address -
                                    // a pasted URL the bridge marked up - keeps
                                    // just the text; repeating the address in
                                    // parentheses would show such links twice.
                                    let shows_address = label_start.is_some_and(|start| {
                                        let label = out[start..].trim();
                                        label == url
                                            || label.trim_end_matches('/')
                                                == url.trim_end_matches('/')
                                    });
                                    if !shows_address {
                                        out.push_str(" (");
                                        out.push_str(&url);
                                        out.push(')');
                                    }
                                }
                            }
                        } else {
                            link = safe_html_href(tag);
                            // Mention links render as their @name token, as
                            // WhatsApp's do, so markup can chip them.
                            if link
                                .as_deref()
                                .is_some_and(|url| url.starts_with("https://matrix.to/#/@"))
                            {
                                out.push('@');
                            }
                            link_label = link.as_ref().map(|_| out.len());
                        }
                    }
                    if matches!(
                        name.as_str(),
                        "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3"
                    ) {
                        let broke = !out.ends_with('\n') && !out.ends_with("> ");
                        if broke {
                            for marker in emitted.iter().rev() {
                                out.push(*marker);
                            }
                            emitted.clear();
                            out.push('\n');
                        }
                        if quote_depth > 0 && !in_pre && !out.ends_with("> ") {
                            out.push_str("> ");
                        }
                        if name == "li" && !closing {
                            out.push_str("• ");
                        }
                    }
                }
                rest = &rest[end + 1..];
                continue;
            }
        }
        // A text run up to the next tag. Entities decode here, before
        // the code escaping sees the text, so an entity-encoded backtick
        // or fence line cannot slip past the escaping.
        let end = rest.find('<').unwrap_or(rest.len());
        let (taken, run) = if end == 0 {
            // A '<' that opens no known tag is literal text.
            let ch = rest.chars().next().unwrap();
            (ch.len_utf8(), ch.to_string())
        } else {
            (end, decode_entities(&rest[..end]))
        };
        rest = &rest[taken..];
        if blocked.is_some() {
            continue;
        }
        for ch in run.chars() {
            if in_pre {
                pre_content.push(ch);
                continue;
            }
            if ch == '\n' {
                // A break closes the styles emitted on this line; they
                // re-emit lazily at the next text char.
                pending_boundary = None;
                for marker in emitted.iter().rev() {
                    out.push(*marker);
                }
                emitted.clear();
                out.push('\n');
                if quote_depth > 0 {
                    out.push_str("> ");
                }
                continue;
            }
            // Markers covering this char materialize now: after a break,
            // after a neutral tag, wherever the text actually is. A
            // style that opens mid-word gets the zero-width boundary
            // hint so the renderer accepts the marker.
            while emitted.len() < styles.len() {
                let marker = styles[emitted.len()];
                if out.ends_with(char::is_alphanumeric) {
                    out.push('\u{200B}');
                }
                out.push(marker);
                emitted.push(marker);
            }
            if let Some(closed) = pending_boundary.take()
                && ch.is_alphanumeric()
                && out.ends_with(closed)
            {
                out.push('\u{200B}');
            }
            if in_code && matches!(ch, '`' | '\\') {
                out.push('\\');
            }
            out.push(ch);
        }
    }
    // Text runs and URLs decode at emission, so `out` is already decoded:
    // a second pass would misdecode literal entity-looking text.
    out.trim().to_owned()
}

/// A message with nothing to show: no text, no attachment, and a type that
/// carries no content of its own. Reactions reach the chat list this way.
/// A message's emoji reactions as one run, each distinct emoji once in the
/// order first seen, with a count when several people used it: "🤝 👍2".
/// Reactions that are not emoji (custom stickers) are left out.
fn reaction_run(reactions: &[BeeperReaction]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for reaction in reactions.iter().filter(|r| r.emoji) {
        let key = reaction.reaction_key.as_str();
        match counts.iter_mut().find(|entry| entry.0 == key) {
            Some(entry) => entry.1 += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(key, count)| {
            if count > 1 {
                format!("{key}{count}")
            } else {
                key.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_empty_preview(m: &BeeperMessage) -> bool {
    m.text.as_deref().is_none_or(str::is_empty)
        && m.attachments.is_empty()
        && matches!(
            m.message_type.as_deref(),
            None | Some("" | "REACTION" | "TEXT")
        )
}

/// The newest message that has something to show: not hidden or deleted, not
/// a content-free event, and not a system notice.
fn newest_content_message(items: Vec<BeeperMessage>) -> Option<BeeperMessage> {
    items
        .into_iter()
        .filter(|m| {
            m.is_sender.is_some()
                && !m.is_hidden
                && !m.is_deleted
                && !is_empty_preview(m)
                && !matches!(
                    m.message_type.as_deref(),
                    Some("SYSTEM" | "NOTICE" | "EVENT")
                )
        })
        .max_by_key(|m| unix(&Some(m.timestamp.clone())))
}

fn forwardable_text(message: &BeeperMessage) -> bool {
    matches!(message.message_type.as_deref(), None | Some("TEXT"))
        && message
            .text
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty())
}

fn body_of(m: &BeeperMessage) -> (String, bool) {
    if let Some(t) = &m.text
        && !t.is_empty()
    {
        return (plain_rich_text(t), false);
    }
    let kind = m.message_type.as_deref().unwrap_or("");
    let attachment = m
        .attachments
        .first()
        .and_then(|a| a.file_name.clone())
        .map(|f| format!(": {f}"))
        .unwrap_or_default();
    let label = match kind {
        "IMAGE" => format!("[image{attachment}]"),
        "VIDEO" => format!("[video{attachment}]"),
        "VOICE" => "[voice message]".to_string(),
        "AUDIO" => format!("[audio{attachment}]"),
        "FILE" => format!("[file{attachment}]"),
        "STICKER" => "[sticker]".to_string(),
        "LOCATION" => "[location]".to_string(),
        "REACTION" => "[reaction]".to_string(),
        // The bridge may type a poll without any text; the API carries no
        // options or votes, so the row stays an honest non-interactive note.
        "POLL" => "[poll]".to_string(),
        other => format!("[{}]", if other.is_empty() { "message" } else { other }),
    };
    (label, true)
}

fn attachment_content(m: &BeeperMessage) -> Option<Content> {
    let a = m.attachments.first()?;
    // Older payloads lacking a usable source stay honest text placeholders.
    if a.id.is_none() && a.src_url.is_none() {
        return None;
    }
    let media = crate::model::Media {
        mime: a.mime_type.clone().unwrap_or_default(),
        size: a.file_size,
        width: a.size.as_ref().map(|s| s.width),
        height: a.size.as_ref().map(|s| s.height),
        album: None,
        path: None,
        state: Default::default(),
    };
    let caption = m
        .text
        .as_ref()
        .map(|text| plain_rich_text(text))
        .filter(|t| !t.is_empty());
    let seconds = a.duration.map(|d| d.max(0.0).min(u32::MAX as f64) as u32);
    Some(match a.kind.as_deref().unwrap_or("") {
        "img" if a.is_sticker => Content::Sticker {
            media,
            animated: false,
        },
        "img" => Content::Image {
            caption,
            media,
            motion: None,
        },
        "video" => Content::Video {
            caption,
            media,
            seconds,
            gif: a.is_gif,
            note: false,
        },
        "audio" => Content::Audio {
            media,
            seconds,
            voice_note: a.is_voice_note,
            waveform: Vec::new(),
        },
        _ => Content::Document {
            media,
            file_name: a.file_name.clone().unwrap_or_else(|| "Attachment".into()),
            caption,
            pages: None,
        },
    })
}

/// What a `seen` receipt establishes. `true` or a read timestamp is the
/// bridge's per-message read claim. A per-participant map only lists some
/// participants and carries no member count, so another reader there is
/// "read by some", never all. The sender's own entry never counts. `false`,
/// an empty string and an empty map say nothing.
fn read_state(value: &serde_json::Value, own: &str) -> Option<Delivery> {
    match value {
        serde_json::Value::Bool(true) => Some(Delivery::Read),
        serde_json::Value::String(time) if !time.is_empty() => Some(Delivery::Read),
        serde_json::Value::Object(by) => by
            .iter()
            .any(|(who, v)| who != own && read_state(v, own).is_some())
            .then_some(Delivery::ReadBySome),
        _ => None,
    }
}

/// The read time a `seen` receipt carries, when it names one. A timestamp
/// string is the bridge's per-message read claim. A per-participant map
/// counts only when a single other reader's entry holds a timestamp: with
/// several readers the one `read_at` field cannot say whose time it would
/// show, so a broader map stays time-unknown rather than misattributed. The
/// sender's own entry never counts, as in `read_state`.
fn read_time(value: &serde_json::Value, own: &str) -> Option<i64> {
    match value {
        serde_json::Value::String(time) if !time.is_empty() => jiff::Timestamp::from_str(time)
            .ok()
            .map(|stamp| stamp.as_second()),
        serde_json::Value::Object(by) => {
            let mut times = by
                .iter()
                .filter(|(who, _)| who.as_str() != own)
                .filter_map(|(_, entry)| read_time(entry, own));
            let first = times.next()?;
            if times.next().is_none() {
                Some(first)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn delivery_of(m: &BeeperMessage) -> Delivery {
    if !m.is_sender.unwrap_or(false) {
        return Delivery::None;
    }
    if let Some(read) = m.seen.as_ref().and_then(|v| read_state(v, &m.sender_id)) {
        return read;
    }
    match m.send_status.as_ref() {
        Some(s) if s.status.starts_with("FAIL_") => Delivery::Failed,
        Some(s) if s.status == "PENDING" => Delivery::Pending,
        Some(s) if !s.delivered_to_users.is_empty() => Delivery::Delivered,
        Some(s) if s.status == "SUCCESS" => Delivery::Sent,
        // Absent or unrecognized send status is not evidence of "sent":
        // show a neutral unknown state instead of a false definitive tick.
        _ => Delivery::Unknown,
    }
}

/// Synthetic wire fixture exercises the real mapper for offline review captures.
#[cfg(any(test, feature = "demo"))]
pub(crate) fn supplied_preview_demo(chat: &str) -> Message {
    let wire: BeeperMessage = serde_json::from_value(serde_json::json!({
        "id":"ada-link", "chatID": chat,
        "senderID": chat,
        "text":"Summer plans (https://example.com/trips?utm_source=chat#july)",
        "links":[{"title":"Summer plans", "summary":"A few ideas for the long summer evenings.", "url":"https://example.com/trips", "originalURL":"https://example.com/trips?utm_source=chat#july", "img":"https://example.com/not-fetched.jpg"}]
    })).expect("synthetic preview wire fixture");
    map_message(&wire, chat)
}

#[cfg(test)]
thread_local! {
    static SUPPLIED_PREVIEW_BODY_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Associate supplied metadata only with a link actually present in the body.
/// Keep that original destination for opening/copying: a resolved or canonical
/// metadata URL is not permission to change where the message sends the user.
fn supplied_link_preview(message: &BeeperMessage, body: &str) -> Option<crate::model::LinkPreview> {
    if message.links.is_empty() {
        return None;
    }
    fn label(value: Option<&str>, max: usize) -> Option<String> {
        let text: String = value?
            .chars()
            .filter(|ch| !ch.is_control() || ch.is_whitespace())
            .take(max)
            .collect();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    }
    #[cfg(test)]
    SUPPLIED_PREVIEW_BODY_PARSES.with(|count| count.set(count.get() + 1));
    for original in crate::markup::links(body) {
        let Some(valid) = crate::safety::preview_url(&original) else {
            continue;
        };
        let Some(preview) = message.links.iter().take(16).find(|preview| {
            let source = preview.original_url.as_deref().unwrap_or(&preview.url);
            crate::safety::preview_url(source).as_ref() == Some(&valid)
        }) else {
            continue;
        };
        let title = label(preview.title.as_deref(), 240);
        let description = label(preview.summary.as_deref(), 600);
        if title.is_some() || description.is_some() {
            return Some(crate::model::LinkPreview {
                url: original,
                title,
                description,
            });
        }
    }
    None
}

fn map_message(m: &BeeperMessage, fallback_chat: &str) -> Message {
    let (body, _labelled) = body_of(m);
    let status = delivery_of(m);
    // The read timestamp is the one receipt time the API carries, inside
    // `seen`; a delivery time has no field and stays unrecorded rather than
    // guessed. Message info shows "Time not recorded" for that.
    let read_at = if matches!(status, Delivery::Read | Delivery::ReadBySome) {
        m.seen
            .as_ref()
            .and_then(|seen| read_time(seen, &m.sender_id))
    } else {
        None
    };
    Message {
        id: m.id.clone(),
        chat: if m.chat_id.is_empty() {
            fallback_chat.to_string()
        } else {
            m.chat_id.clone()
        },
        sender: m.sender_id.clone(),
        sender_name: readable_name(&m.sender_name)
            .or_else(|| opaque_identity(&m.sender_id).then(|| "Contact (name unavailable)".into())),
        from_me: m.is_sender.unwrap_or(false),
        timestamp: unix(&Some(m.timestamp.clone())),
        history_order: None,
        content: attachment_content(m).unwrap_or_else(|| Content::Text {
            preview: supplied_link_preview(m, &body),
            text: body,
        }),
        status,
        delivered_at: None,
        read_at,
        quoted: m.linked_message_id.as_ref().map(|id| crate::model::Quoted {
            chat: None,
            id: id.clone(),
            sender: String::new(),
            sender_name: None,
            summary: "Reply to earlier message".into(),
            labelled: Some(false),
            mentions: Vec::new(),
        }),
        reactions: m
            .reactions
            .iter()
            .filter(|r| r.emoji)
            .map(|r| crate::model::Reaction {
                sender: r.participant_id.clone(),
                from_me: false,
                emoji: r.reaction_key.clone(),
            })
            .collect(),
        edited: m.edited_timestamp.is_some(),
        mentions: m
            .text
            .as_deref()
            .map(rich_text_mentions)
            .unwrap_or_default(),
        forwarded: false,
        thumbnail: None,
    }
}

#[cfg(test)]
fn map_chat(c: &BeeperChat, network_label: &dyn Fn(&BeeperChat) -> String) -> Chat {
    map_chat_for(c, network_label, None)
}

/// `own` is the account's own user id when known: that participant's handles
/// are never the other person's, even if the server leaves `isSelf` out.
fn map_chat_for(
    c: &BeeperChat,
    network_label: &dyn Fn(&BeeperChat) -> String,
    own: Option<&str>,
) -> Chat {
    let title = if c.title.trim().is_empty() || opaque_identity(&c.title) {
        c.participants
            .as_ref()
            .and_then(|p| {
                p.items.iter().find(|u| u.id == c.title).and_then(|u| {
                    readable_name(&u.full_name).or_else(|| readable_name(&u.username))
                })
            })
            .unwrap_or_else(|| {
                format!(
                    "{} conversation (name unavailable)",
                    if c.network.is_empty() {
                        "Beeper"
                    } else {
                        &c.network
                    }
                )
            })
    } else {
        c.title.clone()
    };
    let mut chat = Chat::new(c.id.clone(), title);
    chat.kind = if c.chat_type == "group" {
        ChatKind::Group
    } else {
        ChatKind::Direct
    };
    chat.group_subject_known = true;
    chat.last_activity = unix(&c.last_activity);
    chat.unread = c.unread_count.clamp(0, u32::MAX as i64) as u32;
    chat.archived = c.is_archived;
    chat.pinned = c.is_pinned;
    chat.muted_until = if c.is_muted { Some(0) } else { None };
    chat.read_only = c.is_read_only;
    chat.poll_capability = c.capabilities.as_ref().and_then(|caps| caps.poll);
    if let Some(participants) = &c.participants {
        chat.participants = participants.items.iter().map(|u| u.id.clone()).collect();
        // Members who never sent a loaded message have no other name
        // source; without these the mention picker lists them as "Unknown".
        chat.participant_names = participants
            .items
            .iter()
            .filter_map(|u| {
                readable_name(&u.full_name)
                    .or_else(|| readable_name(&u.username))
                    .map(|name| (u.id.clone(), name))
            })
            .collect();
    }
    if chat.kind == ChatKind::Direct
        && let Some(participants) = &c.participants
    {
        // Only the other person's handles: ours would be in every chat. A
        // participant id counts only when it is a phone number; opaque ids
        // such as "contact42" never identify anyone across networks.
        let mut handles: Vec<String> = participants
            .items
            .iter()
            .filter(|u| u.is_self != Some(true) && own != Some(u.id.as_str()))
            .flat_map(|u| {
                [
                    u.phone_number
                        .as_deref()
                        .and_then(crate::suggest::normalized_phone),
                    crate::suggest::normalized_phone(&u.id),
                    u.username
                        .as_deref()
                        .and_then(crate::suggest::normalized_username),
                ]
            })
            .flatten()
            .collect();
        handles.sort();
        handles.dedup();
        chat.handles = handles;
    }
    chat.network = Some(network_label(c));
    chat.backend_account = Some(c.account_id.clone());
    chat.backend_network = Some(canonical_network(&c.network));
    chat.last = c
        .preview
        .as_ref()
        .filter(|p| {
            p.is_sender.is_some()
                && !p.is_hidden
                && !p.is_deleted
                && !is_empty_preview(p)
                && !matches!(
                    p.message_type.as_deref(),
                    Some("SYSTEM" | "NOTICE" | "EVENT")
                )
        })
        .map(|p| {
            let (body, labelled) = body_of(p);
            Chat::last_preview(p, body, labelled)
        });
    chat
}

/// The tick beside a chat's last message. A bridge that reports more than
/// "sent" (pending, delivered, read, failed) is shown as it said; one that
/// reports nothing keeps the plain sent tick the list has always drawn.
fn preview_status(p: &BeeperMessage) -> Delivery {
    match delivery_of(p) {
        Delivery::Unknown => Delivery::Sent,
        status => status,
    }
}

impl Chat {
    fn last_preview(p: &BeeperMessage, body: String, labelled: bool) -> LastMessage {
        // Emoji reactions on the newest message lead the row's text, so a
        // reaction is visible without opening the chat.
        let reactions = reaction_run(&p.reactions);
        let (summary, full) = if reactions.is_empty() {
            (body.lines().next().unwrap_or("").to_string(), body)
        } else {
            (
                format!("{reactions} {}", body.lines().next().unwrap_or("")),
                format!("{reactions} {body}"),
            )
        };
        LastMessage {
            from_me: p.is_sender.unwrap_or(false),
            sender: p.sender_id.clone(),
            sender_name: readable_name(&p.sender_name),
            summary,
            full,
            labelled,
            status: preview_status(p),
        }
    }
}

/// Per-chat network label. When several connected accounts share one network
/// (two WhatsApp numbers, say), the account's own name is appended so the
/// rows stay distinguishable: "WhatsApp · Alice".
fn network_labeller(
    chats: &[BeeperChat],
    accounts: &[BeeperAccount],
) -> impl Fn(&BeeperChat) -> String {
    let mut seen: HashMap<String, HashSet<&str>> = HashMap::new();
    for c in chats {
        seen.entry(canonical_network(&c.network))
            .or_default()
            .insert(c.account_id.as_str());
    }
    for account in accounts {
        let network = account.navigation_network();
        if !network.is_empty() {
            seen.entry(network).or_default().insert(&account.account_id);
        }
    }
    let duplicated: HashSet<String> = seen
        .iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(n, _)| n.to_string())
        .collect();
    let user_by_account: HashMap<String, String> = accounts
        .iter()
        .map(|a| {
            let label = a
                .user
                .as_ref()
                .and_then(|u| u.full_name.clone().or(u.username.clone()))
                .unwrap_or_else(|| a.account_id.clone());
            (a.account_id.clone(), label)
        })
        .collect();
    move |c: &BeeperChat| {
        let network = if c.network.is_empty() {
            "chat".to_owned()
        } else {
            canonical_network(&c.network)
        };
        if duplicated.contains(&network) {
            let who = user_by_account
                .get(&c.account_id)
                .cloned()
                .unwrap_or_else(|| c.account_id.clone());
            let duplicate_name = user_by_account
                .values()
                .filter(|name| *name == &who)
                .count()
                > 1;
            if duplicate_name {
                format!("{network} · {who} [{}]", c.account_id)
            } else {
                format!("{network} · {who}")
            }
        } else {
            network.to_string()
        }
    }
}

// ---------- Worker ----------

struct ChatCursors {
    oldest: Option<String>,
    newest: Option<String>,
    has_more_older: bool,
}

/// Search/quote jumps must not silently fall through the Beeper command match.
/// Walk the actual older-history cursors, retaining good pages on failure.
fn load_until_message(
    api: &Api,
    cursors: &mut HashMap<ChatId, ChatCursors>,
    chat: &str,
    target: &str,
    emit: &dyn Fn(Event),
) {
    let mut seen = HashSet::new();
    for _ in 0..50 {
        let state = cursors.get(chat);
        if state.is_some_and(|state| !state.has_more_older) {
            break;
        }
        let cursor = state.and_then(|state| state.oldest.clone());
        let Some(cursor) = cursor else {
            emit(Event::BeeperHistoryState { chat: chat.into(), error: Some("Cannot reach the search result: older-history cursor unavailable. Retry after history recovers.".into()) });
            break;
        };
        if !seen.insert(cursor.clone()) {
            break;
        }
        match api.list_messages(chat, Some(&cursor), Some("before")) {
            Ok(page) => {
                let found = page
                    .items
                    .iter()
                    .any(|m| m.id == target && !m.is_hidden && !m.is_deleted);
                let next =
                    history_cursor(page.oldest_cursor.as_deref(), Some(&cursor), page.has_more);
                let error = next.as_ref().err().cloned();
                if let Some(state) = cursors.get_mut(chat) {
                    if let Ok(next) = next {
                        state.oldest = next;
                    }
                    state.has_more_older = page.has_more;
                }
                let mut messages: Vec<_> = page
                    .items
                    .iter()
                    .filter(|m| !m.is_hidden && !m.is_deleted)
                    .map(|m| api.map_message(m, chat))
                    .collect();
                messages.sort_by_key(|m| m.timestamp);
                emit(Event::Messages {
                    chat: chat.into(),
                    messages,
                    older: true,
                    complete: !page.has_more,
                });
                emit(Event::BeeperHistoryState {
                    chat: chat.into(),
                    error: error.clone(),
                });
                if found {
                    emit(Event::OlderFetched {
                        chat: chat.into(),
                        more: page.has_more,
                        silent: false,
                    });
                    return;
                }
                if error.is_some() || !page.has_more {
                    break;
                }
            }
            Err(error) => {
                emit(Event::BeeperHistoryState {
                    chat: chat.into(),
                    error: Some(format!("Search result history unavailable: {error}")),
                });
                break;
            }
        }
    }
    emit(Event::BeeperHistoryState { chat: chat.into(), error: Some("Search result not reached. History may be incomplete or the 50-page safety limit was reached. Click the result again to continue.".into()) });
    emit(Event::OlderFetched {
        chat: chat.into(),
        more: cursors.get(chat).is_none_or(|state| state.has_more_older),
        silent: false,
    });
}

fn history_cursor(
    next: Option<&str>,
    previous: Option<&str>,
    more: bool,
) -> Result<Option<String>, String> {
    let next = next.filter(|cursor| !cursor.is_empty());
    if more && (next.is_none() || next == previous) {
        return Err("Older history cursor did not advance; some history may be missing. Retry history after the service recovers.".into());
    }
    Ok(next.or(previous).map(str::to_owned))
}

/// Received cache entries must not be relabeled as saved favorites.
fn local_sticker_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(512)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

pub fn run(
    mut cfg: BeeperConfig,
    dirs: AccountDirs,
    events: std::sync::mpsc::Sender<Event>,
    mut commands: UnboundedReceiver<Command>,
    waker: Waker,
) {
    let emit = |ev: Event| {
        let _ = events.send(ev);
        waker.wake();
    };

    emit(Event::Link(LinkStatus::Connecting));

    if cfg.token.is_empty() {
        match crate::beeper_signin::stored(&cfg.base_url) {
            Ok(token) => cfg.token = token.to_string(),
            Err(error) => emit(Event::Info(error)),
        }
    }
    let api = match Api::new(&cfg) {
        Ok(api) => api,
        Err(e) => {
            emit(Event::Link(LinkStatus::Failed(e)));
            // Stay alive only to honour Shutdown.
            while let Some(cmd) = commands.blocking_recv() {
                if matches!(cmd, Command::Shutdown) {
                    return;
                }
            }
            return;
        }
    };

    if let Err(e) = trim_cache(&dirs, cache_budget(&dirs)) {
        emit(Event::Info(format!("Cache cleanup failed: {e}")));
    }

    let gallery_tx = gallery::worker(api.clone(), events.clone(), waker.clone());

    // Bounded single download worker: a slow image must not block typing,
    // read state, history or send confirmations on the control worker.
    let (media_tx, media_rx) =
        std::sync::mpsc::sync_channel::<(bool, Option<usize>, ChatId, String)>(8);
    let media_api = api.clone();
    let media_dirs = dirs.clone();
    let media_events = events.clone();
    let media_waker = waker.clone();
    std::thread::Builder::new()
        .name("beeper-media".into())
        .spawn(move || {
            while let Ok((manual, card, chat, message)) = media_rx.recv() {
                let result = if card.is_some() {
                    Err("Carousel attachments are not supported yet".into())
                } else {
                    media_api.download_media(&chat, &message, &media_dirs, manual)
                };
                if media_events
                    .send(Event::Media {
                        card,
                        chat,
                        message,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                media_waker.wake();
            }
        })
        .expect("Beeper media worker");

    let (avatar_tx, avatar_rx) = std::sync::mpsc::sync_channel::<(String, bool)>(8);
    let avatar_api = api.clone();
    let avatar_dirs = dirs.clone();
    let avatar_events = events.clone();
    let avatar_waker = waker.clone();
    std::thread::Builder::new()
        .name("beeper-avatar".into())
        .spawn(move || {
            while let Ok((id, full)) = avatar_rx.recv() {
                let path = avatar_api.download_avatar(&id, &avatar_dirs).ok();
                if avatar_events
                    .send(Event::Avatar { id, full, path })
                    .is_err()
                {
                    break;
                }
                avatar_waker.wake();
            }
        })
        .expect("Beeper avatar worker");

    let upload_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Drop on every control-worker exit, before queued batches can drain.
    let _stop_uploads = StopUploads(upload_cancel.clone());
    type UploadBatch = (
        ChatId,
        Vec<std::path::PathBuf>,
        bool,
        Option<String>,
        Option<(u32, Option<crate::model::ReplyTarget>, Vec<f32>)>,
        // A picked-file or pasted-image batch's reply quote. Sent on the
        // first file only; a voice note carries its own inside its tuple.
        Option<crate::model::ReplyTarget>,
        // App-created temporary file (voice recording, pasted image):
        // deleted once its message is accepted, never the user's own files.
        bool,
    );
    let (upload_tx, upload_rx) = std::sync::mpsc::sync_channel::<UploadBatch>(2);
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel::<(ChatId, String)>();
    let (pending_media_tx, pending_media_rx) =
        std::sync::mpsc::sync_channel::<crate::pending_media::Job>(8);
    let (pending_media_accepted_tx, pending_media_accepted_rx) = std::sync::mpsc::channel();
    let pending_api = api.clone();
    let pending_events = events.clone();
    let pending_waker = waker.clone();
    let pending_cancel = upload_cancel.clone();
    std::thread::Builder::new()
        .name("beeper-pending-media".into())
        .spawn(move || {
            while let Ok(job) = pending_media_rx.recv() {
                let result = pending_api.send_pending_media(&job, &pending_cancel);
                let state = match result {
                    Ok(accepted) => {
                        let _ = pending_media_accepted_tx.send((
                            job.local.clone(),
                            job.member.clone(),
                            accepted.pending_id.clone(),
                        ));
                        crate::pending_media::State::Accepted {
                            pending: accepted.pending_id,
                        }
                    }
                    Err(state) => state,
                };
                let _ = pending_events.send(Event::BeeperMediaProgress {
                    local: job.local,
                    member: job.member,
                    state,
                });
                pending_waker.wake();
            }
        })
        .expect("pending media worker");
    let (forward_tx, forward_rx) =
        std::sync::mpsc::sync_channel::<(u64, bool, Vec<(usize, crate::composed_forward::Item)>)>(
            1,
        );
    let (forward_accepted_tx, forward_accepted_rx) =
        std::sync::mpsc::channel::<(String, String, String)>();
    let forward_api = api.clone();
    let forward_dirs = dirs.clone();
    let forward_events = events.clone();
    let forward_waker = waker.clone();
    let forward_cancel = upload_cancel.clone();
    std::thread::Builder::new()
        .name("beeper-forward".into())
        .spawn(move || {
            use crate::composed_forward::State;
            while let Ok((generation, prepared, mut items)) = forward_rx.recv() {
                if !prepared {
                    let expanded = forward_api
                        .plan_forwards(items.into_iter().map(|(_, item)| item).collect());
                    let _ = forward_events.send(Event::BeeperForwardPrepared {
                        generation,
                        items: expanded.clone(),
                    });
                    forward_waker.wake();
                    items = expanded.into_iter().enumerate().collect();
                }
                for (index, item) in items {
                    if !matches!(item.state, State::Waiting) {
                        continue;
                    }
                    if upload_stopped(Some(&forward_cancel)) {
                        return;
                    }
                    let send_state = |state| {
                        let _ = forward_events.send(Event::BeeperForwardProgress {
                            generation,
                            index,
                            state,
                        });
                        forward_waker.wake();
                    };
                    send_state(State::Sending);
                    match forward_api.send_forward(&item, &forward_dirs, &forward_cancel) {
                        Ok(accepted) => {
                            let _ = forward_events.send(Event::BeeperForwardMarked {
                                origin: forward_api.base_url.clone(),
                                account: item.destination_account.clone(),
                                chat: item.destination.clone(),
                                message: accepted.pending_id.clone(),
                            });
                            let _ = forward_accepted_tx.send((
                                item.destination.clone(),
                                accepted.pending_id.clone(),
                                item.destination_account.clone(),
                            ));
                            send_state(State::Accepted {
                                pending_id: accepted.pending_id,
                            });
                        }
                        Err(state) => send_state(state),
                    }
                }
            }
        })
        .expect("Beeper forward worker");
    let typed_accepted_tx = accepted_tx.clone();
    let upload_api = api.clone();
    let upload_events = events.clone();
    let upload_waker = waker.clone();
    std::thread::Builder::new()
        .name("beeper-upload".into())
        .spawn(move || {
            while let Ok((chat, paths, documents, caption, voice, quoting, temporary)) =
                upload_rx.recv()
            {
                for (index, path) in paths.iter().enumerate() {
                    if upload_stopped(Some(&upload_cancel)) {
                        return;
                    }
                    let kind = match (&voice, documents) {
                        (Some((seconds, ..)), _) => Attachment::Voice(*seconds),
                        (None, true) => Attachment::Document,
                        (None, false) => Attachment::Auto,
                    };
                    let reply = if index == 0 {
                        voice
                            .as_ref()
                            .and_then(|(_, quoting, _)| quoting.as_ref())
                            .or(quoting.as_ref())
                            .map(|quoting| quoting.id.as_str())
                    } else {
                        None
                    };
                    match upload_api.send_attachment(
                        &chat,
                        path,
                        kind,
                        if index == 0 { caption.as_deref() } else { None },
                        reply,
                        Some(&upload_cancel),
                    ) {
                        Ok(accepted) => {
                            // A sent recording is no longer needed; a failed upload
                            // returns its raw samples to the composer instead.
                            discard_sent_recording(
                                voice.is_some() || temporary || take_temp_upload(path),
                                path,
                            );
                            let target = if accepted.chat.is_empty() {
                                chat.clone()
                            } else {
                                accepted.chat
                            };
                            if accepted_tx.send((target, accepted.pending_id)).is_err() {
                                return;
                            }
                            upload_waker.wake();
                        }
                        Err(error) => {
                            if let Some((_, quoting, samples)) = voice {
                                // The raw recording and its quote go back to the
                                // composer; the encoded copy is no longer needed.
                                // No automatic resend: the outcome may be unknown.
                                discard_sent_recording(true, path);
                                let _ = upload_events.send(Event::BeeperVoiceUncertain {
                                    chat: chat.clone(),
                                    quoting,
                                    samples,
                                    message: format!(
                                        "{error}. Recording kept; check history before sending it again."
                                    ),
                                });
                                upload_waker.wake();
                                break;
                            }
                            let _ = upload_events.send(Event::BeeperFilesUncertain {
                                chat: chat.clone(),
                                paths: paths[index..].to_vec(),
                                documents,
                                caption: first_unsent(&caption, index),
                                quoting: first_unsent(&quoting, index),
                                message: format!(
                                    "{error}. Remaining files kept; check history before retrying."
                                ),
                            });
                            upload_waker.wake();
                            break;
                        }
                    }
                }
            }
        })
        .expect("Beeper upload worker");

    // Typed media uses a bounded upload worker too. No public GIF request uses
    // the authenticated Beeper client, and accepted media retains its type.
    let (typed_tx, typed_rx) = std::sync::mpsc::sync_channel::<(
        ChatId,
        std::path::PathBuf,
        super::MediaKind,
        Option<String>,
        Option<crate::model::ReplyTarget>,
    )>(2);
    let typed_api = api.clone();
    let typed_events = events.clone();
    let typed_waker = waker.clone();
    let typed_cancel = _stop_uploads.0.clone();
    std::thread::Builder::new()
        .name("beeper-typed-upload".into())
        .spawn(move || {
            while let Ok((chat, path, kind, caption, quoting)) = typed_rx.recv() {
                if upload_stopped(Some(&typed_cancel)) {
                    return;
                }
                let attachment = match kind {
                    super::MediaKind::Gif => Attachment::Gif,
                    super::MediaKind::Sticker => Attachment::Sticker,
                };
                let result = if quoting
                    .as_ref()
                    .is_some_and(|q| q.source_chat(&chat) != chat)
                {
                    Err("The reply belongs to another source; media kept".into())
                } else {
                    typed_api.send_attachment(
                        &chat,
                        &path,
                        attachment,
                        caption.as_deref(),
                        quoting.as_ref().map(|q| q.id.as_str()),
                        Some(&typed_cancel),
                    )
                };
                match result {
                    Ok(accepted) => {
                        discard_sent_recording(take_temp_upload(&path), &path);
                        let target = if accepted.chat.is_empty() {
                            chat
                        } else {
                            accepted.chat
                        };
                        if typed_accepted_tx
                            .send((target, accepted.pending_id))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if let Err(error) = typed_events.send(Event::TypedMediaUncertain {
                            chat,
                            path,
                            kind,
                            caption,
                            quoting,
                            message: format!("{error}. Media kept; check history before retrying."),
                        }) {
                            error.0.discard_owned_media();
                        }
                    }
                }
                typed_waker.wake();
            }
        })
        .expect("Beeper typed upload worker");

    let mut accounts = loop {
        match api.accounts() {
            Ok(a) => break a,
            Err(e) => {
                emit(Event::Link(LinkStatus::Disconnected {
                    reason: format!("Beeper accounts unavailable: {e}"),
                }));
                for _ in 0..25 {
                    match commands.try_recv() {
                    Ok(Command::Shutdown) => return,
                    Ok(Command::SendText { chat, text, .. }) => emit(Event::BeeperSendUncertain {
                        chat, text, message: "Beeper is not connected yet. Text restored; nothing was sent or queued for automatic retry.".into(),
                    }),
                    Ok(Command::EditText { chat, id, text, .. }) => emit(Event::BeeperEditUncertain {
                        chat, id, text, message: "Beeper is not connected yet. Edit retained; nothing was changed.".into(),
                    }),
                    Ok(Command::SendFiles { chat, paths, documents, caption, quoting, .. }) => emit(Event::BeeperFilesUncertain {
                        chat, paths, documents, caption, quoting, message: "Beeper is not connected yet. Files retained; nothing was sent or queued for automatic retry.".into(),
                    }),
                    Ok(Command::SendTypedMedia { chat, path, kind, caption, quoting }) => emit(Event::TypedMediaUncertain { chat, path, kind, caption, quoting, message: "Beeper is not connected. Media kept; nothing sent.".into() }),
                    Ok(Command::SendVoice { chat, samples, quoting }) => emit(Event::SendRefused {
                        chat, quoting, unsent: crate::backend::Unsent::Voice(samples), reason: crate::backend::Refusal::Offline,
                    }),
                    Ok(Command::SendImage { chat, width, height, rgba, view_once, caption, quoting, .. }) => emit(Event::SendRefused {
                        chat, quoting, unsent: crate::backend::Unsent::Image { width, height, rgba, view_once, caption }, reason: crate::backend::Refusal::Offline,
                    }),
                    Ok(Command::LoadRestoredChat { chat, token }) => emit(Event::BeeperRestoredHistory { chat, token, success: false }),
                    Ok(command) => match refuse_unsupported_payload(command) {
                        Err(event)=>emit(*event),
                        Ok(_)=>emit(Event::Info("Beeper is not connected yet. Requested action was not performed; retry after connection recovers.".into())),
                    },
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {},
                }
                    std::thread::sleep(LOOP_TICK);
                }
            }
        }
    };
    emit(Event::BeeperAccounts {
        origin: api.base_url.clone(),
        accounts: account_inventory(&accounts),
    });
    let me = accounts.first().and_then(|a| a.user.clone());
    emit(Event::Link(LinkStatus::Connected));
    if api.read_only {
        emit(Event::Info(
            "Beeper read-only mode: sends, typing and read-state changes are blocked".into(),
        ));
    }
    emit(Event::Me {
        id: me.as_ref().map(|u| u.id.clone()).unwrap_or_default(),
        lid: None,
        name: me.as_ref().and_then(|u| u.full_name.clone()),
        about: None,
    });
    // Nothing is synced, archived locally, or pinned here yet.
    emit(Event::Syncing(false));
    // Do not impose WhatsApp's three-pin cap on unrelated Beeper accounts.
    // Beeper itself remains authoritative about its supported limits.
    emit(Event::PinLimit(usize::MAX));
    emit(Event::Labels(Vec::new()));
    // Separate encrypted draft-only archive. Never store duplicate message history.
    // OS keyring failure is explicit; never fall back to plaintext.
    let draft_path = dirs.state.join("beeper-drafts").join("drafts.db");
    let mut draft_store = if draft_path.exists() {
        match crate::archive::Archive::open(&draft_path) {
            Ok(store) => Some(store),
            Err(_) => {
                emit(Event::Error("Encrypted Beeper drafts could not be unlocked. Unlock the OS keyring and restart; existing drafts were not changed.".into()));
                None
            }
        }
    } else {
        None
    };
    if let Some(store) = &draft_store {
        match stored_drafts(store, &api.base_url) {
            Ok(drafts) => emit(Event::Drafts(drafts)),
            Err(e) => emit(Event::Info(e)),
        }
    } else {
        emit(Event::Drafts(Vec::new()));
    }

    // The favorites bar's order persists beside the drafts: Beeper has no
    // favorites of its own, so the archive is the only copy.
    let favorites = draft_store
        .as_ref()
        .and_then(|store| store.favorites().ok())
        .map(|favorites| {
            favorites
                .into_iter()
                .map(|favorite| favorite.chat)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    emit(Event::Favorites(favorites));

    emit(Event::Contacts(Vec::new()));

    let mut chat_cursors: HashMap<ChatId, ChatCursors> = HashMap::new();
    let mut search_cache = std::collections::HashMap::<String, CachedSearch>::new();
    let mut chat_search_cache = (String::new(), CachedSearch::default());
    let mut open_chat: Option<ChatId> = None;
    let mut watched_chats: Vec<ChatId> = Vec::new();
    // Chats the Server answered 404 for: permanent per id, so polling and
    // history stop instead of retrying forever.
    let mut dead_chats: HashSet<ChatId> = HashSet::new();
    let mut last_chat_refresh = Instant::now() - CHAT_REFRESH;
    let mut last_msg_poll = Instant::now();
    let mut last_receipt_refresh = Instant::now();
    let mut pending_sends: Vec<(ChatId, String, Instant)> = Vec::new();
    let mut tracked_media: HashMap<(String, String), (String, crate::merged::Member)> =
        HashMap::new();
    let mut forward_pending = HashMap::new();
    let mut pending_storage: Option<(u64, bool)> = None;
    // Set by Command::Resumed: after a wake every poll timer is due at once,
    // so a link the server dropped during sleep is noticed on the next tick.
    let mut refresh_after_wake = false;
    // Downloads the worker's queue had no room for wait here, not fail.
    let mut media_backlog = MediaBacklog::default();
    // A viewer-album request waits for a busy gallery worker instead of failing.
    let mut gallery_backlog = GalleryBacklog::default();

    loop {
        if let Some((chat, around, _)) = gallery_backlog.flush(&gallery_tx) {
            emit(Event::BeeperViewerMedia {
                chat,
                around,
                result: Err("Media history is busy; try the arrow again".into()),
            });
        }
        for (card, chat, message) in media_backlog.flush(&media_tx) {
            emit(Event::Media {
                card,
                chat,
                message,
                result: Err("Media queue is busy; retry after current downloads finish".into()),
            });
        }
        loop {
            let cmd = match commands.try_recv() {
                Ok(command) => command,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
            };
            let cmd = match refuse_unsupported_payload(cmd) {
                Ok(cmd) => cmd,
                Err(event) => {
                    emit(*event);
                    continue;
                }
            };
            let (cmd, restoration) = match cmd {
                Command::LoadRestoredChat { chat, token } => {
                    (Command::LoadChat { chat, before: None }, Some(token))
                }
                cmd => (cmd, None),
            };
            match cmd {
                Command::BeeperMedia(job) => {
                    if let Err(error) = pending_media_tx.try_send(job) {
                        let job = match error {
                            std::sync::mpsc::TrySendError::Full(job)
                            | std::sync::mpsc::TrySendError::Disconnected(job) => job,
                        };
                        emit(Event::BeeperMediaProgress {
                            local: job.local,
                            member: job.member,
                            state: crate::pending_media::State::NotSubmitted(
                                "Upload queue unavailable; nothing submitted".into(),
                            ),
                        });
                    }
                }
                Command::Shutdown => return,
                Command::SaveDraft { chat, text } => {
                    if draft_store.is_none() && !text.trim().is_empty() {
                        match crate::archive::Archive::open(&draft_path) {
                            Ok(store) => draft_store = Some(store),
                            Err(_) => {
                                emit(Event::Error("Draft not saved: OS keyring unavailable. Keep the app open or copy the draft before closing; no plaintext fallback was used.".into()));
                                continue;
                            }
                        }
                    }
                    if let Some(store) = &draft_store
                        && store
                            .set_draft(
                                &draft_namespace(&api.base_url, &chat),
                                &text,
                                jiff::Timestamp::now().as_second(),
                            )
                            .is_err()
                    {
                        emit(Event::Error("Draft not saved: encrypted local write failed. Keep the draft before closing.".into()));
                    }
                }
                Command::SetFavorite(chat, favorite) => {
                    if draft_store.is_none() {
                        match crate::archive::Archive::open(&draft_path) {
                            Ok(store) => draft_store = Some(store),
                            Err(_) => {
                                emit(Event::Error("Favorite not saved: OS keyring unavailable. The mark lasts until restart; unlock the OS keyring to keep it.".into()));
                                continue;
                            }
                        }
                    }
                    if let Some(store) = &draft_store {
                        match store
                            .set_favorite(&chat, favorite)
                            .map(|_| store.favorites())
                        {
                            Ok(Ok(favorites)) => emit(Event::Favorites(
                                favorites
                                    .into_iter()
                                    .map(|favorite| favorite.chat)
                                    .collect(),
                            )),
                            Ok(Err(e)) => emit(Event::Info(format!(
                                "Favorites could not be read back: {e}"
                            ))),
                            Err(e) => emit(Event::Info(format!("Favorite not saved: {e}"))),
                        }
                    }
                }
                Command::MoveFavorite { chat, position } => {
                    // No archive yet means no stored favorites, so a move
                    // is a no-op; the interface already shows its own.
                    if let Some(store) = &draft_store {
                        match store.reorder_favorite(&chat, position) {
                            Ok(Some(favorites)) => emit(Event::Favorites(
                                favorites
                                    .into_iter()
                                    .map(|favorite| favorite.chat)
                                    .collect(),
                            )),
                            Ok(None) => {}
                            Err(e) => emit(Event::Info(format!("Favorite reorder not saved: {e}"))),
                        }
                    }
                }
                Command::LoadStorageStats { token } => pending_storage = Some((token, false)),
                Command::ClearMediaCache { token } => pending_storage = Some((token, true)),
                Command::WatchBeeperChats(chats) => {
                    watched_chats = chats.into_iter().take(8).collect();
                }
                Command::MirrorPins(pins) => {
                    if draft_store.is_none() {
                        match crate::archive::Archive::open(&draft_path) {
                            Ok(store) => draft_store = Some(store),
                            Err(_) => {
                                emit(Event::Info("Pins not mirrored into favorites: OS keyring unavailable. Unlock it and restart to keep favorites across restarts.".into()));
                                continue;
                            }
                        }
                    }
                    if let Some(store) = &draft_store {
                        let mut changed = false;
                        for (chat, pinned) in &pins {
                            match store.mirror_pin(chat, *pinned) {
                                Ok(did) => changed |= did,
                                Err(e) => {
                                    emit(Event::Info(format!("Pin mirror not saved: {e}")));
                                    break;
                                }
                            }
                        }
                        if changed {
                            match store.favorites() {
                                Ok(favorites) => emit(Event::Favorites(
                                    favorites
                                        .into_iter()
                                        .map(|favorite| favorite.chat)
                                        .collect(),
                                )),
                                Err(e) => emit(Event::Info(format!(
                                    "Favorites could not be read back: {e}"
                                ))),
                            }
                        }
                    }
                }
                Command::LoadChat { chat, before } => {
                    open_chat = Some(chat.clone());
                    let older = before.is_some();
                    if older && chat_cursors.get(&chat).is_some_and(|c| !c.has_more_older) {
                        emit(Event::Messages {
                            chat,
                            messages: Vec::new(),
                            older: true,
                            complete: true,
                        });
                        continue;
                    }
                    let cursor = if older {
                        chat_cursors.get(&chat).and_then(|c| c.oldest.clone())
                    } else {
                        None
                    };
                    if older && cursor.is_none() {
                        emit(Event::BeeperHistoryState {chat:chat.clone(),error:Some("Older history cursor unavailable; completeness is unknown. Retry history after the service recovers.".into())});
                        emit(Event::Error(
                            "Beeper older history cursor unavailable; completeness is unknown"
                                .into(),
                        ));
                        emit(Event::Messages {
                            chat,
                            messages: Vec::new(),
                            older: true,
                            complete: false,
                        });
                        continue;
                    }
                    match api.list_messages(
                        &chat,
                        cursor.as_deref(),
                        if older { Some("before") } else { None },
                    ) {
                        Ok(page) => {
                            let next = history_cursor(
                                page.oldest_cursor.as_deref(),
                                cursor.as_deref(),
                                page.has_more,
                            );
                            let error = next.as_ref().err().cloned();
                            if older {
                                if let Some(c) = chat_cursors.get_mut(&chat) {
                                    if let Ok(next) = next {
                                        c.oldest = next;
                                    }
                                    c.has_more_older = page.has_more;
                                }
                            } else {
                                chat_cursors.insert(
                                    chat.clone(),
                                    ChatCursors {
                                        oldest: next.ok().flatten(),
                                        newest: page.newest_cursor.clone(),
                                        has_more_older: page.has_more,
                                    },
                                );
                            }
                            if let Some(error) = &error {
                                emit(Event::Error(error.clone()));
                            }
                            let successful = error.is_none();
                            emit(Event::BeeperHistoryState {
                                chat: chat.clone(),
                                error,
                            });
                            let mut messages: Vec<Message> = page
                                .items
                                .iter()
                                .filter(|m| !m.is_hidden && !m.is_deleted)
                                .map(|m| api.map_message(m, &chat))
                                .collect();
                            messages.sort_by_key(|m| m.timestamp);
                            emit(Event::Messages {
                                chat: chat.clone(),
                                messages,
                                older,
                                complete: !page.has_more,
                            });
                            if let Some(token) = restoration {
                                emit(Event::BeeperRestoredHistory {
                                    chat,
                                    token,
                                    success: successful,
                                });
                            }
                        }
                        Err(e) => {
                            if not_found(&e) {
                                dead_chats.insert(chat.clone());
                            }
                            let copy = history_error_copy(&e);
                            emit(Event::BeeperHistoryState {
                                chat: chat.clone(),
                                error: Some(copy.clone()),
                            });
                            emit(Event::Messages {
                                chat: chat.clone(),
                                messages: Vec::new(),
                                older,
                                complete: false,
                            });
                            if let Some(token) = restoration {
                                emit(Event::BeeperRestoredHistory {
                                    chat,
                                    token,
                                    success: false,
                                });
                            }
                            emit(Event::Error(copy));
                        }
                    }
                }
                Command::LoadUntil {
                    chat,
                    id,
                    before: _,
                } => {
                    load_until_message(&api, &mut chat_cursors, &chat, &id, &emit);
                }
                Command::FetchOlder { chat, explicit: _ } => {
                    if chat_cursors.get(&chat).is_some_and(|c| !c.has_more_older) {
                        emit(Event::OlderFetched {
                            chat,
                            more: false,
                            silent: false,
                        });
                        continue;
                    }
                    let cursor = chat_cursors.get(&chat).and_then(|c| c.oldest.clone());
                    if let Some(cursor) = cursor {
                        match api.list_messages(&chat, Some(&cursor), Some("before")) {
                            Ok(page) => {
                                let next = history_cursor(
                                    page.oldest_cursor.as_deref(),
                                    Some(&cursor),
                                    page.has_more,
                                );
                                let error = next.as_ref().err().cloned();
                                if let Some(c) = chat_cursors.get_mut(&chat) {
                                    if let Ok(next) = next {
                                        c.oldest = next;
                                    }
                                    c.has_more_older = page.has_more;
                                }
                                if let Some(error) = &error {
                                    emit(Event::Error(error.clone()));
                                }
                                emit(Event::BeeperHistoryState {
                                    chat: chat.clone(),
                                    error,
                                });
                                let mut messages: Vec<Message> = page
                                    .items
                                    .iter()
                                    .filter(|m| !m.is_hidden && !m.is_deleted)
                                    .map(|m| api.map_message(m, &chat))
                                    .collect();
                                messages.sort_by_key(|m| m.timestamp);
                                emit(Event::Messages {
                                    chat: chat.clone(),
                                    messages,
                                    older: true,
                                    complete: !page.has_more,
                                });
                                emit(Event::OlderFetched {
                                    chat,
                                    more: page.has_more,
                                    silent: false,
                                });
                            }
                            Err(e) => {
                                emit(Event::BeeperHistoryState {
                                    chat: chat.clone(),
                                    error: Some(e.clone()),
                                });
                                emit(Event::OlderFetched {
                                    chat,
                                    more: true,
                                    silent: false,
                                });
                                emit(Event::Error(format!("Beeper older history failed: {e}")));
                            }
                        }
                    } else {
                        emit(Event::BeeperHistoryState { chat: chat.clone(), error: Some("Older history cursor unavailable; completeness is unknown. Retry history after the service recovers.".into()) });
                        emit(Event::OlderFetched {
                            chat,
                            more: true,
                            silent: false,
                        });
                    }
                }
                Command::BeeperResumeForwards(marks) => {
                    for mark in crate::composed_forward::unresolved_marks(marks, &api.base_url) {
                        let key = (mark.member.chat.clone(), mark.message.clone());
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            forward_pending.entry(key)
                        {
                            entry.insert(mark.member.account);
                            pending_sends.push((mark.member.chat, mark.message, Instant::now()));
                        }
                    }
                }
                Command::BeeperForward {
                    generation,
                    prepared,
                    items,
                } => {
                    if let Err(error) = forward_tx.try_send((generation, prepared, items)) {
                        let (generation, _, items) = match error {
                            std::sync::mpsc::TrySendError::Full(value)
                            | std::sync::mpsc::TrySendError::Disconnected(value) => value,
                        };
                        for (index, _) in items {
                            emit(Event::BeeperForwardProgress {
                                generation,
                                index,
                                state: crate::composed_forward::State::FailedBeforeSend(
                                    "Forward queue is busy; nothing submitted for this item".into(),
                                ),
                            });
                        }
                    }
                }
                Command::SendText {
                    chat,
                    text,
                    quoting,
                    mentions,
                } => {
                    if quoting
                        .as_ref()
                        .is_some_and(|q| q.source_chat(&chat) != chat)
                    {
                        emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: super::Unsent::Text(text),
                            reason: super::Refusal::QuoteUnavailable,
                        });
                        emit(Event::Info("Beeper cross-chat replies are not implemented yet; message kept in composer".into()));
                        continue;
                    }
                    match api.send_text(
                        &chat,
                        &text,
                        quoting.as_ref().map(|q| q.id.as_str()),
                        &mentions,
                    ) {
                        Ok(accepted) => {
                            let target = if accepted.chat.is_empty() {
                                chat.clone()
                            } else {
                                accepted.chat
                            };
                            pending_sends.push((target, accepted.pending_id, Instant::now()));
                        }
                        Err(e) => {
                            if not_found(&e) {
                                dead_chats.insert(chat.clone());
                            }
                            emit(Event::BeeperSendUncertain {
                                chat: chat.clone(),
                                text,
                                message: send_error_copy(&e),
                            })
                        }
                    }
                    // Pull the echo (and anything else new) straight away.
                    if poll_newer(&api, &chat, &mut chat_cursors, &emit) {
                        dead_chats.insert(chat.clone());
                    }
                }
                Command::ResolveQuotes { chat, refs } => {
                    for (message_id, quoted_id) in refs {
                        let Ok(fetched) = api.get_message(&chat, &quoted_id) else {
                            // Deleted or unreachable quoted message: keep the
                            // generic placeholder rather than guessing.
                            continue;
                        };
                        let (body, labelled) = body_of(&fetched);
                        let summary: String = body.chars().take(120).collect();
                        emit(Event::QuoteResolved {
                            chat: chat.clone(),
                            message: message_id,
                            quoted: crate::model::Quoted {
                                chat: None,
                                id: quoted_id,
                                sender: fetched.sender_id.clone(),
                                sender_name: readable_name(&fetched.sender_name),
                                summary,
                                labelled: Some(labelled),
                                mentions: Vec::new(),
                            },
                        });
                    }
                }
                Command::ListScheduled {
                    scheduler_url,
                    scheduler_token,
                } => {
                    match crate::scheduler_client::SchedulerClient::new(
                        &scheduler_url,
                        &scheduler_token,
                    )
                    .and_then(|client| client.list())
                    {
                        Ok(items) => emit(Event::ScheduledList(items)),
                        Err(e) => emit(Event::Info(format!("Send Later list unavailable: {e}"))),
                    }
                }
                Command::ScheduledAction {
                    scheduler_url,
                    scheduler_token,
                    id,
                    action,
                } => {
                    let result = crate::scheduler_client::SchedulerClient::new(
                        &scheduler_url,
                        &scheduler_token,
                    )
                    .and_then(|client| match action.as_str() {
                        "cancel" => client.cancel(&id),
                        "retry" => client.retry(&id),
                        "send-now" => client.send_now(&id),
                        other => Err(format!("unknown scheduled action {other}")),
                    });
                    if let Err(e) = result {
                        emit(Event::Info(format!("Scheduled message: {e}")));
                    }
                    // The list always follows, so the dialog reflects the
                    // action's real outcome rather than a hoped-for one.
                    if let Ok(client) = crate::scheduler_client::SchedulerClient::new(
                        &scheduler_url,
                        &scheduler_token,
                    ) && let Ok(items) = client.list()
                    {
                        emit(Event::ScheduledList(items));
                    }
                }
                Command::ScheduleSend {
                    scheduler_url,
                    scheduler_token,
                    account,
                    chat,
                    text,
                    due_at,
                    quoting,
                    replaces,
                } => {
                    let result = crate::scheduler_client::SchedulerClient::new(
                        &scheduler_url,
                        &scheduler_token,
                    )
                    .and_then(|client| {
                        client.add(
                            &account,
                            &chat,
                            &text,
                            quoting.as_deref(),
                            due_at,
                            replaces.as_deref(),
                        )
                    });
                    match result {
                        Ok(id) => {
                            let when = jiff::Timestamp::from_second(due_at)
                                .map(|t| {
                                    t.to_zoned(jiff::tz::TimeZone::system())
                                        .strftime("%a %d %b, %H:%M")
                                        .to_string()
                                })
                                .unwrap_or_else(|_| due_at.to_string());
                            emit(Event::Info(format!(
                                "Scheduled for {when}; the scheduler service sends it at that time (id {id})"
                            )));
                            emit(Event::ScheduledAccepted);
                        }
                        Err(e) => emit(Event::Info(format!("Send later failed: {e}"))),
                    }
                }
                Command::SetBeeperReminder { chat, at } => {
                    let path = format!("/v1/chats/{}/reminders", encode_segment(&chat));
                    let result = if let Some(at) = at {
                        if jiff::Timestamp::from_str(&at).is_err() {
                            emit(Event::Info(
                                "Invalid reminder timestamp; nothing sent".into(),
                            ));
                            continue;
                        }
                        api.post(&path, serde_json::json!({"reminder":{"remindAt":at,"dismissOnIncomingMessage":false}}))
                    } else {
                        api.mutate(reqwest::Method::DELETE, &path, None)
                    };
                    match result {
                        Ok(()) => emit(Event::Info("Beeper accepted the reminder change; delivery depends on Beeper's reminder service".into())),
                        Err(e) => emit(Event::Info(format!("Reminder change failed: {e}"))),
                    }
                }
                Command::SetArchived(chat, value) => {
                    match api.post(
                        &format!("/v1/chats/{}/archive", encode_segment(&chat)),
                        serde_json::json!({"archived": value}),
                    ) {
                        // The archive POST echoes nothing: the chat's own
                        // record answers. A local Silence promise holds the
                        // archived state either way.
                        Ok(()) => {
                            match api.get_chat(&chat) {
                                Ok(state) => {
                                    if let Some(report) = Api::unconfirmed_state(
                                        "isArchived",
                                        value,
                                        state.is_archived,
                                        value,
                                    ) {
                                        emit(Event::Info(report));
                                    }
                                }
                                Err(e) => emit(Event::Info(e)),
                            }
                            api.refresh_chat(&chat, &accounts, &emit);
                        }
                        Err(e) => {
                            emit(Event::Info(e));
                            api.refresh_chat(&chat, &accounts, &emit);
                        }
                    }
                    last_chat_refresh = Instant::now() - CHAT_REFRESH;
                }
                Command::SetPinned(chat, value) => {
                    if let Err(e) = api.change_chat(&chat, "isPinned", value) {
                        emit(Event::Info(e));
                    }
                    api.refresh_chat(&chat, &accounts, &emit);
                    last_chat_refresh = Instant::now() - CHAT_REFRESH;
                }
                Command::SetMuted(chat, until) => {
                    if until.is_some_and(|v| v != 0) {
                        emit(Event::Info(
                            "Beeper supports indefinite mute only; no timed mute was sent".into(),
                        ));
                        api.refresh_chat(&chat, &accounts, &emit);
                    } else {
                        let expected = until.is_some();
                        // A forever mute is also a local promise ("Hide and
                        // mute indefinitely"): what the bridge drops still
                        // holds here.
                        let held = until == Some(0);
                        match api.change_chat(&chat, "isMuted", expected) {
                            Ok(Some(echo)) => {
                                if echo != expected {
                                    emit(Event::Info(Api::unconfirmed_change(
                                        if expected { "mute" } else { "unmute" },
                                        held,
                                    )));
                                }
                                api.refresh_chat(&chat, &accounts, &emit);
                            }
                            // No echo to check: the chat's own record answers.
                            Ok(None) => {
                                match api.get_chat(&chat) {
                                    Ok(state) => {
                                        if let Some(report) = Api::unconfirmed_state(
                                            "isMuted",
                                            expected,
                                            state.is_muted,
                                            held,
                                        ) {
                                            emit(Event::Info(report));
                                        }
                                    }
                                    Err(e) => emit(Event::Info(e)),
                                }
                                api.refresh_chat(&chat, &accounts, &emit);
                            }
                            Err(e) => {
                                emit(Event::Info(e));
                                api.refresh_chat(&chat, &accounts, &emit);
                            }
                        }
                    }
                    last_chat_refresh = Instant::now() - CHAT_REFRESH;
                }
                Command::EditText {
                    chat,
                    id,
                    text,
                    mentions,
                } => {
                    let result = if !mentions.is_empty() {
                        Err("Beeper mention edits are not supported; no edit sent".into())
                    } else {
                        api.mutate(
                            reqwest::Method::PUT,
                            &format!(
                                "/v1/chats/{}/messages/{}",
                                encode_segment(&chat),
                                encode_segment(&id)
                            ),
                            Some(serde_json::json!({"text": text})),
                        )
                    };
                    match result {
                        Ok(()) => api.refresh_message(&chat, &id, &emit),
                        Err(e) => emit(Event::BeeperEditUncertain {
                            chat,
                            id,
                            text,
                            message: format!(
                                "Edit could not be confirmed: {e}. Check current message before retrying; proposed edit retained."
                            ),
                        }),
                    }
                }
                Command::Revoke { chat, id } => {
                    match api.mutate(
                        reqwest::Method::DELETE,
                        &format!(
                            "/v1/chats/{}/messages/{}?forEveryone=true",
                            encode_segment(&chat),
                            encode_segment(&id)
                        ),
                        None,
                    ) {
                        Ok(()) => emit(Event::MessageDeleted { chat, id }),
                        Err(e) => emit(Event::Info(format!("Delete failed: {e}"))),
                    }
                }
                Command::DeleteLocal { chat, id } => {
                    match api.mutate(
                        reqwest::Method::DELETE,
                        &format!(
                            "/v1/chats/{}/messages/{}?forEveryone=false",
                            encode_segment(&chat),
                            encode_segment(&id)
                        ),
                        None,
                    ) {
                        Ok(()) => emit(Event::MessageDeleted { chat, id }),
                        Err(e) => emit(Event::Info(format!("Delete failed: {e}"))),
                    }
                }
                Command::React {
                    chat,
                    message,
                    emoji,
                } => {
                    if emoji.is_empty() {
                        let wire = api.get_message(&chat, &message);
                        let chat_wire = api.get::<BeeperChat>(
                            &format!("/v1/chats/{}", encode_segment(&chat)),
                            &[],
                        );
                        let own_id = chat_wire.ok().and_then(|c| {
                            accounts
                                .iter()
                                .find(|a| a.account_id == c.account_id)
                                .and_then(|a| a.user.as_ref())
                                .map(|u| u.id.clone())
                        });
                        let key = wire.ok().and_then(|m| {
                            m.reactions
                                .into_iter()
                                .find(|r| own_id.as_deref() == Some(r.participant_id.as_str()))
                                .map(|r| r.reaction_key)
                        });
                        if let Some(key) = key {
                            match api.mutate(
                                reqwest::Method::DELETE,
                                &format!(
                                    "/v1/chats/{}/messages/{}/reactions/{}",
                                    encode_segment(&chat),
                                    encode_segment(&message),
                                    encode_segment(&key)
                                ),
                                None,
                            ) {
                                Ok(()) => api.refresh_message(&chat, &message, &emit),
                                Err(e) => {
                                    emit(Event::Info(format!("Reaction removal failed: {e}")))
                                }
                            }
                        } else {
                            emit(Event::Info(
                                "Could not identify your current reaction; no removal sent".into(),
                            ));
                        }
                        continue;
                    }
                    match api.post(
                        &format!(
                            "/v1/chats/{}/messages/{}/reactions",
                            encode_segment(&chat),
                            encode_segment(&message)
                        ),
                        serde_json::json!({"reactionKey": emoji}),
                    ) {
                        Ok(()) => api.refresh_message(&chat, &message, &emit),
                        Err(e) => emit(Event::Info(format!("Reaction failed: {e}"))),
                    }
                }
                Command::MarkRead { chat, receipts } => {
                    // The API cannot separate marking read from sending a network
                    // read receipt. Respect the account's privacy switch.
                    if !receipts {
                        continue;
                    }
                    if let Err(e) = api.mark_read(&chat)
                        && !api.read_only
                    {
                        emit(Event::Info(format!("Beeper read-state update failed: {e}")));
                    }
                }
                Command::MarkUnread(chat) => {
                    if let Err(e) = api.mark_unread(&chat)
                        && !api.read_only
                    {
                        emit(Event::Info(format!("Beeper unread update failed: {e}")));
                    }
                }
                Command::Composing { chat, composing } => {
                    let _ = api.typing(&chat, composing);
                }
                Command::SearchChatMessages {
                    chat,
                    query,
                    from,
                    until,
                } => {
                    chat_search_cache = (
                        format!("{chat}\0{query}\0{from:?}\0{until:?}"),
                        CachedSearch::default(),
                    );
                    run_chat_search_cached(
                        &api,
                        chat,
                        query,
                        from,
                        until,
                        &emit,
                        &mut chat_search_cache.1,
                    );
                }
                Command::SearchChatMessagesMore {
                    chat,
                    query,
                    from,
                    until,
                } => {
                    let key = format!("{chat}\0{query}\0{from:?}\0{until:?}");
                    if chat_search_cache.0 == key {
                        run_chat_search_cached(
                            &api,
                            chat,
                            query,
                            from,
                            until,
                            &emit,
                            &mut chat_search_cache.1,
                        );
                    }
                }
                Command::SearchMessages { query } => {
                    if search_cache.len() > 8 {
                        search_cache.retain(|_, cache| cache.pages > 1);
                    }
                    let cache = search_cache.entry(query.clone()).or_default();
                    *cache = CachedSearch::default();
                    run_message_search_cached(&api, query, &emit, cache);
                }
                Command::SearchMessagesMore { query } => {
                    let cache = search_cache.entry(query.clone()).or_default();
                    run_message_search_cached(&api, query, &emit, cache);
                }
                Command::Picked {
                    chat,
                    paths,
                    documents,
                } => emit(Event::Picked {
                    chat,
                    paths,
                    documents,
                }),
                Command::PickFiles { chat, documents } => {
                    // The picker is a desktop dialog, shown from a side
                    // thread so a slow answer never stalls the worker.
                    let events = events.clone();
                    let waker = waker.clone();
                    std::thread::spawn(move || {
                        let paths = rfd::FileDialog::new()
                            .set_title("Attach to chat")
                            .pick_files()
                            .unwrap_or_default();
                        let _ = events.send(Event::Picked {
                            chat,
                            paths,
                            documents,
                        });
                        waker.wake();
                    });
                }
                Command::SendFiles {
                    chat,
                    paths,
                    documents,
                    view_once,
                    caption,
                    mentions,
                    quoting,
                } => {
                    if quoting
                        .as_ref()
                        .is_some_and(|q| q.source_chat(&chat) != chat)
                    {
                        emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: crate::backend::Unsent::Files {
                                paths,
                                documents,
                                view_once,
                                caption,
                            },
                            reason: crate::backend::Refusal::QuoteUnavailable,
                        });
                        continue;
                    }
                    if view_once || !mentions.is_empty() {
                        emit(Event::BeeperFilesUncertain { chat, paths, documents, caption, quoting, message: "Beeper mentions and view-once are not supported with attachments; files kept".into() });
                        continue;
                    }
                    if let Err(error) =
                        upload_tx.try_send((chat, paths, documents, caption, None, quoting, false))
                    {
                        let (chat, paths, documents, caption, _, quoting, _) = match error {
                            std::sync::mpsc::TrySendError::Full(batch)
                            | std::sync::mpsc::TrySendError::Disconnected(batch) => batch,
                        };
                        emit(Event::BeeperFilesUncertain {
                            chat,
                            paths,
                            documents,
                            caption,
                            quoting,
                            message: "Upload queue is busy; files kept without sending".into(),
                        });
                    }
                }
                Command::SendImage {
                    chat,
                    width,
                    height,
                    rgba,
                    view_once,
                    caption,
                    quoting,
                    ..
                } => {
                    if quoting
                        .as_ref()
                        .is_some_and(|q| q.source_chat(&chat) != chat)
                    {
                        emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: crate::backend::Unsent::Image {
                                width,
                                height,
                                rgba,
                                view_once,
                                caption,
                            },
                            reason: crate::backend::Refusal::QuoteUnavailable,
                        });
                        continue;
                    }
                    // A pasted image encodes to a temporary JPEG and takes the
                    // same upload pipeline as a picked file. The unsupported
                    // gate has already returned mentions and view-once; a
                    // quote rides along to the first (and only) file.
                    let encoded = encode_pasted_image(width, height, &rgba);
                    match encoded {
                        Ok(path) => {
                            if let Err(error) = upload_tx.try_send((
                                chat.clone(),
                                vec![path],
                                false,
                                caption.clone(),
                                None,
                                quoting.clone(),
                                true,
                            )) {
                                let batch = match error {
                                    std::sync::mpsc::TrySendError::Full(batch)
                                    | std::sync::mpsc::TrySendError::Disconnected(batch) => batch,
                                };
                                emit(Event::BeeperFilesUncertain {
                                    chat: batch.0,
                                    paths: batch.1,
                                    documents: false,
                                    caption: batch.3,
                                    quoting: batch.5,
                                    message: "Upload queue is busy; image kept without sending"
                                        .into(),
                                });
                            }
                        }
                        Err(_) => emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: crate::backend::Unsent::Image {
                                width,
                                height,
                                rgba,
                                view_once,
                                caption,
                            },
                            reason: crate::backend::Refusal::UnsupportedBackend,
                        }),
                    }
                }
                Command::RecentStickers => {
                    emit(Event::Stickers {
                        favorites: local_sticker_files(&dirs.saved_sticker_dir()),
                        packs: Vec::new(),
                        recent: Vec::new(),
                        received: local_sticker_files(&dirs.sticker_cache_dir()),
                        emojis: Default::default(),
                    });
                }
                Command::SendTypedMedia {
                    chat,
                    path,
                    kind,
                    caption,
                    quoting,
                } => {
                    if let Err(error) = typed_tx.try_send((chat, path, kind, caption, quoting)) {
                        let (chat, path, kind, caption, quoting) = match error {
                            std::sync::mpsc::TrySendError::Full(item)
                            | std::sync::mpsc::TrySendError::Disconnected(item) => item,
                        };
                        emit(Event::TypedMediaUncertain {
                            chat,
                            path,
                            kind,
                            caption,
                            quoting,
                            message: "Upload queue is busy. Media kept; nothing sent.".into(),
                        });
                    }
                }
                Command::SendVoice {
                    chat,
                    samples,
                    quoting,
                } => {
                    if quoting
                        .as_ref()
                        .is_some_and(|q| q.source_chat(&chat) != chat)
                    {
                        emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: crate::backend::Unsent::Voice(samples),
                            reason: crate::backend::Refusal::QuoteUnavailable,
                        });
                        continue;
                    }
                    let mut recording = samples.clone();
                    crate::voice::normalize(&mut recording);
                    let seconds = (recording.len() as f64 / f64::from(crate::voice::RATE))
                        .round()
                        .max(1.0) as u32;
                    let file = crate::voice::encode(&recording).and_then(|bytes| {
                        let path = std::env::temp_dir()
                            .join(format!("voice-note-{}.ogg", crate::util::now()));
                        std::fs::write(&path, bytes)
                            .map(|()| path)
                            .map_err(|error| error.to_string())
                    });
                    match file {
                        Ok(path) => {
                            let voice = Some((seconds, quoting.clone(), samples.clone()));
                            let batch = (chat.clone(), vec![path], false, None, voice, None, true);
                            if upload_tx.try_send(batch).is_err() {
                                emit(Event::SendRefused {
                                    chat,
                                    quoting,
                                    unsent: crate::backend::Unsent::Voice(samples),
                                    reason: crate::backend::Refusal::Offline,
                                });
                            }
                        }
                        Err(_) => emit(Event::SendRefused {
                            chat,
                            quoting,
                            unsent: crate::backend::Unsent::Voice(samples),
                            reason: crate::backend::Refusal::UnsupportedBackend,
                        }),
                    }
                }
                Command::LoadChatMedia {
                    chat,
                    around,
                    fresh,
                } => {
                    if let Some((chat, around, _)) =
                        gallery_backlog.submit(&gallery_tx, (chat, around, fresh))
                    {
                        emit(Event::BeeperViewerMedia {
                            chat,
                            around,
                            result: Err("Media history is busy; try the arrow again".into()),
                        });
                    }
                }
                Command::Download {
                    manual,
                    card,
                    chat,
                    message,
                } => {
                    // A full queue parks the request; it is sent as slots free.
                    // Only an overflowing backlog or a dead worker reports failure.
                    if let Some((card, chat, message)) =
                        media_backlog.submit(&media_tx, (manual, card, chat, message))
                    {
                        emit(Event::Media {
                            card,
                            chat,
                            message,
                            result: Err(
                                "Media queue is busy; retry after current downloads finish".into(),
                            ),
                        });
                    }
                }
                Command::FetchAvatar { id, full } => {
                    if let Err(error) = avatar_tx.try_send((id, full)) {
                        let (id, full) = match error {
                            std::sync::mpsc::TrySendError::Full(v)
                            | std::sync::mpsc::TrySendError::Disconnected(v) => v,
                        };
                        emit(Event::Avatar {
                            id,
                            full,
                            path: None,
                        });
                    }
                }
                Command::SendSticker { .. }
                | Command::CreateGroup { .. }
                | Command::SetProfile { .. }
                | Command::SetGroupName { .. }
                | Command::SetGroupPicture { .. }
                | Command::Forward { .. }
                | Command::CreatePoll { .. }
                | Command::VotePoll { .. }
                | Command::RefreshPoll { .. } => {
                    emit(Event::Info("This action is not supported by the Beeper client. Nothing was sent or changed.".into()));
                }
                Command::Resumed => {
                    log::info!("woke from sleep: refreshing chats and messages now");
                    refresh_after_wake = true;
                }
                _ => {}
            }
        }

        if refresh_after_wake {
            refresh_after_wake = false;
            force_polls_due(
                &mut last_chat_refresh,
                &mut last_msg_poll,
                &mut last_receipt_refresh,
            );
        }

        if let Some((token, clear)) = pending_storage
            && let Ok(_guard) = api.cache_lock.try_lock()
        {
            let result = trim_cache(&dirs, if clear { 0 } else { cache_budget(&dirs) });
            if let Err(e) = &result {
                emit(Event::Info(format!("Cache cleanup failed: {e}")));
            }
            emit(Event::StorageStats {
                stats: measure_cache(&dirs).ok(),
                token,
                cleared: clear && result.is_ok(),
            });
            pending_storage = None;
        }

        while let Ok((local, member, pending_id)) = pending_media_accepted_rx.try_recv() {
            tracked_media.insert(
                (member.chat.clone(), pending_id.clone()),
                (local, member.clone()),
            );
            pending_sends.push((member.chat, pending_id, Instant::now()));
        }
        while let Ok((chat, pending_id, account)) = forward_accepted_rx.try_recv() {
            forward_pending.insert((chat.clone(), pending_id.clone()), account);
            pending_sends.push((chat, pending_id, Instant::now()));
        }
        while let Ok((chat, pending_id)) = accepted_rx.try_recv() {
            pending_sends.push((chat, pending_id, Instant::now()));
        }
        if last_msg_poll.elapsed() >= MSG_POLL {
            last_msg_poll = Instant::now();
            let targets = if watched_chats.is_empty() {
                open_chat.clone().into_iter().collect()
            } else {
                watched_chats.clone()
            };
            for chat in targets {
                if dead_chats.contains(&chat) {
                    continue;
                }
                if poll_newer(&api, &chat, &mut chat_cursors, &emit) {
                    dead_chats.insert(chat);
                }
            }
            pending_sends.retain(|(chat, pending_id, started)| {
                let key = (chat.clone(), pending_id.clone());
                if let Some((local, member)) = tracked_media.get(&key).cloned() {
                    let keep = poll_media_send(
                        &api,
                        &local,
                        &member,
                        pending_id,
                        started.elapsed(),
                        &emit,
                    );
                    if !keep {
                        tracked_media.remove(&key);
                    }
                    keep
                } else if let Some(account) = forward_pending.get(&key).cloned() {
                    let keep = poll_forward_mark(
                        &api,
                        chat,
                        pending_id,
                        &account,
                        started.elapsed(),
                        &emit,
                    );
                    if !keep {
                        forward_pending.remove(&key);
                    }
                    keep
                } else {
                    poll_pending_send(&api, chat, pending_id, started.elapsed(), &emit)
                }
            });
        }
        if last_chat_refresh.elapsed() >= CHAT_REFRESH {
            last_chat_refresh = Instant::now();
            refresh_inventory(&api, &mut accounts, &emit);
        }
        // Receipt refresh: the forward-only poll never re-returns already
        // loaded messages, so their delivery/read ticks stall. Refetch the
        // open chat's newest page on a slow cadence; the UI merge upserts
        // by id, so scroll position, composer and read state are untouched
        // and no extra notifications fire (no Incoming events here).
        // A linked person's other chats are watched, not open, and their
        // ticks stall the same way, so every watched chat is refreshed.
        if last_receipt_refresh.elapsed() >= RECEIPT_REFRESH {
            last_receipt_refresh = Instant::now();
            let mut targets = watched_chats.clone();
            if let Some(open) = &open_chat
                && !targets.contains(open)
            {
                targets.push(open.clone());
            }
            for chat in &targets {
                let Ok(page) = api.list_messages(chat, None, None) else {
                    continue;
                };
                let mut messages: Vec<Message> = page
                    .items
                    .iter()
                    .filter(|m| !m.is_hidden && !m.is_deleted)
                    .map(|m| api.map_message(m, chat))
                    .collect();
                messages.sort_by_key(|m| m.timestamp);
                if !messages.is_empty() {
                    emit(Event::Messages {
                        chat: chat.clone(),
                        messages,
                        older: false,
                        complete: false,
                    });
                }
            }
        }

        std::thread::sleep(LOOP_TICK);
    }
}

/// Re-read a bounded cumulative result window on explicit Load more. Repeating
/// earlier pages keeps cursor state scoped to the exact query/filters and avoids
/// merging a stale response into a changed search. Stop on repeated/missing cursors.
/// A single API request per explicit Load more click. Keeps earlier pages and
/// cursors on the worker, so paging neither restarts the query nor blocks normal
/// chat updates behind a growing batch of already-fetched requests.
#[derive(Default)]
struct CachedSearch {
    page: Option<Page<BeeperMessage>>,
    pages: usize,
    seen: HashSet<String>,
}

impl CachedSearch {
    fn next(
        &mut self,
        api: &Api,
        mut params: Vec<(&str, String)>,
    ) -> Result<Page<BeeperMessage>, String> {
        let mut cursor = None;
        if let Some(page) = &self.page {
            if !page.has_more || self.pages >= 50 {
                return Ok(page.clone());
            }
            let next = page
                .oldest_cursor
                .as_ref()
                .filter(|c| !c.is_empty())
                .ok_or("Search continuation cursor unavailable; earlier results are kept")?;
            if self.seen.contains(next) {
                return Err("Search continuation did not advance; earlier results are kept".into());
            }
            params.push(("cursor", next.clone()));
            cursor = Some(next.clone());
        }
        let mut page: Page<BeeperMessage> = api.get("/v1/messages/search", &params)?;
        if page.has_more
            && page
                .oldest_cursor
                .as_ref()
                .is_some_and(|next| Some(next) == cursor.as_ref() || self.seen.contains(next))
        {
            return Err("Search continuation did not advance; earlier results are kept".into());
        }
        if let Some(old) = &self.page {
            let newer = std::mem::take(&mut page.items);
            page.items = old.items.clone();
            page.items.extend(newer);
        }
        let mut ids = HashSet::new();
        page.items
            .retain(|m| ids.insert((m.chat_id.clone(), m.id.clone())));
        if let Some(cursor) = cursor {
            self.seen.insert(cursor);
        }
        self.pages += 1;
        self.page = Some(page.clone());
        Ok(page)
    }
}

fn run_message_search_cached(
    api: &Api,
    query: String,
    emit: &dyn Fn(Event),
    cache: &mut CachedSearch,
) {
    let result = cache.next(api, vec![("query", query.clone())]);
    emit_message_search(api, query, emit, result, cache.pages);
}

#[allow(clippy::too_many_arguments)]
fn run_chat_search_cached(
    api: &Api,
    chat: ChatId,
    query: String,
    from: Option<i64>,
    until: Option<i64>,
    emit: &dyn Fn(Event),
    cache: &mut CachedSearch,
) {
    let result =
        chat_search_params(&chat, &query, from, until).and_then(|params| cache.next(api, params));
    emit_chat_search(api, chat, query, from, until, emit, result, cache.pages);
}

type MediaJob = (bool, Option<usize>, ChatId, String);

type GalleryRequest = (ChatId, Option<String>, bool);

/// One viewer-album request waits here when the gallery worker is mid-fetch,
/// instead of failing with "Media history is busy". The newest request wins,
/// and a parked `fresh` stays set so a dropped reset is never lost.
#[derive(Default)]
struct GalleryBacklog {
    waiting: Option<GalleryRequest>,
}

impl GalleryBacklog {
    /// Sends now if the worker has room, otherwise parks. Hands the request
    /// back only when the worker is gone.
    fn submit(
        &mut self,
        tx: &std::sync::mpsc::SyncSender<GalleryRequest>,
        request: GalleryRequest,
    ) -> Option<GalleryRequest> {
        let request = match self.waiting.take() {
            Some((_, _, fresh)) => (request.0, request.1, request.2 || fresh),
            None => request,
        };
        self.send(tx, request)
    }

    fn flush(
        &mut self,
        tx: &std::sync::mpsc::SyncSender<GalleryRequest>,
    ) -> Option<GalleryRequest> {
        let request = self.waiting.take()?;
        self.send(tx, request)
    }

    fn send(
        &mut self,
        tx: &std::sync::mpsc::SyncSender<GalleryRequest>,
        request: GalleryRequest,
    ) -> Option<GalleryRequest> {
        use std::sync::mpsc::TrySendError;
        match tx.try_send(request) {
            Ok(()) => None,
            Err(TrySendError::Full(request)) => {
                self.waiting = Some(request);
                None
            }
            Err(TrySendError::Disconnected(request)) => Some(request),
        }
    }
}

/// Download requests the bounded worker queue had no room for. Without it a
/// screenful of auto-downloading videos filled the queue and every overflow
/// became a permanent "Media queue is busy" failure, with no retry.
#[derive(Default)]
struct MediaBacklog {
    waiting: std::collections::VecDeque<MediaJob>,
}

impl MediaBacklog {
    const LIMIT: usize = 256;

    /// Queues the job, or hands back what must be reported as failed.
    fn submit(
        &mut self,
        tx: &std::sync::mpsc::SyncSender<MediaJob>,
        job: MediaJob,
    ) -> Option<(Option<usize>, ChatId, String)> {
        use std::sync::mpsc::TrySendError;
        if self.waiting.is_empty() {
            match tx.try_send(job) {
                Ok(()) => return None,
                Err(TrySendError::Disconnected((_, card, chat, message))) => {
                    return Some((card, chat, message));
                }
                Err(TrySendError::Full(job)) => return self.park(job),
            }
        }
        self.park(job)
    }

    fn park(&mut self, job: MediaJob) -> Option<(Option<usize>, ChatId, String)> {
        if let Some(index) = self
            .waiting
            .iter()
            .position(|waiting| (&waiting.2, &waiting.3, waiting.1) == (&job.2, &job.3, job.1))
        {
            // A click upgrades an automatic request, including the permission
            // to download media which automatic-download settings may forbid.
            if !job.0 || self.waiting[index].0 {
                return None;
            }
            self.waiting.remove(index);
        } else if self.waiting.len() >= Self::LIMIT {
            // Keep accepted automatic work in FIFO order; a new click may
            // replace one automatic job, but never another accepted click.
            if job.0
                && let Some(index) = self.waiting.iter().rposition(|waiting| !waiting.0)
            {
                let (_, card, chat, message) = self.waiting.remove(index).expect("index in range");
                self.insert(job);
                return Some((card, chat, message));
            }
            return Some((job.1, job.2, job.3));
        }
        self.insert(job);
        None
    }

    fn insert(&mut self, job: MediaJob) {
        if job.0 {
            let index = self
                .waiting
                .iter()
                .position(|waiting| !waiting.0)
                .unwrap_or(self.waiting.len());
            self.waiting.insert(index, job);
        } else {
            self.waiting.push_back(job);
        }
    }

    /// Moves waiting jobs into freed queue slots; returns those the worker can no longer take.
    fn flush(
        &mut self,
        tx: &std::sync::mpsc::SyncSender<MediaJob>,
    ) -> Vec<(Option<usize>, ChatId, String)> {
        use std::sync::mpsc::TrySendError;
        let mut failed = Vec::new();
        while let Some(job) = self.waiting.pop_front() {
            match tx.try_send(job) {
                Ok(()) => {}
                Err(TrySendError::Full(job)) => {
                    self.waiting.push_front(job);
                    break;
                }
                Err(TrySendError::Disconnected((_, card, chat, message))) => {
                    failed.push((card, chat, message));
                }
            }
        }
        failed
    }
}

#[cfg(test)]
fn run_chat_search(
    api: &Api,
    chat: ChatId,
    query: String,
    from: Option<i64>,
    until: Option<i64>,
    emit: &dyn Fn(Event),
) {
    run_chat_search_cached(
        api,
        chat,
        query,
        from,
        until,
        emit,
        &mut CachedSearch::default(),
    );
}

fn chat_search_params(
    chat: &str,
    query: &str,
    from: Option<i64>,
    until: Option<i64>,
) -> Result<Vec<(&'static str, String)>, String> {
    let mut params = vec![
        ("query", query.to_string()),
        ("chatIDs", chat.to_string()),
        ("excludeLowPriority", "false".into()),
    ];
    if let Some(from) = from {
        // API is strictly-after; UI uses an inclusive second boundary.
        let time = jiff::Timestamp::from_second(from.checked_sub(1).ok_or("Invalid search date")?)
            .map_err(|_| "Invalid search date")?;
        params.push(("dateAfter", time.to_string()));
    }
    if let Some(until) = until {
        params.push((
            "dateBefore",
            jiff::Timestamp::from_second(until)
                .map_err(|_| "Invalid search date")?
                .to_string(),
        ));
    }
    Ok(params)
}

#[allow(clippy::too_many_arguments)]
fn emit_chat_search(
    api: &Api,
    chat: ChatId,
    query: String,
    from: Option<i64>,
    until: Option<i64>,
    emit: &dyn Fn(Event),
    result: Result<Page<BeeperMessage>, String>,
    pages: usize,
) {
    match result {
        Ok(page) => {
            // Check scope locally as well, even if a server ignores filters.
            let mut messages: Vec<Message> = page
                .items
                .iter()
                .filter(|m| !m.is_hidden && !m.is_deleted && m.chat_id == chat)
                .map(|m| api.map_message(m, &chat))
                .filter(|m| {
                    from.is_none_or(|from| m.timestamp >= from)
                        && until.is_none_or(|until| m.timestamp < until)
                })
                .collect();
            messages.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
            emit(Event::ChatHits {
                chat: chat.clone(),
                query: query.clone(),
                from,
                until,
                messages,
                truncated: page.has_more && pages < 50,
            });
            if pages >= 50 && page.has_more {
                emit(Event::BeeperChatSearchError { chat, query, from, until,
                    error: "Showing the first 50 pages. Narrow the search or choose a day for older matches.".into() });
            }
        }
        Err(error) => emit(Event::BeeperChatSearchError {
            chat,
            query,
            from,
            until,
            error: format!("Message search unavailable: {error}"),
        }),
    }
}

#[cfg(test)]
fn run_message_search(api: &Api, query: String, emit: &dyn Fn(Event)) {
    run_message_search_cached(api, query, emit, &mut CachedSearch::default());
}

fn emit_message_search(
    api: &Api,
    query: String,
    emit: &dyn Fn(Event),
    result: Result<Page<BeeperMessage>, String>,
    pages: usize,
) {
    match result {
        Ok(page) => {
            let mut messages: Vec<Message> = page
                .items
                .iter()
                .filter(|m| !m.is_hidden && !m.is_deleted)
                .map(|m| api.map_message(m, ""))
                .collect();
            messages.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
            emit(Event::BeeperSearchState {
                query: query.clone(),
                error: (pages >= 50 && page.has_more).then(|| {
                    "Showing the first 50 pages. Narrow the search for older matches.".into()
                }),
                truncated: page.has_more && pages < 50,
            });
            emit(Event::SearchHits { query, messages });
        }
        Err(error) => emit(Event::BeeperSearchState {
            query,
            error: Some(format!("Message search unavailable: {error}")),
            truncated: pages > 0 && pages < 50,
        }),
    }
}

fn refresh_inventory(api: &Api, accounts: &mut Vec<BeeperAccount>, emit: &dyn Fn(Event)) {
    let account_error = match api.accounts() {
        Ok(updated) => {
            *accounts = updated;
            emit(Event::BeeperAccounts {
                origin: api.base_url.clone(),
                accounts: account_inventory(accounts),
            });
            None
        }
        Err(error) => Some(error),
    };
    match api.list_chats() {
        Ok(page) => {
            if let Some(error) = account_error {
                emit(Event::Link(LinkStatus::Disconnected {
                    reason: format!(
                        "Beeper account inventory refresh failed; account labels may be stale: {error}"
                    ),
                }));
            } else {
                emit(Event::Link(LinkStatus::Connected));
            }
            let label = network_labeller(&page.items, accounts);
            emit(Event::Chats(
                page.items.iter().map(|c| api.map_chat(c, &label)).collect(),
            ));
        }
        Err(error) => emit(Event::Link(LinkStatus::Disconnected {
            reason: format!("Beeper chat refresh failed: {error}"),
        })),
    }
}

fn poll_media_send(
    api: &Api,
    local: &str,
    member: &crate::merged::Member,
    pending_id: &str,
    elapsed: Duration,
    emit: &dyn Fn(Event),
) -> bool {
    let mut keep = true;
    if let Ok(wire) = api.get_message(&member.chat, pending_id)
        && wire.account_id == member.account
        && wire.chat_id == member.chat
        && wire.is_sender == Some(true)
    {
        let message = api.map_message(&wire, &member.chat);
        // Delivery evidence is independent of attachment metadata availability.
        let complete = wire.send_status.as_ref().is_some_and(|s| {
            s.status == "SUCCESS"
                || (!s.status.starts_with("FAIL_") && !s.delivered_to_users.is_empty())
        });
        emit(Event::BeeperMediaResolved {
            local: local.to_owned(),
            member: member.clone(),
            message: Box::new(message),
            complete,
        });
        if complete {
            keep = false;
        } else if wire
            .send_status
            .as_ref()
            .is_some_and(|s| s.status.starts_with("FAIL_"))
        {
            emit(Event::BeeperMediaProgress {
                local: local.to_owned(),
                member: member.clone(),
                state: crate::pending_media::State::Unknown(
                    "Bridge reports failure; check history before sending again".into(),
                ),
            });
            keep = false;
        }
    }
    if keep && elapsed >= Duration::from_secs(60) {
        emit(Event::BeeperMediaProgress {
            local: local.to_owned(),
            member: member.clone(),
            state: crate::pending_media::State::Unknown(
                "Confirmation timed out; check chat history".into(),
            ),
        });
        keep = false;
    }
    keep
}

fn poll_forward_mark(
    api: &Api,
    chat: &ChatId,
    pending: &str,
    account: &str,
    elapsed: Duration,
    emit: &dyn Fn(Event),
) -> bool {
    if let Ok(wire) = api.get_message(chat, pending)
        && wire.chat_id == *chat
        && wire.account_id == account
        && wire.is_sender == Some(true)
    {
        let confirmed = wire.send_status.as_ref().is_some_and(|s| {
            s.status == "SUCCESS"
                || (!s.status.starts_with("FAIL_") && !s.delivered_to_users.is_empty())
        });
        let mapped = api.map_message(&wire, chat);
        let failed = matches!(mapped.status, Delivery::Failed);
        if confirmed {
            emit(Event::BeeperSendConfirmed {
                origin: api.base_url.clone(),
                account: account.into(),
                chat: chat.clone(),
            });
        }
        if (confirmed || failed) && !wire.id.is_empty() {
            emit(Event::BeeperForwardResolved {
                origin: api.base_url.clone(),
                account: account.into(),
                chat: chat.clone(),
                pending: pending.into(),
                final_id: wire.id.clone(),
            });
        }
        if failed {
            emit(Event::Info(
                "Beeper reports that a message failed to send".into(),
            ));
        }
        emit(Event::Messages {
            chat: chat.clone(),
            messages: vec![mapped],
            older: false,
            complete: false,
        });
        if confirmed || failed {
            return false;
        }
    }
    // Timeout never prunes persisted identity: a later startup resolves it.
    if elapsed >= Duration::from_secs(60) {
        emit(Event::Info(
            "Beeper forward confirmation timed out; check history before retrying".into(),
        ));
        false
    } else {
        true
    }
}

fn poll_pending_send(
    api: &Api,
    chat: &ChatId,
    pending_id: &str,
    elapsed: Duration,
    emit: &dyn Fn(Event),
) -> bool {
    let message: Result<BeeperMessage, String> = api.get_message(chat, pending_id);
    match message {
        Ok(m) => {
            // Retrieving by pendingMessageID may resolve to the final ID.
            // The authenticated exact-chat lookup binds that alias; comparing
            // the returned ID with the pending ID rejects valid SUCCESS replies.
            let explicit_success = m.chat_id == *chat
                && m.is_sender == Some(true)
                && m.send_status.as_ref().is_some_and(|s| {
                    s.status == "SUCCESS"
                        || (!s.status.starts_with("FAIL_") && !s.delivered_to_users.is_empty())
                });
            if explicit_success && !m.account_id.is_empty() {
                emit(Event::BeeperSendConfirmed {
                    origin: api.base_url.clone(),
                    account: m.account_id.clone(),
                    chat: chat.clone(),
                });
            }
            let mapped = api.map_message(&m, chat);
            // Some bridges (reported for WhatsApp) never attach a sendStatus.
            // Once the lookup has resolved to a final message ID that is ours,
            // in this chat, with no status at all, the message is no longer a
            // pending echo and nothing says it failed: stop polling quietly
            // rather than toasting after 60 seconds. This is not a delivery
            // confirmation, so no BeeperSendConfirmed is emitted for it.
            let resolved_without_status = m.id != pending_id
                && !m.id.is_empty()
                && m.chat_id == *chat
                && m.is_sender == Some(true)
                && m.send_status.is_none();
            let complete = explicit_success
                || resolved_without_status
                || matches!(mapped.status, Delivery::Failed);
            if matches!(mapped.status, Delivery::Failed) {
                emit(Event::Info(
                    "Beeper reports that a message failed to send".into(),
                ));
            }
            emit(Event::Messages {
                chat: chat.clone(),
                messages: vec![mapped],
                older: false,
                complete: false,
            });
            if !complete && elapsed >= Duration::from_secs(60) {
                emit(Event::Info("Beeper has not confirmed this send after 60 seconds; check history before retrying".into()));
                false
            } else {
                !complete
            }
        }
        Err(_) if elapsed < Duration::from_secs(60) => true,
        Err(_) => {
            emit(Event::Info(
                "Beeper send confirmation timed out; check history before retrying".into(),
            ));
            false
        }
    }
}

/// Returns true when the Server answered 404 for this chat: terminal,
/// the caller marks it dead instead of polling again.
fn poll_newer(
    api: &Api,
    chat: &ChatId,
    cursors: &mut HashMap<ChatId, ChatCursors>,
    emit: &dyn Fn(Event),
) -> bool {
    // Empty chats may have no cursor. Keep polling the newest page until one exists.
    let mut newest = cursors.get(chat).and_then(|c| c.newest.clone());
    let mut seen = HashSet::new();
    if let Some(cursor) = &newest {
        seen.insert(cursor.clone());
    }
    let mut pages = 0;
    loop {
        if pages >= 32 {
            emit(Event::BeeperLiveState {
                chat: chat.clone(),
                error: Some("Live updates are catching up in bounded batches".into()),
            });
            break;
        }
        pages += 1;
        match api.list_messages(
            chat,
            newest.as_deref(),
            if newest.is_some() {
                Some("after")
            } else {
                None
            },
        ) {
            Ok(page) => {
                let more = page.has_more;
                let next = page.newest_cursor.clone();
                let stalled = more
                    && newest.is_some()
                    && next.as_ref().is_none_or(|cursor| seen.contains(cursor));
                if !stalled {
                    emit(Event::BeeperLiveState {
                        chat: chat.clone(),
                        error: None,
                    });
                    if let Some(c) = cursors.get_mut(chat) {
                        c.newest = next.clone().or(c.newest.clone());
                    }
                }
                let mut messages: Vec<Message> = page
                    .items
                    .iter()
                    .filter(|m| !m.is_hidden && !m.is_deleted)
                    .map(|m| api.map_message(m, chat))
                    .collect();
                messages.sort_by_key(|m| m.timestamp);
                for m in page.items.iter().filter(|m| m.is_deleted) {
                    emit(Event::MessageDeleted {
                        chat: chat.clone(),
                        id: m.id.clone(),
                    });
                }
                if !messages.is_empty() {
                    let incoming: Vec<Message> =
                        messages.iter().filter(|m| !m.from_me).cloned().collect();
                    emit(Event::Messages {
                        chat: chat.clone(),
                        messages,
                        older: false,
                        complete: false,
                    });
                    for message in incoming {
                        emit(Event::Incoming {
                            chat: chat.clone(),
                            message: Box::new(message),
                        });
                    }
                }
                // A bad continuation cursor must not discard valid messages on this page.
                if stalled {
                    emit(Event::BeeperLiveState {chat:chat.clone(),error:Some("Beeper could not advance to the next update page. Messages already loaded are kept, but newer messages may be missing".into())});
                    break;
                }
                if !more || newest.is_none() {
                    break;
                }
                let Some(next) = next else {
                    break;
                };
                if !seen.insert(next.clone()) {
                    break;
                }
                newest = Some(next);
            }
            Err(e) => {
                let (copy, terminal) = live_error_copy(&e);
                emit(Event::BeeperLiveState {
                    chat: chat.clone(),
                    error: Some(copy),
                });
                if terminal {
                    return true;
                }
                break;
            }
        }
    }
    false
}

/// `Command::Resumed`: after a wake every poll is due at once. A link open
/// across sleep can look established while nothing arrives, so the next tick
/// re-reads everything instead of waiting the timers out.
fn force_polls_due(
    last_chat_refresh: &mut Instant,
    last_msg_poll: &mut Instant,
    last_receipt_refresh: &mut Instant,
) {
    *last_chat_refresh = Instant::now() - CHAT_REFRESH;
    *last_msg_poll = Instant::now() - MSG_POLL;
    *last_receipt_refresh = Instant::now() - RECEIPT_REFRESH;
}

#[cfg(test)]
mod tests {
    //! Integration tests against a stub Beeper Client API (std TCP, no deps).
    //! They pin the demo's contract: multiple accounts on one network stay
    //! distinguishable, background startup/polling causes NO mutating HTTP
    //! calls, and only deliberate commands (send, mark-read, typing) POST.

    #[test]
    fn supplied_preview_parsing_only_runs_for_text_with_metadata() {
        for (attachment, metadata, expected_parses) in [
            (false, false, 0),
            (true, false, 0),
            (true, true, 0),
            (false, true, 1),
        ] {
            let mut raw = serde_json::json!({
                "id":"parse-gate", "text":"https://example.org/story"
            });
            if metadata {
                raw["links"] = serde_json::json!([{
                    "url":"https://example.org/story", "title":"Story"
                }]);
            }
            if attachment {
                raw["attachments"] = serde_json::json!([{
                    "type":"img", "id":"mxc://example/image", "mimeType":"image/png"
                }]);
            }
            let wire: BeeperMessage = serde_json::from_value(raw).unwrap();
            SUPPLIED_PREVIEW_BODY_PARSES.with(|count| count.set(0));
            let message = map_message(&wire, "chat");
            assert_eq!(
                SUPPLIED_PREVIEW_BODY_PARSES.with(|count| count.get()),
                expected_parses,
                "attachment={attachment}, metadata={metadata}"
            );
            if attachment {
                assert!(matches!(message.content, Content::Image { .. }));
            } else {
                let Content::Text { text, preview } = message.content else {
                    panic!("text")
                };
                assert_eq!(text, "https://example.org/story");
                assert_eq!(preview.is_some(), metadata);
            }
        }
    }

    #[test]
    fn supplied_previews_are_bound_to_original_message_links_without_fetching() {
        let raw = serde_json::json!({
            "id":"preview", "text":"A title (https://example.org/news?utm_source=chat#part)",
            "links":[{"title":"  Article\nheadline  ", "summary":"Short description", "url":"https://redirected.example/article", "originalURL":"https://example.org/news?utm_source=chat#part", "img":"http://127.0.0.1/private", "favicon":"file:///secret"}]
        });
        let wire: BeeperMessage = serde_json::from_value(raw.clone()).unwrap();
        let message = map_message(&wire, "chat");
        let Content::Text {
            text,
            preview: Some(preview),
        } = message.content
        else {
            panic!("supplied preview");
        };
        assert_eq!(
            text,
            "A title (https://example.org/news?utm_source=chat#part)"
        );
        assert_eq!(preview.url, "https://example.org/news?utm_source=chat#part");
        assert_eq!(preview.title.as_deref(), Some("Article headline"));
        assert_eq!(preview.description.as_deref(), Some("Short description"));
        assert!(
            message.thumbnail.is_none(),
            "supplied remote image URLs must never cause a network fetch"
        );
        for url in [
            "file:///secret",
            "https://unrelated.example/",
            "javascript:alert(1)",
        ] {
            let mut raw = raw.clone();
            raw["links"][0]["originalURL"] = url.into();
            let wire: BeeperMessage = serde_json::from_value(raw).unwrap();
            assert!(matches!(
                map_message(&wire, "chat").content,
                Content::Text { preview: None, .. }
            ));
        }
        let wire: BeeperMessage =
            serde_json::from_str(r#"{"id":"plain","text":"https://example.org/"}"#).unwrap();
        assert!(matches!(
            map_message(&wire, "chat").content,
            Content::Text { preview: None, .. }
        ));
    }

    #[test]
    fn supplied_original_url_is_authoritative_over_resolved_url() {
        let raw = serde_json::json!({"id":"preview", "text":"https://example.org/story", "links":[{"originalURL":"https://unrelated.example/redirect", "url":"https://example.org/story", "title":"Wrong article"}]});
        let wire: BeeperMessage = serde_json::from_value(raw.clone()).unwrap();
        assert!(matches!(
            map_message(&wire, "chat").content,
            Content::Text { preview: None, .. }
        ));
        let mut raw = raw;
        raw["links"][0]
            .as_object_mut()
            .unwrap()
            .remove("originalURL");
        let wire: BeeperMessage = serde_json::from_value(raw).unwrap();
        assert!(matches!(
            map_message(&wire, "chat").content,
            Content::Text {
                preview: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn supplied_preview_chooses_body_link_order_and_bounds_metadata() {
        let wire: BeeperMessage = serde_json::from_value(serde_json::json!({"id":"multi", "text":"https://first.example/ then https://second.example/", "links":[
            {"url":"https://second.example/", "title":"second"},
            {"url":"https://first.example/", "title":"x".repeat(2000), "summary":"y".repeat(3000)}
        ]})).unwrap();
        let Content::Text {
            preview: Some(preview),
            ..
        } = map_message(&wire, "chat").content
        else {
            panic!()
        };
        assert_eq!(preview.url, "https://first.example/");
        assert_eq!(preview.title.unwrap().len(), 240);
        assert_eq!(preview.description.unwrap().len(), 600);
    }

    fn avatar_chat(kind: &str, chat_img: Option<&str>) -> BeeperChat {
        let mut value = serde_json::json!({
            "id": "chat-1", "accountID": "a", "network": "WhatsApp", "title": "Tea", "type": kind,
            "participants": {"items": [
                {"id": "@me:beeper.local", "isSelf": true, "imgURL": "file:///me.png"},
                {"id": "tea", "fullName": "Tea", "imgURL": "file:///tea.png"}
            ]}
        });
        if let Some(img) = chat_img {
            value["imgURL"] = serde_json::json!(img);
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_direct_chat_without_its_own_avatar_shows_the_other_persons() {
        let api = Api::new(&BeeperConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "fixture".into(),
        })
        .unwrap();
        api.map_chat(&avatar_chat("single", None), &|c| c.network.clone());
        let sources = api.avatar_sources.lock().unwrap();
        assert_eq!(
            sources.get("chat-1").map(String::as_str),
            Some("file:///tea.png")
        );
    }

    #[test]
    fn a_borrowed_avatar_goes_when_the_contact_photo_is_removed() {
        let api = Api::new(&BeeperConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "fixture".into(),
        })
        .unwrap();
        let label = |c: &BeeperChat| c.network.clone();
        api.map_chat(&avatar_chat("single", None), &label);
        assert!(api.avatar_sources.lock().unwrap().contains_key("chat-1"));
        // The next inventory reports the participant without a picture.
        let mut gone = avatar_chat("single", None);
        for user in &mut gone.participants.as_mut().unwrap().items {
            if user.id == "tea" {
                user.image_url = None;
            }
        }
        api.map_chat(&gone, &label);
        let sources = api.avatar_sources.lock().unwrap();
        assert!(!sources.contains_key("chat-1"), "borrowed source dropped");
        // Participant-id sources are not touched by this.
        assert!(sources.contains_key("@me:beeper.local"));
    }

    #[test]
    fn an_inventory_without_participants_keeps_the_borrowed_avatar() {
        let api = Api::new(&BeeperConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "fixture".into(),
        })
        .unwrap();
        let label = |c: &BeeperChat| c.network.clone();
        api.map_chat(&avatar_chat("single", None), &label);
        let mut partial = avatar_chat("single", None);
        partial.participants = None;
        api.map_chat(&partial, &label);
        assert_eq!(
            api.avatar_sources
                .lock()
                .unwrap()
                .get("chat-1")
                .map(String::as_str),
            Some("file:///tea.png")
        );
    }

    #[test]
    fn a_chat_owned_avatar_is_not_dropped_by_the_borrow_cleanup() {
        let api = Api::new(&BeeperConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "fixture".into(),
        })
        .unwrap();
        let label = |c: &BeeperChat| c.network.clone();
        api.map_chat(&avatar_chat("single", Some("file:///chat.png")), &label);
        let mut no_photo = avatar_chat("single", Some("file:///chat.png"));
        for user in &mut no_photo.participants.as_mut().unwrap().items {
            user.image_url = None;
        }
        api.map_chat(&no_photo, &label);
        assert_eq!(
            api.avatar_sources
                .lock()
                .unwrap()
                .get("chat-1")
                .map(String::as_str),
            Some("file:///chat.png")
        );
    }

    #[test]
    fn the_chats_own_avatar_wins_and_groups_never_borrow_a_members() {
        fn own(wire: &BeeperChat) -> Option<&str> {
            direct_chat_avatar(wire, Some("@me:beeper.local"))
        }
        assert_eq!(own(&avatar_chat("single", None)), Some("file:///tea.png"));
        assert_eq!(own(&avatar_chat("single", Some("file:///chat.png"))), None);
        assert_eq!(own(&avatar_chat("group", None)), None);
        let mut two = avatar_chat("single", None);
        two.participants.as_mut().unwrap().items.push(
            serde_json::from_value(serde_json::json!({"id": "x", "imgURL": "file:///x.png"}))
                .unwrap(),
        );
        assert_eq!(own(&two), None, "ambiguous: more than one other person");
    }

    #[test]
    fn own_user_and_opaque_ids_never_become_handles() {
        let wire: BeeperChat = serde_json::from_str(
            r#"{"id":"c2","accountID":"a","network":"Signal","title":"Anna Example","type":"single",
            "participants":{"items":[
              {"id":"@me:beeper.local","username":"my_name","phoneNumber":"+358401111111"},
              {"id":"contact42","fullName":"Anna Example"},
              {"id":"+358 40 222 2222"}]}}"#,
        )
        .unwrap();
        let chat = map_chat_for(&wire, &|c| c.network.clone(), Some("@me:beeper.local"));
        assert_eq!(chat.handles, ["+358402222222"]);
    }

    #[test]
    fn direct_chats_keep_only_the_other_persons_handles() {
        let wire: BeeperChat = serde_json::from_str(
            r#"{"id":"c1","accountID":"a","network":"Signal","title":"Anna Example","type":"single",
            "participants":{"items":[
              {"id":"@me:beeper.local","fullName":"Me","phoneNumber":"+358401111111","isSelf":true},
              {"id":"@anna:beeper.local","fullName":"Anna Example","phoneNumber":"+358 40 222 2222","username":"anna_x"}]}}"#,
        )
        .unwrap();
        let chat = map_chat(&wire, &|c| c.network.clone());
        assert_eq!(chat.handles, ["+358402222222", "anna_x"]);
    }

    #[test]
    fn full_media_queue_parks_downloads_and_sends_them_as_slots_free() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<MediaJob>(1);
        let mut backlog = MediaBacklog::default();
        let job = |m: &str, manual| (manual, None, "c".to_string(), m.to_string());
        assert!(backlog.submit(&tx, job("a", false)).is_none());
        // Queue is full: these wait instead of failing.
        assert!(backlog.submit(&tx, job("b", false)).is_none());
        assert!(
            backlog.submit(&tx, job("b", false)).is_none(),
            "duplicate ignored"
        );
        assert!(backlog.submit(&tx, job("click", true)).is_none());
        assert_eq!(backlog.waiting.len(), 2);
        assert!(backlog.flush(&tx).is_empty());
        assert_eq!(rx.recv().unwrap().3, "a");
        assert!(backlog.flush(&tx).is_empty());
        assert_eq!(
            rx.recv().unwrap().3,
            "click",
            "a click goes before automatic ones"
        );
        assert!(backlog.flush(&tx).is_empty());
        assert_eq!(rx.recv().unwrap().3, "b");
        assert!(backlog.waiting.is_empty());
    }

    #[test]
    fn media_click_promotes_a_duplicate_and_preserves_click_fifo_at_capacity() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<MediaJob>(1);
        let mut backlog = MediaBacklog::default();
        let job = |message: &str, manual| (manual, None, "c".to_owned(), message.to_owned());
        backlog.submit(&tx, job("in-flight", false));
        backlog.submit(&tx, job("auto", false));
        backlog.submit(&tx, job("first-click", true));
        backlog.submit(&tx, job("auto", true));
        backlog.submit(&tx, job("auto", false));
        assert_eq!(backlog.waiting.len(), 2);
        rx.recv().unwrap();
        backlog.flush(&tx);
        assert_eq!(rx.recv().unwrap().3, "first-click");
        backlog.flush(&tx);
        let upgraded = rx.recv().unwrap();
        assert_eq!(upgraded.3, "auto");
        assert!(upgraded.0, "a click retains manual download permission");
        backlog.submit(&tx, job("in-flight", false));
        for index in 0..MediaBacklog::LIMIT {
            backlog.submit(&tx, job(&format!("auto-{index}"), false));
        }
        let refused = backlog.submit(&tx, job("overflow-auto", false)).unwrap();
        assert_eq!(refused.2, "overflow-auto");
        let displaced = backlog.submit(&tx, job("priority-click", true)).unwrap();
        assert_eq!(displaced.2, format!("auto-{}", MediaBacklog::LIMIT - 1));
        assert!(backlog.waiting.front().unwrap().0);
        assert_eq!(backlog.waiting.len(), MediaBacklog::LIMIT);
    }

    #[test]
    fn gallery_backlog_parks_while_the_worker_is_busy_and_sends_when_free() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<GalleryRequest>(1);
        let mut backlog = GalleryBacklog::default();
        let req = |m: &str, fresh| ("chat".to_string(), Some(m.to_string()), fresh);
        assert!(backlog.submit(&tx, req("a", true)).is_none()); // fills the queue
        // The worker is busy: these must park, not fail.
        assert!(backlog.submit(&tx, req("b", true)).is_none());
        assert!(backlog.submit(&tx, req("c", false)).is_none());
        assert!(backlog.flush(&tx).is_none()); // still full, stays parked
        assert_eq!(rx.recv().unwrap().1.as_deref(), Some("a"));
        assert!(backlog.flush(&tx).is_none());
        // The newest request won and kept the dropped one's reset.
        assert_eq!(rx.recv().unwrap(), req("c", true));
        assert!(backlog.flush(&tx).is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn gallery_backlog_reports_only_a_dead_worker() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<GalleryRequest>(1);
        drop(rx);
        let mut backlog = GalleryBacklog::default();
        let request = ("chat".to_string(), None, false);
        assert_eq!(backlog.submit(&tx, request.clone()), Some(request));
    }

    #[test]
    fn media_backlog_is_bounded_and_reports_a_dead_worker() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<MediaJob>(1);
        let mut backlog = MediaBacklog::default();
        let job = |m: String| (false, None, "c".to_string(), m);
        assert!(backlog.submit(&tx, job("first".into())).is_none());
        for i in 0..MediaBacklog::LIMIT {
            assert!(backlog.submit(&tx, job(format!("m{i}"))).is_none());
        }
        assert!(backlog.submit(&tx, job("over".into())).is_some());
        assert_eq!(backlog.waiting.len(), MediaBacklog::LIMIT);
        drop(rx);
        assert_eq!(backlog.flush(&tx).len(), MediaBacklog::LIMIT);
        assert!(backlog.submit(&tx, job("dead".into())).is_some());
    }

    #[test]
    fn downloads_without_a_message_type_get_an_extension_from_name_header_or_bytes() {
        let mp4 = *b"\0\0\0\x18ftypisom\0\0\0\0";
        assert_eq!(media_extension(Some("video/mp4"), None, None, &[]), "mp4");
        assert_eq!(media_extension(None, Some("IMG_1.MOV"), None, &[]), "mov");
        assert_eq!(
            media_extension(Some(""), None, Some("video/mp4; x=1"), &[]),
            "mp4"
        );
        assert_eq!(media_extension(None, None, None, &mp4), "mp4");
        let iphone = *b"\0\0\0\x14ftypqt  \0\0\0\0";
        assert_eq!(media_extension(None, None, None, &iphone), "mov");
        let ebml = |doc: &[u8]| {
            let mut head = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x84];
            head.extend_from_slice(doc);
            head
        };
        assert_eq!(
            media_extension(None, Some("noext"), None, &ebml(b"webm")),
            "webm"
        );
        assert_eq!(media_extension(None, None, None, &ebml(b"matroska")), "mkv");
        assert_eq!(media_extension(None, None, None, &ebml(b"????")), "bin");
        // A major brand the table does not know falls back to the compatible brands.
        let mut iso8 = b"\0\0\0\x18ftypiso8\0\0\0\0".to_vec();
        iso8.extend_from_slice(b"iso8mp41");
        assert_eq!(media_extension(None, None, None, &iso8), "mp4");
        let mut m4a8 = b"\0\0\0\x18ftypzzzz\0\0\0\0".to_vec();
        m4a8.extend_from_slice(b"M4A zzzz");
        assert_eq!(media_extension(None, None, None, &m4a8), "m4a");
        for (brand, ext) in [
            (b"M4A ", "m4a"),
            (b"heic", "heic"),
            (b"avif", "avif"),
            (b"3gp4", "3gp"),
            (b"mp42", "mp4"),
            (b"zzzz", "bin"),
        ] {
            let mut head = b"\0\0\0\x18ftyp".to_vec();
            head.extend_from_slice(brand);
            head.extend_from_slice(&[0; 8]);
            assert_eq!(
                media_extension(None, None, None, &head),
                ext,
                "brand {brand:?}"
            );
        }
        assert_eq!(
            media_extension(None, None, Some("application/octet-stream"), b"junk"),
            "bin"
        );
    }

    use super::*;

    #[test]
    fn a_wake_makes_every_poll_due_at_once() {
        let mut chat = Instant::now();
        let mut messages = Instant::now();
        let mut receipts = Instant::now();
        force_polls_due(&mut chat, &mut messages, &mut receipts);
        assert!(chat.elapsed() >= CHAT_REFRESH);
        assert!(messages.elapsed() >= MSG_POLL);
        assert!(receipts.elapsed() >= RECEIPT_REFRESH);
    }
    use crate::paths::AppDirs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    fn pending_media_fixture(
        responses: Vec<(&'static str, u16, String)>,
    ) -> (Api, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (expected, status, body) in responses {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && start.elapsed() < Duration::from_secs(10) =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("waiting for {expected}: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut buf = [0; 8192];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let header = String::from_utf8_lossy(&request[..header_end]);
                assert!(header.starts_with(expected), "unexpected request: {header}");
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + length {
                    let mut buf = [0; 8192];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (
            Api::new(&BeeperConfig {
                base_url,
                token: "fixture".into(),
            })
            .unwrap(),
            worker,
        )
    }
    fn pending_media_job(origin: &str, path: &std::path::Path) -> crate::pending_media::Job {
        crate::pending_media::Job {
            local: "local-fixture".into(),
            temporary: false,
            member: crate::merged::Member {
                origin: origin.into(),
                account: "signal".into(),
                chat: "target".into(),
            },
            payload: crate::pending_media::Payload::File {
                path: path.into(),
                document: false,
            },
            caption: None,
            quoting: None,
        }
    }
    #[test]
    fn pending_media_submission_boundary_is_not_inferred_from_error_words() {
        for (upload_status, send_status, expected_unknown) in
            [(503, 200, false), (200, 503, true), (200, 200, false)]
        {
            let mut responses = vec![
                (
                    "GET /v1/chats/target ",
                    200,
                    r#"{"id":"target","accountID":"signal"}"#.into(),
                ),
                (
                    "POST /v1/assets/upload/base64 ",
                    upload_status,
                    r#"{"uploadID":"asset"}"#.into(),
                ),
            ];
            if upload_status == 200 {
                responses.push((
                    "GET /v1/chats/target ",
                    200,
                    r#"{"id":"target","accountID":"signal"}"#.into(),
                ));
                responses.push((
                    "POST /v1/chats/target/messages ",
                    send_status,
                    r#"{"chatID":"target","pendingMessageID":"pending"}"#.into(),
                ));
            }
            let (api, server) = pending_media_fixture(responses);
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), b"synthetic").unwrap();
            let result = api.send_pending_media(
                &pending_media_job(&api.base_url, file.path()),
                &AtomicBool::new(false),
            );
            match result {
                Ok(accepted) => {
                    assert_eq!(send_status, 200);
                    assert_eq!(upload_status, 200);
                    assert_eq!(accepted.pending_id, "pending");
                }
                Err(crate::pending_media::State::NotSubmitted(_)) => assert!(!expected_unknown),
                Err(crate::pending_media::State::Unknown(_)) => assert!(expected_unknown),
                other => panic!("unexpected {other:?}"),
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn pending_media_acceptance_uses_requested_chat_when_omitted() {
        for (chat_field, pending, accepted) in [
            ("", "pending", true),
            (",\"chatID\":\"target\"", "pending", true),
            (",\"chatID\":\"foreign\"", "pending", false),
            ("", "", false),
        ] {
            let response = format!("{{\"pendingMessageID\":\"{pending}\"{chat_field}}}");
            let (api, server) = pending_media_fixture(vec![
                (
                    "GET /v1/chats/target ",
                    200,
                    r#"{"id":"target","accountID":"signal"}"#.into(),
                ),
                (
                    "POST /v1/assets/upload/base64 ",
                    200,
                    r#"{"uploadID":"asset"}"#.into(),
                ),
                (
                    "GET /v1/chats/target ",
                    200,
                    r#"{"id":"target","accountID":"signal"}"#.into(),
                ),
                ("POST /v1/chats/target/messages ", 200, response),
            ]);
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), b"synthetic").unwrap();
            let result = api.send_pending_media(
                &pending_media_job(&api.base_url, file.path()),
                &AtomicBool::new(false),
            );
            assert_eq!(
                result.is_ok(),
                accepted,
                "chat field {chat_field:?}, pending {pending:?}"
            );
            if let Ok(result) = result {
                assert_eq!(result.chat, "target");
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn pending_media_poll_requires_exact_account_and_binds_final_id() {
        for (account, sender, expect_resolved) in [
            ("foreign", true, false),
            ("signal", false, false),
            ("signal", true, true),
        ] {
            let body = serde_json::json!({"id":"final","chatID":"target","accountID":account,"isSender":sender,"attachments":[{"type":"audio","id":"asset","isVoiceNote":true,"duration":1}],"sendStatus":{"status":"SUCCESS"}}).to_string();
            let (api, server) =
                pending_media_fixture(vec![("GET /v1/chats/target/messages/pending ", 200, body)]);
            let member = pending_media_job(&api.base_url, std::path::Path::new("unused")).member;
            let events = Mutex::new(Vec::new());
            let keep = poll_media_send(
                &api,
                "local",
                &member,
                "pending",
                Duration::from_secs(61),
                &|event| events.lock().unwrap().push(event),
            );
            assert!(!keep);
            let events = events.into_inner().unwrap();
            assert_eq!(events.iter().any(|e| matches!(e, Event::BeeperMediaResolved { local, message, complete: true, .. } if local == "local" && message.id == "final")), expect_resolved);
            assert_eq!(
                events.iter().any(|e| matches!(
                    e,
                    Event::BeeperMediaProgress {
                        state: crate::pending_media::State::Unknown(_),
                        ..
                    }
                )),
                !expect_resolved
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn pending_media_sparse_success_is_confirmed_not_unknown_at_timeout() {
        let (api, server) = pending_media_fixture(vec![("GET /v1/chats/target/messages/pending ", 200,
            r#"{"id":"final","chatID":"target","accountID":"signal","isSender":true,"sendStatus":{"status":"SUCCESS"}}"#.into())]);
        let member = pending_media_job(&api.base_url, std::path::Path::new("unused")).member;
        let events = Mutex::new(Vec::new());
        assert!(!poll_media_send(
            &api,
            "local",
            &member,
            "pending",
            Duration::from_secs(61),
            &|e| events.lock().unwrap().push(e)
        ));
        let events = events.into_inner().unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::BeeperMediaResolved { complete: true, .. }))
        );
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::BeeperMediaProgress {
                state: crate::pending_media::State::Unknown(_),
                ..
            }
        )));
        server.join().unwrap();
    }

    #[test]
    fn pending_media_destination_mismatch_refuses_before_upload() {
        let (api, server) = pending_media_fixture(vec![(
            "GET /v1/chats/target ",
            200,
            r#"{"id":"target","accountID":"foreign"}"#.into(),
        )]);
        let result = api.send_pending_media(
            &pending_media_job(&api.base_url, std::path::Path::new("not-read")),
            &AtomicBool::new(false),
        );
        assert!(matches!(
            result,
            Err(crate::pending_media::State::NotSubmitted(_))
        ));
        assert_eq!(server.join().unwrap().len(), 1);
    }

    fn forward_fixture_item(origin: &str) -> crate::composed_forward::Item {
        crate::composed_forward::Item {
            origin: origin.into(),
            planned: true,
            source_chat: "source".into(),
            source_message: "message".into(),
            source_account: "telegram".into(),
            destination_account: "signal".into(),
            attachment: None,
            destination: "target".into(),
            text: "Forwarded from Ada (Telegram), date:\nhello @somebody".into(),
            state: crate::composed_forward::State::Waiting,
        }
    }

    /// A finite, timeout-bounded server. Assert every network request and payload,
    /// so a premature message POST cannot pass as a safe preparation failure.
    fn forward_fixture(
        responses: Vec<(&'static str, u16, String)>,
    ) -> (Api, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (expected, status, body) in responses {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && start.elapsed() < Duration::from_secs(10) =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("waiting for {expected}: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut buf = [0; 8192];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let header = String::from_utf8_lossy(&request[..header_end]);
                assert!(header.starts_with(expected), "unexpected request: {header}");
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + length {
                    let mut buf = [0; 8192];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (
            Api::new(&BeeperConfig {
                base_url,
                token: "fixture".into(),
            })
            .unwrap(),
            worker,
        )
    }
    fn forward_identity_responses() -> Vec<(&'static str, u16, String)> {
        vec![
            ("GET /v1/chats/source ", 200, r#"{"id":"source","accountID":"telegram"}"#.into()),
            ("GET /v1/chats/target ", 200, r#"{"id":"target","accountID":"signal"}"#.into()),
            ("GET /v1/chats/source/messages/message ", 200, r#"{"id":"message","chatID":"source","accountID":"telegram","text":"hello @somebody"}"#.into()),
        ]
    }
    fn forward_test_dirs() -> AccountDirs {
        AppDirs::under(
            &std::env::temp_dir().join(format!("solmu-forward-fixture-{}", std::process::id())),
        )
        .account(&"1".into())
    }
    #[test]
    fn composed_forward_accepts_pending_id_and_drops_source_mentions() {
        let mut responses = forward_identity_responses();
        responses.push((
            "POST /v1/chats/target/messages ",
            200,
            r#"{"pendingMessageID":"queued","chatID":"target"}"#.into(),
        ));
        let (api, server) = forward_fixture(responses);
        let item = forward_fixture_item(&api.base_url);
        let accepted = api
            .send_forward(&item, &forward_test_dirs(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(accepted.pending_id, "queued");
        let requests = server.join().unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(requests.last().unwrap().split("\r\n\r\n").nth(1).unwrap())
                .unwrap();
        assert_eq!(payload, serde_json::json!({"text":item.text}));
        assert!(payload.get("mentions").is_none());
    }

    #[test]
    fn composed_forward_reuploads_only_the_selected_attachment_with_original_filename() {
        let mut responses = forward_identity_responses();
        let message = r#"{"id":"message","chatID":"source","accountID":"telegram","attachments":[{"id":"mxc://first","fileName":"first.txt","mimeType":"text/plain"},{"id":"mxc://second","fileName":"second.txt","mimeType":"text/plain"}]}"#;
        responses.last_mut().unwrap().2 = message.into();
        responses.extend([
            (
                "GET /v1/chats/source/messages/message ",
                200,
                message.into(),
            ),
            (
                "GET /v1/assets/serve?url=mxc%3A%2F%2Fsecond ",
                200,
                "second attachment".into(),
            ),
            (
                "POST /v1/assets/upload/base64 ",
                200,
                r#"{"uploadID":"uploaded-second"}"#.into(),
            ),
            (
                "POST /v1/chats/target/messages ",
                200,
                r#"{"pendingMessageID":"queued","chatID":"target"}"#.into(),
            ),
        ]);
        let (api, server) = forward_fixture(responses);
        let mut item = forward_fixture_item(&api.base_url);
        item.attachment = Some(1);
        let accepted = api
            .send_forward(&item, &forward_test_dirs(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(accepted.pending_id, "queued");
        let requests = server.join().unwrap();
        let json = |index: usize| {
            serde_json::from_str::<serde_json::Value>(
                requests[index].split("\r\n\r\n").nth(1).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            json(5),
            serde_json::json!({"content":"c2Vjb25kIGF0dGFjaG1lbnQ=","fileName":"second.txt"})
        );
        assert_eq!(
            json(6),
            serde_json::json!({"text":item.text,"attachment":{"uploadID":"uploaded-second","fileName":"second.txt"}})
        );
        let _ = std::fs::remove_dir_all(forward_test_dirs().cache);
    }

    #[test]
    fn composed_forward_post_errors_and_sparse_acceptance_are_not_retryable() {
        for (status, body) in [
            (500, "{}"),
            (200, "{}"),
            (200, r#"{"pendingMessageID":"queued","chatID":"wrong"}"#),
        ] {
            let mut responses = forward_identity_responses();
            responses.push(("POST /v1/chats/target/messages ", status, body.into()));
            let (api, server) = forward_fixture(responses);
            assert!(matches!(
                api.send_forward(
                    &forward_fixture_item(&api.base_url),
                    &forward_test_dirs(),
                    &AtomicBool::new(false)
                ),
                Err(crate::composed_forward::State::Uncertain(_))
            ));
            server.join().unwrap();
        }
    }
    #[test]
    fn composed_forward_source_identity_failure_never_posts() {
        let mut responses = forward_identity_responses();
        responses.last_mut().unwrap().2 =
            r#"{"id":"message","chatID":"other","accountID":"telegram"}"#.into();
        let (api, server) = forward_fixture(responses);
        assert!(matches!(
            api.send_forward(
                &forward_fixture_item(&api.base_url),
                &forward_test_dirs(),
                &AtomicBool::new(false)
            ),
            Err(crate::composed_forward::State::FailedBeforeSend(_))
        ));
        assert_eq!(server.join().unwrap().len(), 3);
    }
    #[test]
    fn composed_forward_refuses_display_placeholders_from_nontext_sources() {
        for kind in ["LOCATION", "POLL", "CALL", "NOTICE", "IMAGE"] {
            let body = serde_json::json!({"id":"message","chatID":"source","accountID":"telegram","type":kind}).to_string();
            let (api, server) =
                forward_fixture(vec![("GET /v1/chats/source/messages/message ", 200, body)]);
            let mut item = forward_fixture_item(&api.base_url);
            item.planned = false;
            let planned = api.plan_forwards(vec![item]);
            assert!(
                matches!(
                    planned[0].state,
                    crate::composed_forward::State::FailedBeforeSend(_)
                ),
                "{kind}"
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn composed_forward_poll_preserves_confirmation_failure_and_progress_feedback() {
        for (status, keep, confirmed, failed) in [
            ("SUCCESS", false, true, false),
            ("FAIL_SEND", false, false, true),
            ("PENDING", true, false, false),
        ] {
            let body = serde_json::json!({"id":"final", "chatID":"target", "accountID":"signal", "isSender":true, "sendStatus":{"status":status}}).to_string();
            let (api, server) =
                forward_fixture(vec![("GET /v1/chats/target/messages/pending ", 200, body)]);
            let events = Mutex::new(Vec::new());
            assert_eq!(
                poll_forward_mark(
                    &api,
                    &"target".into(),
                    "pending",
                    "signal",
                    Duration::ZERO,
                    &|e| events.lock().unwrap().push(e)
                ),
                keep,
                "{status}"
            );
            let events = events.into_inner().unwrap();
            assert_eq!(events.iter().any(|e| matches!(e, Event::BeeperSendConfirmed { account, chat, .. } if account == "signal" && chat == "target")), confirmed, "{status}");
            assert!(events.iter().any(|e| matches!(e, Event::Messages { chat, messages, .. } if chat == "target" && messages.len() == 1)), "{status}: missing chat feedback");
            assert_eq!(events.iter().any(|e| matches!(e, Event::Messages { messages, .. } if messages.iter().any(|m| matches!(m.status, Delivery::Failed)))), failed, "{status}");
            assert_eq!(
                events.iter().any(|e| matches!(e, Event::Info(_))),
                failed,
                "{status}"
            );
            assert_eq!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::BeeperForwardResolved { .. })),
                confirmed || failed,
                "{status}"
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn composed_forward_feedback_rejects_foreign_message_identity() {
        for (chat, account, sender) in [
            ("foreign", "signal", true),
            ("target", "foreign", true),
            ("target", "signal", false),
        ] {
            let body = serde_json::json!({"id":"final", "chatID":chat, "accountID":account, "isSender":sender, "sendStatus":{"status":"SUCCESS"}}).to_string();
            let (api, server) =
                forward_fixture(vec![("GET /v1/chats/target/messages/pending ", 200, body)]);
            let events = Mutex::new(Vec::new());
            assert!(poll_forward_mark(
                &api,
                &"target".into(),
                "pending",
                "signal",
                Duration::ZERO,
                &|e| events.lock().unwrap().push(e)
            ));
            assert!(events.into_inner().unwrap().is_empty());
            server.join().unwrap();
        }
    }

    #[test]
    fn composed_forward_restart_reloads_exact_pending_identity_and_resolves_final_mark() {
        for (account, sender, expected) in [
            ("signal", true, true),
            ("foreign", true, false),
            ("signal", false, false),
        ] {
            let body=serde_json::json!({"id":"final","chatID":"target","accountID":account,"isSender":sender,"sendStatus":{"status":"SUCCESS"}}).to_string();
            let (api, server) =
                forward_fixture(vec![("GET /v1/chats/target/messages/pending ", 200, body)]);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            let mark = crate::composed_forward::ForwardedMessage {
                member: crate::merged::Member {
                    origin: api.base_url.clone(),
                    account: "signal".into(),
                    chat: "target".into(),
                },
                message: "pending".into(),
                pending: true,
            };
            std::fs::write(
                &path,
                serde_json::json!({"beeper":{"forwarded_messages":[mark]}}).to_string(),
            )
            .unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let marks: Vec<crate::composed_forward::ForwardedMessage> =
                serde_json::from_value(saved["beeper"]["forwarded_messages"].clone()).unwrap();
            let restored = crate::composed_forward::unresolved_marks(marks.clone(), &api.base_url);
            assert_eq!(restored.len(), 1);
            assert!(crate::composed_forward::unresolved_marks(marks, "foreign-origin").is_empty());
            let events = Mutex::new(Vec::new());
            assert!(!poll_forward_mark(
                &api,
                &restored[0].member.chat,
                &restored[0].message,
                &restored[0].member.account,
                Duration::from_secs(61),
                &|e| events.lock().unwrap().push(e)
            ));
            assert_eq!(events.into_inner().unwrap().iter().any(|e| matches!(e,Event::BeeperForwardResolved {pending,final_id,..} if pending=="pending" && final_id=="final")),expected);
            server.join().unwrap();
        }
    }

    #[test]
    fn composed_forward_retry_replans_failed_preparation_without_losing_accepted_rows() {
        let (api, server) = forward_fixture(vec![
            ("GET /v1/chats/source/messages/message ", 503, "{}".into()),
            (
                "GET /v1/chats/source/messages/message ",
                200,
                r#"{"id":"message","attachments":[{"id":"mxc://one"},{"id":"mxc://two"}]}"#.into(),
            ),
        ]);
        let mut item = forward_fixture_item(&api.base_url);
        item.planned = false;
        let mut items = api.plan_forwards(vec![item]);
        assert!(!items[0].planned);
        assert!(matches!(
            items[0].state,
            crate::composed_forward::State::FailedBeforeSend(_)
        ));
        let mut accepted = forward_fixture_item(&api.base_url);
        accepted.state = crate::composed_forward::State::Accepted {
            pending_id: "previous".into(),
        };
        items.insert(0, accepted.clone());
        crate::composed_forward::retry_failed(&mut items);
        let expanded = api.plan_forwards(items);
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0], accepted);
        assert_eq!(expanded[1].attachment, Some(0));
        assert_eq!(expanded[2].attachment, Some(1));
        assert_eq!(expanded[2].text.lines().count(), 1);
        assert!(expanded[1].planned && expanded[2].planned);
        server.join().unwrap();
    }

    #[test]
    fn queue_confirmation_requires_explicit_success_not_pending_failure_or_sparse_echo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in [
                None,
                Some("PENDING"),
                Some("FAIL_PERMANENT"),
                Some("SUCCESS"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let mut body = serde_json::json!({"id":"fixture","chatID":"chat","accountID":"work","isSender":true});
                if let Some(status) = status {
                    body["sendStatus"] = serde_json::json!({"status":status});
                }
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        for confirmed in [false, false, false, true] {
            let events = Mutex::new(Vec::new());
            poll_pending_send(
                &api,
                &"chat".into(),
                "fixture",
                Duration::from_secs(1),
                &|e| events.lock().unwrap().push(e),
            );
            assert_eq!(
                events
                    .into_inner()
                    .unwrap()
                    .iter()
                    .any(|e| matches!(e,Event::BeeperSendConfirmed{account,..} if account=="work")),
                confirmed
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn server_network_aliases_share_navigation_without_changing_account() {
        let accounts: Vec<BeeperAccount> = serde_json::from_str(
            r#"[
            {"accountID":"private-facebook","network":"Facebook","bridge":{"type":"facebookgo"}},
            {"accountID":"private-signal","bridge":{"type":"signal"}},
            {"accountID":"private-unknown","bridge":{"type":"new-unknown-protocol"}}
        ]"#,
        )
        .unwrap();
        let inventory = account_inventory(&accounts);
        assert_eq!(inventory[0].network, "Facebook/Messenger");
        assert_eq!(inventory[1].network, "Signal");
        assert_eq!(inventory[2].network, "");
        assert_eq!(inventory[0].id, "private-facebook");
        for alias in [
            "Facebook",
            "Facebook/Messenger",
            "facebook messenger",
            "Messenger",
        ] {
            let chat: BeeperChat = serde_json::from_value(serde_json::json!({
                "id":"original-chat", "accountID":"private-facebook", "network":alias
            }))
            .unwrap();
            let label = network_labeller(&[], &accounts);
            let mapped = map_chat(&chat, &label);
            assert_eq!(
                mapped.backend_network.as_deref(),
                Some(inventory[0].network.as_str())
            );
            assert_eq!(mapped.backend_account.as_deref(), Some("private-facebook"));
            assert_eq!(mapped.id, "original-chat");
        }
        assert_eq!(
            canonical_network("Unknown Custom Network"),
            "Unknown Custom Network"
        );
    }

    #[test]
    fn ordinary_send_omits_optional_reply_instead_of_json_null() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 8192];
            let n = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]);
            let body: serde_json::Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let status = if body.get("replyToMessageID").is_some() {
                400
            } else {
                200
            };
            let response = r#"{"pendingMessageID":"fixture"}"#;
            write!(stream,"HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let result = api.send_text("fixture", "hello", None, &[]);
        server.join().unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn mentions_are_posted_as_user_id_list() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 8192];
            let n = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]);
            let body: serde_json::Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let mentions_ok = body["mentions"]
                == serde_json::json!(["@wl_1:local-signal.localhost", "3585@s.whatsapp.net"]);
            let status = if mentions_ok { 200 } else { 400 };
            let response = r#"{"pendingMessageID":"fixture"}"#;
            write!(stream,"HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let mentions = vec![
            "@wl_1:local-signal.localhost".to_string(),
            "3585@s.whatsapp.net".to_string(),
        ];
        let result = api.send_text("fixture", "hi @m", None, &mentions);
        server.join().unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn chat_participant_names_survive_mapping() {
        let wire: BeeperChat = serde_json::from_str(
            r#"{
                "id": "!room:local", "accountID": "one", "network": "Signal",
                "title": "Family", "type": "group",
                "participants": {"items": [
                    {"id": "@anna:local-signal.localhost", "fullName": "Anna Example"},
                    {"id": "@bob:local-signal.localhost", "username": "bob"},
                    {"id": "@opaque:beeper.local", "fullName": "@opaque:beeper.local"}
                ]}
            }"#,
        )
        .unwrap();
        let chat = map_chat(&wire, &|c| c.network.clone());
        assert_eq!(
            chat.participant_names
                .get("@anna:local-signal.localhost")
                .map(String::as_str),
            Some("Anna Example")
        );
        assert_eq!(
            chat.participant_names
                .get("@bob:local-signal.localhost")
                .map(String::as_str),
            Some("bob")
        );
        // An opaque identity is nobody's name.
        assert!(!chat.participant_names.contains_key("@opaque:beeper.local"));
    }

    #[test]
    fn mention_labels_decode_like_the_body() {
        let raw = "<p>ask <a href=\"https://matrix.to/#/@tom:local-signal.localhost\">Tom &amp; Anna</a> and <a href=\"https://matrix.to/#/@dana:local-signal.localhost\">d&#x27;Angelo</a> \u{1f35e}</p>";
        let mentions = rich_text_mentions(raw);
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].id, "@tom:local-signal.localhost");
        assert_eq!(mentions[0].user, "Tom & Anna");
        assert_eq!(mentions[1].user, "d'Angelo");
        // The body carries the very tokens the labels resolve to.
        let body = plain_rich_text(raw);
        assert!(body.contains("@Tom & Anna"), "{body}");
        assert!(body.contains("@d'Angelo"), "{body}");
    }

    #[test]
    fn matrix_mention_links_keep_the_name_without_the_url() {
        let raw = "<p>hey <a href=\"https://matrix.to/#/@wl_6ds:local-signal.localhost\">Mikael Kasurinen</a> look</p>";
        assert_eq!(plain_rich_text(raw), "hey @Mikael Kasurinen look");
        let mentions = rich_text_mentions(raw);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "@wl_6ds:local-signal.localhost");
        assert_eq!(mentions[0].user, "Mikael Kasurinen");
        // Ordinary links keep their destination beside the label.
        let raw = "<p>see <a href=\"https://example.com/page\">this</a></p>";
        assert_eq!(plain_rich_text(raw), "see this (https://example.com/page)");
    }

    #[test]
    fn inline_html_keeps_link_labels_destinations_and_breaks() {
        assert_eq!(plain_rich_text("Hello<br>next"), "Hello\nnext");
        assert_eq!(
            plain_rich_text(
                r#"Hello <a href="https://example.com/page" rel="noreferrer">Fixture Name</a>!"#
            ),
            "Hello Fixture Name (https://example.com/page)!"
        );
        assert_eq!(
            plain_rich_text(r#"Before <a href="javascript:alert(1)">safe label</a> after"#),
            "Before safe label after"
        );
        assert_eq!(
            plain_rich_text("ordinary < comparison > text"),
            "ordinary < comparison > text"
        );
    }

    /// Bridge-labelled fixture matrix: the normalized-HTML shapes each
    /// network's bridge is expected to deliver through Beeper's `text`
    /// field, plus the raw/plain/Markdown fallbacks seen when a bridge does
    /// no conversion at all. Each expectation is the WhatsApp-style markup
    /// string the renderer (`crate::markup`) turns back into styled spans.
    /// These prove parser behaviour only; only live captures per bridge
    /// prove every bridge really sends these shapes.
    #[test]
    fn rich_text_style_fixture_matrix() {
        let fixtures: &[(&str, &str, &str)] = &[
            // WhatsApp: *bold*, _italic_, ~strike~, `mono` arrive as tags.
            ("whatsapp bold", "<p><strong>bold</strong></p>", "*bold*"),
            ("whatsapp bold b", "<p><b>bold</b></p>", "*bold*"),
            ("whatsapp italics", "<p><em>italics</em></p>", "_italics_"),
            ("whatsapp italics i", "<p><i>italics</i></p>", "_italics_"),
            (
                "whatsapp overstrike",
                "<p><del>overstrike</del></p>",
                "~overstrike~",
            ),
            ("telegram strike s", "<p><s>gone</s></p>", "~gone~"),
            (
                "signal strike strike",
                "<p><strike>gone</strike></p>",
                "~gone~",
            ),
            (
                "inline code",
                "<p>run <code>make test</code> now</p>",
                "run `make test` now",
            ),
            (
                "pre block",
                "<pre>line one\nline two</pre>",
                "```\nline one\nline two\n```",
            ),
            (
                "pre wrapping code",
                "<pre><code>fn main() {}</code></pre>",
                "```\nfn main() {}\n```",
            ),
            ("blockquote", "<blockquote>quoted</blockquote>", "> quoted"),
            (
                "blockquote paragraphs",
                "<blockquote><p>one</p><p>two</p></blockquote>",
                "> one\n> two",
            ),
            (
                "blockquote raw newline",
                "<blockquote>one\ntwo</blockquote>",
                "> one\n> two",
            ),
            (
                "nested bold italic",
                "<p><strong>bold <em>both</em></strong></p>",
                "*bold _both_*",
            ),
            (
                "styled entities",
                "<p><strong>a &amp; b</strong></p>",
                "*a & b*",
            ),
            (
                "bullet list",
                "<ul><li>one</li><li>two</li></ul>",
                "\u{2022} one\n\u{2022} two",
            ),
            (
                "numbered list keeps bullets",
                "<ol><li>one</li><li>two</li></ol>",
                "\u{2022} one\n\u{2022} two",
            ),
            (
                "styled link label",
                r#"<a href="https://example.com/page"><strong>Fixture</strong></a>"#,
                "*Fixture* (https://example.com/page)",
            ),
            // Raw fallbacks: no tags, so the field passes through untouched
            // and the renderer applies WhatsApp rules as it always has.
            (
                "raw whatsapp markers",
                "*bold* _italics_ ~overstrike~",
                "*bold* _italics_ ~overstrike~",
            ),
            ("raw bullet list", "* one\n* two", "* one\n* two"),
            (
                "markdown fallback stays literal",
                "**bold** and __italic__",
                "**bold** and __italic__",
            ),
            (
                "entity-escaped tags stay literal",
                "&lt;del&gt;literal&lt;/del&gt;",
                "<del>literal</del>",
            ),
            // A style that starts or ends mid-word gets a zero-width
            // boundary hint: the renderer's markers only open and close at
            // word boundaries, so a bare `a*b*c` would stay literal.
            (
                "bold mid-word",
                "<p>a<strong>b</strong>c</p>",
                "a\u{200B}*b*\u{200B}c",
            ),
            (
                "bold ending before a digit",
                "<p><strong>x</strong>2</p>",
                "*x*\u{200B}2",
            ),
            // The renderer parses styles per line, so a style spanning a
            // break closes at the line end and reopens on the next line.
            (
                "bold across a break",
                "<p><strong>one<br>two</strong></p>",
                "*one*\n*two*",
            ),
            (
                "bold across a raw newline",
                "<p><strong>one\ntwo</strong></p>",
                "*one*\n*two*",
            ),
            // Code content that carries backticks or fence lines is
            // backslash-escaped so it cannot close its own span or block.
            (
                "inline code with backticks",
                "<p><code>foo`bar`baz</code></p>",
                "`foo\\`bar\\`baz`",
            ),
            (
                "inline code with a backslash",
                "<p><code>a\\b</code></p>",
                "`a\\\\b`",
            ),
            (
                "pre holding a fence line",
                "<pre>first\n```\nsecond</pre>",
                "```\nfirst\n\\```\nsecond\n```",
            ),
            // A native backslash line keeps its bytes: only an exact
            // fence line is escaped.
            (
                "pre holding a backslash line",
                "<pre>\\home\\user</pre>",
                "```\n\\home\\user\n```",
            ),
            // Entities decode before escaping, so an entity-encoded
            // backtick or fence line cannot slip past it.
            (
                "entity backtick in code",
                "<p><code>foo&#96;bar</code></p>",
                "`foo\\`bar`",
            ),
            (
                "entity fence in pre",
                "<pre>a\n&#96;&#96;&#96;\nb</pre>",
                "```\na\n\\```\nb\n```",
            ),
            // The escape sits right before the backticks so the renderer
            // resolves it while the indentation survives.
            (
                "indented fence in pre",
                "<pre>  ```</pre>",
                "```\n  \\```\n```",
            ),
            // A neutral tag between a close and the next word keeps the
            // boundary hint; an empty style never emits a marker pair.
            (
                "neutral span keeps the boundary",
                "<strong>a</strong><span>b</span>",
                "*a*\u{200B}b",
            ),
            (
                "empty styled line stays clean",
                "<strong><br>word</strong>",
                "*word*",
            ),
            (
                "empty style pair vanishes",
                "<p><strong></strong>b</p>",
                "b",
            ),
            // Nested quotes keep the outer bar once the inner quote ends.
            (
                "nested blockquotes keep the outer bar",
                "<blockquote>outer<blockquote>inner</blockquote>tail</blockquote>",
                "> outer\n> inner\n> tail",
            ),
        ];
        for (label, raw, expected) in fixtures {
            assert_eq!(&plain_rich_text(raw), expected, "{label}");
        }
    }

    /// The exact shapes behind the 2026-11-11 WhatsApp self-chat report:
    /// "*bold*", "_italics_", "~overstrike~" and a "* " bullet list
    /// arrived as unstyled plain text, overstrike as literal "<del>" tags.
    #[test]
    fn whatsapp_self_message_styles_survive() {
        let raw = "<p><strong>bold</strong></p><p><em>italics</em></p><p><del>overstrike</del></p><ul><li>one</li><li>two</li></ul>";
        assert_eq!(
            plain_rich_text(raw),
            "*bold*\n_italics_\n~overstrike~\n\u{2022} one\n\u{2022} two"
        );
    }

    /// Styling must never become an injection channel: blocked elements stay
    /// dropped, unsafe URLs stay label-only, event attributes never reach the
    /// output, and the mapper never emits raw HTML of its own.
    #[test]
    fn rich_text_styles_stay_safe() {
        assert_eq!(
            plain_rich_text("<p>a</p><script>alert(1)</script><p><strong>b</strong></p>"),
            "a\n*b*"
        );
        assert_eq!(
            plain_rich_text("<style>* { color: red }</style><p><em>x</em></p>"),
            "_x_"
        );
        assert_eq!(
            plain_rich_text(r#"<a href="https://example.com" onclick="alert(1)">x</a>"#),
            "x (https://example.com)"
        );
        assert_eq!(
            plain_rich_text(r#"<del><a href="javascript:alert(1)">x</a></del>"#),
            "~x~"
        );
        // An unknown tag stays literal text; styles around it still apply.
        assert_eq!(
            plain_rich_text("<strong>a</strong> <foo>bar</foo>"),
            "*a* <foo>bar</foo>"
        );
    }

    /// Malformed and crossing tags degrade to literal marker characters the
    /// renderer leaves unstyled, never to panics or raw HTML.
    #[test]
    fn rich_text_malformed_tags_degrade_safely() {
        assert_eq!(plain_rich_text("<strong>bold"), "*bold");
        // Overlapping styles style by their spans: a is bold, b is bold
        // and italic, c is italic; boundary hints keep the markers parsing.
        assert_eq!(
            plain_rich_text("<b>a<i>b</b>c</i>"),
            "*a\u{200B}_b*\u{200B}c_"
        );
        assert_eq!(plain_rich_text("<strong>ok</strong> <del"), "*ok* <del");
    }

    #[test]
    fn dropped_command_channel_stops_connected_control_worker() {
        let (base_url, _, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (events, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let ended = Arc::new(AtomicBool::new(false));
        let ended2 = ended.clone();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "fixture".into(),
                },
                dirs,
                events,
                rx,
                Waker::default(),
            );
            ended2.store(true, Ordering::Relaxed);
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        drop(tx);
        let start = Instant::now();
        while !ended.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let stopped = ended.load(Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        if stopped {
            worker.join().unwrap();
        }
        assert!(
            stopped,
            "closed command channel must stop background control polling"
        );
    }

    #[test]
    fn history_page_rejects_explicit_other_chat_before_mapping() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for page in [true, false] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let body = if page {
                    r#"{"items":[{"id":"foreign","chatID":"other","text":"wrong conversation"}],"newestCursor":"bad"}"#
                } else {
                    r#"{"id":"foreign","chatID":"other","text":"wrong conversation"}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let result = api.list_messages("requested", None, None);
        assert!(
            result.is_err(),
            "foreign history must not be mapped into requested chat"
        );
        let error = result.err().unwrap();
        assert!(!error.contains("other"));
        assert!(!error.contains("wrong conversation"));
        assert!(api.get_message("requested", "foreign").is_err());
        worker.join().unwrap();
    }

    #[test]
    fn missing_older_cursor_remains_partial_and_never_refetches_latest_as_older() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let done = Arc::new(AtomicBool::new(false));
        let done2 = done.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests2 = requests.clone();
        let server = std::thread::spawn(move || {
            while !done2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request = [0u8; 8192];
                        let n = stream.read(&mut request).unwrap();
                        let request = String::from_utf8_lossy(&request[..n]);
                        let path = request
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap();
                        requests2.lock().unwrap().push(path.to_string());
                        let body = if path == "/v1/accounts" {
                            "[]"
                        } else if path == "/v1/chats"
                            || path.starts_with("/v1/chats/live-watch/messages")
                        {
                            EMPTY_PAGE
                        } else {
                            r#"{"items":[{"id":"kept","timestamp":"2026-10-28T00:00:00Z","text":"keep"}],"hasMore":true}"#
                        };
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("fixture: {e}"),
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "fixture".into(),
                },
                dirs,
                event_tx,
                cmd_rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        // Keep ordinary live polling active, but on a separate conversation.
        // Reads of fixture history below must come only from explicit commands.
        cmd_tx
            .send(Command::WatchBeeperChats(vec!["live-watch".into()]))
            .unwrap();
        cmd_tx
            .send(Command::LoadChat {
                chat: "fixture".into(),
                before: None,
            })
            .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Messages { older: false, .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::BeeperHistoryState { error: Some(_), .. }))
        );
        assert!(
            events.iter().any(
                |e| matches!(e,Event::Messages{complete:false,messages,..} if messages.len()==1)
            )
        );
        cmd_tx
            .send(Command::FetchOlder {
                chat: "fixture".into(),
                explicit: true,
            })
            .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::OlderFetched { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::OlderFetched { more: true, .. }))
        );
        cmd_tx
            .send(Command::LoadChat {
                chat: "fixture".into(),
                before: Some((1, "kept".into())),
            })
            .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Messages { older: true, .. })
        });
        assert!(events.iter().any(|e|matches!(e,Event::Messages{older:true,complete:false,messages,..} if messages.is_empty())));
        // Prove that a background poll really occurred during this test.
        let live_events = collect_until(
            &event_rx,
            Duration::from_secs(5),
            &|e| matches!(e, Event::BeeperLiveState { chat, .. } if chat == "live-watch"),
        );
        assert!(live_events.iter().any(|e| {
            matches!(e, Event::BeeperLiveState { chat, error: None } if chat == "live-watch")
        }));
        cmd_tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        done.store(true, Ordering::Relaxed);
        server.join().unwrap();
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.starts_with("/v1/chats/fixture/messages"))
                .count(),
            1
        );
    }

    #[test]
    fn older_history_cursors_never_claim_completion_when_broken() {
        assert!(history_cursor(None, None, true).is_err());
        assert!(history_cursor(Some(""), None, true).is_err());
        assert!(history_cursor(Some("old"), Some("old"), true).is_err());
        assert_eq!(
            history_cursor(Some("older"), Some("old"), true)
                .unwrap()
                .as_deref(),
            Some("older")
        );
        assert_eq!(
            history_cursor(None, Some("old"), false).unwrap().as_deref(),
            Some("old")
        );
    }

    #[test]
    fn pasted_image_encodes_a_lossless_png_with_alpha() {
        let rgba = vec![10, 20, 30, 128];
        let path = encode_pasted_image(1, 1, &rgba).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(take_temp_upload(&path), "the file is ours to clean");
        assert!(!take_temp_upload(&path), "the claim is spent");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(&bytes[..4], b"\x89PNG");
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 30, 128]);
    }

    #[test]
    fn pasted_image_with_invalid_pixels_is_rejected() {
        let error = encode_pasted_image(2, 2, &[1, 2, 3, 255]).unwrap_err();
        assert_eq!(error, "Clipboard image data is invalid");
    }

    #[test]
    fn plain_image_send_passes_the_unsupported_gate() {
        let command = Command::SendImage {
            chat: "work".into(),
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
            view_once: false,
            caption: Some("caption".into()),
            mentions: vec![],
            quoting: None,
        };
        let command = refuse_unsupported_payload(command).expect("plain image passes");
        assert!(matches!(command, Command::SendImage { .. }));
    }

    #[test]
    fn quoted_image_send_passes_the_unsupported_gate() {
        let command = Command::SendImage {
            chat: "work".into(),
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
            view_once: false,
            caption: None,
            mentions: vec![],
            quoting: Some(crate::model::ReplyTarget {
                id: "source".into(),
                chat: None,
            }),
        };
        let command = refuse_unsupported_payload(command).expect("quoted image passes");
        assert!(matches!(
            command,
            Command::SendImage {
                quoting: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn unsupported_image_send_retains_pixels_caption_and_reply() {
        let reply = crate::model::ReplyTarget {
            id: "source".into(),
            chat: None,
        };
        let command = Command::SendImage {
            chat: "work".into(),
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
            view_once: true,
            caption: Some("keep caption".into()),
            mentions: vec![],
            quoting: Some(reply.clone()),
        };
        let event = refuse_unsupported_payload(command).expect_err("payload not returned");
        let Event::SendRefused {
            chat,
            quoting,
            unsent: super::super::Unsent::Image { rgba, caption, .. },
            ..
        } = *event
        else {
            panic!("payload not returned")
        };
        assert_eq!(chat, "work");
        assert_eq!(quoting, Some(reply));
        assert_eq!(rgba, vec![1, 2, 3, 255]);
        assert_eq!(caption.as_deref(), Some("keep caption"));
    }

    #[test]
    fn scoped_search_filters_chat_and_half_open_day_range() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("chatIDs=work%2Fchat"));
            assert!(request.contains("dateAfter="));
            assert!(request.contains("dateBefore="));
            let body = r#"{"items":[{"id":"before","chatID":"work/chat","timestamp":"2026-10-27T23:59:59Z"},{"id":"start","chatID":"work/chat","timestamp":"2026-10-28T00:00:00Z"},{"id":"end","chatID":"work/chat","timestamp":"2026-10-29T00:00:00Z"},{"id":"other","chatID":"other/chat","timestamp":"2026-10-28T12:00:00Z"}],"hasMore":true}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        let from = jiff::Timestamp::from_str("2026-10-28T00:00:00Z")
            .unwrap()
            .as_second();
        run_chat_search(
            &api,
            "work/chat".into(),
            "fixture".into(),
            Some(from),
            Some(from + 86400),
            &|event| events.lock().unwrap().push(event),
        );
        let events = events.into_inner().unwrap();
        assert!(events.iter().any(|e|matches!(e,Event::ChatHits{messages,truncated:true,..} if messages.len()==1 && messages[0].id=="start")));
        worker.join().unwrap();
    }

    #[test]
    fn load_until_follows_older_cursor_and_emits_target() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("cursor=older"));
            assert!(request.contains("direction=before"));
            let body = r#"{"items":[{"id":"target","chatID":"chat","text":"fixture","timestamp":"2026-06-19T12:00:00Z"}],"hasMore":false}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let mut cursors = HashMap::from([(
            "chat".into(),
            ChatCursors {
                oldest: Some("older".into()),
                newest: None,
                has_more_older: true,
            },
        )]);
        let events = std::sync::Mutex::new(Vec::new());
        load_until_message(&api, &mut cursors, "chat", "target", &|event| {
            events.lock().unwrap().push(event)
        });
        assert!(events.into_inner().unwrap().iter().any(|event| matches!(event, Event::Messages { messages, older: true, .. } if messages.iter().any(|m| m.id=="target"))));
        worker.join().unwrap();
    }

    #[test]
    fn explicit_search_more_keeps_query_and_follows_oldest_cursor() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for second in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                assert!(request.contains("query=fixture"));
                assert_eq!(request.contains("cursor=next"), second);
                let body = if second {
                    r#"{"items":[],"hasMore":false}"#
                } else {
                    r#"{"items":[],"hasMore":true,"oldestCursor":"next"}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let mut cache = CachedSearch::default();
        let first = cache.next(&api, vec![("query", "fixture".into())]).unwrap();
        assert!(first.has_more);
        let result = cache.next(&api, vec![("query", "fixture".into())]).unwrap();
        assert!(!result.has_more);
        assert_eq!(cache.pages, 2);
        // Exhausted results do not issue a third request.
        assert!(
            !cache
                .next(&api, vec![("query", "fixture".into())])
                .unwrap()
                .has_more
        );
        worker.join().unwrap();
    }

    #[test]
    fn search_cache_keeps_results_and_cursor_after_a_failed_page_and_can_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                assert!(request.contains("query=fixture"));
                assert_eq!(request.contains("cursor=next"), index > 0);
                let body = match index {
                    0 => {
                        r#"{"items":[{"id":"one","chatID":"chat","text":"fixture","timestamp":"2026-06-19T12:00:00Z"}],"hasMore":true,"oldestCursor":"next"}"#
                    }
                    1 => "invalid-json",
                    _ => {
                        r#"{"items":[{"id":"one","chatID":"chat","text":"fixture","timestamp":"2026-06-19T12:00:00Z"},{"id":"two","chatID":"chat","text":"fixture","timestamp":"2026-06-19T11:00:00Z"}],"hasMore":false}"#
                    }
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let params = || vec![("query", "fixture".into())];
        let mut cache = CachedSearch::default();
        assert_eq!(cache.next(&api, params()).unwrap().items.len(), 1);
        assert!(cache.next(&api, params()).is_err());
        assert_eq!(cache.pages, 1);
        assert_eq!(cache.page.as_ref().unwrap().items.len(), 1);
        assert_eq!(cache.next(&api, params()).unwrap().items.len(), 2);
        assert_eq!(cache.pages, 2);
        worker.join().unwrap();
    }

    #[test]
    fn cache_limits_and_bad_cursors_do_not_issue_another_request() {
        let api = Api::new(&BeeperConfig {
            base_url: "http://127.0.0.1:9".into(),
            token: "fixture".into(),
        })
        .unwrap();
        let page = || {
            serde_json::from_str::<Page<BeeperMessage>>(
                r#"{"items":[],"hasMore":true,"oldestCursor":"next"}"#,
            )
            .unwrap()
        };
        let params = || vec![("query", "fixture".into())];
        let mut cache = CachedSearch {
            page: Some(page()),
            pages: 50,
            seen: HashSet::new(),
        };
        assert!(cache.next(&api, params()).unwrap().has_more);
        assert_eq!(cache.pages, 50);
        cache.pages = 1;
        cache.seen.insert("next".into());
        assert!(
            cache
                .next(&api, params())
                .unwrap_err()
                .contains("did not advance")
        );
        cache.seen.clear();
        cache.page.as_mut().unwrap().oldest_cursor = None;
        assert!(
            cache
                .next(&api, params())
                .unwrap_err()
                .contains("unavailable")
        );
        assert_eq!(cache.pages, 1);
    }

    #[test]
    fn first_search_page_reports_partial_results() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#"{"items":[],"hasMore":true}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        run_message_search(&api, "fixture".into(), &|event| {
            events.lock().unwrap().push(event)
        });
        assert!(events.into_inner().unwrap().iter().any(|event| matches!(
            event,
            Event::BeeperSearchState {
                error: None,
                truncated: true,
                ..
            }
        )));
        worker.join().unwrap();
    }

    #[test]
    fn failed_search_returns_scoped_error_not_a_silent_empty_result() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            write!(stream,"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        run_message_search(&api, "fixture query".into(), &|event| {
            events.lock().unwrap().push(event)
        });
        assert!(events.into_inner().unwrap().iter().any(|event|matches!(event,Event::BeeperSearchState{query,error:Some(_),..} if query=="fixture query")));
        worker.join().unwrap();
    }

    #[test]
    fn every_control_worker_exit_sets_upload_stop_flag() {
        let stopped = Arc::new(AtomicBool::new(false));
        {
            let _guard = StopUploads(stopped.clone());
            assert!(!upload_stopped(Some(&stopped)));
        }
        assert!(upload_stopped(Some(&stopped)));
    }

    #[test]
    fn upload_cancelled_after_asset_response_never_submits_message() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request);
            stopped.store(true, Ordering::SeqCst);
            let body = r#"{"uploadID":"fixture-upload"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            drop(stream);
            listener.set_nonblocking(true).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                listener.accept().is_err(),
                "message submission started after cancellation"
            );
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"fixture").unwrap();
        assert!(
            api.send_file_cancellable("chat", file.path(), true, None, Some(&cancel))
                .unwrap_err()
                .contains("stopped")
        );
        worker.join().unwrap();
    }

    fn capture_voice_send(reply: Option<&str>) -> (Result<SendAccepted, String>, String) {
        capture_typed_send(reply, Attachment::Voice(7))
    }

    fn capture_typed_send(
        reply: Option<&str>,
        kind: Attachment,
    ) -> (Result<SendAccepted, String>, String) {
        let (result, requests) = capture_typed_send_both(reply, kind);
        let last = requests
            .split("\n--NEXT--\n")
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or_default()
            .to_owned();
        (result, last)
    }

    fn capture_typed_send_both(
        reply: Option<&str>,
        kind: Attachment,
    ) -> (Result<SendAccepted, String>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut sent = String::new();
            for body in [
                r#"{"uploadID":"up1"}"#,
                r#"{"pendingMessageID":"p1","chatID":"chat"}"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                // Read until the declared body has arrived: headers and body
                // can come in separate segments.
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let want = head
                        .to_ascii_lowercase()
                        .split("content-length:")
                        .nth(1)
                        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
                        .unwrap_or(0usize);
                    if body.len() >= want {
                        break;
                    }
                }
                sent.push_str(&String::from_utf8_lossy(&request));
                sent.push_str("\n--NEXT--\n");
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            sent
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"ogg").unwrap();
        let result = api.send_attachment("chat", file.path(), kind, None, reply, None);
        (result, worker.join().unwrap())
    }

    #[test]
    fn linked_typed_media_uses_native_type_and_retains_reply() {
        for (kind, expected) in [(Attachment::Gif, "gif"), (Attachment::Sticker, "sticker")] {
            let (result, request) = capture_typed_send(Some("source-message"), kind);
            assert_eq!(result.unwrap().chat, "chat");
            assert!(request.starts_with("POST /v1/chats/chat/messages "));
            let value: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(value["attachment"]["type"], expected);
            assert_eq!(value["replyToMessageID"], "source-message");
            assert!(value.get("text").is_none());
        }
    }

    #[test]
    fn image_send_uploads_the_file_name_and_caption_without_a_type_override() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut sent = String::new();
            for body in [
                r#"{"uploadID":"up1"}"#,
                r#"{"pendingMessageID":"p1","chatID":"chat"}"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let want = head
                        .to_ascii_lowercase()
                        .split("content-length:")
                        .nth(1)
                        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
                        .unwrap_or(0usize);
                    if body.len() >= want {
                        break;
                    }
                }
                sent = String::from_utf8_lossy(&request).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            sent
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("pasted-image-1.png");
        std::fs::write(&image, b"jpeg").unwrap();
        let result = api
            .send_attachment(
                "chat",
                &image,
                Attachment::Auto,
                Some("a caption"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.pending_id, "p1");
        let request = worker.join().unwrap();
        // Beeper picks the type from the uploaded file; the client sends no
        // override, and the caption is the message's text.
        assert!(
            request.contains(r#""fileName":"pasted-image-1.png""#),
            "{request}"
        );
        assert!(!request.contains(r#""type":"#), "{request}");
        assert!(request.contains(r#""text":"a caption""#), "{request}");
    }

    #[test]
    fn voice_send_declares_a_voice_note_with_its_duration() {
        let (result, request) = capture_voice_send(None);
        assert_eq!(result.unwrap().pending_id, "p1");
        assert!(request.contains(r#""type":"voice-note""#), "{request}");
        assert!(request.contains(r#""duration":7"#), "{request}");
        assert!(!request.contains("replyToMessageID"), "{request}");
    }

    #[test]
    fn recorded_voice_declares_opus_mime_on_upload_and_message() {
        let (result, requests) = capture_typed_send_both(None, Attachment::Voice(7));
        assert!(result.is_ok());
        let requests: Vec<_> = requests
            .split("\n--NEXT--\n")
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(requests.len(), 2);
        let upload: serde_json::Value =
            serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        let send: serde_json::Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(upload["mimeType"], "audio/ogg; codecs=opus");
        assert_eq!(send["attachment"]["mimeType"], "audio/ogg; codecs=opus");
        assert_eq!(send["attachment"]["type"], "voice-note");
        assert_eq!(send["attachment"]["duration"], 7);
    }

    #[test]
    fn ordinary_audio_upload_keeps_auto_detected_mime() {
        let (result, requests) = capture_typed_send_both(None, Attachment::Auto);
        assert!(result.is_ok());
        for request in requests.split("\n--NEXT--\n").filter(|s| !s.is_empty()) {
            let body: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert!(body.get("mimeType").is_none());
            assert!(
                body.get("attachment")
                    .is_none_or(|a| a.get("mimeType").is_none() && a.get("type").is_none())
            );
        }
    }

    #[test]
    fn voice_reply_carries_the_quoted_message_id() {
        let (result, request) = capture_voice_send(Some("msg-42"));
        assert!(result.is_ok());
        assert!(
            request.contains(r#""replyToMessageID":"msg-42""#),
            "{request}"
        );
    }

    #[test]
    fn a_partial_upload_does_not_requote_the_unsent_suffix() {
        let quote = Some(crate::model::ReplyTarget {
            id: "source".into(),
            chat: None,
        });
        assert_eq!(first_unsent(&quote, 0), quote);
        assert_eq!(first_unsent(&quote, 1), None);
        assert_eq!(first_unsent(&None::<crate::model::ReplyTarget>, 0), None);
    }

    #[test]
    fn a_quoted_attachment_carries_the_file_and_the_reply_together() {
        let (result, request) = capture_typed_send(Some("msg-42"), Attachment::Document);
        assert!(result.is_ok());
        let value: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(value["attachment"]["type"], "file");
        assert_eq!(value["attachment"]["uploadID"], "up1");
        assert_eq!(value["replyToMessageID"], "msg-42");
    }

    #[test]
    fn only_a_sent_recording_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let recording = dir.path().join("voice-note-1.ogg");
        let document = dir.path().join("report.pdf");
        std::fs::write(&recording, b"ogg").unwrap();
        std::fs::write(&document, b"pdf").unwrap();
        discard_sent_recording(false, &document);
        assert!(document.exists());
        discard_sent_recording(true, &recording);
        assert!(!recording.exists());
    }

    #[test]
    fn failed_voice_upload_keeps_the_recording() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"ogg").unwrap();
        assert!(
            api.send_attachment("chat", file.path(), Attachment::Voice(3), None, None, None)
                .is_err()
        );
        assert!(file.path().exists());
        worker.join().unwrap();
    }

    #[test]
    fn attachment_reader_bounds_bytes_after_open() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"12345").unwrap();
        assert!(
            read_attachment_bounded(file.path(), 4)
                .unwrap_err()
                .contains("limit")
        );
        assert_eq!(read_attachment_bounded(file.path(), 5).unwrap(), b"12345");
    }

    #[test]
    fn failed_account_refresh_is_not_masked_by_successful_chat_refresh() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for (status, body) in [(503, "{}"), (200, r#"{"items":[],"hasMore":false}"#)] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                write!(stream,"HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let mut accounts: Vec<BeeperAccount> = serde_json::from_str(ACCOUNTS_JSON).unwrap();
        let old = accounts.len();
        let events = std::sync::Mutex::new(Vec::new());
        refresh_inventory(&api, &mut accounts, &|event| {
            events.lock().unwrap().push(event)
        });
        assert_eq!(accounts.len(), old);
        let events = events.into_inner().unwrap();
        assert!(events.iter().any(|e| matches!(e, Event::Chats(_))));
        assert!(events.iter().any(|e|matches!(e,Event::Link(LinkStatus::Disconnected{reason}) if reason.contains("account inventory"))));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Link(LinkStatus::Connected)))
        );
        worker.join().unwrap();
    }

    #[test]
    fn pending_lookup_accepts_final_id_but_not_foreign_or_unconfirmed_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (chat, sender, status) in [
                ("chat", true, Some("SUCCESS")),
                ("foreign", true, Some("SUCCESS")),
                ("chat", false, Some("SUCCESS")),
                ("chat", true, Some("PENDING")),
                ("chat", true, None),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let length = stream.read(&mut request).unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..length])
                        .starts_with("GET /v1/chats/chat/messages/pending-fixture ")
                );
                let mut body = serde_json::json!({"id":"final-fixture", "chatID":chat,
                    "accountID":"work", "isSender":sender});
                if let Some(status) = status {
                    body["sendStatus"] = serde_json::json!({"status":status});
                }
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        // A resolved final id with no sendStatus (last case) ends polling
        // quietly: no confirmation event and no "check history" toast.
        for (success, toast) in [
            (true, false),
            (false, true),
            (false, true),
            (false, true),
            (false, false),
        ] {
            let events = Mutex::new(Vec::new());
            assert!(!poll_pending_send(
                &api,
                &"chat".into(),
                "pending-fixture",
                Duration::from_secs(61),
                &|event| events.lock().unwrap().push(event)
            ));
            let events = events.into_inner().unwrap();
            assert_eq!(events.iter().any(|event| matches!(event, Event::BeeperSendConfirmed { account, chat, .. } if account == "work" && chat == "chat")), success);
            assert_eq!(
                events.iter().any(
                    |event| matches!(event, Event::Info(text) if text.contains("check history before retrying"))
                ),
                toast
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn resolved_final_id_without_send_status_stops_polling_without_a_toast() {
        for (status, expect_done) in [(None, true), (Some("PENDING"), false)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let mut body = serde_json::json!({"id":"final-event","chatID":"chat","accountID":"wa","isSender":true});
                if let Some(status) = status {
                    body["sendStatus"] = serde_json::json!({"status":status});
                }
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            });
            let api = Api::new(&BeeperConfig {
                base_url,
                token: "fixture".into(),
            })
            .unwrap();
            let events = Mutex::new(Vec::new());
            let keep = poll_pending_send(
                &api,
                &"chat".into(),
                "pending-id",
                Duration::from_secs(61),
                &|e| events.lock().unwrap().push(e),
            );
            let events = events.into_inner().unwrap();
            assert!(!keep, "past the deadline polling always ends");
            let toasted = events
                .iter()
                .any(|e| matches!(e, Event::Info(text) if text.contains("60 seconds")));
            assert_eq!(toasted, !expect_done);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Event::BeeperSendConfirmed { .. }))
            );
            worker.join().unwrap();
        }
    }

    #[test]
    fn still_pending_confirmation_at_deadline_warns_without_claiming_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body =
                r#"{"id":"pending-fixture","isSender":true,"sendStatus":{"status":"PENDING"}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        assert!(!poll_pending_send(
            &api,
            &"fixture-chat".into(),
            "pending-fixture",
            Duration::from_secs(61),
            &|event| events.lock().unwrap().push(event)
        ));
        assert!(events.into_inner().unwrap().iter().any(|event|matches!(event,Event::Info(text) if text.contains("check history before retrying"))));
        worker.join().unwrap();
    }

    #[test]
    fn invalid_live_cursor_keeps_last_good_cursor_and_reports_gap() {
        for next in [None, Some("seed")] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let worker = std::thread::spawn(move || {
                // Permit a second request so a bad implementation fails assertions,
                // rather than waiting on a connection that will never be served.
                listener.set_nonblocking(true).unwrap();
                let until = Instant::now() + Duration::from_millis(250);
                while Instant::now() < until {
                    if let Ok((mut stream, _)) = listener.accept() {
                        let mut request = [0u8; 4096];
                        let _ = stream.read(&mut request);
                        let body=serde_json::json!({"items":[{"id":"arrived","text":"fixture","isSender":false}],"hasMore":true,"newestCursor":next}).to_string();
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    } else {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            });
            let api = Api::new(&BeeperConfig {
                base_url,
                token: "fixture".into(),
            })
            .unwrap();
            let chat = "fixture-chat".to_string();
            let mut cursors = HashMap::new();
            cursors.insert(
                chat.clone(),
                ChatCursors {
                    oldest: None,
                    newest: Some("seed".into()),
                    has_more_older: false,
                },
            );
            let events = std::sync::Mutex::new(Vec::new());
            poll_newer(&api, &chat, &mut cursors, &|event| {
                events.lock().unwrap().push(event)
            });
            assert_eq!(cursors[&chat].newest.as_deref(), Some("seed"));
            let events = events.into_inner().unwrap();
            assert!(events.iter().any(|event|matches!(event,Event::Messages{messages,..} if messages.iter().any(|m|m.id=="arrived"))));
            assert!(
                matches!(events.last(),Some(Event::BeeperLiveState{error:Some(text),..}) if text.contains("newer messages may be missing"))
            );
            worker.join().unwrap();
        }
    }

    #[test]
    fn live_poll_yields_after_32_advancing_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..32 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let body =
                    format!(r#"{{"items":[],"hasMore":true,"newestCursor":"next-{index}"}}"#);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let chat = "fixture-chat".to_string();
        let mut cursors = HashMap::new();
        cursors.insert(
            chat.clone(),
            ChatCursors {
                oldest: None,
                newest: Some("seed".into()),
                has_more_older: false,
            },
        );
        let events = std::sync::Mutex::new(Vec::new());
        poll_newer(&api, &chat, &mut cursors, &|event| {
            events.lock().unwrap().push(event)
        });
        assert_eq!(cursors[&chat].newest.as_deref(), Some("next-31"));
        assert!(
            matches!(events.into_inner().unwrap().last(),Some(Event::BeeperLiveState{error:Some(text),..}) if text.contains("catching up"))
        );
        worker.join().unwrap();
    }

    #[test]
    fn background_history_failure_is_visible_and_recovery_clears_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for status in [503, 200] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let body = r#"{"items":[],"hasMore":false}"#;
                write!(stream,"HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let mut cursors = HashMap::new();
        let events = std::sync::Mutex::new(Vec::new());
        let emit = |event| events.lock().unwrap().push(event);
        poll_newer(&api, &"fixture-chat".into(), &mut cursors, &emit);
        poll_newer(&api, &"fixture-chat".into(), &mut cursors, &emit);
        let events = events.into_inner().unwrap();
        assert!(
            matches!(&events[0],Event::BeeperLiveState{chat,error:Some(_)} if chat=="fixture-chat")
        );
        assert!(
            matches!(&events[1],Event::BeeperLiveState{chat,error:None} if chat=="fixture-chat")
        );
        worker.join().unwrap();
    }

    #[test]
    fn startup_outage_recovers_without_losing_or_replaying_queued_text() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let healthy = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let requests: Requests = Arc::new(Mutex::new(Vec::new()));
        let (h, done, log) = (healthy.clone(), stop.clone(), requests.clone());
        let server = std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut bytes = [0u8; 8192];
                    let n = stream.read(&mut bytes).unwrap_or(0);
                    let request = String::from_utf8_lossy(&bytes[..n]);
                    let mut parts = request.split_whitespace();
                    let method = parts.next().unwrap_or("").to_owned();
                    let path = parts.next().unwrap_or("").to_owned();
                    log.lock()
                        .unwrap()
                        .push((method, path.clone(), String::new()));
                    let ok = h.load(Ordering::Relaxed);
                    let body = if !ok {
                        "{}".to_string()
                    } else if path == "/v1/accounts" {
                        ACCOUNTS_JSON.to_string()
                    } else if path.contains("/messages") {
                        HISTORY_JSON.replace("!fam:beeper.local", "exact-work")
                    } else {
                        CHATS_JSON.to_string()
                    };
                    let status = if ok { "200 OK" } else { "503 Unavailable" };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(root.path()).as_account();
        let (events, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "fixture".into(),
                },
                dirs,
                events,
                rx,
                Waker::default(),
            )
        });
        let disconnected = collect_until(&event_rx, Duration::from_secs(3), &|e| {
            matches!(e, Event::Link(LinkStatus::Disconnected { .. }))
        });
        tx.send(Command::SendText {
            chat: "exact-work".into(),
            text: "retain this".into(),
            quoting: None,
            mentions: vec![],
        })
        .unwrap();
        let refused = collect_until(&event_rx, Duration::from_secs(2), &|e| {
            matches!(e, Event::BeeperSendUncertain { .. })
        });
        tx.send(Command::EditText {
            chat: "exact-work".into(),
            id: "edit-source".into(),
            text: "retain edit".into(),
            mentions: vec![],
        })
        .unwrap();
        let edit = collect_until(&event_rx, Duration::from_secs(2), &|e| {
            matches!(e, Event::BeeperEditUncertain { .. })
        });
        tx.send(Command::SendFiles {
            chat: "exact-work".into(),
            paths: vec![std::path::PathBuf::from("fixture-kept.txt")],
            documents: true,
            view_once: false,
            caption: Some("retain caption".into()),
            mentions: vec![],
            quoting: None,
        })
        .unwrap();
        let files = collect_until(&event_rx, Duration::from_secs(2), &|e| {
            matches!(e, Event::BeeperFilesUncertain { .. })
        });
        healthy.store(true, Ordering::Relaxed);
        let recovered = collect_until(&event_rx, Duration::from_secs(8), &|e| {
            matches!(e, Event::Chats(_))
        });
        healthy.store(false, Ordering::Relaxed);
        tx.send(Command::SendText {
            chat: "exact-work".into(),
            text: "do not replay after drop".into(),
            quoting: None,
            mentions: vec![],
        })
        .unwrap();
        let dropped = collect_until(&event_rx, Duration::from_secs(3), &|e| {
            matches!(e, Event::BeeperSendUncertain { .. })
        });
        healthy.store(true, Ordering::Relaxed);
        tx.send(Command::LoadChat {
            chat: "exact-work".into(),
            before: None,
        })
        .unwrap();
        let resumed = collect_until(
            &event_rx,
            Duration::from_secs(3),
            &|e| matches!(e,Event::Messages {messages,..} if !messages.is_empty()),
        );
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        assert!(edit.iter().any(|e| matches!(e,Event::BeeperEditUncertain {chat,id,text,..} if chat=="exact-work" && id=="edit-source" && text=="retain edit")));
        assert!(files.iter().any(|e| matches!(e,Event::BeeperFilesUncertain {chat,paths,documents,caption,..} if chat=="exact-work" && paths==&vec![std::path::PathBuf::from("fixture-kept.txt")] && *documents && caption.as_deref()==Some("retain caption"))));
        assert!(dropped.iter().any(
            |e| matches!(e,Event::BeeperSendUncertain {text,..} if text=="do not replay after drop")
        ));
        assert!(
            resumed
                .iter()
                .any(|e| matches!(e,Event::Messages {messages,..} if !messages.is_empty()))
        );
        assert!(
            disconnected
                .iter()
                .any(|e| matches!(e, Event::Link(LinkStatus::Disconnected { .. })))
        );
        assert!(refused.iter().any(|e| matches!(e,Event::BeeperSendUncertain {chat,text,..} if chat=="exact-work" && text=="retain this")),"startup must return unsent text, not silently consume it");
        assert!(
            recovered
                .iter()
                .any(|e| matches!(e,Event::Chats(chats) if !chats.is_empty()))
        );
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _, _)| method == "POST")
                .count(),
            1,
            "only deliberate post-connect attempt, no startup or recovery replay"
        );
    }

    #[test]
    fn decode_errors_never_echo_response_values_or_private_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#""secret-response-value""#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let error = api
            .get::<Vec<BeeperAccount>>("/private-account-path", &[])
            .unwrap_err();
        assert!(!error.contains("secret-response-value"));
        assert!(!error.contains("private-account-path"));
        server.join().unwrap();
    }

    #[test]
    fn account_inventory_carries_own_user_ids() {
        let accounts: Vec<BeeperAccount> = serde_json::from_str(
            r#"[{"accountID":"work","network":"WhatsApp","user":{"id":"@me:beeper.local","fullName":"Me"}},{"accountID":"empty"}]"#,
        )
        .unwrap();
        let inventory = account_inventory(&accounts);
        assert_eq!(
            inventory[0].own_user_id.as_deref(),
            Some("@me:beeper.local")
        );
        assert_eq!(inventory[1].own_user_id, None);
    }

    #[test]
    fn not_found_copy_marks_404_terminal_and_keeps_text() {
        assert!(not_found("Beeper API GET returned 404 Not Found"));
        assert!(not_found("Beeper API POST returned 404 Not Found"));
        assert!(not_found("Beeper change refused: 404 Not Found"));
        assert!(!not_found(
            "Beeper API GET returned 500 Internal Server Error"
        ));
        assert!(!not_found("Beeper read failed: connection refused"));
        let (live, terminal) = live_error_copy("Beeper API GET returned 404 Not Found");
        assert!(terminal);
        assert!(live.contains("does not exist"));
        let (_, terminal) = live_error_copy("Beeper API GET returned 500 Internal Server Error");
        assert!(!terminal);
        assert!(
            history_error_copy("Beeper API GET returned 404 Not Found").contains("does not exist")
        );
        assert!(
            history_error_copy("Beeper API GET returned 500 Internal Server Error")
                .contains("Beeper history failed")
        );
        assert!(send_error_copy("Beeper send returned 404 Not Found").contains("was not sent"));
        assert!(
            send_error_copy("Beeper API POST returned 502 Bad Gateway").contains("check history")
        );
    }

    #[test]
    fn account_inventory_includes_chatless_accounts_without_guessing_network() {
        let accounts: Vec<BeeperAccount> = serde_json::from_str(r#"[{"accountID":"empty-one","network":"Signal","status":"disconnected","user":{"id":"own","fullName":"Work"}},{"accountID":"unknown"}]"#).unwrap();
        let inventory = account_inventory(&accounts);
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0].id, "empty-one");
        assert_eq!(inventory[0].network, "Signal");
        assert_eq!(inventory[0].status, "disconnected");
        assert_eq!(inventory[1].network, "");
        assert_eq!(inventory[1].name, "unknown");
        assert_eq!(inventory[1].status, "Not supplied");
    }

    #[test]
    fn rich_html_stays_safe_and_opaque_names_are_not_presented_as_names() {
        // Styles survive as renderer markup; scripts and raw HTML never do.
        assert_eq!(
            plain_rich_text("<p>Hi &amp; welcome</p><br><p>Next <b>bold</b></p>"),
            "Hi & welcome\nNext *bold*"
        );
        assert_eq!(
            plain_rich_text(
                "<p>Safe<script>secret()</script><img src='https://invalid/image'> end</p>"
            ),
            "Safe end"
        );
        assert_eq!(plain_rich_text("a < b && c > d"), "a < b && c > d");
        assert_eq!(
            plain_rich_text("<p>&lt;script&gt;not executed&lt;/script&gt; &#x1f600;</p>"),
            "<script>not executed</script> 😀"
        );
        let wire:BeeperChat=serde_json::from_str(r#"{"id":"!room:local","accountID":"one","network":"LinkedIn","title":"@opaque:beeper.local"}"#).unwrap();
        assert_eq!(
            map_chat(&wire, &|c| c.network.clone()).name,
            "LinkedIn conversation (name unavailable)"
        );
        assert!(readable_name(&Some("@opaque:beeper.local".into())).is_none());
        assert_eq!(
            readable_name(&Some("Sarah W.".into())).as_deref(),
            Some("Sarah W.")
        );
    }

    #[test]
    fn avatar_sources_refuse_external_hosts_and_use_authenticated_assets_with_limits() {
        assert!(avatar_source("https://example.org/profile.png").is_err());
        assert!(avatar_source("data:image/svg+xml,<svg>bad</svg>").is_err());
        // Use a native absolute path: /tmp/... has no drive on Windows.
        let local_path = std::env::temp_dir().join("profile image.png");
        let expected = reqwest::Url::from_file_path(&local_path).unwrap();
        assert!(expected.as_str().contains("profile%20image.png"));
        assert_eq!(
            avatar_source(local_path.to_str().unwrap()).unwrap(),
            expected.as_str()
        );
        assert!(avatar_source("relative/profile image.png").is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0; 4096];
            let n = stream.read(&mut req).unwrap();
            let req = String::from_utf8_lossy(&req[..n]);
            assert!(req.starts_with("GET /v1/assets/serve?url=mxc%3A%2F%2Ffixture%2Favatar "));
            assert!(
                req.to_ascii_lowercase()
                    .contains("authorization: bearer local-fixture")
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                png.len()
            )
            .unwrap();
            stream.write_all(&png).unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "local-fixture".into(),
        })
        .unwrap();
        api.avatar_sources
            .lock()
            .unwrap()
            .insert("chat".into(), "mxc://fixture/avatar".into());
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(root.path()).as_account();
        let path = api.download_avatar("chat", &dirs).unwrap();
        assert!(path.is_file());
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("beeper-avatar-")
        );
        assert_eq!(api.download_avatar("chat", &dirs).unwrap(), path);
        server.join().unwrap();
        assert!(measure_cache(&dirs).is_ok());
        api.avatar_sources
            .lock()
            .unwrap()
            .insert("remote".into(), "https://example.org/private".into());
        assert!(api.download_avatar("remote", &dirs).is_err());
        assert!(api.download_avatar("missing", &dirs).is_err());
    }

    const ACCOUNTS_JSON: &str = r#"[
      {"accountID":"local-whatsapp*ba*111","bridge":{"id":"whatsapp","provider":"local","type":"whatsapp"},"status":"connected","user":{"id":"@me:beeper.local","fullName":"Personal"}},
      {"accountID":"local-whatsapp*ba*222","bridge":{"id":"whatsapp","provider":"local","type":"whatsapp"},"status":"connected","user":{"id":"@me-work:beeper.local","fullName":"Work"}},
      {"accountID":"slackgo.T01-U01","bridge":{"id":"slackgo","provider":"cloud","type":"slack"},"status":"connected","user":{"id":"@demo:slack","fullName":"Demo user"}}
    ]"#;

    const CHATS_JSON: &str = r#"{"items":[
      {"id":"!fam:beeper.local","accountID":"local-whatsapp*ba*111","network":"WhatsApp","title":"Family","type":"group","unreadCount":3,"lastActivity":"2026-10-27T14:02:11.000Z"},
      {"id":"!alice:beeper.local","accountID":"local-whatsapp*ba*222","network":"WhatsApp","title":"Alice (work)","type":"single","unreadCount":0,"lastActivity":"2026-10-26T09:40:00.000Z"},
      {"id":"!standup:beeper.local","accountID":"slackgo.T01-U01","network":"Slack","title":"Eng standup","type":"group","unreadCount":12,"lastActivity":"2026-10-27T08:15:00.000Z"}
    ],"hasMore":false,"oldestCursor":null,"newestCursor":null}"#;

    const HISTORY_JSON: &str = r#"{"items":[
      {"id":"m1","chatID":"!fam:beeper.local","senderID":"@bob:beeper.local","senderName":"Bob","sortKey":"01","timestamp":"2026-10-27T13:58:02.000Z","text":"hello","type":"TEXT","isSender":false},
      {"id":"m2","chatID":"!fam:beeper.local","senderID":"@me:beeper.local","sortKey":"02","timestamp":"2026-10-27T13:59:44.000Z","text":"hi","type":"TEXT","isSender":true},
      {"id":"m3","chatID":"!fam:beeper.local","senderID":"@alice:beeper.local","senderName":"Alice","sortKey":"03","timestamp":"2026-10-27T14:00:10.000Z","type":"IMAGE","isSender":false,"attachments":[{"type":"img","fileName":"cake.jpg","mimeType":"image/jpeg"}]}
    ],"hasMore":true,"oldestCursor":"c-oldest","newestCursor":"c-newest"}"#;

    const OLDER_JSON: &str = r#"{"items":[
      {"id":"m0","chatID":"!fam:beeper.local","senderID":"@alice:beeper.local","senderName":"Alice","sortKey":"00","timestamp":"2026-10-20T18:00:00.000Z","text":"old news","type":"TEXT","isSender":false}
    ],"hasMore":false,"oldestCursor":null,"newestCursor":"c-older"}"#;

    const EMPTY_PAGE: &str =
        r#"{"items":[],"hasMore":false,"oldestCursor":null,"newestCursor":null}"#;

    type Requests = Arc<Mutex<Vec<(String, String, String)>>>;

    fn start_stub() -> (String, Requests, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let requests: Requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (reqs, stop2) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            loop {
                if stop2.load(Ordering::Relaxed) {
                    return;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 8192];
                        let (mut headers_done, mut body_len) = (false, 0usize);
                        loop {
                            match stream.read(&mut chunk) {
                                Ok(0) => break,
                                Ok(n) => {
                                    buf.extend_from_slice(&chunk[..n]);
                                    if !headers_done
                                        && let Some(pos) =
                                            buf.windows(4).position(|w| w == b"\r\n\r\n")
                                    {
                                        headers_done = true;
                                        let head = String::from_utf8_lossy(&buf[..pos]);
                                        body_len = head
                                            .lines()
                                            .find(|l| {
                                                l.to_ascii_lowercase()
                                                    .starts_with("content-length:")
                                            })
                                            .and_then(|l| l[15..].trim().parse().ok())
                                            .unwrap_or(0);
                                    }
                                    if headers_done {
                                        let head_end =
                                            buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap()
                                                + 4;
                                        if buf.len() >= head_end + body_len {
                                            break;
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let text = String::from_utf8_lossy(&buf).to_string();
                        let mut lines = text.lines();
                        let request_line = lines.next().unwrap_or("");
                        let mut parts = request_line.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let path = parts.next().unwrap_or("").to_string();
                        // A preconnected socket can time out before any HTTP request.
                        // It must not become a fake mutation in fixture assertions.
                        if method.is_empty() || path.is_empty() {
                            continue;
                        }
                        let body_start = text.find("\r\n\r\n").map(|p| p + 4).unwrap_or(0);
                        let body = text[body_start..].to_string();
                        reqs.lock()
                            .unwrap()
                            .push((method.clone(), path.clone(), body));
                        let (base, query) = path.split_once('?').unwrap_or((path.as_str(), ""));
                        if base == "/v1/chats/fail-history/messages" {
                            let _ = stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            continue;
                        }
                        let payload = match (method.as_str(), base) {
                        ("GET", "/v1/accounts") => ACCOUNTS_JSON.to_string(),
                        ("GET", "/v1/chats") => CHATS_JSON.to_string(),
                        ("GET", "/v1/messages/search") => EMPTY_PAGE.to_string(),
                        ("GET", p) if p.ends_with("/messages") => {
                            if query.contains("direction=before") {
                                OLDER_JSON.to_string()
                            } else {
                                HISTORY_JSON.to_string()
                            }
                        }
                        ("GET", p) if p.contains("/messages/") => r#"{"id":"m/4","chatID":"work/chat","accountID":"local-whatsapp*ba*222","text":"updated fixture","isSender":true,"editedTimestamp":"2026-10-27T17:00:00Z","reactions":[{"participantID":"@me-work:beeper.local","reactionKey":"👍","emoji":true}]}"#.to_string(),
                        ("GET", p) if p.starts_with("/v1/chats/") => r#"{"id":"work/chat","accountID":"local-whatsapp*ba*222","network":"WhatsApp","title":"Work fixture"}"#.to_string(),
                        ("POST", p) if p.ends_with("/messages") => r#"{"pendingMessageID":"pending-test"}"#.to_string(),
                        ("PATCH", p) if p.starts_with("/v1/chats/") => {
                            r#"{"id":"work/chat","isMuted":true,"isArchived":true,"isPinned":false}"#
                                .to_string()
                        }
                        _ => "{}".to_string(),
                    };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            payload.len(),
                            payload
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        (format!("http://127.0.0.1:{port}"), requests, stop)
    }

    #[test]
    fn stub_ignores_idle_connections_without_losing_the_next_request() {
        let (base_url, requests, stop) = start_stub();
        let address = base_url.trim_start_matches("http://");
        let idle = std::net::TcpStream::connect(address).unwrap();
        std::thread::sleep(Duration::from_millis(2300));
        assert!(requests.lock().unwrap().is_empty());
        drop(idle);
        let mut next = std::net::TcpStream::connect(address).unwrap();
        next.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            next,
            "GET /v1/accounts HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        next.read_to_string(&mut response).unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "GET");
        assert_eq!(recorded[0].1, "/v1/accounts");
    }

    fn collect_until(
        rx: &std::sync::mpsc::Receiver<Event>,
        deadline: Duration,
        want: &dyn Fn(&Event) -> bool,
    ) -> Vec<Event> {
        let start = Instant::now();
        let mut out = Vec::new();
        while start.elapsed() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ev) => {
                    let hit = want(&ev);
                    out.push(ev);
                    if hit {
                        return out;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return out,
            }
        }
        out
    }

    #[test]
    fn restored_history_completion_is_tagged_and_emitted_only_after_the_fetch_result() {
        let (base_url, _requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|event| {
            matches!(event, Event::Chats(_))
        });
        tx.send(Command::LoadChat {
            chat: "!fam:beeper.local".into(),
            before: None,
        })
        .unwrap();
        let ordinary = collect_until(
            &event_rx,
            Duration::from_secs(5),
            &|event| matches!(event, Event::Messages { chat, older: false, .. } if chat == "!fam:beeper.local"),
        );
        assert!(
            !ordinary
                .iter()
                .any(|event| matches!(event, Event::BeeperRestoredHistory { .. }))
        );
        for (chat, token, success) in [
            ("!fam:beeper.local", 41, true),
            ("foreign-chat", 42, false),
            ("fail-history", 43, false),
        ] {
            tx.send(Command::LoadRestoredChat {
                chat: chat.into(),
                token,
            })
            .unwrap();
            let events = collect_until(
                &event_rx,
                Duration::from_secs(5),
                &|event| matches!(event, Event::BeeperRestoredHistory { token: got, .. } if *got == token),
            );
            assert!(
                matches!(events.last(), Some(Event::BeeperRestoredHistory { token: got, success: got_success, .. }) if *got == token && *got_success == success)
            );
            assert!(events.iter().any(|event| matches!(event, Event::Messages { chat: got, older: false, .. } if got == chat)), "history (or failure placeholder) arrives before completion");
        }
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn edit_refusal_retains_proposal_and_message_identity() {
        let (base_url, requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        tx.send(Command::EditText {
            chat: "work/chat".into(),
            id: "message".into(),
            text: "proposed words".into(),
            mentions: vec!["unsupported".into()],
        })
        .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::BeeperEditUncertain { .. })
        });
        assert!(events.iter().any(|e| matches!(e,Event::BeeperEditUncertain {chat,id,text,..} if chat=="work/chat" && id=="message" && text=="proposed words")));
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _, _)| method == "GET")
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn unknown_length_media_never_leaves_a_file_above_cache_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = r#"{"id":"media","attachments":[{"type":"img","id":"mxc://x/a","mimeType":"image/png"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut buffer);
            write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").unwrap();
            let bytes = [7u8; 65536];
            for _ in 0..257 {
                if stream.write_all(&bytes).is_err() {
                    break;
                }
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(root.path()).as_account();
        set_cache_budget(&dirs, 16).unwrap();
        assert!(
            api.download_media("chat", "media", &dirs, false)
                .unwrap_err()
                .contains("size limit")
        );
        assert_eq!(measure_cache(&dirs).unwrap().bytes_total(), 0);
        assert!(
            std::fs::read_dir(dirs.media_cache_dir())
                .unwrap()
                .next()
                .is_none()
        );
        server.join().unwrap();
    }

    #[test]
    fn reminder_commands_use_server_contract_and_clear_route() {
        let (base_url, requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        tx.send(Command::SetBeeperReminder {
            chat: "work/chat".into(),
            at: Some("2026-10-28T14:00:00Z".into()),
        })
        .unwrap();
        collect_until(
            &event_rx,
            Duration::from_secs(5),
            &|e| matches!(e, Event::Info(text) if text.contains("accepted the reminder")),
        );
        tx.send(Command::SetBeeperReminder {
            chat: "work/chat".into(),
            at: None,
        })
        .unwrap();
        collect_until(
            &event_rx,
            Duration::from_secs(5),
            &|e| matches!(e, Event::Info(text) if text.contains("accepted the reminder")),
        );
        tx.send(Command::SetBeeperReminder {
            chat: "work/chat".into(),
            at: Some("invalid".into()),
        })
        .unwrap();
        collect_until(
            &event_rx,
            Duration::from_secs(5),
            &|e| matches!(e, Event::Info(text) if text.contains("Invalid reminder")),
        );
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        let requests = requests.lock().unwrap();
        let writes: Vec<_> = requests
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .collect();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].0, "POST");
        assert_eq!(writes[1].0, "DELETE");
        assert!(
            writes
                .iter()
                .all(|(_, path, _)| path == "/v1/chats/work%2Fchat/reminders")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&writes[0].2).unwrap(),
            serde_json::json!({"reminder":{"remindAt":"2026-10-28T14:00:00Z","dismissOnIncomingMessage":false}})
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn reaction_removal_uses_current_own_key_and_account() {
        let (base_url, requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        tx.send(Command::React {
            chat: "work/chat".into(),
            message: "m/4".into(),
            emoji: String::new(),
        })
        .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::MessageUpdated(_))
        });
        assert!(events.iter().any(|e| matches!(e, Event::MessageUpdated(m) if m.reactions.first().is_some_and(|r| r.from_me))));
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        let requests = requests.lock().unwrap();
        let writes: Vec<_> = requests
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "DELETE");
        assert_eq!(
            writes[0].1,
            "/v1/chats/work%2Fchat/messages/m%2F4/reactions/%F0%9F%91%8D"
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn a_change_chat_patch_reads_the_returned_chat() {
        let (base, _requests, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        assert_eq!(
            api.change_chat("work/chat", "isMuted", true).unwrap(),
            Some(true)
        );
        assert_eq!(
            api.change_chat("work/chat", "isPinned", true).unwrap(),
            Some(false)
        );
        // A field the answer does not carry stays unknown, not a refusal.
        assert_eq!(
            api.change_chat("work/chat", "isLowPriority", true).unwrap(),
            None
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn a_contradicting_synced_state_earns_one_named_report() {
        // Agreement stays quiet; contradiction names where the intent holds.
        assert!(Api::unconfirmed_state("isMuted", true, true, true).is_none());
        let report = Api::unconfirmed_state("isMuted", true, false, true).unwrap();
        assert!(report.contains("hasn't confirmed the mute"));
        assert!(report.contains("still holds in Solmu"));
        let report = Api::unconfirmed_state("isArchived", false, true, false).unwrap();
        assert!(report.contains("hasn't confirmed the unarchive"));
        assert!(report.contains("check the chat before relying on it"));
    }

    #[test]
    fn get_chat_reads_one_chats_own_record() {
        let (base, _requests, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let chat = api.get_chat("work/chat").unwrap();
        assert_eq!(chat.id, "work/chat");
        assert!(!chat.is_muted);
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn readonly_worker_refuses_new_chat_and_message_mutations() {
        // Boundary coverage does not rely on a process-global environment flag.
        let (base, requests, stop) = start_stub();
        let mut api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        api.read_only = true;
        for (method, path, body) in [
            (
                reqwest::Method::PATCH,
                "/v1/chats/x",
                Some(serde_json::json!({"isMuted":true})),
            ),
            (
                reqwest::Method::PUT,
                "/v1/chats/x/messages/y",
                Some(serde_json::json!({"text":"no"})),
            ),
            (
                reqwest::Method::DELETE,
                "/v1/chats/x/messages/y?forEveryone=true",
                None,
            ),
            (
                reqwest::Method::DELETE,
                "/v1/chats/x/messages/y/reactions/key",
                None,
            ),
        ] {
            assert!(api.mutate(method, path, body).is_err());
        }
        assert!(
            api.post(
                "/v1/chats/x/reminders",
                serde_json::json!({"reminder":{"remindAt":"2026-10-28T14:00:00Z"}})
            )
            .is_err()
        );
        assert!(
            api.mutate(reqwest::Method::DELETE, "/v1/chats/x/reminders", None)
                .is_err()
        );
        assert!(
            api.post("/v1/chats/x/archive", serde_json::json!({"archived":true}))
                .is_err()
        );
        assert!(
            api.post(
                "/v1/chats/x/messages/y/reactions",
                serde_json::json!({"reactionKey":"👍"})
            )
            .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn own_reactions_follow_message_account_not_first_account() {
        let (base, _, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        api.accounts().unwrap();
        let wire: BeeperMessage = serde_json::from_value(
            serde_json::json!({"id":"r", "accountID":"local-whatsapp*ba*222", "reactions":[
                {"participantID":"@me-work:beeper.local","reactionKey":"👍","emoji":true},
                {"participantID":"@me:beeper.local","reactionKey":"❤️","emoji":true}
            ]}),
        )
        .unwrap();
        let mapped = api.map_message(&wire, "chat");
        assert!(mapped.reactions[0].from_me);
        assert!(!mapped.reactions[1].from_me);
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn backend_command_dispatch_routes_archive_mute_pin_edit_delete_reaction() {
        let (base_url, requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        for command in [
            Command::SetArchived("work/chat".into(), true),
            Command::SetPinned("work/chat".into(), true),
            Command::SetMuted("work/chat".into(), Some(0)),
            Command::EditText {
                chat: "work/chat".into(),
                id: "m/1".into(),
                text: "edited".into(),
                mentions: vec![],
            },
            Command::DeleteLocal {
                chat: "work/chat".into(),
                id: "m/2".into(),
            },
            Command::Revoke {
                chat: "work/chat".into(),
                id: "m/3".into(),
            },
            Command::React {
                chat: "work/chat".into(),
                message: "m/4".into(),
                emoji: "👍".into(),
            },
        ] {
            tx.send(command).unwrap();
        }
        let started = Instant::now();
        while requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .count()
            < 7
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        let requests = requests.lock().unwrap();
        let writes: Vec<_> = requests
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .collect();
        assert_eq!(writes.len(), 7);
        assert!(
            writes
                .iter()
                .all(|(_, path, _)| path.starts_with("/v1/chats/work%2Fchat"))
        );
        assert!(writes.iter().any(|(method,path,_)| method == "DELETE" && path.ends_with("?forEveryone=false")));
        assert!(
            writes
                .iter()
                .any(|(method, path, _)| method == "DELETE" && path.ends_with("?forEveryone=true"))
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn cache_eviction_only_touches_disposable_owned_files() {
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(root.path()).as_account();
        std::fs::create_dir_all(dirs.media_cache_dir()).unwrap();
        std::fs::create_dir_all(&dirs.state).unwrap();
        std::fs::write(dirs.archive_db(), b"message history").unwrap();
        let folder = dirs.media_cache_dir();
        std::fs::write(folder.join("saved-user-download.pdf"), b"saved").unwrap();
        std::fs::write(folder.join("beeper-old.jpg"), [1u8; 8]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(folder.join("beeper-new.jpg"), [2u8; 8]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dirs.archive_db(), folder.join("beeper-linked.db")).unwrap();
        trim_cache(&dirs, 8).unwrap();
        assert!(!folder.join("beeper-old.jpg").exists());
        assert!(folder.join("beeper-new.jpg").exists());
        assert_eq!(measure_cache(&dirs).unwrap().bytes_total(), 8);
        trim_cache(&dirs, 0).unwrap();
        assert!(dirs.archive_db().exists());
        assert!(folder.join("saved-user-download.pdf").exists());
        assert_eq!(measure_cache(&dirs).unwrap().bytes_total(), 0);
        assert_eq!(cache_budget(&dirs), 256 * 1024 * 1024);
        set_cache_budget(&dirs, 64).unwrap();
        assert_eq!(cache_budget(&dirs), 64 * 1024 * 1024);
    }

    #[test]
    fn account_identity_is_not_a_display_label() {
        let one: BeeperChat = serde_json::from_str(
            r#"{"id":"a","accountID":"one","network":"WhatsApp","title":"Same"}"#,
        )
        .unwrap();
        let two: BeeperChat = serde_json::from_str(
            r#"{"id":"b","accountID":"two","network":"WhatsApp","title":"Same"}"#,
        )
        .unwrap();
        assert_eq!(
            map_chat(&one, &|_| "Same".into())
                .backend_account
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            map_chat(&two, &|_| "Same".into())
                .backend_account
                .as_deref(),
            Some("two")
        );
    }

    #[test]
    fn chat_and_message_changes_use_exact_sdk_verbs_and_fields() {
        let (base, reqs, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        api.change_chat("chat/a", "isPinned", true).unwrap();
        api.change_chat("chat/a", "isMuted", false).unwrap();
        api.post(
            "/v1/chats/chat%2Fa/archive",
            serde_json::json!({"archived":true}),
        )
        .unwrap();
        api.mutate(
            reqwest::Method::PUT,
            "/v1/chats/chat%2Fa/messages/m%2F1",
            Some(serde_json::json!({"text":"edited"})),
        )
        .unwrap();
        api.mutate(
            reqwest::Method::DELETE,
            "/v1/chats/chat%2Fa/messages/m%2F1?forEveryone=false",
            None,
        )
        .unwrap();
        api.post(
            "/v1/chats/chat%2Fa/messages/m%2F1/reactions",
            serde_json::json!({"reactionKey":"👍"}),
        )
        .unwrap();
        let captured = reqs.lock().unwrap();
        assert_eq!(captured[0].0, "PATCH");
        assert_eq!(captured[0].1, "/v1/chats/chat%2Fa");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&captured[0].2).unwrap(),
            serde_json::json!({"isPinned":true})
        );
        assert_eq!(captured[3].0, "PUT");
        assert_eq!(captured[4].0, "DELETE");
        assert!(captured[4].1.ends_with("?forEveryone=false"));
        let mut readonly = api.clone();
        readonly.read_only = true;
        assert!(readonly.change_chat("x", "isPinned", true).is_err());
        assert!(
            readonly
                .mutate(reqwest::Method::DELETE, "/x", None)
                .is_err()
        );
        assert_eq!(captured.len(), 6);
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn slow_media_does_not_block_control_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let read_seen = Arc::new(AtomicBool::new(false));
        let media_started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (read2, media2, stop2) = (read_seen.clone(), media_started.clone(), stop.clone());
        let server = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let (read2, media2) = (read2.clone(), media2.clone());
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let length = stream.read(&mut buffer).unwrap();
                    let request = String::from_utf8_lossy(&buffer[..length]);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let body = if path.contains("/assets/serve") {
                        media2.store(true, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_secs(2));
                        "image"
                    } else if path.ends_with("/messages/media") {
                        r#"{"id":"media","attachments":[{"type":"img","id":"mxc://x/a"}]}"#
                    } else if path == "/v1/accounts" {
                        ACCOUNTS_JSON
                    } else if path.starts_with("/v1/chats?") {
                        CHATS_JSON
                    } else {
                        if request.starts_with("POST ") && path.ends_with("/read") {
                            read2.store(true, Ordering::Relaxed);
                        }
                        "{}"
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url: base,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        tx.send(Command::Download {
            manual: false,
            card: None,
            chat: "chat".into(),
            message: "media".into(),
        })
        .unwrap();
        let deadline = Instant::now();
        while !media_started.load(Ordering::Relaxed) && deadline.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(media_started.load(Ordering::Relaxed));
        tx.send(Command::MarkRead {
            chat: "chat".into(),
            receipts: true,
        })
        .unwrap();
        let deadline = Instant::now();
        while !read_seen.load(Ordering::Relaxed) && deadline.elapsed() < Duration::from_millis(800)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            read_seen.load(Ordering::Relaxed),
            "Control command blocked on media stream"
        );
        collect_until(&event_rx, Duration::from_secs(4), &|e| {
            matches!(e, Event::Media { .. })
        });
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
    }

    #[test]
    fn slow_upload_does_not_block_control_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let read_seen = Arc::new(AtomicBool::new(false));
        let upload_started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (read2, media2, stop2) = (read_seen.clone(), upload_started.clone(), stop.clone());
        let server = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let (read2, media2) = (read2.clone(), media2.clone());
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let length = stream.read(&mut buffer).unwrap();
                    let request = String::from_utf8_lossy(&buffer[..length]);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let body = if path.contains("/assets/upload/base64") {
                        media2.store(true, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_secs(2));
                        r#"{"uploadID":"slow-upload"}"#
                    } else if request.starts_with("POST ") && path.ends_with("/messages") {
                        r#"{"pendingMessageID":"pending-upload","chatID":"chat"}"#
                    } else if path.ends_with("/messages/media") {
                        r#"{"id":"media","attachments":[{"type":"img","id":"mxc://x/a"}]}"#
                    } else if path == "/v1/accounts" {
                        ACCOUNTS_JSON
                    } else if path.starts_with("/v1/chats?") {
                        CHATS_JSON
                    } else {
                        if request.starts_with("POST ") && path.ends_with("/read") {
                            read2.store(true, Ordering::Relaxed);
                        }
                        "{}"
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url: base,
                    token: "test".into(),
                },
                dirs,
                event_tx,
                rx,
                Waker::default(),
            )
        });
        collect_until(&event_rx, Duration::from_secs(5), &|e| {
            matches!(e, Event::Chats(_))
        });
        let path = dir.path().join("small.txt");
        std::fs::write(&path, b"test").unwrap();
        tx.send(Command::SendFiles {
            chat: "chat".into(),
            paths: vec![path],
            documents: true,
            view_once: false,
            caption: None,
            mentions: vec![],
            quoting: None,
        })
        .unwrap();
        let deadline = Instant::now();
        while !upload_started.load(Ordering::Relaxed) && deadline.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(upload_started.load(Ordering::Relaxed));
        tx.send(Command::MarkRead {
            chat: "chat".into(),
            receipts: true,
        })
        .unwrap();
        let deadline = Instant::now();
        while !read_seen.load(Ordering::Relaxed) && deadline.elapsed() < Duration::from_millis(800)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            read_seen.load(Ordering::Relaxed),
            "Control command blocked on upload stream"
        );
        std::thread::sleep(Duration::from_secs(3));
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
    }

    #[test]
    fn empty_chat_without_cursor_is_still_polled() {
        let (base_url, requests, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "test".into(),
        })
        .unwrap();
        let chat = "!fam:beeper.local".to_string();
        let mut cursors = HashMap::new();
        cursors.insert(
            chat.clone(),
            ChatCursors {
                oldest: None,
                newest: None,
                has_more_older: false,
            },
        );
        let events = std::sync::Mutex::new(Vec::new());
        poll_newer(&api, &chat, &mut cursors, &|e| {
            events.lock().unwrap().push(e)
        });
        assert!(!requests.lock().unwrap().is_empty());
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, Event::Messages { .. }))
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn readonly_blocks_every_post_at_http_boundary() {
        let (base_url, requests, stop) = start_stub();
        let mut api = Api::new(&BeeperConfig {
            base_url,
            token: "test".into(),
        })
        .unwrap();
        api.read_only = true;
        assert!(api.mark_read("chat").is_err());
        assert!(api.mark_unread("chat").is_err());
        assert!(api.typing("chat", true).is_err());
        assert!(api.send_text("chat", "hello", None, &[]).is_err());
        assert!(
            api.send_file("chat", std::path::Path::new("missing"), false, None)
                .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn attachment_upload_then_send_routes_to_selected_chat_only() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 8192];
                let length = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]);
                let body = if index == 0 {
                    assert!(request.starts_with("POST /v1/assets/upload/base64 "));
                    assert!(request.contains("aGVsbG8="));
                    r#"{"uploadID":"upload-one"}"#
                } else {
                    assert!(request.starts_with("POST /v1/chats/work-account-chat/messages "));
                    assert!(
                        request.contains("upload-one")
                            && request.contains("caption")
                            && request.contains("file")
                    );
                    r#"{"pendingMessageID":"pending-one","chatID":"work-account-chat"}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        let accepted = api
            .send_file("work-account-chat", &path, true, Some("caption"))
            .unwrap();
        assert_eq!(accepted.chat, "work-account-chat");
        worker.join().unwrap();
    }

    #[test]
    fn attachment_send_omits_the_text_field_without_a_caption() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 8192];
                let length = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]);
                let body = if index == 0 {
                    assert!(request.starts_with("POST /v1/assets/upload/base64 "));
                    r#"{"uploadID":"upload-no-caption"}"#
                } else {
                    assert!(request.starts_with("POST /v1/chats/chat/messages "));
                    assert!(request.contains("upload-no-caption"));
                    // An explicit null text is not the contract's optional
                    // string; without a caption the field goes away.
                    assert!(!request.contains("\"text\""));
                    r#"{"pendingMessageID":"pending-no-caption","chatID":"chat"}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        let accepted = api.send_file("chat", &path, false, None).unwrap();
        assert_eq!(accepted.pending_id, "pending-no-caption");
        worker.join().unwrap();
    }

    #[test]
    fn attachment_send_error_names_the_rejected_field() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 8192];
                let length = stream.read(&mut buffer).unwrap();
                let _ = String::from_utf8_lossy(&buffer[..length]);
                let (status, body) = if index == 0 {
                    ("200 OK", r#"{"uploadID":"upload-refused"}"#)
                } else {
                    (
                        "400 Bad Request",
                        r#"{"error":{"path":["text"],"message":"expected string, received null","input":"REDACTED-CONTENT"}}"#,
                    )
                };
                write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        let error = api.send_file("chat", &path, false, None).unwrap_err();
        assert!(error.contains("400"), "{error}");
        assert!(error.contains("invalid field text"), "{error}");
        // The server's message/input values never reach the error.
        assert!(!error.contains("REDACTED-CONTENT"), "{error}");
        worker.join().unwrap();
    }

    #[test]
    fn read_receipts_in_every_shape_the_api_sends_count_as_read() {
        use serde_json::json;
        for (seen, expected) in [
            (json!(true), Delivery::Read),
            (json!("2026-11-03T07:48:00Z"), Delivery::Read),
            (json!({"a": "2026-11-03T07:48:00Z"}), Delivery::ReadBySome),
            (json!({"a": false, "b": true}), Delivery::ReadBySome),
            (json!(false), Delivery::Sent),
            (json!(""), Delivery::Sent),
            (json!({"a": false}), Delivery::Sent),
            (json!({}), Delivery::Sent),
            (json!({"@me": "2026-11-03T07:48:00Z"}), Delivery::Sent),
        ] {
            let wire = json!({
                "id": "x", "isSender": true, "senderID": "@me",
                "seen": seen.clone(),
                "sendStatus": {"status": "SUCCESS"}
            });
            let m: BeeperMessage = serde_json::from_value(wire).unwrap();
            assert_eq!(map_message(&m, "chat").status, expected, "{seen}");
        }
    }

    #[test]
    fn a_seen_timestamp_becomes_the_read_time_and_nothing_invents_one() {
        use serde_json::json;
        use std::str::FromStr;
        let read_at = |seen: serde_json::Value| {
            let wire = json!({
                "id": "x", "isSender": true, "senderID": "@me",
                "seen": seen, "sendStatus": {"status": "SUCCESS"}
            });
            let m: BeeperMessage = serde_json::from_value(wire).unwrap();
            map_message(&m, "chat").read_at
        };
        let stamp = jiff::Timestamp::from_str("2026-11-03T07:48:00Z")
            .expect("valid")
            .as_second();
        // A timestamp string is the read time.
        assert_eq!(read_at(json!("2026-11-03T07:48:00Z")), Some(stamp));
        // A single other reader's map entry is too.
        assert_eq!(
            read_at(json!({"@me": "2026-11-01T00:00:00Z", "them": "2026-11-03T07:48:00Z"})),
            Some(stamp)
        );
        // Several readers: no single time to show.
        assert_eq!(
            read_at(json!({"a": "2026-11-03T07:48:00Z", "b": "2026-11-03T08:00:00Z"})),
            None
        );
        // Bare "read" without a time, an unreadable string, and unread states
        // all stay unrecorded.
        assert_eq!(read_at(json!(true)), None);
        assert_eq!(read_at(json!("soon")), None);
        assert_eq!(read_at(json!(false)), None);
        assert_eq!(read_at(json!({"a": false})), None);
    }

    #[test]
    fn a_chat_preview_shows_what_the_bridge_reported_not_always_sent() {
        let preview = |wire: serde_json::Value| {
            let m: BeeperMessage = serde_json::from_value(wire).unwrap();
            preview_status(&m)
        };
        let read = serde_json::json!({"id": "x", "isSender": true, "seen": true});
        assert_eq!(preview(read), Delivery::Read);
        let bare = serde_json::json!({"id": "x", "isSender": true});
        assert_eq!(preview(bare), Delivery::Sent);
        let incoming = serde_json::json!({"id": "x", "isSender": false});
        assert_eq!(preview(incoming), Delivery::None);
    }

    #[test]
    fn bridge_delivery_status_is_not_reported_as_success() {
        for (status, expected) in [
            ("PENDING", Delivery::Pending),
            ("FAIL_PERMANENT", Delivery::Failed),
            ("SUCCESS", Delivery::Sent),
            ("", Delivery::Unknown),
        ] {
            let wire =
                serde_json::json!({"id":"x", "isSender":true, "sendStatus":{"status":status}});
            let m: BeeperMessage = serde_json::from_value(wire).unwrap();
            assert_eq!(map_message(&m, "chat").status, expected);
        }
    }

    #[test]
    fn attachment_download_size_limit_is_streamed_and_cleaned() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                if index == 0 {
                    let body = r#"{"id":"media","attachments":[{"type":"img","id":"mxc://x/media","mimeType":"image/png","fileSize":1}]}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                } else {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: 67108865\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        assert!(
            api.download_media("chat", "media", &dirs, false)
                .unwrap_err()
                .contains("limit")
        );
        assert!(!dirs.media_cache_dir().exists());
        worker.join().unwrap();
    }

    #[test]
    fn unsafe_endpoint_configuration_is_refused() {
        for base in [
            "http://example.com",
            "file:///tmp/anything",
            "https://user:pass@example.com",
            "http://127.0.0.1?token=secret",
            "http://127.0.0.1/#private",
            "https://example.com/v1/accounts",
        ] {
            assert!(
                Api::new(&BeeperConfig {
                    base_url: base.into(),
                    token: "test".into()
                })
                .is_err()
            );
        }
    }

    #[test]
    fn chat_inventory_stops_at_row_and_page_bounds() {
        assert!(check_chat_inventory_bounds(100_000, 1).is_ok());
        assert!(check_chat_inventory_bounds(100_001, 1).is_err());
        assert!(check_chat_inventory_bounds(0, 1000).is_ok());
        assert!(check_chat_inventory_bounds(0, 1001).is_err());
    }

    #[test]
    fn advancing_empty_chat_pages_cannot_run_forever() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..1000 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let body =
                    format!(r#"{{"items":[],"hasMore":true,"oldestCursor":"cursor-{index}"}}"#);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        assert!(api.list_chats().unwrap_err().contains("1000 pages"));
        worker.join().unwrap();
    }

    #[test]
    fn send_confirmation_is_bounded_and_never_replayed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "fixture".into(),
        })
        .unwrap();
        let error = api
            .send_text("fixture-chat", "fixture-message", None, &[])
            .unwrap_err();
        assert!(error.contains("16 MB"));
        assert!(error.contains("check history before retrying"));
        worker.join().unwrap();
    }

    #[test]
    fn all_chat_pages_are_loaded_and_account_names_stay_unique() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let length = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]);
                if index == 1 {
                    assert!(request.contains("cursor=next-page"));
                }
                let body = if index == 0 {
                    r#"{"items":[{"id":"a","accountID":"one","network":"WhatsApp"}],"hasMore":true,"oldestCursor":"next-page"}"#
                } else {
                    r#"{"items":[{"id":"b","accountID":"two","network":"WhatsApp"}],"hasMore":false}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let chats = api.list_chats().unwrap().items;
        assert_eq!(chats.len(), 2);
        let accounts: Vec<BeeperAccount> = serde_json::from_str(r#"[{"accountID":"one","user":{"id":"u1","fullName":"Same"}},{"accountID":"two","user":{"id":"u2","fullName":"Same"}}]"#).unwrap();
        let label = network_labeller(&chats, &accounts);
        assert_ne!(label(&chats[0]), label(&chats[1]));
        worker.join().unwrap();
    }

    #[test]
    fn media_download_uses_api_not_source_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let length = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]);
                assert!(request.starts_with("GET "));
                let body = if index == 0 {
                    assert!(request.contains("/v1/chats/chat/messages/media"));
                    r#"{"id":"media","attachments":[{"type":"img","id":"mxc://untrusted.example/media","mimeType":"image/png","size":{"width":12,"height":8}}]}"#
                } else {
                    assert!(request.contains("/v1/assets/serve?url=mxc"));
                    "test-image-bytes"
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = Api::new(&BeeperConfig {
            base_url: base,
            token: "test".into(),
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let path = api.download_media("chat", "media", &dirs, false).unwrap();
        assert!(path.starts_with(dirs.media_cache_dir()));
        assert_eq!(std::fs::read(path).unwrap(), b"test-image-bytes");
        worker.join().unwrap();
        let m: BeeperMessage = serde_json::from_str(r#"{"id":"media","attachments":[{"type":"audio","id":"mxc://x/a","isVoiceNote":true,"duration":3.0}]}"#).unwrap();
        assert!(matches!(
            map_message(&m, "chat").content,
            Content::Audio {
                voice_note: true,
                seconds: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn polling_merges_content_not_just_notifications() {
        let (base_url, _requests, stop) = start_stub();
        let api = Api::new(&BeeperConfig {
            base_url,
            token: "test".into(),
        })
        .unwrap();
        let chat = "!fam:beeper.local".to_string();
        let mut cursors = HashMap::new();
        cursors.insert(
            chat.clone(),
            ChatCursors {
                oldest: None,
                newest: Some("newest".into()),
                has_more_older: false,
            },
        );
        let events = std::sync::Mutex::new(Vec::new());
        poll_newer(&api, &chat, &mut cursors, &|e| {
            events.lock().unwrap().push(e)
        });
        let events = events.into_inner().unwrap();
        assert!(events.iter().any(
            |e| matches!(e, Event::Messages { older: false, messages, .. } if !messages.is_empty())
        ));
        assert!(events.iter().any(|e| matches!(e, Event::Incoming { .. })));
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn startup_is_read_only_and_labels_same_network_accounts() {
        let (base_url, requests, stop) = start_stub();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(dir.path()).as_account();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = std::thread::spawn(move || {
            run(
                BeeperConfig {
                    base_url,
                    token: "test-token".to_string(),
                },
                dirs,
                event_tx,
                cmd_rx,
                Waker::default(),
            )
        });

        // Startup: Link(Connected) and a Chats snapshot arrive.
        let events = collect_until(&event_rx, Duration::from_secs(5), &|ev| {
            matches!(ev, Event::Chats(_))
        });
        assert!(
            events.iter().any(|ev| matches!(
                ev,
                Event::Link(status) if status.is_connected()
            )),
            "expected Link(Connected), got {events:?}"
        );
        let chats = events
            .iter()
            .find_map(|ev| match ev {
                Event::Chats(chats) => Some(chats.clone()),
                _ => None,
            })
            .expect("chat snapshot");
        assert_eq!(chats.len(), 3);

        // Same-network accounts are disambiguated by account identity;
        // a solo network stays bare; identities do not leak across networks.
        let by_id: HashMap<&str, &Chat> = chats.iter().map(|c| (c.id.as_str(), c)).collect();
        assert_eq!(
            by_id["!fam:beeper.local"].network.as_deref(),
            Some("WhatsApp · Personal")
        );
        assert_eq!(
            by_id["!alice:beeper.local"].network.as_deref(),
            Some("WhatsApp · Work")
        );
        assert_eq!(
            by_id["!standup:beeper.local"].network.as_deref(),
            Some("Slack")
        );
        assert_eq!(by_id["!fam:beeper.local"].kind, ChatKind::Group);
        assert_eq!(by_id["!alice:beeper.local"].kind, ChatKind::Direct);
        assert_eq!(by_id["!fam:beeper.local"].unread, 3);

        // Let background timers tick: still no mutating request.
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _, _)| method == "GET"),
            "startup/polling must be read-only: {:?}",
            requests.lock().unwrap()
        );

        // Opening a chat pages history (GETs only).
        cmd_tx
            .send(Command::LoadChat {
                chat: "!fam:beeper.local".to_string(),
                before: None,
            })
            .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|ev| {
            matches!(ev, Event::Messages { older: false, .. })
        });
        let messages = events
            .iter()
            .find_map(|ev| match ev {
                Event::Messages { messages, .. } => Some(messages.clone()),
                _ => None,
            })
            .expect("messages");
        assert_eq!(messages.len(), 3);
        assert!(!messages[2].from_me);
        // The image renders as a labelled text placeholder in this demo.
        match &messages.iter().find(|m| m.id == "m3").unwrap().content {
            Content::Text { text, .. } => assert_eq!(text, "[image: cake.jpg]"),
            other => panic!("expected text placeholder, got {other:?}"),
        }

        cmd_tx
            .send(Command::LoadChat {
                chat: "!fam:beeper.local".to_string(),
                before: Some((1, "m1".to_string())),
            })
            .unwrap();
        let older_events = collect_until(&event_rx, Duration::from_secs(5), &|ev| {
            matches!(ev, Event::Messages { older: true, .. })
        });
        assert!(older_events.iter().any(|ev| matches!(ev,
            Event::Messages { older: true, messages, .. } if messages.len() == 1)));

        cmd_tx
            .send(Command::LoadChat {
                chat: "!fam:beeper.local".into(),
                before: None,
            })
            .unwrap();
        collect_until(&event_rx, Duration::from_secs(5), &|ev| {
            matches!(ev, Event::Messages { older: false, .. })
        });
        cmd_tx
            .send(Command::FetchOlder {
                chat: "!fam:beeper.local".to_string(),
                explicit: true,
            })
            .unwrap();
        let events = collect_until(&event_rx, Duration::from_secs(5), &|ev| {
            matches!(ev, Event::OlderFetched { .. })
        });
        assert!(events.iter().any(|ev| matches!(
            ev,
            Event::Messages { older: true, messages, .. } if messages.len() == 1
        )));
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _, _)| method == "GET")
        );

        // Deliberate actions POST - and only they do.
        cmd_tx
            .send(Command::SendText {
                chat: "!fam:beeper.local".to_string(),
                text: "on my way".to_string(),
                quoting: None,
                mentions: Vec::new(),
            })
            .unwrap();
        cmd_tx
            .send(Command::MarkRead {
                chat: "!fam:beeper.local".to_string(),
                receipts: true,
            })
            .unwrap();
        cmd_tx
            .send(Command::Composing {
                chat: "!fam:beeper.local".to_string(),
                composing: true,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let posts: Vec<(String, String)> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m == "POST")
            .map(|(_, p, b)| (p.clone(), b.clone()))
            .collect();
        assert_eq!(posts.len(), 3, "only the deliberate commands may POST");
        assert!(posts[0].0.ends_with("/messages") && posts[0].1.contains("on my way"));
        assert!(posts[1].0.ends_with("/read"));
        assert!(posts[2].0.ends_with("/typing") && posts[2].1.contains("typing"));

        cmd_tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn content_free_preview_is_not_shown_as_a_message_placeholder() {
        let chat: BeeperChat = serde_json::from_value(serde_json::json!({
            "id":"c1", "accountID":"a1", "network":"WhatsApp",
            "preview":{"id":"r1","senderID":"s","sortKey":"1","timestamp":"2026-11-02T19:28:00Z","isSender":false}
        }))
        .unwrap();
        let mapped = map_chat(&chat, &|c| c.network.clone());
        assert!(mapped.last.is_none(), "no \"[message]\" placeholder");
        let real: BeeperChat = serde_json::from_value(serde_json::json!({
            "id":"c1", "accountID":"a1", "network":"WhatsApp",
            "preview":{"id":"m1","senderID":"s","sortKey":"1","timestamp":"2026-11-02T19:28:00Z","isSender":false,"text":"no panic","type":"TEXT"}
        }))
        .unwrap();
        assert_eq!(
            map_chat(&real, &|c| c.network.clone())
                .last
                .unwrap()
                .summary,
            "no panic"
        );
    }

    #[test]
    fn tag_free_rich_text_decodes_one_level() {
        // Live-payload evidence (2026-11-03 probe of the reported Dropbox
        // message): the Server sends a TEXT payload with the link separator
        // already entity-escaped - wire bytes 26 61 6d 70 3b ("&amp;"), zero
        // bare ampersands in the 113-char text, no HTML branch. The field's
        // contract is rich text, so the wire form decodes one level.
        assert_eq!(
            plain_rich_text(
                "https://www.dropbox.com/scl/fi/aaa/bbb/hh_uunista_ulos.mp4?rlkey=k&amp;dl=0"
            ),
            "https://www.dropbox.com/scl/fi/aaa/bbb/hh_uunista_ulos.mp4?rlkey=k&dl=0"
        );
        assert_eq!(
            plain_rich_text("see (https://x.test/?a=1&amp;b=2). then &amp; more"),
            "see (https://x.test/?a=1&b=2). then & more"
        );
        // One level only: typed literally, "&amp;" arrives double-escaped
        // and a single decode restores the literal.
        assert_eq!(
            plain_rich_text("https://x.test/?q=1&amp;amp;r=2"),
            "https://x.test/?q=1&amp;r=2"
        );
        // Percent-encoded text is not entity encoding and stays untouched.
        assert_eq!(
            plain_rich_text("https://x.test/?q=a%26amp%3Bb&r=%2B1"),
            "https://x.test/?q=a%26amp%3Bb&r=%2B1"
        );
        assert_eq!(
            plain_rich_text("https://x.test/?a=1&#38;b=2 and https://x.test/?a=1&#x26;b=2"),
            "https://x.test/?a=1&b=2 and https://x.test/?a=1&b=2"
        );
    }

    #[test]
    fn server_escaped_text_decodes_one_level() {
        // The reported WhatsApp quote, exactly as it arrived on the wire.
        assert_eq!(
            plain_rich_text("&quot;t\u{e4}\u{e4}h\u{e4}n on ku tilausty\u{f6} meille&quot;"),
            "\"t\u{e4}\u{e4}h\u{e4}n on ku tilausty\u{f6} meille\""
        );
        // Every entity in the wire form decodes.
        assert_eq!(
            plain_rich_text("&quot;a&quot; &amp; &lt;b&gt; &#8220;c&#8221;"),
            "\"a\" & <b> \u{201c}c\u{201d}"
        );
        // One level only: a typed "&quot;" arrived double-escaped on the
        // wire, and a single decode restores exactly the typed literal.
        assert_eq!(plain_rich_text("&amp;quot;"), "&quot;");
        // A hand-typed entity arrives double-escaped and stays literal.
        assert_eq!(
            plain_rich_text("&amp;quot;tagged&amp;quot; and &amp;amp;"),
            "&quot;tagged&quot; and &amp;"
        );
        // Unknown entity names stay literal, so even a contract-violating
        // bridge's unrecognized escapes pass through undamaged.
        assert_eq!(plain_rich_text("&quot;a&quot; &bogus;"), "\"a\" &bogus;");
    }

    #[test]
    fn a_link_showing_its_address_is_not_repeated_in_parentheses() {
        // The reported Yle share: the bridge marked the pasted URL up as an
        // anchor whose text is the address itself.
        assert_eq!(
            plain_rich_text(
                r#"<a href="https://yle.fi/a/7-10106742?utm_source=a&amp;utm_medium=b">https://yle.fi/a/7-10106742?utm_source=a&amp;utm_medium=b</a>"#
            ),
            "https://yle.fi/a/7-10106742?utm_source=a&utm_medium=b"
        );
        // A trailing-slash difference still reads as the same address.
        assert_eq!(
            plain_rich_text(r#"<a href="https://example.com/">https://example.com</a>"#),
            "https://example.com"
        );
        // A distinct label keeps the parenthesized destination.
        assert_eq!(
            plain_rich_text(r#"<a href="https://example.com/page">this</a>"#),
            "this (https://example.com/page)"
        );
    }

    #[test]
    fn a_reaction_on_the_newest_message_leads_the_list_preview() {
        let preview = |reactions: serde_json::Value| {
            let chat: BeeperChat = serde_json::from_value(serde_json::json!({
                "id":"c1", "accountID":"a1", "network":"WhatsApp",
                "preview":{"id":"m1","senderID":"s","sortKey":"1","timestamp":"2026-11-03T09:47:00Z","isSender":true,"text":"joo, kunhan saan sen rakennettua","type":"TEXT","reactions":reactions}
            }))
            .unwrap();
            map_chat(&chat, &|c| c.network.clone()).last.unwrap()
        };
        let one = preview(serde_json::json!([
            {"participantID":"p","reactionKey":"🤝","emoji":true}
        ]));
        assert_eq!(one.summary, "🤝 joo, kunhan saan sen rakennettua");
        assert_eq!(one.full, one.summary);
        assert!(one.from_me, "the sent message keeps its ownership");
        let many = preview(serde_json::json!([
            {"participantID":"p","reactionKey":"👍","emoji":true},
            {"participantID":"q","reactionKey":"🤝","emoji":true},
            {"participantID":"r","reactionKey":"👍","emoji":true},
            {"participantID":"s","reactionKey":"sticker-id","emoji":false}
        ]));
        assert_eq!(many.summary, "👍2 🤝 joo, kunhan saan sen rakennettua");
        // A removed reaction leaves the plain text again.
        let none = preview(serde_json::json!([]));
        assert_eq!(none.summary, "joo, kunhan saan sen rakennettua");
    }

    #[test]
    fn newest_content_message_skips_empty_hidden_and_system_events() {
        let items: Vec<BeeperMessage> = serde_json::from_value(serde_json::json!([
            {"id":"old","senderID":"s","timestamp":"2026-11-02T19:00:00Z","isSender":false,"text":"older","type":"TEXT"},
            {"id":"real","senderID":"s","timestamp":"2026-11-02T19:27:00Z","isSender":true,"text":"no panic","type":"TEXT"},
            {"id":"react","senderID":"s","timestamp":"2026-11-02T19:28:00Z","isSender":false,"type":"REACTION"},
            {"id":"blank","senderID":"s","timestamp":"2026-11-02T19:29:00Z","isSender":false},
            {"id":"sys","senderID":"s","timestamp":"2026-11-02T19:30:00Z","isSender":false,"text":"joined","type":"SYSTEM"},
            {"id":"hidden","senderID":"s","timestamp":"2026-11-02T19:31:00Z","isSender":false,"text":"x","type":"TEXT","isHidden":true}
        ]))
        .unwrap();
        assert_eq!(newest_content_message(items).unwrap().id, "real");
        assert!(newest_content_message(Vec::new()).is_none());
    }

    #[test]
    fn placeholder_labels_cover_media_kinds() {
        let mut m: BeeperMessage = serde_json::from_str(
            r#"{"id":"x","senderID":"s","sortKey":"1","timestamp":"2026-10-27T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(body_of(&m).0, "[message]");
        m.message_type = Some("IMAGE".to_string());
        assert_eq!(body_of(&m).0, "[image]");
        m.message_type = Some("VOICE".to_string());
        assert_eq!(body_of(&m).0, "[voice message]");
        m.message_type = Some("VIDEO".to_string());
        m.attachments = vec![BeeperAttachment {
            kind: Some("video".to_string()),
            file_name: Some("clip.mp4".to_string()),
            mime_type: None,
            ..Default::default()
        }];
        assert_eq!(body_of(&m).0, "[video: clip.mp4]");
        m.text = Some("caption wins".to_string());
        assert_eq!(body_of(&m).0, "caption wins");
    }

    #[test]
    fn a_poll_without_text_gets_an_honest_placeholder() {
        let m: BeeperMessage = serde_json::from_str(
            r#"{"id":"x","senderID":"s","sortKey":"1","timestamp":"2026-10-27T00:00:00Z","type":"POLL"}"#,
        )
        .unwrap();
        assert_eq!(body_of(&m).0, "[poll]");
    }

    #[test]
    fn undocumented_message_fields_are_tolerated() {
        // A bridge may already send poll payloads the schema does not
        // document; unknown fields must never break the mapping.
        let m: BeeperMessage = serde_json::from_str(
            r#"{"id":"x","senderID":"s","sortKey":"1","timestamp":"2026-10-27T00:00:00Z","type":"POLL","poll":{"question":"q","options":["a","b"]},"futureField":1}"#,
        )
        .unwrap();
        assert_eq!(body_of(&m).0, "[poll]");
    }

    #[test]
    fn chat_poll_capability_is_carried() {
        let label = |c: &BeeperChat| c.network.clone();
        let with: BeeperChat = serde_json::from_str(
            r#"{"id":"!a:b","accountID":"acc","network":"WhatsApp","title":"T","capabilities":{"poll":2}}"#,
        )
        .unwrap();
        assert_eq!(map_chat(&with, &label).poll_capability, Some(2));
        let without: BeeperChat = serde_json::from_str(
            r#"{"id":"!a:b","accountID":"acc","network":"WhatsApp","title":"T"}"#,
        )
        .unwrap();
        assert_eq!(map_chat(&without, &label).poll_capability, None);
    }

    #[test]
    fn chat_ids_are_url_encoded() {
        assert_eq!(encode_segment("!a:b.c/d e"), "%21a%3Ab.c%2Fd%20e");
    }
}
