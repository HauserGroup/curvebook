#![forbid(unsafe_code)]

//! Clean-room deterministic workspace screenshot generator (`R0.14`).
//!
//! Renders `workspace-split-fit.png` (Light theme Split Workspace with XY dose-response
//! data, 4PL nonlinear fit + 95% confidence band ribbon + SD error bars + embedded analysis
//! result table) and `workspace-layout-dark.png` (Dark theme multi-panel `LayoutSheet`
//! with linked panels `A` and `B`, physical millimeter rulers, visual scale bar, and
//! page-layout inspector) into `<out_dir>`.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use lumencraft_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
use lumencraft_render::RenderTargetSpec;
use lumencraft_ui_egui::{RenderError, UiApp, UiThemeMode, dispatch_ui_command, render_workspace_frame_png};
use serde_json::json;

const WIDTH_PX: u32 = 1440;
const HEIGHT_PX: u32 = 900;
const DPI: u16 = 96;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = std::env::args_os().nth(1).map_or_else(|| PathBuf::from("docs/images"), PathBuf::from);
    fs::create_dir_all(&out_dir)?;

    let split_png = build_split_fit_png()?;
    write_png(&out_dir.join("workspace-split-fit.png"), &split_png)?;

    let layout_png = build_layout_dark_png()?;
    write_png(&out_dir.join("workspace-layout-dark.png"), &layout_png)?;

    Ok(())
}

fn write_png(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    fs::write(path, bytes)?;
    Ok(())
}

fn render_png_bytes(app: &UiApp) -> Result<Vec<u8>, RenderError> {
    render_workspace_frame_png(app, RenderTargetSpec { width_px: WIDTH_PX, height_px: HEIGHT_PX, dpi: DPI })
}

fn build_synthetic_dose_response_sheet(id: &str) -> Result<DataSheet, Box<dyn Error>> {
    // Log10 molar concentrations spanning -9.0 to -4.5 (clean-room synthetic 4PL agonist curve).
    let log_concs = [-9.0_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(3), 2, log_concs.len())?;
    table.x_title = String::from("Log10 [Compound X] (M)");
    table.data_sets[0].title = String::from("Wild-Type");
    table.data_sets[1].title = String::from("Variant R1");

    let offsets_wt = [-2.4_f64, 0.3, 2.1];
    let offsets_vr = [-1.9_f64, 0.2, 1.7];

    for (row, &x) in log_concs.iter().enumerate() {
        table.x_cells[row][0] = Cell::number(x);

        let y_wt = 8.0 + 88.0 / (1.0 + 10.0_f64.powf((-6.85 - x) * 1.15));
        let y_vr = 10.0 + 72.0 / (1.0 + 10.0_f64.powf((-5.95 - x) * 0.95));

        for (rep, (&ow, &ov)) in offsets_wt.iter().zip(offsets_vr.iter()).enumerate() {
            let phase = if row % 2 == 0 { 1.0 } else { -1.0 };
            table.data_sets[0].cells[row][rep] = Cell::number(((y_wt + ow * phase) * 100.0).round() / 100.0);
            table.data_sets[1].cells[row][rep] = Cell::number(((y_vr + ov * phase) * 100.0).round() / 100.0);
        }
    }

    Ok(DataSheet::new(SheetId::new(id), "Dose-Response Assay", TableArchetype::Xy(table))?)
}

fn build_synthetic_efficacy_sheet(id: &str) -> Result<DataSheet, Box<dyn Error>> {
    let groups = [
        ("Vehicle", [11.2_f64, 12.6, 10.4, 11.9, 12.1]),
        ("10 nM", [28.4, 31.1, 27.6, 29.8, 30.2]),
        ("100 nM", [64.5, 68.2, 62.9, 66.1, 65.8]),
        ("1 uM", [88.9, 92.4, 86.7, 90.5, 91.2]),
    ];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), 5)?;
    for (col, (label, vals)) in groups.iter().enumerate() {
        table.data_sets[col].title = String::from(*label);
        for (row, &val) in vals.iter().enumerate() {
            table.data_sets[col].cells[row][0] = Cell::number(val);
        }
    }
    Ok(DataSheet::new(SheetId::new(id), "Potency Summary", TableArchetype::Column(table))?)
}

