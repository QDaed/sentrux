//! Pro plugin registry — runtime extension point for Pro features.
//!
//! The free binary ships with basic functionality. The Pro dylib
//! registers additional capabilities at startup via this registry.
//! All access is through the global REGISTRY singleton.
//!
//! ## Architecture
//!
//! The registry stores function pointers and data registered by pro.dylib.
//! The free binary checks `pro_registry::has(Feature)` before showing
//! Pro-gated UI or returning Pro-gated MCP data.
//!
//! This replaces the old `tier.is_pro()` pattern — Pro features are
//! no longer gated by a flag in the free binary. They're either
//! registered (dylib loaded) or not.

use std::sync::Mutex;

/// Global Pro plugin registry.
static REGISTRY: Mutex<ProRegistry> = Mutex::new(ProRegistry::new());

/// Feature capabilities that Pro can register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProFeature {
    /// Extra color modes (Age, Churn, Risk, Git, ExecDepth, BlastRadius)
    ExtraColorModes,
    /// Full file detail panel (per-function metrics, imports, dependents)
    FileDetailPanel,
    /// Full evolution display (hotspots, coupling, bus factor details)
    EvolutionDetails,
    /// What-if analysis panel
    WhatIfAnalysis,
    /// Root-cause-organized diagnostics in MCP health tool
    McpDiagnostics,
    /// Unlimited rules in MCP check_rules
    UnlimitedRules,
}

/// Registry state — tracks which Pro features are available.
struct ProRegistry {
    features: u32, // bitfield of registered features
    plugin_name: Option<String>,
    plugin_version: Option<String>,
}

impl ProRegistry {
    const fn new() -> Self {
        Self {
            features: 0,
            plugin_name: None,
            plugin_version: None,
        }
    }
}

// ── Public API (called by free binary to CHECK features) ──

/// Check if a Pro feature is available (registered by loaded dylib).
pub fn has(feature: ProFeature) -> bool {
    REGISTRY
        .lock()
        .is_ok_and(|reg| reg.features & (1 << feature as u32) != 0)
}

/// Check if ANY Pro plugin is loaded.
pub fn is_loaded() -> bool {
    REGISTRY.lock().is_ok_and(|reg| reg.features != 0)
}

/// Get the loaded Pro plugin name and version.
pub fn plugin_info() -> Option<(String, String)> {
    match REGISTRY.lock() {
        Ok(reg) => {
            let name = reg.plugin_name.clone()?;
            let version = reg.plugin_version.clone()?;
            Some((name, version))
        }
        Err(_) => None,
    }
}

// ── Registration API (called by pro.dylib to REGISTER features) ──

/// Register a Pro feature as available. Called by pro.dylib during init.
pub fn register(feature: ProFeature) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.features |= 1 << feature as u32;
    }
}

/// Register all standard Pro features at once.
pub fn register_all_features() {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.features = (1 << ProFeature::ExtraColorModes as u32)
            | (1 << ProFeature::FileDetailPanel as u32)
            | (1 << ProFeature::EvolutionDetails as u32)
            | (1 << ProFeature::WhatIfAnalysis as u32)
            | (1 << ProFeature::McpDiagnostics as u32)
            | (1 << ProFeature::UnlimitedRules as u32);
    }
}

/// Set the Pro plugin metadata. Called by pro.dylib during init.
pub fn set_plugin_info(name: &str, version: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.plugin_name = Some(name.to_string());
        reg.plugin_version = Some(version.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_no_features() {
        // In a fresh process, no features are registered
        // (can't test reliably since tests share process, but verify the API compiles)
        let _ = has(ProFeature::ExtraColorModes);
        let _ = is_loaded();
        let _ = plugin_info();
    }

    #[test]
    fn register_and_check() {
        register(ProFeature::McpDiagnostics);
        assert!(has(ProFeature::McpDiagnostics));
    }

    #[test]
    fn register_all() {
        register_all_features();
        assert!(has(ProFeature::ExtraColorModes));
        assert!(has(ProFeature::FileDetailPanel));
        assert!(has(ProFeature::EvolutionDetails));
        assert!(has(ProFeature::WhatIfAnalysis));
        assert!(has(ProFeature::McpDiagnostics));
        assert!(has(ProFeature::UnlimitedRules));
        assert!(is_loaded());
    }

    #[test]
    fn plugin_info_roundtrip() {
        set_plugin_info("sentrux-pro", "0.5.7");
        let info = plugin_info();
        assert!(info.is_some());
        let (name, version) = info.unwrap();
        assert_eq!(name, "sentrux-pro");
        assert_eq!(version, "0.5.7");
    }
}
