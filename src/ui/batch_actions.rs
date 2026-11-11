//! Explicit destructive batch review and per-item outcomes. No automatic retries.
use crate::{
    app::App,
    model::{Action, Chat, ChatId},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub chat: ChatId,
    pub message: Option<String>,
    pub everyone: bool,
    pub result: Option<Result<(), String>>,
}
#[derive(Default)]
pub struct State {
    pub request: Option<u64>,
    pub selected_chats: Vec<ChatId>,
    pub owner: String,
    pub reviewing_archive: bool,
    pub results: Vec<Item>,
}

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
    } else {
        app.batch_ui.selected_chats.push(chat.into());
    }
}

pub fn archive_bar(app: &mut App, ui: &mut egui::Ui) {
    if app.batch_ui.owner != app.account().id.0 {
        return;
    }
    if !app.batch_ui.selected_chats.is_empty() {
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} chats selected",
                app.batch_ui.selected_chats.len()
            ));
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
            }
        });
    }
    for item in &app.batch_ui.results {
        let status = match &item.result {
            None => "Awaiting result; do not retry".to_owned(),
            Some(Ok(())) => "Accepted".to_owned(),
            Some(Err(error)) => {
                format!("Failed or uncertain: {error}. Check state before any manual retry.")
            }
        };
        ui.label(format!(
            "{} {}: {status}",
            item.chat,
            item.message.as_deref().unwrap_or("archive")
        ));
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
            None => Some(format!("{id}: chat unavailable")),
            Some(chat) => {
                archive_refusal(chat).map(|why| format!("{}: {why}", app.chat_title(chat)))
            }
        })
        .collect();
    let pending = app
        .batch_ui
        .results
        .iter()
        .any(|item| item.result.is_none());
    egui::Modal::new(egui::Id::new("batch-archive-review")).show(ctx, |ui| {
        ui.heading(format!("Archive {} chats?", ids.len()));
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
                        chat: chat.clone(),
                    });
                }
                app.batch_ui.selected_chats.clear();
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
