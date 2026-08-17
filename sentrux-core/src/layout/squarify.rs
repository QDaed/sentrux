//! Squarified treemap algorithm (Bruls, Huizing, van Wijk 2000).
//!
//! Generic implementation: caller provides weighted items and a viewport rect,
//! algorithm calls back with positioned rectangles. Optimizes aspect ratios
//! via the squarify heuristic. Used by both treemap and blueprint layouts.

/// A weighted item for squarification
pub struct WeightedItem {
    /// Relative weight determining area proportion
    pub weight: f64,
    /// Original index into the caller's item list
    pub index: usize,
}

/// Groups the viewport rectangle and spacing parameters for squarification.
pub struct SquarifyConfig {
    /// Top-left X of the target rectangle
    pub x: f64,
    /// Top-left Y of the target rectangle
    pub y: f64,
    /// Width of the target rectangle
    pub w: f64,
    /// Height of the target rectangle
    pub h: f64,
    /// Gap between items (applied as half-gutter inset on each side).
    /// Pass 0.0 for no gutter (treemap mode).
    pub gutter: f64,
    /// Minimum dimension for placed rectangles (from Settings).
    pub min_rect: f64,
}

/// Compute the worst aspect ratio for a row candidate using min/max weight tracking.
fn worst_aspect_ratio(
    new_min: f64,
    new_max: f64,
    new_weight: f64,
    row_area: f64,
    row_side: f64,
) -> f64 {
    let area_max = (new_max / new_weight) * row_area;
    let side_max = area_max / row_side;
    let asp_max = (side_max / row_side).max(row_side / side_max);

    let area_min = (new_min / new_weight) * row_area;
    let side_min = area_min / row_side;
    let asp_min = (side_min / row_side).max(row_side / side_min);

    let w = asp_max.max(asp_min);
    if w.is_finite() {
        w
    } else {
        f64::INFINITY
    }
}

/// Result of building one squarified row.
struct RowResult {
    count: usize,
    weight: f64,
}

/// Build one row: find optimal number of items starting at `start_idx`.
fn build_row(
    items: &[WeightedItem],
    start_idx: usize,
    rem_weight: f64,
    cw: f64,
    ch: f64,
    side: f64,
) -> RowResult {
    let mut row_count = 0usize;
    let mut row_weight = 0.0_f64;
    let mut best_aspect = f64::INFINITY;
    let mut row_min_w = f64::INFINITY;
    let mut row_max_w = 0.0_f64;

    for (offset, item) in items[start_idx..].iter().enumerate() {
        let new_weight = row_weight + item.weight;
        if new_weight < 1e-12 {
            break;
        }
        let row_area = (new_weight / rem_weight) * cw * ch;
        let row_side = row_area / side;
        if row_side < 1e-6 {
            break;
        }

        let iw = item.weight;
        let new_min = row_min_w.min(iw);
        let new_max = row_max_w.max(iw);
        let wa = worst_aspect_ratio(new_min, new_max, new_weight, row_area, row_side);

        if wa <= best_aspect || row_count == 0 {
            row_count = offset + 1;
            row_weight = new_weight;
            best_aspect = wa;
            row_min_w = new_min;
            row_max_w = new_max;
        } else {
            break;
        }
    }
    RowResult {
        count: row_count,
        weight: row_weight,
    }
}

/// Current remaining viewport during squarification — tracks the cursor position
/// and remaining dimensions as rows are consumed.
struct Viewport {
    cx: f64,
    cy: f64,
    cw: f64,
    ch: f64,
}

/// Emit a single item rect, applying gutter inset if applicable.
fn emit_item<F>(
    callback: &mut F,
    index: usize,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    sc: &SquarifyConfig,
) where
    F: FnMut(usize, f64, f64, f64, f64),
{
    if sc.gutter > 0.0 {
        let g = sc.gutter / 2.0;
        let gw = rw - sc.gutter;
        let gh = rh - sc.gutter;
        if gw >= sc.min_rect && gh >= sc.min_rect {
            callback(index, rx + g, ry + g, gw, gh);
        } else if rw >= 1.0 && rh >= 1.0 {
            callback(index, rx, ry, rw, rh);
        }
    } else {
        callback(index, rx, ry, rw, rh);
    }
}

