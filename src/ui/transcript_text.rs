//! Local transcript text, separate from playback controls.
use egui::{Color32, FontId, Id, Response, Sense, Ui, vec2};

pub struct Appearance<'a> {
    pub font: FontId,
    pub text: Color32,
    pub link: Color32,
    pub more: &'a str,
    pub less: &'a str,
}

pub struct Drawn {
    pub expanded: bool,
    pub total_lines: usize,
    #[allow(dead_code)]
    pub shown_lines: usize,
    #[allow(dead_code)]
    pub toggle: Option<Response>,
}

/// Clip the full shaped text rather than cutting UTF-8 or changing copy data.
/// The identity supplied by the caller includes account, chat, message and text.
pub fn show(ui: &mut Ui, id: Id, text: &str, look: Appearance<'_>) -> Drawn {
    let galley = ui.painter().layout(
        format!("\"{text}\""),
        look.font,
        look.text,
        ui.available_width().max(1.0),
    );
    let total_lines = galley.rows.len();
    let expanded = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false);
    let shown_lines = if expanded {
        total_lines
    } else {
        total_lines.min(4)
    };
    let height = galley
        .rows
        .get(shown_lines.saturating_sub(1))
        .map_or(0.0, |row| row.pos.y + row.row.size.y);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, height), Sense::hover());
    let clip = ui.clip_rect().intersect(rect);
    ui.painter()
        .with_clip_rect(clip)
        .galley(rect.min, galley.clone(), look.text);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    let toggle = (total_lines > 4).then(|| {
        let label = if expanded {
            look.less.to_owned()
        } else {
            look.more
                .replace("{count}", &(total_lines - shown_lines).to_string())
        };
        let response =
            ui.add(egui::Button::new(egui::RichText::new(label).color(look.link)).frame(false));
        if response.clicked() {
            ui.ctx().data_mut(|data| data.insert_temp(id, !expanded));
        }
        response
    });
    Drawn {
        expanded,
        total_lines,
        shown_lines,
        toggle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn show_more_and_less_clicks_change_only_this_transcript() {
        let ctx = egui::Context::default();
        let id = Id::new("one");
        let text = "line\n".repeat(20);
        let mut target = None;
        for frame in 0..5 {
            let events = if frame == 1 || frame == 3 {
                let pos = target.unwrap();
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let draw = show(
                        ui,
                        id,
                        &text,
                        Appearance {
                            font: FontId::proportional(12.0),
                            text: Color32::WHITE,
                            link: Color32::LIGHT_BLUE,
                            more: "Show more (+{count} lines)",
                            less: "Show less",
                        },
                    );
                    target = Some(draw.toggle.as_ref().unwrap().rect.center());
                    if frame == 2 {
                        assert!(draw.expanded);
                    }
                    if frame == 4 {
                        assert!(!draw.expanded);
                    }
                },
            );
            output.textures_delta.clear();
        }
    }
    #[test]
    fn long_transcript_collapses_expands_and_keeps_other_messages_collapsed() {
        let ctx = egui::Context::default();
        let text = (0..20)
            .map(|i| format!("Line {i} with Finnish ä and emoji 🙂."))
            .collect::<Vec<_>>()
            .join("\n");
        let id = Id::new(("account", "chat", "message", &text));
        for expanded in [false, true, false] {
            ctx.data_mut(|data| data.insert_temp(id, expanded));
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.set_max_width(220.0);
                let draw = show(
                    ui,
                    id,
                    &text,
                    Appearance {
                        font: FontId::proportional(12.0),
                        text: Color32::WHITE,
                        link: Color32::LIGHT_BLUE,
                        more: "Show more (+{count} lines)",
                        less: "Show less",
                    },
                );
                assert!(draw.total_lines >= 20);
                assert_eq!(
                    draw.shown_lines,
                    if expanded { draw.total_lines } else { 4 }
                );
                assert!(draw.toggle.is_some());
                let other = show(
                    ui,
                    Id::new(("other-account", "chat", "message", &text)),
                    &text,
                    Appearance {
                        font: FontId::proportional(12.0),
                        text: Color32::WHITE,
                        link: Color32::LIGHT_BLUE,
                        more: "Show more (+{count} lines)",
                        less: "Show less",
                    },
                );
                assert_eq!(other.shown_lines, 4);
            });
            output.textures_delta.clear();
        }
    }
}
