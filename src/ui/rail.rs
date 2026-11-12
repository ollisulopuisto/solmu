//! The left rail: one narrow vertical column of icons. Destinations (Chats
//! Starred, Archived and Locked chats while there are any;
//! Settings and our own avatar, the account switcher) at the top, the quick
//! filters, queues, labels and the favorite and pinned chats as icon and
//! avatar jumps down the middle, and the Beeper networks as one plate per
//! row at the bottom. Every icon carries its unread badge; every name and
//! shortcut lives on the tooltip and accessibility label. It replaces the
//! old network-selector rail, so the chat-list column keeps only its search
//! field and chats. While the chat list is hidden, the chats' avatars fill
//! the rail instead, so every chat is still one click away.

use egui::{Frame, Margin, Rect, Sense, Vec2, vec2};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Page};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::widgets;

/// Width of the rail: one column of icons and avatars.
pub const COMPACT: f32 = 52.0;
/// Height of one destination at the top: the list header's title and
/// search field sits below Chats.
const CELL: f32 = 48.0;
/// The lit square behind a destination's icon.
const PLATE: f32 = 36.0;
/// A destination's icon.
const ICON: f32 = 20.0;
/// Our avatar at the foot of the rail.
const AVATAR: f32 = 32.0;
/// Space above the first destination, as the list header's own margin, and
/// between the bottom group's rows, the avatar and the window's edge.
const GAP: f32 = 8.0;
/// Tighter rhythm inside the bottom group's wrapped plate rows, so the
/// networks and destinations read as one compact cluster.
const BOTTOM_GAP: f32 = GAP / 2.0;
/// How far below the window's top the macOS traffic lights end, centred as
/// they are in the 60-point title bar, with a little room under them.
const LIGHTS_CLEARANCE: f32 = theme::TOP_BAR_HEIGHT / 2.0 + 12.0;
/// The least room the chats' avatars keep while the list is hidden.
const MIN_AVATARS: f32 = 64.0;
/// Width of the hairline between the destinations and the avatars.
const DIVIDER: f32 = 24.0;

/// How many bottom plates fit in one row at this rail width, and how many
/// rows `count` plates wrap into. The compact rail stacks one per row; the
/// wide rail groups several beside each other.
fn bottom_wrap(width: f32, count: usize) -> (usize, usize) {
    let per_row = (((width + BOTTOM_GAP) / (PLATE + BOTTOM_GAP)).floor() as usize).max(1);
    (per_row, count.div_ceil(per_row))
}

/// The center of each plate in one bottom row, the row centered in the rail.
fn bottom_row_centers(cell: Rect, slots: usize) -> impl Iterator<Item = egui::Pos2> {
    let row_width = slots as f32 * (PLATE + BOTTOM_GAP) - BOTTOM_GAP;
    let start = cell.min.x + (cell.width() - row_width) / 2.0 + PLATE / 2.0;
    let y = cell.center().y;
    (0..slots).map(move |slot| egui::pos2(start + slot as f32 * (PLATE + BOTTOM_GAP), y))
}

/// Where the rail was laid out, for layout tests.
pub(crate) fn rect_id() -> egui::Id {
    egui::Id::new("nav-rail")
}

/// Stable id of a destination's button, for tests and the demo.
pub fn destination_id(destination: Destination) -> egui::Id {
    egui::Id::new(("nav-rail-destination", destination as u8))
}

/// Stable id of a network's plate, for tests and the demo.
pub fn network_plate_id(network: &str) -> egui::Id {
    egui::Id::new(("nav-rail-network", network))
}

/// How far the macOS traffic lights reach past the rail: what a header
/// beside the rail leaves free at its left. Zero elsewhere and in full
/// screen. The caption buttons Solmu draws on the left (GNOME) are not
/// counted here: headers measure their room from `ui::caption_room`.
pub fn lights_overhang(ctx: &egui::Context) -> f32 {
    if !theme::macos_chrome(ctx) {
        return 0.0;
    }
    let width = ctx
        .data(|data| data.get_temp::<f32>(width_id()))
        .unwrap_or(COMPACT);
    (theme::traffic_light_inset(ctx) - width).max(0.0)
}

/// This frame's rail width, written by the layout before the headers draw,
/// for the traffic-light clearance above.
pub(crate) fn width_id() -> egui::Id {
    egui::Id::new("nav-rail-width")
}

/// A place the rail leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Chats,
    Starred,
    Saved,
    Archived,
    Locked,
    Settings,
}

