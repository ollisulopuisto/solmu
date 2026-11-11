//! One-level local list groups. Every alias opens the original chat.
use crate::{
    app::App,
    flat_groups::Member,
    model::{Action, Chat},
};
use std::collections::HashSet;

#[derive(Default)]
pub struct State {
    pub name: String,
    pub assigning: Option<(Member, Vec<u64>)>,
    pub undo: Option<(u64, Member)>,
    collapsed: HashSet<u64>,
}

pub fn member(app: &App, chat: &Chat) -> Member {
    Member {
        account: app.account().id.0.clone(),
        origin: chat.backend_origin.clone(),
        bridge_account: chat.backend_account.clone(),
        chat: chat.id.clone(),
    }
}

/// Resolve all currently unlocked source identities of a linked row.
fn identities(app: &App, chat: &Chat) -> Vec<Member> {
    if chat.locked {
        return vec![member(app, chat)];
    }
    let person = app
        .settings
        .beeper
        .people
        .iter()
        .find(|p| p.members.iter().any(|m| m.matches(chat)));
    match person {
        Some(person) => app
            .chats
            .iter()
            .filter(|c| !c.locked && person.members.iter().any(|m| m.matches(c)))
            .map(|c| member(app, c))
            .collect(),
        None => vec![member(app, chat)],
    }
}
fn belongs(group: &crate::flat_groups::Group, keys: &[Member]) -> bool {
    keys.iter().any(|key| group.members.contains(key))
}
fn draw_row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let response = super::chats::row(app, ui, chat);
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

pub fn toggle(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let grouped = app.settings.flat_groups.grouped_view;
        for (name, mode) in [
            (crate::i18n::gettext(app.locale, "Latest"), false),
            (crate::i18n::gettext(app.locale, "Groups"), true),
        ] {
            if ui.selectable_label(grouped == mode, name).clicked() && grouped != mode {
                app.actions.push(Action::ToggleFlatGroups);
            }
        }
    });
}

/// Keyboard order follows group order, then Ungrouped, deduplicating aliases.
/// Collapsed sections are expanded by list() when their chat is requested.
pub fn navigation_chats(app: &App) -> Vec<&Chat> {
    let chats = app.visible_chats();
    if !app.settings.flat_groups.grouped_view
        || app.show_archived
        || app.show_starred
        || app.locked_folder_open()
        || app.secret_code_matched()
        || !app.search.trim().is_empty()
    {
        return chats;
    }
    let mut ordered = Vec::new();
    for group in &app.settings.flat_groups.items {
        for chat in &chats {
            if belongs(group, &identities(app, chat))
                && !ordered
                    .iter()
                    .any(|existing: &&Chat| existing.id == chat.id)
            {
                ordered.push(*chat);
            }
        }
    }
    for chat in chats {
        if !ordered.iter().any(|existing| existing.id == chat.id) {
            ordered.push(chat);
        }
    }
    ordered
}

