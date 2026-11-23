#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R15.2` Wave 2 Virtualized `Column` Data Grid integration tests:
//! - Frozen 3-tier header band (`20 / 24 / 20 pt`), Corner Archetype Trigger (`Column`),
//!   Super-Column IDs (`Col A`, `Col B`, ...), Editable Titles, Subcolumn Roles (`Y`, `Mean/SD/N`)
//! - Frozen `#` (`48 pt`) + collapsible `Row Titles` (`112 pt` / `34 pt`)
//! - Two-axis virtualized viewport + non-overlapping `10 pt` scrollbars (`640×420` & `1440×600`)
//! - Excluded cells (italic + strikethrough + corner marker + reason tooltip + `Cmd/Ctrl+E`)
//! - `LinkedDataRegion` `2 px` border + read-only guard popover (`Go To Source`, `Unlink`, `Close`)
//! - Buffered single-commit editing (`Enter`, `Tab`, focus-loss, `Escape` cancel, `request_discard` guard)
//! - CJK/IME preedit + commit, all `14` `BETA_ACTIVE_LOCALES`, themes, and `1,000,000`-cell timing gate

use std::time::Instant;

use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind,
    LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::data_grid::{ColumnGridEguiState, ColumnGridFrameReport, ColumnGridRenderContext, render_column_grid};
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

struct GridHarnessState {
    app: UiApp,
    grid: ColumnGridEguiState,
    last_report: ColumnGridFrameReport,
    discard_once: bool,
    discarded_passes: usize,
    committed_engine_commands: Vec<(String, serde_json::Value)>,
    outside_clicks_blocked: usize,
}

fn widget_center<S>(driver: &EguiHumanPathDriver<'_, S>, widget_id: &str, require_enabled: bool) -> Result<egui::Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if require_enabled && !entry.enabled {
        return Err(format!("widget `{widget_id}` is disabled"));
    }
    Ok(entry.clipped_rect.center())
}

impl GridHarnessState {
    fn new(app: UiApp) -> Self {
        Self {
            app,
            grid: ColumnGridEguiState::default(),
            last_report: ColumnGridFrameReport::default(),
            discard_once: false,
            discarded_passes: 0,
            committed_engine_commands: Vec::new(),
            outside_clicks_blocked: 0,
        }
    }
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
        state.last_report =
            render_column_grid(ui, &state.app, &mut state.grid, ColumnGridRenderContext::new(&tokens, modal_blocked), meta, &mut intents);
    });
    if state.last_report.outside_click_blocked {
        state.outside_clicks_blocked += 1;
    }
    let will_discard = ui.ctx().will_discard();
    if will_discard {
        state.discarded_passes += 1;
    } else {
        for intent in intents.intents() {
            if let UiIntent::EngineCommand { command_id, params } = intent {
                state.committed_engine_commands.push((command_id.clone(), params.clone()));
            }
        }
    }
    intents.commit_if_settled(&mut state.app, will_discard).expect("commit settled intents");
}

fn capture_grid_shapes(state: &mut GridHarnessState, width: f32, height: f32) -> Vec<ClippedShape> {
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let mut meta = FrameInspectMeta::new();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
    let out = ctx.run_ui(raw, |ui| {
        meta.begin_pass(ui.ctx());
        render_grid_harness(ui, &mut meta, state);
    });
    let shapes = out.shapes.clone();
    out.drop_without_applying_deltas();
    shapes
}

fn build_column_doc(cols: usize, rows: usize, mode: SubcolumnMode) -> ProjectDocument {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, mode, cols, rows).expect("create column table");
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
    let ds = DataSheet::new("col-main", "Column Assay", TableArchetype::Column(col)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data sheet");
    doc.select_in_navigator(&SheetId::from("col-main")).expect("select sheet");
    doc
}

fn build_column_driver(cols: usize, rows: usize, mode: SubcolumnMode, w: f64, h: f64) -> EguiHumanPathDriver<'static, GridHarnessState> {
    let app = UiApp::from_session(Session::from_document(build_column_doc(cols, rows, mode)));
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        GridHarnessState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
        build_driver_font_definitions(),
        render_grid_harness,
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver
}

fn has_strikethrough_italic_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
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

