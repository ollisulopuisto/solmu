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
    account: String,
    ids: Vec<String>,
    replies: HashMap<Key, Vec<Key>>,
    parents: HashMap<Key, Key>,
    rects: HashMap<Key, egui::Rect>,
    hover: Option<Key>,
    chains: HashMap<Key, egui::Rect>,
    active: bool,
    included: HashSet<Key>,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            scope: egui::Id::NULL,
            account: String::new(),
            ids: vec![],
            replies: HashMap::new(),
            parents: HashMap::new(),
            rects: HashMap::new(),
            hover: None,
            chains: HashMap::new(),
            active: false,
            included: HashSet::new(),
        }
    }
}
/// A jump's way back, kept outside any view's scope so it survives the
/// cross-chat navigation the jump itself triggers. The first view whose
/// chats include the jump's origin adopts it into its own scope, where the
/// chips restore in place; any other view offers plain navigation back to
/// the origin message instead (see `pending_nav`).
#[derive(Clone)]
struct Pending {
    account: String,
    source: Key,
    scroll: Option<(egui::Id, egui::scroll_area::State)>,
    thread: Option<Key>,
}
fn pending_id() -> egui::Id {
    egui::Id::new("reply-jump-back-pending")
}
type Shared = std::sync::Arc<std::sync::Mutex<Frame>>;
fn frame(ctx: &egui::Context) -> Option<Shared> {
    ctx.data(|d| d.get_temp::<Shared>(frame_id()))
}
fn frame_id() -> egui::Id {
    egui::Id::new("loaded-reply-frame")
}
pub(crate) fn scope(app: &App, ids: &[String]) -> egui::Id {
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
                account: app.account().id.0.clone(),
                ids: ids.to_vec(),
                replies,
                parents,
                active,
                included,
                ..Default::default()
            })),
        )
    });
    // A jump's way back travels out of band because the jump itself may
    // switch chats. The view whose chats contain the jump's origin adopts
    // it into its own scope, replacing any older return point there; a
    // leftover from another account is stale and dropped. Any other view
    // leaves it for `pending_nav` to offer as plain navigation.
    let pending = ui.ctx().data(|d| d.get_temp::<Pending>(pending_id()));
    if let Some(pending) = pending {
        if pending.account != app.account().id.0 {
            ui.ctx().data_mut(|d| d.remove::<Pending>(pending_id()));
        } else if ids.iter().any(|id| id == &pending.source.chat) {
            ui.ctx().data_mut(|d| {
                d.remove::<Pending>(pending_id());
                d.remove::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"));
                d.remove::<Key>(scope.with("jump-back-thread"));
                if let Some(scroll) = pending.scroll {
                    d.insert_temp(scope.with("jump-back"), scroll);
                }
                if let Some(thread) = pending.thread {
                    d.insert_temp(scope.with("jump-back-thread"), thread);
                }
            });
        }
    }
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

/// Chat width below which an open thread fills the view instead of floating
/// as a modal over it.
pub const MODAL_MIN_WIDTH: f32 = 560.0;

/// True while the open thread draws as its own inset modal: the caller keeps
/// the background list rendering (dimmed under the overlay) but yields
/// scrolling and keyboard focus to the thread.
pub fn modal(app: &App, ui: &egui::Ui) -> bool {
    ui.available_width() >= MODAL_MIN_WIDTH && owns(app, ui.ctx())
}

/// Whether this message takes part in any loaded reply relationship.
#[cfg(test)]
pub fn is_linked(ctx: &egui::Context, chat: &str, id: &str) -> bool {
    let key = Key {
        chat: chat.to_owned(),
        id: id.to_owned(),
    };
    frame(ctx).is_some_and(|shared| {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        f.parents.contains_key(&key) || f.replies.contains_key(&key)
    })
}

/// The reply a jump from `replies`' parent lands on next: after the last
/// landing, wrapping back to the first.
fn next_reply(replies: &[Key], last: Option<usize>) -> usize {
    match last {
        Some(i) if !replies.is_empty() => (i + 1) % replies.len(),
        _ => 0,
    }
}

