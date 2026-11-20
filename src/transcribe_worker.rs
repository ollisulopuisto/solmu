//! The transcription worker: one background thread owning the engine and
//! the store, fed voice-note jobs over a channel, reporting per-message
//! status the conversation view reads each frame. Audio never leaves the
//! machine: the engine is a local service or binary behind `Engine`.
//!
//! Deletion semantics are immediate and generation-checked, not queued:
//! remove_chat/clear bump a shared generation, purge statuses, and delete
//! the files on the calling thread. Every job carries the generation it
//! was queued under; when its engine finishes, the result stores and
//! renders only if the generation is still current - a delete, a clear,
//! a re-transcribe after re-adding, or the worker's own drop (a settings
//! respawn) invalidates the late result for good.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::transcriber::{Engine, Hit, Job, JobQueue, Store, resolve_language};

/// What the bubble shows for one message's transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    /// The transcript text, hot in memory; the store keeps the durable copy.
    Ready(String),
    Failed(String),
}

/// Status map key: (account_id, chat_id, message_id). Message ids are
/// unique within a chat, chat ids within an account; the account half
/// keeps same-shaped ids of two accounts from colliding in memory.
type Key = (String, String, String);

/// Deletion state shared between the calling thread and the worker
/// thread. A job's captured generation goes stale the moment its chat is
/// removed or everything is cleared, so only the freshest job for a chat
/// may store or render - deleting and re-adding a chat revives nothing
/// old.
#[derive(Default)]
pub struct Generations {
    /// (account, chat) -> how many times it was deleted.
    chats: HashMap<(String, String), u64>,
    accounts: HashMap<String, u64>,
    /// How many times everything was deleted.
    clears: u64,
}

impl Generations {
    /// The (clear, chat) generation a new job for this chat captures.
    fn current(&self, account: &str, chat: &str) -> (u64, u64, u64) {
        (
            self.clears,
            self.accounts.get(account).copied().unwrap_or(0),
            self.chats
                .get(&(account.to_owned(), chat.to_owned()))
                .copied()
                .unwrap_or(0),
        )
    }
}

/// Privacy commands ride the same channel so the worker thread can wake
/// the view; their effects already landed on the calling thread.
pub enum Command {
    Transcribe {
        job: Job,
        account_id: String,
        global_language: Option<String>,
        /// (clear, chat) generation at queue time.
        generation: (u64, u64, u64),
    },
    RemoveChat {
        account_id: String,
        chat_id: String,
    },
    Clear,
    Shutdown,
}

/// State the worker thread shares with its handle: statuses, deletion
/// generations, the store-mutation lock, and the alive flag.
pub struct Shared {
    /// (account_id, chat_id, message_id) -> status, read by the view.
    pub status: Arc<Mutex<HashMap<Key, Status>>>,
    /// Deletion generations, checked when in-flight work finishes.
    generations: Arc<Mutex<Generations>>,
    /// Serializes store mutations against completions: a removal and a
    /// finishing job's check + put + status can never interleave.
    store_io: Arc<Mutex<()>>,
    /// False once the worker is dropped (respawn on a settings change):
    /// the old thread's late result is dropped instead of stored.
    alive: Arc<AtomicBool>,
}

pub struct Worker {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    /// The store search runs against; `None` on stub workers.
    store: Option<Store>,
    /// Search memo: (store version, query, hits).
    search_cache: SearchCache,
    scoped_search_cache: ScopedSearchCache,
    restored: RestoredCache,
    active_scans: Arc<AtomicUsize>,
    restore_scans: Arc<AtomicUsize>,
    wake_search: Arc<dyn Fn() + Send + Sync>,
    /// What a stub was told, for tests; always empty on a live worker.
    recorded: Arc<Mutex<Vec<String>>>,
}

/// Search memo: (store version, query, hits).
type SearchCache = Arc<Mutex<Option<(u64, String, Vec<Hit>)>>>;
// Store-versioned restoration entries: None is in flight; Some(None) is absent.
type RestoredCache = Arc<Mutex<VecDeque<(u64, Key, Option<Option<String>>)>>>;
const SCOPED_CACHE_SLOTS: usize = 4;
struct ScopedSnapshot {
    version: u64,
    key: (String, String, String, bool),
    hits: Option<Arc<[Hit]>>,
    truncated: bool,
}
type ScopedSearchCache = Arc<Mutex<VecDeque<ScopedSnapshot>>>;

/// Releases admission and unsettles a failed request even if scanning panics.
struct ScanPermit {
    active: Arc<AtomicUsize>,
    cache: ScopedSearchCache,
    version: u64,
    key: (String, String, String, bool),
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Drop for ScanPermit {
    fn drop(&mut self) {
        self.cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|entry| {
                !(entry.version == self.version && entry.key == self.key && entry.hits.is_none())
            });
        self.active.fetch_sub(1, Ordering::Relaxed);
        (self.wake)();
    }
}

