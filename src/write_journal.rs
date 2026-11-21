//! Durable metadata-only claim-before-send journal. A claimed or uncertain request is never retried automatically.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Outcome {
    Claimed,
    Accepted { message_id: String },
    Rejected,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    digest: String,
    at: i64,
    outcome: Outcome,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    entries: BTreeMap<String, Entry>,
    #[serde(default)]
    attempts: Vec<Attempt>,
    #[serde(default)]
    attempts_total: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    client_hash: String,
    at: i64,
}
pub enum Claim {
    New,
    Existing(Outcome),
}
pub struct WriteJournal {
    path: PathBuf,
}
impl WriteJournal {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn lock(&self) -> Result<std::fs::File, String> {
        // Never unlink/recover a pathname. The OS owns exclusivity and releases
        // it on close or process exit, including crashes on Windows.
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(self.path.with_extension("writes.oslock"))
            .map_err(|_| "write audit lock unavailable")?;
        file.lock().map_err(|_| "write audit lock unavailable")?;
        Ok(file)
    }
    fn load(&self) -> Result<Journal, String> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Journal::default()),
            Err(_) => return Err("write audit unavailable".into()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err("write audit permissions must be 0600".into());
            }
        }
        if meta.len() > 8 * 1024 * 1024 {
            return Err("write audit full; archive explicitly before granting more writes".into());
        }
        serde_json::from_slice(&std::fs::read(&self.path).map_err(|_| "write audit unavailable")?)
            .map_err(|_| "write audit invalid".into())
    }
    fn save(&self, journal: &Journal) -> Result<(), String> {
        use std::io::Write;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut tmp =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "write audit unavailable")?;
        serde_json::to_writer(tmp.as_file_mut(), journal)
            .map_err(|_| "write audit serialization failed")?;
        tmp.as_file_mut()
            .flush()
            .map_err(|_| "write audit flush failed")?;
        tmp.as_file()
            .sync_all()
            .map_err(|_| "write audit sync failed")?;
        tmp.persist(&self.path)
            .map_err(|_| "write audit persistence failed")?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| "write audit directory sync failed")?;
        Ok(())
    }
    /// Record receipt before inspecting write content, including denied callers.
    pub fn audit_attempt(&self, client_hash: &str, at: i64) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut journal = self.load()?;
        // Receipt history is a bounded recent window, not the durable claim
        // ledger. Rejected traffic must never consume idempotency capacity.
        if journal.attempts.len() >= 10000 {
            journal.attempts.drain(..journal.attempts.len() - 9999);
        }
        journal.attempts_total = journal.attempts_total.saturating_add(1);
        journal.attempts.push(Attempt {
            client_hash: client_hash.into(),
            at,
        });
        self.save(&journal)
    }
    pub fn claim(
        &self,
        client_hash: &str,
        request_id: &str,
        payload_digest: &str,
        at: i64,
    ) -> Result<Claim, String> {
        let _lock = self.lock()?;
        let mut journal = self.load()?;
        let key = format!("{client_hash}:{request_id}");
        if let Some(entry) = journal.entries.get(&key) {
            return if entry.digest == payload_digest {
                Ok(Claim::Existing(entry.outcome.clone()))
            } else {
                Err("request id already used for different content or destination".into())
            };
        }
        if journal.entries.len() >= 10000 {
            return Err("write audit full".into());
        }
        journal.entries.insert(
            key,
            Entry {
                digest: payload_digest.into(),
                at,
                outcome: Outcome::Claimed,
            },
        );
        self.save(&journal)?;
        Ok(Claim::New)
    }
    pub fn finish(
        &self,
        client_hash: &str,
        request_id: &str,
        outcome: Outcome,
    ) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut journal = self.load()?;
        let entry = journal
            .entries
            .get_mut(&format!("{client_hash}:{request_id}"))
            .ok_or("write audit claim missing")?;
        if entry.outcome != Outcome::Claimed {
            return Err("write attempt already finished".into());
        }
        entry.outcome = outcome;
        self.save(&journal)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_claim_prevents_resend_and_content_substitution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.json");
        let j = WriteJournal::new(path.clone());
        assert!(matches!(
            j.claim("tokenhash", "r", "digest", 1).unwrap(),
            Claim::New
        ));
        let j = WriteJournal::new(path);
        assert!(matches!(
            j.claim("tokenhash", "r", "digest", 2).unwrap(),
            Claim::Existing(Outcome::Claimed)
        ));
        assert!(j.claim("tokenhash", "r", "different", 2).is_err());
        j.finish(
            "tokenhash",
            "r",
            Outcome::Accepted {
                message_id: "m".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            j.claim("tokenhash", "r", "digest", 3).unwrap(),
            Claim::Existing(Outcome::Accepted { .. })
        ));
        assert!(j.finish("tokenhash", "r", Outcome::Unknown).is_err());
    }
    #[test]
    fn unavailable_storage_refuses_claim() {
        let j = WriteJournal::new(PathBuf::from("/nonexistent/audit/record"));
        assert!(j.claim("hash", "r", "digest", 1).is_err());
    }
}
