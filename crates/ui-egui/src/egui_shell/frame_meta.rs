//! Settled-frame AccessKit & `egui_shell` inspection side-table metadata and geometry stamp.

use egui::Rect;
use serde::{Deserialize, Serialize};

use crate::inspect::ZLayer;
use crate::menus::{ActionRank, default_command_action_rank};

/// Metadata recorded for a single rendered `egui` widget during a settled frame pass.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectNodeMeta {
    pub egui_id: egui::Id,
    pub accesskit_id: egui::accesskit::NodeId,
    pub widget_id: String,
    pub role: String,
    pub label: String,
    pub command_id: Option<String>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    /// Unclipped interaction rectangle before UI scroll/panel clipping (logical `pt`).
    pub unclipped_rect: Rect,
    /// Interaction rectangle clipped against the active UI clip rect (logical `pt`).
    pub clipped_rect: Rect,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<String>,
    /// Structural action hierarchy rank (`Primary`, `Secondary`, `Overflow`) when applicable.
    pub action_rank: Option<ActionRank>,
}

fn infer_slot_action_rank(widget_id: &str, role: &str, command_id: Option<&str>) -> Option<ActionRank> {
    if !role.eq_ignore_ascii_case("button") {
        return None;
    }
    match widget_id {
        "modal.ok"
        | "dialog.button.save"
        | "welcome.clone_modal.apply"
        | "welcome.action.create"
        | "power.commit"
        | "import.commit"
        | "transform.commit" => return Some(ActionRank::Primary),
        "modal.cancel" | "modal.apply" | "modal.close" | "dialog.button.cancel" | "dialog.button.dont_save" | "welcome.clone_modal.close" => {
            return Some(ActionRank::Secondary);
        }
        _ => {}
    }
    if widget_id.starts_with("toolbar.") {
        return command_id.map_or(Some(ActionRank::Overflow), |cid| Some(default_command_action_rank(cid)));
    }
    command_id.map(default_command_action_rank)
}

/// Minimum clipped width and height (logical `pt`) required before a modal widget is recorded in [`FrameInspectMeta`].
pub const MODAL_INSPECT_MIN_CLIP_EXTENT_PT: f32 = 0.5;

/// Parameter bundle for [`FrameInspectMeta::record_response`].
#[derive(Debug, Clone)]
pub struct FrameInspectEntryInput<'a> {
    pub widget_id: &'a str,
    pub role: &'a str,
    pub label: &'a str,
    pub command_id: Option<&'a str>,
    pub value: Option<serde_json::Value>,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub z_layer: ZLayer,
    pub ellipsized: bool,
    pub full_label: Option<&'a str>,
}

impl<'a> FrameInspectEntryInput<'a> {
    /// Construct modal-dialog inspection metadata with [`ZLayer::ModalDialog`] and the default
    /// `"unavailable"` disabled reason when `enabled` is `false`.
    #[must_use]
    pub fn modal(widget_id: &'a str, role: &'a str, label: &'a str, value: Option<String>, enabled: bool) -> Self {
        Self {
            widget_id,
            role,
            label,
            command_id: None,
            value: value.map(serde_json::Value::String),
            enabled,
            disabled_reason: (!enabled).then_some("unavailable"),
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        }
    }

    /// Construct modal-dialog button inspection metadata (`role = "button"`, `value = None`).
    #[must_use]
    pub fn modal_button(widget_id: &'a str, label: &'a str, enabled: bool) -> Self {
        Self::modal(widget_id, "button", label, None, enabled)
    }
}

/// Settled-pass (`!ctx.will_discard()`) side-table mapping `egui`/AccessKit nodes to CurveBook
/// `widget_id`, `command_id`, `disabled_reason`, clipped interaction `Rect`, and ellipsis metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInspectMeta {
    pub pass_nr: u64,
    pub scale_factor: f32,
    pub viewport_rect: Rect,
    pub entries: Vec<FrameInspectNodeMeta>,
}