impl Destination {
    /// Whether the window shows this destination now: its icon is lit.
    pub fn lit(self, app: &App) -> bool {
        let chats = app.page == Page::Chats && !app.show_starred && !app.show_saved;
        match self {
            Self::Chats => {
                chats
                    && !app.show_archived
                    && !app.locked_folder_open()
                    && !app.beeper_scope_is_active()
            }
            Self::Starred => app.page == Page::Chats && app.show_starred,
            Self::Saved => app.page == Page::Chats && app.show_saved,
            Self::Archived => chats && app.show_archived,
            Self::Locked => chats && app.locked_folder_open(),
            Self::Settings => matches!(app.page, Page::Settings | Page::Wallpaper),
        }
    }

    /// Whether the rail offers this destination: Archived while chats are
    /// archived and Locked while chats are locked, as their chips used to.
    pub fn offered(self, app: &App) -> bool {
        match self {
            Self::Archived => app.archived_count() > 0 || app.show_archived,
            Self::Locked => app.locked_count() > 0 || app.locked_folder_open(),
            _ => true,
        }
    }

    /// The actions that bring the window here. None of the destinations is
    /// a toggle but Settings, whose button closes it again as Escape does.
    pub fn actions(self, app: &App) -> Vec<Action> {
        let mut actions = Vec::new();
        let to_chats = |actions: &mut Vec<Action>| {
            if app.page != Page::Chats {
                actions.push(Action::Open(Page::Chats));
            }
            if app.show_starred {
                actions.push(Action::ToggleStarred);
            }
            if app.show_saved {
                actions.push(Action::ToggleSaved);
            }
        };
        match self {
            Self::Chats => {
                to_chats(&mut actions);
                if app.show_archived {
                    actions.push(Action::ShowArchived(false));
                }
                if app.locked_folder {
                    actions.push(Action::CloseLockedFolder);
                }
            }
            Self::Starred => {
                if app.page != Page::Chats {
                    actions.push(Action::Open(Page::Chats));
                }
                if !app.show_starred {
                    actions.push(Action::ToggleStarred);
                }
            }
            Self::Saved => {
                if app.page != Page::Chats {
                    actions.push(Action::Open(Page::Chats));
                }
                if !app.show_saved {
                    actions.push(Action::ToggleSaved);
                }
            }
            Self::Archived => {
                if !self.lit(app) {
                    to_chats(&mut actions);
                    // The archive opens in the list column, as it did.
                    if !app.sidebar_visible {
                        actions.push(Action::ToggleSidebar);
                    }
                    actions.push(Action::ShowArchived(true));
                }
            }
            Self::Locked => {
                if !self.lit(app) {
                    to_chats(&mut actions);
                    actions.push(Action::OpenLockedFolder);
                }
            }
            Self::Settings => actions.push(Action::ToggleSettings),
        }
        actions
    }
}

