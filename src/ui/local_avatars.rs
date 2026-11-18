//! Local avatar editing. Picking and decoding a file run away from the UI thread.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use egui::{Color32, Frame, Id, Margin, Vec2};

use crate::app::App;
use crate::local_avatars::{self, Appearance, Identity};
use crate::theme;

type PickResult = Result<Option<(String, Vec<u8>)>, String>;
type Registry = HashMap<String, (Appearance, std::path::PathBuf)>;

pub struct Editor {
    pub identities: Vec<Identity>,
    pub name: String,
    pub appearance: Appearance,
    image: Option<(String, Vec<u8>)>,
    pending: Option<Receiver<PickResult>>,
    error: Option<String>,
    saving: Option<Receiver<Result<(), String>>>,
}

fn identities(app: &App, id: &str) -> Vec<Identity> {
    if app.chats.iter().filter(|chat| chat.id == id).count() > 1 {
        return Vec::new();
    }
    let account = app.account().id.0.clone();
    let source = |id: &str| {
        let chat = app.chat(id);
        Identity {
            account: account.clone(),
            origin: chat.and_then(|chat| chat.backend_origin.clone()),
            backend_account: chat.and_then(|chat| chat.backend_account.clone()),
            chat: id.to_owned(),
        }
    };
    if let Some(chat) = app.chat(id)
        && let Some(person) = app
            .settings
            .beeper
            .people
            .iter()
            .find(|person| person.members.iter().any(|member| member.matches(chat)))
    {
        return person
            .avatar_members(&app.chats, chat)
            .iter()
            .map(|member| source(&member.chat))
            .collect();
    }
    vec![source(id)]
}

pub fn open(app: &mut App, id: &str, name: &str) {
    let identities = identities(app, id);
    if identities.is_empty() || app.chats.iter().filter(|chat| chat.id == id).count() > 1 {
        return;
    }
    let appearance = local_avatars::resolve(&app.settings.local_avatars, &identities)
        .cloned()
        .unwrap_or_else(|| Appearance::Initials {
            text: local_avatars::initials(&crate::util::initials(name)),
            colour: [115, 89, 165],
        });
    app.dialog = Some(crate::model::Dialog::CustomizeAvatar);
    app.local_avatar_editor = Some(Editor {
        identities,
        name: name.to_owned(),
        appearance,
        image: None,
        pending: None,
        error: None,
        saving: None,
    });
}

/// One active-account snapshot; never a global id-only preference map.
/// The shared painter covers favourites, compact rail, chat rows, headers and info.
pub fn install(app: &App, ctx: &egui::Context) {
    let mut registry = Registry::new();
    for entry in &app.settings.local_avatars {
        for key in &entry.identities {
            if key.account != app.account().id.0 {
                continue;
            }
            let keys = identities(app, &key.chat);
            // Source tuple changes do not inherit an old identity's override.
            if !keys.contains(key) {
                continue;
            }
            if let Some(appearance) = local_avatars::resolve(&app.settings.local_avatars, &keys) {
                for key in keys {
                    registry.insert(key.chat, (appearance.clone(), app.dirs.state.clone()));
                }
            }
        }
    }
    ctx.data_mut(|data| data.insert_temp(registry_id(), Arc::new(registry)));
}
fn registry_id() -> Id {
    Id::new("local-avatar-overrides")
}

/// Returns true when an override was painted. The source is still the fallback.
pub fn paint(ui: &egui::Ui, rect: egui::Rect, id: &str) -> bool {
    let found = ui.ctx().data(|data| {
        data.get_temp::<Arc<Registry>>(registry_id())
            .and_then(|map| map.get(id).cloned())
    });
    let Some((appearance, state)) = found else {
        return false;
    };
    paint_appearance(ui, rect, &appearance, &state)
}
fn paint_appearance(
    ui: &egui::Ui,
    rect: egui::Rect,
    appearance: &Appearance,
    state: &std::path::Path,
) -> bool {
    match appearance {
        Appearance::Initials { text, colour } => {
            let text = local_avatars::initials(text);
            if text.is_empty() {
                return false;
            }
            let colour = Color32::from_rgb(colour[0], colour[1], colour[2]);
            ui.painter()
                .circle_filled(rect.center(), rect.width() / 2.0, colour);
            // Keep initials readable even if a local preference file changes the colour.
            let ink = if theme::contrast(Color32::WHITE, colour)
                >= theme::contrast(Color32::BLACK, colour)
            {
                Color32::WHITE
            } else {
                Color32::BLACK
            };
            let galley = initials_galley(ui, rect.width(), &text, ink);
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, ink);
            true
        }
        Appearance::Image { file } => {
            let Some(path) = local_avatars::image_path(state, file) else {
                return false;
            };
            let image = super::widgets::file_image(ui, &path)
                .fit_to_exact_size(rect.size())
                .corner_radius(rect.width() / 2.0);
            if let Ok(egui::load::TexturePoll::Ready { .. }) =
                image.load_for_size(ui.ctx(), rect.size())
            {
                image.paint_at(ui, rect);
                true
            } else {
                false
            }
        }
    }
}