#[test]
fn frozen_three_tier_headers_corner_trigger_and_row_titles_toggle() -> Result<(), String> {
    let mut driver = build_column_driver(18, 80, SubcolumnMode::Single, 1280.0, 800.0);
    let parity = driver.verify_pixel_to_hit_box_parity();
    assert!(parity.is_clean(), "initial parity violations: {:?}", parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());

    // Canonical 3-tier header heights (`24 / 24 / 24`) & `#` (`48 pt`) + `Row Titles` (`112 pt` expanded)
    let corner = driver.frame_meta().find_by_widget_id("grid.corner.archetype").ok_or("missing corner trigger")?;
    assert_eq!(corner.clipped_rect.width(), 48.0);
    let super_a = driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("missing col_super.0")?;
    let title_a = driver.frame_meta().find_by_widget_id("grid.col_title.0").ok_or("missing col_title.0")?;
    let sub_a = driver.frame_meta().find_by_widget_id("grid.subcol_title.0.0").ok_or("missing subcol_title.0.0")?;
    assert_eq!(super_a.label, "Col A");
    assert_eq!(title_a.label, "Group 1");
    assert_eq!(sub_a.label, "Y");
    assert!((super_a.clipped_rect.height() - 24.0).abs() < 0.1);
    assert!((title_a.clipped_rect.height() - 24.0).abs() < 0.1);
    assert!((sub_a.clipped_rect.height() - 24.0).abs() < 0.1);
    let rt_band = driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("missing row_titles.band")?;
    assert!((rt_band.clipped_rect.width() - 112.0).abs() < 0.5);

    // Collapse & expand Row Titles (`112 pt` -> `34 pt` -> `112 pt`)
    driver.click_widget_id("grid.row_titles.toggle")?;
    assert!(driver.state().grid.row_titles_collapsed);
    let toggle_collapsed = driver.frame_meta().find_by_widget_id("grid.row_titles.toggle").ok_or("missing collapsed toggle")?;
    assert!((toggle_collapsed.clipped_rect.width() - 34.0).abs() < 0.5);
    assert!(driver.frame_meta().find_by_widget_id("grid.row_title_collapsed.0").is_some());

    driver.click_widget_id("grid.row_titles.toggle")?;
    assert!(!driver.state().grid.row_titles_collapsed);
    let rt_band2 = driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("missing expanded band")?;
    assert!((rt_band2.clipped_rect.width() - 112.0).abs() < 0.5);

    // Wheel scroll down & right reveals off-screen cell while headers & left gutter remain frozen!
    let cell0_pos = widget_center(&driver, "grid.cell.2.1.0", true)?;
    let super_a_top = driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("super_a")?.clipped_rect.top();
    let row0_left = driver.frame_meta().find_by_widget_id("grid.row_num.0").ok_or("row_num.0")?.clipped_rect.left();
    driver.wheel_at(cell0_pos, egui::vec2(-500.0, -720.0))?;
    assert!(driver.state().grid.scroll_offset_pt.y > 200.0);
    assert!(driver.state().grid.scroll_offset_pt.x > 100.0);
    assert!(driver.frame_meta().find_by_widget_id("grid.cell.35.6.0").is_some(), "expected scrolled cell (35,6,0) in FrameInspectMeta");

    // Frozen header & frozen row number column remain pinned at top/left
    let super_g = driver.frame_meta().find_by_widget_id("grid.col_super.6").ok_or("col_super.6")?;
    assert!((super_g.clipped_rect.top() - super_a_top).abs() < 0.5);
    let row35 = driver.frame_meta().find_by_widget_id("grid.row_num.35").ok_or("row_num.35")?;
    assert!((row35.clipped_rect.left() - row0_left).abs() < 0.5);

    // Click Corner Archetype Trigger -> opens `ConvertArchetype` dialog via PendingUiIntents
    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));

    // SubcolumnMode::MeanSdN / MeanSemN / MeanCvN Tier-3 headers
    for (mode, expected_labels) in [
        (SubcolumnMode::MeanSdN, ["Mean", "SD", "N"]),
        (SubcolumnMode::MeanSemN, ["Mean", "SEM", "N"]),
        (SubcolumnMode::MeanCvN, ["Mean", "%CV", "N"]),
    ] {
        let sub_drv = build_column_driver(3, 6, mode, 1024.0, 600.0);
        for (sub_idx, expected) in expected_labels.iter().enumerate() {
            let wid = format!("grid.subcol_title.0.{sub_idx}");
            let node = sub_drv.frame_meta().find_by_widget_id(&wid).ok_or_else(|| format!("missing {wid}"))?;
            assert_eq!(node.label, *expected);
        }
    }
    Ok(())
}

