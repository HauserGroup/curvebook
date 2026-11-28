#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests on native `egui`:
//! inline editing & raw paste, `MultipleVariables` Data-tab variable picker,
//! and Portfolio Gallery / `F6` visible-pane cycling / quick-stats footer.

use std::sync::Arc;

use crate::common::{TestRes, cell_number, ensure, replace_focused_text};
use crate::human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport};
use crate::suite_helpers::{add_init_graph, assert_widgets, build_mv_sheet, new_shell_app};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec, GraphKind, InfoSheet,
    LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype, VariableRole, VariableSource,
    XyTable,
};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, VariableRoleFilter};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{ScreenshotRegionTarget, StubPlatformClipboardAdapter, UiApp};
use egui::{Key, Modifiers};

fn build_inline_edit_workspace() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Inline Edit & Paste").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = "Vehicle".into();
    xy.data_sets[1].title = "Drug".into();
    xy.row_titles[0] = "Baseline".into();
    xy.set_x_cell(0, Cell::number(1.0)).map_err(|e| e.to_string())?;
    xy.set_x_cell(1, Cell::number(2.0)).map_err(|e| e.to_string())?;
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[1].cells[0][0] = Cell::number(20.0);
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("col-2", "Secondary Cohort", TableArchetype::Column(col)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("xy-1")).map_err(|e| e.to_string())?;
    Ok(new_shell_app(doc))
}

#[test]
fn r10_23_inline_edit_grid_x_row_header_sheet_title_escape_and_cmd_v_paste() -> TestRes {
    let mut app = build_inline_edit_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());
    let mut h = EguiWfHarness::from_app(app, default_viewport());
    let initial_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    // 1. `GridXCell` double-click + live textbox check + Enter commit.
    h.dbl_click_widget("grid.xcell.0.0")?;
    replace_focused_text(&mut h.driver, "7.5")?;
    let x_live = h.driver.frame_meta().entries.iter().rfind(|e| e.widget_id == "grid.xcell.0.0").ok_or("missing live grid.xcell.0.0")?;
    assert_eq!((x_live.role.as_str(), x_live.value.as_ref().and_then(|v| v.as_str())), ("textbox", Some("7.5")));
    h.driver.press_key("Enter", KeyModifiers::default())?;
    let xy_id = SheetId::from("xy-1");
    assert_eq!(h.app().document().data_sheet(&xy_id).and_then(|s| s.x_or_time_cell(0, 0)).and_then(Cell::active_number), Some(7.5));
    assert_ne!(h.screenshot_png(ScreenshotRegionTarget::FullWindow)?, initial_png);

    // 2. Row title inline edit (`grid.row_title.0`) -> Escape cancel -> Enter commit.
    let row_title_0 = |app: &UiApp| {
        app.document().data_sheet(&xy_id).and_then(|s| match &s.archetype {
            TableArchetype::Xy(xy) => xy.row_titles.first().cloned(),
            _ => None,
        })
    };
    h.dbl_click_widget("grid.row_title.0")?;
    h.driver.type_text("TempCancel")?;
    h.driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(row_title_0(h.app()).as_deref(), Some("Baseline"));

    h.type_replace_cell("grid.row_title.0", "Stimulated")?;
    assert_eq!(row_title_0(h.app()).as_deref(), Some("Stimulated"));

    // 3. Navigator sheet title rename (`nav.sheet.xy-1` -> `F2` -> `Escape` cancel -> `Enter` commit).
    h.click("nav.sheet.xy-1")?;
    h.driver.press_egui_key(Key::F2, Modifiers::NONE);
    assert_widgets(&h.driver, &["nav.rename_input"])?;
    h.driver.type_text(" Draft")?;
    h.driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(h.app().document().data_sheet(&xy_id).map(|s| s.title.as_str()), Some("Dose Response"));

    h.click("nav.sheet.xy-1")?;
    h.driver.press_egui_key(Key::F2, Modifiers::NONE);
    replace_focused_text(&mut h.driver, "Assay B")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(h.app().document().data_sheet(&xy_id).map(|s| s.title.as_str()), Some("Assay B"));

    // 4. Raw `Cmd+V` & `Ctrl+V` TSV paste into grid + inline paste.
    clip_stub.seed_text("101.5\t202.5\n303.5\t404.5");
    h.click("grid.cell.1.0.0")?;
    h.driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let ds = h.app().document().data_sheet(&xy_id).ok_or("missing xy-1")?;
    for (r, c, exp) in [(1, 0, 101.5), (1, 1, 202.5), (2, 0, 303.5), (2, 1, 404.5)] {
        assert_eq!(cell_number(ds, r, c), Some(exp));
    }
    clip_stub.seed_text("555.25");
    h.click("grid.cell.4.0.0")?;
    h.driver.press_egui_key(Key::V, Modifiers::CTRL);
    assert_eq!(cell_number(h.app().document().data_sheet(&xy_id).ok_or("xy-1")?, 4, 0), Some(555.25));

    clip_stub.seed_text("88");
    h.dbl_click_widget("grid.cell.5.0.0")?;
    h.driver.dispatch_events(vec![egui::Event::Paste("88".into())]);
    h.driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(cell_number(h.app().document().data_sheet(&xy_id).ok_or("xy-1")?, 5, 0), Some(88.0));

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

