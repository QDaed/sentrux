//! Panel orchestration — draws all UI panels (toolbar, side panels, status bar).
//!
//! Extracted from update_loop.rs to keep the main loop focused on lifecycle
//! management and reduce fan-out (each panel import is an edge).

use super::SentruxApp;

/// Outcome of drawing all panels — tells the update loop what actions to take.
pub(crate) struct PanelResult {
    pub layout_changed: bool,
    pub visual_changed: bool,
    pub breadcrumb_changed: bool,
    pub layout_mode_changed: bool,
}

/// Draw the top toolbar and update result flags.
fn draw_toolbar_panel(app: &mut SentruxApp, ctx: &egui::Context, result: &mut PanelResult) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        let old_layout_mode = app.state.layout_mode;
        let (lc, vc) = crate::app::toolbar::draw_toolbar(ui, &mut app.state);
        if lc {
            result.layout_changed = true;
            if app.state.layout_mode != old_layout_mode {
                result.layout_mode_changed = true;
            }
        }
        if vc {
            result.visual_changed = true;
        }
    });
}

/// Draw settings panel if open, updating result flags.
fn draw_settings_if_open(app: &mut SentruxApp, ctx: &egui::Context, result: &mut PanelResult) {
    if !app.state.settings_open {
        return;
    }
    let (s_layout, s_visual) = crate::app::settings_panel::draw_settings_panel(
        ctx,
        &mut app.state.settings,
        &mut app.state.settings_open,
    );
    if s_layout {
        result.layout_changed = true;
    }
    if s_visual {
        result.visual_changed = true;
    }
}

/// Draw optional side panels (settings, metrics, activity, DSM).
fn draw_side_panels(app: &mut SentruxApp, ctx: &egui::Context, result: &mut PanelResult) {
    draw_settings_if_open(app, ctx, result);

    if app.state.snapshot.is_some() {
        crate::app::panels::metrics_panel::draw_metrics_panel(ctx, &mut app.state);
    }

    // Right panel: always visible when project loaded (context-sensitive)
    // Shows file detail when a file is selected, activity otherwise
    if app.state.snapshot.is_some()
        && crate::app::panels::activity_panel::draw_activity_panel(ctx, &mut app.state)
    {
        result.visual_changed = true;
    }

    if app.state.dsm_panel_open
        && app.state.snapshot.is_some()
        && crate::app::panels::dsm_panel::draw_dsm_panel(ctx, &mut app.state)
    {
        result.visual_changed = true;
    }
}

/// Draw all non-canvas panels. Returns what changed so update_loop can act.
pub(crate) fn draw_all_panels(app: &mut SentruxApp, ctx: &egui::Context) -> PanelResult {
    let mut result = PanelResult {
        layout_changed: false,
        visual_changed: false,
        breadcrumb_changed: false,
        layout_mode_changed: false,
    };

    draw_toolbar_panel(app, ctx, &mut result);
    draw_side_panels(app, ctx, &mut result);

    egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
        crate::app::status_bar::draw_status_bar(ui, &app.state);
    });

    // Breadcrumb: always visible when a project is loaded
    if app.state.root_path.is_some() {
        egui::TopBottomPanel::top("breadcrumb").show(ctx, |ui| {
            result.breadcrumb_changed = crate::app::breadcrumb::draw_breadcrumb(ui, &mut app.state);
        });
    }

    result
}

/// Draw the progress overlay on the canvas when scanning.
pub(crate) fn draw_progress(
    ui: &mut egui::Ui,
    state: &crate::app::state::AppState,
    has_render: bool,
) {
    crate::app::progress::draw_progress_overlay(ui, state, has_render);
}
