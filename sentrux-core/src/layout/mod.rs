//! Layout engine — transforms file trees into positioned rectangles and routed edges.
//!
//! Supports two layout modes: squarified treemap (space-filling) and blueprint
//! (grid-based). Both produce `RenderData` containing flat rects, edge paths,
//! and anchor points ready for the renderer to draw.

pub mod aggregation;
pub mod blueprint;
pub mod blueprint_dag;
pub mod routing;
pub mod spatial_index;
pub mod squarify;
pub mod treemap_layout;
pub mod types;
pub mod viewport;
pub mod weight;

#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests2;

use crate::core::settings::Settings;
use crate::core::snapshot::Snapshot;
use crate::core::types::FileNode;
use std::collections::HashSet;
use types::{FocusMode, LayoutMode, RenderData, ScaleMode, SizeMode};

/// Bundles the common configuration parameters for layout computation.
/// Reduces argument counts in `compute_layout_from_snapshot` (13 -> 6) and
/// `dispatch_layout` (13 -> 6).
pub struct LayoutConfig<'a> {
    /// Which metric determines file block area
    pub size_mode: SizeMode,
    /// Scaling transform for size compression
    pub scale_mode: ScaleMode,
    /// Spatial arrangement algorithm (treemap or blueprint)
    pub layout_mode: LayoutMode,
    /// Live heat values for heat-based sizing
    pub heat_map: Option<&'a std::collections::HashMap<String, f64>>,
    /// User-tunable layout and rendering parameters
    pub settings: &'a Settings,
    /// Focus filter controlling which files appear
    pub focus_mode: &'a FocusMode,
    /// Entry point file paths for focus filtering
    pub entry_point_files: &'a HashSet<String>,
    /// User-hidden paths to exclude from layout
    pub hidden_paths: &'a HashSet<String>,
    /// Pre-computed impact set for ImpactRadius focus mode
    pub impact_files: Option<&'a HashSet<String>>,
}

/// Find a subtree node by path. Used for drill-down navigation.
fn find_subtree<'a>(node: &'a FileNode, target_path: &str) -> Option<&'a FileNode> {
    find_subtree_inner(node, target_path, 0)
}

fn find_subtree_inner<'a>(
    node: &'a FileNode,
    target_path: &str,
    depth: u32,
) -> Option<&'a FileNode> {
    if depth >= weight::MAX_DEPTH {
        return None;
    }
    if node.path == target_path && node.is_dir {
        return Some(node);
    }
    if let Some(children) = &node.children {
        for child in children {
            if child.path == target_path
                || (target_path.starts_with(&child.path)
                    && target_path.as_bytes().get(child.path.len()) == Some(&b'/'))
            {
                if let Some(found) = find_subtree_inner(child, target_path, depth + 1) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Main entry point: compute full layout + routing from cached snapshot.
/// Returns pre-computed RenderData ready for TS to draw.
///
/// Edge routing for imports/calls/inherits is parallelized via rayon::join.
pub fn compute_layout_from_snapshot(
    snapshot: &Snapshot,
    viewport_w: f64,
    viewport_h: f64,
    drill_path: Option<&str>,
    cfg: &LayoutConfig<'_>,
) -> RenderData {
    let layout_root = resolve_drill_target(&snapshot.root, drill_path);

    let (rects, anchors, content_w, content_h, route_margin) =
        dispatch_layout(layout_root, snapshot, viewport_w, viewport_h, cfg);

    let (edge_paths, edge_adjacency) =
        aggregation::compute_all_edge_paths(snapshot, &anchors, cfg.settings);

    RenderData {
        rects,
        anchors,
        edge_paths,
        content_width: content_w,
        content_height: content_h,
        route_margin,
        edge_adjacency,
    }
}

/// Resolve drill-down path to the subtree node, or fall back to root.
fn resolve_drill_target<'a>(root: &'a FileNode, drill_path: Option<&str>) -> &'a FileNode {
    match drill_path {
        Some(dp) if !dp.is_empty() => find_subtree(root, dp).unwrap_or(root),
        _ => root,
    }
}

/// Dispatch to blueprint or treemap layout engine.
fn dispatch_layout(
    layout_root: &FileNode,
    snapshot: &Snapshot,
    viewport_w: f64,
    viewport_h: f64,
    cfg: &LayoutConfig<'_>,
) -> (
    Vec<types::LayoutRectSlim>,
    std::collections::HashMap<String, types::Anchor>,
    f64,
    f64,
    f64,
) {
    if cfg.layout_mode.is_blueprint() {
        let (rects, anchors, cw, ch) = blueprint::layout_blueprint(layout_root, snapshot, cfg);
        (rects, anchors, cw, ch, cfg.settings.blueprint_route_margin)
    } else {
        let (rects, anchors) =
            treemap_layout::layout_treemap(layout_root, viewport_w, viewport_h, cfg);
        (rects, anchors, viewport_w, viewport_h, 10.0)
    }
}
