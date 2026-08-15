//! Design Structure Matrix (DSM) — NxN adjacency matrix of file dependencies.
//!
//! Based on Baldwin & Clark 2000: "Design Rules, Vol. 1: The Power of Modularity"
//!
//! The DSM reveals:
//! - Clusters of tightly coupled files (modules that should be grouped)
//! - Off-diagonal marks = cross-module dependencies (potential architecture violations)
//! - Above-diagonal = upward dependencies (architectural inversions)
//!
//! Data only — rendering is handled separately (egui panel or MCP JSON).

use super::arch;
use crate::core::types::ImportEdge;
use std::collections::{HashMap, HashSet};

// ── Trait: DsmComputer ──

/// Interface for computing Design Structure Matrices from dependency edges.
///
/// Abstracts the DSM construction so that:
/// - Tests can provide synthetic DSMs without building from real edges
/// - Alternative matrix representations (sparse, compressed) can be swapped in
/// - Pre-computed DSMs can be loaded from cache
pub trait DsmComputer {
    /// Build a DSM from import edges.
    fn build(&self, edges: &[ImportEdge]) -> DesignStructureMatrix;

    /// Compute summary statistics from a DSM.
    fn stats(&self, dsm: &DesignStructureMatrix) -> DsmStats;

    /// Render the DSM as a text table.
    fn render(&self, dsm: &DesignStructureMatrix, max_label_width: usize) -> String;
}

/// Default implementation using adjacency-matrix construction with level ordering.
pub struct DefaultDsmComputer;

impl DsmComputer for DefaultDsmComputer {
    fn build(&self, edges: &[ImportEdge]) -> DesignStructureMatrix {
        build_dsm(edges)
    }

    fn stats(&self, dsm: &DesignStructureMatrix) -> DsmStats {
        compute_stats(dsm)
    }

    fn render(&self, dsm: &DesignStructureMatrix, max_label_width: usize) -> String {
        render_text(dsm, max_label_width)
    }
}

// ── Public types ──

/// A Design Structure Matrix representation.
#[derive(Debug, Clone)]
pub struct DesignStructureMatrix {
    /// File names in display order (sorted by level, then alphabetical).
    pub files: Vec<String>,

    /// file_name → index in `files` vec.
    #[allow(dead_code)] // Lookup index for DSM panel interactions
    pub index: HashMap<String, usize>,

    /// NxN matrix: matrix[row][col] = true means files[row] imports files[col].
    /// Row = importer (from), Col = imported (to).
    pub matrix: Vec<Vec<bool>>,

    /// Per-file level (from levelization).
    pub levels: HashMap<String, u32>,

    /// Number of non-zero cells (total dependencies).
    pub edge_count: usize,

    /// Number of above-diagonal marks (upward dependencies = inversions).
    pub above_diagonal: usize,

    /// Number of below-diagonal marks (correct direction).
    pub below_diagonal: usize,

    /// Number of same-level edges (lateral — neither correct nor inversion).
    /// Previously untracked, making edge_count != above + below + same_level,
    /// which violated conservation and confused DSM panel users.
    pub same_level: usize,

    /// Adjacency list built during matrix construction — adj[i] = list of j where matrix[i][j].
    /// Avoids O(N²) matrix scan when computing propagation cost.
    pub adj: Vec<Vec<usize>>,

    /// Cluster boundaries: indices where level changes (for visual grouping).
    pub level_breaks: Vec<usize>,

    /// Matrix dimension (may be < total_files if truncated).
    pub size: usize,

    /// Total unique files before truncation. When size < total_files,
    /// stats are computed on a truncated subset and the UI should indicate this.
    pub total_files: usize,

    /// Level range of dropped files (min_dropped, max_dropped). None if no truncation.
    /// Previously no information about which levels were missing, so the UI couldn't
    /// tell the user what part of the architecture was invisible.
    pub dropped_level_range: Option<(u32, u32)>,
}