impl Default for FrameInspectMeta {
    fn default() -> Self {
        Self { pass_nr: 0, scale_factor: 1.0, viewport_rect: Rect::NOTHING, entries: Vec::new() }
    }
}

impl FrameInspectMeta {
    /// Construct an empty settled-frame side-table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset side-table entries for a fresh frame pass.
    pub fn begin_pass(&mut self, ctx: &egui::Context) {
        self.pass_nr = ctx.cumulative_pass_nr();
        self.scale_factor = ctx.pixels_per_point();
        self.viewport_rect = ctx.content_rect();
        self.entries.clear();
    }

    /// Clear all recorded entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of recorded widget entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no widget entries are recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a pre-built [`FrameInspectNodeMeta`] entry.
    pub fn record(&mut self, entry: FrameInspectNodeMeta) {
        self.entries.push(entry);
    }

    /// Record inspection metadata from an `egui::Response`, clipping its interaction rect to `ui.clip_rect()`.
    pub fn record_response(&mut self, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>) {
        let inferred = infer_slot_action_rank(input.widget_id, input.role, input.command_id);
        self.record_response_inner(ui, resp, resp.rect, input, inferred);
    }

    /// Record inspection metadata with an explicit unclipped logical rect (for virtualized scroll bands).
    pub fn record_response_with_unclipped(&mut self, ui: &egui::Ui, resp: &egui::Response, unclipped_rect: Rect, input: FrameInspectEntryInput<'_>) {
        let inferred = infer_slot_action_rank(input.widget_id, input.role, input.command_id);
        self.record_response_inner(ui, resp, unclipped_rect, input, inferred);
    }

    /// Record inspection metadata with an explicit per-slot [`ActionRank`].
    pub fn record_ranked_response(&mut self, ui: &egui::Ui, resp: &egui::Response, input: FrameInspectEntryInput<'_>, rank: ActionRank) {
        self.record_response_inner(ui, resp, resp.rect, input, Some(rank));
    }

    fn record_response_inner(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        unclipped_rect: Rect,
        input: FrameInspectEntryInput<'_>,
        action_rank: Option<ActionRank>,
    ) {
        let clipped_rect = resp.interact_rect.intersect(ui.clip_rect());
        self.entries.push(FrameInspectNodeMeta {
            egui_id: resp.id,
            accesskit_id: resp.id.accesskit_id(),
            widget_id: input.widget_id.to_owned(),
            role: input.role.to_owned(),
            label: input.label.to_owned(),
            command_id: input.command_id.map(str::to_owned),
            value: input.value,
            enabled: input.enabled,
            disabled_reason: input.disabled_reason.map(str::to_owned),
            unclipped_rect,
            clipped_rect,
            z_layer: input.z_layer,
            ellipsized: input.ellipsized,
            full_label: input.full_label.map(str::to_owned),
            action_rank,
        });
    }

    /// Record inspection metadata for a modal-dialog widget when its clipped interaction rectangle
    /// exceeds [`MODAL_INSPECT_MIN_CLIP_EXTENT_PT`] in both axes, enforcing [`ZLayer::ModalDialog`]
    /// and a default `"unavailable"` disabled reason when `input.enabled` is `false`.
    pub fn record_modal_response(&mut self, ui: &egui::Ui, resp: &egui::Response, mut input: FrameInspectEntryInput<'_>) {
        let clipped = resp.interact_rect.intersect(ui.clip_rect());
        if clipped.width() <= MODAL_INSPECT_MIN_CLIP_EXTENT_PT || clipped.height() <= MODAL_INSPECT_MIN_CLIP_EXTENT_PT {
            return;
        }
        input.z_layer = ZLayer::ModalDialog;
        if !input.enabled && input.disabled_reason.is_none() {
            input.disabled_reason = Some("unavailable");
        }
        self.record_response(ui, resp, input);
    }

