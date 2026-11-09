//! The command palette (Ctrl+P): keyboard-first access to chats, quick
//! filters and common actions. Message search stays in the existing
//! search fields; the palette finds and acts, Alfred-style.

use egui::{Key, Modifiers, Sense, vec2};

use super::{keys, theme, widgets};
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
    JumpToChat,
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
    const EVERY: [Self; 10] = [
        Self::SearchChats,
        Self::JumpToChat,
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
            Self::JumpToChat => gettext(locale, "Jump to open chat"),
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

    /// The registry id of the key that does the same thing, so the row can
    /// show it. Commands with no key of their own show none.
    fn shortcut_id(self) -> Option<&'static str> {
        match self {
            Self::SearchChats => Some("find"),
            Self::JumpToChat => Some("jump-to-chat"),
            Self::NewChat => Some("new-chat"),
            Self::Settings => Some("settings"),
            Self::Shortcuts => Some("shortcuts"),
            _ => None,
        }
    }
}

/// A rendered row: what it says and what it does.
pub struct PaletteEntry {
    pub title: String,
    pub detail: String,
    /// The key that does the same thing, in the platform's own notation.
    pub keys: Option<String>,
    pub row: PaletteRow,
}

impl PaletteRow {
    /// Filters can be tried one after another, so the palette stays open
    /// after one. Opening a chat or running a command is a one-off: it closes.
    fn keeps_palette_open(&self) -> bool {
        matches!(self, Self::Filter(_))
    }
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
            keys: None,
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
            keys: None,
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
            keys: command
                .shortcut_id()
                .and_then(|id| keys::shortcut(id).label)
                .map(keys::label),
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
            PaletteCommand::JumpToChat => app.actions.push(Action::ShowDialog(Dialog::JumpToChat)),
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
    if !row.keeps_palette_open() {
        app.actions.push(Action::CloseDialog);
    }
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
                let hint_x = if index == app.palette_selected {
                    ui.painter().text(
                        rect.right_center() + vec2(-10.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        "⏎",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                    -34.0
                } else {
                    -10.0
                };
                if let Some(hint) = &entry.keys {
                    ui.painter().text(
                        rect.right_center() + vec2(hint_x, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        hint,
                        theme::regular(12.0),
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
    fn commands_show_their_keys_and_chats_do_not() {
        let (_root, app) = app();
        let rows = entries(&app, "");
        let keys_of = |wanted: PaletteCommand| {
            rows.iter()
                .find(|e| e.row == PaletteRow::Command(wanted))
                .and_then(|e| e.keys.clone())
        };
        assert_eq!(keys_of(PaletteCommand::JumpToChat), Some(keys::label("Ctrl+K")));
        assert_eq!(keys_of(PaletteCommand::Settings), Some(keys::label("Ctrl+,")));
        assert_eq!(keys_of(PaletteCommand::WhatsNew), None);
    }

    #[test]
    fn filters_keep_the_palette_open_and_everything_else_closes_it() {
        let (_root, mut app) = app();
        execute(&mut app, &PaletteRow::Filter(ChatFilter::Unread));
        assert!(!app.actions.iter().any(|a| matches!(a, Action::CloseDialog)));
        app.actions.clear();
        execute(&mut app, &PaletteRow::Command(PaletteCommand::Settings));
        assert!(app.actions.iter().any(|a| matches!(a, Action::CloseDialog)));
        app.actions.clear();
        execute(&mut app, &PaletteRow::Chat("1@example".into()));
        assert!(app.actions.iter().any(|a| matches!(a, Action::CloseDialog)));
    }

    #[test]
    fn jump_to_chat_is_listed_and_opens_the_switcher() {
        let (_root, mut app) = app();
        assert!(
            entries(&app, "jump")
                .iter()
                .any(|e| e.row == PaletteRow::Command(PaletteCommand::JumpToChat))
        );
        execute(&mut app, &PaletteRow::Command(PaletteCommand::JumpToChat));
        assert!(
            app.actions
                .iter()
                .any(|a| matches!(a, Action::ShowDialog(Dialog::JumpToChat)))
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
