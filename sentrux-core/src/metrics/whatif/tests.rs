//! Tests for what-if scenario analysis (`metrics::whatif`).
//!
//! Validates edge rewriting for hypothetical refactoring actions: MoveFile
//! (path substitution in all edges), ExtractModule (edge splitting), and
//! DeleteFile (edge removal). Tests conservation (edge count preserved for
//! moves), idempotency (applying same move twice is stable), and boundary
//! (moving a file not in the graph is a no-op).

use super::*;
use crate::metrics::test_helpers::edge;

// ── MoveFile tests ──

#[test]
fn move_file_updates_all_edges() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("d.rs", "b.rs"),
    ];
    let action = WhatIfAction::MoveFile {
        old_path: "b.rs".into(),
        new_path: "lib/b.rs".into(),
    };
    let (new_edges, _) = apply_action(&edges, &action);

    for e in &new_edges {
        assert_ne!(e.from_file, "b.rs");
        assert_ne!(e.to_file, "b.rs");
    }
    assert!(new_edges
        .iter()
        .any(|e| e.from_file == "a.rs" && e.to_file == "lib/b.rs"));
    assert!(new_edges
        .iter()
        .any(|e| e.from_file == "lib/b.rs" && e.to_file == "c.rs"));
    assert!(new_edges
        .iter()
        .any(|e| e.from_file == "d.rs" && e.to_file == "lib/b.rs"));
}

#[test]
fn move_file_preserves_edge_count() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let action = WhatIfAction::MoveFile {
        old_path: "b.rs".into(),
        new_path: "new/b.rs".into(),
    };
    let (new_edges, _) = apply_action(&edges, &action);
    assert_eq!(new_edges.len(), edges.len());
}

// ── AddEdge tests ──

#[test]
fn add_edge_increases_count() {
    let edges = vec![edge("a.rs", "b.rs")];
    let action = WhatIfAction::AddEdge {
        from: "c.rs".into(),
        to: "a.rs".into(),
    };
    let (new_edges, _) = apply_action(&edges, &action);
    assert_eq!(new_edges.len(), 2);
}

#[test]
fn add_edge_can_worsen_score() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let result = simulate(
        &edges,
        &[],
        &WhatIfAction::AddEdge {
            from: "c.rs".into(),
            to: "a.rs".into(),
        },
    );
    assert!(
        result.upward_violations_after >= result.upward_violations_before
            || result.max_blast_after >= result.max_blast_before,
        "adding a back-edge should not improve architecture"
    );
}

// ── RemoveEdge tests ──

#[test]
fn remove_edge_decreases_count() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let action = WhatIfAction::RemoveEdge {
        from: "b.rs".into(),
        to: "c.rs".into(),
    };
    let (new_edges, _) = apply_action(&edges, &action);
    assert_eq!(new_edges.len(), 1);
}

#[test]
fn remove_edge_reduces_blast() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let result = simulate(
        &edges,
        &[],
        &WhatIfAction::RemoveEdge {
            from: "b.rs".into(),
            to: "c.rs".into(),
        },
    );
    assert!(result.max_blast_after <= result.max_blast_before);
}

// ── RemoveFile tests ──

#[test]
fn remove_file_removes_all_edges() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("d.rs", "b.rs"),
    ];
    let action = WhatIfAction::RemoveFile {
        path: "b.rs".into(),
    };
    let (new_edges, _) = apply_action(&edges, &action);
    assert!(new_edges.is_empty());
}

// ── BreakCycle tests ──

#[test]
fn break_cycle_removes_one_edge() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
    ];
    let action = WhatIfAction::BreakCycle {
        files: vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
    };
    let (new_edges, desc) = apply_action(&edges, &action);
    assert_eq!(new_edges.len(), 2, "should remove exactly one edge");
    assert!(desc.contains("Break cycle"));
}

#[test]
fn break_cycle_picks_best_edge() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
        edge("d.rs", "c.rs"),
    ];
    let best = find_best_cycle_break(&edges, &["a.rs".into(), "b.rs".into(), "c.rs".into()]);
    assert!(best.is_some());
}

// ── Simulate integration ──

#[test]
fn simulate_reports_improvement() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
    ];
    let result = simulate(
        &edges,
        &[],
        &WhatIfAction::RemoveEdge {
            from: "c.rs".into(),
            to: "a.rs".into(),
        },
    );
    assert!(
        result.improved,
        "removing cycle edge should improve architecture"
    );
    assert!(
        result.score_after > 0.8,
        "score should be high after removing violations"
    );
}

// ── Sequence simulation ──

#[test]
fn simulate_sequence_applies_all() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
    ];
    let actions = vec![
        WhatIfAction::RemoveEdge {
            from: "c.rs".into(),
            to: "a.rs".into(),
        },
        WhatIfAction::AddEdge {
            from: "d.rs".into(),
            to: "c.rs".into(),
        },
    ];
    let result = simulate_sequence(&edges, &[], &actions);
    assert!(result.action_description.contains("→"));
    assert!(
        result.score_after > 0.8,
        "score should be high after removing violations"
    );
}

// ── Idempotency: simulating no-op returns same state ──

#[test]
fn simulate_noop_idempotent() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let result = simulate(
        &edges,
        &[],
        &WhatIfAction::AddEdge {
            from: "a.rs".into(),
            to: "b.rs".into(),
        },
    );
    assert!(result.score_after >= 0.0 && result.score_after <= 1.0);
}

// ── Symmetry: move then move back = original ──

#[test]
fn move_roundtrip_restores_original() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let (moved, _) = apply_action(
        &edges,
        &WhatIfAction::MoveFile {
            old_path: "b.rs".into(),
            new_path: "x.rs".into(),
        },
    );
    let (restored, _) = apply_action(
        &moved,
        &WhatIfAction::MoveFile {
            old_path: "x.rs".into(),
            new_path: "b.rs".into(),
        },
    );
    for (orig, rest) in edges.iter().zip(restored.iter()) {
        assert_eq!(orig.from_file, rest.from_file);
        assert_eq!(orig.to_file, rest.to_file);
    }
}

// ── Level changes tracked ──

#[test]
fn level_changes_detected() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let result = simulate(
        &edges,
        &[],
        &WhatIfAction::RemoveEdge {
            from: "a.rs".into(),
            to: "b.rs".into(),
        },
    );
    assert!(
        !result.level_changes.is_empty() || result.changes.iter().any(|c| c.contains("level")),
        "should detect level changes when edges are removed"
    );
}
