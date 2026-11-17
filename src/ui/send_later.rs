//! Composer Send Later presets. No message is queued until a preset is chosen.

use egui::{Key, Modifiers};
use jiff::{Zoned, civil::Date};

use super::focus::{Stop, TabStop};
use super::widgets;
use crate::app::App;
use crate::model::{Action, Chat, Content};
use crate::theme::{self, Icon};

fn popup_id() -> egui::Id {
    egui::Id::new("send-later-presets")
}

#[derive(Clone, Default)]
struct Menu {
    selected: usize,
    chosen: Option<usize>,
    owner: Option<(crate::model::AccountId, String)>,
    morning: Option<Zoned>,
    parsed: Option<Zoned>,
}

/// Consume menu keys before a focused composer can insert digits or send.
pub fn keyboard(ctx: &egui::Context) -> bool {
    if !egui::Popup::is_id_open(ctx, popup_id()) {
        return false;
    }
    let mut menu = ctx
        .data_mut(|data| data.get_temp::<Menu>(popup_id()))
        .unwrap_or_default();
    let mut escaped = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if modifiers.matches_exact(Modifiers::NONE) => {
                match key {
                    Key::ArrowUp => {
                        if let Some(next) =
                            ((menu.selected + 1)..4).find(|i| *i != 1 || menu.parsed.is_some())
                        {
                            menu.selected = next;
                        }
                    }
                    Key::ArrowDown => {
                        if let Some(next) = (0..menu.selected)
                            .rev()
                            .find(|i| *i != 1 || menu.parsed.is_some())
                        {
                            menu.selected = next;
                        }
                    }
                    Key::Enter => menu.chosen = Some(menu.selected),
                    Key::Escape => escaped = true,
                    Key::Num1 => menu.chosen = Some(0),
                    Key::Num2 => menu.chosen = Some(1),
                    Key::Num3 => menu.chosen = Some(2),
                    Key::Num4 => menu.chosen = Some(3),
                    _ => return true,
                }
                false
            }
            egui::Event::Text(text) if ["1", "2", "3", "4"].contains(&text.as_str()) => false,
            _ => true,
        });
    });
    ctx.data_mut(|data| data.insert_temp(popup_id(), menu));
    if escaped {
        egui::Popup::close_id(ctx, popup_id());
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
    }
    true
}

pub fn width(ctx: &egui::Context) -> f32 {
    if egui::Popup::is_id_open(ctx, popup_id()) {
        104.0
    } else {
        34.0
    }
}

pub fn open(ctx: &egui::Context, app: &App, chat: &Chat) {
    let now = Zoned::now();
    ctx.data_mut(|data| {
        data.insert_temp(
            popup_id(),
            Menu {
                owner: Some((app.account().id.clone(), chat.id.clone())),
                morning: morning(&now),
                parsed: previous_time(app, &chat.id, &now),
                ..Default::default()
            },
        )
    });
    egui::Popup::open_id(ctx, popup_id());
}

