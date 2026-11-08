//! One local person timeline, with exact member routing and no remote merge calls.
use crate::{
    app::App,
    merged::{Member, Person},
    model::{Action, Chat, ChatKind},
};
#[derive(Clone, Default)]
struct RowCache {
    width: f32,
    /// Row heights by chat and message id, so a lookup borrows the two
    /// strings instead of copying them for every loaded row of every frame.
    heights: std::collections::HashMap<String, std::collections::HashMap<String, f32>>,
    count: usize,
}

impl RowCache {
    fn height(&self, chat: &str, id: &str) -> Option<f32> {
        self.heights.get(chat)?.get(id).copied()
    }

    fn note_height(&mut self, chat: &str, id: &str, height: f32) {
        let by_id = self.heights.entry(chat.to_owned()).or_default();
        self.count += usize::from(!by_id.contains_key(id));
        by_id.insert(id.to_owned(), height);
    }

    fn clear(&mut self) {
        self.heights.clear();
        self.count = 0;
    }
}

/// The local calendar day a timestamp falls on, for day headers.
fn day_stamp(timestamp: i64) -> String {
    crate::util::day_key(timestamp)
        .map(|date| date.to_string())
        .unwrap_or_default()
}
#[derive(Clone, Default)]
struct State {
    catch_up: Option<crate::merged::CatchUp>,
    /// Which chats could not load older messages, after the last attempt.
    older_note: Option<String>,
    destination: Option<Member>,
    reply: Option<(Member, String)>,
    explicit: bool,
    search: String,
    source_filter: Option<Member>,
    search_open: bool,
}
/// What "Load older" could not do, once it stopped: the networks whose older
/// messages did not come, and why. `None` when every chat caught up or ran out.
fn older_note(app: &App, catch_up: &crate::merged::CatchUp) -> Option<String> {
    let mut not_now = Vec::new();
    let mut empty = Vec::new();
    for (chat, why) in catch_up.stopped() {
        let source = app.chat(chat).map(network_label).unwrap_or("Chat");
        let list = match why {
            crate::merged::Stopped::NotNow => &mut not_now,
            crate::merged::Stopped::NoProgress => &mut empty,
        };
        if !list.contains(&source) {
            list.push(source);
        }
    }
    let mut parts = Vec::new();
    if !not_now.is_empty() {
        parts.push(format!(
            "{}: older messages can't be requested right now (phone not connected, or waiting after an empty answer). Press Load older to try again.",
            not_now.join(", ")
        ));
    }
    if !empty.is_empty() {
        parts.push(format!(
            "{}: no older messages came back; this may be as far back as it goes.",
            empty.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
/// Command maps to Cmd on macOS and Ctrl elsewhere, matching the rest of the app.
pub fn latest_reply_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT | egui::Modifiers::ALT,
        egui::Key::R,
    )
}
fn bounded_note(ui: &mut egui::Ui, palette: &crate::theme::Palette, text: &str, small: bool) {
    let width = (ui.clip_rect().right() - ui.cursor().left())
        .min(ui.available_width())
        .max(20.0);
    let font = if small {
        egui::TextStyle::Small
    } else {
        egui::TextStyle::Body
    }
    .resolve(ui.style());
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, palette.text, width);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    ui.painter().galley(rect.min, galley, palette.text);
}

pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> bool {
    if chat.backend_account.is_none() {
        return false;
    }
    if chat.kind != ChatKind::Direct {
        return false;
    }
    if app.chats.iter().filter(|c| c.id == chat.id).count() != 1 {
        return false;
    }
    let current = Member::of(chat);
    let person = app
        .settings
        .beeper
        .people
        .iter()
        .find(|p| p.members.iter().any(|m| Some(m) == current.as_ref()))
        .cloned();
    if person.is_none() {
        // Linking lives in Contact info; the conversation top stays clean.
        return false;
    }
    let person = person.unwrap();
    let valid: Vec<_> = person
        .members
        .iter()
        .filter(|m| {
            app.chats.iter().any(|c| {
                m.matches(c)
                    && !c.locked
                    && c.kind == ChatKind::Direct
                    && c.backend_origin == chat.backend_origin
                    && app.chats.iter().filter(|known| known.id == c.id).count() == 1
            })
        })
        .cloned()
        .collect();
    if valid.len() < 2 {
        app.ensure_person_loaded(&[]);
        if ui.button("Remove local link").clicked() {
            app.unlink_person(&person);
        }
        ui.colored_label(egui::Color32::YELLOW,"Linked person unavailable: member identity missing or locked. Open members separately.");
        return true;
    }
    if valid.len() > 8 {
        ui.label("Linked view supports up to 8 exact member chats.");
        return true;
    }
    let ids: Vec<_> = valid.iter().map(|m| m.chat.clone()).collect();
    app.ensure_person_loaded(&ids);
    let key = egui::Id::new(("linked-person-state", &person.members));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<State>(key))
        .unwrap_or_default();
    // Demo captures seed a reply through a real route.
    #[cfg(feature = "demo")]
    if crate::demo::linked_reply() {
        let mut latest: Option<(Member, String, i64)> = None;
        for member in &valid {
            if let Some(conversation) = app.conversations.get(&member.chat) {
                for message in &conversation.messages {
                    if latest
                        .as_ref()
                        .is_none_or(|(_, _, ts)| message.timestamp > *ts)
                    {
                        latest = Some((member.clone(), message.id.clone(), message.timestamp));
                    }
                }
            }
        }
        if let Some((member, id, _)) = latest {
            state.reply = Some((member.clone(), id));
            state.destination = Some(member);
        }
    }
    if state
        .destination
        .as_ref()
        .is_some_and(|m| !valid.contains(m))
    {
        state = State::default();
    }
    // A reply whose source left the link is dropped, never re-routed, and
    // the note below says so.
    let mut reply_cleared = false;
    if state
        .reply
        .as_ref()
        .is_some_and(|(source, _)| !valid.contains(source))
    {
        state.reply = None;
        reply_cleared = true;
    }
    let mut routing_error = reply_cleared.then_some(
        "The chat you were replying in is no longer linked, so that reply was cleared. Pick a destination or reply again.",
    );
    if state.destination.is_none() && state.reply.is_none() {
        match person.default_destination(&valid) {
            Ok(primary) => state.destination = primary.cloned(),
            Err(error) => routing_error = Some(error),
        }
    }
    if let Some(mut catch_up) = state.catch_up.take() {
        let paging: Vec<_> = valid
            .iter()
            .map(|m| {
                let conversation = app.conversations.get(&m.chat);
                crate::merged::Paging {
                    chat: m.chat.clone(),
                    oldest: conversation
                        .and_then(|c| c.messages.first())
                        .map(|message| message.timestamp),
                    exhausted: conversation.is_none_or(|c| c.complete && c.phone_exhausted),
                    busy: conversation.is_some_and(|c| c.loading_older || c.fetching_phone),
                }
            })
            .collect();
        let (requests, waiting) = catch_up.plan(&paging);
        for chat in &requests {
            let oldest = app
                .conversations
                .get(chat)
                .and_then(|c| c.messages.first())
                .map_or(0, |message| message.timestamp);
            app.load_older(chat, true);
            let went_out = app
                .conversations
                .get(chat)
                .is_some_and(|c| c.loading_older || c.fetching_phone);
            if went_out {
                catch_up.asked(chat, oldest);
            } else {
                catch_up.unavailable(chat);
            }
        }
        if waiting || !requests.is_empty() {
            let ctx = ui.ctx().clone();
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            state.catch_up = Some(catch_up);
        } else {
            state.older_note = older_note(app, &catch_up);
        }
    }
    let message_count: usize = valid
        .iter()
        .map(|m| {
            app.conversations
                .get(&m.chat)
                .map_or(0, |c| c.messages.len())
        })
        .sum();
    // Identity only: management and the explanatory notes live in the
    // Contact info panel, so the timeline keeps its room.
    let searching =
        state.search_open || !state.search.trim().is_empty() || state.source_filter.is_some();
    let tail = format!("{} linked chats · local only", valid.len());
    let actions = |ui: &mut egui::Ui, app: &mut App, state: &mut State| {
        // A load over the tunnel is otherwise invisible: while one runs, the
        // button rests and a spinner beside it says the fetch is on its way.
        let fetching = state.catch_up.is_some()
            || valid.iter().any(|member| {
                app.conversations
                    .get(&member.chat)
                    .is_some_and(|c| c.loading_older || c.fetching_phone)
            });
        if ui
            .add_enabled_ui(!fetching, |ui| {
                super::conversation::header_icon(ui, &app.conversation_palette(), crate::theme::Icon::Clock, fetching, "Load older messages from each linked chat")
            }).inner
            .on_hover_text(format!(
                "Load older from each chat. {message_count} loaded messages · Each member pages independently; this is not the full remote history."
            ))
            .clicked()
        {
            let oldest = valid
                .iter()
                .filter_map(|m| app.conversations.get(&m.chat))
                .filter_map(|c| c.messages.first().map(|message| message.timestamp))
                .min();
            state.older_note = None;
            state.catch_up = oldest.map(crate::merged::CatchUp::new);
        }
        if fetching {
            crate::theme::spinner(ui, 14.0, app.conversation_palette().accent);
        }
        if super::conversation::header_icon(
            ui,
            &app.conversation_palette(),
            crate::theme::Icon::Search,
            searching,
            "Search the loaded messages and narrow by source",
        )
        .clicked()
        {
            state.search_open = !searching;
            if !state.search_open {
                state.search.clear();
                state.source_filter = None;
            }
        }
    };
    egui::Frame::new()
        .fill(app.conversation_palette().surface)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let has_error = valid
                    .iter()
                    .any(|member| app.beeper_live_errors.contains_key(&member.chat));
                let actions_width = if has_error { 224.0 } else { 192.0 };
                let identity_width = (ui.available_width() - actions_width).max(30.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(identity_width, 40.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&person.name).strong()).truncate(),
                        )
                        .on_hover_text(&person.name);
                        ui.add(egui::Label::new(&tail).truncate())
                            .on_hover_text(&tail);
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    actions(ui, app, &mut state);
                    super::conversation::chat_actions_toolbar(app, ui, chat);
                    super::conversation::connection_status(
                        app,
                        ui,
                        &valid.iter().map(|m| m.chat.as_str()).collect::<Vec<_>>(),
                    );
                });
            });
        });
    if let Some(error) = routing_error {
        ui.colored_label(egui::Color32::YELLOW, error);
    }
    if let Some(note) = &state.older_note {
        let warning = app.conversation_palette().warning;
        ui.colored_label(warning, note);
    }
    for member in &valid {
        if let Some(error) = app.person_history_error(&member.chat) {
            let source = source_label(app, &valid, member);
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    crate::i18n::gettext(
                        app.locale,
                        "Partial timeline: {source} history failed: {error}",
                    )
                    .replace("{source}", &source)
                    .replace("{error}", &error),
                );
                if ui
                    .button(crate::i18n::gettext(app.locale, "Retry history"))
                    .clicked()
                {
                    app.retry_person_history(&member.chat);
                }
            });
        }
    }
    if valid.len() != person.members.len() {
        ui.colored_label(
            egui::Color32::YELLOW,
            "Some linked chats are unavailable or locked; they are excluded.",
        );
    }
    if searching {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search loaded messages")
                    .desired_width(220.0),
            );
            let label = state
                .source_filter
                .as_ref()
                .and_then(|m| app.chat(&m.chat))
                .map(network_label)
                .unwrap_or("All sources");
            egui::ComboBox::from_id_salt("linked-source-filter")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.source_filter, None, "All sources");
                    for member in &valid {
                        let c = app.chat(&member.chat).unwrap();
                        ui.selectable_value(
                            &mut state.source_filter,
                            Some(member.clone()),
                            format!("{} [{}]", network_label(c), member.account),
                        );
                    }
                });
            if ui.button("Clear search").clicked() {
                state.search.clear();
                state.source_filter = None;
            }
        });
    }
    // Routing safety: no reply target and no chosen destination means no
    // route at all - new text goes nowhere without an explicit choice.
    if state.reply.is_none() {
        app.reply_to = None;
    }
    let cross_over = if let Some((source, id)) = &state.reply {
        app.reply_to = state
            .destination
            .as_ref()
            .and_then(|destination| crate::merged::reply_route(source, destination, id));
        state.destination.as_ref() != Some(source)
    } else {
        false
    };
    // The composer pins to the bottom of the view; the destination and
    // Send Later ride its right end, grouped beside Send. The first bottom
    // panel shown lands lowest.
    let destination = state
        .destination
        .as_ref()
        .and_then(|m| app.chat(&m.chat))
        .cloned();
    if let Some(destination) = &destination {
        let had_quote = app.reply_to.is_some();
        let before = app.actions.len();
        let chosen = destination_label(app, state.destination.as_ref());
        let extras_width = measure_extras(ui);
        let mut render = |app: &mut App, ui: &mut egui::Ui| {
            destination_extras(app, ui, &mut state, &valid, &person, cross_over, &chosen);
        };
        super::conversation::composer_with_capabilities(
            app,
            ui,
            destination,
            false,
            Some(super::conversation::ComposerExtras {
                width: extras_width,
                render: &mut render,
            }),
        );
        if had_quote && app.reply_to.is_none() {
            state.reply = None;
            if !state.explicit {
                state.destination = None;
            }
        }
        if app.actions[before..]
            .iter()
            .any(|a| matches!(a, Action::SendText { .. } | Action::SendPending { .. }))
        {
            state.reply = None;
            if !state.explicit {
                state.destination = None;
            }
        }
    } else {
        // No route yet: the composer stays visible but inert, and says why.
        egui::Panel::bottom("linked-composer-off")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 8,
                    }),
            )
            .show(ui, |ui| {
                // An inert frame shaped like the input: no disabled fade,
                // no cursor, muted theme-aware text that stays legible in
                // both themes while reading as inactive.
                let palette = app.conversation_palette();
                egui::Frame::new()
                    .fill(palette.bubble_in)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(24))
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 16,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Choose a destination to write")
                                    .color(palette.secondary),
                            );
                            let right = egui::Layout::right_to_left(egui::Align::Center);
                            ui.with_layout(right, |ui| {
                                egui::ComboBox::from_id_salt("person-destination")
                                    .selected_text("Choose destination")
                                    .show_ui(ui, |ui| {
                                        destination_entries(app, ui, &mut state, &valid, &person)
                                    });
                            });
                        });
                    });
            });
    }
    // Home/End and PageUp/PageDown from the key map land here: the linked
    // timeline owns its scroll area, and the single-chat message list never
    // runs while this view is up.
    let pending_scroll = app.scroll_page.take();
    let forced_bottom = std::mem::take(&mut app.scroll_to_bottom_forced);

    // Caching the merged timeline order avoids collecting and sorting O(N log N)
    // messages across member chats on every frame while scrolling. We track the
    // member conversation lengths, first message timestamps, and last message
    // timestamps as a composite fingerprint. Storing (chat_id, index) pairs allows
    // O(1) index lookup into member conversations without linear search or string churn.
    let timeline_fingerprint: Vec<(String, usize, Option<i64>, Option<i64>)> = valid
        .iter()
        .map(|m| {
            let conv = app.conversations.get(&m.chat);
            let len = conv.map_or(0, |c| c.messages.len());
            let first = conv
                .and_then(|c| c.messages.first())
                .map(|msg| msg.timestamp);
            let last = conv
                .and_then(|c| c.messages.last())
                .map(|msg| msg.timestamp);
            (m.chat.clone(), len, first, last)
        })
        .collect();

    let cached_order_key = key.with("timeline-order");
    type CachedTimeline = (
        Vec<(String, usize, Option<i64>, Option<i64>)>,
        Vec<(String, usize)>,
    );

    let cached_order = ui
        .ctx()
        .data(|d| d.get_temp::<CachedTimeline>(cached_order_key));
    let ordered_indices: Vec<(String, usize)> = match cached_order {
        Some((fp, indices)) if fp == timeline_fingerprint => indices,
        _ => {
            let mut flat_indexed: Vec<(String, usize, &crate::model::Message)> = valid
                .iter()
                .flat_map(|m| {
                    app.conversations.get(&m.chat).into_iter().flat_map(|c| {
                        c.messages
                            .iter()
                            .enumerate()
                            .map(|(idx, msg)| (m.chat.clone(), idx, msg))
                    })
                })
                .collect();
            flat_indexed.sort_unstable_by(|a, b| {
                a.2.timestamp
                    .cmp(&b.2.timestamp)
                    .then(a.2.chat.cmp(&b.2.chat))
                    .then(a.2.history_order.cmp(&b.2.history_order))
                    .then(a.2.id.cmp(&b.2.id))
            });
            flat_indexed.dedup_by(|a, b| a.2.chat == b.2.chat && a.2.id == b.2.id);
            let indices: Vec<(String, usize)> = flat_indexed
                .into_iter()
                .map(|(chat, idx, _)| (chat, idx))
                .collect();
            ui.ctx().data_mut(|d| {
                d.insert_temp(cached_order_key, (timeline_fingerprint, indices.clone()))
            });
            indices
        }
    };

    let messages: Vec<&crate::model::Message> = ordered_indices
        .iter()
        .filter_map(|(chat_id, idx)| {
            app.conversations
                .get(chat_id)
                .and_then(|conv| conv.messages.get(*idx))
        })
        .collect();
    let filtered = !state.search.trim().is_empty() || state.source_filter.is_some();
    let visible: Vec<_> = messages
        .into_iter()
        .filter(|message| {
            if state
                .source_filter
                .as_ref()
                .is_some_and(|member| member.chat != message.chat)
            {
                return false;
            }
            if state.search.trim().is_empty() {
                return true;
            }
            let source = app
                .chat(&message.chat)
                .map(network_label)
                .unwrap_or("Beeper");
            let body = match &message.content {
                crate::model::Content::Text { text, .. }
                | crate::model::Content::Interactive { text, .. } => {
                    std::borrow::Cow::Borrowed(text.as_str())
                }
                other => std::borrow::Cow::Owned(other.full_summary()),
            };
            crate::merged::search_matches(
                &body,
                &state.search,
                source,
                message.sender_name.as_deref().unwrap_or(&message.sender),
            )
        })
        .collect();
    if searching && filtered {
        ui.small(format!(
            "{} matches in loaded history only. Search does not change sending destination.",
            visible.len()
        ));
    }
    let mut selected_reply = None;
    let shortcut = latest_reply_shortcut();
    let shortcut_label = ui.ctx().format_shortcut(&shortcut);
    let keyboard_reply = ui.input_mut(|input| input.consume_shortcut(&shortcut));
    let mut clicked_reply = false;
    if searching {
        ui.horizontal_wrapped(|ui| {
            clicked_reply = ui.add_enabled(
                crate::merged::latest_replyable(&visible).is_some(),
                egui::Button::new(format!("Reply to latest match ({shortcut_label})")),
            ).on_hover_text(format!("Keyboard fallback: {shortcut_label}. Uses the latest non-deleted loaded search result and its exact source. Narrow by search/source first. Or double-click a message / use its reply menu.")).clicked();
        });
    }
    if (clicked_reply || keyboard_reply)
        && let Some(message) = crate::merged::latest_replyable(&visible)
        && let Some(member) = valid.iter().find(|member| member.chat == message.chat)
    {
        selected_reply = Some((member.clone(), message.id.clone()));
    }
    let mut actions = Vec::new();
    let mut refused_action = false;
    let redo = ui.ctx().current_pass_index() > 0;
    let rows_key = key.with("row-heights");
    let mut cache = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<RowCache>(rows_key).unwrap_or_default());
    let timeline = ui
        .available_rect_before_wrap()
        .shrink2(egui::vec2(super::conversation::TIMELINE_SIDE, 0.0));
    let scroll_rect = egui::Rect::from_min_max(
        timeline.min,
        timeline.max + egui::vec2(super::conversation::TIMELINE_SIDE, 0.0),
    );
    if (cache.width - timeline.width()).abs() > 0.5 {
        cache.width = timeline.width();
        cache.clear();
    }
    // Cap retained layout metadata, not the user's loaded history.
    if cache.count > 20_000 {
        cache.clear();
    }
    let mut grew_above = 0.0;
    let mut previous: Option<&crate::model::Message> = None;
    let mut previous_laid_out = false;
    let mut previous_on_screen = false;
    let mut displayed_sources = std::collections::HashSet::new();
    let output = ui
        // The scroll area runs to the pane's edge so its overlay bar sits in
        // the side margin; rows keep the inset width and never sit under it.
        .scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
            crate::theme::shot_bars(ui);
            let remaining = ui.available_height().max(100.0);
            // The bottom jump applies after the pass, once the content's
            // height is known; everything else is an offset the area can
            // clamp itself.
            if pending_scroll.is_some() || forced_bottom {
                let scroll_id =
                    ui.make_persistent_id(egui::IdSalt::new(("person-timeline", &person.members)));
                let mut state =
                    egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
                let page = remaining * 0.9;
                match pending_scroll {
                    Some(crate::model::Scroll::Top) => state.offset.y = 0.0,
                    Some(crate::model::Scroll::PageUp) => {
                        state.offset.y = (state.offset.y - page).max(0.0)
                    }
                    Some(crate::model::Scroll::PageDown) => state.offset.y += page,
                    _ => {}
                }
                state.store(ui.ctx(), scroll_id);
            }
            egui::ScrollArea::vertical()
                .id_salt(("person-timeline", &person.members))
                .stick_to_bottom(!filtered)
                .min_scrolled_height(remaining)
                .max_height(remaining)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(timeline.width());
                    // The same tight row rhythm as an ordinary chat's transcript.
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let viewport = ui.clip_rect();
                    if visible.is_empty() {
                        ui.label(if filtered {
                            "No matches in loaded history."
                        } else {
                            "Loading linked histories…"
                        });
                    }
                    for (visible_index, original_message) in visible.iter().copied().enumerate() {
                        let top = ui.cursor().top();
                        // A row the reader cannot see costs its cached or
                        // estimated height and nothing else: no timezone
                        // math and no string copies. The day and the header
                        // boundary are computed only for rows actually laid
                        // out, from the previous row's message whether that
                        // row was laid out or skipped.
                        let height =
                            match cache.height(&original_message.chat, &original_message.id) {
                                Some(height) => height,
                                None => {
                                    let height = super::conversation::linked_estimated_height(
                                        original_message,
                                        cache.width,
                                    );
                                    cache.note_height(
                                        &original_message.chat,
                                        &original_message.id,
                                        height,
                                    );
                                    height
                                }
                            };
                        if !crate::merged::row_near_view(
                            top,
                            height,
                            viewport.top(),
                            viewport.bottom(),
                            250.0,
                        ) {
                            ui.add_space(height);
                            previous_laid_out = false;
                            previous_on_screen = false;
                            previous = Some(original_message);
                            continue;
                        }
                        let day = day_stamp(original_message.timestamp);
                        let previous_fields = previous.map(|message| {
                            (
                                message.chat.as_str(),
                                message.sender.as_str(),
                                day_stamp(message.timestamp),
                                message.from_me,
                            )
                        });
                        let boundary = crate::merged::header_boundary(
                            previous_fields
                                .as_ref()
                                .map(|(chat, sender, day, from_me)| {
                                    (*chat, *sender, day.as_str(), *from_me)
                                }),
                            (
                                original_message.chat.as_str(),
                                original_message.sender.as_str(),
                                day.as_str(),
                                original_message.from_me,
                            ),
                        );
                        let day_boundary = previous_fields
                            .as_ref()
                            .is_none_or(|(_, _, previous_day, _)| previous_day != &day);
                        previous = Some(original_message);
                        let mut updated;
                        let message = if let Some(quote) = &original_message.quoted {
                            if quote
                                .chat
                                .as_deref()
                                .is_none_or(|source| source == original_message.chat)
                            {
                                if let Some(original) = app
                                    .conversations
                                    .get(&original_message.chat)
                                    .and_then(|history| history.message(&quote.id))
                                {
                                    updated = original_message.clone();
                                    let quote = updated.quoted.as_mut().unwrap();
                                    quote.sender = original.sender.clone();
                                    quote.sender_name = original.sender_name.clone();
                                    quote.summary = original.content.full_summary();
                                    quote.labelled = Some(original.content.labelled());
                                    quote.mentions = original.mentions.clone();
                                    &updated
                                } else {
                                    original_message
                                }
                            } else {
                                original_message
                            }
                        } else {
                            original_message
                        };
                        let Some(member) = valid.iter().find(|m| m.chat == message.chat) else {
                            continue;
                        };
                        let source_chat = app.chat(&message.chat).unwrap().clone();
                        let on_screen = top >= viewport.top();
                        // The day alone earns a text line, where it changes.
                        // The source reads from the bubble's network tint,
                        // and from the small network icon beside the bubble
                        // where the network changes (or history starts), so
                        // colour is never the only cue; the icon's hover
                        // carries the full label with the exact time.
                        ui.push_id((&message.chat, &message.id), |ui| {
                            let sender = if message.from_me {
                                "You"
                            } else {
                                message.sender_name.as_deref().unwrap_or("Sender")
                            };
                            if day_boundary {
                                ui.add_space(super::conversation::DAY_SPACE_ABOVE);
                                ui.small(crate::util::day_label(app.locale, message.timestamp));
                                ui.add_space(super::conversation::DAY_SPACE_BELOW);
                            } else if boundary {
                                ui.add_space(4.0);
                            }
                            // The mark earns its place only where the
                            // network changes from the previous message (or
                            // history starts); a sender or direction change
                            // alone never repeats it. The gutter room below
                            // stays reserved either way, so every bubble
                            // keeps the same column.
                            let network = network_label(&source_chat);
                            let show_mark = crate::merged::shows_network_mark(
                                previous_fields
                                    .as_ref()
                                    .and_then(|(chat, _, _, _)| app.chat(chat))
                                    .map(network_label),
                                network,
                            );
                            // The hover label is computed only for a row
                            // that shows the mark.
                            let meta = show_mark.then(|| {
                                let exact = jiff::Timestamp::from_second(message.timestamp)
                                    .map(|t| {
                                        let zoned = t.to_zoned(jiff::tz::TimeZone::system());
                                        format!(
                                            "{} {:02}:{:02}",
                                            zoned.date(),
                                            zoned.time().hour(),
                                            zoned.time().minute()
                                        )
                                    })
                                    .unwrap_or_default();
                                format!(
                                    "{exact} · {}{} · {sender}",
                                    network,
                                    account_name(app, &source_chat),
                                )
                            });
                            let tint = crate::theme::network_tint(network)
                                .unwrap_or(app.conversation_palette().secondary);
                            let icon = crate::theme::network_icon(network);
                            let before = actions.len();
                            // The bubble is laid out in the plain column, as in an
                            // ordinary chat, so rows keep its rhythm. The chip is
                            // painted beside it afterwards, in the room set aside
                            // on the chip's side; it never adds to the row height.
                            let chip = 20.0;
                            let room = chip + ui.spacing().item_spacing.x;
                            let mut area = ui.available_rect_before_wrap();
                            if message.from_me {
                                area.max.x -= room;
                            } else {
                                area.min.x += room;
                            }
                            let bubble_rect = ui
                                .scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(area)
                                        .layout(egui::Layout::top_down(egui::Align::Min)),
                                    |ui| {
                                        super::conversation::linked_bubble(
                                            app,
                                            ui,
                                            &source_chat,
                                            message,
                                            super::conversation::last_minute_in_run(
                                                original_message,
                                                visible.get(visible_index + 1).copied(),
                                            ),
                                            timeline.right() - 8.0,
                                            &mut actions,
                                        )
                                    },
                                )
                                .inner;
                            if bubble_rect.is_some_and(|rect| rect.intersects(viewport)) {
                                displayed_sources.insert(message.chat.clone());
                            }
                            if show_mark && let Some(rect) = bubble_rect {
                                let side = if message.from_me {
                                    area.max.x + ui.spacing().item_spacing.x
                                } else {
                                    area.min.x - room
                                };
                                let y = rect.top() + (rect.height() / 2.0).min(chip);
                                let centre = egui::pos2(side + chip / 2.0, y);
                                let slot =
                                    egui::Rect::from_center_size(centre, egui::Vec2::splat(chip));
                                let response = ui.interact(
                                    slot,
                                    ui.id().with(("network-chip", &message.chat, &message.id)),
                                    egui::Sense::hover(),
                                );
                                crate::theme::paint_network_mark(ui, icon, slot, chip, tint);
                                if let Some(meta) = &meta {
                                    response.on_hover_text(meta);
                                }
                            }
                            let reply = actions.drain(before..).collect::<Vec<_>>();
                            for action in reply {
                                match action {
                                    Action::Reply(id) => {
                                        selected_reply = Some((member.clone(), id));
                                    }
                                    Action::PrefillComposer { text, mentions, .. } => {
                                        if app.composer.trim().is_empty()
                                            && app.reply_to.is_none()
                                            && app.editing.is_none()
                                            && app.recording.is_none()
                                            && !app.drafts.contains_key(&chat.id)
                                        {
                                            state.destination = Some(member.clone());
                                            state.explicit = true;
                                            actions.push(Action::PrefillComposer {
                                                chat: chat.id.clone(),
                                                text,
                                                mentions,
                                            });
                                        } else {
                                            refused_action = true;
                                        }
                                    }
                                    Action::Edit(_)
                                    | Action::SelectMessage(_)
                                    | Action::SweepMessages { .. }
                                    | Action::TranslateLocally(_) => refused_action = true,
                                    other => actions.push(other),
                                }
                            }
                        });
                        previous_laid_out = true;
                        previous_on_screen = on_screen;
                        let measured = (ui.cursor().top() - top).max(1.0);
                        if top < viewport.top() {
                            grew_above += measured - height;
                        }
                        cache.note_height(&message.chat, &message.id, measured);
                    }
                    // Keep the last bubble/hover actions above the fixed composer.
                    ui.add_space(20.0);
                })
        })
        .inner;
    let bottom = (output.content_size.y - output.inner_rect.height()).max(0.0);
    if matches!(pending_scroll, Some(crate::model::Scroll::Bottom)) || forced_bottom {
        let mut scroll = output.state;
        scroll.offset.y = bottom;
        scroll.store(ui.ctx(), output.id);
        ui.ctx()
            .request_discard("linked view jumps to the newest message");
    } else if grew_above.abs() > 0.5
        && !redo
        && output.state.offset.y > 1.0
        && output.state.offset.y < bottom - 2.0
    {
        let mut scroll = output.state;
        scroll.offset.y = (scroll.offset.y + grew_above).max(0.0);
        scroll.store(ui.ctx(), output.id);
        ui.ctx()
            .request_discard("linked row height changed above viewport");
    }
    ui.ctx().data_mut(|data| data.insert_temp(rows_key, cache));
    if let Some((member, id)) = selected_reply {
        state.reply = Some((member.clone(), id.clone()));
        // Each reply routes home by default: the source's own network and
        // account. Picking another destination afterwards is the explicit
        // cross-over, and the warning says the reply reference does not travel.
        state.explicit = false;
        state.destination = Some(member.clone());
        app.reply_to = state
            .destination
            .as_ref()
            .and_then(|destination| crate::merged::reply_route(&member, destination, &id));
    }
    if refused_action {
        app.toast(
            "Open this source chat separately for that action; linked routing was not guessed",
        );
    }
    app.finish_deferred_linked_reads(&displayed_sources.into_iter().collect::<Vec<_>>());
    app.actions.extend(actions);
    ui.ctx().data_mut(|d| d.insert_temp(key, state));
    true
}

