//! Git evolution metrics — code churn, change coupling, temporal hotspots, code age, bus factor.
//!
//! All data comes from `git2` log walking — no shell-out to `git`.
//! Designed for parallel extraction where possible via rayon.
//!
//! Theory:
//! - Code churn × complexity = risk (Nagappan & Ball 2005)
//! - Change coupling = logical coupling (Gall et al. 1998)
//! - Bus factor = knowledge distribution (Ricca et al. 2011)
//! - Code age + churn = decay indicator

use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub mod git_walker;
#[cfg(test)]
mod tests;

use self::git_walker::{epoch_now, walk_git_log, CommitRecord};

// ── Named constants ──

/// Default lookback window in days for churn/coupling analysis.
const DEFAULT_LOOKBACK_DAYS: u32 = 90;

/// Minimum co-change count to report a coupling pair.
const MIN_COUPLING_COUNT: u32 = 3;

// Thresholds removed — continuous [0,1] scores replace letter grades.

// ── Trait: EvolutionProvider ──

/// Interface for retrieving evolution metrics from a repository.
///
/// Abstracts the concrete git-based implementation so that:
/// - Tests can inject synthetic evolution data without a real git repo
/// - Alternative VCS backends (e.g., Mercurial, Perforce) could implement this
/// - Cached/snapshot-based providers can skip expensive git walks
pub trait EvolutionProvider {
    /// Compute the full evolution report for a repository.
    fn evolution_report(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        complexity_map: &HashMap<String, u32>,
        lookback_days: Option<u32>,
    ) -> Result<EvolutionReport, String>;

    /// Compute per-file churn only (subset of full report).
    fn churn(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        lookback_days: Option<u32>,
    ) -> Result<HashMap<String, FileChurn>, String>;

    /// Compute change coupling pairs only.
    fn coupling(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        lookback_days: Option<u32>,
    ) -> Result<Vec<CouplingPair>, String>;
}

/// Default implementation backed by git2 log walking.
pub struct GitEvolutionProvider;

impl EvolutionProvider for GitEvolutionProvider {
    fn evolution_report(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        complexity_map: &HashMap<String, u32>,
        lookback_days: Option<u32>,
    ) -> Result<EvolutionReport, String> {
        compute_evolution(root, known_files, complexity_map, lookback_days)
    }

    fn churn(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        lookback_days: Option<u32>,
    ) -> Result<HashMap<String, FileChurn>, String> {
        let days = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);
        let records = walk_git_log(root, days)?;
        Ok(compute_churn(&records, known_files))
    }

    fn coupling(
        &self,
        root: &std::path::Path,
        known_files: &HashSet<String>,
        lookback_days: Option<u32>,
    ) -> Result<Vec<CouplingPair>, String> {
        let days = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);
        let records = walk_git_log(root, days)?;
        Ok(compute_coupling(&records, known_files))
    }
}

// ── Public types ──

/// Complete evolution report for a repository.
#[derive(Debug, Clone)]
pub struct EvolutionReport {
    /// Per-file churn (commit count + lines added/removed).
    pub churn: HashMap<String, FileChurn>,

    /// Change coupling: file pairs that frequently co-change.
    pub coupling_pairs: Vec<CouplingPair>,

    /// Temporal hotspots: churn × complexity risk score.
    pub hotspots: Vec<TemporalHotspot>,

    /// Per-file code age (days since last commit touching that file).
    pub code_age: HashMap<String, u32>,

    /// Per-file author count (bus factor).
    pub authors: HashMap<String, AuthorInfo>,

    /// Ratio of files with only 1 author.
    pub single_author_ratio: f64,

    /// Bus factor score [0,1]: 1.0 = no single-author files, 0.0 = all single-author.
    pub bus_factor_score: f64,

    /// Churn concentration score [0,1]: 1.0 = uniform churn, 0.0 = all churn in top 10%.
    pub churn_score: f64,

    /// Overall evolution score [0,1]: min of sub-scores.
    pub evolution_score: f64,

    /// Analysis window in days.
    pub lookback_days: u32,

    /// Total commits analyzed.
    pub commits_analyzed: u32,
}

