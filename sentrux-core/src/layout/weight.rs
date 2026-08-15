//! Weight computation for layout sizing — maps file metrics to visual area.
//!
//! Provides size extraction (lines/logic/funcs/heat), scaling transforms
//! (linear/sqrt/log/smooth), and bottom-up weight precomputation with
//! focus-mode filtering and hidden-path exclusion.

use super::types::{FocusMode, ScaleMode, SizeMode};
use crate::core::types::FileNode;
use std::collections::{HashMap, HashSet};

/// Pre-sorted hidden paths for O(log H) prefix matching via binary search.
/// Replaces the O(H) linear scan in `is_hidden()`.
pub struct SortedHiddenPaths {
    exact: HashSet<String>,
    sorted: Vec<String>,
}

impl SortedHiddenPaths {
    /// Build from a HashSet: clone into a sorted Vec for binary-search prefix matching.
    pub fn new(hidden_paths: &HashSet<String>) -> Self {
        let exact = hidden_paths.clone();
        let mut sorted: Vec<String> = hidden_paths.iter().cloned().collect();
        sorted.sort();
        Self { exact, sorted }
    }

    /// Check if `path` is hidden: exact match or prefix match (dir hides all children).
    /// Uses binary search on the sorted list to find the closest prefix candidate,
    /// making this O(log H) instead of O(H).
    #[inline]
    pub fn is_hidden(&self, path: &str) -> bool {
        if self.exact.contains(path) {
            return true;
        }
        // Binary search for the last entry <= path. If that entry is a prefix of path
        // (with a '/' separator), then path is hidden under that directory.
        let idx = self.sorted.partition_point(|hp| hp.as_str() <= path);
        if idx > 0 {
            let candidate = &self.sorted[idx - 1];
            if path.starts_with(candidate.as_str())
                && path.as_bytes().get(candidate.len()) == Some(&b'/')
            {
                return true;
            }
        }
        false
    }
}

/// Configuration for weight precomputation — groups the 7 read-only params
/// that flow through every recursive call of `precompute_weights`.
pub struct WeightConfig<'a> {
    /// Which metric determines file block area
    pub size_mode: SizeMode,
    /// Scaling transform to compress extreme ranges
    pub scale_mode: ScaleMode,
    /// Live heat values for SizeMode::Heat (None when not active)
    pub heat_map: Option<&'a HashMap<String, f64>>,
    /// Floor weight ensuring tiny files remain visible
    pub min_child_weight: f64,
    /// Focus filter controlling which files appear in layout
    pub focus_mode: &'a FocusMode,
    /// Entry point file paths for FocusMode::EntryPoints filtering
    pub entry_point_files: &'a HashSet<String>,
    /// User-hidden paths to exclude from layout
    pub hidden_paths: &'a HashSet<String>,
    /// Pre-computed impact set for ImpactRadius focus mode
    pub impact_files: Option<&'a HashSet<String>>,
}

/// Look up the precomputed weight for a node from the cache. Returns 0.0 if absent.
/// Used by both treemap and blueprint layout engines after `precompute_weights`.
pub fn get_w(node: &FileNode, weights: &std::collections::HashMap<String, f64>) -> f64 {
    weights.get(&node.path).copied().unwrap_or(0.0)
}

/// Apply scaling transform to compress extreme size differences.
/// - linear: raw value
/// - sqrt: sqrt(x), moderate compression
/// - log: log2(1+x), heavy compression — makes small files visible
/// - smooth: x^0.6, between linear and sqrt — best balance
pub fn apply_scale(value: f64, mode: ScaleMode) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    match mode {
        ScaleMode::Linear => value,
        ScaleMode::Sqrt => value.sqrt(),
        ScaleMode::Log => (1.0 + value).log2(),
        ScaleMode::Smooth => value.powf(0.6),
    }
}

/// Extract weight from FileNode based on size mode.
/// `heat_map` provides live heat values from HeatTracker (keyed by file path).
/// Required for SizeMode::Heat — without it, Heat falls back to Uniform.
pub fn get_size_weight(
    node: &FileNode,
    mode: SizeMode,
    heat_map: Option<&std::collections::HashMap<String, f64>>,
) -> f64 {
    match mode {
        SizeMode::Lines => (node.lines as f64).max(1.0),
        SizeMode::Logic => (node.logic as f64).max(1.0),
        SizeMode::Funcs => (node.funcs as f64).max(1.0),
        SizeMode::Comments => (node.comments as f64).max(1.0),
        SizeMode::Blanks => (node.blanks as f64).max(1.0),
        SizeMode::Heat => {
            // BUG 1 fix: use live heat from HeatTracker instead of dead node.heat field
            let h = heat_map
                .and_then(|m| m.get(&node.path))
                .copied()
                .unwrap_or(0.0);
            (h * 100.0 + 1.0).max(1.0)
        }
        SizeMode::Uniform => 1.0,
    }
}

/// Maximum recursion depth guard for tree traversals.
/// Prevents stack overflow on pathological or cyclic tree structures.
pub const MAX_DEPTH: u32 = 128;

/// Bottom-up weight precomputation: children computed first, parent sums from
/// cache. O(n) instead of the naive O(n×d) recursive approach.
/// Shared by both treemap and blueprint layout engines.
pub fn precompute_weights(
    node: &FileNode,
    wc: &WeightConfig<'_>,
    cache: &mut HashMap<String, f64>,
) {
    // Build sorted hidden paths once for O(log H) prefix matching
    let sorted_hidden = SortedHiddenPaths::new(wc.hidden_paths);
    precompute_weights_inner(node, wc, cache, &sorted_hidden, 0);
}

