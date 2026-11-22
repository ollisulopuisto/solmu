//! Explicit destructive batch review and per-item outcomes. No automatic retries.
use crate::{
    app::App,
    i18n::{Locale, gettext, ngettext},
    model::{Action, Chat},
};

pub use crate::model::{BatchItem as Item, BatchState as State};

fn technical_details(
    ui: &mut egui::Ui,
    label: &str,
    section: &str,
    identity: impl std::hash::Hash + std::fmt::Debug,
    error: &str,
) -> egui::collapsing_header::CollapsingResponse<()> {
    egui::CollapsingHeader::new(label)
        .id_salt((section, identity))
        .show(ui, |ui| {
            ui.add(egui::Label::new(error).wrap());
        })
}

pub fn next_request() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn toggle_chat(app: &mut App, chat: &str) {
    let owner = app.account().id.0.clone();
    if app.batch_ui.owner != owner {
        if app
            .batch_ui
            .results
            .iter()
            .any(|item| item.result.is_none())
        {
            return;
        }
        app.batch_ui = State {
            owner,
            ..Default::default()
        };
    }
    if app.batch_ui.selected_chats.iter().any(|id| id == chat) {
        app.batch_ui.selected_chats.retain(|id| id != chat);
        app.batch_ui.selected_identities.remove(chat);
    } else {
        if let Some(identity) = app.chat(chat).and_then(crate::merged::Member::of) {
            app.batch_ui
                .selected_identities
                .insert(chat.into(), identity);
        }
        app.batch_ui.selected_chats.push(chat.into());
    }
}

pub fn archive_bar(app: &mut App, ui: &mut egui::Ui) {
    // Isolate the header's IDs without adding spacing when this bar is empty.
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("batch-action-bar"));
    archive_bar_contents(app, &mut child);
    let rect = child.min_rect();
    drop(child);
    if rect.is_positive() {
        ui.advance_cursor_after_rect(rect);
    } else {
        // Reserve the same parent ID as advance_cursor_after_rect would use.
        ui.skip_ahead_auto_ids(1);
    }
}

fn archive_bar_contents(app: &mut App, ui: &mut egui::Ui) {
    super::ghost_groups::bar(app, ui);
    if app.batch_ui.owner != app.account().id.0 {
        return;
    }
    // A receipt notice is useful briefly; it is not a persistent task or
    // proof that every remote participant has received the deletion.
    if let Some(wait) = app
        .batch_ui
        .expire_delete_notices(std::time::Instant::now())
    {
        ui.ctx().request_repaint_after(wait);
    }
    if !app.batch_ui.selected_chats.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                ngettext(
                    app.locale,
                    "{count} chat selected",
                    "{count} chats selected",
                    app.batch_ui.selected_chats.len() as u32,
                )
                .replace("{count}", &app.batch_ui.selected_chats.len().to_string()),
            );
            if ui
                .button(crate::i18n::gettext(app.locale, "Archive selected…"))
                .clicked()
            {
                app.batch_ui.reviewing_archive = true;
            }
            if ui
                .button(crate::i18n::gettext(app.locale, "Clear selection"))
                .clicked()
            {
                app.batch_ui.selected_chats.clear();
                app.batch_ui.selected_identities.clear();
            }
        });
    }
    for item in &app.batch_ui.results {
        ui.add(egui::Label::new(result_line(app, item)).wrap());
        if let Some(Err(error)) = &item.result {
            technical_details(
                ui,
                &gettext(app.locale, "Technical details"),
                "batch-current",
                (
                    &app.batch_ui.owner,
                    app.batch_ui.request,
                    &item.chat,
                    &item.message,
                    item.everyone,
                ),
                error,
            );
        }
    }
    if !app.batch_ui.previous_session_results.is_empty() {
        ui.label(gettext(
            app.locale,
            "Unconfirmed or failed outcomes from a previous connection (not the current session)",
        ));
        for (index, item) in app.batch_ui.previous_session_results.iter().enumerate() {
            ui.label(
                gettext(
                    app.locale,
                    "{chat} {message}: check the previous connection before retrying",
                )
                .replace("{chat}", &gettext(app.locale, "Previous connection chat"))
                .replace("{message}", &result_action(app.locale, item)),
            );
            if let Some(Err(error)) = &item.result {
                technical_details(
                    ui,
                    &gettext(app.locale, "Technical details"),
                    "batch-previous",
                    index,
                    error,
                );
            }
        }

        if ui
            .scope_builder(
                egui::UiBuilder::new()
                    .id(ui.make_persistent_id(("dismiss-previous-results", &app.batch_ui.owner))),
                |ui| ui.button(gettext(app.locale, "Dismiss previous connection history")),
            )
            .inner
            .clicked()
        {
            app.batch_ui.previous_session_results.clear();
        }
    }
    if !app.batch_ui.results.is_empty()
        && app
            .batch_ui
            .results
            .iter()
            .all(|item| item.result.is_some())
        && ui
            .scope_builder(
                egui::UiBuilder::new().id(ui.make_persistent_id((
                    "dismiss-current-results",
                    &app.batch_ui.owner,
                    app.batch_ui.request,
                ))),
                |ui| ui.button(gettext(app.locale, "Dismiss action results")),
            )
            .inner
            .clicked()
    {
        app.batch_ui.results.clear();
    }
}