/// The loop: drain commands until Shutdown or the channel closes. `wake`
/// repaints the view after each status change.
fn run(
    rx: Receiver<Command>,
    mut engine: Box<dyn Engine + Send>,
    store: Store,
    shared: Arc<Shared>,
    wake: impl Fn(),
) {
    let Shared {
        status,
        generations,
        store_io,
        alive,
    } = &*shared;
    let status = Arc::clone(status);
    let generations = Arc::clone(generations);
    let store_io = Arc::clone(store_io);
    let alive = Arc::clone(alive);
    let mut queue = JobQueue::new(64);
    while let Ok(command) = rx.recv() {
        match command {
            Command::Transcribe {
                job,
                account_id,
                global_language,
                generation,
            } => {
                if !queue.push(job) {
                    continue;
                }
                // One at a time: engines are local models; parallel runs
                // would thrash the machine for no latency win.
                while let Some(job) = queue.claim() {
                    let key = (
                        account_id.clone(),
                        job.chat_id.clone(),
                        job.message_id.clone(),
                    );
                    let current = |generations: &Arc<Mutex<Generations>>| {
                        generations
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .current(&account_id, &job.chat_id)
                    };
                    if current(&generations) != generation {
                        // Its chat was deleted before the job even started.
                        queue.finish(true);
                        continue;
                    }
                    let language =
                        resolve_language(job.language.as_deref(), global_language.as_deref());
                    set(&status, &key, Status::Running);
                    wake();
                    let result = engine.transcribe(&job.audio, language.as_deref().unwrap_or(""));
                    // The decisive section, under store_io: a delete/clear
                    // either already landed (generation mismatch: drop) or
                    // lands right after us (it purges what we set and
                    // wrote). The two orders are both final-correct.
                    let done = {
                        let _io = store_io.lock().unwrap_or_else(|p| p.into_inner());
                        if !alive.load(Ordering::Relaxed) || current(&generations) != generation {
                            None
                        } else {
                            let result = result.and_then(|text| {
                                store
                                    .put(&account_id, &job.chat_id, &job.message_id, &text)
                                    .map_err(|e| format!("transcript store: {e}"))?;
                                Ok(text)
                            });
                            let state = match result {
                                Ok(text) => Status::Ready(text),
                                Err(error) => Status::Failed(error),
                            };
                            let done = matches!(state, Status::Ready(_));
                            set(&status, &key, state);
                            Some(done)
                        }
                    };
                    match done {
                        Some(done) => queue.finish(done),
                        None => {
                            drop_status(&status, &key);
                            queue.finish(true);
                        }
                    }
                    wake();
                }
            }
            Command::RemoveChat { .. } | Command::Clear => {
                // The effects landed on the calling thread; this is only
                // the view's repaint.
                wake();
            }
            Command::Shutdown => return,
        }
    }
}

fn set(status: &Arc<Mutex<HashMap<Key, Status>>>, key: &Key, state: Status) {
    status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key.clone(), state);
}

fn drop_status(status: &Arc<Mutex<HashMap<Key, Status>>>, key: &Key) {
    status.lock().unwrap_or_else(|p| p.into_inner()).remove(key);
}

