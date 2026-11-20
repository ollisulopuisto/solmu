//! One entry point for one chat or a local separate-send audience.
use crate::{
    app::App,
    i18n::{gettext, ngettext},
    model::{Action, Dialog},
};
use std::sync::mpsc;
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    let mut draft = std::mem::take(&mut app.new_message);
    if let Some(rx) = &draft.picker {
        match rx.try_recv() {
            Ok(files) => {
                draft.files.extend(files);
                draft.picker = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => draft.picker = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    ui.heading(gettext(locale, "New message"));
    ui.label(gettext(locale,"One recipient opens a chat. Several recipients receive separate messages, not a network group."));
    ui.add(
        egui::TextEdit::singleline(&mut app.new_chat_search)
            .hint_text(gettext(locale, "Search contacts or groups"))
            .desired_width(f32::INFINITY),
    );
    egui::ScrollArea::vertical()
        .id_salt("new-message-body")
        .max_height((ui.ctx().content_rect().height() - 260.0).max(120.0))
        .show(ui, |ui| {
            let needle = crate::util::search_key(&app.new_chat_search);
            let mut choices: Vec<_> = app
                .chats
                .iter()
                .filter(|chat| {
                    chat.can_send()
                        && !chat.locked
                        && chat.backend_origin.is_some()
                        && chat.backend_account.is_some()
                        && (needle.is_empty()
                            || crate::util::search_key(&app.chat_title(chat)).contains(&needle))
                })
                .map(|chat| {
                    (
                        super::flat_groups::member(app, chat),
                        chat.network
                            .as_ref()
                            .filter(|name| !name.is_empty())
                            .map_or_else(
                                || app.chat_title(chat),
                                |network| format!("{} · {network}", app.chat_title(chat)),
                            ),
                    )
                })
                .collect();
            choices.sort_by(|a, b| a.1.cmp(&b.1));
            egui::ScrollArea::vertical()
                .id_salt("new-message-recipients")
                .max_height(190.0)
                .show(ui, |ui| {
                    for (member, title) in choices {
                        let mut selected = draft.members.contains(&member);
                        if ui.checkbox(&mut selected, title).changed() {
                            if selected {
                                if draft.members.len() < 50 {
                                    draft.members.push(member);
                                }
                            } else {
                                draft.members.retain(|known| known != &member);
                            }
                        }
                    }
                });
            // Keep the original single-contact/self route, including contacts without
            // an existing bridge chat. It never discards a prepared audience draft.
            if draft.members.is_empty() && draft.text.is_empty() && draft.files.is_empty() {
                ui.collapsing(gettext(locale, "Open a contact chat"), |ui| {
                    if app.offers_self(&needle) {
                        super::chats::self_row(app, ui);
                    }
                    for contact in super::dialogs::contact_choices(app, &needle) {
                        ui.push_id(&contact.id, |ui| {
                            super::chats::contact_row(app, ui, &contact)
                        });
                    }
                });
            }
            let mut remove_member = None;
            ui.horizontal_wrapped(|ui| {
                for (index, member) in draft.members.iter().enumerate() {
                    let name = app
                        .chats
                        .iter()
                        .find(|chat| {
                            !chat.locked && super::flat_groups::member(app, chat) == *member
                        })
                        .map(|chat| app.chat_title(chat))
                        .unwrap_or_else(|| gettext(locale, "Unavailable chat").into_owned());
                    if ui
                        .button(format!("{name} ×"))
                        .on_hover_text(gettext(locale, "Remove recipient"))
                        .clicked()
                    {
                        remove_member = Some(index);
                    }
                }
            });
            if let Some(index) = remove_member {
                draft.members.remove(index);
            }
            ui.label(
                ngettext(
                    locale,
                    "{count} recipient selected",
                    "{count} recipients selected",
                    draft.members.len() as u32,
                )
                .replace("{count}", &draft.members.len().to_string()),
            );
            ui.add(
                egui::TextEdit::multiline(&mut draft.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(gettext(locale, "Write a message…")),
            );
            let mut remove = None;
            for (index, path) in draft.files.iter().enumerate() {
                ui.horizontal(|ui| {
                    if crate::app::Pending::is_picture_file(path) {
                        let image = super::widgets::file_image(ui, path);
                        ui.add(image.fit_to_exact_size(egui::vec2(64.0, 48.0)));
                    }
                    ui.add(
                        egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .wrap(),
                    );
                    if ui.button(gettext(locale, "Remove")).clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                draft.files.remove(index);
            }
            if ui
                .add_enabled(
                    draft.picker.is_none(),
                    egui::Button::new(gettext(locale, "Add attachments…")),
                )
                .clicked()
            {
                let (tx, rx) = mpsc::channel();
                let ctx = ui.ctx().clone();
                let title = gettext(locale, "Choose attachments").into_owned();
                if std::thread::Builder::new()
                    .name("audience-file-picker".into())
                    .spawn(move || {
                        let paths = rfd::FileDialog::new()
                            .set_title(title)
                            .pick_files()
                            .unwrap_or_default();
                        let _ = tx.send(paths);
                        ctx.request_repaint();
                    })
                    .is_ok()
                {
                    draft.picker = Some(rx);
                }
            }
            if draft.members.len() > 1 {
                ui.checkbox(
                    &mut draft.keep,
                    gettext(locale, "Save this audience for 7 days without use"),
                );
                if draft.keep {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .hint_text(gettext(locale, "Audience name")),
                    );
                }
            }
            if let Some(notice) = &draft.notice {
                ui.colored_label(ui.visuals().warn_fg_color, notice);
            }
        });
    let mut done = false;
    ui.horizontal_wrapped(|ui| {
        let label = if draft.members.is_empty() {
            gettext(locale, "Choose recipients").into_owned()
        } else if draft.members.len() == 1 {
            gettext(locale, "Open chat").into_owned()
        } else {
            ngettext(
                locale,
                "Send to {count} chat",
                "Send to {count} chats",
                draft.members.len() as u32,
            )
            .replace("{count}", &draft.members.len().to_string())
        };
        let ready = !draft.members.is_empty()
            && draft.picker.is_none()
            && (draft.members.len() == 1
                || !draft.text.trim().is_empty()
                || !draft.files.is_empty());
        if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
            if draft.members.len() == 1 {
                let member = &draft.members[0];
                if member.account != app.account().id.0
                    || !app.chats.iter().any(|chat| {
                        super::flat_groups::member(app, chat) == *member
                            && chat.can_send()
                            && !chat.locked
                    })
                {
                    draft.notice = Some(
                        gettext(locale, "Selected chat unavailable. Nothing was changed.")
                            .into_owned(),
                    );
                } else {
                    app.actions.push(Action::PrepareNewMessage {
                        member: member.clone(),
                        text: draft.text.clone(),
                        files: draft.files.clone(),
                    });
                }
            } else {
                let compose = super::ghost_groups::Compose {
                    group: None,
                    members: draft.members.clone(),
                    name: draft.name.clone(),
                    keep: draft.keep,
                    text: draft.text.clone(),
                    files: draft.files.clone(),
                    notice: None,
                    from_selection: false,
                };
                super::ghost_groups::review(app, compose);
                done = true;
                app.actions.push(Action::CloseDialog);
            }
        }
        ui.menu_button(gettext(locale, "Other actions"), |ui| {
            if ui.button(gettext(locale, "New group")).clicked() {
                app.actions.push(Action::StartNewGroup);
                ui.close();
            }
            if ui.button(gettext(locale, "Add contact")).clicked() {
                app.actions.push(Action::ShowDialog(Dialog::NewContact));
                ui.close();
            }
        });
    });
    if !done {
        app.new_message = draft;
    }
}
