#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 1 (`SEC-18`) regression suite (`crates/ui-egui/tests/audit_w1_sec18_modals.rs`):
//! - `FA-SEC18-02`: 5-stage `Import & Paste Special` wizard commits Stage 1/3/4/5 drafts
//!   (encoding normalization, missing sentinels, text filter predicate, row-wrap block reshape
//!   including values/row & rows/dataset, info metadata constants, note summary), persists
//!   `Save Preset` across close/reopen, clears on `Reset Defaults`, and keeps `wiz.stage`
//!   synchronized on `Next >` / `< Back`.
//! - `FA-SEC18-03`: `Format Graph` Heatmap modal loads existing continuous colormap/reversed/bounds
//!   and `x_axis.tick_angle_deg`, preserves unmodified continuous/categorical color and custom/existing
//!   `tick_angle_deg` on commit, commits explicit changes on `Apply`/`OK`, and discards unapplied
//!   heatmap edits on `Cancel`.
//! - `FA-SEC18-04`: Standalone and edit-mode `Simulate XY Curve`, `Random Sample Rows`, and
//!   `Extract & Rearrange` column bindings (`binding_x`, `binding_y`, `binding_grp`) render dedicated
//!   controls, validate invalid inputs, and commit real parameters cleanly.

use curvebook_doc::{
    AnalysisKind, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet,
    GroupedTable, HeatmapColumnLabelAngle, InfoSheet, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument,
    ResultStatus, SheetId, SimulationNoise, SizeChannelMapping, SubcolumnMode, TableArchetype, TransformAnalysisSpec, VariableRole, XColumnMode,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId, PaletteId};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::import_wizard::{ImportWizardStep, WizardBlockReshape};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::grid_view::{ImportOriginKind, ImportWizardStage};
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Key, Modifiers};
use serde_json::json;

type TestRes = Result<(), String>;

struct ModalDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl ModalDriverState {
    fn new(mut app: UiApp) -> Self {
        app.set_gui_only(true);
        app.set_human_path_only(true);
        app.nav.welcome.open = false;
        app.egui_views.launcher_open = false;
        Self { app, views: EguiShellViewState::default(), intents: PendingUiIntents::new(), last_intent_error: None }
    }
}

fn make_driver(doc: ProjectDocument) -> EguiHumanPathDriver<'static, ModalDriverState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ModalDriverState::new(app),
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            st.views.canvas_view.synchronous_raster = true;
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_intent_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, ModalDriverState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn replace_focused_text(driver: &mut EguiHumanPathDriver<'_, ModalDriverState>, text: &str) -> TestRes {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)?;
    Ok(())
}

fn set_textbox(driver: &mut EguiHumanPathDriver<'_, ModalDriverState>, wid: &str, text: &str) -> TestRes {
    driver.click_widget_id(wid)?;
    replace_focused_text(driver, text)
}

fn assert_modal_parity(driver: &EguiHumanPathDriver<'_, ModalDriverState>, label: &str) -> TestRes {
    let parity = driver.verify_pixel_to_hit_box_parity();
    if !parity.is_clean() {
        return Err(format!("{label} pixel-to-hit-box parity failed: {:?}", parity.violations));
    }
    Ok(())
}

fn build_import_doc() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("SEC18 Import Test").map_err(|e| e.to_string())?;
    let info = InfoSheet::new_preseeded("info-1", "Project Info").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).map_err(|e| e.to_string())?;
    let mut ds = DataSheet::new("ds-col", "Target Sheet", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-col"));
    Ok(doc)
}

fn build_heatmap_doc() -> Result<(ProjectDocument, SheetId), String> {
    let mut doc = ProjectDocument::new("SEC18 Heatmap Test").map_err(|e| e.to_string())?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 3, 3).map_err(|e| e.to_string())?;
    for (c, title) in ["Ctrl", "DrugA", "DrugB"].into_iter().enumerate() {
        grp.data_sets[c].title = String::from(title);
    }
    for r in 0..3 {
        grp.row_titles[r] = format!("R{}", r + 1);
        for c in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number((r * 3 + c + 1) as f64 * 4.0);
        }
    }
    let grp_ds = DataSheet::new("ds-grp", "Matrix", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;

    let mut gs = GraphSheet::new_uninitialized_for_data("g-heat", &grp_ds).map_err(|e| e.to_string())?;
    gs.title = String::from("Heatmap");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    gs.spec.x_axis.tick_angle_deg = 45.0;
    gs.spec.heatmap_labels.column_angle = HeatmapColumnLabelAngle::Angled45;
    gs.spec.heatmap_labels.annotate_cells = true;
    gs.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Continuous {
            column: 1,
            ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: true },
            domain_clamp: Some((2.0, 42.0)),
        },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    let gid = gs.id.clone();
    doc.add_data_sheet(grp_ds).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    Ok((doc, gid))
}

