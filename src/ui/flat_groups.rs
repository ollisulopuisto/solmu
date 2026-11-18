//! One-level local list groups. Every alias opens the original chat.
use crate::{
    app::App,
    flat_groups::Member,
    model::{Action, Chat},
};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct State {
    pub name: String,
    /// Popup sizing passes cannot keep text focus; wait for its visible pass.
    focus_name: bool,
    pub assigning: Option<(Member, Vec<u64>)>,
    /// Headings folded shut this session: a group's id, or `None` for Other.
    pub collapsed: HashSet<Option<u64>>,
    /// Recent sections opened past their unread filter this session: a
    /// group's id, or `None` for Other.
    pub show_all: HashSet<Option<u64>>,
    /// Demo/test instrumentation: actual alias widgets built in the last frame.
    #[cfg(any(test, feature = "demo"))]
    pub built_rows: usize,
}

pub fn member(app: &App, chat: &Chat) -> Member {
    Member {
        account: app.account().id.0.clone(),
        origin: chat.backend_origin.clone(),
        bridge_account: chat.backend_account.clone(),
        chat: chat.id.clone(),
    }
}

/// Build once per navigation/draw pass. No per-group full-chat scans.
struct IdentityIndex {
    linked: HashMap<crate::merged::Member, Vec<Member>>,
}
impl IdentityIndex {
    fn new(app: &App) -> Self {
        let available: HashMap<_, _> = app
            .chats
            .iter()
            .filter(|chat| !chat.locked)
            .filter_map(|chat| crate::merged::Member::of(chat).map(|id| (id, member(app, chat))))
            .collect();
        let mut linked = HashMap::new();
        for person in &app.settings.beeper.people {
            let members: Vec<_> = person
                .members
                .iter()
                .filter_map(|id| available.get(id).cloned())
                .collect();
            for id in &person.members {
                // Existing lookup chose the first person containing an identity.
                linked.entry(id.clone()).or_insert_with(|| members.clone());
            }
        }
        Self { linked }
    }
    fn identities(&self, app: &App, chat: &Chat) -> Vec<Member> {
        if !chat.locked
            && let Some(id) = crate::merged::Member::of(chat)
            && let Some(members) = self.linked.get(&id)
        {
            return members.clone();
        }
        vec![member(app, chat)]
    }
}
fn identities(app: &App, chat: &Chat) -> Vec<Member> {
    IdentityIndex::new(app).identities(app, chat)
}
/// Shared ordered membership plan. Exact identities are indexed once, and
/// membership is joined via the index instead of scanning chats per group.
fn membership_rows(app: &App, chats: &[&Chat]) -> Vec<Vec<usize>> {
    let identities = IdentityIndex::new(app);
    let mut aliases: HashMap<Member, Vec<usize>> = HashMap::new();
    for (index, chat) in chats.iter().enumerate() {
        for key in identities.identities(app, chat) {
            aliases.entry(key).or_default().push(index);
        }
    }
    let mut seen = HashSet::new();
    app.settings
        .flat_groups
        .items
        .iter()
        .map(|group| {
            let mut rows: Vec<_> = group
                .members
                .iter()
                .filter_map(|key| aliases.get(key))
                .flatten()
                .copied()
                .collect();
            rows.sort_unstable();
            rows.dedup();
            rows.retain(|index| seen.insert(*index));
            rows
        })
        .collect()
}

fn belongs(group: &crate::flat_groups::Group, keys: &[Member]) -> bool {
    keys.iter().any(|key| group.members.contains(key))
}

/// What the row under a filtered Recent section offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expander {
    /// The section shows only its unread and just-opened rows; open it to
    /// every member.
    ShowAll,
    /// The section was opened to every member; fold it back to unread rows.
    UnreadOnly,
}

/// One section of the grouped list in display order: its member rows, the
/// rows on screen now, and the expander under them if any. Row numbers are
/// indexes into the chats slice the plan was built from.
struct Section {
    /// The group's id; `None` for Other.
    id: Option<u64>,
    name: String,
    /// Every member row, in list order.
    all: Vec<usize>,
    /// The rows drawn and walked by the keyboard right now.
    shown: Vec<usize>,
    /// `shown` holds the unread subset: just-opened rows in it stay dimmed.
    filtering: bool,
    expander: Option<Expander>,
}

/// Recent's unread filter for one group: quiet groups list every chat; a
/// group with something unread - or just opened from it - lists only those
/// rows. The unread test is the folded heading's: counted or marked, and
/// never a muted chat. Other never filters.
fn recent_filtered(app: &App, chats: &[&Chat], indices: &[usize]) -> Vec<usize> {
    let now = crate::util::now();
    indices
        .iter()
        .copied()
        .filter(|index| {
            let chat = chats[*index];
            (chat.looks_unread() && !chat.muted(now)) || app.unread_kept_contains(&chat.id)
        })
        .collect()
}

/// Sections in display order. Other modes keep the saved drag order; Recent
/// sorts sections by their freshest chat - a new message lifts its section
/// to the top - and keeps Other last. `activity` aligns with `chats`.
fn plan_sections(app: &App, chats: &[&Chat], activity: &[i64]) -> Vec<Section> {
    let recent = app.list_mode == crate::model::ListMode::Recent;
    let section = |id: Option<u64>, name: String, all: Vec<usize>| {
        // Other always lists every chat: it is the default bucket, not a
        // curated group, so the ungrouped list never hides behind a row.
        let filtered = if recent && id.is_some() {
            recent_filtered(app, chats, &all)
        } else {
            Vec::new()
        };
        let active = !filtered.is_empty();
        let expanded = app.flat_group_ui.show_all.contains(&id);
        let (shown, filtering, expander) = if !active {
            (all.clone(), false, None)
        } else if expanded {
            (all.clone(), false, Some(Expander::UnreadOnly))
        } else {
            let expander = (filtered.len() < all.len()).then_some(Expander::ShowAll);
            (filtered, true, expander)
        };
        Section {
            id,
            name,
            all,
            shown,
            filtering,
            expander,
        }
    };
    let mut sections: Vec<Section> = app
        .settings
        .flat_groups
        .items
        .iter()
        .zip(membership_rows(app, chats))
        .map(|(group, all)| section(Some(group.id), group.name.clone(), all))
        .collect();
    let grouped: HashSet<usize> = sections
        .iter()
        .flat_map(|section| section.all.iter().copied())
        .collect();
    let other: Vec<usize> = (0..chats.len())
        .filter(|index| !grouped.contains(index))
        .collect();
    if recent {
        // Stable: a tie keeps the saved order. Empty sections sink.
        sections.sort_by_key(|section| {
            std::cmp::Reverse(
                section
                    .all
                    .iter()
                    .map(|index| activity[*index])
                    .max()
                    .unwrap_or(i64::MIN),
            )
        });
    }
    sections.push(section(
        None,
        crate::i18n::gettext(app.locale, "Other").into_owned(),
        other,
    ));
    sections
}
fn draw_row(app: &mut App, ui: &mut egui::Ui, chat: &Chat, dim_kept: bool) {
    #[cfg(any(test, feature = "demo"))]
    {
        app.flat_group_ui.built_rows += 1;
    }
    // A chat opened from a filtered section stays dimmed until the view
    // changes (Action::KeepUnread); it is no longer unread itself.
    if dim_kept && !chat.looks_unread() && app.unread_kept_contains(&chat.id) {
        ui.multiply_opacity(0.55);
    }
    let response = super::chats::row(app, ui, chat);
    ui.ctx().data_mut(|data| {
        data.insert_temp(super::chats::chat_row_id(&chat.id), response.rect);
        data.insert_temp(
            super::chats::chat_row_id(&chat.id).with("widget"),
            response.id,
        );
    });
    if app.scroll_chat_into_view.as_ref() == Some(&chat.id) && ui.is_enabled() {
        response.scroll_to_me(Some(egui::Align::Center));
        app.scroll_chat_into_view = None;
    }
    if response.clicked() && !app.show_archived {
        // A keep from a quiet section would filter it to just this row:
        // send only where the list is filtered - a Recent section showing
        // its unread rows, or the modes that list unread chats throughout.
        if dim_kept
            || app.list_mode != crate::model::ListMode::Recent
            || app.chat_filter == crate::model::ChatFilter::Unread
        {
            app.actions.push(Action::KeepUnread(chat.id.clone()));
        }
    }
}

