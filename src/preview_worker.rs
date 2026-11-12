//! Bounded, account-scoped local preview jobs. Cancelling drops late results.
//! URLs are never logged or used as filenames. UI owns consent and visibility.
use crate::link_preview::Preview;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};
const READY_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const ERROR_TTL: Duration = Duration::from_secs(15 * 60);
const CACHE_BYTES: usize = 50 * 1024 * 1024;
const CACHE_ITEMS: usize = 256;

#[derive(Clone, Debug)]
pub enum Status {
    Loading,
    Ready(Preview),
    Failed,
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    account: String,
    url: String,
}
struct Entry {
    status: Status,
    at: Instant,
    used: Instant,
}
struct Shared {
    entries: HashMap<Key, Entry>,
    generation: u64,
    cancellation: Arc<AtomicBool>,
}
struct Job {
    key: Key,
    generation: u64,
    cancellation: Arc<AtomicBool>,
}
type Fetch = dyn Fn(&str, &AtomicBool) -> Result<Preview, &'static str> + Send + Sync;
pub struct Worker {
    tx: SyncSender<Job>,
    shared: Arc<Mutex<Shared>>,
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}
impl Worker {
    pub fn new() -> Self {
        Self::with_fetch(Arc::new(crate::link_preview::fetch))
    }
    fn with_fetch(fetch: Arc<Fetch>) -> Self {
        let (tx, rx) = sync_channel::<Job>(16);
        let shared = Arc::new(Mutex::new(Shared {
            entries: HashMap::new(),
            generation: 0,
            cancellation: Arc::new(AtomicBool::new(false)),
        }));
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..2 {
            let fetch = fetch.clone();
            let shared = shared.clone();
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("link-preview-{index}"))
                .spawn(move || {
                    loop {
                        let Ok(job) = rx.lock().unwrap_or_else(|e| e.into_inner()).recv() else {
                            return;
                        };
                        if job.cancellation.load(Ordering::Relaxed) {
                            continue;
                        }
                        let result = fetch(&job.key.url, &job.cancellation);
                        let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                        if shared.generation != job.generation
                            || job.cancellation.load(Ordering::Relaxed)
                        {
                            continue;
                        }
                        if let Some(entry) = shared.entries.get_mut(&job.key) {
                            entry.status = result.map(Status::Ready).unwrap_or(Status::Failed);
                            entry.at = Instant::now();
                            entry.used = Instant::now();
                        }
                        Self::trim(&mut shared);
                    }
                })
                .expect("preview worker thread");
        }
        Self { tx, shared }
    }
    /// Synthetic demo/test fixture only. Never usable by production callers.
    #[cfg(any(test, feature = "demo"))]
    pub fn seed_fixture(&self, account: &str, url: &str, preview: Preview) {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.entries.insert(
            Self::key(account, url),
            Entry {
                status: Status::Ready(preview),
                at: Instant::now(),
                used: Instant::now(),
            },
        );
        Self::trim(&mut shared);
    }
    fn key(account: &str, url: &str) -> Key {
        // Fragments are not HTTP request data. Queries must stay distinct.
        Key {
            account: account.into(),
            url: url.split('#').next().unwrap_or(url).into(),
        }
    }
    /// A missing status never queues a request. The caller must supply consent.
    pub fn status(&self, account: &str, url: &str) -> Option<Status> {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let key = Self::key(account, url);
        let entry = shared.entries.get_mut(&key)?;
        let ttl = if matches!(entry.status, Status::Failed) {
            ERROR_TTL
        } else {
            READY_TTL
        };
        if entry.at.elapsed() > ttl {
            shared.entries.remove(&key);
            return None;
        }
        entry.used = Instant::now();
        Some(entry.status.clone())
    }
    pub fn request(&self, account: &str, url: &str, retry: bool) -> bool {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let key = Self::key(account, url);
        if shared.entries.get(&key).is_some_and(|entry| {
            matches!(entry.status, Status::Loading | Status::Ready(_)) || !retry
        }) {
            return false;
        }
        let job = Job {
            key: key.clone(),
            generation: shared.generation,
            cancellation: shared.cancellation.clone(),
        };
        if self.tx.try_send(job).is_err() {
            return false;
        }
        shared.entries.insert(
            key,
            Entry {
                status: Status::Loading,
                at: Instant::now(),
                used: Instant::now(),
            },
        );
        Self::trim(&mut shared);
        true
    }
    /// Call on lock, account switch or automatic-mode disable. Completed cards
    /// remain local; queued/in-flight generations cannot add results afterwards.
    pub fn cancel(&self) {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.cancellation.store(true, Ordering::Relaxed);
        shared.generation = shared.generation.wrapping_add(1);
        shared.cancellation = Arc::new(AtomicBool::new(false));
        shared.entries.retain(|_, entry| {
            matches!(&entry.status, Status::Ready(preview) if !preview.no_store)
                || matches!(entry.status, Status::Failed)
        });
    }
    pub fn clear(&self) {
        self.cancel();
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .clear();
    }
    fn trim(shared: &mut Shared) {
        let cost = |entry: &Entry| match &entry.status {
            Status::Ready(preview) => {
                preview.image.as_ref().map_or(0, Vec::len)
                    + preview.title.as_ref().map_or(0, String::len)
                    + preview.description.as_ref().map_or(0, String::len)
            }
            _ => 0,
        };
        while shared.entries.len() > CACHE_ITEMS
            || shared.entries.values().map(cost).sum::<usize>() > CACHE_BYTES
        {
            let Some(oldest) = shared
                .entries
                .iter()
                .filter(|(_, entry)| !matches!(entry.status, Status::Loading))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            shared.entries.remove(&oldest);
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn preview() -> Preview {
        Preview {
            title: Some("Fixture".into()),
            description: None,
            image: None,
            no_store: false,
        }
    }
    fn wait_for(worker: &Worker, account: &str, url: &str, ready: bool) {
        let start = Instant::now();
        loop {
            let state = worker.status(account, url);
            if (ready && matches!(state, Some(Status::Ready(_))))
                || (!ready && matches!(state, Some(Status::Failed)))
            {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "worker did not settle"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn missing_status_never_requests_and_cache_keys_preserve_account_and_query() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let worker = Worker::with_fetch(Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(preview())
        }));
        assert!(worker.status("one", "https://example.com/").is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(worker.shared.lock().unwrap().entries.is_empty());
        assert!(
            Worker::key("one", "https://example.com/?q=1#part")
                == Worker::key("one", "https://example.com/?q=1#other")
        );
        assert!(
            Worker::key("one", "https://example.com/?q=1")
                != Worker::key("two", "https://example.com/?q=1")
        );
        assert!(
            Worker::key("one", "https://example.com/?q=1")
                != Worker::key("one", "https://example.com/?q=2")
        );
        assert!(worker.request("one", "https://example.com/?q=1#part", false));
        wait_for(&worker, "one", "https://example.com/?q=1", true);
        assert!(!worker.request("one", "https://example.com/?q=1#other", false));
        assert!(!worker.request("one", "https://example.com/?q=1", true));
        assert!(worker.status("two", "https://example.com/?q=1").is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn clear_cancels_queued_and_late_results_without_duplicate_loading_retries() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let gate = release.clone();
        let worker = Worker::with_fetch(Arc::new(move |_, cancelled| {
            started_tx.send(()).unwrap();
            while !gate.load(Ordering::SeqCst) && !cancelled.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(preview())
        }));
        assert!(worker.request("one", "https://example.com/a", false));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!worker.request("one", "https://example.com/a", true));
        assert!(worker.request("one", "https://example.com/b", false));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(worker.request("one", "https://example.com/queued", false));
        worker.clear();
        release.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        assert!(worker.shared.lock().unwrap().entries.is_empty());
        assert!(started_rx.try_recv().is_err());
    }
    #[test]
    fn bounded_cache_expiry_negative_retry_and_no_store() {
        let worker = Worker::with_fetch(Arc::new(|_, _| Err("fixture")));
        assert!(worker.request("one", "https://example.com/a", false));
        wait_for(&worker, "one", "https://example.com/a", false);
        assert!(!worker.request("one", "https://example.com/a", false));
        assert!(worker.request("one", "https://example.com/a", true));
        wait_for(&worker, "one", "https://example.com/a", false);
        {
            let mut shared = worker.shared.lock().unwrap();
            shared
                .entries
                .get_mut(&Worker::key("one", "https://example.com/a"))
                .unwrap()
                .at = Instant::now() - ERROR_TTL - Duration::from_secs(1);
        }
        assert!(worker.status("one", "https://example.com/a").is_none());
        {
            let mut shared = worker.shared.lock().unwrap();
            for index in 0..300 {
                shared.entries.insert(
                    Worker::key("one", &format!("https://example.com/{index}")),
                    Entry {
                        status: Status::Ready(preview()),
                        at: Instant::now(),
                        used: Instant::now(),
                    },
                );
            }
            Worker::trim(&mut shared);
            assert_eq!(shared.entries.len(), CACHE_ITEMS);
            let mut ephemeral = preview();
            ephemeral.no_store = true;
            shared.entries.insert(
                Worker::key("one", "https://example.com/private"),
                Entry {
                    status: Status::Ready(ephemeral),
                    at: Instant::now(),
                    used: Instant::now(),
                },
            );
        }
        worker.cancel();
        assert!(
            worker
                .status("one", "https://example.com/private")
                .is_none()
        );
        worker.clear();
        assert!(worker.shared.lock().unwrap().entries.is_empty());
    }
}