fn build_transform_doc() -> Result<(ProjectDocument, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("SEC18 Transform Test").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 3)
        .map_err(|e| e.to_string())?;
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Ctrl");
    xy.data_sets[1].title = String::from("Treated");
    for r in 0..3 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(25.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "XY Sheet", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(4, 10).map_err(|e| e.to_string())?;
    let names = ["ID", "Dose", "Response", "Cohort"];
    for (idx, name) in names.iter().enumerate() {
        mv.variables[idx].title = String::from(*name);
        mv.variables[idx].role = if idx == 3 { VariableRole::Categorical } else { VariableRole::Continuous };
    }
    for r in 0..10 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = Cell::number((r as f64 + 1.0) * 0.5);
        mv.variables[2].cells[r] = Cell::number(100.0 - r as f64 * 3.0);
        mv.variables[3].cells[r] = Cell::text(if r % 2 == 0 { "A" } else { "B" });
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok((doc, SheetId::new("ds-xy"), SheetId::new("ds-mv")))
}

#[test]
fn fa_sec18_02_import_wizard_next_back_stage_sync_and_all_stage_drafts_commit() -> TestRes {
    let mut driver = make_driver(build_import_doc()?);
    driver.state_mut().intents.push(UiIntent::OpenImportWizard {
        origin: ImportOriginKind::Clipboard,
        is_live_link: false,
        target_sheet_id: Some(SheetId::new("ds-col")),
    });
    commit_intents(&mut driver);
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Source);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Source);

    // 1. Verify Next / Back keeps backend `wiz.stage` synchronized across all 5 steps.
    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Preview);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Source);

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::FilterAndUnstack);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Filter);

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::PlacementAndReshape);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Placement);

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Metadata);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Provenance);

    driver.click_widget_id("modal.back")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::PlacementAndReshape);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Placement);

    driver.click_widget_id("modal.back")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::FilterAndUnstack);
    assert_eq!(driver.state().app.grid.import_wizard.stage, ImportWizardStage::Filter);

    // 2. Configure Stage 1 (Comma, UTF-16 normalization, custom missing sentinel, CSV w/ KV headers).
    driver.click_widget_id("import_paste.tab.source")?;
    driver.click_widget_id("import_paste.delim.comma")?;
    driver.click_widget_id("import_paste.encoding.utf16")?;
    set_textbox(&mut driver, "import_paste.input.missing_tokens", "MISS")?;
    driver.click_widget_id("import_paste.input.csv")?;
    driver.state_mut().views.import_wizard.raw_source_draft =
        String::from("\0# Assay: Kinase\n# Temp = 37.5\nKEEP_10,MISS,2.5*\nDROP_99,11,12\nKEEP_20,13,MISS\nKEEP_30,14,15");
    driver.run_settled();

    // 3. Configure Stage 3 (`filter_predicate_draft`).
    driver.click_widget_id("import_paste.tab.filter")?;
    set_textbox(&mut driver, "import_paste.input.filter_pattern", "KEEP_")?;

    // 4. Configure Stage 4 (`RowWrapBlock`, Values/Row=3, Rows/DataSet=2).
    driver.click_widget_id("import_paste.tab.placement")?;
    driver.click_widget_id("import_paste.reshape.row_wrap")?;
    set_textbox(&mut driver, "import_paste.input.plate_values_per_row", "3")?;
    set_textbox(&mut driver, "import_paste.input.plate_rows_per_dataset", "2")?;

    // 5. Configure Stage 5 (provenance note, Extract Info constants, Attach Summary note) and Import.
    driver.click_widget_id("import_paste.tab.notes")?;
    set_textbox(&mut driver, "import_paste.input.notes", "Batch QC verified")?;
    driver.click_widget_id("import_paste.opt.extract_info_metadata")?;
    driver.click_widget_id("import_paste.opt.add_floating_note")?;
    assert_modal_parity(&driver, "ImportWizard Stage 5")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);

    // Inspect committed DataSheet: 9 cells from KEEP_ rows reshaped into 2x3 + 1x3 blocks (2 rows x 6 cols).
    let doc = driver.state().app.document();
    let ds = doc.data_sheet(&SheetId::new("ds-col")).ok_or("missing ds-col")?;
    let TableArchetype::Column(col_tbl) = &ds.archetype else {
        return Err(String::from("expected Column table"));
    };
    assert_eq!(col_tbl.data_sets.len(), 6, "3-wide x 2-row blocks across 9 values must span 6 columns");
    assert_eq!(ds.archetype.data_cell(curvebook_doc::CellCoord::new(0, 0, 0)), Some(&Cell::text("KEEP_10")));
    assert_eq!(ds.archetype.data_cell(curvebook_doc::CellCoord::new(0, 1, 0)), Some(&Cell::empty()));
    let c02 = ds.archetype.data_cell(curvebook_doc::CellCoord::new(0, 2, 0)).ok_or("missing (0,2)")?;
    assert!(c02.excluded, "asterisk exclusion 2.5* must survive serialization");
    assert_eq!(ds.archetype.data_cell(curvebook_doc::CellCoord::new(0, 3, 0)), Some(&Cell::text("KEEP_30")));
    assert_eq!(ds.archetype.data_cell(curvebook_doc::CellCoord::new(1, 0, 0)), Some(&Cell::text("KEEP_20")));

    // Inspect InfoSheet constants & notes_markdown from Stage 5.
    let info = doc.info_sheet(&SheetId::new("info-1")).ok_or("missing info-1")?;
    let assay_val = info.constants.iter().find(|c| c.name == "Assay").map(|c| &c.value);
    let temp_val = info.constants.iter().find(|c| c.name == "Temp").map(|c| &c.value);
    assert_eq!(assay_val, Some(&ConstantValue::Text(String::from("Kinase"))));
    assert_eq!(temp_val, Some(&ConstantValue::Number(37.5)));
    assert!(info.notes_markdown.contains("Batch QC verified"), "provenance note missing from InfoSheet: {:?}", info.notes_markdown);
    assert!(info.notes_markdown.contains("[Import Summary]"), "summary note missing from InfoSheet: {:?}", info.notes_markdown);
    Ok(())
}

