//! Orthogonal visibility-grid routing between fixed bubble border ports.
//! Expanded rectangles include corner radius, so rounding keeps clearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Box2 {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}
impl Box2 {
    fn inflate(self, n: f32) -> Self {
        Self {
            l: self.l - n,
            t: self.t - n,
            r: self.r + n,
            b: self.b + n,
        }
    }
}
const PAD: f32 = 7.0;
fn blocked(a: Point, b: Point, obstacles: &[Box2]) -> bool {
    obstacles.iter().any(|r| {
        if (a.x - b.x).abs() < 0.01 {
            a.x > r.l && a.x < r.r && a.y.min(b.y) < r.b && a.y.max(b.y) > r.t
        } else {
            a.y > r.t && a.y < r.b && a.x.min(b.x) < r.r && a.x.max(b.x) > r.l
        }
    })
}
#[cfg(test)]
pub fn route(source: Box2, target: Box2, others: &[Box2], viewport: Box2) -> Option<Vec<Point>> {
    if [source, target]
        .iter()
        .any(|r| r.l < viewport.l || r.r > viewport.r || r.t < viewport.t || r.b > viewport.b)
    {
        return None;
    }
    let right = (source.l + source.r) <= (target.l + target.r);
    let sign = if right { 1.0 } else { -1.0 };
    let start = Point {
        x: if right { source.r } else { source.l },
        y: (source.t + source.b) / 2.0,
    };
    let end = Point {
        x: if right { target.l } else { target.r },
        y: (target.t + target.b) / 2.0,
    };
    let a = Point {
        x: start.x + sign * (PAD + 1.0),
        y: start.y,
    };
    let b = Point {
        x: end.x - sign * (PAD + 1.0),
        y: end.y,
    };
    route_ports(source, target, others, viewport, start, end, a, b)
}

/// Route to the top of the reply's chain, not a guessed body midpoint.
pub fn route_to_chain(
    source: Box2,
    target: Box2,
    chain: Box2,
    others: &[Box2],
    viewport: Box2,
) -> Option<Vec<Point>> {
    if [source, target, chain]
        .iter()
        .any(|r| r.l < viewport.l || r.r > viewport.r || r.t < viewport.t || r.b > viewport.b)
    {
        return None;
    }
    let end = Point {
        x: (chain.l + chain.r) / 2.0,
        y: chain.t,
    };
    let right = end.x >= (source.l + source.r) / 2.0;
    let start = Point {
        x: if right { source.r } else { source.l },
        y: (source.t + source.b) / 2.0,
    };
    let a = Point {
        x: start.x + if right { PAD + 1.0 } else { -PAD - 1.0 },
        y: start.y,
    };
    let b = Point {
        x: end.x,
        y: end.y - PAD - 1.0,
    };
    if [start, end, a, b]
        .iter()
        .any(|p| p.x < viewport.l || p.x > viewport.r || p.y < viewport.t || p.y > viewport.b)
    {
        return None;
    }
    route_ports(source, target, others, viewport, start, end, a, b)
}