/// The saved default destination for new messages, kept per linked person
/// (the same setting Contact info shows). `None` means choose each time.
fn set_default(app: &mut App, person: &Person, primary: Option<Member>) {
    if let Some(link) = app
        .settings
        .beeper
        .people
        .iter_mut()
        .find(|p| **p == *person)
    {
        link.primary = primary;
    }
    app.mark_settings_dirty();
}

/// The dropdown's rows: each exact member (the saved default is marked),
/// then the saved-default controls. Picking a member is a one-off choice for
/// the message being written; the default only changes through the button.
fn destination_entries(
    app: &mut App,
    ui: &mut egui::Ui,
    state: &mut State,
    valid: &[Member],
    person: &Person,
) {
    for member in valid {
        let Some(c) = app.chat(&member.chat) else {
            continue;
        };
        let marker = if person.primary.as_ref() == Some(member) {
            "  (default)"
        } else {
            ""
        };
        let label = format!("{} · {}{marker}", c.name, source_label(app, valid, member));
        if ui
            .selectable_label(state.destination.as_ref() == Some(member), label)
            .clicked()
        {
            state.explicit = true;
            state.destination = Some(member.clone());
            app.reply_to = state
                .reply
                .as_ref()
                .and_then(|(source, id)| crate::merged::reply_route(source, member, id));
        }
    }
    ui.separator();
    if let Some(current) = state
        .destination
        .clone()
        .filter(|member| person.primary.as_ref() != Some(member))
        && ui
            .button("Make this the default for new messages")
            .on_hover_text(
                "Saved for this person. Replies still go to the message's own network and account.",
            )
            .clicked()
    {
        set_default(app, person, Some(current));
    }
    if person.primary.is_some() && ui.button("Clear default").clicked() {
        set_default(app, person, None);
    }
}

