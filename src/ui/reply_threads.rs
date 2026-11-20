//! Local, loaded-history reply relationships. No network lookup or persisted history changes.
pub use crate::reply_graph::Key;
use crate::{app::App, model::Message};
use std::collections::{HashMap, HashSet};
impl Key {
    fn of(m: &Message) -> Self {
        Self {
            chat: m.chat.clone(),
            id: m.id.clone(),
        }
    }
}
use crate::reply_graph::chain;
#[derive(Clone)]
struct Frame {
    scope: egui::Id,
    ids: Vec<String>,
    replies: HashMap<Key, Vec<Key>>,
    parents: HashMap<Key, Key>,
    rects: HashMap<Key, egui::Rect>,
    hover: Option<Key>,
    active: bool,
    included: HashSet<Key>,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            scope: egui::Id::NULL,
            ids: vec![],
            replies: HashMap::new(),
            parents: HashMap::new(),
            rects: HashMap::new(),
            hover: None,
            active: false,
            included: HashSet::new(),
        }
    }
}
type Shared = std::sync::Arc<std::sync::Mutex<Frame>>;
fn frame(ctx: &egui::Context) -> Option<Shared> {
    ctx.data(|d| d.get_temp::<Shared>(frame_id()))
}
fn frame_id() -> egui::Id {
    egui::Id::new("loaded-reply-frame")
}
fn scope(app: &App, ids: &[String]) -> egui::Id {
    egui::Id::new(("reply-scope", &app.account().id.0, ids))
}
fn index(messages: &[&Message]) -> (HashMap<Key, Vec<Key>>, HashMap<Key, Key>) {
    crate::reply_graph::index(messages.iter().map(|m| {
        (
            Key::of(m),
            m.quoted.as_ref().map(|q| Key {
                chat: q.chat.clone().unwrap_or_else(|| m.chat.clone()),
                id: q.id.clone(),
            }),
        )
    }))
}
fn hidden(app: &App, chat: &str, id: &str) -> bool {
    app.chat(chat).is_none_or(|c| {
        crate::hidden_messages::ids_in(&app.settings.beeper.hidden_messages, &app.later_key(c))
            .contains(id)
    })
}
pub fn begin(app: &App, ui: &egui::Ui, ids: &[String]) {
    let messages: Vec<_> = ids
        .iter()
        .filter_map(|id| app.chat(id).filter(|c| !c.locked))
        .flat_map(|c| {
            app.conversations.get(&c.id).into_iter().flat_map(|v| {
                v.messages.iter().filter(|m| {
                    !hidden(app, &c.id, &m.id)
                        && !matches!(m.content, crate::model::Content::Revoked)
                })
            })
        })
        .collect();
    let (replies, parents) = index(&messages);
    let scope = scope(app, ids);
    #[cfg(feature = "demo")]
    if crate::demo::reply_thread() {
        let marker = scope.with("demo-seeded");
        if !ui
            .ctx()
            .data(|d| d.get_temp::<bool>(marker))
            .unwrap_or(false)
        {
            demo_open(app, ui.ctx());
            ui.ctx().data_mut(|d| d.insert_temp(marker, true));
        }
    }
    let root = ui.ctx().data(|d| d.get_temp::<Key>(scope));
    let active = root.is_some();
    let included = root
        .as_ref()
        .map(|root| chain(root, &parents))
        .unwrap_or_default();
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            frame_id(),
            std::sync::Arc::new(std::sync::Mutex::new(Frame {
                scope,
                ids: ids.to_vec(),
                replies,
                parents,
                active,
                included,
                ..Default::default()
            })),
        )
    });
}
pub fn current(ctx: &egui::Context) -> Option<Key> {
    frame(ctx).and_then(|f| {
        let f = f.lock().unwrap_or_else(|e| e.into_inner());
        ctx.data(|d| d.get_temp::<Key>(f.scope))
    })
}
pub fn owns(app: &App, ctx: &egui::Context) -> bool {
    frame(ctx).is_some_and(|shared| {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        app.chat_view()
            && app.open_chat.as_ref().is_some_and(|id| f.ids.contains(id))
            && scope(app, &f.ids) == f.scope
            && ctx.data(|d| d.get_temp::<Key>(f.scope)).is_some()
    })
}
/// Explicit navigation and selection take over the full transcript, without restoring an old offset over the jump.
pub fn leave_for_navigation(ctx: &egui::Context) {
    if let Some(shared) = frame(ctx) {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        ctx.data_mut(|d| {
            d.remove::<Key>(f.scope);
            d.remove::<bool>(f.scope.with("restore"));
        });
    }
}
pub fn close(ctx: &egui::Context) {
    if let Some(f) = frame(ctx) {
        let f = f.lock().unwrap_or_else(|e| e.into_inner());
        ctx.data_mut(|d| {
            d.remove::<Key>(f.scope);
            d.insert_temp(f.scope.with("restore"), true);
        });
        ctx.request_repaint();
    }
}
pub fn complete_full_scroll(
    ctx: &egui::Context,
    id: egui::Id,
    state: egui::scroll_area::State,
) -> bool {
    let Some(shared) = frame(ctx) else {
        return false;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    let restore = ctx
        .data_mut(|d| d.remove_temp::<bool>(f.scope.with("restore")))
        .unwrap_or(false);
    if restore
        && let Some((saved_id, saved)) = ctx.data(|d| {
            d.get_temp::<(egui::Id, egui::scroll_area::State)>(f.scope.with("full-scroll"))
        })
    {
        saved.store(ctx, saved_id);
        ctx.request_discard("restore full chat position after thread");
        return true;
    }
    if !f.active {
        ctx.data_mut(|d| d.insert_temp(f.scope.with("full-scroll"), (id, state)));
    }
    false
}
pub fn has_links(ctx: &egui::Context) -> bool {
    frame(ctx).is_some_and(|f| {
        !f.lock()
            .unwrap_or_else(|e| e.into_inner())
            .parents
            .is_empty()
    })
}
#[cfg(feature = "demo")]
pub fn demo_open(app: &App, ctx: &egui::Context) {
    if let Some(chat) = app.current_chat() {
        let key = scope(app, std::slice::from_ref(&chat.id));
        if let Some(m) = app
            .conversations
            .get(&chat.id)
            .and_then(|c| c.messages.iter().find(|m| m.quoted.is_some()))
        {
            let q = m.quoted.as_ref().unwrap();
            ctx.data_mut(|d| {
                d.insert_temp(
                    key,
                    Key {
                        chat: q.chat.clone().unwrap_or_else(|| m.chat.clone()),
                        id: q.id.clone(),
                    },
                )
            });
        }
    }
}
pub fn active(ctx: &egui::Context) -> bool {
    current(ctx).is_some()
}
pub fn includes(ctx: &egui::Context, m: &Message) -> bool {
    frame(ctx).is_none_or(|f| {
        let f = f.lock().unwrap_or_else(|e| e.into_inner());
        !f.active || f.included.contains(&Key::of(m))
    })
}
pub fn bar(ui: &mut egui::Ui) -> bool {
    if !active(ui.ctx()) {
        return false;
    }
    let mut back = false;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                back = ui.button("‹ Back to full chat").clicked();
                ui.label("Thread · known loaded replies only");
            });
        });
    if back {
        close(ui.ctx());
    }
    active(ui.ctx())
}
pub fn row(
    ui: &mut egui::Ui,
    m: &Message,
    response: &egui::Response,
    color: egui::Color32,
    actions: &mut Vec<crate::model::Action>,
) {
    let key = Key::of(m);
    let Some(shared) = frame(ui.ctx()) else {
        return;
    };
    let mut f = shared.lock().unwrap_or_else(|e| e.into_inner());
    f.rects.insert(key.clone(), response.rect);
    if response.hovered() || response.contains_pointer() || response.has_focus() {
        f.hover = Some(key.clone());
    }
    if !f.active
        && let Some(parent) = f.parents.get(&key)
        && ui
            .small_button("↑ Original")
            .on_hover_text(
                "Jump to the original message. Available even when no clear connector route fits.",
            )
            .clicked()
    {
        actions.push(crate::model::Action::OpenMessage {
            chat: parent.chat.clone(),
            message: parent.id.clone(),
        });
    }
    if let Some(replies) = f.replies.get(&key).filter(|r| !r.is_empty()) {
        let n = replies.len();
        let label = if n == 1 {
            "1 loaded reply ›".into()
        } else {
            format!("{n} loaded replies ›")
        };
        if ui
            .add(egui::Button::new(egui::RichText::new(label).color(color)).frame(false))
            .on_hover_text("Only replies in loaded history are counted. Open this thread.")
            .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(f.scope, key));
            ui.ctx().request_repaint();
        }
    }
}
pub fn finish(ui: &egui::Ui, color: egui::Color32) {
    let Some(shared) = frame(ui.ctx()) else {
        return;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    if f.active {
        return;
    }
    let Some(hover) = f.hover.as_ref() else {
        return;
    };
    let (parent, child) = if let Some(parent) = f.parents.get(hover) {
        (parent, hover)
    } else if let Some(child) = f.replies.get(hover).and_then(|r| r.first()) {
        (hover, child)
    } else {
        return;
    };
    let (Some(a), Some(b)) = (f.rects.get(parent), f.rects.get(child)) else {
        return;
    };
    if !a.intersects(ui.clip_rect()) || !b.intersects(ui.clip_rect()) {
        return;
    }
    let rect = |r: &egui::Rect| crate::reply_router::Box2 {
        l: r.left(),
        t: r.top(),
        r: r.right(),
        b: r.bottom(),
    };
    let obstacles: Vec<_> = f
        .rects
        .iter()
        .filter(|(key, _)| *key != parent && *key != child)
        .map(|(_, r)| rect(r))
        .collect();
    let Some(points) =
        crate::reply_router::route(rect(a), rect(b), &obstacles, rect(&ui.clip_rect()))
    else {
        return;
    };
    let points: Vec<_> = points.iter().map(|p| egui::pos2(p.x, p.y)).collect();
    let stroke = egui::Stroke::new(1.8, color);
    let mut previous = points[0];
    for i in 1..points.len() - 1 {
        let a = points[i - 1];
        let corner = points[i];
        let b = points[i + 1];
        let radius = 4.0_f32
            .min(a.distance(corner) / 2.0)
            .min(b.distance(corner) / 2.0);
        let before = corner + (a - corner).normalized() * radius;
        let after = corner + (b - corner).normalized() * radius;
        ui.painter().line_segment([previous, before], stroke);
        ui.painter()
            .add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    before,
                    before + (corner - before) * 0.66,
                    after + (corner - after) * 0.66,
                    after,
                ],
                false,
                egui::Color32::TRANSPARENT,
                stroke,
            ));
        previous = after;
    }
    ui.painter()
        .line_segment([previous, *points.last().unwrap()], stroke);
}
pub fn ordinary(app: &mut App, ui: &mut egui::Ui, chat: &crate::model::Chat) -> bool {
    if !bar(ui) {
        return false;
    }
    let Some(root) = current(ui.ctx()) else {
        return false;
    };
    let rows: Vec<_> = app
        .conversations
        .get(&chat.id)
        .into_iter()
        .flat_map(|c| c.messages.iter())
        .filter(|m| includes(ui.ctx(), m) && !hidden(app, &chat.id, &m.id))
        .cloned()
        .collect();
    let mut actions = vec![];
    egui::ScrollArea::vertical()
        .id_salt(("reply-thread", &root))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if rows.is_empty() {
                ui.label("This thread is no longer available in loaded history.");
            }
            for m in rows {
                let right = ui.max_rect().right();
                super::conversation::linked_bubble(
                    app,
                    ui,
                    chat,
                    &m,
                    true,
                    right,
                    None,
                    &mut actions,
                );
                ui.add_space(8.0);
            }
        });
    app.actions.extend(actions);
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn m(chat: &str, id: &str, parent: Option<(&str, &str)>) -> Message {
        Message {
            id: id.into(),
            chat: chat.into(),
            sender: "s".into(),
            sender_name: None,
            from_me: false,
            timestamp: 1,
            history_order: None,
            content: crate::model::Content::text("x"),
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: parent.map(|(c, i)| crate::model::Quoted {
                chat: Some(c.into()),
                id: i.into(),
                sender: "s".into(),
                sender_name: None,
                summary: "x".into(),
                labelled: None,
                mentions: vec![],
            }),
            reactions: vec![],
            edited: false,
            mentions: vec![],
            forwarded: false,
            thumbnail: None,
        }
    }
    fn seeded() -> (tempfile::TempDir, App, egui::Context) {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) =
            App::headless(crate::paths::AppDirs::under(dir.path()), Default::default());
        let chat = crate::model::Chat::new("a".into(), "Group".into());
        app.chats.push(chat);
        app.open_chat = Some("a".into());
        app.conversations.insert(
            "a".into(),
            crate::app::Conversation {
                messages: vec![m("a", "1", None), m("a", "2", Some(("a", "1")))],
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        let key = scope(&app, &["a".into()]);
        ctx.data_mut(|d| {
            d.insert_temp(
                key,
                Key {
                    chat: "a".into(),
                    id: "1".into(),
                },
            )
        });
        (dir, app, ctx)
    }
    #[test]
    fn explicit_quote_search_and_selection_actions_leave_thread_without_scroll_restore() {
        for action in [
            crate::model::Action::ScrollTo("1".into()),
            crate::model::Action::OpenMessage {
                chat: "a".into(),
                message: "1".into(),
            },
            crate::model::Action::SelectMessage("1".into()),
        ] {
            let (_dir, mut app, ctx) = seeded();
            assert!(active(&ctx));
            app.apply(action, &ctx);
            assert!(!active(&ctx));
            let f = frame(&ctx).unwrap();
            let f = f.lock().unwrap();
            assert!(
                ctx.data(|d| d.get_temp::<bool>(f.scope.with("restore")))
                    .is_none()
            );
        }
    }
    #[test]
    fn stale_thread_frame_does_not_own_settings_or_another_chat() {
        let (_dir, mut app, ctx) = seeded();
        assert!(owns(&app, &ctx));
        app.page = crate::model::Page::Settings;
        assert!(!owns(&app, &ctx));
        app.page = crate::model::Page::Chats;
        app.open_chat = Some("other".into());
        assert!(!owns(&app, &ctx));
    }
    #[test]
    fn index_requires_loaded_exact_chat_and_deduplicates() {
        let a = m("a", "same", None);
        let b = m("b", "same", None);
        let c = m("a", "reply", Some(("b", "same")));
        let missing = m("a", "missing", Some(("c", "same")));
        let (r, p) = index(&[&a, &b, &c, &c, &missing]);
        assert!(!r.contains_key(&Key::of(&a)));
        assert_eq!(r[&Key::of(&b)].len(), 1);
        assert_eq!(p.len(), 1);
    }
    #[test]
    fn thread_includes_descendants_not_unrelated_and_terminates_cycles() {
        let a = m("a", "1", Some(("a", "2")));
        let b = m("a", "2", Some(("a", "1")));
        let c = m("a", "3", Some(("a", "2")));
        let d = m("a", "4", None);
        let (_, p) = index(&[&a, &b, &c, &d]);
        let set = chain(&Key::of(&a), &p);
        assert_eq!(set.len(), 3);
        assert!(!set.contains(&Key::of(&d)));
    }
}
