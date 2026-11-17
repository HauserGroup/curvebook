#![forbid(unsafe_code)]
//! `R10.5` Wave 5 Shard `w5e` Human-Path Verification Suite:
//! - Scrollable multi-column result tables (all columns, section headers, column sizing, 2-axis scroll, selection & copy)
//! - Clickable sub-sheet tabs with horizontal overflow prev/next + wheel scroll & keyboard switching
//! - Expandable Interpret / Assumption Checklist side drawer (`R9.7` diagnostics-only, zero remediation buttons) & diagnostic banners (`R2-S10-09`)
//! - `Cmd/Ctrl+T` and `Parameters (Cmd+T)` button -> `EditAnalysisParameters`
//! - Multi-theme, HiDPI, and RTL/CJK multi-locale `R10.2` zero-shortcut parity.

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, DataSheet, OneWayAnovaSpec,
    OneWayMethod, ProjectDocument, RemediationKind, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId,
    TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ChecklistSeverityFilter};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::views::results::InterpretDrawerTab;
use serde_json::json;

type TestRes = Result<(), String>;

fn build_column_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Drug 10mg");
    col.data_sets[2].title = String::from("Drug 30mg");
    let g0 = [10.2, 11.0, 9.9, 10.5, 10.8];
    let g1 = [14.1, 15.2, 13.8, 14.9, 15.0];
    let g2 = [21.3, 22.1, 20.5, 21.8, 22.4];
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(g0[r]);
        col.data_sets[1].cells[r][0] = Cell::number(g1[r]);
        col.data_sets[2].cells[r][0] = Cell::number(g2[r]);
    }
    let sheet = DataSheet::new("col-w5e", "Pharmacology Assay", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(sheet).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("col-w5e")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

fn make_wide_many_row_tab(id: &str, title: &str, kind: ResultTabKind, rows_n: usize) -> ResultTab {
    let headers = vec![
        String::from("Estimate"),
        String::from("Difference"),
        String::from("SE of Diff."),
        String::from("Test Statistic"),
        String::from("df"),
        String::from("95% CI Lower..Upper"),
        String::from("Raw P"),
        String::from("Adjusted P"),
        String::from("VIF"),
        String::from("Significant"),
    ];
    let mut row_labels = Vec::with_capacity(rows_n);
    let mut rows = Vec::with_capacity(rows_n);
    for r in 0..rows_n {
        if r == 0 {
            row_labels.push(String::from("Primary Comparisons"));
            rows.push(vec![ResultCell::Empty; headers.len()]);
            continue;
        }
        row_labels.push(format!("Pair {} vs Baseline", r));
        let p = if r % 2 == 1 { 0.0012 * (r as f64).min(8.0) } else { 0.1420 };
        let stars = if p < 0.001 {
            "***"
        } else if p < 0.01 {
            "**"
        } else if p < 0.05 {
            "*"
        } else {
            ""
        };
        rows.push(vec![
            ResultCell::Number { value: 10.5 + (r as f64) * 0.4, prefix: None, units: Some(String::from("mg/L")) },
            ResultCell::Number { value: 3.25 + (r as f64) * 0.25, prefix: None, units: None },
            ResultCell::Number { value: 0.412, prefix: None, units: None },
            ResultCell::Number { value: 7.88 + (r as f64) * 0.1, prefix: None, units: None },
            ResultCell::Number { value: 24.0, prefix: None, units: None },
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(2.1 + (r as f64) * 0.1), upper: CiBound::Finite(4.8 + (r as f64) * 0.1) },
            ResultCell::PValue { raw: p * 0.5, formatted: format!("{:.4}", p * 0.5), stars: String::from(stars) },
            ResultCell::PValue { raw: p, formatted: format!("{p:.4}"), stars: String::from(stars) },
            ResultCell::Number { value: 1.45 + (r as f64) * 0.05, prefix: None, units: None },
            ResultCell::Boolean(!stars.is_empty()),
        ]);
    }
    ResultTab {
        id: id.to_owned(),
        title: title.to_owned(),
        kind,
        column_headers: headers,
        row_labels,
        rows,
        footnotes: vec![
            String::from("Multiplicity-adjusted P values reported alongside 95% confidence bounds."),
            String::from("Diagnostics-only presentation; parameter edits are explicit via Cmd/Ctrl+T."),
        ],
    }
}

