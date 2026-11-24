//! Message identity and outgoing delivery receipts, bound to the original source.

use egui::{Align, Frame, Layout, Margin, Sense, vec2};

use super::widgets;
use crate::app::App;
use crate::i18n::{gettext, ngettext};
use crate::model::{Action, Delivery, Message, MessageReceipts, Recipient};
use crate::theme::{self, Icon, Palette};

/// Resolve only the message's original source, never the linked person's default.
pub(crate) fn source_facts(app: &App, message: &Message) -> Vec<(String, String)> {
    let missing = gettext(app.locale, "Not available").into_owned();
    let label = |text| gettext(app.locale, text).into_owned();
    let Some(chat) = app.chat(&message.chat) else {
        return vec![
            (label("Bridge"), missing.clone()),
            (label(if message.from_me { "To" } else { "From" }), missing),
        ];
    };
    let inventory = chat
        .backend_origin
        .as_ref()
        .and_then(|origin| app.beeper_accounts.get(origin))
        .and_then(|accounts| {
            accounts
                .iter()
                .find(|a| Some(&a.id) == chat.backend_account.as_ref())
        });
    let bridge = chat
        .backend_network
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            inventory
                .map(|a| a.network.as_str())
                .filter(|s| !s.trim().is_empty())
        })
        .or(chat.network.as_deref().filter(|s| !s.trim().is_empty()))
        .unwrap_or(
            if chat.backend_origin.is_none() && chat.backend_account.is_none() {
                "WhatsApp"
            } else {
                "Beeper"
            },
        );
    let account = if chat.backend_origin.is_some() || chat.backend_account.is_some() {
        inventory
            .map(|a| a.name.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| chat.backend_account.clone())
    } else {
        app.me_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| app.me.clone())
    }
    .unwrap_or_else(|| missing.clone());
    let target = if message.from_me {
        chat.name.clone()
    } else if chat.backend_account.is_some() {
        message
            .sender_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| chat.participant_names.get(&message.sender).cloned())
            .unwrap_or_else(|| message.sender.clone())
    } else {
        app.display_name_or(&message.sender, message.sender_name.as_deref())
    };
    let mut facts = vec![
        (label("Bridge"), bridge.into()),
        (label("Account"), account),
        (
            label(if message.from_me { "To" } else { "From" }),
            if target.trim().is_empty() {
                missing
            } else {
                target
            },
        ),
    ];
    if !message.from_me && chat.kind == crate::model::ChatKind::Group {
        facts.push((label("In"), chat.name.clone()));
    }
    // A reported handle identifies the counterpart, not our own account.
    if chat.kind == crate::model::ChatKind::Direct {
        let handles = chat
            .handles
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if !handles.is_empty() {
            facts.push((label("Username / phone"), handles.join("\n")));
        }
    }
    facts
}

fn identity(app: &App, ui: &mut egui::Ui, message: &Message) {
    for (label, value) in source_facts(app, message) {
        ui.add_space(8.0);
        theme::text(ui, label, theme::regular(12.0), app.palette.secondary);
        let response = ui.add(
            egui::Label::new(
                egui::RichText::new(value)
                    .font(theme::medium(14.0))
                    .color(app.palette.text),
            )
            .wrap(),
        );
        #[cfg(test)]
        assert!(
            response.rect.right() <= ui.max_rect().right() + 1.0,
            "identity value overflowed: {:?}",
            response.rect
        );
        #[cfg(not(test))]
        let _ = response;
    }
    ui.add_space(8.0);
    ui.separator();
}

pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &str, id: &str) {
    let palette = app.palette;
    let locale = app.locale;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(locale, "Message info"),
            theme::semibold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &gettext(locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    let Some(message) = app
        .conversations
        .get(chat)
        .and_then(|c| c.message(id))
        .cloned()
    else {
        widgets::rich_text(
            ui,
            &gettext(locale, "This message is no longer available."),
            theme::regular(14.0),
            palette.secondary,
        );
        return;
    };
    // Everything below the title scrolls together, and only once the dialog
    // is as tall as poll results can grow, so a short list shows in full.
    let height = widgets::dialog_scroll_height(ui);
    let area = egui::ScrollArea::vertical()
        .id_salt(("message-info", chat, id))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.add_space(8.0);
            receipts(app, ui, chat, id, &message);
            // The list ends clear of the dialog's edge rather than on it.
            ui.add_space(8.0);
        });
    let bubble = super::conversation::bubble_id(chat, id);
    ui.ctx().data_mut(|data| {
        data.insert_temp(bubble.with("message-info-viewport"), area.inner_rect);
        data.insert_temp(bubble.with("message-info-content"), area.content_size);
    });
}

