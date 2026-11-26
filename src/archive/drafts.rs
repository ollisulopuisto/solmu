//! Per-chat composer drafts, so unsent text survives a restart.
//!
//! Drafts hold text the user typed and did not send, so they live in the
//! encrypted archive next to the messages they belong to and are dropped with
//! the chat.

use super::{Archive, Result, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS drafts (
    chat TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0
);
";

impl Archive {
    pub fn write_draft_state(
        &self,
        chat: &str,
        text: &str,
        shelf_key: &str,
        shelf: &str,
        at: i64,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        for (key, value) in [(chat, text), (shelf_key, shelf)] {
            tx.execute("INSERT INTO drafts (chat,text,updated_at) VALUES (?1,?2,?3) ON CONFLICT(chat) DO UPDATE SET text=excluded.text,updated_at=excluded.updated_at", params![key,value,at])?;
        }
        tx.commit()
    }
    /// Every stored draft, so the composer can restore them at startup.
    pub fn drafts(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare("SELECT chat, text FROM drafts")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Stores a draft for a chat. An empty or blank text clears the row
    /// instead, because a blank composer is not a draft.
    pub fn set_draft(&self, chat: &str, text: &str, at: i64) -> Result<()> {
        if text.trim().is_empty() {
            return self.clear_draft(chat);
        }
        self.connection.execute(
            "INSERT INTO drafts (chat, text, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET
                text = excluded.text,
                updated_at = excluded.updated_at",
            params![chat, text, at],
        )?;
        Ok(())
    }

    /// Drops the draft for a chat, for example once its text is sent.
    pub fn clear_draft(&self, chat: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM drafts WHERE chat = ?1", params![chat])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_draft_commands_save_and_ack_without_a_network_request() {
        use crate::backend::beeper::{local_draft_command, stored_drafts};
        use crate::backend::{Command, Event};
        use std::sync::Mutex;
        let root = tempfile::tempdir().unwrap();
        let mut store = Some(crate::archive::Archive::in_memory().unwrap());
        let path = root.path().join("unused.db");
        let origin = "http://127.0.0.1:1";
        let events = Mutex::new(Vec::new());
        let emit = |event| events.lock().unwrap().push(event);
        let mut warning_shown = false;
        assert!(local_draft_command(
            Command::SaveDraft {
                chat: "one".into(),
                text: "offline text".into()
            },
            true,
            &mut store,
            &path,
            origin,
            &emit,
            &mut warning_shown
        ));
        assert!(
            stored_drafts(store.as_ref().unwrap(), origin)
                .unwrap()
                .iter()
                .any(|(chat, text)| chat == "one" && text == "offline text")
        );
        assert!(local_draft_command(
            Command::WriteDraftState {
                chat: "one".into(),
                revision: 7,
                text: "new offline text".into(),
                shelf: Vec::new()
            },
            true,
            &mut store,
            &path,
            origin,
            &emit,
            &mut warning_shown
        ));
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            event,
            Event::DraftWritten {
                revision: 7,
                ok: true,
                ..
            }
        )));
        assert!(
            stored_drafts(store.as_ref().unwrap(), origin)
                .unwrap()
                .iter()
                .any(|(chat, text)| chat == "one" && text == "new offline text")
        );
        assert!(local_draft_command(
            Command::WriteDraftState {
                chat: "one".into(),
                revision: 8,
                text: "unread shelf".into(),
                shelf: Vec::new()
            },
            false,
            &mut store,
            &path,
            origin,
            &emit,
            &mut warning_shown
        ));
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            event,
            Event::DraftWritten {
                revision: 8,
                ok: false,
                ..
            }
        )));
        assert!(!local_draft_command(
            Command::Shutdown,
            true,
            &mut store,
            &path,
            origin,
            &emit,
            &mut warning_shown
        ));
    }

    #[test]
    fn composer_database_uses_full_sync_and_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("durable-drafts.db");
        let key = [37; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.enable_draft_durability().unwrap();
            let sync: i64 = archive
                .connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(sync, 2, "FULL, not NORMAL");
            archive.set_draft("chat", "unsent composer", 1).unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert_eq!(
            archive.drafts().unwrap(),
            vec![("chat".into(), "unsent composer".into())]
        );
    }

    #[test]
    fn locked_start_then_unlock_cannot_replace_unread_shelf() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("drafts.db");
        let archive = crate::archive::Archive::in_memory().unwrap();
        let origin = "http://localhost:23373";
        let chat = "one";
        let key = serde_json::to_string(&(origin, chat, "parked-v1")).unwrap();
        archive
            .write_draft_state(
                &crate::backend::beeper::draft_namespace(origin, chat),
                "original",
                &key,
                "retained",
                1,
            )
            .unwrap();
        // Startup failed before reading the shelf. The keyring/store then becomes available.
        let mut unlocked = Some(archive);
        assert!(!crate::backend::beeper::write_local_draft_state(
            false,
            &mut unlocked,
            &path,
            origin,
            chat,
            "replace",
            &[]
        ));
        let rows = unlocked.unwrap().drafts().unwrap();
        assert!(rows.iter().any(|(k, t)| k == &key && t == "retained"));
        assert!(rows.iter().any(|(k, t)| k
            == &crate::backend::beeper::draft_namespace(origin, chat)
            && t == "original"));
    }

    #[test]
    fn beeper_drafts_are_encrypted_durable_and_backend_isolated() {
        use crate::backend::beeper::{draft_namespace, stored_drafts};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("drafts.db");
        let key = [29; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive
                .set_draft(
                    &draft_namespace("http://127.0.0.1:23373", "same-chat"),
                    "private original",
                    1,
                )
                .unwrap();
            archive
                .set_draft(
                    &draft_namespace("http://127.0.0.1:8077", "same-chat"),
                    "separate endpoint",
                    2,
                )
                .unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows(b"private original".len())
                .any(|window| window == b"private original")
        );
        assert_ne!(&bytes[..16], b"SQLite format 3\0");
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert_eq!(
            stored_drafts(&archive, "http://127.0.0.1:23373").unwrap(),
            vec![("same-chat".into(), "private original".into())]
        );
        assert_eq!(
            stored_drafts(&archive, "http://127.0.0.1:8077").unwrap(),
            vec![("same-chat".into(), "separate endpoint".into())]
        );
        archive
            .set_draft(
                &draft_namespace("http://127.0.0.1:23373", "same-chat"),
                "",
                3,
            )
            .unwrap();
        assert!(
            stored_drafts(&archive, "http://127.0.0.1:23373")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            stored_drafts(&archive, "http://127.0.0.1:8077")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn drafts_round_trip_survive_a_restart_and_clear() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [17; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat("1@s.whatsapp.net", "A").unwrap();
            archive.ensure_chat("2@s.whatsapp.net", "B").unwrap();
            archive.set_draft("1@s.whatsapp.net", "unsent", 10).unwrap();
            archive.set_draft("2@s.whatsapp.net", "other", 11).unwrap();
            // The same chat keeps one row, whatever the text.
            archive
                .set_draft("1@s.whatsapp.net", "unsent again", 12)
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        let mut drafts = archive.drafts().unwrap();
        drafts.sort();
        assert_eq!(
            drafts,
            vec![
                ("1@s.whatsapp.net".to_owned(), "unsent again".to_owned()),
                ("2@s.whatsapp.net".to_owned(), "other".to_owned()),
            ]
        );

        // A blank composer is not a draft.
        archive.set_draft("1@s.whatsapp.net", "   ", 13).unwrap();
        assert_eq!(
            archive.drafts().unwrap(),
            vec![("2@s.whatsapp.net".to_owned(), "other".to_owned())]
        );

        // Sending the text, or dropping the chat, drops the draft.
        archive.clear_draft("2@s.whatsapp.net").unwrap();
        assert!(archive.drafts().unwrap().is_empty());
        archive.set_draft("2@s.whatsapp.net", "again", 14).unwrap();
        archive.clear_chat("2@s.whatsapp.net").unwrap();
        assert!(archive.drafts().unwrap().is_empty());
    }
}

#[cfg(test)]
mod multi_tests {
    use super::*;
    #[test]
    fn active_and_shelf_are_written_together_without_touching_another_chat() {
        let archive = Archive::in_memory().unwrap();
        archive.set_draft("other", "unchanged", 1).unwrap();
        archive
            .write_draft_state("active", "new", "shelf", "[]", 2)
            .unwrap();
        let rows = archive.drafts().unwrap();
        assert!(rows.contains(&("active".into(), "new".into())));
        assert!(rows.contains(&("shelf".into(), "[]".into())));
        assert!(rows.contains(&("other".into(), "unchanged".into())));
        archive.set_draft("active", "", 3).unwrap();
        assert!(
            archive
                .drafts()
                .unwrap()
                .contains(&("shelf".into(), "[]".into()))
        );
    }
}