fn build_rich_multi_tab_result_app() -> Result<UiApp, String> {
    let mut app = build_column_app()?;
    let mut doc = app.document().clone();

    let spec = AnalysisSpec::new(
        "rs-rich-1",
        "col-w5e",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, compute_effect_sizes: true, ..OneWayAnovaSpec::default() }),
    )
    .map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.status = ResultStatus::UpToDate;
    rs.scalars.push(ResultScalarEntry { key: String::from("F statistic"), data_set_index: None, value: 42.18, units: None });
    rs.scalars.push(ResultScalarEntry { key: String::from("R squared"), data_set_index: None, value: 0.875, units: None });
    rs.tabs = vec![
        make_wide_many_row_tab("main-summary", "Main Summary", ResultTabKind::MainSummary, 32),
        make_wide_many_row_tab("multiple-comparisons", "Multiple comparisons", ResultTabKind::MultipleComparisons, 30),
        make_wide_many_row_tab("residuals", "Residuals", ResultTabKind::Residuals, 28),
        make_wide_many_row_tab("cleaned-data", "Cleaned data", ResultTabKind::DescriptiveSummary, 16),
        make_wide_many_row_tab("clustered-data", "Clustered data", ResultTabKind::DescriptiveSummary, 14),
        make_wide_many_row_tab("vif", "VIF", ResultTabKind::Diagnostics, 10),
        make_wide_many_row_tab("classification-table", "Classification table", ResultTabKind::Diagnostics, 12),
        make_wide_many_row_tab("diagnostics", "Diagnostics", ResultTabKind::Diagnostics, 12),
        make_wide_many_row_tab("normal-qq", "Normal Q-Q", ResultTabKind::NormalQq, 18),
        make_wide_many_row_tab("covariance-matrix", "Covariance Matrix", ResultTabKind::CovarianceMatrix, 8),
        make_wide_many_row_tab("model-comparison", "Model Comparison", ResultTabKind::ModelComparison, 8),
    ];
    rs.checklist = vec![
        ChecklistItem {
            assumption: String::from("Group independence"),
            status: ChecklistStatus::Pass,
            diagnostic_metric: Some(String::from("Design check passed")),
            explanation: String::from("Groups are mutually exclusive across columns."),
            remediation: None,
        },
        ChecklistItem {
            assumption: String::from("Variance homogeneity"),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some(String::from("Brown-Forsythe F = 8.42, P = 0.008")),
            explanation: String::from("Consider switching to Welch's t or exclude outlier observations."),
            remediation: Some(RemediationKind::WelchAnova),
        },
        ChecklistItem {
            assumption: String::from("Normality of residuals"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(String::from("Shapiro-Wilk W = 0.88, P = 0.031")),
            explanation: String::from("Mild positive skew observed in tail quantiles."),
            remediation: Some(RemediationKind::KruskalWallis),
        },
        ChecklistItem {
            assumption: String::from("Sample size adequacy"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("min n = 5, total N = 15")),
            explanation: String::from("Normality test power is modest at small n."),
            remediation: None,
        },
    ];

    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("rs-rich-1")).map_err(|e| e.to_string())?;
    app = UiApp::from_session(Session::from_document(doc));
    Ok(app)
}

