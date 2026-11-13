//! The left panel: the chat list.

use egui::{Align, Frame, Key, Layout, Margin, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Chat, ChatFilter, ChatId, Contact, Dialog, Message, Page};
use crate::theme::{self, Icon, Palette};

use super::conversation;
use super::focus::{Stop, TabStop};
use super::labels;
use super::widgets;

// Roomier by default; retain the existing explicit compact-density choice.
pub(super) fn chat_row_height(compact: bool) -> f32 {
    if compact { 56.0 } else { 80.0 }
}

// Keep the two text lines together and centred as density changes. Avatars
// remain 48 points and network badges keep their existing geometry.
fn chat_row_text_top(rect: Rect) -> f32 {
    rect.center().y - 20.0
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    search_keyboard(app, ui);
    let palette = app.palette;
    let panel = egui::Panel::left("chats")
        // The explicit strip below owns resizing. Keep content minimums
        // from widening the panel during a leftward pointer drag.
        .exact_size(app.settings.sidebar_width.clamp(260.0, 520.0))
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.sidebar())
                .inner_margin(Margin::ZERO),
        );
    let response = panel.show(ui, |ui| {
        header(app, ui);
        body(app, ui);
    });
    #[cfg(any(test, feature = "demo"))]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new("chat-panel-right"),
            response.response.rect.right(),
        )
    });
    let width = response.response.rect.width();
    if (width - app.settings.sidebar_width).abs() > 1.0 {
        app.settings.sidebar_width = width;
        app.actions.push(Action::SettingsChanged);
    }
    // Resize strip: egui's own panel resize zone is a few pixels wide and
    // easy to miss, so an explicit wider strip handles hover and drags.
    let rect = response.response.rect;
    let strip = Rect::from_min_max(
        egui::pos2(rect.right() - 2.0, rect.top()),
        egui::pos2(rect.right() + 6.0, rect.bottom()),
    );
    let resize = ui.interact(
        strip,
        egui::Id::new("chats-resize-strip"),
        egui::Sense::click_and_drag(),
    );
    // The strip reaches onto the conversation's side of the edge, where the
    // conversation's own widgets cover it, so a press there never reaches
    // this interact. Follow the raw pointer instead: a press starting
    // anywhere in the strip drags the edge until the button is released,
    // even after the moving edge leaves the press point behind.
    let drag_id = egui::Id::new("chats-resize-strip-active");
    let was_dragging = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(drag_id))
        .unwrap_or(false);
    let strip_drag = ui.ctx().input(|input| {
        input.pointer.primary_down()
            && (was_dragging
                || input
                    .pointer
                    .press_origin()
                    .is_some_and(|origin| strip.contains(origin)))
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(drag_id, strip_drag));
    if resize.hovered() || strip_drag {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if strip_drag && let Some(pos) = ui.ctx().input(|input| input.pointer.latest_pos()) {
        // The pointer rarely grabs the exact edge: a press two points onto
        // the conversation's side must not widen the list by those two
        // points. Anchor the drag where it started and move the edge by
        // the pointer's travel instead.
        let grab_id = egui::Id::new("chats-resize-strip-grab");
        let existing = ui.ctx().data(|data| data.get_temp::<(f32, f32)>(grab_id));
        let grab = existing.unwrap_or((pos.x, rect.width()));
        ui.ctx().data_mut(|data| data.insert_temp(grab_id, grab));
        let dragged = (grab.1 + pos.x - grab.0).clamp(260.0, 520.0);
        if (dragged - app.settings.sidebar_width).abs() > 1.0 {
            app.settings.sidebar_width = dragged;
            app.actions.push(Action::SettingsChanged);
        }
        // The panel sizes itself from its persisted PanelState once that
        // exists, ignoring default_size, so a settings write alone leaves
        // the edge frozen. Drop the persisted state; the next frame takes
        // the dragged width from settings.
        ui.ctx()
            .data_mut(|data| data.remove::<egui::PanelState>(egui::Id::new("chats")));
    }
    // Separate the panel from the conversation, as the header's line does.
    widgets::paint_edge_beside(ui, &palette, response.response.rect);
}

/// The chat list across the whole window, when the window is too narrow to
/// hold it beside a conversation. Opening a chat replaces it.
pub fn full_show(app: &mut App, ui: &mut egui::Ui) {
    search_keyboard(app, ui);
    let palette = app.palette;
    super::rail::show(app, ui, false);
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            header(app, ui);
            body(app, ui);
        });
}

/// What the list pane holds under its header: the chats.
fn body(app: &mut App, ui: &mut egui::Ui) {
    let height = (ui.available_height() - 56.0 - ui.spacing().item_spacing.y).max(0.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_min_height(height);
            ui.set_max_height(height);
            list(app, ui);
        },
    );
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            let button = ui
                .add_sized(
                    vec2(ui.available_width(), 40.0),
                    egui::Button::new(crate::i18n::gettext(app.locale, "New message"))
                        .fill(palette.accent.gamma_multiply(0.18)),
                )
                .tab_stop(Stop::NewChat)
                .on_hover_text(super::keys::label("Ctrl+N"));
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("new-message-footer"), button.rect)
            });
            if button.clicked() {
                app.actions.push(Action::ShowDialog(Dialog::NewChat));
            }
        });
}

/// Walks matching chats while the global search field keeps keyboard focus.
/// Enter leaves search and opens the reached chat ready for typing. A
/// disabled list (beneath a narrow window's sliding chat) leaves the keys
/// to the view coming in.
fn search_keyboard(app: &mut App, ui: &egui::Ui) {
    let field = egui::Id::new("chat-search");
    if !ui.is_enabled()
        || app.search.trim().is_empty()
        || app.locked_folder_open()
        || app.secret_code_matched()
        || !ui.memory(|memory| memory.has_focus(field))
    {
        return;
    }
    let (down, up, enter) = ui.input_mut(|input| {
        (
            super::keys::take_plain(input, Key::ArrowDown),
            super::keys::take_plain(input, Key::ArrowUp),
            super::keys::take_plain(input, Key::Enter),
        )
    });
    if !down && !up && !enter {
        return;
    }
    let chats: Vec<_> = app
        .visible_chats()
        .into_iter()
        .map(|chat| chat.id.clone())
        .collect();
    if chats.is_empty() {
        app.search_selected = None;
        return;
    }
    let before = app.search_selected.clone();
    let mut index = before
        .as_ref()
        .and_then(|selected| chats.iter().position(|chat| chat == selected));
    if down {
        index = Some(index.map_or(0, |index| (index + 1).min(chats.len() - 1)));
    }
    if up {
        index = index.map(|index| index.saturating_sub(1));
    }
    app.search_selected = index.map(|index| chats[index].clone());
    if app.search_selected != before {
        let selected = app.search_selected.clone();
        app.scroll_chat_into_view = selected;
    }
    if enter {
        let chat = app
            .search_selected
            .clone()
            .unwrap_or_else(|| chats[0].clone());
        ui.memory_mut(|memory| memory.surrender_focus(field));
        app.actions.push(Action::Search(String::new()));
        app.actions.push(Action::OpenChat(chat));
    }
}

/// The Bookmarks list's two kinds, as one switch under its header: Starred
/// (WhatsApp stars) and Saved (local, any network). Pinned messages have no
/// list across chats, so they are not here.
fn bookmarks_switch(app: &mut App, ui: &mut egui::Ui) {
    Frame::new()
        .inner_margin(Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                let starred = ui.add_sized(
                    [width, 32.0],
                    egui::Button::selectable(
                        app.show_starred,
                        crate::i18n::gettext(app.locale, "Starred"),
                    ),
                );
                let saved = ui.add_sized(
                    [width, 32.0],
                    egui::Button::selectable(
                        app.show_saved,
                        crate::i18n::gettext(app.locale, "Saved"),
                    ),
                );
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("bookmarks-switch", "starred")), starred.rect);
                    d.insert_temp(egui::Id::new(("bookmarks-switch", "saved")), saved.rect);
                });
                if starred.clicked() && !app.show_starred {
                    app.actions.push(Action::ToggleStarred);
                }
                if saved.clicked() && !app.show_saved {
                    app.actions.push(Action::ToggleSaved);
                }
            });
        });
    ui.add_space(6.0);
}

/// Size of the Bookmarks icon that ends the mode row.
const BOOKMARKS_ICON: f32 = 20.0;

/// Search, title and queues, then the composed scope. Shared by every platform.
fn header(app: &mut App, ui: &mut egui::Ui) {
    if app.show_starred || app.show_saved || app.show_archived || app.locked_folder_open() {
        legacy_header(app, ui);
        if app.show_starred || app.show_saved {
            bookmarks_switch(app, ui);
        }
        super::batch_actions::archive_bar(app, ui);
        return;
    }
    Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            // Keep editable controls below native traffic lights / drawn captions.
            let room = super::caption_room(ui.ctx(), ui.max_rect(), LIST_HEADER_MIN_CONTENT);
            let top = if theme::macos_chrome(ui.ctx()) {
                38.0
            } else {
                room.top.max(
                    (theme::caption_footprint(ui.ctx()).height
                        - (ui.max_rect().top() - ui.ctx().content_rect().top()))
                    .max(0.0),
                )
            };
            if top > 0.0 {
                let (drag, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), top), Sense::hover());
                super::titlebar_drag(ui, drag);
            }
            search_field(app, ui);
            super::batch_actions::archive_bar(app, ui);
            ui.add_space(8.0);
            let row = ui.horizontal(|ui| {
                // Three mode tabs share the row with the Bookmarks icon, which
                // keeps its own square so the labels still fit at 300 points.
                let bookmarks_edge = BOOKMARKS_ICON + theme::ICON_BUTTON_PAD;
                let width =
                    (ui.available_width() - 3.0 * ui.spacing().item_spacing.x - bookmarks_edge)
                        / 3.0;
                for mode in crate::model::ListMode::EVERY {
                    let response = ui.add_sized(
                        [width, 36.0],
                        egui::Button::selectable(app.list_mode == mode, mode.label(app.locale))
                            .truncate(),
                    );
                    let stop = match mode {
                        crate::model::ListMode::Recent => Stop::Recent,
                        crate::model::ListMode::Unread => Stop::UnreadMode,
                        crate::model::ListMode::NeedsMe => Stop::NeedsMe,
                    };
                    let response = response.tab_stop(stop);
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("list-mode", mode.key())), response.rect)
                    });
                    if response.clicked() {
                        app.actions.push(Action::SetListMode(mode));
                    }
                }
                // Bookmarks is the Saved list, not a mode: Cmd+D skips it and
                // the header swaps to the list's own Back arrow while it is open.
                let response = theme::icon_button(
                    ui,
                    Icon::Bookmark,
                    BOOKMARKS_ICON,
                    app.palette.secondary,
                    app.palette.text,
                    "Bookmarks",
                )
                .tab_stop(Stop::Bookmarks);
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("list-mode", "bookmarks")), response.rect)
                });
                if response.clicked() {
                    app.actions.push(Action::ToggleSaved);
                }
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
            ui.add_space(6.0);
            if app.global_search {
                ui.label(
                    egui::RichText::new(crate::i18n::gettext(
                        app.locale,
                        "Search all chats and messages",
                    ))
                    .small(),
                );
            }
            let count = app.outside_scope_matches();
            if count > 0
                && ui
                    .add(
                        egui::Button::new(
                            crate::i18n::ngettext(
                                app.locale,
                                "{} match outside current scope",
                                "{} matches outside current scope",
                                count.min(u32::MAX as usize) as u32,
                            )
                            .replace("{}", &count.to_string()),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Show available matches in all scopes")
                    .clicked()
            {
                app.clear_chat_scope();
            }
        });
}

fn queue_width(available: f32) -> f32 {
    available.clamp(128.0, 168.0)
}

fn legacy_header(app: &mut App, ui: &mut egui::Ui) {
    if theme::macos_chrome(ui.ctx()) {
        macos_header(app, ui);
        return;
    }
    let palette = app.palette;
    // The same margins as the conversation header beside it, and a row as
    // tall as its own, so the two titles share a centre line. The right
    // margin matches the left, so the search field and the chips end where
    // the rows' timestamps do.
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            // Where Solmu draws the title bar, the header's top drags the
            // window and keeps clear of the caption buttons over a corner it
            // reaches: the left one, or both while it fills a narrow window.
            let top = ui.max_rect();
            let room = super::caption_room(ui.ctx(), top, LIST_HEADER_MIN_CONTENT);
            super::caption_drag(
                ui,
                Rect::from_min_size(
                    top.min - vec2(14.0, 8.0),
                    vec2(
                        top.width() + 28.0,
                        8.0 + room.top + super::conversation::HEADER_ROW,
                    ),
                ),
            );
            ui.add_space(room.top);
            let row = ui.allocate_ui_with_layout(
                vec2(ui.available_width(), super::conversation::HEADER_ROW),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add_space(room.left);
                    let buttons = list_header_buttons(app, ui.ctx());
                    // The title measures its room from the buttons and the
                    // caption buttons' room after them.
                    let buttons_width = header_buttons_width(ui, &buttons) + room.right;
                    if app.show_starred {
                        if theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            "Back to chats",
                        )
                        .tab_stop(Stop::Back)
                        .clicked()
                        {
                            app.actions.push(Action::ToggleStarred);
                        }
                        header_title(
                            ui,
                            crate::i18n::gettext(app.locale, "Starred").as_ref(),
                            theme::bold(20.0),
                            palette.text,
                            buttons_width,
                        );
                    } else if app.show_saved {
                        if theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            "Back to chats",
                        )
                        .tab_stop(Stop::Back)
                        .clicked()
                        {
                            app.actions.push(Action::ToggleSaved);
                        }
                        header_title(
                            ui,
                            crate::i18n::gettext(app.locale, "Saved").as_ref(),
                            theme::bold(20.0),
                            palette.text,
                            buttons_width,
                        );
                    } else if app.show_archived || app.locked_folder {
                        if theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            "Back to chats",
                        )
                        .tab_stop(Stop::Back)
                        .clicked()
                        {
                            app.show_archived = false;
                            if app.locked_folder {
                                app.actions.push(Action::CloseLockedFolder);
                            }
                        }
                        header_title(
                            ui,
                            if app.locked_folder {
                                "Locked chats"
                            } else {
                                "Archived"
                            },
                            theme::bold(20.0),
                            palette.text,
                            buttons_width,
                        );
                    } else {
                        // Our avatar and the account switcher are in the
                        // rail beside the list.
                        header_title(
                            ui,
                            &crate::i18n::gettext(app.locale, tab_title(&app.page)),
                            theme::bold(20.0),
                            palette.text,
                            buttons_width,
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(room.right);
                        header_button_row(app, ui, &buttons, false);
                    });
                },
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));

            ui.add_space(6.0);
            search_and_filters(app, ui);
        });
}

/// The header's title beside our avatar: the tab the list shows.
fn tab_title(_page: &Page) -> &'static str {
    "Chats"
}

/// The search field under the header's first row, and the filter chips
/// under it.
fn search_and_filters(app: &mut App, ui: &mut egui::Ui) {
    search_field(app, ui);
    facet_row(app, ui);
}

fn search_field(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let width = ui.available_width();
    let mut text = app.search.clone();
    let response = widgets::search_field(
        ui,
        &palette,
        egui::Id::new("chat-search"),
        &mut text,
        crate::i18n::gettext(app.locale, "Search").as_ref(),
        width,
    )
    .tab_stop(Stop::Search);
    if text != app.search {
        app.actions.push(Action::Search(text));
    }
    // A disabled field cannot take the keyboard; the request waits.
    if app.focus_search && ui.is_enabled() {
        app.focus_search = false;
        response.request_focus();
    }
}

/// Four permanent queue cells, independent of the additional scope.
fn facet_row(app: &mut App, ui: &mut egui::Ui) {
    if app.show_starred || app.show_saved {
        return;
    }
    let palette = app.palette;
    let width = queue_width(ui.available_width());
    let cell = width / 4.0;
    ui.spacing_mut().item_spacing.x = 0.0;
    // Off avoids even the count passes; no sorting/history work for badges.
    let counts = app
        .settings
        .beeper
        .show_queue_counts
        .then(|| app.queue_chat_counts());
    ui.horizontal(|ui| {
        for (index, (filter, icon, stop)) in [
            (ChatFilter::All, Icon::QueueAll, Stop::All),
            (ChatFilter::Unread, Icon::QueueUnread, Stop::Unread),
            (ChatFilter::ReplyLater, Icon::QueueReply, Stop::ReplyLater),
            (ChatFilter::ReadLater, Icon::QueueRead, Stop::ReadLater),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = app.chat_filter == filter;
            let shortcut = ChatFilter::EVERY.iter().position(|f| *f == filter).unwrap() + 1;
            let mut tooltip = format!(
                "{} · {}",
                filter.label(app.locale),
                super::keys::label(&format!("Ctrl+Alt+{shortcut}"))
            );
            if let Some(counts) = counts {
                tooltip.push_str(&format!(
                    " · {} {}",
                    counts[index],
                    crate::i18n::gettext(app.locale, "chats")
                ));
            }
            let (rect, response) = ui.allocate_exact_size(vec2(cell, 36.0), Sense::click());
            let response = response.tab_stop(stop).on_hover_text(&tooltip);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    selected,
                    &tooltip,
                )
            });
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, palette.outline),
                egui::StrokeKind::Inside,
            );
            if selected || response.hovered() {
                ui.painter().rect_filled(
                    rect.shrink(1.0),
                    2.0,
                    palette
                        .accent
                        .gamma_multiply(if selected { 0.22 } else { 0.10 }),
                );
            }
            let badge_count = queue_badge_count(filter, counts.map(|counts| counts[index]));
            theme::focus_outline(ui, response.id, rect, 3.0);
            theme::paint_icon(
                ui,
                icon,
                if badge_count.is_some() {
                    rect.translate(vec2(-3.0, 2.0))
                } else {
                    rect
                },
                if badge_count.is_some() { 19.0 } else { 22.0 },
                if selected {
                    palette.accent
                } else {
                    palette.secondary
                },
            );
            if let Some(count) = badge_count {
                paint_queue_count(ui, &palette, rect, count);
            }
            ui.ctx()
                .data_mut(|data| data.insert_temp(filter_chip_id(filter), rect));
            if response.clicked() {
                app.actions.push(Action::SetChatFilter(filter));
            }
        }
    });
}