/// The name of the group a chat lives in, for the tag a flat list shows: the
/// first group holding any of its linked identities, as the sections choose.
pub fn home_name(app: &App, chat: &Chat) -> Option<String> {
    let linked = app
        .settings
        .beeper
        .people
        .iter()
        .any(|person| person.members.iter().any(|m| m.matches(chat)));
    let keys = if linked {
        identities(app, chat)
    } else {
        vec![member(app, chat)]
    };
    app.settings
        .flat_groups
        .items
        .iter()
        .find(|group| belongs(group, &keys))
        .map(|group| group.name.clone())
}

pub fn assign(app: &mut App, chat: &Chat) {
    let target = member(app, chat);
    let targets = identities(app, chat);
    let selected = app
        .settings
        .flat_groups
        .items
        .iter()
        .filter(|g| belongs(g, &targets))
        .map(|g| g.id)
        .collect();
    app.flat_group_ui.assigning = Some((target, selected));
}

/// Puts a chat (with every linked alias) in one group, or in Other for
/// `None`, and queues the settings save.
fn set_home(app: &mut App, chat: &Chat, home: Option<u64>) {
    let target = member(app, chat);
    let targets = identities(app, chat);
    let selected: Vec<u64> = home.into_iter().collect();
    app.settings
        .flat_groups
        .assign_linked(&target, &targets, &selected);
    app.actions.push(Action::SettingsChanged);
}

/// Dropping a chat on the Other heading takes it, and its aliases, out of
/// its group.
fn drop_on_other(groups: &mut crate::flat_groups::Groups, target: &Member, sources: &[Member]) {
    groups.assign_linked(target, sources, &[]);
}

/// The chat menu's "Assign groups" submenu: every group at once, one click to
/// pick (it opens on hover). A chat has one home group, so this replaces the
/// current one; "Other" removes it. Same effect as the assign window.
pub fn assign_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let locale = app.locale;
    let targets = identities(app, chat);
    let current = app
        .settings
        .flat_groups
        .items
        .iter()
        .find(|group| belongs(group, &targets))
        .map(|group| group.id);
    let groups: Vec<(u64, String)> = app
        .settings
        .flat_groups
        .items
        .iter()
        .map(|group| (group.id, group.name.clone()))
        .collect();
    ui.menu_button(crate::i18n::gettext(locale, "Assign groups"), |ui| {
        let mut choice: Option<Option<u64>> = None;
        for (id, name) in &groups {
            if ui.selectable_label(current == Some(*id), name).clicked() {
                choice = Some(Some(*id));
            }
        }
        if groups.is_empty() {
            ui.label(crate::i18n::gettext(
                locale,
                "Create a group in the Groups list first.",
            ));
        } else {
            ui.separator();
        }
        if ui
            .selectable_label(current.is_none(), crate::i18n::gettext(locale, "Other"))
            .clicked()
        {
            choice = Some(None);
        }
        ui.separator();
        if ui
            .button(crate::i18n::gettext(locale, "Choose in a window…"))
            .clicked()
        {
            assign(app, chat);
            ui.close();
        }
        if let Some(choice) = choice {
            set_home(app, chat, choice);
            ui.close();
        }
    });
}

/// Keyboard order follows group order, then Ungrouped, deduplicating aliases.
/// Headers are static and never change the navigation order.
pub fn navigation_chats(app: &App) -> Vec<&Chat> {
    let chats = app.visible_chats();
    if app.list_is_flat()
        || app.show_archived
        || app.show_starred
        || app.locked_folder_open()
        || app.secret_code_matched()
        || !app.search.trim().is_empty()
    {
        return chats;
    }
    // The order list() draws: sections by freshest chat in Recent, and only
    // the rows each section actually shows. Plan on the same projected rows
    // list() draws - a linked person is her freshest, summed preview - then
    // map row indexes back to the borrowed chats, one to one and in order.
    let display: Vec<Chat> = chats
        .iter()
        .map(|chat| app.beeper_person_preview(chat))
        .collect();
    let display_refs: Vec<&Chat> = display.iter().collect();
    let activity: Vec<i64> = display.iter().map(|chat| chat.last_activity).collect();
    let sections = plan_sections(app, &display_refs, &activity);
    let mut ordered = Vec::with_capacity(chats.len());
    let mut seen = HashSet::new();
    // A folded group's rows are not on screen, so the keyboard skips them.
    for section in &sections {
        if app.flat_group_ui.collapsed.contains(&section.id) {
            continue;
        }
        for index in &section.shown {
            if seen.insert(&chats[*index].id) {
                ordered.push(chats[*index]);
            }
        }
    }
    ordered
}

/// Reserve all alias geometry, but build only rows intersecting the viewport.
/// This is one scroll area, not one nested scroll area per group.
fn visible_range(
    top: f32,
    row_height: f32,
    count: usize,
    clip: egui::Rect,
) -> std::ops::Range<usize> {
    let first = ((clip.top() - top) / row_height).floor().max(0.0) as usize;
    let end = ((clip.bottom() - top) / row_height).ceil().max(0.0) as usize;
    first.min(count)..end.min(count).max(first.min(count))
}
fn virtual_aliases(
    app: &mut App,
    ui: &mut egui::Ui,
    chats: &[&Chat],
    salt: (&str, u64),
    dim_kept: bool,
) {
    let height = super::chats::chat_row_height(app.settings.beeper.compact_list);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height * chats.len() as f32),
        egui::Sense::hover(),
    );
    if ui.is_enabled()
        && let Some(index) = app
            .scroll_chat_into_view
            .as_ref()
            .and_then(|target| chats.iter().position(|chat| &chat.id == target))
    {
        let row = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, index as f32 * height),
            egui::vec2(rect.width(), height),
        );
        ui.scroll_to_rect(row, Some(egui::Align::Center));
        app.scroll_chat_into_view = None;
    }
    // Outside the main chat cycle egui walks ordinary row widgets. Keep
    // their geometry while a row holds focus so Tab can reach the next row.
    let range = visible_range(
        rect.top(),
        height,
        chats.len(),
        ui.clip_rect().expand2(egui::vec2(0.0, height * 3.0)),
    );
    if range.is_empty() {
        return;
    }
    let visible = egui::Rect::from_min_max(
        rect.min + egui::vec2(0.0, range.start as f32 * height),
        egui::pos2(rect.right().min(ui.clip_rect().right()), rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new().id_salt(salt).max_rect(visible),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for chat in &chats[range] {
                ui.push_id(&chat.id, |ui| {
                    draw_row(app, ui, chat, dim_kept);
                });
            }
        },
    );
}