/// Used by both the ordinary and linked composers, inside their source route.
pub fn button(app: &mut App, ui: &mut egui::Ui, chat: &Chat, open: bool) {
    let enabled = app.editing.is_none()
        && !app.composer.trim().is_empty()
        && app.dialog.is_none()
        && !app.has_unsent_media_here();
    let owner = (app.account().id.clone(), chat.id.clone());
    let menu = ui
        .ctx()
        .data(|data| data.get_temp::<Menu>(popup_id()))
        .unwrap_or_default();
    if !enabled || menu.owner.as_ref().is_some_and(|known| known != &owner) {
        egui::Popup::close_id(ui.ctx(), popup_id());
    }
    if open && enabled {
        self::open(ui.ctx(), app, chat);
    }
    #[cfg(feature = "demo")]
    if crate::demo::send_later_menu() && enabled && !egui::Popup::is_id_open(ui.ctx(), popup_id()) {
        self::open(ui.ctx(), app, chat);
    }
    let expanded = egui::Popup::is_id_open(ui.ctx(), popup_id());
    let palette = app.conversation_palette();
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            if expanded {
                ui.add_sized(
                    [104.0, 30.0],
                    egui::Button::new(crate::i18n::gettext(app.locale, "Send later")),
                )
            } else {
                theme::icon_button(
                    ui,
                    Icon::Clock,
                    22.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "Send later (Cmd/Ctrl+Shift+Enter)").as_ref(),
                )
            }
        })
        .inner
        .tab_stop(Stop::SendLater);
    if response.clicked() && !expanded {
        self::open(ui.ctx(), app, chat);
    }
    let snapshot = ui
        .ctx()
        .data(|data| data.get_temp::<Menu>(popup_id()))
        .unwrap_or_default();
    let morning = snapshot.morning;
    let parsed = snapshot.parsed;
    let mut chosen = ui.ctx().data_mut(|data| {
        let menu = data.get_temp_mut_or_default::<Menu>(popup_id());
        menu.chosen.take()
    });
    let selected = ui
        .ctx()
        .data(|data| data.get_temp::<Menu>(popup_id()))
        .unwrap_or_default()
        .selected;
    let mut still_open = false;
    egui::Popup::menu(&response)
        .id(popup_id())
        .open_memory(Some(egui::SetOpenCommand::Bool(egui::Popup::is_id_open(
            ui.ctx(),
            popup_id(),
        ))))
        .align(egui::RectAlign::TOP_END)
        .width(330.0)
        .frame(widgets::menu_frame(&palette))
        .show(|ui| {
            still_open = true;
            ui.small(crate::i18n::gettext(
                app.locale,
                "↑↓ select · 1–4 choose · Enter confirm · Esc close",
            ));
            ui.small(crate::i18n::gettext(
                app.locale,
                "Parsed time: most recent matching text in this destination chat.",
            ));
            // Upward menu: the fastest preset is nearest the anchor.
            for index in (0..4).rev() {
                let label = match index {
                    0 => crate::i18n::gettext(app.locale, "1   Send at 8am · {when}").replace(
                        "{when}",
                        &morning
                            .as_ref()
                            .map(|z| z.strftime("%a %d %b %H:%M").to_string())
                            .unwrap_or_else(|| {
                                crate::i18n::gettext(app.locale, "unavailable").into_owned()
                            }),
                    ),
                    1 => crate::i18n::gettext(app.locale, "2   Send at {when}").replace(
                        "{when}",
                        &parsed
                            .as_ref()
                            .map(|z| z.strftime("%a %d %b %H:%M").to_string())
                            .unwrap_or_else(|| {
                                crate::i18n::gettext(app.locale, "[no clear future time]")
                                    .into_owned()
                            }),
                    ),
                    2 => {
                        crate::i18n::gettext(app.locale, "3   Send at arbitrary time…").into_owned()
                    }
                    _ => crate::i18n::gettext(app.locale, "4   Send recurring…").into_owned(),
                };
                let available = enabled
                    && match index {
                        0 => morning.is_some(),
                        1 => parsed.is_some(),
                        _ => true,
                    };
                let row = ui.add_enabled(
                    available,
                    egui::Button::new(label).selected(index == selected),
                );
                if row.hovered() || row.has_focus() {
                    ui.ctx().data_mut(|data| {
                        data.get_temp_mut_or_default::<Menu>(popup_id()).selected = index;
                    });
                }
                if row.clicked() {
                    chosen = Some(index);
                }
            }
            if chosen.is_some() {
                ui.close();
            }
        });
    if let Some(index) = chosen.filter(|_| enabled && snapshot.owner.as_ref() == Some(&owner)) {
        match index {
            0 | 1 => {
                if let Some(due) = if index == 0 { morning } else { parsed } {
                    if due.timestamp().as_second() <= crate::util::now() + 30 {
                        app.toast_error(
                            crate::i18n::gettext(
                                app.locale,
                                "That preset is now in the past. Reopen Send later.",
                            )
                            .into_owned(),
                        );
                        return;
                    }
                    {
                        let chat = chat.id.clone();
                        app.actions.push(Action::ScheduleSend {
                            chat,
                            text: app.composer.trim().to_owned(),
                            due_at: due.timestamp().as_second(),
                            quoting: app.reply_to.as_ref().map(|target| target.id.clone()),
                            replaces: None,
                            repeat: None,
                        });
                    }
                }
            }
            _ => app.actions.push(Action::OpenSendLater {
                chat: chat.id.clone(),
                recurring: index == 3,
            }),
        }
        app.focus_composer = true;
    } else if expanded && !still_open {
        app.focus_composer = true;
    }
}

