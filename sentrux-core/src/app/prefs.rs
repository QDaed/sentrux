//! User preferences that persist across app restarts.
//!
//! Saved/loaded via eframe's built-in storage (ron format). Captures the
//! user's layout mode, scale mode, size mode, color mode, theme, edge filter,
//! and panel visibility toggles. Automatically serialized on shutdown and
//! restored on launch so the UI remembers its last configuration.
//! Key type: `UserPrefs` (serializable subset of `AppState`).

use crate::core::settings::Theme;
use crate::layout::types::ColorMode;
use crate::layout::types::EdgeFilter;
use crate::layout::types::{LayoutMode, ScaleMode, SizeMode};
use serde::{Deserialize, Serialize};

const PREFS_KEY: &str = "sentrux_user_prefs";

/// Serializable subset of AppState that persists across app restarts.
/// Stored in eframe's built-in ron-format storage.
#[derive(Serialize, Deserialize)]
pub struct UserPrefs {
    pub theme: Theme,
    pub color_mode: ColorMode,
    pub size_mode: SizeMode,
    pub scale_mode: ScaleMode,
    pub layout_mode: LayoutMode,
    pub edge_filter: EdgeFilter,
    pub show_all_edges: bool,
    pub activity_panel_open: bool,
    pub last_root_path: Option<String>,
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            theme: Theme::Calm,
            color_mode: ColorMode::Monochrome,
            size_mode: SizeMode::Lines,
            scale_mode: ScaleMode::Smooth,
            layout_mode: LayoutMode::Treemap,
            edge_filter: EdgeFilter::All,
            show_all_edges: false,
            activity_panel_open: false,
            last_root_path: None,
        }
    }
}

impl UserPrefs {
    /// Load from eframe storage, falling back to defaults.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        eframe::get_value(storage, PREFS_KEY).unwrap_or_default()
    }

    /// Save to eframe storage.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, PREFS_KEY, self);
    }

    /// Snapshot current app state into prefs.
    pub fn from_state(state: &crate::app::state::AppState) -> Self {
        Self {
            theme: state.theme,
            color_mode: state.color_mode,
            size_mode: state.size_mode,
            scale_mode: state.scale_mode,
            layout_mode: state.layout_mode,
            edge_filter: state.edge_filter,
            show_all_edges: state.show_all_edges,
            activity_panel_open: state.activity_panel_open,
            last_root_path: state.root_path.clone(),
        }
    }

    /// Apply saved prefs to app state.
    pub fn apply_to(&self, state: &mut crate::app::state::AppState) {
        state.theme = self.theme;
        state.theme_config = crate::core::settings::ThemeConfig::from_theme(self.theme);
        state.color_mode = self.color_mode;
        state.size_mode = self.size_mode;
        state.scale_mode = self.scale_mode;
        state.layout_mode = self.layout_mode;
        state.edge_filter = self.edge_filter;
        state.show_all_edges = self.show_all_edges;
        state.activity_panel_open = self.activity_panel_open;
        if self.last_root_path.is_some() {
            state.root_path = self.last_root_path.clone();
        }
    }
}