/// The destination selector's caption: the member's network and account,
/// capped so the composer keeps its field.
fn destination_label(app: &App, destination: Option<&Member>) -> String {
    let Some(member) = destination else {
        return "Choose destination".into();
    };
    let Some(chat) = app.chat(&member.chat) else {
        return "Choose destination".into();
    };
    let label = format!("{}{}", network_label(chat), account_name(app, chat));
    let mut short: String = label.chars().take(26).collect();
    if label.chars().count() > 26 {
        short.push('\u{2026}');
    }
    short
}

/// The width the composer's extras reserve: the clock, its gap, and the
/// selector's icon button, plus slack.
fn measure_extras(ui: &egui::Ui) -> f32 {
    34.0 + ui.spacing().item_spacing.x + SELECTOR_SIZE + 10.0
}

/// The destination selector is the network's icon alone; the network and
/// account names stay on its tooltip, accessible label and menu rows.
const SELECTOR_SIZE: f32 = 28.0;

/// The composer's right-end controls in the linked view: Send Later, then
/// the destination, which names the network and account until Send.
fn destination_extras(
    app: &mut App,
    ui: &mut egui::Ui,
    state: &mut State,
    valid: &[Member],
    person: &Person,
    cross_over: bool,
    chosen: &str,
) {
    let palette = app.conversation_palette();
    let full = state
        .destination
        .as_ref()
        .and_then(|member| app.chat(&member.chat))
        .map(|chat| format!("{}{}", network_label(chat), account_name(app, chat)))
        .unwrap_or_else(|| chosen.to_owned());
    let schedulable =
        app.editing.is_none() && (!app.composer.trim().is_empty() || !app.pending.is_empty());
    let clock = ui
        .add_enabled_ui(schedulable, |ui| {
            crate::theme::icon_button(
                ui,
                crate::theme::Icon::Clock,
                22.0,
                palette.secondary,
                palette.text,
                "Send later",
            )
        })
        .inner;
    if clock.clicked() {
        app.actions
            .push(Action::ShowDialog(crate::model::Dialog::SendLater));
    }
    let network = state
        .destination
        .as_ref()
        .and_then(|member| app.chat(&member.chat))
        .map(|chat| network_label(chat).to_owned())
        .unwrap_or_default();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(SELECTOR_SIZE, SELECTOR_SIZE),
        egui::Sense::click(),
    );
    crate::theme::reveal_focus(&response);
    crate::theme::focus_outline(ui, response.id, rect, 6.0);
    let hint = format!(
        "{full}\n{}",
        if cross_over {
            "Cross-over: a plain new message on the selected destination, without the reply reference"
        } else {
            "The network and account new messages send from"
        }
    );
    let label = format!("Send to {full}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
    }
    let tint = if cross_over {
        palette.warning
    } else {
        crate::theme::network_tint(&network).unwrap_or(palette.secondary)
    };
    crate::theme::paint_network_mark(
        ui,
        crate::theme::network_icon(&network),
        rect.shrink(4.0),
        20.0,
        tint,
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hint);
    egui::Popup::menu(&response).show(|ui| destination_entries(app, ui, state, valid, person));
}