/// Summary statistics for the DSM.
#[derive(Debug, Clone)]
pub struct DsmStats {
    /// Matrix dimension (number of files included)
    pub size: usize,
    /// Total non-zero cells in the matrix
    pub edge_count: usize,
    /// Edge density: edges / (size^2 - size), 0.0-1.0
    pub density: f64,
    /// Dependencies from higher-level to lower-level (architectural inversions)
    pub above_diagonal: usize,
    /// Dependencies in the correct direction (lower imports higher)
    pub below_diagonal: usize,
    /// Lateral edges between files at the same level
    pub same_level: usize,
    /// Propagation cost: avg transitive reach / N (Baldwin & Clark 2000), 0.0-1.0
    pub propagation_cost: f64,
    /// Detected clusters of tightly coupled files
    pub clusters: Vec<DsmCluster>,
}

/// A cluster of tightly coupled files detected from the DSM.
#[derive(Debug, Clone)]
pub struct DsmCluster {
    /// File paths in this cluster
    pub files: Vec<String>,
    /// Number of directed dependency edges within the cluster (A->B and B->A count as 2)
    pub internal_edges: usize,
    /// Minimum DAG level of files in the cluster
    pub level: u32,
}

// ── Public API ──

/// Build DSM from import edges. Files are sorted by level (low→high), then alphabetically.
pub fn build_dsm(edges: &[ImportEdge]) -> DesignStructureMatrix {
    if edges.is_empty() {
        return empty_dsm();
    }

    // Get all unique files
    let mut file_set: HashSet<&str> = HashSet::new();
    for edge in edges {
        file_set.insert(&edge.from_file);
        file_set.insert(&edge.to_file);
    }
    let total_files = file_set.len();

    // Filter mod-declaration edges before computing levels, consistent with arch report.
    // Previously used raw edges, causing DSM level assignments to disagree with arch report.
    let dep_edges: Vec<ImportEdge> = edges
        .iter()
        .filter(|e| !crate::metrics::types::is_mod_declaration_edge(e))
        .cloned()
        .collect();
    let (levels, _max_level) = arch::compute_levels(&dep_edges);
    let mut files: Vec<String> = file_set.iter().map(|s| s.to_string()).collect();
    sort_files_by_level(&mut files, &levels);

    // Cap matrix size to prevent O(N²) OOM on large codebases.
    let dropped_level_range = truncate_to_extremes(&mut files, &levels);

    let size = files.len();
    let index: HashMap<String, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.clone(), i))
        .collect();

    // Build matrix + adjacency list + classify edges.
    // Use dep_edges (mod-declarations filtered out) so matrix content is consistent
    // with the level assignments used for above/below classification. [H12 fix]
    let (matrix, adj, edge_count, above_diagonal, below_diagonal, same_level) =
        populate_matrix(&dep_edges, &index, &files, &levels, size);

    let level_breaks = find_level_breaks(&files, &levels);

    DesignStructureMatrix {
        files,
        index,
        matrix,
        levels,
        edge_count,
        above_diagonal,
        below_diagonal,
        same_level,
        adj,
        level_breaks,
        size,
        total_files,
        dropped_level_range,
    }
}

fn empty_dsm() -> DesignStructureMatrix {
    DesignStructureMatrix {
        files: Vec::new(),
        index: HashMap::new(),
        matrix: Vec::new(),
        levels: HashMap::new(),
        edge_count: 0,
        above_diagonal: 0,
        below_diagonal: 0,
        same_level: 0,
        adj: Vec::new(),
        level_breaks: Vec::new(),
        size: 0,
        total_files: 0,
        dropped_level_range: None,
    }
}

/// Sort files by level (ascending), then alphabetically within each level.
fn sort_files_by_level(files: &mut [String], levels: &HashMap<String, u32>) {
    files.sort_by(|a, b| {
        let la = levels.get(a).copied().unwrap_or(0);
        let lb = levels.get(b).copied().unwrap_or(0);
        la.cmp(&lb).then_with(|| a.cmp(b))
    });
}

/// Cap matrix size to MAX_DSM_SIZE, keeping both extremes (lowest + highest level files)
/// so the architectural spine is not dropped. Returns the level range of dropped middle files.
fn truncate_to_extremes(
    files: &mut Vec<String>,
    levels: &HashMap<String, u32>,
) -> Option<(u32, u32)> {
    const MAX_DSM_SIZE: usize = 2000;
    if files.len() <= MAX_DSM_SIZE {
        return None;
    }
    let half = MAX_DSM_SIZE / 2;
    let tail_start = files.len() - half;
    let drop_min = levels.get(&files[half]).copied().unwrap_or(0);
    let drop_max = levels.get(&files[tail_start - 1]).copied().unwrap_or(0);
    let mut kept = Vec::with_capacity(MAX_DSM_SIZE);
    kept.extend_from_slice(&files[..half]);
    kept.extend_from_slice(&files[tail_start..]);
    *files = kept;
    Some((drop_min, drop_max))
}