/// Per-file churn statistics over the lookback window.
#[derive(Debug, Clone)]
pub struct FileChurn {
    /// Number of commits that touched this file
    pub commit_count: u32,
    /// Total lines added across all commits
    pub lines_added: u32,
    /// Total lines removed across all commits
    pub lines_removed: u32,
    /// Total churn: lines_added + lines_removed
    pub total_churn: u32,
}

/// A pair of files that frequently co-change (logical coupling).
#[derive(Debug, Clone)]
pub struct CouplingPair {
    /// First file in the pair
    pub file_a: String,
    /// Second file in the pair
    pub file_b: String,
    /// Number of commits where both files changed together
    pub co_change_count: u32,
    /// Jaccard similarity: co_changes / (changes_a union changes_b)
    pub coupling_strength: f64,
}

/// Temporal hotspot: file with high churn AND high complexity (highest risk).
#[derive(Debug, Clone)]
pub struct TemporalHotspot {
    /// File path
    pub file: String,
    /// Number of commits touching this file
    pub churn_count: u32,
    /// Maximum cyclomatic complexity of any function in this file
    pub max_complexity: u32,
    /// Risk score: churn_count * max_complexity (higher = more dangerous)
    pub risk_score: u64,
}

/// Per-file author information for bus factor analysis.
#[derive(Debug, Clone)]
pub struct AuthorInfo {
    /// All authors who committed to this file (sorted by commit count)
    pub authors: Vec<String>,
    /// Number of distinct authors
    pub author_count: u32,
    /// Author with the most commits to this file
    pub primary_author: String,
    /// Fraction of commits by the primary author (1.0 = single author)
    pub primary_ratio: f64,
}

// ── Public API ──

/// Compute evolution metrics for a repository.
///
/// `root`: path to the repository (or subdirectory — git2 discovers the repo).
/// `known_files`: set of files from the scan snapshot, to filter out deleted/renamed files.
/// `complexity_map`: file → max cyclomatic complexity (from HealthReport), for temporal hotspots.
/// `lookback_days`: how far back to analyze (default 90 if None).
pub fn compute_evolution(
    root: &Path,
    known_files: &HashSet<String>,
    complexity_map: &HashMap<String, u32>,
    lookback_days: Option<u32>,
) -> Result<EvolutionReport, String> {
    let days = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);

    // Step 1: Walk git log, collect per-commit records
    let records = walk_git_log(root, days)?;
    let commits_analyzed = records.len() as u32;

    if records.is_empty() {
        return Ok(empty_report(days));
    }

    // Step 2: Compute per-file churn (parallel aggregation)
    let churn = compute_churn(&records, known_files);

    // Step 3: Compute change coupling
    let coupling_pairs = compute_coupling(&records, known_files);

    // Step 4: Compute code age
    let now_epoch = epoch_now();
    let code_age = compute_code_age(&records, known_files, now_epoch);

    // Step 5: Compute bus factor (author distribution)
    let (authors, single_author_ratio) = compute_authors(&records, known_files);

    // Step 6: Compute temporal hotspots (churn × complexity)
    let hotspots = compute_hotspots(&churn, complexity_map);

    // Step 7: Score
    let bus_factor_score = score_bus_factor(single_author_ratio);
    let churn_score = score_churn_concentration(&churn);
    let evolution_score = bus_factor_score.min(churn_score);

    Ok(EvolutionReport {
        churn,
        coupling_pairs,
        hotspots,
        code_age,
        authors,
        single_author_ratio,
        bus_factor_score,
        churn_score,
        evolution_score,
        lookback_days: days,
        commits_analyzed,
    })
}

// ── Churn computation ──

pub(crate) fn compute_churn(
    records: &[CommitRecord],
    known_files: &HashSet<String>,
) -> HashMap<String, FileChurn> {
    let mut churn: HashMap<String, FileChurn> = HashMap::new();

    for record in records {
        for file in &record.files {
            if !known_files.contains(&file.path) {
                continue;
            }
            let entry = churn.entry(file.path.clone()).or_insert(FileChurn {
                commit_count: 0,
                lines_added: 0,
                lines_removed: 0,
                total_churn: 0,
            });
            entry.commit_count += 1;
            entry.lines_added += file.added;
            entry.lines_removed += file.removed;
            // Use saturating_add to prevent u32 overflow on high-churn files. [M17 fix]
            entry.total_churn = entry.lines_added.saturating_add(entry.lines_removed);
        }
    }

    churn
}

