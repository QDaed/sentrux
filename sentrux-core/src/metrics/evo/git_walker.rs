//! Git log walking and commit parsing for evolution metrics.
//!
//! Extracts per-commit records from a repository using `git2` —
//! no shell-out to `git`. Designed for efficient sequential walking
//! with early cutoff by date.

use git2::{Repository, Sort};
use std::path::Path;

/// Maximum files per commit to consider (skip mega-merges that add noise).
const MAX_FILES_PER_COMMIT: usize = 50;

// ── Public types ──

/// Per-commit record: which files changed, who authored it, when.
pub(crate) struct CommitRecord {
    pub author: String,
    pub epoch: i64,
    pub files: Vec<CommitFile>,
}

pub(crate) struct CommitFile {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

// ── Public API ──

/// Walk the git log from HEAD back `lookback_days` days, collecting per-commit records.
///
/// Skips merge commits and mega-commits (> 50 files). Returns records in
/// reverse chronological order.
pub(crate) fn walk_git_log(root: &Path, lookback_days: u32) -> Result<Vec<CommitRecord>, String> {
    let repo = Repository::discover(root).map_err(|e| format!("Git discover failed: {e}"))?;
    let workdir = repo
        .workdir()
        .ok_or("Bare repository — no working directory")?;

    let cutoff = epoch_now() - (lookback_days as i64 * 86400);

    let mut revwalk = repo.revwalk().map_err(|e| format!("Revwalk failed: {e}"))?;
    revwalk
        .set_sorting(Sort::TIME)
        .map_err(|e| format!("Sort failed: {e}"))?;
    revwalk
        .push_head()
        .map_err(|e| format!("Push HEAD failed: {e}"))?;

    let prefix = scan_root_prefix(root, workdir);
    let (records, skip_counts) = collect_commits(&repo, revwalk, cutoff, &prefix);

    let total_skipped = skip_counts.oid + skip_counts.commit + skip_counts.parse;
    if total_skipped > 0 {
        eprintln!(
            "[evolution] walked {} commits, skipped {} (oid_err={}, commit_err={}, unparseable={})",
            records.len() + total_skipped as usize,
            total_skipped,
            skip_counts.oid,
            skip_counts.commit,
            skip_counts.parse
        );
    }

    Ok(records)
}

/// Counts of skipped commits by reason.
struct SkipCounts {
    oid: u32,
    commit: u32,
    parse: u32,
}

/// Walk the revwalk iterator, collecting commit records and counting skips.
fn collect_commits(
    repo: &Repository,
    revwalk: git2::Revwalk<'_>,
    cutoff: i64,
    prefix: &str,
) -> (Vec<CommitRecord>, SkipCounts) {
    let mut records = Vec::new();
    let mut skips = SkipCounts {
        oid: 0,
        commit: 0,
        parse: 0,
    };

    for oid_result in revwalk {
        let oid = match oid_result {
            Ok(o) => o,
            Err(_) => {
                skips.oid += 1;
                continue;
            }
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => {
                skips.commit += 1;
                continue;
            }
        };
        if commit.time().seconds() < cutoff {
            break;
        }
        match parse_commit(repo, &commit, prefix) {
            Some(record) => records.push(record),
            None => {
                skips.parse += 1;
            }
        }
    }

    (records, skips)
}

// ── Internal helpers ──

pub(crate) fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Determine the scan root relative to the workdir, for path filtering.
fn scan_root_prefix(root: &Path, workdir: &Path) -> String {
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workdir_canonical = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    root_canonical
        .strip_prefix(&workdir_canonical)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .to_string()
}

/// Parse a single commit into a CommitRecord, returning None for merge commits,
/// mega-commits, or commits that produce no relevant files.
fn parse_commit(
    repo: &Repository,
    commit: &git2::Commit<'_>,
    prefix: &str,
) -> Option<CommitRecord> {
    // Skip merge commits — they double-count changes.
    if commit.parent_count() > 1 {
        return None;
    }

    let author = commit.author().name().unwrap_or("unknown").to_string();
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(e) => {
            crate::debug_log!("[evolution] commit {}: tree() failed: {}", commit.id(), e);
            return None;
        }
    };
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) {
        Ok(d) => d,
        Err(e) => {
            crate::debug_log!("[evolution] commit {}: diff failed: {}", commit.id(), e);
            return None;
        }
    };

    if let Err(e) = diff.stats() {
        crate::debug_log!(
            "[evolution] commit {}: diff.stats() failed: {}",
            commit.id(),
            e
        );
        return None;
    }

    let num_deltas = diff.deltas().len();
    if num_deltas > MAX_FILES_PER_COMMIT {
        return None;
    }

    let files = collect_diff_files(&diff, prefix);
    if files.is_empty() {
        return None;
    }

    Some(CommitRecord {
        author,
        epoch: commit.time().seconds(),
        files,
    })
}

/// Collect changed files from a diff, filtering to the scan root prefix.
fn collect_diff_files(diff: &git2::Diff<'_>, prefix: &str) -> Vec<CommitFile> {
    let mut files = Vec::new();
    for (i, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string());
        let path = match path {
            Some(p) => p,
            None => continue,
        };
        let rel_path = if prefix.is_empty() {
            path
        } else if let Some(stripped) = path.strip_prefix(&format!("{prefix}/")) {
            stripped.to_string()
        } else {
            continue;
        };
        let (added, removed) = get_patch_stats(diff, i);
        files.push(CommitFile {
            path: rel_path,
            added,
            removed,
        });
    }
    files
}

/// Extract added/removed line counts from a diff patch for a specific delta index.
fn get_patch_stats(diff: &git2::Diff, delta_idx: usize) -> (u32, u32) {
    let mut added = 0u32;
    let mut removed = 0u32;

    if let Ok(Some(patch)) = git2::Patch::from_diff(diff, delta_idx) {
        let (_, a, r) = patch.line_stats().unwrap_or((0, 0, 0));
        added = a as u32;
        removed = r as u32;
    }

    (added, removed)
}
