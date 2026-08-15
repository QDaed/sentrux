//! Grid-based spatial index for O(1) point-in-rect hit testing.
//!
//! Divides the layout canvas into a fixed grid of cells. Each cell stores
//! indices of rects that overlap it. Mouse hover/click queries check only
//! the cell under the cursor, avoiding O(N) rect iteration.

use super::types::{LayoutRectSlim, RectKind};

/// BUG 11 fix: increased cell size from 100 to 200 and reduced MAX_GRID_DIM
/// from 2000 to 500 to prevent excessive allocation. Old worst case: 2000²=4M
/// cells × 24 bytes = 96MB for empty Vecs alone. New worst case: 500²=250K
/// cells × 24 bytes = 6MB. The larger cell size means ~4 rects per cell on
/// average for typical layouts, maintaining O(1) hit testing. [ref:93cf32d4]
const CELL_SIZE: f64 = 200.0;
const MAX_GRID_DIM: usize = 500;

/// Grid-based spatial index for O(1) hit testing on file and section rects.
pub struct SpatialIndex {
    cells: Vec<Vec<usize>>,
    cols: usize,
    rows: usize,
    rects: Vec<HitRect>,
    /// Section (directory) rects for context-menu hit testing
    section_cells: Vec<Vec<usize>>,
    section_rects: Vec<HitRect>,
}

/// Minimal rect data stored in the spatial index for hit testing.
/// Keeps only geometry + path to avoid cloning full LayoutRectSlim.
struct HitRect {
    pub path: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl SpatialIndex {
    /// Build spatial index from layout rects (files + sections).
    pub fn build(rects: &[LayoutRectSlim], content_w: f64, content_h: f64) -> Self {
        let cols = ((content_w / CELL_SIZE).ceil() as usize).clamp(1, MAX_GRID_DIM);
        let rows = ((content_h / CELL_SIZE).ceil() as usize).clamp(1, MAX_GRID_DIM);
        let mut cells = vec![Vec::new(); cols * rows];
        let mut hit_rects = Vec::new();
        let mut section_cells = vec![Vec::new(); cols * rows];
        let mut section_rects = Vec::new();

        for r in rects {
            let is_file = r.kind == RectKind::File;
            let is_section = r.kind == RectKind::Section;
            if !is_file && !is_section {
                continue;
            }

            let hr = HitRect {
                path: r.path.clone(),
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            };

            let cell_range = match compute_cell_range(r, cols, rows) {
                Some(cr) => cr,
                None => continue,
            };

            if is_file {
                register_in_cells(hr, &cell_range, cols, &mut cells, &mut hit_rects);
            } else {
                register_in_cells(
                    hr,
                    &cell_range,
                    cols,
                    &mut section_cells,
                    &mut section_rects,
                );
            }
        }

        Self {
            cells,
            cols,
            rows,
            rects: hit_rects,
            section_cells,
            section_rects,
        }
    }
}

/// Cell range for a rect: (col_start, col_end, row_start, row_end).
struct CellRange {
    c0: usize,
    c1: usize,
    r0: usize,
    r1: usize,
}

/// Compute the grid cell range that a rect overlaps. Returns None if offscreen.
fn compute_cell_range(r: &LayoutRectSlim, cols: usize, rows: usize) -> Option<CellRange> {
    let rx = r.x.max(0.0);
    let ry = r.y.max(0.0);
    let rx_end = (r.x + r.w).max(0.0);
    let ry_end = (r.y + r.h).max(0.0);
    if rx_end <= 0.0 || ry_end <= 0.0 {
        return None;
    }
    Some(CellRange {
        c0: ((rx / CELL_SIZE).floor() as usize).min(cols - 1),
        c1: ((rx_end / CELL_SIZE).floor() as usize).min(cols - 1),
        r0: ((ry / CELL_SIZE).floor() as usize).min(rows - 1),
        r1: ((ry_end / CELL_SIZE).floor() as usize).min(rows - 1),
    })
}

/// Register a HitRect in the given grid cells across all overlapping rows/cols.
fn register_in_cells(
    hr: HitRect,
    cr: &CellRange,
    cols: usize,
    grid_cells: &mut [Vec<usize>],
    rects: &mut Vec<HitRect>,
) {
    let idx = rects.len();
    rects.push(hr);
    for row in cr.r0..=cr.r1 {
        for col in cr.c0..=cr.c1 {
            grid_cells[row * cols + col].push(idx);
        }
    }
}

/// Compute the grid cell index for a world coordinate. Returns None if out of bounds.
fn cell_index_for(wx: f64, wy: f64, cols: usize, rows: usize, grid_len: usize) -> Option<usize> {
    if wx < 0.0 || wy < 0.0 {
        return None;
    }
    let col = ((wx / CELL_SIZE).floor() as usize).min(cols.saturating_sub(1));
    let row = ((wy / CELL_SIZE).floor() as usize).min(rows.saturating_sub(1));
    let idx = row * cols + col;
    if idx >= grid_len {
        None
    } else {
        Some(idx)
    }
}

/// Find the smallest-area rect containing (wx, wy) among candidates.
fn find_smallest_containing<'a>(
    candidates: &[usize],
    rects: &'a [HitRect],
    wx: f64,
    wy: f64,
) -> Option<&'a str> {
    let mut best: Option<(usize, f64)> = None;
    for &rect_idx in candidates {
        let r = &rects[rect_idx];
        if wx >= r.x && wx < r.x + r.w && wy >= r.y && wy < r.y + r.h {
            let area = r.w * r.h;
            if best.is_none() || area < best.unwrap().1 {
                best = Some((rect_idx, area));
            }
        }
    }
    best.map(|(idx, _)| rects[idx].path.as_str())
}

impl SpatialIndex {
    /// Find the section (directory) path at a world coordinate.
    /// Returns the deepest (most nested) section containing the point.
    pub fn hit_test_section(&self, wx: f64, wy: f64) -> Option<&str> {
        let cell_idx = cell_index_for(wx, wy, self.cols, self.rows, self.section_cells.len())?;
        find_smallest_containing(&self.section_cells[cell_idx], &self.section_rects, wx, wy)
    }

    /// Find the file path at a world coordinate. Returns the smallest (most nested) rect.
    pub fn hit_test(&self, wx: f64, wy: f64) -> Option<&str> {
        let cell_idx = cell_index_for(wx, wy, self.cols, self.rows, self.cells.len())?;
        find_smallest_containing(&self.cells[cell_idx], &self.rects, wx, wy)
    }
}
