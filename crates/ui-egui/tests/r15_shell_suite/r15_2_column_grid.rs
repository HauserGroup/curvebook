#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R15.2` Wave 2 Virtualized `Column` Data Grid integration tests.

use std::time::Instant;

use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind,
    LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::data_grid::{
    ColumnGridEguiState, ColumnGridFrameReport, ColumnGridRenderContext, render_column_grid, render_data_grid,
};
use curvebook_ui_egui::egui_shell::driver::{
    AccessKitRoleBounds, BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions,
    check_role_bounds_scrollbar_non_overlap, collect_tooltip_texts_from_accesskit,
};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::i18n::{is_rtl_locale, tr};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
use egui::epaint::{ClippedShape, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use crate::common::{TestRes, assert_clean_parity};

struct GridHarnessState {
    app: UiApp,
    grid: ColumnGridEguiState,
    last_report: ColumnGridFrameReport,
    multi_grid: bool,
    discard_once: bool,
    discarded_passes: usize,
    committed_engine_commands: Vec<(String, serde_json::Value)>,
    outside_clicks_blocked: usize,
}

impl GridHarnessState {
    fn new(app: UiApp, multi_grid: bool) -> Self {
        Self {
            app,
            grid: ColumnGridEguiState::default(),
            last_report: ColumnGridFrameReport::default(),
            multi_grid,
            discard_once: false,
            discarded_passes: 0,
            committed_engine_commands: Vec::new(),
            outside_clicks_blocked: 0,
        }
    }
}

fn widget_center<S>(driver: &EguiHumanPathDriver<'_, S>, widget_id: &str, require_enabled: bool) -> Result<egui::Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if require_enabled && !entry.enabled {
        return Err(format!("widget `{widget_id}` is disabled"));
    }
    Ok(entry.clipped_rect.center())
}

fn render_grid_harness(ui: &mut egui::Ui, meta: &mut FrameInspectMeta, state: &mut GridHarnessState) {
    if state.discard_once {
        state.discard_once = false;
        ui.ctx().request_discard("r15_2 test multi-pass");
    }
    let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
    tokens.apply_to_ctx(ui.ctx());
    let modal_blocked = state.app.canvas_dialogs.active_dialog != ActiveDialog::None;
    let mut intents = PendingUiIntents::new();
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
        let ctx = ColumnGridRenderContext::new(&tokens, modal_blocked);
        state.last_report = if state.multi_grid {
            render_data_grid(ui, &state.app, &mut state.grid, ctx, meta, &mut intents)
        } else {
            render_column_grid(ui, &state.app, &mut state.grid, ctx, meta, &mut intents)
        };
    });
    if state.last_report.outside_click_blocked {
        state.outside_clicks_blocked += 1;
    }
    let will_discard = ui.ctx().will_discard();
    if will_discard {
        state.discarded_passes += 1;
    } else if !state.multi_grid {
        for intent in intents.intents() {
            if let UiIntent::EngineCommand { command_id, params } = intent {
                state.committed_engine_commands.push((command_id.clone(), params.clone()));
            }
        }
    }
    intents.commit_if_settled(&mut state.app, will_discard).expect("commit settled intents");
}

fn make_grid_driver(app: UiApp, w: f64, h: f64, tokens: Tokens, multi_grid: bool) -> EguiHumanPathDriver<'static, GridHarnessState> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        GridHarnessState::new(app, multi_grid),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
        build_driver_font_definitions(),
        render_grid_harness,
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver
}

fn capture_grid_shapes(state: &mut GridHarnessState, width: f32, height: f32) -> Vec<ClippedShape> {
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let mut meta = FrameInspectMeta::new();
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() },
        |ui| {
            meta.begin_pass(ui.ctx());
            render_grid_harness(ui, &mut meta, state);
        },
    );
    let shapes = out.shapes.clone();
    out.drop_without_applying_deltas();
    shapes
}

fn build_column_doc(cols: usize, rows: usize, mode: SubcolumnMode) -> ProjectDocument {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, mode, cols, rows).expect("column table");
    let subs = mode.subcolumn_count();
    for c in 0..cols {
        col.data_sets[c].title = format!("Group {}", c + 1);
        for r in 0..rows {
            if c == 0 {
                col.row_titles[r] = format!("Subject {}", r + 1);
            }
            for s in 0..subs {
                col.data_sets[c].cells[r][s] = Cell::number(((r + 1) as f64) * 1.5 + (c as f64) * 0.25 + (s as f64) * 0.05);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("col-main", "Column Assay", TableArchetype::Column(col)).expect("ds")).expect("add");
    doc.select_in_navigator(&SheetId::from("col-main")).expect("select");
    doc
}

fn build_column_driver(cols: usize, rows: usize, mode: SubcolumnMode, w: f64, h: f64) -> EguiHumanPathDriver<'static, GridHarnessState> {
    make_grid_driver(UiApp::from_session(Session::from_document(build_column_doc(cols, rows, mode))), w, h, Tokens::default(), false)
}

fn make_xy_app(id: &str, title: &str, xs: &[f64], rows: &[&[f64]]) -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": id, "title": title, "kind": "xy", "cols": 2, "rows": rows.len(), "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setXCells", json!({ "sheet_id": id, "values": xs })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": id, "rows": rows })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": id })).map_err(|e| e.to_string())?;
    app.sync_views();
    Ok(app)
}

fn main_cell<'a>(driver: &'a EguiHumanPathDriver<'_, GridHarnessState>, row: usize, col: usize) -> &'a Cell {
    driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(row, col, 0)).unwrap()
}