#[test]
fn fa_sec18_02_save_preset_persists_across_reopen_and_reset_defaults_clears() -> TestRes {
    let mut driver = make_driver(build_import_doc()?);
    driver.state_mut().intents.push(UiIntent::OpenImportWizard {
        origin: ImportOriginKind::Clipboard,
        is_live_link: false,
        target_sheet_id: Some(SheetId::new("ds-col")),
    });
    commit_intents(&mut driver);

    driver.click_widget_id("import_paste.delim.comma")?;
    driver.click_widget_id("import_paste.encoding.latin1")?;
    set_textbox(&mut driver, "import_paste.input.missing_tokens", "NULL, -")?;
    driver.click_widget_id("import_paste.tab.filter")?;
    set_textbox(&mut driver, "import_paste.input.filter_pattern", "PASS")?;
    driver.click_widget_id("import_paste.tab.placement")?;
    driver.click_widget_id("import_paste.reshape.plate96")?;
    driver.click_widget_id("import_paste.tab.notes")?;
    driver.click_widget_id("import_paste.opt.extract_info_metadata")?;

    driver.click_widget_id("import_paste.btn.save_default")?;
    assert!(driver.state().views.import_wizard.saved_preset.is_some());
    assert!(driver.state().views.import_wizard.preset_saved_feedback);

    // Cancel and re-open: saved preset must restore cleanly onto both backend `wiz` and UI drafts.
    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.grid.import_wizard.open);
    driver.state_mut().intents.push(UiIntent::OpenImportWizard {
        origin: ImportOriginKind::Clipboard,
        is_live_link: false,
        target_sheet_id: Some(SheetId::new("ds-col")),
    });
    commit_intents(&mut driver);

    assert_eq!(driver.state().app.grid.import_wizard.delimiter, ',');
    assert_eq!(driver.state().views.import_wizard.encoding_name, "Latin-1");
    assert_eq!(driver.state().views.import_wizard.missing_tokens_draft, "NULL, -");
    assert_eq!(driver.state().views.import_wizard.filter_predicate_draft, "PASS");
    assert_eq!(driver.state().views.import_wizard.block_reshape, WizardBlockReshape::Plate96);
    assert!(driver.state().views.import_wizard.extract_info_metadata);

    // Reset Defaults must clear the saved preset and restore baseline defaults.
    driver.click_widget_id("import_paste.tab.notes")?;
    driver.click_widget_id("import_paste.btn.reset_defaults")?;
    assert!(driver.state().views.import_wizard.saved_preset.is_none());
    assert_eq!(driver.state().app.grid.import_wizard.delimiter, '\t');
    assert_eq!(driver.state().views.import_wizard.encoding_name, "UTF-8");
    assert_eq!(driver.state().views.import_wizard.missing_tokens_draft, "NA, NaN, .");
    assert!(driver.state().views.import_wizard.filter_predicate_draft.is_empty());
    assert_eq!(driver.state().views.import_wizard.block_reshape, WizardBlockReshape::None);
    assert!(!driver.state().views.import_wizard.extract_info_metadata);
    Ok(())
}

