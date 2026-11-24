//! Two-stage keyboard menus. Targets and choices are snapshots, never positions.
use crate::{
    app::App,
    model::{Action, Content},
};
use egui::{Id, Key, Modifiers};
#[derive(Clone)]
struct Menu {
    account: String,
    owner: String,
    chat: String,
    message: String,
    templates: bool,
    choices: Vec<String>,
}
fn id() -> Id {
    Id::new("message-key-menu")
}
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Menu>(id())).is_some()
}
fn target(app: &App) -> Option<(String, String)> {
    let chat = app.current_chat()?;
    let members = super::merged::person_for_view(app, chat)
        .map(|p| super::merged::valid_members(app, p, chat));
    app.conversations
        .iter()
        .filter(|(id, _)| {
            members
                .as_ref()
                .map_or(*id == &chat.id, |m| m.iter().any(|x| &x.chat == *id))
        })
        .flat_map(|(chat, c)| c.messages.iter().filter(move |m| &m.chat == chat))
        .filter(|m| !m.from_me && !matches!(m.content, Content::Revoked | Content::Call { .. }))
        .max_by_key(|m| (m.timestamp, &m.chat, m.history_order, &m.id))
        .map(|m| (m.chat.clone(), m.id.clone()))
}
fn valid(app: &App, menu: &Menu) -> bool {
    app.account().id.0 == menu.account
        && app.open_chat.as_deref() == Some(menu.owner.as_str())
        && app.chat_view()
        && !app.app_lock.is_locked()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && app.image_preview.is_none()
        && app.sticker_view.is_none()
        && app.picture_edit.is_none()
        && app.pending_picture_preview.is_none()
        && app.local_avatar_editor.is_none()
        && app.selection.is_none()
        && app
            .chat(&menu.chat)
            .is_some_and(|c| !c.read_only && (!c.locked || app.locked_folder_open()))
        && app
            .conversations
            .get(&menu.chat)
            .and_then(|c| c.message(&menu.message))
            .is_some_and(|m| {
                !m.from_me && !matches!(m.content, Content::Revoked | Content::Call { .. })
            })
        && app.current_chat().is_some_and(|chat| {
            chat.id == menu.chat
                || super::merged::person_for_view(app, chat).is_some_and(|p| {
                    super::merged::valid_members(app, p, chat)
                        .iter()
                        .any(|m| m.chat == menu.chat)
                })
        })
}
pub fn open(app: &App, ctx: &egui::Context, templates: bool) {
    if !app.chat_view()
        || app.show_update
        || app.reaction_target.is_some()
        || app.selection.is_some()
        || super::keys::window_owned(app)
        || app.app_lock.is_locked()
        || egui::Popup::is_any_open(ctx)
    {
        return;
    }
    let Some((chat, message)) = target(app) else {
        return;
    };
    let Some(owner) = app.open_chat.clone() else {
        return;
    };
    let choices = if templates {
        ["Yes", "No", "Later"]
            .iter()
            .map(|s| crate::i18n::gettext(app.locale, s).into_owned())
            .collect()
    } else {
        super::conversation::quick_reactions(
            app.conversations[&chat].message(&message).unwrap(),
            &app.settings.reaction_emoji,
        )
        .into_iter()
        .take(6)
        .map(str::to_owned)
        .collect()
    };
    let menu = Menu {
        account: app.account().id.0.clone(),
        owner,
        chat,
        message,
        templates,
        choices,
    };
    if valid(app, &menu) {
        ctx.data_mut(|d| d.insert_temp(id(), menu));
    }
}
fn choose(app: &mut App, ctx: &egui::Context, menu: &Menu, index: usize) {
    ctx.data_mut(|d| d.remove::<Menu>(id()));
    if !valid(app, menu) {
        return;
    }
    let Some(text) = menu.choices.get(index) else {
        return;
    };
    if menu.templates {
        // Same draft guard and exact-member route as pointer Quick Replies.
        if app.composer.trim().is_empty()
            && app.reply_to.is_none()
            && app.editing.is_none()
            && !app.drafts.contains_key(&menu.owner)
            && app.pending.is_empty()
        {
            if let Some(person) = app
                .current_chat()
                .and_then(|c| super::merged::person_for_view(app, c))
            {
                if let Some(member) = person.members.iter().find(|m| m.chat == menu.chat).cloned() {
                    super::merged::prepare_destination(app, ctx, &member);
                }
            }
        }
        app.actions.push(Action::PrefillComposer {
            chat: menu.owner.clone(),
            text: text.clone(),
            mentions: Vec::new(),
        });
    } else {
        app.actions.push(Action::React {
            chat: menu.chat.clone(),
            message: menu.message.clone(),
            emoji: text.clone(),
        });
    }
}
/// Runs before background fields. Bare digits cannot leak into the draft.
pub(crate) fn keyboard(app: &mut App, ctx: &egui::Context) -> bool {
    let Some(menu) = ctx.data(|d| d.get_temp::<Menu>(id())) else {
        return false;
    };
    if !valid(app, &menu) {
        ctx.data_mut(|d| d.remove::<Menu>(id()));
        return false;
    }
    let mut choice = None;
    let mut close = false;
    ctx.input_mut(|i| {
        i.events.retain(|event| match event {
            egui::Event::Text(_) => false,
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if modifiers.matches_exact(Modifiers::NONE) => {
                if *key == Key::Escape {
                    close = true;
                }
                if let Some(n) = [
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                ]
                .iter()
                .position(|k| k == key)
                {
                    choice = Some(n);
                }
                false
            }
            egui::Event::Key { .. } => false,
            _ => true,
        })
    });
    if close {
        ctx.data_mut(|d| d.remove::<Menu>(id()));
    } else if let Some(n) = choice {
        choose(app, ctx, &menu, n);
    }
    true
}
pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(menu) = ctx.data(|d| d.get_temp::<Menu>(id())) else {
        return;
    };
    if !valid(app, &menu) {
        ctx.data_mut(|d| d.remove::<Menu>(id()));
        return;
    }
    let mut chosen = None;
    let response = egui::Modal::new(id().with("modal")).show(ctx, |ui| {
        ui.set_width(260.0_f32.min((ctx.content_rect().width() - 64.0).max(100.0)));
        ui.heading(crate::i18n::gettext(
            app.locale,
            if menu.templates {
                "Quick Replies"
            } else {
                "React"
            },
        ));
        for (n, text) in menu.choices.iter().enumerate() {
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
            if response.hovered() || response.has_focus() {
                ui.painter()
                    .rect_filled(rect, 4.0, app.palette.surface_hover);
            }
            let label = format!("{}   {}", n + 1, text);
            let line = super::widgets::line(
                ui,
                &label,
                crate::theme::regular(18.0),
                app.palette.text,
                rect.width() - 20.0,
                1,
            );
            line.paint_decorative(
                ui,
                egui::pos2(rect.left() + 10.0, rect.center().y - line.size().y / 2.0),
                app.palette.text,
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if response.clicked() {
                chosen = Some(n);
            }
        }
    });
    if let Some(n) = chosen {
        choose(app, ctx, &menu, n);
    } else if response.should_close() {
        ctx.data_mut(|d| d.remove::<Menu>(id()));
    }
}
