//! Wave 1 (`SEC-18`) regression suite (`FA-SEC18-02`, `FA-SEC18-03`, `FA-SEC18-04`).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DiagnosticPlotKind, GraphKind,
    GraphSheet, GroupedTable, HeatmapColumnLabelAngle, InfoSheet, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode,
    ProjectDocument, ResultStatus, SheetId, SimulationNoise, SizeChannelMapping, SubcolumnMode, TableArchetype, TransformAnalysisSpec, VariableRole,
    XColumnMode, XyTable,
};
use curvebook_palette::{ColorRamp, ColormapId, PaletteId};
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::egui_shell::import_wizard::{ImportWizardStep, WizardBlockReshape};
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::grid_view::{ImportOriginKind, ImportWizardStage};
use serde_json::json;

use crate::common::{ShellTestState, TestRes, assert_clean_parity, commit_shell_intents, set_textbox, widget_selected};
use crate::helpers::{exec, has_widget, make_human_driver, open_edit_params_driver, open_format_graph_driver, widget_enabled};

fn push_and_commit(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, intent: UiIntent) {
    driver.state_mut().intents.push(intent);
    let _ = commit_shell_intents(driver);
}

fn open_import_wizard(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) {
    push_and_commit(
        driver,
        UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: Some(SheetId::new("ds-col")) },
    );
}

fn build_import_doc() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("SEC18 Import Test").map_err(|e| e.to_string())?;
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Project Info").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut ds = DataSheet::new(
        "ds-col",
        "Target Sheet",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-col"));
    Ok(doc)
}

fn build_heatmap_doc() -> Result<(ProjectDocument, SheetId), String> {
    let mut doc = ProjectDocument::new("SEC18 Heatmap Test").map_err(|e| e.to_string())?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 3, 3).map_err(|e| e.to_string())?;
    for (c, t) in ["Ctrl", "DrugA", "DrugB"].into_iter().enumerate() {
        grp.data_sets[c].title = t.into();
    }
    for r in 0..3 {
        grp.row_titles[r] = format!("R{}", r + 1);
        for c in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number((r * 3 + c + 1) as f64 * 4.0);
        }
    }
    let grp_ds = DataSheet::new("ds-grp", "Matrix", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;
    let mut gs = GraphSheet::new_uninitialized_for_data("g-heat", &grp_ds).map_err(|e| e.to_string())?;
    gs.title = "Heatmap".into();
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
    xy.x_title = "Conc".into();
    xy.data_sets[0].title = "Ctrl".into();
    xy.data_sets[1].title = "Treated".into();
    for r in 0..3 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(25.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "XY Sheet", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(4, 10).map_err(|e| e.to_string())?;
    for (i, name) in ["ID", "Dose", "Response", "Cohort"].into_iter().enumerate() {
        mv.variables[i].title = name.into();
        mv.variables[i].role = if i == 3 { VariableRole::Categorical } else { VariableRole::Continuous };
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
    let mut driver = make_human_driver(build_import_doc()?);
    open_import_wizard(&mut driver);
    assert_eq!(
        (driver.state().views.import_wizard.active_step, driver.state().app.grid.import_wizard.stage),
        (ImportWizardStep::Source, ImportWizardStage::Source)
    );

    for (btn, step, stage) in [
        ("modal.next", ImportWizardStep::Preview, ImportWizardStage::Source),
        ("modal.next", ImportWizardStep::FilterAndUnstack, ImportWizardStage::Filter),
        ("modal.next", ImportWizardStep::PlacementAndReshape, ImportWizardStage::Placement),
        ("modal.next", ImportWizardStep::Metadata, ImportWizardStage::Provenance),
        ("modal.back", ImportWizardStep::PlacementAndReshape, ImportWizardStage::Placement),
        ("modal.back", ImportWizardStep::FilterAndUnstack, ImportWizardStage::Filter),
    ] {
        driver.click_widget_id(btn)?;
        assert_eq!((driver.state().views.import_wizard.active_step, driver.state().app.grid.import_wizard.stage), (step, stage));
    }

    for wid in ["import_paste.tab.source", "import_paste.delim.comma", "import_paste.encoding.utf16"] {
        driver.click_widget_id(wid)?;
    }
    set_textbox(&mut driver, "import_paste.input.missing_tokens", "MISS")?;
    driver.click_widget_id("import_paste.input.csv")?;
    driver.state_mut().views.import_wizard.raw_source_draft =
        String::from("\0# Assay: Kinase\n# Temp = 37.5\nKEEP_10,MISS,2.5*\nDROP_99,11,12\nKEEP_20,13,MISS\nKEEP_30,14,15");
    driver.run_settled();

    driver.click_widget_id("import_paste.tab.filter")?;
    set_textbox(&mut driver, "import_paste.input.filter_pattern", "KEEP_")?;
    driver.click_widget_id("import_paste.tab.placement")?;
    driver.click_widget_id("import_paste.reshape.row_wrap")?;
    set_textbox(&mut driver, "import_paste.input.plate_values_per_row", "3")?;
    set_textbox(&mut driver, "import_paste.input.plate_rows_per_dataset", "2")?;
    driver.click_widget_id("import_paste.tab.notes")?;
    set_textbox(&mut driver, "import_paste.input.notes", "Batch QC verified")?;
    driver.click_widget_id("import_paste.opt.extract_info_metadata")?;
    driver.click_widget_id("import_paste.opt.add_floating_note")?;
    assert_clean_parity(&driver, "ImportWizard Stage 5")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);

    let doc = driver.state().app.document();
    let ds = doc.data_sheet(&SheetId::new("ds-col")).ok_or("ds-col")?;
    let TableArchetype::Column(col_tbl) = &ds.archetype else {
        return Err("expected Column".into());
    };
    assert_eq!(col_tbl.data_sets.len(), 6);
    assert_eq!(ds.archetype.data_cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("KEEP_10")));
    assert_eq!(ds.archetype.data_cell(CellCoord::new(0, 1, 0)), Some(&Cell::empty()));
    assert!(ds.archetype.data_cell(CellCoord::new(0, 2, 0)).is_some_and(|c| c.excluded));
    assert_eq!(ds.archetype.data_cell(CellCoord::new(0, 3, 0)), Some(&Cell::text("KEEP_30")));
    assert_eq!(ds.archetype.data_cell(CellCoord::new(1, 0, 0)), Some(&Cell::text("KEEP_20")));

    let info = doc.info_sheet(&SheetId::new("info-1")).ok_or("info-1")?;
    assert_eq!(info.constants.iter().find(|c| c.name == "Assay").map(|c| &c.value), Some(&ConstantValue::Text("Kinase".into())));
    assert_eq!(info.constants.iter().find(|c| c.name == "Temp").map(|c| &c.value), Some(&ConstantValue::Number(37.5)));
    assert!(info.notes_markdown.contains("Batch QC verified") && info.notes_markdown.contains("[Import Summary]"));
    Ok(())
}