/// All stays icon-only; counts remain available in its tooltip.
fn queue_badge_count(filter: ChatFilter, count: Option<usize>) -> Option<usize> {
    count.filter(|count| filter != ChatFilter::All && *count > 0)
}

/// Small count badge contained by its segment, with the full count in its tooltip.
fn paint_queue_count(ui: &egui::Ui, palette: &Palette, cell: Rect, count: usize) {
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, theme::semibold(8.5), palette.on_accent);
    let width = (galley.size().x + 6.0).max(12.0);
    let badge = Rect::from_min_size(
        pos2(cell.right() - width - 2.0, cell.top() + 2.0),
        vec2(width, 12.0),
    );
    ui.painter().rect_filled(badge, 6.0, palette.accent);
    ui.painter().galley(
        badge.center() - galley.size() / 2.0,
        galley,
        palette.on_accent,
    );
}

/// A button on the right of the chat list header's first row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderButton {
    HideList,
}

/// The size of the header buttons' icons.
const HEADER_ICON: f32 = 18.0;

/// What the list header's first row needs beside the caption buttons
/// Solmu draws: the title and its buttons.
const LIST_HEADER_MIN_CONTENT: f32 = 230.0;

/// The buttons on the right of the chat list header, rightmost first, as
/// `header_button_row` draws them: New chat, and Hide the chat list unless
/// the window is narrow. Starred and Settings are in the rail beside
/// the list. The title before them measures its room from the same list.
fn header_buttons(ctx: &egui::Context) -> Vec<HeaderButton> {
    let mut buttons = Vec::new();
    if !super::narrow(ctx) {
        buttons.push(HeaderButton::HideList);
    }
    buttons
}

/// The buttons the list header draws for what it shows: none over the
/// starred messages, whose Back leads out of them.
fn list_header_buttons(app: &App, ctx: &egui::Context) -> Vec<HeaderButton> {
    if app.show_starred {
        return Vec::new();
    }
    header_buttons(ctx)
}

/// Draws `buttons` in a right-to-left layout, with the macOS shortcuts in
/// their tooltips when `macos`.
fn header_button_row(app: &mut App, ui: &mut egui::Ui, buttons: &[HeaderButton], macos: bool) {
    let palette = app.palette;
    for button in buttons {
        match button {
            HeaderButton::HideList => {
                if theme::icon_button(
                    ui,
                    Icon::PanelLeft,
                    HEADER_ICON,
                    palette.secondary,
                    palette.text,
                    if macos {
                        "Hide the chat list (⌘B)"
                    } else {
                        "Hide the chat list (Ctrl+B)"
                    },
                )
                .tab_stop(Stop::Sidebar)
                .clicked()
                {
                    app.actions.push(Action::ToggleSidebar);
                }
            }
        }
    }
}

/// How wide `buttons` are drawn, with the spacing between them.
fn header_buttons_width(ui: &egui::Ui, buttons: &[HeaderButton]) -> f32 {
    row_width(
        buttons.len(),
        HEADER_ICON + theme::ICON_BUTTON_PAD,
        ui.spacing().item_spacing.x,
    )
}

/// How wide `count` items `edge` points wide are with `spacing` between them.
fn row_width(count: usize, edge: f32, spacing: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    count as f32 * edge + (count - 1) as f32 * spacing
}

/// Draws the chat list header's title when it fits before the buttons that
/// follow it on the right, `buttons` points wide, and leaves it out
/// otherwise: in a narrow list it ran under them. The avatar or Back before
/// it says where you are.
fn header_title(
    ui: &mut egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    buttons: f32,
) {
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x;
    let spacing = ui.spacing().item_spacing.x;
    if title_fits(width, ui.available_width(), buttons, spacing) {
        theme::text(ui, text, font, color);
    }
}

/// Whether a title `width` points wide fits in `available` beside a row of
/// buttons `buttons` points wide, with `spacing` between the two.
fn title_fits(width: f32, available: f32, buttons: f32, spacing: f32) -> bool {
    width + spacing + buttons <= available
}

/// Where the chat list header's first row was laid out, for layout tests.
pub(crate) fn header_row_id() -> egui::Id {
    egui::Id::new("chat-list-header-row")
}

fn macos_header(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // The traffic lights sit over the rail and the start of this header.
    let mut drag = ui.max_rect();
    drag.max.y = drag.min.y + 60.0;
    super::titlebar_drag(ui, drag);
    Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            let row = ui.horizontal(|ui| {
                ui.set_min_height(super::conversation::HEADER_ROW);
                // The traffic lights reach past the rail over this header.
                ui.add_space(super::rail::lights_overhang(ui.ctx()));
                let buttons = list_header_buttons(app, ui.ctx());
                if app.show_starred {
                    if theme::icon_button(
                        ui,
                        Icon::ArrowLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Back to chats",
                    )
                    .tab_stop(Stop::Back)
                    .clicked()
                    {
                        app.actions.push(Action::ToggleStarred);
                    }
                    header_title(
                        ui,
                        crate::i18n::gettext(app.locale, "Starred").as_ref(),
                        theme::bold(16.0),
                        palette.text,
                        header_buttons_width(ui, &buttons),
                    );
                } else if app.show_saved {
                    if theme::icon_button(
                        ui,
                        Icon::ArrowLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Back to chats",
                    )
                    .tab_stop(Stop::Back)
                    .clicked()
                    {
                        app.actions.push(Action::ToggleSaved);
                    }
                    header_title(
                        ui,
                        crate::i18n::gettext(app.locale, "Saved").as_ref(),
                        theme::bold(16.0),
                        palette.text,
                        header_buttons_width(ui, &buttons),
                    );
                } else if app.show_archived || app.locked_folder {
                    if theme::icon_button(
                        ui,
                        Icon::ArrowLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Back to chats",
                    )
                    .tab_stop(Stop::Back)
                    .clicked()
                    {
                        app.show_archived = false;
                        if app.locked_folder {
                            app.actions.push(Action::CloseLockedFolder);
                        }
                    }
                    header_title(
                        ui,
                        if app.locked_folder {
                            "Locked chats"
                        } else {
                            "Archived"
                        },
                        theme::bold(16.0),
                        palette.text,
                        header_buttons_width(ui, &buttons),
                    );
                } else {
                    // Our avatar and the account switcher are in the rail,
                    // under the traffic lights.
                    header_title(
                        ui,
                        &crate::i18n::gettext(app.locale, tab_title(&app.page)),
                        theme::bold(20.0),
                        palette.text,
                        header_buttons_width(ui, &buttons),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    header_button_row(app, ui, &buttons, true);
                });
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
            ui.add_space(6.0);
            search_and_filters(app, ui);
        });
}

/// Small network icons on a chat's avatar: the chat's own network, or every
/// distinct network of a linked person's chats. The unread, mute and
/// disappearing marks keep their places; the first badge shifts left of the
/// disappearing-timer mark when one is drawn.
fn network_badges(app: &App, ui: &egui::Ui, palette: &Palette, avatar: Rect, chat: &Chat) {
    let members = app
        .settings
        .beeper
        .people
        .iter()
        .find(|p| p.members.iter().any(|m| m.matches(chat)))
        .map(|p| p.members.clone());
    let mut networks: Vec<String> = Vec::new();
    let mut note = |c: &Chat| {
        if let Some(n) = c.backend_network.as_deref().or(c.network.as_deref())
            && !networks.iter().any(|known| known == n)
        {
            networks.push(n.to_owned());
        }
    };
    match members {
        Some(members) => {
            for member in &members {
                if let Some(c) = app.chat(&member.chat) {
                    note(c);
                }
            }
        }
        None if chat.backend_account.is_some() => note(chat),
        None => {}
    }
    let first = usize::from(chat.disappearing_timer().is_some());
    for (slot, network) in networks.iter().take(2).enumerate() {
        widgets::paint_network_badge(ui, palette, avatar, network, first + slot);
    }
}

/// Stable main-list row id used by interaction tests.
pub fn chat_row_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-row", chat))
}

/// Stable filter-chip id used by interaction tests.
pub fn filter_chip_id(filter: ChatFilter) -> egui::Id {
    egui::Id::new(("chat-filter", filter as u8))
}

/// One filter as a compact icon button with its unread count in a corner
/// badge. The filter's name and Option-digit shortcut sit on the tooltip
/// and the accessibility label, so the whole filter row stays one line
/// high. Selected keeps the accent wash and tint of the old full rows.
fn filter_icon(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    count: usize,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(16.0 + theme::ICON_BUTTON_PAD), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, 6.0);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter()
                .rect_filled(rect.shrink(1.0), 6.0, palette.accent.gamma_multiply(0.18));
        } else if response.hovered() {
            let hover = if palette.vibrant() {
                palette.tint(theme::TINT_HOVER)
            } else {
                palette.surface
            };
            ui.painter().rect_filled(rect.shrink(1.0), 6.0, hover);
        }
        let tint = if selected {
            palette.accent
        } else if response.hovered() || response.has_focus() {
            palette.text
        } else {
            palette.secondary
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        theme::paint_icon(ui, icon, rect, 16.0 * scale, tint);
        if count > 0 {
            // Mini unread badge on the icon's top-right corner, a scaled
            // down widgets::badge.
            let label = if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            };
            let galley =
                ui.painter()
                    .layout_no_wrap(label, theme::semibold(8.5), palette.on_accent);
            let width = (galley.size().x + 8.0).max(14.0);
            let badge = Rect::from_center_size(
                pos2(rect.right() - 3.0, rect.top() + 3.0),
                vec2(width, 14.0),
            );
            ui.painter().rect_filled(badge, 7.0, palette.accent);
            ui.painter().galley(
                badge.center() - galley.size() / 2.0,
                galley,
                palette.on_accent,
            );
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.on_hover_text(tooltip)
}

/// The rail's quick filters: All, Unread and the later facets are the
/// labeled row above the chat list, so the rail keeps the rest. Search
/// lists every match, so the chips hide there.
pub(super) fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    if app.show_starred || app.show_saved {
        // The starred and saved panels replace the chat list, so its
        // filters go with it.
        return;
    }
    if !app.locked_folder_open() && !app.search.trim().is_empty() {
        return;
    }
    let palette = app.palette;
    ui.add_space(8.0);
    widgets::chip_scroll_row(ui, "chat-filters", |ui| {
        // One icon per row down the rail's single column: the icon carries
        // the filter's unread badge, its name and shortcut the tooltip and
        // accessibility label.
        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
        ui.vertical_centered(|ui| {
            // All, Unread and the later facets moved to the labeled row
            // above the chat list; the rail keeps the rest.
            for filter in [
                ChatFilter::Private,
                ChatFilter::Favorites,
                ChatFilter::Groups,
                ChatFilter::Channels,
            ] {
                let count = match filter {
                    ChatFilter::All => 0,
                    _ => app.unread_chats(filter),
                };
                let selected = !app.locked_folder_open()
                    && !app.show_archived
                    && match filter {
                        ChatFilter::Favorites => app.favorites_only,
                        _ => app.category_filter == Some(filter),
                    };
                let icon = match filter {
                    ChatFilter::All => Icon::MessageCircle,
                    ChatFilter::Unread => Icon::Bell,
                    ChatFilter::ReplyLater => Icon::Clock,
                    ChatFilter::ReadLater => Icon::ListChecks,
                    ChatFilter::Private => Icon::User,
                    ChatFilter::Favorites => Icon::Star,
                    ChatFilter::Groups => Icon::Users,
                    ChatFilter::Channels => Icon::Volume2,
                };
                let name = filter.label(app.locale);
                let tooltip = match ChatFilter::EVERY.iter().position(|f| f == &filter) {
                    Some(position) => format!(
                        "{name} ({})",
                        super::keys::label(&format!("Ctrl+Alt+{}", position + 1))
                    ),
                    None => name.to_string(),
                };
                let row = filter_icon(ui, &palette, icon, count, selected, &tooltip).tab_stop(
                    match filter {
                        ChatFilter::All => Stop::All,
                        ChatFilter::Unread => Stop::Unread,
                        ChatFilter::ReplyLater => Stop::ReplyLater,
                        ChatFilter::ReadLater => Stop::ReadLater,
                        ChatFilter::Private => Stop::Private,
                        ChatFilter::Favorites => Stop::Favorites,
                        ChatFilter::Groups => Stop::Groups,
                        ChatFilter::Channels => Stop::Channels,
                    },
                );
                // Store the button rect for interaction tests.
                ui.ctx()
                    .data_mut(|data| data.insert_temp(filter_chip_id(filter), row.rect));
                if filter == ChatFilter::Channels {
                    let now = crate::util::now();
                    let all_muted = app
                        .chats
                        .iter()
                        .filter(|chat| chat.is_channel())
                        .all(|chat| chat.muted(now));
                    row.context_menu(|ui| {
                        let (icon, label) = if all_muted {
                            (Icon::Bell, "Unmute all channels")
                        } else {
                            (Icon::BellOff, "Mute all channels")
                        };
                        if widgets::menu_item(ui, &palette, Some(icon), label) {
                            app.actions.push(Action::MuteAllChannels(!all_muted));
                            ui.close();
                        }
                    });
                }
                if row.clicked() {
                    // A second click on the active filter returns to every chat.
                    app.actions.push(Action::SetChatFilter(filter));
                }
            }
        });
        // Networks live beside the rail's bottom destinations; the
        // follow-up queues are icons in the same column.
        if !app.beeper_browse_networks().is_empty() {
            ui.add_space(6.0);
            let unreplied = filter_icon(
                ui,
                &palette,
                Icon::Reply,
                0,
                app.beeper_unreplied,
                "Unreplied\nLatest known message is from someone else. Unknown/system previews excluded. Cmd+Option+9",
            );
            if unreplied.clicked() {
                app.beeper_unreplied = !app.beeper_unreplied;
                app.actions.push(Action::SetChatFilter(ChatFilter::All));
            }
        }
        ui.add_space(4.0);
    });
    labels::chip_row(app, ui, &palette);
}

/// Stable id for a starred row, so its menu can be addressed by the demo,
/// by tests, and by the row that keeps it open.
pub(crate) fn list_row_id(chat: &str, id: &str) -> egui::Id {
    egui::Id::new(("list-row", chat, id))
}

/// The starred messages, in the place the chat list usually takes. Newest
/// star first, each row naming the chat, the moment, and the message.
fn starred_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if app.starred.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Star,
            crate::i18n::gettext(app.locale, "No starred messages").as_ref(),
            crate::i18n::gettext(
                app.locale,
                "Right-click a message and choose Star to keep it here.",
            )
            .as_ref(),
        );
        return;
    }
    // Rows read the list from the app while they hand their actions back, so
    // the actions are collected here and applied once the list is drawn.
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in &app.starred {
                if app
                    .chat(&entry.message.chat)
                    .is_some_and(|chat| chat.locked)
                {
                    continue;
                }
                starred_row(
                    app,
                    ui,
                    &palette,
                    &entry.message,
                    entry.starred_at,
                    conversation::Marks { starred: true },
                    &mut actions,
                );
            }
        });
    app.actions.append(&mut actions);
}