// ── Change coupling ──

pub(crate) fn compute_coupling(
    records: &[CommitRecord],
    known_files: &HashSet<String>,
) -> Vec<CouplingPair> {
    let (file_commit_count, pair_count) = aggregate_co_changes(records, known_files);

    let mut pairs: Vec<CouplingPair> = pair_count
        .into_par_iter()
        .filter_map(|((a, b), count)| {
            if count < MIN_COUPLING_COUNT {
                return None;
            }
            let count_a = file_commit_count.get(&a).copied().unwrap_or(1);
            let count_b = file_commit_count.get(&b).copied().unwrap_or(1);
            let union = count_a + count_b - count;
            let strength = if union > 0 {
                count as f64 / union as f64
            } else {
                0.0
            };
            Some(CouplingPair {
                file_a: a,
                file_b: b,
                co_change_count: count,
                coupling_strength: strength,
            })
        })
        .collect();

    // Sort by coupling_strength descending, with deterministic tiebreaker by file names.
    // Previously non-deterministic due to HashMap iteration order + par_iter.
    pairs.sort_by(|a, b| {
        b.coupling_strength
            .partial_cmp(&a.coupling_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file_a.cmp(&b.file_a))
            .then_with(|| a.file_b.cmp(&b.file_b))
    });
    pairs.truncate(50);
    pairs
}

/// Maximum files per commit for pair counting — prevents O(N²) blowup on huge commits.
const MAX_FILES_PER_COMMIT: usize = 50;

/// Count all pairs of co-changed files within a commit's file list.
/// Caps at MAX_FILES_PER_COMMIT to bound quadratic pair generation.
fn count_file_pairs(files: &[&str], pair_count: &mut HashMap<(String, String), u32>) {
    let capped = &files[..files.len().min(MAX_FILES_PER_COMMIT)];
    for i in 0..capped.len() {
        for j in (i + 1)..capped.len() {
            let (a, b) = if capped[i] < capped[j] {
                (capped[i].to_string(), capped[j].to_string())
            } else {
                (capped[j].to_string(), capped[i].to_string())
            };
            *pair_count.entry((a, b)).or_default() += 1;
        }
    }
}

fn aggregate_co_changes(
    records: &[CommitRecord],
    known_files: &HashSet<String>,
) -> (HashMap<String, u32>, HashMap<(String, String), u32>) {
    let mut file_commit_count: HashMap<String, u32> = HashMap::new();
    let mut pair_count: HashMap<(String, String), u32> = HashMap::new();

    for record in records {
        let files: Vec<&str> = record
            .files
            .iter()
            .filter(|f| known_files.contains(&f.path))
            .map(|f| f.path.as_str())
            .collect();

        for &f in &files {
            *file_commit_count.entry(f.to_string()).or_default() += 1;
        }
        count_file_pairs(&files, &mut pair_count);
    }

    (file_commit_count, pair_count)
}

// ── Code age ──

pub(crate) fn compute_code_age(
    records: &[CommitRecord],
    known_files: &HashSet<String>,
    now_epoch: i64,
) -> HashMap<String, u32> {
    let mut last_modified: HashMap<String, i64> = HashMap::new();

    for record in records {
        for file in &record.files {
            if !known_files.contains(&file.path) {
                continue;
            }
            let entry = last_modified.entry(file.path.clone()).or_insert(0);
            if record.epoch > *entry {
                *entry = record.epoch;
            }
        }
    }

    last_modified
        .into_iter()
        .map(|(path, epoch)| {
            let age_days = ((now_epoch - epoch) / 86400).max(0) as u32;
            (path, age_days)
        })
        .collect()
}

// ── Bus factor (author distribution) ──

