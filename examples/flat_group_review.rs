use solmu::{
    app::App,
    model::Chat,
    paths::AppDirs,
    settings::Settings,
    ui::flat_groups::{self, member},
};
use std::time::Instant;
fn main() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
    let ctx = egui::Context::default();
    solmu::theme::install(&ctx);
    let n = 1000;
    app.chats = (0..n)
        .map(|i| {
            let mut c = Chat::new(format!("chat-{i}"), format!("Person {i}"));
            c.last_activity = (n - i) as i64;
            c.backend_origin = Some("origin".into());
            c.backend_account = Some(format!("network-{}", i % 2));
            c
        })
        .collect();
    let people = app
        .chats
        .chunks(2)
        .map(|pair| solmu::merged::Person {
            name: pair[0].name.clone(),
            members: pair
                .iter()
                .map(|chat| solmu::merged::Member::of(chat).unwrap())
                .collect(),
            primary: None,
        })
        .collect();
    app.settings.beeper.people = people;
    let keys: Vec<_> = app.chats.iter().map(|chat| member(&app, chat)).collect();
    for g in 0..100 {
        let id = app
            .settings
            .flat_groups
            .create(&format!("Group {g}"))
            .unwrap();
        for key in &keys {
            app.settings.flat_groups.add(id, key.clone());
        }
    }
    app.settings.flat_groups.grouped_view = true;
    let start = Instant::now();
    for _ in 0..10 {
        std::hint::black_box(app.visible_chats());
    }
    println!(
        "base visible_chats mean={:.2}ms",
        start.elapsed().as_secs_f64() * 100.
    );
    let start = Instant::now();
    for _ in 0..10 {
        std::hint::black_box(app.beeper_display_chats());
    }
    println!(
        "base beeper_display_chats mean={:.2}ms",
        start.elapsed().as_secs_f64() * 100.
    );
    let nav = Instant::now();
    for _ in 0..10 {
        let ordered = flat_groups::navigation_chats(&app);
        assert!(!ordered.is_empty());
        let ids: std::collections::HashSet<_> = ordered.iter().map(|c| &c.id).collect();
        assert_eq!(ids.len(), ordered.len());
        std::hint::black_box(ordered);
    }
    let nav_ms = nav.elapsed().as_secs_f64() * 1000. / 10.;
    let mut frames = Vec::new();
    for _ in 0..6 {
        let start = Instant::now();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360., 780.),
                )),
                ..Default::default()
            },
            |ui| flat_groups::list(&mut app, ui, false),
        );
        output.textures_delta.clear();
        frames.push(start.elapsed().as_secs_f64() * 1000.);
        assert!(
            app.flat_group_ui.built_rows < 20,
            "built {} alias widgets",
            app.flat_group_ui.built_rows
        );
    }
    println!(
        "100 groups x 1000 memberships, 500 linked people; navigation mean={nav_ms:.2}ms; six draw frames ms={frames:?}; last actual alias widgets={}",
        app.flat_group_ui.built_rows
    );
    app.scroll_chat_into_view = Some("chat-998".into());
    for _ in 0..4 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360., 780.),
                )),
                ..Default::default()
            },
            |ui| flat_groups::list(&mut app, ui, false),
        );
        output.textures_delta.clear();
    }
    assert!(app.scroll_chat_into_view.is_none());
    assert!(app.flat_group_ui.built_rows < 20);
    println!(
        "far keyboard reveal consumed, alias widgets={}",
        app.flat_group_ui.built_rows
    );
}