fn has_strikethrough_italic_text(shapes: &[ClippedShape], needle: &str) -> bool {
    fn visit(shape: &Shape, needle: &str) -> bool {
        match shape {
            Shape::Text(ts) => {
                ts.galley.text().contains(needle)
                    && ts.galley.job.sections.iter().any(|sec| sec.format.italics && sec.format.strikethrough != egui::Stroke::NONE)
            }
            Shape::Vec(children) => children.iter().any(|c| visit(c, needle)),
            _ => false,
        }
    }
    shapes.iter().any(|cs| visit(&cs.shape, needle))
}

fn linked_region(id: &str, src_sheet: &str, src_range: SheetCellRange, target_range: SheetCellRange, summary: &str) -> LinkedDataRegion {
    LinkedDataRegion {
        id: id.to_owned(),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::from(src_sheet),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: src_range,
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range,
        provenance_summary: summary.to_owned(),
    }
}

#[test]
fn frozen_three_tier_headers_corner_trigger_and_row_titles_toggle() -> TestRes {
    let mut driver = build_column_driver(18, 80, SubcolumnMode::Single, 1280.0, 800.0);
    assert_clean_parity(&driver, "initial column grid")?;

    assert_eq!(driver.frame_meta().find_by_widget_id("grid.corner.archetype").ok_or("corner")?.clipped_rect.width(), 48.0);
    for (wid, expected_label) in [("grid.col_super.0", "Col A"), ("grid.col_title.0", "Group 1"), ("grid.subcol_title.0.0", "Y")] {
        let node = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing {wid}"))?;
        assert_eq!(node.label, expected_label);
        assert!((node.clipped_rect.height() - 24.0).abs() < 0.1);
    }
    assert!((driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("row_titles.band")?.clipped_rect.width() - 112.0).abs() < 0.5);

    driver.click_widget_id("grid.row_titles.toggle")?;
    assert!(driver.state().grid.row_titles_collapsed);
    assert!((driver.frame_meta().find_by_widget_id("grid.row_titles.toggle").ok_or("toggle")?.clipped_rect.width() - 34.0).abs() < 0.5);
    assert!(driver.frame_meta().find_by_widget_id("grid.row_title_collapsed.0").is_some());

    driver.click_widget_id("grid.row_titles.toggle")?;
    assert!(!driver.state().grid.row_titles_collapsed);
    assert!((driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("expanded band")?.clipped_rect.width() - 112.0).abs() < 0.5);

    let cell0_pos = widget_center(&driver, "grid.cell.2.1.0", true)?;
    let super_a_top = driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("super_a")?.clipped_rect.top();
    let row0_left = driver.frame_meta().find_by_widget_id("grid.row_num.0").ok_or("row_num.0")?.clipped_rect.left();
    driver.wheel_at(cell0_pos, egui::vec2(-500.0, -720.0))?;
    assert!(driver.state().grid.scroll_offset_pt.y > 200.0 && driver.state().grid.scroll_offset_pt.x > 100.0);
    assert!(driver.frame_meta().find_by_widget_id("grid.cell.35.6.0").is_some());
    assert!((driver.frame_meta().find_by_widget_id("grid.col_super.6").ok_or("col_super.6")?.clipped_rect.top() - super_a_top).abs() < 0.5);
    assert!((driver.frame_meta().find_by_widget_id("grid.row_num.35").ok_or("row_num.35")?.clipped_rect.left() - row0_left).abs() < 0.5);

    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));

    for (mode, expected) in [
        (SubcolumnMode::MeanSdN, ["Mean", "SD", "N"]),
        (SubcolumnMode::MeanSemN, ["Mean", "SEM", "N"]),
        (SubcolumnMode::MeanCvN, ["Mean", "%CV", "N"]),
    ] {
        let sub_drv = build_column_driver(3, 6, mode, 1024.0, 600.0);
        for (sub_idx, label) in expected.iter().enumerate() {
            let wid = format!("grid.subcol_title.0.{sub_idx}");
            assert_eq!(sub_drv.frame_meta().find_by_widget_id(&wid).ok_or_else(|| format!("missing {wid}"))?.label, *label);
        }
    }
    Ok(())
}

