//! One 3x3 message action grid. Six reactions, gap, then picker/Reply/Star.
use egui::{Color32, Rect, Stroke, pos2, vec2};
pub const CELL: f32 = 44.0;
const GAP: f32 = 4.0;
const SECTION_GAP: f32 = 8.0;
pub fn focus_order(bubble: egui::Id, reply: bool, star: bool) -> Vec<egui::Id> {
    let area = bubble.with("hover-actions");
    let mut order = vec![bubble];
    order.extend((0_usize..6).map(|i| area.with(("emoji", i))));
    order.push(area.with(("action", 2_usize)));
    if reply {
        order.push(area.with(("action", 0_usize)));
    }
    if star {
        order.push(area.with(("action", 1_usize)));
    }
    order.push(bubble.with("react-keyboard"));
    order
}
pub fn next_focus(
    order: &[egui::Id],
    focused: Option<egui::Id>,
    reverse: bool,
) -> Option<egui::Id> {
    let at = order.iter().position(|id| Some(*id) == focused)?;
    let next = if reverse { at.checked_sub(1)? } else { at + 1 };
    order.get(next).copied()
}
pub fn opening_anchor(bubble: Rect, pointer: Option<egui::Pos2>, keyboard: bool) -> egui::Pos2 {
    if keyboard {
        return bubble.center();
    }
    pointer
        .filter(|p| bubble.contains(*p))
        .unwrap_or(bubble.center())
}
pub fn show_grid(scrolling: bool, pointer_active: bool, keyboard_active: bool) -> bool {
    !scrolling && (pointer_active || keyboard_active)
}
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub reactions: Rect,
    pub vertical: Rect,
    pub reply: Rect,
    pub star: Rect,
}
impl Geometry {
    #[cfg(test)]
    pub fn place(bubble: Rect, bounds: Rect, count: usize) -> Option<Self> {
        if count != 7 {
            return None;
        }
        let width = 3.0 * CELL + 2.0 * GAP;
        let height = 3.0 * CELL + 2.0 * GAP + SECTION_GAP;
        if bounds.width() < width || bounds.height() < height {
            return None;
        }
        let x = (bubble.center().x - width / 2.0).clamp(bounds.left(), bounds.right() - width);
        let y = (bubble.center().y - height / 2.0).clamp(bounds.top(), bounds.bottom() - height);
        let candidates = [
            pos2(x, bubble.bottom() + 8.0),
            pos2(x, bubble.top() - height - 8.0),
            pos2(bubble.right() + 8.0, y),
            pos2(bubble.left() - width - 8.0, y),
        ];
        for at in candidates {
            let whole = Rect::from_min_size(at, vec2(width, height));
            if !bounds.contains_rect(whole) || whole.shrink(0.01).intersects(bubble) {
                continue;
            }
            let reactions = Rect::from_min_size(at, vec2(width, 2.0 * CELL + GAP));
            let vertical = Rect::from_min_size(
                pos2(at.x, reactions.bottom() + GAP + SECTION_GAP),
                vec2(width, CELL),
            );
            let reply = Rect::from_min_size(vertical.min + vec2(CELL + GAP, 0.0), vec2(CELL, CELL));
            let star = Rect::from_min_size(
                vertical.min + vec2(2.0 * (CELL + GAP), 0.0),
                vec2(CELL, CELL),
            );
            return Some(Self {
                reactions,
                vertical,
                reply,
                star,
            });
        }
        None
    }
    /// Side-only placement protects the target bubble. Other rows may be covered.
    pub fn beside_pointer(
        bubble: Rect,
        bounds: Rect,
        count: usize,
        pointer: egui::Pos2,
    ) -> Option<Self> {
        if count != 7 {
            return None;
        }
        let width = 3.0 * CELL + 2.0 * GAP;
        let height = 3.0 * CELL + 2.0 * GAP + SECTION_GAP;
        if bounds.width() < width || bounds.height() < height {
            return None;
        }
        let y = pointer.y.clamp(bounds.top(), bounds.bottom() - height);
        let sides = [bubble.right() + 8.0, bubble.left() - width - 8.0];
        for x in sides {
            let whole = Rect::from_min_size(pos2(x, y), vec2(width, height));
            if !bounds.contains_rect(whole) || whole.shrink(0.01).intersects(bubble) {
                continue;
            }
            let reactions = Rect::from_min_size(whole.min, vec2(width, 2.0 * CELL + GAP));
            let vertical = Rect::from_min_size(
                pos2(x, reactions.bottom() + GAP + SECTION_GAP),
                vec2(width, CELL),
            );
            return Some(Self {
                reactions,
                vertical,
                reply: Rect::from_min_size(vertical.min + vec2(CELL + GAP, 0.0), vec2(CELL, CELL)),
                star: Rect::from_min_size(
                    vertical.min + vec2(2.0 * (CELL + GAP), 0.0),
                    vec2(CELL, CELL),
                ),
            });
        }
        None
    }

