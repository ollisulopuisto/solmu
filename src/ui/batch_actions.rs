//! Explicit destructive batch review and per-item outcomes. No automatic retries.
use crate::{
    app::App,
    i18n::{Locale, gettext, ngettext},
    model::{Action, Chat},
};

pub use crate::model::{BatchItem as Item, BatchState as State};

pub fn next_request() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn toggle_chat(app: &mut App, chat: &str) {
    let owner = app.account().id.0.clone();
    if app.batch_ui.owner != owner {
        if app
            .batch_ui
            .results
            .iter()
            .any(|item| item.result.is_none())
        {
            return;
        }
        app.batch_ui = State {
            owner,
            ..Default::default()
        };
    }
    if app.batch_ui.selected_chats.iter().any(|id| id == chat) {
        app.batch_ui.selected_chats.retain(|id| id != chat);
        app.batch_ui.selected_identities.remove(chat);
    } else {
        if let Some(identity) = app.chat(chat).and_then(crate::merged::Member::of) {
            app.batch_ui
                .selected_identities
                .insert(chat.into(), identity);
        }
        app.batch_ui.selected_chats.push(chat.into());
    }
}

pub fn archive_bar(app: &mut App, ui: &mut egui::Ui) {
    super::ghost_groups::bar(app, ui);
    if app.batch_ui.owner != app.account().id.0 {
        return;
    }
    if !app.batch_ui.selected_chats.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                ngettext(
                    app.locale,
                    "{count} chat selected",
                    "{count} chats selected",
                    app.batch_ui.selected_chats.len() as u32,
                )
                .replace("{count}", &app.batch_ui.selected_chats.len().to_string()),
            );
            if ui
                .button(crate::i18n::gettext(app.locale, "Archive selected…"))
                .clicked()
            {
                app.batch_ui.reviewing_archive = true;
            }
            if ui
                .button(crate::i18n::gettext(app.locale, "Clear selection"))
                .clicked()
            {
                app.batch_ui.selected_chats.clear();
                app.batch_ui.selected_identities.clear();
            }
        });
    }
    for item in &app.batch_ui.results {
        let status = match &item.result {
            None => gettext(app.locale, "Awaiting result; do not retry").into_owned(),
            Some(Ok(())) => gettext(app.locale, "Accepted").into_owned(),
            Some(Err(error)) => gettext(
                app.locale,
                "Failed or uncertain: {error}. Check state before any manual retry.",
            )
            .replace("{error}", error),
        };
        ui.label(format!(
            "{} {}: {status}",
            item.chat,
            item.message
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| gettext(app.locale, "archive").into_owned())
        ));
    }
    if !app.batch_ui.previous_session_results.is_empty() {
        ui.label(gettext(
            app.locale,
            "Unconfirmed or failed outcomes from a previous connection (not the current session)",
        ));
        for item in &app.batch_ui.previous_session_results {
            ui.label(
                gettext(
                    app.locale,
                    "{chat} {message}: check the previous connection before retrying",
                )
                .replace("{chat}", &item.chat)
                .replace("{message}", item.message.as_deref().unwrap_or("")),
            );
            if let Some(Err(error)) = &item.result {
                ui.label(error);
            }
        }

        if ui
            .button(gettext(app.locale, "Dismiss previous connection history"))
            .clicked()
        {
            app.batch_ui.previous_session_results.clear();
        }
    }
    if !app.batch_ui.results.is_empty()
        && app
            .batch_ui
            .results
            .iter()
            .all(|item| item.result.is_some())
        && ui
            .button(crate::i18n::gettext(app.locale, "Dismiss results"))
            .clicked()
    {
        app.batch_ui.results.clear();
    }
}