#[allow(clippy::too_many_arguments)]
fn route_ports(
    source: Box2,
    target: Box2,
    others: &[Box2],
    viewport: Box2,
    start: Point,
    end: Point,
    a: Point,
    b: Point,
) -> Option<Vec<Point>> {
    let mut obs: Vec<_> = others.iter().copied().map(|r| r.inflate(PAD)).collect();
    if blocked(start, a, &obs) || blocked(b, end, &obs) {
        return None;
    }
    obs.extend([source.inflate(PAD), target.inflate(PAD)]);
    let mut xs = vec![a.x, b.x, viewport.l + 2.0, viewport.r - 2.0];
    let mut ys = vec![a.y, b.y, viewport.t + 2.0, viewport.b - 2.0];
    for r in &obs {
        xs.extend([r.l - 1.0, r.r + 1.0]);
        ys.extend([r.t - 1.0, r.b + 1.0]);
    }
    xs.retain(|x| *x >= viewport.l && *x <= viewport.r);
    ys.retain(|y| *y >= viewport.t && *y <= viewport.b);
    xs.sort_by(f32::total_cmp);
    ys.sort_by(f32::total_cmp);
    xs.dedup();
    ys.dedup();
    if xs.len() * ys.len() > 10000 {
        return None;
    }
    let locate = |p: Point| {
        Some(ys.iter().position(|y| *y == p.y)? * xs.len() + xs.iter().position(|x| *x == p.x)?)
    };
    let begin = locate(a)?;
    let goal = locate(b)?;
    let n = xs.len() * ys.len();
    let point = |i: usize| Point {
        x: xs[i % xs.len()],
        y: ys[i / xs.len()],
    };
    let mut dist = vec![f32::INFINITY; n * 2];
    let mut prev = vec![None; n * 2];
    let mut heap = std::collections::BinaryHeap::new();
    dist[begin * 2] = 0.0;
    heap.push((std::cmp::Reverse(0u64), begin * 2));
    let mut found = None;
    while let Some((std::cmp::Reverse(cost), state)) = heap.pop() {
        let cost = f64::from_bits(cost) as f32;
        if cost > dist[state] {
            continue;
        }
        let i = state / 2;
        let dir = state % 2;
        if i == goal {
            found = Some(state);
            break;
        }
        let x = i % xs.len();
        let y = i / xs.len();
        let mut next = vec![];
        if x > 0 {
            next.push((i - 1, 0));
        }
        if x + 1 < xs.len() {
            next.push((i + 1, 0));
        }
        if y > 0 {
            next.push((i - xs.len(), 1));
        }
        if y + 1 < ys.len() {
            next.push((i + xs.len(), 1));
        }
        for (j, nd) in next {
            let p = point(i);
            let q = point(j);
            if blocked(p, q, &obs) {
                continue;
            }
            let cost =
                cost + (p.x - q.x).abs() + (p.y - q.y).abs() + if dir != nd { 16.0 } else { 0.0 };
            let ns = j * 2 + nd;
            if cost < dist[ns] {
                dist[ns] = cost;
                prev[ns] = Some(state);
                heap.push((std::cmp::Reverse((cost as f64).to_bits()), ns));
            }
        }
    }
    let mut state = found?;
    let mut path = vec![end, b];
    while state != begin * 2 {
        path.push(point(state / 2));
        state = prev[state]?;
    }
    path.push(a);
    path.push(start);
    path.reverse();
    path.dedup();
    let mut simplified = vec![path[0]];
    for i in 1..path.len() - 1 {
        let a = *simplified.last().unwrap();
        let b = path[i];
        let c = path[i + 1];
        if !((a.x == b.x && b.x == c.x) || (a.y == b.y && b.y == c.y)) {
            simplified.push(b);
        }
    }
    simplified.push(end);
    Some(simplified)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn r(l: f32, t: f32, rr: f32, b: f32) -> Box2 {
        Box2 { l, t, r: rr, b }
    }
    fn view() -> Box2 {
        r(0., 0., 800., 800.)
    }
    #[test]
    fn chain_port_is_exact_and_outer_route_avoids_reply_body() {
        let source = r(350., 20., 600., 90.);
        let target = r(90., 300., 400., 410.);
        let chain = r(60., 264., 90., 294.);
        let p = route_to_chain(source, target, chain, &[], view()).unwrap();
        assert_eq!(*p.last().unwrap(), Point { x: 75., y: 264. });
        assert!(p.windows(2).all(|p| p[0].x == p[1].x || p[0].y == p[1].y));
        assert!(p.windows(2).all(|p| !blocked(p[0], p[1], &[target])));
    }
    #[test]
    fn chain_outside_viewport_is_not_routed_to() {
        assert!(
            route_to_chain(
                r(100., 10., 300., 50.),
                r(100., 100., 300., 200.),
                r(-40., 64., -10., 94.),
                &[],
                view()
            )
            .is_none()
        );
    }
    #[test]
    fn endpoints_touch_inner_borders_and_segments_are_orthogonal() {
        let p = route(
            r(20., 20., 180., 80.),
            r(196., 200., 390., 270.),
            &[],
            view(),
        )
        .unwrap();
        assert_eq!(p[0].x, 180.);
        assert_eq!(p.last().unwrap().x, 196.);
        assert!(p.windows(2).all(|p| p[0].x == p[1].x || p[0].y == p[1].y));
    }
    #[test]
    fn routes_around_intervening_bubble() {
        let obstacle = r(160., 100., 350., 180.);
        let p = route(
            r(20., 20., 180., 80.),
            r(196., 220., 390., 290.),
            &[obstacle],
            view(),
        )
        .unwrap();
        assert!(
            p.windows(2)
                .all(|s| !blocked(s[0], s[1], &[obstacle.inflate(3.)]))
        );
    }
    #[test]
    fn offscreen_or_sealed_route_returns_none() {
        assert!(
            route(
                r(-10., 0., 100., 50.),
                r(200., 200., 300., 250.),
                &[],
                view()
            )
            .is_none()
        );
        assert!(
            route(
                r(20., 20., 180., 80.),
                r(196., 220., 390., 290.),
                &[r(0., 90., 800., 200.)],
                view()
            )
            .is_none()
        );
    }
}
