//! One 3x3 message action grid. Six reactions, gap, then picker/Reply/Star.
use egui::{Color32, Rect, Stroke, pos2, vec2};
/// One registry owns both visible corner hints and modal bare-key dispatch.
pub const GRID_KEYS: [(egui::Key, &str); 9] = [
    (egui::Key::Num1, "1"),
    (egui::Key::Num2, "2"),
    (egui::Key::Num3, "3"),
    (egui::Key::Num4, "4"),
    (egui::Key::Num5, "5"),
    (egui::Key::Num6, "6"),
    (egui::Key::Num7, "7"),
    (egui::Key::Num8, "8"),
    (egui::Key::Num9, "9"),
];
pub fn grid_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<usize> {
    if modifiers != egui::Modifiers::NONE {
        return None;
    }
    GRID_KEYS
        .iter()
        .position(|(candidate, _)| *candidate == key)
}
pub fn click_feedback(age: f64, reduced: bool) -> f32 {
    if reduced {
        return 0.0;
    }
    (1.0 - (age / 0.16).clamp(0.0, 1.0)) as f32
}
pub const CELL: f32 = 44.0;
pub const OPEN_DELAY: f64 = 0.15;
pub const CLOSE_DELAY: f64 = 0.25;
pub const HIT_PADDING: f32 = 12.0;
pub const ARRIVAL_PADDING: f32 = 24.0;
const _: () = assert!(CLOSE_DELAY > OPEN_DELAY);

