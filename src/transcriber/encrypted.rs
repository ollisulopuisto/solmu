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
            .map_or(true, |m| !m.is_file() || m.file_type().is_symlink())
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
            let tx = c.transaction().map_err(io)?;
            tx.execute("INSERT INTO local_transcripts(chat,message,text,modified) VALUES (?1,?2,?3,?4) ON CONFLICT(chat,message) DO UPDATE SET text=excluded.text, modified=excluded.modified", params![chat,message,text,modified]).map_err(io)?;
            tx.commit().map_err(io)?;
            Ok(())
        })?;
        // Preserve the existing global byte cap across account archives.
        let mut rows = Vec::new();
        let mut total = 0u64;
        for owner in self.accounts() {
            let sizes = self.with(&owner, |c| {
                let mut statement = c.prepare("SELECT chat,message,length(CAST(text AS BLOB)),modified FROM local_transcripts").map_err(io)?;
                let rows = statement.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)? as u64,row.get::<_,i64>(3)?))).map_err(io)?;
                rows.collect::<Result<Vec<_>,_>>().map_err(io)
            })?;
            for (chat, message, size, modified) in sizes {
                total += size;
                rows.push((modified, owner.clone(), chat, message, size));
            }
        }
        rows.sort();
        for (_, owner, chat, message, size) in rows {
            if total <= max {
                break;
            }
            self.remove(&owner, Some(&chat), Some(&message))?;
            total = total.saturating_sub(size);
        }
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
    /// One account transaction is committed and re-read before any source file
    /// is removed. Interrupted migration is safe to repeat; rows win on conflict.
    pub fn migrate(&self, root: &Path) -> std::io::Result<()> {
        for encoded in super::subdirs(root) {
            let account =
                super::decode(&encoded).ok_or_else(|| io("Invalid legacy transcript account"))?;
            let mut source = Vec::new();
            for chat_dir in super::subdirs(&root.join(&encoded)) {
                let chat =
                    super::decode(&chat_dir).ok_or_else(|| io("Invalid legacy transcript chat"))?;
                for entry in std::fs::read_dir(root.join(&encoded).join(&chat_dir))? {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    let name = entry.file_name();
                    let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".txt")) else {
                        continue;
                    };
                    let message = super::decode(stem)
                        .ok_or_else(|| io("Invalid legacy transcript message"))?;
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
                continue;
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
            for (_, _, _, _, path) in source {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
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
        backend.migrate(&legacy).unwrap();
        assert!(store.get("1", "chat", "voice").is_none());
        assert_eq!(
            backend.get("1", "chat", "voice").as_deref(),
            Some("legacy private words")
        );
        backend.migrate(&legacy).unwrap();
        store
            .put("1", "chat", "voice", "different source must not be lost")
            .unwrap();
        assert!(backend.migrate(&legacy).is_err());
        assert_eq!(
            store.get("1", "chat", "voice").as_deref(),
            Some("different source must not be lost")
        );
        store
            .put("2", "chat", "voice", "no account or key yet")
            .unwrap();
        assert!(backend.migrate(&legacy).is_err());
        assert!(store.get("2", "chat", "voice").is_some());
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