#[test]
fn fa_sec18_02_save_preset_persists_across_reopen_and_reset_defaults_clears() -> TestRes {
    let mut driver = make_human_driver(build_import_doc()?);
    open_import_wizard(&mut driver);
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
    assert!(driver.state().views.import_wizard.saved_preset.is_some() && driver.state().views.import_wizard.preset_saved_feedback);

    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.grid.import_wizard.open);
    open_import_wizard(&mut driver);
    let iw = &driver.state().views.import_wizard;
    assert_eq!(
        (
            driver.state().app.grid.import_wizard.delimiter,
            iw.encoding_name.as_str(),
            iw.missing_tokens_draft.as_str(),
            iw.filter_predicate_draft.as_str(),
            iw.block_reshape,
            iw.extract_info_metadata
        ),
        (',', "Latin-1", "NULL, -", "PASS", WizardBlockReshape::Plate96, true)
    );

    driver.click_widget_id("import_paste.tab.notes")?;
    driver.click_widget_id("import_paste.btn.reset_defaults")?;
    let iw = &driver.state().views.import_wizard;
    assert!(iw.saved_preset.is_none());
    assert_eq!(
        (
            driver.state().app.grid.import_wizard.delimiter,
            iw.encoding_name.as_str(),
            iw.missing_tokens_draft.as_str(),
            iw.filter_predicate_draft.as_str(),
            iw.block_reshape,
            iw.extract_info_metadata
        ),
        ('\t', "UTF-8", "NA, NaN, .", "", WizardBlockReshape::None, false)
    );
    Ok(())
}