/// One starred message: where it is, when it was starred, and the message's own
/// words inside a bubble drawn from its side and fill. Clicking opens its chat
/// at the message; right-clicking the bubble opens the chat's own menu.
fn starred_row(
    app: &App,
    ui: &mut egui::Ui,
    palette: &Palette,
    message: &Message,
    marked_at: i64,
    marks: conversation::Marks,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let clock = widgets::line(
        ui,
        &crate::util::clock(message.timestamp),
        theme::regular(11.0),
        palette.secondary,
        80.0,
        1,
    );
    let body_width = (width - 32.0 - 24.0 - clock.size().x - 10.0).max(60.0);
    // The message's own words with WhatsApp's markup taken off and mentions
    // named, the way the chat list previews one. Color emoji and wrapping go
    // through the same line helper as chat text.
    let summary = message.content.full_summary_in(app.locale);
    let words = crate::markup::plain(&app.resolve_mention_tokens(&summary), &[]);
    let body = widgets::line(
        ui,
        &words,
        theme::regular(13.0),
        palette.text,
        body_width,
        12,
    );
    let bubble_height = body.size().y + 12.0;
    let row_id = list_row_id(&message.chat, &message.id);
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, 26.0 + bubble_height + 10.0), Sense::hover());
    let response = ui.interact(rect, row_id, Sense::click());
    theme::reveal_focus(&response);
    // The chat's own title, as the chat list writes it: a group without a
    // subject is named after its members, never "Unknown".
    let name = app.chat_title_of(&message.chat);
    let label = format!("{name}, {words}");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, palette.surface_hover.gamma_multiply(0.5));
    }
    let when = crate::util::moment_stamp(app.locale, marked_at);
    let name_line = widgets::line(
        ui,
        &name,
        theme::medium(14.0),
        palette.text,
        width - 140.0,
        1,
    );
    name_line.paint(
        ui,
        pos2(
            rect.left() + 16.0,
            rect.top() + 14.0 - name_line.size().y / 2.0,
        ),
        palette.text,
    );
    ui.painter().text(
        pos2(rect.right() - 16.0, rect.top() + 14.0),
        egui::Align2::RIGHT_CENTER,
        when,
        theme::regular(11.5),
        palette.secondary,
    );
    // The message itself, as the bubble the chat shows: same fill, same side,
    // with the time in its bottom corner.
    let bubble_width = (body.size().x + 20.0 + clock.size().x + 6.0).min(width - 32.0);
    let left = if message.from_me {
        rect.right() - 16.0 - bubble_width
    } else {
        rect.left() + 16.0
    };
    let bubble = Rect::from_min_size(
        pos2(left, rect.top() + 26.0),
        vec2(bubble_width, bubble_height),
    );
    ui.painter().rect_filled(
        bubble,
        10.0,
        if message.from_me {
            palette.bubble_out
        } else {
            palette.bubble_in
        },
    );
    body.paint(
        ui,
        pos2(bubble.left() + 10.0, bubble.top() + 6.0),
        palette.text,
    );
    clock.paint(
        ui,
        pos2(
            bubble.right() - 10.0 - clock.size().x,
            bubble.bottom() - 6.0 - clock.size().y,
        ),
        palette.secondary,
    );
    // The bubble's rect is what the menu is anchored to, and what a test reads
    // to click it.
    ui.ctx()
        .data_mut(|data| data.insert_temp(row_id.with("bubble"), bubble));
    // Right-click is read from the input because the row's own response cannot
    // tell the bubble from the name line above it. The menu opens only over the
    // bubble, and only when no floating layer covers the chat panel.
    let right_clicked = ui.input(|input| {
        input.pointer.secondary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| bubble.contains(pos))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer == response.layer_id)
        });
    #[cfg(any(test, feature = "demo"))]
    let force_menu = app
        .open_list_menu
        .as_ref()
        .is_some_and(|(chat, id)| chat == &message.chat && id == &message.id);
    #[cfg(not(any(test, feature = "demo")))]
    let force_menu = false;
    let open = if right_clicked || force_menu {
        Some(egui::SetOpenCommand::Bool(true))
    } else if response.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let popup = egui::Popup::menu(&response)
        .open_memory(open)
        .width(conversation::menu_width(
            ui,
            app.locale,
            &app.settings.reaction_emoji,
            message,
        ))
        .frame(widgets::menu_frame(palette));
    let popup = if force_menu {
        popup.at_position(bubble.left_top() + vec2(12.0, 8.0))
    } else {
        popup.at_pointer_fixed()
    };
    popup.show(|ui| {
        let chat = app
            .chat(&message.chat)
            .cloned()
            .unwrap_or_else(|| Chat::new(message.chat.clone(), name.clone()));
        conversation::list_menu(app, ui, &chat, message, marks, actions);
    });
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        actions.push(Action::OpenMessage {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
}

/// The locally saved messages, newest bookmark first, filtered by the
/// panel's own search over text, sender and chat. Clicking an entry opens
/// its chat at the message; removing a bookmark never deletes the message,
/// and saving never marked the chat read.
fn saved_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let width = ui.available_width();
    let mut text = app.saved_search.clone();
    widgets::search_field(
        ui,
        &palette,
        egui::Id::new("saved-search"),
        &mut text,
        crate::i18n::gettext(app.locale, "Search saved messages").as_ref(),
        width,
    );
    if text != app.saved_search {
        app.saved_search = text;
    }
    ui.add_space(4.0);
    if app.settings.beeper.saved.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Bookmark,
            crate::i18n::gettext(app.locale, "No saved messages").as_ref(),
            crate::i18n::gettext(
                app.locale,
                "Right-click a message and choose Save for later to keep it here.",
            )
            .as_ref(),
        );
        return;
    }
    // Rows read the list from the app while they hand their actions back, so
    // the actions are collected here and applied once the list is drawn.
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in crate::saved::filtered(&app.settings.beeper.saved, &app.saved_search) {
                if app.chat(&entry.member.chat).is_some_and(|chat| chat.locked) {
                    continue;
                }
                saved_row(app, ui, &palette, entry, &mut actions);
            }
        });
    app.actions.append(&mut actions);
}

/// One saved message: where it is, its date and the words it had when it
/// was saved. Clicking opens the chat at the message while the original is
/// still known; once it is gone the row says so instead of jumping.
fn saved_row(
    app: &App,
    ui: &mut egui::Ui,
    palette: &Palette,
    entry: &crate::saved::SavedMessage,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let clock = widgets::line(
        ui,
        &crate::util::clock(entry.timestamp),
        theme::regular(11.0),
        palette.secondary,
        80.0,
        1,
    );
    let body_width = (width - 32.0 - clock.size().x - 10.0).max(60.0);
    let body = widgets::line(
        ui,
        &entry.preview,
        theme::regular(13.0),
        palette.text,
        body_width,
        12,
    );
    let bubble_height = body.size().y + 12.0;
    let row_id = list_row_id(&entry.member.chat, &entry.message);
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, 26.0 + bubble_height + 10.0), Sense::hover());
    let response = ui.interact(rect, row_id, Sense::click());
    theme::reveal_focus(&response);
    // The jump works only while the chat is still there and the message is
    // known locally; a fully loaded chat without it means it is gone.
    let chat = app.chat(&entry.member.chat);
    let available = chat.is_some()
        && app
            .conversations
            .get(&entry.member.chat)
            .is_none_or(|conversation| {
                !conversation.complete
                    || conversation
                        .messages
                        .iter()
                        .any(|message| message.id == entry.message)
            });
    let name = chat
        .map(|chat| app.chat_title_of(&chat.id))
        .unwrap_or_else(|| entry.chat_title.clone());
    let label = if available {
        format!("{name}, {}", entry.preview)
    } else {
        format!(
            "{name}, {}",
            crate::i18n::gettext(app.locale, "Original message unavailable")
        )
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, palette.surface_hover.gamma_multiply(0.5));
    }
    let when = crate::util::moment_stamp(app.locale, entry.timestamp);
    let name_line = widgets::line(
        ui,
        &name,
        theme::medium(14.0),
        palette.text,
        width - 140.0,
        1,
    );
    name_line.paint(
        ui,
        pos2(
            rect.left() + 16.0,
            rect.top() + 14.0 - name_line.size().y / 2.0,
        ),
        palette.text,
    );
    ui.painter().text(
        pos2(rect.right() - 16.0, rect.top() + 14.0),
        egui::Align2::RIGHT_CENTER,
        when,
        theme::regular(11.5),
        palette.secondary,
    );
    let bubble_width = (body.size().x + 20.0 + clock.size().x + 6.0).min(width - 32.0);
    let left = if entry.from_me {
        rect.right() - 16.0 - bubble_width
    } else {
        rect.left() + 16.0
    };
    let bubble = Rect::from_min_size(
        pos2(left, rect.top() + 26.0),
        vec2(bubble_width, bubble_height),
    );
    ui.painter().rect_filled(
        bubble,
        10.0,
        if entry.from_me {
            palette.bubble_out
        } else {
            palette.bubble_in
        },
    );
    body.paint(
        ui,
        pos2(bubble.left() + 10.0, bubble.top() + 6.0),
        palette.text,
    );
    clock.paint(
        ui,
        pos2(
            bubble.right() - 10.0 - clock.size().x,
            bubble.bottom() - 6.0 - clock.size().y,
        ),
        palette.secondary,
    );
    if !available {
        ui.painter().text(
            pos2(rect.left() + 16.0, bubble.bottom() + 4.0),
            egui::Align2::LEFT_CENTER,
            crate::i18n::gettext(app.locale, "Original message unavailable"),
            theme::regular(11.0),
            palette.dim,
        );
    }
    response.context_menu(|ui| {
        if ui
            .button(crate::i18n::gettext(app.locale, "Remove from Saved"))
            .clicked()
        {
            actions.push(Action::SetSaved {
                chat: entry.member.chat.clone(),
                message: entry.message.clone(),
                saved: false,
            });
            ui.close();
        }
    });
    if available
        && response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        actions.push(Action::OpenMessage {
            chat: entry.member.chat.clone(),
            message: entry.message.clone(),
        });
    }
}

/// Side of a favorite chip: the avatar, its badges and a little air.
const FAVORITE_CELL: f32 = 44.0;
/// The avatar inside a favorite chip.
const FAVORITE_AVATAR: f32 = 36.0;

/// Favorite chats span network/account navigation exactly like the compact
/// pins: the phone's favorites list, as direct jumps from the sidebar.
/// Each favorite is its avatar and unread count only, never a message
/// preview, so the bar stays a strip of quick jump targets rather than a
/// second chat list. Favorite toggles, context menus and unread badges
/// behave exactly as in the list.
/// The favorites bar's chats in bar order: favorite position, then recent
/// activity, then id. Command+1..9 jumps by this same order, so the bar and
/// the keys never disagree about which chat a number means.
pub(super) fn favorite_bar_chats(app: &App) -> Vec<Chat> {
    let mut favorites: Vec<Chat> = app
        .chats
        .iter()
        .filter(|c| {
            c.backend_account.is_some() && c.favorite && !c.is_channel() && !c.locked && !c.archived
        })
        .cloned()
        .collect();
    favorites.sort_by(|a, b| {
        a.favorite_position
            .cmp(&b.favorite_position)
            .then_with(|| b.last_activity.cmp(&a.last_activity))
            .then(a.id.cmp(&b.id))
    });
    favorites
        .into_iter()
        .map(|chat| app.beeper_person_preview(&chat))
        .collect()
}

pub(super) fn favorites_row(app: &mut App, ui: &mut egui::Ui) {
    let favorites = favorite_bar_chats(app);
    if favorites.is_empty() {
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    // Reorder drag state: the dragged chip and the gap the pointer is over,
    // both kept across frames until the drop clears them.
    let drag_id = ui.id().with("favorite-reorder-drag");
    let gap_id = ui.id().with("favorite-reorder-gap");
    let mut dragged: Option<ChatId> = ui.ctx().data(|data| data.get_temp(drag_id));
    let mut drop: Option<(ChatId, usize)> = None;
    let mut rects: Vec<Rect> = Vec::with_capacity(favorites.len());
    // One avatar per row down the rail's column: a per-chip scope stopped
    // the wrap and the last chips overflowed the rail; a capping ScrollArea
    // clipped the second row mid-avatar. The rail's own scroll keeps every
    // favorite reachable instead of a section cap.
    widgets::chip_scroll_row(ui, "beeper-global-favorites", |ui| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (index, chat) in favorites.iter().enumerate() {
                let shortcut =
                    (index < 9).then(|| super::keys::label(&format!("Ctrl+{}", index + 1)));
                let response = favorite_chip(app, ui, chat, shortcut);
                rects.push(response.rect);
                if response.drag_started() {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(drag_id, chat.id.clone()));
                    dragged = Some(chat.id.clone());
                }
                if response.drag_stopped() && dragged.as_ref() == Some(&chat.id) {
                    let gap = ui
                        .ctx()
                        .data(|data| data.get_temp::<usize>(gap_id))
                        .unwrap_or(index);
                    drop = Some((chat.id.clone(), gap));
                }
            }
            // While a chip is dragged the gap follows the pointer across the
            // column, and an accent line marks the place a drop inserts at;
            // the line stands in for a dragged ghost of the chip.
            if dragged.is_some() {
                if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                    let gap = rects
                        .iter()
                        .position(|rect| pointer.y < rect.center().y)
                        .unwrap_or(rects.len());
                    ui.ctx().data_mut(|data| data.insert_temp(gap_id, gap));
                    let y = if gap == 0 {
                        rects.first().map(|rect| rect.top() - 2.0)
                    } else if gap >= rects.len() {
                        rects.last().map(|rect| rect.bottom() + 2.0)
                    } else {
                        Some((rects[gap - 1].bottom() + rects[gap].top()) / 2.0)
                    };
                    if let (Some(y), Some(first)) = (y, rects.first()) {
                        ui.painter().hline(
                            first.left() + 4.0..=first.right() - 4.0,
                            y,
                            Stroke::new(2.0, app.palette.accent),
                        );
                    }
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        });
    });
    if let Some((chat, gap)) = drop {
        ui.ctx().data_mut(|data| {
            data.remove::<ChatId>(drag_id);
            data.remove::<usize>(gap_id);
        });
        // The gap counts the chip in its old place: a drop past it moves the
        // chip one place less far.
        if let Some(from) = favorites.iter().position(|known| known.id == chat) {
            let target = if gap > from { gap - 1 } else { gap };
            if target != from {
                app.actions.push(Action::MoveFavorite(chat, target));
            }
        }
    }
    ui.add_space(4.0);
}

/// One favorite in the sidebar's bar: the chat's avatar with its unread
/// count, opening the chat on click. The hover text names the chat and the
/// right-click menu is the chat row's, so nothing the full row offered is
/// lost with the preview.
fn favorite_chip(
    app: &mut App,
    ui: &mut egui::Ui,
    chat: &Chat,
    shortcut: Option<String>,
) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    let selected = app.open_chat.as_deref() == Some(chat.id.as_str());
    // allocate_space joins the row's wrapping layout; the interact id (not
    // a scope) keeps the chip's widget state and context menu stable.
    let (_auto, rect) = ui.allocate_space(Vec2::splat(FAVORITE_CELL));
    // click_and_drag: a press still opens the chat, a drag reorders the bar.
    let response = ui.interact(
        rect,
        ui.id().with(("favorite-chip", &chat.id)),
        Sense::click_and_drag(),
    );
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            unread_announcement(&title, chat),
        )
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                Rect::from_center_size(rect.center(), Vec2::splat(FAVORITE_CELL - 4.0)),
                egui::CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        let avatar_rect = Rect::from_center_size(rect.center(), Vec2::splat(FAVORITE_AVATAR));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &chat.id,
            picture.as_deref(),
        );
        if selected {
            paint_active_avatar_ring(ui, &palette, avatar_rect);
        }
        if chat.disappearing_timer().is_some() {
            widgets::paint_disappearing_badge(ui, &palette, avatar_rect);
        }
        network_badges(app, ui, &palette, avatar_rect, chat);
        if chat.looks_unread() {
            // The badge respects the same settings as the chat list: a dot
            // only when counts are off, dimmed in a muted chat.
            let badge_width = widgets::unread_indicator(
                ui,
                &palette,
                compact_badge_center(avatar_rect),
                if app.settings.beeper.show_unread_counts {
                    chat.unread
                } else {
                    0
                },
                chat.marked_unread || !app.settings.beeper.show_unread_counts,
                chat.muted(crate::util::now()),
            );
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    favorite_badge_id(&chat.id),
                    Rect::from_center_size(
                        compact_badge_center(avatar_rect),
                        Vec2::splat(badge_width),
                    ),
                )
            });
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(favorite_chip_id(&chat.id), response.rect));
    // With no name or preview on screen, the hover names the chat (and its
    // network when the backend supplied one) and its number shortcut.
    let mut hover = match &chat.network {
        Some(network) => format!("{}\n{}", title, network),
        None => title,
    };
    if let Some(shortcut) = shortcut {
        hover.push_str(&format!("\n{shortcut}"));
    }
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hover);
    if response.clicked() {
        if ui.input(|input| input.modifiers.command) {
            super::batch_actions::toggle_chat(app, &chat.id);
        } else {
            app.actions.push(Action::OpenChat(chat.id.clone()));
        }
    }
    let menu_palette = palette;
    // The favorite item is translated, so its width counts in the reader's
    // language.
    let favorite_label = if chat.favorite {
        crate::i18n::gettext(app.locale, "Remove from favorites")
    } else {
        crate::i18n::gettext(app.locale, "Add to favorites")
    };
    let menu_width = widgets::menu_width(
        ui,
        &[
            "Mark as read",
            "Mark as unread",
            "Pin to top",
            favorite_label.as_ref(),
            "Unarchive",
            &crate::i18n::gettext(app.locale, "Leave group"),
            &crate::i18n::gettext(app.locale, "Leave channel"),
            "Unlock chat",
            "Copy number",
            &crate::i18n::gettext(app.locale, "Clear chat"),
            &crate::i18n::gettext(app.locale, "Delete chat"),
        ],
        true,
    )
    // Submenu rows also carry a chevron.
    .max(
        widgets::menu_width(
            ui,
            &[
                &crate::i18n::gettext(app.locale, "Mute"),
                "Notification sound",
                "Voice message speed",
                &crate::i18n::gettext(app.locale, "Labels"),
            ],
            true,
        ) + 24.0,
    )
    .max(190.0);
    let popup = egui::Popup::context_menu(&response)
        .width(menu_width)
        .frame(widgets::menu_frame(&menu_palette));
    #[cfg(any(test, feature = "demo"))]
    let popup = if app.open_chat_menu.as_deref() == Some(chat.id.as_str()) {
        popup
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .at_position(response.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup
    };
    popup.show(|ui| context_menu(app, ui, chat, &menu_palette));
    response
}

/// Section headers are virtual rows too: keyboard reveal and wheel scrolling
/// use the same row indices as painting, without invisible pixel offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChatListRow {
    Header(bool),
    Chat(usize),
}

fn sectioned_rows(favorites: &[bool]) -> Vec<ChatListRow> {
    let split = favorites.iter().take_while(|favorite| **favorite).count();
    let grouped = split > 0 && split < favorites.len();
    let mut rows = Vec::with_capacity(favorites.len() + if grouped { 2 } else { 0 });
    for index in 0..favorites.len() {
        if grouped && (index == 0 || index == split) {
            rows.push(ChatListRow::Header(index == 0));
        }
        rows.push(ChatListRow::Chat(index));
    }
    rows
}

fn section_height(pinned: bool) -> f32 {
    if pinned { 26.0 } else { 32.0 }
}

fn section_offsets(rows: &[ChatListRow], row_height: f32) -> Vec<f32> {
    let mut offsets = vec![0.0];
    for row in rows {
        offsets.push(
            offsets.last().unwrap()
                + match row {
                    ChatListRow::Header(pinned) => section_height(*pinned),
                    ChatListRow::Chat(_) => row_height,
                },
        );
    }
    offsets
}