/// The rail as a fixed panel on the left of the chat list.
/// `hidden` says the chat list is hidden: the rail then offers to show
/// it again and stands beside the conversation.
pub fn show(app: &mut App, ui: &mut egui::Ui, hidden: bool) {
    let palette = app.palette;
    let width = COMPACT;
    // Its own panel id: sharing the list's would hand that resizable panel
    // this width, and it would save it as the list's width.
    let panel = egui::Panel::left("chat-rail")
        .resizable(false)
        .exact_size(width)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.rail()).inner_margin(Margin::ZERO));
    let response = panel.show(ui, |ui| {
        // The avatars keep the spacing they always had; the rest is tight.
        let row_gap = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        if theme::traffic_light_inset(ui.ctx()) > 0.0 {
            // The traffic lights, or the caption buttons Solmu draws where
            // the desktop puts them on the left (GNOME's round ones end 42
            // points down), take the rail's top; it starts under them.
            let (strip, _) = ui.allocate_exact_size(vec2(width, LIGHTS_CLEARANCE), Sense::hover());
            super::titlebar_drag(ui, strip);
        } else {
            ui.add_space(GAP);
        }
        let bottom: Vec<Destination> = [
            Destination::Starred,
            Destination::Saved,
            Destination::Archived,
            Destination::Locked,
            Destination::Settings,
        ]
        .into_iter()
        .filter(|destination| destination.offered(app))
        .collect();
        let networks = if hidden {
            app.beeper_browse_networks()
        } else {
            Vec::new()
        };
        let top_height = 2.0 * CELL;
        let (_, network_rows) = bottom_wrap(width, networks.len());
        let (_, bottom_rows) = bottom_wrap(width, bottom.len());
        let foot =
            (network_rows + bottom_rows) as f32 * (PLATE + BOTTOM_GAP) + BOTTOM_GAP + AVATAR + GAP;
        let minimum = top_height
            + foot
            + if hidden {
                CELL + GAP + MIN_AVATARS
            } else {
                0.0
            };
        let height = ui.available_height().max(minimum);
        // Demo captures can ask for the rail already scrolled to its end,
        // as after a wheel scroll, to prove the lower sections stay
        // reachable at this height.
        #[cfg(feature = "demo")]
        if crate::demo::rail_bottom() {
            let id = ui.make_persistent_id(egui::IdSalt::new("navigation-rail"));
            let mut state = egui::scroll_area::State::load(ui.ctx(), id).unwrap_or_default();
            // Seed a finite overshoot (f32::MAX overflows egui's rect
            // math) while the last laid-out content height still leaves
            // more than a viewport below the offset: egui clamps the seed
            // to the real maximum at the end of the frame, and a
            // still-growing content height re-seeds until the capture
            // settles at the true end.
            if crate::demo::rail_content_height() - state.offset.y > height + 16.0 {
                state.offset.y = 1_000_000.0;
                state.store(ui.ctx(), id);
            }
        }
        egui::ScrollArea::vertical()
            .id_salt("navigation-rail")
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(ui, |ui| {
                // Keep the usual spacing in a tall window. In a short one
                // the content keeps its minimum height and scrolls instead
                // of leaving the bottom buttons beyond the window.
                ui.set_height(height);
                destination_button(app, ui, &palette, Destination::Chats, CELL);
                if hidden {
                    // A cell of its own under Chats: nothing above it moves
                    // when the list comes and goes.
                    show_list_button(app, ui, &palette);
                    ui.add_space(GAP / 2.0);
                    let line = Rect::from_center_size(
                        ui.cursor().min + vec2(width / 2.0, 0.0),
                        vec2(DIVIDER, 1.0),
                    );
                    ui.painter().rect_filled(line, 0.0, palette.outline);
                    ui.add_space(GAP / 2.0);
                    let middle = (ui.available_height() - foot).max(MIN_AVATARS);
                    ui.allocate_ui_with_layout(
                        vec2(width, middle),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = row_gap;
                            super::chats::compact_list(app, ui);
                        },
                    );
                }
                if !hidden {
                    // Quick filters, queues, labels and the favorite chats
                    // live here, so the chat-list column keeps only its
                    // search field and chats.
                    ui.spacing_mut().item_spacing.y = row_gap;
                    if hidden {
                        super::chats::filter_chips(app, ui);
                    }
                    super::chats::favorites_row(app, ui);
                    ui.spacing_mut().item_spacing.y = 0.0;
                }
                let rest = (ui.available_height() - foot).max(0.0);
                ui.add_space(rest);
                // Beeper networks sit beside the other bottom destinations,
                // wrapped into compact rows so the cluster stays short: a
                // plate filters to its network, a right-click its accounts.
                let (per_row, _) = bottom_wrap(width, networks.len().max(bottom.len()));
                ui.add_space(BOTTOM_GAP);
                let mut index = 0;
                for chunk in networks.chunks(per_row) {
                    let (cell, _) = ui.allocate_exact_size(vec2(width, PLATE), Sense::hover());
                    for (center, network) in bottom_row_centers(cell, chunk.len()).zip(chunk) {
                        network_plate(app, ui, &palette, network, index, center);
                        index += 1;
                    }
                    ui.add_space(BOTTOM_GAP);
                }
                for chunk in bottom.chunks(per_row) {
                    let (cell, _) = ui.allocate_exact_size(vec2(width, PLATE), Sense::hover());
                    for (center, destination) in bottom_row_centers(cell, chunk.len()).zip(chunk) {
                        let plate = Rect::from_center_size(center, vec2(PLATE, PLATE));
                        destination_plate(app, ui, &palette, *destination, plate);
                    }
                    ui.add_space(BOTTOM_GAP);
                }
                // The account button keeps the compact column at the rail's
                // foot, so widening the sidebar beside the list moves nothing.
                ui.allocate_ui_with_layout(
                    vec2(COMPACT, AVATAR),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        super::accounts::avatar_button(app, ui, AVATAR).tab_stop(Stop::Profile);
                    },
                );
                // Let next frame's rail-bottom seed judge convergence from
                // the real laid-out height (offset-independent).
                #[cfg(feature = "demo")]
                if crate::demo::rail_bottom() {
                    crate::demo::set_rail_content_height(ui.min_rect().height());
                }
            });
    });
    let rect = response.response.rect;
    ui.ctx().data_mut(|data| data.insert_temp(rect_id(), rect));
    // No line between the rail and the list, whose shades tell them apart;
    // beside the conversation the rail ends as the list does.
    if hidden {
        widgets::paint_edge_beside(ui, &palette, rect);
    }
}

