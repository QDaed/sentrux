//! Color mapping functions for all ColorMode variants.
//!
//! Maps file attributes (language, git status, age, blast radius, churn)
//! to `Color32` values. Palette is desaturated for readability — colors
//! distinguish categories without competing with text labels or edges.

use crate::core::settings::ThemeConfig;
use egui::Color32;

/// Blast radius → red gradient. High blast = bright red (dangerous to change),
/// low blast = dim green (safe to change).
pub fn blast_radius_color(radius: u32, max_radius: u32, tc: &ThemeConfig) -> Color32 {
    if max_radius == 0 {
        return tc.diff_added; // all safe — uses theme's "added/green" semantic color
    }
    let t = (radius as f32 / max_radius as f32).min(1.0);
    // green(safe) → yellow → red(dangerous)
    // Derive gradient endpoints from theme semantic colors
    let [gr, gg, gb, _] = tc.diff_added.to_array();
    let [rr, rg, rb, _] = tc.status_error.to_array();
    let r = (gr as f32 + t * (rr as f32 - gr as f32)) as u8;
    let g = (gg as f32 + t * (rg as f32 - gg as f32)) as u8;
    let b = (gb as f32 + t * (rb as f32 - gb as f32)) as u8;
    Color32::from_rgb(r, g, b)
}

/// Language → color from plugin profile.
/// Each plugin declares its color in plugin.toml: `color_rgb = [65, 105, 145]`.
/// Languages without plugins (json, toml, yaml, etc.) get default gray.
pub fn language_color(lang: &str) -> Color32 {
    let rgb = crate::analysis::lang_registry::profile(lang).color_rgb;
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Git status → color from ThemeConfig semantic palette.
pub fn git_color(gs: &str, tc: &ThemeConfig) -> Color32 {
    match gs {
        "A" => tc.diff_added,                           // green — new/added
        "M" => tc.diff_modified,                        // blue — modified
        "MM" => tc.diff_modified.linear_multiply(1.15), // slightly brighter — staged+working
        "D" => tc.diff_removed,                         // red — deleted
        "R" => tc.status_warning,                       // amber — renamed
        "?" => tc.diff_added,                           // green — untracked (new to git)
        _ => tc.text_muted,                             // fallback — muted
    }
}

/// Exec depth → blue gradient. Depth 0 (entry points) = bright/prominent,
/// deeper dependencies = dimmer. Inverted t so shallow = visually important.
pub fn exec_depth_color(depth: u32) -> Color32 {
    let t = 1.0 - (depth as f32 / 8.0).min(1.0); // invert: 0=bright, 8+=dim
    let r = (40.0 + t * 60.0) as u8;
    let g = (60.0 + t * 100.0) as u8;
    let b = (180.0 + t * 75.0) as u8;
    Color32::from_rgb(r, g, b)
}