/// Emit remaining items at minimum size in a grid pattern.
fn emit_tail<F>(
    items: &[WeightedItem],
    mut start_idx: usize,
    vp: &Viewport,
    sc: &SquarifyConfig,
    callback: &mut F,
) where
    F: FnMut(usize, f64, f64, f64, f64),
{
    if vp.cw < sc.min_rect || vp.ch < sc.min_rect {
        return;
    }
    let mut cx = vp.cx;
    let mut cy = vp.cy;
    let rem_x = cx;
    let x_end = cx + vp.cw;
    let y_end = cy + vp.ch;
    let step = sc.min_rect + sc.gutter;
    while start_idx < items.len() {
        if cy + sc.min_rect > y_end {
            break;
        }
        let rw = sc.min_rect.min(x_end - cx);
        let rh = sc.min_rect.min(y_end - cy);
        if rw >= 1.0 && rh >= 1.0 {
            callback(items[start_idx].index, cx, cy, rw, rh);
        }
        cx += step;
        if cx + sc.min_rect > x_end {
            cx = rem_x;
            cy += step;
        }
        start_idx += 1;
    }
}

/// Emit all items in a single row, computing per-item position and size.
#[allow(clippy::too_many_arguments)]
fn emit_row<F>(
    items: &[WeightedItem],
    start_idx: usize,
    row: &RowResult,
    vp: &Viewport,
    is_wide: bool,
    row_extent: f64,
    sc: &SquarifyConfig,
    callback: &mut F,
) where
    F: FnMut(usize, f64, f64, f64, f64),
{
    let mut offset = 0.0;
    for item in &items[start_idx..(start_idx + row.count)] {
        let frac = if row.weight > 0.0 {
            item.weight / row.weight
        } else {
            1.0 / row.count as f64
        };
        let item_extent = if is_wide { vp.ch } else { vp.cw } * frac;
        let (rx, ry, rw, rh) = if is_wide {
            (vp.cx, vp.cy + offset, row_extent, item_extent)
        } else {
            (vp.cx + offset, vp.cy, item_extent, row_extent)
        };
        emit_item(callback, item.index, rx, ry, rw, rh, sc);
        offset += item_extent;
    }
}

/// Squarify a list of weighted items into a rectangle.
/// Calls `callback(item_index, x, y, w, h)` for each placed item.
/// Items must be sorted by weight descending for best aspect ratios.
/// Filter and sort items, returning None if nothing valid remains.
fn prepare_items(items: &[WeightedItem]) -> Option<Vec<WeightedItem>> {
    let mut clean: Vec<WeightedItem> = items
        .iter()
        .filter(|i| i.weight.is_finite() && i.weight >= 0.0)
        .map(|i| WeightedItem {
            weight: i.weight,
            index: i.index,
        })
        .collect();
    clean.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let total: f64 = clean.iter().map(|i| i.weight).sum();
    if clean.is_empty() || total <= 0.0 {
        None
    } else {
        Some(clean)
    }
}

/// Process one row of the squarify layout, returning consumed extent.
fn process_row<F>(
    items: &[WeightedItem],
    start_idx: usize,
    rem_weight: f64,
    vp: &Viewport,
    sc: &SquarifyConfig,
    callback: &mut F,
) -> Option<(usize, f64, bool)>
where
    F: FnMut(usize, f64, f64, f64, f64),
{
    let is_wide = vp.cw >= vp.ch;
    let side = if is_wide { vp.ch } else { vp.cw };
    let row = build_row(items, start_idx, rem_weight, vp.cw, vp.ch, side);
    if row.count == 0 {
        return None;
    }
    let row_frac = row.weight / rem_weight;
    let row_extent = if is_wide {
        vp.cw * row_frac
    } else {
        vp.ch * row_frac
    };
    emit_row(
        items, start_idx, &row, vp, is_wide, row_extent, sc, callback,
    );
    Some((row.count, row_extent, is_wide))
}

