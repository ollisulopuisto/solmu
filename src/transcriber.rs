//! Voice-note transcription core: jobs, a bounded on-disk transcript store,
//! and language resolution. No I/O to any network here - the engine lives
//! behind the `Engine` trait so the module is testable without one.
//!
//! Design rules (from the client's scope notes): local speech services only,
//! never a cloud API; the transcript store is bounded and private; a global
//! default language with an optional per-message override.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// One voice note waiting for (or finishing) transcription.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// Stable message id the transcript belongs to.
    pub message_id: String,
    /// Chat the message lives in, for store partitioning.
    pub chat_id: String,
    /// Local audio file to transcribe.
    pub audio: PathBuf,
    /// Duration from the attachment metadata, when known.
    pub duration_secs: Option<f64>,
    /// Per-message language override; `None` uses the global default.
    pub language: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JobState {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

/// The speech engine. Implemented per local backend (whisper.cpp CLI,
/// Wyoming service, parakeet-mlx); never a cloud endpoint.
pub trait Engine {
    /// Transcribes one audio file. `language` is a BCP-47-ish code the
    /// engine understands ("fi", "en", ...). Errors are definitive
    /// failures; a missing binary or unreadable file is not retried.
    fn transcribe(&mut self, audio: &Path, language: &str) -> Result<String, String>;
}

/// FIFO of jobs. Bounded so a history sync cannot queue unbounded work;
/// the oldest pending jobs are dropped first, never running ones.
pub struct JobQueue {
    pending: VecDeque<Job>,
    running: Option<Job>,
    done: usize,
    cap: usize,
}

impl JobQueue {
    pub fn new(cap: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            running: None,
            done: 0,
            cap: cap.max(1),
        }
    }

    /// Queues a job. Returns false when the same message is already queued
    /// or running; transcription of one voice note never duplicates.
    pub fn push(&mut self, job: Job) -> bool {
        let dup = self
            .pending
            .iter()
            .chain(self.running.iter())
            .any(|j| j.message_id == job.message_id);
        if dup {
            return false;
        }
        if self.pending.len() >= self.cap {
            self.pending.pop_front();
        }
        self.pending.push_back(job);
        true
    }

    /// Claims the next pending job as running.
    pub fn claim(&mut self) -> Option<Job> {
        let job = self.pending.pop_front()?;
        self.running = Some(job.clone());
        Some(job)
    }

    /// Completes the running job, successfully or not.
    pub fn finish(&mut self, ok: bool) {
        if self.running.take().is_some() && ok {
            self.done += 1;
        }
    }

    /// A crash with a job running requeues it once, at the front: audio is
    /// idempotent to re-read, unlike a sent message.
    pub fn recover(&mut self) {
        if let Some(job) = self.running.take() {
            self.pending.push_front(job);
        }
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Per-message language resolution: an explicit per-message choice wins,
/// then the global default, then the engine's own auto-detection.
pub fn resolve_language(per_message: Option<&str>, global: Option<&str>) -> Option<String> {
    per_message
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .or_else(|| global.map(str::trim).filter(|l| !l.is_empty()))
        .map(str::to_string)
}

/// On-disk transcripts: one text file per message under a per-chat dir,
/// bounded in total size with oldest-first eviction. The directory is
/// created 0700 and files 0600 - transcripts are message content.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    /// Total bytes the store may hold before evicting oldest.
    max_bytes: u64,
    /// Bumped on every mutation so cached searches invalidate.
    version: Arc<AtomicU64>,
}

impl Store {
    pub fn new(root: PathBuf, max_bytes: u64) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            root,
            max_bytes,
            version: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Mutation counter: search results are cached against it.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    fn file_for(&self, account_id: &str, chat_id: &str, message_id: &str) -> PathBuf {
        self.root
            .join(encode(account_id))
            .join(encode(chat_id))
            .join(format!("{}.txt", encode(message_id)))
    }

    pub fn get(&self, account_id: &str, chat_id: &str, message_id: &str) -> Option<String> {
        std::fs::read_to_string(self.file_for(account_id, chat_id, message_id)).ok()
    }

    /// Removes one transcript, e.g. when its message is deleted.
    pub fn remove(&self, account_id: &str, chat_id: &str, message_id: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.file_for(account_id, chat_id, message_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }?;
        self.bump();
        Ok(())
    }

    /// Privacy control: removes every transcript of one chat.
    pub fn remove_chat(&self, account_id: &str, chat_id: &str) -> std::io::Result<()> {
        let file = self.file_for(account_id, chat_id, "_");
        let dir = file.parent().map(Path::to_path_buf).unwrap_or(file);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }?;
        self.bump();
        Ok(())
    }

    /// Privacy control: removes every transcript of every chat.
    pub fn clear(&self) -> std::io::Result<()> {
        for entry in std::fs::read_dir(&self.root)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            }
        }
        self.bump();
        Ok(())
    }

    /// Writes one transcript and evicts oldest files past the cap.
    pub fn put(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        text: &str,
    ) -> std::io::Result<()> {
        let file = self.file_for(account_id, chat_id, message_id);
        use std::io::Write;
        for dir in [
            self.root.clone(),
            self.root.join(encode(account_id)),
            file.parent().unwrap().to_path_buf(),
        ] {
            if std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Transcript directory must not be a symlink",
                ));
            }
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        // tempfile creates a new private0600 file before any text is written.
        // Atomic replacement never follows a planted destination symlink.
        let mut temporary = tempfile::NamedTempFile::new_in(file.parent().unwrap())?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&file).map_err(|error| error.error)?;
        self.evict()?;
        self.bump();
        Ok(())
    }

    /// Evicts oldest-modified transcript files until under the cap.
    /// Symlinks are never followed or removed through: symlink_metadata
    /// keeps a planted link from pointing the store outside its root.
    fn evict(&self) -> std::io::Result<()> {
        let mut files: Vec<(u64, std::time::SystemTime, PathBuf)> = Vec::new();
        for account in std::fs::read_dir(&self.root)?.flatten() {
            if !account.path().is_dir() {
                continue;
            }
            for chat in std::fs::read_dir(account.path())?.flatten() {
                if !chat.path().is_dir() {
                    continue;
                }
                for entry in std::fs::read_dir(chat.path())?.flatten() {
                    // DirEntry::file_type never follows links, so a symlink
                    // is excluded before any metadata call touches its target.
                    if let Ok(file_type) = entry.file_type()
                        && file_type.is_file()
                        && let Ok(meta) = entry.metadata()
                    {
                        files.push((
                            meta.len(),
                            meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                            entry.path(),
                        ));
                    }
                }
            }
        }
        let mut total: u64 = files.iter().map(|(len, _, _)| len).sum();
        files.sort_by_key(|(_, modified, _)| *modified);
        for (len, _, path) in files {
            if total <= self.max_bytes {
                break;
            }
            std::fs::remove_file(&path)?;
            total = total.saturating_sub(len);
        }
        Ok(())
    }
}

