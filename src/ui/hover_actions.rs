//! Cross-under toolbar geometry. Each action owns one disjoint rectangle.
use egui::{Color32, CornerRadius, Rect, Stroke, Vec2, pos2, vec2};

pub const CELL: f32 = 36.0;
pub const BAND: f32 = 44.0;
pub const STEM: f32 = 44.0;

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub reactions: Rect,
    pub vertical: Rect,
    pub reply: Rect,
    pub star: Rect,
}
impl Geometry {
    pub fn place(bubble: Rect, bounds: Rect, count: usize) -> Option<Self> {
        let width = count as f32 * CELL + 12.0;
        let height = BAND + STEM * 2.0;
        if bounds.width() < width || bounds.height() < height {
            return None;
        }
        // Keep both flush joins on the right, including for outgoing messages.
        let edge = bubble.right() + 6.0 + STEM;
        let x = (edge - width).clamp(bounds.left(), bounds.right() - width);
        let y = (bubble.bottom() + 8.0).clamp(bounds.top() + STEM, bounds.bottom() - BAND - STEM);
        let reactions = Rect::from_min_size(pos2(x, y), vec2(width, BAND));
        let stem_x = reactions.right() - STEM;
        let vertical = Rect::from_min_size(pos2(stem_x, y - STEM), vec2(STEM, height));
        let placed = Self {
            reactions,
            vertical,
            reply: Rect::from_min_size(vertical.min, Vec2::splat(STEM)),
            star: Rect::from_min_size(pos2(stem_x, reactions.bottom()), Vec2::splat(STEM)),
        };
        // Edge clamping must never turn hover chrome into a cover over the
        // message's own links, voice controls or double-click padding.
        if [placed.reactions, placed.reply, placed.star]
            .into_iter()
            .any(|rect| rect.shrink(0.01).intersects(bubble))
        {
            return None;
        }
        Some(placed)
    }
    pub fn bounds(self) -> Rect {
        self.reactions.union(self.vertical)
    }
    pub fn cell(self, index: usize) -> Rect {
        Rect::from_min_size(
            self.reactions.min + vec2(6.0 + index as f32 * CELL, 4.0),
            Vec2::splat(CELL),
        )
    }
}

/// Rear rounded stem first; foreground bar has flush corners at the crossing.
pub fn paint(
    painter: &egui::Painter,
    g: Geometry,
    rear: Color32,
    front: Color32,
    outline: Color32,
) {
    painter.rect(
        g.vertical,
        12.0,
        rear,
        Stroke::new(1.0, outline),
        egui::StrokeKind::Inside,
    );
    let corners = CornerRadius {
        nw: 12,
        sw: 12,
        ne: 0,
        se: 0,
    };
    painter.rect(
        g.reactions,
        corners,
        front,
        Stroke::new(1.0, outline),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edges_clamps_and_hit_regions_are_disjoint() {
        for x in [0.0, 80.0, 390.0] {
            for y in [-30.0, 120.0, 590.0] {
                let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 640.0));
                let g = Geometry::place(
                    Rect::from_min_size(pos2(x, y), vec2(100.0, 50.0)),
                    bounds,
                    7,
                );
                let Some(g) = g else {
                    continue;
                };
                assert!(bounds.contains_rect(g.bounds()));
                assert!(!g.reply.shrink(0.01).intersects(g.reactions));
                assert!(!g.star.shrink(0.01).intersects(g.reactions));
                for i in 0..7 {
                    assert!(g.reactions.contains_rect(g.cell(i)));
                    assert!(!g.cell(i).intersects(g.reply));
                    assert!(!g.cell(i).intersects(g.star));
                    for j in 0..i {
                        assert!(!g.cell(i).shrink(0.01).intersects(g.cell(j)));
                    }
                }
            }
        }
    }
    #[test]
    fn edge_clamp_never_covers_the_hovered_message() {
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 640.0));
        for x in (0..400).step_by(20) {
            for y in (0..620).step_by(20) {
                let bubble = Rect::from_min_size(pos2(x as f32, y as f32), vec2(120.0, 50.0));
                if let Some(g) = Geometry::place(bubble, bounds, 7) {
                    for r in [g.reactions, g.reply, g.star] {
                        assert!(!r.shrink(0.01).intersects(bubble));
                    }
                }
            }
        }
        assert!(
            Geometry::place(
                Rect::from_min_max(pos2(220.0, 580.0), pos2(410.0, 630.0)),
                bounds,
                7
            )
            .is_none()
        );
    }
    #[test]
    fn tiny_viewports_keep_existing_affordance_instead_of_overflowing() {
        let bubble = Rect::from_min_size(pos2(20.0, 20.0), vec2(80.0, 50.0));
        for size in [vec2(220.0, 640.0), vec2(420.0, 120.0)] {
            assert!(
                Geometry::place(bubble, Rect::from_min_size(egui::Pos2::ZERO, size), 7).is_none()
            );
        }
    }
}
