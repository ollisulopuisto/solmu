use solmu::{
    app::App,
    model::{Action, Chat, ChatFilter, ChatKind},
    paths::AppDirs,
    settings::Settings,
};

fn frame(app: &mut App, ctx: &egui::Context) {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 780.0),
            )),
            ..Default::default()
        },
        |ui| {
            app.background_frame(ui.ctx());
            app.frame_ui(ui);
        },
    );
    out.textures_delta.clear();
}
#[test]
fn composed_scope_survives_actions_and_search() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
    let ctx = egui::Context::default();
    app.attach(&ctx);
    let mut group = Chat::new("group@g.us".into(), "Needle group".into());
    group.kind = ChatKind::Group;
    group.unread = 1;
    let direct = Chat::new("peer@s.whatsapp.net".into(), "Needle peer".into());
    let mut locked = Chat::new("secret@s.whatsapp.net".into(), "Needle secret".into());
    locked.locked = true;
    app.chats = vec![group, direct, locked];
    app.actions.push(Action::SetChatFilter(ChatFilter::Unread));
    app.actions.push(Action::SetChatFilter(ChatFilter::Groups));
    frame(&mut app, &ctx);
    assert_eq!(app.chat_filter, ChatFilter::Unread);
    assert_eq!(app.category_filter, Some(ChatFilter::Groups));
    app.search = "needle".into();
    assert_eq!(app.visible_chats().len(), 1);
    assert_eq!(app.outside_scope_matches(), 1);
    app.clear_chat_scope();
    assert_eq!(app.search, "needle");
    assert_eq!(app.visible_chats().len(), 2);
    assert_eq!(app.outside_scope_matches(), 0);
}

#[test]
fn footer_stays_at_bottom_with_zero_results() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
    let ctx = egui::Context::default();
    app.attach(&ctx);
    app.page = solmu::model::Page::Chats;
    app.link = solmu::backend::LinkStatus::Connected;
    app.search = "no results".into();
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    let rect = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("new-message-footer")))
        .unwrap();
    assert!(rect.top() > 700.0 && rect.bottom() <= 780.0, "{rect:?}");
}

fn category_app(ctx: &egui::Context) -> App {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
    app.attach(ctx);
    let now = solmu::util::now();
    let mut recent = Chat::new("recent@s.whatsapp.net".into(), "Recent".into());
    recent.last_activity = now - 3600;
    let mut quiet = Chat::new("quiet@s.whatsapp.net".into(), "Quiet".into());
    quiet.last_activity = now - 200 * 24 * 3600;
    let mut channel = Chat::new("news@newsletter".into(), "News".into());
    channel.kind = ChatKind::Broadcast;
    channel.last_activity = now - 3600;
    let mut group = Chat::new("group@g.us".into(), "Group".into());
    group.kind = ChatKind::Group;
    group.last_activity = now - 3600;
    app.chats = vec![recent, quiet, channel, group];
    app
}

fn visible_names(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app
        .visible_chats()
        .iter()
        .map(|chat| chat.name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn automatic_category_filter_lists_only_that_category() {
    use solmu::categories::AutoCategory;
    let ctx = egui::Context::default();
    let mut app = category_app(&ctx);
    // The All list leaves channels out, as the official apps do.
    assert_eq!(visible_names(&app), vec!["Group", "Quiet", "Recent"]);
    for (category, expected) in [
        (AutoCategory::People, vec!["Recent"]),
        (AutoCategory::Quiet, vec!["Quiet"]),
        (AutoCategory::Notifications, vec!["News"]),
    ] {
        app.actions.push(Action::SelectAutoCategory(Some(category)));
        frame(&mut app, &ctx);
        assert_eq!(visible_names(&app), expected, "{category:?}");
    }
    app.actions.push(Action::SelectAutoCategory(None));
    frame(&mut app, &ctx);
    assert_eq!(visible_names(&app), vec!["Group", "Quiet", "Recent"]);
}

#[test]
fn a_chat_moved_by_hand_stays_put_and_automatic_restores_the_rules() {
    use solmu::categories::AutoCategory;
    let ctx = egui::Context::default();
    let mut app = category_app(&ctx);
    app.actions.push(Action::SetChatCategory(
        "quiet@s.whatsapp.net".into(),
        Some(AutoCategory::People),
    ));
    frame(&mut app, &ctx);
    app.actions
        .push(Action::SelectAutoCategory(Some(AutoCategory::People)));
    frame(&mut app, &ctx);
    assert_eq!(visible_names(&app), vec!["Quiet", "Recent"]);
    // The override is saved, so a restart keeps it.
    let saved = serde_json::to_string(&app.settings.beeper).unwrap();
    let loaded: solmu::settings::BeeperPreferences = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.category_overrides.len(), 1);
    app.actions
        .push(Action::SetChatCategory("quiet@s.whatsapp.net".into(), None));
    frame(&mut app, &ctx);
    assert_eq!(visible_names(&app), vec!["Recent"]);
    assert!(app.settings.beeper.category_overrides.is_empty());
}

#[test]
fn groups_cannot_be_moved_into_a_category_and_reset_clears_the_category() {
    use solmu::categories::AutoCategory;
    let ctx = egui::Context::default();
    let mut app = category_app(&ctx);
    app.actions.push(Action::SetChatCategory(
        "group@g.us".into(),
        Some(AutoCategory::Notifications),
    ));
    frame(&mut app, &ctx);
    assert!(app.settings.beeper.category_overrides.is_empty());
    app.actions
        .push(Action::SelectAutoCategory(Some(AutoCategory::Quiet)));
    frame(&mut app, &ctx);
    assert!(app.search_scope_active());
    app.clear_chat_scope();
    assert_eq!(app.auto_category_filter, None);
    assert!(!app.search_scope_active());
}
