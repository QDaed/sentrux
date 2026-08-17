//! Cross-validation of quality signal via compression ratio (FREE).
//!
//! Computes an independent quality estimate by measuring the compressibility
//! of a label-independent structural encoding of the dependency edge list.
//! High compressibility = high redundancy/pattern = lower quality.
//! Low compressibility = diverse local structure = higher quality.
//!
//! This provides a second opinion on the quality_signal from root_causes.
//! If both agree, confidence is high. If they disagree, one sensor may be blind.

use crate::core::types::ImportEdge;
use std::collections::HashMap;
use std::io::Write;

/// Independent quality estimate based on DEFLATE compression of the
/// dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CrossValidation {
    /// compressed_size / original_size, clamped to `[0,1]`.
    /// Lower = more compressible = more redundant = lower quality.
    pub compression_ratio: f64,
    /// How closely `compression_ratio` aligns with `quality_signal`, `[0,1]`.
    pub agreement: f64,
    /// Combined confidence in the quality signal, `[0,1]`.
    pub confidence: f64,
}

/// Deterministic 64-bit FNV-1a hash for a sequence of `u64`s.
fn fnv1a_hash_u64s(values: &[u64]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;
    let mut h = OFFSET;
    for v in values {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
    }
    h
}

/// Compute a compression-based cross-validation of the root-cause quality signal.
///
/// Returns `None` when there is no structural dependency data, because a
/// compression estimate needs edges to be meaningful.
///
/// Each node is replaced by a label-independent "portrait" hash built from its
/// in/out degrees and the sorted degrees of its neighbors, so the compressed
/// edge stream reflects graph topology rather than raw path strings. This makes
/// the metric stable under file renames and automorphic relabelings.
pub fn compute(edges: &[ImportEdge], quality_signal: f64) -> Option<CrossValidation> {
    if edges.is_empty() {
        return None;
    }

    // Build label-to-index maps. Indices are only used to build adjacency; the
    // final metric depends on structural hashes, not on these indices or the
    // original path strings.
    let mut name_to_idx = HashMap::with_capacity(edges.len() * 2);
    let mut indexed_edges: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
    for e in edges
        .iter()
        .filter(|e| !e.from_file.is_empty() && !e.to_file.is_empty())
    {
        let from = match name_to_idx.get(e.from_file.as_str()) {
            Some(&idx) => idx,
            None => {
                let idx = name_to_idx.len();
                name_to_idx.insert(e.from_file.as_str(), idx);
                idx
            }
        };
        let to = match name_to_idx.get(e.to_file.as_str()) {
            Some(&idx) => idx,
            None => {
                let idx = name_to_idx.len();
                name_to_idx.insert(e.to_file.as_str(), idx);
                idx
            }
        };
        indexed_edges.push((from, to));
    }
    if indexed_edges.is_empty() {
        return None;
    }

    let n = name_to_idx.len();
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut out_deg = vec![0u64; n];
    let mut in_deg = vec![0u64; n];

    for (u, v) in &indexed_edges {
        out_edges[*u].push(*v);
        in_edges[*v].push(*u);
        out_deg[*u] += 1;
        in_deg[*v] += 1;
    }

    // Compute a label-independent color for each node via a few rounds of the
    // Weisfeiler-Lehman color-refinement.  Each round hashes a node's in/out
    // degrees together with the sorted colors of its outgoing and incoming
    // neighbors.  This makes the encoding sensitive to local topology while
    // remaining invariant under file renames and automorphic relabelings.
    let mut colors: Vec<u64> = (0..n)
        .map(|i| fnv1a_hash_u64s(&[out_deg[i], in_deg[i]]))
        .collect();
    for _ in 0..3 {
        let mut next: Vec<u64> = Vec::with_capacity(n);
        let mut scratch: Vec<u64> = Vec::with_capacity(
            out_edges
                .iter()
                .map(|v| v.len())
                .max()
                .unwrap_or(0)
                .max(in_edges.iter().map(|v| v.len()).max().unwrap_or(0)),
        );
        for i in 0..n {
            let mut parts: Vec<u64> =
                Vec::with_capacity(2 + out_edges[i].len() + in_edges[i].len());
            parts.push(out_deg[i]);
            parts.push(in_deg[i]);

            scratch.clear();
            for &j in &out_edges[i] {
                scratch.push(colors[j]);
            }
            scratch.sort_unstable();
            parts.extend(&scratch);

            scratch.clear();
            for &j in &in_edges[i] {
                scratch.push(colors[j]);
            }
            scratch.sort_unstable();
            parts.extend(&scratch);

            next.push(fnv1a_hash_u64s(&parts));
        }
        colors = next;
    }
    let portraits = colors;

    // Encode each edge as a single canonical hash of its ordered endpoint
    // portraits, then sort the hashes.  This gives a label-independent
    // multiset representation of the edge set; sorting by the combined hash
    // avoids long runs of repeated source bytes and makes the compression ratio
    // reflect edge diversity rather than adjacency-list regularity.
    let mut edge_hashes: Vec<u64> = Vec::with_capacity(indexed_edges.len());
    for (u, v) in indexed_edges {
        edge_hashes.push(fnv1a_hash_u64s(&[portraits[u], portraits[v]]));
    }
    edge_hashes.sort_unstable();

    let mut original = Vec::with_capacity(edge_hashes.len() * 8);
    for h in edge_hashes {
        original.extend_from_slice(&h.to_le_bytes());
    }

    let original_len = original.len() as f64;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&original).ok()?;
    let compressed = encoder.finish().ok()?;
    let compressed_len = compressed.len() as f64;

    // Clamp to [0, 1]. Values > 1 mean the data did not compress (overhead);
    // we treat that as fully incompressible, i.e. high structural uniqueness.
    let compression_ratio = (compressed_len / original_len).clamp(0.0, 1.0);
    let expected = quality_signal.clamp(0.0, 1.0);
    let agreement = (1.0 - (compression_ratio - expected).abs()).clamp(0.0, 1.0);
    let confidence = agreement;

    Some(CrossValidation {
        compression_ratio,
        agreement,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from_file: from.into(),
            to_file: to.into(),
        }
    }

    /// Deterministic splitmix64 PRNG.
    fn splitmix64(x: &mut u64) -> u64 {
        *x = x.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = *x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// A random tournament on `n` nodes: for each unordered pair, pick one
    /// direction deterministically from `seed`.  Random tournaments usually
    /// have trivial automorphism groups, so color refinement produces unique
    /// node colors and the sorted edge stream is essentially random, giving a
    /// high compression ratio.
    fn random_tournament(n: usize, seed: u64) -> Vec<ImportEdge> {
        let mut result = Vec::with_capacity(n * (n - 1) / 2);
        let mut rng = seed;
        for i in 0..n {
            for j in (i + 1)..n {
                if splitmix64(&mut rng).is_multiple_of(2) {
                    result.push(edge(
                        &format!("src/node_{}.rs", i),
                        &format!("src/node_{}.rs", j),
                    ));
                } else {
                    result.push(edge(
                        &format!("src/node_{}.rs", j),
                        &format!("src/node_{}.rs", i),
                    ));
                }
            }
        }
        result
    }

    #[test]
    fn empty_edges_returns_none() {
        assert!(compute(&[], 0.5).is_none());
    }

    #[test]
    fn compressible_redundant_graph_agrees_with_low_quality() {
        // 100 identical edges are highly compressible => low ratio => low quality agreement.
        let edges: Vec<_> = (0..100).map(|_| edge("src/a.rs", "src/b.rs")).collect();
        let cv = compute(&edges, 0.05).unwrap();
        assert!(
            cv.compression_ratio < 0.5,
            "repeated edges should compress well"
        );
        assert!(
            cv.agreement > 0.5,
            "low quality should agree with low ratio"
        );
        assert!(cv.confidence > 0.0);
    }

    #[test]
    fn incompressible_unique_graph_agrees_with_high_quality() {
        // A random tournament has unique node colors and an essentially random
        // edge stream, so it should be incompressible and agree with a high
        // quality signal.
        let unique = random_tournament(50, 1);
        let redundant: Vec<_> = (0..100).map(|_| edge("src/a.rs", "src/b.rs")).collect();
        let cv_unique = compute(&unique, 0.95).unwrap();
        let cv_redundant = compute(&redundant, 0.5).unwrap();
        assert!(
            cv_unique.compression_ratio > cv_redundant.compression_ratio,
            "unique graph should be less compressible than redundant graph"
        );
        assert!(
            cv_unique.agreement > 0.5,
            "high quality should agree with high ratio"
        );
    }

    #[test]
    fn high_quality_with_compressible_graph_produces_low_agreement() {
        // A high quality signal paired with a highly redundant graph is a mismatch.
        let edges: Vec<_> = (0..100).map(|_| edge("src/a.rs", "src/b.rs")).collect();
        let cv = compute(&edges, 0.95).unwrap();
        assert!(cv.agreement < 0.5, "mismatch should produce low agreement");
    }

    #[test]
    fn unique_graph_has_higher_ratio_than_redundant_graph() {
        let redundant: Vec<_> = (0..100).map(|_| edge("src/a.rs", "src/b.rs")).collect();
        let unique = random_tournament(50, 2);
        let cv_redundant = compute(&redundant, 0.5).unwrap();
        let cv_unique = compute(&unique, 0.5).unwrap();
        assert!(
            cv_unique.compression_ratio > cv_redundant.compression_ratio,
            "unique graph should be less compressible than redundant graph"
        );
    }

    #[test]
    fn clamps_quality_signal_and_ratio() {
        // A single edge is too small to compress; ratio clamps to 1.
        let edges = vec![edge("a.rs", "b.rs")];
        let cv = compute(&edges, -0.5).unwrap();
        assert!(cv.compression_ratio >= 0.0 && cv.compression_ratio <= 1.0);
        assert!(cv.agreement >= 0.0 && cv.agreement <= 1.0);
        assert!(cv.confidence >= 0.0 && cv.confidence <= 1.0);
    }

    #[test]
    fn rename_does_not_change_cross_validation() {
        // Two isomorphic graphs with different file names should produce the
        // same compression ratio (and therefore same agreement/confidence).
        let base = vec![
            edge("src/original.rs", "src/mid.rs"),
            edge("src/mid.rs", "src/leaf.rs"),
        ];
        let renamed = vec![
            edge("src/renamed.rs", "src/mid.rs"),
            edge("src/mid.rs", "src/leaf.rs"),
        ];
        let cv_base = compute(&base, 0.8).unwrap();
        let cv_renamed = compute(&renamed, 0.8).unwrap();
        assert!(
            (cv_base.compression_ratio - cv_renamed.compression_ratio).abs() < f64::EPSILON,
            "renaming a node should not change the structural compression ratio"
        );
    }
}