pub fn squarify<F>(items: &[WeightedItem], sc: &SquarifyConfig, mut callback: F)
where
    F: FnMut(usize, f64, f64, f64, f64),
{
    let (x, y, w, h) = (sc.x, sc.y, sc.w, sc.h);
    if w < sc.min_rect || h < sc.min_rect {
        return;
    }
    let items = match prepare_items(items) {
        Some(v) => v,
        None => return,
    };

    let mut rem_weight: f64 = items.iter().map(|i| i.weight).sum();
    let mut start_idx = 0usize;
    let mut total_x_consumed = 0.0_f64;
    let mut total_y_consumed = 0.0_f64;

    while start_idx < items.len() {
        let vp = Viewport {
            cx: x + total_x_consumed,
            cy: y + total_y_consumed,
            cw: w - total_x_consumed,
            ch: h - total_y_consumed,
        };
        if vp.cw < sc.min_rect || vp.ch < sc.min_rect || rem_weight <= 0.0 {
            break;
        }

        match process_row(&items, start_idx, rem_weight, &vp, sc, &mut callback) {
            Some((count, extent, is_wide)) => {
                if is_wide {
                    total_x_consumed += extent;
                } else {
                    total_y_consumed += extent;
                }
                rem_weight = items[start_idx + count..].iter().map(|i| i.weight).sum();
                start_idx += count;
            }
            None => break,
        }
    }

    let tail_vp = Viewport {
        cx: x + total_x_consumed,
        cy: y + total_y_consumed,
        cw: w - total_x_consumed,
        ch: h - total_y_consumed,
    };
    emit_tail(&items, start_idx, &tail_vp, sc, &mut callback);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(x: f64, y: f64, w: f64, h: f64, gutter: f64, min_rect: f64) -> SquarifyConfig {
        SquarifyConfig {
            x,
            y,
            w,
            h,
            gutter,
            min_rect,
        }
    }

    #[test]
    fn test_empty_items() {
        let mut calls = 0;
        squarify(
            &[],
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |_, _, _, _, _| calls += 1,
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn test_single_item() {
        let items = vec![WeightedItem {
            weight: 10.0,
            index: 0,
        }];
        let mut rects = Vec::new();
        squarify(
            &items,
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |idx, x, y, w, h| {
                rects.push((idx, x, y, w, h));
            },
        );
        assert_eq!(rects.len(), 1);
        let (idx, x, y, w, h) = rects[0];
        assert_eq!(idx, 0);
        // Single item should fill entire area
        assert!((x - 0.0).abs() < 1e-6);
        assert!((y - 0.0).abs() < 1e-6);
        assert!((w - 100.0).abs() < 1e-6);
        assert!((h - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_conservation_area() {
        // Child areas should sum ≤ parent area (with gutter they'll be less)
        let items: Vec<WeightedItem> = (0..5)
            .map(|i| WeightedItem {
                weight: (i + 1) as f64 * 10.0,
                index: i,
            })
            .collect();
        let parent_area = 200.0 * 150.0;
        let mut total_child_area = 0.0;
        squarify(
            &items,
            &sc(0.0, 0.0, 200.0, 150.0, 0.0, 3.0),
            |_, _, _, w, h| {
                total_child_area = w.mul_add(h, total_child_area);
            },
        );
        assert!(
            total_child_area <= parent_area + 1e-6,
            "child area {} > parent area {}",
            total_child_area,
            parent_area
        );
    }

    #[test]
    fn test_conservation_area_with_gutter() {
        let items: Vec<WeightedItem> = (0..5)
            .map(|i| WeightedItem {
                weight: (i + 1) as f64 * 10.0,
                index: i,
            })
            .collect();
        let parent_area = 200.0 * 150.0;
        let mut total_child_area = 0.0;
        squarify(
            &items,
            &sc(0.0, 0.0, 200.0, 150.0, 4.0, 3.0),
            |_, _, _, w, h| {
                total_child_area = w.mul_add(h, total_child_area);
            },
        );
        assert!(total_child_area <= parent_area);
        // With gutter, area should be strictly less
        assert!(total_child_area < parent_area);
    }

    #[test]
    fn test_idempotency() {
        let items: Vec<WeightedItem> = (0..4)
            .map(|i| WeightedItem {
                weight: (i + 1) as f64 * 5.0,
                index: i,
            })
            .collect();

        let cfg = sc(10.0, 20.0, 300.0, 200.0, 2.0, 3.0);
        let mut rects1 = Vec::new();
        squarify(&items, &cfg, |idx, x, y, w, h| {
            rects1.push((idx, x, y, w, h));
        });

        let mut rects2 = Vec::new();
        squarify(&items, &cfg, |idx, x, y, w, h| {
            rects2.push((idx, x, y, w, h));
        });

        assert_eq!(rects1.len(), rects2.len());
        for i in 0..rects1.len() {
            assert!((rects1[i].1 - rects2[i].1).abs() < 1e-10);
            assert!((rects1[i].2 - rects2[i].2).abs() < 1e-10);
            assert!((rects1[i].3 - rects2[i].3).abs() < 1e-10);
            assert!((rects1[i].4 - rects2[i].4).abs() < 1e-10);
        }
    }

    #[test]
    fn test_zero_weight_items_no_nan() {
        // Bug fix: zero-weight items must not produce NaN coordinates.
        // Previously 0.0/0.0 in aspect ratio calculation caused NaN propagation.
        let items = vec![
            WeightedItem {
                weight: 0.0,
                index: 0,
            },
            WeightedItem {
                weight: 10.0,
                index: 1,
            },
            WeightedItem {
                weight: 5.0,
                index: 2,
            },
        ];
        let mut rects = Vec::new();
        squarify(
            &items,
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |idx, x, y, w, h| {
                assert!(x.is_finite(), "x is NaN/Inf for item {}", idx);
                assert!(y.is_finite(), "y is NaN/Inf for item {}", idx);
                assert!(w.is_finite(), "w is NaN/Inf for item {}", idx);
                assert!(h.is_finite(), "h is NaN/Inf for item {}", idx);
                rects.push((idx, x, y, w, h));
            },
        );
        // Non-zero items should be placed; zero-weight item may or may not be placed
        // but must never produce NaN
        assert!(
            rects.len() >= 2,
            "expected at least 2 rects, got {}",
            rects.len()
        );
    }

    #[test]
    fn test_all_zero_weight_items() {
        let items = vec![
            WeightedItem {
                weight: 0.0,
                index: 0,
            },
            WeightedItem {
                weight: 0.0,
                index: 1,
            },
        ];
        let mut calls = 0;
        squarify(
            &items,
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |_, _, _, _, _| calls += 1,
        );
        // All-zero weights: rem_weight=0 → early return, no rects
        assert_eq!(calls, 0);
    }

    #[test]
    fn test_float_accumulation_no_item_loss() {
        // Many small items — verify none are silently dropped by float drift.
        let n = 100;
        let items: Vec<WeightedItem> = (0..n)
            .map(|i| WeightedItem {
                weight: 1.0,
                index: i,
            })
            .collect();
        let mut placed = 0;
        squarify(
            &items,
            &sc(0.0, 0.0, 500.0, 500.0, 0.0, 3.0),
            |_, _, _, _, _| placed += 1,
        );
        assert_eq!(
            placed, n,
            "all {} items should be placed, got {}",
            n, placed
        );
    }

    #[test]
    fn test_too_small_viewport() {
        let items = vec![WeightedItem {
            weight: 10.0,
            index: 0,
        }];
        let mut calls = 0;
        squarify(
            &items,
            &sc(0.0, 0.0, 2.0, 2.0, 0.0, 3.0),
            |_, _, _, _, _| calls += 1,
        );
        assert_eq!(calls, 0, "viewport < min_rect should produce zero rects");
    }

    #[test]
    fn test_nan_weight_no_nan_coords() {
        // NaN weights must be treated as zero, never producing NaN coordinates.
        let items = vec![
            WeightedItem {
                weight: f64::NAN,
                index: 0,
            },
            WeightedItem {
                weight: 10.0,
                index: 1,
            },
            WeightedItem {
                weight: 5.0,
                index: 2,
            },
        ];
        let mut rects = Vec::new();
        squarify(
            &items,
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |idx, x, y, w, h| {
                assert!(x.is_finite(), "x is NaN/Inf for item {}", idx);
                assert!(y.is_finite(), "y is NaN/Inf for item {}", idx);
                assert!(w.is_finite(), "w is NaN/Inf for item {}", idx);
                assert!(h.is_finite(), "h is NaN/Inf for item {}", idx);
                rects.push((idx, x, y, w, h));
            },
        );
        assert!(rects.len() >= 2, "finite-weight items must be placed");
    }

    #[test]
    fn test_all_nan_weights() {
        let items = vec![
            WeightedItem {
                weight: f64::NAN,
                index: 0,
            },
            WeightedItem {
                weight: f64::NAN,
                index: 1,
            },
        ];
        let mut calls = 0;
        squarify(
            &items,
            &sc(0.0, 0.0, 100.0, 100.0, 0.0, 3.0),
            |_, _, _, _, _| calls += 1,
        );
        assert_eq!(calls, 0, "all-NaN weights should produce zero rects");
    }
}
