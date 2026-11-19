#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests for:
//! - Bullet 3 (**Inline editing & raw paste**): `GridXCell`, `grid.row_title.*`, and Navigator sheet
//!   title (`nav.sheet.*` -> `sheet.rename`) inline editing via raw pointer/double-click +
//!   keyboard/IME input, raw `Escape` cancellation without mutation, live framebuffer buffer
//!   updates, and raw `Cmd/Ctrl+V` TSV/text paste via [`StubPlatformClipboardAdapter`].
//! - Bullet 5 (**`MultipleVariables` Data-tab variable picker**): Role filter chips
//!   (`All`, `Continuous`, `Categorical`, `Label`, `Calculated`) and colour-coded role badges
//!   (`NUM`, `CAT`, `LBL`, `FX=`) rendered in both `EditAnalysisParameters` (`stat_params`) and
//!   `AnalyzeData` (`analyze_picker`), verified with `verify_pixel_to_hit_box_parity` across themes
//!   (`Light`, `Dark`, `HighContrast`), RTL (`ar`, `he`), and HiDPI (`1.0x`, `2.0x`, `3.0x`).
//! - Bullet 6 (**Gallery, `F6`/`Shift+F6` pane cycling & quick-stats footer**): drawn, scrollable,
//!   folder-filterable, single-card-clickable `PortfolioGalleryState`, raw `F6` / `Shift+F6`
//!   visible-pane focus cycling, and inline drag-selection quick-stats footer
//!   (`n`, `mean`, `SD`, `SEM`, `median`, `sum`).

mod human_wf_common;

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec, GraphKind,
    GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, MultipleVariablesTable, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableArchetype,
    VariableRole, VariableSource, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, VariableRoleFilter};
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{InlineEditorTarget, KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{StubPlatformClipboardAdapter, UiApp};
use serde_json::json;

fn check(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn rect_pixels_slice(rgba: &[u8], img_w: u32, rect: curvebook_ui_egui::layout::LayoutRectPx) -> Vec<u8> {
    let mut out = Vec::new();
    let x0 = usize::try_from(rect.x.max(0)).unwrap_or(0);
    let y0 = usize::try_from(rect.y.max(0)).unwrap_or(0);
    let x1 = usize::try_from(rect.right().max(0)).unwrap_or(0).min(img_w as usize);
    let y1 = usize::try_from(rect.bottom().max(0)).unwrap_or(0);
    let stride = (img_w as usize) * 4;
    for y in y0..y1 {
        let row_start = y.saturating_mul(stride);
        if row_start >= rgba.len() {
            break;
        }
        let s = row_start.saturating_add(x0.saturating_mul(4)).min(rgba.len());
        let e = row_start.saturating_add(x1.saturating_mul(4)).min(rgba.len());
        if s < e {
            out.extend_from_slice(&rgba[s..e]);
        }
    }
    out
}

fn build_inline_edit_workspace() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Inline Edit & Paste").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Drug");
    xy.row_titles[0] = String::from("Baseline");
    xy.set_x_cell(0, Cell::number(1.0)).map_err(|e| e.to_string())?;
    xy.set_x_cell(1, Cell::number(2.0)).map_err(|e| e.to_string())?;
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[1].cells[0][0] = Cell::number(20.0);
    let ds = DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    let ds2 = DataSheet::new("col-2", "Secondary Cohort", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds2).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("xy-1")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

fn exercise_grid_x_inline_edit(driver: &mut HumanPathDriver, frame_initial: &[u8]) -> Result<(), String> {
    let [x0_lx, x0_ly] = human_wf_common::widget_hit_center(driver, &WidgetId::from("grid.xcell.0.0"))?;
    let x_begin = driver.double_click_logical(x0_lx, x0_ly)?;
    assert!(x_begin.handled);
    assert!(matches!(driver.app().inline_editor.target, Some(InlineEditorTarget::GridXCell { row: 0, subcol: 0, .. })));

    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    driver.set_ime_preedit(Some(String::from("7.")));
    let _ = driver.type_text("7.5")?;
    driver.set_ime_preedit(None);
    let x_live_tree = driver.layout_tree();
    let x_live_node = x_live_tree.node("grid.xcell.0.0").ok_or("missing grid.xcell.0.0 during live edit")?;
    assert_eq!(x_live_node.a11y.value_text.as_deref(), Some("7.5"));
    let frame_during_x_edit = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_during_x_edit.rgba, frame_initial, "live inline X edit buffer must paint in framebuffer");

    let x_commit = driver.press_key("Enter", KeyModifiers::default())?;
    assert!(x_commit.handled);
    assert!(!driver.app().inline_editor.active);
    let x_val = driver.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.x_or_time_cell(0, 0)).and_then(Cell::active_number);
    assert_eq!(x_val, Some(7.5), "committed X cell edit must persist to engine document");
    Ok(())
}

