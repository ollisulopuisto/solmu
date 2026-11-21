//! Transcript rows share each account's existing SQLCipher archive and OS key.
use super::{Hit, SNAPSHOT_HIT_BYTES, SNAPSHOT_HITS, SNAPSHOT_TEXT_BYTES, Snapshot};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct Encrypted {
    state: PathBuf,
    connections: Mutex<HashMap<String, Connection>>,
}
fn io(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
impl Encrypted {
    pub fn new(state: PathBuf) -> Self {
        Self {
            state,
            connections: Mutex::new(HashMap::new()),
        }
    }
    fn path(&self, account: &str) -> std::io::Result<PathBuf> {
        if !crate::model::AccountId::is_safe(account) {
            return Err(io("Invalid transcript account"));
        }
        let accounts = self.state.join("accounts");
        if std::fs::symlink_metadata(&accounts).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io("Transcript accounts directory must not be a symlink"));
        }
        let dir = accounts.join(account);
        // Never recreate a removed account or follow a planted account link.
        if std::fs::symlink_metadata(&dir)
            .map_or(true, |m| !m.is_dir() || m.file_type().is_symlink())
        {
            return Err(io("Transcript account is unavailable"));
        }
        let path = dir.join("archive.db");
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err(io("Encrypted account archive is unavailable"));
        }
        Ok(path)
    }
    fn with<T>(
        &self,
        account: &str,
        f: impl FnOnce(&mut Connection) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let path = self.path(account)?;
        let mut connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
        if !connections.contains_key(account) {
            let connection = crate::archive::transcript_connection(&path).map_err(io)?;
            connection.execute_batch("CREATE TABLE IF NOT EXISTS local_transcripts(chat TEXT NOT NULL, message TEXT NOT NULL, text TEXT NOT NULL, modified INTEGER NOT NULL, PRIMARY KEY(chat,message));").map_err(io)?;
            connections.insert(account.into(), connection);
        }
        f(connections.get_mut(account).unwrap())
    }
    pub fn accounts(&self) -> Vec<String> {
        super::subdirs(&self.state.join("accounts"))
            .into_iter()
            .filter(|id| crate::model::AccountId::is_safe(id))
            .collect()
    }
    pub fn put(
        &self,
        account: &str,
        chat: &str,
        message: &str,
        text: &str,
        modified: i64,
        max: u64,
    ) -> std::io::Result<()> {
        self.with(account, |c| {
            let tx=c.transaction().map_err(io)?;
            tx.execute("INSERT INTO local_transcripts(chat,message,text,modified) VALUES (?1,?2,?3,?4) ON CONFLICT(chat,message) DO UPDATE SET text=excluded.text,modified=excluded.modified",params![chat,message,text,modified]).map_err(io)?;
            let mut total:i64=tx.query_row("SELECT coalesce(sum(length(CAST(text AS BLOB))),0) FROM local_transcripts",[],|r|r.get(0)).map_err(io)?;
            while total > max as i64 {
                let oldest:(String,String,i64)=tx.query_row("SELECT chat,message,length(CAST(text AS BLOB)) FROM local_transcripts ORDER BY modified,chat,message LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(io)?;
                tx.execute("DELETE FROM local_transcripts WHERE chat=?1 AND message=?2",params![oldest.0,oldest.1]).map_err(io)?;
                total-=oldest.2;
            }
            tx.commit().map_err(io)
        })?;
        Ok(())
    }
    pub fn get(&self, account: &str, chat: &str, message: &str) -> Option<String> {
        self.with(account, |c| {
            c.query_row(
                "SELECT text FROM local_transcripts WHERE chat=?1 AND message=?2",
                params![chat, message],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)
        })
        .ok()
        .flatten()
    }
    pub fn remove(
        &self,
        account: &str,
        chat: Option<&str>,
        message: Option<&str>,
    ) -> std::io::Result<()> {
        let result = self.with(account, |c| {
            c.execute("DELETE FROM local_transcripts WHERE (?1 IS NULL OR chat=?1) AND (?2 IS NULL OR message=?2)", params![chat,message]).map_err(io)?;
            Ok(())
        });
        // Closing our connection is needed before Windows account-folder removal.
        if chat.is_none() && message.is_none() {
            self.connections
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(account);
        }
        result
    }
    pub fn snapshot(
        &self,
        query: &str,
        account: &str,
        chat: Option<&str>,
        current: impl Fn() -> bool,
    ) -> Snapshot {
        let mut result = Snapshot::default();
        let needle = crate::util::search_key(query.trim());
        let mut bytes = 0;
        let read = self.with(account, |c| {
            let mut statement = c.prepare("SELECT chat,message,text,modified FROM local_transcripts WHERE (?1 IS NULL OR chat=?1) ORDER BY modified DESC, chat, message").map_err(io)?;
            let rows = statement.query_map(params![chat], |row| Ok(Hit { account_id: account.into(),chat_id: row.get(0)?,message_id: row.get(1)?,text:row.get(2)?,modified:row.get(3)? })).map_err(io)?;
            for row in rows {
                if !current() { return Ok(false); }
                let mut hit = row.map_err(io)?;
                if !crate::util::search_key(&hit.text).contains(&needle) { continue; }
                if hit.text.len() > SNAPSHOT_HIT_BYTES {
                    result.truncated = true;
                    if !needle.is_empty() { hit.text = super::preview(&hit.text, query); }
                    let mut end = SNAPSHOT_HIT_BYTES.min(hit.text.len()); while !hit.text.is_char_boundary(end) { end -= 1; } hit.text.truncate(end);
                }
                if result.hits.len() >= SNAPSHOT_HITS || bytes + hit.text.len() > SNAPSHOT_TEXT_BYTES { result.truncated = true; continue; }
                bytes += hit.text.len(); result.hits.push(hit);
            }
            Ok(true)
        });
        if matches!(read, Ok(true)) {
            result
        } else {
            Snapshot::default()
        }
    }
    /// Search hits carry the full transcript text, unclipped and uncapped; the
    /// store sorts the merged accounts and applies the caller's limit itself.
    pub fn search(&self, query: &str, account: &str, chat: Option<&str>) -> Vec<Hit> {
        let needle = crate::util::search_key(query.trim());
        self.with(account, |c| {
            let mut statement = c.prepare("SELECT chat,message,text,modified FROM local_transcripts WHERE (?1 IS NULL OR chat=?1) ORDER BY modified DESC, chat, message").map_err(io)?;
            let rows = statement.query_map(params![chat], |row| Ok(Hit { account_id: account.into(),chat_id: row.get(0)?,message_id: row.get(1)?,text:row.get(2)?,modified:row.get(3)? })).map_err(io)?;
            let mut hits = Vec::new();
            for row in rows {
                let hit = row.map_err(io)?;
                if !crate::util::search_key(&hit.text).contains(&needle) { continue; }
                hits.push(hit);
            }
            Ok(hits)
        })
        .unwrap_or_default()
    }
    /// One account transaction is committed and re-read before any source file
    /// is removed. Interrupted migration is safe to repeat; rows win on conflict.
    /// Migrates each legacy account directory independently: one failing
    /// account (missing archive, conflicting row, unreadable file) keeps its
    /// plaintext for the next launch and never blocks later accounts, whose
    /// transcripts still move and purge. Returns how many transcripts moved
    /// and the first per-account error, if any.
    pub fn migrate(&self, root: &Path) -> (usize, Option<std::io::Error>) {
        let mut migrated = 0usize;
        let mut first_error = None;
        let mut encoded_accounts = super::subdirs(root);
        encoded_accounts.sort();
        for encoded in encoded_accounts {
            match self.migrate_account(root, &encoded) {
                Ok(count) => migrated += count,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        (migrated, first_error)
    }

    fn migrate_account(&self, root: &Path, encoded: &str) -> std::io::Result<usize> {
        let account =
            super::decode(encoded).ok_or_else(|| io("Invalid legacy transcript account"))?;
        let mut source = Vec::new();
        for chat_dir in super::subdirs(&root.join(encoded)) {
            let chat =
                super::decode(&chat_dir).ok_or_else(|| io("Invalid legacy transcript chat"))?;
            for entry in std::fs::read_dir(root.join(encoded).join(&chat_dir))? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name();
                let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".txt")) else {
                    continue;
                };
                let message =
                    super::decode(stem).ok_or_else(|| io("Invalid legacy transcript message"))?;
                if entry.metadata()?.len() > super::MAX_SOURCE_BYTES {
                    return Err(io("Legacy transcript exceeds safe migration size"));
                }
                let text = std::fs::read_to_string(entry.path())?;
                let modified = entry
                    .metadata()?
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(io)?
                    .as_secs() as i64;
                source.push((chat.clone(), message, text, modified, entry.path()));
            }
        }
        if source.is_empty() {
            return Ok(0);
        }
        self.with(&account, |c| {
            let tx = c.transaction().map_err(io)?;
            for (chat,message,text,modified,_) in &source {
                tx.execute("INSERT INTO local_transcripts(chat,message,text,modified) VALUES(?1,?2,?3,?4) ON CONFLICT(chat,message) DO NOTHING",params![chat,message,text,modified]).map_err(io)?;
            }
            tx.commit().map_err(io)?;
            // Read back exact committed text before deleting any plaintext source.
            for (chat,message,_text,_,_) in &source {
                let saved: String = c.query_row("SELECT text FROM local_transcripts WHERE chat=?1 AND message=?2", params![chat,message], |r| r.get(0)).map_err(io)?;
                if &saved != _text { return Err(io("Encrypted transcript migration verification failed")); }
            }
            Ok(())
        })?;
        let count = source.len();
        for (_, _, _, _, path) in source {
            std::fs::remove_file(path)?;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: [u8; 32] = [7; 32];
    fn fixture(state: &Path, accounts: &[&str]) -> Encrypted {
        let backend = Encrypted::new(state.into());
        for &account in accounts {
            let path = state.join("accounts").join(account).join("archive.db");
            let connection = crate::archive::transcript_connection_with_key(&path, &KEY).unwrap();
            connection.execute_batch("CREATE TABLE IF NOT EXISTS local_transcripts(chat TEXT NOT NULL,message TEXT NOT NULL,text TEXT NOT NULL,modified INTEGER NOT NULL,PRIMARY KEY(chat,message));").unwrap();
            backend
                .connections
                .lock()
                .unwrap()
                .insert(account.into(), connection);
        }
        backend
    }
    #[test]
    fn encrypted_restart_search_scope_deletion_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let backend = fixture(root.path(), &["1", "2"]);
        backend
            .put(
                "1",
                "chat",
                "voice",
                "secret voice fixture marker",
                4,
                1 << 20,
            )
            .unwrap();
        backend
            .put(
                "1",
                "chat",
                "video",
                "secret video fixture marker",
                5,
                1 << 20,
            )
            .unwrap();
        backend
            .put(
                "2",
                "chat",
                "voice",
                "other account fixture marker",
                6,
                1 << 20,
            )
            .unwrap();
        drop(backend);
        for entry in std::fs::read_dir(root.path().join("accounts/1")).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(b"secret voice fixture marker".len())
                    .any(|w| w == b"secret voice fixture marker")
            );
        }
        assert!(
            Connection::open(root.path().join("accounts/1/archive.db"))
                .unwrap()
                .query_row("SELECT count(*) FROM local_transcripts", [], |row| row
                    .get::<_, i64>(0))
                .is_err()
        );
        let backend = fixture(root.path(), &["1", "2"]);
        assert_eq!(
            backend.get("1", "chat", "voice").as_deref(),
            Some("secret voice fixture marker")
        );
        let hits = backend.snapshot("fixture", "1", None, || true).hits;
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.account_id == "1"));
        backend.remove("1", Some("chat"), None).unwrap();
        assert!(backend.get("1", "chat", "voice").is_none());
        assert!(backend.get("2", "chat", "voice").is_some());
        backend.remove("2", None, None).unwrap();
        // The removal closed the keyed fixture connection, so a snapshot now
        // reopens through the real keyring and fails closed. Prove the rows are
        // gone by reopening the database with the fixture key instead.
        let connection = crate::archive::transcript_connection_with_key(
            &root.path().join("accounts/2/archive.db"),
            &KEY,
        )
        .unwrap();
        let remaining: i64 = connection
            .query_row("SELECT count(*) FROM local_transcripts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0, "account deletion left transcripts behind");
        assert!(
            backend
                .snapshot("fixture", "2", None, || true)
                .hits
                .is_empty()
        );
    }
    #[test]
    fn migration_verifies_before_purge_and_is_restart_safe() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("transcripts");
        let store = super::super::Store::new(legacy.clone(), 1 << 20).unwrap();
        store
            .put("1", "chat", "voice", "legacy private words")
            .unwrap();
        let backend = fixture(root.path(), &["1"]);
        let (migrated, error) = backend.migrate(&legacy);
        assert_eq!(migrated, 1);
        assert!(error.is_none());
        assert!(store.get("1", "chat", "voice").is_none());
        assert_eq!(
            backend.get("1", "chat", "voice").as_deref(),
            Some("legacy private words")
        );
        let (migrated, error) = backend.migrate(&legacy);
        assert_eq!(migrated, 0);
        assert!(error.is_none());
        store
            .put("1", "chat", "voice", "different source must not be lost")
            .unwrap();
        // A conflicting row keeps its account's plaintext, reports the
        // failure, and leaves the migrated archive text alone.
        let (migrated, error) = backend.migrate(&legacy);
        assert_eq!(migrated, 0);
        assert!(error.is_some());
        assert_eq!(
            store.get("1", "chat", "voice").as_deref(),
            Some("different source must not be lost")
        );
        assert_eq!(
            backend.get("1", "chat", "voice").as_deref(),
            Some("legacy private words")
        );
        store
            .put("2", "chat", "voice", "no account or key yet")
            .unwrap();
        let (migrated, error) = backend.migrate(&legacy);
        assert_eq!(migrated, 0);
        assert!(error.is_some());
        assert!(store.get("2", "chat", "voice").is_some());
    }
    #[test]
    fn a_bad_legacy_account_never_blocks_later_accounts() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("transcripts");
        let store = super::super::Store::new(legacy.clone(), 1 << 20).unwrap();
        // "1" sorts first and has no account archive: its plaintext must stay.
        store
            .put("1", "chat", "stuck", "orphaned legacy words")
            .unwrap();
        store
            .put("2", "chat", "voice", "later account words")
            .unwrap();
        let backend = fixture(root.path(), &["2"]);
        let (migrated, error) = backend.migrate(&legacy);
        assert_eq!(migrated, 1, "the later healthy account still migrates");
        assert!(error.is_some(), "the first account's failure is reported");
        assert_eq!(
            backend.get("2", "chat", "voice").as_deref(),
            Some("later account words")
        );
        assert!(store.get("2", "chat", "voice").is_none());
        assert_eq!(
            store.get("1", "chat", "stuck").as_deref(),
            Some("orphaned legacy words"),
            "the failing account's plaintext waits for the next attempt"
        );
    }
    #[test]
    fn partial_migration_wakes_views_and_bumps_the_store_version() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("transcripts");
        let mut store = super::super::Store::new(legacy.clone(), 1 << 20).unwrap();
        store
            .put("1", "chat", "stuck", "orphaned legacy words")
            .unwrap();
        store
            .put("2", "chat", "voice", "later account words")
            .unwrap();
        store.encrypted = Some(std::sync::Arc::new(fixture(root.path(), &["2"])));
        let before = store.version();

        // The worker's migration thread wakes the view even though one
        // account failed, so the migrated account's text becomes searchable.
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&wakes);
        let _worker =
            crate::transcribe_worker::Worker::archive_with_wake(store.clone(), move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while wakes.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "partial migration never woke the view"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            store.version() > before,
            "cached results were not invalidated by the partial migration"
        );
        assert_eq!(
            backend_get(&store, "2", "chat", "voice").as_deref(),
            Some("later account words")
        );
        assert!(!store.file_for("2", "chat", "voice").exists());
        assert!(store.file_for("1", "chat", "stuck").exists());

        // Negative control: with nothing left to migrate, the retry still
        // reports the failing account but does not bump the version again,
        // so cached views are not invalidated a second time.
        let settled = store.version();
        let (migrated, error) = store.migrate_legacy();
        assert_eq!(migrated, 0);
        assert!(error.is_some());
        assert_eq!(
            store.version(),
            settled,
            "an empty retry must not invalidate cached views"
        );
    }

    fn backend_get(
        store: &super::super::Store,
        account: &str,
        chat: &str,
        message: &str,
    ) -> Option<String> {
        store.encrypted.as_ref()?.get(account, chat, message)
    }

    fn row_count(root: &tempfile::TempDir, account: &str) -> i64 {
        let connection = crate::archive::transcript_connection_with_key(
            &root.path().join(format!("accounts/{account}/archive.db")),
            &KEY,
        )
        .unwrap();
        connection
            .query_row("SELECT count(*) FROM local_transcripts", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// Clear attempts accounts in sorted order. `failing` loses its keyed
    /// connection so its next access fails closed through the real keyring.
    fn clear_with_failing_account(
        accounts: &[&str],
        failing: &str,
    ) -> (
        super::super::Store,
        tempfile::TempDir,
        std::io::Result<()>,
        u64,
    ) {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("transcripts");
        let mut store = super::super::Store::new(legacy.clone(), 1 << 20).unwrap();
        store
            .put(failing, "chat", "voice", "orphan legacy words")
            .unwrap();
        let backend = std::sync::Arc::new(fixture(root.path(), accounts));
        for (offset, account) in accounts.iter().enumerate() {
            backend
                .put(
                    account,
                    "chat",
                    "voice",
                    "account words",
                    offset as i64 + 1,
                    1 << 20,
                )
                .unwrap();
        }
        backend.connections.lock().unwrap().remove(failing);
        store.encrypted = Some(std::sync::Arc::clone(&backend));
        let before = store.version();
        let result = store.clear();
        (store, root, result, before)
    }

    #[test]
    fn clear_reports_a_failing_account_but_still_clears_the_others() {
        // Failing account first: the loop must continue past the error.
        let (store, root, result, before) = clear_with_failing_account(&["1", "2"], "1");
        assert!(result.is_err(), "the failing account must be reported");
        assert!(store.version() > before, "cached views still invalidate");
        assert_eq!(
            row_count(&root, "1"),
            1,
            "the failing account keeps its rows"
        );
        assert_eq!(
            row_count(&root, "2"),
            0,
            "the healthy account was cleared after the failure"
        );
        assert!(
            !store.file_for("1", "chat", "voice").exists(),
            "pending legacy plaintext still went away"
        );

        // Failing account last: its error must still surface after successes.
        let (_store, root, result, _before) = clear_with_failing_account(&["1", "9"], "9");
        assert!(result.is_err(), "a later failure must still be reported");
        assert_eq!(row_count(&root, "1"), 0, "the healthy account was cleared");
        assert_eq!(
            row_count(&root, "9"),
            1,
            "the failing account keeps its rows"
        );
    }
    #[test]
    fn fresh_beeper_profile_initializes_only_within_existing_account_and_ignores_unavailable_other_account()
     {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("accounts/1/beeper-drafts")).unwrap();
        let backend = Encrypted::new(root.path().into());
        let path = backend.path("1").unwrap();
        assert!(!path.exists());
        // Test key substitutes only OS-keyring retrieval; initializer and SQLCipher are real.
        let connection = crate::archive::transcript_connection_with_key(&path, &KEY).unwrap();
        connection.execute_batch("CREATE TABLE local_transcripts(chat TEXT NOT NULL,message TEXT NOT NULL,text TEXT NOT NULL,modified INTEGER NOT NULL,PRIMARY KEY(chat,message));").unwrap();
        backend
            .connections
            .lock()
            .unwrap()
            .insert("1".into(), connection);
        std::fs::create_dir_all(root.path().join("accounts/2")).unwrap();
        std::fs::write(
            root.path().join("accounts/2/archive.db"),
            b"unavailable archive fixture",
        )
        .unwrap();
        backend
            .put("1", "chat", "voice", "healthy save", 0, 1 << 20)
            .unwrap();
        assert_eq!(
            backend.get("1", "chat", "voice").as_deref(),
            Some("healthy save")
        );
        assert!(!root.path().join("accounts/3").exists());
    }

    #[test]
    fn failed_unlock_never_falls_back_to_plaintext_or_recreates_removed_account() {
        let root = tempfile::tempdir().unwrap();
        let backend = fixture(root.path(), &["1"]);
        backend
            .put("1", "chat", "voice", "protected words", 0, 1 << 20)
            .unwrap();
        drop(backend);
        let path = root.path().join("accounts/1/archive.db");
        let before = std::fs::read(&path).unwrap();
        assert!(crate::archive::transcript_connection_with_key(&path, &[9; 32]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let backend = Encrypted::new(root.path().into());
        assert!(
            backend
                .put("../x", "chat", "voice", "no", 0, 1 << 20)
                .is_err()
        );
        assert!(backend.put("2", "chat", "voice", "no", 0, 1 << 20).is_err());
        assert!(!root.path().join("accounts/2").exists());
    }
}