/// The account's own name from the Beeper inventory, for "Sending via":
/// " · name" when the inventory knows it, nothing otherwise.
fn account_name(app: &App, chat: &Chat) -> String {
    app.beeper_accounts
        .values()
        .flatten()
        .find(|account| Some(&account.id) == chat.backend_account.as_ref())
        .filter(|account| !account.name.is_empty())
        .map(|account| format!(" · {}", account.name))
        .unwrap_or_default()
}

/// The network a Beeper chat shows: the canonical API name first, then the
/// decorated bridge label, then the generic fallback. Matches the field the
/// bubble tint reads, so colour and text never disagree.
fn network_label(chat: &Chat) -> &str {
    chat.backend_network
        .as_deref()
        .or(chat.network.as_deref())
        .unwrap_or("Beeper")
}

/// How a member reads in the linked view: its network, with the account's
/// own name appended only when another member shares that network.
fn source_label(app: &App, valid: &[Member], member: &Member) -> String {
    let chat = app.chat(&member.chat);
    let network = chat.map(network_label).unwrap_or("Beeper").to_owned();
    let shared = valid.iter().any(|other| {
        other != member && app.chat(&other.chat).map(network_label) == Some(network.as_str())
    });
    if shared {
        if let Some(chat) = chat {
            let label = account_name(app, chat);
            if !label.is_empty() {
                return format!("{network}{label}");
            }
        }
        format!("{network} [{}]", member.account)
    } else {
        network
    }
}