#[test]
fn buffered_single_commit_edit_undo_journal_and_request_discard_safety() -> TestRes {
    let mut driver = build_column_driver(4, 10, SubcolumnMode::Single, 1024.0, 640.0);
    assert_eq!(driver.state().app.session().undo_depth(), 0);

    let c00 = widget_center(&driver, "grid.cell.0.0.0", true)?;
    driver.click_at(c00)?;
    driver.click_at(c00)?;
    assert!(driver.state().grid.inline_edit.is_some());
    for _ in 0..9 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("314.15")?;
    assert_eq!(driver.state().app.session().undo_depth(), 0);
    driver.press_key("Enter", KeyModifiers::default())?;
    assert!(driver.state().grid.inline_edit.is_none());
    assert_eq!(driver.state().app.session().undo_depth(), 1);
    assert_eq!(main_cell(&driver, 0, 0).value, CellValue::Number(314.15));
    assert_eq!(driver.state().committed_engine_commands.len(), 1);
    assert_eq!(driver.state().committed_engine_commands[0].0, "table.setCell");
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(1, 0, 0)));

    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    assert_eq!(main_cell(&driver, 0, 0).value, CellValue::Number(1.5));

    let c01 = widget_center(&driver, "grid.cell.0.1.0", true)?;
    driver.click_at(c01)?;
    driver.click_at(c01)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("55.5");
    driver.press_key("Tab", KeyModifiers::default())?;
    assert_eq!(main_cell(&driver, 0, 1).value, CellValue::Number(55.5));
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(0, 2, 0)));

    let c10 = widget_center(&driver, "grid.cell.1.0.0", true)?;
    driver.click_at(c10)?;
    driver.click_at(c10)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("88.25");
    let c20 = widget_center(&driver, "grid.cell.2.0.0", true)?;
    driver.click_at(c20)?;
    assert_eq!(main_cell(&driver, 1, 0).value, CellValue::Number(88.25));
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(2, 0, 0)));

    let undo_before_esc = driver.state().app.session().undo_depth();
    driver.click_at(c20)?;
    driver.click_at(c20)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("9999.0");
    driver.press_key("Escape", KeyModifiers::default())?;
    assert!(driver.state().grid.inline_edit.is_none() && driver.state().app.session().undo_depth() == undo_before_esc);
    assert_eq!(main_cell(&driver, 2, 0).value, CellValue::Number(4.5));

    for (wid, draft) in [("grid.col_title.0", "Vehicle Control"), ("grid.row_title.0", "Mouse A1")] {
        let pos = widget_center(&driver, wid, true)?;
        driver.click_at(pos)?;
        driver.click_at(pos)?;
        driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = draft.to_owned();
        driver.press_key("Enter", KeyModifiers::default())?;
    }
    let TableArchetype::Column(col_t) = &driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().archetype else {
        panic!("Column")
    };
    assert_eq!((col_t.data_sets[0].title.as_str(), col_t.row_titles[0].as_str()), ("Vehicle Control", "Mouse A1"));

    let undo_before_discard = driver.state().app.session().undo_depth();
    driver.click_at(c00)?;
    driver.click_at(c00)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("777.0");
    driver.state_mut().discard_once = true;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert!(driver.state().discarded_passes >= 1);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before_discard + 1);
    assert_eq!(main_cell(&driver, 0, 0).value, CellValue::Number(777.0));
    Ok(())
}

#[test]
fn excluded_cell_styling_reason_tooltip_and_cmd_e_shortcut() -> TestRes {
    let mut doc = build_column_doc(3, 8, SubcolumnMode::Single);
    doc.data_sheet_mut(&SheetId::from("col-main"))
        .unwrap()
        .set_excluded_with_reason(CellCoord::new(1, 0, 0), "rout_outlier", "Pipetting bubble", "2026-11-19T12:00:00Z")
        .unwrap();
    let mut driver = make_grid_driver(UiApp::from_session(Session::from_document(doc)), 1024.0, 640.0, Tokens::default(), false);

    assert!(has_strikethrough_italic_text(&capture_grid_shapes(driver.state_mut(), 1024.0, 640.0), "3"));
    driver.hover_widget_id("grid.cell.1.0.0")?;
    assert!(driver.visible_tooltip_texts().iter().any(|t| t.contains("rout_outlier") && t.contains("Pipetting bubble")));

    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_key("e", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(main_cell(&driver, 0, 0).excluded && driver.state().app.session().undo_depth() == 1);
    Ok(())
}

#[test]
fn linked_data_region_border_readonly_guard_and_actions() -> TestRes {
    let mut doc = build_column_doc(3, 8, SubcolumnMode::Single);
    let mut upstream = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    upstream.data_sets[0].cells[0][0] = Cell::number(99.0);
    doc.add_data_sheet(DataSheet::new("raw-plate", "Raw Plate", TableArchetype::Column(upstream)).unwrap()).unwrap();
    let rng = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0));
    doc.data_sheet_mut(&SheetId::from("col-main"))
        .unwrap()
        .add_linked_region(linked_region("link-plate", "raw-plate", rng, rng, "Raw Plate › A1:B3"))
        .unwrap();
    doc.select_in_navigator(&SheetId::from("col-main")).unwrap();
    let mut driver = make_grid_driver(UiApp::from_session(Session::from_document(doc)), 1024.0, 640.0, Tokens::default(), false);

    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.type_text("9")?;
    assert!(driver.state().grid.active_link_popover.is_some() && driver.state().last_report.rendered_modal_guard_open);
    assert_eq!(driver.state().app.session().undo_depth(), 0);
    assert!(driver.frame_meta().find_by_widget_id("grid.link_popover.panel").is_some());

    let blocked_before = driver.state().outside_clicks_blocked;
    driver.click_at(widget_center(&driver, "grid.cell.5.2.0", false)?)?;
    assert!(driver.state().outside_clicks_blocked > blocked_before);
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(1, 0, 0)));

    driver.click_widget_id("grid.link_popover.unlink")?;
    assert!(driver.state().grid.active_link_popover.is_none());
    assert!(driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().linked_regions.is_empty());

    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().linked_regions.len(), 1);
    driver.run_settled();
    driver.click_widget_id("grid.link_badge.0")?;
    assert!(driver.state().grid.active_link_popover.is_some());
    let goto_node = driver.frame_meta().find_by_widget_id("grid.link_popover.goto").ok_or("missing grid.link_popover.goto")?;
    assert_eq!(goto_node.role, "link");
    assert!(goto_node.unclipped_rect.height() >= 24.0);
    driver.click_widget_id("grid.link_popover.goto")?;
    assert_eq!(driver.state().app.document().active_sheet_id, Some(SheetId::from("raw-plate")));
    Ok(())
}