/// Reject DST gaps/overlaps rather than silently moving the chosen wall time.
fn wall(date: Date, hour: i8, minute: i8, now: &Zoned) -> Option<Zoned> {
    let local = date.at(hour, minute, 0, 0);
    let ambiguous = now.time_zone().to_ambiguous_zoned(local);
    let early = ambiguous.clone().earlier().ok()?;
    let late = ambiguous.later().ok()?;
    (early == late && early.datetime() == local).then_some(early)
}

fn morning(now: &Zoned) -> Option<Zoned> {
    let today = wall(now.date(), 8, 0, now)?;
    if today.timestamp().as_second() > now.timestamp().as_second() + 30 {
        Some(today)
    } else {
        wall(now.date().tomorrow().ok()?, 8, 0, now)
    }
}

fn previous_time(app: &App, chat: &str, now: &Zoned) -> Option<Zoned> {
    let messages = &app.conversations.get(chat)?.messages;
    let mut texts: Vec<_> = messages
        .iter()
        .filter_map(|message| {
            if let Content::Text { text, .. } = &message.content {
                Some((message.timestamp, &message.id, text.as_str()))
            } else {
                None
            }
        })
        .collect();
    texts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    for (sent, _, text) in texts {
        if let Some(result) = expression(text, sent, now) {
            return result;
        }
    }
    None
}

