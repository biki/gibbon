//! Commit graph: lane assignment over a topo-ordered log, and the painter
//! that draws one row of it.
//!
//! Each row keeps the lanes as they are at the top of the row and at the
//! bottom. Lanes never shift sideways, so a line that passes a row is
//! straight, and rows join without gaps.

use std::collections::HashMap;

use gpui_kit::*;

use crate::git::Commit;

pub const LANE_W: f32 = 14.;
/// The width of the lines.
const LINE_W: f32 = 1.6;
/// Lanes wider than this are clipped (the widest histories have hundreds).
pub const MAX_LANES: usize = 14;

#[derive(Clone, Copy, Debug)]
pub enum Edge {
    /// From the top of lane `from` into the node.
    In { from: usize, color: usize },
    /// From the node to the bottom of lane `to`.
    Out { to: usize, color: usize },
    /// Straight through the row on lane `lane`.
    Pass { lane: usize, color: usize },
}

#[derive(Clone, Debug)]
pub struct GraphRow {
    pub lane: usize,
    pub color: usize,
    pub merge: bool,
    pub edges: Vec<Edge>,
}

pub struct Graph {
    pub rows: Vec<GraphRow>,
    /// The widest row, in lanes.
    pub width: usize,
}

pub fn layout(commits: &[Commit]) -> Graph {
    // Each lane waits for one commit (by index), with the lane's color.
    let index: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.sha.as_str(), i))
        .collect();
    let mut lanes: Vec<Option<(usize, usize)>> = Vec::new();
    let mut next_color = 0usize;
    let mut new_color = || {
        let c = next_color;
        next_color += 1;
        c
    };
    let mut rows = Vec::with_capacity(commits.len());
    let mut width = 1;

    for (i, commit) in commits.iter().enumerate() {
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(l, s)| (s.map(|(c, _)| c) == Some(i)).then_some(l))
            .collect();
        let (lane, color) = match waiting.first() {
            Some(&l) => (l, lanes[l].map(|(_, c)| c).unwrap_or(0)),
            None => (alloc(&mut lanes), new_color()),
        };
        let mut edges = Vec::new();
        for (l, slot) in lanes.iter().enumerate() {
            if let Some((_, c)) = slot {
                if waiting.contains(&l) {
                    edges.push(Edge::In { from: l, color: *c });
                } else {
                    edges.push(Edge::Pass { lane: l, color: *c });
                }
            }
        }
        for &l in &waiting {
            lanes[l] = None;
        }
        // Parents outside the loaded log (the history is cut off) get no line.
        let parents: Vec<usize> = commit
            .parents
            .iter()
            .filter_map(|p| index.get(p.as_str()).copied())
            .collect();
        for (n, &p) in parents.iter().enumerate() {
            let existing = lanes.iter().position(|s| s.map(|(c, _)| c) == Some(p));
            match existing {
                // Join the lane that already waits for this parent. A first
                // parent only joins to its left, so the main line never
                // drifts right; otherwise both lanes meet at the parent.
                Some(l) if n > 0 || l < lane => {
                    let c = lanes[l].map(|(_, c)| c).unwrap_or(0);
                    edges.push(Edge::Out { to: l, color: c });
                }
                _ if n == 0 => {
                    lanes[lane] = Some((p, color));
                    edges.push(Edge::Out { to: lane, color });
                }
                _ => {
                    let l = alloc(&mut lanes);
                    let c = new_color();
                    lanes[l] = Some((p, c));
                    edges.push(Edge::Out { to: l, color: c });
                }
            }
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        width = width.max(lanes.len()).max(lane + 1);
        rows.push(GraphRow {
            lane,
            color,
            merge: commit.parents.len() > 1,
            edges,
        });
    }
    Graph { rows, width }
}