#[test]
fn buffered_single_commit_edit_undo_journal_and_request_discard_safety() -> Result<(), String> {
    let mut driver = build_column_driver(4, 10, SubcolumnMode::Single, 1024.0, 640.0);
    assert_eq!(driver.state().app.session().undo_depth(), 0);

    // 1. Double-click cell (0,0,0), type draft, confirm intermediate buffer does NOT mutate Session, commit on Enter
    let c00 = widget_center(&driver, "grid.cell.0.0.0", true)?;
    driver.click_at(c00)?;
    driver.click_at(c00)?;
    assert!(driver.state().grid.inline_edit.is_some());
    driver.press_key("Backspace", KeyModifiers::default())?;
    for _ in 0..8 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("314.15")?;
    assert_eq!(driver.state().app.session().undo_depth(), 0, "typing inside buffered TextEdit must not mutate Session");
    driver.press_key("Enter", KeyModifiers::default())?;
    assert!(driver.state().grid.inline_edit.is_none());
    assert_eq!(driver.state().app.session().undo_depth(), 1, "Enter must commit strictly once");
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().value,
        CellValue::Number(314.15)
    );
    assert_eq!(driver.state().committed_engine_commands.len(), 1);
    assert_eq!(driver.state().committed_engine_commands[0].0, "table.setCell");
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(1, 0, 0)));

    // Undo restores original 1.5
    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().value,
        CellValue::Number(1.5)
    );

    // 2. Edit with Tab commit -> advances right
    let c01 = widget_center(&driver, "grid.cell.0.1.0", true)?;
    driver.click_at(c01)?;
    driver.click_at(c01)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("55.5");
    driver.press_key("Tab", KeyModifiers::default())?;
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(0, 1, 0)).unwrap().value,
        CellValue::Number(55.5)
    );
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(0, 2, 0)));

    // 3. Edit with Focus-loss commit (clicking another cell)
    let c10 = widget_center(&driver, "grid.cell.1.0.0", true)?;
    driver.click_at(c10)?;
    driver.click_at(c10)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("88.25");
    let c20 = widget_center(&driver, "grid.cell.2.0.0", true)?;
    driver.click_at(c20)?;
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(1, 0, 0)).unwrap().value,
        CellValue::Number(88.25)
    );
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(2, 0, 0)));

    // 4. Escape cancels without committing
    let undo_before_esc = driver.state().app.session().undo_depth();
    driver.click_at(c20)?;
    driver.click_at(c20)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("9999.0");
    driver.press_key("Escape", KeyModifiers::default())?;
    assert!(driver.state().grid.inline_edit.is_none());
    assert_eq!(driver.state().app.session().undo_depth(), undo_before_esc);
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(2, 0, 0)).unwrap().value,
        CellValue::Number(4.5)
    );

    // 5. Tier 2 Column Title & Row Title double-click inline edit
    let hdr0 = widget_center(&driver, "grid.col_title.0", true)?;
    driver.click_at(hdr0)?;
    driver.click_at(hdr0)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("Vehicle Control");
    driver.press_key("Enter", KeyModifiers::default())?;
    let ds = driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap();
    let TableArchetype::Column(col_t) = &ds.archetype else { panic!("expected Column") };
    assert_eq!(col_t.data_sets[0].title, "Vehicle Control");

    let rt0 = widget_center(&driver, "grid.row_title.0", true)?;
    driver.click_at(rt0)?;
    driver.click_at(rt0)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("Mouse A1");
    driver.press_key("Enter", KeyModifiers::default())?;
    let ds = driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap();
    let TableArchetype::Column(col_t) = &ds.archetype else { panic!("expected Column") };
    assert_eq!(col_t.row_titles[0], "Mouse A1");

    // 6. Multi-pass `request_discard()` guard commits strictly once!
    let undo_before_discard = driver.state().app.session().undo_depth();
    driver.click_at(c00)?;
    driver.click_at(c00)?;
    driver.state_mut().grid.inline_edit.as_mut().unwrap().draft = String::from("777.0");
    driver.state_mut().discard_once = true;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert!(driver.state().discarded_passes >= 1);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before_discard + 1, "multi-pass discard must not duplicate Session::execute");
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().value,
        CellValue::Number(777.0)
    );

    Ok(())
}