#[test]
fn fa_sec18_03_heatmap_modal_loads_existing_preserves_unmodified_and_cancels_cleanly() -> TestRes {
    let (doc, gid) = build_heatmap_doc()?;
    let mut driver = make_human_driver(doc);

    open_format_graph_driver(&mut driver, &gid)?;
    let gm = &driver.state().views.graph_modals;
    assert_eq!(
        (gm.heatmap_colormap, gm.heatmap_reversed, gm.heatmap_auto_range, gm.heatmap_min_draft.as_str(), gm.heatmap_max_draft.as_str()),
        (ColormapId::Plasma, true, false, "2", "42")
    );
    assert!((gm.heatmap_tick_angle_deg - 45.0).abs() < 1e-6);
    assert_eq!(widget_selected(&driver, "format_graph.heatmap.cmap.Plasma"), Some(true));

    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    assert_eq!(widget_selected(&driver, "format_graph.heatmap.angle.45"), Some(true));
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let gs = driver.state().app.document().graph_sheet(&gid).ok_or("g-heat")?;
    assert!(!gs.spec.heatmap_labels.annotate_cells && (gs.spec.x_axis.tick_angle_deg - 45.0).abs() < 1e-6);
    assert!(
        matches!(gs.spec.multivariate.as_ref().ok_or("mv")?.color, ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: true }, domain_clamp: Some((lo, hi)), .. } if (lo - 2.0).abs() < 1e-9 && (hi - 42.0).abs() < 1e-9)
    );

    let mut custom_mv = driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.clone()).ok_or("mv")?;
    custom_mv.color = ColorChannelMapping::Categorical { column: 0, palette: PaletteId::default(), level_order_override: None };
    exec(&mut driver.state_mut().app, "graph.setSpec", json!({ "sheet_id": gid.as_str(), "multivariate": custom_mv }))?;
    exec(&mut driver.state_mut().app, "graph.setAxes", json!({ "sheet_id": gid.as_str(), "axis": "x", "tick_angle_deg": 30.0 }))?;
    open_format_graph_driver(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    driver.click_widget_id("modal.ok")?;
    let gs2 = driver.state().app.document().graph_sheet(&gid).ok_or("g-heat")?;
    assert!(
        (gs2.spec.x_axis.tick_angle_deg - 30.0).abs() < 1e-6
            && matches!(gs2.spec.multivariate.as_ref().map(|m| &m.color), Some(ColorChannelMapping::Categorical { .. }))
    );

    open_format_graph_driver(&mut driver, &gid)?;
    for wid in [
        "edit.tab.format_graph.color_mapping",
        "format_graph.heatmap.cmap.DivergingBlueRed",
        "edit.tab.format_graph.heatmap_labels",
        "format_graph.heatmap.angle.90",
        "modal.apply",
        "edit.tab.format_graph.color_mapping",
        "format_graph.heatmap.cmap.Grayscale",
        "modal.cancel",
    ] {
        driver.click_widget_id(wid)?;
    }
    let gs3 = driver.state().app.document().graph_sheet(&gid).ok_or("g-heat")?;
    assert!(
        (gs3.spec.x_axis.tick_angle_deg - 90.0).abs() < 1e-6
            && matches!(
                gs3.spec.multivariate.as_ref().map(|m| &m.color),
                Some(ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::DivergingBlueRed, .. }, .. })
            )
    );
    Ok(())
}

