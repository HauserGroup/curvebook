#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests migrated cleanly onto native `egui`:
//! - Bullet 3 (**Inline editing & raw paste**): `GridXCell`, `grid.row_title.*`, Navigator sheet
//!   title (`nav.sheet.*` -> `F2` -> `sheet.rename`), raw `Escape` cancellation, and raw `Cmd/Ctrl+V`
//!   TSV/text paste via [`StubPlatformClipboardAdapter`].
//! - Bullet 5 (**`MultipleVariables` Data-tab variable picker**): Role filter chips and role badges
//!   (`NUM`, `CAT`, `LBL`, `FX=`) in `AnalyzeData` and `EditAnalysisParameters`, plus Light/Dark/
//!   HighContrast × LTR/RTL × `1.0x`/`2.0x`/`3.0x` HiDPI parity.
//! - Bullet 6 (**Gallery, `F6`/`Shift+F6` pane cycling & quick-stats footer**): `PortfolioGallery`,
//!   `F6`/`Shift+F6` visible-pane focus cycling, and drag-select quick-stats footer.

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec, GraphKind,
    GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, MultipleVariablesTable, ProjectDocument, ResultSheet, SheetFolderKind, SheetId, SubcolumnMode,
    TableArchetype, VariableRole, VariableSource, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, VariableRoleFilter};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{ScreenshotRegionTarget, StubPlatformClipboardAdapter, UiApp};
use egui::{Key, Modifiers};

fn check(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
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
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

#[test]
fn r10_23_inline_edit_grid_x_row_header_sheet_title_escape_and_cmd_v_paste() -> Result<(), String> {
    let mut app = build_inline_edit_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());

    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, crate::human_wf_common::default_viewport());
    let initial_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    // 1. `GridXCell` double-click + inline edit + Enter commit.
    let x_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.xcell.0.0")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing grid.xcell.0.0"))?;
    harness.driver.click_at(x_pos)?;
    harness.driver.click_at(x_pos)?;
    for _ in 0..8 {
        harness.driver.press_key("Backspace", KeyModifiers::default())?;
    }
    harness.driver.type_text("7.5")?;
    let x_live = harness
        .driver
        .frame_meta()
        .entries
        .iter()
        .rfind(|e| e.widget_id == "grid.xcell.0.0")
        .ok_or_else(|| String::from("missing live grid.xcell.0.0 editor"))?;
    assert_eq!(x_live.role, "textbox", "active inline X cell editor must report textbox role");
    assert_eq!(x_live.value.as_ref().and_then(|v| v.as_str()), Some("7.5"));
    harness.driver.press_key("Enter", KeyModifiers::default())?;
    let x_val = harness.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.x_or_time_cell(0, 0)).and_then(Cell::active_number);
    assert_eq!(x_val, Some(7.5), "committed X cell edit must persist to engine document");
    let committed_x_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(committed_x_png, initial_png, "committed X cell edit must repaint framebuffer");

    // 2. Row title inline edit (`grid.row_title.0`) -> Escape cancel -> Enter commit.
    let rt_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.row_title.0")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing grid.row_title.0"))?;
    harness.driver.click_at(rt_pos)?;
    harness.driver.click_at(rt_pos)?;
    harness.driver.type_text("TempCancel")?;
    harness.driver.press_key("Escape", KeyModifiers::default())?;
    let rt0_esc = harness.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| match &s.archetype {
        TableArchetype::Xy(xy) => xy.row_titles.first().cloned(),
        _ => None,
    });
    assert_eq!(rt0_esc.as_deref(), Some("Baseline"), "Escape must leave row header unmutated");

    let rt_pos2 = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.row_title.0")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing grid.row_title.0"))?;
    harness.driver.click_at(rt_pos2)?;
    harness.driver.click_at(rt_pos2)?;
    harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    harness.driver.press_key("Backspace", KeyModifiers::default())?;
    harness.driver.type_text("Stimulated")?;
    harness.driver.press_key("Enter", KeyModifiers::default())?;
    let rt0_committed = harness.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| match &s.archetype {
        TableArchetype::Xy(xy) => xy.row_titles.first().cloned(),
        _ => None,
    });
    assert_eq!(rt0_committed.as_deref(), Some("Stimulated"), "Enter must commit row header rename");

    // 3. Navigator sheet title rename (`nav.sheet.xy-1` -> `F2` -> `Escape` cancel -> `Enter` commit).
    harness.click("nav.sheet.xy-1")?;
    harness.driver.press_egui_key(Key::F2, Modifiers::NONE);
    check(harness.driver.frame_meta().find_by_widget_id("nav.rename_input").is_some(), "F2 must open nav.rename_input")?;
    harness.driver.type_text(" Draft")?;
    harness.driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(harness.app().document().data_sheet(&SheetId::from("xy-1")).map(|s| s.title.as_str()), Some("Dose Response"));

    harness.click("nav.sheet.xy-1")?;
    harness.driver.press_egui_key(Key::F2, Modifiers::NONE);
    harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    harness.driver.press_key("Backspace", KeyModifiers::default())?;
    harness.driver.type_text("Assay B")?;
    harness.driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(harness.app().document().data_sheet(&SheetId::from("xy-1")).map(|s| s.title.as_str()), Some("Assay B"));

    // 4. Raw `Cmd+V` & `Ctrl+V` TSV paste into grid + inline paste.
    clip_stub.seed_text("101.5\t202.5\n303.5\t404.5");
    harness.click("grid.cell.1.0.0")?;
    harness.driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let ds = harness.app().document().data_sheet(&SheetId::from("xy-1")).ok_or("missing xy-1")?;
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(101.5));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(202.5));
    assert_eq!(ds.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(303.5));
    assert_eq!(ds.cell(CellCoord::new(2, 1, 0)).and_then(Cell::active_number), Some(404.5));

    clip_stub.seed_text("555.25");
    harness.click("grid.cell.4.0.0")?;
    harness.driver.press_egui_key(Key::V, Modifiers::CTRL);
    assert_eq!(
        harness.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.cell(CellCoord::new(4, 0, 0))).and_then(Cell::active_number),
        Some(555.25)
    );

    clip_stub.seed_text("88");
    let c5_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.cell.5.0.0")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing grid.cell.5.0.0"))?;
    harness.driver.click_at(c5_pos)?;
    harness.driver.click_at(c5_pos)?;
    harness.driver.dispatch_events(vec![egui::Event::Paste(String::from("88"))]);
    harness.driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(
        harness.app().document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.cell(CellCoord::new(5, 0, 0))).and_then(Cell::active_number),
        Some(88.0)
    );

    crate::human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
    crate::human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
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
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