#[test]
fn excluded_cell_styling_reason_tooltip_and_cmd_e_shortcut() -> Result<(), String> {
    let mut doc = build_column_doc(3, 8, SubcolumnMode::Single);
    {
        let ds = doc.data_sheet_mut(&SheetId::from("col-main")).unwrap();
        ds.set_excluded_with_reason(CellCoord::new(1, 0, 0), "rout_outlier", "Pipetting bubble", "2026-11-19T12:00:00Z").unwrap();
    }
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        GridHarnessState::new(app),
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        render_grid_harness,
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    // Visual italic + strikethrough rendered on excluded cell `(1, 0, 0)` (value `3`)
    let shapes = capture_grid_shapes(driver.state_mut(), 1024.0, 640.0);
    assert!(has_strikethrough_italic_text(&shapes, "3"), "excluded cell must render with italic + strikethrough TextFormat");

    // Hover excluded cell -> tooltip shows reason tag & audit note
    driver.hover_widget_id("grid.cell.1.0.0")?;
    let tips = driver.visible_tooltip_texts();
    assert!(
        tips.iter().any(|t| t.contains("rout_outlier") && t.contains("Pipetting bubble")),
        "expected exclusion reason tooltip on hover, got {tips:?}"
    );

    // Select `(0, 0, 0)` and press `Cmd+E` -> toggles excluded via PendingUiIntents
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_key("e", KeyModifiers { meta: true, ..Default::default() })?;
    let ds = driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap();
    assert!(ds.cell(CellCoord::new(0, 0, 0)).unwrap().excluded, "Cmd+E must mark cell excluded");
    assert_eq!(driver.state().app.session().undo_depth(), 1);
    Ok(())
}

#[test]
fn linked_data_region_border_readonly_guard_and_actions() -> Result<(), String> {
    let mut doc = build_column_doc(3, 8, SubcolumnMode::Single);
    let mut upstream = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    upstream.data_sets[0].cells[0][0] = Cell::number(99.0);
    doc.add_data_sheet(DataSheet::new("raw-plate", "Raw Plate", TableArchetype::Column(upstream)).unwrap()).unwrap();
    {
        let ds = doc.data_sheet_mut(&SheetId::from("col-main")).unwrap();
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-plate"),
            spec: ImportLinkSpec {
                source: LinkSource::InternalSheet {
                    sheet_id: SheetId::from("raw-plate"),
                    kind: InternalLinkKind::DataSheet,
                    tab_id: None,
                    range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0)),
                },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0)),
            provenance_summary: String::from("Raw Plate › A1:B3"),
        })
        .unwrap();
    }
    doc.select_in_navigator(&SheetId::from("col-main")).unwrap();
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        GridHarnessState::new(app),
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        render_grid_harness,
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

    // Clicking a linked cell & typing triggers read-only guard popover without mutating Session
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.type_text("9")?;
    assert!(driver.state().grid.active_link_popover.is_some());
    assert!(driver.state().last_report.rendered_modal_guard_open);
    assert_eq!(driver.state().app.session().undo_depth(), 0);
    assert!(driver.frame_meta().find_by_widget_id("grid.link_popover.panel").is_some());

    // Outside click is blocked
    let blocked_before = driver.state().outside_clicks_blocked;
    let outside = widget_center(&driver, "grid.cell.5.2.0", false)?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_clicks_blocked > blocked_before, "outside click must be blocked by guard backdrop");
    assert_eq!(driver.state().app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(1, 0, 0)));

    // Click `Unlink` -> detaches link cleanly via table.clearCells (undoable)
    driver.click_widget_id("grid.link_popover.unlink")?;
    assert!(driver.state().grid.active_link_popover.is_none());
    assert!(driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().linked_regions.is_empty());

    // Undo restores the link, then test `Go To Source`
    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().linked_regions.len(), 1);
    driver.run_settled();
    driver.click_widget_id("grid.link_badge.0")?;
    assert!(driver.state().grid.active_link_popover.is_some());
    driver.click_widget_id("grid.link_popover.goto")?;
    assert_eq!(driver.state().app.document().active_sheet_id, Some(SheetId::from("raw-plate")));
    Ok(())
}

