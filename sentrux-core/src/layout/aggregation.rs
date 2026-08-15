//! Edge aggregation and routing — transforms raw graph edges into drawable paths.
//!
//! Aggregates duplicate (from, to) edges into weighted connections, computes
//! alpha/line-width from edge count, routes polylines through the layout, and
//! builds the `EdgeAdjacency` index for O(1) spotlight lookups.

use super::routing::{assign_lanes, compute_edge_path};
use super::types::{Anchor, EdgeAdjacency, EdgePath};
use crate::core::settings::Settings;
use crate::core::snapshot::Snapshot;
use std::collections::{HashMap, HashSet};

/// Aggregated edge: from → to with count. Uses references to snapshot strings
/// to avoid cloning every edge endpoint.
struct AggEdge<'a> {
    from: &'a str,
    to: &'a str,
    count: usize,
}

/// Aggregate raw edges by (from, to) pair.
/// Accepts anything that yields (&str, &str) pairs,
/// so callers don't need to clone strings into intermediate Vecs.
fn aggregate_edges<'a>(
    pairs: impl Iterator<Item = (&'a str, &'a str)>,
    anchors: &HashMap<String, Anchor>,
) -> Vec<AggEdge<'a>> {
    let mut agg: HashMap<(&str, &str), usize> = HashMap::new();
    for (from, to) in pairs {
        if from == to {
            continue;
        }
        if !anchors.contains_key(from) || !anchors.contains_key(to) {
            continue;
        }
        *agg.entry((from, to)).or_insert(0) += 1;
    }

    agg.into_iter()
        .map(|((from, to), count)| AggEdge { from, to, count })
        .collect()
}

/// Style parameters for a batch of edge routing — bundles the 7 style-related
/// parameters of route_edges into a struct (11->4 params).
struct EdgeStyleParams<'a> {
    color: (u8, u8, u8),
    edge_type: &'a str,
    alpha_base: f64,
    alpha_step: f64,
    alpha_max: f64,
    lw_base: f64,
    lw_step: f64,
    lw_max: f64,
}

