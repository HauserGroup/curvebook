#![forbid(unsafe_code)]
//! `R10.3` Analyze Picker Tree, Search & Duplicate Disambiguation Human-Path Tests.

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, GroupedTable, MultipleVariablesTable, NestedTable,
    OneWayAnovaSpec, PartsOfWholeTable, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::compositor::FramePalette;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics, execute_widget_action};
use curvebook_ui_egui::layout::a11y::{FocusScope, WidgetRole};
use curvebook_ui_egui::layout::hit_test::hit_test_logical;
use curvebook_ui_egui::layout::verify::verify_tree_invariants;
use curvebook_ui_egui::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use curvebook_ui_egui::text_bridge::DesktopTextMeasurer;
use curvebook_ui_egui::views::a11y::labeled_meta;
use curvebook_ui_egui::views::analyze_picker::{
    DuplicateChoice, build_duplicate_analysis_controls, build_duplicate_analysis_modal_body, resolve_duplicate_intent,
};

use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible,
};

fn build_all_eight_archetypes_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("All Archetypes").map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    let col_vals = [[10.0, 11.0, 9.8, 10.5, 10.2], [14.0, 15.2, 13.8, 14.9, 14.4], [19.5, 20.1, 18.9, 20.4, 19.8]];
    for c in 0..3 {
        col.data_sets[c].title = format!("Group {}", c + 1);
        for r in 0..5 {
            col.data_sets[c].cells[r][0] = Cell::number(col_vals[c][r]);
        }
    }
    doc.add_data_sheet(DataSheet::new("ds-col", "Column Data", TableArchetype::Column(col)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).map_err(|e| e.to_string())?;
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 3) as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "XY Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-grp", "Grouped Factors", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut cont = ContingencyTable::new(2, 2).map_err(|e| e.to_string())?;
    cont.data_sets[0].cells[0] = vec![Cell::number(18.0)];
    cont.data_sets[0].cells[1] = vec![Cell::number(6.0)];
    cont.data_sets[1].cells[0] = vec![Cell::number(5.0)];
    cont.data_sets[1].cells[1] = vec![Cell::number(19.0)];
    doc.add_data_sheet(DataSheet::new("ds-cont", "Contingency 2x2", TableArchetype::Contingency(cont)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let surv = SurvivalTable::new(2, 5).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-surv", "Survival Cohort", TableArchetype::Survival(surv)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let pow = PartsOfWholeTable::new(1, 4).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-pow", "Parts of Whole", TableArchetype::PartsOfWhole(pow)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(4, 5).map_err(|e| e.to_string())?;
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].role = VariableRole::Categorical;
    mv.variables[3].role = VariableRole::Label;
    doc.add_data_sheet(DataSheet::new("ds-mv", "Multiple Variables", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let nest = NestedTable::new(2, 2, 3).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-nest", "Nested Replicates", TableArchetype::Nested(nest)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("ds-col")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn r10_3_analyze_picker_menu_toolbar_shortcut_and_all_eight_archetypes() -> Result<(), String> {
    let app = build_all_eight_archetypes_app()?;
    let mut driver = new_human_driver(app, default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 1. Open via Toolbar (`toolbar.analyze`) -> verify Column compatibility & Cancel via `modal.cancel`.
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("expected toolbar.analyze to open AnalyzeSheet"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none() {
        return Err(String::from("expected stats.one_way_anova on Column table"));
    }
    if driver.find_widget("analyze.entry.stats.survival_km").is_some() {
        return Err(String::from("survival_km must be filtered out on Column table"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected modal.cancel to dismiss AnalyzeSheet"));
    }

    // 2. Open via Analyze Menu -> `menu.ui.analyze.open`, dismiss via Escape.
    let _ = click_widget_center(&mut driver, &WidgetId::from("menu.analyze"))?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("popup.menu.analyze"), &WidgetId::from("menu.ui.analyze.open"), 120.0, 24)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("menu.ui.analyze.open"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("expected menu.ui.analyze.open to open AnalyzeSheet"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected Escape to dismiss AnalyzeSheet"));
    }

    // 3. Open via keyboard shortcut `Cmd+Shift+A`, verify `modal.close`.
    let _ = press_key_combo(&mut driver, "A", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("expected Cmd+Shift+A to open AnalyzeSheet"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.close"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected modal.close to dismiss AnalyzeSheet"));
    }

    // 4. Switch across all 8 table archetypes via Navigator and verify filtered tree + parity!
    let expectations: [(&str, &str, &str); 8] = [
        ("ds-col", "analyze.entry.stats.one_way_anova", "analyze.entry.fit.linear"),
        ("ds-xy", "analyze.entry.fit.nonlinear", "analyze.entry.stats.one_way_anova"),
        ("ds-grp", "analyze.entry.stats.factorial_anova", "analyze.entry.stats.survival_km"),
        ("ds-cont", "analyze.entry.stats.contingency", "analyze.entry.stats.one_way_anova"),
        ("ds-surv", "analyze.entry.stats.survival_km", "analyze.entry.fit.linear"),
        ("ds-pow", "analyze.entry.stats.contingency", "analyze.entry.stats.factorial_anova"),
        ("ds-mv", "analyze.entry.stats.glm", "analyze.entry.stats.nested"),
        ("ds-nest", "analyze.entry.stats.nested", "analyze.entry.fit.nonlinear"),
    ];

    for (sheet_id, expected_present, expected_absent) in expectations {
        let _ = click_widget_center(&mut driver, &WidgetId::from(format!("nav.sheet.{sheet_id}").as_str()))?;
        let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
        let _ = assert_zero_shortcut_compliance(&mut driver)?;
        if driver.find_widget(expected_absent).is_some() {
            return Err(format!("expected `{expected_absent}` to be filtered out initially for `{sheet_id}`"));
        }
        scroll_until_widget_visible(&mut driver, &WidgetId::from("analyze.entries_list"), &WidgetId::from(expected_present), 88.0, 12)?;
        if driver.find_widget(expected_absent).is_some() {
            return Err(format!("expected `{expected_absent}` to stay filtered out after scroll for `{sheet_id}`"));
        }
        let absent_id = expected_absent.strip_prefix("analyze.entry.").unwrap_or(expected_absent);
        if driver.app().canvas_dialogs.analyze.compatible_entries().iter().any(|item| item.id == absent_id) {
            return Err(format!("catalog compatibility must exclude `{absent_id}` for `{sheet_id}`"));
        }
        let _ = assert_zero_shortcut_compliance(&mut driver)?;
        let _ = click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;
    }

    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_3_analyze_picker_search_clear_collapse_confirm_and_cmd_t() -> Result<(), String> {
    let app = build_all_eight_archetypes_app()?;
    let mut driver = new_human_driver(app, default_viewport());

    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    let clear_id = WidgetId::from("analyze.search.clear");
    let clear_initial = driver.find_widget(clear_id.as_str()).ok_or_else(|| String::from("missing analyze.search.clear"))?;
    if clear_initial.state.enablement.enabled {
        return Err(String::from("Clear button should be disabled when search query is empty"));
    }

    // 1. Live search filtering via click_and_type on `analyze.search`.
    let _ = click_and_type(&mut driver, &WidgetId::from("analyze.search"), "welch")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let clear_active = driver.find_widget(clear_id.as_str()).ok_or_else(|| String::from("missing clear button after typing"))?;
    if !clear_active.state.enablement.enabled {
        return Err(String::from("Clear button must become enabled once search query is non-empty"));
    }
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none() || driver.find_widget("analyze.entry.stats.two_group").is_none() {
        return Err(String::from("expected Welch search to match two_group and one_way_anova"));
    }
    if driver.find_widget("analyze.entry.transform.math").is_some() {
        return Err(String::from("expected Welch search to filter out transform.math"));
    }

    // 2. Empty-search feedback state -> Clear button restores full tree.
    let _ = click_and_type(&mut driver, &WidgetId::from("analyze.search"), "zzznomatch")?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    if driver.find_widget("analyze.empty_feedback").is_none() {
        return Err(String::from("expected analyze.empty_feedback when search has zero matches"));
    }
    let _ = click_widget_center(&mut driver, &clear_id)?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    if driver.find_widget("analyze.entry.transform.math").is_none() {
        return Err(String::from("expected Clear button to restore transform.math"));
    }

    // 3. Collapsible category headers toggle expand/collapse cleanly.
    let grp_transform = WidgetId::from("analyze.tree.group.transform");
    let _ = click_widget_center(&mut driver, &grp_transform)?;
    if driver.find_widget("analyze.entry.transform.math").is_some() {
        return Err(String::from("expected collapsing transform group to hide transform.math"));
    }
    let grp_node = driver.find_widget(grp_transform.as_str()).ok_or_else(|| String::from("missing transform group header"))?;
    if grp_node.a11y.expanded != Some(false) {
        return Err(format!("expected expanded=Some(false) after collapse, got {:?}", grp_node.a11y.expanded));
    }
    let _ = click_widget_center(&mut driver, &grp_transform)?;
    if driver.find_widget("analyze.entry.transform.math").is_none() {
        return Err(String::from("expected expanding transform group to show transform.math"));
    }

    // 4. Category sidebar filtering & selection summary pane update.
    let _ = click_widget_center(&mut driver, &WidgetId::from("analyze.cat.column"))?;
    if driver.find_widget("analyze.entry.transform.math").is_some() {
        return Err(String::from("Column category filter must exclude transform.math"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("analyze.entry.stats.one_way_anova"))?;
    if driver.app().canvas_dialogs.analyze.selected_entry_id.as_deref() != Some("stats.one_way_anova") {
        return Err(String::from("expected selected_entry_id == stats.one_way_anova"));
    }
    let summary_runs_ok =
        driver.layout_tree().text_runs.iter().any(|r| r.owner_widget_id.as_str() == "analyze.summary_pane" && r.text.contains("One-way ANOVA"));
    if !summary_runs_ok {
        return Err(String::from("expected analyze.summary_pane to display selected One-way ANOVA title & summary"));
    }

    // 5. Verify ModalDialog parity at HiDPI 2.0x and full parity at compact 1024x640 before confirming.
    driver.set_metrics(ViewportMetrics::new(1280.0, 800.0, 2.0));
    let hidpi_tree = driver.layout_tree();
    let hidpi_report = driver.verify_parity();
    let modal_hidpi_violations: Vec<_> = hidpi_report
        .violations
        .into_iter()
        .filter(|v| {
            hidpi_tree.node(&v.widget_id).is_some_and(|n| n.z_layer >= ZLayer::ModalDialog)
                || v.widget_id.starts_with("analyze.")
                || v.widget_id.starts_with("modal.")
        })
        .collect();
    if !modal_hidpi_violations.is_empty() {
        return Err(format!("HiDPI 2.0x Analyze modal parity violations: {modal_hidpi_violations:?}"));
    }
    driver.set_metrics(ViewportMetrics::new(1024.0, 640.0, 1.0));
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    driver.set_metrics(default_viewport());

    // 6. Confirm via `modal.ok` -> ResultSheet created & active -> `Cmd+T` opens EditAnalysisParameters!
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected AnalyzeSheet to close after modal.ok"));
    }
    if driver.app().document().result_sheets.len() != 1 {
        return Err(format!("expected 1 ResultSheet, got {}", driver.app().document().result_sheets.len()));
    }
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("expected Cmd+T on newly created ResultSheet to open EditAnalysisParameters"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;

    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

fn build_duplicate_prompt_tree(app: &UiApp, metrics: ViewportMetrics) -> curvebook_ui_egui::layout::LayoutTree {
    let measurer = DesktopTextMeasurer::global();
    let palette = FramePalette::from_theme(&app.theme);
    let mut builder = LayoutBuilder::new(app, metrics, measurer, &palette);
    builder.tree.active_focus_scope = FocusScope::ModalDialog;
    let w = metrics.physical_width_px();
    let h = metrics.physical_height_px();
    let mw = (w * 16 / 25).clamp(180, w.saturating_sub(24));
    let mh = (h * 17 / 25).clamp(120, h.saturating_sub(24));
    let mx = i32::try_from((w.saturating_sub(mw)) / 2).unwrap_or(0);
    let my = i32::try_from((h.saturating_sub(mh)) / 2).unwrap_or(0);
    let modal_rect = LayoutRectPx::new(mx, my, mw, mh);
    let body_rect = LayoutRectPx::new(mx + 12, my + 36, mw.saturating_sub(24), mh.saturating_sub(74));
    let modal_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.dialog"),
        parent: None,
        z_layer: ZLayer::ModalDialog,
        rect_px: modal_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Analysis Already Exists", FocusScope::ModalDialog, "Escape"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&modal_wid), modal_rect, palette.surface);
    build_duplicate_analysis_modal_body(&mut builder, modal_wid, body_rect);
    builder.finish()
}

fn verify_duplicate_direct_controls_parity(app: &UiApp, metrics: ViewportMetrics) -> Result<(), String> {
    let measurer = DesktopTextMeasurer::global();
    let palette = FramePalette::from_theme(&app.theme);
    let mut builder = LayoutBuilder::new(app, metrics, measurer, &palette);
    builder.tree.active_focus_scope = FocusScope::ModalDialog;
    let w = metrics.physical_width_px();
    let h = metrics.physical_height_px();
    let modal_rect = LayoutRectPx::new(0, 0, w, h);
    let body_rect = LayoutRectPx::new(16, 40, w.saturating_sub(32), h.saturating_sub(80));
    let modal_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.dialog"),
        parent: None,
        z_layer: ZLayer::ModalDialog,
        rect_px: modal_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Analysis Already Exists", FocusScope::ModalDialog, "Escape"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&modal_wid), modal_rect, palette.surface);
    if let ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, existing_result_ids, pending_entry_id } = &app.canvas_dialogs.active_dialog {
        build_duplicate_analysis_controls(&mut builder, modal_wid, body_rect, source_sheet_id, existing_result_ids, pending_entry_id);
    }
    let tree = builder.finish();
    let report = verify_tree_invariants(&tree);
    if !report.is_clean() {
        return Err(format!("duplicate controls parity violations at {metrics:?}: {:?}", report.violations));
    }
    Ok(())
}

fn click_duplicate_control_at_center(app: &mut UiApp, metrics: ViewportMetrics, wid: &str) -> Result<(), String> {
    let tree = build_duplicate_prompt_tree(app, metrics);
    let report = verify_tree_invariants(&tree);
    if !report.is_clean() {
        return Err(format!("duplicate controls parity violations: {:?}", report.violations));
    }
    let node = tree.node(wid).ok_or_else(|| format!("duplicate widget `{wid}` missing"))?;
    let [cx, cy] = node.bounds.visible_rect_px.center();
    let [lx, ly] = metrics.physical_to_logical_px([f64::from(cx), f64::from(cy)]);
    let hit = hit_test_logical(&tree, lx, ly).ok_or_else(|| format!("hit_test_logical missed `{wid}`"))?;
    if hit.id.as_str() != wid {
        return Err(format!("expected center hit on `{wid}`, hit `{}`", hit.id.as_str()));
    }
    let _ = execute_widget_action(app, &hit.action, metrics)?;
    Ok(())
}

#[test]
fn r10_3_duplicate_analysis_disambiguation_flow() -> Result<(), String> {
    let mut app = build_all_eight_archetypes_app()?;
    let mut doc = app.document().clone();
    let r1 = ResultSheet::new(
        AnalysisSpec::new("res-anova-1", "ds-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let r2 = ResultSheet::new(
        AnalysisSpec::new("res-anova-2", "ds-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    doc.add_result_sheet(r1).map_err(|e| e.to_string())?;
    doc.add_result_sheet(r2).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("ds-col")).map_err(|e| e.to_string())?;
    app = UiApp::from_session(Session::from_document(doc));

    // 1. Human-path confirmation of matching analysis opens DuplicateAnalysisPrompt with both existing sheets.
    let mut driver = new_human_driver(app, default_viewport());
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("analyze.entry.stats.one_way_anova"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    match &driver.app().canvas_dialogs.active_dialog {
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, pending_entry_id, .. } => {
            if existing_result_ids.len() != 2 || pending_entry_id != "stats.one_way_anova" {
                return Err(format!("unexpected DuplicateAnalysisPrompt state: {existing_result_ids:?} / {pending_entry_id}"));
            }
        }
        other => return Err(format!("expected DuplicateAnalysisPrompt, got {other:?}")),
    }

    // 2. Drive interactive `build_duplicate_analysis_controls` (ModifyExisting + multi-target selection + CreateNew).
    let mut prompt_app = driver.app().clone();
    for dup_metrics in [ViewportMetrics::new(1280.0, 800.0, 2.0), ViewportMetrics::new(1024.0, 640.0, 1.0)] {
        verify_duplicate_direct_controls_parity(&prompt_app, dup_metrics)?;
    }
    let metrics = default_viewport();
    click_duplicate_control_at_center(&mut prompt_app, metrics, "analyze_picker.dup.modify_existing")?;
    if prompt_app.view_shards.analyze_picker.duplicate_choice != DuplicateChoice::ModifyExisting {
        return Err(String::from("expected ModifyExisting choice"));
    }

    click_duplicate_control_at_center(&mut prompt_app, metrics, "analyze_picker.dup.target.res-anova-2")?;
    if prompt_app.view_shards.analyze_picker.duplicate_target_id.as_deref() != Some("res-anova-2") {
        return Err(String::from("expected target res-anova-2 selected"));
    }
    let existing = vec![SheetId::from("res-anova-1"), SheetId::from("res-anova-2")];
    let modify_intent = resolve_duplicate_intent(&prompt_app, &prompt_app.view_shards.analyze_picker, &existing)
        .ok_or_else(|| String::from("missing modify intent"))?;
    if modify_intent.command_id != "analysis.updateParams" || modify_intent.params["sheet_id"] != "res-anova-2" {
        return Err(format!("unexpected modify intent: {modify_intent:?}"));
    }

    click_duplicate_control_at_center(&mut prompt_app, metrics, "analyze_picker.dup.create_new")?;
    if prompt_app.view_shards.analyze_picker.duplicate_choice != DuplicateChoice::CreateNew {
        return Err(String::from("expected CreateNew choice"));
    }
    let create_intent = resolve_duplicate_intent(&prompt_app, &prompt_app.view_shards.analyze_picker, &existing)
        .ok_or_else(|| String::from("missing create intent"))?;
    if create_intent.command_id != "analysis.run" {
        return Err(format!("expected analysis.run for CreateNew, got {:?}", create_intent.command_id));
    }

    // 3. Execute `dup_confirm` for ModifyExisting -> modifies `res-anova-2` without creating a 3rd ResultSheet!
    click_duplicate_control_at_center(&mut prompt_app, metrics, "analyze_picker.dup.target.res-anova-2")?;
    let _ = execute_widget_action(
        &mut prompt_app,
        &WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("dup_confirm") },
        metrics,
    )?;
    if prompt_app.canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected DuplicateAnalysisPrompt to close after dup_confirm"));
    }
    if prompt_app.document().result_sheets.len() != 2 {
        return Err(format!("expected 2 ResultSheets after ModifyExisting, got {}", prompt_app.document().result_sheets.len()));
    }

    Ok(())
}