    /// Only the corridor directly between the facing edges, never the full
    /// bounding box that also contains unrelated neighbouring message rows.
    pub fn bridge(self, bubble: Rect) -> Rect {
        let grid = self.bounds();
        if grid.top() >= bubble.bottom() {
            Rect::from_min_max(
                pos2(grid.left().max(bubble.left()), bubble.bottom()),
                pos2(grid.right().min(bubble.right()), grid.top()),
            )
        } else if grid.bottom() <= bubble.top() {
            Rect::from_min_max(
                pos2(grid.left().max(bubble.left()), grid.bottom()),
                pos2(grid.right().min(bubble.right()), bubble.top()),
            )
        } else if grid.left() >= bubble.right() {
            Rect::from_min_max(
                pos2(bubble.right(), grid.top().max(bubble.top())),
                pos2(grid.left(), grid.bottom().min(bubble.bottom())),
            )
        } else {
            Rect::from_min_max(
                pos2(grid.right(), grid.top().max(bubble.top())),
                pos2(bubble.left(), grid.bottom().min(bubble.bottom())),
            )
        }
    }
    pub fn bounds(self) -> Rect {
        self.reactions.union(self.vertical)
    }
    pub fn cell(self, index: usize) -> Rect {
        if index == 6 {
            return Rect::from_min_size(self.vertical.min, vec2(CELL, CELL));
        }
        Rect::from_min_size(
            self.reactions.min
                + vec2(
                    (index % 3) as f32 * (CELL + GAP),
                    (index / 3) as f32 * (CELL + GAP),
                ),
            vec2(CELL, CELL),
        )
    }
}
pub fn paint(p: &egui::Painter, g: Geometry, rear: Color32, front: Color32, outline: Color32) {
    for i in 0..9 {
        let rect = match i {
            6 => g.cell(6),
            7 => g.reply,
            8 => g.star,
            _ => g.cell(i),
        };
        p.rect(
            rect,
            8.0,
            if i < 6 { front } else { rear },
            Stroke::new(1.0, outline),
            egui::StrokeKind::Inside,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_and_outside_pointer_anchor_to_target() {
        let b = Rect::from_min_max(pos2(20., 20.), pos2(200., 80.));
        assert_eq!(opening_anchor(b, Some(pos2(100., 600.)), true), b.center());
        assert_eq!(opening_anchor(b, Some(pos2(100., 600.)), false), b.center());
        assert_eq!(
            opening_anchor(b, Some(pos2(100., 30.)), false),
            pos2(100., 30.)
        );
    }
    #[test]
    fn pointer_grid_can_cover_another_row_but_never_its_target() {
        let bounds = Rect::from_min_max(pos2(0., 0.), pos2(700., 700.));
        let bubble = Rect::from_min_max(pos2(20., 20.), pos2(360., 500.));
        let neighbor = Rect::from_min_max(pos2(370., 380.), pos2(650., 620.));
        let pointer = pos2(350., 380.);
        let g = Geometry::beside_pointer(bubble, bounds, 7, pointer).unwrap();
        assert!(g.bounds().left() >= bubble.right());
        assert!((g.bounds().top() - pointer.y).abs() < 16.);
        assert!(bounds.contains_rect(g.bounds()));
        assert!(g.bounds().intersects(neighbor));
        assert!(!g.bounds().intersects(bubble));
    }
    #[test]
    fn pointer_on_left_half_still_prefers_right_side() {
        let bounds = Rect::from_min_max(pos2(0., 0.), pos2(700., 700.));
        let bubble = Rect::from_min_max(pos2(250., 80.), pos2(450., 200.));
        let g = Geometry::beside_pointer(bubble, bounds, 7, pos2(260., 100.)).unwrap();
        assert_eq!(g.bounds().left(), bubble.right() + 8.);
        assert_eq!(g.bounds().top(), 100.);
        assert!(!g.bounds().intersects(bubble));
    }
    #[test]
    fn pointer_grid_flips_left_clamps_and_refuses_top_bottom_fallback() {
        let bounds = Rect::from_min_max(pos2(0., 0.), pos2(700., 700.));
        let bubble = Rect::from_min_max(pos2(340., 20.), pos2(680., 400.));
        let g = Geometry::beside_pointer(bubble, bounds, 7, pos2(670., 690.)).unwrap();
        assert!(g.bounds().right() <= bubble.left());
        assert_eq!(g.bounds().bottom(), bounds.bottom());
        let full = Rect::from_min_max(pos2(10., 200.), pos2(690., 300.));
        assert!(Geometry::beside_pointer(full, bounds, 7, full.center()).is_none());
    }

    #[test]
    fn focus_order_skips_disabled_controls_and_scroll_yields() {
        let b = egui::Id::new("owner");
        let a = b.with("hover-actions");
        let order = focus_order(b, false, false);
        assert_eq!(order.len(), 9);
        assert!(!order.contains(&a.with(("action", 0_usize))));
        assert!(!order.contains(&a.with(("action", 1_usize))));
        assert_eq!(
            next_focus(&order, Some(order[6]), false),
            Some(a.with(("action", 2_usize)))
        );
        assert_eq!(next_focus(&order, Some(order[0]), true), None);
        assert_eq!(next_focus(&order, order.last().copied(), false), None);
        assert_eq!(next_focus(&order, Some(order[1]), true), Some(b));
        assert!(!show_grid(true, true, true));
        assert!(show_grid(false, true, false));
        assert!(show_grid(false, false, true));
    }
    #[test]
    fn below_grid_bridge_excludes_next_bubble_side_strip() {
        let bubble = Rect::from_min_max(pos2(20., 20.), pos2(300., 80.));
        let g = Geometry::place(
            bubble,
            Rect::from_min_max(pos2(0., 0.), pos2(420., 640.)),
            7,
        )
        .unwrap();
        let next = pos2(25., 140.);
        assert!(bubble.union(g.bounds()).contains(next));
        assert!(!g.bridge(bubble).contains(next));
        assert!(!g.bounds().contains(next));
        assert!(g.bridge(bubble).contains(pos2(g.bounds().center().x, 84.)));
    }
    #[test]
    fn bottom_message_flips_above_and_all_targets_stay_visible() {
        let b = Rect::from_min_max(pos2(210., 520.), pos2(405., 630.));
        let bounds = Rect::from_min_max(pos2(0., 0.), pos2(420., 640.));
        let g = Geometry::place(b, bounds, 7).unwrap();
        assert!(g.bounds().bottom() < b.top());
        assert!(bounds.contains_rect(g.bounds()));
    }
    #[test]
    fn tall_message_uses_side_or_context_fallback() {
        let bounds = Rect::from_min_max(pos2(0., 0.), pos2(620., 640.));
        let b = Rect::from_min_max(pos2(0., -200.), pos2(300., 900.));
        assert!(Geometry::place(b, bounds, 7).is_some());
        assert!(
            Geometry::place(
                Rect::from_min_max(pos2(0., -200.), pos2(620., 900.)),
                bounds,
                7
            )
            .is_none()
        );
    }
    #[test]
    fn all_nine_targets_disjoint_and_actions_separated() {
        let g = Geometry::place(
            Rect::from_min_max(pos2(30., 30.), pos2(200., 100.)),
            Rect::from_min_max(pos2(0., 0.), pos2(420., 640.)),
            7,
        )
        .unwrap();
        let mut cells = (0..7).map(|i| g.cell(i)).collect::<Vec<_>>();
        cells.extend([g.reply, g.star]);
        for (i, a) in cells.iter().enumerate() {
            assert!(g.bounds().contains_rect(*a));
            for b in &cells[..i] {
                assert!(!a.shrink(0.01).intersects(*b));
            }
        }
        assert!(g.vertical.top() - g.reactions.bottom() >= SECTION_GAP);
    }
}
