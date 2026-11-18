//! Warm frame-time harness for long conversations, with demo data only.
//!
//! Run it with:
//!
//! ```sh
//! cargo test --locked --lib --features demo frame_perf -- --ignored --nocapture
//! ```
//!
//! Every cell prints the first (cold) frame and the median warm frame of a
//! synthetic chat: 10, 100 or 1000 messages, short bodies or some very long
//! ones, with and without a held text selection (which lays out every row).
//! The numbers are evidence for conversation-layout caching work, not a CI
//! gate: nothing here asserts on wall time, so the harness never flakes.

use super::tests::app;
use super::*;
use crate::model::{Action, Content};
use std::time::{Duration, Instant};

/// The cell's chat: `rows` messages in the first sample conversation, every
/// seventh one very long when `long`, spread over one day.
fn perf_chat(rows: usize, long: bool) -> (App, egui::Context, String, crate::util::fixed_clock::Guard) {
    let clock = crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
        // 14:00 UTC on Wednesday 11 March 2026, as in long_chat_tests.
        now: 1_773_237_600,
        zone: jiff::tz::TimeZone::UTC,
        twelve_hour: false,
    });
    let mut app = app();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    let chat = SAMPLES[0].id.to_owned();
    let conversation = app.conversations.get_mut(&chat).unwrap();
    let template = conversation.messages.last().unwrap().clone();
    // A body near 30 KB with one unbreakable token near 3 000 chars: the
    // pasted-article shape that made manual.55 crawl.
    let sentence = "The quick brown fox jumps over the lazy dog near Helsinki harbour. ";
    let long_body = format!(
        "{}{}",
        sentence.repeat(430),
        "a".repeat(3_000)
    );
    conversation.messages.clear();
    for n in 0..rows {
        let mut row = template.clone();
        row.id = format!("perf-{n}");
        row.from_me = n % 3 == 0;
        row.timestamp = template.timestamp - (rows - n) as i64 * 240;
        row.content = if long && n % 7 == 3 {
            Content::text(format!("Row {n} {long_body}"))
        } else {
            Content::text(format!("Row {n} lorem ipsum dolor sit amet"))
        };
        conversation.messages.push(row);
    }
    // No unread divider: its first placement lays out every row.
    for row in &mut app.chats {
        row.unread = 0;
    }
    app.actions.push(Action::OpenChat(chat.clone()));
    (app, ctx, chat, clock)
}

fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Duration {
    let start = Instant::now();
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
            let ctx = ui.ctx().clone();
            app.background_frame(&ctx);
            app.frame_ui(ui);
        },
    );
    output.textures_delta.clear();
    start.elapsed()
}

/// Starts a drag selection over the visible messages and keeps the button
/// held. Returns whether a selection is live afterwards.
fn hold_selection(app: &mut App, ctx: &egui::Context) -> bool {
    let press = egui::pos2(600.0, 700.0);
    frame(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    // Drag upward over several frames so the sweep crosses message rows.
    for step in 1..=6 {
        frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                600.0,
                700.0 - 70.0 * step as f32,
            ))],
        );
    }
    ctx.plugin_opt::<egui::text_selection::LabelSelectionState>()
        .is_some_and(|plugin| plugin.lock().has_selection())
}

/// One matrix cell: cold frame, then warm frames, median reported. A slow
/// cell stops early rather than hold the harness for minutes.
fn cell(rows: usize, long: bool, select: bool) {
    let (mut app, ctx, _chat, _clock) = perf_chat(rows, long);
    let cold = frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    let selecting = select && hold_selection(&mut app, &ctx);
    let budget = Duration::from_secs(60);
    let mut warm: Vec<Duration> = Vec::new();
    let spent = Instant::now();
    for _ in 0..5 {
        if !warm.is_empty() && spent.elapsed() > budget {
            break;
        }
        warm.push(frame(&mut app, &ctx, Vec::new()));
    }
    warm.sort();
    let median = warm[warm.len() / 2];
    println!(
        "CELL messages={rows} body={} selection={} selecting={selecting} cold_ms={:.1} warm_median_ms={:.1} frames={}",
        if long { "long" } else { "short" },
        if select { "on" } else { "off" },
        cold.as_secs_f64() * 1e3,
        median.as_secs_f64() * 1e3,
        warm.len()
    );
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn frame_perf_matrix() {
    println!(
        "PROFILE {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for rows in [10_usize, 100, 1000] {
        for long in [false, true] {
            for select in [false, true] {
                cell(rows, long, select);
            }
        }
    }
}