#[test]
fn fa_sec18_03_heatmap_modal_loads_existing_preserves_unmodified_and_cancels_cleanly() -> TestRes {
    let (doc, gid) = build_heatmap_doc()?;
    let mut driver = make_driver(doc);

    // 1. Open FormatGraph on pre-configured Plasma (reversed, [2, 42]) + 45 deg tick angle.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &gid));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
        driver.run_settled();
    }
    assert_eq!(driver.state().views.graph_modals.heatmap_colormap, ColormapId::Plasma);
    assert!(driver.state().views.graph_modals.heatmap_reversed);
    assert!(!driver.state().views.graph_modals.heatmap_auto_range);
    assert_eq!(driver.state().views.graph_modals.heatmap_min_draft, "2");
    assert_eq!(driver.state().views.graph_modals.heatmap_max_draft, "42");
    assert!((driver.state().views.graph_modals.heatmap_tick_angle_deg - 45.0).abs() < 1e-6);
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.cmap.Plasma").is_some_and(|e| e.value == Some(json!({ "selected": true }))));

    // 2. Toggle only cell annotations on Labels & Ticks and commit OK: color & tick angle must stay preserved!
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.angle.45").is_some_and(|e| e.value == Some(json!({ "selected": true }))));
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-heat")?;
        assert!(!gs.spec.heatmap_labels.annotate_cells);
        assert!((gs.spec.x_axis.tick_angle_deg - 45.0).abs() < 1e-6, "tick_angle_deg was overwritten");
        let mv = gs.spec.multivariate.as_ref().ok_or("missing multivariate")?;
        assert!(matches!(
            mv.color,
            ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: true }, domain_clamp: Some((lo, hi)), .. }
            if (lo - 2.0).abs() < 1e-9 && (hi - 42.0).abs() < 1e-9
        ));
    }

    // 3. Preserve Categorical color mapping & custom 30 deg tick angle when toggling annotations.
    {
        let mut custom_mv = driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.clone()).ok_or("missing mv")?;
        custom_mv.color = ColorChannelMapping::Categorical { column: 0, palette: PaletteId::default(), level_order_override: None };
        driver
            .state_mut()
            .app
            .execute_command("graph.setSpec", json!({ "sheet_id": gid.as_str(), "multivariate": custom_mv }))
            .map_err(|e| e.to_string())?;
        driver
            .state_mut()
            .app
            .execute_command("graph.setAxes", json!({ "sheet_id": gid.as_str(), "axis": "x", "tick_angle_deg": 30.0 }))
            .map_err(|e| e.to_string())?;
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &gid));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
        driver.run_settled();
    }
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    driver.click_widget_id("modal.ok")?;
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-heat after categorical preserve")?;
        assert!((gs.spec.x_axis.tick_angle_deg - 30.0).abs() < 1e-6);
        assert!(matches!(gs.spec.multivariate.as_ref().map(|m| &m.color), Some(ColorChannelMapping::Categorical { .. })));
    }

    // 4. Explicit Apply -> DivergingBlueRed & 90 deg, then edit to Grayscale and Cancel -> DivergingBlueRed & 90 deg remain!
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &gid));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
        driver.run_settled();
    }
    driver.click_widget_id("edit.tab.format_graph.color_mapping")?;
    driver.click_widget_id("format_graph.heatmap.cmap.DivergingBlueRed")?;
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    driver.click_widget_id("format_graph.heatmap.angle.90")?;
    driver.click_widget_id("modal.apply")?;
    driver.click_widget_id("edit.tab.format_graph.color_mapping")?;
    driver.click_widget_id("format_graph.heatmap.cmap.Grayscale")?;
    driver.click_widget_id("modal.cancel")?;
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-heat after apply+cancel")?;
        assert!((gs.spec.x_axis.tick_angle_deg - 90.0).abs() < 1e-6);
        assert!(matches!(
            gs.spec.multivariate.as_ref().map(|m| &m.color),
            Some(ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::DivergingBlueRed, .. }, .. })
        ));
    }
    Ok(())
}