#[test]
fn r10_5_multi_column_scrollable_table_section_headers_selection_and_copy() -> TestRes {
    let app = build_rich_multi_tab_result_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    let tree0 = driver.layout_tree();

    // Section header bar + column headers + first row visible with zero 6-row/first-col cap.
    let sec = tree0.node("result.section_header").ok_or_else(|| String::from("missing result.section_header"))?;
    if !sec.a11y.label.contains("32 x 10") {
        return Err(format!("expected section header to report 32 x 10 dimensions, got `{}`", sec.a11y.label));
    }
    let visible_hdrs = tree0.nodes.iter().filter(|n| n.id.as_str().starts_with("result.col_hdr.") && n.drawn_in_frame).count();
    if visible_hdrs < 3 {
        return Err(format!("expected multiple result columns rendered, got {visible_hdrs}"));
    }

    // Horizontal wheel scroll reveals rightmost columns (e.g. col 9 `Significant`).
    let tbl_id = WidgetId::from("result.table");
    for _ in 0..8 {
        if driver.find_widget("result.col_hdr.9").is_some_and(|n| n.drawn_in_frame) {
            break;
        }
        let _ = human_wf_common::scroll_widget_wheel(&mut driver, &tbl_id, 220.0, 0.0)?;
    }
    let hdr9 = driver.find_widget("result.col_hdr.9").ok_or_else(|| String::from("col_hdr.9 not reachable via horizontal wheel"))?;
    if !hdr9.drawn_in_frame || hdr9.a11y.label != "Significant" {
        return Err(format!("unexpected col_hdr.9 state: drawn={}, label={}", hdr9.drawn_in_frame, hdr9.a11y.label));
    }

    // Vertical wheel scroll reveals deep rows (> 23) and footnotes.
    let cell_target = WidgetId::from("result.cell.23.9");
    human_wf_common::scroll_until_widget_visible(&mut driver, &tbl_id, &cell_target, 180.0, 10)?;

    // Scroll left slightly to test CI & P-value cell selection + copy on odd row 23 (p=0.0096, **).
    for _ in 0..4 {
        if driver.find_widget("result.cell.23.7").is_some_and(|n| n.drawn_in_frame) {
            break;
        }
        let _ = human_wf_common::scroll_widget_wheel(&mut driver, &tbl_id, -180.0, 0.0)?;
    }
    let p_cell_id = WidgetId::from("result.cell.23.7");
    let _ = human_wf_common::click_widget_center(&mut driver, &p_cell_id)?;
    let sel = driver.app().view_shards.results.selection_bounds("rs-rich-1", 0);
    if sel != Some((23, 23, 7, 7)) {
        return Err(format!("expected single cell selection (23,23,7,7), got {sel:?}"));
    }

    let copy_btn = WidgetId::from("result.copy_btn");
    let _ = human_wf_common::click_widget_center(&mut driver, &copy_btn)?;
    let copied = driver.app().interaction.last_clipboard_text.clone().unwrap_or_default();
    if !copied.contains("0.0096") || !copied.contains("**") {
        return Err(format!("expected copied p-value + significance stars, got `{copied}`"));
    }
    if driver.app().view_shards.results.last_copied_tsv.as_deref() != Some(copied.as_str()) {
        return Err(String::from("ResultsViewState::last_copied_tsv out of sync with clipboard"));
    }

    // Row header click selects full row; Copy formats row label + tab-separated values across all 10 cols.
    let row_hdr = WidgetId::from("result.row_hdr.23");
    let _ = human_wf_common::click_widget_center(&mut driver, &row_hdr)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &copy_btn)?;
    let row_tsv = driver.app().interaction.last_clipboard_text.clone().unwrap_or_default();
    if !row_tsv.starts_with("Pair 23 vs Baseline\t") || row_tsv.matches('\t').count() != 10 {
        return Err(format!("expected row TSV with 10 tabs, got `{row_tsv}`"));
    }

    // Column header click selects full column; Copy formats header + all 32 rows.
    let col_hdr = WidgetId::from("result.col_hdr.7");
    let _ = human_wf_common::click_widget_center(&mut driver, &col_hdr)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &copy_btn)?;
    let col_tsv = driver.app().interaction.last_clipboard_text.clone().unwrap_or_default();
    let lines: Vec<&str> = col_tsv.lines().collect();
    if lines.first().copied() != Some("Adjusted P") || lines.len() != 33 {
        return Err(format!("expected column TSV header + 32 rows (33 lines), got {} lines", lines.len()));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 100.0)?;
    Ok(())
}