fn exercise_row_header_inline_edit(driver: &mut HumanPathDriver) -> Result<(), String> {
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("grid.row_title.0"))?;
    assert!(driver.app().view_shards.grid_core.editing_title.is_some());
    let _ = driver.type_text("TempCancel")?;
    let esc_row = driver.press_key("Escape", KeyModifiers::default())?;
    assert!(esc_row.handled, "raw KeyDown(Escape) must cancel active row title editor");
    assert!(driver.app().view_shards.grid_core.editing_title.is_none());
    let rt0_after_esc = driver.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| match &s.archetype {
        TableArchetype::Xy(xy) => xy.row_titles.first().cloned(),
        _ => None,
    });
    assert_eq!(rt0_after_esc.as_deref(), Some("Baseline"), "Escape must leave row header unmutated");

    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("grid.row_title.0"))?;
    let _ = driver.press_key("a", KeyModifiers { meta: true, ..Default::default() })?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let _ = driver.type_text("Stimulated")?;
    let enter_row = driver.press_key("Enter", KeyModifiers::default())?;
    assert!(enter_row.handled);
    assert!(driver.app().view_shards.grid_core.editing_title.is_none());
    let rt0_committed = driver.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| match &s.archetype {
        TableArchetype::Xy(xy) => xy.row_titles.first().cloned(),
        _ => None,
    });
    assert_eq!(rt0_committed.as_deref(), Some("Stimulated"), "Enter must commit row header rename");
    Ok(())
}

fn exercise_sheet_title_inline_edit(driver: &mut HumanPathDriver) -> Result<(), String> {
    let [nav_lx, nav_ly] = human_wf_common::widget_hit_center(driver, &WidgetId::from("nav.sheet.xy-1"))?;
    let nav_begin = driver.double_click_logical(nav_lx, nav_ly)?;
    assert!(nav_begin.handled);
    assert!(matches!(driver.app().inline_editor.target, Some(InlineEditorTarget::SheetTitle { ref sheet_id }) if sheet_id.as_str() == "xy-1"));

    let nav_rect = driver.layout_tree().node("nav.sheet.xy-1").ok_or("missing nav.sheet.xy-1")?.bounds.visible_rect_px;
    let nav_before_type = rect_pixels_slice(&driver.render_frame().map_err(|e| e.to_string())?.rgba, 1280, nav_rect);
    let _ = driver.type_text(" Draft")?;
    let nav_during_type = rect_pixels_slice(&driver.render_frame().map_err(|e| e.to_string())?.rgba, 1280, nav_rect);
    assert_ne!(nav_before_type, nav_during_type, "typing in Navigator sheet title editor must update live pixels");

    let esc_sheet = driver.press_key("Escape", KeyModifiers::default())?;
    assert!(esc_sheet.handled);
    assert!(!driver.app().inline_editor.active);
    assert_eq!(
        driver.app().document().data_sheet(&SheetId::from("xy-1")).map(|s| s.title.as_str()),
        Some("Dose Response"),
        "Escape must cancel Navigator sheet title edit without mutation"
    );

    let [nav_lx2, nav_ly2] = human_wf_common::widget_hit_center(driver, &WidgetId::from("nav.sheet.xy-1"))?;
    let _ = driver.double_click_logical(nav_lx2, nav_ly2)?;
    for _ in 0.."Dose Response".chars().count() {
        let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    }
    let _ = driver.type_text("Assay B")?;
    let nav_commit = driver.press_key("Enter", KeyModifiers::default())?;
    assert!(nav_commit.handled);
    assert_eq!(nav_commit.command_id.as_deref(), Some("sheet.rename"));
    assert_eq!(
        driver.app().document().data_sheet(&SheetId::from("xy-1")).map(|s| s.title.as_str()),
        Some("Assay B"),
        "Enter must commit Navigator sheet title edit via sheet.rename"
    );
    Ok(())
}