/// Text badges are deliberately network initials, not invented vendor logos.
/// Every network is directly reachable; accounts expand under the active one.
fn show_list_button(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), CELL),
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            if theme::icon_button(
                ui,
                Icon::PanelLeft,
                18.0,
                palette.secondary,
                palette.text,
                &super::keys::label("Show the chat list (Ctrl+B)"),
            )
            .tab_stop(Stop::Sidebar)
            .clicked()
            {
                app.actions.push(Action::ToggleSidebar);
            }
        },
    );
}

/// One destination: its icon on a rounded plate, lit as the selected chat
/// row is while the window shows it, with its name and shortcut in the
/// tooltip, in a cell `height` points tall.
/// One Beeper network as a bottom plate: click filters the chats to that
/// network (again clears), right-click lists its accounts. The plate
/// lights while its network scope is active.
fn network_plate(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    network: &str,
    index: usize,
    center: egui::Pos2,
) {
    let lit = app.beeper_network_filter.as_deref() == Some(network);
    let name: &str = if network.is_empty() {
        "Network not supplied"
    } else {
        network
    };
    let accounts = app.beeper_network_accounts(network);
    let mut label = if index < 10 {
        format!(
            "{name} ({})",
            super::keys::label(&format!("Alt+{}", (index + 1) % 10))
        )
    } else {
        name.to_owned()
    };
    if !accounts.is_empty() {
        label.push_str("\nRight-click: accounts");
    }
    let plate = Rect::from_center_size(center, Vec2::splat(PLATE));
    let response = ui
        .interact(plate, network_plate_id(network), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            lit,
            label.as_str(),
        )
    });
    if ui.is_rect_visible(plate) {
        let hovered = response.hovered() || response.has_focus();
        if lit {
            widgets::highlight_card(ui, palette, plate, palette.surface_active);
        } else if hovered {
            widgets::highlight_card(ui, palette, plate, palette.surface_hover);
        }
        let tint = if lit || hovered {
            palette.text
        } else {
            palette.secondary
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        theme::paint_icon(ui, theme::network_icon(network), plate, ICON * scale, tint);
    }
    let response = response.on_hover_text(label);
    if !accounts.is_empty() {
        response.context_menu(|ui| {
            for (key, account) in &accounts {
                let selected = app.beeper_account_filter.as_ref() == Some(key);
                let item = if selected {
                    format!("{account} (showing - pick to show the whole network)")
                } else {
                    account.clone()
                };
                if widgets::menu_item(ui, palette, None, item.as_str()) {
                    let account = if selected { None } else { Some(key.clone()) };
                    app.select_beeper_scope(Some(network.to_owned()), account);
                    ui.close();
                }
            }
        });
    }
    let response = response.tab_stop(Stop::Network(index.min(u8::MAX as usize) as u8));
    if response.clicked() {
        // A second click on the active network shows every network again.
        let next = if lit { None } else { Some(network.to_owned()) };
        app.select_beeper_scope(next, None);
    }
}