#[test]
fn cjk_ime_14_beta_locales_themes_ellipsis_and_kittest_scrollbar_non_overlap() -> Result<(), String> {
    let non_rtl: Vec<&'static str> = curvebook_ui_egui::i18n::LOCALES.iter().copied().filter(|l| !is_rtl_locale(l)).collect();
    assert_eq!(non_rtl.as_slice(), &BETA_LOCALE_CODES);

    // 1. All 14 Beta locales × Light/Dark/HighContrast at 640×420 and 1440×600
    for &locale in &BETA_LOCALE_CODES {
        for mode in UiThemeMode::ALL {
            let mut app = UiApp::from_session(Session::from_document(build_column_doc(8, 30, SubcolumnMode::Single)));
            assert_eq!(app.set_locale(locale), locale);
            app.set_theme_mode(mode);
            let mut ds_doc = app.document().clone();
            if let Some(ds) = ds_doc.data_sheet_mut(&SheetId::from("col-main"))
                && let TableArchetype::Column(col) = &mut ds.archetype
            {
                col.data_sets[0].title = String::from("Very Long Experimental Treatment Group Header Label That Must Ellipsize Cleanly");
            }
            *app.session_mut() = Session::from_document(ds_doc);
            app.sync_views();

            let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale);
            let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
                GridHarnessState::new(app),
                ViewportMetrics::new(1440.0, 600.0, 1.0),
                tokens,
                build_driver_font_definitions(),
                render_grid_harness,
            );
            driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
            let wide_rep = driver.verify_pixel_to_hit_box_parity();
            assert!(wide_rep.is_clean(), "{locale}/{mode:?} 1440x600 parity violations: {:?}", wide_rep.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty());

            // Verify ellipsized header exposes full label in AccessKit + hover tooltip
            let full_col_title = "Very Long Experimental Treatment Group Header Label That Must Ellipsize Cleanly";
            driver.verify_ellipsis_tooltip_and_accesskit("grid.col_title.0", full_col_title)?;
            let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
            let tips = collect_tooltip_texts_from_accesskit(&ak.nodes);
            assert!(tips.iter().any(|t| t == full_col_title), "{locale}/{mode:?}: expected hover tooltip `{full_col_title}`");
            let hdr_meta = driver.frame_meta().find_by_widget_id("grid.col_title.0").ok_or("col_title.0")?;
            assert!(hdr_meta.ellipsized);
            let ak_node = ak.nodes.iter().find(|(id, _)| *id == hdr_meta.accesskit_id).map(|(_, n)| n).ok_or("ak node")?;
            assert_eq!(accesskit_accessible_name(ak_node), Some(full_col_title));

            // Compact 640×420 @ 2.0x DPI
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 2.0));
            let compact_rep = driver.verify_pixel_to_hit_box_parity();
            assert!(compact_rep.is_clean(), "{locale}/{mode:?} 640x420@2x parity violations: {:?}", compact_rep.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty());
        }
    }

    // 2. CJK IME preedit + commit inside cell
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
    assert_eq!(
        ime_driver.state().app.document().data_sheet(&SheetId::from("col-main")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().value,
        CellValue::Text(String::from("対照群"))
    );

    // 3. `egui_kittest::Harness` AccessKit + Role::ScrollBar non-overlap at 640×420 and 1440×600
    for (w, h) in [(640.0_f32, 420.0), (1440.0, 600.0)] {
        let fonts = build_driver_font_definitions();
        let app = UiApp::from_session(Session::from_document(build_column_doc(20, 60, SubcolumnMode::Single)));
        let mut harness = Harness::builder().with_size(egui::vec2(w, h)).allow_missing_glyphs().build_ui_state(
            |ui, state: &mut GridHarnessState| {
                let mut meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                render_grid_harness(ui, &mut meta, state);
            },
            GridHarnessState::new(app),
        );
        harness.ctx.set_fonts(fonts);
        harness.run();
        harness.check_accessibility();
        let scrollbars = harness.query_all_by_role(egui::accesskit::Role::ScrollBar).count();
        assert_eq!(scrollbars, 2, "{w}x{h}: expected 2 active Role::ScrollBar nodes");
        let bounds: Vec<AccessKitRoleBounds> = harness
            .root()
            .children_recursive()
            .filter_map(|n| {
                let ak = n.accesskit_node();
                if ak.is_hidden() || ak.bounding_box().is_none() {
                    return None;
                }
                Some(AccessKitRoleBounds {
                    id: ak.locate().0,
                    role: ak.role(),
                    label: ak.label().or_else(|| ak.value()).unwrap_or_else(|| String::from("<unnamed>")),
                    rect: n.rect(),
                })
            })
            .collect();
        let sb_violations = check_role_bounds_scrollbar_non_overlap(&bounds);
        assert!(sb_violations.is_empty(), "{w}x{h} ScrollBar overlap violations: {sb_violations:?}");
        let _ = tr("en", "table.column");
    }

    Ok(())
}