fn initials_galley(ui: &egui::Ui, diameter: f32, text: &str, ink: Color32) -> Arc<egui::Galley> {
    let mut font = theme::semibold(diameter * 0.38);
    let mut galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), ink);
    let width = diameter * 0.78;
    if galley.size().x > width {
        font.size *= width / galley.size().x;
        galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    }
    galley
}

fn apply(app: &mut App, editor: Editor, reset: bool) {
    app.dialog = None;
    local_avatars::replace(
        &mut app.settings.local_avatars,
        editor.identities,
        (!reset).then_some(editor.appearance),
    );
    app.mark_settings_dirty();
}

#[cfg(test)]
fn record(ctx: &egui::Context, name: &'static str, response: &egui::Response) {
    ctx.data_mut(|data| data.insert_temp(Id::new(name), response.rect));
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut editor) = app.local_avatar_editor.take() else {
        crate::image_cache::forget_prefix(ctx, "bytes://local-avatar-");
        app.dialog = None;
        return;
    };
    if editor
        .identities
        .first()
        .is_none_or(|key| key.account != app.account().id.0)
    {
        app.dialog = None;
        return; // An account switch must not expose or edit the previous account's draft.
    }
    if let Some(receiver) = editor.pending.as_ref() {
        match receiver.try_recv() {
            Ok(result) => {
                editor.pending = None;
                match result {
                    Ok(Some((file, bytes))) => {
                        crate::image_cache::forget_prefix(ctx, "bytes://local-avatar-");
                        editor.appearance = Appearance::Image { file: file.clone() };
                        editor.image = Some((file, bytes));
                        editor.error = None;
                    }
                    Ok(None) => {}
                    Err(error) => editor.error = Some(error),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                editor.pending = None;
                editor.error = Some("Image picker stopped".into());
            }
        }
    }
    if let Some(receiver) = &editor.saving {
        match receiver.try_recv() {
            Ok(Ok(())) => {
                if !editor
                    .identities
                    .iter()
                    .all(|key| identities(app, &key.chat).contains(key))
                {
                    app.dialog = None;
                    return;
                }
                crate::image_cache::forget_prefix(ctx, "bytes://local-avatar-");
                apply(app, editor, false);
                ctx.request_repaint();
                return;
            }
            Ok(Err(error)) => {
                editor.saving = None;
                editor.error = Some(error);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                editor.saving = None;
                editor.error = Some("Could not save avatar".into());
            }
        }
    }
    let palette = app.palette;
    let locale = app.locale;
    let mut save = false;
    let mut reset = false;
    let mut cancel = false;
    let frame = Frame::new()
        .fill(palette.overlay)
        .inner_margin(Margin::same(22))
        .corner_radius(12)
        .shadow(palette.modal_shadow());
    let response = egui::Modal::new(Id::new("local-avatar-editor")).frame(frame).show(ctx, |ui| {
        ui.set_width(420.0_f32.min((ctx.content_rect().width() - 64.0).max(180.0)));
        ui.heading(crate::i18n::gettext(locale, "Customize avatar"));
        ui.label(&editor.name);
        ui.label(crate::i18n::gettext(locale, "Only in Solmu. Not sent to contacts or Beeper."));
        ui.add_space(12.0);
        if editor.saving.is_some() || editor.pending.is_some() {
            ui.spinner();
            ui.label(if editor.saving.is_some() { crate::i18n::gettext(locale, "Saving avatar…") } else { crate::i18n::gettext(locale, "Preparing image…") });
            let button = ui.button(crate::i18n::gettext(locale, "Cancel"));
            #[cfg(test)] record(ctx, "local-avatar-cancel", &button);
            cancel = button.clicked();
            return;
        }
        ui.horizontal(|ui| {
            let (_, rect) = ui.allocate_space(Vec2::splat(110.0));
            let mut painted = false;
            if let (Appearance::Image { file }, Some((picked, bytes))) = (&editor.appearance, &editor.image)
                && file == picked
            {
                let uri = format!("bytes://local-avatar-{file}");
                crate::image_cache::include(ui.ctx(), &uri, bytes);
                egui::Image::new(uri).fit_to_exact_size(rect.size()).corner_radius(55.0).paint_at(ui, rect);
                painted = true;
            }
            if !painted { paint_appearance(ui, rect, &editor.appearance, &app.dirs.state); }
            ui.vertical(|ui| {
                if ui.add_enabled(editor.pending.is_none(), egui::Button::new(crate::i18n::gettext(locale, "Choose image…"))).clicked() {
                    let (send, receive) = mpsc::channel();
                    let context = ctx.clone();
                    editor.pending = Some(receive);
                    std::thread::spawn(move || {
                        let result = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif"]).pick_file().map(|path| local_avatars::import(&path)).transpose();
                        let _ = send.send(result);
                        context.request_repaint();
                    });
                }
                ui.label(crate::i18n::gettext(locale, "PNG, JPEG, WebP or GIF. Up to 10 MB."));
                ui.label(crate::i18n::gettext(locale, "Square centre crop. GIF uses its first frame."));
                if editor.pending.is_some() { ui.spinner(); }
            });
        });
        ui.add_space(12.0);
        if ui.selectable_label(matches!(editor.appearance, Appearance::Initials { .. }), crate::i18n::gettext(locale, "Use initials and colour")).clicked() && !matches!(editor.appearance, Appearance::Initials { .. }) {
            editor.appearance = Appearance::Initials { text: local_avatars::initials(&crate::util::initials(&editor.name)), colour: [115, 89, 165] };
        }
        if let Appearance::Initials { text, colour } = &mut editor.appearance {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::gettext(locale, "Initials"));
                ui.add(egui::TextEdit::singleline(text).char_limit(12).desired_width(80.0));
                *text = local_avatars::initials(text);
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::gettext(locale, "Colour"));
                for value in [[115, 89, 165], [61, 108, 118], [138, 84, 56], [95, 116, 55], [85, 91, 104]] {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::click());
                    ui.painter().circle_filled(rect.center(), 12.0, Color32::from_rgb(value[0], value[1], value[2]));
                    if *colour == value { ui.painter().circle_stroke(rect.center(), 15.0, egui::Stroke::new(2.0, palette.text)); }
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, *colour == value, format!("{}: #{:02x}{:02x}{:02x}", crate::i18n::gettext(locale, "Colour"), value[0], value[1], value[2])));
                    if response.clicked() || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Space) || input.key_pressed(egui::Key::Enter))) { *colour = value; }
                }
            });
        }
        if let Some(error) = &editor.error { ui.colored_label(palette.danger, error); }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let idle = editor.pending.is_none() && editor.saving.is_none();
            let reset_button = ui.add_enabled(idle, egui::Button::new(crate::i18n::gettext(locale, "Reset to default")));
            #[cfg(test)] record(ctx, "local-avatar-reset", &reset_button);
            reset = reset_button.clicked();
            let cancel_button = ui.button(crate::i18n::gettext(locale, "Cancel"));
            #[cfg(test)] record(ctx, "local-avatar-cancel", &cancel_button);
            cancel = cancel_button.clicked();
            let valid = !matches!(&editor.appearance, Appearance::Initials { text, .. } if local_avatars::initials(text).is_empty());
            let save_button = ui.add_enabled(valid && idle, egui::Button::new(egui::RichText::new(crate::i18n::gettext(locale, "Save")).color(palette.on_accent)).fill(palette.accent));
            #[cfg(test)] record(ctx, "local-avatar-save", &save_button);
            save = save_button.clicked();
            if editor.saving.is_some() { ui.spinner(); }
        });
    });
    if cancel || response.should_close() {
        crate::image_cache::forget_prefix(ctx, "bytes://local-avatar-");
        app.dialog = None;
        return;
    }
    if save || reset {
        // A backend identity replaced while the editor was open must not receive its draft.
        let valid = editor
            .identities
            .iter()
            .all(|key| identities(app, &key.chat).contains(key));
        if !valid {
            app.dialog = None;
            return;
        }
        if save
            && let Some((file, bytes)) = &editor.image
            && editor.appearance == (Appearance::Image { file: file.clone() })
        {
            let Some(path) = local_avatars::image_path(&app.dirs.state, file) else {
                return;
            };
            let bytes = bytes.clone();
            let (send, receive) = mpsc::channel();
            let context = ctx.clone();
            editor.saving = Some(receive);
            std::thread::spawn(move || {
                let result = fastframe_file::write_private(&path, &bytes)
                    .map_err(|_| "Could not save avatar".to_owned());
                let _ = send.send(result);
                context.request_repaint();
            });
            app.local_avatar_editor = Some(editor);
        } else {
            crate::image_cache::forget_prefix(ctx, "bytes://local-avatar-");
            apply(app, editor, reset);
        }
        ctx.request_repaint();
    } else {
        app.local_avatar_editor = Some(editor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    fn app() -> (tempfile::TempDir, App, egui::Context) {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        crate::demo::populate(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (root, app, ctx)
    }
    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.background_frame(ui.ctx());
                app.frame_ui(ui);
            },
        )
    }
    fn settle(app: &mut App, ctx: &egui::Context) {
        for _ in 0..4 {
            let mut output = frame(app, ctx, Vec::new());
            output.textures_delta.clear();
        }
    }
    fn click(app: &mut App, ctx: &egui::Context, name: &'static str) {
        settle(app, ctx);
        let at = ctx
            .data(|data| data.get_temp::<egui::Rect>(Id::new(name)))
            .expect(name)
            .center();
        for pressed in [true, false] {
            let mut output = frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        settle(app, ctx);
    }
    #[test]
    fn contact_info_entry_cancel_save_and_reset_drive_the_real_editor() {
        let (_root, mut app, ctx) = app();
        crate::demo::apply_flags(&mut app, Some("info"));
        click(&mut app, &ctx, "local-avatar-open");
        let editor = app
            .local_avatar_editor
            .as_mut()
            .expect("Contact info opens the editor");
        editor.appearance = Appearance::Initials {
            text: "AK".into(),
            colour: [115, 89, 165],
        };
        let preview_uri = format!("bytes://local-avatar-{}.png", "c".repeat(64));
        crate::image_cache::include(&ctx, &preview_uri, b"preview bytes");
        assert!(ctx.try_load_bytes(&preview_uri).is_ok());
        click(&mut app, &ctx, "local-avatar-cancel");
        assert!(
            ctx.try_load_bytes(&preview_uri).is_err(),
            "Cancel releases discarded preview resources"
        );
        assert!(app.local_avatar_editor.is_none());
        assert!(
            app.settings.local_avatars.is_empty(),
            "cancel cannot save a draft"
        );
        click(&mut app, &ctx, "local-avatar-open");
        app.local_avatar_editor.as_mut().unwrap().appearance = Appearance::Initials {
            text: "AK".into(),
            colour: [115, 89, 165],
        };
        click(&mut app, &ctx, "local-avatar-save");
        assert!(app.local_avatar_editor.is_none());
        assert_eq!(app.settings.local_avatars.len(), 1);
        let chat = app.open_chat.clone().unwrap();
        let source = app.avatar(&chat);
        install(&app, &ctx);
        assert!(ctx.data(|data| {
            data.get_temp::<Arc<Registry>>(registry_id())
                .unwrap()
                .contains_key(&chat)
        }));
        let persisted = serde_json::to_vec(&app.settings).unwrap();
        let restored: Settings = serde_json::from_slice(&persisted).unwrap();
        assert_eq!(restored.local_avatars, app.settings.local_avatars);
        click(&mut app, &ctx, "local-avatar-open");
        click(&mut app, &ctx, "local-avatar-reset");
        assert!(app.settings.local_avatars.is_empty());
        install(&app, &ctx);
        assert!(ctx.data(|data| {
            data.get_temp::<Arc<Registry>>(registry_id())
                .unwrap()
                .is_empty()
        }));
        assert_eq!(
            source,
            app.avatar(&chat),
            "reset does not replace source caches"
        );
    }
    #[test]
    fn saved_image_is_prepared_before_settings_and_cancelled_picks_cannot_apply() {
        let (_root, mut app, ctx) = app();
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Alex Korpi");
        let bytes = crate::local_avatars::normalize(&crate::demo::sample_thumbnail(1)).unwrap();
        let file = format!("{}.png", "a".repeat(64));
        let editor = app.local_avatar_editor.as_mut().unwrap();
        editor.appearance = Appearance::Image { file: file.clone() };
        editor.image = Some((file.clone(), bytes.clone()));
        click(&mut app, &ctx, "local-avatar-save");
        for _ in 0..100 {
            if app.local_avatar_editor.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            settle(&mut app, &ctx);
        }
        assert!(app.local_avatar_editor.is_none());
        let path = local_avatars::image_path(&app.dirs.state, &file).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(matches!(
            local_avatars::resolve(&app.settings.local_avatars, &identities(&app, &chat)),
            Some(Appearance::Image { .. })
        ));
        open(&mut app, &chat, "Alex Korpi");
        let (send, receive) = mpsc::channel();
        app.local_avatar_editor.as_mut().unwrap().pending = Some(receive);
        click(&mut app, &ctx, "local-avatar-cancel");
        assert!(
            send.send(Ok(Some((file, Vec::new())))).is_err(),
            "cancel drops the pending picker route"
        );
        assert_eq!(app.settings.local_avatars.len(), 1);
    }
    #[test]
    fn linked_avatar_is_shared_across_views_and_stale_source_replacements_are_excluded() {
        let (_root, mut app, ctx) = app();
        crate::demo::apply_flags(&mut app, Some("linked"));
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Alex Korpi");
        let mut editor = app.local_avatar_editor.take().unwrap();
        assert!(editor.identities.len() > 1);
        let aliases = editor
            .identities
            .iter()
            .map(|key| key.chat.clone())
            .collect::<Vec<_>>();
        editor.appearance = Appearance::Initials {
            text: "AK".into(),
            colour: [115, 89, 165],
        };
        apply(&mut app, editor, false);
        for page in ["info", "rail"] {
            crate::demo::apply_flags(&mut app, Some(page));
            settle(&mut app, &ctx);
            let map = ctx
                .data(|data| data.get_temp::<Arc<Registry>>(registry_id()))
                .unwrap();
            for alias in &aliases {
                assert!(
                    map.contains_key(alias),
                    "{page}: all linked aliases get the same local override"
                );
            }
        }
        // The same visible id at a replaced origin is not the same person.
        for source in &mut app.chats {
            if aliases.contains(&source.id) {
                source.backend_origin = Some("replacement".into());
            }
        }
        install(&app, &ctx);
        assert!(ctx.data(|data| {
            data.get_temp::<Arc<Registry>>(registry_id())
                .unwrap()
                .is_empty()
        }));
    }
}

