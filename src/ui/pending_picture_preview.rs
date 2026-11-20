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
        Pending::File(path) => Pending::is_picture_file(path),
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
                    ui.label(format!("{name} · {} / {}", index + 1, pictures.len()));
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
                        if ui.button(gettext(locale, "Crop / rotate")).clicked() {
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
        assert!(super::super::keys::menu_actions(&app, "send").is_empty());
        app.pending.clear();
        show(&mut app, &ctx);
        assert!(app.pending_picture_preview.is_none());
    }
}
