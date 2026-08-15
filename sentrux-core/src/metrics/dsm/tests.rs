//! Tests for the Design Structure Matrix (DSM) builder.
//!
//! Validates DSM construction from import edges: empty graphs, single edges,
//! level-sorted ordering, bidirectional edges, and matrix symmetry properties.
//! Tests that the DSM index is consistent with the adjacency matrix and that
//! files are ordered by dependency level (sinks first).
//! Key function tested: `build_dsm`.

use super::*;
use crate::metrics::test_helpers::edge;

#[test]
fn empty_dsm() {
    let dsm = build_dsm(&[]);
    assert_eq!(dsm.size, 0);
    assert!(dsm.matrix.is_empty());
}

#[test]
fn single_edge_dsm() {
    let edges = vec![edge("a.rs", "b.rs")];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.size, 2);
    assert_eq!(dsm.edge_count, 1);
    let a_idx = dsm.index["a.rs"];
    let b_idx = dsm.index["b.rs"];
    assert!(dsm.matrix[a_idx][b_idx]);
}

#[test]
fn dsm_sorted_by_level() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.files[0], "c.rs");
    assert_eq!(dsm.files[1], "b.rs");
    assert_eq!(dsm.files[2], "a.rs");
}

#[test]
fn dsm_above_below_diagonal() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.below_diagonal, 2, "clean DAG = all below diagonal");
    assert_eq!(dsm.above_diagonal, 0);
}

#[test]
fn dsm_same_level_edges_not_counted() {
    let edges = vec![
        edge("a.rs", "c.rs"),
        edge("b.rs", "c.rs"),
        edge("a.rs", "b.rs"),
        edge("b.rs", "a.rs"),
    ];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.below_diagonal, 2, "only cross-level edges counted");
    assert_eq!(
        dsm.above_diagonal, 0,
        "same-level edges not counted as inversions"
    );
    assert_eq!(dsm.same_level, 2, "cycle edges are same-level");
    assert_eq!(dsm.edge_count, 4, "all edges still in matrix");
    assert_eq!(
        dsm.above_diagonal + dsm.below_diagonal + dsm.same_level,
        dsm.edge_count,
        "edge classification must be exhaustive"
    );
}

#[test]
fn dsm_detects_inversion() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
    ];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.edge_count, 3);
    assert_eq!(
        dsm.above_diagonal + dsm.below_diagonal,
        0,
        "cycle edges are same-level, not classified as above/below"
    );
}

#[test]
fn dsm_detects_cross_level_inversion() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "b.rs"),
    ];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.edge_count, 3);
    assert!(dsm.below_diagonal >= 1, "a→b should be below diagonal");
}

#[test]
fn dsm_stats_density() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let dsm = build_dsm(&edges);
    let stats = compute_stats(&dsm);
    assert!((stats.density - 2.0 / 6.0).abs() < 0.001);
}

#[test]
fn dsm_propagation_cost() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let dsm = build_dsm(&edges);
    let stats = compute_stats(&dsm);
    // a reaches {b,c}=2, b reaches {c}=1, c reaches {}=0. Total=3.
    // Normalized by N*(N-1) = 3*2 = 6 (max reachability per node is N-1, not N).
    let expected = 3.0 / 6.0;
    assert!(
        (stats.propagation_cost - expected).abs() < 0.01,
        "propagation_cost={}, expected={}",
        stats.propagation_cost,
        expected
    );
}

#[test]
fn dsm_clusters_one_directional_no_cluster() {
    let edges = vec![
        edge("a.rs", "c.rs"),
        edge("b.rs", "c.rs"),
        edge("a.rs", "b.rs"),
    ];
    let dsm = build_dsm(&edges);
    let stats = compute_stats(&dsm);
    assert_eq!(
        stats.clusters.len(),
        0,
        "one-directional edge = no mutual dependency cluster"
    );
}

#[test]
fn dsm_clusters_mutual_dependency() {
    let edges = vec![
        edge("a.rs", "c.rs"),
        edge("b.rs", "c.rs"),
        edge("a.rs", "b.rs"),
        edge("b.rs", "a.rs"),
    ];
    let dsm = build_dsm(&edges);
    let stats = compute_stats(&dsm);
    assert_eq!(stats.clusters.len(), 1, "mutual dependency = cluster");
    assert_eq!(stats.clusters[0].files.len(), 2);
    assert_eq!(
        stats.clusters[0].internal_edges, 2,
        "both directed edges counted"
    );
}

#[test]
fn render_text_produces_output() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let dsm = build_dsm(&edges);
    let text = render_text(&dsm, 10);
    assert!(text.contains("×"));
    assert!(text.contains("■"));
}

// ── Idempotency ──

#[test]
fn dsm_idempotent() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("a.rs", "c.rs"),
    ];
    let d1 = build_dsm(&edges);
    let d2 = build_dsm(&edges);
    assert_eq!(d1.files, d2.files);
    assert_eq!(d1.edge_count, d2.edge_count);
    assert_eq!(d1.matrix, d2.matrix);
}

// ── Conservation: edge count preserved ──

#[test]
fn dsm_conserves_edges() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "d.rs"),
        edge("a.rs", "d.rs"),
    ];
    let dsm = build_dsm(&edges);
    assert_eq!(dsm.edge_count, edges.len());
}
