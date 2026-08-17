//! Cross-validation of quality signal via compression ratio (FREE).
//!
//! Computes an independent quality estimate by measuring the compressibility
//! of the dependency edge list. High compressibility = high redundancy/pattern
//! = lower quality. Low compressibility = unique structure = higher quality.
//!
//! This provides a second opinion on the quality_signal from root_causes.
//! If both agree, confidence is high. If they disagree, one sensor may be blind.

use crate::core::types::ImportEdge;
use std::collections::HashMap;
use std::io::Write;

/// Independent quality estimate based on DEFLATE compression of the
/// dependency graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossValidation {
    /// compressed_size / original_size, clamped to `[0,1]`.
    /// Lower = more compressible = more redundant = lower quality.
    pub compression_ratio: f64,
    /// How closely `compression_ratio` aligns with `quality_signal`, `[0,1]`.
    pub agreement: f64,
    /// Combined confidence in the quality signal, `[0,1]`.
    pub confidence: f64,
}

/// Deterministic 64-bit FNV-1a hash used to assign label-independent node IDs.
fn fnv1a_hash(s: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;
    let mut h = OFFSET;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Compute a compression-based cross-validation of the root-cause quality signal.
///
/// Returns `None` when there is no structural dependency data, because a
/// compression estimate needs edges to be meaningful.
///
/// Edges are encoded as pairs of deterministic 64-bit node IDs derived from file
/// paths, so the metric reflects edge-structure redundancy rather than the
/// compressibility of the raw path strings.
pub fn compute(edges: &[ImportEdge], quality_signal: f64) -> Option<CrossValidation> {
    if edges.is_empty() {
        return None;
    }

    let mut ids = HashMap::with_capacity(edges.len() * 2);
    let mut pairs: Vec<(u64, u64)> = Vec::with_capacity(edges.len());
    for e in edges
        .iter()
        .filter(|e| !e.from_file.is_empty() && !e.to_file.is_empty())
    {
        let from_id = *ids
            .entry(&e.from_file)
            .or_insert_with(|| fnv1a_hash(&e.from_file));
        let to_id = *ids
            .entry(&e.to_file)
            .or_insert_with(|| fnv1a_hash(&e.to_file));
        pairs.push((from_id, to_id));
    }
    if pairs.is_empty() {
        return None;
    }

    pairs.sort_unstable();

    let mut original = Vec::with_capacity(pairs.len() * 16);
    for (from, to) in pairs {
        original.extend_from_slice(&from.to_le_bytes());
        original.extend_from_slice(&to.to_le_bytes());
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

    /// Deterministic pseudo-random printable-ASCII string for incompressible edges.
    fn random_name(seed: u64) -> String {
        fn next(x: &mut u64) -> u64 {
            *x = x.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = *x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        }

        let mut x = seed;
        let bytes: Vec<u8> = (0..8)
            .flat_map(|_| {
                let v = next(&mut x);
                v.to_le_bytes().map(|b| b % 95 + 32)
            })
            .collect();
        String::from_utf8(bytes).unwrap()
    }

    fn unique_edges(count: usize) -> Vec<ImportEdge> {
        (0..count)
            .map(|i| {
                let base = i as u64 * 2;
                edge(&random_name(base), &random_name(base + 1))
            })
            .collect()
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
        // 100 unique pseudo-random edges are less compressible than a redundant graph
        // and should agree with a high quality signal.
        let unique = unique_edges(100);
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
        let unique = unique_edges(100);
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
}