pub fn archive_refusal(chat: &Chat) -> Option<&'static str> {
    (chat.mutation_capabilities.archive != Some(true))
        .then_some("Archive support not confirmed by this chat's bridge")
}

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    if !app.batch_ui.reviewing_archive {
        return;
    }
    if app.batch_ui.owner != app.account().id.0 {
        app.batch_ui.reviewing_archive = false;
        return;
    }
    let ids = app.batch_ui.selected_chats.clone();
    let refusals: Vec<_> = ids
        .iter()
        .filter_map(|id| match app.chat(id) {
            None => Some(gettext(app.locale, "{id}: chat unavailable").replace("{id}", id)),
            Some(chat)
                if app
                    .batch_ui
                    .selected_identities
                    .get(id)
                    .is_none_or(|identity| !identity.matches(chat)) =>
            {
                Some(
                    gettext(
                        app.locale,
                        "Chat identity changed or is unavailable; clear the selection",
                    )
                    .into_owned(),
                )
            }
            Some(chat) => archive_refusal(chat).map(|why| {
                format!(
                    "{}: {}",
                    app.chat_title(chat),
                    refusal_text(app.locale, why)
                )
            }),
        })
        .collect();
    let pending = app
        .batch_ui
        .results
        .iter()
        .any(|item| item.result.is_none());
    egui::Modal::new(egui::Id::new("batch-archive-review")).show(ctx, |ui| {
        ui.heading(
            ngettext(
                app.locale,
                "Archive {count} chat?",
                "Archive {count} chats?",
                ids.len() as u32,
            )
            .replace("{count}", &ids.len().to_string()),
        );
        ui.label(crate::i18n::gettext(
            app.locale,
            "They remain available in Archived. Messages are not deleted.",
        ));
        for refusal in &refusals {
            ui.label(refusal);
        }
        if !refusals.is_empty() {
            ui.label(crate::i18n::gettext(
                app.locale,
                "Nothing will be sent. Remove unsupported chats from the selection first.",
            ));
        }
        if pending {
            ui.label(crate::i18n::gettext(
                app.locale,
                "An earlier batch still has pending results.",
            ));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !pending && refusals.is_empty() && !ids.is_empty(),
                    egui::Button::new(crate::i18n::gettext(app.locale, "Archive")),
                )
                .clicked()
            {
                let request = next_request();
                app.batch_ui.request = Some(request);
                app.batch_ui.results = ids
                    .iter()
                    .map(|chat| Item {
                        chat: chat.clone(),
                        message: None,
                        everyone: false,
                        result: None,
                    })
                    .collect();
                for chat in &ids {
                    app.actions.push(Action::BatchArchive {
                        request,
                        expected: app.batch_ui.selected_identities[chat].clone(),
                        chat: chat.clone(),
                    });
                }
                app.batch_ui.selected_chats.clear();
                app.batch_ui.selected_identities.clear();
                app.batch_ui.reviewing_archive = false;
            }
            if ui
                .button(crate::i18n::gettext(app.locale, "Cancel"))
                .clicked()
            {
                app.batch_ui.reviewing_archive = false;
            }
        });
    });
}

/// Keep refusal literals visible to gettext extraction, without translating backend logic.
pub fn refusal_text(locale: Locale, reason: &str) -> String {
    match reason {
        "Archive support not confirmed by this chat's bridge" => gettext(
            locale,
            "Archive support not confirmed by this chat's bridge",
        )
        .into_owned(),
        "Delete for me is not confirmed by this bridge" => {
            gettext(locale, "Delete for me is not confirmed by this bridge").into_owned()
        }
        "Delete for everyone is not confirmed by this bridge" => gettext(
            locale,
            "Delete for everyone is not confirmed by this bridge",
        )
        .into_owned(),
        "Only your own messages can be deleted for everyone here" => gettext(
            locale,
            "Only your own messages can be deleted for everyone here",
        )
        .into_owned(),
        "Message age could not be verified" => {
            gettext(locale, "Message age could not be verified").into_owned()
        }
        "The bridge's delete age limit has passed" => {
            gettext(locale, "The bridge's delete age limit has passed").into_owned()
        }
        _ => reason.to_owned(),
    }
}