/// Build the NxN matrix and adjacency list, classifying edges by level direction.
/// Returns (matrix, adj, edge_count, above_diagonal, below_diagonal, same_level).
/// Result of populating the DSM matrix: (matrix, adj, edge_count, above_diagonal, below_diagonal, same_level).
type DsmMatrixResult = (Vec<Vec<bool>>, Vec<Vec<usize>>, usize, usize, usize, usize);

fn populate_matrix(
    edges: &[ImportEdge],
    index: &HashMap<String, usize>,
    files: &[String],
    levels: &HashMap<String, u32>,
    size: usize,
) -> DsmMatrixResult {
    let mut matrix = vec![vec![false; size]; size];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); size];
    let mut edge_count = 0;
    let mut above_diagonal = 0;
    let mut below_diagonal = 0;
    let mut same_level = 0;

    for edge in edges {
        if let (Some(&row), Some(&col)) = (index.get(&edge.from_file), index.get(&edge.to_file)) {
            if row != col && !matrix[row][col] {
                matrix[row][col] = true;
                adj[row].push(col);
                edge_count += 1;
                let row_level = levels.get(&files[row]).copied().unwrap_or(0);
                let col_level = levels.get(&files[col]).copied().unwrap_or(0);
                if row_level > col_level {
                    below_diagonal += 1;
                } else if row_level < col_level {
                    above_diagonal += 1;
                } else {
                    same_level += 1;
                }
            }
        }
    }

    (
        matrix,
        adj,
        edge_count,
        above_diagonal,
        below_diagonal,
        same_level,
    )
}

/// Find indices where the level number changes (for visual grouping).
fn find_level_breaks(files: &[String], levels: &HashMap<String, u32>) -> Vec<usize> {
    let mut breaks = Vec::new();
    for i in 1..files.len() {
        let prev = levels.get(&files[i - 1]).copied().unwrap_or(0);
        let curr = levels.get(&files[i]).copied().unwrap_or(0);
        if curr != prev {
            breaks.push(i);
        }
    }
    breaks
}

/// Compute summary statistics from a DSM.
pub fn compute_stats(dsm: &DesignStructureMatrix) -> DsmStats {
    let possible = if dsm.size > 1 {
        dsm.size * (dsm.size - 1)
    } else {
        1
    };
    let density = dsm.edge_count as f64 / possible as f64;

    // Compute propagation cost: average transitive reachability.
    // For each file, how many files can it reach transitively?
    let propagation_cost = compute_propagation_cost(&dsm.adj, dsm.size);

    // Detect clusters: groups of files at the same level with mutual dependencies.
    let clusters = detect_clusters(dsm);

    DsmStats {
        size: dsm.size,
        edge_count: dsm.edge_count,
        density,
        above_diagonal: dsm.above_diagonal,
        below_diagonal: dsm.below_diagonal,
        same_level: dsm.same_level,
        propagation_cost,
        clusters,
    }
}

/// Render the DSM as a text table (for CLI / MCP output).
/// Uses '×' for dependencies, '·' for empty, '■' for diagonal.
pub fn render_text(dsm: &DesignStructureMatrix, max_label_width: usize) -> String {
    if dsm.size == 0 {
        return "(empty matrix)".to_string();
    }

    let display_size = dsm.size.min(40);
    let truncated = dsm.size > display_size;

    let mut lines = Vec::new();
    render_header(&mut lines, max_label_width, display_size);
    render_rows(&mut lines, dsm, max_label_width, display_size);

    if truncated {
        lines.push(format!("... ({} more files)", dsm.size - display_size));
    }

    lines.join("\n")
}

/// Render the column-index header row.
fn render_header(lines: &mut Vec<String>, max_label_width: usize, display_size: usize) {
    let pad = " ".repeat(max_label_width + 2);
    let header: String = (0..display_size)
        .map(|i| format!("{:>2}", i))
        .collect::<Vec<_>>()
        .join("");
    lines.push(format!("{pad}{header}"));
}