#[test]
fn cjk_ime_14_beta_locales_themes_ellipsis_and_kittest_scrollbar_non_overlap() -> TestRes {
    let non_rtl: Vec<&'static str> = curvebook_ui_egui::i18n::LOCALES.iter().copied().filter(|l| !is_rtl_locale(l)).collect();
    assert_eq!(non_rtl.as_slice(), &BETA_LOCALE_CODES);
    let full_col_title = "Very Long Experimental Treatment Group Header Label That Must Ellipsize Cleanly";

    for &locale in &BETA_LOCALE_CODES {
        for mode in UiThemeMode::ALL {
            let mut app = UiApp::from_session(Session::from_document(build_column_doc(8, 30, SubcolumnMode::Single)));
            assert_eq!(app.set_locale(locale), locale);
            app.set_theme_mode(mode);
            let mut ds_doc = app.document().clone();
            if let Some(ds) = ds_doc.data_sheet_mut(&SheetId::from("col-main"))
                && let TableArchetype::Column(col) = &mut ds.archetype
            {
                col.data_sets[0].title = full_col_title.to_owned();
            }
            *app.session_mut() = Session::from_document(ds_doc);
            app.sync_views();

            let mut driver = make_grid_driver(app, 1440.0, 600.0, Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale), false);
            assert_clean_parity(&driver, &format!("{locale}/{mode:?} 1440x600"))?;
            driver.verify_ellipsis_tooltip_and_accesskit("grid.col_title.0", full_col_title)?;
            let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
            assert!(collect_tooltip_texts_from_accesskit(&ak.nodes).iter().any(|t| t == full_col_title));
            let hdr_meta = driver.frame_meta().find_by_widget_id("grid.col_title.0").ok_or("col_title.0")?;
            assert!(hdr_meta.ellipsized);
            let ak_node = ak.nodes.iter().find(|(id, _)| *id == hdr_meta.accesskit_id).map(|(_, n)| n).ok_or("ak node")?;
            assert_eq!(accesskit_accessible_name(ak_node), Some(full_col_title));

            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 2.0));
            assert_clean_parity(&driver, &format!("{locale}/{mode:?} 640x420@2x"))?;
        }
    }

    let mut ime_driver = build_column_driver(3, 6, SubcolumnMode::Single, 900.0, 560.0);
    assert_eq!(ime_driver.state_mut().app.set_locale("ja"), "ja");
    let c00 = widget_center(&ime_driver, "grid.cell.0.0.0", true)?;
    ime_driver.click_at(c00)?;
    ime_driver.click_at(c00)?;
    ime_driver.state_mut().grid.inline_edit.as_mut().unwrap().draft.clear();
    ime_driver.ime_preedit("たいしょう", Some(0..5));
    assert_eq!(ime_driver.state().grid.inline_edit.as_ref().unwrap().draft, "たいしょう");
    assert!(ime_driver.last_ime_output().is_some());
    ime_driver.ime_commit("対照群");
    assert_eq!(ime_driver.state().grid.inline_edit.as_ref().unwrap().draft, "対照群");
    ime_driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(main_cell(&ime_driver, 0, 0).value, CellValue::Text(String::from("対照群")));

    for (w, h) in [(640.0_f32, 420.0), (1440.0, 600.0)] {
        let app = UiApp::from_session(Session::from_document(build_column_doc(20, 60, SubcolumnMode::Single)));
        let mut harness = Harness::builder().with_size(egui::vec2(w, h)).allow_missing_glyphs().build_ui_state(
            |ui, state: &mut GridHarnessState| {
                let mut meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                render_grid_harness(ui, &mut meta, state);
            },
            GridHarnessState::new(app, false),
        );
        harness.ctx.set_fonts(build_driver_font_definitions());
        harness.run();
        harness.check_accessibility();
        assert_eq!(harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count(), 2);
        let bounds: Vec<AccessKitRoleBounds> = harness
            .root()
            .children_recursive()
            .filter_map(|n| {
                let ak = n.accesskit_node();
                (!ak.is_hidden() && ak.bounding_box().is_some()).then(|| AccessKitRoleBounds {
                    id: ak.locate().0,
                    role: ak.role(),
                    label: ak.label().or_else(|| ak.value()).unwrap_or_else(|| String::from("<unnamed>")),
                    rect: n.rect(),
                })
            })
            .collect();
        assert!(check_role_bounds_scrollbar_non_overlap(&bounds).is_empty());
        let _ = tr("en", "table.column");
    }
    Ok(())
}

#[test]
fn column_grid_1m_cells_virtualization_and_release_frame_budget() -> TestRes {
    let mut driver = build_column_driver(1_000, 1_000, SubcolumnMode::Single, 1280.0, 800.0);
    driver.run_settled();

    let cell0 = widget_center(&driver, "grid.cell.0.0.0", true)?;
    let frames = 24usize;
    let (mut total_ms, mut max_ms) = (0.0_f64, 0.0_f64);
    for step in 0..frames {
        let t0 = Instant::now();
        if step > 0 {
            driver.wheel_at(cell0, egui::vec2(-48.0, -48.0))?;
        } else {
            driver.run_settled();
        }
        let dt_ms = t0.elapsed().as_secs_f64() * 1_000.0;
        total_ms += dt_ms;
        max_ms = max_ms.max(dt_ms);
        assert!(driver.state().last_report.rendered_cells < 600);
    }

    let mean_ms = total_ms / (frames as f64);
    let max_input_ms = driver.max_input_to_paint_ms();
    let steady_budget_ms = if cfg!(debug_assertions) { 120.0 } else { 16.67 };
    let input_budget_ms = if cfg!(debug_assertions) { 250.0 } else { 50.0 };
    assert!(mean_ms <= steady_budget_ms, "1M-cell Column grid mean={mean_ms:.2}ms max={max_ms:.2}ms > {steady_budget_ms:.2}ms");
    assert!(max_input_ms <= input_budget_ms, "1M-cell input-to-paint={max_input_ms:.2}ms > {input_budget_ms:.2}ms");
    Ok(())
}