#[test]
fn r10_23_mv_data_tab_variable_picker_filter_badges_and_matrix_parity() -> Result<(), String> {
    let app = build_mv_cohort_app()?;
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, crate::human_wf_common::default_viewport());

    // Verify MV data-sheet role badge controls are rendered (`mv_formula.role_badge.0..4`).
    for idx in 0..=4 {
        let wid = format!("mv_formula.role_badge.{idx}");
        check(harness.driver.frame_meta().find_by_widget_id(&wid).is_some(), format!("missing `{wid}` on MV grid"))?;
    }

    // 1. Open Analyze Data dialog on MultipleVariables table and verify role filter chips & variable toggles.
    harness.click("toolbar.analyze")?;
    check(matches!(harness.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "AnalyzeSheet should open")?;
    for id in [
        "analyze.mv.filter.all",
        "analyze.mv.filter.continuous",
        "analyze.mv.filter.categorical",
        "analyze.mv.filter.label",
        "analyze.mv.filter.calculated",
        "analyze.mv.var.0",
        "analyze.mv.var.1",
        "analyze.mv.var.2",
        "analyze.mv.var.4",
    ] {
        check(harness.driver.frame_meta().find_by_widget_id(id).is_some(), format!("missing `{id}` in Analyze MV picker"))?;
    }

    harness.click("analyze.mv.filter.calculated")?;
    assert_eq!(harness.app().canvas_dialogs.analyze.role_filter, VariableRoleFilter::CalculatedOnly);
    check(harness.driver.frame_meta().find_by_widget_id("analyze.mv.var.4").is_some(), "Calculated col 4 must stay visible")?;
    check(harness.driver.frame_meta().find_by_widget_id("analyze.mv.var.1").is_none(), "Categorical col 1 must be filtered out")?;

    harness.click("analyze.mv.filter.all")?;
    harness.click("analyze.mv.var.2")?;
    assert!(!harness.app().canvas_dialogs.analyze.selected_data_sets.contains(&2));
    harness.click("modal.cancel")?;

    // 2. Open EditAnalysisParameters (`Cmd+T` on GLM result sheet) & check matrix parity across themes, locales & HiDPI.
    harness.click("nav.chip.results")?;
    harness.click("nav.sheet.res-glm")?;
    harness.open_cmd_t()?;
    harness.click("edit.tab.data")?;
    for id in ["edit.glm.resp.3", "edit.glm.pred.1", "edit.glm.pred.2"] {
        check(harness.driver.frame_meta().find_by_widget_id(id).is_some(), format!("missing `{id}` in GLM EditAnalysisParameters Data tab"))?;
    }
    harness.click("edit.tab.experimental_design")?;
    check(
        harness.driver.frame_meta().find_by_widget_id("edit.glm.compute_vif").is_some(),
        "missing `edit.glm.compute_vif` in GLM EditAnalysisParameters Model & Formula tab",
    )?;

    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        harness.app_mut().set_theme_mode(theme);
        for locale in ["en", "ar"] {
            assert_eq!(harness.app_mut().set_locale(locale), locale);
            for scale in [1.0, 2.0, 3.0] {
                harness.driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, scale));
                harness.assert_parity_and_layout(&format!("MV picker theme={theme:?} locale={locale} scale={scale}"))?;
            }
        }
    }
    crate::human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}