pub(super) fn reveal_bounds(current: f32, viewport: f32, top: f32, bottom: f32) -> f32 {
    if top < current {
        top
    } else if bottom > current + viewport {
        (bottom - viewport).max(0.0)
    } else {
        current
    }
}

fn section_header(app: &App, ui: &mut egui::Ui, pinned: bool, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    if !pinned {
        ui.painter().hline(
            (rect.left() + 14.0)..=(rect.right() - 14.0),
            rect.top() + 1.0,
            egui::Stroke::new(1.0, app.palette.outline),
        );
    }
    // Keep each catalog key a literal call: extraction reads gettext's
    // argument, not a conditional expression.
    let label = if pinned {
        crate::i18n::gettext(app.locale, "Pinned")
    } else {
        crate::i18n::gettext(app.locale, "Other chats")
    };
    ui.painter().text(
        rect.left_center() + vec2(16.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        app.palette.secondary,
    );
}

fn list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // Taken here so a send seen while the list is hidden cannot move it later.
    let to_top = std::mem::take(&mut app.scroll_chats_to_top);
    if app.locked_folder_open() {
        locked_list(app, ui);
        return;
    }
    if app.secret_code_matched() {
        // Typing the secret code hides every other result: the folder entry
        // is all the search reveals until it is clicked.
        locked_entry(app, ui);
        return;
    }
    if app.show_starred {
        starred_list(app, ui);
        return;
    }
    if app.show_saved {
        saved_list(app, ui);
        return;
    }
    if !app.search.trim().is_empty() {
        results(app, ui);
        return;
    }
    if !app.show_archived {
        super::flat_groups::list(app, ui, to_top);
        return;
    }
    // A pin is a shortcut, not a removal: pinned chats keep their rail
    // chips AND their ordinary rows here, linked ones included.
    // One traversal keeps both the ids and the favorite flags: a per-row
    // app.chat lookup would repeat a hash search for every chat every frame.
    let (chat_ids, favorites): (Vec<ChatId>, Vec<bool>) = app
        .visible_chats()
        .into_iter()
        .map(|chat| (chat.id.clone(), chat.favorite))
        .unzip();
    if chat_ids.is_empty() {
        // The favorites title is translated, so it is bound here: the tuple
        // below borrows it for this frame, and every other arm stays a literal.
        let favorites_title;
        let (title, body) = if app.show_archived {
            ("Nothing archived", "Archived chats appear here.")
        } else if app.chat_filter != ChatFilter::All {
            let title = match app.chat_filter {
                ChatFilter::Unread => "No unread chats",
                ChatFilter::ReplyLater => "Nothing marked to reply to later",
                ChatFilter::ReadLater => "Nothing marked to read later",
                ChatFilter::Private => "No private chats",
                ChatFilter::Channels => "No channels",
                ChatFilter::Favorites => {
                    favorites_title = crate::i18n::gettext(app.locale, "No favorites yet");
                    favorites_title.as_ref()
                }
                _ => "No groups",
            };
            (title, "Choose All to see every chat.")
        } else if app.beeper_network_filter.is_some() {
            ("No chats in this view", "Choose Chats for all networks.")
        } else if app.syncing {
            ("Loading your chats", "Receiving history from your phone.")
        } else {
            (
                "No chats yet",
                "Use New chat to message a contact or yourself.",
            )
        };
        widgets::empty_state(ui, &palette, Icon::MessageCircle, title, body);
        return;
    }
    // Labels appear only when both groups remain in this filtered view.
    let list_rows = sectioned_rows(&favorites);
    // Virtual headers have short, noninteractive rows of their own.
    ui.spacing_mut().item_spacing.y = 0.0;
    let row_height = chat_row_height(app.settings.beeper.compact_list);
    let offsets = section_offsets(&list_rows, row_height);
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("chat-list")
        .auto_shrink([false, false]);
    if to_top && near_top(ui, "chat-list", row_height) {
        scroll_area = scroll_area.vertical_scroll_offset(0.0);
    }
    let target_row = app.scroll_chat_into_view.as_ref().and_then(|target| {
        list_rows
            .iter()
            .position(|row| matches!(row, ChatListRow::Chat(index) if &chat_ids[*index] == target))
    });
    // The copies a narrow window's slide draws have widget ids of their own:
    // they show the real list's offset instead of keeping one.
    let copy = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(slide_copy_id()))
        .is_some();
    if copy
        && let Some(offset) = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(shared_offset_id()))
    {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    // A disabled list leaves the request and any carried scroll to the list
    // drawn for real.
    if let Some(target_row) = target_row.filter(|_| ui.is_enabled()) {
        let id = ui.make_persistent_id(egui::IdSalt::new("chat-list"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = reveal_bounds(
            current,
            ui.available_height(),
            offsets[target_row],
            offsets[target_row + 1],
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    // A scroll gesture that began over the list stays with it (#274).
    let carried = if ui.is_enabled() {
        app.scroll_route.take(crate::app::ScrollPane::Chats)
    } else {
        0.0
    };
    let output = scroll_area.show_viewport(ui, |ui, viewport| {
        ui.set_height(*offsets.last().unwrap_or(&0.0));
        let start = offsets
            .partition_point(|offset| *offset <= viewport.top())
            .saturating_sub(1)
            .min(list_rows.len());
        let end = offsets
            .partition_point(|offset| *offset < viewport.bottom())
            .min(list_rows.len());
        let top = ui.max_rect().top();
        let rect = Rect::from_x_y_ranges(
            ui.max_rect().x_range(),
            (top + offsets[start])..=(top + offsets[end]),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.skip_ahead_auto_ids(start);
            let range = start..end;
            if carried != 0.0 {
                ui.scroll_with_delta_animation(
                    vec2(0.0, carried),
                    egui::style::ScrollAnimation::none(),
                );
            }
            for index in range {
                let chat_index = match list_rows[index] {
                    ChatListRow::Header(pinned) => {
                        section_header(app, ui, pinned, section_height(pinned));
                        continue;
                    }
                    ChatListRow::Chat(index) => index,
                };
                let Some(display) = app
                    .chat(&chat_ids[chat_index])
                    .map(|chat| app.beeper_person_preview(chat))
                else {
                    continue;
                };
                // Key by chat so an open menu survives list reordering.
                let response = ui
                    .push_id(("chat", &display.id), |ui| row(app, ui, &display))
                    .inner;
                // Store the row rect for interaction tests.
                ui.ctx()
                    .data_mut(|data| data.insert_temp(chat_row_id(&display.id), response.rect));
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(chat_row_id(&display.id).with("widget"), response.id);
                });
                if response.clicked() && !app.show_archived {
                    app.actions.push(Action::KeepUnread(display.id.clone()));
                }
            }
        });
    });
    if !copy {
        ui.ctx()
            .data_mut(|data| data.insert_temp(shared_offset_id(), output.state.offset.y));
    }
    app.scroll_route
        .place(crate::app::ScrollPane::Chats, output.inner_rect);
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(list_offset_id(), output.state.offset.y);
        data.insert_temp(list_offset_id().with("id"), output.id);
    });
}

/// Whether the list scrolled by `salt` is near enough its top to follow a sent
/// message's chat there: at most one row down, so its first row still shows
/// at least in part. A reader further down is working through older chats, and
/// a send there keeps their place instead of throwing them back to the top.
pub(super) fn near_top(ui: &egui::Ui, salt: &str, row_height: f32) -> bool {
    let id = ui.make_persistent_id(egui::IdSalt::new(salt));
    let offset = egui::scroll_area::State::load(ui.ctx(), id)
        .unwrap_or_default()
        .offset
        .y;
    offset <= row_height
}

/// Set while a narrow window's slide draws copies of the list.
pub(crate) fn slide_copy_id() -> egui::Id {
    egui::Id::new("chat-list-slide-copy")
}

/// The real list's scroll offset, for the copies a slide draws.
pub(super) fn shared_offset_id() -> egui::Id {
    egui::Id::new("chat-list-real-offset")
}

/// Where the chat list's scroll offset is kept for tests.
#[cfg(test)]
pub(crate) fn list_offset_id() -> egui::Id {
    egui::Id::new("chat-list-offset")
}

/// The row the secret code reveals: the only thing the search then shows.
fn locked_entry(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let count = app.locked_count();
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(22.0));
        Icon::LockOpen
            .image(palette.accent, 22.0)
            .paint_at(ui, icon_rect);
        ui.painter().text(
            pos2(rect.left() + 76.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Locked chats",
            theme::medium(14.5),
            palette.text,
        );
        ui.painter().text(
            pos2(rect.right() - 16.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            theme::regular(12.5),
            palette.accent,
        );
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::OpenLockedFolder);
    }
}

/// The authenticated folder, including archived chats and local title search.
fn locked_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let row_height = chat_row_height(app.settings.beeper.compact_list);
    let chats = app.visible_chats();
    if chats.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::LockOpen,
            "No locked chats",
            "Lock a chat from its context menu to hide it here.",
        );
        return;
    }
    let chats: Vec<Chat> = chats.into_iter().cloned().collect();
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("locked-chats")
        .auto_shrink([false, false]);
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    if let Some(target_row) = target_row {
        let id = ui.make_persistent_id(egui::IdSalt::new("locked-chats"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = row_scroll_offset(
            current,
            ui.available_height(),
            target_row,
            row_height,
            ui.spacing().item_spacing.y,
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    scroll_area.show_rows(ui, row_height, chats.len(), |ui, range| {
        for chat in &chats[range] {
            ui.push_id(("chat", &chat.id), |ui| row(app, ui, chat));
        }
    });
}

/// Returns the smallest offset that fully reveals a fixed-height row.
fn row_scroll_offset(
    current: f32,
    viewport_height: f32,
    row: usize,
    row_height: f32,
    spacing: f32,
) -> f32 {
    let top = row as f32 * (row_height + spacing);
    let bottom = top + row_height;
    if top < current {
        top
    } else if bottom > current + viewport_height {
        (bottom - viewport_height).max(0.0)
    } else {
        current
    }
}

/// Search results grouped into chats, messages, and contacts.
fn results(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let chats: Vec<Chat> = app.beeper_display_chats();
    let hits: Vec<Message> = app.scoped_search_hits();
    if let Some((query, error, truncated)) = &app.beeper_search_state
        && query == app.search.trim()
    {
        if let Some(error) = error {
            ui.add(egui::Label::new(egui::RichText::new(error).color(palette.warning)).wrap());
            if *truncated && ui.button("Retry loading message results").clicked() {
                app.actions.push(Action::SearchMore);
            }
        } else if *truncated {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "More message results are available. Load more to search older matches.",
                    )
                    .color(palette.warning),
                )
                .wrap(),
            );
            if ui.button("Load more message results").clicked() {
                app.actions.push(Action::SearchMore);
            }
        }
    }
    let contacts: Vec<Contact> = if app.search_scope_active() {
        Vec::new()
    } else {
        app.matching_contacts().into_iter().cloned().collect()
    };
    let needle = crate::util::search_key(app.search.trim());
    let offer_self = !app.search_scope_active()
        && !needle.is_empty()
        && app.offers_self(&needle)
        && !chats
            .iter()
            .any(|chat| app.me.as_deref() == Some(chat.id.as_str()));
    if chats.is_empty() && hits.is_empty() && contacts.is_empty() && !offer_self {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            if app
                .beeper_search_state
                .as_ref()
                .is_some_and(|(query, error, _)| query == app.search.trim() && error.is_some())
            {
                "Message search failed"
            } else {
                "No results"
            },
            "Try another name, number, or message text.",
        );
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("search-results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !chats.is_empty() {
                section(ui, &palette, "Chats");
                for chat in &chats {
                    let reveal = ui.is_enabled()
                        && app.scroll_chat_into_view.as_deref() == Some(chat.id.as_str());
                    let response = ui
                        .push_id(("chat", &chat.id), |ui| row(app, ui, chat))
                        .inner;
                    if reveal {
                        response.scroll_to_me(None);
                        app.scroll_chat_into_view = None;
                    }
                }
            }
            if !hits.is_empty() {
                section(ui, &palette, "Messages");
                for hit in &hits {
                    ui.push_id(("hit", &hit.chat, &hit.id), |ui| hit_row(app, ui, hit));
                }
            }
            if !contacts.is_empty() || offer_self {
                section(ui, &palette, "Contacts");
                if offer_self {
                    ui.push_id("self", |ui| self_row(app, ui));
                }
                for contact in &contacts {
                    ui.push_id(("contact", &contact.id), |ui| contact_row(app, ui, contact));
                }
            }
            ui.add_space(8.0);
        });
}

fn section(ui: &mut egui::Ui, palette: &Palette, label: &str) {
    ui.add_space(10.0);
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 0,
            bottom: 10,
        })
        .show(ui, |ui| {
            theme::text(ui, label, theme::semibold(12.5), palette.accent);
        });
}

/// A message search result. Clicking it opens the chat at that message.
fn hit_row(app: &mut App, ui: &mut egui::Ui, hit: &Message) {
    let palette = app.palette;
    let title = match app.chat(&hit.chat) {
        Some(chat) => app.chat_title(&chat.clone()),
        None => app.display_name_or(&hit.chat, None),
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&hit.chat);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &hit.chat,
            picture.as_deref(),
        );
        let left = rect.left() + 76.0;
        let right = rect.right() - 14.0;
        // A hit without a real message time (a transcript hit whose
        // message is not loaded) paints no stamp.
        let name_top = rect.top()
            + if app.settings.beeper.compact_list {
                7.0
            } else {
                14.0
            };
        // A hit without a real message time (a transcript hit whose
        // message is not loaded) paints no stamp: the file's mtime is not
        // the message's time.
        let stamp_width = if hit.timestamp > 0 {
            let stamp_galley = ui.painter().layout_no_wrap(
                crate::util::chat_stamp(app.locale, hit.timestamp),
                theme::regular(11.5),
                palette.dim,
            );
            let width = stamp_galley.size().x;
            ui.painter().galley(
                pos2(right - width, name_top + 1.0),
                stamp_galley,
                palette.dim,
            );
            width
        } else {
            0.0
        };
        let name_width = (right - stamp_width - 8.0 - left).max(0.0);
        let name_color = app
            .chat(&hit.chat)
            .cloned()
            .and_then(|chat| quiet_tint(app.chat_category(&chat), &palette))
            .unwrap_or(palette.text);
        let name = widgets::line(ui, &title, theme::medium(14.5), name_color, name_width, 1);
        name.paint(ui, pos2(left, name_top), name_color);
        // Show the sender for group messages.
        let line_y = rect.top()
            + if app.settings.beeper.compact_list {
                29.0
            } else {
                38.0
            };
        let mut x = left;
        if hit.from_me {
            let who = widgets::line(
                ui,
                "You: ",
                theme::regular(13.0),
                palette.dim,
                (right - x) * 0.5,
                1,
            );
            who.paint(ui, pos2(x, line_y), palette.dim);
            x += who.size().x;
        } else if crate::model::ChatKind::from_id(&hit.chat) == crate::model::ChatKind::Group {
            let sender = app.display_name_or(&hit.sender, hit.sender_name.as_deref());
            let first = app.short_name(&hit.sender, &sender);
            let who = widgets::line(
                ui,
                &format!("{first}: "),
                theme::regular(13.0),
                palette.dim,
                (right - x) * 0.5,
                1,
            );
            who.paint(ui, pos2(x, line_y), palette.dim);
            x += who.size().x;
        }
        let words = widgets::line(
            ui,
            &app.preview_line(&hit.summary_in(app.locale), hit),
            theme::regular(13.0),
            palette.dim,
            (right - x).max(0.0),
            1,
        );
        words.paint(ui, pos2(x, line_y), palette.dim);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: hit.chat.clone(),
            message: hit.id.clone(),
        });
    }
}

/// A contact without a chat. Clicking starts one.
pub(super) fn contact_row(app: &mut App, ui: &mut egui::Ui, contact: &Contact) {
    let name = app.display_name(&contact.id);
    let detail = crate::model::phone_of(&contact.id).map(crate::util::phone);
    if person_row(app, ui, &contact.id, &name, detail.as_deref()).clicked() {
        app.actions.push(Action::StartChat {
            id: contact.id.clone(),
            name,
            dismiss_dialog: true,
        });
    }
}

/// Offers the chat with ourselves, as "Name (You)" over "Message yourself".
pub(super) fn self_row(app: &mut App, ui: &mut egui::Ui) {
    let Some(me) = app.me.clone() else {
        return;
    };
    let name = app.self_title();
    let detail = crate::i18n::gettext(app.locale, "Message yourself");
    if person_row(app, ui, &me, &name, Some(detail.as_ref())).clicked() {
        app.actions.push(Action::MessageYourself);
    }
}