/// Fuzzy name matching for link suggestions: 0 exact, 1 prefix, 2
/// substring, 3 subsequence; None when nothing matches. Case-insensitive.
pub(crate) fn name_score(query: &str, candidate: &str) -> Option<u32> {
    let query = query.trim().to_lowercase();
    let candidate = candidate.trim().to_lowercase();
    if query.is_empty() || candidate.is_empty() {
        return None;
    }
    if candidate == query {
        return Some(0);
    }
    if candidate.starts_with(&query) {
        return Some(1);
    }
    if candidate.contains(&query) {
        return Some(2);
    }
    let mut chars = candidate.chars();
    if query
        .chars()
        .all(|wanted| chars.by_ref().any(|have| have == wanted))
    {
        return Some(3);
    }
    None
}

/// Chats the picker may offer for linking with `chat`: the same backend's
/// unlocked direct chats, each known once, not already linked to anyone.
fn link_candidates(app: &App, chat: &Chat) -> Vec<Chat> {
    app.chats
        .iter()
        .filter(|c| {
            c.kind == ChatKind::Direct
                && !c.locked
                && c.id != chat.id
                && c.backend_account.is_some()
                && c.backend_origin == chat.backend_origin
                && app.chats.iter().filter(|known| known.id == c.id).count() == 1
                && !app
                    .settings
                    .beeper
                    .people
                    .iter()
                    .any(|p| p.members.iter().any(|m| m.matches(c)))
        })
        .cloned()
        .collect()
}