/// Reversible filename encoding for store paths: ASCII alphanumerics and
/// `-_.` stay, every other UTF-8 byte percent-encodes, so walking the store
/// recovers the real account/chat/message ids a search hit navigates to.
fn encode(id: &str) -> String {
    if matches!(id, "" | "." | "..") {
        return id
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>()
            + if id.is_empty() { "%00" } else { "" };
    }
    let mut out = String::with_capacity(id.len());
    for &byte in id.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The inverse of [`encode`]; `None` for names it never produced.
fn decode(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = bytes.get(at + 1..at + 3)?;
            let text = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Subdirectory names directly under `dir`, skipping unreadable entries.
fn subdirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

/// One local transcript search hit, with the real ids needed to navigate
/// to the source message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub account_id: String,
    pub chat_id: String,
    pub message_id: String,
    /// The full transcript; the caller picks the matching line to show.
    pub text: String,
    /// File modification time in unix seconds, for newest-first ordering.
    pub modified: i64,
}

impl Store {
    /// Local transcript search: transcripts whose normalized text contains
    /// the normalized query, newest first, capped at `limit` hits. Text
    /// never leaves the machine; matching uses the same normalization as
    /// chat search, and hits carry the real account/chat/message ids.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        self.search_scoped(query, limit, None)
    }

    /// Filter account/chat before the cap, so another chat cannot hide hits.
    /// An empty scoped query returns saved transcripts for the Voice panel.
    pub fn search_scoped(
        &self,
        query: &str,
        limit: usize,
        scope: Option<(&str, &str)>,
    ) -> Vec<Hit> {
        let needle = crate::util::search_key(query.trim());
        let mut hits = Vec::new();
        if !needle.is_empty() || scope.is_some() {
            for account in subdirs(&self.root) {
                let Some(account_id) = decode(&account) else {
                    continue;
                };
                if scope.is_some_and(|(wanted, _)| wanted != account_id) {
                    continue;
                }
                for chat in subdirs(&self.root.join(&account)) {
                    let Some(chat_id) = decode(&chat) else {
                        continue;
                    };
                    if scope.is_some_and(|(_, wanted)| wanted != chat_id) {
                        continue;
                    }
                    let dir = self.root.join(&account).join(&chat);
                    let Ok(entries) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let Some(name) = name.to_str() else {
                            continue;
                        };
                        let Some(stem) = name.strip_suffix(".txt") else {
                            continue;
                        };
                        let Some(message_id) = decode(stem) else {
                            continue;
                        };
                        let Ok(text) = std::fs::read_to_string(entry.path()) else {
                            continue;
                        };
                        if !crate::util::search_key(&text).contains(&needle) {
                            continue;
                        }
                        let modified = entry
                            .metadata()
                            .ok()
                            .and_then(|meta| meta.modified().ok())
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|duration| duration.as_secs() as i64)
                            .unwrap_or(0);
                        hits.push(Hit {
                            account_id: account_id.clone(),
                            chat_id: chat_id.clone(),
                            message_id,
                            text,
                            modified,
                        });
                    }
                }
            }
        }
        hits.sort_by_key(|hit| std::cmp::Reverse(hit.modified));
        hits.truncate(limit);
        hits
    }
}