fn destination_button(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    destination: Destination,
    height: f32,
) {
    let (cell, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    // The plate holds the compact rail's column whichever width the rail
    // has, so the list coming and going moves nothing.
    let plate = Rect::from_center_size(
        egui::pos2(cell.min.x + COMPACT / 2.0, cell.center().y),
        Vec2::splat(PLATE),
    );
    destination_plate(app, ui, palette, destination, plate);
}

/// One destination's plate at a given center: the top destinations call it
/// through destination_button's full-width cell, the bottom group through
/// its wrapped rows.
fn destination_plate(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    destination: Destination,
    plate: Rect,
) {
    let lit = destination.lit(app);
    let mut badge = 0;
    let (icon, stop, label) = match destination {
        Destination::Chats => (
            Icon::MessageCircle,
            Stop::Chats,
            format!(
                "{} ({})",
                gettext(app.locale, "Chats"),
                super::keys::label(
                    super::keys::shortcut("rail-chats")
                        .label
                        .unwrap_or_default(),
                )
            ),
        ),
        Destination::Starred => (
            Icon::Star,
            Stop::Starred,
            gettext(app.locale, "Starred messages").into_owned(),
        ),
        Destination::Saved => (
            Icon::Bookmark,
            Stop::Saved,
            gettext(app.locale, "Saved messages").into_owned(),
        ),
        Destination::Archived => {
            // The count the Archived chip carried: archived chats unread.
            badge = app.archived_unread() as u32;
            (
                Icon::Archive,
                Stop::Archived,
                gettext(app.locale, "Archived").into_owned(),
            )
        }
        Destination::Locked => (
            Icon::Lock,
            Stop::Locked,
            gettext(app.locale, "Locked").into_owned(),
        ),
        // Same as before the rail: the label says what the click does now.
        Destination::Settings => (
            Icon::Settings,
            Stop::Settings,
            super::keys::label(if lit {
                "Close settings (Ctrl+,)"
            } else {
                "Settings (Ctrl+,)"
            }),
        ),
    };
    let response = ui
        .interact(plate, destination_id(destination), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    let spoken = if badge > 0 {
        format!("{label}, {badge}")
    } else {
        label.clone()
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            lit,
            spoken.as_str(),
        )
    });
    if ui.is_rect_visible(plate) {
        let hovered = response.hovered() || response.has_focus();
        if lit {
            widgets::highlight_card(ui, palette, plate, palette.surface_active);
        } else if hovered {
            widgets::highlight_card(ui, palette, plate, palette.surface_hover);
        }
        let tint = if lit || hovered {
            palette.text
        } else {
            palette.secondary
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        theme::paint_icon(ui, icon, plate, ICON * scale, tint);
        if badge > 0 {
            widgets::unread_indicator(
                ui,
                palette,
                plate.right_top() + vec2(-3.0, 5.0),
                badge,
                false,
                false,
            );
        }
    }
    let response = response.on_hover_text(label).tab_stop(stop);
    if response.clicked() {
        if destination == Destination::Chats {
            app.select_beeper_scope(None, None);
        }
        let actions = destination.actions(app);
        app.actions.extend(actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        (root, app)
    }

    #[test]
    fn one_destination_is_lit_at_a_time() {
        let (_root, mut app) = app();
        let lit = |app: &App| {
            [
                Destination::Chats,
                Destination::Starred,
                Destination::Archived,
                Destination::Locked,
                Destination::Settings,
            ]
            .into_iter()
            .filter(|destination| destination.lit(app))
            .collect::<Vec<_>>()
        };
        assert_eq!(lit(&app), [Destination::Chats]);
        app.show_archived = true;
        assert_eq!(lit(&app), [Destination::Archived]);
        app.show_archived = false;
        app.show_starred = true;
        assert_eq!(lit(&app), [Destination::Starred]);
        app.page = Page::Wallpaper;
        assert_eq!(lit(&app), [Destination::Settings]);
    }

    #[test]
    fn starred_is_left_through_chats_not_toggled() {
        let (_root, mut app) = app();
        // Away from Chats first: Starred from a Chats page only toggles.
        app.page = Page::Settings;
        assert!(matches!(
            Destination::Starred.actions(&app).as_slice(),
            [Action::Open(Page::Chats), Action::ToggleStarred]
        ));
        app.page = Page::Chats;
        app.show_starred = true;
        assert!(Destination::Starred.actions(&app).is_empty());
        assert!(matches!(
            Destination::Chats.actions(&app).as_slice(),
            [Action::ToggleStarred]
        ));
    }

    /// Archived opens the archive in the list column, bringing a hidden
    /// list back, and Chats leaves it; it is offered only while chats are
    /// archived.
    #[test]
    fn archived_opens_in_the_list_and_chats_leaves_it() {
        let (_root, mut app) = app();
        assert!(!Destination::Archived.offered(&app));
        assert!(!Destination::Locked.offered(&app));
        let mut chat = crate::model::Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        chat.archived = true;
        chat.last_activity = 1;
        app.chats.push(chat);
        assert!(Destination::Archived.offered(&app));
        app.sidebar_visible = false;
        assert!(matches!(
            Destination::Archived.actions(&app).as_slice(),
            [Action::ToggleSidebar, Action::ShowArchived(true)]
        ));
        app.sidebar_visible = true;
        app.show_archived = true;
        assert!(Destination::Archived.actions(&app).is_empty());
        assert!(matches!(
            Destination::Chats.actions(&app).as_slice(),
            [Action::ShowArchived(false)]
        ));
    }
}
