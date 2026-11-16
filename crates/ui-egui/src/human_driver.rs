//! `R10.2` Human-Path Input Driver (`0` external UI-test crate dependencies).
//!
//! Enforces [`UiApp::set_human_path_only(true)`] and drives workflows strictly through physical/logical
//! pointer coordinates, scroll wheels, focus navigation, raw keystrokes, and IME text events resolved
//! against rendered [`LayoutTree`] bounds — measuring real input-to-paint latency (`R10.22 <= 50 ms`).

use curvebook_render::{RasterImage, RenderError, RenderTargetSpec};

use crate::UiApp;
use crate::compositor::render_workspace_rgba;
use crate::inspect::{UiTreeSnapshot, inspect_ui_tree};
use crate::interaction::{
    KeyModifiers, PointerButton, UiEventOutcome, UiInputEvent, ViewportMetrics, dispatch_mouse_wheel, dispatch_ui_event_without_layout_rebuild,
};
use crate::layout::verify::{ParityReport, verify_pixel_to_hit_box_parity};
use crate::layout::{LayoutTree, WidgetNode};

/// Test & QA driver simulating real human pointer, wheel, focus, keystroke, and IME interactions.
#[derive(Debug, Clone)]
pub struct HumanPathDriver {
    app: UiApp,
    metrics: ViewportMetrics,
    last_input_to_paint_ms: f64,
    max_input_to_paint_ms: f64,
}

impl HumanPathDriver {
    /// Wrap `app` under `metrics`, enabling both `gui_only` and `human_path_only` strict guards.
    #[must_use]
    pub fn new(mut app: UiApp, metrics: ViewportMetrics) -> Self {
        app.set_gui_only(true);
        app.set_human_path_only(true);
        let _ = app.build_and_cache_layout_tree(metrics);
        let driver = Self { app, metrics, last_input_to_paint_ms: 0.0, max_input_to_paint_ms: 0.0 };
        let _ = render_workspace_rgba(&driver.app, driver.render_target_spec());
        driver
    }

    #[must_use]
    pub const fn app(&self) -> &UiApp {
        &self.app
    }

    pub const fn app_mut(&mut self) -> &mut UiApp {
        &mut self.app
    }

    #[must_use]
    pub const fn metrics(&self) -> ViewportMetrics {
        self.metrics
    }

    pub fn set_metrics(&mut self, metrics: ViewportMetrics) {
        self.metrics = metrics;
        let _ = self.app.build_and_cache_layout_tree(metrics);
    }

    #[must_use]
    pub const fn last_input_to_paint_ms(&self) -> f64 {
        self.last_input_to_paint_ms
    }

    #[must_use]
    pub const fn max_input_to_paint_ms(&self) -> f64 {
        self.max_input_to_paint_ms
    }

    /// Rebuild (or reuse cached) [`LayoutTree`] for current state and viewport.
    pub fn layout_tree(&mut self) -> LayoutTree {
        self.app.build_and_cache_layout_tree(self.metrics)
    }

    /// Render the active frame into RGBA pixels at [`Self::metrics`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if target dimensions are invalid.
    pub fn render_frame(&mut self) -> Result<RasterImage, RenderError> {
        render_workspace_rgba(&self.app, self.render_target_spec())
    }

    #[must_use]
    pub fn render_target_spec(&self) -> RenderTargetSpec {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let dpi = (self.metrics.clamped_scale_factor() * 96.0).round().clamp(72.0, 384.0) as u16;
        RenderTargetSpec { width_px: self.metrics.physical_width_px(), height_px: self.metrics.physical_height_px(), dpi }
    }

    /// Run `verify_pixel_to_hit_box_parity` against the current frame.
    pub fn verify_parity(&mut self) -> ParityReport {
        verify_pixel_to_hit_box_parity(&mut self.app, self.metrics)
    }

    /// Inspect semantic UI tree projected from [`LayoutTree`].
    #[must_use]
    pub fn inspect_tree(&self, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
        inspect_ui_tree(&self.app, scope, include_bounds)
    }

    /// Return the logical `[cx, cy]` center of rendered widget `widget_id` if drawn and enabled.
    #[must_use]
    pub fn widget_center_logical(&mut self, widget_id: &str) -> Option<[f64; 2]> {
        let tree = self.layout_tree();
        let node = tree.node(widget_id)?;
        if !node.drawn_in_frame || node.bounds.visible_rect_px.width == 0 || node.bounds.visible_rect_px.height == 0 {
            return None;
        }
        let [cx, cy] = node.bounds.visible_rect_px.center();
        Some(self.metrics.physical_to_logical_px([f64::from(cx), f64::from(cy)]))
    }

    /// Lookup a copy of rendered [`WidgetNode`] by ID.
    #[must_use]
    pub fn find_widget(&mut self, widget_id: &str) -> Option<WidgetNode> {
        self.layout_tree().node(widget_id).cloned()
    }