/// Outer Some means an expression was found, even when unsafe/past. Do not
/// skip an ambiguous newer expression and offer an unrelated older one.
fn expression(text: &str, sent: i64, now: &Zoned) -> Option<Option<Zoned>> {
    let lower = text.to_lowercase();
    let words: Vec<_> = lower
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| ",!?;()[]\"'".contains(c))
                .trim_end_matches('.')
        })
        .collect();
    let mut times = Vec::new();
    let mut unknown_number = false;
    let mut saw_clock = false;
    let mut invalid_clock = false;
    for (i, word) in words.iter().enumerate() {
        let token = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':' && c != '.');
        let meridiem = if token.ends_with("am") {
            Some(false)
        } else if token.ends_with("pm") {
            Some(true)
        } else {
            words
                .get(i + 1)
                .and_then(|next| match next.trim_end_matches([',', '.', '!', '?']) {
                    "am" => Some(false),
                    "pm" => Some(true),
                    _ => None,
                })
        };
        let token = token.trim_end_matches("am").trim_end_matches("pm");
        let prefixed = i > 0 && ["at", "klo", "kello"].contains(&words[i - 1]);
        if token.is_empty() {
            continue;
        }
        let clock_shaped = token.chars().any(|c| c.is_ascii_digit())
            && (token.contains(':') || prefixed || meridiem.is_some());
        saw_clock |= clock_shaped;
        let split = token
            .split_once(':')
            .or_else(|| prefixed.then(|| token.split_once('.')).flatten());
        let (hour, minute) = if let Some((h, m)) = split {
            if h.len() > 2 || m.len() != 2 {
                invalid_clock = true;
                continue;
            }
            (h.parse::<i8>().ok(), m.parse::<i8>().ok())
        } else if prefixed || meridiem.is_some() {
            (token.parse::<i8>().ok(), Some(0))
        } else {
            if token.chars().any(|c| c.is_ascii_digit()) && token.parse::<Date>().is_err() {
                unknown_number = true;
            }
            continue;
        };
        if let (Some(mut hour), Some(minute)) = (hour, minute) {
            if let Some(pm) = meridiem {
                if !(1..=12).contains(&hour) {
                    return Some(None);
                }
                hour = hour % 12 + if pm { 12 } else { 0 };
            }
            if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
                return Some(None);
            }
            // Bare 1..12 has no am/pm meaning unless explicitly 24h HH:MM.
            if meridiem.is_none() && split.is_none() && hour < 13 {
                return Some(None);
            }
            times.push((hour, minute));
        } else if clock_shaped {
            invalid_clock = true;
        }
    }
    if times.is_empty() {
        return saw_clock.then_some(None);
    }
    if words.iter().any(|w| {
        [
            "utc", "gmt", "eet", "eest", "pst", "pdt", "cet", "cest", "est", "edt", "cst", "cdt",
            "mst", "mdt", "ast", "adt", "bst", "ist", "jst", "kst", "aest", "aedt", "acst", "acdt",
            "awst", "nzst", "nzdt", "hkt", "sgt", "msk", "wat", "cat", "sast", "noon", "midnight",
        ]
        .contains(w)
            || w.contains('/')
            || w.contains("utc+")
            || w.contains("utc-")
            || w.contains("gmt+")
            || w.contains("gmt-")
    }) {
        return Some(None);
    }
    if times.len() != 1 || unknown_number || invalid_clock {
        return Some(None);
    }
    let sent = jiff::Timestamp::from_second(sent)
        .ok()?
        .to_zoned(now.time_zone().clone());
    let tomorrow = words.iter().any(|w| ["tomorrow", "huomenna"].contains(w));
    let today = words.iter().any(|w| ["today", "tänään"].contains(w));
    // Unsupported named days must not be mistaken for the next clock time.
    if words.iter().any(|word| {
        [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "mon",
            "tue",
            "wed",
            "thu",
            "fri",
            "sat",
            "sun",
            "ma",
            "ti",
            "ke",
            "to",
            "pe",
            "la",
            "su",
            "maanantaina",
            "tiistaina",
            "keskiviikkona",
            "torstaina",
            "perjantaina",
            "lauantaina",
            "sunnuntaina",
            "next",
            "ensi",
            "yesterday",
            "eilen",
        ]
        .contains(word)
    }) {
        return Some(None);
    }
    let dates: Vec<Date> = words
        .iter()
        .filter_map(|word| word.trim_end_matches([',', '.', '!', '?']).parse().ok())
        .collect();
    if dates.len() > 1 || (tomorrow && today) {
        return Some(None);
    }
    let explicit = tomorrow || today || !dates.is_empty();
    let date = if let Some(date) = dates.first() {
        *date
    } else if tomorrow {
        sent.date().tomorrow().ok()?
    } else if today {
        sent.date()
    } else {
        now.date()
    };
    let (hour, minute) = times[0];
    let Some(mut due) = wall(date, hour, minute, now) else {
        return Some(None);
    };
    if due.timestamp().as_second() <= now.timestamp().as_second() + 30 {
        if explicit {
            return Some(None);
        }
        let Some(next) = date
            .tomorrow()
            .ok()
            .and_then(|date| wall(date, hour, minute, now))
        else {
            return Some(None);
        };
        due = next;
    }
    Some(Some(due))
}