#[test]
fn fa_sec18_04_simulate_xy_standalone_validation_and_edit_params() -> TestRes {
    let (doc, xy_id, _) = build_transform_doc()?;
    let mut driver = make_human_driver(doc);
    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: xy_id, kind: TransformModalKind::Simulate });
    assert_clean_parity(&driver, "Simulate XY modal")?;
    assert!(has_widget(&driver, "edit.transform.simulate.equation.linear") && !has_widget(&driver, "edit.transform.extract.binding_x"));

    driver.click_widget_id("edit.transform.simulate.equation.linear")?;
    set_textbox(&mut driver, "edit.transform.simulate.x_min", "50")?;
    set_textbox(&mut driver, "edit.transform.simulate.x_max", "10")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some() && !widget_enabled(&driver, "modal.ok")?);

    for (wid, val) in [
        ("edit.transform.simulate.x_min", "0"),
        ("edit.transform.simulate.x_max", "20"),
        ("edit.transform.simulate.row_count", "21"),
        ("edit.transform.simulate.replicates", "4"),
        ("edit.transform.simulate.seed", "777"),
        ("edit.transform.simulate.params", "slope=2.5, intercept=10"),
    ] {
        set_textbox(&mut driver, wid, val)?;
    }
    driver.click_widget_id("edit.transform.simulate.noise.gaussian")?;
    set_textbox(&mut driver, "edit.transform.simulate.noise_param", "0.25")?;
    assert_eq!(driver.state().views.transform_modals.validation_error, None);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let sim_rs_id = {
        let rs = driver.state().app.document().result_sheets.last().ok_or("missing Simulate rs")?;
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(
            rs.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
                seed: 777,
                row_count: 21,
                replicates: 4,
                x_min: 0.0,
                x_max: 20.0,
                equation_id: "linear".into(),
                parameters: vec![("slope".into(), 2.5), ("intercept".into(), 10.0)],
                noise: SimulationNoise::Gaussian { sd: 0.25 },
                simulation_mode: None,
            })
        );
        let data = rs.analyzable_data.as_ref().ok_or("missing analyzable_data")?;
        let TableArchetype::Xy(xy_out) = &data.archetype else {
            return Err("expected XY".into());
        };
        assert_eq!((data.archetype.row_count(), xy_out.data_sets[0].subcolumn_count()), (21, 4));
        rs.id.clone()
    };

    open_edit_params_driver(&mut driver, &sim_rs_id)?;
    let tm = &driver.state().views.transform_modals;
    assert_eq!((tm.sim_seed_draft.as_str(), tm.sim_row_count_draft.as_str(), tm.sim_noise_kind), ("777", "21", 1));
    set_textbox(&mut driver, "edit.transform.simulate.seed", "888")?;
    set_textbox(&mut driver, "edit.transform.simulate.row_count", "11")?;
    driver.click_widget_id("edit.transform.simulate.noise.poisson")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let rs_after = driver.state().app.document().result_sheet(&sim_rs_id).ok_or("missing updated rs")?;
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
    let mut driver = make_human_driver(doc);

    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: mv_id.clone(), kind: TransformModalKind::RandomSample });
    assert_clean_parity(&driver, "RandomSample modal")?;
    assert!(has_widget(&driver, "edit.transform.random_sample.sample_size") && !has_widget(&driver, "edit.transform.extract.binding_x"));
    set_textbox(&mut driver, "edit.transform.random_sample.sample_size", "6")?;
    set_textbox(&mut driver, "edit.transform.random_sample.seed", "99")?;
    driver.click_widget_id("edit.transform.random_sample.stratify.3")?;
    driver.click_widget_id("edit.transform.random_sample.proportional")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let rand_rs_id = {
        let rs = driver.state().app.document().result_sheets.last().ok_or("missing RandomSample rs")?;
        assert_eq!(
            (rs.status.clone(), &rs.analysis.kind, rs.analyzable_data.as_ref().map(|d| d.archetype.row_count())),
            (
                ResultStatus::UpToDate,
                &AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                    seed: 99,
                    sample_size: 6,
                    stratify_column: Some(3),
                    proportional_allocation: false
                }),
                Some(6)
            )
        );
        rs.id.clone()
    };

    open_edit_params_driver(&mut driver, &rand_rs_id)?;
    let tm = &driver.state().views.transform_modals;
    assert_eq!(
        (tm.rand_sample_size_draft.as_str(), tm.rand_seed_draft.as_str(), tm.rand_stratify_col, tm.rand_proportional_allocation),
        ("6", "99", Some(3), false)
    );
    set_textbox(&mut driver, "edit.transform.random_sample.sample_size", "4")?;
    set_textbox(&mut driver, "edit.transform.random_sample.seed", "123")?;
    driver.click_widget_id("edit.transform.random_sample.proportional")?;
    driver.click_widget_id("modal.ok")?;
    let rs_after = driver.state().app.document().result_sheet(&rand_rs_id).ok_or("updated rand rs")?;
    assert_eq!(
        (&rs_after.analysis.kind, rs_after.analyzable_data.as_ref().map(|d| d.archetype.row_count())),
        (
            &AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                seed: 123,
                sample_size: 4,
                stratify_column: Some(3),
                proportional_allocation: true
            }),
            Some(4)
        )
    );

    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: mv_id, kind: TransformModalKind::ExtractAndRearrange });
    driver.click_widget_id("edit.transform.target_archetype.xy")?;
    driver.click_widget_id("edit.tab.data_arrangement")?;
    set_textbox(&mut driver, "edit.transform.extract.binding_x", "NoSuchCol")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some() && !widget_enabled(&driver, "modal.ok")?);

    set_textbox(&mut driver, "edit.transform.extract.binding_x", "Dose")?;
    set_textbox(&mut driver, "edit.transform.extract.binding_y", "Response")?;
    assert_eq!(driver.state().views.transform_modals.validation_error, None);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let ext_rs = driver.state().app.document().result_sheets.last().ok_or("missing Extract rs")?;
    assert_eq!(ext_rs.analysis.selected_data_sets, vec![1, 2]);
    let TableArchetype::Xy(xy_out) = &ext_rs.analyzable_data.as_ref().ok_or("extracted table")?.archetype else {
        return Err("expected XY".into());
    };
    assert_eq!((xy_out.x_title.as_str(), xy_out.data_sets.len(), xy_out.data_sets[0].title.as_str()), ("Dose", 1, "Response"));
    Ok(())
}
