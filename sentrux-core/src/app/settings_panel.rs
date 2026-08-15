//! Settings panel — exposes all tunable parameters with sliders and inputs.
//!
//! Organized by category (edges, layout, viewport, animation, scanner).
//! Returns `(layout_changed, visual_changed)` so the caller can decide
//! whether to trigger a re-layout or just a repaint.

use crate::core::settings::Settings;

/// Draw the settings side panel. Returns (layout_changed, visual_changed).
pub fn draw_settings_panel(
    ctx: &egui::Context,
    settings: &mut Settings,
    open: &mut bool,
) -> (bool, bool) {
    let mut layout_changed = false;
    let mut visual_changed = false;

    egui::SidePanel::right("settings_panel")
        .default_width(280.0)
        .resizable(true)
        .show_animated(ctx, *open, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset All").clicked() {
                        settings.reset();
                        layout_changed = true;
                    }
                    if ui.button("Close").clicked() {
                        *open = false;
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                let (lc, vc) = draw_edge_sections(ui, settings);
                layout_changed |= lc;
                visual_changed |= vc;

                let (lc, vc) = draw_layout_sections(ui, settings);
                layout_changed |= lc;
                visual_changed |= vc;

                let (lc, vc) = draw_viewport_sections(ui, settings);
                layout_changed |= lc;
                visual_changed |= vc;

                let (lc, vc) = draw_misc_sections(ui, settings);
                layout_changed |= lc;
                visual_changed |= vc;

                draw_privacy_section(ui);
            });
        });

    (layout_changed, visual_changed)
}

/// Edge colors, rendering, and routing sections.
fn draw_edge_sections(ui: &mut egui::Ui, settings: &mut Settings) -> (bool, bool) {
    let mut lc = false;
    let mut vc = false;
    ui.collapsing("Edge Colors", |ui| {
        vc |= color_picker(ui, "Import", &mut settings.import_color);
        vc |= color_picker(ui, "Call", &mut settings.call_color);
        vc |= color_picker(ui, "Inherit", &mut settings.inherit_color);
    });
    ui.collapsing("Edge Rendering", |ui| {
        vc |= slider_f64(ui, "Alpha Base", &mut settings.edge_alpha_base, 0.1..=1.0);
        vc |= slider_f64(ui, "Alpha Max", &mut settings.edge_alpha_max, 0.1..=1.0);
        vc |= slider_f64(
            ui,
            "Line Width Base",
            &mut settings.edge_line_w_base,
            0.5..=5.0,
        );
        vc |= slider_f64(
            ui,
            "Line Width Max",
            &mut settings.edge_line_w_max,
            1.0..=10.0,
        );
        vc |= slider_f32(ui, "Dash Length", &mut settings.dash_len, 2.0..=20.0);
        vc |= slider_f32(ui, "Dash Gap", &mut settings.dash_gap, 1.0..=15.0);
        vc |= slider_f32(
            ui,
            "Dash Anim Speed",
            &mut settings.dash_anim_speed,
            5.0..=100.0,
        );
    });
    ui.collapsing("Edge Routing", |ui| {
        lc |= slider_f64(
            ui,
            "Min Edge Length",
            &mut settings.min_edge_len,
            0.0..=50.0,
        );
        lc |= slider_f64(ui, "Lane Width", &mut settings.lane_width, 1.0..=15.0);
        lc |= slider_f64(
            ui,
            "Border Padding",
            &mut settings.edge_border_pad,
            0.0..=10.0,
        );
    });
    (lc, vc)
}

