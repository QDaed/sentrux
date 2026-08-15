//! Git status integration — retrieves per-file git status with TTL caching.
//!
//! Uses `git2` (libgit2 bindings) for efficient status queries. Results are
//! cached for 2 seconds to avoid expensive git operations on every frame.
//! Supports both index and workdir status detection.
//!
//! Cache invalidation: TTL-based (2 seconds). This balances freshness against
//! cost — git2 status queries touch every tracked file in the working directory.
//! The cache is cleared entirely on directory switch to prevent cross-project
//! status leakage.

use dashmap::DashMap;
use git2::{Repository, StatusOptions, StatusShow};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

/// Cached git statuses: root_path -> (timestamp, statuses)
/// TTL-based invalidation: re-fetch if older than 2 seconds
/// Cached git status entry: (timestamp, file_path -> status_string).
type GitStatusEntry = (Instant, HashMap<String, String>);

static STATUS_CACHE: LazyLock<DashMap<String, GitStatusEntry>> = LazyLock::new(DashMap::new);

const STATUS_CACHE_TTL_MS: u128 = 2000;

/// Clear all cached git statuses — called on directory switch to prevent
/// stale entries from a previous project persisting. [ref:93cf32d4]
pub fn clear_cache() {
    STATUS_CACHE.clear();
}

/// Get git status for all files in a repo. Returns map of relative_path -> status_string.
/// Results are cached with a 2-second TTL to avoid repeated expensive git operations.
pub fn get_statuses(root: &str) -> HashMap<String, String> {
    // Check cache first
    if let Some(cached) = STATUS_CACHE.get(root) {
        if cached.0.elapsed().as_millis() < STATUS_CACHE_TTL_MS {
            return cached.1.clone();
        }
    }

    let result = fetch_statuses(root);

    // Evict stale entries (older than 60s) to prevent unbounded memory growth
    const MAX_CACHE_AGE_MS: u128 = 60_000;
    STATUS_CACHE.retain(|_, v| v.0.elapsed().as_millis() < MAX_CACHE_AGE_MS);

    // Always cache the result (including empty = clean repo). Previously only
    // non-empty results were cached, causing clean repos to trigger a full
    // `git status` on every call instead of using the 2-second TTL cache.
    // Note: fetch_statuses returns empty HashMap for BOTH clean repos and git
    // failures, but failures already log via eprintln and the 2s TTL limits
    // retry frequency regardless.
    STATUS_CACHE.insert(root.to_string(), (Instant::now(), result.clone()));

    result
}

/// Check if the status represents a new/added file.
fn is_new(status: git2::Status) -> bool {
    status.is_index_new() || status.is_wt_new()
}

/// Check if the status represents a deleted file.
fn is_deleted(status: git2::Status) -> bool {
    status.is_index_deleted() || status.is_wt_deleted()
}

/// Check if the status represents a renamed file.
fn is_renamed(status: git2::Status) -> bool {
    status.is_index_renamed() || status.is_wt_renamed()
}

/// Map a git2 Status bitflags to a short string code.
/// Returns None for ignored entries (caller should skip).
fn status_to_code(status: git2::Status) -> Option<&'static str> {
    if is_new(status) {
        Some("A")
    } else if is_deleted(status) {
        Some("D")
    } else if is_renamed(status) {
        Some("R")
    } else if status.is_index_modified() && status.is_wt_modified() {
        Some("MM")
    } else if status.is_index_modified() || status.is_wt_modified() {
        Some("M")
    } else if status.is_ignored() {
        None
    } else {
        Some("?")
    }
}

fn fetch_statuses(root: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();

    let repo = match Repository::discover(root) {
        Ok(r) => r,
        Err(e) => {
            crate::debug_log!("[sentrux:git] fetch_statuses discover failed: {}", e);
            return result;
        }
    };

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show(StatusShow::IndexAndWorkdir);

    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(e) => {
            crate::debug_log!("[sentrux:git] fetch_statuses statuses failed: {}", e);
            return result;
        }
    };

    // Bare repos have no workdir — return empty result immediately
    let workdir = match repo.workdir() {
        Some(w) => w,
        None => return result, // bare repo: no working directory
    };
    let root_path = Path::new(root);

    collect_status_entries(&statuses, workdir, root_path, &mut result);

    result
}

/// Iterate git status entries, map each to a code, and insert scan-root-relative paths.
fn collect_status_entries(
    statuses: &git2::Statuses<'_>,
    workdir: &Path,
    root_path: &Path,
    result: &mut HashMap<String, String>,
) {
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            let code = match status_to_code(entry.status()) {
                Some(c) => c,
                None => continue,
            };

            let full = workdir.join(path);
            // Convert to scan-root-relative path. If strip_prefix fails
            // (scan root is outside git workdir), skip this entry rather
            // than inserting a workdir-relative path that won't match
            // any file in the scan. [ref:93cf32d4]
            if let Ok(rel) = full.strip_prefix(root_path) {
                result.insert(rel.to_string_lossy().to_string(), code.to_string());
            }
        }
    }
}
