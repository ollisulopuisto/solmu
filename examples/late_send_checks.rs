//! A late send failure must retain old text without replacing a newer draft.
use solmu::{
    app::App,
    backend::{Event, LinkStatus},
    model::Chat,
    paths::AppDirs,
    settings::Settings,
};
fn main() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
    let ctx = egui::Context::default();
    app.attach(&ctx);
    app.link = LinkStatus::Connected;
    app.chats.push(Chat::new("work".into(), "Fixture".into()));
    app.open_chat = Some("work".into());
    app.composer = "newer draft".into();
    events
        .send(Event::BeeperSendUncertain {
            chat: "work".into(),
            text: "older failed text".into(),
            message: "Delivery unconfirmed".into(),
        })
        .unwrap();
    app.background_frame(&ctx);
    assert_eq!(app.composer, "newer draft");
    assert!(
        app.toasts
            .iter()
            .any(|t| t.message.contains("older failed text")),
        "failed text must remain available for copying"
    );
    app.open_chat = Some("other".into());
    app.drafts
        .insert("work".into(), "newer parked draft".into());
    events
        .send(Event::BeeperSendUncertain {
            chat: "work".into(),
            text: "parked failed text".into(),
            message: "Delivery unconfirmed".into(),
        })
        .unwrap();
    app.background_frame(&ctx);
    assert_eq!(app.drafts["work"], "newer parked draft");
    assert!(
        app.toasts
            .iter()
            .any(|t| t.message.contains("parked failed text"))
    );
    events
        .send(Event::BeeperFilesUncertain {
            chat: "work".into(),
            paths: vec![std::path::PathBuf::from("fixture.txt")],
            documents: true,
            caption: Some("failed caption".into()),
            quoting: None,
            message: "Attachment unconfirmed".into(),
        })
        .unwrap();
    app.background_frame(&ctx);
    assert_eq!(app.drafts["work"], "newer parked draft");
    assert!(
        app.toasts
            .iter()
            .any(|t| t.message.contains("failed caption"))
    );
    app.open_chat = Some("work".into());
    app.composer.clear();
    events
        .send(Event::BeeperSendUncertain {
            chat: "work".into(),
            text: "restored when empty".into(),
            message: "Delivery unconfirmed".into(),
        })
        .unwrap();
    app.background_frame(&ctx);
    assert_eq!(app.composer, "restored when empty");
    app.backend.shutdown();
    println!(
        "PASS: late failure text retained in copyable error without replacing current or parked draft"
    );
}
