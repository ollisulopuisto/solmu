//! The command palette (Ctrl+P): keyboard-first access to chats, quick
//! filters and common actions. Message search stays in the existing
//! search fields; the palette finds and acts, Alfred-style.

use egui::{Key, Modifiers, Sense, vec2};

use super::{theme, widgets};
use crate::app::App;
use crate::model::{Action, ChatFilter, ChatId, Dialog, Page};

/// One executable palette row.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteRow {
    /// Opens the chat.
    Chat(ChatId),
    /// Applies a chat-list filter and shows the list.
    Filter(ChatFilter),
    /// Runs a fixed command.
    Command(PaletteCommand),
}

/// The fixed commands the palette offers. Each maps onto an existing
/// action, so the palette adds no new behavior of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCommand {
    SearchChats,
    NewChat,
    Unreplied,
    ReplyLater,
    Settings,
    Shortcuts,
    WhatsNew,
    ScheduledQueue,
    SendLater,
}

impl PaletteCommand {
    const EVERY: [Self; 9] = [
        Self::SearchChats,
        Self::NewChat,
        Self::Unreplied,
        Self::ReplyLater,
        Self::Settings,
        Self::Shortcuts,
        Self::WhatsNew,
        Self::ScheduledQueue,
        Self::SendLater,
    ];

    fn title(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        use crate::i18n::gettext;
        match self {
            Self::SearchChats => gettext(locale, "Search chats"),
            Self::NewChat => gettext(locale, "New chat"),
            Self::Unreplied => gettext(locale, "Show unreplied chats"),
            Self::ReplyLater => gettext(locale, "Show reply-later queue"),
            Self::Settings => gettext(locale, "Open settings"),
            Self::Shortcuts => gettext(locale, "Keyboard shortcuts"),
            Self::WhatsNew => gettext(locale, "What's New"),
            Self::ScheduledQueue => gettext(locale, "Send Later queue"),
            Self::SendLater => gettext(locale, "Schedule this message"),
        }
    }
}

/// A rendered row: what it says and what it does.
pub struct PaletteEntry {
    pub title: String,
    pub detail: String,
    pub row: PaletteRow,
}

/// Computes the palette rows for a query. Pure over app state, so the
/// ordering and matching are testable without a frame.
///
/// Order: matching chats (list order, at most 8), then filters, then
/// commands. An empty query shows the first chats, every filter and
/// every command, so the palette is discoverable before typing.
pub fn entries(app: &App, query: &str) -> Vec<PaletteEntry> {
    let needle = query.trim().to_lowercase();
    let mut out = Vec::new();
    for chat in &app.chats {
        let title = app.chat_title(chat);
        if !needle.is_empty() && !title.to_lowercase().contains(&needle) {
            continue;
        }
        let detail = chat.network.clone().unwrap_or_default();
        out.push(PaletteEntry {
            title,
            detail,
            row: PaletteRow::Chat(chat.id.clone()),
        });
        if out.len() >= 8 {
            break;
        }
    }
    for filter in ChatFilter::EVERY {
        let title = filter.label(app.locale).into_owned();
        if !needle.is_empty() && !title.to_lowercase().contains(&needle) {
            continue;
        }
        out.push(PaletteEntry {
            detail: crate::i18n::gettext(app.locale, "Filter").into_owned(),
            title,
            row: PaletteRow::Filter(filter),
        });
    }
    for command in PaletteCommand::EVERY {
        if matches!(
            command,
            PaletteCommand::Unreplied | PaletteCommand::ReplyLater
        ) && app.beeper_browse_networks().is_empty()
        {
            // The Beeper-only filters only make sense with Beeper chats.
            continue;
        }
        let title = command.title(app.locale).into_owned();
        if !needle.is_empty() && !title.to_lowercase().contains(&needle) {
            continue;
        }
        out.push(PaletteEntry {
            detail: crate::i18n::gettext(app.locale, "Action").into_owned(),
            title,
            row: PaletteRow::Command(command),
        });
    }
    out
}

/// Executes a row: the matching existing action, then close.
fn execute(app: &mut App, row: &PaletteRow) {
    match row {
        PaletteRow::Chat(id) => {
            app.actions.push(Action::OpenChat(id.clone()));
        }
        PaletteRow::Filter(filter) => {
            app.beeper_unreplied = false;
            app.actions.push(Action::SetChatFilter(*filter));
            app.actions.push(Action::Open(Page::Chats));
        }
        PaletteRow::Command(command) => match command {
            PaletteCommand::SearchChats => app.actions.push(Action::FocusSearch),
            PaletteCommand::NewChat => app.actions.push(Action::ShowDialog(Dialog::NewChat)),
            PaletteCommand::Unreplied => {
                app.beeper_unreplied = true;
                app.actions.push(Action::SetChatFilter(ChatFilter::All));
                app.actions.push(Action::Open(Page::Chats));
            }
            PaletteCommand::ReplyLater => {
                app.beeper_unreplied = false;
                app.actions
                    .push(Action::SetChatFilter(ChatFilter::ReplyLater));
                app.actions.push(Action::Open(Page::Chats));
            }
            PaletteCommand::Settings => app.actions.push(Action::ToggleSettings),
            PaletteCommand::Shortcuts => app.actions.push(Action::ShowDialog(Dialog::Shortcuts)),
            PaletteCommand::WhatsNew => app.actions.push(Action::ShowDialog(Dialog::WhatsNew)),
            PaletteCommand::ScheduledQueue => {
                app.actions.push(Action::ShowDialog(Dialog::ScheduledList))
            }
            PaletteCommand::SendLater => app.actions.push(Action::ShowDialog(Dialog::SendLater)),
        },
    }
    app.actions.push(Action::CloseDialog);
}