/// Render each matrix row with its label and cell symbols.
fn render_rows(
    lines: &mut Vec<String>,
    dsm: &DesignStructureMatrix,
    max_label_width: usize,
    display_size: usize,
) {
    for row in 0..display_size {
        let label = &dsm.files[row];
        // Use ceil_char_boundary to avoid panic on multi-byte UTF-8 paths.
        let short = if label.len() > max_label_width {
            let start = label.len() - max_label_width;
            &label[label.ceil_char_boundary(start)..]
        } else {
            label
        };

        let cells: String = (0..display_size)
            .map(|col| {
                if row == col {
                    " \u{25a0}"
                } else if dsm.matrix[row][col] {
                    " \u{00d7}"
                } else {
                    " \u{00b7}"
                }
            })
            .collect();

        lines.push(format!(
            "{:>width$} \u{2502}{cells}",
            short,
            width = max_label_width
        ));
    }
}

// ── Internal helpers ──

/// Propagation cost (Baldwin & Clark 2000): fraction of the matrix reachable
/// on average. PC = (1/N) × Σ_i (|reachable(i)| / (N-1)) — normalized to [0, 1].
///
/// 0.0 = fully decoupled (no transitive reach), 1.0 = fully connected.
/// The theoretical maximum reachability per node is (N-1), not N (a node cannot
/// reach itself), so we normalize by (N-1) to get a true [0, 1] range.
///
/// Uses per-node BFS instead of Floyd-Warshall to avoid O(n³) freeze on large graphs.
/// BFS per-node is O(n × (n + E)) worst case but exits early on sparse graphs.
///
/// Samples uniformly across levels instead of taking the first N nodes (which
/// are all leaves with near-zero reachability, biasing the result downward).
fn compute_propagation_cost(adj: &[Vec<usize>], size: usize) -> f64 {
    if size <= 1 {
        return 0.0;
    }

    let sample_indices = build_propagation_sample(adj, size);
    let compute_size = sample_indices.len();
    let total_reach = bfs_total_reach(adj, size, &sample_indices);

    // Normalize: avg_reach / (N-1) = (total_reach / compute_size) / (N-1)
    // This gives [0, 1] where 1.0 = every sampled node reaches every other node.
    // The max reachability per node is (N-1), not N, since a node cannot reach itself.
    total_reach as f64 / (compute_size as f64 * (size - 1) as f64)
}

/// Build a uniform sample of node indices for propagation cost estimation.
/// Guarantees the highest-degree node is included to avoid underestimation.
fn build_propagation_sample(adj: &[Vec<usize>], size: usize) -> Vec<usize> {
    const MAX_PROPAGATION_NODES: usize = 500;

    if size <= MAX_PROPAGATION_NODES {
        return (0..size).collect();
    }

    let step = size as f64 / MAX_PROPAGATION_NODES as f64;
    let mut indices: Vec<usize> = (0..MAX_PROPAGATION_NODES)
        .map(|i| ((i as f64) * step) as usize)
        .collect();

    // Include the node with the most outgoing edges (highest connectivity).
    if let Some((max_idx, _)) = adj.iter().enumerate().max_by_key(|(_, a)| a.len()) {
        let idx_set: std::collections::HashSet<usize> = indices.iter().copied().collect();
        if !idx_set.contains(&max_idx) {
            if let Some(last) = indices.last_mut() {
                *last = max_idx;
            }
        }
    }

    // Dedup: floating-point stepping can produce duplicate indices (e.g., when
    // step < 1.0 truncation maps two i values to the same usize).
    indices.sort_unstable();
    indices.dedup();

    indices
}

/// BFS from each sampled node, summing total transitive reach.
fn bfs_total_reach(adj: &[Vec<usize>], size: usize, sample_indices: &[usize]) -> usize {
    let mut total_reach: usize = 0;
    let mut visited = vec![false; size];
    let mut queue = std::collections::VecDeque::new();

    for &start in sample_indices {
        visited.fill(false);
        visited[start] = true;
        queue.clear();
        queue.push_back(start);
        let mut reach_count: usize = 0;
        while let Some(node) = queue.pop_front() {
            for &j in &adj[node] {
                if !visited[j] {
                    visited[j] = true;
                    reach_count += 1;
                    queue.push_back(j);
                }
            }
        }
        total_reach += reach_count;
    }
    total_reach
}

