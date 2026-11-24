//! `Format Selected Points` modal body (`gui-layout-spec.md` §8.11).

use curvebook_palette::MarkerShape;
use egui::Ui;
use serde_json::{Map, Value, json};

use super::super::UiIntent;
use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

pub(super) fn render_format_selected_points_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fp = &gctx.app.canvas_dialogs.format_points;
    ui.label(format!("Selected points: {}", fp.selected_points.len()));
    ui.add_space(4.0);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Color:");
        for (name, hex) in
            [("Black", "#111827"), ("Red", "#DC2626"), ("Blue", "#2563EB"), ("Green", "#16A34A"), ("Orange", "#EA580C"), ("Purple", "#7C3AED")]
        {
            let selected = fp.color.as_deref() == Some(hex);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("fmt_pts.col.{name}"),
                    label: name,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("color"), Value::String(hex.to_owned()));
                patch.insert(String::from("fill_color"), Value::String(hex.to_owned()));
                gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Symbol:");
        for (lbl, shape) in [
            ("Circle", MarkerShape::Circle),
            ("Square", MarkerShape::Square),
            ("Triangle", MarkerShape::TriangleUp),
            ("Diamond", MarkerShape::Diamond),
        ] {
            let slug = format!("{shape:?}");
            let selected = fp.symbol.as_deref() == Some(slug.as_str());
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("fmt_pts.sym.{lbl}"),
                    label: lbl,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("symbol"), Value::String(slug));
                gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
            }
        }
    });

    ui.add_space(4.0);
    let size_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.5,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("fmt_pts.size", "Point Size", 2.0, 24.0)
    };
    let size_out = render_scrub_unit_field(ui, fp.size_pt.unwrap_or(5.0), &mut state.pts_size_spinner, size_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = size_out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("size_pt"), json!(val.clamp(2.0, 24.0)));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }

    let bw_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.25,
        decimals: 2,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("fmt_pts.border", "Border Width", 0.5, 6.0)
    };
    let bw_out = render_scrub_unit_field(ui, fp.border_width_pt.unwrap_or(1.0), &mut state.pts_border_spinner, bw_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = bw_out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("border_width_pt"), json!(val.clamp(0.5, 6.0)));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }

    ui.add_space(4.0);
    let mut hidden = fp.hidden.unwrap_or(false);
    let h_chk = ui.checkbox(&mut hidden, "Hide Selected Points");
    record_checkbox(ui, &h_chk, gctx.meta, "fmt_pts.hidden", "Hide Selected Points", hidden);
    if h_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("hidden"), Value::Bool(hidden));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }

    let mut clear = fp.clear_overrides;
    let c_chk = ui.checkbox(&mut clear, "Clear Point Overrides");
    record_checkbox(ui, &c_chk, gctx.meta, "fmt_pts.clear", "Clear Point Overrides", clear);
    if c_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("clear_overrides"), Value::Bool(clear));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }
}