fn build_mv_cohort_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 MultipleVariables Data-Tab Picker").map_err(|e| e.to_string())?;
    let ds = build_mv_sheet(
        "mv-1",
        "Patient Variables",
        &[
            ("PatientID", VariableRole::Label),
            ("Cohort", VariableRole::Categorical),
            ("Age", VariableRole::Continuous),
            ("Biomarker", VariableRole::Continuous),
            ("LogScore", VariableRole::Calculated),
        ],
        6,
        |mv| {
            mv.variables[4].source = VariableSource::Calculated { formula: "ln([Biomarker] + 1)".into() };
            for r in 0..6 {
                mv.variables[0].cells[r] = Cell::text(format!("P-0{r}"));
                mv.variables[1].cells[r] = Cell::text(if r < 3 { "Control" } else { "Treated" });
                mv.variables[2].cells[r] = Cell::number(32.0 + (r as f64) * 4.5);
                mv.variables[3].cells[r] = Cell::number(10.0 + (r as f64) * 3.0);
                mv.variables[4].cells[r] = Cell::number((11.0 + (r as f64) * 3.0).ln());
            }
        },
    )?;
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
            ..Default::default()
        }),
    )
    .map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("mv-1")).map_err(|e| e.to_string())?;
    Ok(new_shell_app(doc))
}

