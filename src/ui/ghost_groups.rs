//! Ghost groups: pick chats in the sidebar, write one message, and it goes to
//! each chat separately. The group itself lives only on this computer, can be
//! disbanded in one click, and expires after 7 days without use.
use crate::{
    app::App,
    ghost_groups::{Member, Refusal},
    i18n::{gettext, ngettext},
    model::Action,
};

#[derive(Default)]
pub struct State {
    pub compose: Option<Compose>,
}

pub struct Compose {
    /// An existing group being used again, or `None` for a fresh selection.
    pub group: Option<u64>,
    pub members: Vec<Member>,
    pub name: String,
    pub keep: bool,
    pub text: String,
    pub notice: Option<String>,
    /// The selection this came from, cleared after sending.
    pub from_selection: bool,
}

fn chat_for<'a>(app: &'a App, member: &Member) -> Option<&'a crate::model::Chat> {
    app.chats
        .iter()
        .find(|chat| super::flat_groups::member(app, chat) == *member)
}

/// A recipient can be sent to when its chat is known, writable and not locked.
pub fn sendable(app: &App, member: &Member) -> bool {
    chat_for(app, member).is_some_and(|chat| chat.can_send() && !chat.locked)
}

fn refusal_text(app: &App, refusal: Refusal) -> String {
    match refusal {
        Refusal::TooFewMembers => gettext(app.locale, "Pick at least two chats.").into_owned(),
        Refusal::TooManyMembers => {
            gettext(app.locale, "A ghost group holds up to 50 chats.").into_owned()
        }
        Refusal::TooManyGroups => {
            gettext(app.locale, "You have 20 ghost groups. Disband one first.").into_owned()
        }
        Refusal::BadName => gettext(
            app.locale,
            "The name must be one line of at most 60 characters.",
        )
        .into_owned(),
    }
}

fn selection(app: &App) -> Vec<Member> {
    if app.batch_ui.owner != app.account().id.0 {
        return Vec::new();
    }
    app.batch_ui
        .selected_chats
        .iter()
        .filter_map(|id| app.chat(id))
        .map(|chat| super::flat_groups::member(app, chat))
        .collect()
}

