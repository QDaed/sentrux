//! Edge path routing — computes polyline waypoints between file blocks.
//!
//! Routes edges as orthogonal L-paths (one bend) or straight lines depending
//! on alignment. Handles border clipping so edges start/end at rect edges,
//! not centers. Lane assignment separates parallel edges between the same pair.

use super::types::{Anchor, Point};
use crate::core::settings::Settings;

/// Clip a ray from `outside` toward `target` center to the target rect border.
/// Works for diagonal/arbitrary approach directions (straight-line edges).
fn clip_ray_to_border(outside: &Point, target: &Anchor, border_pad: f64) -> Point {
    let dx = target.cx - outside.x;
    let dy = target.cy - outside.y;

    if dx.abs() < 0.01 && dy.abs() < 0.01 {
        return Point {
            x: target.cx,
            y: target.cy,
        };
    }

    let half_w = (target.bw / 2.0).max(1.0);
    let half_h = (target.bh / 2.0).max(1.0);

    // Find where the ray from outside→center crosses the rect border.
    let sx = if dx.abs() > 0.01 {
        half_w / dx.abs()
    } else {
        f64::MAX
    };
    let sy = if dy.abs() > 0.01 {
        half_h / dy.abs()
    } else {
        f64::MAX
    };
    let s = sx.min(sy);

    // Clamp s to [0, 1] so the border point is always on the NEAR side of
    // the target rect relative to `outside`. Without this clamp, when `outside`
    // is inside the rect (s > 1.0), the border point would be on the FAR side,
    // producing edges that visually pass through the entire rect. [ref:93cf32d4]
    //
    let s = s.min(1.0);
    let bx = dx.mul_add(-s, target.cx);
    let by = dy.mul_add(-s, target.cy);

    // Nudge inward slightly so arrowhead doesn't overlap the edge
    let pad_x = if dx.abs() > 0.01 {
        border_pad * dx.signum()
    } else {
        0.0
    };
    let pad_y = if dy.abs() > 0.01 {
        border_pad * dy.signum()
    } else {
        0.0
    };

    Point {
        x: bx + pad_x,
        y: by + pad_y,
    }
}

/// For an orthogonal last-segment approaching the target from directly
/// above/below/left/right, compute the border intersection.
/// This handles the L-path case where pre_target may be INSIDE the rect.
fn clip_ortho_to_border(pre: &Point, target: &Anchor, border_pad: f64) -> Point {
    let dx = target.cx - pre.x;
    let dy = target.cy - pre.y;

    // Vertical approach (same x as center)
    if dx.abs() < 0.01 {
        if dy.abs() < 0.01 {
            return Point {
                x: target.cx,
                y: target.cy,
            };
        }
        // Hit top or bottom border
        let border_y = if dy > 0.0 {
            target.by + border_pad // approaching from above → top border
        } else {
            target.by + target.bh - border_pad // approaching from below → bottom border
        };
        return Point {
            x: target.cx,
            y: border_y,
        };
    }

    // Horizontal approach (same y as center)
    if dy.abs() < 0.01 {
        let border_x = if dx > 0.0 {
            target.bx + border_pad // approaching from left → left border
        } else {
            target.bx + target.bw - border_pad // approaching from right → right border
        };
        return Point {
            x: border_x,
            y: target.cy,
        };
    }

    // Fallback: diagonal (shouldn't happen for L-path last segment, but safe)
    clip_ray_to_border(pre, target, border_pad)
}

