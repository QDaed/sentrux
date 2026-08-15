//! Squarified treemap layout engine (Bruls, Huizing, van Wijk 2000).
//!
//! Recursively subdivides viewport area proportionally to file weights.
//! Produces flat `LayoutRectSlim` output (no tree structure needed by renderer).
//! Handles directory padding, headers, gutters, and min-rect thresholds.

use super::squarify::{squarify, SquarifyConfig, WeightedItem};
use super::types::{Anchor, LayoutCtx, LayoutRectSlim, RectKind, ViewportRect};
use super::weight::{self, get_w, WeightConfig, MAX_DEPTH};
use super::LayoutConfig;
use crate::core::settings::Settings;
use crate::core::types::FileNode;
use std::collections::HashMap;

/// Compute padding and header for a section.
/// Simple rules — no fractional heuristics:
///   pad = min(settings.pad, 15% of smaller dimension)
///   header = min(settings.header, 20% of height)
/// Both are always > 0 for sections with enough space.
fn chrome_for_depth(w: f64, h: f64, s: &Settings) -> (f64, f64) {
    let min_dim = w.min(h);
    let frac = s.treemap_max_chrome_frac;
    let pad = s.treemap_dir_pad.min(min_dim * frac * 0.6).max(1.0);
    let header = if h > 20.0 && w > 16.0 {
        s.treemap_dir_header.min(h * frac * 0.8).max(6.0)
    } else {
        0.0
    };
    (pad, header)
}

/// Compute treemap layout: fills the given viewport w x h.
/// Returns flat rects and anchor map.
pub fn layout_treemap(
    root: &FileNode,
    viewport_w: f64,
    viewport_h: f64,
    cfg: &LayoutConfig<'_>,
) -> (Vec<LayoutRectSlim>, HashMap<String, Anchor>) {
    let mut rects = Vec::new();
    let mut anchors = HashMap::new();

    // Pre-compute weights into a HashMap keyed by path (focus mode + hidden filters here)
    let mut weights: HashMap<String, f64> = HashMap::new();
    let wc = WeightConfig {
        size_mode: cfg.size_mode,
        scale_mode: cfg.scale_mode,
        heat_map: cfg.heat_map,
        min_child_weight: cfg.settings.min_child_weight,
        focus_mode: cfg.focus_mode,
        entry_point_files: cfg.entry_point_files,
        hidden_paths: cfg.hidden_paths,
        impact_files: cfg.impact_files,
    };
    weight::precompute_weights(root, &wc, &mut weights);

    let mut lctx = LayoutCtx {
        weights: &weights,
        rects: &mut rects,
        anchors: &mut anchors,
        settings: cfg.settings,
    };
    layout_node(
        root,
        &ViewportRect::new(0.0, 0.0, viewport_w, viewport_h),
        0,
        &mut lctx,
    );

    (rects, anchors)
}

fn make_rect(node: &FileNode, x: f64, y: f64, w: f64, h: f64, depth: u32) -> LayoutRectSlim {
    let kind = if node.is_dir {
        RectKind::Section
    } else {
        RectKind::File
    };
    let section_id = if node.is_dir {
        node.path.clone()
    } else {
        match node.path.rfind('/') {
            Some(pos) if pos > 0 => node.path[..pos].to_string(),
            _ => String::new(),
        }
    };
    LayoutRectSlim {
        path: node.path.clone(),
        x,
        y,
        w,
        h,
        depth,
        kind,
        section_id,
        grid_coord: None,
        header_h: 0.0,
    }
}

fn layout_node(node: &FileNode, vp: &ViewportRect, depth: u32, lctx: &mut LayoutCtx<'_>) {
    if depth >= MAX_DEPTH {
        return;
    }
    if vp.w < lctx.settings.treemap_min_rect || vp.h < lctx.settings.treemap_min_rect {
        return;
    }

    // For files, emit rect immediately. For directories, defer until we confirm
    // there are visible children to avoid phantom empty sections. [ref:93cf32d4]
    if !node.is_dir {
        emit_file_leaf(node, vp, depth, lctx);
        return;
    }

    layout_dir_children(node, vp, depth, lctx);
}

/// Emit a file leaf rect and its anchor into the output vectors.
fn emit_file_leaf(node: &FileNode, vp: &ViewportRect, depth: u32, lctx: &mut LayoutCtx<'_>) {
    lctx.rects
        .push(make_rect(node, vp.x, vp.y, vp.w, vp.h, depth));
    let section_id = match node.path.rfind('/') {
        Some(pos) if pos > 0 => node.path[..pos].to_string(),
        _ => String::new(),
    };
    lctx.anchors.insert(
        node.path.clone(),
        Anchor {
            file_path: node.path.clone(),
            cx: vp.x + vp.w / 2.0,
            cy: vp.y + vp.h / 2.0,
            section_id,
            bx: vp.x,
            by: vp.y,
            bw: vp.w,
            bh: vp.h,
        },
    );
}

/// Layout children of a directory node using squarified treemap subdivision.
fn layout_dir_children(node: &FileNode, vp: &ViewportRect, depth: u32, lctx: &mut LayoutCtx<'_>) {
    let children = match &node.children {
        Some(c) if !c.is_empty() => c,
        _ => return,
    };

    let settings = lctx.settings;
    let (pad, header) = chrome_for_depth(vp.w, vp.h, settings);
    let inner = ViewportRect::new(
        vp.x + pad,
        vp.y + header + pad,
        vp.w - pad * 2.0,
        vp.h - header - pad * 2.0,
    );
    if inner.w < settings.treemap_min_rect || inner.h < settings.treemap_min_rect {
        return;
    }

    // Filter and sort children by weight descending
    let mut kids: Vec<&FileNode> = children
        .iter()
        .filter(|c| get_w(c, lctx.weights) > 0.0)
        .collect();
    kids.sort_by(|a, b| get_w(b, lctx.weights).total_cmp(&get_w(a, lctx.weights)));
    if kids.is_empty() {
        return;
    }

    // Emit section rect with header_h stored for renderer
    let mut sec_rect = make_rect(node, vp.x, vp.y, vp.w, vp.h, depth);
    sec_rect.header_h = header;
    lctx.rects.push(sec_rect);

    let items: Vec<WeightedItem> = kids
        .iter()
        .enumerate()
        .map(|(i, k)| WeightedItem {
            weight: get_w(k, lctx.weights),
            index: i,
        })
        .collect();

    let gutter = if depth == 0 {
        settings.treemap_gutter_root
    } else {
        settings.treemap_gutter_inner
    };
    let sc = SquarifyConfig {
        x: inner.x,
        y: inner.y,
        w: inner.w,
        h: inner.h,
        gutter,
        min_rect: settings.squarify_min_rect,
    };

    // Collect placements first, then recurse (avoids borrow conflict with lctx)
    let mut placed: Vec<(usize, f64, f64, f64, f64)> = Vec::new();
    squarify(&items, &sc, |idx, rx, ry, rw, rh| {
        placed.push((idx, rx, ry, rw, rh));
    });
    for (idx, rx, ry, rw, rh) in placed {
        layout_node(
            kids[idx],
            &ViewportRect::new(rx, ry, rw, rh),
            depth + 1,
            lctx,
        );
    }
}