#[cfg(test)]
mod write_failure_tests {
    use super::*;
    #[test]
    fn failed_image_write_keeps_editor_and_previous_preferences() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        crate::demo::populate(&mut app);
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Alex Korpi");
        let editor = app.local_avatar_editor.as_mut().unwrap();
        let (send, receive) = mpsc::channel();
        editor.saving = Some(receive);
        send.send(Err("Could not save avatar".into())).unwrap();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_ui| show(&mut app, &ctx));
        output.textures_delta.clear();
        assert!(
            app.settings.local_avatars.is_empty(),
            "a failed asset write cannot publish its reference"
        );
        let editor = app
            .local_avatar_editor
            .as_ref()
            .expect("failure leaves draft open for retry");
        assert!(editor.error.is_some());
        assert!(editor.saving.is_none());
    }
}

#[cfg(test)]
mod modal_input_tests {
    use super::*;
    #[test]
    fn editor_paste_cannot_change_the_background_composer() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        crate::demo::populate(&mut app);
        app.composer = "Keep this draft unchanged".into();
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Alex Korpi");
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for events in [Vec::new(), vec![egui::Event::Paste("not a message".into())]] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.background_frame(ui.ctx());
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(app.composer, "Keep this draft unchanged");
        assert!(app.picture_edit.is_none());
        assert!(app.pending_media.is_empty());
        assert!(app.local_avatar_editor.is_some());
    }
}