fn build_multi_sheet_gallery_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Portfolio Gallery & F6 Cycling").map_err(|e| e.to_string())?;
    for i in 0..4 {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
        for r in 0..4 {
            let v = f64::from((r + 1) as u32) * 10.0;
            col.data_sets[0].cells[r][0] = Cell::number(v);
            col.data_sets[1].cells[r][0] = Cell::number(v + 5.0);
        }
        let ds = DataSheet::new(format!("ds-{i}"), format!("Dataset {i}"), TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        let info = InfoSheet::new_preseeded(format!("info-{i}"), format!("Protocol {i}")).map_err(|e| e.to_string())?;
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
        gs.title = format!("Graph {i}");
        gs.initialized = true;
        gs.spec.kind = GraphKind::Column;
        doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        let ls = LayoutSheet::new(format!("layout-{i}"), format!("Figure Layout {i}"), LayoutSpec::default()).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;
    }
    doc.select_in_navigator(&SheetId::from("ds-0")).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

#[test]
fn r10_23_portfolio_gallery_f6_visible_pane_cycling_and_quick_stats_footer() -> Result<(), String> {
    let app = build_multi_sheet_gallery_app()?;
    let mut harness = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 720.0, 1.0));

    // 1. Open Portfolio Gallery via footer toggle, filter Graphs, return to All, and click layout-1 card.
    harness.click("status.section.gallery")?;
    assert!(harness.app().nav.gallery.open, "gallery must open");
    for id in [
        "gallery.close",
        "gallery.filter.all",
        "gallery.filter.data",
        "gallery.filter.info",
        "gallery.filter.results",
        "gallery.filter.graphs",
        "gallery.filter.layouts",
    ] {
        check(harness.driver.frame_meta().find_by_widget_id(id).is_some(), format!("missing `{id}` in gallery"))?;
    }

    harness.click("gallery.filter.graphs")?;
    assert_eq!(harness.app().nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    check(harness.driver.frame_meta().find_by_widget_id("gallery.card.graph-0").is_some(), "missing graph-0 card")?;
    check(harness.driver.frame_meta().find_by_widget_id("gallery.card.layout-1").is_none(), "layout-1 must be filtered out")?;

    harness.click("gallery.filter.all")?;
    harness.click("gallery.card.layout-1")?;
    assert!(!harness.app().nav.gallery.open, "clicking gallery card must close gallery");
    assert_eq!(harness.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    // 2. Visible-pane `F6` / `Shift+F6` cycling + skipping hidden Inspector/Family panes.
    harness.select_sheet(&SheetId::from("ds-0"))?;
    harness.app_mut().nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    harness.driver.run_settled();

    let mut visited = Vec::new();
    for _ in 0..5 {
        harness.driver.press_egui_key(Key::F6, Modifiers::NONE);
        visited.push(harness.app().nav.active_pane);
    }
    check(visited.contains(&ActiveWorkspacePane::Inspector), "F6 must visit Inspector")?;
    check(visited.contains(&ActiveWorkspacePane::Navigator), "F6 must visit Navigator")?;
    check(visited.contains(&ActiveWorkspacePane::FamilyStrip), "F6 must visit FamilyStrip")?;

    harness.driver.press_egui_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!harness.app().family_open);
    for _ in 0..5 {
        harness.driver.press_egui_key(Key::F6, Modifiers::NONE);
        check(
            harness.app().nav.active_pane != ActiveWorkspacePane::FamilyStrip,
            format!("F6 must skip hidden FamilyStrip, got {:?}", harness.app().nav.active_pane),
        )?;
    }

    // 3. Drag-select cells (0,0)..(3,0) (`10, 20, 30, 40`) -> verify full quick-stats footer.
    let c0 = harness.driver.frame_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
    let c3 = harness.driver.frame_meta().find_by_widget_id("grid.cell.3.0.0").ok_or("missing grid.cell.3.0.0")?.clipped_rect.center();
    harness.driver.drag(c0, c3)?;
    let status_lbl = harness.driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?.label.clone();
    for expected in ["N=4", "Mean=25", "Med=25", "SD=12.9", "SEM=6.45"] {
        check(status_lbl.contains(expected), format!("status.summary missing `{expected}` in `{status_lbl}`"))?;
    }

    harness.assert_parity_and_layout("gallery/F6/footer")?;
    crate::human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}
