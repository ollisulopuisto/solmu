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
            ui.label(format!(
                "{} unread chats",
                app.settings.flat_groups.unread_total(unread)
            ));
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
                let closed = app.flat_group_ui.collapsed.contains(&group.id);
                let heading = format!(
                    "{} {}   {} chats / {} unread",
                    if closed { "+" } else { "−" },
                    group.name,
                    included.len(),
                    unread
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
                    if keys.iter().flatten().any(|key| key == target.as_ref())
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
            egui::CollapsingHeader::new(format!("Ungrouped · {} chats", ungrouped.len()))
                .id_salt("flat-ungrouped")
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
        for group in &mut app.settings.flat_groups.items {
            group.members.retain(|m| !targets.contains(m));
            if selected.contains(&group.id) {
                group.members.push(target.clone());
            }
        }
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
}