/// The message, then who received, read, or played it.
fn receipts(app: &mut App, ui: &mut egui::Ui, chat: &str, id: &str, message: &Message) {
    let palette = app.palette;
    let delivery = super::conversation::display_delivery(
        message.status,
        message.from_me,
        crate::model::ChatKind::from_id(chat),
        app.account_receipts_off,
    );
    preview(ui, &palette, app.locale, message, delivery);
    // The bubble's stamp is short; here the full moment, with seconds and
    // the zone, pins the receipt times beside it to an exact instant.
    identity(app, ui, message);
    ui.add_space(4.0);
    theme::text(
        ui,
        gettext(
            app.locale,
            if message.from_me {
                "Sent"
            } else {
                "Message time"
            },
        ),
        theme::medium(13.0),
        palette.text,
    );
    theme::text(
        ui,
        crate::util::full_stamp(message.timestamp),
        theme::regular(12.5),
        palette.secondary,
    );
    if !message.from_me {
        return;
    }
    if app
        .chat(chat)
        .map(|c| c.kind)
        .unwrap_or_else(|| crate::model::ChatKind::from_id(chat))
        != crate::model::ChatKind::Group
    {
        direct(app, ui, message);
        return;
    }
    let Some(receipts) = app
        .message_receipts
        .clone()
        .filter(|receipts| receipts.chat == chat && receipts.message == id)
    else {
        widgets::rich_text(
            ui,
            &gettext(app.locale, "Loading…"),
            theme::regular(13.0),
            palette.secondary,
        );
        return;
    };
    group(app, ui, &receipts);
}

/// The message as its bubble shows it, shortened to a few lines.
fn preview(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    message: &Message,
    delivery: Delivery,
) {
    let text = widgets::line(
        ui,
        &message.content.summary_in(locale),
        theme::regular(14.0),
        palette.text,
        (ui.available_width() * 0.85 - 24.0).max(1.0),
        4,
    );
    // Our bubbles sit on the right, as wide as their text.
    let width = text.size().x.max(90.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 0.0),
        if message.from_me {
            Layout::right_to_left(Align::Min)
        } else {
            Layout::left_to_right(Align::Min)
        },
        |ui| {
            // Room for the tail, which reaches out past the bubble's side.
            ui.add_space(widgets::TAIL_WIDTH);
            let stamp = crate::util::bubble_stamp(locale, message.timestamp);
            bubble(ui, palette, text, width, &stamp, delivery, message.from_me);
        },
    );
}

fn bubble(
    ui: &mut egui::Ui,
    palette: &Palette,
    text: widgets::Line,
    width: f32,
    stamp: &str,
    delivery: Delivery,
    own: bool,
) {
    // Drawn as the chat draws our own messages: shadow, raised edge, tail.
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let shown = Frame::new()
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let clock = ui.painter().layout_no_wrap(
                stamp.to_owned(),
                theme::regular(11.5),
                palette.secondary,
            );
            let (rect, _) =
                ui.allocate_exact_size(vec2(width, text.size().y + 18.0), Sense::hover());
            text.paint(ui, rect.min, palette.text);
            let ticks = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 16.0, rect.bottom() - 16.0),
                vec2(16.0, 16.0),
            );
            if own {
                widgets::ticks(ui, palette, ticks, delivery);
            }
            ui.painter().galley(
                egui::pos2(
                    ticks.left() - 4.0 - clock.size().x,
                    ticks.center().y - clock.size().y / 2.0,
                ),
                clock,
                palette.secondary,
            );
        });
    ui.painter().set(
        backdrop,
        widgets::bubble_shape(
            palette,
            shown.response.rect,
            if own {
                palette.bubble_out
            } else {
                palette.bubble_in
            },
            Some(if own {
                widgets::Side::Right
            } else {
                widgets::Side::Left
            }),
            ui.ctx().pixels_per_point(),
            ui.clip_rect().y_range(),
        ),
    );
}

/// A direct chat's single recipient: the times its ticks turned.
fn direct(app: &App, ui: &mut egui::Ui, message: &Message) {
    let palette = app.palette;
    let locale = app.locale;
    let delivery = super::conversation::display_delivery(
        message.status,
        message.from_me,
        crate::model::ChatKind::from_id(&message.chat),
        app.account_receipts_off,
    );
    let read = matches!(delivery, Delivery::Read | Delivery::Played);
    let delivered = read || delivery == Delivery::Delivered;
    let label = if delivery == Delivery::Played {
        gettext(locale, "Played")
    } else {
        gettext(locale, "Read")
    };
    let stages = [
        (
            label,
            read,
            message.read_at,
            if read {
                palette.read
            } else {
                palette.secondary
            },
        ),
        (
            gettext(locale, "Delivered"),
            delivered,
            message.delivered_at,
            palette.secondary,
        ),
    ];
    for (label, reached, at, color) in stages {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            theme::paint_icon(ui, Icon::DeliveryTicks, rect, 18.0, color);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                theme::text(ui, label, theme::medium(14.0), palette.text);
                let when = match (reached, known_time(at)) {
                    (true, Some(at)) => crate::util::moment_stamp(locale, at),
                    (true, None) => gettext(locale, "Time not recorded").into_owned(),
                    (false, _) => "—".to_owned(),
                };
                theme::text(ui, when, theme::regular(12.0), palette.secondary);
            });
        });
    }
}