/// A row that starts something, drawn like a person's: an icon on the
/// accent colour where the photo would be. New chat's New group and Add
/// contact.
pub(super) fn action_row(app: &App, ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let palette = app.palette;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if ui.is_rect_visible(rect) {
        if response.hovered() || response.has_focus() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let circle =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        ui.painter()
            .circle_filled(circle.center(), circle.width() / 2.0, palette.accent);
        theme::paint_icon(ui, icon, circle, 22.0, palette.on_accent);
        let left = rect.left() + 76.0;
        let line = widgets::line(
            ui,
            label,
            theme::medium(14.5),
            palette.text,
            rect.right() - 14.0 - left,
            1,
        );
        line.paint(
            ui,
            pos2(left, rect.center().y - line.size().y / 2.0),
            palette.text,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn person_row(
    app: &mut App,
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    detail: Option<&str>,
) -> egui::Response {
    let palette = app.palette;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(id);
        widgets::paint_avatar(ui, &palette, avatar_rect, name, id, picture.as_deref());
        let left = rect.left() + 76.0;
        let name_line = widgets::line(
            ui,
            name,
            theme::medium(14.5),
            palette.text,
            rect.right() - 14.0 - left,
            1,
        );
        name_line.paint(ui, pos2(left, rect.top() + 14.0), palette.text);
        if let Some(detail) = detail {
            let phone_line = widgets::line(
                ui,
                detail,
                theme::regular(13.0),
                palette.dim,
                rect.right() - 14.0 - left,
                1,
            );
            phone_line.paint(ui, pos2(left, rect.top() + 38.0), palette.dim);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// How far a chat's name fades toward the background while the list is a
/// search result, by quiet tier. People and Notifications keep the normal
/// text colour. The name keeps the text's own hue; only its opacity drops,
/// and never below 4.5:1 on the panel or a hovered row, so a custom palette
/// cannot hide a name either.
fn quiet_tint(
    category: Option<crate::categories::AutoCategory>,
    palette: &Palette,
) -> Option<egui::Color32> {
    use crate::categories::AutoCategory;
    let fade = match category? {
        AutoCategory::Quiet => 0.25,
        AutoCategory::LongQuiet => 0.45,
        _ => return None,
    };
    let faded = theme::blend(palette.text, palette.panel, fade);
    let faded = theme::readable_on(palette.surface_hover, faded, palette.text, 4.5);
    Some(theme::readable_on(palette.panel, faded, palette.text, 4.5))
}

pub(super) fn row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    // Beeper demo: tag each row with its bridge network/account when the
    // backend supplied one (WhatsApp rows are unchanged).
    let title = match &chat.network {
        Some(network) if app.settings.beeper.show_network_labels => format!("{title} · {network}"),
        _ => title,
    };
    // While searching, the result reached with the arrows is the selection;
    // before any arrow press it stays the open chat, as a click leaves it.
    let batch_selected =
        app.batch_ui.owner == app.account().id.0 && app.batch_ui.selected_chats.contains(&chat.id);
    let selected = app
        .search_selected
        .as_ref()
        .filter(|_| !app.search.trim().is_empty())
        .or(app.open_chat.as_ref())
        .is_some_and(|selected| *selected == chat.id);
    let now = crate::util::now();
    let muted = chat.muted(now);
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            chat_row_height(app.settings.beeper.compact_list),
        ),
        Sense::click_and_drag(),
    );
    response.dnd_set_drag_payload(super::flat_groups::member(app, chat));
    theme::reveal_focus(&response);
    // The preview area and the whole last message, when the row cuts it short.
    let mut full_preview: Option<(Rect, String, String)> = None;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            unread_announcement(&title, chat),
        )
    });
    if ui.is_rect_visible(rect) {
        if selected || batch_selected {
            widgets::row_highlight(ui, &palette, rect, palette.surface_active);
        } else if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &chat.id,
            picture.as_deref(),
        );
        if chat.disappearing_timer().is_some() {
            widgets::paint_disappearing_badge(ui, &palette, avatar_rect);
        }
        network_badges(app, ui, &palette, avatar_rect, chat);

        let left = rect.left() + 76.0;
        let right = rect.right() - 14.0;
        let stamp = if chat.last_activity > 0 {
            crate::util::chat_stamp(app.locale, chat.last_activity)
        } else {
            String::new()
        };
        let unread = chat.looks_unread();
        let stamp_color = if unread && !muted {
            palette.accent
        } else {
            palette.dim
        };
        let stamp_galley = ui
            .painter()
            .layout_no_wrap(stamp, theme::regular(11.5), stamp_color);
        let name_top = chat_row_text_top(rect);
        ui.painter().galley(
            pos2(right - stamp_galley.size().x, name_top + 1.0),
            stamp_galley.clone(),
            stamp_color,
        );
        let name_width = (right - stamp_galley.size().x - 8.0 - left).max(0.0);
        let name_font = if unread {
            theme::semibold(14.5)
        } else {
            theme::medium(14.5)
        };
        // While searching, the name fades by quiet tier; the selected row stays plain.
        let name_color = if app.search.trim().is_empty() || selected {
            palette.text
        } else {
            quiet_tint(app.chat_category(chat), &palette).unwrap_or(palette.text)
        };
        let name = widgets::line(ui, &title, name_font, name_color, name_width, 1);
        name.paint(ui, pos2(left, name_top), name_color);

        // Leave room for badges beside the latest-message preview.
        let mut badge_right = right;
        let line_y = name_top + 24.0;
        if unread {
            let width = widgets::unread_indicator(
                ui,
                &palette,
                pos2(badge_right - 10.0, line_y + 8.0),
                if app.settings.beeper.show_unread_counts {
                    chat.unread
                } else {
                    0
                },
                chat.marked_unread || !app.settings.beeper.show_unread_counts,
                muted,
            );
            badge_right -= width + 6.0;
        }
        if chat.unread > 0 && chat.unread_mention.is_some() {
            // A mention calls even in a muted chat, as on WhatsApp.
            let width =
                widgets::mention_badge(ui, &palette, pos2(badge_right - 10.0, line_y + 8.0));
            badge_right -= width + 6.0;
        }
        if muted {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(15.0));
            Icon::VolumeX
                .image(palette.dim, 15.0)
                .paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        if chat.favorite {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(14.0));
            Icon::Heart.image(palette.dim, 14.0).paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        if app.reply_later_contains(chat) {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(14.0));
            Icon::Clock.image(palette.dim, 14.0).paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        if app.read_later_contains(chat) {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(14.0));
            Icon::ListChecks
                .image(palette.dim, 14.0)
                .paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        let mut x = left;
        let typing = app.typing_in(&chat.id);
        let preview_color = if unread && !muted {
            palette.secondary
        } else {
            palette.dim
        };
        let preview = if !app.settings.beeper.show_previews
            || !app
                .settings
                .beeper
                .account(chat.backend_settings_key().as_deref())
                .previews
        {
            widgets::line(ui, "", theme::regular(13.0), preview_color, 1.0, 1)
        } else if !typing.is_empty() {
            let who = if chat.is_group() {
                format!("{} is typing…", typing[0].1.trim_start_matches('~'))
            } else {
                "typing…".to_owned()
            };
            widgets::line(
                ui,
                &who,
                theme::medium(13.0),
                palette.accent,
                badge_right - x,
                1,
            )
        } else if let Some(draft) = app.draft_preview(&chat.id) {
            // Unsent text waits here as in WhatsApp, marked in the accent.
            let label = format!("{} ", crate::i18n::gettext(app.locale, "Draft:"));
            let label = widgets::line(
                ui,
                &label,
                theme::medium(13.0),
                palette.accent,
                (badge_right - x) * 0.5,
                1,
            );
            let width = label.size().x;
            label.paint(ui, pos2(x, line_y), palette.accent);
            x += width;
            widgets::line(
                ui,
                &draft,
                theme::regular(13.0),
                preview_color,
                (badge_right - x).max(0.0),
                1,
            )
        } else if let Some(last) = &chat.last {
            let mut prefix = String::new();
            if last.from_me {
                let tick_rect =
                    Rect::from_center_size(pos2(x + 8.0, line_y + 8.0), Vec2::splat(16.0));
                let delivery = super::conversation::display_delivery(
                    last.status,
                    last.from_me,
                    chat.kind,
                    app.account_receipts_off,
                );
                widgets::ticks(ui, &palette, tick_rect, delivery);
                x += advance_after_ticks(delivery);
            } else if chat.is_group() {
                let sender = app.display_name_or(&last.sender, last.sender_name.as_deref());
                let first = app.short_name(&last.sender, &sender);
                prefix = format!("{first}: ");
                let sender = widgets::line(
                    ui,
                    &prefix,
                    theme::regular(13.0),
                    preview_color,
                    (badge_right - x) * 0.5,
                    1,
                );
                let width = sender.size().x;
                sender.paint(ui, pos2(x, line_y), preview_color);
                x += width;
            }
            let words = widgets::line(
                ui,
                &crate::markup::plain(
                    &app.resolve_mention_tokens(&last.summary_in(app.locale)),
                    &[],
                ),
                theme::regular(13.0),
                preview_color,
                (badge_right - x).max(0.0),
                1,
            );
            // The row shows one line: offer the whole message when that line
            // was cut short or the message has more lines than it.
            if words.galley.elided || last.full.trim_end() != last.summary {
                let area = Rect::from_min_max(
                    pos2(left, line_y - 4.0),
                    pos2(badge_right, line_y + words.size().y.max(16.0) + 4.0),
                );
                full_preview = Some((area, prefix, last.full_in(app.locale).into_owned()));
            }
            words
        } else {
            widgets::line(ui, "", theme::regular(13.0), preview_color, 1.0, 1)
        };
        preview.paint(ui, pos2(x, line_y), preview_color);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = if let Some(network) = &chat.network {
        response.on_hover_text(format!("{}\n{}", app.chat_title(chat), network))
    } else {
        response
    };
    if let Some((area, prefix, full)) = full_preview {
        full_preview_tooltip(app, ui, &chat.id, area, &prefix, &full);
    }
    if response.clicked() {
        if ui.input(|input| input.modifiers.command) {
            super::batch_actions::toggle_chat(app, &chat.id);
        } else {
            app.actions.push(Action::OpenChat(chat.id.clone()));
        }
    }
    let menu_palette = palette;
    // The favorite item is translated, so its width counts in the reader's
    // language.
    let favorite_label = if chat.favorite {
        crate::i18n::gettext(app.locale, "Remove from favorites")
    } else {
        crate::i18n::gettext(app.locale, "Add to favorites")
    };
    let menu_width = widgets::menu_width(
        ui,
        &[
            "Mark as read",
            "Mark as unread",
            "Pin to top",
            favorite_label.as_ref(),
            "Unarchive",
            &crate::i18n::gettext(app.locale, "Leave group"),
            &crate::i18n::gettext(app.locale, "Leave channel"),
            "Unlock chat",
            "Copy number",
            &crate::i18n::gettext(app.locale, "Clear chat"),
            &crate::i18n::gettext(app.locale, "Delete chat"),
        ],
        true,
    )
    // Submenu rows also carry a chevron.
    .max(
        widgets::menu_width(
            ui,
            &[
                &crate::i18n::gettext(app.locale, "Mute"),
                "Notification sound",
                "Voice message speed",
                &crate::i18n::gettext(app.locale, "Labels"),
            ],
            true,
        ) + 24.0,
    )
    .max(190.0);
    let popup = egui::Popup::context_menu(&response)
        .width(menu_width)
        .frame(widgets::menu_frame(&menu_palette));
    #[cfg(any(test, feature = "demo"))]
    let popup = if app.open_chat_menu.as_deref() == Some(chat.id.as_str()) {
        popup
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .at_position(response.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup
    };
    popup.show(|ui| context_menu(app, ui, chat, &menu_palette));
    response
}

/// The widest a chat row's full-message tooltip grows before it wraps.
const FULL_PREVIEW_WIDTH: f32 = 360.0;
/// Lines a full-message tooltip shows before it ends in an ellipsis.
const FULL_PREVIEW_ROWS: usize = 12;
/// Characters of a message the tooltip lays out: far more than its rows
/// hold, so a very long message costs no more than a long one.
const FULL_PREVIEW_CHARS: usize = 2_000;

/// Where a chat row's latest-message preview sits, for hover tests.
pub fn preview_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-preview", chat))
}

/// Shows the whole last message while the pointer rests on a chat row's
/// cut-short preview, as WhatsApp Web does. It keeps out of the way of an
/// open menu and of a drag.
fn full_preview_tooltip(
    app: &App,
    ui: &egui::Ui,
    chat: &str,
    area: Rect,
    prefix: &str,
    full: &str,
) {
    let hover = ui.interact(area, preview_id(chat), Sense::hover());
    if egui::Popup::is_any_open(ui.ctx()) || ui.ctx().dragged_id().is_some() {
        return;
    }
    egui::Tooltip::for_enabled(&hover)
        .width(FULL_PREVIEW_WIDTH)
        .show(|ui| {
            let full: String = full.chars().take(FULL_PREVIEW_CHARS).collect();
            let text = format!(
                "{prefix}{}",
                crate::markup::plain(&app.resolve_mention_tokens(&full), &[])
            );
            let color = ui.visuals().text_color();
            let line = widgets::line(
                ui,
                text.trim_end(),
                theme::regular(13.0),
                color,
                FULL_PREVIEW_WIDTH,
                FULL_PREVIEW_ROWS,
            );
            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
            line.paint(ui, rect.min, color);
        });
}

/// Height of one avatar cell in the collapsed list.
const COMPACT_CELL: f32 = 52.0;
/// Avatar size inside a collapsed cell.
const COMPACT_AVATAR: f32 = 36.0;
/// Height of the fade over the rail's avatars where they scroll on.
const COMPACT_FADE: f32 = 16.0;

/// Where one avatar in the collapsed chat list was drawn.
pub fn compact_chat_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-rail", chat))
}

/// Where one favorite chip in the sidebar's favorites bar was drawn.
pub fn favorite_chip_id(chat: &str) -> egui::Id {
    egui::Id::new(("favorite-chip", chat))
}

/// Where a favorite chip's unread badge was drawn, when the chat has one.
pub fn favorite_badge_id(chat: &str) -> egui::Id {
    egui::Id::new(("favorite-chip-badge", chat))
}

/// Width of the compact rail, which the chats' avatars fill while the
/// list is hidden. On macOS the traffic lights reach past it, over the
/// header beside it.
pub fn compact_width(_ctx: &egui::Context) -> f32 {
    super::rail::COMPACT
}

/// The rail's avatars while the list is hidden: the chats the full list
/// would show right now, under the same filter, search, archive, and
/// locked-folder state.
pub(super) fn compact_list(app: &mut App, ui: &mut egui::Ui) {
    // The copies a narrow window's slide draws show the real column's
    // offset, and leave its requests and carried scroll to it.
    let copy = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(slide_copy_id()))
        .is_some();
    let to_top = !copy && std::mem::take(&mut app.scroll_chats_to_top);
    if !app.locked_folder_open() && app.secret_code_matched() {
        // As in the full list, the secret code reveals only the way in.
        compact_locked_entry(app, ui);
        return;
    }
    let chats: Vec<Chat> = app.beeper_display_chats();
    // Exactly the height the rail gives it: egui's least scrolled height
    // would reach past the window's bottom in a short one.
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("chat-rail")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0);
    if to_top && near_top(ui, "chat-rail", COMPACT_CELL) {
        scroll_area = scroll_area.vertical_scroll_offset(0.0);
    }
    if copy
        && let Some(offset) = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(shared_offset_id().with("rail")))
    {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    // Alt+Up/Down reveals the chat it opens here too.
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .filter(|_| !copy)
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    if let Some(target_row) = target_row {
        let id = ui.make_persistent_id(egui::IdSalt::new("chat-rail"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = row_scroll_offset(
            current,
            ui.available_height(),
            target_row,
            COMPACT_CELL,
            ui.spacing().item_spacing.y,
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    let pane = crate::app::ScrollPane::Chats;
    let carried = if copy {
        0.0
    } else {
        app.scroll_route.take(pane)
    };
    // Only the avatars on screen are laid out, however many chats there are.
    let output = scroll_area.show_rows(ui, COMPACT_CELL, chats.len(), |ui, range| {
        let mut focused = None;
        if carried != 0.0 {
            ui.scroll_with_delta_animation(
                vec2(0.0, carried),
                egui::style::ScrollAnimation::none(),
            );
        }
        for chat in &chats[range] {
            let response = ui
                .push_id(("chat", &chat.id), |ui| compact_row(app, ui, chat))
                .inner;
            if response.clicked() && !app.locked_folder_open() && !app.show_archived {
                app.actions.push(Action::KeepUnread(chat.id.clone()));
            }
            if response.has_focus() {
                focused = Some(response);
            }
        }
        focused
    });
    // The rail itself scrolls in a short window. The inner scroll area has
    // consumed the avatar's focus target; reveal its viewport in the outer
    // one as well, so a focused avatar cannot stay clipped by the rail.
    if let Some(mut focused) = output.inner {
        focused.rect = output.inner_rect;
        focused.interact_rect = focused.rect.intersect(ui.clip_rect());
        theme::reveal_focus_in(ui, &focused);
    }
    if !copy {
        ui.ctx().data_mut(|data| {
            data.insert_temp(shared_offset_id().with("rail"), output.state.offset.y);
        });
        app.scroll_route.place(pane, output.inner_rect);
    }
    // Avatars cut off at either end fade into the rail, which says the
    // column scrolls on. A translucent rail would show the fade as a band.
    let palette = app.palette;
    if !palette.vibrant() {
        let rect = output.inner_rect;
        let offset = output.state.offset.y;
        if offset > 0.5 {
            widgets::fade_top(ui, rect, COMPACT_FADE, palette.rail());
        }
        if output.content_size.y - offset - rect.height() > 0.5 {
            widgets::fade_bottom(ui, rect, COMPACT_FADE, palette.rail());
        }
    }
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(list_offset_id().with("compact"), output.state.offset.y);
        data.insert_temp(list_offset_id().with("compact-viewport"), output.inner_rect);
        data.insert_temp(list_offset_id().with("compact-id"), output.id);
    });
}

/// The collapsed form of the entry the secret code reveals.
fn compact_locked_entry(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), COMPACT_CELL), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Locked chats")
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_CELL - 4.0)),
                egui::CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        Icon::LockOpen
            .image(palette.accent, 22.0)
            .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(22.0)));
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(compact_locked_id(), response.rect));
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Locked chats")
        .clicked()
    {
        app.actions.push(Action::OpenLockedFolder);
    }
}

/// Where the collapsed locked-chats entry was drawn.
fn compact_locked_id() -> egui::Id {
    egui::Id::new("chat-rail-locked")
}