#[derive(Clone, Default)]
struct Picker {
    open: bool,
    search: String,
    selected: Vec<String>,
}

/// One checkbox row per candidate, grouped as suggestions first, then a
/// tree by network; the selection survives searching and folding.
fn picker(
    app: &App,
    ui: &mut egui::Ui,
    chat: &Chat,
    base_name: &str,
    state: &mut Picker,
    already: usize,
) -> Vec<Member> {
    let candidates = link_candidates(app, chat);
    if candidates.is_empty() {
        ui.label("No other chats on this Beeper server can be linked.");
        return Vec::new();
    }
    ui.add(
        egui::TextEdit::singleline(&mut state.search)
            .hint_text("Search chats")
            .desired_width(f32::INFINITY),
    );
    // The action stays visible at the top; a long candidate list
    // must not scroll it out of reach.
    let room = 8_usize.saturating_sub(already);
    let over = state.selected.len() > room;
    let names: Vec<_> = state
        .selected
        .iter()
        .filter_map(|id| app.chat(id))
        .map(|c| c.name.clone())
        .collect();
    if !state.selected.is_empty() {
        ui.small(format!(
            "{} selected: {}",
            state.selected.len(),
            names.join(", ")
        ));
    }
    if over {
        ui.colored_label(
            egui::Color32::YELLOW,
            format!("A linked person supports up to 8 chats; {room} left."),
        );
    }
    if ui
        .add_enabled(
            !state.selected.is_empty() && !over,
            egui::Button::new("Link selected"),
        )
        .clicked()
    {
        return state
            .selected
            .iter()
            .filter_map(|id| app.chat(id))
            .filter_map(Member::of)
            .collect();
    }
    let query = state.search.trim().to_lowercase();
    let matches = |c: &&Chat| {
        query.is_empty()
            || c.name.to_lowercase().contains(&query)
            || network_label(c).to_lowercase().contains(&query)
    };
    let row = |ui: &mut egui::Ui, state: &mut Picker, c: &Chat| {
        let account = account_name(app, c);
        let network = network_label(c);
        let mut on = state.selected.contains(&c.id);
        if ui
            .checkbox(&mut on, format!("{} · {network}{account}", c.name))
            .changed()
        {
            if on {
                state.selected.push(c.id.clone());
            } else {
                state.selected.retain(|id| id != &c.id);
            }
        }
    };
    if query.is_empty() {
        let mut suggested: Vec<_> = candidates
            .iter()
            .filter_map(|c| name_score(base_name, &c.name).map(|score| (score, c)))
            .collect();
        suggested.sort_by_key(|(score, _)| *score);
        suggested.truncate(3);
        if !suggested.is_empty() {
            ui.label("Suggested by name:");
            for (_, c) in suggested {
                row(ui, state, c);
            }
            ui.add_space(4.0);
        }
    }
    let mut networks: Vec<_> = candidates
        .iter()
        .filter(matches)
        .map(|c| network_label(c).to_owned())
        .collect();
    networks.sort();
    networks.dedup();
    for network in networks {
        let chats: Vec<_> = candidates
            .iter()
            .filter(|c| network_label(c) == network)
            .filter(matches)
            .collect();
        egui::CollapsingHeader::new(format!("{network} ({})", chats.len()))
            .id_salt(("link-picker-network", &network))
            .default_open(true)
            .show(ui, |ui| {
                for c in chats {
                    row(ui, state, c);
                }
            });
    }
    Vec::new()
}