/// Inner recursive implementation with depth guard and pre-built sorted hidden paths.
fn precompute_weights_inner(
    node: &FileNode,
    wc: &WeightConfig<'_>,
    cache: &mut HashMap<String, f64>,
    sorted_hidden: &SortedHiddenPaths,
    depth: u32,
) {
    if depth >= MAX_DEPTH {
        cache.insert(node.path.clone(), 0.0);
        return;
    }

    if sorted_hidden.is_hidden(&node.path) {
        cache.insert(node.path.clone(), 0.0);
        return;
    }

    if node.is_dir {
        precompute_dir_weight(node, wc, cache, sorted_hidden, depth);
    } else {
        precompute_file_weight(node, wc, cache);
    }
}

/// Compute weight for a directory node: recurse children, then sum with floor.
fn precompute_dir_weight(
    node: &FileNode,
    wc: &WeightConfig<'_>,
    cache: &mut HashMap<String, f64>,
    sorted_hidden: &SortedHiddenPaths,
    depth: u32,
) {
    if let Some(children) = &node.children {
        for c in children {
            precompute_weights_inner(c, wc, cache, sorted_hidden, depth + 1);
        }
        let mut sum: f64 = children
            .iter()
            .map(|c| cache.get(&c.path).copied().unwrap_or(0.0))
            .sum();
        // Ensure directory has minimum weight based on children with non-zero content.
        // If all children are hidden/filtered (non_zero == 0 && sum == 0.0), keep sum = 0.0
        // to avoid phantom empty directories appearing in layout.
        if !children.is_empty() {
            let non_zero = children
                .iter()
                .filter(|c| cache.get(&c.path).copied().unwrap_or(0.0) > 0.0)
                .count();
            if non_zero > 0 {
                let min_w = non_zero as f64 * wc.min_child_weight;
                if sum < min_w {
                    sum = min_w;
                }
            }
        }
        cache.insert(node.path.clone(), sum.max(0.0));
    } else {
        cache.insert(node.path.clone(), 0.0);
    }
}

/// Compute weight for a file node: apply focus filter, size mode, and scale.
fn precompute_file_weight(
    node: &FileNode,
    wc: &WeightConfig<'_>,
    cache: &mut HashMap<String, f64>,
) {
    let is_entry = wc.entry_point_files.contains(&node.path);
    if !wc
        .focus_mode
        .includes_with_impact(&node.path, &node.lang, is_entry, wc.impact_files)
    {
        cache.insert(node.path.clone(), 0.0);
        return;
    }
    let raw = get_size_weight(node, wc.size_mode, wc.heat_map);
    let w = apply_scale(raw, wc.scale_mode);
    cache.insert(node.path.clone(), w.max(wc.min_child_weight));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_scale_zero() {
        assert_eq!(apply_scale(0.0, ScaleMode::Linear), 0.0);
        assert_eq!(apply_scale(0.0, ScaleMode::Sqrt), 0.0);
        assert_eq!(apply_scale(0.0, ScaleMode::Log), 0.0);
        assert_eq!(apply_scale(0.0, ScaleMode::Smooth), 0.0);
    }

    #[test]
    fn test_apply_scale_negative() {
        assert_eq!(apply_scale(-5.0, ScaleMode::Linear), 0.0);
    }

    #[test]
    fn test_apply_scale_positive() {
        assert_eq!(apply_scale(100.0, ScaleMode::Linear), 100.0);
        assert!((apply_scale(100.0, ScaleMode::Sqrt) - 10.0).abs() < 1e-10);
        // log2(1+100) = log2(101) ≈ 6.658
        assert!((apply_scale(100.0, ScaleMode::Log) - (1.0 + 100.0_f64).log2()).abs() < 0.01);
        // 100^0.6 ≈ 15.848
        assert!((apply_scale(100.0, ScaleMode::Smooth) - 100.0_f64.powf(0.6)).abs() < 1e-10);
    }

    #[test]
    fn test_monotonicity_scale() {
        // More input → more output for all modes
        for mode in [
            ScaleMode::Linear,
            ScaleMode::Sqrt,
            ScaleMode::Log,
            ScaleMode::Smooth,
        ] {
            let a = apply_scale(10.0, mode);
            let b = apply_scale(100.0, mode);
            let c = apply_scale(1000.0, mode);
            assert!(b > a, "mode {:?}: 100 should > 10", mode);
            assert!(c > b, "mode {:?}: 1000 should > 100", mode);
        }
    }

    #[test]
    fn test_get_size_weight_uniform() {
        let node = FileNode {
            path: "test.rs".into(),
            name: "test.rs".into(),
            is_dir: false,
            lines: 500,
            logic: 300,
            comments: 50,
            blanks: 150,
            funcs: 10,
            mtime: 0.0,
            gs: String::new(),
            lang: "rust".into(),
            sa: None,
            children: None,
        };
        assert_eq!(get_size_weight(&node, SizeMode::Uniform, None), 1.0);
        assert_eq!(get_size_weight(&node, SizeMode::Lines, None), 500.0);
        assert_eq!(get_size_weight(&node, SizeMode::Funcs, None), 10.0);
    }
}
