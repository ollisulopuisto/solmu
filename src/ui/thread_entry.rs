//! A reserved, exact-message thread entry. Hidden ink never changes geometry.
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    hovered: bool,
    label: &str,
    font: egui::FontId,
    color: egui::Color32,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if hovered || response.hovered() || response.has_focus() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font,
            color,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn hidden_entry_keeps_rect_and_pointer_and_keyboard_use_exact_identity() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(30.0, 50.0), egui::vec2(100.0, 20.0));
        let id = egui::Id::new(("chat", "original", "open-thread"));
        let mut clicked = 0;
        let mut frame = |events, hovered| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let response = super::show(
                        ui,
                        rect,
                        id,
                        hovered,
                        "Open thread",
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );
                    assert_eq!(response.rect, rect);
                    if response.clicked() {
                        clicked += 1;
                    }
                },
            );
            output.textures_delta.clear();
        };
        frame(vec![], false);
        frame(vec![egui::Event::PointerMoved(rect.center())], true);
        for pressed in [true, false] {
            frame(
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        ctx.memory_mut(|m| m.request_focus(id));
        frame(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            false,
        );
        assert_eq!(clicked, 2);
    }
}