fn result_chat_name(app: &App, item: &Item) -> String {
    let title = app.chat(&item.chat).map(|chat| app.chat_title(chat));
    title
        .filter(|name| !name.trim().is_empty() && name != &item.chat)
        .unwrap_or_else(|| gettext(app.locale, "Unavailable chat").into_owned())
}

fn result_action(locale: Locale, item: &Item) -> std::borrow::Cow<'static, str> {
    match (item.message.is_some(), item.everyone) {
        (false, _) => gettext(locale, "Archive chat"),
        (true, false) => gettext(locale, "Delete message for me"),
        (true, true) => gettext(locale, "Delete message for everyone"),
    }
}

fn result_line(app: &App, item: &Item) -> String {
    let status = match &item.result {
        None => gettext(app.locale, "Waiting for confirmation; do not retry"),
        Some(Ok(())) => gettext(app.locale, "Request accepted, completion not confirmed"),
        Some(Err(_)) => gettext(
            app.locale,
            "Failed or uncertain; check the chat before retrying",
        ),
    };
    let action = result_action(app.locale, item);
    format!("{}: {action}. {status}", result_chat_name(app, item))
}

pub fn archive_refusal(chat: &Chat) -> Option<&'static str> {
    (chat.mutation_capabilities.archive != Some(true))
        .then_some("Archive support not confirmed by this chat's bridge")
}

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    if !app.batch_ui.reviewing_archive {
        return;
    }
    if app.batch_ui.owner != app.account().id.0 {
        app.batch_ui.reviewing_archive = false;
        return;
    }
    let ids = app.batch_ui.selected_chats.clone();
    let refusals: Vec<_> = ids
        .iter()
        .filter_map(|id| match app.chat(id) {
            None => Some(gettext(app.locale, "{id}: chat unavailable").replace("{id}", id)),
            Some(chat)
                if app
                    .batch_ui
                    .selected_identities
                    .get(id)
                    .is_none_or(|identity| !identity.matches(chat)) =>
            {
                Some(
                    gettext(
                        app.locale,
                        "Chat identity changed or is unavailable; clear the selection",
                    )
                    .into_owned(),
                )
            }
            Some(chat) => archive_refusal(chat).map(|why| {
                format!(
                    "{}: {}",
                    app.chat_title(chat),
                    refusal_text(app.locale, why)
                )
            }),
        })
        .collect();
    let pending = app
        .batch_ui
        .results
        .iter()
        .any(|item| item.result.is_none());
    egui::Modal::new(egui::Id::new("batch-archive-review")).show(ctx, |ui| {
        ui.heading(
            ngettext(
                app.locale,
                "Archive {count} chat?",
                "Archive {count} chats?",
                ids.len() as u32,
            )
            .replace("{count}", &ids.len().to_string()),
        );
        ui.label(crate::i18n::gettext(
            app.locale,
            "They remain available in Archived. Messages are not deleted.",
        ));
        for refusal in &refusals {
            ui.label(refusal);
        }
        if !refusals.is_empty() {
            ui.label(crate::i18n::gettext(
                app.locale,
                "Nothing will be sent. Remove unsupported chats from the selection first.",
            ));
        }
        if pending {
            ui.label(crate::i18n::gettext(
                app.locale,
                "An earlier batch still has pending results.",
            ));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !pending && refusals.is_empty() && !ids.is_empty(),
                    egui::Button::new(crate::i18n::gettext(app.locale, "Archive")),
                )
                .clicked()
            {
                let request = next_request();
                app.batch_ui.request = Some(request);
                app.batch_ui.results = ids
                    .iter()
                    .map(|chat| Item {
                        chat: chat.clone(),
                        message: None,
                        everyone: false,
                        accepted_delete_at: None,
                        result: None,
                    })
                    .collect();
                for chat in &ids {
                    app.actions.push(Action::BatchArchive {
                        request,
                        expected: app.batch_ui.selected_identities[chat].clone(),
                        chat: chat.clone(),
                    });
                }
                app.batch_ui.selected_chats.clear();
                app.batch_ui.selected_identities.clear();
                app.batch_ui.reviewing_archive = false;
            }
            if ui
                .button(crate::i18n::gettext(app.locale, "Cancel"))
                .clicked()
            {
                app.batch_ui.reviewing_archive = false;
            }
        });
    });
}

