//! Root cause metrics — 6 fundamental structural properties of a codebase.
//!
//! These are NOT proxies. They directly measure the mathematical properties
//! of the dependency graph and node distributions:
//!
//!   1. Modularity Q    (Newman 2004) — edge clustering quality
//!   2. Acyclicity      — absence of circular dependencies
//!   3. Depth           — longest dependency chain
//!   4. Structural Entropy (Shannon 1948) — consistency of node properties
//!   5. Complexity Gini (Gini 1912) — concentration of complexity
//!   6. Redundancy      (Kolmogorov) — unnecessary code fraction

use crate::core::types::{CallEdge, FileNode, ImportEdge};
use std::collections::{HashMap, HashSet};

/// All 5 root cause scores, each normalized to [0, 1] where 1 = best.
///
/// 5 independent structural properties of a codebase-as-graph:
///   1. Modularity  — how well the graph clusters into modules
///   2. Acyclicity  — absence of circular dependencies
///   3. Depth       — shallow dependency chains
///   4. Equality    — complexity evenly distributed (no god files)
///   5. Redundancy  — no dead/duplicate code
///
/// Quality signal = geometric mean of all 5.
#[derive(Debug, Clone)]
pub struct RootCauseScores {
    /// Newman's Modularity Q normalized to [0, 1]. Higher = better modular structure.
    pub modularity: f64,
    /// Acyclicity score. 1.0 = no cycles, decays with more cycles.
    pub acyclicity: f64,
    /// Depth score. 1.0 = shallow, decays with deeper chains.
    pub depth: f64,
    /// Complexity equality. 1.0 = evenly distributed, 0.0 = all in one god file.
    pub equality: f64,
    /// Non-redundancy. 1.0 = no waste, 0.0 = all redundant.
    pub redundancy: f64,
}

/// Raw (un-normalized) root cause values for display.
#[derive(Debug, Clone)]
pub struct RootCauseRaw {
    /// Newman's Q ∈ [-0.5, 1.0]
    pub modularity_q: f64,
    /// Cycle count
    pub cycle_count: usize,
    /// Max dependency depth
    pub max_depth: u32,
    /// Gini coefficient ∈ [0, 1]
    pub complexity_gini: f64,
    /// Redundancy ratio ∈ [0, 1]
    pub redundancy_ratio: f64,
}

// ══════════════════════════════════════════════════════════════════
//  1. MODULARITY Q (Newman 2004)
// ══════════════════════════════════════════════════════════════════

/// Compute Newman's Modularity Q for the dependency graph.
///
/// Q = (1/m) * Σ_ij [A_ij - k_out_i * k_in_j / m] * δ(c_i, c_j)
///
/// For directed graphs. Measures how much the actual edge distribution
/// within modules exceeds what would be expected in a random graph
/// with the same degree sequence.
///
/// Q > 0.3 = significant modular structure
/// Q > 0.6 = strong modular structure
/// Q ≤ 0   = worse than random (anti-modular)
///
/// Uses both import and call edges for language-fair measurement.
pub fn compute_modularity_q(
    import_edges: &[ImportEdge],
    call_edges: &[CallEdge],
    files: &[&FileNode],
) -> f64 {
    // Collect all edges into (from, to) pairs, deduplicated
    let mut edges: HashSet<(&str, &str)> = HashSet::new();
    for e in import_edges {
        edges.insert((e.from_file.as_str(), e.to_file.as_str()));
    }
    for e in call_edges {
        edges.insert((e.from_file.as_str(), e.to_file.as_str()));
    }

    let m = edges.len();
    if m == 0 {
        return 1.0; // No edges → trivially modular (nothing connects)
    }

    // Compute out-degree and in-degree per node
    let mut k_out: HashMap<&str, usize> = HashMap::new();
    let mut k_in: HashMap<&str, usize> = HashMap::new();
    for &(from, to) in &edges {
        *k_out.entry(from).or_default() += 1;
        *k_in.entry(to).or_default() += 1;
    }

    // Q = (1/m) * Σ_ij [A_ij - k_out_i * k_in_j / m] * δ(c_i, c_j)
    let m_f = m as f64;

    // Term 1: actual intra-module edges
    let mut intra_module_edges: usize = 0;
    for &(from, to) in &edges {
        if crate::core::path_utils::module_of(from) == crate::core::path_utils::module_of(to) {
            intra_module_edges += 1;
        }
    }

    // Term 2: expected intra-module edges under null model
    // For each module, sum of k_out * sum of k_in / m
    let mut mod_k_out_sum: HashMap<&str, f64> = HashMap::new();
    let mut mod_k_in_sum: HashMap<&str, f64> = HashMap::new();

    // Collect all nodes (from files, not just edges, to handle isolated files)
    let mut all_nodes: HashSet<&str> = HashSet::new();
    for f in files {
        if !f.is_dir && !f.lang.is_empty() {
            all_nodes.insert(f.path.as_str());
        }
    }
    for &(from, to) in &edges {
        all_nodes.insert(from);
        all_nodes.insert(to);
    }

    for &node in &all_nodes {
        let m_node = crate::core::path_utils::module_of(node);
        let ko = *k_out.get(node).unwrap_or(&0) as f64;
        let ki = *k_in.get(node).unwrap_or(&0) as f64;
        *mod_k_out_sum.entry(m_node).or_default() += ko;
        *mod_k_in_sum.entry(m_node).or_default() += ki;
    }

    let mut expected_intra: f64 = 0.0;
    for (module, &ko_sum) in &mod_k_out_sum {
        let ki_sum = mod_k_in_sum.get(module).copied().unwrap_or(0.0);
        expected_intra += ko_sum * ki_sum / m_f;
    }

    let q = (intra_module_edges as f64 - expected_intra) / m_f;

    // Clamp to [-0.5, 1.0] (theoretical bounds)
    q.clamp(-0.5, 1.0)
}