/// Route a batch of aggregated edges into EdgePath objects.
/// Each edge gets lane_offset = 0 here — real lane separation is done in
/// post-processing for edges that actually share the same (from, to) pair.
fn route_edges(
    agg: &[AggEdge<'_>],
    anchors: &HashMap<String, Anchor>,
    style: &EdgeStyleParams<'_>,
    settings: &Settings,
) -> Vec<EdgePath> {
    let mut paths = Vec::with_capacity(agg.len());

    for edge in agg.iter() {
        let from = match anchors.get(edge.from) {
            Some(a) => a,
            None => continue,
        };
        let to = match anchors.get(edge.to) {
            Some(a) => a,
            None => continue,
        };
        let alpha = (style.alpha_base + edge.count as f64 * style.alpha_step).min(style.alpha_max);
        // Quantize to 1.0 or 2.0 — terminal pixel style, no fractional widths
        let raw_w = (style.lw_base + edge.count as f64 * style.lw_step).min(style.lw_max);
        let line_w = if raw_w >= 1.5 { 2.0 } else { 1.0 };

        if let Some((pts, from_side)) = compute_edge_path(from, to, 0.0, settings) {
            paths.push(EdgePath {
                pts,
                r: style.color.0,
                g: style.color.1,
                b: style.color.2,
                alpha,
                line_w,
                edge_type: style.edge_type.to_string(),
                from_file: edge.from.to_string(),
                to_file: edge.to.to_string(),
                from_side,
            });
        }
    }
    paths
}

/// Compute all edge paths for the layout — imports, calls, inherits.
/// The three edge categories are computed in parallel via rayon::join.
///
/// Also builds an EdgeAdjacency index for O(1) spotlight lookups, replacing
/// the O(E)-per-frame scan in rects.rs.
///
/// Aggregation borrows directly from snapshot edge structs to avoid cloning
/// every from_file/to_file string into intermediate Vecs (previously ~4MB
/// of needless allocation for 50K edges).
pub fn compute_all_edge_paths(
    snapshot: &Snapshot,
    anchors: &HashMap<String, Anchor>,
    settings: &Settings,
) -> (Vec<EdgePath>, EdgeAdjacency) {
    // Aggregate directly from snapshot edge structs — zero intermediate clones.
    let import_agg = aggregate_edges(
        snapshot
            .import_graph
            .iter()
            .map(|e| (e.from_file.as_str(), e.to_file.as_str())),
        anchors,
    );
    let call_agg = aggregate_edges(
        snapshot
            .call_graph
            .iter()
            .map(|e| (e.from_file.as_str(), e.to_file.as_str())),
        anchors,
    );
    let inherit_agg = aggregate_edges(
        snapshot
            .inherit_graph
            .iter()
            .map(|e| (e.child_file.as_str(), e.parent_file.as_str())),
        anchors,
    );

    // Route all three edge types in parallel, then build adjacency + lane offsets.
    let mut paths = route_all_parallel(&import_agg, &call_agg, &inherit_agg, anchors, settings);
    let adjacency = build_adjacency(&import_agg, &call_agg, &inherit_agg);
    apply_lane_offsets(&mut paths, anchors, settings);

    (paths, adjacency)
}

/// Route import, call, and inherit edges in parallel via rayon, returning
/// all EdgePaths merged into one Vec.
fn route_all_parallel(
    import_agg: &[AggEdge<'_>],
    call_agg: &[AggEdge<'_>],
    inherit_agg: &[AggEdge<'_>],
    anchors: &HashMap<String, Anchor>,
    settings: &Settings,
) -> Vec<EdgePath> {
    let ab = settings.edge_alpha_base;
    let am = settings.edge_alpha_max;
    let lb = settings.edge_line_w_base;
    let lm = settings.edge_line_w_max;

    let import_style = EdgeStyleParams {
        color: settings.import_color,
        edge_type: "import",
        alpha_base: ab,
        alpha_step: 0.10,
        alpha_max: am,
        lw_base: lb,
        lw_step: 0.3,
        lw_max: lm,
    };
    let call_style = EdgeStyleParams {
        color: settings.call_color,
        edge_type: "call",
        alpha_base: ab,
        alpha_step: 0.12,
        alpha_max: am,
        lw_base: lb,
        lw_step: 0.3,
        lw_max: lm,
    };
    let inherit_style = EdgeStyleParams {
        color: settings.inherit_color,
        edge_type: "inherit",
        alpha_base: ab,
        alpha_step: 0.10,
        alpha_max: am,
        lw_base: lb,
        lw_step: 0.3,
        lw_max: lm,
    };

    let (import_paths, (call_paths, inherit_paths)) = rayon::join(
        || route_edges(import_agg, anchors, &import_style, settings),
        || {
            rayon::join(
                || route_edges(call_agg, anchors, &call_style, settings),
                || route_edges(inherit_agg, anchors, &inherit_style, settings),
            )
        },
    );

    let total = import_paths.len() + call_paths.len() + inherit_paths.len();
    let mut paths = Vec::with_capacity(total);
    paths.extend(import_paths);
    paths.extend(call_paths);
    paths.extend(inherit_paths);
    paths
}

/// Build bidirectional adjacency index for O(1) spotlight lookups in renderer.
/// Note: String clones in `build_adj` are necessary because `EdgeAdjacency` owns
/// its data and outlives the borrowed `AggEdge` slices. The adjacency is cached
/// across frames, so the clone cost is amortized.
fn build_adjacency(
    import_agg: &[AggEdge<'_>],
    call_agg: &[AggEdge<'_>],
    inherit_agg: &[AggEdge<'_>],
) -> EdgeAdjacency {
    fn build_adj(agg: &[AggEdge<'_>], map: &mut HashMap<String, HashSet<String>>) {
        for e in agg {
            map.entry(e.from.to_string())
                .or_default()
                .insert(e.to.to_string());
            map.entry(e.to.to_string())
                .or_default()
                .insert(e.from.to_string());
        }
    }
    let mut adjacency = EdgeAdjacency::default();
    build_adj(import_agg, &mut adjacency.import);
    build_adj(call_agg, &mut adjacency.call);
    build_adj(inherit_agg, &mut adjacency.inherit);
    adjacency
}

/// Group edge indices by canonical (min,max) file pair.
fn group_edges_by_pair(paths: &[EdgePath]) -> Vec<Vec<usize>> {
    let mut groups: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (i, ep) in paths.iter().enumerate() {
        let key = if ep.from_file <= ep.to_file {
            (ep.from_file.as_str(), ep.to_file.as_str())
        } else {
            (ep.to_file.as_str(), ep.from_file.as_str())
        };
        groups.entry(key).or_default().push(i);
    }
    groups.into_values().collect()
}

/// Re-route a single edge path with the given lane offset.
fn reroute_edge_with_lane(
    paths: &mut [EdgePath],
    pi: usize,
    lane_offset: f64,
    anchors: &HashMap<String, Anchor>,
    settings: &Settings,
) {
    let from_a = match anchors.get(&paths[pi].from_file) {
        Some(a) => a,
        None => return,
    };
    let to_a = match anchors.get(&paths[pi].to_file) {
        Some(a) => a,
        None => return,
    };
    if let Some((pts, from_side)) = compute_edge_path(from_a, to_a, lane_offset, settings) {
        paths[pi].pts = pts;
        paths[pi].from_side = from_side;
    }
}

/// Post-process: edges sharing the same (from, to) pair across types get
/// per-group lane offsets to prevent visual overlap.
///
/// Uses canonical (min,max) string pair as key — NOT a hash — to avoid
/// silent hash collisions between unrelated file pairs.
fn apply_lane_offsets(
    paths: &mut [EdgePath],
    anchors: &HashMap<String, Anchor>,
    settings: &Settings,
) {
    let pair_groups = group_edges_by_pair(paths);
    for indices in &pair_groups {
        if indices.len() <= 1 {
            continue;
        }
        let lanes = assign_lanes(indices.len(), settings.lane_width);
        for (li, &pi) in indices.iter().enumerate() {
            reroute_edge_with_lane(paths, pi, lanes[li], anchors, settings);
        }
    }
}

// Badge rendering is handled entirely in renderer/badges.rs using screen-space
// coordinates (correct zoom behavior). The previous compute_entry_badges()
// computed world-space positions that were never read by the renderer — dead code
// removed to avoid confusion about which code path actually draws badges.