fn style_dose_graph(app: &mut UiApp, graph_id: &str, result_id: &str, compact_for_layout: bool) -> Result<(), Box<dyn Error>> {
    app.execute_command(
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "frame_preset": "open_l",
            "x_axis": { "title": "Log10 [Compound X] (M)" },
            "y_axis": { "title": "Normalized Signal (%)" }
        }),
    )?;

    let (canvas, legend_pos) = if compact_for_layout {
        (json!({ "width": 270.0, "height": 230.0 }), json!([62.0, 24.0]))
    } else {
        (json!({ "width": 410.0, "height": 290.0 }), json!([58.0, 24.0]))
    };
    app.execute_command(
        "graph.setSpec",
        json!({
            "sheet_id": graph_id,
            "spec": {
                "canvas_size_pt": canvas,
                "legend": { "visible": true, "bordered": true, "position_pt": legend_pos }
            }
        }),
    )?;

    app.execute_command(
        "graph.setRegressionOverlay",
        json!({
            "sheet_id": graph_id,
            "result_sheet_id": result_id,
            "all_series": true,
            "sample_segments": 512,
            "show_confidence_band": true,
            "shaded_ribbon": true
        }),
    )?;

    if compact_for_layout {
        app.execute_command(
            "graph.addAnnotation",
            json!({
                "sheet_id": graph_id,
                "kind": "text",
                "text": "**4PL Fit Summary**\nWT EC50: -6.85\nR1 EC50: -5.95\n95% CI Ribbon",
                "bounds_pt": [62.0, 62.0, 88.0, 48.0],
                "bordered": true
            }),
        )?;
    } else {
        app.execute_command(
            "graph.addAnnotation",
            json!({
                "sheet_id": graph_id,
                "kind": "embedded_result_table",
                "result_sheet_id": result_id,
                "bounds_pt": [60.0, 74.0, 152.0, 56.0]
            }),
        )?;
    }

    app.sync_views();
    Ok(())
}