// ══════════════════════════════════════════════════════════════════
//  4. STRUCTURAL ENTROPY (Shannon 1948)
// ══════════════════════════════════════════════════════════════════

/// Compute normalized Shannon entropy of a distribution.
/// H = -Σ p_i * log2(p_i), normalized by log2(N).
/// Returns 0.0 for empty/single-element distributions.
fn shannon_entropy_normalized(values: &[f64]) -> f64 {
    let n = values.len();
    if n <= 1 {
        return 0.0;
    }
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }

    let mut h: f64 = 0.0;
    for &v in values {
        if v > 0.0 {
            let p = v / total;
            h -= p * p.log2();
        }
    }

    let max_h = (n as f64).log2();
    if max_h > 0.0 {
        h / max_h
    } else {
        0.0
    }
}

/// Compute structural entropy: average normalized entropy of
/// file size distribution and function complexity distribution.
///
/// Low entropy = consistent (all files similar size, all functions similar CC)
/// High entropy = inconsistent (wildly varying sizes and complexities)
pub fn compute_structural_entropy(files: &[&FileNode]) -> f64 {
    let code_files: Vec<&&FileNode> = files
        .iter()
        .filter(|f| !f.is_dir && !f.lang.is_empty() && f.lang != "unknown")
        .collect();

    if code_files.len() <= 1 {
        return 1.0; // 0-1 files = trivially consistent (no variance possible)
    }

    // File size distribution entropy
    let sizes: Vec<f64> = code_files.iter().map(|f| f.lines as f64).collect();
    let size_entropy = shannon_entropy_normalized(&sizes);

    // Function complexity distribution entropy
    let mut complexities: Vec<f64> = Vec::new();
    for f in &code_files {
        if let Some(sa) = &f.sa {
            if let Some(funcs) = &sa.functions {
                for func in funcs {
                    if let Some(cc) = func.cc {
                        complexities.push(cc as f64);
                    }
                }
            }
        }
    }
    let cc_entropy = if complexities.len() > 1 {
        shannon_entropy_normalized(&complexities)
    } else {
        0.0
    };

    // Average of both distributions
    if complexities.len() > 1 {
        (size_entropy + cc_entropy) / 2.0
    } else {
        size_entropy // Only file sizes available
    }
}

// ══════════════════════════════════════════════════════════════════
//  5. COMPLEXITY GINI (Gini 1912)
// ══════════════════════════════════════════════════════════════════

/// Compute Gini coefficient of complexity distribution.
///
/// G = 0: perfectly equal (every function has same CC)
/// G = 1: perfectly unequal (one god function, rest trivial)
///
/// Uses per-function cyclomatic complexity. Falls back to per-file
/// line counts if no CC data available.
pub fn compute_complexity_gini(files: &[&FileNode]) -> f64 {
    // Collect all function CCs
    let mut values: Vec<f64> = Vec::new();
    for f in files {
        if f.is_dir || f.lang.is_empty() {
            continue;
        }
        if let Some(sa) = &f.sa {
            if let Some(funcs) = &sa.functions {
                for func in funcs {
                    values.push(func.cc.unwrap_or(1) as f64);
                }
            }
        }
    }

    // Fallback to file line counts if no function data
    if values.len() <= 1 {
        values = files
            .iter()
            .filter(|f| !f.is_dir && !f.lang.is_empty() && f.lang != "unknown")
            .map(|f| f.lines as f64)
            .collect();
    }

    if values.len() <= 1 {
        return 0.0;
    }

    gini_coefficient(&mut values)
}

/// Compute Gini coefficient from a mutable slice of non-negative values.
fn gini_coefficient(values: &mut [f64]) -> f64 {
    let n = values.len();
    if n <= 1 {
        return 0.0;
    }

    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }

    let mut numerator: f64 = 0.0;
    for (i, &v) in values.iter().enumerate() {
        numerator += (2.0 * (i + 1) as f64 - n as f64 - 1.0) * v;
    }

    (numerator / (n as f64 * total)).clamp(0.0, 1.0)
}