pub fn list(app: &mut App, ui: &mut egui::Ui) {
    let chats = app.beeper_display_chats();
    let groups = app.settings.flat_groups.items.clone();
    let keys: Vec<Vec<Member>> = chats.iter().map(|c| identities(app, c)).collect();
    let mut changed = false;
    egui::ScrollArea::vertical()
        .id_salt("flat-group-list")
        .show(ui, |ui| {
            ui.label(crate::i18n::gettext(
                app.locale,
                "Local only. No parent groups or subgroups.",
            ));
            let unread: Vec<_> = chats
                .iter()
                .zip(&keys)
                .filter(|(c, _)| c.looks_unread())
                .filter_map(|(_, k)| k.first())
                .collect();
            let count = app.settings.flat_groups.unread_total(unread);
            ui.label(
                crate::i18n::ngettext(
                    app.locale,
                    "{count} unread chat",
                    "{count} unread chats",
                    count as u32,
                )
                .replace("{count}", &count.to_string()),
            );
            if let Some((id, target)) = app.flat_group_ui.undo.clone() {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::gettext(app.locale, "Added to group"));
                    if ui
                        .button(crate::i18n::gettext(app.locale, "Undo"))
                        .clicked()
                    {
                        app.settings.flat_groups.remove(id, &target);
                        app.flat_group_ui.undo = None;
                        changed = true;
                    }
                });
            }
            for group in &groups {
                let included: Vec<_> = chats
                    .iter()
                    .zip(&keys)
                    .filter(|(_, k)| belongs(group, k))
                    .map(|(c, _)| c)
                    .collect();
                let unread = included.iter().filter(|c| c.looks_unread()).count();
                if app
                    .scroll_chat_into_view
                    .as_ref()
                    .is_some_and(|id| included.iter().any(|chat| &chat.id == id))
                {
                    app.flat_group_ui.collapsed.remove(&group.id);
                }
                let closed = app.flat_group_ui.collapsed.contains(&group.id);
                let counts = crate::i18n::gettext(app.locale, "{chats} chats / {unread} unread")
                    .replace("{chats}", &included.len().to_string())
                    .replace("{unread}", &unread.to_string());
                let heading = format!(
                    "{} {}   {}",
                    if closed { "+" } else { "−" },
                    group.name,
                    counts
                );
                let response =
                    ui.add_sized([ui.available_width(), 36.0], egui::Button::new(heading));
                if response.clicked() {
                    if closed {
                        app.flat_group_ui.collapsed.remove(&group.id);
                    } else {
                        app.flat_group_ui.collapsed.insert(group.id);
                    }
                }
                if let Some(target) = response.dnd_release_payload::<Member>() {
                    // A stale drag from another account must not mutate the current view.
                    if keys
                        .iter()
                        .any(|row| row.contains(target.as_ref()) && !belongs(group, row))
                        && app.settings.flat_groups.add(group.id, (*target).clone())
                    {
                        app.flat_group_ui.undo = Some((group.id, (*target).clone()));
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
                if !closed {
                    ui.push_id(("flat-group", group.id), |ui| {
                        for chat in included {
                            ui.push_id(&chat.id, |ui| {
                                draw_row(app, ui, chat);
                            });
                        }
                    });
                }
            }
            let ungrouped: Vec<_> = chats
                .iter()
                .zip(&keys)
                .filter(|(_, key)| !groups.iter().any(|g| belongs(g, key)))
                .map(|(c, _)| c)
                .collect();
            egui::CollapsingHeader::new(
                crate::i18n::ngettext(
                    app.locale,
                    "Ungrouped · {count} chat",
                    "Ungrouped · {count} chats",
                    ungrouped.len() as u32,
                )
                .replace("{count}", &ungrouped.len().to_string()),
            )
            .id_salt("flat-ungrouped")
            .open(
                app.scroll_chat_into_view
                    .as_ref()
                    .is_some_and(|id| ungrouped.iter().any(|chat| &chat.id == id))
                    .then_some(true),
            )
            .show(ui, |ui| {
                for chat in ungrouped {
                    ui.push_id(("ungrouped", &chat.id), |ui| {
                        draw_row(app, ui, chat);
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.flat_group_ui.name)
                        .hint_text(crate::i18n::gettext(app.locale, "New group"))
                        .char_limit(80)
                        .desired_width(140.0),
                );
                if ui
                    .button(crate::i18n::gettext(app.locale, "Create"))
                    .clicked()
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
            "Same chat in several groups. No copied history.",
        ));
        for group in &app.settings.flat_groups.items {
            let mut checked = selected.contains(&group.id);
            if ui.checkbox(&mut checked, &group.name).changed() {
                selected.retain(|id| id != &group.id);
                if checked {
                    selected.push(group.id);
                }
            }
        }
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
        app.flat_group_ui.undo = None;
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
        let group = app.settings.flat_groups.create("Work").unwrap();
        let work = member(&app, &app.chats[1]);
        app.settings.flat_groups.add(group, work);
        app.settings.flat_groups.grouped_view = true;
        app.flat_group_ui.collapsed.insert(group);
        assert_eq!(
            navigation_chats(&app)
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["work", "ungrouped"]
        );
        let ctx = egui::Context::default();
        for target in ["work", "ungrouped"] {
            app.scroll_chat_into_view = Some(target.into());
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| list(&mut app, ui));
            output.textures_delta.clear();
            assert!(
                app.scroll_chat_into_view.is_none(),
                "keyboard target must be drawn and reveal request consumed"
            );
        }
        assert!(!app.flat_group_ui.collapsed.contains(&group));
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