pub(crate) fn compute_authors(
    records: &[CommitRecord],
    known_files: &HashSet<String>,
) -> (HashMap<String, AuthorInfo>, f64) {
    let mut file_authors: HashMap<String, HashMap<String, u32>> = HashMap::new();

    for record in records {
        for file in &record.files {
            if !known_files.contains(&file.path) {
                continue;
            }
            *file_authors
                .entry(file.path.clone())
                .or_default()
                .entry(record.author.clone())
                .or_default() += 1;
        }
    }

    let total_files = file_authors.len();

    // Build author info and count single-author files in a single pass.
    // Previously used a side-effecting closure inside .map() which would be a
    // data race if changed to par_iter.
    let author_entries: Vec<(String, AuthorInfo, bool)> = file_authors
        .into_iter()
        .map(|(path, author_map)| {
            let (info, is_single) = build_author_info(author_map);
            (path, info, is_single)
        })
        .collect();

    let single_author_count = author_entries
        .iter()
        .filter(|(_, _, is_single)| *is_single)
        .count() as u32;
    let authors: HashMap<String, AuthorInfo> = author_entries
        .into_iter()
        .map(|(path, info, _)| (path, info))
        .collect();

    let single_ratio = if total_files > 0 {
        single_author_count as f64 / total_files as f64
    } else {
        0.0
    };

    (authors, single_ratio)
}

fn build_author_info(author_map: HashMap<String, u32>) -> (AuthorInfo, bool) {
    let total_commits: u32 = author_map.values().sum();
    let mut sorted: Vec<(String, u32)> = author_map.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    let primary = sorted
        .first()
        .map(|(n, _)| n.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let primary_count = sorted.first().map(|(_, c)| *c).unwrap_or(0);
    let author_count = sorted.len() as u32;

    let info = AuthorInfo {
        authors: sorted.iter().map(|(n, _)| n.clone()).collect(),
        author_count,
        primary_author: primary,
        primary_ratio: if total_commits > 0 {
            primary_count as f64 / total_commits as f64
        } else {
            1.0
        },
    };
    (info, author_count == 1)
}

// ── Temporal hotspots ──

pub(crate) fn compute_hotspots(
    churn: &HashMap<String, FileChurn>,
    complexity_map: &HashMap<String, u32>,
) -> Vec<TemporalHotspot> {
    let mut hotspots: Vec<TemporalHotspot> = churn
        .par_iter()
        .filter_map(|(path, fc)| {
            let cc = complexity_map.get(path).copied().unwrap_or(1);
            let risk = fc.commit_count as u64 * cc as u64;
            if risk == 0 {
                return None;
            }
            Some(TemporalHotspot {
                file: path.clone(),
                churn_count: fc.commit_count,
                max_complexity: cc,
                risk_score: risk,
            })
        })
        .collect();

    hotspots.sort_by_key(|b| std::cmp::Reverse(b.risk_score));
    hotspots.truncate(30);
    hotspots
}

// ── Scoring ──

/// Continuous bus factor score [0,1]. 1.0 = no single-author files, 0.0 = all single-author.
pub(crate) fn score_bus_factor(single_author_ratio: f64) -> f64 {
    (1.0 - single_author_ratio).clamp(0.0, 1.0)
}

/// Continuous churn concentration score [0,1]. 1.0 = uniform, 0.0 = all churn in top 10%.
pub(crate) fn score_churn_concentration(churn: &HashMap<String, FileChurn>) -> f64 {
    if churn.is_empty() {
        return 1.0;
    }

    let mut churns: Vec<u32> = churn.values().map(|c| c.total_churn).collect();
    churns.sort_unstable_by(|a, b| b.cmp(a));

    let total: u64 = churns.iter().map(|c| *c as u64).sum();
    if total == 0 {
        return 1.0;
    }

    let top_n = (churns.len() / 10).max(1);
    let top_churn: u64 = churns.iter().take(top_n).map(|c| *c as u64).sum();
    let concentration = top_churn as f64 / total as f64;

    (1.0 - concentration).clamp(0.0, 1.0)
}

fn empty_report(days: u32) -> EvolutionReport {
    EvolutionReport {
        churn: HashMap::new(),
        coupling_pairs: Vec::new(),
        hotspots: Vec::new(),
        code_age: HashMap::new(),
        authors: HashMap::new(),
        single_author_ratio: 0.0,
        bus_factor_score: 1.0,
        churn_score: 1.0,
        evolution_score: 1.0,
        lookback_days: days,
        commits_analyzed: 0,
    }
}