fn exercise_raw_cmd_ctrl_v_paste(driver: &mut HumanPathDriver, clip_stub: &StubPlatformClipboardAdapter) -> Result<(), String> {
    clip_stub.seed_text("101.5\t202.5\n303.5\t404.5");
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("grid.cell.1.0.0"))?;
    let cmd_v = human_wf_common::press_cmd_shortcut(driver, "v")?;
    assert!(cmd_v.handled);
    assert_eq!(cmd_v.command_id.as_deref(), Some("table.paste"));
    let ds = driver.app().document().data_sheet(&SheetId::from("xy-1")).ok_or("missing xy-1")?;
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(101.5));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(202.5));
    assert_eq!(ds.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(303.5));
    assert_eq!(ds.cell(CellCoord::new(2, 1, 0)).and_then(Cell::active_number), Some(404.5));

    clip_stub.seed_text("555.25");
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("grid.cell.4.0.0"))?;
    let ctrl_v = driver.press_key("v", KeyModifiers { ctrl: true, ..Default::default() })?;
    assert!(ctrl_v.handled);
    assert_eq!(
        driver.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.cell(CellCoord::new(4, 0, 0))).and_then(Cell::active_number),
        Some(555.25)
    );

    clip_stub.seed_text("88");
    let [c5_lx, c5_ly] = human_wf_common::widget_hit_center(driver, &WidgetId::from("grid.cell.5.0.0"))?;
    let _ = driver.double_click_logical(c5_lx, c5_ly)?;
    let inline_v = driver.press_key("v", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(inline_v.handled);
    assert_eq!(driver.app().inline_editor.buffer, "88");
    let _ = driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(
        driver.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.cell(CellCoord::new(5, 0, 0))).and_then(Cell::active_number),
        Some(88.0)
    );
    Ok(())
}

#[test]
fn r10_23_inline_edit_grid_x_row_header_sheet_title_escape_and_cmd_v_paste() -> Result<(), String> {
    let mut app = build_inline_edit_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());

    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    let frame_initial = driver.render_frame().map_err(|e| e.to_string())?;

    exercise_grid_x_inline_edit(&mut driver, &frame_initial.rgba)?;
    exercise_row_header_inline_edit(&mut driver)?;
    exercise_sheet_title_inline_edit(&mut driver)?;
    exercise_raw_cmd_ctrl_v_paste(&mut driver, clip_stub.as_ref())?;

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