    /// Move pointer over logical `(lx, ly)` and repaint.
    pub fn move_pointer(&mut self, lx: f64, ly: f64) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            dispatch_ui_event_without_layout_rebuild(app, UiInputEvent::PointerMove { x: lx, y: ly, modifiers: KeyModifiers::default() }, metrics)
        })
    }

    /// Move pointer onto rendered widget `widget_id` to trigger hover styling & tooltip.
    pub fn hover_widget(&mut self, widget_id: &str) -> Result<UiEventOutcome, String> {
        let [lx, ly] = self.widget_center_logical(widget_id).ok_or_else(|| format!("widget `{widget_id}` is not visibly drawn"))?;
        self.move_pointer(lx, ly)
    }

    /// Perform a full physical move + primary down + primary up click at logical `(lx, ly)`.
    pub fn click_logical(&mut self, lx: f64, ly: f64) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            let _ = dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerMove { x: lx, y: ly, modifiers: KeyModifiers::default() },
                metrics,
            )?;
            let down = dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )?;
            let up = dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerUp { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )?;
            Ok(if up.handled { up } else { down })
        })
    }

    /// Click rendered widget `widget_id` strictly by moving to its rendered center `(cx, cy)` and clicking.
    pub fn click_widget(&mut self, widget_id: &str) -> Result<UiEventOutcome, String> {
        let tree = self.layout_tree();
        let node = tree.node(widget_id).ok_or_else(|| format!("widget `{widget_id}` not found in LayoutTree"))?;
        if !node.drawn_in_frame {
            return Err(format!("widget `{widget_id}` is not drawn_in_frame"));
        }
        if !node.state.enablement.enabled {
            return Err(format!("widget `{widget_id}` is disabled: {}", node.state.enablement.disabled_reason.as_deref().unwrap_or("unavailable")));
        }
        let [cx, cy] = node.bounds.visible_rect_px.center();
        let [lx, ly] = self.metrics.physical_to_logical_px([f64::from(cx), f64::from(cy)]);
        self.click_logical(lx, ly)
    }

    /// Double-click at logical `(lx, ly)`.
    pub fn double_click_logical(&mut self, lx: f64, ly: f64) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::DoubleClick { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )
        })
    }

    /// Drag pointer from `(from_lx, from_ly)` to `(to_lx, to_ly)`.
    pub fn drag_logical(&mut self, from_lx: f64, from_ly: f64, to_lx: f64, to_ly: f64) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            let _ = dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerDown { x: from_lx, y: from_ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )?;
            let _ = dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerMove { x: to_lx, y: to_ly, modifiers: KeyModifiers::default() },
                metrics,
            )?;
            dispatch_ui_event_without_layout_rebuild(
                app,
                UiInputEvent::PointerUp { x: to_lx, y: to_ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )
        })
    }

    /// Dispatch a real scroll wheel / trackpad gesture at logical `(lx, ly)`.
    pub fn wheel_logical(&mut self, lx: f64, ly: f64, delta_x: f64, delta_y: f64, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| dispatch_mouse_wheel(app, metrics, lx, ly, delta_x, delta_y, modifiers))
    }

    /// Scroll over rendered widget/container `widget_id` via mouse wheel.
    pub fn wheel_widget(&mut self, widget_id: &str, delta_x: f64, delta_y: f64, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
        let [lx, ly] = self.widget_center_logical(widget_id).ok_or_else(|| format!("widget `{widget_id}` is not visibly drawn"))?;
        self.wheel_logical(lx, ly, delta_x, delta_y, modifiers)
    }

    /// Press key `key` with `modifiers` through the unified keyboard handler.
    pub fn press_key(&mut self, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            dispatch_ui_event_without_layout_rebuild(app, UiInputEvent::KeyDown { key: key.to_owned(), modifiers }, metrics)
        })
    }

    /// Type text into the currently focused field (no target-jumping).
    pub fn type_text(&mut self, text: &str) -> Result<UiEventOutcome, String> {
        self.measure_input_to_paint(|app, metrics| {
            dispatch_ui_event_without_layout_rebuild(app, UiInputEvent::TextInput { text: text.to_owned() }, metrics)
        })
    }

    /// Stage IME composition preedit text on the active focused field.
    pub fn set_ime_preedit(&mut self, preedit: Option<String>) {
        if let Some(fid) = self.app.focus_widget_id.clone() {
            let entry = self.app.interaction.text_field_buffers.entry(fid).or_default();
            entry.set_preedit(preedit);
            let _ = self.app.build_and_cache_layout_tree(self.metrics);
        }
    }

    fn measure_input_to_paint<F>(&mut self, op: F) -> Result<UiEventOutcome, String>
    where
        F: FnOnce(&mut UiApp, ViewportMetrics) -> Result<UiEventOutcome, String>,
    {
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = std::time::Instant::now();

        let out = op(&mut self.app, self.metrics)?;
        let _ = self.app.build_and_cache_layout_tree(self.metrics);
        render_workspace_rgba(&self.app, self.render_target_spec()).map_err(|e| e.to_string())?;

        #[cfg(not(target_arch = "wasm32"))]
        {
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            self.last_input_to_paint_ms = elapsed_ms;
            if elapsed_ms > self.max_input_to_paint_ms {
                self.max_input_to_paint_ms = elapsed_ms;
            }
        }
        Ok(out)
    }
}
