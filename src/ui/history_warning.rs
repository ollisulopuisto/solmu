//! Presentation of backend history diagnostics. Keep transport/API details raw,
//! but put every Solmu-owned sentence through gettext at the UI boundary.
use crate::i18n::{Locale, gettext};

pub(crate) fn reason(locale: Locale, error: &str) -> String {
    match error {
        "Search result not reached: Beeper reported no older messages. The result may no longer be available in this chat's returned history." => gettext(locale, "Search result not reached: Beeper reported no older messages. The result may no longer be available in this chat's returned history.").into_owned(),
        "Cannot reach the search result: older-history cursor unavailable. Retry after history recovers." => gettext(locale, "Cannot reach the search result: older-history cursor unavailable. Retry after history recovers.").into_owned(),
        "Search result not reached: an older-history cursor repeated. Retry after history recovers." => gettext(locale, "Search result not reached: an older-history cursor repeated. Retry after history recovers.").into_owned(),
        "Older history cursor did not advance; some history may be missing. Retry history after the service recovers." => gettext(locale, "Older history cursor did not advance; some history may be missing. Retry history after the service recovers.").into_owned(),
        _ => {
            if let Some(detail) = error.strip_prefix("Search result history unavailable: ") {
                return gettext(locale, "Search result history unavailable: {error}").replace("{error}", detail);
            }
            if let Some(pages) = error.strip_prefix("Search result not reached: Solmu stopped at its ")
                .and_then(|s| s.strip_suffix("-page safety limit. Click the result again to continue."))
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            {
                return gettext(locale, "Search result not reached: Solmu stopped at its {pages}-page safety limit. Click the result again to continue.").replace("{pages}", pages);
            }
            error.into()
        }
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    error: &str,
    color: egui::Color32,
) -> egui::Response {
    let reason = reason(locale, error);
    let text = gettext(locale, "History: {error}").replace("{error}", &reason);
    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_and_unrecognized_diagnostics_remain_literal() {
        for error in [
            "Search result history unavailable: upstream fixture 503 {pages} <tag>",
            "Search result not reached: Solmu stopped at its 400-page safety limit. Click the result again to continue.",
            "Search result not reached: Solmu stopped at its not-a-number-page safety limit. Click the result again to continue.",
            "Search result not reached: Solmu stopped at its -page safety limit. Click the result again to continue.",
            "Search result not reached: an older-history cursor repeated. Retry after history recovers.",
            "unknown backend diagnostic: {error} <literal>",
        ] {
            assert_eq!(reason(Locale::English, error), error);
        }
    }

    #[test]
    fn ordinary_warning_wraps_inside_narrow_and_wide_viewports() {
        let error = "Search result not reached: Solmu stopped at its 400-page safety limit. Click the result again to continue.";
        for width in [240.0, 720.0] {
            let ctx = egui::Context::default();
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 220.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    rect = show(ui, Locale::English, error, egui::Color32::YELLOW).rect;
                },
            );
            assert!(
                rect.height() > 0.0 && rect.max.x <= width,
                "{rect:?} at {width}"
            );
            let text: String = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) => Some(t.galley.text()),
                    _ => None,
                })
                .collect();
            assert!(text.contains("400-page safety limit"), "{text:?}");
            assert!(
                text.contains("Click the result again to continue."),
                "{text:?}"
            );
            output.textures_delta.clear();
        }
    }
}