/// Remember where a jump started, for the chip that offers the way back:
/// the full-chat scroll position, or the open thread the jump leaves.
/// `source` is the bubble the jump started from; it becomes the navigation
/// target when the jump lands in another chat (see `Pending`).
pub fn prepare_jump_back(ctx: &egui::Context, source: &Key) {
    let Some(shared) = frame(ctx) else {
        return;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    // The open thread is the way back when the jump leaves one; otherwise
    // the saved full-chat scroll position.
    let (scroll, thread) = if let Some(root) = ctx.data(|d| d.get_temp::<Key>(f.scope)) {
        (None, Some(root))
    } else {
        (
            ctx.data(|d| {
                d.get_temp::<(egui::Id, egui::scroll_area::State)>(f.scope.with("full-scroll"))
            }),
            None,
        )
    };
    if scroll.is_none() && thread.is_none() {
        return;
    }
    // Kept outside this view's scope: the jump may land in another chat,
    // whose begin adopts it or offers it as navigation (see `Pending`).
    // Replacing the slot wholesale keeps one way back, never a stale pair.
    ctx.data_mut(|d| {
        d.insert_temp(
            pending_id(),
            Pending {
                account: f.account.clone(),
                source: source.clone(),
                scroll,
                thread,
            },
        )
    });
    ctx.request_repaint();
}

/// Jump to a message the open thread already shows: queue an in-thread
/// scroll and say whether that was possible. False means the jump has to
/// leave the thread (or there is none) and the caller navigates instead.
pub fn jump_within_thread(ctx: &egui::Context, key: &Key) -> bool {
    let Some(shared) = frame(ctx) else {
        return false;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !(f.active && f.included.contains(key)) {
        return false;
    }
    // Only a view that renders `thread_scroll` can deliver the hop; the
    // linked timeline shows the thread in place but never consumes the
    // queue, so there the caller navigates as if no thread were open.
    let scrollable = ctx
        .data(|d| d.get_temp::<bool>(f.scope.with("thread-scroll-here")))
        .unwrap_or(false);
    if !scrollable {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(f.scope.with("thread-scroll-to"), key.clone()));
    ctx.request_repaint();
    true
}

/// Ctrl/Command-click on a bubble: a reply jumps to its parent, a parent
/// cycles through its replies (a fading chip names the landing), and an
/// unrelated bubble keeps its old meaning, so the caller falls through to
/// selection. True when this handled the click.
pub fn command_click(
    ctx: &egui::Context,
    chat: &str,
    id: &str,
    actions: &mut Vec<crate::model::Action>,
) -> bool {
    let Some(shared) = frame(ctx) else {
        return false;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    let key = Key {
        chat: chat.to_owned(),
        id: id.to_owned(),
    };
    if let Some(parent) = f.parents.get(&key).cloned() {
        drop(f);
        if !jump_within_thread(ctx, &parent) {
            prepare_jump_back(ctx, &key);
            actions.push(crate::model::Action::OpenMessage {
                chat: parent.chat,
                message: parent.id,
            });
        }
        return true;
    }
    if let Some(replies) = f.replies.get(&key).filter(|r| !r.is_empty()).cloned() {
        // Each parent remembers its own position: coming back to a parent
        // continues its cycle instead of restarting at the first reply.
        let last = ctx.data(|d| d.get_temp::<usize>(f.scope.with(("cycle", &key))));
        let n = next_reply(&replies, last);
        let target = replies[n].clone();
        ctx.data_mut(|d| {
            d.insert_temp(f.scope.with(("cycle", &key)), n);
            d.insert_temp(
                f.scope.with("cycle-chip"),
                (
                    target.clone(),
                    n + 1,
                    replies.len(),
                    std::time::Instant::now(),
                ),
            );
        });
        drop(f);
        if !jump_within_thread(ctx, &target) {
            prepare_jump_back(ctx, &key);
            actions.push(crate::model::Action::OpenMessage {
                chat: target.chat,
                message: target.id,
            });
        }
        return true;
    }
    false
}

/// The way back a pending jump offers while the user stands outside its
/// origin chat: plain navigation to the message the jump started from.
/// `None` once the origin view has adopted the pending state (its chips
/// restore in place), or when there is none.
fn pending_nav(ctx: &egui::Context) -> Option<Key> {
    let shared = frame(ctx)?;
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    ctx.data(|d| d.get_temp::<Pending>(pending_id()))
        .filter(|p| !f.ids.contains(&p.source.chat))
        .map(|p| p.source.clone())
}

/// The chips that follow a jump: a fading "reply N of M" at the landing,
/// and the standing offer to go back to where the jump started. Returns
/// the origin message when the way back needs a cross-chat navigation the
/// chips cannot perform themselves; the caller turns it into an
/// `OpenMessage` action.
pub fn paint_jump_chips(ui: &egui::Ui, locale: crate::i18n::Locale) -> Option<Key> {
    let shared = frame(ui.ctx())?;
    let (scope, rects, active) = {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        (f.scope, f.rects.clone(), f.active)
    };
    let ctx = ui.ctx();
    let cycle: Option<(Key, usize, usize, std::time::Instant)> =
        ctx.data(|d| d.get_temp(scope.with("cycle-chip")));
    if let Some((landed, n, total, since)) = cycle {
        const FADE: f32 = 1.6;
        let t = since.elapsed().as_secs_f32();
        if t >= FADE {
            ctx.data_mut(|d| {
                d.remove::<(Key, usize, usize, std::time::Instant)>(scope.with("cycle-chip"))
            });
        } else if let Some(rect) = rects.get(&landed) {
            let strength = (FADE - t) / FADE;
            egui::Area::new(egui::Id::new(("reply-cycle-chip", &landed)))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + egui::vec2(8.0, -4.0))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(ui.visuals().window_fill())
                        .stroke(ui.visuals().window_stroke())
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            let count = if total == 1 {
                                crate::i18n::gettext(locale, "1 reply").into_owned()
                            } else {
                                crate::i18n::gettext(locale, "reply {n} of {total}")
                                    .replace("{n}", &n.to_string())
                                    .replace("{total}", &total.to_string())
                            };
                            ui.label(
                                egui::RichText::new(count)
                                    .small()
                                    .color(ui.visuals().text_color().gamma_multiply(strength)),
                            );
                        });
                });
            ctx.request_repaint();
        }
    }
    if active {
        return None;
    }
    let jump_back: Option<(egui::Id, egui::scroll_area::State)> =
        ctx.data(|d| d.get_temp(scope.with("jump-back")));
    let jump_back_thread: Option<Key> = ctx.data(|d| d.get_temp(scope.with("jump-back-thread")));
    // A jump that landed in another chat offers the way back as plain
    // navigation to the origin message; the chips cannot restore a
    // scroll position or a thread in a view they are not part of.
    let nav = pending_nav(ctx);
    let (label, show) = if jump_back.is_some() || nav.is_some() {
        (
            crate::i18n::gettext(locale, "‹ Back to where you were"),
            true,
        )
    } else if jump_back_thread.is_some() {
        (crate::i18n::gettext(locale, "‹ Back to thread"), true)
    } else {
        ("".into(), false)
    };
    if !show {
        return None;
    }
    let top = ui.clip_rect().center_top() + egui::vec2(0.0, 8.0);
    let outcome = egui::Area::new(egui::Id::new(("reply-jump-back", scope)))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(top)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(ui.visuals().window_fill())
                .stroke(ui.visuals().window_stroke())
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let back = ui.button(label).clicked();
                        let dismiss = ui.small_button("×").clicked();
                        (back, dismiss)
                    })
                    .inner
                })
                .inner
        })
        .inner;
    let (back, dismiss) = outcome;
    if back {
        if let Some((saved_id, saved)) = jump_back {
            saved.store(ctx, saved_id);
            ctx.request_discard("return to where the jump started");
            ctx.data_mut(|d| {
                d.remove::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"))
            });
        } else if let Some(root) = jump_back_thread {
            ctx.data_mut(|d| {
                d.remove::<Key>(scope.with("jump-back-thread"));
                d.insert_temp(scope, root);
            });
            ctx.request_repaint();
        } else if let Some(key) = nav {
            ctx.data_mut(|d| d.remove::<Pending>(pending_id()));
            return Some(key);
        }
    } else if dismiss {
        ctx.data_mut(|d| {
            d.remove::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"));
            d.remove::<Key>(scope.with("jump-back-thread"));
            d.remove::<Pending>(pending_id());
        });
    }
    None
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
pub fn bar(ui: &mut egui::Ui, locale: crate::i18n::Locale) -> bool {
    if !active(ui.ctx()) {
        return false;
    }
    let mut back = false;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                back = ui
                    .button(crate::i18n::gettext(locale, "‹ Back to full chat"))
                    .clicked();
                ui.label(crate::i18n::gettext(
                    locale,
                    "Thread · known loaded replies only",
                ));
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
    _color: egui::Color32,
    _locale: crate::i18n::Locale,
) {
    let key = Key::of(m);
    let Some(shared) = frame(ui.ctx()) else {
        return;
    };
    let mut f = shared.lock().unwrap_or_else(|e| e.into_inner());
    let pass = ui.ctx().cumulative_pass_nr();
    let body = ui
        .ctx()
        .data(|d| {
            d.get_temp::<egui::Rect>(super::conversation::bubble_id(&m.chat, &m.id).with("rect"))
        })
        .unwrap_or(response.rect);
    f.rects.insert(key.clone(), body);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let marker_active = ui.ctx().data(|d| {
        let id = super::conversation::bubble_id(&m.chat, &m.id);
        d.get_temp::<u64>(id.with("quote-pass")) == Some(pass)
            && (d.get_temp::<bool>(id.with("quote-focus")).unwrap_or(false)
                || d.get_temp::<egui::Rect>(id.with("quote"))
                    .is_some_and(|rect| pointer.is_some_and(|p| rect.contains(p))))
    });
    if let Some(chain) = ui.ctx().data(|d| {
        let id = super::conversation::bubble_id(&m.chat, &m.id);
        (d.get_temp::<u64>(id.with("quote-pass")) == Some(pass))
            .then(|| d.get_temp::<egui::Rect>(id.with("quote")))
            .flatten()
    }) {
        f.chains.insert(key.clone(), chain);
    }
    if marker_active {
        f.hover = Some(key.clone());
    }
}
/// Loaded history determines whether thread navigation is available, never a count label.
pub fn has_replies(ctx: &egui::Context, m: &Message) -> bool {
    frame(ctx).is_some_and(|shared| {
        shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .get(&Key::of(m))
            .is_some_and(|replies| !replies.is_empty())
    })
}