#[test]
fn fa_sec18_04_simulate_xy_standalone_validation_and_edit_params() -> TestRes {
    let (doc, xy_id, _) = build_transform_doc()?;
    let mut driver = make_driver(doc);

    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: xy_id, kind: TransformModalKind::Simulate });
    commit_intents(&mut driver);
    assert_modal_parity(&driver, "Simulate XY modal")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.simulate.equation.linear").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.extract.binding_x").is_none());

    driver.click_widget_id("edit.transform.simulate.equation.linear")?;
    set_textbox(&mut driver, "edit.transform.simulate.x_min", "50")?;
    set_textbox(&mut driver, "edit.transform.simulate.x_max", "10")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled));

    set_textbox(&mut driver, "edit.transform.simulate.x_min", "0")?;
    set_textbox(&mut driver, "edit.transform.simulate.x_max", "20")?;
    set_textbox(&mut driver, "edit.transform.simulate.row_count", "21")?;
    set_textbox(&mut driver, "edit.transform.simulate.replicates", "4")?;
    set_textbox(&mut driver, "edit.transform.simulate.seed", "777")?;
    set_textbox(&mut driver, "edit.transform.simulate.params", "slope=2.5, intercept=10")?;
    driver.click_widget_id("edit.transform.simulate.noise.gaussian")?;
    set_textbox(&mut driver, "edit.transform.simulate.noise_param", "0.25")?;
    assert_eq!(driver.state().views.transform_modals.validation_error, None);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let sim_rs_id = {
        let doc = driver.state().app.document();
        let rs = doc.result_sheets.last().ok_or("missing Simulate result sheet")?;
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(
            rs.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
                seed: 777,
                row_count: 21,
                replicates: 4,
                x_min: 0.0,
                x_max: 20.0,
                equation_id: String::from("linear"),
                parameters: vec![(String::from("slope"), 2.5), (String::from("intercept"), 10.0)],
                noise: SimulationNoise::Gaussian { sd: 0.25 },
            })
        );
        let data = rs.analyzable_data.as_ref().ok_or("missing simulated analyzable_data")?;
        assert_eq!(data.archetype.row_count(), 21);
        let TableArchetype::Xy(xy_out) = &data.archetype else {
            return Err(String::from("simulated table must be XY"));
        };
        assert_eq!(xy_out.data_sets[0].subcolumn_count(), 4);
        rs.id.clone()
    };

    // Re-open in EditAnalysisParameters and modify seed/row_count/noise.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &sim_rs_id));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
        driver.run_settled();
    }
    assert_eq!(driver.state().views.transform_modals.sim_seed_draft, "777");
    assert_eq!(driver.state().views.transform_modals.sim_row_count_draft, "21");
    assert_eq!(driver.state().views.transform_modals.sim_noise_kind, 1);
    set_textbox(&mut driver, "edit.transform.simulate.seed", "888")?;
    set_textbox(&mut driver, "edit.transform.simulate.row_count", "11")?;
    driver.click_widget_id("edit.transform.simulate.noise.poisson")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let rs_after = driver.state().app.document().result_sheet(&sim_rs_id).ok_or("missing updated simulate rs")?;
    assert!(matches!(
        rs_after.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::Simulate { seed: 888, row_count: 11, noise: SimulationNoise::Poisson, .. })
    ));
    assert_eq!(rs_after.analyzable_data.as_ref().map(|d| d.archetype.row_count()), Some(11));
    Ok(())
}