/// Clip source center OUTWARD to the source rect border based on exit direction.
/// `exit_dir` is the first meaningful movement point after the source center.
/// Returns (point_on_border, side_char) where side_char is 'l','r','t','b'.
fn clip_source_to_border(
    from: &Anchor,
    exit_dx: f64,
    exit_dy: f64,
    border_pad: f64,
) -> (Point, char) {
    if exit_dx.abs() < 0.01 && exit_dy.abs() < 0.01 {
        return (
            Point {
                x: from.cx,
                y: from.cy,
            },
            'r',
        );
    }
    // Clamp border_pad so the clipped point never crosses the center.
    // On small rects (e.g. 4×4 in blueprint), border_pad=1.5 > bw/2=2.0
    // would place the "border" point on the wrong side of center. [ref:5d9ed05d]
    let pad_x = border_pad.min(from.bw / 2.0 - 0.5).max(0.0);
    let pad_y = border_pad.min(from.bh / 2.0 - 0.5).max(0.0);
    if exit_dx.abs() >= exit_dy.abs() {
        // Primarily horizontal exit → clip to left/right border
        if exit_dx > 0.0 {
            (
                Point {
                    x: from.bx + from.bw - pad_x,
                    y: from.cy,
                },
                'r',
            )
        } else {
            (
                Point {
                    x: from.bx + pad_x,
                    y: from.cy,
                },
                'l',
            )
        }
    } else {
        // Primarily vertical exit → clip to top/bottom border
        if exit_dy > 0.0 {
            (
                Point {
                    x: from.cx,
                    y: from.by + from.bh - pad_y,
                },
                'b',
            )
        } else {
            (
                Point {
                    x: from.cx,
                    y: from.by + pad_y,
                },
                't',
            )
        }
    }
}

/// Compute edge path between two anchors with minimum bends.
/// - Start point: clipped to source block border (not center)
/// - End point: clipped to target block border (not center)
/// - 0 bends (straight line): if centers are roughly aligned on X or Y
/// - 1 bend (L-path): single right-angle turn
///
/// `lane_offset` fans out co-routed edges so they don't overlap.
/// Returns None if distance too short.
/// Returns (points, from_side) where from_side is 'l','r','t','b'.
pub fn compute_edge_path(
    from: &Anchor,
    to: &Anchor,
    lane_offset: f64,
    settings: &Settings,
) -> Option<(Vec<Point>, char)> {
    let border_pad = settings.edge_border_pad;
    let dist = (to.cx - from.cx).hypot(to.cy - from.cy);
    if dist < settings.min_edge_len {
        return None;
    }

    let dx = to.cx - from.cx;
    let dy = to.cy - from.cy;
    let adx = dx.abs();
    let ady = dy.abs();

    // 0 bends: straight line if nearly aligned on one axis AND no lane offset.
    let align_t = settings.edge_align_threshold;
    let lane_t = settings.edge_lane_threshold;
    if (ady < align_t || adx < align_t) && lane_offset.abs() < lane_t {
        let (start, side) = clip_source_to_border(from, dx, dy, border_pad);
        let end = clip_ray_to_border(&start, to, border_pad);
        return Some((vec![start, end], side));
    }

    // 1 bend: L-path — horizontal-dominant or vertical-dominant.
    if adx >= ady {
        route_horizontal_dominant(from, to, lane_offset, lane_t, border_pad)
    } else {
        route_vertical_dominant(from, to, lane_offset, lane_t, border_pad)
    }
}

/// Clamp a bend coordinate outside the source rect to prevent edges passing through it.
fn clamp_bend_outside(raw: f64, rect_min: f64, rect_size: f64, positive: bool) -> f64 {
    if raw > rect_min && raw < rect_min + rect_size {
        if positive {
            rect_min + rect_size
        } else {
            rect_min
        }
    } else {
        raw
    }
}

/// Compute the bend offset for an L-path, clamped outside the source rect.
fn compute_bend_offset(
    lane_offset: f64,
    lane_t: f64,
    center: f64,
    start_pos: f64,
    rect_min: f64,
    rect_size: f64,
) -> f64 {
    if lane_offset.abs() < lane_t {
        center
    } else {
        clamp_bend_outside(
            start_pos + lane_offset,
            rect_min,
            rect_size,
            lane_offset > 0.0,
        )
    }
}

/// Build the final path points given bend, target intersection check, and border padding.
#[allow(clippy::too_many_arguments)]
fn finalize_lpath(
    start: Point,
    bend: Point,
    bend_is_degenerate: bool,
    inside_target: bool,
    direct_border: Point,
    pre_target: Point,
    to: &Anchor,
    border_pad: f64,
) -> Vec<Point> {
    if inside_target {
        build_pts_opt_bend(start, bend, direct_border, bend_is_degenerate)
    } else {
        let end = clip_ortho_to_border(&pre_target, to, border_pad);
        build_pts_with_pre(start, bend, pre_target, end, bend_is_degenerate)
    }
}