/// Both reply icons and root icons enter the exact loaded thread.
pub fn open_for_message(ctx: &egui::Context, message: &Message) {
    if let Some(shared) = frame(ctx) {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        let mut root = Key::of(message);
        let mut seen = HashSet::new();
        while seen.insert(root.clone()) {
            let Some(parent) = f.parents.get(&root) else {
                break;
            };
            root = parent.clone();
        }
        ctx.data_mut(|d| d.insert_temp(f.scope, root));
        ctx.request_repaint();
    }
}

pub fn finish(ui: &egui::Ui, color: egui::Color32) {
    if active(ui.ctx()) {
        return;
    }
    finish_routes(ui, color, false);
}
fn finish_routes(ui: &egui::Ui, color: egui::Color32, thread: bool) {
    let Some(shared) = frame(ui.ctx()) else {
        return;
    };
    let f = shared.lock().unwrap_or_else(|e| e.into_inner());
    let pairs: Vec<_> = if thread {
        f.parents
            .iter()
            .map(|(child, parent)| (parent, child))
            .collect()
    } else if let Some(hover) = f.hover.as_ref() {
        if let Some(parent) = f.parents.get(hover) {
            vec![(parent, hover)]
        } else if let Some(child) = f.replies.get(hover).and_then(|r| r.first()) {
            vec![(hover, child)]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    for (parent, child) in pairs {
        let (Some(a), Some(b)) = (f.rects.get(parent), f.rects.get(child)) else {
            continue;
        };
        if !a.intersects(ui.clip_rect()) || !b.intersects(ui.clip_rect()) {
            continue;
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
        let Some(chain) = f.chains.get(child) else {
            continue;
        };
        let Some(points) = crate::reply_router::route_to_chain(
            rect(a),
            rect(b),
            rect(chain),
            &obstacles,
            rect(&ui.clip_rect()),
        ) else {
            continue;
        };
        let points: Vec<_> = points.iter().map(|p| egui::pos2(p.x, p.y)).collect();
        let color = ui
            .ctx()
            .data(|data| {
                data.get_temp::<egui::Color32>(
                    super::conversation::bubble_id(&child.chat, &child.id).with("connector-color"),
                )
            })
            .unwrap_or(color);
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
}
pub fn ordinary(app: &mut App, ui: &mut egui::Ui, chat: &crate::model::Chat) -> bool {
    if !bar(ui, app.locale) {
        return false;
    }
    let Some(root) = current(ui.ctx()) else {
        return false;
    };
    thread_scroll(app, ui, chat, &root);
    true
}

/// The thread's own scrolling list of linked bubbles, shared by the
/// full-bleed thread view and the modal: only the frame around it differs.
/// Queued in-thread jumps land here, and paged scrolling (PgUp/PgDn, Home,
/// End) moves this list while the thread owns the view.
fn thread_scroll(app: &mut App, ui: &mut egui::Ui, chat: &crate::model::Chat, root: &Key) {
    if let Some(shared) = frame(ui.ctx()) {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        // This view delivers in-thread jumps; the glyph may queue them here.
        ui.ctx()
            .data_mut(|d| d.insert_temp(f.scope.with("thread-scroll-here"), true));
    }
    let rows: Vec<_> = app
        .conversations
        .get(&chat.id)
        .into_iter()
        .flat_map(|c| c.messages.iter())
        .filter(|m| includes(ui.ctx(), m) && !hidden(app, &chat.id, &m.id))
        .cloned()
        .collect();
    let pending = frame(ui.ctx()).and_then(|shared| {
        let f = shared.lock().unwrap_or_else(|e| e.into_inner());
        ui.ctx().data_mut(|d| {
            let pending = d.get_temp::<Key>(f.scope.with("thread-scroll-to"));
            d.remove::<Key>(f.scope.with("thread-scroll-to"));
            pending
        })
    });
    let mut actions = vec![];
    let output = egui::ScrollArea::vertical()
        .id_salt(("reply-thread", root))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            super::conversation::with_bubble_ns(super::conversation::THREAD_BUBBLE_NS, || {
                // The modal follows a full timeline pass. Never route using
                // stale background coordinates, especially for virtualized rows.
                if let Some(shared) = frame(ui.ctx()) {
                    let mut f = shared.lock().unwrap_or_else(|e| e.into_inner());
                    f.rects.clear();
                    f.chains.clear();
                    f.hover = None;
                }
                ui.set_min_width(ui.available_width());
                if rows.is_empty() {
                    ui.label(crate::i18n::gettext(
                        app.locale,
                        "This thread is no longer available in loaded history.",
                    ));
                }
                let mut landing = None;
                for (index, m) in rows.iter().enumerate() {
                    let right = ui.max_rect().right();
                    let rect = super::conversation::linked_bubble(
                        app,
                        ui,
                        chat,
                        m,
                        None,
                        super::conversation::starts_sender_run(
                            m,
                            index.checked_sub(1).map(|i| &rows[i]),
                        ),
                        true,
                        right,
                        None,
                        &mut actions,
                    );
                    if let (Some(rect), Some(target)) = (rect, pending.as_ref())
                        && target.chat == m.chat
                        && target.id == m.id
                    {
                        landing = Some(rect);
                    }
                    ui.add_space(8.0);
                }
                if let Some(rect) = landing {
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                finish_routes(ui, app.conversation_palette().secondary, true);
            })
        });
    if let Some(scroll) = app.scroll_page.take() {
        let mut state = output.state;
        let page = output.inner_rect.height() * 0.9;
        match scroll {
            crate::model::Scroll::PageUp => state.offset.y = (state.offset.y - page).max(0.0),
            crate::model::Scroll::PageDown => state.offset.y += page,
            crate::model::Scroll::Top => state.offset.y = 0.0,
            crate::model::Scroll::Bottom => {
                // Finite: egui lays out from the stored offset before it can
                // clamp, so infinity would put the rows at nonfinite edges.
                state.offset.y = (output.content_size.y - output.inner_rect.height()).max(0.0);
            }
        }
        state.store(ui.ctx(), output.id);
        ui.ctx().request_repaint();
    }
    app.actions.extend(actions);
}

/// The open thread as an inset modal over the dimmed chat: accent border and
/// banner, its own scroll, a click on the dimmed backdrop or Esc to close.
/// The composer stays outside, in the dimmed window below.
/// `backdrop` is everything the dim veil covers, composer included; `area`
/// is just the message region the modal panel insets itself into.
pub fn modal_overlay(
    app: &mut App,
    ui: &mut egui::Ui,
    chat: &crate::model::Chat,
    backdrop: egui::Rect,
    area: egui::Rect,
) {
    if !modal(app, ui) {
        return;
    }
    let Some(root) = current(ui.ctx()) else {
        return;
    };
    let ctx = ui.ctx().clone();
    let palette = app.conversation_palette();
    let dim = egui::Area::new(egui::Id::new("reply-thread-dim"))
        .order(egui::Order::Foreground)
        .fixed_pos(backdrop.min)
        .show(&ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(backdrop.size(), egui::Sense::click());
            ui.painter()
                .rect_filled(rect, 0, egui::Color32::from_black_alpha(150));
            response
        })
        .inner;
    if dim.clicked() {
        close(&ctx);
        return;
    }
    let rect = egui::Rect::from_min_max(
        area.min + egui::vec2(42.0, 28.0),
        area.max - egui::vec2(42.0, 56.0),
    );
    let tint = crate::theme::tinted_strength(palette.window, palette.accent, 0.08);
    let mut back = false;
    egui::Area::new(egui::Id::new("reply-thread-modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(&ctx, |ui| {
            // Registered before the content, so clicks in the gaps between
            // widgets land here and never fall through to the dim backdrop.
            let _swallow = ui.interact(rect, ui.id().with("backdrop"), egui::Sense::click());
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                egui::Frame::new()
                    .fill(tint)
                    .stroke(egui::Stroke::new(2.5, palette.accent))
                    .corner_radius(egui::CornerRadius::same(14))
                    .show(ui, |ui| {
                        ui.set_min_size(rect.size());
                        ui.set_max_size(rect.size());
                        egui::Frame::new()
                            .fill(palette.accent)
                            .corner_radius(egui::CornerRadius {
                                nw: 12,
                                ne: 12,
                                sw: 0,
                                se: 0,
                            })
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_min_width(rect.width() - 4.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(crate::i18n::gettext(
                                            app.locale,
                                            "IN THREAD",
                                        ))
                                        .strong()
                                        .color(palette.on_accent),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .button(
                                                    egui::RichText::new(crate::i18n::gettext(
                                                        app.locale,
                                                        "‹ Back to full chat (Esc)",
                                                    ))
                                                    .color(palette.on_accent),
                                                )
                                                .clicked()
                                            {
                                                back = true;
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(crate::i18n::gettext(
                                app.locale,
                                "Thread · known loaded replies only",
                            ))
                            .small()
                            .color(palette.secondary),
                        );
                        ui.add_space(2.0);
                        thread_scroll(app, ui, chat, &root);
                    });
            });
        });
    if back {
        close(&ctx);
    }
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
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
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
    fn chain_entry_on_reply_opens_exact_ancestor_not_only_its_descendants() {
        let (_dir, app, ctx) = seeded();
        let reply = app.conversations["a"].message("2").unwrap();
        open_for_message(&ctx, reply);
        assert_eq!(current(&ctx).unwrap().id, "1");
    }
    #[test]
    fn thread_routes_are_permanent_and_background_geometry_is_not_reused() {
        let ctx = egui::Context::default();
        let mut f = Frame::default();
        f.active = true;
        let parent = Key {
            chat: "chat".into(),
            id: "parent".into(),
        };
        let child = Key {
            chat: "chat".into(),
            id: "child".into(),
        };
        f.parents.insert(child.clone(), parent.clone());
        f.rects.insert(
            parent.clone(),
            egui::Rect::from_min_size(egui::pos2(50., 50.), egui::vec2(150., 50.)),
        );
        f.rects.insert(
            child.clone(),
            egui::Rect::from_min_size(egui::pos2(50., 220.), egui::vec2(150., 50.)),
        );
        f.chains.insert(
            child.clone(),
            egui::Rect::from_min_size(egui::pos2(65., 220.), egui::Vec2::ZERO),
        );
        ctx.data_mut(|d| d.insert_temp(frame_id(), std::sync::Arc::new(std::sync::Mutex::new(f))));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500., 400.),
                )),
                ..Default::default()
            },
            |ui| finish_routes(ui, egui::Color32::WHITE, true),
        );
        output.textures_delta.clear();
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::LineSegment { .. })),
            "thread route must paint without hover"
        );
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
    #[test]
    fn next_reply_advances_and_wraps() {
        let replies: Vec<Key> = (0..3)
            .map(|i| Key {
                chat: "a".into(),
                id: format!("r{i}"),
            })
            .collect();
        assert_eq!(next_reply(&replies, None), 0);
        assert_eq!(next_reply(&replies, Some(0)), 1);
        assert_eq!(next_reply(&replies, Some(2)), 0);
        assert_eq!(next_reply(&[], None), 0);
        assert_eq!(next_reply(&[], Some(5)), 0);
    }
    #[test]
    fn is_linked_marks_replies_and_parents_only() {
        let (_dir, _app, ctx) = seeded();
        assert!(is_linked(&ctx, "a", "1"), "a parent is linked");
        assert!(is_linked(&ctx, "a", "2"), "a reply is linked");
        assert!(!is_linked(&ctx, "a", "elsewhere"));
        assert!(
            !is_linked(&ctx, "other", "2"),
            "the chat is part of the key"
        );
    }
    #[test]
    fn command_click_on_a_reply_jumps_to_its_parent() {
        let (_dir, _app, ctx) = seeded();
        let mut actions = vec![];
        assert!(command_click(&ctx, "a", "2", &mut actions));
        assert_eq!(
            actions,
            [crate::model::Action::OpenMessage {
                chat: "a".into(),
                message: "1".into(),
            }]
        );
        assert!(command_click(&ctx, "a", "1", &mut actions));
        assert_eq!(actions.len(), 2, "the parent jumps to its reply");
        assert!(!command_click(&ctx, "a", "unrelated", &mut actions));
        assert_eq!(actions.len(), 2, "an unlinked bubble keeps its click");
    }
    #[test]
    fn command_click_on_a_parent_cycles_its_replies() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) =
            App::headless(crate::paths::AppDirs::under(dir.path()), Default::default());
        app.chats
            .push(crate::model::Chat::new("a".into(), "Group".into()));
        app.open_chat = Some("a".into());
        app.conversations.insert(
            "a".into(),
            crate::app::Conversation {
                messages: vec![
                    m("a", "1", None),
                    m("a", "2", Some(("a", "1"))),
                    m("a", "3", Some(("a", "1"))),
                ],
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        let mut actions = vec![];
        for expected in ["2", "3", "2"] {
            assert!(command_click(&ctx, "a", "1", &mut actions));
            let Some(crate::model::Action::OpenMessage { message, .. }) = actions.last() else {
                panic!("each click jumps to a reply");
            };
            assert_eq!(message, expected, "the cycle advances and wraps");
        }
    }
    #[test]
    fn jump_within_thread_queues_a_scroll_only_for_shown_messages() {
        // seeded() inserts the root after its first begin; rebuild the
        // frame so it sees the open thread.
        let (_dir, app, ctx) = seeded();
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        let f = frame(&ctx).unwrap();
        let scope = f.lock().unwrap().scope;
        // Only a view that renders `thread_scroll` consumes the queued jump;
        // mark one present, as the thread's scroll area does while open.
        ctx.data_mut(|d| d.insert_temp(scope.with("thread-scroll-here"), true));
        let root = Key {
            chat: "a".into(),
            id: "1".into(),
        };
        assert!(jump_within_thread(&ctx, &root), "the root is in the thread");
        assert_eq!(
            ctx.data(|d| d.get_temp::<Key>(scope.with("thread-scroll-to"))),
            Some(root)
        );
        let outside = Key {
            chat: "a".into(),
            id: "unrelated".into(),
        };
        assert!(!jump_within_thread(&ctx, &outside));
    }
    #[test]
    fn command_click_remembers_each_parents_own_cycle_position() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) =
            App::headless(crate::paths::AppDirs::under(dir.path()), Default::default());
        app.chats
            .push(crate::model::Chat::new("a".into(), "Group".into()));
        app.open_chat = Some("a".into());
        app.conversations.insert(
            "a".into(),
            crate::app::Conversation {
                messages: vec![
                    m("a", "1", None),
                    m("a", "2", Some(("a", "1"))),
                    m("a", "3", Some(("a", "1"))),
                    m("a", "4", None),
                    m("a", "5", Some(("a", "4"))),
                ],
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        let mut actions = vec![];
        let mut clicked = |parent: &str| {
            assert!(command_click(&ctx, "a", parent, &mut actions));
            let Some(crate::model::Action::OpenMessage { message, .. }) = actions.last() else {
                panic!("each click jumps to a reply");
            };
            message.clone()
        };
        assert_eq!(clicked("1"), "2");
        assert_eq!(clicked("4"), "5");
        assert_eq!(
            clicked("1"),
            "3",
            "returning to the first parent continues its cycle",
        );
    }
    #[test]
    fn prepare_jump_back_replaces_the_older_return_point() {
        let (_dir, app, ctx) = seeded();
        let source = Key {
            chat: "a".into(),
            id: "2".into(),
        };
        let scope = scope(&app, &["a".into()]);
        // A jump out of the thread leaves only the thread as the way back:
        // an older transcript position must not win in the chips.
        ctx.data_mut(|d| {
            d.insert_temp(
                scope.with("jump-back"),
                (egui::Id::new("scroll"), egui::scroll_area::State::default()),
            );
        });
        prepare_jump_back(&ctx, &source);
        // The way back settles into the view's scope across a frame.
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope.with("jump-back-thread")))
                .is_some()
        );
        assert!(
            ctx.data(
                |d| d.get_temp::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"))
            )
            .is_none(),
            "leaving a thread replaces the stale transcript return point",
        );
        // And the other way: a jump in the transcript clears the thread.
        ctx.data_mut(|d| {
            d.remove::<Key>(scope);
            d.insert_temp(
                scope.with("full-scroll"),
                (egui::Id::new("scroll"), egui::scroll_area::State::default()),
            );
        });
        prepare_jump_back(&ctx, &source);
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        assert!(
            ctx.data(
                |d| d.get_temp::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"))
            )
            .is_some()
        );
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope.with("jump-back-thread")))
                .is_none(),
            "a transcript jump replaces the stale thread return point",
        );
    }
    #[test]
    fn prepare_jump_back_captures_the_open_thread_or_the_full_scroll() {
        let (_dir, app, ctx) = seeded();
        let source = Key {
            chat: "a".into(),
            id: "2".into(),
        };
        let scope = scope(&app, &["a".into()]);
        // The seeded frame has an open thread: the way back is the thread.
        prepare_jump_back(&ctx, &source);
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope.with("jump-back-thread")))
                .is_some()
        );
        // With no thread open, the saved full-chat scroll becomes the way back.
        ctx.data_mut(|d| {
            d.remove::<Key>(scope);
            d.insert_temp(
                scope.with("full-scroll"),
                (egui::Id::new("scroll"), egui::scroll_area::State::default()),
            );
        });
        prepare_jump_back(&ctx, &source);
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        assert!(
            ctx.data(
                |d| d.get_temp::<(egui::Id, egui::scroll_area::State)>(scope.with("jump-back"))
            )
            .is_some()
        );
    }
    /// A jump that lands in another chat cannot restore in place there:
    /// the chip offers plain navigation back to the origin message.
    #[test]
    fn a_cross_chat_jump_offers_navigation_back() {
        let (_dir, app, ctx) = seeded();
        let source = Key {
            chat: "a".into(),
            id: "2".into(),
        };
        prepare_jump_back(&ctx, &source);
        // The jump lands in chat b: its frame must not adopt a's way back.
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["b".into()]));
        output.textures_delta.clear();
        assert_eq!(
            pending_nav(&ctx),
            Some(source),
            "another chat offers navigation back to the origin message",
        );
        let scope_b = scope(&app, &["b".into()]);
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope_b.with("jump-back-thread")))
                .is_none()
        );
        assert!(
            ctx.data(
                |d| d.get_temp::<(egui::Id, egui::scroll_area::State)>(scope_b.with("jump-back"))
            )
            .is_none(),
            "a non-origin view never adopts the way back",
        );
    }
    /// The origin view adopts the pending way back into its own scope,
    /// where the same-chat restore reads it.
    #[test]
    fn the_origin_view_adopts_a_pending_jump_back() {
        let (_dir, app, ctx) = seeded();
        let source = Key {
            chat: "a".into(),
            id: "2".into(),
        };
        prepare_jump_back(&ctx, &source);
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        let scope_a = scope(&app, &["a".into()]);
        assert!(
            ctx.data(|d| d.get_temp::<Pending>(pending_id())).is_none(),
            "adopted, not pending",
        );
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope_a.with("jump-back-thread")))
                .is_some()
        );
        assert!(
            pending_nav(&ctx).is_none(),
            "the origin restores in place, no navigation offer",
        );
    }
    /// A pending way back from another account is stale: dropped, never
    /// adopted, never shown.
    #[test]
    fn a_pending_jump_back_from_another_account_is_dropped() {
        let (_dir, app, ctx) = seeded();
        ctx.data_mut(|d| {
            d.insert_temp(
                pending_id(),
                Pending {
                    account: "another-account".into(),
                    source: Key {
                        chat: "a".into(),
                        id: "2".into(),
                    },
                    scroll: None,
                    thread: Some(Key {
                        chat: "a".into(),
                        id: "1".into(),
                    }),
                },
            )
        });
        let mut output = ctx.run_ui(Default::default(), |ui| begin(&app, ui, &["a".into()]));
        output.textures_delta.clear();
        let scope_a = scope(&app, &["a".into()]);
        assert!(ctx.data(|d| d.get_temp::<Pending>(pending_id())).is_none());
        assert!(
            ctx.data(|d| d.get_temp::<Key>(scope_a.with("jump-back-thread")))
                .is_none(),
            "another account's way back is never adopted",
        );
    }
}
