//! One exact-reply tail and hit target. No timeline or sender-run inference.
use egui::{Color32, Painter, Rect, Stroke, pos2, vec2};

pub fn geometry(body: Rect, own: bool) -> Rect {
    let x = if own {
        body.right()
    } else {
        body.left() - 30.0
    };
    Rect::from_min_size(pos2(x, body.top()), vec2(30.0, 30.0))
}

pub fn paint(painter: &Painter, body: Rect, own: bool, fill: Color32, ink: Color32) {
    let rect = geometry(body, own);
    // A filled neck grows from the bubble, not a second chip inside it.
    let points = if own {
        vec![
            pos2(body.right() - 3.0, body.top()),
            pos2(rect.right() - 5.0, rect.top()),
            pos2(rect.right(), rect.top() + 5.0),
            pos2(rect.right(), rect.bottom() - 12.0),
            pos2(rect.right() - 4.0, rect.bottom() - 8.0),
            pos2(body.right() - 3.0, rect.bottom()),
        ]
    } else {
        vec![
            pos2(rect.left() + 5.0, rect.top()),
            pos2(body.left() + 3.0, body.top()),
            pos2(body.left() + 3.0, rect.bottom()),
            pos2(rect.left() + 4.0, rect.bottom() - 8.0),
            pos2(rect.left(), rect.bottom() - 12.0),
            pos2(rect.left(), rect.top() + 5.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
    // Two interlocking links, compact and passive except for the one tail target.
    let centre = rect.center();
    let stroke = Stroke::new(1.6, ink.gamma_multiply(0.85));
    for offset in [-3.5, 3.5] {
        let link = Rect::from_center_size(centre + vec2(offset, -offset * 0.6), vec2(9.0, 13.0));
        painter.rect_stroke(link, 4.0, stroke, egui::StrokeKind::Inside);
    }
}

/// Stable exact-source identity keeps keyboard focus and click dispatch on
/// the reply even when another reply is inserted before it.
pub fn interact(ui: &mut egui::Ui, rect: Rect, id: egui::Id) -> egui::Response {
    ui.interact(rect, id, egui::Sense::click())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_tail_is_outside_its_own_body_and_mirrors_without_reflow() {
        let body = Rect::from_min_size(pos2(80.0, 50.0), vec2(180.0, 80.0));
        for own in [false, true] {
            let target = geometry(body, own);
            assert!(!target.shrink(0.01).intersects(body));
            assert_eq!(target.top(), body.top());
            assert_eq!(target.size(), vec2(30.0, 30.0));
        }
    }
    #[test]
    fn pointer_and_keyboard_keep_each_reply_target_independent() {
        let ctx = egui::Context::default();
        let ids = [
            egui::Id::new(("reply-a", "original-one")),
            egui::Id::new(("reply-b", "original-one")),
            egui::Id::new(("reply-c", "original-two")),
        ];
        let mut clicked = Vec::new();
        let mut frame = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(500.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    for (i, id) in ids.iter().enumerate() {
                        let body = Rect::from_min_size(
                            pos2(50.0, 50.0 + i as f32 * 70.0),
                            vec2(200.0, 60.0),
                        );
                        // Register an earlier broad body hit area, as the app does.
                        ui.interact(body, id.with("body"), egui::Sense::click());
                        if interact(ui, geometry(body, true), *id).clicked() {
                            clicked.push(i);
                        }
                    }
                },
            );
            output.textures_delta.clear();
        };
        frame(vec![]);
        for index in 0..3 {
            let pos = pos2(265.0, 65.0 + index as f32 * 70.0);
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        }
        ctx.memory_mut(|m| m.request_focus(ids[1]));
        frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(clicked, vec![0, 1, 2, 1]);
    }
}