/// Treemap and blueprint layout sections.
fn draw_layout_sections(ui: &mut egui::Ui, settings: &mut Settings) -> (bool, bool) {
    let mut lc = false;
    let vc = false;
    ui.collapsing("Layout: Treemap", |ui| {
        lc |= slider_f64(ui, "Dir Padding", &mut settings.treemap_dir_pad, 0.0..=20.0);
        lc |= slider_f64(
            ui,
            "Dir Header",
            &mut settings.treemap_dir_header,
            0.0..=40.0,
        );
        lc |= slider_f64(
            ui,
            "Min Rect Size",
            &mut settings.treemap_min_rect,
            1.0..=20.0,
        );
    });
    ui.collapsing("Layout: Blueprint", |ui| {
        lc |= slider_f64(
            ui,
            "Section Padding",
            &mut settings.blueprint_section_pad,
            0.0..=20.0,
        );
        lc |= slider_f64(
            ui,
            "Section Header",
            &mut settings.blueprint_section_header,
            0.0..=40.0,
        );
        lc |= slider_f64(
            ui,
            "Min Rect Size",
            &mut settings.blueprint_min_rect,
            1.0..=20.0,
        );
        lc |= slider_f64(
            ui,
            "Gutter Base",
            &mut settings.blueprint_gutter_base,
            0.0..=20.0,
        );
        lc |= slider_f64(
            ui,
            "Gutter Top",
            &mut settings.blueprint_gutter_top,
            0.0..=30.0,
        );
        lc |= slider_f64(
            ui,
            "Route Margin",
            &mut settings.blueprint_route_margin,
            0.0..=100.0,
        );
    });
    (lc, vc)
}

/// Font, viewport, and minimap sections.
fn draw_viewport_sections(ui: &mut egui::Ui, settings: &mut Settings) -> (bool, bool) {
    let mut lc = false;
    let mut vc = false;
    ui.collapsing("Font", |ui| {
        vc |= slider_f32(ui, "Font Scale", &mut settings.font_scale, 0.05..=0.40);
        vc |= slider_f32(ui, "UI Scale", &mut settings.ui_scale, 0.5..=3.0);
    });
    ui.collapsing("Viewport", |ui| {
        vc |= slider_f64(ui, "Zoom Min", &mut settings.zoom_min, 0.01..=1.0);
        vc |= slider_f64(ui, "Zoom Max", &mut settings.zoom_max, 5.0..=200.0);
        vc |= slider_f64(
            ui,
            "Scroll Zoom Factor",
            &mut settings.zoom_scroll_factor,
            1.01..=2.0,
        );
        lc |= slider_f64(
            ui,
            "Fit Content Padding",
            &mut settings.fit_content_padding,
            0.0..=100.0,
        );
    });
    ui.collapsing("Minimap", |ui| {
        vc |= slider_f32(ui, "Width", &mut settings.minimap_w, 60.0..=400.0);
        vc |= slider_f32(ui, "Height", &mut settings.minimap_h, 40.0..=300.0);
        vc |= slider_f32(ui, "Padding", &mut settings.minimap_pad, 0.0..=30.0);
    });
    (lc, vc)
}

/// Animation, rect rendering, graph analysis, squarify, scanner, and timing sections.
fn draw_misc_sections(ui: &mut egui::Ui, settings: &mut Settings) -> (bool, bool) {
    let mut lc = false;
    let mut vc = false;
    ui.collapsing("Animation / Heat", |ui| {
        vc |= slider_f64(
            ui,
            "Heat Half-Life (s)",
            &mut settings.heat_half_life,
            1.0..=30.0,
        );
        vc |= slider_f64(
            ui,
            "Ripple Duration (s)",
            &mut settings.ripple_duration,
            0.1..=3.0,
        );
        vc |= slider_f64(
            ui,
            "Trail Max Age (s)",
            &mut settings.trail_max_age,
            5.0..=120.0,
        );
        vc |= slider_f32(
            ui,
            "Trail Dot Radius",
            &mut settings.trail_dot_radius,
            1.0..=10.0,
        );
    });
    ui.collapsing("Rect Rendering", |ui| {
        vc |= slider_f32(
            ui,
            "File Inset (gap)",
            &mut settings.file_rect_inset,
            0.0..=5.0,
        );
    });
    ui.collapsing("Graph Analysis", |ui| {
        lc |= slider_usize(
            ui,
            "Max Call Targets",
            &mut settings.max_call_targets,
            1..=50,
        );
        lc |= slider_f64(
            ui,
            "Min Child Weight",
            &mut settings.min_child_weight,
            1.0..=20.0,
        );
    });
    ui.collapsing("Squarify / Chrome", |ui| {
        lc |= slider_f64(
            ui,
            "Squarify Min Rect",
            &mut settings.squarify_min_rect,
            1.0..=20.0,
        );
        lc |= slider_f64(
            ui,
            "Treemap Chrome Frac",
            &mut settings.treemap_max_chrome_frac,
            0.05..=0.5,
        );
        lc |= slider_f64(
            ui,
            "Blueprint Chrome Frac",
            &mut settings.blueprint_max_chrome_frac,
            0.05..=0.5,
        );
    });
    ui.collapsing("Scanner Limits", |ui| {
        vc |= slider_u64(
            ui,
            "Max File Size (KB)",
            &mut settings.max_file_size_kb,
            256..=8192,
        );
        vc |= slider_usize(
            ui,
            "Max Parse Size (KB)",
            &mut settings.max_parse_size_kb,
            128..=2048,
        );
    });
    ui.collapsing("Timing / Debounce", |ui| {
        vc |= slider_u64(
            ui,
            "File Change Debounce (ms)",
            &mut settings.file_change_debounce_ms,
            100..=2000,
        );
        vc |= slider_u64(
            ui,
            "Watcher Debounce (ms)",
            &mut settings.watcher_debounce_ms,
            50..=1000,
        );
        vc |= slider_u64(
            ui,
            "Heat Repaint (ms)",
            &mut settings.heat_repaint_ms,
            16..=200,
        );
    });
    (lc, vc)
}