/// One avatar in the collapsed chat list: a click opens the chat, hovering
/// names it, and unread chats carry the same badge as a full row.
fn compact_row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    let selected = app.open_chat.as_deref() == Some(chat.id.as_str());
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), COMPACT_CELL), Sense::click());
    // A short window leaves the rail's avatars little more than one row.
    theme::reveal_focus_in(ui, &response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            unread_announcement(&title, chat),
        )
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_CELL - 4.0)),
                egui::CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        let avatar_rect = Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_AVATAR));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &chat.id,
            picture.as_deref(),
        );
        if selected {
            paint_active_avatar_ring(ui, &palette, avatar_rect);
        }
        if chat.disappearing_timer().is_some() {
            widgets::paint_disappearing_badge(ui, &palette, avatar_rect);
        }
        network_badges(app, ui, &palette, avatar_rect, chat);
        if chat.looks_unread() {
            // Top right, clear of the disappearing-messages timer in the
            // bottom right corner. A muted chat's badge is dimmed, as in the
            // full row.
            widgets::unread_indicator(
                ui,
                &palette,
                compact_badge_center(avatar_rect),
                chat.unread,
                chat.marked_unread,
                chat.muted(crate::util::now()),
            );
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(compact_chat_id(&chat.id), response.rect));
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(title);
    if response.clicked() {
        app.actions.push(Action::OpenChat(chat.id.clone()));
    }
    response
}

/// Separated ring, drawn before the badges so their backing masks its arc.
/// Its 47-point outside diameter fits both 52-point avatar cells.
fn paint_active_avatar_ring(ui: &egui::Ui, palette: &Palette, avatar: Rect) {
    let radius = avatar.width() / 2.0 + 3.0;
    ui.painter().circle_stroke(
        avatar.center(),
        radius,
        egui::Stroke::new(5.0, palette.rail()),
    );
    ui.painter().circle_stroke(
        avatar.center(),
        radius,
        egui::Stroke::new(2.5, palette.accent),
    );
}

/// Centre of the unread badge on a collapsed avatar. A "99+" badge, the
/// widest, still ends inside the rail.
fn compact_badge_center(avatar: Rect) -> egui::Pos2 {
    pos2(avatar.right() - 10.0, avatar.top() + 2.0)
}

fn context_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
    if ui
        .button(crate::i18n::gettext(app.locale, "Select for archive"))
        .clicked()
    {
        super::batch_actions::toggle_chat(app, &chat.id);
        ui.close();
    }
    super::flat_groups::assign_menu(app, ui, chat);
    let locale = app.locale;
    if chat.looks_unread()
        && widgets::menu_item(
            ui,
            palette,
            Some(Icon::CheckCheck),
            &crate::i18n::gettext(locale, "Mark as read"),
        )
    {
        app.actions.push(Action::MarkRead(chat.id.clone()));
    }
    if !chat.looks_unread()
        && widgets::menu_item(
            ui,
            palette,
            Some(Icon::MessageCircle),
            &crate::i18n::gettext(locale, "Mark as unread"),
        )
    {
        app.actions.push(Action::MarkUnread(chat.id.clone()));
    }
    let queued = app.reply_later_contains(chat) || app.read_later_contains(chat);
    let label = if queued {
        crate::i18n::gettext(locale, "Clear deal with later")
    } else {
        crate::i18n::gettext(locale, "Deal with later")
    };
    if widgets::menu_item(ui, palette, Some(Icon::Clock), &label) {
        app.actions.push(Action::ToggleDealLater(chat.id.clone()));
    }
    // Move a one-to-one chat out of its automatic category for good. The
    // move is local, and the rules never re-sort a chat placed this way.
    if !chat.is_group() {
        let current = app.chat_category(chat);
        ui.menu_button(crate::i18n::gettext(locale, "Move to…"), |ui| {
            for category in crate::categories::AutoCategory::EVERY {
                if ui
                    .selectable_label(current == Some(category), category.label(locale))
                    .clicked()
                {
                    app.actions
                        .push(Action::SetChatCategory(chat.id.clone(), Some(category)));
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .button(crate::i18n::gettext(locale, "Sort automatically"))
                .clicked()
            {
                app.actions
                    .push(Action::SetChatCategory(chat.id.clone(), None));
                ui.close();
            }
        });
    }
    // Channels cannot be favorites, as on the phone. A Beeper chat's mark
    // lives here only: no phone knows its network's chats to sync it to.
    if !chat.is_channel() {
        // Bound before the call so the translated text outlives the borrow.
        let favorite_label = if chat.favorite {
            crate::i18n::gettext(locale, "Remove from favorites")
        } else {
            crate::i18n::gettext(locale, "Add to favorites")
        };
        if widgets::menu_item(ui, palette, Some(Icon::Heart), favorite_label.as_ref()) {
            app.actions
                .push(Action::SetFavorite(chat.id.clone(), !chat.favorite));
        }
    }
    let archive_label = if chat.archived {
        crate::i18n::gettext(locale, "Unarchive")
    } else {
        crate::i18n::gettext(locale, "Archive")
    };
    if widgets::menu_item(ui, palette, Some(Icon::Archive), archive_label.as_ref()) {
        app.actions
            .push(Action::SetArchived(chat.id.clone(), !chat.archived));
    }
    // One shot at total quiet, for high-volume chats: mute indefinitely and
    // archive, so the chat leaves the list, stops counting and never
    // notifies or resurfaces on its own. Undo with Unarchive and Unmute.
    if !(chat.archived && chat.muted(crate::util::now()))
        && widgets::menu_item(
            ui,
            palette,
            Some(Icon::BellOff),
            &crate::i18n::gettext(locale, "Hide and mute indefinitely"),
        )
    {
        app.actions.push(Action::SetMuted(chat.id.clone(), Some(0)));
        app.actions.push(Action::SetArchived(chat.id.clone(), true));
    }
    if chat.backend_account.is_some() {
        widgets::submenu(
            ui,
            palette,
            Icon::Bell,
            "Remind me",
            &["In 1 hour", "In 24 hours", "Clear reminder"],
            |ui| {
                for (label, seconds) in [
                    ("In 1 hour", Some(3600)),
                    ("In 24 hours", Some(86400)),
                    ("Clear reminder", None),
                ] {
                    if widgets::menu_item(ui, palette, None, label) {
                        let at = seconds
                            .and_then(|offset| {
                                jiff::Timestamp::from_second(crate::util::now() + offset).ok()
                            })
                            .map(|stamp| stamp.to_string());
                        app.actions.push(Action::SetBeeperReminder {
                            chat: chat.id.clone(),
                            at,
                        });
                        ui.close();
                    }
                }
            },
        );
    }
    // Bound before the call so the translated text outlives the borrow.
    let leave_label = if chat.is_channel() {
        crate::i18n::gettext(locale, "Leave channel")
    } else {
        crate::i18n::gettext(locale, "Leave group")
    };
    if chat.can_leave(&app.our_ids())
        && widgets::menu_item(ui, palette, Some(Icon::LogOut), leave_label.as_ref())
    {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(
                chat.id.clone(),
            )));
    }
    let now = crate::util::now();
    if chat.muted(now) {
        if widgets::menu_item(
            ui,
            palette,
            Some(Icon::Bell),
            &crate::i18n::gettext(locale, "Unmute"),
        ) {
            app.actions.push(Action::SetMuted(chat.id.clone(), None));
        }
    } else {
        mute_menu(app, ui, palette, chat, now);
    }
    if chat.backend_account.is_some() {
        widgets::menu_separator(ui, palette);
        if chat.kind == crate::model::ChatKind::Direct
            && widgets::menu_item(ui, palette, Some(Icon::Pencil), "Rename…")
        {
            // A local name, edited in Contact info, which opens focused on it.
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("rename-focus"), chat.id.clone()));
            app.actions.push(Action::OpenChat(chat.id.clone()));
            app.actions.push(Action::OpenInfo(chat.id.clone()));
        }
        if widgets::menu_item(ui, palette, Some(Icon::Info), "Info") {
            app.actions.push(Action::OpenChat(chat.id.clone()));
            app.actions.push(Action::OpenInfo(chat.id.clone()));
        }
        // Unsupported WhatsApp-only actions are not offered as working Beeper controls.
        return;
    }
    sound_menu(app, ui, palette, chat);
    voice_speed_menu(app, ui, palette, chat);
    labels::chat_menu(app, ui, chat, palette);
    let lock_label = if chat.locked {
        crate::i18n::gettext(locale, "Unlock chat")
    } else {
        crate::i18n::gettext(locale, "Lock chat")
    };
    if widgets::menu_item(
        ui,
        palette,
        Some(if chat.locked {
            Icon::LockOpen
        } else {
            Icon::Lock
        }),
        lock_label.as_ref(),
    ) {
        app.actions.push(if chat.locked {
            Action::SetLocked(chat.id.clone(), false)
        } else {
            Action::ShowDialog(Dialog::ConfirmLockChat(chat.id.clone()))
        });
    }
    widgets::menu_separator(ui, palette);
    if let Some(phone) = chat.phone()
        && widgets::menu_item(
            ui,
            palette,
            Some(Icon::Copy),
            &crate::i18n::gettext(locale, "Copy number"),
        )
    {
        app.actions.push(Action::CopyText(format!("+{phone}")));
    }
    if widgets::menu_item(
        ui,
        palette,
        Some(Icon::Info),
        &crate::i18n::gettext(locale, "Info"),
    ) {
        // The panel sits beside the conversation, so the chat opens first.
        app.actions.push(Action::OpenChat(chat.id.clone()));
        app.actions.push(Action::OpenInfo(chat.id.clone()));
    }
    // Clearing and deleting reach the phone, so both wait for a connection.
    let connected = app.is_connected();
    if widgets::menu_item_enabled(
        ui,
        palette,
        Some(Icon::Eraser),
        &crate::i18n::gettext(locale, "Clear chat"),
        connected,
    ) {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmClearChat(
                chat.id.clone(),
            )));
    }
    if widgets::menu_item_enabled(
        ui,
        palette,
        Some(Icon::Trash),
        &crate::i18n::gettext(locale, "Delete chat"),
        connected,
    ) {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmDeleteChat(
                chat.id.clone(),
            )));
    }
}

/// What a screen reader reads out for a chat's unread state. A chat marked
/// unread by hand has no count to read, so it must not announce zero.
fn unread_announcement(title: &str, chat: &Chat) -> String {
    if chat.marked_unread && chat.unread == 0 {
        format!("{title}, unread")
    } else if chat.unread > 0 && chat.unread_mention.is_some() {
        format!("{title}, {} unread messages, mentioned you", chat.unread)
    } else {
        format!("{title}, {} unread messages", chat.unread)
    }
}

/// A chat's own notification sound, overriding Settings for this chat.
fn sound_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette, chat: &Chat) {
    use crate::settings::NotificationSound;
    let locale = app.locale;
    let current = chat.notification_sound.clone();
    let default_label = crate::i18n::gettext(locale, "Default");
    let system_label = crate::i18n::gettext(locale, "System sound");
    let none_label = crate::i18n::gettext(locale, "No sound");
    let sounds: [(Option<NotificationSound>, std::borrow::Cow<'static, str>); 5] = [
        (None, default_label),
        (Some(NotificationSound::Receive), "Pidgin".into()),
        (Some(NotificationSound::Alert), "Pidgin alert".into()),
        (Some(NotificationSound::System), system_label),
        (Some(NotificationSound::None), none_label),
    ];
    let custom = match &current {
        Some(NotificationSound::Custom(path)) => Some(path.file_name().map_or_else(
            || crate::i18n::gettext(locale, "Custom").into_owned(),
            |name| name.to_string_lossy().into_owned(),
        )),
        _ => None,
    };
    let choose = crate::i18n::gettext(locale, "Choose a file…");
    let items: Vec<&str> = sounds
        .iter()
        .map(|(_, label)| label.as_ref())
        .chain(custom.as_deref())
        .chain([choose.as_ref()])
        .collect();
    let menu_label = crate::i18n::gettext(locale, "Notification sound");
    widgets::submenu(ui, palette, Icon::Volume2, &menu_label, &items, |ui| {
        for (sound, label) in &sounds {
            let checked = current == *sound;
            if widgets::menu_item(ui, palette, checked.then_some(Icon::Check), label.as_ref()) {
                app.actions.push(Action::SetChatSound {
                    chat: chat.id.clone(),
                    sound: sound.clone(),
                });
                ui.close();
            }
        }
        if let Some(name) = &custom {
            widgets::menu_item(ui, palette, Some(Icon::Check), name);
        }
        if widgets::menu_item(ui, palette, None, choose.as_ref()) {
            app.actions.push(Action::PickChatSound(chat.id.clone()));
            ui.close();
        }
    });
}

/// Exactly two mute durations. Beeper timed mutes are local to this app.
fn mute_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette, chat: &Chat, now: i64) {
    let morning = crate::util::next_mute_morning(now);
    let until = crate::i18n::gettext(app.locale, "Until 08:00");
    let forever = crate::i18n::gettext(app.locale, "Mute forever");
    let items = [until.as_ref(), forever.as_ref()];
    widgets::submenu(ui, palette, Icon::BellOff, "Mute", &items, |ui| {
        if chat.backend_account.is_some() {
            ui.small(crate::i18n::gettext(
                app.locale,
                "Timed mute applies only in Solmu.",
            ));
        }
        if let Some(end) = morning
            && widgets::menu_item(ui, palette, None, items[0])
        {
            app.actions
                .push(Action::SetMuted(chat.id.clone(), Some(end)));
            ui.close();
        }
        if widgets::menu_item(ui, palette, None, items[1]) {
            app.actions.push(Action::SetMuted(chat.id.clone(), Some(0)));
            ui.close();
        }
    });
}

/// A chat's own voice message speed, overriding Settings for this chat.
fn voice_speed_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette, chat: &Chat) {
    let options: Vec<(Option<f32>, String)> = std::iter::once((
        None,
        format!(
            "Default ({})",
            crate::audio::speed_label(app.settings.voice_speed)
        ),
    ))
    .chain(
        crate::audio::SPEEDS
            .into_iter()
            .map(|speed| (Some(speed), crate::audio::speed_label(speed))),
    )
    .collect();
    let items: Vec<&str> = options.iter().map(|(_, label)| label.as_str()).collect();
    widgets::submenu(
        ui,
        palette,
        Icon::Play,
        "Voice message speed",
        &items,
        |ui| {
            for (speed, label) in &options {
                let speed = *speed;
                let checked = chat.voice_speed == speed;
                if widgets::menu_item(ui, palette, checked.then_some(Icon::Check), label) {
                    app.actions.push(Action::SetChatVoiceSpeed {
                        chat: chat.id.clone(),
                        speed,
                    });
                    ui.close();
                }
            }
        },
    );
}