/// The local display name: the linked person's name, or this chat's own.
/// Nothing is sent to the network or to other people.
fn rename_field(
    app: &mut App,
    ui: &mut egui::Ui,
    chat: &Chat,
    person: Option<&Person>,
    palette: &crate::theme::Palette,
) {
    ui.label(egui::RichText::new("Display name").strong());
    let mut name = match person {
        Some(person) => person.name.clone(),
        None => app
            .settings
            .beeper
            .chat_names
            .get(&chat.id)
            .cloned()
            .unwrap_or_default(),
    };
    let before = name.clone();
    let response = ui.add(
        egui::TextEdit::singleline(&mut name)
            .hint_text(chat.name.as_str())
            .desired_width(f32::INFINITY),
    );
    let focus = egui::Id::new("rename-focus");
    let wanted = ui
        .ctx()
        .data(|d| d.get_temp::<String>(focus))
        .is_some_and(|id| id == chat.id);
    if wanted {
        response.request_focus();
        ui.ctx().data_mut(|d| d.remove::<String>(focus));
    }
    if name != before {
        match person {
            Some(person) => {
                if let Some(stored) = app
                    .settings
                    .beeper
                    .people
                    .iter_mut()
                    .find(|p| p.members == person.members)
                {
                    stored.name = name;
                }
            }
            None if name.trim().is_empty() => {
                app.settings.beeper.chat_names.remove(&chat.id);
            }
            None => {
                app.settings.beeper.chat_names.insert(chat.id.clone(), name);
            }
        }
        app.mark_settings_dirty();
    }
    bounded_note(
        ui,
        palette,
        "Local name only. Nothing changes for other people or on the network.",
        false,
    );
}