fn group(app: &mut App, ui: &mut egui::Ui, receipts: &MessageReceipts) {
    let palette = app.palette;
    let locale = app.locale;
    let known = receipts.audience_known();
    // Receipts were not kept before this dialog existed, and a message sent
    // before then has no saved audience. Say so instead of implying that
    // nobody has read it.
    if receipts.recipients.is_empty() {
        ui.add_space(8.0);
        note(
            ui,
            &gettext(
                locale,
                "No receipts were recorded for this message. Details are available for messages sent from now on.",
            ),
            &palette,
        );
        return;
    }
    if !known {
        ui.add_space(8.0);
        note(
            ui,
            &gettext(
                locale,
                "Some receipts for this message were not recorded. Details are complete for messages sent from now on.",
            ),
            &palette,
        );
    }
    let played = receipts.played();
    if !played.is_empty() {
        section(
            app,
            ui,
            &gettext(locale, "Played by"),
            palette.read,
            &played,
            |recipient| recipient.played_at,
        );
    }
    section(
        app,
        ui,
        &gettext(locale, "Read by"),
        palette.read,
        &receipts.read(),
        |recipient| recipient.read_at,
    );
    section(
        app,
        ui,
        &gettext(locale, "Delivered to"),
        palette.secondary,
        &receipts.delivered(),
        |recipient| recipient.delivered_at,
    );
    let remaining = receipts.remaining();
    if remaining > 0 {
        ui.add_space(12.0);
        theme::text(
            ui,
            ngettext(locale, "{} remaining", "{} remaining", remaining as u32)
                .replace("{}", &remaining.to_string()),
            theme::regular(13.0),
            palette.secondary,
        );
    }
}

fn section(
    app: &mut App,
    ui: &mut egui::Ui,
    title: &str,
    color: egui::Color32,
    recipients: &[&Recipient],
    at: impl Fn(&Recipient) -> Option<i64>,
) {
    let palette = app.palette;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        theme::paint_icon(ui, Icon::DeliveryTicks, rect, 16.0, color);
        theme::text(ui, title, theme::semibold(14.0), palette.text);
    });
    if recipients.is_empty() {
        ui.add_space(2.0);
        theme::text(
            ui,
            gettext(app.locale, "No one yet"),
            theme::regular(12.5),
            palette.secondary,
        );
        return;
    }
    for recipient in recipients {
        ui.add_space(6.0);
        let name = app.display_name(&recipient.id);
        let picture = app.avatar(&recipient.id);
        ui.horizontal(|ui| {
            widgets::avatar(ui, &palette, &name, &recipient.id, 34.0, picture.as_deref());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                widgets::rich_text(ui, &name, theme::regular(14.0), palette.text);
                if let Some(at) = known_time(at(recipient)) {
                    theme::text(
                        ui,
                        crate::util::moment_stamp(app.locale, at),
                        theme::regular(12.0),
                        palette.secondary,
                    );
                }
            });
        });
    }
}

/// A receipt time of zero or less is unknown, not the Unix epoch. Archives
/// filed before history receipts dropped it still hold a zero.
fn known_time(at: Option<i64>) -> Option<i64> {
    at.filter(|&at| at > 0)
}

/// Secondary text wrapped over as many lines as it needs.
fn note(ui: &mut egui::Ui, text: &str, palette: &Palette) {
    let width = ui.available_width().max(1.0);
    let line = widgets::line(
        ui,
        text,
        theme::regular(13.0),
        palette.secondary,
        width,
        usize::MAX,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, line.size().y), Sense::hover());
    line.paint(ui, rect.min, palette.secondary);
}

#[cfg(test)]
mod tests {
    use super::known_time;

    #[test]
    fn a_zero_receipt_time_is_unknown() {
        assert_eq!(known_time(Some(0)), None);
        assert_eq!(known_time(Some(-1)), None);
        assert_eq!(known_time(None), None);
        assert_eq!(known_time(Some(1_700_000_000)), Some(1_700_000_000));
    }
}