/// Height of a section's Show-all row.
const EXPANDER_H: f32 = 28.0;

/// The row under a filtered Recent section: open it to every member, or
/// fold it back to the unread rows.
fn expander_row(app: &mut App, ui: &mut egui::Ui, section: &Section) {
    let label = match section.expander {
        Some(Expander::ShowAll) => crate::i18n::gettext(app.locale, "Show all {count} in {group}")
            .replace("{count}", &section.all.len().to_string())
            .replace("{group}", &section.name),
        Some(Expander::UnreadOnly) => {
            crate::i18n::gettext(app.locale, "Show unread only in {group}")
                .replace("{group}", &section.name)
        }
        None => return,
    };
    let palette = app.palette;
    // Stay inside the clip so a focused row is fully visible.
    let width = ui
        .available_width()
        .min(ui.clip_rect().right() - ui.next_widget_position().x);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, EXPANDER_H), egui::Sense::click());
    crate::theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.clone())
    });
    if response.hovered() {
        super::widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
    }
    // A chevron matching the section headings: right to open, down to fold.
    let center = egui::pos2(rect.left() + 30.0, rect.center().y);
    let points = match section.expander {
        Some(Expander::ShowAll) => vec![
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(3.0, 0.0),
            center + egui::vec2(-2.0, 4.0),
        ],
        _ => vec![
            center + egui::vec2(-4.0, -2.0),
            center + egui::vec2(4.0, -2.0),
            center + egui::vec2(0.0, 3.0),
        ],
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        palette.dim,
        egui::Stroke::NONE,
    ));
    ui.painter().text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &label,
        crate::theme::regular(12.0),
        palette.dim,
    );
    if response.clicked() {
        match section.expander {
            Some(Expander::ShowAll) => {
                app.flat_group_ui.show_all.insert(section.id);
            }
            Some(Expander::UnreadOnly) => {
                app.flat_group_ui.show_all.remove(&section.id);
            }
            None => {}
        }
    }
}

/// What a dragged group heading carries, so dropping it on another heading
/// reorders the groups. Chats dragged onto a heading carry a `Member` instead.
struct GroupDrag(u64);

/// A 16-point divider with a fold chevron. It accepts a drop and a click:
/// a click folds or unfolds the section. A folded heading carries the number
/// of unread chats hidden under it.
fn section_header(
    ui: &mut egui::Ui,
    name: &str,
    palette: &crate::theme::Palette,
    collapsed: bool,
    unread: usize,
    draggable: bool,
) -> egui::Response {
    let sense = if draggable {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    // Stay inside the clip so a focused heading is fully visible.
    let width = ui
        .available_width()
        .min(ui.clip_rect().right() - ui.next_widget_position().x);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 16.0), sense);
    // Tab can land on a heading now; keep it scrolled into view.
    crate::theme::reveal_focus(&response);
    #[cfg(test)]
    {
        let key = egui::Id::new(("group-heading-rect", name.to_owned()));
        ui.ctx().data_mut(|data| data.insert_temp(key, rect));
    }
    let painter = ui.painter();
    let text = if collapsed && unread > 0 {
        format!("{name}  {unread}")
    } else {
        name.to_owned()
    };
    let color = if collapsed && unread > 0 {
        palette.accent
    } else {
        palette.secondary
    };
    let label = painter.layout_no_wrap(text, crate::theme::regular(11.0), color);
    let at = egui::pos2(rect.left() + 28.0, rect.center().y - label.size().y * 0.5);
    let gap = egui::Rect::from_min_size(
        egui::pos2(at.x - 6.0, rect.top()),
        egui::vec2(label.size().x + 12.0, rect.height()),
    );
    // Chevron: points right when folded, down when open.
    let center = egui::pos2(rect.left() + 14.0, rect.center().y);
    let points = if collapsed {
        vec![
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(3.0, 0.0),
            center + egui::vec2(-2.0, 4.0),
        ]
    } else {
        vec![
            center + egui::vec2(-4.0, -2.0),
            center + egui::vec2(4.0, -2.0),
            center + egui::vec2(0.0, 3.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        palette.secondary,
        egui::Stroke::NONE,
    ));
    let stroke = egui::Stroke::new(1.0, palette.dim);
    if gap.left() > rect.left() + 20.0 {
        painter.line_segment(
            [
                egui::pos2(rect.left() + 20.0, rect.center().y),
                egui::pos2(gap.left(), rect.center().y),
            ],
            stroke,
        );
    }
    if gap.right() < rect.right() {
        painter.line_segment(
            [
                egui::pos2(gap.right(), rect.center().y),
                rect.right_center(),
            ],
            stroke,
        );
    }
    painter.galley(at, label, color);
    response
}

/// Unread chats among `chats`: the same test Needs me uses, so a muted chat
/// does not count.
fn unread_count(chats: &[&Chat]) -> usize {
    let now = crate::util::now();
    chats
        .iter()
        .filter(|chat| chat.looks_unread() && !chat.muted(now))
        .count()
}

/// Other's heading follows the empty-heading rule, except while a chat is
/// being dragged: then it stays as the drop target that ungroups.
fn other_header_hidden(app: &App, empty: bool, dragging_chat: bool) -> bool {
    header_hidden(app, empty) && !dragging_chat
}

/// Folds every section shut, or unfolds them all.
fn fold_all(app: &mut App, groups: &[crate::flat_groups::Group], fold: bool) {
    app.flat_group_ui.collapsed.clear();
    if fold {
        app.flat_group_ui
            .collapsed
            .extend(groups.iter().map(|group| Some(group.id)));
        app.flat_group_ui.collapsed.insert(None);
    }
}

/// The heading menu's fold-everything entries. Shift-click on a chevron does
/// the same as the entry that applies.
fn fold_menu(app: &mut App, ui: &mut egui::Ui, groups: &[crate::flat_groups::Group]) {
    if ui
        .button(crate::i18n::gettext(app.locale, "Collapse all groups"))
        .clicked()
    {
        fold_all(app, groups, true);
        ui.close();
    }
    if ui
        .button(crate::i18n::gettext(app.locale, "Expand all groups"))
        .clicked()
    {
        fold_all(app, groups, false);
        ui.close();
    }
}

/// Whether an empty section's heading is dropped: only Recent keeps the
/// empty ones, since they are drop targets for assigning chats.
fn header_hidden(app: &App, empty: bool) -> bool {
    empty && app.list_mode != crate::model::ListMode::Recent
}

