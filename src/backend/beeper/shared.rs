//! Bounded metadata-only history for the shared-items panel.
use super::*;

fn collect(
    chat: &str,
    mut fetch: impl FnMut(Option<&str>) -> Result<Page<BeeperMessage>, String>,
) -> Result<crate::model::ChatMedia, String> {
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    let mut incomplete = true;
    for _ in 0..10 {
        let page = fetch(cursor.as_deref())?;
        let next = history_cursor(
            page.oldest_cursor.as_deref(),
            cursor.as_deref(),
            page.has_more,
        )?;
        if page.has_more && next.as_ref().is_some_and(|c| !seen.insert(c.clone())) {
            return Err("Shared-items history cursor repeated".into());
        }
        let mut row_cap_hit = false;
        for wire in page.items {
            if !wire.chat_id.is_empty() && wire.chat_id != chat {
                return Err("Shared-items history did not match the requested chat".into());
            }
            // Newest occurrence wins, including a deleted/hidden tombstone.
            if !ids.insert(wire.id.clone()) || wire.is_hidden || wire.is_deleted {
                continue;
            }
            rows.push(map_message(&wire, chat));
            if rows.len() >= 20_000 {
                row_cap_hit = true;
                break;
            }
        }
        incomplete = page.has_more || row_cap_hit;
        cursor = next;
        if !incomplete || rows.len() >= 20_000 {
            break;
        }
    }
    let mut listing =
        crate::model::ChatMedia::collect(chat.into(), rows, crate::model::MEDIA_LIST_LIMIT);
    listing.history_incomplete = incomplete;
    Ok(listing)
}

pub(super) fn worker(
    api: Api,
    events: std::sync::mpsc::Sender<Event>,
    waker: Waker,
) -> std::sync::mpsc::SyncSender<(ChatId, u64)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<(ChatId, u64)>(1);
    std::thread::Builder::new()
        .name("beeper-shared-items".into())
        .spawn(move || {
            while let Ok((chat, request)) = rx.recv() {
                let result = collect(&chat, |cursor| {
                    api.list_messages(&chat, cursor, cursor.map(|_| "before"))
                });
                if events.send(Event::ChatShared { request, result }).is_err() {
                    break;
                }
                waker.wake();
            }
        })
        .expect("Beeper shared-items worker");
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(
        chat: &str,
        id: &str,
        kind: &str,
        cursor: Option<&str>,
        more: bool,
    ) -> Page<BeeperMessage> {
        serde_json::from_value(serde_json::json!({
            "items": [{ "id": id, "chatID": chat, "timestamp": "2026-11-11T10:00:00Z",
                "attachments": [{ "id": id, "type": kind, "mimeType": if kind == "audio" { "audio/ogg" } else { "image/png" }, "srcURL":"mxc://example/test" }],
                "type": if kind == "audio" { "AUDIO" } else { "IMAGE" } }],
            "hasMore": more, "oldestCursor": cursor
        })).unwrap()
    }
    #[test]
    fn lists_audio_and_images_and_returns_completion_without_downloading() {
        let mut pages = vec![
            page("c", "photo", "img", Some("older"), true),
            page("c", "voice", "audio", None, false),
        ]
        .into_iter();
        let result = collect("c", |_| Ok(pages.next().unwrap())).unwrap();
        assert_eq!(result.media.len(), 1);
        assert_eq!(result.voice.len(), 1);
        assert!(!result.history_incomplete);
        assert!(result.voice[0].content.media().unwrap().path.is_none());
    }
    #[test]
    fn bounded_scan_reports_partial_history_and_rejects_bad_scope_or_cursor() {
        let mut calls = 0;
        let result = collect("c", |_| {
            calls += 1;
            Ok(page(
                "c",
                &format!("m{calls}"),
                "img",
                Some(&calls.to_string()),
                true,
            ))
        })
        .unwrap();
        assert_eq!(calls, 10);
        assert!(result.history_incomplete);
        assert!(collect("c", |_| Ok(page("elsewhere", "x", "img", None, false))).is_err());
        assert!(collect("c", |_| Ok(page("c", "x", "img", Some("same"), true))).is_err());
        assert!(collect("c", |_| Err("offline".into())).is_err());
    }
}