/// Calendar selection supplements, rather than removes, keyboard date entry.
pub fn calendar(app: &mut App, ui: &mut egui::Ui) {
    let picked = app
        .send_later_date
        .parse::<Date>()
        .unwrap_or_else(|_| crate::util::today());
    let id = egui::Id::new(("send-later-month", app.account().id.clone()));
    let mut month = ui
        .ctx()
        .data(|data| data.get_temp::<Date>(id))
        .unwrap_or(picked.first_of_month());
    ui.horizontal(|ui| {
        if ui
            .button("‹")
            .on_hover_text(crate::i18n::gettext(app.locale, "Previous month"))
            .clicked()
        {
            month = month
                .checked_add(jiff::Span::new().months(-1))
                .unwrap_or(month);
        }
        ui.label(crate::util::month_heading(app.locale, month));
        if ui
            .button("›")
            .on_hover_text(crate::i18n::gettext(app.locale, "Next month"))
            .clicked()
        {
            month = month
                .checked_add(jiff::Span::new().months(1))
                .unwrap_or(month);
        }
    });
    egui::Grid::new("send-later-calendar")
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for heading in crate::util::weekday_headings(app.locale) {
                ui.label(heading);
            }
            ui.end_row();
            let lead = month.weekday().to_monday_zero_offset();
            for _ in 0..lead {
                ui.label("");
            }
            for day in 1..=month.days_in_month() {
                if let Ok(date) = Date::new(month.year(), month.month(), day) {
                    let response = ui.add_enabled(
                        date >= crate::util::today(),
                        egui::Button::new(day.to_string()).selected(date == picked),
                    );
                    if response.on_hover_text(date.to_string()).clicked() {
                        app.send_later_date = date.to_string();
                    }
                }
                if (i32::from(lead) + i32::from(day)) % 7 == 0 {
                    ui.end_row();
                }
            }
        });
    ui.ctx().data_mut(|data| data.insert_temp(id, month));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morning_is_next_eight_and_never_less_than_thirty_seconds_away() {
        for (now, expected) in [
            (
                "2026-11-17T07:59:00+02:00[Europe/Helsinki]",
                "2026-11-17T08:00:00+02:00[Europe/Helsinki]",
            ),
            (
                "2026-11-17T07:59:45+02:00[Europe/Helsinki]",
                "2026-11-18T08:00:00+02:00[Europe/Helsinki]",
            ),
        ] {
            assert_eq!(
                morning(&now.parse().unwrap()),
                Some(expected.parse().unwrap())
            );
        }
    }

    #[test]
    fn expressions_use_message_day_and_reject_ambiguous_or_past_times() {
        let now: Zoned = "2026-11-17T10:00:00+02:00[Europe/Helsinki]"
            .parse()
            .unwrap();
        let sent = now.timestamp().as_second();
        let read = |text| {
            expression(text, sent, &now)
                .flatten()
                .map(|z| z.strftime("%Y-%m-%d %H:%M").to_string())
        };
        assert_eq!(read("Huomenna klo 14.30"), Some("2026-11-18 14:30".into()));
        assert_eq!(read("tomorrow at 2:30 pm"), Some("2026-11-18 14:30".into()));
        assert_eq!(read("at 08:00"), Some("2026-11-18 08:00".into()));
        assert_eq!(read("2026-11-20 at 18:45"), Some("2026-11-20 18:45".into()));
        for text in [
            "at 8",
            "today at 08:00",
            "tomorrow at 14:00 or 15:00",
            "Friday at 14:00",
            "Friday, at 14:00",
            "tomorrow at 14:30:45",
            "at 29:00",
            "18.11 at 14:30",
            "tomorrow at 14:30 UTC",
            "tomorrow at 14:30 EST",
            "tomorrow at 14:30 America/New_York",
            "tomorrow at 14:30 or 15:00:00",
            "tomorrow at 14:30 or 15:xx",
            "tomorrow at 14:30 or 16",
        ] {
            assert_eq!(read(text), None, "{text}");
        }
        assert!(expression("no time here", sent, &now).is_none());
        assert!(expression("I have 3 cats", sent, &now).is_none());
        assert_eq!(expression("tomorrow at 14:30:45", sent, &now), Some(None));
        assert!(
            expression("tomorrow at 14:30", sent - 3 * 86400, &now)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn menu_keys_follow_visual_direction_and_escape_keeps_the_draft() {
        let ctx = egui::Context::default();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let pass = |events: Vec<egui::Event>| {
            let was_open = egui::Popup::is_id_open(&ctx, popup_id());
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    if was_open {
                        egui::Popup::open_id(ui.ctx(), popup_id());
                    }
                    keyboard(ui.ctx());
                },
            );
            output.textures_delta.clear();
        };
        ctx.data_mut(|d| {
            d.insert_temp(
                popup_id(),
                Menu {
                    parsed: Some(
                        "2026-11-18T14:30:00+02:00[Europe/Helsinki]"
                            .parse()
                            .unwrap(),
                    ),
                    ..Default::default()
                },
            )
        });
        egui::Popup::open_id(&ctx, popup_id());
        pass(vec![key(Key::ArrowUp), key(Key::ArrowUp)]);
        assert_eq!(
            ctx.data(|d| d.get_temp::<Menu>(popup_id()))
                .unwrap()
                .selected,
            2
        );
        pass(vec![key(Key::ArrowDown), key(Key::Enter)]);
        assert_eq!(
            ctx.data(|d| d.get_temp::<Menu>(popup_id())).unwrap().chosen,
            Some(1)
        );
        pass(vec![key(Key::Num4), egui::Event::Text("4".into())]);
        assert_eq!(
            ctx.data(|d| d.get_temp::<Menu>(popup_id())).unwrap().chosen,
            Some(3)
        );
        pass(vec![key(Key::Escape)]);
        assert!(!egui::Popup::is_id_open(&ctx, popup_id()));
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("composer-text"))
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![key(Key::Num1), egui::Event::Text("1".into())],
                ..Default::default()
            },
            |ui| {
                assert!(!keyboard(ui.ctx()));
                assert_eq!(ui.ctx().input(|i| i.events.len()), 2);
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn presets_never_guess_dst_gaps_or_repeated_wall_times() {
        let spring: Zoned = "2026-03-28T12:00:00+02:00[Europe/Helsinki]"
            .parse()
            .unwrap();
        assert!(
            expression("tomorrow at 03:30", spring.timestamp().as_second(), &spring)
                .unwrap()
                .is_none()
        );
        let autumn: Zoned = "2026-10-24T12:00:00+03:00[Europe/Helsinki]"
            .parse()
            .unwrap();
        assert!(
            expression("tomorrow at 03:30", autumn.timestamp().as_second(), &autumn)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            morning(&autumn)
                .unwrap()
                .strftime("%Y-%m-%d %H:%M %:z")
                .to_string(),
            "2026-10-25 08:00 +02:00"
        );
    }
    mod integration {
        use super::*;

        #[test]
        fn choosing_a_preset_keeps_the_draft_and_uses_the_exact_destination() {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _events) = App::headless(
                crate::paths::AppDirs::under(directory.path()),
                crate::settings::Settings::default(),
            );
            let chat = Chat::new("destination".into(), "Destination".into());
            app.open_chat = Some("visible".into());
            app.composer = "Keep this draft until acknowledged".into();
            let ctx = egui::Context::default();
            open(&ctx, &app, &chat);
            ctx.data_mut(|d| d.get_temp_mut_or_default::<Menu>(popup_id()).chosen = Some(0));
            let mut output =
                ctx.run_ui(Default::default(), |ui| button(&mut app, ui, &chat, false));
            output.textures_delta.clear();
            assert!(app.actions.iter().any(|action| matches!(action,
                Action::ScheduleSend { chat, text, repeat: None, .. }
                if chat == "destination" && text == "Keep this draft until acknowledged")));
            assert_eq!(app.composer, "Keep this draft until acknowledged");
            app.actions.clear();
            open(&ctx, &app, &chat);
            ctx.data_mut(|d| d.get_temp_mut_or_default::<Menu>(popup_id()).chosen = Some(3));
            let mut output =
                ctx.run_ui(Default::default(), |ui| button(&mut app, ui, &chat, false));
            output.textures_delta.clear();
            assert!(app.actions.contains(&Action::OpenSendLater {
                chat: "destination".into(),
                recurring: true
            }));
        }

        #[test]
        fn changing_destination_while_menu_is_open_never_sends_to_the_new_chat() {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _events) = App::headless(
                crate::paths::AppDirs::under(directory.path()),
                crate::settings::Settings::default(),
            );
            let first = Chat::new("first".into(), "First".into());
            let second = Chat::new("second".into(), "Second".into());
            app.open_chat = Some(first.id.clone());
            app.composer = "Do not reroute".into();
            let ctx = egui::Context::default();
            open(&ctx, &app, &first);
            ctx.data_mut(|d| d.get_temp_mut_or_default::<Menu>(popup_id()).chosen = Some(0));
            let mut output = ctx.run_ui(Default::default(), |ui| {
                button(&mut app, ui, &second, false)
            });
            output.textures_delta.clear();
            assert!(app.actions.is_empty());
            assert_eq!(app.composer, "Do not reroute");
        }
    }
}