#[test]
fn column_grid_cmd_ctrl_cut_and_paste_roundtrip() -> TestRes {
    let mut driver = build_column_driver(4, 10, SubcolumnMode::Single, 1024.0, 640.0);
    let src_val = main_cell(&driver, 0, 0).value.finite_f64().ok_or("missing (0,0,0) value")?;

    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_key("x", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(main_cell(&driver, 0, 0).value.is_empty());

    driver.click_widget_id("grid.cell.1.1.0")?;
    driver.press_key("v", KeyModifiers { ctrl: true, ..Default::default() })?;
    assert_eq!(main_cell(&driver, 1, 1).value.finite_f64(), Some(src_val));
    Ok(())
}

#[test]
fn da_05_03_right_click_x_cut_and_multi_y_exclude_with_linked_guard() -> TestRes {
    let app = make_xy_app("xy-ctx", "XY Context", &[10.0, 20.0, 30.0, 40.0], &[&[1.0, 2.0], &[3.0, 4.0], &[5.0, 6.0], &[7.0, 8.0]])?;
    let mut driver = make_grid_driver(app, 1024.0, 640.0, Tokens::default(), true);

    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.click_at_button(widget_center(&driver, "grid.xcell.1.0", false)?, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.cut")?;
    let ds = driver.state().app.document().data_sheet(&SheetId::from("xy-ctx")).ok_or("xy-ctx")?;
    assert!(ds.archetype.x_or_time_cell(1, 0).is_some_and(|c| c.value.is_empty()));
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.value.finite_f64()), Some(1.0));

    let (c00, c11) = (widget_center(&driver, "grid.cell.0.0.0", false)?, widget_center(&driver, "grid.cell.1.1.0", false)?);
    driver.drag(c00, c11)?;
    driver.click_at_button(c11, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.exclude")?;
    let ds2 = driver.state().app.document().data_sheet(&SheetId::from("xy-ctx")).ok_or("xy-ctx")?;
    for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        assert!(ds2.cell(CellCoord::new(r, c, 0)).is_some_and(|cell| cell.excluded));
    }

    {
        let mut doc = driver.state().app.document().clone();
        doc.data_sheet_mut(&SheetId::from("xy-ctx"))
            .unwrap()
            .add_linked_region(linked_region(
                "link-xy",
                "xy-ctx",
                SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
                SheetCellRange::new(CellCoord::new(2, 0, 0), CellCoord::new(3, 0, 0)),
                "Linked Source › A3:A4",
            ))
            .map_err(|e| e.to_string())?;
        *driver.state_mut().app.session_mut() = Session::from_document(doc);
        driver.state_mut().app.sync_views();
        driver.run_settled();
    }
    let (c20, c31) = (widget_center(&driver, "grid.cell.2.0.0", false)?, widget_center(&driver, "grid.cell.3.1.0", false)?);
    driver.drag(c20, c31)?;
    driver.click_at_button(c31, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.exclude")?;
    assert!(driver.state().grid.active_link_popover.is_some());
    assert!(
        !driver
            .state()
            .app
            .document()
            .data_sheet(&SheetId::from("xy-ctx"))
            .ok_or("xy-ctx")?
            .cell(CellCoord::new(2, 0, 0))
            .is_some_and(|c| c.excluded)
    );

    driver.state_mut().grid.active_link_popover = None;
    driver.run_settled();
    driver.click_at_button(c31, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.cut")?;
    assert!(driver.state().grid.active_link_popover.is_some());
    assert_eq!(
        driver
            .state()
            .app
            .document()
            .data_sheet(&SheetId::from("xy-ctx"))
            .ok_or("xy-ctx")?
            .cell(CellCoord::new(2, 0, 0))
            .and_then(|c| c.value.finite_f64()),
        Some(5.0)
    );
    Ok(())
}

#[test]
fn da_05_04_shift_tab_from_y_col0_inline_editor_focuses_x_cell() -> TestRes {
    let app = make_xy_app("xy-tab", "XY Tab", &[1.0, 2.0, 3.0], &[&[10.0, 20.0], &[30.0, 40.0], &[50.0, 60.0]])?;
    let mut driver = make_grid_driver(app, 1024.0, 640.0, Tokens::default(), true);

    let c10 = widget_center(&driver, "grid.cell.1.0.0", false)?;
    driver.click_at(c10)?;
    driver.click_at(c10)?;
    assert!(driver.state().grid.inline_edit.is_some());
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("35.5");
    driver.run_settled();
    driver.press_egui_key(egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(
        driver
            .state()
            .app
            .document()
            .data_sheet(&SheetId::from("xy-tab"))
            .ok_or("xy-tab")?
            .cell(CellCoord::new(1, 0, 0))
            .and_then(|c| c.value.finite_f64()),
        Some(35.5)
    );

    driver.press_egui_key(egui::Key::Delete, egui::Modifiers::NONE);
    let ds_after = driver.state().app.document().data_sheet(&SheetId::from("xy-tab")).ok_or("xy-tab")?;
    assert!(ds_after.archetype.x_or_time_cell(1, 0).is_some_and(|c| c.value.is_empty()));
    assert_eq!(ds_after.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.value.finite_f64()), Some(35.5));
    Ok(())
}

#[test]
fn aud_07_and_09_grid_selection_linked_subcol_paste_dup_and_discard_regressions() -> TestRes {
    use crate::helpers::seed_column_app;
    use curvebook_doc::GroupedTable;
    use curvebook_tools::GridCellCoord;
    use curvebook_ui_egui::egui_shell::inspect_bridge::prepare_grid_type_text_target;
    use curvebook_ui_egui::grid_view::{handle_grid_key_down, handle_grid_text_input, linked_region_for_cell_subcol};
    use curvebook_ui_egui::interaction::{UiInputEvent, VirtualWorkspaceFolder, dispatch_ui_event};
    use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
    use curvebook_ui_egui::{ImportWarningItem, handle_control_line};
    use std::sync::Arc;

    // 1. AUD-07-10/11: OOB discontiguous coords clear stale quick_stats; excludeWithReason targets all_selected_coords.
    let (mut app, ds_id) = seed_column_app(&[("A", &[1.0, 2.0, 3.0]), ("B", &[11.0, 12.0, 13.0])], ColumnDesignHint::UnpairedReplicates);
    let ds = app.document().data_sheet(&ds_id).unwrap().clone();
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    app.grid.refresh_selection_stats(&app.document().clone());
    assert!(app.grid.quick_stats.is_some());
    app.grid.selected_coords = vec![GridCellCoord::new(0, 99, 0), GridCellCoord::new(0, 0, 5)];
    assert!(app.grid.all_selected_coords(&ds).is_empty() && app.grid.selected_graph_point_targets(&ds, 2).is_empty());
    app.grid.refresh_selection_stats(&app.document().clone());
    assert!(app.grid.quick_stats.is_none() && app.grid.extended_stats.is_none());

    app.grid.selected_coords = vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0)];
    let ex_raw = handle_control_line(&mut app, &json!({ "id": "ex", "method": "ui.grid.excludeWithReason", "params": { "sheet_id": ds_id.as_str(), "reason_tag": "outlier", "commit": true } }).to_string());
    assert!(ex_raw.contains("\"ok\":true"));
    let ds_ex = app.document().data_sheet(&ds_id).unwrap();
    assert!(
        ds_ex.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded)
            && ds_ex.cell(CellCoord::new(2, 1, 0)).is_some_and(|c| c.excluded)
            && ds_ex.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.excluded)
    );

    // 2. AUD-07-16/18: Subcol-aware LinkedDataRegion + single atomic undo on discontiguous Delete & Cmd+E.
    let mut gdoc = ProjectDocument::new("LinkSub").unwrap();
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    for c in 0..2 {
        for r in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number((r + 1) as f64);
            grp.data_sets[c].cells[r][1] = Cell::number((r + 10) as f64);
        }
    }
    let mut gds = DataSheet::new("grp-1", "Grp", TableArchetype::Grouped(grp)).unwrap();
    gds.linked_regions.push(LinkedDataRegion {
        id: "link-sub0".into(),
        spec: ImportLinkSpec { source: LinkSource::Clipboard {}, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: "sub0".into(),
    });
    gdoc.data_sheets.push(gds);
    gdoc.active_sheet_id = Some(SheetId::new("grp-1"));
    let mut gapp = UiApp::from_session(Session::from_document(gdoc));
    let gref = gapp.document().data_sheet(&SheetId::new("grp-1")).unwrap();
    assert!(linked_region_for_cell_subcol(gref, 0, 0, 0).is_some() && linked_region_for_cell_subcol(gref, 0, 0, 1).is_none());
    gapp.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    assert_eq!(handle_grid_text_input(&gapp, "5").action, "grid_core.link_read_only_prompt");
    assert!(prepare_grid_type_text_target(&mut gapp, Some("grid.cell.0.0.0"), None, false, false).is_err());
    gapp.grid.select_single_sync(GridCellCoord::new(0, 0, 1));
    assert!(
        !handle_grid_text_input(&gapp, "5").handled && prepare_grid_type_text_target(&mut gapp, Some("grid.cell.0.0.1"), None, false, false).is_ok()
    );

    gapp.grid.selected_coords = vec![GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 0)];
    let u0 = gapp.session().undo_depth();
    assert_eq!(handle_grid_key_down(&mut gapp, "Delete", KeyModifiers::default()).unwrap().action, "grid_core.clear_selection");
    assert_eq!(gapp.session().undo_depth(), u0 + 1);
    let _ = gapp.session_mut().undo().unwrap();
    gapp.grid.selected_coords = vec![GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 0)];
    let ue = gapp.session().undo_depth();
    assert_eq!(
        dispatch_ui_event(
            &mut gapp,
            UiInputEvent::KeyDown { key: "e".into(), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
            ViewportMetrics::default()
        )
        .unwrap()
        .action,
        "shortcut:grid.toggle_exclude"
    );
    assert_eq!(gapp.session().undo_depth(), ue + 1);

    // 3. AUD-07-20/21/22: VirtualWorkspace load reset, colliding sanitized SheetIds, German locale paste & failing shortcut Err.
    app.save_to_virtual_workspace("p.curvebook").unwrap();
    app.import_warnings.warnings.push(ImportWarningItem { kind: "w".into(), source: "t".into(), message: "stale".into() });
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.brush_from_grid_cell(&ds_id, GridCellCoord::new(0, 0, 0));
    app.load_from_virtual_workspace("p.curvebook").unwrap();
    assert!(app.import_warnings.warnings.is_empty() && app.canvas_dialogs.active_dialog == ActiveDialog::None && app.brushed_points.is_empty());

    app.set_locale("de");
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("1,5\n2,25");
    app.set_clipboard_adapter(stub);
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    assert_eq!(
        dispatch_ui_event(
            &mut app,
            UiInputEvent::KeyDown { key: "v".into(), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
            ViewportMetrics::default()
        )
        .unwrap()
        .action,
        "shortcut:table.paste"
    );
    let ds_de = app.document().data_sheet(&ds_id).unwrap();
    assert_eq!(
        (ds_de.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), ds_de.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number)),
        (Some(1.5), Some(2.25))
    );
    app.shortcuts.set_shortcut("ui.export.figure.open", "F9").unwrap();
    assert!(
        dispatch_ui_event(&mut app, UiInputEvent::KeyDown { key: "F9".into(), modifiers: KeyModifiers::default() }, ViewportMetrics::default())
            .is_err()
    );

    let mut cdoc = ProjectDocument::new("Collide").unwrap();
    for id in ["exp.1", "exp+1"] {
        cdoc.data_sheets
            .push(DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap())).unwrap());
    }
    let mut vf = VirtualWorkspaceFolder::default();
    let _ = vf.save_unpacked_snapshot(&cdoc).unwrap();
    assert_eq!(vf.files.keys().filter(|k| k.starts_with("sheets/")).count(), 2);
    assert_eq!(vf.open_unpacked_snapshot().unwrap().data_sheets.len(), 2);
    Ok(())
}