fn build_mv_cohort_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 MultipleVariables Data-Tab Picker").map_err(|e| e.to_string())?;
    let mut mv = MultipleVariablesTable::new(5, 6).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("PatientID");
    mv.variables[0].role = VariableRole::Label;
    mv.variables[0].role_inferred = false;
    mv.variables[1].title = String::from("Cohort");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[1].role_inferred = false;
    mv.variables[2].title = String::from("Age");
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[2].role_inferred = false;
    mv.variables[3].title = String::from("Biomarker");
    mv.variables[3].role = VariableRole::Continuous;
    mv.variables[3].role_inferred = false;
    mv.variables[4].title = String::from("LogScore");
    mv.variables[4].role = VariableRole::Calculated;
    mv.variables[4].role_inferred = false;
    mv.variables[4].source = VariableSource::Calculated { formula: String::from("ln([Biomarker] + 1)") };

    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::text(format!("P-0{r}"));
        mv.variables[1].cells[r] = Cell::text(if r < 3 { "Control" } else { "Treated" });
        mv.variables[2].cells[r] = Cell::number(32.0 + (r as f64) * 4.5);
        mv.variables[3].cells[r] = Cell::number(10.0 + (r as f64) * 3.0);
        mv.variables[4].cells[r] = Cell::number((11.0 + (r as f64) * 3.0).ln());
    }
    let ds = DataSheet::new("mv-1", "Patient Variables", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    let spec = AnalysisSpec::new(
        "res-glm",
        "mv-1",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 3,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .map_err(|e| e.to_string())?;
    let rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("mv-1")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn r10_23_mv_data_tab_variable_picker_filter_badges_and_matrix_parity() -> Result<(), String> {
    let app = build_mv_cohort_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 1. Open Analyze Data dialog on MultipleVariables table and exercise role filter + colour-coded badges.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    check(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "AnalyzeSheet should open")?;
    let _ = driver.render_frame().map_err(|e| e.to_string())?;

    let an_tree = driver.layout_tree();
    for id in [
        "analyze.mv.filter.all",
        "analyze.mv.filter.continuous",
        "analyze.mv.filter.categorical",
        "analyze.mv.filter.label",
        "analyze.mv.filter.calculated",
        "analyze.mv.var.0.role_badge",
        "analyze.mv.var.1.role_badge",
        "analyze.mv.var.2.role_badge",
        "analyze.mv.var.4.role_badge",
    ] {
        check(an_tree.node(id).is_some_and(|n| n.drawn_in_frame), format!("missing `{id}` in Analyze MV picker"))?;
    }
    assert_eq!(an_tree.node("analyze.mv.var.0.role_badge").map(|n| n.a11y.label.as_str()), Some("LBL"));
    assert_eq!(an_tree.node("analyze.mv.var.1.role_badge").map(|n| n.a11y.label.as_str()), Some("CAT"));
    assert_eq!(an_tree.node("analyze.mv.var.2.role_badge").map(|n| n.a11y.label.as_str()), Some("NUM"));
    assert_eq!(an_tree.node("analyze.mv.var.4.role_badge").map(|n| n.a11y.label.as_str()), Some("FX="));

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("analyze.mv.filter.calculated"))?;
    assert_eq!(driver.app().canvas_dialogs.analyze.role_filter, VariableRoleFilter::CalculatedOnly);
    let calc_tree = driver.layout_tree();
    check(calc_tree.node("analyze.mv.var.4").is_some(), "Calculated variable col 4 must be visible")?;
    check(calc_tree.node("analyze.mv.var.1").is_none(), "Categorical variable col 1 must be filtered out")?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("analyze.mv.filter.all"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("analyze.mv.var.2"))?;
    assert!(!driver.app().canvas_dialogs.analyze.selected_data_sets.contains(&2));
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;

    // 2. Open EditAnalysisParameters (`Cmd+T` on result sheet) and test the Data-tab / Design-tab MV variable picker.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "res-glm" })).map_err(|e| e.to_string())?;
    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    check(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "EditAnalysisParameters should open")?;

    let ep_tree = driver.layout_tree();
    for id in [
        "edit.mv.filter.all",
        "edit.mv.filter.continuous",
        "edit.mv.filter.categorical",
        "edit.mv.filter.label",
        "edit.mv.filter.calculated",
        "edit.dataset.0",
        "edit.dataset.0.role_badge",
        "edit.dataset.1",
        "edit.dataset.1.role_badge",
        "edit.dataset.2",
        "edit.dataset.2.role_badge",
        "edit.dataset.4",
        "edit.dataset.4.role_badge",
    ] {
        check(ep_tree.node(id).is_some_and(|n| n.drawn_in_frame), format!("missing `{id}` in stat_params MV variable picker"))?;
    }
    assert_eq!(ep_tree.node("edit.dataset.0.role_badge").map(|n| n.a11y.label.as_str()), Some("LBL"));
    assert_eq!(ep_tree.node("edit.dataset.1.role_badge").map(|n| n.a11y.label.as_str()), Some("CAT"));
    assert_eq!(ep_tree.node("edit.dataset.2.role_badge").map(|n| n.a11y.label.as_str()), Some("NUM"));
    assert_eq!(ep_tree.node("edit.dataset.4.role_badge").map(|n| n.a11y.label.as_str()), Some("FX="));

    // Role filters via raw pointer clicks.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.mv.filter.continuous"))?;
    let cont_tree = driver.layout_tree();
    check(cont_tree.node("edit.dataset.2").is_some() && cont_tree.node("edit.dataset.3").is_some(), "Continuous vars expected")?;
    check(cont_tree.node("edit.dataset.0").is_none() && cont_tree.node("edit.dataset.1").is_none(), "Non-continuous vars must be hidden")?;

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.mv.filter.categorical"))?;
    let cat_tree = driver.layout_tree();
    check(cat_tree.node("edit.dataset.1").is_some() && cat_tree.node("edit.dataset.2").is_none(), "Only categorical var expected")?;

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.mv.filter.label"))?;
    let lbl_tree = driver.layout_tree();
    check(lbl_tree.node("edit.dataset.0").is_some() && lbl_tree.node("edit.dataset.4").is_none(), "Only label var expected")?;

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.mv.filter.calculated"))?;
    let fx_tree = driver.layout_tree();
    check(fx_tree.node("edit.dataset.4").is_some() && fx_tree.node("edit.dataset.0").is_none(), "Only calculated var expected")?;

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.mv.filter.all"))?;
    // Toggle variable 2 off via raw pointer click and verify draft updates cleanly.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.dataset.2"))?;
    let selected = driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| d.selected_data_sets.clone()).unwrap_or_default();
    assert!(!selected.contains(&2), "toggling edit.dataset.2 must exclude column 2");
    assert_eq!(selected, vec![0, 1, 3, 4]);

    // 3. Matrix verification: verify_pixel_to_hit_box_parity across Light/Dark/HighContrast, LTR/RTL, and 1x/2x/3x HiDPI.
    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        driver.app_mut().set_theme_mode(theme);
        for locale in ["en", "ar"] {
            let _ = driver.app_mut().set_locale(locale);
            for scale in [1.0, 2.0, 3.0] {
                driver.set_metrics(ViewportMetrics::new(1280.0, 800.0, scale));
                let _ = driver.render_frame().map_err(|e| e.to_string())?;
                let report = driver.verify_parity();
                check(
                    report.is_clean(),
                    format!("MV picker parity failed for theme={theme:?}, locale={locale}, scale={scale}: {:?}", report.violations),
                )?;
            }
        }
    }

    let _ = driver.app_mut().set_locale("en");
    driver.app_mut().set_theme_mode(UiThemeMode::Light);
    driver.set_metrics(human_wf_common::default_viewport());
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