/// The Contact info section owning person links: the explanation, primary
/// destination, unlinking, and the searchable picker. Returns false when
/// the chat is not a linkable Beeper direct chat.
pub(crate) fn sidebar(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> bool {
    if chat.backend_account.is_none()
        || chat.kind != ChatKind::Direct
        || app.chats.iter().filter(|c| c.id == chat.id).count() != 1
    {
        return false;
    }
    let current = Member::of(chat);
    let person = app
        .settings
        .beeper
        .people
        .iter()
        .find(|p| p.members.iter().any(|m| Some(m) == current.as_ref()))
        .cloned();
    let palette = app.palette;
    ui.add_space(4.0);
    rename_field(app, ui, chat, person.as_ref(), &palette);
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Linked person").strong());
    let key = egui::Id::new(("link-picker", &chat.id));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<Picker>(key))
        .unwrap_or_else(|| {
            #[cfg(feature = "demo")]
            if let Some((search, selected)) = crate::demo::link_picker_seed() {
                return Picker {
                    open: true,
                    search,
                    selected,
                };
            }
            Picker::default()
        });
    if let Some(person) = &person {
        bounded_note(
            ui,
            &palette,
            "One timeline. Reply keeps its source. Choosing another destination sends a new message.",
            false,
        );
        bounded_note(
            ui,
            &palette,
            "Text sends only in linked view. Attachments and typing indicators are disabled here until source routing is verified.",
            true,
        );
        // The primary destination new messages fall back to.
        let valid: Vec<_> = person
            .members
            .iter()
            .filter(|m| {
                app.chats.iter().any(|c| {
                    m.matches(c)
                        && !c.locked
                        && c.kind == ChatKind::Direct
                        && c.backend_origin == chat.backend_origin
                        && app.chats.iter().filter(|known| known.id == c.id).count() == 1
                })
            })
            .cloned()
            .collect();
        let state_key = egui::Id::new(("linked-person-state", &person.members));
        let mut linked = ui
            .ctx()
            .data_mut(|d| d.get_temp::<State>(state_key).unwrap_or_default());
        let primary = person
            .primary
            .as_ref()
            .and_then(|m| app.chat(&m.chat).filter(|c| m.matches(c)))
            .map(|c| format!("{}{}", network_label(c), account_name(app, c)))
            .unwrap_or_else(|| "none, choose each time".to_owned());
        egui::ComboBox::from_id_salt("person-primary")
            .selected_text(format!("Default for new messages: {primary}"))
            .show_ui(ui, |ui| {
                let mut set = |app: &mut App, primary: Option<Member>| {
                    if let Some(link) = app
                        .settings
                        .beeper
                        .people
                        .iter_mut()
                        .find(|p| **p == *person)
                    {
                        link.primary = primary;
                    }
                    if !linked.explicit && linked.reply.is_none() {
                        linked.destination = None;
                    }
                    app.mark_settings_dirty();
                };
                if ui
                    .selectable_label(person.primary.is_none(), "No default (choose each time)")
                    .clicked()
                {
                    set(app, None);
                }
                for member in &valid {
                    let c = app.chat(&member.chat).unwrap();
                    if ui
                        .selectable_label(
                            person.primary.as_ref() == Some(member),
                            format!("{} · {}", c.name, source_label(app, &valid, member)),
                        )
                        .clicked()
                    {
                        set(app, Some(member.clone()));
                    }
                }
            });
        ui.ctx().data_mut(|d| d.insert_temp(state_key, linked));
        if ui.button("Unlink").clicked() {
            app.ensure_person_loaded(&[]);
            app.unlink_person(person);
            ui.ctx().data_mut(|d| d.insert_temp(key, Picker::default()));
            return true;
        }
        let base = person.name.clone();
        let already = person.members.len();
        if ui
            .selectable_label(state.open, "Add chats to this person…")
            .clicked()
        {
            state.open = !state.open;
        }
        if state.open {
            let chosen = picker(app, ui, chat, &base, &mut state, already);
            if !chosen.is_empty() {
                if let Some(link) = app
                    .settings
                    .beeper
                    .people
                    .iter_mut()
                    .find(|p| **p == *person)
                {
                    link.members.extend(chosen);
                }
                app.mark_settings_dirty();
                state = Picker::default();
            }
        }
    } else {
        bounded_note(
            ui,
            &palette,
            "Local link only: combine this chat with the same person's other chats on this Beeper server.",
            true,
        );
        if ui.selectable_label(state.open, "Link person…").clicked() {
            state.open = !state.open;
        }
        if state.open {
            let chosen = picker(app, ui, chat, &chat.name, &mut state, 1);
            if !chosen.is_empty()
                && let Some(first) = current.clone()
            {
                app.settings.beeper.people.push(Person {
                    name: chat.name.clone(),
                    members: std::iter::once(first).chain(chosen).collect(),
                    primary: None,
                });
                app.mark_settings_dirty();
                state = Picker::default();
            }
        }
    };
    // Demo captures show the open picker without a manual scroll.
    #[cfg(feature = "demo")]
    if crate::demo::link_picker_seed().is_some() {
        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, state));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member_chat(id: &str, account: &str) -> Chat {
        Chat {
            kind: ChatKind::Direct,
            backend_origin: Some("local".into()),
            poll_capability: None,
            backend_account: Some(account.into()),
            ..Chat::new(id.into(), format!("Ada {account}"))
        }
    }

    fn linked_app(
        chats: Vec<Chat>,
        people: Vec<Person>,
    ) -> (tempfile::TempDir, App, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.chats = chats;
        app.settings.beeper.people = people;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (directory, app, ctx)
    }

    /// One frame of the conversation view's merged-person gate.
    fn handled(app: &mut App, ctx: &egui::Context, chat: &Chat) -> bool {
        let mut handled = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| handled = show(app, ui, chat),
        );
        output.textures_delta.clear();
        handled
    }

    fn ada_pair() -> (Chat, Chat, Person) {
        let wa = member_chat("ada-wa@s.whatsapp.net", "whatsapp");
        let tg = member_chat("ada-tg@telegram", "telegram");
        let person = Person {
            name: "Ada".into(),
            members: vec![Member::of(&wa).unwrap(), Member::of(&tg).unwrap()],
            primary: None,
        };
        (wa, tg, person)
    }

    #[test]
    fn name_suggestions_rank_exact_prefix_substring_subsequence() {
        assert_eq!(name_score("ada", "Ada"), Some(0));
        assert_eq!(name_score("ada", "Ada Lovelace"), Some(1));
        assert_eq!(name_score("love", "Ada Lovelace"), Some(2));
        assert_eq!(name_score("aal", "Ada Lovelace"), Some(3));
        assert_eq!(name_score("zzz", "Ada Lovelace"), None);
        assert_eq!(name_score("", "Ada"), None);
    }

    #[test]
    fn the_picker_offers_only_linkable_chats() {
        let (wa, tg, person) = ada_pair();
        let group = Chat {
            kind: ChatKind::Group,
            backend_origin: Some("local".into()),
            poll_capability: None,
            backend_account: Some("whatsapp".into()),
            ..Chat::new("family@g.us".into(), "Family".into())
        };
        let mut locked = member_chat("locked@s.whatsapp.net", "whatsapp");
        locked.locked = true;
        let other_origin = Chat {
            backend_origin: Some("elsewhere".into()),
            poll_capability: None,
            ..member_chat("ada-elsewhere@s.whatsapp.net", "whatsapp")
        };
        let free = member_chat("ada-free@signal", "signal");
        let (_dir, app, _ctx) = linked_app(
            vec![wa.clone(), tg, group, locked, other_origin, free.clone()],
            vec![person],
        );
        let candidates = link_candidates(&app, &wa);
        assert_eq!(
            candidates.iter().map(|c| &c.id).collect::<Vec<_>>(),
            vec![&free.id],
            "groups, locked chats, other origins and linked chats stay out"
        );
    }

    #[test]
    fn a_linked_chat_opens_the_combined_view() {
        let (wa, tg, person) = ada_pair();
        let plain = member_chat("bob@s.whatsapp.net", "whatsapp");
        let (_dir, mut app, ctx) =
            linked_app(vec![wa.clone(), tg.clone(), plain.clone()], vec![person]);
        assert!(
            handled(&mut app, &ctx, &wa),
            "a linked chat takes the combined view"
        );
        assert!(
            handled(&mut app, &ctx, &tg),
            "either member opens the same person"
        );
        assert!(
            !handled(&mut app, &ctx, &plain),
            "an unlinked chat stays a plain conversation"
        );
    }

    #[test]
    fn the_source_label_names_the_canonical_network() {
        let (mut wa, mut tg, person) = ada_pair();
        wa.backend_network = Some("WhatsApp".into());
        tg.backend_network = Some("Telegram".into());
        let (_dir, app, _ctx) = linked_app(vec![wa, tg], vec![person]);
        let valid = app.settings.beeper.people[0].members.clone();
        assert_eq!(source_label(&app, &valid, &valid[0]), "WhatsApp");
        assert_eq!(source_label(&app, &valid, &valid[1]), "Telegram");
    }

    #[test]
    fn a_locked_member_keeps_the_link_with_its_warning() {
        let (wa, mut tg, person) = ada_pair();
        tg.locked = true;
        let (_dir, mut app, ctx) = linked_app(vec![wa.clone(), tg], vec![person]);
        assert!(
            handled(&mut app, &ctx, &wa),
            "the link still owns the row and explains itself instead of splitting"
        );
    }

    fn sample_message(chat: &str, id: &str, timestamp: i64) -> crate::model::Message {
        crate::model::Message {
            id: id.into(),
            chat: chat.into(),
            sender: chat.into(),
            sender_name: None,
            from_me: false,
            timestamp,
            content: crate::model::Content::text(id),
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn linked_day_headers_use_shared_labels_not_iso_keys() {
        let now = "2026-03-07T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        let _clock = crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
            now,
            zone: jiff::tz::TimeZone::UTC,
            twelve_hour: false,
        });
        let (wa, tg, person) = ada_pair();
        let (_dir, mut app, ctx) = linked_app(vec![wa.clone(), tg], vec![person]);
        app.conversations.insert(
            wa.id.clone(),
            crate::app::Conversation {
                messages: vec![
                    sample_message(&wa.id, "older", now - 2 * 86_400),
                    sample_message(&wa.id, "newer", now),
                ],
                complete: true,
                ..Default::default()
            },
        );
        handled(&mut app, &ctx, &wa);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                assert!(show(&mut app, ui, &wa));
            },
        );
        output.textures_delta.clear();
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text())
                } else {
                    None
                }
            })
            .collect();
        for timestamp in [now - 2 * 86_400, now] {
            let label = crate::util::day_label(app.locale, timestamp);
            assert!(
                text.contains(&label.as_str()),
                "missing day header: {label}"
            );
            assert!(
                !text.contains(&day_stamp(timestamp).as_str()),
                "ISO day key leaked into the UI"
            );
        }
    }

    #[test]
    fn cached_timeline_order_persists_across_passes_and_invalidates_on_new_messages() {
        let (wa, tg, person) = ada_pair();
        let (_dir, mut app, ctx) = linked_app(vec![wa.clone(), tg.clone()], vec![person.clone()]);
        app.conversations.insert(
            wa.id.clone(),
            crate::app::Conversation {
                messages: vec![
                    sample_message(&wa.id, "m1", 10),
                    sample_message(&wa.id, "m3", 30),
                ],
                ..Default::default()
            },
        );
        app.conversations.insert(
            tg.id.clone(),
            crate::app::Conversation {
                messages: vec![
                    sample_message(&tg.id, "m2", 20),
                    sample_message(&tg.id, "m4", 40),
                ],
                ..Default::default()
            },
        );

        // First pass populates the cached order
        assert!(handled(&mut app, &ctx, &wa));

        let key = egui::Id::new(("linked-person-state", &person.members)).with("timeline-order");
        type CachedTimeline = (
            Vec<(String, usize, Option<i64>, Option<i64>)>,
            Vec<(String, usize)>,
        );
        let cached = ctx.data(|d| d.get_temp::<CachedTimeline>(key));
        assert!(cached.is_some(), "timeline order should be cached");
        let (_, indices) = cached.unwrap();
        assert_eq!(
            indices,
            vec![
                (wa.id.clone(), 0),
                (tg.id.clone(), 0),
                (wa.id.clone(), 1),
                (tg.id.clone(), 1)
            ]
        );

        // Second pass reuses the cached order
        assert!(handled(&mut app, &ctx, &wa));
        let cached2 = ctx.data(|d| d.get_temp::<CachedTimeline>(key)).unwrap();
        assert_eq!(cached2.1, indices);

        // Adding an older message (as when loading history backwards) invalidates and reorders
        app.conversations
            .get_mut(&wa.id)
            .unwrap()
            .messages
            .insert(0, sample_message(&wa.id, "m0", 5));
        assert!(handled(&mut app, &ctx, &wa));
        let cached3 = ctx.data(|d| d.get_temp::<CachedTimeline>(key)).unwrap();
        assert_eq!(
            cached3.1,
            vec![
                (wa.id.clone(), 0),
                (wa.id.clone(), 1),
                (tg.id.clone(), 0),
                (wa.id.clone(), 2),
                (tg.id.clone(), 1)
            ]
        );
    }
}