fn build_split_fit_png() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("Dose-Response Assay Workspace")?;
    doc.add_data_sheet(build_synthetic_dose_response_sheet("xy-dose")?)?;

    let mut app = UiApp::from_session(lumencraft_engine::Session::from_document(doc));
    app.set_theme_mode(UiThemeMode::Light);

    app.execute_command(
        "fit.run",
        json!({
            "sheet_id": "fit-4pl",
            "source_sheet": "xy-dose",
            "kind": "four_parameter_logistic",
            "title": "4PL Dose-Response Fit"
        }),
    )?;

    app.execute_command(
        "graph.create",
        json!({
            "sheet_id": "g-dose",
            "data_sheet_id": "xy-dose",
            "kind": "xy",
            "title": "4PL Curve Fit"
        }),
    )?;
    style_dose_graph(&mut app, "g-dose", "fit-4pl", false)?;

    app.execute_command("sheet.select", json!({ "sheet_id": "g-dose" }))?;
    let doc_snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc_snap, &SheetId::new("g-dose"));
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.family_open = true;
    app.split.enabled = true;
    app.split.set_ratio(0.42);
    app.split.primary_sheet_id = Some(SheetId::new("xy-dose"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-dose"));
    app.canvas_dialogs.status_banner = Some(String::from("4PL Fit Ready | R2 = 0.996 | 95% CI Ribbon | SD Error Bars"));
    app.sync_views();

    Ok(render_png_bytes(&app)?)
}

fn build_layout_dark_png() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("Publication Layout Studio")?;
    doc.add_data_sheet(build_synthetic_dose_response_sheet("xy-dose")?)?;
    doc.add_data_sheet(build_synthetic_efficacy_sheet("col-eff")?)?;

    let mut app = UiApp::from_session(lumencraft_engine::Session::from_document(doc));
    app.set_theme_mode(UiThemeMode::Dark);

    app.execute_command(
        "fit.run",
        json!({
            "sheet_id": "fit-4pl",
            "source_sheet": "xy-dose",
            "kind": "four_parameter_logistic",
            "title": "4PL Fit Analysis"
        }),
    )?;
    app.execute_command(
        "stats.run",
        json!({
            "sheet_id": "anova-eff",
            "source_sheet": "col-eff",
            "kind": "one_way_anova",
            "title": "One-Way ANOVA"
        }),
    )?;

    app.execute_command(
        "graph.create",
        json!({
            "sheet_id": "g-dose",
            "data_sheet_id": "xy-dose",
            "kind": "xy",
            "title": "Concentration-Response"
        }),
    )?;
    style_dose_graph(&mut app, "g-dose", "fit-4pl", true)?;

    app.execute_command(
        "graph.create",
        json!({
            "sheet_id": "g-eff",
            "data_sheet_id": "col-eff",
            "kind": "column",
            "title": "Efficacy Comparison"
        }),
    )?;
    app.execute_command(
        "graph.setAxes",
        json!({
            "sheet_id": "g-eff",
            "frame_preset": "open_l",
            "x_axis": { "title": "Treatment Group" },
            "y_axis": { "title": "Activation (%)", "min": 0.0, "max": 124.0, "major_tick_interval": 25.0 }
        }),
    )?;
    app.execute_command(
        "graph.setSpec",
        json!({
            "sheet_id": "g-eff",
            "spec": {
                "canvas_size_pt": { "width": 308.0, "height": 304.0 },
                "legend": { "visible": false }
            }
        }),
    )?;
    app.execute_command(
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-eff",
            "result_sheet_id": "anova-eff",
            "significant_only": true,
            "style": "square",
            "label_mode": "stars",
            "font_size_pt": 8.0,
            "bracket_step_pt": 17.0
        }),
    )?;

    // Physical scale bar drawn onto Panel B canvas corner (bottom-right of 308x304 pt graph).
    app.execute_command(
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-eff",
            "kind": "line",
            "start_pt": [241.3, 292.0],
            "end_pt": [298.0, 292.0],
            "arrowhead": false,
            "color": "#111827"
        }),
    )?;
    app.execute_command(
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-eff",
            "kind": "line",
            "start_pt": [241.3, 288.0],
            "end_pt": [241.3, 296.0],
            "arrowhead": false,
            "color": "#111827"
        }),
    )?;
    app.execute_command(
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-eff",
            "kind": "line",
            "start_pt": [298.0, 288.0],
            "end_pt": [298.0, 296.0],
            "arrowhead": false,
            "color": "#111827"
        }),
    )?;
    app.execute_command(
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-eff",
            "kind": "text",
            "text": "20 mm",
            "bounds_pt": [251.3, 276.0, 44.0, 14.0],
            "bordered": false
        }),
    )?;

    // Double-column 174 mm x 120 mm figure page (~493.2 x 340.2 pt).
    app.execute_command(
        "layout.create",
        json!({
            "sheet_id": "fig-pub",
            "title": "Figure 1 - Assay Summary",
            "preset": "double_column",
            "grid_template": [1, 2],
            "align_plot_origins": true,
            "panel_badges": "uppercase"
        }),
    )?;
    app.execute_command(
        "layout.addPanel",
        json!({
            "sheet_id": "fig-pub",
            "kind": "graph",
            "graph_sheet_id": "g-dose",
            "bounds_pt": [18.0, 22.0, 224.0, 220.0],
            "badge_override": "A"
        }),
    )?;
    app.execute_command(
        "layout.addPanel",
        json!({
            "sheet_id": "fig-pub",
            "kind": "graph",
            "graph_sheet_id": "g-eff",
            "bounds_pt": [254.0, 22.0, 224.0, 220.0],
            "badge_override": "B"
        }),
    )?;
    app.execute_command(
        "layout.addPanel",
        json!({
            "sheet_id": "fig-pub",
            "kind": "table",
            "source_sheet_id": "col-eff",
            "bounds_pt": [18.0, 254.0, 460.0, 70.0]
        }),
    )?;

    app.execute_command("sheet.select", json!({ "sheet_id": "fig-pub" }))?;
    let doc_snap = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc_snap, &SheetId::new("fig-pub"));
    dispatch_ui_command(&mut app, "ui.rulers.toggle", &json!({ "visible": true, "unit": "mm" })).map_err(std::io::Error::other)?;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.family_open = true;
    app.split.enabled = false;
    app.canvas_dialogs.status_banner = Some(String::from("Double Column 174 mm | Rulers: mm | Scale bar: 20 mm"));
    app.sync_views();

    Ok(render_png_bytes(&app)?)
}