impl Worker {
    /// Spawns the worker thread. `wake` must repaint the conversation view.
    pub fn spawn(
        engine: Box<dyn Engine + Send>,
        store: Store,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            status: Arc::new(Mutex::new(HashMap::new())),
            generations: Arc::new(Mutex::new(Generations::default())),
            store_io: Arc::new(Mutex::new(())),
            alive: Arc::new(AtomicBool::new(true)),
        });
        let thread_shared = Arc::clone(&shared);
        let thread_store = store.clone();
        let wake = Arc::new(wake);
        let wake_search = wake.clone();
        std::thread::Builder::new()
            .name("transcribe".to_string())
            .spawn(move || run(rx, engine, thread_store, thread_shared, move || wake()))
            .expect("transcribe worker thread");
        Self {
            tx,
            shared,
            store: Some(store),
            search_cache: Arc::new(Mutex::new(None)),
            scoped_search_cache: Arc::new(Mutex::new(VecDeque::new())),
            restored: Arc::new(Mutex::new(VecDeque::new())),
            active_scans: Arc::new(AtomicUsize::new(0)),
            restore_scans: Arc::new(AtomicUsize::new(0)),
            wake_search,
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Queues one voice note, capturing the chat's deletion generation:
    /// only if nothing is deleted before the result lands does it store
    /// or render. Duplicate message ids are dropped in the queue; a Ready
    /// status short-circuits before a job is even sent.
    pub fn transcribe(&self, job: Job, account_id: String, global_language: Option<String>) {
        let generation = self
            .shared
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .current(&account_id, &job.chat_id);
        let known = self
            .shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(
                account_id.clone(),
                job.chat_id.clone(),
                job.message_id.clone(),
            ))
            .cloned();
        if matches!(known, Some(Status::Ready(_)) | Some(Status::Running)) {
            return;
        }
        let _ = self.tx.send(Command::Transcribe {
            job,
            account_id,
            global_language,
            generation,
        });
    }

    /// Deletes one chat's transcripts, effective on return: the
    /// generation bumps, the statuses purge, and the files go - before
    /// any in-flight engine for the chat can finish, and out of search.
    pub fn remove_chat(&self, account_id: String, chat_id: String) {
        let _io = self
            .shared
            .store_io
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.shared
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .chats
            .entry((account_id.clone(), chat_id.clone()))
            .and_modify(|n| *n += 1)
            .or_insert(1);
        self.shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(account, chat, _), _| !(account == &account_id && chat == &chat_id));
        if let Some(store) = &self.store {
            let _ = store.remove_chat(&account_id, &chat_id);
        }
        drop(_io);
        let _ = self.tx.send(Command::RemoveChat {
            account_id,
            chat_id,
        });
    }

    /// Logout/removal invalidates every queued/restoring result for this account.
    pub fn remove_account(&self, account: String) -> Result<(), String> {
        let _io = self
            .shared
            .store_io
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *self
            .shared
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .accounts
            .entry(account.clone())
            .or_default() += 1;
        self.shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(owner, _, _), _| owner != &account);
        self.restored
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(_, key, _)| key.0 != account);
        let result = self.store.as_ref().map(|store| store.remove_account(&account)).transpose().map_err(|_| "Could not remove this account's saved transcripts. Its encrypted archive may still contain them.".to_owned());
        drop(_io);
        let _ = self.tx.send(Command::Clear);
        result.map(|_| ())
    }

    /// Deletes every transcript, effective on return: same immediate
    /// semantics as remove_chat, for all accounts and chats.
    pub fn clear(&self) {
        let _io = self
            .shared
            .store_io
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.shared
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clears += 1;
        self.shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        if let Some(store) = &self.store {
            let _ = store.clear();
        }
        drop(_io);
        let _ = self.tx.send(Command::Clear);
    }

    /// Local transcript search over the store, newest first. Cached on the
    /// store version so repaints don't rescan disk; a deletion bumps the
    /// version on the calling thread, so deleted transcripts leave the
    /// results immediately. Callers scope the hits to their account.
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let Some(store) = &self.store else {
            return Vec::new();
        };
        let key = query.trim().to_owned();
        if key.is_empty() {
            return Vec::new();
        }
        let version = store.version();
        {
            let cache = self.search_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((cached_version, cached_key, hits)) = &*cache
                && *cached_version == version
                && *cached_key == key
            {
                return hits.clone();
            }
        }
        let hits = store.search(&key, 50);
        *self.search_cache.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((version, key, hits.clone()));
        hits
    }

    /// Saved text only: never queues the engine or downloads audio. A
    /// versioned scoped memo keeps repeated paint passes off disk.
    pub fn saved_in_chat(&self, account: &str, chat: &str, query: &str) -> Arc<[Hit]> {
        self.saved_scope(account, chat, query, false)
    }

    pub fn saved_for_account(&self, account: &str, query: &str) -> Arc<[Hit]> {
        if query.trim().is_empty() {
            return Arc::default();
        }
        self.saved_scope(account, "", query, true)
    }

    fn saved_scope(&self, account: &str, chat: &str, query: &str, all_chats: bool) -> Arc<[Hit]> {
        let Some(store) = &self.store else {
            return self
                .shared
                .status
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .filter_map(|((a, c, m), status)| {
                    if a != account || (!all_chats && c != chat) {
                        return None;
                    }
                    let Status::Ready(text) = status else {
                        return None;
                    };
                    crate::util::search_key(text)
                        .contains(&crate::util::search_key(query))
                        .then(|| Hit {
                            account_id: a.clone(),
                            chat_id: c.clone(),
                            message_id: m.clone(),
                            text: text.clone(),
                            modified: 0,
                        })
                })
                .collect();
        };
        let version = store.version();
        let key = (
            account.to_owned(),
            chat.to_owned(),
            crate::util::search_key(query.trim()),
            all_chats,
        );
        let mut memo = self
            .scoped_search_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        memo.retain(|entry| entry.version == version);
        if let Some(entry) = memo.iter().find(|entry| entry.key == key) {
            return entry.hits.clone().unwrap_or_default();
        }
        if self.active_scans.load(Ordering::Relaxed) >= SCOPED_CACHE_SLOTS {
            return Arc::default();
        }
        if memo.len() >= SCOPED_CACHE_SLOTS {
            if let Some(index) = memo.iter().position(|entry| entry.hits.is_some()) {
                memo.remove(index);
            } else {
                return Arc::default();
            }
        }
        memo.push_back(ScopedSnapshot {
            version,
            key: key.clone(),
            hits: None,
            truncated: false,
        });
        drop(memo);
        self.active_scans.fetch_add(1, Ordering::Relaxed);
        let active_scans = Arc::clone(&self.active_scans);
        let store = store.clone();
        let cache = Arc::clone(&self.scoped_search_cache);
        let shared = Arc::clone(&self.shared);
        let wake = Arc::clone(&self.wake_search);
        let permit = ScanPermit {
            active: active_scans,
            cache: cache.clone(),
            version,
            key: key.clone(),
            wake,
        };
        std::thread::spawn(move || {
            let _permit = permit;
            let snapshot = {
                let _io = shared.store_io.lock().unwrap_or_else(|p| p.into_inner());
                let current = || {
                    shared.alive.load(Ordering::Relaxed)
                        && store.version() == version
                        && cache
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .iter()
                            .any(|entry| entry.version == version && entry.key == key)
                };
                if key.3 {
                    store.account_snapshot_while(&key.2, &key.0, current)
                } else {
                    store.snapshot_while(&key.2, &key.0, &key.1, current)
                }
            };
            let mut memo = cache.lock().unwrap_or_else(|p| p.into_inner());
            // A stale completion wakes once so the next frame can request the
            // current version. It never replaces newer text or revives deletion.
            if store.version() == version
                && shared.alive.load(Ordering::Relaxed)
                && let Some(entry) = memo
                    .iter_mut()
                    .find(|entry| entry.version == version && entry.key == key)
            {
                entry.hits = Some(snapshot.hits.into());
                entry.truncated = snapshot.truncated;
            }
            drop(memo);
        });
        Arc::default()
    }

    pub fn saved_loading(&self, account: &str, chat: &str, query: &str) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let key = (
            account.to_owned(),
            chat.to_owned(),
            crate::util::search_key(query.trim()),
            false,
        );
        self.scoped_search_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|entry| entry.version == store.version() && entry.key == key)
            .is_none_or(|entry| entry.hits.is_none())
    }

    pub fn saved_truncated(&self, account: &str, chat: &str, query: &str) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let key = (
            account.to_owned(),
            chat.to_owned(),
            crate::util::search_key(query.trim()),
            false,
        );
        self.scoped_search_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|entry| entry.version == store.version() && entry.key == key && entry.truncated)
    }

    pub fn saved_account_state(&self, account: &str, query: &str) -> (bool, bool) {
        let Some(store) = &self.store else {
            return (false, false);
        };
        let key = (
            account.to_owned(),
            String::new(),
            crate::util::search_key(query.trim()),
            true,
        );
        self.scoped_search_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|entry| entry.version == store.version() && entry.key == key)
            .map_or((true, false), |entry| {
                (entry.hits.is_none(), entry.truncated)
            })
    }

    /// Read/delete access to saved text without an engine or worker thread.
    /// Its channel has no receiver, so it cannot ever start inference.
    pub fn archive(store: Store) -> Self {
        Self::archive_with_wake(store, || {})
    }

    pub fn archive_with_wake(store: Store, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, _rx) = channel();
        Self {
            tx,
            shared: Arc::new(Shared {
                status: Arc::new(Mutex::new(HashMap::new())),
                generations: Arc::new(Mutex::new(Generations::default())),
                store_io: Arc::new(Mutex::new(())),
                alive: Arc::new(AtomicBool::new(true)),
            }),
            store: Some(store),
            search_cache: Arc::new(Mutex::new(None)),
            scoped_search_cache: Arc::new(Mutex::new(VecDeque::new())),
            restored: Arc::new(Mutex::new(VecDeque::new())),
            active_scans: Arc::new(AtomicUsize::new(0)),
            restore_scans: Arc::new(AtomicUsize::new(0)),
            wake_search: Arc::new(wake),
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A worker with no engine behind it, for demos and tests: the given
    /// statuses are what the view sees; queued work is only recorded.
    pub fn stub(pairs: impl IntoIterator<Item = (Key, Status)>) -> Self {
        let (tx, rx) = channel();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&recorded);
        std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                let line = match &command {
                    Command::Transcribe {
                        job,
                        account_id,
                        global_language,
                        ..
                    } => format!(
                        "transcribe {} in {} as {account_id} (global {global_language:?}, message {:?}) from {}",
                        job.message_id,
                        job.chat_id,
                        job.language,
                        job.audio.display()
                    ),
                    Command::RemoveChat {
                        account_id,
                        chat_id,
                    } => format!("remove-chat {chat_id} of {account_id}"),
                    Command::Clear => "clear".to_owned(),
                    Command::Shutdown => "shutdown".to_owned(),
                };
                log.lock().unwrap_or_else(|p| p.into_inner()).push(line);
            }
        });
        Self {
            tx,
            shared: Arc::new(Shared {
                status: Arc::new(Mutex::new(pairs.into_iter().collect())),
                generations: Arc::new(Mutex::new(Generations::default())),
                store_io: Arc::new(Mutex::new(())),
                alive: Arc::new(AtomicBool::new(true)),
            }),
            store: None,
            search_cache: Arc::new(Mutex::new(None)),
            scoped_search_cache: Arc::new(Mutex::new(VecDeque::new())),
            restored: Arc::new(Mutex::new(VecDeque::new())),
            active_scans: Arc::new(AtomicUsize::new(0)),
            restore_scans: Arc::new(AtomicUsize::new(0)),
            wake_search: Arc::new(|| {}),
            recorded,
        }
    }

    /// A stub whose store is real, for search tests: no engine behind it,
    /// but the given transcripts are searchable.
    #[cfg(test)]
    pub fn stub_with_store(store: Store) -> Self {
        let mut worker = Self::stub([]);
        worker.store = Some(store);
        worker
    }

    /// What a stub was told, for tests. The draining thread parks on the
    /// channel; callers poll until the expectation holds. Available in
    /// every build so the field is used outside test targets too.
    pub fn recorded(&self) -> Vec<String> {
        self.recorded
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Status for one message. Cold saved text loads off the paint thread;
    /// both hits and misses are memoized until the store changes.
    pub fn get(&self, account_id: &str, chat_id: &str, message_id: &str) -> Option<Status> {
        let key = (
            account_id.to_owned(),
            chat_id.to_owned(),
            message_id.to_owned(),
        );
        if let Some(status) = self
            .shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned()
        {
            return Some(status);
        }
        let store = self.store.as_ref()?;
        let version = store.version();
        let mut cache = self.restored.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain(|(v, _, _)| *v == version);
        if let Some((_, _, text)) = cache.iter().find(|(_, k, _)| *k == key) {
            return text.clone().flatten().map(Status::Ready);
        }
        if cache.iter().filter(|(_, _, text)| text.is_none()).count() >= 4 {
            return None;
        }
        if cache.len() >= 128 {
            let index = cache.iter().position(|(_, _, text)| text.is_some())?;
            cache.remove(index);
        }
        if self
            .restore_scans
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < 4).then_some(count + 1)
            })
            .is_err()
        {
            return None;
        }
        cache.push_back((version, key.clone(), None));
        drop(cache);
        let store = store.clone();
        let restored = self.restored.clone();
        let scans = self.restore_scans.clone();
        let shared = self.shared.clone();
        let wake = self.wake_search.clone();
        std::thread::spawn(move || {
            let _io = shared.store_io.lock().unwrap_or_else(|p| p.into_inner());
            let text = store.get(&key.0, &key.1, &key.2);
            if shared.alive.load(Ordering::Relaxed) && store.version() == version {
                let mut cache = restored.lock().unwrap_or_else(|p| p.into_inner());
                if let Some((_, _, slot)) = cache
                    .iter_mut()
                    .find(|(v, k, _)| *v == version && *k == key)
                {
                    *slot = Some(text);
                }
            }
            drop(_io);
            scans.fetch_sub(1, Ordering::Relaxed);
            wake();
        });
        None
    }
}

