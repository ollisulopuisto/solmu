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
            // A locked chat's name stays hidden until its folder is open.
            Some(chat) if chat.locked && !app.locked_folder_open() => {
                (gettext(app.locale, "Locked chat").into_owned(), false)
            }
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
    if send && submit(app, &mut compose) {
        return;
    }
    app.ghost_ui.compose = Some(compose);
}

/// Checks the compose state and queues the sends. Returns true when the
/// messages were queued. On a refusal it sets `compose.notice` and changes
/// nothing else: no group is kept or touched, nothing is queued and no draft
/// is cleared.
fn submit(app: &mut App, compose: &mut Compose) -> bool {
    let now = crate::util::now();
    let targets: Vec<String> = compose
        .members
        .iter()
        .filter(|member| sendable(app, member))
        .filter_map(|member| chat_for(app, member).map(|chat| chat.id.clone()))
        .collect();
    // Refuse before anything is kept or sent. A send in the main
    // composer's edit mode would turn the first message into an edit,
    // and a send clears the target chat's saved draft.
    let refusal = if app.editing.is_some() {
        Some(
            gettext(
                app.locale,
                "Finish or cancel the message you are editing first.",
            )
            .into_owned(),
        )
    } else if compose.members.len() > crate::ghost_groups::MAX_MEMBERS {
        Some(gettext(app.locale, "A ghost group holds up to 50 chats.").into_owned())
    } else if targets.iter().any(|chat| {
        app.parked_draft(chat)
            // The open chat's draft lives in the composer and staged files.
            || (app.open_chat.as_deref() == Some(chat.as_str())
                && (!app.composer.trim().is_empty() || !app.pending.is_empty()))
    }) {
        Some(
            gettext(
                app.locale,
                "A chat here has an unsent draft. Send or clear it first.",
            )
            .into_owned(),
        )
    } else if compose.keep
        && compose.group.is_none()
        && compose
            .members
            .iter()
            .any(|member| chat_for(app, member).is_some_and(|chat| chat.locked))
    {
        // Saved groups sit in settings.json; locked chats stay out of it.
        Some(
            gettext(
                app.locale,
                "Locked chats cannot be kept in a ghost group. Untick Keep or remove them.",
            )
            .into_owned(),
        )
    } else {
        None
    };
    if let Some(text) = refusal {
        compose.notice = Some(text);
        return false;
    }
    // Keep or reuse the group first: a refusal sends nothing.
    match compose.group {
        Some(id) => {
            if !app.settings.ghost_groups.touch(id, now) {
                compose.notice =
                    Some(gettext(app.locale, "This ghost group has expired.").into_owned());
                return false;
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
                    return false;
                }
            }
        }
        None => {}
    }
    let text = compose.text.trim().to_owned();
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
    // No per-chat result list yet: say where to look so nobody resends all.
    app.toast(gettext(
        app.locale,
        "Queued for each chat. Check each chat for its result, and do not send the same text to all again.",
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Chat;

    fn app_with(ids: &[&str]) -> (App, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats = ids
            .iter()
            .map(|id| Chat::new((*id).into(), (*id).to_uppercase()))
            .collect();
        (app, root)
    }

    fn compose_for(app: &App, ids: &[&str], keep: bool) -> Compose {
        let members = ids
            .iter()
            .map(|id| {
                let chat = app.chats.iter().find(|chat| chat.id == *id).unwrap();
                super::super::flat_groups::member(app, chat)
            })
            .collect();
        Compose {
            group: None,
            members,
            name: String::new(),
            keep,
            text: "hello".into(),
            notice: None,
            from_selection: true,
        }
    }

    /// A refusal queues nothing, keeps no group and says why.
    fn assert_refused(app: &App, compose: &Compose) {
        assert!(compose.notice.is_some(), "a refusal must tell the user why");
        assert!(app.actions.is_empty(), "nothing may be queued");
        assert!(
            app.settings.ghost_groups.items.is_empty(),
            "no group may be kept"
        );
    }

    #[test]
    fn a_saved_draft_in_a_target_chat_refuses_and_stays_untouched() {
        let (mut app, _root) = app_with(&["a", "b"]);
        app.drafts.insert("a".into(), "half written".into());
        let mut compose = compose_for(&app, &["a", "b"], true);
        assert!(!submit(&mut app, &mut compose));
        assert_refused(&app, &compose);
        assert_eq!(
            app.drafts.get("a").map(String::as_str),
            Some("half written")
        );
    }

    #[test]
    fn the_open_chats_composer_text_refuses_and_stays_untouched() {
        let (mut app, _root) = app_with(&["a", "b"]);
        app.open_chat = Some("a".into());
        app.composer = "typing now".into();
        let mut compose = compose_for(&app, &["a", "b"], false);
        assert!(!submit(&mut app, &mut compose));
        assert_refused(&app, &compose);
        assert_eq!(app.composer, "typing now");
    }

    #[test]
    fn an_active_edit_refuses() {
        let (mut app, _root) = app_with(&["a", "b"]);
        app.editing = Some("message-1".into());
        let mut compose = compose_for(&app, &["a", "b"], true);
        assert!(!submit(&mut app, &mut compose));
        assert_refused(&app, &compose);
        assert_eq!(app.editing.as_deref(), Some("message-1"));
    }

    #[test]
    fn a_locked_member_cannot_be_kept_and_nothing_is_sent() {
        let (mut app, _root) = app_with(&["a", "b", "l"]);
        app.chats[2].locked = true;
        let mut compose = compose_for(&app, &["a", "b", "l"], true);
        assert!(!submit(&mut app, &mut compose));
        assert_refused(&app, &compose);
    }

    #[test]
    fn more_than_fifty_chats_refuse_even_when_not_kept() {
        let ids: Vec<String> = (0..51).map(|i| format!("c{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (mut app, _root) = app_with(&refs);
        let mut compose = compose_for(&app, &refs, false);
        assert!(!submit(&mut app, &mut compose));
        assert_refused(&app, &compose);
    }

    #[test]
    fn a_clean_send_queues_one_message_per_chat_and_keeps_the_group() {
        let (mut app, _root) = app_with(&["a", "b"]);
        let mut compose = compose_for(&app, &["a", "b"], true);
        assert!(submit(&mut app, &mut compose));
        let sent: Vec<&str> = app
            .actions
            .iter()
            .filter_map(|action| match action {
                Action::SendText { chat, text, .. } if text == "hello" => Some(chat.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(sent, vec!["a", "b"]);
        assert_eq!(app.settings.ghost_groups.items.len(), 1);
    }
}
