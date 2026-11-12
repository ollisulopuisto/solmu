//! On-demand, chat-scoped media history. Never downloads attachment bodies.
use super::*;

#[derive(Default)]
struct Album {
    chat: String,
    items: Vec<crate::model::MediaItem>,
    cursor: Option<String>,
    seen: HashSet<String>,
    started: bool,
    complete: bool,
}

impl Album {
    fn load(
        &mut self,
        chat: &str,
        around: Option<&str>,
        mut fetch: impl FnMut(Option<&str>) -> Result<Page<BeeperMessage>, String>,
    ) -> Result<(Vec<crate::model::MediaItem>, bool), String> {
        if self.chat != chat {
            *self = Self {
                chat: chat.into(),
                ..Self::default()
            };
        }
        // One gesture does bounded work. A sparse/very old chat can continue
        // on the next gesture instead of blocking sends or scanning forever.
        for _ in 0..10 {
            let position = around.and_then(|id| self.items.iter().position(|item| item.id == id));
            if self.complete || (self.started && position.is_some_and(|index| index > 0)) {
                break;
            }
            if self.items.len() >= 20_000 {
                return Err("Media history reached its 20000-item safety limit".into());
            }
            let page = fetch(self.cursor.as_deref())?;
            let next = history_cursor(
                page.oldest_cursor.as_deref(),
                self.cursor.as_deref(),
                page.has_more,
            )?;
            if page.has_more
                && next
                    .as_ref()
                    .is_some_and(|cursor| self.seen.contains(cursor))
            {
                return Err("Media history cursor repeated; earlier items are kept".into());
            }
            for wire in &page.items {
                if !wire.chat_id.is_empty() && wire.chat_id != chat {
                    return Err("Media history did not match the requested chat".into());
                }
                self.items.retain(|item| item.id != wire.id);
                if wire.is_hidden || wire.is_deleted {
                    continue;
                }
                let message = map_message(wire, chat);
                for item in crate::model::MediaItem::of_content(
                    &message.id,
                    message.timestamp,
                    &message.content,
                    None,
                    |_| false,
                ) {
                    if !self
                        .items
                        .iter()
                        .any(|known| known.id == item.id && known.card == item.card)
                    {
                        self.items.push(item);
                    }
                }
            }
            self.items.sort_by(|a, b| {
                a.timestamp
                    .cmp(&b.timestamp)
                    .then(a.id.cmp(&b.id))
                    .then(a.card.cmp(&b.card))
            });
            self.cursor = next;
            if let Some(cursor) = &self.cursor {
                self.seen.insert(cursor.clone());
            }
            self.complete = !page.has_more;
            self.started = true;
            // Newest-page opens do not need an older-history scan yet.
            if around.is_none() {
                break;
            }
        }
        Ok((self.items.clone(), !self.complete))
    }
}

pub(super) fn worker(
    api: Api,
    events: std::sync::mpsc::Sender<Event>,
    waker: Waker,
) -> std::sync::mpsc::SyncSender<(ChatId, Option<String>, bool)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<(ChatId, Option<String>, bool)>(1);
    std::thread::Builder::new()
        .name("beeper-gallery".into())
        .spawn(move || {
            let mut album = Album::default();
            while let Ok((chat, around, fresh)) = rx.recv() {
                if fresh {
                    album = Album::default();
                }
                let result = album.load(&chat, around.as_deref(), |cursor| {
                    api.list_messages(&chat, cursor, cursor.map(|_| "before"))
                });
                if events
                    .send(Event::BeeperViewerMedia {
                        chat,
                        around,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                waker.wake();
            }
        })
        .expect("Beeper gallery worker");
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(ids: &[(&str, i64)], cursor: Option<&str>, more: bool) -> Page<BeeperMessage> {
        serde_json::from_value(serde_json::json!({
            "items": ids.iter().map(|(id, stamp)| serde_json::json!({
                "id":id,"chatID":"c","timestamp":format!("2026-11-11T10:00:{stamp:02}Z"),
                "attachments":[{"id":id,"type":"img","mimeType":"image/png","srcURL":"mxc://example/test"}],
                "type":"IMAGE"
            })).collect::<Vec<_>>(),"hasMore":more,"oldestCursor":cursor
        }))
        .unwrap()
    }
    #[test]
    fn loads_across_messages_and_continues_only_at_the_older_edge() {
        let mut album = Album::default();
        let mut pages = vec![
            page(&[("b", 2), ("c", 3)], Some("next"), true),
            page(&[("a", 1)], None, false),
        ]
        .into_iter();
        let (items, more) = album
            .load("c", Some("c"), |_| Ok(pages.next().unwrap()))
            .unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert!(more);
        let (items, more) = album
            .load("c", Some("b"), |cursor| {
                assert_eq!(cursor, Some("next"));
                Ok(pages.next().unwrap())
            })
            .unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert!(!more);
        album
            .load("c", Some("a"), |_| panic!("true end does not fetch"))
            .unwrap();
    }
    #[test]
    fn missing_and_repeated_cursors_fail_instead_of_claiming_an_end() {
        let mut album = Album::default();
        assert!(
            album
                .load("c", Some("old"), |_| Ok(page(&[], None, true)))
                .is_err()
        );
        assert!(
            album
                .load("c", Some("old"), |_| Ok(page(&[], Some("same"), true)))
                .is_err()
        );
    }
    #[test]
    fn wrong_chat_never_enters_the_album() {
        let mut album = Album::default();
        assert!(
            album
                .load("other", None, |_| Ok(page(&[("a", 1)], None, false)))
                .is_err()
        );
    }
}