/// Renders the palette dialog.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let entries = entries(app, &app.palette_query);
    if entries.is_empty() {
        app.palette_selected = 0;
    } else if app.palette_selected >= entries.len() {
        app.palette_selected = entries.len() - 1;
    }

    // Consume navigation before the query field can move its caret.
    let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    if down && !entries.is_empty() {
        app.palette_selected = (app.palette_selected + 1) % entries.len();
    }
    if up && !entries.is_empty() {
        app.palette_selected = (app.palette_selected + entries.len() - 1) % entries.len();
    }
    let chosen = if enter && !entries.is_empty() {
        Some(entries[app.palette_selected].row.clone())
    } else {
        None
    };

    let width = ui.available_width();
    let hint = crate::i18n::gettext(app.locale, "Chats, filters, actions");
    let search = widgets::search_field(
        ui,
        &palette,
        egui::Id::new("command-palette-query"),
        &mut app.palette_query,
        &hint,
        width,
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        search.request_focus();
    }
    if search.changed() {
        app.palette_selected = 0;
    }

    ui.add_space(6.0);
    if entries.is_empty() {
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "No matches"),
            theme::regular(13.0),
            palette.secondary,
        );
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("command-palette-rows")
        .max_height((ui.ctx().content_rect().height() - 280.0).clamp(120.0, 380.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, entry) in entries.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                if index == app.palette_selected || response.hovered() {
                    widgets::row_highlight(ui, &palette, rect, palette.surface_active);
                }
                if response.hovered() {
                    app.palette_selected = index;
                }
                ui.painter().text(
                    rect.left_center() + vec2(10.0, -8.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.title,
                    theme::regular(14.0),
                    palette.text,
                );
                ui.painter().text(
                    rect.left_center() + vec2(10.0, 9.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.detail,
                    theme::regular(11.0),
                    palette.secondary,
                );
                if index == app.palette_selected {
                    ui.painter().text(
                        rect.right_center() + vec2(-10.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        "⏎",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                }
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        index == app.palette_selected,
                        format!("{}, {}", entry.title, entry.detail),
                    )
                });
                if response.clicked() {
                    clicked = Some(entry.row.clone());
                }
            }
        });
    if let Some(row) = clicked.or(chosen) {
        execute(app, &row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let (app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        (root, app)
    }

    #[test]
    fn empty_query_lists_chats_filters_and_commands() {
        let (_root, app) = app();
        let rows = entries(&app, "");
        assert!(
            rows.iter()
                .any(|e| matches!(e.row, PaletteRow::Filter(ChatFilter::Unread)))
        );
        assert!(
            rows.iter()
                .any(|e| matches!(e.row, PaletteRow::Command(PaletteCommand::Settings)))
        );
        // The Beeper-only commands hide without Beeper networks.
        assert!(
            !rows
                .iter()
                .any(|e| matches!(e.row, PaletteRow::Command(PaletteCommand::Unreplied)))
        );
    }

    #[test]
    fn query_matches_chat_titles_case_insensitively() {
        let (_root, mut app) = app();
        let mut chat = crate::model::Chat::new("1@example".into(), "Mikael".into());
        chat.network = Some("WhatsApp".into());
        app.chats.push(chat);
        let rows = entries(&app, "mik");
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0].row, PaletteRow::Chat(id) if id == "1@example"));
        assert_eq!(rows[0].detail, "WhatsApp");
        // No match: only the command/filter rows that contain the needle.
        assert!(
            entries(&app, "zzz-no-such-chat")
                .iter()
                .all(|e| !matches!(e.row, PaletteRow::Chat(_)))
        );
    }

    #[test]
    fn chat_rows_cap_at_eight() {
        let (_root, mut app) = app();
        for i in 0..12 {
            app.chats.push(crate::model::Chat::new(
                format!("{i}@example"),
                format!("Chat {i}"),
            ));
        }
        let chats = entries(&app, "")
            .iter()
            .filter(|e| matches!(e.row, PaletteRow::Chat(_)))
            .count();
        assert_eq!(chats, 8);
    }
}