    /// Look up the first recorded entry matching `widget_id`.
    #[must_use]
    pub fn find_by_widget_id(&self, widget_id: &str) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.widget_id == widget_id)
    }

    /// Look up the first recorded entry matching `egui_id`.
    #[must_use]
    pub fn find_by_egui_id(&self, egui_id: egui::Id) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.egui_id == egui_id)
    }

    /// Look up the first recorded entry matching `accesskit_id`.
    #[must_use]
    pub fn find_by_accesskit_id(&self, accesskit_id: egui::accesskit::NodeId) -> Option<&FrameInspectNodeMeta> {
        self.entries.iter().find(|entry| entry.accesskit_id == accesskit_id)
    }
}

/// Geometry and DPI synchronization epoch used to prevent stale pre-resize `ui.screenshot` captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct GeometryStamp {
    pub frame_seq: u64,
    pub pass_nr: u64,
    pub physical_width_px: u32,
    pub physical_height_px: u32,
    /// Scale factor quantized in basis points (`round(pixels_per_point * 10_000.0)`).
    pub scale_bps: u32,
    pub settled: bool,
}

impl GeometryStamp {
    /// Quantize a `pixels_per_point` float into integer basis points (`10_000` == `1.0x`).
    #[must_use]
    pub fn quantize_scale(pixels_per_point: f32) -> u32 {
        let clamped = if pixels_per_point.is_finite() { pixels_per_point.clamp(0.5, 8.0) } else { 1.0 };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bps = (f64::from(clamped) * 10_000.0).round() as u32;
        bps
    }

    /// Construct a [`GeometryStamp`] from a settled/unsettled `egui::Context` frame pass.
    #[must_use]
    pub fn from_context(frame_seq: u64, ctx: &egui::Context, physical_size_px: [u32; 2], settled: bool) -> Self {
        Self {
            frame_seq,
            pass_nr: ctx.cumulative_pass_nr(),
            physical_width_px: physical_size_px[0],
            physical_height_px: physical_size_px[1],
            scale_bps: Self::quantize_scale(ctx.pixels_per_point()),
            settled,
        }
    }

    /// Return `true` when this stamp is settled (`!ctx.will_discard()`) and matches the requested
    /// framebuffer pixel dimensions and scale factor.
    #[must_use]
    pub fn matches_target(&self, width_px: u32, height_px: u32, pixels_per_point: f32) -> bool {
        self.settled
            && self.physical_width_px == width_px
            && self.physical_height_px == height_px
            && self.scale_bps == Self::quantize_scale(pixels_per_point)
    }

    /// Recover the floating-point `pixels_per_point` scale factor from [`Self::scale_bps`].
    #[must_use]
    pub fn scale_factor(self) -> f32 {
        (f64::from(self.scale_bps.max(5_000)) / 10_000.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameInspectEntryInput, FrameInspectMeta, GeometryStamp};
    use crate::inspect::ZLayer;

    #[test]
    fn frame_inspect_meta_clips_rects_and_geometry_stamp_validates_target() {
        let ctx = egui::Context::default();
        let mut meta = FrameInspectMeta::new();
        let mut captured_id = None;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(120.0, 28.0), egui::Sense::click());
                captured_id = Some(resp.id);
                meta.record_response(
                    ui,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: "toolbar.analyze",
                        role: "Button",
                        label: "Analyze",
                        command_id: Some("ui.analyze.open"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                assert!(rect.width() >= 28.0);
            },
        );
        output.textures_delta.clear();
        let hit = meta.find_by_widget_id("toolbar.analyze").expect("entry recorded");
        assert_eq!(hit.command_id.as_deref(), Some("ui.analyze.open"));
        assert_eq!(Some(hit.egui_id), captured_id);
        assert!(hit.clipped_rect.width() > 0.0);

        let stale = GeometryStamp::from_context(1, &ctx, [640, 420], false);
        assert!(!stale.matches_target(640, 420, 1.0));
        let settled = GeometryStamp::from_context(2, &ctx, [1280, 840], true);
        assert!(settled.matches_target(1280, 840, 1.0));
        assert!(!settled.matches_target(640, 420, 1.0));
    }
}