fn build_multi_sheet_gallery_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Portfolio Gallery & F6").map_err(|e| e.to_string())?;
    for i in 0..4 {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).map_err(|e| e.to_string())?;
        col.data_sets[0].title = format!("Series A{i}");
        col.data_sets[1].title = format!("Series B{i}");
        let mut ds = DataSheet::new(format!("ds-{i}"), format!("Cohort Table {i}"), TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        if i == 0 {
            ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0)).map_err(|e| e.to_string())?;
            ds.set_cell(CellCoord::new(1, 0, 0), Cell::number(20.0)).map_err(|e| e.to_string())?;
            ds.set_cell(CellCoord::new(2, 0, 0), Cell::number(30.0)).map_err(|e| e.to_string())?;
            ds.set_cell(CellCoord::new(3, 0, 0), Cell::number(40.0)).map_err(|e| e.to_string())?;
        }
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        let info = InfoSheet::new_preseeded(format!("info-{i}"), format!("Protocol Info {i}")).map_err(|e| e.to_string())?;
        doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        let spec =
            AnalysisSpec::new(format!("res-{i}"), "ds-0", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
        let rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
        doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    }
    let ds0 = doc.data_sheet(&SheetId::from("ds-0")).ok_or("missing ds-0")?.clone();
    for i in 0..2 {
        let mut gs = GraphSheet::new_uninitialized_for_data(format!("graph-{i}"), &ds0).map_err(|e| e.to_string())?;
        gs.initialized = true;
        gs.title = format!("Cohort Graph {i}");
        gs.spec.kind = GraphKind::Column;
        doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        let ls = LayoutSheet::new(format!("layout-{i}"), format!("Figure Layout {i}"), LayoutSpec::default()).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;
    }
    doc.select_in_navigator(&SheetId::from("ds-0")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn r10_23_portfolio_gallery_f6_visible_pane_cycling_and_quick_stats_footer() -> Result<(), String> {
    let app = build_multi_sheet_gallery_app()?;
    let mut driver = human_wf_common::new_human_driver(app, ViewportMetrics::new(1280.0, 620.0, 1.0));
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 1. Open Portfolio Gallery via toolbar button -> verify drawn header, folder filter tabs, and scrollable cards.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("toolbar.open"))?;
    assert!(driver.app().nav.gallery.open, "toolbar Open button must open Portfolio Gallery");
    let gal_tree = driver.layout_tree();
    for id in [
        "gallery.root",
        "gallery.title",
        "gallery.close",
        "gallery.filter.all",
        "gallery.filter.data",
        "gallery.filter.info",
        "gallery.filter.results",
        "gallery.filter.graphs",
        "gallery.filter.layouts",
        "gallery.viewport",
    ] {
        check(gal_tree.node(id).is_some_and(|n| n.drawn_in_frame), format!("missing `{id}` in gallery LayoutTree"))?;
    }
    let _ = driver.render_frame().map_err(|e| e.to_string())?;

    // Filter to Graphs only -> verify 2 graph cards, then return to All.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("gallery.filter.graphs"))?;
    assert_eq!(driver.app().nav.gallery.cards.len(), 2);
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("gallery.filter.all"))?;
    assert_eq!(driver.app().nav.gallery.cards.len(), 12);

    // Wheel-scroll gallery viewport to reveal card 11 ("Figure Layout 1"), then single-click it to open that sheet.
    human_wf_common::scroll_until_widget_visible(&mut driver, &WidgetId::from("gallery.viewport"), &WidgetId::from("gallery.card.11"), 120.0, 8)?;
    let card_click = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("gallery.card.11"))?;
    assert!(card_click.handled);
    assert!(!driver.app().nav.gallery.open, "clicking a single gallery card must open sheet and close gallery");
    assert_eq!(driver.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    // 2. Test `F6` and `Shift+F6` visible-pane cycling across workspace.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "ds-0" })).map_err(|e| e.to_string())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    let f_main = driver.render_frame().map_err(|e| e.to_string())?;

    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::Inspector);
    let f_insp = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(f_main.rgba, f_insp.rgba, "F6 focus ring must repaint target pane");

    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::Navigator);
    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::FamilyStrip);
    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

    let _ = driver.press_key("F6", KeyModifiers { shift: true, ..Default::default() })?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::FamilyStrip);

    // Hide Inspector (`Cmd+Shift+I`) and Family (`Cmd+Shift+F`): F6 must skip hidden panes.
    let _ = driver.press_key("i", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    let _ = driver.press_key("f", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    assert!(!driver.app().nav.inspector_open);
    assert!(!driver.app().family_open);
    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::Navigator);
    let _ = driver.press_key("F6", KeyModifiers::default())?;
    assert_eq!(driver.app().nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

    // 3. Drag-select cells (0, 0) .. (3, 0) on `ds-0` (`10, 20, 30, 40`) -> verify full quick-stats footer in LayoutTree & pixels.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("grid.cell.0.0.0"))?;
    let footer_rect = driver.layout_tree().node("status.summary").ok_or("missing status.summary")?.bounds.visible_rect_px;
    let footer_before = rect_pixels_slice(&driver.render_frame().map_err(|e| e.to_string())?.rgba, 1280, footer_rect);

    let [start_x, start_y] = human_wf_common::widget_hit_center(&mut driver, &WidgetId::from("grid.cell.0.0.0"))?;
    let [end_x, end_y] = human_wf_common::widget_hit_center(&mut driver, &WidgetId::from("grid.cell.3.0.0"))?;
    let drag_out = driver.drag_logical(start_x, start_y, end_x, end_y)?;
    assert!(drag_out.handled);

    let status_lbl = driver.layout_tree().node("status.summary").ok_or("missing status.summary after drag")?.a11y.label.clone();
    for expected in ["n=4", "excl=0", "mean=25.000", "SD=12.910", "SEM=6.455", "median=25.000", "sum=100.000"] {
        check(status_lbl.contains(expected), format!("status.summary missing `{expected}` in `{status_lbl}`"))?;
    }
    let footer_after = rect_pixels_slice(&driver.render_frame().map_err(|e| e.to_string())?.rgba, 1280, footer_rect);
    assert_ne!(footer_before, footer_after, "drag-selecting multiple cells must repaint status.summary quick-stats footer");

    let parity = driver.verify_parity();
    check(parity.is_clean(), format!("gallery/F6/footer parity failed: {:?}", parity.violations))?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}
