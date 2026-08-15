//! Canvas interaction handler — pan, zoom, hover, click, minimap.
//!
//! Processes raw egui input events and translates them into viewport
//! transforms and file selection state updates. All coordinate math
//! uses the ViewportTransform for world/screen conversion.

use crate::renderer::minimap;
use std::sync::Arc;
use std::time::Duration;

use super::SentruxApp;

impl SentruxApp {
    /// Handle canvas interaction: pan, zoom, hover, click, minimap click
    pub(crate) fn handle_canvas_interaction(
        &mut self,
        response: &egui::Response,
        canvas_rect: egui::Rect,
    ) {
        let ctx_ptr = response.ctx.clone();

        // Update canvas dimensions
        self.state.viewport.canvas_w = canvas_rect.width() as f64;
        self.state.viewport.canvas_h = canvas_rect.height() as f64;

        self.handle_zoom(response, &ctx_ptr, canvas_rect);
        self.handle_pan(response, &ctx_ptr, canvas_rect);
        self.update_interacting_state(&ctx_ptr);
        self.handle_hover(response, &ctx_ptr, canvas_rect);

        // Click: check minimap first, then canvas select/deselect
        if self.handle_click(response, &ctx_ptr, canvas_rect) {
            return; // consumed by minimap
        }

        let drill_changed = self.handle_double_click(response);
        let drill_popped = self.handle_escape(&ctx_ptr);

        if drill_changed || drill_popped {
            self.state.rendered_version = 0;
            self.request_layout();
        }

        self.handle_right_click(response, &ctx_ptr, canvas_rect);
        self.handle_context_menu(response);

        if self.state.interacting {
            ctx_ptr.request_repaint();
        }
    }

    /// Compute the dynamic minimum zoom level based on content dimensions.
    fn dynamic_min_zoom(&self) -> f64 {
        self.state
            .render_data
            .as_ref()
            .map_or(self.state.settings.zoom_min, |rd| {
                self.state.viewport.min_zoom_for_content(
                    rd.content_width,
                    rd.content_height,
                    0.8,
                    self.state.settings.zoom_min,
                )
            })
    }