/// Route an L-path when horizontal distance dominates (adx >= ady).
fn route_horizontal_dominant(
    from: &Anchor,
    to: &Anchor,
    lane_offset: f64,
    lane_t: f64,
    border_pad: f64,
) -> Option<(Vec<Point>, char)> {
    let dx = to.cx - from.cx;
    let (start, side) = if lane_offset.abs() < lane_t {
        clip_source_to_border(from, dx, 0.0, border_pad)
    } else {
        clip_source_to_border(from, 0.0, lane_offset, border_pad)
    };
    let bend_y = compute_bend_offset(lane_offset, lane_t, from.cy, start.y, from.by, from.bh);
    let bend = Point {
        x: start.x,
        y: bend_y,
    };
    let bend_degen = (bend.x - start.x).abs() < 0.01 && (bend.y - start.y).abs() < 0.01;
    let inside = bend_y >= to.by + border_pad && bend_y <= to.by + to.bh - border_pad;
    let border_x = if from.cx < to.cx {
        to.bx + border_pad
    } else {
        to.bx + to.bw - border_pad
    };
    let pts = finalize_lpath(
        start,
        bend,
        bend_degen,
        inside,
        Point {
            x: border_x,
            y: bend_y,
        },
        Point {
            x: to.cx,
            y: bend_y,
        },
        to,
        border_pad,
    );
    Some((pts, side))
}

/// Route an L-path when vertical distance dominates (ady > adx).
fn route_vertical_dominant(
    from: &Anchor,
    to: &Anchor,
    lane_offset: f64,
    lane_t: f64,
    border_pad: f64,
) -> Option<(Vec<Point>, char)> {
    let dy = to.cy - from.cy;
    let (start, side) = if lane_offset.abs() < lane_t {
        clip_source_to_border(from, 0.0, dy, border_pad)
    } else {
        clip_source_to_border(from, lane_offset, 0.0, border_pad)
    };
    let bend_x = compute_bend_offset(lane_offset, lane_t, from.cx, start.x, from.bx, from.bw);
    let bend = Point {
        x: bend_x,
        y: start.y,
    };
    let bend_degen = (bend.x - start.x).abs() < 0.01 && (bend.y - start.y).abs() < 0.01;
    let inside = bend_x >= to.bx + border_pad && bend_x <= to.bx + to.bw - border_pad;
    let border_y = if from.cy < to.cy {
        to.by + border_pad
    } else {
        to.by + to.bh - border_pad
    };
    let pts = finalize_lpath(
        start,
        bend,
        bend_degen,
        inside,
        Point {
            x: bend_x,
            y: border_y,
        },
        Point {
            x: bend_x,
            y: to.cy,
        },
        to,
        border_pad,
    );
    Some((pts, side))
}

/// Build point list: [start, end] or [start, bend, end] depending on degeneracy.
fn build_pts_opt_bend(start: Point, bend: Point, end: Point, degenerate: bool) -> Vec<Point> {
    if degenerate {
        vec![start, end]
    } else {
        vec![start, bend, end]
    }
}

/// Build point list with optional bend and optional pre-target waypoint.
fn build_pts_with_pre(
    start: Point,
    bend: Point,
    pre: Point,
    end: Point,
    bend_degenerate: bool,
) -> Vec<Point> {
    let mut pts = vec![start];
    if !bend_degenerate {
        pts.push(bend);
    }
    if (pre.x - end.x).abs() >= 2.0 || (pre.y - end.y).abs() >= 2.0 {
        pts.push(pre);
    }
    pts.push(end);
    pts
}

/// Assign each edge a unique lane offset so parallel segments don't overlap.
/// Lanes spread symmetrically: 0, -1, +1, -2, +2, ... × lane_width
pub fn assign_lanes(count: usize, lane_width: f64) -> Vec<f64> {
    let mut lanes = Vec::with_capacity(count);
    for i in 0..count {
        let lane = if i == 0 {
            0.0
        } else {
            let half = i.div_ceil(2) as f64;
            let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
            sign * half
        };
        lanes.push(lane * lane_width);
    }
    lanes
}