/// Shown at the top of the chat list: the selection button and the live groups.
pub fn bar(app: &mut App, ui: &mut egui::Ui) {
    let now = crate::util::now();
    if app.settings.ghost_groups.any_expired(now) {
        app.settings.ghost_groups.prune(now);
        app.actions.push(Action::SettingsChanged);
    }
    let picked = selection(app);
    if picked.len() >= crate::ghost_groups::MIN_MEMBERS
        && ui
            .button(
                ngettext(
                    app.locale,
                    "Message {count} chat separately…",
                    "Message {count} chats separately…",
                    picked.len() as u32,
                )
                .replace("{count}", &picked.len().to_string()),
            )
            .clicked()
    {
        app.ghost_ui.compose = Some(Compose {
            group: None,
            members: picked,
            name: String::new(),
            keep: false,
            text: String::new(),
            notice: None,
            from_selection: true,
        });
    }
    if app.settings.ghost_groups.items.is_empty() {
        return;
    }
    let groups: Vec<(u64, String, usize, i64)> = app
        .settings
        .ghost_groups
        .items
        .iter()
        .map(|group| {
            (
                group.id,
                group.name.clone(),
                group.members.len(),
                group.days_left(now),
            )
        })
        .collect();
    let mut open = None;
    let mut disband = None;
    ui.label(gettext(app.locale, "Ghost groups"));
    for (id, name, count, days) in groups {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                ngettext(
                    app.locale,
                    "{name} ({count} chat), {days} d left",
                    "{name} ({count} chats), {days} d left",
                    count as u32,
                )
                .replace("{name}", &name)
                .replace("{count}", &count.to_string())
                .replace("{days}", &days.to_string()),
            );
            if ui.button(gettext(app.locale, "Message…")).clicked() {
                open = Some(id);
            }
            if ui.button(gettext(app.locale, "Disband")).clicked() {
                disband = Some(id);
            }
        });
    }
    if let Some(id) = disband {
        app.settings.ghost_groups.disband(id);
        app.actions.push(Action::SettingsChanged);
    }
    if let Some(id) = open
        && let Some(group) = app.settings.ghost_groups.get(id)
    {
        app.ghost_ui.compose = Some(Compose {
            group: Some(id),
            members: group.members.clone(),
            name: group.name.clone(),
            keep: true,
            text: String::new(),
            notice: None,
            from_selection: false,
        });
    }
}

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let Some(mut compose) = app.ghost_ui.compose.take() else {
        return;
    };
    let rows: Vec<(String, bool)> = compose
        .members
        .iter()
        .map(|member| match chat_for(app, member) {
            Some(chat) => (app.chat_title(chat), sendable(app, member)),
            None => (gettext(app.locale, "Chat unavailable").into_owned(), false),
        })
        .collect();
    let ready = rows.iter().filter(|(_, ok)| *ok).count();
    let connected = app.is_connected();
    let mut remove = None;
    let mut send = false;
    let mut cancel = false;
    egui::Modal::new(egui::Id::new("ghost-group-compose")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(
            ngettext(
                app.locale,
                "Message {count} chat separately",
                "Message {count} chats separately",
                compose.members.len() as u32,
            )
            .replace("{count}", &compose.members.len().to_string()),
        );
        ui.label(gettext(
            app.locale,
            "Each chat gets its own message. Nobody sees who else received it.",
        ));
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for (index, (title, ok)) in rows.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(if *ok {
                            title.clone()
                        } else {
                            format!("{title} ({})", gettext(app.locale, "cannot send"))
                        });
                        if ui.small_button("✕").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        ui.add(
            egui::TextEdit::multiline(&mut compose.text)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        if compose.group.is_none() {
            ui.checkbox(
                &mut compose.keep,
                gettext(
                    app.locale,
                    "Keep as a ghost group (disbands after 7 days without use)",
                ),
            );
            if compose.keep {
                ui.add(
                    egui::TextEdit::singleline(&mut compose.name)
                        .hint_text(gettext(app.locale, "Name (optional)")),
                );
            }
        }
        if let Some(notice) = &compose.notice {
            ui.colored_label(ui.visuals().warn_fg_color, notice);
        }
        if !connected {
            ui.label(gettext(
                app.locale,
                "Not connected. Nothing can be sent yet.",
            ));
        }
        ui.horizontal(|ui| {
            let enabled = connected && ready >= 1 && !compose.text.trim().is_empty();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(
                        ngettext(
                            app.locale,
                            "Send to {count} chat",
                            "Send to {count} chats",
                            ready as u32,
                        )
                        .replace("{count}", &ready.to_string()),
                    ),
                )
                .clicked()
            {
                send = true;
            }
            if ui.button(gettext(app.locale, "Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if let Some(index) = remove {
        compose.members.remove(index);
    }
    if cancel || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        return;
    }
    if send {
        let now = crate::util::now();
        // Keep or reuse the group first: a refusal sends nothing.
        match compose.group {
            Some(id) => {
                if !app.settings.ghost_groups.touch(id, now) {
                    compose.notice =
                        Some(gettext(app.locale, "This ghost group has expired.").into_owned());
                    app.ghost_ui.compose = Some(compose);
                    return;
                }
            }
            None if compose.keep => {
                match app
                    .settings
                    .ghost_groups
                    .create(&compose.name, compose.members.clone(), now)
                {
                    Ok(_) => {}
                    Err(refusal) => {
                        compose.notice = Some(refusal_text(app, refusal));
                        app.ghost_ui.compose = Some(compose);
                        return;
                    }
                }
            }
            None => {}
        }
        let text = compose.text.trim().to_owned();
        let targets: Vec<String> = compose
            .members
            .iter()
            .filter(|member| sendable(app, member))
            .filter_map(|member| chat_for(app, member).map(|chat| chat.id.clone()))
            .collect();
        for chat in targets {
            app.actions.push(Action::SendText {
                chat,
                text: text.clone(),
                quoting: None,
            });
        }
        if compose.from_selection {
            app.batch_ui.selected_chats.clear();
            app.batch_ui.selected_identities.clear();
        }
        app.actions.push(Action::SettingsChanged);
        return;
    }
    app.ghost_ui.compose = Some(compose);
}