    /// Handle scroll-wheel zoom centered on cursor position.
    fn handle_zoom(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
    ) {
        if !response.hovered() {
            return;
        }
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() <= 0.1 {
            return;
        }

        let zf = self.state.settings.zoom_scroll_factor;
        let factor = if scroll > 0.0 { zf } else { 1.0 / zf };
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            let dynamic_min = self.dynamic_min_zoom();
            self.state.viewport.zoom_at(
                pos.x,
                pos.y,
                canvas_rect.min,
                factor,
                dynamic_min,
                self.state.settings.zoom_max,
            );
            self.state.last_interaction = std::time::Instant::now();
            self.state.interacting = true;
        }
    }

    /// Check if a drag started on the minimap (should not initiate canvas pan).
    fn is_drag_on_minimap(&self, ctx: &egui::Context, canvas_rect: egui::Rect) -> bool {
        ctx.input(|i| {
            i.pointer.hover_pos().is_some_and(|pos| {
                self.state.render_data.as_ref().is_some_and(|rd| {
                    minimap::minimap_rect(canvas_rect, &self.state.settings).contains(pos)
                        && rd.content_width > 0.0
                })
            })
        })
    }

    /// Apply drag delta to viewport offset while dragging.
    fn apply_drag_delta(&mut self, ctx: &egui::Context) {
        let (start, (ox, oy)) = match (self.state.drag_start_screen, self.state.drag_start_offset) {
            (Some(s), Some(o)) => (s, o),
            _ => return,
        };
        let current = match ctx.input(|i| i.pointer.hover_pos()) {
            Some(c) => c,
            None => return,
        };
        let dx = (current.x - start.x) as f64 / self.state.viewport.scale;
        let dy = (current.y - start.y) as f64 / self.state.viewport.scale;
        self.state.viewport.offset_x = ox - dx;
        self.state.viewport.offset_y = oy - dy;
        self.state.last_interaction = std::time::Instant::now();
        self.state.interacting = true;
    }

    /// Handle drag-based panning (start, move, stop).
    fn handle_pan(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
    ) {
        if response.drag_started() && !self.is_drag_on_minimap(ctx, canvas_rect) {
            self.state.dragging = true;
            self.state.drag_start_screen = ctx.input(|i| i.pointer.hover_pos());
            self.state.drag_start_offset =
                Some((self.state.viewport.offset_x, self.state.viewport.offset_y));
        }

        if self.state.dragging {
            self.apply_drag_delta(ctx);
        }

        if response.drag_stopped() {
            self.state.dragging = false;
            self.state.drag_start_screen = None;
            self.state.drag_start_offset = None;
        }
    }

    /// Clear interacting flag after 150ms idle.
    fn update_interacting_state(&mut self, ctx: &egui::Context) {
        if self.state.interacting
            && self.state.last_interaction.elapsed() > Duration::from_millis(150)
            && !self.state.dragging
        {
            self.state.interacting = false;
            ctx.request_repaint();
        }
    }

    /// Perform spatial hit-test at cursor position and update hovered path if changed.
    fn update_hover_from_cursor(&mut self, ctx: &egui::Context, canvas_rect: egui::Rect) {
        let pos = match ctx.input(|i| i.pointer.hover_pos()) {
            Some(p) => p,
            None => return,
        };
        let (wx, wy) = self
            .state
            .viewport
            .screen_to_world(pos.x, pos.y, canvas_rect.min);
        let hit = self
            .state
            .spatial_index
            .as_ref()
            .and_then(|si| si.hit_test(wx, wy))
            .map(|s| s.to_string());
        if hit != self.state.hovered_path {
            self.state.hovered_path = hit;
            ctx.request_repaint();
        }
    }

    /// Update hovered file path via spatial index hit testing.
    fn handle_hover(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
    ) {
        if response.hovered() && !self.state.dragging {
            self.update_hover_from_cursor(ctx, canvas_rect);
        } else if !response.hovered() && self.state.hovered_path.is_some() {
            self.state.hovered_path = None;
            ctx.request_repaint();
        }
    }

    /// Try to navigate the viewport via minimap click. Returns true if consumed.
    fn try_minimap_navigate(
        &mut self,
        pos: egui::Pos2,
        canvas_rect: egui::Rect,
        ctx: &egui::Context,
    ) -> bool {
        let rd = match &self.state.render_data {
            Some(rd) => rd,
            None => return false,
        };
        let (wx, wy) =
            match minimap::minimap_click_to_world(pos, canvas_rect, rd, &self.state.settings) {
                Some(coords) => coords,
                None => return false,
            };
        let vp = &self.state.viewport;
        let half_w = vp.canvas_w / vp.scale / 2.0;
        let half_h = vp.canvas_h / vp.scale / 2.0;
        self.state.viewport.offset_x = wx - half_w;
        self.state.viewport.offset_y = wy - half_h;
        ctx.request_repaint();
        true
    }

    /// Toggle file selection based on the currently hovered path.
    fn toggle_file_selection(&mut self) {
        if let Some(path) = &self.state.hovered_path {
            if self.state.selected_path.as_deref() == Some(path) {
                self.state.selected_path = None;
            } else {
                self.state.selected_path = Some(path.clone());
            }
        } else {
            self.state.selected_path = None;
        }
    }

    /// Handle left-click: minimap navigation or file select/deselect.
    /// Returns true if the click was consumed by the minimap.
    fn handle_click(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
    ) -> bool {
        // Use ui.input() to check if pointer had a double-click this frame.
        // egui's response.clicked() fires on the first click of a double-click too,
        // so checking response.double_clicked() alone misses the first click.
        // By checking the raw pointer state, we suppress single-click side effects
        // entirely when a double-click occurred.
        let double_clicked_this_frame = response.double_clicked()
            || ctx.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            });
        if !(response.clicked() && !self.state.dragging && !double_clicked_this_frame) {
            return false;
        }
        let pos = match ctx.input(|i| i.pointer.hover_pos()) {
            Some(p) => p,
            None => return false,
        };

        if self.try_minimap_navigate(pos, canvas_rect, ctx) {
            return true;
        }

        self.toggle_file_selection();
        false
    }

    /// Handle double-click: drill into parent directory. Returns true if drill changed.
    fn handle_double_click(&mut self, response: &egui::Response) -> bool {
        if !response.double_clicked() {
            return false;
        }
        if let Some(path) = &self.state.hovered_path {
            if let Some(pos) = path.rfind('/') {
                let dir = path[..pos].to_string();
                // Only drill deeper — don't push if already at or below this directory
                let already_at = self.state.drill_stack.last().is_some_and(|current| {
                    dir == *current || dir.starts_with(&format!("{}/", current))
                });
                if already_at {
                    // We're already in this dir — try to drill into a subdirectory
                    // by finding the next level down from the current drill point
                    if let Some(current) = self.state.drill_stack.last() {
                        let rest = path.strip_prefix(&format!("{}/", current)).unwrap_or(path);
                        if let Some(next_slash) = rest.find('/') {
                            let next_dir = format!("{}/{}", current, &rest[..next_slash]);
                            self.state.drill_stack.push(next_dir);
                            return true;
                        }
                    }
                } else {
                    self.state.drill_stack.push(dir);
                    return true;
                }
            }
        }
        false
    }

    /// Handle ESC key: clear selection or pop drill stack. Returns true if drill popped.
    fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return false;
        }
        if self.state.selected_path.is_some() {
            self.state.selected_path = None;
            false
        } else if !self.state.drill_stack.is_empty() {
            self.state.drill_stack.pop();
            true
        } else {
            false
        }
    }

    /// Resolve what the user right-clicked on: a file, a section, or nothing.
    fn resolve_click_target(
        &self,
        wx: f64,
        wy: f64,
    ) -> Option<crate::app::state::ContextMenuTarget> {
        let si = self.state.spatial_index.as_ref()?;
        if let Some(path) = si.hit_test(wx, wy) {
            return Some(crate::app::state::ContextMenuTarget {
                path: path.to_string(),
                is_dir: false,
            });
        }
        si.hit_test_section(wx, wy)
            .map(|path| crate::app::state::ContextMenuTarget {
                path: path.to_string(),
                is_dir: true,
            })
    }

    /// Handle right-click: set context menu target (file or directory section).
    fn handle_right_click(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
    ) {
        if !response.secondary_clicked() {
            return;
        }
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            let (wx, wy) = self
                .state
                .viewport
                .screen_to_world(pos.x, pos.y, canvas_rect.min);
            self.state.context_menu_target = self.resolve_click_target(wx, wy);
        }
    }

    /// Show the context menu and apply hide/unhide actions.
    fn handle_context_menu(&mut self, response: &egui::Response) {
        let (hide_action, unhide_action) = Self::draw_context_menu_ui(
            response,
            &self.state.context_menu_target,
            &self.state.root_path,
            &self.state.hidden_paths,
        );

        if let Some(path) = hide_action {
            Arc::make_mut(&mut self.state.hidden_paths).insert(path);
            self.request_layout();
        }
        if unhide_action {
            self.state.hidden_paths = Arc::new(std::collections::HashSet::new());
            self.request_layout();
        }
    }

    /// Draw context menu items for a specific file/folder target.
    fn draw_target_menu_items(
        ui: &mut egui::Ui,
        t: &crate::app::state::ContextMenuTarget,
        root_path: &Option<String>,
    ) -> Option<String> {
        let name = t.path.rsplit('/').next().unwrap_or(&t.path);
        let kind = if t.is_dir { "folder" } else { "file" };
        ui.label(egui::RichText::new(name).strong().monospace());
        ui.separator();
        if ui.button("Copy abs path").clicked() {
            let text = root_path
                .as_ref()
                .map(|r| format!("{}/{}", r, t.path))
                .unwrap_or_else(|| t.path.clone());
            ui.ctx().copy_text(text);
            ui.close_menu();
        }
        if ui.button("Reveal in Finder").clicked() {
            if let Some(root) = root_path {
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(format!("{}/{}", root, t.path))
                    .spawn();
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button(format!("Hide {}", kind)).clicked() {
            ui.close_menu();
            return Some(t.path.clone());
        }
        None
    }

    /// Render context menu items, returning (hide_action, unhide_action).
    fn draw_context_menu_ui(
        response: &egui::Response,
        target: &Option<crate::app::state::ContextMenuTarget>,
        root_path: &Option<String>,
        hidden_paths: &std::collections::HashSet<String>,
    ) -> (Option<String>, bool) {
        let mut hide_action: Option<String> = None;
        let mut unhide_action = false;
        response.context_menu(|ui| {
            if let Some(t) = target {
                hide_action = Self::draw_target_menu_items(ui, t, root_path);
            } else {
                ui.label(egui::RichText::new("(empty area)").weak());
            }
            if !hidden_paths.is_empty() {
                ui.separator();
                if ui
                    .button(format!("Unhide all ({} hidden)", hidden_paths.len()))
                    .clicked()
                {
                    unhide_action = true;
                    ui.close_menu();
                }
            }
        });
        (hide_action, unhide_action)
    }
}