#[cfg(test)]
mod dismissed_dialog_tests {
    use super::*;
    #[test]
    fn externally_closed_dialog_discards_its_draft_and_unblocks_navigation() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        crate::demo::populate(&mut app);
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Alex Korpi");
        app.dialog = None;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.frame_ui(ui));
        output.textures_delta.clear();
        assert!(app.local_avatar_editor.is_none());
        assert!(app.settings.local_avatars.is_empty());
    }
}

#[cfg(test)]
mod initials_layout_tests {
    use super::*;
    #[test]
    fn three_wide_initials_fit_the_sidebar_and_editor_circles() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for diameter in [28.0, 40.0, 110.0] {
                for text in ["WWW", "漢字名", "AK"] {
                    let galley = initials_galley(ui, diameter, text, Color32::WHITE);
                    assert!(galley.size().x <= diameter * 0.8, "{text} at {diameter}");
                }
            }
        });
        output.textures_delta.clear();
    }
}

#[cfg(test)]
mod preview_cleanup_tests {
    use super::*;
    #[test]
    fn closing_avatar_editor_releases_registered_preview_bytes() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        crate::demo::populate(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        open(&mut app, &chat, "Ada");
        let uri = format!("bytes://local-avatar-{}.png", "a".repeat(64));
        crate::image_cache::include(&ctx, &uri, b"preview");
        assert!(ctx.try_load_bytes(&uri).is_ok());
        app.dialog = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.frame_ui(ui));
        output.textures_delta.clear();
        assert!(ctx.try_load_bytes(&uri).is_err());
    }
}
