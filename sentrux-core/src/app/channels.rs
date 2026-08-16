//! Inter-thread communication types for scanner and layout workers.
//!
//! All communication between the main UI thread and background workers
//! (scanner, layout) goes through typed channels using these messages.
//! Each message carries a generation counter for stale-result rejection.

use crate::core::settings::Settings;
use crate::core::snapshot::{ScanProgress, Snapshot};
use crate::layout::types::FocusMode;
use crate::layout::types::{LayoutMode, RenderData, ScaleMode, SizeMode};
use crate::metrics::arch::ArchReport;
use crate::metrics::evo::EvolutionReport;
use crate::metrics::rules::checks::RuleCheckResult;
use crate::metrics::testgap::TestGapReport;
use crate::metrics::HealthReport;
use std::collections::HashSet;
use std::sync::Arc;

/// Commands sent to the scanner thread.
/// Each command carries its own `gen` (generation counter) so the scanner
/// doesn't need a shared atomic — eliminates the send/bump race where
/// the scanner could load a stale generation between try_send and fetch_add
/// on the main thread. `[ref:13696c9c]`
pub enum ScanCommand {
    /// Full scan of a directory — walks filesystem, parses all files, builds graphs.
    FullScan {
        /// Absolute path to the directory to scan
        root: String,
        /// Resource limits for scanning and parsing
        limits: crate::app::scan_threads::ScanLimits,
        /// Generation counter for stale-result rejection
        gen: u64,
        /// Cancellation token — set to true when a newer scan is requested.
        /// Scanner checks this at each progress step and aborts if set.
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    /// Incremental rescan — re-parses only changed files, patches existing snapshot.
    Rescan {
        /// Absolute path to the directory root
        root: String,
        /// Relative paths of files that changed (from watcher)
        changed: Vec<String>,
        /// Previous snapshot to patch (graph rebuild uses old data for unchanged files)
        old_snap: Arc<Snapshot>,
        /// Resource limits for scanning and parsing
        limits: crate::app::scan_threads::ScanLimits,
        /// Generation counter for stale-result rejection
        gen: u64,
        /// Cancellation token
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
}

/// All reports computed on the scanner thread after scan completion.
/// Bundled into a struct to keep ScanMsg::Complete tidy as we add more analyses.
pub struct ScanReports {
    /// Code health report (coupling, complexity, dead code, etc.)
    pub health: Option<HealthReport>,
    /// Architecture report (layers, abstractness, distance from main sequence)
    pub arch: Option<ArchReport>,
    /// Git evolution report (churn, temporal coupling, bus factor)
    pub evolution: Option<EvolutionReport>,
    /// Test gap analysis (untested high-risk files)
    pub test_gaps: Option<TestGapReport>,
    /// Architecture rule check results
    pub rules: Option<RuleCheckResult>,
}

/// Messages from scanner thread → main thread.
/// TreeReady and Complete carry a generation counter so the main thread
/// can reject stale results from a previous scan (e.g., after rapid
/// directory switches). `[ref:93cf32d4]`
pub enum ScanMsg {
    /// Scan progress update (step name + percentage)
    Progress(ScanProgress),
    /// File tree ready (before graphs) — enables early rendering
    TreeReady(Arc<Snapshot>, u64),
    /// Scan fully complete with all analysis reports
    Complete(Arc<Snapshot>, u64, Box<ScanReports>),
    /// Scan failed with error message
    Error(String, u64),
}

/// Messages from main thread → layout thread.
/// Contains all data the layout engine needs to produce RenderData.
pub struct LayoutRequest {
    /// Current scan snapshot (shared, not cloned)
    pub snapshot: Arc<Snapshot>,
    /// What metric determines file block area
    pub size_mode: SizeMode,
    /// Scaling transform for size compression
    pub scale_mode: ScaleMode,
    /// Spatial arrangement algorithm
    pub layout_mode: LayoutMode,
    /// Available viewport width in screen pixels
    pub viewport_w: f64,
    /// Available viewport height in screen pixels
    pub viewport_h: f64,
    /// Current drill-down path prefix (empty = show everything)
    pub drill_path: Option<String>,
    /// Layout version at time of request — returned in LayoutMsg::Ready for matching.
    pub version: u64,
    /// BUG 1 fix: snapshot of heat values from HeatTracker for SizeMode::Heat layout.
    /// None when heat mode is not active (avoids cloning HashMap every frame).
    pub heat_map: Option<std::collections::HashMap<String, f64>>,
    /// User-tunable settings (cloned per request so layout thread reads consistent values)
    pub settings: Settings,
    /// Focus mode filter — controls which files are visible in layout
    pub focus_mode: FocusMode,
    /// Entry-point file paths for FocusMode::EntryPoints filtering.
    /// Wrapped in Arc so cloning from AppState is O(1) atomic increment.
    pub entry_point_files: Arc<HashSet<String>>,
    /// User-hidden paths (files or directory prefixes) — weight 0 in layout.
    /// Wrapped in Arc so cloning from AppState is O(1) atomic increment.
    pub hidden_paths: Arc<HashSet<String>>,
    /// Pre-computed impact files for ImpactRadius focus mode (transitive dependents).
    /// Wrapped in Arc so cloning from AppState is O(1) atomic increment.
    pub impact_files: Option<Arc<HashSet<String>>>,
}

/// Messages from the layout thread to the main UI thread.
pub enum LayoutMsg {
    /// Layout computation complete: render data + version for stale-result rejection
    Ready(RenderData, u64),
}