/// Keep refusal literals visible to gettext extraction, without translating backend logic.
pub fn refusal_text(locale: Locale, reason: &str) -> String {
    match reason {
        "Archive support not confirmed by this chat's bridge" => gettext(
            locale,
            "Archive support not confirmed by this chat's bridge",
        )
        .into_owned(),
        "Delete for me is not confirmed by this bridge" => {
            gettext(locale, "Delete for me is not confirmed by this bridge").into_owned()
        }
        "Delete for everyone is not confirmed by this bridge" => gettext(
            locale,
            "Delete for everyone is not confirmed by this bridge",
        )
        .into_owned(),
        "Only your own messages can be deleted for everyone here" => gettext(
            locale,
            "Only your own messages can be deleted for everyone here",
        )
        .into_owned(),
        "Message age could not be verified" => {
            gettext(locale, "Message age could not be verified").into_owned()
        }
        "The bridge's delete age limit has passed" => {
            gettext(locale, "The bridge's delete age limit has passed").into_owned()
        }
        _ => reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch_bar_frame(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Pos2)> {
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_owned(), t.pos + t.galley.size() / 2.0))
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        texts(item, out);
                    }
                }
                _ => {}
            }
        }
        let mut input = egui::RawInput {
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .inner_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(520.0, 700.0),
        ));
        let mut output = ctx.run_ui(input, |ui| {
            archive_bar(app, ui);
            // Model a following header control without giving it an explicit ID.
            let _ = ui.button("After the bar");
        });
        let mut result = Vec::new();
        for shape in &output.shapes {
            texts(&shape.shape, &mut result);
        }
        output.textures_delta.clear();
        result
    }

    #[test]
    fn expiring_a_receipt_keeps_the_same_failure_details_open() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let now = std::time::Instant::now();
        let item = |id: &str, result, accepted_delete_at| Item {
            chat: "synthetic".into(),
            message: Some(id.into()),
            everyone: true,
            result: Some(result),
            accepted_delete_at,
        };
        app.batch_ui = State {
            owner: app.account().id.0.clone(),
            request: Some(77),
            results: vec![
                item("accepted", Ok(()), Some(now)),
                item("first", Err("FIRST_FAILURE_DETAIL".into()), None),
                item("second", Err("SECOND_FAILURE_DETAIL".into()), None),
            ],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.interaction.selectable_labels = false;
        });
        let label = gettext(app.locale, "Technical details").into_owned();
        let initial = batch_bar_frame(&mut app, &ctx, vec![]);
        let point = initial.iter().find(|(text, _)| text == &label).unwrap().1;
        batch_bar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        batch_bar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let open = batch_bar_frame(&mut app, &ctx, vec![]);
        assert!(open.iter().any(|(text, _)| text == "FIRST_FAILURE_DETAIL"));
        assert!(!open.iter().any(|(text, _)| text == "SECOND_FAILURE_DETAIL"));
        app.batch_ui.results[0].accepted_delete_at = Some(now - std::time::Duration::from_secs(11));
        let after = batch_bar_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.batch_ui.results.len(), 2);
        assert!(
            after.iter().any(|(text, _)| text == "FIRST_FAILURE_DETAIL"),
            "the same failure must stay expanded"
        );
        assert!(
            !after
                .iter()
                .any(|(text, _)| text == "SECOND_FAILURE_DETAIL"),
            "expansion must not move to another failure"
        );
    }

    #[test]
    fn expiring_a_receipt_preserves_dismiss_and_following_control_focus() {
        let mut moved = Vec::new();
        for (target, retain_failure) in [
            ("Dismiss action results", true),
            ("Dismiss previous connection history", true),
            ("After the bar", true),
            ("After the bar", false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (mut app, _) = App::headless(
                crate::paths::AppDirs::under(root.path()),
                Default::default(),
            );
            let now = std::time::Instant::now();
            let item = |id: &str, accepted: bool| Item {
                chat: "synthetic".into(),
                message: Some(id.into()),
                everyone: true,
                result: Some(if accepted {
                    Ok(())
                } else {
                    Err("synthetic error".into())
                }),
                accepted_delete_at: accepted.then_some(now),
            };
            app.batch_ui = State {
                owner: app.account().id.0.clone(),
                request: Some(77),
                results: if retain_failure {
                    vec![item("accepted", true), item("failed", false)]
                } else {
                    vec![item("accepted", true)]
                },
                previous_session_results: if retain_failure {
                    vec![item("previous", false)]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let label = if target == "After the bar" {
                target.into()
            } else {
                gettext(app.locale, target).into_owned()
            };
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| {
                style.animation_time = 0.0;
                style.interaction.selectable_labels = false;
            });
            let mut focused = None;
            for _ in 0..16 {
                let frame = batch_bar_frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
                let point = frame.iter().find(|(text, _)| text == &label).unwrap().1;
                if let Some(id) = ctx.memory(|m| m.focused())
                    && ctx
                        .read_response(id)
                        .is_some_and(|response| response.rect.contains(point))
                {
                    focused = Some(id);
                    break;
                }
            }
            let focused = focused.expect("Tab must reach the requested control before expiry");
            app.batch_ui.results[0].accepted_delete_at =
                Some(now - std::time::Duration::from_secs(11));
            batch_bar_frame(&mut app, &ctx, vec![]);
            // egui hit-testing reads previous-pass rectangles; settle the new layout.
            let after = batch_bar_frame(&mut app, &ctx, vec![]);
            assert_eq!(app.batch_ui.results.len(), usize::from(retain_failure));
            let point = after.iter().find(|(text, _)| text == &label).unwrap().1;
            if ctx.memory(|m| m.focused()) != Some(focused)
                || !ctx
                    .read_response(focused)
                    .is_some_and(|response| response.rect.contains(point))
            {
                moved.push((target, retain_failure));
            }
        }
        assert!(
            moved.is_empty(),
            "focus left or transferred from these controls after expiry: {moved:?}"
        );
    }

    #[test]
    fn an_empty_batch_bar_does_not_add_header_spacing() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let before = ui.cursor().min;
            archive_bar(&mut app, ui);
            assert_eq!(
                ui.cursor().min,
                before,
                "an empty bar must not move the following header control"
            );
        });
        output.textures_delta.clear();
    }

    #[test]
    fn technical_detail_expansion_is_independent_between_items_and_sessions() {
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        let mut open = Vec::new();
        let frame = |ctx: &egui::Context, ids: &mut Vec<egui::Id>, open: &mut Vec<bool>| {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.vertical(|ui| {
                    for scope in ["batch-current", "batch-previous"] {
                        for index in 0..2 {
                            let response =
                                technical_details(ui, "Technical details", scope, index, "error");
                            ids.push(response.header_response.id);
                            open.push(response.body_returned.is_some());
                        }
                    }
                });
            });
            output.textures_delta.clear();
        };
        frame(&ctx, &mut ids, &mut open);
        let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
        assert_eq!(unique.len(), 4);
        // Store the explicit state exactly as clicking the first header does.
        let mut state =
            egui::collapsing_header::CollapsingState::load_with_default_open(&ctx, ids[0], false);
        state.set_open(true);
        state.store(&ctx);
        ids.clear();
        open.clear();
        frame(&ctx, &mut ids, &mut open);
        assert_eq!(open, [true, false, false, false]);
    }

    #[test]
    fn result_names_action_and_state_never_internal_identifiers() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(root.path()),
            Default::default(),
        );
        app.chats.push(Chat::new(
            "!raw:bridge.localhost".into(),
            "Ada's group".into(),
        ));
        app.chats[0].kind = crate::model::ChatKind::Group;
        let mut item = Item {
            chat: "!raw:bridge.localhost".into(),
            message: Some("402555".into()),
            everyone: false,
            accepted_delete_at: None,
            result: Some(Ok(())),
        };
        let line = result_line(&app, &item);
        assert!(line.contains("Ada's group"));
        assert!(line.contains("Delete message for me"));
        assert!(line.contains("completion not confirmed"));
        assert!(!line.contains("localhost"));
        assert!(!line.contains("402555"));
        item.everyone = true;
        assert!(result_line(&app, &item).contains("for everyone"));
        item.message = None;
        assert!(result_line(&app, &item).contains("Archive chat"));
        item.result = None;
        assert!(result_line(&app, &item).contains("Waiting for confirmation"));
        item.result = Some(Err("raw internal failure".into()));
        assert!(result_line(&app, &item).contains("Failed or uncertain"));
        item.chat = "gone.internal".into();
        assert!(result_line(&app, &item).starts_with("Unavailable chat"));
    }
}