/// Detect clusters of tightly coupled files using connected-component analysis
/// on the mutual-dependency subgraph. Previously grouped files by same level
/// and counted internal edges — this missed cross-level coupling (e.g., a
/// controller at level 5 tightly coupled to its model at level 3).
///
/// New approach: build an undirected graph of mutual dependencies (A↔B if both
/// A→B and B→A exist in the DSM), then find connected components. Each component
/// with ≥2 files and ≥1 internal edge is a cluster.
fn detect_clusters(dsm: &DesignStructureMatrix) -> Vec<DsmCluster> {
    if dsm.size < 2 {
        return Vec::new();
    }

    const MAX_CLUSTER_SIZE: usize = 2000;
    let n = dsm.size.min(MAX_CLUSTER_SIZE);

    let (adj, mutual_edges) = build_mutual_adjacency(dsm, n);
    if mutual_edges == 0 {
        return Vec::new();
    }

    let component = bfs_components(&adj, n);
    let comp_files = group_by_component(&component);

    let mut clusters: Vec<DsmCluster> = comp_files
        .into_values()
        .filter(|indices| indices.len() >= 2)
        .map(|indices| build_cluster(dsm, &indices))
        .collect();

    clusters.sort_by_key(|b| std::cmp::Reverse(b.internal_edges));
    clusters
}

/// Build undirected adjacency from mutual dependencies (both A→B and B→A exist).
/// Uses pre-built dsm.adj for O(E) instead of O(N²) matrix scan.
fn build_mutual_adjacency(dsm: &DesignStructureMatrix, n: usize) -> (Vec<Vec<usize>>, usize) {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut mutual_edges = 0usize;
    for i in 0..n {
        for &j in &dsm.adj[i] {
            if j > i && j < n && dsm.matrix[j][i] {
                adj[i].push(j);
                adj[j].push(i);
                mutual_edges += 1;
            }
        }
    }
    (adj, mutual_edges)
}

/// BFS from a single start node, labeling all reachable nodes with comp_id.
fn bfs_label_component(
    start: usize,
    comp_id: i32,
    adj: &[Vec<usize>],
    component: &mut [i32],
    queue: &mut std::collections::VecDeque<usize>,
) {
    component[start] = comp_id;
    queue.push_back(start);
    while let Some(node) = queue.pop_front() {
        for &neighbor in &adj[node] {
            if component[neighbor] < 0 {
                component[neighbor] = comp_id;
                queue.push_back(neighbor);
            }
        }
    }
}

/// Find connected components via BFS on the mutual-dependency graph.
fn bfs_components(adj: &[Vec<usize>], n: usize) -> Vec<i32> {
    let mut component: Vec<i32> = vec![-1; n];
    let mut comp_id = 0i32;
    let mut queue = std::collections::VecDeque::new();
    for start in 0..n {
        if component[start] >= 0 || adj[start].is_empty() {
            continue;
        }
        bfs_label_component(start, comp_id, adj, &mut component, &mut queue);
        comp_id += 1;
    }
    component
}

/// Group node indices by their component ID.
fn group_by_component(component: &[i32]) -> HashMap<i32, Vec<usize>> {
    let mut comp_files: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, &cid) in component.iter().enumerate() {
        if cid >= 0 {
            comp_files.entry(cid).or_default().push(i);
        }
    }
    comp_files
}

/// Build a DsmCluster from a set of node indices, counting internal edges via adjacency list.
fn build_cluster(dsm: &DesignStructureMatrix, indices: &[usize]) -> DsmCluster {
    let member_set: HashSet<usize> = indices.iter().copied().collect();
    let mut internal = 0usize;
    for &i in indices {
        for &j in &dsm.adj[i] {
            if member_set.contains(&j) {
                internal += 1;
            }
        }
    }
    let level = indices
        .iter()
        .filter_map(|&i| dsm.levels.get(&dsm.files[i]).copied())
        .min()
        .unwrap_or(0);
    DsmCluster {
        files: indices.iter().map(|&i| dsm.files[i].clone()).collect(),
        internal_edges: internal,
        level,
    }
}

#[cfg(test)]
mod tests;
