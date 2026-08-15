//! Viewport transform — maps world coordinates to screen pixels.
//!
//! Handles pan (offset), zoom (scale), fit-to-content, and visibility
//! culling. All renderer code uses `ViewportTransform` methods rather
//! than doing raw coordinate math.

/// Viewport transform: world coordinates → screen coordinates.
/// offset_x/y = world position of top-left corner of viewport.
/// scale = pixels per world unit.
pub struct ViewportTransform {
    /// World X coordinate of the viewport's left edge
    pub offset_x: f64,
    /// World Y coordinate of the viewport's top edge
    pub offset_y: f64,
    /// Zoom level: screen pixels per world unit (>1 = zoomed in)
    pub scale: f64,
    /// Canvas width in screen pixels
    pub canvas_w: f64,
    /// Canvas height in screen pixels
    pub canvas_h: f64,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportTransform {
    /// Create a default viewport (1x zoom, no offset, 800x600 canvas).
    pub fn new() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            canvas_w: 800.0,
            canvas_h: 600.0,
        }
    }

    /// World → screen X
    #[inline]
    pub fn wx(&self, world_x: f64) -> f32 {
        ((world_x - self.offset_x) * self.scale) as f32
    }

    /// World → screen Y
    #[inline]
    pub fn wy(&self, world_y: f64) -> f32 {
        ((world_y - self.offset_y) * self.scale) as f32
    }

    /// World size → screen size
    #[inline]
    pub fn ws(&self, world_size: f64) -> f32 {
        (world_size * self.scale) as f32
    }

    /// World rect → screen rect (with canvas offset applied)
    #[inline]
    pub fn world_to_screen_rect(
        &self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        canvas_origin: egui::Pos2,
    ) -> egui::Rect {
        let sx = canvas_origin.x + self.wx(x);
        let sy = canvas_origin.y + self.wy(y);
        let sw = self.ws(w);
        let sh = self.ws(h);
        egui::Rect::from_min_size(egui::pos2(sx, sy), egui::vec2(sw, sh))
    }

    /// Screen position → world position.
    /// Guards against scale ≤ 0 to prevent Infinity/NaN propagation.
    #[inline]
    pub fn screen_to_world(
        &self,
        screen_x: f32,
        screen_y: f32,
        canvas_origin: egui::Pos2,
    ) -> (f64, f64) {
        if self.scale <= 0.0 {
            return (self.offset_x, self.offset_y);
        }
        let wx = (screen_x - canvas_origin.x) as f64 / self.scale + self.offset_x;
        let wy = (screen_y - canvas_origin.y) as f64 / self.scale + self.offset_y;
        (wx, wy)
    }

    /// Zoom toward a screen point.
    /// BUG 12 fix: guards against NaN/Infinity factor (from malfunctioning input
    /// devices). NaN.clamp(min, max) returns NaN in Rust, which would freeze
    /// the viewport in an unrecoverable state with all offsets as NaN.
    pub fn zoom_at(
        &mut self,
        screen_x: f32,
        screen_y: f32,
        canvas_origin: egui::Pos2,
        factor: f64,
        zoom_min: f64,
        zoom_max: f64,
    ) {
        if !factor.is_finite() || factor <= 0.0 {
            return; // reject NaN, Infinity, negative, and zero factors
        }
        let (wx, wy) = self.screen_to_world(screen_x, screen_y, canvas_origin);
        let new_scale = (self.scale * factor).clamp(zoom_min, zoom_max);
        self.offset_x = wx - (screen_x - canvas_origin.x) as f64 / new_scale;
        self.offset_y = wy - (screen_y - canvas_origin.y) as f64 / new_scale;
        self.scale = new_scale;
    }

    /// Compute the minimum zoom so content fills at least `fill_pct` of the screen.
    /// Returns a dynamic floor based on content dimensions. If content is unknown, falls back to `fallback`.
    pub fn min_zoom_for_content(
        &self,
        content_w: f64,
        content_h: f64,
        fill_pct: f64,
        fallback: f64,
    ) -> f64 {
        if content_w <= 0.0 || content_h <= 0.0 || self.canvas_w <= 0.0 || self.canvas_h <= 0.0 {
            return fallback;
        }
        let scale_x = self.canvas_w / content_w;
        let scale_y = self.canvas_h / content_h;
        scale_x.min(scale_y) * fill_pct
    }

    /// Fit content to viewport with padding, centered on both axes.
    /// `zoom_min` parameter controls the minimum zoom level (previously hardcoded to 0.05).
    pub fn fit_content(&mut self, content_w: f64, content_h: f64, padding: f64) {
        if content_w <= 0.0 || content_h <= 0.0 {
            return;
        }
        let usable_w = (self.canvas_w - padding * 2.0).max(1.0);
        let usable_h = (self.canvas_h - padding * 2.0).max(1.0);
        let scale_x = usable_w / content_w;
        let scale_y = usable_h / content_h;
        // Use a reasonable minimum; callers can adjust via zoom_at if needed
        self.scale = scale_x.min(scale_y).max(0.01);
        // Center content on both axes
        self.offset_x = -(self.canvas_w / self.scale - content_w) / 2.0;
        self.offset_y = -(self.canvas_h / self.scale - content_h) / 2.0;
    }

    /// Check if a world-space rect is visible in the current viewport
    #[inline]
    pub fn is_visible(&self, x: f64, y: f64, w: f64, h: f64) -> bool {
        if self.scale <= 0.0 {
            return false;
        }
        let right = self.offset_x + self.canvas_w / self.scale;
        let bottom = self.offset_y + self.canvas_h / self.scale;
        // Use >= instead of > so zero-dimension rects (vertical/horizontal
        // edges) at the viewport boundary are not incorrectly culled.
        x + w >= self.offset_x && x <= right && y + h >= self.offset_y && y <= bottom
    }
}
