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
    pub assigning: Option<(Member, Vec<u64>)>,
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
fn draw_row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    #[cfg(any(test, feature = "demo"))]
    {
        app.flat_group_ui.built_rows += 1;
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
        app.actions.push(Action::KeepUnread(chat.id.clone()));
    }
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

/// Keyboard order follows group order, then Ungrouped, deduplicating aliases.
/// Headers are static and never change the navigation order.
pub fn navigation_chats(app: &App) -> Vec<&Chat> {
    let chats = app.visible_chats();
    if app.show_archived
        || app.show_starred
        || app.locked_folder_open()
        || app.secret_code_matched()
        || !app.search.trim().is_empty()
    {
        return chats;
    }
    let groups = membership_rows(app, &chats);
    let mut ordered = Vec::with_capacity(chats.len());
    let mut seen = HashSet::new();
    for index in groups.into_iter().flatten().chain(0..chats.len()) {
        if seen.insert(&chats[index].id) {
            ordered.push(chats[index]);
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
fn virtual_aliases(app: &mut App, ui: &mut egui::Ui, chats: &[&Chat], salt: (&str, u64)) {
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
                    draw_row(app, ui, chat);
                });
            }
        },
    );
}

/// A static 16-point divider. It accepts a drop but never a click,
/// so a group's heading cannot collapse or focus the list.
fn section_header(
    ui: &mut egui::Ui,
    name: &str,
    palette: &crate::theme::Palette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let painter = ui.painter();
    let label = painter.layout_no_wrap(
        name.to_owned(),
        crate::theme::regular(11.0),
        palette.secondary,
    );
    let at = egui::pos2(rect.left() + 16.0, rect.center().y - label.size().y * 0.5);
    let gap = egui::Rect::from_min_size(
        egui::pos2(at.x - 6.0, rect.top()),
        egui::vec2(label.size().x + 12.0, rect.height()),
    );
    let stroke = egui::Stroke::new(1.0, palette.dim);
    painter.line_segment(
        [rect.left_center(), egui::pos2(gap.left(), rect.center().y)],
        stroke,
    );
    if gap.right() < rect.right() {
        painter.line_segment(
            [
                egui::pos2(gap.right(), rect.center().y),
                rect.right_center(),
            ],
            stroke,
        );
    }
    painter.galley(at, label, palette.secondary);
    response
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
    let rows = membership_rows(app, &chats.iter().collect::<Vec<_>>());
    let grouped: HashSet<usize> = rows.iter().flatten().copied().collect();
    let mut changed = false;
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("chat-list")
        .auto_shrink([false, false]);
    let row_height = super::chats::chat_row_height(app.settings.beeper.compact_list);
    if to_top && super::chats::near_top(ui, "chat-list", row_height) {
        area = area.vertical_scroll_offset(0.0);
    }
    if ui.is_enabled()
        && let Some(target) = app.scroll_chat_into_view.as_ref()
    {
        let mut top = 0.0;
        let mut target_top = None;
        for indices in &rows {
            top += 16.0;
            for index in indices {
                if &chats[*index].id == target {
                    target_top = Some(top);
                }
                top += row_height;
            }
        }
        top += 16.0;
        for (_, chat) in chats
            .iter()
            .enumerate()
            .filter(|(i, _)| !grouped.contains(i))
        {
            if &chat.id == target {
                target_top = Some(top);
            }
            top += row_height;
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
        for (group, indices) in groups.iter().zip(&rows) {
            let included: Vec<_> = indices.iter().map(|index| &chats[*index]).collect();
            let heading = &group.name;
            let response = section_header(ui, heading, &app.palette);
            if let Some(target) = response.dnd_release_payload::<Member>() {
                // A stale drag from another account must not mutate the current view.
                if keys.iter().any(|row| row.contains(target.as_ref()))
                    && !indices
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
                        .assign_linked(target.as_ref(), &sources, &[group.id]);
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
                    app.settings.flat_groups.delete(group.id);
                    changed = true;
                    ui.close();
                }
            });
            ui.push_id(("flat-group", group.id), |ui| {
                virtual_aliases(app, ui, &included, ("flat-group", group.id));
            });
        }
        let ungrouped: Vec<_> = chats
            .iter()
            .enumerate()
            .filter(|(index, _)| !grouped.contains(index))
            .map(|(_, chat)| chat)
            .collect();
        section_header(ui, &crate::i18n::gettext(app.locale, "Other"), &app.palette);
        virtual_aliases(app, ui, &ungrouped, ("other", 0));
        ui.separator();
        ui.horizontal(|ui| {
            let name = ui.add(
                egui::TextEdit::singleline(&mut app.flat_group_ui.name)
                    .hint_text(crate::i18n::gettext(app.locale, "New group"))
                    .char_limit(80)
                    .desired_width(140.0),
            );
            crate::theme::reveal_focus(&name);
            let create = ui.button(crate::i18n::gettext(app.locale, "Create"));
            crate::theme::reveal_focus(&create);
            if create.clicked()
                && app
                    .settings
                    .flat_groups
                    .create(&app.flat_group_ui.name)
                    .is_some()
            {
                app.flat_group_ui.name.clear();
                changed = true;
            }
        });
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
    fn divider_header_is_dense_and_does_not_offer_clicks() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let palette = crate::theme::Palette::dark();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = section_header(ui, "Work", &palette);
            assert_eq!(response.rect.height(), 16.0);
            assert_eq!(response.sense, egui::Sense::hover());
        });
        out.textures_delta.clear();
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
}
