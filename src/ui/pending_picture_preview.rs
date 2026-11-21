//! Inspect the staged copy without sending it or changing the original.
use crate::{
    app::{App, Pending},
    i18n::gettext,
    model::Action,
};
use egui::{Align, Layout, Rect, Sense, Vec2, pos2};

pub fn is_picture(item: &Pending) -> bool {
    match item {
        Pending::Picture { .. } => true,
        // Only what the loaders can show; anything else would open on an
        // error message.
        Pending::File(path) => Pending::is_preview_supported_file(path),
        Pending::Document(_) => false,
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(target) = app.pending_picture_preview else {
        return;
    };
    let pictures: Vec<_> = app
        .pending
        .iter()
        .filter(|row| is_picture(&row.item))
        .map(|row| row.id)
        .collect();
    let Some(index) = pictures.iter().position(|id| *id == target) else {
        app.pending_picture_preview = None;
        return;
    };
    let Some(at) = app.staged_at(target) else {
        return;
    };
    let mut image = None;
    let mut file = None;
    let name = match &app.pending[at].item {
        Pending::File(path) => {
            file = Some(path.clone());
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
        Pending::Picture {
            width,
            height,
            rgba,
            ..
        } => {
            let edit = crate::model::PictureEdit::new(
                target,
                crate::model::PictureSource::Pasted(rgba.clone()),
                *width as u32,
                *height as u32,
            );
            if let Some(handle) = super::image_editor::pasted_texture(app, ctx, &edit) {
                image = Some(egui::Image::from_texture((handle.id(), handle.size_vec2())));
            }
            gettext(app.locale, "Pasted picture").into_owned()
        }
        Pending::Document(_) => return,
    };
    let palette = app.palette;
    let locale = app.locale;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("pending-picture-preview"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, palette.window);
            let room = rect.shrink(28.0);
            let picture = Rect::from_min_max(room.min, pos2(room.right(), room.bottom() - 84.0));
            let image = file
                .as_ref()
                .map(|path| super::widgets::file_image(ui, path))
                .or(image);
            if let Some(image) = image {
                match image.load_for_size(ctx, picture.size()) {
                    Ok(egui::load::TexturePoll::Ready { texture }) => {
                        let size = texture.size;
                        let scale = (picture.width() / size.x.max(1.0))
                            .min(picture.height() / size.y.max(1.0))
                            .min(1.0);
                        image
                            .fit_to_exact_size(size * scale)
                            .paint_at(ui, Rect::from_center_size(picture.center(), size * scale));
                    }
                    Ok(egui::load::TexturePoll::Pending { .. }) => {
                        ui.put(
                            Rect::from_center_size(picture.center(), Vec2::splat(24.0)),
                            egui::Spinner::new(),
                        );
                    }
                    Err(_) => {
                        ui.put(
                            picture,
                            egui::Label::new(gettext(
                                locale,
                                "Could not display this picture. The staged file is unchanged.",
                            )),
                        );
                    }
                }
            }
            let controls = Rect::from_min_max(pos2(room.left(), picture.bottom() + 10.0), room.max);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(controls)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.add(
                        egui::Label::new(format!("{name} · {} / {}", index + 1, pictures.len()))
                            .truncate(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(index > 0, egui::Button::new(gettext(locale, "Previous")))
                            .clicked()
                        {
                            app.pending_picture_preview = Some(pictures[index - 1]);
                        }
                        if ui
                            .add_enabled(
                                index + 1 < pictures.len(),
                                egui::Button::new(gettext(locale, "Next")),
                            )
                            .clicked()
                        {
                            app.pending_picture_preview = Some(pictures[index + 1]);
                        }
                        // Pasted pictures crop from memory; staged files only
                        // when the editor can decode and rewrite them.
                        let croppable = file.as_deref().is_none_or(Pending::is_crop_supported_file);
                        if ui
                            .add_enabled(
                                croppable,
                                egui::Button::new(gettext(locale, "Crop / rotate")),
                            )
                            .clicked()
                        {
                            app.actions.push(Action::EditPicture(target));
                        }
                        if ui.button(gettext(locale, "Done")).clicked() {
                            app.pending_picture_preview = None;
                        }
                    });
                },
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_target_closes_and_preview_shortcuts_do_not_send() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let id = app.stage(Pending::Picture {
            width: 2,
            height: 2,
            rgba: std::sync::Arc::from(vec![255u8; 16]),
            texture: None,
        });
        app.pending_picture_preview = Some(id);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |_| super::super::keys::handle(&mut app, &ctx),
        );
        output.textures_delta.clear();
        assert!(app.actions.is_empty());
        // "go-bookmarks" is a real menu command: blocked during the preview,
        // back once it closes.
        assert!(super::super::keys::menu_actions(&app, "go-bookmarks").is_empty());
        app.pending.clear();
        show(&mut app, &ctx);
        assert!(app.pending_picture_preview.is_none());
        assert!(!super::super::keys::menu_actions(&app, "go-bookmarks").is_empty());
    }
    #[test]
    fn native_menu_actions_stay_out_of_an_owned_window() {
        use crate::model::Dialog;
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let guarded = vec![
            Action::ShowDialog(Dialog::NewChat),
            Action::OpenUrl("https://zapfast.rocks/using-zapfast/".into()),
            Action::OpenChatSearch,
            Action::FocusSearch,
            Action::ToggleSidebar,
            Action::ToggleInfoSidebar,
            Action::MenuCommand("go-bookmarks".into()),
        ];
        // The caret chords pass so the dispatcher can hand them to a focused
        // dialog field; menu_actions still blocks the background jump.
        let kept = vec![
            Action::Quit,
            Action::CloseWindow,
            Action::ShowWindow,
            Action::MenuCommand("chat-oldest".into()),
            Action::MenuCommand("chat-newest".into()),
        ];
        let mut all = guarded.clone();
        all.extend(kept.iter().cloned());
        // Nothing owns the window: every action passes through.
        assert_eq!(
            super::super::keys::guard_native_menu(&app, all.clone()),
            all
        );
        let id = app.stage(Pending::Picture {
            width: 2,
            height: 2,
            rgba: std::sync::Arc::from(vec![255u8; 16]),
            texture: None,
        });
        app.pending_picture_preview = Some(id);
        // The preview owns the window: only window management survives.
        assert_eq!(super::super::keys::guard_native_menu(&app, all), kept);
    }
    #[test]
    fn a_native_caret_chord_reaches_a_focused_dialog_field() {
        use crate::model::{Chat, Dialog};
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats.push(Chat::new("target".into(), "Target".into()));
        app.open_chat = Some("target".into());
        // The New Message dialog owns the window, and its draft field holds
        // focus with text in it.
        app.dialog = Some(Dialog::NewChat);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut draft = String::from("a draft");
        let field = egui::Id::new("new-message-draft");
        ctx.memory_mut(|memory| memory.request_focus(field));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.add(egui::TextEdit::multiline(&mut draft).id(field));
        });
        output.textures_delta.clear();
        // The native menu accelerator passes the guard...
        let actions = super::super::keys::guard_native_menu(
            &app,
            vec![Action::MenuCommand("chat-oldest".into())],
        );
        assert_eq!(actions, vec![Action::MenuCommand("chat-oldest".into())]);
        // ...and the dispatcher hands the chord back to the field instead of
        // jumping the chat behind the dialog.
        let mut jumped = false;
        let mut reached_field = false;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.add(egui::TextEdit::multiline(&mut draft).id(field));
            assert!(ui.ctx().text_edit_focused(), "the draft field lost focus");
            app.apply(Action::MenuCommand("chat-oldest".into()), ui.ctx());
            jumped = app
                .actions
                .iter()
                .any(|action| matches!(action, Action::ScrollPage(_)));
            reached_field = ui.ctx().input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::ArrowUp,
                            modifiers,
                            ..
                        } if modifiers.command
                    )
                })
            });
        });
        output.textures_delta.clear();
        assert!(!jumped, "the chord jumped the chat behind the dialog");
        assert!(reached_field, "the chord never reached the field");
    }
    #[test]
    fn full_view_preview_tab_and_send_chords_never_reach_background_composer() {
        use crate::model::{Chat, Dialog};
        for enter_sends in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let (mut app, _) = App::headless(
                crate::paths::AppDirs::under(root.path()),
                Default::default(),
            );
            app.link = crate::backend::LinkStatus::Connected;
            app.chats.push(Chat::new("target".into(), "Target".into()));
            app.open_chat = Some("target".into());
            app.settings.enter_sends = enter_sends;
            app.composer = "must stay unsent".into();
            let id = app.stage(Pending::Picture {
                width: 2,
                height: 2,
                rgba: std::sync::Arc::from(vec![255u8; 16]),
                texture: None,
            });
            app.pending_picture_preview = Some(id);
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
            for key in std::iter::repeat_n(egui::Key::Tab, 12).chain([egui::Key::Enter]) {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 760.0),
                        )),
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: if key == egui::Key::Enter && !enter_sends {
                                egui::Modifiers::COMMAND
                            } else {
                                egui::Modifiers::NONE
                            },
                        }],
                        ..Default::default()
                    },
                    |ui| super::super::show(&mut app, ui),
                );
                output.textures_delta.clear();
                assert!(!app.actions.iter().any(|action| matches!(
                    action,
                    Action::SendText { .. }
                        | Action::SendPending { .. }
                        | Action::ShowDialog(Dialog::SendLater)
                )));
                assert_eq!(app.composer, "must stay unsent");
                assert_eq!(app.pending.len(), 1);
            }
        }
    }
}