#[test]
fn r10_5_sub_sheet_tabs_overflow_controls_and_keyboard_switching() -> TestRes {
    let app = build_rich_multi_tab_result_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let tree0 = driver.layout_tree();
    let drawn_tabs = tree0.nodes.iter().filter(|n| n.id.as_str().starts_with("result.tab.") && n.drawn_in_frame).count();
    if drawn_tabs < 5 {
        return Err(format!("expected >= 5 sub-tabs without 4-tab cap, got {drawn_tabs}"));
    }

    // Next overflow button reveals later tabs (`Classification table`, `Model Comparison`).
    let next_btn = WidgetId::from("result.tabs.next");
    for _ in 0..8 {
        if driver.find_widget("result.tab.classification-table").is_some_and(|n| n.drawn_in_frame) {
            break;
        }
        let _ = human_wf_common::click_widget_center(&mut driver, &next_btn)?;
    }
    let class_tab = WidgetId::from("result.tab.classification-table");
    let _ = human_wf_common::click_widget_center(&mut driver, &class_tab)?;
    if driver.app().canvas_dialogs.result_view.active_tab_index != 6 {
        return Err(format!("expected Classification table (index 6), got {}", driver.app().canvas_dialogs.result_view.active_tab_index));
    }

    // Scroll to the final tab (`Model Comparison`, index 10) so overflow buttons are exercised
    // regardless of proportional system font widths.
    for _ in 0..8 {
        if driver.find_widget("result.tab.model-comparison").is_some_and(|n| n.drawn_in_frame) {
            break;
        }
        let _ = human_wf_common::click_widget_center(&mut driver, &next_btn)?;
    }
    if !driver.find_widget("result.tab.model-comparison").is_some_and(|n| n.drawn_in_frame) {
        return Err(String::from("expected result.tabs.next to reveal final tab `Model Comparison`"));
    }

    // Prev overflow button scrolls back toward first tabs (disabling `result.tabs.prev` at offset 0).
    let prev_btn = WidgetId::from("result.tabs.prev");
    for _ in 0..8 {
        if driver.find_widget("result.tab.main-summary").is_some_and(|n| n.drawn_in_frame)
            && driver.find_widget("result.tab.multiple-comparisons").is_some_and(|n| n.drawn_in_frame)
        {
            break;
        }
        let _ = human_wf_common::click_widget_center(&mut driver, &prev_btn)?;
    }
    if !driver.find_widget("result.tab.multiple-comparisons").is_some_and(|n| n.drawn_in_frame) {
        return Err(String::from("expected result.tabs.prev to bring `Multiple comparisons` back into view"));
    }

    // Keyboard Tab + Space activates another sub-tab cleanly.
    let mut switched = false;
    for _ in 0..24 {
        let _ = human_wf_common::press_key_combo(&mut driver, "Tab", KeyModifiers::default())?;
        if let Some(fid) = driver.app().focus_widget_id.clone()
            && fid.starts_with("result.tab.")
            && fid != "result.tab.classification-table"
        {
            let _ = human_wf_common::press_key_combo(&mut driver, "Space", KeyModifiers::default())?;
            switched = driver.app().canvas_dialogs.result_view.active_tab_index != 6;
            if switched {
                break;
            }
        }
    }
    if !switched {
        return Err(String::from("expected Tab + Space to switch active ResultTab via keyboard"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn r10_5_interpret_drawer_diagnostic_banners_and_r9_7_diagnostics_only() -> TestRes {
    let app = build_rich_multi_tab_result_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Click main diagnostic banner -> jumps to primary diagnostic detail tab (`Residuals`, index 2) without mutating document.
    let doc_before = driver.app().document().clone();
    let banner_btn = WidgetId::from("result.diagnostic_banner");
    let _ = human_wf_common::click_widget_center(&mut driver, &banner_btn)?;
    if driver.app().canvas_dialogs.result_view.active_tab_index != 2 {
        return Err(format!(
            "expected diagnostic banner click to select first detail tab Residuals (index 2), got {}",
            driver.app().canvas_dialogs.result_view.active_tab_index
        ));
    }

    // Clickable diagnostic banner detail link jumps directly to `Diagnostics` (index 7) without mutating document.
    let diag_link = WidgetId::from("result.diagnostic_banner.tab.diagnostics");
    let _ = human_wf_common::click_widget_center(&mut driver, &diag_link)?;
    if driver.app().canvas_dialogs.result_view.active_tab_index != 7 {
        return Err(format!(
            "expected diagnostic banner link to switch to Diagnostics (index 7), got {}",
            driver.app().canvas_dialogs.result_view.active_tab_index
        ));
    }

    // Clickable detail link for `Residuals` jumps back to index 2 without mutating document.
    let res_link = WidgetId::from("result.diagnostic_banner.tab.residuals");
    let _ = human_wf_common::click_widget_center(&mut driver, &res_link)?;
    if driver.app().canvas_dialogs.result_view.active_tab_index != 2 {
        return Err(format!(
            "expected Residuals banner link to switch back to Residuals (index 2), got {}",
            driver.app().canvas_dialogs.result_view.active_tab_index
        ));
    }
    if *driver.app().document() != doc_before {
        return Err(String::from("diagnostic banner inspection unexpectedly mutated ProjectDocument"));
    }

    // Expand Interpret / Assumption Checklist side drawer.
    let interp_btn = WidgetId::from("result.interpret_toggle_btn");
    let _ = human_wf_common::click_widget_center(&mut driver, &interp_btn)?;
    let drawer_tree = driver.layout_tree();
    if !drawer_tree.node("result.interpret.drawer").is_some_and(|n| n.drawn_in_frame) {
        return Err(String::from("Interpret drawer did not open when toggled"));
    }

    // R9.7 / QOL-7 strictly diagnostics-only verification: zero remediation buttons and zero prescriptive wording!
    for node in &drawer_tree.nodes {
        let lower_lbl = node.a11y.label.to_ascii_lowercase();
        let is_remediation_cmd = matches!(
            &node.action,
            curvebook_ui_egui::layout::WidgetAction::InvokeCommand { command_id, .. } if command_id == "ui.result.applyRemediation"
        );
        if is_remediation_cmd || lower_lbl.contains("apply remediation") || lower_lbl.contains("switch to welch") {
            return Err(format!("R9.7 violation: found remediation control `{}` ({})", node.id.as_str(), node.a11y.label));
        }
    }
    for run in &drawer_tree.text_runs {
        let lower = run.text.to_ascii_lowercase();
        if lower.contains("consider switching") || lower.contains("exclude outlier") {
            return Err(format!("R9.7 violation: prescriptive text leaked into drawer: `{}`", run.text));
        }
    }

    // Severity filter toggles (`Flagged` vs `All`) and drawer tab switching (`Narrative`, `Methods`).
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.filter_flagged"))?;
    if driver.app().canvas_dialogs.result_view.checklist_filter != ChecklistSeverityFilter::WarningsAndViolations {
        return Err(String::from("expected Flagged filter active"));
    }
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.filter_all"))?;
    if driver.app().canvas_dialogs.result_view.checklist_filter != ChecklistSeverityFilter::All {
        return Err(String::from("expected All filter active"));
    }

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.tab.narrative"))?;
    if driver.app().view_shards.results.interpret_tab != InterpretDrawerTab::Narrative {
        return Err(String::from("expected Narrative drawer tab selected"));
    }

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.tab.methods"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.fmt.latex"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.copy_methods_btn"))?;
    let report = driver.app().canvas_dialogs.result_view.last_methods_report.clone().unwrap_or_default();
    if !report.contains("\\section") && !report.contains("ANOVA") {
        return Err(format!("expected generated LaTeX methods report, got `{report}`"));
    }

    // Wheel scroll inside Interpret drawer body.
    let _ = human_wf_common::scroll_widget_wheel(&mut driver, &WidgetId::from("result.interpret.body"), 0.0, 80.0)?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Close drawer cleanly.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.interpret.close_btn"))?;
    if driver.layout_tree().node("result.interpret.drawer").is_some_and(|n| n.drawn_in_frame) {
        return Err(String::from("Interpret drawer stayed drawn after close click"));
    }
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Split-pane R2-S10-09 primary banner guard: two ResultSheets keep primary banner & pass parity!
    let sec_spec = AnalysisSpec::new("rs-rich-2", "col-w5e", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let mut sec_rs = ResultSheet::new(sec_spec).map_err(|e| e.to_string())?;
    sec_rs.checklist.push(ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: None,
        explanation: String::from("Skewed tail"),
        remediation: None,
    });
    driver.app_mut().session_mut().execute_as_ui("project.inspect", json!({})).map_err(|e| e.to_string())?;
    let mut doc2 = driver.app().document().clone();
    doc2.add_result_sheet(sec_rs).map_err(|e| e.to_string())?;
    let mut split_app = UiApp::from_session(Session::from_document(doc2));
    split_app.split.enabled = true;
    split_app.split.primary_sheet_id = Some(SheetId::from("rs-rich-1"));
    split_app.split.secondary_sheet_id = Some(SheetId::from("rs-rich-2"));
    let mut split_driver = human_wf_common::new_human_driver(split_app, human_wf_common::default_viewport());
    let split_tree = split_driver.layout_tree();
    if split_tree.result_banner_sheet_id.as_ref() != Some(&SheetId::from("rs-rich-1")) {
        return Err(format!("R2-S10-09 primary banner overwritten in split mode: {:?}", split_tree.result_banner_sheet_id));
    }
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut split_driver)?;
    Ok(())
}

#[test]
fn r10_5_cmd_and_ctrl_t_route_to_edit_analysis_parameters() -> TestRes {
    let app = build_column_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Create a real ANOVA ResultSheet through human Analyze flow.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    let list_id = WidgetId::from("analyze.entries_list");
    let anova_id = WidgetId::from("analyze.entry.stats.one_way_anova");
    human_wf_common::scroll_until_widget_visible(&mut driver, &list_id, &anova_id, -180.0, 6)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &anova_id)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let initial_ds_count = driver.app().document().data_sheets.len();
    let rs_id = driver.app().document().result_sheets.first().map(|r| r.id.clone()).ok_or_else(|| String::from("missing created ResultSheet"))?;

    // Cmd+T opens EditAnalysisParameters (never creates blank DataSheet!).
    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(&driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { result_sheet_id } if *result_sheet_id == rs_id) {
        return Err(format!("expected Cmd+T to open EditAnalysisParameters for {rs_id:?}, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }
    if driver.app().document().data_sheets.len() != initial_ds_count {
        return Err(String::from("Cmd+T on ResultSheet unexpectedly created a DataSheet"));
    }
    let _ = human_wf_common::press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;

    // Ctrl+T also opens EditAnalysisParameters.
    let _ = human_wf_common::press_key_combo(&mut driver, "T", KeyModifiers { ctrl: true, ..Default::default() })?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!("expected Ctrl+T to open EditAnalysisParameters, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }
    let _ = human_wf_common::press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;

    // Clicking rendered Parameters (Cmd+T) button opens EditAnalysisParameters and updates in place.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.edit_params_btn"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!(
            "expected result.edit_params_btn click to open EditAnalysisParameters, got {:?}",
            driver.app().canvas_dialogs.active_dialog
        ));
    }
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("edit.assume_gaussian"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let updated_title = driver.app().document().result_sheet(&rs_id).map(|r| r.title().to_owned()).unwrap_or_default();
    if !updated_title.contains("Kruskal-Wallis") {
        return Err(format!("expected Kruskal-Wallis after toggling Assume Gaussian, got `{updated_title}`"));
    }

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 100.0)?;
    Ok(())
}

#[test]
fn r10_5_themes_hidpi_and_multi_locale_rtl_parity() -> TestRes {
    let base_app = build_rich_multi_tab_result_app()?;
    let std_viewports = [ViewportMetrics::new(1280.0, 800.0, 1.0), ViewportMetrics::new(1024.0, 640.0, 1.0)];
    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        for locale in ["en", "de", "ja", "ar", "he"] {
            for vp in std_viewports {
                let mut app = base_app.clone();
                app.set_theme_mode(theme);
                app.set_locale(locale);
                app.view_shards.results.interpret_open_by_sheet.insert(String::from("rs-rich-1"), true);
                let mut driver = human_wf_common::new_human_driver(app, vp);
                let _ = driver.render_frame().map_err(|e| format!("render ({theme:?}, {locale}, {vp:?}): {e}"))?;
                let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)
                    .map_err(|e| format!("parity ({theme:?}, {locale}, {vp:?}): {e}"))?;
                let tree = driver.layout_tree();
                if let (Some(tbl), Some(drw)) = (tree.node("result.table"), tree.node("result.interpret.drawer")) {
                    let is_rtl = matches!(locale, "ar" | "he");
                    if is_rtl && drw.bounds.rect_px.x >= tbl.bounds.rect_px.x {
                        return Err(format!("expected RTL drawer left of table in {locale}"));
                    }
                    if !is_rtl && drw.bounds.rect_px.x <= tbl.bounds.rect_px.x {
                        return Err(format!("expected LTR drawer right of table in {locale}"));
                    }
                }
            }
        }
    }

    // HiDPI 1.5x and 2.0x: verify zero `result.*` pixel-to-hit-box parity violations and physical click roundtrip.
    for scale in [1.5, 2.0] {
        let vp = ViewportMetrics::new(1280.0, 800.0, scale);
        let mut app = base_app.clone();
        app.set_theme_mode(UiThemeMode::Dark);
        app.view_shards.results.interpret_open_by_sheet.insert(String::from("rs-rich-1"), true);
        let mut driver = human_wf_common::new_human_driver(app, vp);
        let _ = driver.render_frame().map_err(|e| format!("render HiDPI {scale}x: {e}"))?;
        let report = driver.verify_parity();
        let res_violations: Vec<_> = report.violations.iter().filter(|v| v.widget_id.starts_with("result.")).collect();
        if !res_violations.is_empty() {
            return Err(format!("HiDPI {scale}x result.* parity violations: {res_violations:?}"));
        }
        let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("result.tab.multiple-comparisons"))?;
        if driver.app().canvas_dialogs.result_view.active_tab_index != 1 {
            return Err(format!("HiDPI {scale}x click on Multiple comparisons tab failed"));
        }
    }
    Ok(())
}
