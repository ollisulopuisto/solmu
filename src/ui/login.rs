//! Sign-in to the local Beeper endpoint.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if !app.settings.media_tools_onboarding_done {
        card(
            app,
            ui,
            "media-tools-onboarding",
            "Voice and video transcription",
            |app, ui| {
                super::media_tools::show(app, ui);
                let busy = app.media_tool_setup.busy();
                ui.add_enabled_ui(!busy, |ui| {
                    if ui
                        .button(crate::i18n::gettext(app.locale, "Continue"))
                        .clicked()
                        || ui
                            .button(crate::i18n::gettext(app.locale, "Skip for now"))
                            .clicked()
                    {
                        app.settings.media_tools_onboarding_done = true;
                        app.mark_settings_dirty();
                    }
                });
            },
        );
        return;
    }
    card(
        app,
        ui,
        "login",
        "One client for your Beeper accounts.",
        body,
    );
}

/// The centred card of the sign-in and lock screens: the logo, the name,
/// a line under it, and `body`.
pub(super) fn card(
    app: &mut App,
    ui: &mut egui::Ui,
    salt: &str,
    tagline: &str,
    body: impl FnOnce(&mut App, &mut egui::Ui),
) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let top = theme::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = (460.0_f32.min(rect.width() - 24.0)).max(0.0);
            // Center the card using its previous height. Its content determines
            // the next frame's height.
            let height_id = ui.id().with(format!("{salt}-card-height"));
            let known_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(height_id))
                .unwrap_or(560.0);
            let card_height = (known_height.min(rect.height() - 24.0)).max(0.0);
            let card =
                egui::Rect::from_center_size(rect.center(), Vec2::new(card_width, card_height));
            let mut card_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card)
                    .layout(Layout::top_down(Align::Center)),
            );
            let shown = Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(32))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    ui.set_width((card_width - 64.0).max(0.0));
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
                    theme::mark(ui, logo.center(), 64.0);
                    ui.add_space(4.0);
                    theme::text(ui, "Solmu", theme::bold(28.0), palette.text);
                    theme::text(ui, tagline, theme::regular(14.5), palette.secondary);
                    ui.add_space(16.0);
                    body(app, ui);
                });
            let height = shown.response.rect.height();
            if (height - known_height).abs() > 2.0 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(height_id, height.round()));
                ui.ctx().request_repaint();
            }
        });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if ui.button("Sign in to local Beeper Server").clicked() {
        app.beeper_signin.open = true;
    }
    match app.link.clone() {
        LinkStatus::Starting | LinkStatus::Connecting => {
            busy(ui, palette.accent, &app.link.connection_copy(true));
        }
        LinkStatus::Connected => {
            busy(ui, palette.accent, &app.link.connection_copy(true));
        }
        LinkStatus::Disconnected { .. } => {
            theme::icon(ui, Icon::WifiOff, 28.0, palette.warning);
            theme::paragraph(
                ui,
                app.link.connection_copy(true),
                theme::regular(13.5),
                palette.text,
            );
            ui.add_space(8.0);
            theme::paragraph(
                ui,
                "Solmu will keep checking this existing endpoint. No messages are queued for automatic sending.",
                theme::regular(12.0),
                palette.secondary,
            );
        }
        LinkStatus::LoggedOut => {
            theme::icon(ui, Icon::WifiOff, 28.0, palette.warning);
            theme::paragraph(
                ui,
                "The endpoint signed this client out. Reconnecting.",
                theme::regular(14.0),
                palette.text,
            );
            ui.add_space(8.0);
            busy(ui, palette.accent, &app.link.connection_copy(true));
        }
        LinkStatus::Failed(message) => {
            theme::icon(ui, Icon::CircleAlert, 28.0, palette.danger);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(message)
                        .font(theme::regular(13.5))
                        .color(palette.danger),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            theme::paragraph(
                ui,
                "Check the existing API endpoint and local token, then relaunch Solmu. No accounts were linked or migrated.",
                theme::regular(13.0),
                palette.secondary,
            );
        }
    }
}

pub(super) fn busy(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        let width = 24.0
            + 8.0
            + ui.painter()
                .layout_no_wrap(label.to_owned(), theme::medium(14.0), color)
                .size()
                .x;
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        theme::spinner(ui, 18.0, color);
        theme::text(ui, label, theme::medium(14.0), color);
    });
}
