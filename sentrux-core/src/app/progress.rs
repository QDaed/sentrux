//! Scan progress overlay — shows scanning status with a progress bar.
//!
//! Renders either a compact corner overlay (during incremental rescan) or
//! a centered overlay (during initial full scan). All colors from ThemeConfig.

use super::state::AppState;

/// Color palette for progress bar rendering — bundles the 4 color params
/// that both draw_compact_content and draw_centered_content need.
/// Reduces draw_compact_content: 7->4, draw_centered_content: 8->4.
struct ProgressColors {
    text_color: egui::Color32,
    text_muted: egui::Color32,
    bar_bg: egui::Color32,
    bar_fill: egui::Color32,
}

/// Draw scan progress overlay — terminal pixel style.
/// All colors from ThemeConfig. No hardcoded values.
pub fn draw_progress_overlay(ui: &mut egui::Ui, state: &AppState, compact: bool) {
    if !state.scanning {
        return;
    }

    let tc = &state.theme_config;
    let available = ui.available_rect_before_wrap();

    let (overlay_w, overlay_h, origin) = if compact {
        let w = 240.0_f32;
        let h = 56.0_f32;
        let margin = 12.0_f32;
        let x = available.right() - w - margin;
        let y = available.bottom() - h - margin;
        (w, h, egui::pos2(x, y))
    } else {
        let w = 300.0_f32;
        let h = 80.0_f32;
        let x = available.center().x - w / 2.0;
        let y = available.center().y - h / 2.0;
        (w, h, egui::pos2(x, y))
    };

    let overlay_rect = egui::Rect::from_min_size(origin, egui::vec2(overlay_w, overlay_h));
    let painter = ui.painter();

    // Background + border from theme
    painter.rect_filled(overlay_rect, egui::CornerRadius::ZERO, tc.header_strip_bg);
    painter.rect_stroke(
        overlay_rect,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0_f32, tc.section_border),
        egui::StrokeKind::Middle,
    );

    let colors = ProgressColors {
        text_color: tc.text_primary,
        text_muted: tc.text_secondary,
        bar_bg: tc.file_surface,
        bar_fill: tc.selected_stroke,
    };

    if compact {
        draw_compact_content(painter, overlay_rect, overlay_w, state, &colors);
    } else {
        draw_centered_content(painter, overlay_rect, overlay_w, state, &colors);
    }
}

/// Compact corner overlay: single text line + thin progress bar.
fn draw_compact_content(
    painter: &egui::Painter,
    overlay_rect: egui::Rect,
    overlay_w: f32,
    state: &AppState,
    pc: &ProgressColors,
) {
    let text = format!("{} -- {}%", state.scan_step, state.scan_pct);
    painter.text(
        egui::pos2(overlay_rect.left() + 12.0, overlay_rect.top() + 16.0),
        egui::Align2::LEFT_CENTER,
        &text,
        egui::FontId::monospace(11.0),
        pc.text_color,
    );
    let bar_y = overlay_rect.top() + 34.0;
    let bar_w = overlay_w - 24.0;
    let bar_x = overlay_rect.left() + 12.0;
    let bar_rect = egui::Rect::from_min_size(egui::pos2(bar_x, bar_y), egui::vec2(bar_w, 8.0));
    painter.rect_filled(bar_rect, egui::CornerRadius::ZERO, pc.bar_bg);
    let fill_w = (bar_w * (state.scan_pct.min(100) as f32 / 100.0)).min(bar_w);
    if fill_w > 0.0 {
        let fill_rect =
            egui::Rect::from_min_size(egui::pos2(bar_x, bar_y), egui::vec2(fill_w, 8.0));
        painter.rect_filled(fill_rect, egui::CornerRadius::ZERO, pc.bar_fill);
    }
}

/// Centered overlay: title, thick progress bar, and percentage label.
fn draw_centered_content(
    painter: &egui::Painter,
    overlay_rect: egui::Rect,
    overlay_w: f32,
    state: &AppState,
    pc: &ProgressColors,
) {
    painter.text(
        egui::pos2(overlay_rect.center().x, overlay_rect.top() + 22.0),
        egui::Align2::CENTER_CENTER,
        &state.scan_step,
        egui::FontId::monospace(12.0),
        pc.text_color,
    );
    let bar_y = overlay_rect.top() + 45.0;
    let bar_w = overlay_w - 40.0;
    let bar_x = overlay_rect.left() + 20.0;
    let bar_rect = egui::Rect::from_min_size(egui::pos2(bar_x, bar_y), egui::vec2(bar_w, 12.0));
    painter.rect_filled(bar_rect, egui::CornerRadius::ZERO, pc.bar_bg);
    let fill_w = (bar_w * (state.scan_pct.min(100) as f32 / 100.0)).min(bar_w);
    if fill_w > 0.0 {
        let fill_rect =
            egui::Rect::from_min_size(egui::pos2(bar_x, bar_y), egui::vec2(fill_w, 12.0));
        painter.rect_filled(fill_rect, egui::CornerRadius::ZERO, pc.bar_fill);
    }
    painter.text(
        egui::pos2(overlay_rect.center().x, bar_y + 12.0 + 10.0),
        egui::Align2::CENTER_CENTER,
        format!("{}%", state.scan_pct),
        egui::FontId::monospace(10.0),
        pc.text_muted,
    );
}