/// The chat-list row's advance past the delivery ticks before the preview
/// text. `ReadBySome` also paints a "+" after the ticks (widgets::ticks_in),
/// so it needs the plus's width too, or the plus overlaps the preview.
fn advance_after_ticks(delivery: crate::model::Delivery) -> f32 {
    if delivery == crate::model::Delivery::ReadBySome {
        30.0
    } else {
        20.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_tiers_fade_in_steps_stay_readable_and_people_stay_plain() {
        use crate::categories::AutoCategory;
        for palette in [Palette::dark(), Palette::light()] {
            let quiet = quiet_tint(Some(AutoCategory::Quiet), &palette).unwrap();
            let long = quiet_tint(Some(AutoCategory::LongQuiet), &palette).unwrap();
            // Each step is dimmer than the last: lower contrast on the panel.
            let on_panel = |color| theme::contrast(color, palette.panel);
            assert!(
                on_panel(palette.text) > on_panel(quiet),
                "dark={}",
                palette.dark
            );
            assert!(on_panel(quiet) > on_panel(long), "dark={}", palette.dark);
            for tint in [quiet, long] {
                assert!(on_panel(tint) >= 4.5, "dark={} {tint:?}", palette.dark);
                assert!(
                    theme::contrast(tint, palette.surface_hover) >= 4.5,
                    "dark={} {tint:?}",
                    palette.dark
                );
            }
            assert_eq!(quiet_tint(Some(AutoCategory::People), &palette), None);
            assert_eq!(
                quiet_tint(Some(AutoCategory::Notifications), &palette),
                None
            );
            assert_eq!(quiet_tint(None, &palette), None);
        }
    }

    #[test]
    fn a_custom_palette_cannot_hide_a_tiered_name() {
        use crate::categories::AutoCategory;
        // A light palette whose panel is exactly the Long quiet blue.
        let mut palette = Palette::light();
        palette.panel = egui::Color32::from_rgb(0x4A, 0x55, 0xA8);
        palette.text = egui::Color32::WHITE;
        for category in [AutoCategory::Quiet, AutoCategory::LongQuiet] {
            let tint = quiet_tint(Some(category), &palette).unwrap();
            assert!(theme::contrast(tint, palette.panel) >= 4.5, "{category:?}");
        }
    }

    #[test]
    fn the_read_by_some_plus_gets_its_own_width_in_the_row() {
        let plus = advance_after_ticks(crate::model::Delivery::ReadBySome);
        for delivery in [
            crate::model::Delivery::None,
            crate::model::Delivery::Unknown,
            crate::model::Delivery::Pending,
            crate::model::Delivery::Sent,
            crate::model::Delivery::Delivered,
            crate::model::Delivery::Read,
            crate::model::Delivery::Played,
            crate::model::Delivery::Failed,
        ] {
            assert!(
                advance_after_ticks(delivery) < plus,
                "{delivery:?} must not reserve the plus's width"
            );
        }
    }

    #[test]
    fn modes_share_a_row_below_search_even_while_typing() {
        for width in [260.0, 340.0, 520.0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = App::headless(AppDirs::under(directory.path()), Settings::default());
            app.search = "needle".into();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..2 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| header(&mut app, ui),
                );
                out.textures_delta.clear();
            }
            let search = ctx
                .read_response(egui::Id::new("chat-search"))
                .unwrap()
                .rect;
            let rects = [
                crate::model::ListMode::Recent,
                crate::model::ListMode::Unread,
                crate::model::ListMode::NeedsMe,
            ]
            .map(|f| {
                ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("list-mode", f.key()))))
                    .unwrap()
            });
            for rect in rects {
                assert!(rect.top() >= search.bottom());
                assert!(rect.right() <= width);
                assert_eq!(rect.height(), 36.0);
            }
            for pair in rects.windows(2) {
                assert_eq!(pair[0].center().y, pair[1].center().y);
                assert!((pair[0].width() - pair[1].width()).abs() < 0.01);
            }
            // The Bookmarks icon ends the same row and stays inside the width.
            let bookmarks = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new(("list-mode", "bookmarks"))))
                .unwrap();
            assert!(bookmarks.left() >= rects[2].right());
            assert!(bookmarks.right() <= width);
            assert!((bookmarks.center().y - rects[0].center().y).abs() < 0.01);
        }
    }

    #[test]
    fn the_bookmarks_switch_sits_under_the_list_header_and_fits() {
        for width in [260.0, 340.0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = App::headless(AppDirs::under(directory.path()), Settings::default());
            app.show_saved = true;
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..2 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| header(&mut app, ui),
                );
                out.textures_delta.clear();
            }
            let rect = |name: &str| {
                ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("bookmarks-switch", name))))
                    .unwrap()
            };
            let (starred, saved) = (rect("starred"), rect("saved"));
            assert!(starred.right() <= saved.left());
            assert!(saved.right() <= width);
            assert_eq!(starred.center().y, saved.center().y);
        }
    }

    #[test]
    fn favorites_sections_exist_only_with_both_visible_groups() {
        use ChatListRow::{Chat, Header};
        assert_eq!(sectioned_rows(&[]), vec![]);
        assert_eq!(sectioned_rows(&[false, false]), vec![Chat(0), Chat(1)]);
        assert_eq!(sectioned_rows(&[true, true]), vec![Chat(0), Chat(1)]);
        assert_eq!(
            sectioned_rows(&[true, true, false]),
            vec![Header(true), Chat(0), Chat(1), Header(false), Chat(2)]
        );
    }

    #[test]
    fn favorites_headers_are_included_in_virtual_scroll_geometry() {
        let rows = sectioned_rows(&[true, false, false]);
        let offsets = section_offsets(&rows, 80.0);
        assert_eq!(offsets, vec![0.0, 26.0, 106.0, 138.0, 218.0, 298.0]);
        assert_eq!(reveal_bounds(0.0, 200.0, offsets[4], offsets[5]), 98.0);
        assert_eq!(reveal_bounds(98.0, 200.0, offsets[1], offsets[2]), 26.0);
        assert_eq!(section_offsets(&rows, 56.0).last(), Some(&226.0));
    }

    #[test]
    fn chat_density_keeps_virtual_rows_and_text_geometry_in_sync() {
        assert_eq!(chat_row_height(false), 80.0);
        assert_eq!(chat_row_height(true), 56.0);
        for compact in [false, true] {
            let height = chat_row_height(compact);
            let rect = Rect::from_min_size(pos2(10.0, 50.0), vec2(320.0, height));
            let top = chat_row_text_top(rect);
            assert_eq!(top - rect.top(), (height - 40.0) / 2.0);
            assert_eq!(top + 24.0 + 16.0, rect.bottom() - (top - rect.top()));
            assert!(rect.center().y - 24.0 >= rect.top());
            assert_eq!(row_scroll_offset(0.0, height * 2.0, 2, height, 0.0), height);
        }
    }

    /// A header title shows only when it fits before its buttons, whatever
    /// the language makes of it.
    #[test]
    fn header_titles_make_way_for_their_buttons() {
        let spacing = 8.0;
        // Two 30-point buttons take 30 + 8 + 30 = 68 points, and the title
        // keeps 8 from them.
        let two = row_width(2, 30.0, spacing);
        assert_eq!(two, 68.0);
        assert!(title_fits(100.0, 176.0, two, spacing));
        assert!(!title_fits(100.0, 175.0, two, spacing));
        // A third button, the Settings one off macOS, needs 38 more.
        let three = row_width(3, 30.0, spacing);
        assert!(!title_fits(100.0, 176.0, three, spacing));
        assert!(title_fits(100.0, 214.0, three, spacing));
        assert_eq!(row_width(0, 30.0, spacing), 0.0);
    }

    /// The Saved list keeps the scoped lists' Back control: opening it must
    /// not drop the arrow that returns to the chat list.
    #[test]
    fn the_saved_list_header_offers_a_way_back() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.show_saved = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        app.attach(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            header(&mut app, ui);
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let labels: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .collect();
        assert!(
            labels.contains(&"Back to chats"),
            "no way back in {labels:?}"
        );
    }

    /// The title measures its room from the list the header draws: New chat,
    /// and Hide the chat list only when the window is wide enough to show the
    /// list beside a chat. Calls, Starred and Settings are in the rail.
    #[test]
    fn header_buttons_follow_the_width() {
        use HeaderButton::HideList;
        let buttons = |width: f32| {
            let ctx = egui::Context::default();
            let mut buttons = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| buttons = header_buttons(ui.ctx()),
            );
            output.textures_delta.clear();
            buttons
        };
        assert_eq!(buttons(1000.0), [HideList]);
        assert!(buttons(400.0).is_empty());
    }

    use crate::paths::AppDirs;
    use crate::settings::Settings;

    #[test]
    fn chat_context_menu_stays_compact_in_wide_windows() {
        for width in [360.0, 1180.0, 2000.0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _events) =
                App::headless(AppDirs::under(directory.path()), Settings::default());
            let chat = Chat::new("fixture@g.us".into(), "Fixture group".into());
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let mut frame = |events| {
                let mut response = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(width, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| response = Some(row(&mut app, ui, &chat)),
                );
                output.textures_delta.clear();
                response.unwrap()
            };
            let response = frame(vec![]);
            let position = response.rect.left_center() + vec2(20.0, 0.0);
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            for _ in 0..3 {
                frame(vec![]);
            }
            let popup = response.id.with("popup");
            assert!(egui::Popup::is_id_open(&ctx, popup));
            let rect = ctx.read_response(popup).expect("chat context menu").rect;
            assert!(
                (190.0..=240.0).contains(&rect.width()),
                "a {width}-point window produced a {}-point menu",
                rect.width()
            );
        }
    }

    fn click_clear_chat_in_the_list_menu(app: &mut App, chat: &Chat) {
        let label = crate::i18n::gettext(app.locale, "Clear chat");
        click_item_in_the_list_menu(app, chat, label.as_ref());
    }

    /// Clicks the chat context menu item carrying this exact translated label.
    fn click_item_in_the_list_menu(app: &mut App, chat: &Chat, label: &str) {
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut frame = |events| {
            let mut response = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 700.0))),
                    events,
                    ..Default::default()
                },
                |ui| response = Some(row(app, ui, chat)),
            );
            output.textures_delta.clear();
            (response.unwrap(), output.shapes)
        };
        let (response, _) = frame(vec![]);
        let position = response.rect.left_center() + vec2(20.0, 0.0);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        // A popup's first frame measures its contents before its final placement.
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(vec![]).1;
        }
        let popup = response.id.with("popup");
        assert!(egui::Popup::is_id_open(&ctx, popup));
        let menu = ctx.read_response(popup).expect("chat context menu").rect;
        // Click the label the reader sees, rather than guessing its row offset.
        let position = shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let position = text.visual_bounding_rect().center();
                    (clipped.clip_rect.contains(position) && menu.contains(position))
                        .then_some(position)
                }
                _ => None,
            })
            .expect("the translated menu item is visible in the chat menu");
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    }

    #[test]
    fn clear_chat_in_the_list_menu_confirms_for_the_clicked_chat_when_connected() {
        for locale in [crate::i18n::Locale::English, crate::i18n::Locale::German] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = App::headless(AppDirs::under(directory.path()), Settings::default());
            let clicked = Chat::new("clicked@g.us".into(), "Clicked group".into());
            let open = Chat::new("open@s.whatsapp.net".into(), "Open chat".into());
            app.open_chat = Some(open.id.clone());
            app.link = crate::backend::LinkStatus::Connected;
            app.locale = locale;

            click_clear_chat_in_the_list_menu(&mut app, &clicked);

            // Only request confirmation for the clicked row: no clearing or navigation.
            assert_eq!(
                app.actions,
                [Action::ShowDialog(Dialog::ConfirmClearChat(
                    clicked.id.clone()
                ))],
                "{locale:?}"
            );
            assert_eq!(app.open_chat.as_deref(), Some(open.id.as_str()));
        }
    }

    #[test]
    fn clear_chat_in_the_list_menu_is_disabled_while_disconnected() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(directory.path()), Settings::default());
        let clicked = Chat::new("clicked@g.us".into(), "Clicked group".into());
        let open = Chat::new("open@s.whatsapp.net".into(), "Open chat".into());
        app.open_chat = Some(open.id.clone());
        app.link = crate::backend::LinkStatus::Disconnected {
            reason: "fixture".into(),
        };
        click_clear_chat_in_the_list_menu(&mut app, &clicked);

        assert!(app.actions.is_empty(), "{:?}", app.actions);
        assert_eq!(app.open_chat.as_deref(), Some(open.id.as_str()));
    }

    #[test]
    fn hide_and_mute_in_the_list_menu_mutes_and_archives_the_chat() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = Chat::new("noisy@g.us".into(), "Noisy group".into());

        let label = crate::i18n::gettext(app.locale, "Hide and mute indefinitely");
        click_item_in_the_list_menu(&mut app, &chat, label.as_ref());

        assert_eq!(
            app.actions,
            [
                Action::SetMuted(chat.id.clone(), Some(0)),
                Action::SetArchived(chat.id.clone(), true),
            ]
        );
    }

    #[test]
    fn alt_navigation_scrolls_the_destination_chat_into_view() {
        let root = std::env::temp_dir().join(format!(
            "solmu-chat-list-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
        let mut first = String::new();
        let mut last = String::new();
        for index in 0..24 {
            let id = format!("49170000{index:04}@s.whatsapp.net");
            let mut chat = Chat::new(id.clone(), format!("Chat {index:02}"));
            chat.last_activity = 100 - i64::from(index);
            if index == 0 {
                first.clone_from(&id);
            }
            last.clone_from(&id);
            app.chats.push(chat);
        }
        app.open_chat = Some(first);

        let ctx = egui::Context::default();
        app.attach(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
            events: vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::ALT,
            }],
            ..Default::default()
        };
        let mut offset = 0.0;
        let mut output = ctx.run_ui(input, |ui| {
            super::super::keys::handle(&mut app, ui.ctx());
            let scroll_id = ui.make_persistent_id(egui::IdSalt::new("chat-list"));
            list(&mut app, ui);
            offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                .expect("chat-list scroll state")
                .offset
                .y;
        });
        output.textures_delta.clear();

        assert!(
            app.actions.contains(&Action::OpenChat(last)),
            "Alt+Up wraps to the last visible chat"
        );
        assert!(app.scroll_chat_into_view.is_none(), "reveal was consumed");
        assert!(offset > 0.0, "the list moved down to reveal the last row");
    }

    /// Moves a drawn list (`id` names where its scroll area's id was kept)
    /// to `offset`, as a reader's scrolling would.
    fn scroll_list_to(ctx: &egui::Context, id: egui::Id, offset: f32) {
        let id = ctx
            .data(|data| data.get_temp::<egui::Id>(id))
            .expect("the list was drawn");
        let mut state = egui::scroll_area::State::load(ctx, id).expect("scroll state");
        state.offset.y = offset;
        state.store(ctx, id);
    }

    /// A send follows its chat to the top only from a list that is within a
    /// row of the top; further down, the reader keeps their place.
    #[test]
    fn a_sent_message_scrolls_the_chat_list_to_the_top_only_from_near_it() {
        let (_directory, mut app, ids, ctx) = rail_app(24);
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    ..Default::default()
                },
                |ui| list(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| data.get_temp::<f32>(list_offset_id()))
                .expect("chat-list offset")
        };
        app.scroll_chat_into_view = ids.last().cloned();
        let scrolled = frame(&mut app);
        let row_height = chat_row_height(app.settings.beeper.compact_list);
        assert!(scrolled > row_height, "the list starts far down");
        app.scroll_chats_to_top = true;
        assert_eq!(
            frame(&mut app),
            scrolled,
            "a reader far down keeps their place"
        );
        assert!(!app.scroll_chats_to_top, "the request was consumed");

        // Just over a row down still counts as working down the list.
        scroll_list_to(&ctx, list_offset_id().with("id"), row_height + 1.0);
        assert_eq!(frame(&mut app), row_height + 1.0);
        app.scroll_chats_to_top = true;
        assert_eq!(frame(&mut app), row_height + 1.0);

        scroll_list_to(&ctx, list_offset_id().with("id"), row_height / 2.0);
        assert_eq!(frame(&mut app), row_height / 2.0);
        app.scroll_chats_to_top = true;
        assert_eq!(frame(&mut app), 0.0, "the list is back at the top");
        assert_eq!(frame(&mut app), 0.0, "the list stays at the top");
    }

    #[test]
    fn a_send_while_search_results_show_does_not_move_the_list_later() {
        let (_directory, mut app, ids, ctx) = rail_app(24);
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    ..Default::default()
                },
                |ui| list(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| data.get_temp::<f32>(list_offset_id()))
                .expect("chat-list offset")
        };
        app.scroll_chat_into_view = ids.last().cloned();
        let scrolled = frame(&mut app);
        assert!(scrolled > 0.0, "the list starts scrolled down");

        app.search = "Chat".into();
        app.scroll_chats_to_top = true;
        frame(&mut app);
        assert!(!app.scroll_chats_to_top, "the request was dropped");
        app.search.clear();
        assert_eq!(frame(&mut app), scrolled, "the list keeps its place");
    }

    /// Numbered shortcuts belong to the favorites bar, which skips locked
    /// chats: inside the unlocked locked folder they open nothing and the
    /// list stays where it is.
    #[test]
    fn numbered_shortcuts_skip_locked_chats_inside_the_locked_folder() {
        let (_directory, mut app, _ids, ctx) = rail_app(24);
        for chat in &mut app.chats {
            chat.locked = true;
        }
        app.page = Page::Chats;
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.actions
            .push(Action::UnlockLockedFolder("fixture-code".into()));
        app.background_frame(&ctx);
        assert!(app.locked_folder_open());
        // A locked favorite never reaches the bar.
        app.chats[8].favorite = true;
        app.chats[8].backend_account = Some("wa-personal".into());

        let mut frame = |key: Option<Key>| {
            let mut offset = 0.0;
            let mut height = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    events: key
                        .into_iter()
                        .map(|key| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        })
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    super::super::keys::handle(&mut app, ui.ctx());
                    let scroll_id = ui.make_persistent_id(egui::IdSalt::new("locked-chats"));
                    height = ui.available_height();
                    list(&mut app, ui);
                    offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .expect("locked-chats scroll state")
                        .offset
                        .y;
                },
            );
            output.textures_delta.clear();
            (offset, height)
        };
        assert_eq!(frame(None).0, 0.0);
        let (offset, _) = frame(Some(Key::Num9));
        assert_eq!(offset, 0.0, "no numbered shortcut scrolls the locked list");
        let (offset, _) = frame(Some(Key::Num1));
        assert_eq!(offset, 0.0, "the locked list stays where it is");
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenChat(_))),
            "locked chats keep their numbers off the favorites bar: {:?}",
            app.actions
        );
        assert!(app.scroll_chat_into_view.is_none());
    }

    #[test]
    fn the_collapsed_list_is_narrow_and_opens_a_chat_on_click() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let mut first = Chat::new("491700000001@s.whatsapp.net".into(), "Alice".into());
        first.unread = 3;
        first.last_activity = 20;
        let second = Chat::new("491700000002@s.whatsapp.net".into(), "Bob".into());
        app.chats.push(first.clone());
        app.chats.push(second.clone());
        app.open_chat = Some(second.id.clone());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| super::super::rail::show(&mut app, ui, true),
            );
            output.textures_delta.clear();
        };
        frame(vec![]);
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(compact_chat_id(&first.id)))
            .expect("the rail draws every visible chat");
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(&second.id)))
                .is_some(),
            "the rail draws the second chat too"
        );
        assert!(
            (rect.width() - compact_width(&ctx)).abs() < 1.0,
            "an avatar cell is one panel wide, not {} points",
            rect.width()
        );
        let position = rect.center();
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(
            app.actions.contains(&Action::OpenChat(first.id.clone())),
            "clicking an avatar opens that chat: {:?}",
            app.actions
        );
    }

    /// No horizontal scrollbars in a narrow rail. The rail's four category
    /// filters (Private, Favorites, Groups, Channels) stack one icon per row
    /// down the rail's column with the label dots below, keeping every
    /// filter/label reachable instead of the old scrolling UI. The labeled
    /// All/Unread/Reply later/Read later facets live in the row above the
    /// chat list; the demo's geometry test covers that row.
    #[test]
    fn narrow_chip_rows_stack_within_the_viewport_without_horizontal_scrolling() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        app.labels = (0..8)
            .map(|index| crate::model::Label {
                id: index.to_string(),
                name: format!("Label {index}"),
                color_hex: "#25d366".into(),
                created_at: 0,
            })
            .collect();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(220.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| filter_chips(&mut app, ui),
            );
            output.textures_delta.clear();
            output.shapes
        };
        // The rail's filters after the primary facets moved above the list.
        const RAIL: [ChatFilter; 4] = [
            ChatFilter::Private,
            ChatFilter::Favorites,
            ChatFilter::Groups,
            ChatFilter::Channels,
        ];
        let rects = || {
            ctx.data(|data| {
                RAIL.into_iter()
                    .map(|filter| data.get_temp::<Rect>(filter_chip_id(filter)).unwrap())
                    .chain((0..8).map(|index| {
                        data.get_temp::<Rect>(super::labels::chip_id(&index.to_string()))
                            .unwrap()
                    }))
                    .collect::<Vec<_>>()
            })
        };
        frame(vec![]);
        let before = rects();
        assert_eq!(before.len(), RAIL.len() + 8);
        for rect in &before {
            assert!(
                rect.is_positive() && rect.left() >= 0.0 && rect.right() <= 220.0,
                "chip outside viewport: {rect:?}"
            );
        }
        // One icon per row: every filter starts below the one before, the
        // whole column shares one center line, and the label dots follow.
        let column_x = before[0].center().x;
        for pair in before[..RAIL.len()].windows(2) {
            assert!(
                pair[1].top() >= pair[0].bottom() - 1.0,
                "filters stack one per row: {pair:?}"
            );
        }
        for rect in &before {
            assert!(
                (rect.center().x - column_x).abs() < 1.0,
                "one centered column: {rect:?}"
            );
        }
        assert!(before.last().unwrap().top() > before[RAIL.len()].top());
        for pointer in [before[0].center(), before[RAIL.len()].center()] {
            for _ in 0..5 {
                frame(vec![egui::Event::PointerMoved(pointer)]);
            }
            frame(vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -60.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ]);
            assert_eq!(
                before,
                rects(),
                "hover/wheel must not hide chips horizontally"
            );
        }
    }

    /// An app with `count` chats, newest first, and a context to draw it in.
    fn rail_app(count: usize) -> (tempfile::TempDir, App, Vec<String>, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let mut ids = Vec::new();
        for index in 0..count {
            let id = format!("49170000{index:04}@s.whatsapp.net");
            let mut chat = Chat::new(id.clone(), format!("Chat {index:03}"));
            chat.last_activity = 10_000 - index as i64;
            app.chats.push(chat);
            ids.push(id);
        }
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (directory, app, ids, ctx)
    }

    fn rail_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::keys::handle(app, ui.ctx());
                super::super::rail::show(app, ui, true);
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn active_avatar_ring_fits_cell_and_leaves_badges_on_top() {
        let (_directory, _app, _ids, ctx) = rail_app(0);
        let palette = Palette::dark();
        let avatar = Rect::from_center_size(pos2(26.0, 26.0), Vec2::splat(COMPACT_AVATAR));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            paint_active_avatar_ring(ui, &palette, avatar);
            widgets::unread_indicator(ui, &palette, compact_badge_center(avatar), 99, false, false);
        });
        output.textures_delta.clear();
        let circles: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Circle(circle) = &shape.shape {
                    Some(circle)
                } else {
                    None
                }
            })
            .collect();
        let accent = circles
            .iter()
            .find(|circle| circle.stroke.color == palette.accent)
            .expect("active ring");
        assert_eq!(accent.radius, COMPACT_AVATAR / 2.0 + 3.0);
        assert_eq!(accent.stroke.width, 2.5);
        assert!(accent.radius + 2.5 < COMPACT_CELL / 2.0);
        let ring_index = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::Shape::Circle(circle) if circle.stroke.color == palette.accent)
            })
            .unwrap();
        assert!(
            ring_index + 1 < output.shapes.len(),
            "badge paints over the ring"
        );
    }

    #[test]
    fn all_is_icon_only_while_the_other_three_keep_positive_counts() {
        assert_eq!(queue_badge_count(ChatFilter::All, Some(123)), None);
        for filter in [
            ChatFilter::Unread,
            ChatFilter::ReplyLater,
            ChatFilter::ReadLater,
        ] {
            assert_eq!(queue_badge_count(filter, Some(123)), Some(123));
            assert_eq!(queue_badge_count(filter, Some(0)), None);
            assert_eq!(queue_badge_count(filter, None), None);
        }
    }

    #[test]
    fn queue_count_badge_caps_large_numbers_inside_narrow_segments() {
        let (_directory, _app, _ids, ctx) = rail_app(0);
        let palette = Palette::dark();
        let cell = Rect::from_min_size(pos2(10.0, 10.0), vec2(32.0, 36.0));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            paint_queue_count(ui, &palette, cell, 1000)
        });
        output.textures_delta.clear();
        let label = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text)
                } else {
                    None
                }
            })
            .expect("count label");
        assert_eq!(label.galley.text(), "99+");
        for shape in &output.shapes {
            if let egui::Shape::Rect(badge) = &shape.shape {
                assert!(cell.contains_rect(badge.rect));
            }
        }
    }

    #[test]
    fn queue_count_preference_off_restores_icon_only_segments() {
        let (root, mut app, _, ctx) = rail_app(2);
        let _ = root;
        app.chats[0].unread = 4;
        let mut draw = |enabled| {
            app.settings.beeper.show_queue_counts = enabled;
            let mut output = ctx.run_ui(Default::default(), |ui| facet_row(&mut app, ui));
            output.textures_delta.clear();
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(draw(false).is_empty());
        let counts = draw(true);
        assert!(!counts.contains(&"2".to_owned()), "All stays icon-only");
        assert!(counts.contains(&"1".to_owned()));
        assert!(!counts.contains(&"0".to_owned()));
    }

    #[test]
    fn the_collapsed_list_lays_out_only_the_avatars_on_screen() {
        let (_directory, mut app, ids, ctx) = rail_app(2_000);
        rail_frame(&mut app, &ctx, vec![]);
        let drawn = ids
            .iter()
            .filter(|id| ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(id)).is_some()))
            .count();
        assert!(drawn > 0, "the first avatars are drawn");
        assert!(
            drawn < 20,
            "a 400-point window has room for a few avatars, yet {drawn} were laid out"
        );
    }

    /// A backlog reply keeps the collapsed sidebar scrolled down even when
    /// normal activity ordering moves the recipient to the top.
    #[test]
    fn sending_far_down_leaves_the_collapsed_list_scrolled_down() {
        let (_directory, mut app, ids, ctx) = rail_app(40);
        let (backend, mut commands) = crate::backend::Backend::recording();
        app.backend = backend;
        app.open_chat = Some(ids[30].clone());
        app.scroll_chat_into_view = Some(ids[30].clone());
        rail_frame(&mut app, &ctx, vec![]);
        rail_frame(&mut app, &ctx, vec![]);
        let offset = || {
            ctx.data(|data| data.get_temp::<f32>(list_offset_id().with("compact")))
                .expect("the collapsed list was drawn")
        };
        let before = offset();
        assert!(before > COMPACT_CELL, "the rail starts far down");
        app.actions.push(Action::SendText {
            chat: ids[30].clone(),
            text: "Backlog reply fixture".into(),
            quoting: None,
        });
        app.background_frame(&ctx);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                crate::backend::Command::SendText { chat, .. } if chat == ids[30]
            ))
        );
        // Normal activity ordering still moves the replied-to chat up.
        app.chats
            .iter_mut()
            .find(|chat| chat.id == ids[30])
            .unwrap()
            .last_activity = 20_000;
        rail_frame(&mut app, &ctx, vec![]);
        rail_frame(&mut app, &ctx, vec![]);
        assert_eq!(offset(), before, "sending keeps the sidebar offset");
        assert_eq!(app.visible_chats()[0].id, ids[30]);
    }

    #[test]
    fn a_sent_message_scrolls_the_collapsed_list_to_the_top_only_from_near_it() {
        let (_directory, mut app, ids, ctx) = rail_app(40);
        let offset = || {
            ctx.data(|data| data.get_temp::<f32>(list_offset_id().with("compact")))
                .expect("the collapsed list was drawn")
        };
        app.scroll_chat_into_view = Some(ids[39].clone());
        rail_frame(&mut app, &ctx, vec![]);
        rail_frame(&mut app, &ctx, vec![]);
        let scrolled = offset();
        assert!(
            scrolled > COMPACT_CELL,
            "the rail starts scrolled to the end"
        );
        app.scroll_chats_to_top = true;
        rail_frame(&mut app, &ctx, vec![]);
        assert!(!app.scroll_chats_to_top, "the request was consumed");
        assert_eq!(offset(), scrolled, "a reader far down keeps their place");

        scroll_list_to(
            &ctx,
            list_offset_id().with("compact-id"),
            COMPACT_CELL / 2.0,
        );
        rail_frame(&mut app, &ctx, vec![]);
        assert_eq!(offset(), COMPACT_CELL / 2.0);
        app.scroll_chats_to_top = true;
        rail_frame(&mut app, &ctx, vec![]);
        assert_eq!(offset(), 0.0, "the rail is back at the top");
    }

    #[test]
    fn alt_navigation_scrolls_the_collapsed_list_too() {
        let (_directory, mut app, ids, ctx) = rail_app(40);
        app.open_chat = Some(ids[0].clone());
        rail_frame(&mut app, &ctx, vec![]);
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(&ids[39])))
                .is_none(),
            "the last avatar starts off screen"
        );
        let alt_up = egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        rail_frame(&mut app, &ctx, vec![alt_up]);
        assert!(app.actions.contains(&Action::OpenChat(ids[39].clone())));
        assert!(app.scroll_chat_into_view.is_none(), "reveal was consumed");
        rail_frame(&mut app, &ctx, vec![]);
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(compact_chat_id(&ids[39])))
            .expect("the destination avatar is laid out");
        assert!(
            rect.bottom() <= 400.0 + 0.5,
            "the destination avatar is on screen: {rect:?}"
        );
    }

    #[test]
    fn the_secret_code_shows_only_the_way_into_the_locked_folder() {
        let (_directory, mut app, ids, ctx) = rail_app(3);
        app.chats[0].locked = true;
        // A code that also matches an ordinary chat's number.
        app.settings.set_chat_lock_code(Some("0001"));
        app.search = "0001".into();
        rail_frame(&mut app, &ctx, vec![]);
        for id in &ids {
            assert!(
                ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(id)))
                    .is_none(),
                "no chat shows while the code is typed"
            );
        }
        let entry = ctx
            .data(|data| data.get_temp::<Rect>(compact_locked_id()))
            .expect("the locked entry is drawn");
        let position = entry.center();
        for pressed in [true, false] {
            rail_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.actions.contains(&Action::OpenLockedFolder));
    }

    #[test]
    fn collapsing_and_expanding_keeps_the_saved_list_width() {
        let (_directory, mut app, _ids, ctx) = rail_app(3);
        app.settings.sidebar_width = 400.0;
        let frame = |app: &mut App, collapsed: bool| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    if collapsed {
                        super::super::rail::show(app, ui, true);
                    } else {
                        show(app, ui);
                    }
                },
            );
            output.textures_delta.clear();
        };
        frame(&mut app, false);
        frame(&mut app, true);
        frame(&mut app, false);
        assert!(
            (app.settings.sidebar_width - 400.0).abs() < 1.0,
            "the full list came back {} points wide",
            app.settings.sidebar_width
        );
        assert!(!app.actions.contains(&Action::SettingsChanged));
    }

    #[test]
    fn the_widest_badge_stays_inside_the_collapsed_list() {
        let (_directory, _app, _ids, ctx) = rail_app(0);
        let rail = Rect::from_min_size(
            egui::Pos2::ZERO,
            vec2(super::super::rail::COMPACT, COMPACT_CELL),
        );
        let avatar = Rect::from_center_size(rail.center(), Vec2::splat(COMPACT_AVATAR));
        let mut width = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let palette = Palette::dark();
            width = widgets::badge(ui, &palette, compact_badge_center(avatar), 120, true);
        });
        output.textures_delta.clear();
        let center = compact_badge_center(avatar);
        assert!(center.x + width / 2.0 <= rail.right(), "{width} too wide");
        assert!(center.y - 10.0 >= rail.top(), "the badge fits its cell");
    }

    #[test]
    fn a_pinned_chat_keeps_its_row_in_the_list() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = |id: &str, name: &str| {
            let mut chat = Chat::new(id.into(), name.into());
            chat.backend_account = Some("wa-personal".into());
            chat.last_activity = 20;
            chat
        };
        let pinned = {
            let mut chat = chat("491700000100@s.whatsapp.net", "Pinned");
            chat.pinned = true;
            chat
        };
        let ordinary = chat("491700000101@s.whatsapp.net", "Ordinary");
        app.chats.push(pinned.clone());
        app.chats.push(ordinary.clone());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 600.0))),
                ..Default::default()
            },
            |ui| show(&mut app, ui),
        );
        output.textures_delta.clear();
        // The list keeps the pinned chat's full row; its favorite chip is
        // the rail's business, never a replacement for the row.
        let row = ctx
            .data(|data| data.get_temp::<Rect>(chat_row_id(&pinned.id)))
            .expect("a pinned chat keeps its ordinary row in the list");
        assert!(
            row.height() > 20.0,
            "the pinned chat's list entry is a full row, not a chip: {row:?}"
        );
        ctx.data(|data| data.get_temp::<Rect>(chat_row_id(&ordinary.id)))
            .expect("an unpinned chat keeps its row as before");
    }

    #[test]
    fn linked_favorite_uses_unread_from_other_unlocked_members() {
        use crate::merged::{Member, Person};
        let directory = tempfile::tempdir().expect("temp dir");
        let (mut app, _) = App::headless(AppDirs::under(directory.path()), Settings::default());
        let mut favorite = Chat::new("ada-wa".into(), "Ada".into());
        favorite.backend_account = Some("wa".into());
        favorite.backend_origin = Some("http://localhost:23374".into());
        favorite.favorite = true;
        let mut other = favorite.clone();
        other.id = "ada-signal".into();
        other.backend_account = Some("signal".into());
        other.favorite = false;
        other.unread = 3;
        other.marked_unread = true;
        app.settings.beeper.people.push(Person {
            name: "Ada".into(),
            members: vec![Member::of(&favorite).unwrap(), Member::of(&other).unwrap()],
            primary: None,
        });
        app.chats = vec![favorite, other];
        let projected = favorite_bar_chats(&app);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].unread, 3);
        assert!(projected[0].marked_unread);
        app.chats[1].locked = true;
        let projected = favorite_bar_chats(&app);
        assert_eq!(projected[0].unread, 0, "locked member unread must not leak");
        assert!(!projected[0].marked_unread);
    }

    #[test]
    fn the_favorites_bar_draws_avatar_chips_without_rows() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let favorite = |n: u32, name: &str| {
            let mut chat = Chat::new(format!("4917000001{n:02}@s.whatsapp.net"), name.into());
            chat.favorite = true;
            chat.favorite_position = n;
            chat.backend_account = Some("wa-personal".into());
            chat.last_activity = 20;
            chat
        };
        let counted = favorite(0, "Counted");
        let dotted = {
            let mut chat = favorite(1, "Dotted");
            chat.marked_unread = true;
            chat
        };
        let quiet = favorite(2, "Quiet");
        let wrapped_a = favorite(3, "Wrapped A");
        let wrapped_b = favorite(4, "Wrapped B");
        let pinned_favorite = {
            let mut chat = favorite(5, "Pinned favorite");
            chat.pinned = true;
            chat
        };
        let mut counted = counted;
        counted.unread = 4;
        for chat in [
            counted.clone(),
            dotted.clone(),
            quiet.clone(),
            wrapped_a.clone(),
            wrapped_b.clone(),
            pinned_favorite.clone(),
        ] {
            app.chats.push(chat);
        }
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| super::super::rail::show(&mut app, ui, false),
            );
            output.textures_delta.clear();
        };
        frame(vec![]);
        let chip = |chat: &Chat| {
            ctx.data(|data| data.get_temp::<Rect>(favorite_chip_id(&chat.id)))
                .expect("the bar draws every favorite")
        };
        let first = chip(&counted);
        for chat in [&dotted, &quiet, &wrapped_a, &wrapped_b] {
            let rect = chip(chat);
            assert!(
                (rect.width() - FAVORITE_CELL).abs() < 1.0
                    && (rect.height() - FAVORITE_CELL).abs() < 1.0,
                "a favorite is one avatar chip of {FAVORITE_CELL} points, not a chat row"
            );
        }
        // Five chips at 44 points plus spacing do not fit one 216-point
        // rail row: the fifth wraps below the first.
        let last = chip(&wrapped_b);
        assert!(
            last.top() > first.bottom() - 1.0,
            "the fifth chip wraps to a second row"
        );
        // A chat that is also pinned is one favorite among many now: its
        // chip sits in the bar by its favorite position, never in a
        // section of its own.
        let pinned_chip = chip(&pinned_favorite);
        assert!(
            pinned_chip.top() >= last.bottom() - 1.0,
            "the pinned favorite's chip wraps with the bar: {pinned_chip:?} vs chip {last:?}"
        );
        // Badges: a count for the unread chat, a dot for the marked one,
        // nothing for the read one, and every badge inside the rail panel.
        let rail_right = super::super::rail::COMPACT;
        let badge =
            |chat: &Chat| ctx.data(|data| data.get_temp::<Rect>(favorite_badge_id(&chat.id)));
        let counted_badge = badge(&counted).expect("an unread favorite carries its count");
        assert!(
            counted_badge.width() > 8.0,
            "a count badge is wider than a bare dot"
        );
        assert!(
            badge(&dotted).is_some(),
            "a chat only marked unread keeps its dot"
        );
        assert!(badge(&quiet).is_none(), "a read favorite shows no badge");
        for chat in [&counted, &dotted] {
            let rect = badge(chat).expect("the badge is drawn");
            assert!(
                rect.right() <= rail_right + 1.0,
                "the badge stays inside the rail"
            );
        }
        let position = first.center();
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                },
            ]);
        }
        assert!(
            app.actions.contains(&Action::OpenChat(counted.id.clone())),
            "clicking a favorite chip opens the chat"
        );
    }
}
