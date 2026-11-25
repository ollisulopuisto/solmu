//! One exact-reply tail and hit target. No timeline or sender-run inference.
use egui::{Color32, Painter, Rect, Stroke, pos2, vec2};

pub fn geometry(body: Rect, own: bool) -> Rect {
    let x = if own {
        body.right()
    } else {
        body.left() - super::widgets::TAIL_WIDTH
    };
    Rect::from_min_size(
        pos2(x, body.top()),
        vec2(super::widgets::TAIL_WIDTH, super::widgets::TAIL_HEIGHT),
    )
}

pub const SLOT_HEIGHT: f32 = 36.0;

/// The chain is separate from the filled tail and never covers its message.
pub fn chain_geometry(body: Rect, own: bool) -> Rect {
    let x = if own {
        body.right() - 30.0
    } else {
        body.left()
    };
    Rect::from_min_size(pos2(x, body.top() - SLOT_HEIGHT), vec2(30.0, 30.0))
}

pub fn paint(painter: &Painter, body: Rect, own: bool, ink: Color32) {
    // The bubble renderer owns the regular filled tail. Do not paint a
    // second, differently shaped extension here.
    // Two interlocking links above the tail, with their own exact-source target.
    let centre = chain_geometry(body, own).center();
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
    fn chain_is_above_and_disjoint_from_body_tail_and_album_sibling() {
        let body = Rect::from_min_size(pos2(80.0, 80.0), vec2(180.0, 132.0));
        for own in [false, true] {
            let chain = chain_geometry(body, own);
            let tail = geometry(body, own);
            assert_eq!(chain.bottom() + 6.0, tail.top());
            assert!(!chain.intersects(body));
            assert!(!chain.intersects(tail));
            let sibling = body.translate(vec2(0.0, 66.0));
            assert!(!chain_geometry(sibling, own).shrink(0.01).intersects(tail));
            assert!(!geometry(sibling, own).intersects(chain));
        }
    }
    #[test]
    fn each_tail_is_outside_its_own_body_and_mirrors_without_reflow() {
        let body = Rect::from_min_size(pos2(80.0, 50.0), vec2(180.0, 80.0));
        for own in [false, true] {
            let target = geometry(body, own);
            assert!(!target.shrink(0.01).intersects(body));
            assert_eq!(target.top(), body.top());
            assert_eq!(
                target.size(),
                vec2(
                    super::super::widgets::TAIL_WIDTH,
                    super::super::widgets::TAIL_HEIGHT
                )
            );
        }
    }
    #[test]
    fn chain_paint_does_not_add_a_second_filled_tail() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint(
                ui.painter(),
                Rect::from_min_size(pos2(80.0, 80.0), vec2(180.0, 80.0)),
                true,
                Color32::WHITE,
            );
        });
        output.textures_delta.clear();
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Path(_))),
            "regular bubble renderer owns the filled tail, chain painter must not add its old polygon"
        );
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
                        if interact(ui, chain_geometry(body, true), *id).clicked() {
                            clicked.push(i);
                        }
                    }
                },
            );
            output.textures_delta.clear();
        };
        frame(vec![]);
        for index in 0..3 {
            let pos = pos2(265.0, 29.0 + index as f32 * 70.0);
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