#[test]
fn aud_09_grid_subranges_german_inline_edit_dup_columns_and_factor_discard() -> TestRes {
    use crate::helpers::{run_egui_pass_with_ctx, seed_column_app};
    use curvebook_doc::{NestedGroupColumn, NestedTable, VariableRole, VariableSource};
    use curvebook_tools::GridCellCoord;
    use curvebook_ui_egui::egui_shell::data_grid::{
        DataGridEguiState, DataGridRenderContext, GridInlineEditTarget, ResolvedGridSelection, build_duplicate_column_intent_for_col,
        column_design_hint_tag, commit_standard_inline_edit, push_clear_selection_intent, render_data_grid_for_sheet, resolve_grid_selection,
        selected_visual_subranges,
    };

    let (mut app, ds_id) = seed_column_app(&[("A", &[1.0, 2.0, 3.0]), ("B", &[11.0, 12.0, 13.0])], ColumnDesignHint::UnpairedReplicates);
    app.set_locale("de");

    // 4. AUD-09-04/05/06 & AUD-13-24: subrange decomposition, Summary tag, German excluded inline edit, Column/Nested/MV duplicate.
    let mut grp3 = UiApp::new();
    grp3.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "dg", "kind": "grouped", "cols": 2, "rows": 4, "subcolumn_mode": { "RawReplicates": 3 }, "auto_graph": false }),
    )
    .unwrap();
    let dgr = grp3.document().data_sheet(&SheetId::new("dg")).unwrap();
    let ranges = selected_visual_subranges(dgr, GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 1));
    assert_eq!((ranges.len(), ranges[0].start.subcol, ranges[0].end.subcol, ranges[1].start.subcol, ranges[1].end.subcol), (2, 1, 2, 0, 1));
    grp3.grid.selected_coords = vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(3, 1, 2)];
    let dgr2 = grp3.document().data_sheet(&SheetId::new("dg")).unwrap();
    let res = resolve_grid_selection(&grp3, dgr2).unwrap();
    assert!(matches!(res, ResolvedGridSelection::Discontiguous(ref v) if v.len() == 2));
    let mut ci = PendingUiIntents::new();
    push_clear_selection_intent(dgr2, &res, &mut ci);
    assert_eq!(ci.intents().len(), 1);
    assert_eq!(column_design_hint_tag(&ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 3).unwrap()), "Summary");
    assert_eq!(column_design_hint_tag(&ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap()), "Unpaired");

    let mut ie = PendingUiIntents::new();
    commit_standard_inline_edit(
        &app,
        &app.document().data_sheet(&ds_id).unwrap().clone(),
        GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)),
        "10,5*".into(),
        &mut ie,
    );
    ie.commit_if_settled(&mut app, false).unwrap();
    assert_eq!(
        app.document().data_sheet(&ds_id).unwrap().cell(CellCoord::new(0, 0, 0)),
        Some(&Cell { value: CellValue::Number(10.5), excluded: true })
    );
    let mut iu = PendingUiIntents::new();
    commit_standard_inline_edit(
        &app,
        &app.document().data_sheet(&ds_id).unwrap().clone(),
        GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)),
        "10,5".into(),
        &mut iu,
    );
    iu.commit_if_settled(&mut app, false).unwrap();
    assert_eq!(
        app.document().data_sheet(&ds_id).unwrap().cell(CellCoord::new(0, 0, 0)),
        Some(&Cell { value: CellValue::Number(10.5), excluded: false })
    );

    let mut dapp = UiApp::new();
    dapp.execute_command("sheet.createData", json!({ "sheet_id": "dc", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false })).unwrap();
    dapp.execute_command("table.setCell", json!({ "sheet_id": "dc", "row": 0, "col": 0, "value": 42.0, "excluded": true })).unwrap();
    let mut di = PendingUiIntents::new();
    di.push(build_duplicate_column_intent_for_col(&dapp.document().data_sheet(&SheetId::new("dc")).unwrap().clone(), 0).unwrap());
    di.commit_if_settled(&mut dapp, false).unwrap();
    assert_eq!(
        dapp.document().data_sheet(&SheetId::new("dc")).unwrap().cell(CellCoord::new(0, 1, 0)),
        Some(&Cell { value: CellValue::Number(42.0), excluded: true })
    );
    dapp.execute_command("edit.undo", json!({})).unwrap();
    assert_eq!(dapp.document().data_sheet(&SheetId::new("dc")).unwrap().archetype.column_count(), 1);

    let mut napp = UiApp::new();
    napp.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "dn", "kind": "nested", "cols": 2, "rows": 2, "replicates": 3, "auto_graph": false }),
    )
    .unwrap();
    napp.execute_command("table.setSubcolumnTitles", json!({ "sheet_id": "dn", "col": 0, "subcol": 2, "title": "MouseC" })).unwrap();
    napp.execute_command("table.setCell", json!({ "sheet_id": "dn", "row": 0, "col": 0, "subcol": 2, "value": 99.0, "excluded": true })).unwrap();
    let mut ni = PendingUiIntents::new();
    ni.push(build_duplicate_column_intent_for_col(&napp.document().data_sheet(&SheetId::new("dn")).unwrap().clone(), 0).unwrap());
    ni.commit_if_settled(&mut napp, false).unwrap();
    if let TableArchetype::Nested(nt) = &napp.document().data_sheet(&SheetId::new("dn")).unwrap().archetype {
        assert_eq!((nt.groups.len(), nt.groups[2].biological_unit_count(), nt.groups[2].subcolumn_titles[2].as_str()), (3, 3, "MouseC"));
        assert_eq!(nt.groups[2].cells[0][2], Cell { value: CellValue::Number(99.0), excluded: true });
    }
    let uneq = DataSheet::new(
        "u",
        "U",
        TableArchetype::Nested(NestedTable {
            row_titles: vec![String::new(); 2],
            row_titles_expanded: false,
            groups: vec![NestedGroupColumn::new("G0", 2, 2).unwrap(), NestedGroupColumn::new("G1", 4, 2).unwrap()],
        }),
    )
    .unwrap();
    assert!(build_duplicate_column_intent_for_col(&uneq, 1).is_none());

    let mut mvapp = UiApp::new();
    mvapp.execute_command("sheet.createData", json!({ "sheet_id": "dm", "kind": "multiple_variables", "cols": 2, "rows": 2, "variables": [{ "title": "Val", "role": "continuous" }, { "title": "Arm", "role": "categorical" }], "auto_graph": false })).unwrap();
    mvapp.execute_command("table.setCell", json!({ "sheet_id": "dm", "row": 0, "col": 0, "value": 4.0 })).unwrap();
    mvapp.execute_command("table.setCell", json!({ "sheet_id": "dm", "row": 0, "col": 1, "value": "Treat" })).unwrap();
    mvapp.execute_command("table.setCalculatedVariable", json!({ "sheet_id": "dm", "title": "Doubled", "formula": "Val * 2" })).unwrap();
    let dsmv = mvapp.document().data_sheet(&SheetId::new("dm")).unwrap().clone();
    let mut mvi = PendingUiIntents::new();
    mvi.push(build_duplicate_column_intent_for_col(&dsmv, 1).unwrap());
    mvi.push(build_duplicate_column_intent_for_col(&dsmv, 2).unwrap());
    mvi.commit_all_if_settled(&mut mvapp, false);
    if let TableArchetype::MultipleVariables(m) = &mvapp.document().data_sheet(&SheetId::new("dm")).unwrap().archetype {
        assert_eq!(m.variables[3].role, VariableRole::Categorical);
        assert_eq!(m.variables[4].source, VariableSource::Calculated { formula: "Val * 2".into() });
    }

    // 5. AUD-09-42: factor title editor survives will_discard pass and commits on settled pass.
    let mut fapp = UiApp::new();
    fapp.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "df", "kind": "grouped", "cols": 2, "rows": 2, "replicates": 2, "auto_graph": false }),
    )
    .unwrap();
    fapp.execute_command("table.setTitles", json!({ "sheet_id": "df", "row_factor_title": "Time" })).unwrap();
    fapp.sync_views();
    let tok = Tokens::from_theme_spec(&fapp.theme, &fapp.locale);
    let (ctx_f, mut gst) = (egui::Context::default(), DataGridEguiState::default());
    let mut m0 = FrameInspectMeta::default();
    run_egui_pass_with_ctx(&ctx_f, egui::vec2(900.0, 500.0), Default::default(), true, |c, ui| {
        m0.begin_pass(c);
        render_data_grid_for_sheet(
            ui,
            &fapp,
            Some(&SheetId::new("df")),
            &mut gst,
            DataGridRenderContext::new(&tok, false),
            &mut m0,
            &mut PendingUiIntents::new(),
        );
    })?;
    let p = m0.find_by_widget_id("grid.row_titles.band").unwrap().clipped_rect.center();
    let dbl = egui::RawInput {
        events: vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        ..Default::default()
    };
    run_egui_pass_with_ctx(&ctx_f, egui::vec2(900.0, 500.0), dbl, true, |_, ui| {
        render_data_grid_for_sheet(
            ui,
            &fapp,
            Some(&SheetId::new("df")),
            &mut gst,
            DataGridRenderContext::new(&tok, false),
            &mut FrameInspectMeta::default(),
            &mut PendingUiIntents::new(),
        );
    })?;
    run_egui_pass_with_ctx(
        &ctx_f,
        egui::vec2(900.0, 500.0),
        egui::RawInput { events: vec![egui::Event::Text("_Upd".into())], ..Default::default() },
        true,
        |_, ui| {
            render_data_grid_for_sheet(
                ui,
                &fapp,
                Some(&SheetId::new("df")),
                &mut gst,
                DataGridRenderContext::new(&tok, false),
                &mut FrameInspectMeta::default(),
                &mut PendingUiIntents::new(),
            );
        },
    )?;
    let enter = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    let (mut pass_idx, mut p0_intents, mut idisc) = (0usize, usize::MAX, PendingUiIntents::new());
    run_egui_pass_with_ctx(&ctx_f, egui::vec2(900.0, 500.0), enter, true, |c, ui| {
        idisc = PendingUiIntents::new();
        if pass_idx == 0 {
            c.request_discard("test");
        }
        render_data_grid_for_sheet(
            ui,
            &fapp,
            Some(&SheetId::new("df")),
            &mut gst,
            DataGridRenderContext::new(&tok, false),
            &mut FrameInspectMeta::default(),
            &mut idisc,
        );
        if pass_idx == 0 {
            p0_intents = idisc.intents().len();
        }
        pass_idx += 1;
    })?;
    assert!(pass_idx >= 2 && p0_intents == 0 && idisc.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, params } if command_id == "table.setTitles" && params.get("row_factor_title").is_some())));
    Ok(())
}