// ══════════════════════════════════════════════════════════════════
//  6. REDUNDANCY (Kolmogorov approximation)
// ══════════════════════════════════════════════════════════════════

/// Compute redundancy ratio: fraction of code that is unnecessary.
///
/// Combines dead functions + duplicate functions as a ratio of total.
/// R = (dead + duplicate) / total_functions
///
/// Returns 0.0 if no functions exist.
pub fn compute_redundancy_ratio(
    dead_count: usize,
    duplicate_count: usize,
    total_functions: usize,
) -> f64 {
    if total_functions == 0 {
        return 0.0;
    }
    let waste = (dead_count + duplicate_count).min(total_functions);
    waste as f64 / total_functions as f64
}

// ══════════════════════════════════════════════════════════════════
//  NORMALIZE + AGGREGATE
// ══════════════════════════════════════════════════════════════════

/// Normalize raw root cause values to [0, 1] scores and compute quality signal.
///
/// Normalization rules (no arbitrary parameters for 3 of 5):
///   Q:         score = (Q + 0.5) / 1.5      linear rescale [-0.5,1] → [0,1]
///   Cycles:    score = 1 / (1 + cycles)      sigmoid (unbounded count)
///   Depth:     score = 1 / (1 + depth / 8)   sigmoid (unbounded count, midpoint=8)
///   Gini:      score = 1 - G                 direct invert [0,1] → [1,0]
///   Redundancy: score = 1 - R                direct invert [0,1] → [1,0]
///
/// Quality signal = geometric mean of all 5 scores.
pub fn compute_root_cause_scores(raw: &RootCauseRaw) -> (RootCauseScores, f64) {
    let modularity = ((raw.modularity_q + 0.5) / 1.5).clamp(0.0, 1.0);
    let acyclicity = 1.0 / (1.0 + raw.cycle_count as f64);
    let depth = 1.0 / (1.0 + raw.max_depth as f64 / 8.0);
    let equality = (1.0 - raw.complexity_gini).clamp(0.0, 1.0);
    let redundancy = (1.0 - raw.redundancy_ratio).clamp(0.0, 1.0);

    let scores = RootCauseScores {
        modularity,
        acyclicity,
        depth,
        equality,
        redundancy,
    };

    // Geometric mean: (a * b * c * d * e)^(1/5)
    let values = [modularity, acyclicity, depth, equality, redundancy];
    let product: f64 = values.iter().map(|v| v.max(0.01)).product();
    let quality_signal = product.powf(1.0 / 5.0);

    (scores, quality_signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gini_equal_distribution() {
        let mut v = vec![10.0, 10.0, 10.0, 10.0];
        assert!(
            (gini_coefficient(&mut v)).abs() < 0.01,
            "equal values = Gini ≈ 0"
        );
    }

    #[test]
    fn gini_unequal_distribution() {
        let mut v = vec![0.0, 0.0, 0.0, 100.0];
        let g = gini_coefficient(&mut v);
        assert!(g > 0.6, "one value dominates = high Gini, got {}", g);
    }

    #[test]
    fn entropy_uniform_is_max() {
        let v = vec![10.0, 10.0, 10.0, 10.0];
        let h = shannon_entropy_normalized(&v);
        assert!(
            (h - 1.0).abs() < 0.01,
            "uniform distribution = max entropy = 1.0, got {}",
            h
        );
    }

    #[test]
    fn entropy_concentrated_is_low() {
        let v = vec![100.0, 1.0, 1.0, 1.0];
        let h = shannon_entropy_normalized(&v);
        assert!(
            h < 0.5,
            "concentrated distribution = low entropy, got {}",
            h
        );
    }

    #[test]
    fn modularity_q_no_edges() {
        assert_eq!(compute_modularity_q(&[], &[], &[]), 1.0); // trivially modular
    }

    #[test]
    fn root_cause_scores_normalize() {
        let raw = RootCauseRaw {
            modularity_q: 0.5,
            cycle_count: 0,
            max_depth: 4,
            complexity_gini: 0.2,
            redundancy_ratio: 0.1,
        };
        let (scores, signal) = compute_root_cause_scores(&raw);
        assert!(
            scores.modularity > 0.6,
            "Q=0.5 → good modularity, got {}",
            scores.modularity
        );
        assert_eq!(scores.acyclicity, 1.0, "0 cycles = perfect");
        assert!(
            scores.depth > 0.5,
            "depth 4 / midpoint 8 = decent, got {}",
            scores.depth
        );
        assert!(
            scores.equality > 0.7,
            "low gini = good equality, got {}",
            scores.equality
        );
        assert!(
            scores.redundancy > 0.8,
            "low redundancy = good, got {}",
            scores.redundancy
        );
        assert!(
            signal > 0.6,
            "overall signal should be decent, got {}",
            signal
        );
    }
}