/// Local group creation belongs beside the group heading, not after the rows.
/// It stays available in either grouped or flat layout, including an empty list.
pub fn controls(app: &mut App, ui: &mut egui::Ui) {
    use super::focus::{Stop, TabStop};
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        let label = crate::i18n::gettext(app.locale, "Group by section");
        let tooltip = format!(
            "{} ({})",
            label,
            super::keys::hint("list-layout").unwrap_or_default()
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui
                .push_id("new-local-group", |ui| {
                    crate::theme::icon_button(
                        ui,
                        crate::theme::Icon::Plus,
                        18.0,
                        app.palette.secondary,
                        app.palette.text,
                        &crate::i18n::gettext(app.locale, "New local group"),
                    )
                })
                .inner
                .tab_stop(Stop::NewLocalGroup);
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("new-local-group-rect"), response.rect));
            if response.clicked() {
                app.flat_group_ui.name.clear();
                app.flat_group_ui.focus_name = true;
            }
            egui::Popup::menu(&response)
                .width(220.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    // TextEdit may consume Escape; capture it before the field.
                    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    ui.label(crate::i18n::gettext(app.locale, "New local group"));
                    let name = ui.add(
                        egui::TextEdit::singleline(&mut app.flat_group_ui.name)
                            .id(egui::Id::new("new-local-group-name"))
                            .hint_text(crate::i18n::gettext(app.locale, "Group name"))
                            .char_limit(80)
                            .desired_width(200.0),
                    );
                    if app.flat_group_ui.focus_name && ui.is_visible() {
                        name.request_focus();
                        app.flat_group_ui.focus_name = false;
                    }
                    let enter = name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let mut create = false;
                    ui.horizontal(|ui| {
                        let button = ui.add_enabled(
                            !app.flat_group_ui.name.trim().is_empty(),
                            egui::Button::new(crate::i18n::gettext(app.locale, "Create")),
                        );
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new("create-local-group-rect"), button.rect)
                        });
                        create = button.clicked() || enter;
                        if ui
                            .button(crate::i18n::gettext(app.locale, "Cancel"))
                            .clicked()
                        {
                            ui.close();
                        }
                    });
                    if escaped {
                        app.flat_group_ui.name.clear();
                        ui.close();
                    }
                    if !escaped
                        && create
                        && app
                            .settings
                            .flat_groups
                            .create(&app.flat_group_ui.name)
                            .is_some()
                    {
                        app.flat_group_ui.name.clear();
                        app.actions.push(Action::SettingsChanged);
                        ui.close();
                    }
                });
            let mut grouped = !app.list_is_flat();
            let response = super::widgets::switch(ui, &app.palette, &mut grouped)
                .tab_stop(Stop::ListLayout)
                .on_hover_text(&tooltip);
            crate::theme::reveal_focus(&response);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    ui.is_enabled(),
                    grouped,
                    &label,
                )
            });
            if response.changed() {
                app.actions.push(crate::model::Action::ToggleListLayout);
            }
            // Reserve both controls before sizing the label. A longer translation
            // truncates on one line rather than pushing + outside the sidebar.
            let label_response = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(label.as_ref())
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    },
                )
                .inner
                .on_hover_text(&tooltip);
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("list-layout-label"), label_response.rect)
            });
            if label_response.clicked() {
                app.actions.push(crate::model::Action::ToggleListLayout);
            }
        });
    });
}