impl Drop for Worker {
    /// A respawned worker (the service address changed) stops meaning
    /// anything: its in-flight engine's result is dropped, not stored.
    fn drop(&mut self) {
        self.shared.alive.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_voice_and_video_restore_after_worker_restart_and_stay_searchable() {
        let root = tempfile::tempdir().unwrap();
        {
            let store = Store::new(root.path().into(), 1 << 20).unwrap();
            let worker = Worker::spawn(Box::new(EchoEngine), store, || {});
            for id in ["voice", "video"] {
                worker.transcribe(job(id), "acc-1".into(), Some("fi".into()));
                assert!(matches!(
                    wait_for(&worker, "acc-1", "c1", id),
                    Status::Ready(_)
                ));
            }
        }
        let store = Store::new(root.path().into(), 1 << 20).unwrap();
        let worker = Worker::archive(store);
        for id in ["voice", "video"] {
            assert!(matches!(
                wait_for(&worker, "acc-1", "c1", id),
                Status::Ready(_)
            ));
        }
        assert_eq!(worker.search("note.ogg").len(), 2);
        worker.remove_chat("acc-1".into(), "c1".into());
        assert_eq!(worker.get("acc-1", "c1", "voice"), None);
        assert!(worker.search("note.ogg").is_empty());
    }

    #[test]
    fn saved_chat_snapshot_filters_before_limit_and_invalidates_on_delete() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), 1 << 20).unwrap();
        for i in 0..60 {
            store
                .put("other", "same-chat", &format!("m{i}"), "station")
                .unwrap();
        }
        store
            .put("mine", "same-chat", "mine", "at the station")
            .unwrap();
        let worker = Worker::stub_with_store(store);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let hits = loop {
            let hits = worker.saved_in_chat("mine", "same-chat", "station");
            if !hits.is_empty() {
                break hits;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "mine");
        assert_eq!(
            worker.saved_in_chat("mine", "same-chat", "station").len(),
            1
        );
        worker.remove_chat("mine".into(), "same-chat".into());
        assert!(worker.saved_in_chat("mine", "same-chat", "").is_empty());
        assert!(
            worker
                .recorded()
                .iter()
                .all(|line| !line.starts_with("transcribe"))
        );
    }

    #[test]
    fn truncation_is_reported_even_when_no_saved_match_can_be_displayed() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), u64::MAX).unwrap();
        store.put("acc", "chat", "huge", "station").unwrap();
        let dir = root.path().join("acc/chat");
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("huge.txt"))
            .unwrap()
            .set_len((32 << 20) + 1)
            .unwrap();
        let worker = Worker::archive(store);
        worker.saved_in_chat("acc", "chat", "station");
        let deadline = Instant::now() + Duration::from_secs(3);
        while worker.saved_loading("acc", "chat", "station") {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.saved_in_chat("acc", "chat", "station").is_empty());
        assert!(worker.saved_truncated("acc", "chat", "station"));
    }

    #[test]
    fn panicking_scan_releases_admission_and_pending_entry() {
        let active = Arc::new(AtomicUsize::new(1));
        let key = ("acc".into(), "chat".into(), "query".into(), false);
        let cache = Arc::new(Mutex::new(VecDeque::from([ScopedSnapshot {
            version: 0,
            key: key.clone(),
            hits: None,
            truncated: false,
        }])));
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let permit = ScanPermit {
            active: active.clone(),
            cache: cache.clone(),
            version: 0,
            key,
            wake: Arc::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        };
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _permit = permit;
                panic!("synthetic scan panic");
            }))
            .is_err()
        );
        assert_eq!(active.load(Ordering::Relaxed), 0);
        assert!(cache.lock().unwrap().is_empty());
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn scan_threads_stay_bounded_across_queries_and_store_versions() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), 1 << 20).unwrap();
        let worker = Worker::archive(store.clone());
        let io = worker.shared.store_io.lock().unwrap();
        for i in 0..20 {
            store
                .put("acc", "chat", "one", &format!("station {i}"))
                .unwrap();
            worker.saved_in_chat("acc", "chat", &i.to_string());
            assert!(worker.active_scans.load(Ordering::Relaxed) <= SCOPED_CACHE_SLOTS);
        }
        assert_eq!(
            worker.active_scans.load(Ordering::Relaxed),
            SCOPED_CACHE_SLOTS
        );
        drop(io);
        let deadline = Instant::now() + Duration::from_secs(3);
        while worker.active_scans.load(Ordering::Relaxed) != 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn account_search_never_waits_for_disk_and_filters_before_caps() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), u64::MAX).unwrap();
        store.put("mine", "chat", "mine", "station").unwrap();
        for i in 0..60 {
            store
                .put("other", "chat", &format!("m{i}"), "station")
                .unwrap();
        }
        let worker = Worker::archive(store);
        let io = worker.shared.store_io.lock().unwrap();
        assert!(worker.saved_for_account("mine", "station").is_empty());
        assert!(worker.saved_account_state("mine", "station").0);
        drop(io);
        let deadline = Instant::now() + Duration::from_secs(3);
        let hits = loop {
            let hits = worker.saved_for_account("mine", "station");
            if !hits.is_empty() {
                break hits;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "mine");
        assert!(Arc::ptr_eq(
            &hits,
            &worker.saved_for_account("mine", "station")
        ));
    }

    #[test]
    fn alternating_scoped_panels_settle_without_rescanning() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), 1 << 20).unwrap();
        store.put("acc", "a", "a1", "station").unwrap();
        store.put("acc", "b", "b1", "station").unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let worker = Worker::archive_with_wake(store, move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while worker.saved_in_chat("acc", "a", "station").is_empty()
            || worker.saved_in_chat("acc", "b", "station").is_empty()
        {
            assert!(
                Instant::now() < deadline,
                "alternating panels never settled"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..20 {
            assert_eq!(worker.saved_in_chat("acc", "a", "station").len(), 1);
            assert_eq!(worker.saved_in_chat("acc", "b", "station").len(), 1);
        }
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            2,
            "cached frames must not launch scans"
        );
    }

    #[test]
    fn snapshot_completion_after_mutation_never_publishes_old_version() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_owned(), 1 << 20).unwrap();
        store.put("acc", "chat", "old", "station").unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let worker = Worker::archive_with_wake(store.clone(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let io = worker.shared.store_io.lock().unwrap();
        assert!(worker.saved_in_chat("acc", "chat", "station").is_empty());
        store.remove_chat("acc", "chat").unwrap();
        store.put("acc", "chat", "fresh", "station").unwrap();
        drop(io);
        wait_wakes(&wakes, 1);
        assert!(
            worker.saved_in_chat("acc", "chat", "station").is_empty(),
            "stale completion was published"
        );
        wait_wakes(&wakes, 2);
        let hits = worker.saved_in_chat("acc", "chat", "station");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "fresh");
    }

    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    struct EchoEngine;
    impl Engine for EchoEngine {
        fn transcribe(
            &mut self,
            audio: &std::path::Path,
            language: &str,
        ) -> Result<String, String> {
            Ok(format!("{language}:{}", audio.display()))
        }
    }

    struct FailEngine;
    impl Engine for FailEngine {
        fn transcribe(
            &mut self,
            _audio: &std::path::Path,
            _language: &str,
        ) -> Result<String, String> {
            Err("engine down".into())
        }
    }

    /// An engine whose runs are gated on test barriers: each transcribe
    /// announces it started and blocks until the test releases it, so the
    /// test controls exactly when the late result lands - no sleeps. The
    /// run number distinguishes an old job's result from a fresh one's.
    struct GatedEngine {
        runs: usize,
        entered: Sender<()>,
        release: Receiver<()>,
    }

    impl Engine for GatedEngine {
        fn transcribe(
            &mut self,
            audio: &std::path::Path,
            _language: &str,
        ) -> Result<String, String> {
            self.runs += 1;
            let run = self.runs;
            let _ = self.entered.send(());
            let _ = self.release.recv();
            Ok(format!("run-{run}:{}", audio.display()))
        }
    }

    fn gated() -> (GatedEngine, Receiver<()>, Sender<()>) {
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        (
            GatedEngine {
                runs: 0,
                entered: entered_tx,
                release: release_rx,
            },
            entered_rx,
            release_tx,
        )
    }

    fn job(id: &str) -> Job {
        Job {
            message_id: id.into(),
            chat_id: "c1".into(),
            audio: PathBuf::from("/tmp/note.ogg"),
            duration_secs: Some(2.0),
            language: None,
        }
    }

    fn store() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tw-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        (Store::new(dir.clone(), 1 << 20).unwrap(), dir)
    }

    fn wait_for(worker: &Worker, account: &str, chat: &str, id: &str) -> Status {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match worker.get(account, chat, id) {
                Some(s @ (Status::Ready(_) | Status::Failed(_))) => return s,
                _ if Instant::now() > deadline => panic!("no final status for {id}"),
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    /// Every status change and every completion, dropped or not, wakes.
    /// Waiting for the count is the deterministic half of "the worker
    /// reached this point".
    fn wait_wakes(wakes: &Arc<AtomicUsize>, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while wakes.load(Ordering::Relaxed) < count {
            assert!(Instant::now() <= deadline, "worker never woke {count}x");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn transcribe_ready_and_stored() {
        let (s, dir) = store();
        let worker = Worker::spawn(Box::new(EchoEngine), s.clone(), || {});
        worker.transcribe(job("m1"), "acc-1".into(), Some("fi".into()));
        assert_eq!(
            wait_for(&worker, "acc-1", "c1", "m1"),
            Status::Ready("fi:/tmp/note.ogg".into())
        );
        // The durable copy landed under account/chat/message isolation.
        assert_eq!(
            s.get("acc-1", "c1", "m1").as_deref(),
            Some("fi:/tmp/note.ogg")
        );
        worker.clear();
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.get("acc-1", "c1", "m1").is_some() {
            assert!(Instant::now() <= deadline, "clear never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failure_reported_not_retried_forever() {
        let (s, dir) = store();
        let worker = Worker::spawn(Box::new(FailEngine), s, || {});
        worker.transcribe(job("m2"), "acc-1".into(), None);
        assert_eq!(
            wait_for(&worker, "acc-1", "c1", "m2"),
            Status::Failed("engine down".into())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_transcribe_is_dropped() {
        let (s, dir) = store();
        let worker = Worker::spawn(Box::new(EchoEngine), s, || {});
        worker.transcribe(job("m3"), "acc-1".into(), None);
        let first = wait_for(&worker, "acc-1", "c1", "m3");
        worker.transcribe(job("m3"), "acc-1".into(), None); // Ready short-circuits
        assert_eq!(worker.get("acc-1", "c1", "m3"), Some(first));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn global_language_used_when_job_has_none() {
        let (s, dir) = store();
        let worker = Worker::spawn(Box::new(EchoEngine), s, || {});
        worker.transcribe(job("m4"), "acc-1".into(), Some("en".into()));
        assert_eq!(
            wait_for(&worker, "acc-1", "c1", "m4"),
            Status::Ready("en:/tmp/note.ogg".into())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn accounts_sharing_ids_stay_separate() {
        let (s, dir) = store();
        let worker = Worker::spawn(Box::new(EchoEngine), s.clone(), || {});
        // Same chat id, same message id, two accounts: no collision in
        // memory, statuses, or the store.
        worker.transcribe(job("m5"), "acc-1".into(), Some("fi".into()));
        worker.transcribe(job("m5"), "acc-2".into(), Some("en".into()));
        assert_eq!(
            wait_for(&worker, "acc-1", "c1", "m5"),
            Status::Ready("fi:/tmp/note.ogg".into())
        );
        assert_eq!(
            wait_for(&worker, "acc-2", "c1", "m5"),
            Status::Ready("en:/tmp/note.ogg".into())
        );
        assert_eq!(
            s.get("acc-1", "c1", "m5").as_deref(),
            Some("fi:/tmp/note.ogg")
        );
        assert_eq!(
            s.get("acc-2", "c1", "m5").as_deref(),
            Some("en:/tmp/note.ogg")
        );
        // Deleting the chat in one account leaves the other's intact.
        worker.remove_chat("acc-1".into(), "c1".into());
        assert_eq!(worker.get("acc-1", "c1", "m5"), None);
        assert_eq!(s.get("acc-1", "c1", "m5"), None);
        assert!(matches!(
            worker.get("acc-2", "c1", "m5"),
            Some(Status::Ready(_))
        ));
        assert!(s.get("acc-2", "c1", "m5").is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn account_logout_invalidates_late_results_without_erasing_other_accounts() {
        let (store, dir) = store();
        store.put("acc-2", "c1", "kept", "other account").unwrap();
        let (engine, entered, release) = gated();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let worker = Worker::spawn(Box::new(engine), store.clone(), move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        worker.transcribe(job("late"), "acc-1".into(), None);
        entered.recv().unwrap();
        worker.remove_account("acc-1".into()).unwrap();
        release.send(()).unwrap();
        wait_wakes(&wakes, 2);
        assert_eq!(store.get("acc-1", "c1", "late"), None);
        assert_eq!(worker.get("acc-1", "c1", "late"), None);
        assert_eq!(
            store.get("acc-2", "c1", "kept").as_deref(),
            Some("other account")
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn remove_chat_is_immediate_and_drops_the_late_result() {
        let (s, dir) = store();
        let wakes = Arc::new(AtomicUsize::new(0));
        let (engine, entered, release) = gated();
        let counter = Arc::clone(&wakes);
        let worker = Worker::spawn(Box::new(engine), s.clone(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        worker.transcribe(job("m6"), "acc-1".into(), None);
        // The engine has the job; the delete lands while it works.
        entered.recv().unwrap();
        worker.remove_chat("acc-1".into(), "c1".into());
        // Immediate: status and files go on the calling thread, while the
        // engine is still blocked.
        assert_eq!(worker.get("acc-1", "c1", "m6"), None);
        // The late result lands now; it must be dropped.
        release.send(()).unwrap();
        wait_wakes(&wakes, 2);
        assert_eq!(worker.get("acc-1", "c1", "m6"), None, "late status landed");
        assert_eq!(s.get("acc-1", "c1", "m6"), None, "late result stored");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clear_during_in_flight_drops_the_late_result() {
        let (s, dir) = store();
        let wakes = Arc::new(AtomicUsize::new(0));
        let (engine, entered, release) = gated();
        let counter = Arc::clone(&wakes);
        let worker = Worker::spawn(Box::new(engine), s.clone(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        worker.transcribe(job("m7"), "acc-1".into(), None);
        entered.recv().unwrap();
        worker.clear();
        assert_eq!(worker.get("acc-1", "c1", "m7"), None);
        release.send(()).unwrap();
        wait_wakes(&wakes, 2);
        assert_eq!(worker.get("acc-1", "c1", "m7"), None, "late status landed");
        assert_eq!(s.get("acc-1", "c1", "m7"), None, "late result stored");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_then_readd_lets_only_the_fresh_job_store() {
        let (s, dir) = store();
        let wakes = Arc::new(AtomicUsize::new(0));
        let (engine, entered, release) = gated();
        let counter = Arc::clone(&wakes);
        let worker = Worker::spawn(Box::new(engine), s.clone(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        // The old job starts; the chat is deleted and re-added; a fresh
        // transcription is requested before the old engine finishes.
        worker.transcribe(job("m8"), "acc-1".into(), None);
        entered.recv().unwrap();
        worker.remove_chat("acc-1".into(), "c1".into());
        worker.transcribe(job("m8"), "acc-1".into(), None);
        // The old job's late result: dropped at its generation check.
        release.send(()).unwrap();
        wait_wakes(&wakes, 2);
        assert_eq!(s.get("acc-1", "c1", "m8"), None, "the old job stored");
        // The fresh job runs (the engine's second gate) and is the one
        // that stores and renders.
        entered.recv().unwrap();
        release.send(()).unwrap();
        assert_eq!(
            wait_for(&worker, "acc-1", "c1", "m8"),
            Status::Ready("run-2:/tmp/note.ogg".into())
        );
        assert_eq!(
            s.get("acc-1", "c1", "m8").as_deref(),
            Some("run-2:/tmp/note.ogg"),
            "only the fresh job's transcript is stored"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dropped_workers_late_result_never_lands() {
        let (s, dir) = store();
        let wakes = Arc::new(AtomicUsize::new(0));
        let (engine, entered, release) = gated();
        let counter = Arc::clone(&wakes);
        let worker = Worker::spawn(Box::new(engine), s.clone(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        worker.transcribe(job("m9"), "acc-1".into(), None);
        entered.recv().unwrap();
        // The settings respawn drops the worker mid-engine.
        drop(worker);
        release.send(()).unwrap();
        wait_wakes(&wakes, 2);
        assert_eq!(
            s.get("acc-1", "c1", "m9"),
            None,
            "a dropped worker's result was stored"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removed_chat_leaves_search_immediately() {
        let (s, dir) = store();
        s.put("acc-1", "c1", "m10", "hei maailma").unwrap();
        let worker = Worker::stub_with_store(s);
        assert_eq!(worker.search("maailma").len(), 1);
        // No engine anywhere: the removal itself takes it out of search.
        worker.remove_chat("acc-1".into(), "c1".into());
        assert!(worker.search("maailma").is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_uses_the_store_and_caches() {
        let (s, dir) = store();
        s.put("acc-1", "c1", "m11", "hei maailma").unwrap();
        let worker = Worker::stub_with_store(s.clone());
        let hits = worker.search("maailma");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m11");
        // A new transcript invalidates the cache through the version bump.
        s.put("acc-1", "c1", "m12", "maailma again").unwrap();
        assert_eq!(worker.search("maailma").len(), 2);
        // A plain stub has no store and finds nothing.
        assert!(Worker::stub([]).search("maailma").is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