#[test]
fn column_grid_1m_cells_virtualization_and_release_frame_budget() -> Result<(), String> {
    let mut driver = build_column_driver(1_000, 1_000, SubcolumnMode::Single, 1280.0, 800.0);
    // Warm up caches
    driver.run_settled();

    let cell0 = widget_center(&driver, "grid.cell.0.0.0", true)?;
    let frames = 24usize;
    let mut total_ms = 0.0_f64;
    let mut max_ms = 0.0_f64;

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
    println!(
        "1M-cell Column grid ({} frames): mean={mean_ms:.2}ms max={max_ms:.2}ms max_input_to_paint={max_input_ms:.2}ms rendered_cells={}",
        frames,
        driver.state().last_report.rendered_cells
    );
    let steady_budget_ms = if cfg!(debug_assertions) { 120.0 } else { 16.67 };
    let input_budget_ms = if cfg!(debug_assertions) { 250.0 } else { 50.0 };
    assert!(
        mean_ms <= steady_budget_ms,
        "1M-cell Column grid mean frame budget exceeded: mean={mean_ms:.2}ms (max={max_ms:.2}ms, limit={steady_budget_ms:.2}ms)"
    );
    assert!(max_input_ms <= input_budget_ms, "1M-cell Column grid input-to-paint exceeded: {max_input_ms:.2}ms > {input_budget_ms:.2}ms");
    Ok(())
}

#[test]
fn column_grid_cmd_ctrl_cut_and_paste_roundtrip() -> Result<(), String> {
    let mut driver = build_column_driver(4, 10, SubcolumnMode::Single, 1024.0, 640.0);
    let src_val = driver
        .state()
        .app
        .document()
        .data_sheet(&SheetId::from("col-main"))
        .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
        .and_then(|c| c.value.finite_f64())
        .ok_or("missing (0,0,0) value")?;

    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_key("x", KeyModifiers { meta: true, ..Default::default() })?;
    let cleared = driver
        .state()
        .app
        .document()
        .data_sheet(&SheetId::from("col-main"))
        .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
        .is_some_and(|c| c.value.is_empty());
    assert!(cleared, "Cmd+X must cut/clear selected ColumnGrid cell");

    driver.click_widget_id("grid.cell.1.1.0")?;
    driver.press_key("v", KeyModifiers { ctrl: true, ..Default::default() })?;
    let pasted = driver
        .state()
        .app
        .document()
        .data_sheet(&SheetId::from("col-main"))
        .and_then(|ds| ds.cell(CellCoord::new(1, 1, 0)))
        .and_then(|c| c.value.finite_f64());
    assert_eq!(pasted, Some(src_val), "Ctrl+V must paste cut value into active ColumnGrid cell");
    Ok(())
}