#[test]
fn r10_23_mv_data_tab_variable_picker_filter_badges_and_matrix_parity() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_mv_cohort_app()?, default_viewport());
    for idx in 0..=4 {
        assert_widgets(&h.driver, &[&format!("mv_formula.role_badge.{idx}")])?;
    }

    // 1. Analyze Data dialog MV role filter chips & variable toggles.
    h.click("toolbar.analyze")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "AnalyzeSheet should open")?;
    assert_widgets(
        &h.driver,
        &[
            "analyze.mv.filter.all",
            "analyze.mv.filter.continuous",
            "analyze.mv.filter.categorical",
            "analyze.mv.filter.label",
            "analyze.mv.filter.calculated",
            "analyze.mv.var.0",
            "analyze.mv.var.1",
            "analyze.mv.var.2",
            "analyze.mv.var.4",
        ],
    )?;
    h.click("analyze.mv.filter.calculated")?;
    assert_eq!(h.app().canvas_dialogs.analyze.role_filter, VariableRoleFilter::CalculatedOnly);
    assert_widgets(&h.driver, &["analyze.mv.var.4"])?;
    ensure(h.driver.frame_meta().find_by_widget_id("analyze.mv.var.1").is_none(), "Categorical col 1 must be filtered out")?;
    h.click("analyze.mv.filter.all")?;
    h.click("analyze.mv.var.2")?;
    assert!(!h.app().canvas_dialogs.analyze.selected_data_sets.contains(&2));
    h.click("modal.cancel")?;

    // 2. EditAnalysisParameters (`Cmd+T` on GLM result sheet) + 3 themes × 2 locales × 3 HiDPI matrix parity.
    h.select_nav_kind_filter("nav.chip.results")?;
    h.click("nav.sheet.res-glm")?;
    h.open_cmd_t()?;
    h.click("edit.tab.data")?;
    assert_widgets(&h.driver, &["edit.glm.resp.3", "edit.glm.pred.1", "edit.glm.pred.2"])?;
    h.click("edit.tab.experimental_design")?;
    assert_widgets(&h.driver, &["edit.glm.compute_vif"])?;

    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        h.app_mut().set_theme_mode(theme);
        for locale in ["en", "ar"] {
            assert_eq!(h.app_mut().set_locale(locale), locale);
            for scale in [1.0, 2.0, 3.0] {
                h.driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, scale));
                h.assert_parity_and_layout(&format!("MV picker theme={theme:?} locale={locale} scale={scale}"))?;
            }
        }
    }
    assert_input_to_paint_budget(&h, 50.0)
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
        doc.add_data_sheet(DataSheet::new(format!("ds-{i}"), format!("Dataset {i}"), TableArchetype::Column(col)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    for i in 0..2 {
        doc.add_info_sheet(InfoSheet::new_preseeded(format!("info-{i}"), format!("Protocol {i}")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let spec =
            AnalysisSpec::new(format!("res-{i}"), "ds-0", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
        doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let gid = add_init_graph(&mut doc, &SheetId::from("ds-0"), &format!("graph-{i}"), GraphKind::Column)?;
        if let Some(gs) = doc.graph_sheet_mut(&gid) {
            gs.title = format!("Graph {i}");
        }
        doc.add_layout_sheet(
            LayoutSheet::new(format!("layout-{i}"), format!("Figure Layout {i}"), LayoutSpec::default()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    doc.select_in_navigator(&SheetId::from("ds-0")).map_err(|e| e.to_string())?;
    Ok(new_shell_app(doc))
}

#[test]
fn r10_23_portfolio_gallery_f6_visible_pane_cycling_and_quick_stats_footer() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_multi_sheet_gallery_app()?, ViewportMetrics::new(1280.0, 720.0, 1.0));

    // 1. Portfolio Gallery toggle via View > Gallery, filter Graphs, return to All, and click layout-1 card.
    ensure(h.driver.frame_meta().find_by_widget_id("status.section.gallery").is_none(), "footer Gallery chip must be removed")?;
    h.click("menu.view")?;
    h.click("menu.ui.gallery.toggle")?;
    assert!(h.app().nav.gallery.open);
    assert_widgets(
        &h.driver,
        &[
            "gallery.close",
            "gallery.filter.all",
            "gallery.filter.data",
            "gallery.filter.info",
            "gallery.filter.results",
            "gallery.filter.graphs",
            "gallery.filter.layouts",
        ],
    )?;
    h.click("gallery.filter.graphs")?;
    assert_eq!(h.app().nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    assert_widgets(&h.driver, &["gallery.card.graph-0"])?;
    ensure(h.driver.frame_meta().find_by_widget_id("gallery.card.layout-1").is_none(), "layout-1 must be filtered out")?;
    h.click("gallery.filter.all")?;
    h.click("gallery.card.layout-1")?;
    assert!(!h.app().nav.gallery.open && h.app().document().active_sheet_id.as_ref().map(SheetId::as_str) == Some("layout-1"));

    // 2. Visible-pane `F6` cycling + skipping hidden Family pane.
    h.select_sheet(&SheetId::from("ds-0"))?;
    h.app_mut().nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    h.driver.run_settled();
    let mut visited = Vec::new();
    for _ in 0..5 {
        h.driver.press_egui_key(Key::F6, Modifiers::NONE);
        visited.push(h.app().nav.active_pane);
    }
    for pane in [ActiveWorkspacePane::Inspector, ActiveWorkspacePane::Navigator, ActiveWorkspacePane::FamilyStrip] {
        ensure(visited.contains(&pane), format!("F6 must visit {pane:?}"))?;
    }
    h.driver.press_egui_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    h.driver.press_egui_key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!h.app().family_open);
    for _ in 0..5 {
        h.driver.press_egui_key(Key::F6, Modifiers::NONE);
        ensure(h.app().nav.active_pane != ActiveWorkspacePane::FamilyStrip, "F6 must skip hidden FamilyStrip")?;
    }

    // 3. Drag-select cells (0,0)..(3,0) (`10, 20, 30, 40`) -> verify quick-stats footer.
    let (c0, c3) = (h.widget_center("grid.cell.0.0.0")?, h.widget_center("grid.cell.3.0.0")?);
    h.driver.drag(c0, c3)?;
    let status_lbl = h.driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?.label.clone();
    for expected in ["N=4", "Mean=25", "Median=25", "SD=12.9", "SEM=6.45"] {
        ensure(status_lbl.contains(expected), format!("status.summary missing `{expected}` in `{status_lbl}`"))?;
    }
    h.assert_parity_and_layout("gallery/F6/footer")?;
    assert_input_to_paint_budget(&h, 50.0)
}