/// One pointer-entry attempt. Keyboard opening bypasses this delay.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenIntent {
    since: Option<f64>,
}
impl OpenIntent {
    pub fn update(&mut self, fresh: bool, eligible: bool, now: f64) -> bool {
        if !eligible {
            self.since = None;
            return false;
        }
        if fresh {
            self.since = Some(now);
        }
        if self.since.is_some_and(|since| now - since >= OPEN_DELAY) {
            self.since = None;
            return true;
        }
        false
    }
    pub fn remaining(self, now: f64) -> Option<f64> {
        self.since
            .map(|since| (OPEN_DELAY - (now - since)).max(0.0))
    }
}

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
/// Capture the pointer position once on entry; movement inside cannot drift the grid.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntryAnchor {
    anchor: Option<egui::Pos2>,
    over: bool,
    pointer: Option<egui::Pos2>,
    fresh: bool,
}
impl EntryAnchor {
    pub fn update(&mut self, bubble: Rect, pointer: Option<egui::Pos2>) {
        let over = pointer.is_some_and(|p| bubble.contains(p));
        self.fresh = over && !self.over && pointer != self.pointer;
        if self.fresh {
            self.anchor = pointer;
        }
        self.over = over;
        self.pointer = pointer;
    }
    pub fn fresh(self) -> bool {
        self.fresh
    }
    pub fn anchor(self) -> Option<egui::Pos2> {
        self.anchor
    }
}
/// Exit is driven by real input, never a layout-only hit-test change.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExitGrace {
    pub since: Option<f64>,
}
impl ExitGrace {
    pub fn update(&mut self, moved: bool, in_reach: bool, dismiss: bool, now: f64) -> bool {
        if dismiss {
            return true;
        }
        if moved {
            if in_reach {
                self.since = None;
            } else if self.since.is_none() {
                self.since = Some(now);
            }
        }
        self.since.is_some_and(|since| now - since >= CLOSE_DELAY)
    }
}
/// A separate foreground layer keeps all target edges above later message rows.
pub fn target_outline_layer(bubble: egui::Id) -> egui::LayerId {
    egui::LayerId::new(egui::Order::Foreground, bubble.with("hover-target-outline"))
}
pub fn paint_hover(painter: &egui::Painter, rect: Rect, fill: Color32, accent: Color32) {
    painter.rect(
        rect.shrink(3.0),
        8.0,
        fill,
        Stroke::new(2.0, accent),
        egui::StrokeKind::Inside,
    );
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
        let middle_row_center = CELL + GAP + CELL / 2.0;
        let y = (pointer.y - middle_row_center).clamp(bounds.top(), bounds.bottom() - height);
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

    /// Invisible tolerance is not an input-capturing widget. The side facing
    /// the target bubble is wider to make the arrival less exacting.
    pub fn padded_bounds(self, bubble: Rect) -> Rect {
        let grid = self.bounds();
        let mut padded = grid.expand(HIT_PADDING);
        if grid.center().x >= bubble.center().x {
            padded.min.x = grid.left() - ARRIVAL_PADDING;
        } else {
            padded.max.x = grid.right() + ARRIVAL_PADDING;
        }
        padded
    }
    /// A short diagonal journey from the trigger toward the grid is safe.
    /// It never protects motion away from the grid or a stationary outside
    /// pointer, and owns no click targets of its own.
    pub fn safe_triangle(
        self,
        origin: egui::Pos2,
        previous: egui::Pos2,
        pointer: egui::Pos2,
    ) -> bool {
        let grid = self.bounds().expand(HIT_PADDING);
        if grid.distance_sq_to_pos(pointer) >= grid.distance_sq_to_pos(previous) {
            return false;
        }
        let x = if grid.center().x >= origin.x {
            grid.left()
        } else {
            grid.right()
        };
        let a = origin;
        let b = pos2(x, grid.top());
        let c = pos2(x, grid.bottom());
        let cross = |p: egui::Pos2, q: egui::Pos2, r: egui::Pos2| {
            (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
        };
        if cross(a, b, c).abs() < 0.01 {
            return false;
        }
        let values = [
            cross(a, b, pointer),
            cross(b, c, pointer),
            cross(c, a, pointer),
        ];
        values.iter().all(|v| *v >= 0.0) || values.iter().all(|v| *v <= 0.0)
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
    fn intent_opens_once_after_delay_and_fast_passes_cancel() {
        let mut intent = OpenIntent::default();
        assert!(!intent.update(true, true, 1.0));
        assert!(!intent.update(false, true, 1.14));
        assert!(intent.update(false, true, 1.16));
        assert!(
            !intent.update(false, true, 9.0),
            "no stationary reopen after dismissal"
        );
        assert!(!intent.update(true, true, 10.0));
        assert!(!intent.update(false, false, 10.1));
        assert!(
            !intent.update(false, true, 11.0),
            "layout-only re-entry is not intent"
        );
    }
    #[test]
    fn padded_arrival_and_safe_diagonal_are_symmetric_and_cancel_exit() {
        let bubble = Rect::from_min_max(pos2(50., 200.), pos2(300., 300.));
        let bounds = Rect::from_min_size(pos2(0., 0.), vec2(800., 600.));
        let g = Geometry::beside_pointer(bubble, bounds, 7, bubble.center()).unwrap();
        assert_eq!(
            g.bounds().left() - g.padded_bounds(bubble).left(),
            ARRIVAL_PADDING
        );
        assert_eq!(
            g.padded_bounds(bubble).right() - g.bounds().right(),
            HIT_PADDING
        );
        let origin = pos2(100., 250.);
        assert!(g.safe_triangle(origin, pos2(150., 250.), pos2(220., 240.)));
        assert!(!g.safe_triangle(origin, pos2(220., 240.), pos2(150., 250.)));
        assert!(!g.safe_triangle(origin, pos2(150., 250.), pos2(220., 30.)));
        let left_bubble = Rect::from_min_max(pos2(600., 200.), pos2(780., 300.));
        let left = Geometry::beside_pointer(left_bubble, bounds, 7, left_bubble.center()).unwrap();
        assert_eq!(
            left.padded_bounds(left_bubble).right() - left.bounds().right(),
            ARRIVAL_PADDING
        );
        assert!(left.safe_triangle(pos2(700., 250.), pos2(680., 250.), pos2(620., 250.)));
        let mut exit = ExitGrace::default();
        assert!(!exit.update(true, false, false, 1.0));
        assert!(!exit.update(true, true, false, 1.2));
        assert!(
            !exit.update(false, false, false, 5.0),
            "re-entry cancels timer"
        );
        assert!(
            exit.update(false, true, true, 5.0),
            "explicit dismiss always wins"
        );
    }
    #[test]
    fn modal_grid_keys_are_unique_exact_and_feedback_respects_reduced_motion() {
        let mut keys = std::collections::HashSet::new();
        for (cell, (key, label)) in GRID_KEYS.iter().enumerate() {
            assert!(keys.insert(*key));
            assert_eq!(*label, (cell + 1).to_string());
            assert_eq!(grid_key(*key, egui::Modifiers::NONE), Some(cell));
            assert_eq!(grid_key(*key, egui::Modifiers::COMMAND), None);
        }
        assert!(click_feedback(0., false) > click_feedback(0.1, false));
        assert_eq!(click_feedback(0., true), 0.);
        assert_eq!(click_feedback(0.16, false), 0.);
    }
    #[test]
    fn entry_y_is_captured_once_and_layout_does_not_reopen() {
        let bubble = Rect::from_min_max(pos2(20., 20.), pos2(220., 220.));
        let mut entry = EntryAnchor::default();
        entry.update(bubble, Some(pos2(100., 100.)));
        assert!(entry.fresh());
        entry.update(bubble, Some(pos2(110., 150.)));
        assert!(!entry.fresh());
        assert_eq!(entry.anchor().unwrap().y, 100.);
        entry.update(bubble.translate(vec2(0., 400.)), Some(pos2(110., 150.)));
        entry.update(bubble, Some(pos2(110., 150.)));
        assert!(!entry.fresh(), "layout must not count as pointer entry");
    }
    #[test]
    fn stationary_layout_shift_and_missing_target_keep_menu_until_real_exit() {
        let mut exit = ExitGrace::default();
        assert!(!exit.update(false, false, false, 1.));
        assert!(!exit.update(false, false, false, 20.));
        assert!(!exit.update(true, false, false, 21.));
        assert!(!exit.update(false, false, false, 21.1));
        assert!(!exit.update(false, false, false, 21.24));
        assert!(exit.update(false, false, false, 21.26));
        assert!(ExitGrace::default().update(false, true, true, 1.));
    }
    #[test]
    fn hover_has_visible_stroke_and_target_outline_is_foreground() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_hover(
                ui.painter(),
                Rect::from_min_size(pos2(10., 10.), vec2(44., 44.)),
                Color32::GRAY,
                Color32::WHITE,
            );
        });
        assert!(out.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.width == 2.0 && rect.stroke.color == Color32::WHITE)));
        out.textures_delta.clear();
        assert_eq!(
            target_outline_layer(egui::Id::new("target")).order,
            egui::Order::Foreground
        );
    }
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
        assert_eq!(g.cell(3).center().y, pointer.y);
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
        assert_eq!(g.cell(3).center().y, 100.);
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