/// Privacy section — telemetry opt-out toggle.
/// Reads/writes SENTRUX_NO_UPDATE_CHECK preference to ~/.sentrux/telemetry_opt_out.
fn draw_privacy_section(ui: &mut egui::Ui) {
    ui.collapsing("Privacy", |ui| {
        let opt_out_path = dirs::home_dir().map(|h| h.join(".sentrux").join("telemetry_opt_out"));

        let mut opted_out = opt_out_path.as_ref().is_some_and(|p| p.exists());

        ui.label("Anonymous usage statistics help improve sentrux.");
        ui.label("No code, file paths, or project data is ever sent.");
        ui.add_space(4.0);

        if ui
            .checkbox(&mut opted_out, "Disable anonymous usage stats")
            .changed()
        {
            if let Some(path) = &opt_out_path {
                if opted_out {
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let _ = std::fs::write(path, "1");
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("What's collected: version, platform, scan count, quality score")
                .weak()
                .small(),
        );
        ui.label(
            egui::RichText::new("What's NOT collected: code, file paths, project names")
                .weak()
                .small(),
        );
    });
}

fn slider_f64(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    let old = *val;
    ui.add(egui::Slider::new(val, range).text(label));
    (*val - old).abs() > f64::EPSILON
}

fn slider_f32(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    let old = *val;
    ui.add(egui::Slider::new(val, range).text(label));
    (*val - old).abs() > f32::EPSILON
}

fn slider_usize(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut usize,
    range: std::ops::RangeInclusive<usize>,
) -> bool {
    let old = *val;
    let mut v = *val as u32;
    ui.add(egui::Slider::new(&mut v, (*range.start() as u32)..=(*range.end() as u32)).text(label));
    *val = v as usize;
    *val != old
}

fn slider_u64(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut u64,
    range: std::ops::RangeInclusive<u64>,
) -> bool {
    // Bug fix: use f64 intermediary instead of truncating u64 → u32.
    // f64 can represent integers exactly up to 2^53.
    let old = *val;
    let mut v = *val as f64;
    ui.add(egui::Slider::new(&mut v, (*range.start() as f64)..=(*range.end() as f64)).text(label));
    *val = v as u64;
    *val != old
}

fn color_picker(ui: &mut egui::Ui, label: &str, color: &mut (u8, u8, u8)) -> bool {
    let mut c = [color.0, color.1, color.2];
    let old = c;
    ui.horizontal(|ui| {
        ui.label(label);
        ui.color_edit_button_srgb(&mut c);
    });
    if c != old {
        *color = (c[0], c[1], c[2]);
        true
    } else {
        false
    }
}