#[test]
fn fa_sec18_04_random_sample_and_extract_bindings_round_trip() -> TestRes {
    let (doc, _, mv_id) = build_transform_doc()?;
    let mut driver = make_driver(doc);

    // 1. Standalone RandomSample with stratify_column=3 ("Cohort") & proportional=false.
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: mv_id.clone(), kind: TransformModalKind::RandomSample });
    commit_intents(&mut driver);
    assert_modal_parity(&driver, "RandomSample modal")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.random_sample.sample_size").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.extract.binding_x").is_none());

    set_textbox(&mut driver, "edit.transform.random_sample.sample_size", "6")?;
    set_textbox(&mut driver, "edit.transform.random_sample.seed", "99")?;
    driver.click_widget_id("edit.transform.random_sample.stratify.3")?;
    driver.click_widget_id("edit.transform.random_sample.proportional")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let rand_rs_id = {
        let doc = driver.state().app.document();
        let rs = doc.result_sheets.last().ok_or("missing RandomSample result")?;
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(
            rs.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                seed: 99,
                sample_size: 6,
                stratify_column: Some(3),
                proportional_allocation: false,
            })
        );
        assert_eq!(rs.analyzable_data.as_ref().map(|d| d.archetype.row_count()), Some(6));
        rs.id.clone()
    };

    // Edit RandomSample parameters via EditAnalysisParameters.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &rand_rs_id));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
        driver.run_settled();
    }
    assert_eq!(driver.state().views.transform_modals.rand_sample_size_draft, "6");
    assert_eq!(driver.state().views.transform_modals.rand_seed_draft, "99");
    assert_eq!(driver.state().views.transform_modals.rand_stratify_col, Some(3));
    assert!(!driver.state().views.transform_modals.rand_proportional_allocation);
    set_textbox(&mut driver, "edit.transform.random_sample.sample_size", "4")?;
    set_textbox(&mut driver, "edit.transform.random_sample.seed", "123")?;
    driver.click_widget_id("edit.transform.random_sample.proportional")?;
    driver.click_widget_id("modal.ok")?;
    let rs_rand_after = driver.state().app.document().result_sheet(&rand_rs_id).ok_or("missing updated rand rs")?;
    assert_eq!(
        rs_rand_after.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 123,
            sample_size: 4,
            stratify_column: Some(3),
            proportional_allocation: true,
        })
    );
    assert_eq!(rs_rand_after.analyzable_data.as_ref().map(|d| d.archetype.row_count()), Some(4));

    // 2. ExtractAndRearrange column bindings ("Dose" -> X, "Response" -> Y into XyTable) + invalid binding guard.
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: mv_id, kind: TransformModalKind::ExtractAndRearrange });
    commit_intents(&mut driver);
    driver.click_widget_id("edit.transform.target_archetype.xy")?;
    driver.click_widget_id("edit.tab.data_arrangement")?;
    set_textbox(&mut driver, "edit.transform.extract.binding_x", "NoSuchCol")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled));

    set_textbox(&mut driver, "edit.transform.extract.binding_x", "Dose")?;
    set_textbox(&mut driver, "edit.transform.extract.binding_y", "Response")?;
    assert_eq!(driver.state().views.transform_modals.validation_error, None);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let ext_rs = driver.state().app.document().result_sheets.last().ok_or("missing ExtractAndRearrange result")?;
    assert_eq!(ext_rs.analysis.selected_data_sets, vec![1, 2]);
    let data = ext_rs.analyzable_data.as_ref().ok_or("missing extracted table")?;
    let TableArchetype::Xy(xy_out) = &data.archetype else {
        return Err(String::from("expected extracted XyTable"));
    };
    assert_eq!(xy_out.x_title, "Dose");
    assert_eq!(xy_out.data_sets.len(), 1);
    assert_eq!(xy_out.data_sets[0].title, "Response");
    Ok(())
}