/// The first free lane, or a new one.
fn alloc(lanes: &mut Vec<Option<(usize, usize)>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(l) => l,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

/// Lane colors, picked to read on dark and light backgrounds.
pub const COLORS: [u32; 8] = [
    0xF2AE3D, 0x3FC9A8, 0x7C8CFF, 0xE66CA0, 0x5BB8F5, 0xB48CF2, 0xA5D46A, 0xF07167,
];

pub fn lane_color(i: usize) -> Hsla {
    rgb(COLORS[i % COLORS.len()]).into()
}

/// Paint one row. `bg` rings the node so lines stop short of it. Lane color
/// 0 is `COLORS[first]`: the theme starts with the color nearest its accent.
pub fn paint_row(
    row: &GraphRow,
    bounds: Bounds<Pixels>,
    bg: Hsla,
    first: usize,
    window: &mut Window,
) {
    let h = f32::from(bounds.size.height);
    let x0 = f32::from(bounds.origin.x);
    let y0 = f32::from(bounds.origin.y);
    let x = |lane: usize| x0 + 8. + lane as f32 * LANE_W;
    let (top, mid, bottom) = (y0, y0 + h / 2., y0 + h);
    let visible = |lane: usize| lane < MAX_LANES;

    for edge in &row.edges {
        let (from, to, color) = match *edge {
            Edge::Pass { lane, color } => ((x(lane), top), (x(lane), bottom), color),
            Edge::In { from, color } => ((x(from), top), (x(row.lane), mid), color),
            Edge::Out { to, color } => ((x(row.lane), mid), (x(to), bottom), color),
        };
        let lanes = match *edge {
            Edge::Pass { lane, .. } => (lane, lane),
            Edge::In { from, .. } => (from, row.lane),
            Edge::Out { to, .. } => (row.lane, to),
        };
        if !visible(lanes.0) || !visible(lanes.1) {
            continue;
        }
        if (from.0 - to.0).abs() < 0.5 {
            // Most lines are straight: a quad costs much less than a path,
            // which is built again on each frame.
            let bounds = Bounds::new(
                point(px(from.0 - LINE_W / 2.), px(from.1)),
                size(px(LINE_W), px(to.1 - from.1)),
            );
            window.paint_quad(fill(bounds, lane_color(first + color)));
            continue;
        }
        // An S-curve that leaves and enters vertically.
        let mut b = PathBuilder::stroke(px(LINE_W));
        b.move_to(point(px(from.0), px(from.1)));
        let my = (from.1 + to.1) / 2.;
        b.cubic_bezier_to(
            point(px(to.0), px(to.1)),
            point(px(from.0), px(my)),
            point(px(to.0), px(my)),
        );
        if let Ok(path) = b.build() {
            window.paint_path(path, lane_color(first + color));
        }
    }

    if !visible(row.lane) {
        return;
    }
    let r = if row.merge { 3.5 } else { 4.5 };
    let (cx, cy) = (x(row.lane), mid);
    let ring = 2.;
    window.paint_quad(
        fill(
            Bounds::new(
                point(px(cx - r - ring), px(cy - r - ring)),
                size(px(2. * (r + ring)), px(2. * (r + ring))),
            ),
            bg,
        )
        .corner_radii(px(r + ring)),
    );
    let node = Bounds::new(point(px(cx - r), px(cy - r)), size(px(2. * r), px(2. * r)));
    let color = lane_color(first + row.color);
    if row.merge {
        window.paint_quad(quad(node, px(r), bg, px(LINE_W), color, BorderStyle::Solid));
    } else {
        window.paint_quad(fill(node, color).corner_radii(px(r)));
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the GPUI glob would shadow the built-in #[test].
    use super::{Edge, layout};
    use crate::git::Commit;

    fn c(sha: &str, parents: &[&str]) -> Commit {
        Commit {
            sha: sha.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            time: 0,
            subject: String::new(),
            refs: vec![],
        }
    }

    #[test]
    fn linear_history_uses_one_lane() {
        let g = layout(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(g.width, 1);
        assert!(g.rows.iter().all(|r| r.lane == 0));
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // m merges f into b; both come from a.
        let g = layout(&[
            c("m", &["b", "f"]),
            c("f", &["a"]),
            c("b", &["a"]),
            c("a", &[]),
        ]);
        assert_eq!(g.width, 2);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1);
        assert_eq!(g.rows[2].lane, 0);
        // b keeps lane 0; both lanes meet at a, on lane 0.
        assert!(matches!(
            g.rows[2].edges.last(),
            Some(Edge::Out { to: 0, .. })
        ));
        assert_eq!(g.rows[3].lane, 0);
        let ins = g.rows[3]
            .edges
            .iter()
            .filter(|e| matches!(e, Edge::In { .. }))
            .count();
        assert_eq!(ins, 2);
    }

    #[test]
    fn side_branch_joins_to_the_left() {
        // Two tips on the same base: the second tip joins lane 0 early.
        let g = layout(&[c("x", &["a"]), c("y", &["a"]), c("a", &[])]);
        assert_eq!(g.rows[1].lane, 1);
        assert!(matches!(
            g.rows[1].edges.last(),
            Some(Edge::Out { to: 0, .. })
        ));
        assert_eq!(g.width, 2);
    }
}