/// Converts mono f32 samples (-1..=1) at `from_rate` into Wyoming's wire
/// format: 16 kHz mono signed 16-bit little-endian. The app's voice
/// decode already yields mono 48 kHz, so the usual path is a clean 3:1
/// linear resample; other rates get the same linear treatment.
pub fn wyoming_pcm_from_mono(samples: &[f32], from_rate: u32) -> Vec<u8> {
    const TARGET: u32 = 16_000;
    if samples.is_empty() || from_rate == 0 {
        return Vec::new();
    }
    let out_len = (samples.len() as u64 * TARGET as u64 / from_rate as u64).max(1) as usize;
    let mut pcm = Vec::with_capacity(out_len * 2);
    for i in 0..out_len {
        // Map the output index back to a source position and interpolate.
        let pos = i as f64 * from_rate as f64 / TARGET as f64;
        let base = (pos as usize).min(samples.len() - 1);
        let next = (base + 1).min(samples.len() - 1);
        let frac = (pos - base as f64) as f32;
        let sample = samples[base] + (samples[next] - samples[base]) * frac;
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    pcm
}

/// Preview selection uses the same normalization as local transcript search.
pub fn preview(text: &str, query: &str) -> String {
    let needle = crate::util::search_key(query.trim());
    text.lines()
        .find(|line| crate::util::search_key(line).contains(&needle))
        .unwrap_or_else(|| text.lines().next().unwrap_or(""))
        .trim()
        .to_owned()
}

pub const SNAPSHOT_HITS: usize = 500;
pub const SNAPSHOT_TEXT_BYTES: usize = 2 << 20;
pub const SNAPSHOT_HIT_BYTES: usize = 64 << 10;
const MAX_SOURCE_BYTES: u64 = 32 << 20;

#[derive(Default)]
pub struct Snapshot {
    pub hits: Vec<Hit>,
    pub truncated: bool,
}

impl Store {
    /// A bounded display snapshot, not a second archive. Search all ordinary
    /// source files before applying the result cap; retain only bounded text.
    /// Oversized external files are skipped and the partial-state notice stays on.
    pub fn snapshot(&self, query: &str, account: &str, chat: &str) -> Snapshot {
        self.snapshot_while(query, account, chat, || true)
    }

    /// Account-wide search uses the same bounded snapshots, filtering before caps.
    pub fn account_snapshot_while(
        &self,
        query: &str,
        account: &str,
        current: impl Fn() -> bool,
    ) -> Snapshot {
        let mut out = Snapshot::default();
        if query.trim().is_empty() {
            return out;
        }
        let mut retained = std::collections::BTreeMap::new();
        let mut bytes = 0;
        for chat in subdirs(&self.root.join(encode(account))) {
            if !current() {
                return Snapshot::default();
            }
            let Some(chat) = decode(&chat) else {
                continue;
            };
            let snapshot = self.snapshot_while(query, account, &chat, &current);
            out.truncated |= snapshot.truncated;
            for hit in snapshot.hits {
                bytes += hit.text.len();
                retained.insert(
                    (
                        std::cmp::Reverse(hit.modified),
                        hit.chat_id.clone(),
                        hit.message_id.clone(),
                    ),
                    hit,
                );
                while retained.len() > SNAPSHOT_HITS || bytes > SNAPSHOT_TEXT_BYTES {
                    if let Some((_, hit)) = retained.pop_last() {
                        bytes -= hit.text.len();
                        out.truncated = true;
                    }
                }
            }
        }
        out.hits = retained.into_values().collect();
        out
    }

    pub fn snapshot_while(
        &self,
        query: &str,
        account: &str,
        chat: &str,
        current: impl Fn() -> bool,
    ) -> Snapshot {
        use std::io::Read;
        let mut retained = std::collections::BTreeMap::new();
        let mut retained_bytes = 0;
        let mut out = Snapshot::default();
        let needle = crate::util::search_key(query.trim());
        let dir = self.root.join(encode(account)).join(encode(chat));
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            if !current() {
                return Snapshot::default();
            }
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".txt")) else {
                continue;
            };
            let Some(message_id) = decode(stem) else {
                continue;
            };
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(file) = std::fs::File::open(entry.path()) else {
                continue;
            };
            let modified = file
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |time| time.as_secs() as i64);
            let mut bytes = Vec::new();
            if file
                .take(MAX_SOURCE_BYTES + 1)
                .read_to_end(&mut bytes)
                .is_err()
            {
                continue;
            }
            if bytes.len() as u64 > MAX_SOURCE_BYTES {
                out.truncated = true;
                continue;
            }
            let Ok(mut text) = String::from_utf8(bytes) else {
                continue;
            };
            if !crate::util::search_key(&text).contains(&needle) {
                continue;
            }
            if text.len() > SNAPSHOT_HIT_BYTES {
                out.truncated = true;
                // Keep the matching line near the start of a clipped result.
                let matching = preview(&text, query);
                text = if needle.is_empty() { text } else { matching };
                let mut end = text.len().min(SNAPSHOT_HIT_BYTES);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            let hit = Hit {
                account_id: account.into(),
                chat_id: chat.into(),
                message_id,
                text,
                modified,
            };
            retained_bytes += hit.text.len();
            retained.insert(
                (std::cmp::Reverse(hit.modified), hit.message_id.clone()),
                hit,
            );
            while retained.len() > SNAPSHOT_HITS || retained_bytes > SNAPSHOT_TEXT_BYTES {
                if let Some((_, removed)) = retained.pop_last() {
                    retained_bytes -= removed.text.len();
                    out.truncated = true;
                }
            }
        }
        out.hits = retained.into_values().collect();
        out
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn private_atomic_writes_harden_parents_and_never_follow_file_links() {
        use std::os::unix::{fs::PermissionsExt, fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("store"), 1 << 20).unwrap();
        store
            .put("account", "chat", "message", "private words")
            .unwrap();
        let file = store.file_for("account", "chat", "message");
        for dir in [
            root.path().join("store"),
            root.path().join("store/account"),
            file.parent().unwrap().into(),
        ] {
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let outside = root.path().join("outside");
        std::fs::write(&outside, "untouched").unwrap();
        std::fs::remove_file(&file).unwrap();
        symlink(&outside, &file).unwrap();
        store
            .put("account", "chat", "message", "replacement")
            .unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "untouched");
        let link = root.path().join("store/linked");
        symlink(root.path(), link).unwrap();
        assert!(store.put("linked", "chat", "message", "no").is_err());
        assert!(
            !store
                .file_for("..", ".", "message")
                .to_string_lossy()
                .contains("/../")
        );
    }

    #[test]
    fn scoped_snapshot_bounds_text_and_hits_and_searches_before_caps() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), u64::MAX).unwrap();
        let file = store.file_for("acc", "chat", "seed");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for i in 0..520 {
            std::fs::write(
                store.file_for("acc", "chat", &format!("m{i:04}")),
                "ordinary",
            )
            .unwrap();
        }
        std::fs::write(store.file_for("acc", "chat", "z-match"), "needle").unwrap();
        let snapshot = store.snapshot("", "acc", "chat");
        assert_eq!(snapshot.hits.len(), SNAPSHOT_HITS);
        assert!(snapshot.truncated);
        let found = store.snapshot("needle", "acc", "chat");
        assert_eq!(found.hits.len(), 1, "query must run before result cap");
        assert_eq!(found.hits[0].message_id, "z-match");
        for i in 0..40 {
            std::fs::write(
                store.file_for("acc", "large", &format!("m{i}")),
                "ää".repeat(40_000),
            )
            .unwrap_or_else(|_| {
                std::fs::create_dir_all(store.file_for("acc", "large", "x").parent().unwrap())
                    .unwrap();
                std::fs::write(
                    store.file_for("acc", "large", &format!("m{i}")),
                    "ää".repeat(40_000),
                )
                .unwrap();
            });
        }
        let snapshot = store.snapshot("", "acc", "large");
        assert!(snapshot.truncated);
        assert!(
            snapshot
                .hits
                .iter()
                .all(|h| h.text.len() <= SNAPSHOT_HIT_BYTES)
        );
        assert!(snapshot.hits.iter().map(|h| h.text.len()).sum::<usize>() <= SNAPSHOT_TEXT_BYTES);
        assert!(store.snapshot("needle", "other", "chat").hits.is_empty());
    }

    #[test]
    fn obsolete_snapshot_stops_before_reading_files() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), 1 << 20).unwrap();
        store.put("acc", "chat", "one", "station").unwrap();
        let snapshot = store.snapshot_while("station", "acc", "chat", || false);
        assert!(snapshot.hits.is_empty());
    }

    #[test]
    fn preview_uses_search_normalization() {
        assert_eq!(
            super::preview("First line.\nMeet at the café.", "cafe"),
            "Meet at the café."
        );
    }
    use super::*;

    fn job(id: &str) -> Job {
        Job {
            message_id: id.into(),
            chat_id: "c1".into(),
            audio: PathBuf::from("/tmp/x.ogg"),
            duration_secs: Some(3.0),
            language: None,
        }
    }

    #[test]
    fn queue_rejects_duplicate_message() {
        let mut q = JobQueue::new(10);
        assert!(q.push(job("m1")));
        assert!(!q.push(job("m1")));
        let claimed = q.claim().unwrap();
        assert_eq!(claimed.message_id, "m1");
        assert!(!q.push(job("m1"))); // running still counts
    }

    #[test]
    fn queue_drops_oldest_pending_past_cap() {
        let mut q = JobQueue::new(2);
        q.push(job("m1"));
        q.push(job("m2"));
        q.push(job("m3"));
        assert_eq!(q.pending_len(), 2);
        assert_eq!(q.claim().unwrap().message_id, "m2");
    }

    #[test]
    fn crash_requeues_running_job_once_at_front() {
        let mut q = JobQueue::new(10);
        q.push(job("m1"));
        q.push(job("m2"));
        q.claim();
        q.recover();
        assert_eq!(q.claim().unwrap().message_id, "m1");
    }

    #[test]
    fn language_prefers_message_then_global_then_none() {
        assert_eq!(
            resolve_language(Some("fi"), Some("en")).as_deref(),
            Some("fi")
        );
        assert_eq!(resolve_language(None, Some("en")).as_deref(), Some("en"));
        assert_eq!(
            resolve_language(Some("  "), Some("en")).as_deref(),
            Some("en")
        );
        assert_eq!(resolve_language(None, None), None);
    }

    #[test]
    fn store_roundtrip_and_unsafe_ids() {
        let dir = std::env::temp_dir().join(format!("tr-store-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        store
            .put("acc-1", "chat 9/4", "m!1", "hei maailma")
            .unwrap();
        assert_eq!(
            store.get("acc-1", "chat 9/4", "m!1").as_deref(),
            Some("hei maailma")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_evicts_oldest_past_cap() {
        let dir = std::env::temp_dir().join(format!("tr-evict-{}", std::process::id()));
        let store = Store::new(dir.clone(), 10).unwrap();
        store.put("a", "c", "old", "12345678").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.put("a", "c", "new", "12345678").unwrap();
        assert_eq!(store.get("a", "c", "old"), None);
        assert_eq!(store.get("a", "c", "new").as_deref(), Some("12345678"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_isolates_accounts_and_chats() {
        let dir = std::env::temp_dir().join(format!("tr-iso-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        store.put("acc-1", "c", "m", "one").unwrap();
        store.put("acc-2", "c", "m", "two").unwrap();
        store.put("acc-1", "d", "m", "three").unwrap();
        assert_eq!(store.get("acc-1", "c", "m").as_deref(), Some("one"));
        assert_eq!(store.get("acc-2", "c", "m").as_deref(), Some("two"));
        assert_eq!(store.get("acc-1", "d", "m").as_deref(), Some("three"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encode_decode_roundtrips_real_ids() {
        for id in [
            "plain-1",
            "358401234567-987654321@g.us",
            "chat 9/4",
            "m!1",
            "100%.txt",
            "v\u{E4}\u{E4}r\u{E4}\u{E4} sn\u{F6}\u{F6}",
        ] {
            assert_eq!(decode(&encode(id)).as_deref(), Some(id), "id {id:?}");
        }
        assert_eq!(decode("not%a%real"), None);
        assert_eq!(decode("trailing%4"), None);
    }

    #[test]
    fn search_finds_and_isolates_transcripts() {
        let dir = std::env::temp_dir().join(format!("tr-search-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        store
            .put(
                "acc-1",
                "358401-987@g.us",
                "m!1",
                "Hei maailma\nsecond line",
            )
            .unwrap();
        store
            .put("acc-1", "other@s.whatsapp.net", "m2", "unrelated")
            .unwrap();
        store
            .put("acc-2", "358401-987@g.us", "m!1", "MAAILMA again")
            .unwrap();

        let hits = store.search("maailma", 50);
        assert_eq!(hits.len(), 2);
        // Real, decoded ids come back for navigation, per account and chat.
        assert!(hits.iter().any(|hit| {
            hit.account_id == "acc-1"
                && hit.chat_id == "358401-987@g.us"
                && hit.message_id == "m!1"
                && hit.text.contains("Hei maailma")
        }));
        assert!(hits.iter().any(|hit| hit.account_id == "acc-2"));

        // No match, blank query, and the cap all behave.
        assert!(store.search("absent", 50).is_empty());
        assert!(store.search("   ", 50).is_empty());
        assert_eq!(store.search("maailma", 1).len(), 1);

        // Deletion takes a transcript out of search.
        store.remove_chat("acc-1", "358401-987@g.us").unwrap();
        let hits = store.search("maailma", 50);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].account_id, "acc-2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn version_counts_mutations() {
        let dir = std::env::temp_dir().join(format!("tr-ver-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        let start = store.version();
        store.put("a", "c", "m", "x").unwrap();
        assert!(store.version() > start);
        let after_put = store.version();
        store.get("a", "c", "m");
        assert_eq!(store.version(), after_put, "reads do not mutate");
        store.remove("a", "c", "m").unwrap();
        assert!(store.version() > after_put);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_delete_controls() {
        let dir = std::env::temp_dir().join(format!("tr-del-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        store.put("a", "c1", "m1", "x").unwrap();
        store.put("a", "c1", "m2", "y").unwrap();
        store.put("a", "c2", "m3", "z").unwrap();
        store.remove("a", "c1", "m1").unwrap();
        assert_eq!(store.get("a", "c1", "m1"), None);
        assert_eq!(store.get("a", "c1", "m2").as_deref(), Some("y"));
        store.remove_chat("a", "c1").unwrap();
        assert_eq!(store.get("a", "c1", "m2"), None);
        assert_eq!(store.get("a", "c2", "m3").as_deref(), Some("z"));
        store.clear().unwrap();
        assert_eq!(store.get("a", "c2", "m3"), None);
        // Deleting what is absent is not an error.
        store.remove("a", "c1", "m1").unwrap();
        store.remove_chat("a", "gone").unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn store_eviction_ignores_symlinks() {
        let dir = std::env::temp_dir().join(format!("tr-sym-{}", std::process::id()));
        let store = Store::new(dir.clone(), 10).unwrap();
        store.put("a", "c", "real", "12345678").unwrap();
        // A planted symlink inside the store pointing outside it: eviction
        // must neither follow nor delete through it.
        let outside = std::env::temp_dir().join(format!("tr-outside-{}", std::process::id()));
        std::fs::write(&outside, "keep me").unwrap();
        std::os::unix::fs::symlink(&outside, store.file_for("a", "c", "link")).unwrap();
        store.put("a", "c", "new", "12345678").unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep me");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn store_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tr-perms-{}", std::process::id()));
        let store = Store::new(dir.clone(), 1 << 20).unwrap();
        store.put("a", "c", "m", "secret words").unwrap();
        let mode = std::fs::metadata(store.file_for("a", "c", "m"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wyoming_pcm_downsamples_48k_to_16k() {
        // 48 samples at 48 kHz = 1 ms of audio -> 16 samples at 16 kHz.
        let samples: Vec<f32> = (0..48).map(|i| i as f32 / 48.0).collect();
        let pcm = wyoming_pcm_from_mono(&samples, 48_000);
        assert_eq!(pcm.len(), 16 * 2);
        // Little-endian i16 of the first sample (0.0) and a later one.
        assert_eq!(&pcm[0..2], &0i16.to_le_bytes());
        let mid = i16::from_le_bytes([pcm[20], pcm[21]]);
        assert!(mid > 0);
    }

    #[test]
    fn wyoming_pcm_clamps_and_handles_edge_cases() {
        let loud = wyoming_pcm_from_mono(&[2.0; 48], 48_000);
        assert_eq!(&loud[0..2], &i16::MAX.to_le_bytes());
        let quiet = wyoming_pcm_from_mono(&[-3.0; 48], 48_000);
        assert_eq!(&quiet[0..2], &(-i16::MAX).to_le_bytes());
        assert!(wyoming_pcm_from_mono(&[], 48_000).is_empty());
        assert!(wyoming_pcm_from_mono(&[0.5], 0).is_empty());
        // Same rate in and out still converts to s16le.
        let same = wyoming_pcm_from_mono(&[0.5, -0.5], 16_000);
        assert_eq!(same.len(), 4);
    }

    struct EchoEngine;
    impl Engine for EchoEngine {
        fn transcribe(&mut self, audio: &Path, language: &str) -> Result<String, String> {
            Ok(format!("{}:{}", language, audio.display()))
        }
    }

    #[test]
    fn engine_trait_object_is_usable() {
        let mut engine: Box<dyn Engine> = Box::new(EchoEngine);
        assert_eq!(
            engine.transcribe(Path::new("/a.ogg"), "fi").unwrap(),
            "fi:/a.ogg"
        );
    }
}