pub fn list(app: &mut App, ui: &mut egui::Ui, to_top: bool) {
    #[cfg(any(test, feature = "demo"))]
    {
        app.flat_group_ui.built_rows = 0;
    }
    let chats = app.beeper_display_chats();
    // Membership plan owns row indexes; do not clone every saved Member just
    // to retain group heading metadata while widgets mutate App.
    let groups: Vec<_> = app
        .settings
        .flat_groups
        .items
        .iter()
        .map(|group| crate::flat_groups::Group {
            id: group.id,
            name: group.name.clone(),
            members: Vec::new(),
        })
        .collect();
    let identity_index = IdentityIndex::new(app);
    let keys: Vec<Vec<Member>> = chats
        .iter()
        .map(|c| identity_index.identities(app, c))
        .collect();
    let chat_refs: Vec<&Chat> = chats.iter().collect();
    let activity: Vec<i64> = chats.iter().map(|chat| chat.last_activity).collect();
    let recent = app.list_mode == crate::model::ListMode::Recent;
    let mut sections = plan_sections(app, &chat_refs, &activity);
    let mut changed = false;
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("chat-list")
        .auto_shrink([false, false]);
    let row_height = super::chats::chat_row_height(app.settings.beeper.compact_list);
    if to_top && super::chats::near_top(ui, "chat-list", row_height) {
        area = area.vertical_scroll_offset(0.0);
    }
    // Revealing a chat unfolds the section holding it, and lifts a Recent
    // section's unread filter when the chat sits outside the filtered rows.
    if ui.is_enabled()
        && let Some(target) = app.scroll_chat_into_view.clone()
        && let Some(index) = chat_refs.iter().position(|chat| chat.id == target)
        && let Some(pos) = sections
            .iter()
            .position(|section| section.all.contains(&index))
    {
        let id = sections[pos].id;
        let mut recalc = app.flat_group_ui.collapsed.remove(&id);
        if sections[pos].expander == Some(Expander::ShowAll)
            && !sections[pos].shown.contains(&index)
        {
            recalc |= app.flat_group_ui.show_all.insert(id);
        }
        if recalc {
            sections = plan_sections(app, &chat_refs, &activity);
        }
    }
    let other_empty = sections.last().is_none_or(|section| section.all.is_empty());
    // While a chat is being dragged, Other stays on screen as a drop target
    // even when it holds nothing, so the chat can always be ungrouped.
    let dragging_chat = egui::DragAndDrop::has_payload_of_type::<Member>(ui.ctx());
    let other_hidden = other_header_hidden(app, other_empty, dragging_chat);
    if ui.is_enabled()
        && let Some(target) = app.scroll_chat_into_view.as_ref()
    {
        let mut top = 0.0;
        let mut target_top = None;
        for section in &sections {
            let hidden = match section.id {
                Some(_) => header_hidden(app, section.all.is_empty()),
                None => other_hidden,
            };
            if hidden {
                continue;
            }
            top += 16.0;
            if app.flat_group_ui.collapsed.contains(&section.id) {
                continue;
            }
            for index in &section.shown {
                if &chat_refs[*index].id == target {
                    target_top = Some(top);
                }
                top += row_height;
            }
            if section.expander.is_some() {
                top += EXPANDER_H;
            }
        }
        if let Some(top) = target_top {
            let id = ui.make_persistent_id(egui::IdSalt::new("chat-list"));
            let current = egui::scroll_area::State::load(ui.ctx(), id)
                .unwrap_or_default()
                .offset
                .y;
            area = area.vertical_scroll_offset(super::chats::reveal_bounds(
                current,
                ui.available_height(),
                top,
                top + row_height,
            ));
            app.scroll_chat_into_view = None;
        }
    }
    let copy = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(super::chats::slide_copy_id()))
        .is_some();
    if copy
        && let Some(offset) = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(super::chats::shared_offset_id()))
    {
        area = area.vertical_scroll_offset(offset);
    }
    let carried = if ui.is_enabled() {
        app.scroll_route.take(crate::app::ScrollPane::Chats)
    } else {
        0.0
    };
    let output = area.show(ui, |ui| {
        if carried != 0.0 {
            ui.scroll_with_delta_animation(
                egui::vec2(0.0, carried),
                egui::style::ScrollAnimation::none(),
            );
        }
        // The plan always ends with Other; every earlier section is a group.
        let (other, sections) = sections.split_last().expect("Other is always planned");
        for section in sections {
            if header_hidden(app, section.all.is_empty()) {
                continue;
            }
            let id = section.id.expect("sections before Other are groups");
            let heading = section.name.as_str();
            let folded = app.flat_group_ui.collapsed.contains(&Some(id));
            let members: Vec<&Chat> = section.all.iter().map(|index| chat_refs[*index]).collect();
            // Recent's sections order themselves by activity: drag-reordering
            // headings stays for the modes that keep the saved order.
            let response = section_header(
                ui,
                heading,
                &app.palette,
                folded,
                unread_count(&members),
                !recent,
            );
            if !recent {
                response.dnd_set_drag_payload(GroupDrag(id));
                if let Some(moving) = response.dnd_hover_payload::<GroupDrag>()
                    && moving.0 != id
                {
                    let items = &app.settings.flat_groups.items;
                    let moving_from = items.iter().position(|g| g.id == moving.0);
                    let here = items.iter().position(|g| g.id == id);
                    let y = if moving_from < here {
                        response.rect.bottom()
                    } else {
                        response.rect.top()
                    };
                    ui.painter().hline(
                        response.rect.x_range(),
                        y,
                        egui::Stroke::new(2.0, app.palette.accent),
                    );
                }
                if let Some(moving) = response.dnd_release_payload::<GroupDrag>()
                    && app.settings.flat_groups.move_to(moving.0, id)
                {
                    changed = true;
                }
            }
            if response.clicked() {
                if ui.input(|input| input.modifiers.shift) {
                    fold_all(app, &groups, !folded);
                } else if folded {
                    app.flat_group_ui.collapsed.remove(&Some(id));
                } else {
                    app.flat_group_ui.collapsed.insert(Some(id));
                }
            }
            if let Some(target) = response.dnd_release_payload::<Member>() {
                // A stale drag from another account must not mutate the current view.
                if keys.iter().any(|row| row.contains(target.as_ref()))
                    && !section
                        .all
                        .iter()
                        .any(|index| keys[*index].contains(target.as_ref()))
                {
                    let sources = keys
                        .iter()
                        .find(|row| row.contains(target.as_ref()))
                        .cloned()
                        .unwrap_or_default();
                    app.settings
                        .flat_groups
                        .assign_linked(target.as_ref(), &sources, &[id]);
                    changed = true;
                }
            }
            response.context_menu(|ui| {
                ui.label(crate::i18n::gettext(
                    app.locale,
                    "Deleting a group never deletes chats or messages.",
                ));
                if ui
                    .button(crate::i18n::gettext(app.locale, "Delete group"))
                    .clicked()
                {
                    app.settings.flat_groups.delete(id);
                    changed = true;
                    ui.close();
                }
                fold_menu(app, ui, &groups);
            });
            if !folded {
                let shown: Vec<&Chat> = section
                    .shown
                    .iter()
                    .map(|index| chat_refs[*index])
                    .collect();
                ui.push_id(("flat-group", id), |ui| {
                    virtual_aliases(app, ui, &shown, ("flat-group", id), section.filtering);
                });
                if section.expander.is_some() {
                    expander_row(app, ui, section);
                }
            }
        }
        if !other_hidden {
            let folded = app.flat_group_ui.collapsed.contains(&None);
            let members: Vec<&Chat> = other.all.iter().map(|index| chat_refs[*index]).collect();
            let response = section_header(
                ui,
                &other.name,
                &app.palette,
                folded,
                unread_count(&members),
                false,
            );
            if response.clicked() {
                if ui.input(|input| input.modifiers.shift) {
                    fold_all(app, &groups, !folded);
                } else if folded {
                    app.flat_group_ui.collapsed.remove(&None);
                } else {
                    app.flat_group_ui.collapsed.insert(None);
                }
            }
            if let Some(target) = response.dnd_release_payload::<Member>() {
                // Dropping a chat on Other takes it out of its group.
                if let Some(sources) = keys.iter().find(|row| row.contains(target.as_ref())) {
                    drop_on_other(&mut app.settings.flat_groups, target.as_ref(), sources);
                    changed = true;
                }
            }
            response.context_menu(|ui| fold_menu(app, ui, &groups));
            if !folded {
                let shown: Vec<&Chat> = other.shown.iter().map(|index| chat_refs[*index]).collect();
                virtual_aliases(app, ui, &shown, ("other", 0), other.filtering);
                if other.expander.is_some() {
                    expander_row(app, ui, other);
                }
            }
        }
    });
    if !copy {
        ui.ctx()
            .data_mut(|d| d.insert_temp(super::chats::shared_offset_id(), output.state.offset.y));
    }
    app.scroll_route
        .place(crate::app::ScrollPane::Chats, output.inner_rect);
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        d.insert_temp(super::chats::list_offset_id(), output.state.offset.y);
        d.insert_temp(super::chats::list_offset_id().with("id"), output.id);
    });
    if changed {
        app.actions.push(Action::SettingsChanged);
    }
}

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let Some((target, mut selected)) = app.flat_group_ui.assigning.clone() else {
        return;
    };
    if !app.chats.iter().any(|chat| member(app, chat) == target) {
        app.flat_group_ui.assigning = None;
        return;
    }
    let mut close = false;
    let mut apply = false;
    egui::Modal::new(egui::Id::new("assign-flat-groups")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(crate::i18n::gettext(app.locale, "Assign groups"));
        ui.label(crate::i18n::gettext(
            app.locale,
            "Choose one group. Chat history stays where it is.",
        ));
        let mut home = selected.first().copied();
        ui.radio_value(&mut home, None, crate::i18n::gettext(app.locale, "Other"));
        for group in &app.settings.flat_groups.items {
            ui.radio_value(&mut home, Some(group.id), &group.name);
        }
        selected = home.into_iter().collect();
        if app.settings.flat_groups.items.is_empty() {
            ui.label(crate::i18n::gettext(
                app.locale,
                "Create a group in the Groups list first.",
            ));
        }
        ui.horizontal(|ui| {
            apply = ui
                .button(crate::i18n::gettext(app.locale, "Apply"))
                .clicked();
            close = ui
                .button(crate::i18n::gettext(app.locale, "Cancel"))
                .clicked();
        });
    });
    if apply {
        let targets = app
            .chats
            .iter()
            .find(|chat| member(app, chat) == target)
            .map(|chat| identities(app, chat))
            .unwrap_or_default();
        app.settings
            .flat_groups
            .assign_linked(&target, &targets, &selected);
        app.actions.push(Action::SettingsChanged);
    }
    app.flat_group_ui.assigning = (!(close || apply)).then_some((target, selected));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn divider_header_is_dense_and_folds_on_click() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let palette = crate::theme::Palette::dark();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let open = section_header(ui, "Work", &palette, false, 0, true);
            let folded = section_header(ui, "Work", &palette, true, 3, false);
            assert_eq!(open.rect.height(), 16.0);
            assert_eq!(folded.rect.height(), 16.0);
            assert_eq!(open.sense, egui::Sense::click_and_drag());
            assert_eq!(folded.sense, egui::Sense::click());
        });
        out.textures_delta.clear();
    }

    /// Dragging a heading onto another one reorders the groups and asks the
    /// app to save the settings; the pointer path covers the real gesture.
    #[test]
    fn dragging_a_heading_onto_another_reorders_groups_and_saves() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        // Recent computes its section order, so headings drag in the modes
        // that keep the saved order.
        app.list_mode = crate::model::ListMode::Unread;
        let a = app.settings.flat_groups.create("Alpha").unwrap();
        let b = app.settings.flat_groups.create("Beta").unwrap();
        let c = app.settings.flat_groups.create("Gamma").unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| list(app, ui, false),
            );
            out.textures_delta.clear();
        };
        let center = |name: &str| {
            let key = egui::Id::new(("group-heading-rect", name.to_owned()));
            let rect = ctx.data(|data| data.get_temp::<egui::Rect>(key));
            rect.expect("heading was laid out").center()
        };
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        let (from, to) = (center("Alpha"), center("Gamma"));
        let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, vec![button(from, true)]);
        for step in 1..=4 {
            let pos = from + (to - from) * (step as f32 / 4.0);
            frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
        }
        frame(&mut app, vec![button(to, false)]);
        frame(&mut app, Vec::new());
        let items = &app.settings.flat_groups.items;
        let order: Vec<u64> = items.iter().map(|g| g.id).collect();
        assert_eq!(order, vec![b, c, a]);
        let saved = app
            .actions
            .iter()
            .any(|action| matches!(action, Action::SettingsChanged));
        assert!(saved);
    }

    #[test]
    fn unread_count_skips_read_and_muted_chats() {
        let mut unread = Chat::new("a".into(), "A".into());
        unread.unread = 2;
        let read = Chat::new("b".into(), "B".into());
        let mut marked = Chat::new("c".into(), "C".into());
        marked.marked_unread = true;
        assert_eq!(unread_count(&[&unread, &read, &marked]), 2);
        assert_eq!(unread_count(&[]), 0);
    }

    #[test]
    fn fold_all_folds_every_section_and_unfold_clears_them() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let groups = vec![
            crate::flat_groups::Group {
                id: 1,
                name: "One".into(),
                members: Vec::new(),
            },
            crate::flat_groups::Group {
                id: 2,
                name: "Two".into(),
                members: Vec::new(),
            },
        ];
        fold_all(&mut app, &groups, true);
        assert_eq!(app.flat_group_ui.collapsed.len(), 3);
        assert!(app.flat_group_ui.collapsed.contains(&None));
        fold_all(&mut app, &groups, false);
        assert!(app.flat_group_ui.collapsed.is_empty());
    }

    fn linked_pair() -> (App, tempfile::TempDir, Chat, Chat) {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let mut a = Chat::new("a".into(), "Person".into());
        a.backend_origin = Some("server".into());
        a.backend_account = Some("signal".into());
        let mut b = a.clone();
        b.id = "b".into();
        b.backend_account = Some("whatsapp".into());
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Person".into(),
            members: vec![
                crate::merged::Member::of(&a).unwrap(),
                crate::merged::Member::of(&b).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![a.clone(), b.clone()];
        (app, root, a, b)
    }

    #[test]
    fn menu_choice_moves_a_chat_and_its_alias_between_groups() {
        let (mut app, _root, a, b) = linked_pair();
        let work = app.settings.flat_groups.create("Work").unwrap();
        let family = app.settings.flat_groups.create("Family").unwrap();
        set_home(&mut app, &a, Some(work));
        let refs: Vec<&Chat> = app.chats.iter().collect();
        assert_eq!(membership_rows(&app, &refs), vec![vec![0, 1], vec![]]);
        // Choosing another group replaces the first; the alias follows.
        set_home(&mut app, &b, Some(family));
        let refs: Vec<&Chat> = app.chats.iter().collect();
        assert_eq!(membership_rows(&app, &refs), vec![vec![], vec![0, 1]]);
        assert!(matches!(app.actions.last(), Some(Action::SettingsChanged)));
    }

    #[test]
    fn menu_other_and_drop_on_other_ungroup_the_chat_and_its_alias() {
        let (mut app, _root, a, b) = linked_pair();
        let work = app.settings.flat_groups.create("Work").unwrap();
        set_home(&mut app, &a, Some(work));
        set_home(&mut app, &b, None);
        let refs: Vec<&Chat> = app.chats.iter().collect();
        assert_eq!(membership_rows(&app, &refs), vec![Vec::<usize>::new()]);
        // Same result for a drop on the Other heading.
        set_home(&mut app, &a, Some(work));
        let target = member(&app, &a);
        let sources = identities(&app, &a);
        assert!(sources.len() == 2);
        drop_on_other(&mut app.settings.flat_groups, &target, &sources);
        let refs: Vec<&Chat> = app.chats.iter().collect();
        assert_eq!(membership_rows(&app, &refs), vec![Vec::<usize>::new()]);
        assert!(app.settings.flat_groups.items[0].members.is_empty());
    }

    #[test]
    fn other_stays_a_drop_target_during_a_chat_drag_outside_recent() {
        use crate::model::ListMode;
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.list_mode = ListMode::Unread;
        // Every visible chat grouped means Other is empty: hidden at rest.
        assert!(header_hidden(&app, true));
        // The list keeps it while a chat is being dragged, so a chat can
        // always be dropped on Other to ungroup it.
        assert!(other_header_hidden(&app, true, false));
        assert!(!other_header_hidden(&app, true, true));
        assert!(!other_header_hidden(&app, false, false));
        let ctx = egui::Context::default();
        let chat = Chat::new("a".into(), "A".into());
        egui::DragAndDrop::set_payload(&ctx, member(&app, &chat));
        assert!(egui::DragAndDrop::has_payload_of_type::<Member>(&ctx));
        egui::DragAndDrop::clear_payload(&ctx);
        assert!(!egui::DragAndDrop::has_payload_of_type::<Member>(&ctx));
    }

    #[test]
    fn empty_headers_hide_outside_recent_only() {
        use crate::model::ListMode;
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.list_mode = ListMode::Recent;
        assert!(!header_hidden(&app, true));
        app.list_mode = ListMode::Unread;
        assert!(header_hidden(&app, true));
        assert!(!header_hidden(&app, false));
        app.list_mode = ListMode::NeedsMe;
        assert!(header_hidden(&app, true));
    }

    #[test]
    fn alias_virtual_range_skips_far_rows_and_keeps_partial_rows() {
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(300.0, 500.0));
        assert_eq!(visible_range(0.0, 80.0, 1000, clip), 1..7);
        assert_eq!(visible_range(1000.0, 80.0, 1000, clip), 0..0);
        assert_eq!(visible_range(-100000.0, 80.0, 10, clip), 10..10);
        assert_eq!(visible_range(100.0, 80.0, 0, clip), 0..0);
    }
    #[test]
    fn membership_plan_matches_exact_linked_identities_without_group_scans() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let mut a = Chat::new("a".into(), "A".into());
        a.backend_origin = Some("origin".into());
        a.backend_account = Some("one".into());
        let mut b = a.clone();
        b.id = "b".into();
        b.backend_account = Some("two".into());
        let mut locked = a.clone();
        locked.id = "locked".into();
        locked.locked = true;
        app.settings.beeper.people.push(crate::merged::Person {
            name: "A".into(),
            members: vec![
                crate::merged::Member::of(&a).unwrap(),
                crate::merged::Member::of(&b).unwrap(),
                crate::merged::Member::of(&locked).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![a, b, locked];
        let group = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(group, member(&app, &app.chats[0]));
        let group = app.settings.flat_groups.create("Private").unwrap();
        app.settings
            .flat_groups
            .add(group, member(&app, &app.chats[2]));
        assert_eq!(
            membership_rows(&app, &app.chats.iter().collect::<Vec<_>>()),
            vec![vec![0, 1], vec![2]]
        );
    }
    #[test]
    fn linked_membership_and_unread_survive_representative_changes() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let mut a = Chat::new("a".into(), "Person".into());
        a.backend_origin = Some("server".into());
        a.backend_account = Some("signal".into());
        let mut b = a.clone();
        b.id = "b".into();
        b.backend_account = Some("whatsapp".into());
        b.unread = 2;
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Person".into(),
            members: vec![
                crate::merged::Member::of(&a).unwrap(),
                crate::merged::Member::of(&b).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![a.clone(), b.clone()];
        let group = app.settings.flat_groups.create("Work").unwrap();
        let target = member(&app, &a);
        app.settings.flat_groups.add(group, target);
        let saved = &app.settings.flat_groups.items[0];
        assert!(belongs(saved, &identities(&app, &a)));
        assert!(belongs(saved, &identities(&app, &b)));
        assert_eq!(app.beeper_person_preview(&a).unread, 2);
        assert_eq!(app.beeper_person_preview(&b).unread, 2);
        assign(&mut app, &b);
        assert_eq!(app.flat_group_ui.assigning.as_ref().unwrap().1, vec![group]);
    }
    #[test]
    fn a_flat_list_steps_in_activity_order_and_tags_rows_with_their_group() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats = vec![
            Chat::new("ungrouped".into(), "Ungrouped".into()),
            Chat::new("work".into(), "Work chat".into()),
        ];
        app.chats[0].last_activity = 200;
        app.chats[1].last_activity = 100;
        let group = app.settings.flat_groups.create("Work").unwrap();
        let work = member(&app, &app.chats[1]);
        app.settings.flat_groups.add(group, work);
        let order = |app: &App| {
            navigation_chats(app)
                .iter()
                .map(|c| c.id.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        // Grouped: the group's chat comes first, though it is older.
        assert_eq!(order(&app), ["work", "ungrouped"]);
        app.apply(Action::ToggleListLayout, &egui::Context::default());
        assert_eq!(order(&app), ["ungrouped", "work"]);
        // Only the grouped chat carries a tag.
        assert_eq!(home_name(&app, &app.chats[1]).as_deref(), Some("Work"));
        assert_eq!(home_name(&app, &app.chats[0]), None);
    }

    #[test]
    fn a_linked_chat_is_tagged_with_the_group_of_any_of_its_aliases() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let mut a = Chat::new("a".into(), "Person".into());
        a.backend_origin = Some("server".into());
        a.backend_account = Some("signal".into());
        let mut b = a.clone();
        b.id = "b".into();
        b.backend_account = Some("whatsapp".into());
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Person".into(),
            members: vec![
                crate::merged::Member::of(&a).unwrap(),
                crate::merged::Member::of(&b).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![a.clone(), b.clone()];
        // Only the alias is saved in the group; the row shows the first one.
        let group = app.settings.flat_groups.create("Work").unwrap();
        let alias = member(&app, &b);
        app.settings.flat_groups.add(group, alias);
        assert_eq!(home_name(&app, &a).as_deref(), Some("Work"));
    }

    #[test]
    fn grouped_keyboard_order_and_reveal_expand_collapsed_rows() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats = vec![
            Chat::new("ungrouped".into(), "Ungrouped".into()),
            Chat::new("work".into(), "Work".into()),
        ];
        for chat in &mut app.chats {
            chat.last_activity = 100;
        }
        let group = app.settings.flat_groups.create("Work").unwrap();
        let work = member(&app, &app.chats[1]);
        app.settings.flat_groups.add(group, work);
        app.settings.flat_groups.grouped_view = true;
        assert_eq!(
            navigation_chats(&app)
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["work", "ungrouped"]
        );
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for target in ["work", "ungrouped"] {
            app.scroll_chat_into_view = Some(target.into());
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| list(&mut app, ui, false));
            output.textures_delta.clear();
            assert!(
                app.scroll_chat_into_view.is_none(),
                "keyboard target must be drawn and reveal request consumed"
            );
        }
        app.scroll_chat_into_view = Some("work".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.disable();
            list(&mut app, ui, false);
        });
        output.textures_delta.clear();
        assert_eq!(app.scroll_chat_into_view.as_deref(), Some("work"));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| list(&mut app, ui, false));
        output.textures_delta.clear();
        assert!(app.scroll_chat_into_view.is_none());
    }

    #[test]
    fn locked_assignment_is_exact_and_unlocked_membership_does_not_leak() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let mut a = Chat::new("locked".into(), "Private".into());
        a.backend_origin = Some("server".into());
        a.backend_account = Some("signal".into());
        a.locked = true;
        let mut b = a.clone();
        b.id = "public".into();
        b.locked = false;
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Person".into(),
            members: vec![
                crate::merged::Member::of(&a).unwrap(),
                crate::merged::Member::of(&b).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![a.clone(), b.clone()];
        let private_key = member(&app, &a);
        let public_key = member(&app, &b);
        assert_eq!(identities(&app, &a), vec![private_key]);
        assert_eq!(identities(&app, &b), vec![public_key]);
    }

    fn recent_app(chats: Vec<Chat>) -> (App, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats = chats;
        (app, root)
    }

    fn recent_chat(id: &str, activity: i64, unread: u32) -> Chat {
        let mut chat = Chat::new(id.into(), id.into());
        chat.last_activity = activity;
        chat.unread = unread;
        chat
    }

    fn recent_plan(app: &App) -> Vec<Section> {
        let refs: Vec<&Chat> = app.chats.iter().collect();
        let activity: Vec<i64> = refs.iter().map(|chat| chat.last_activity).collect();
        plan_sections(app, &refs, &activity)
    }

    #[test]
    fn recent_sections_sort_by_freshest_chat_with_other_last() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("old-work", 100, 0),
            recent_chat("fresh-play", 300, 0),
            recent_chat("other", 200, 0),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        let play = app.settings.flat_groups.create("Play").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(play, member(&app, &app.chats[1]));
        let sections = recent_plan(&app);
        let names: Vec<&str> = sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        // A fresher chat lifts its whole section; Other stays last even
        // though its chat is newer than Work's.
        assert_eq!(names, vec!["Play", "Work", "Other"]);
    }

    #[test]
    fn other_modes_keep_the_saved_section_order() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("old-work", 100, 0),
            recent_chat("fresh-play", 300, 0),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        let play = app.settings.flat_groups.create("Play").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(play, member(&app, &app.chats[1]));
        app.apply(
            Action::SetListMode(crate::model::ListMode::Unread),
            &egui::Context::default(),
        );
        let sections = recent_plan(&app);
        let names: Vec<&str> = sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(names, vec!["Work", "Play", "Other"]);
    }

    #[test]
    fn recent_sections_filter_to_unread_and_expand_on_demand() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("read-a", 100, 0),
            recent_chat("unread-b", 200, 2),
            recent_chat("quiet-c", 300, 0),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[1]));
        let play = app.settings.flat_groups.create("Play").unwrap();
        app.settings
            .flat_groups
            .add(play, member(&app, &app.chats[2]));
        let sections = recent_plan(&app);
        // Play is quiet: every member listed, and it is the freshest section.
        assert_eq!(sections[0].name, "Play");
        assert_eq!(sections[0].shown, vec![2]);
        assert!(!sections[0].filtering);
        assert_eq!(sections[0].expander, None);
        // Work filters to its unread row, with a Show-all row under it.
        assert_eq!(sections[1].name, "Work");
        assert_eq!(sections[1].shown, vec![1]);
        assert!(sections[1].filtering);
        assert_eq!(sections[1].expander, Some(Expander::ShowAll));
        drop(sections);
        // Show-all opens the section to every member and offers the way back.
        app.flat_group_ui.show_all.insert(Some(work));
        let sections = recent_plan(&app);
        assert_eq!(sections[1].shown, vec![0, 1]);
        assert!(!sections[1].filtering);
        assert_eq!(sections[1].expander, Some(Expander::UnreadOnly));
    }

    #[test]
    fn recent_opened_chat_stays_dimmed_until_the_view_changes() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("read-a", 100, 0),
            recent_chat("unread-b", 200, 2),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[1]));
        let ctx = egui::Context::default();
        // Reading "unread-b" from the list keeps its row (KeepUnread).
        app.apply(Action::KeepUnread("unread-b".into()), &ctx);
        assert!(app.unread_kept_contains(&"unread-b".into()));
        app.chats[1].unread = 0;
        let sections = recent_plan(&app);
        // No unread left, but the kept row holds the filtered list open.
        assert_eq!(sections[0].name, "Work");
        assert_eq!(sections[0].shown, vec![1]);
        assert!(sections[0].filtering);
        drop(sections);
        // Leaving Recent clears the keep: the section lists every member again.
        app.apply(Action::SetListMode(crate::model::ListMode::Unread), &ctx);
        app.apply(Action::SetListMode(crate::model::ListMode::Recent), &ctx);
        assert!(!app.unread_kept_contains(&"unread-b".into()));
        let sections = recent_plan(&app);
        assert_eq!(sections[0].shown, vec![0, 1]);
        assert!(!sections[0].filtering);
        assert_eq!(sections[0].expander, None);
    }

    #[test]
    fn recent_flat_list_keeps_nothing() {
        let (mut app, _root) = recent_app(vec![recent_chat("a", 100, 1)]);
        let ctx = egui::Context::default();
        app.apply(Action::ToggleListLayout, &ctx);
        assert!(app.list_is_flat());
        app.apply(Action::KeepUnread("a".into()), &ctx);
        assert!(!app.unread_kept_contains(&"a".into()));
    }

    #[test]
    fn keyboard_walks_only_shown_rows_in_plan_order() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("read-a", 100, 0),
            recent_chat("unread-b", 200, 2),
            recent_chat("quiet-c", 300, 0),
            recent_chat("other", 50, 0),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[1]));
        let play = app.settings.flat_groups.create("Play").unwrap();
        app.settings
            .flat_groups
            .add(play, member(&app, &app.chats[2]));
        let order = |app: &App| {
            navigation_chats(app)
                .iter()
                .map(|c| c.id.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        // Sections by freshest chat; the filtered section lists only unread.
        assert_eq!(order(&app), ["quiet-c", "unread-b", "other"]);
        // A folded section is skipped, as before.
        app.flat_group_ui.collapsed.insert(Some(play));
        assert_eq!(order(&app), ["unread-b", "other"]);
    }

    #[test]
    fn reveal_lifts_a_sections_unread_filter() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("read-a", 100, 0),
            recent_chat("unread-b", 200, 2),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[1]));
        app.settings.flat_groups.grouped_view = true;
        // Revealing a read chat opens its filtered section to every member,
        // and the reveal request is consumed.
        app.scroll_chat_into_view = Some("read-a".into());
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| list(&mut app, ui, false));
        output.textures_delta.clear();
        assert!(app.scroll_chat_into_view.is_none());
        assert!(app.flat_group_ui.show_all.contains(&Some(work)));
        let sections = recent_plan(&app);
        assert_eq!(sections[0].shown, vec![0, 1]);
        assert_eq!(sections[0].expander, Some(Expander::UnreadOnly));
    }

    #[test]
    fn clicking_a_quiet_sections_read_chat_keeps_nothing() {
        let (mut app, _root) = recent_app(vec![
            recent_chat("read-a", 100, 0),
            recent_chat("read-b", 200, 0),
            recent_chat("unread-c", 300, 2),
        ]);
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[1]));
        let play = app.settings.flat_groups.create("Play").unwrap();
        app.settings
            .flat_groups
            .add(play, member(&app, &app.chats[2]));
        app.settings.flat_groups.grouped_view = true;
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| list(app, ui, false),
            );
            out.textures_delta.clear();
        };
        let center = |id: &str| {
            ctx.data(|d| d.get_temp::<egui::Rect>(crate::ui::chats::chat_row_id(id)))
                .expect("row was laid out")
                .center()
        };
        let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let click = |app: &mut App, at: egui::Pos2| {
            frame(app, vec![egui::Event::PointerMoved(at)]);
            frame(app, vec![button(at, true)]);
            frame(app, vec![button(at, false)]);
        };
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        // A read chat in a quiet section: no keep, the section stays full.
        let at = center("read-a");
        click(&mut app, at);
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::KeepUnread(_)))
        );
        let sections = recent_plan(&app);
        assert_eq!(sections[1].name, "Work");
        assert_eq!(sections[1].shown, vec![0, 1]);
        assert!(!sections[1].filtering);
        drop(sections);
        // An unread chat in a filtered section is kept, exactly as before.
        app.actions.clear();
        let at = center("unread-c");
        click(&mut app, at);
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::KeepUnread(id) if id == "unread-c"))
        );
    }

    #[test]
    fn keyboard_order_matches_drawn_rows_for_a_linked_person() {
        let (mut app, _root) = recent_app(Vec::new());
        // The representative sorts first on the id tie-break and is read;
        // her linked member carries the unread message.
        let mut ada = recent_chat("ada", 400, 0);
        ada.backend_origin = Some("server".into());
        ada.backend_account = Some("signal".into());
        let mut ada_alt = recent_chat("ada-alt", 300, 1);
        ada_alt.backend_origin = Some("server".into());
        ada_alt.backend_account = Some("whatsapp".into());
        let ben = recent_chat("ben", 200, 1);
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Ada".into(),
            members: vec![
                crate::merged::Member::of(&ada).unwrap(),
                crate::merged::Member::of(&ada_alt).unwrap(),
            ],
            primary: None,
        });
        app.chats = vec![ada, ada_alt, ben];
        let work = app.settings.flat_groups.create("Work").unwrap();
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[0]));
        app.settings
            .flat_groups
            .add(work, member(&app, &app.chats[2]));
        app.settings.flat_groups.grouped_view = true;
        // The drawn plan: the linked person shows as her summed preview.
        let display = app.beeper_display_chats();
        let refs: Vec<&Chat> = display.iter().collect();
        let activity: Vec<i64> = refs.iter().map(|chat| chat.last_activity).collect();
        let plan = plan_sections(&app, &refs, &activity);
        let drawn: Vec<&str> = plan
            .iter()
            .flat_map(|section| section.shown.iter().map(|index| refs[*index].id.as_str()))
            .collect();
        let keyboard: Vec<&str> = navigation_chats(&app)
            .iter()
            .map(|chat| chat.id.as_str())
            .collect();
        // Regression: the raw representative reads as quiet, and the
        // keyboard used to skip her while the drawn list kept her.
        assert_eq!(drawn, vec!["ada", "ben"]);
        assert_eq!(keyboard, drawn);
    }
}
