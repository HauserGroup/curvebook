//! Targeted regression suite for SEC-02 audit findings `FA-SEC02-01`..`FA-SEC02-14`.

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::param_spellings::canonical_command_param_alias;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, Cell, CellCoord, CellExclusionNote, CellValue, CloneDataSheetOptions, ColumnDesignHint, ColumnTable,
    ConstantValue, DataSheet, DocError, FloatingNote, GraphKind, GraphSheet, GraphSpec, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InfoConstant, InfoSheet, JournalWidthPreset, LayoutSheet, LayoutSpec, LevelOrderKind, LevelOrderSpec, LinkSource,
    LinkedDataRegion, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_CELLS, MAX_PROJECT_IMAGE_ASSETS, MAX_SHEET_CELLS, MAX_SHEETS_PER_PROJECT, MAX_TITLE_BYTES,
    MultipleVariablesTable, NestedTable, PlacementMode, PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerTailMode, PowerTestFamily,
    ProjectDocument, ProjectImageAsset, RectPt, ResultScalarEntry, ResultSheet, ResultStatus, SequenceProgression, SheetCellRange, SheetId,
    SummaryPointError, TableArchetype, VariableRole, XColumnMode, XyTable, all_analysis_descriptors, project_document_schema,
};

type TestRes = Result<(), Box<dyn std::error::Error>>;

fn make_col_sheet(id: &str, cols: usize, rows: usize) -> Result<DataSheet, DocError> {
    let tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows)?;
    DataSheet::new(id, id, TableArchetype::Column(tbl))
}

#[test]
fn fa_sec02_01_analysis_shorthand_alias_covers_all_families() -> TestRes {
    for glm_kind in [
        "glm",
        "generalized_linear_model",
        "linear_glm",
        "multiple_linear",
        "logistic_glm",
        "multiple_logistic",
        "poisson_glm",
        "multiple_poisson",
        "multiple_regression",
        "multiple_linear_regression",
        "logistic_regression",
        "poisson_regression",
    ] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(glm_kind), "response_column"), None, "kind={glm_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(glm_kind), "target_column"), Some("response_column"), "kind={glm_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(glm_kind), "predictors"), Some("predictor_columns"), "kind={glm_kind}");
    }

    for mv_kind in [
        "multivariate",
        "pca",
        "principal_components",
        "principal_component_analysis",
        "correlation",
        "correlation_matrix",
        "clustering",
        "kmeans",
        "hierarchical_clustering",
        "multiple_imputation",
        "mice",
    ] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(mv_kind), "response_column"), None, "kind={mv_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(mv_kind), "target_column"), Some("response_column"), "kind={mv_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(mv_kind), "predictors"), Some("predictor_columns"), "kind={mv_kind}");
    }

    for pstack_kind in ["pvalue_stack", "p_value_stack", "pvaluestack", "pvalues", "stats.pvalue_stack", "stats.pvaluestack"] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(pstack_kind), "multiplicity"), Some("method"), "kind={pstack_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(pstack_kind), "alpha"), Some("threshold"), "kind={pstack_kind}");
    }

    for cox_kind in ["cox", "cox_ph", "cox_proportional_hazards", "coxph"] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(cox_kind), "time_column"), None, "kind={cox_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(cox_kind), "status_column"), None, "kind={cox_kind}");
    }

    for km_kind in ["kaplan_meier", "survival", "log_rank", "logrank", "km"] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(km_kind), "time_column"), Some("mv_time_column"), "kind={km_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(km_kind), "status_column"), Some("mv_status_column"), "kind={km_kind}");
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(km_kind), "group_column"), Some("mv_group_column"), "kind={km_kind}");
    }

    for outlier_kind in ["outlier", "outliers", "outlier_detection", "identify_outliers", "rout", "grubbs"] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(outlier_kind), "fdr_q_percent"), Some("q_percent"), "kind={outlier_kind}");
    }

    for sim_kind in ["simulate", "simulate_xy", "transform.simulate", "simulate_data"] {
        assert_eq!(canonical_command_param_alias("analysis.run", &[], Some(sim_kind), "params"), Some("parameters"), "kind={sim_kind}");
    }

    for desc in all_analysis_descriptors() {
        for &sh in desc.shorthands {
            let mapped = canonical_command_param_alias("analysis.run", &[], Some(sh), "threshold");
            if matches!(mapped, None | Some("alpha" | "threshold")) {
                continue;
            }
            return Err(format!("unexpected threshold alias `{mapped:?}` for shorthand `{sh}`").into());
        }
    }
    Ok(())
}

#[test]
fn fa_sec02_02_datasheet_shift_rows_down_updates_exclusions_and_links() -> TestRes {
    let mut ds = make_col_sheet("data-1", 2, 5)?;
    ds.exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(0, 0, 0),
        reason_tag: String::from("note0"),
        note: String::from("stay"),
        timestamp_utc: String::from("2026-11-22T00:00:00Z"),
    });
    ds.exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(2, 1, 0),
        reason_tag: String::from("note2"),
        note: String::from("shift"),
        timestamp_utc: String::from("2026-11-22T00:00:00Z"),
    });
    ds.add_linked_region(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::Clipboard,
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec { anchor: CellCoord::new(2, 0, 0), mode: PlacementMode::Overwrite, ..ImportPlacementSpec::default() },
        },
        target_range: SheetCellRange::new(CellCoord::new(2, 0, 0), CellCoord::new(4, 1, 0)),
        provenance_summary: String::from("Clipboard"),
    })?;

    ds.shift_rows_down(1, 3, 5);
    assert_eq!(ds.exclusion_notes[0].coord.row, 0);
    assert_eq!(ds.exclusion_notes[1].coord.row, 5);
    assert_eq!(ds.linked_regions[0].target_range.start.row, 5);
    assert_eq!(ds.linked_regions[0].target_range.end.row, 7);
    assert_eq!(ds.linked_regions[0].spec.placement.anchor.row, 5);

    // start_row >= existing_rows and frozen == true must leave metadata untouched.
    ds.shift_rows_down(5, 2, 5);
    assert_eq!(ds.exclusion_notes[1].coord.row, 5);
    assert_eq!(ds.linked_regions[0].target_range.start.row, 5);

    ds.frozen = true;
    ds.shift_rows_down(0, 2, 10);
    assert_eq!(ds.exclusion_notes[0].coord.row, 0);
    assert_eq!(ds.exclusion_notes[1].coord.row, 5);
    assert_eq!(ds.linked_regions[0].target_range.start.row, 5);
    Ok(())
}

#[test]
fn fa_sec02_03_upstream_dependencies_preserves_self_cycle() -> TestRes {
    let mut doc = ProjectDocument::new("CycleTest")?;
    let ds = make_col_sheet("data-1", 1, 3)?;
    doc.add_data_sheet(ds)?;

    let mut spec = AnalysisSpec::new("self-res", "self-res", AnalysisKind::DescriptiveStats(Default::default()))?;
    spec.kind = AnalysisKind::Transform(curvebook_doc::TransformAnalysisSpec::Normalize {
        zero_anchor: curvebook_doc::NormalizeAnchor::FixedValue(0.0),
        hundred_anchor: curvebook_doc::NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("self-res"),
            parameter_key: String::from("Mean"),
            data_set_index: None,
        }),
        output_percent: true,
        per_data_set: true,
    });
    doc.result_sheets.push(ResultSheet::new(spec)?);

    let deps = doc.upstream_dependencies(&SheetId::new("self-res"));
    assert!(deps.contains(&SheetId::new("self-res")), "self-dependency must remain visible: {deps:?}");
    Ok(())
}

#[test]
fn fa_sec02_04_project_and_image_asset_deserialize_enforce_budgets() -> TestRes {
    // Over MAX_SHEETS_PER_PROJECT rejected on ProjectDocument::Deserialize.
    let doc = ProjectDocument::new("SheetCap")?;
    let mut doc_json = serde_json::to_value(&doc)?;
    let mut info_arr = Vec::with_capacity(MAX_SHEETS_PER_PROJECT + 1);
    for idx in 0..=MAX_SHEETS_PER_PROJECT {
        let sheet = InfoSheet::new_preseeded(format!("info-{idx}"), "Info")?;
        info_arr.push(serde_json::to_value(&sheet)?);
    }
    doc_json["info_sheets"] = serde_json::Value::Array(info_arr);
    assert!(serde_json::from_value::<ProjectDocument>(doc_json).is_err());

    // Cumulative project cell budget (> MAX_PROJECT_CELLS) enforced in add_data_sheet & add_result_sheet.
    let s1 = make_col_sheet("big-1", 250, 105_000)?;
    let s2 = make_col_sheet("big-2", 250, 105_000)?;
    let s3 = make_col_sheet("big-3", 250, 105_000)?;
    let mut cell_doc = ProjectDocument::new("CellsCap")?;
    cell_doc.add_data_sheet(s1)?;
    assert!(matches!(cell_doc.add_data_sheet(s2), Err(DocError::TooManyCells { max, .. }) if max == MAX_PROJECT_CELLS));

    let rspec = AnalysisSpec::new(
        "res-big",
        "big-1",
        AnalysisKind::Transform(curvebook_doc::TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }),
    )?;
    let mut rsheet = ResultSheet::new(rspec)?;
    rsheet.analyzable_data = Some(s3);
    assert!(matches!(cell_doc.add_result_sheet(rsheet), Err(DocError::TooManyCells { max, .. }) if max == MAX_PROJECT_CELLS));
    drop(cell_doc);

    // Over MAX_PROJECT_IMAGE_ASSETS rejected on ProjectDocument::Deserialize.
    let mut img_doc_json = serde_json::to_value(ProjectDocument::new("ImgCap")?)?;
    let valid_asset = serde_json::json!({
        "asset_id": "img-0123456789abcdef",
        "media_type": "Png",
        "pixel_size": [4, 4]
    });
    img_doc_json["image_assets"] = serde_json::Value::Array(vec![valid_asset; MAX_PROJECT_IMAGE_ASSETS + 1]);
    assert!(serde_json::from_value::<ProjectDocument>(img_doc_json).is_err());

    // Corrupt ProjectImageAsset rejected on Deserialize (including MAX_IMAGE_DIM_PX & MAX_IMAGE_PIXELS).
    for bad_asset in [
        serde_json::json!({ "asset_id": "not-valid-id", "media_type": "Png", "pixel_size": [4, 4] }),
        serde_json::json!({ "asset_id": "img-0123456789abcdef", "media_type": "Png", "pixel_size": [0, 4] }),
        serde_json::json!({ "asset_id": "img-0123456789abcdef", "media_type": "Png", "pixel_size": [16_385, 1] }),
        serde_json::json!({ "asset_id": "img-0123456789abcdef", "media_type": "Png", "pixel_size": [8_193, 8_193] }),
    ] {
        assert!(serde_json::from_value::<ProjectImageAsset>(bad_asset).is_err());
    }

    // Oversized base64 string rejected in visitor before decoding > MAX_IMAGE_ASSET_BYTES.
    let over_encoded = "AAAA".repeat((MAX_IMAGE_ASSET_BYTES.div_ceil(3)) + 2);
    let over_json = serde_json::json!({
        "asset_id": "img-0123456789abcdef",
        "media_type": "Png",
        "pixel_size": [4, 4],
        "bytes": over_encoded
    });
    assert!(serde_json::from_value::<ProjectImageAsset>(over_json).is_err());
    Ok(())
}

#[test]
fn fa_sec02_05_recode_and_ordered_levels_trim_inputs() -> TestRes {
    let mut tbl = MultipleVariablesTable::new(1, 3)?;
    tbl.variables[0].role = VariableRole::Categorical;
    tbl.variables[0].cells[0] = Cell::text("  Alpha ");
    tbl.variables[0].cells[1] = Cell::text("Beta");
    tbl.variables[0].cells[2] = Cell::text("Gamma");

    let changed = tbl.variables[0].recode_levels(&[(String::from("  Alpha  "), String::from("  Treated  "))]);
    assert_eq!(changed, 1);
    assert_eq!(tbl.variables[0].cells[0].value, CellValue::Text(String::from("Treated")));

    let order = LevelOrderSpec {
        order: LevelOrderKind::Explicit(vec![String::from(" Gamma "), String::from("  Treated "), String::from("Beta")]),
        reversed: false,
    };
    let ordered: Vec<String> = tbl.variables[0].ordered_levels_with(&order).into_iter().map(|s| s.level).collect();
    assert_eq!(ordered, vec!["Gamma", "Treated", "Beta"]);
    Ok(())
}

#[test]
fn fa_sec02_06_power_analysis_and_analysis_spec_validate_domain() -> TestRes {
    // Draft PowerAnalysisSpec::default() stays serde-roundtrippable.
    let def_json = serde_json::to_value(PowerAnalysisSpec::default())?;
    assert_eq!(serde_json::from_value::<PowerAnalysisSpec>(def_json)?, PowerAnalysisSpec::default());

    let invalid_alpha = serde_json::json!({
        "family": "TwoSampleUnpairedT",
        "solve_target": "SampleSize",
        "tail_mode": "TwoSided",
        "alpha": 1.5,
        "target_power": 0.8,
        "allocation_ratio": 1.0,
        "effect": { "Standardized": 0.5 },
        "chi_square_df": 1.0,
        "two_proportions_mode": "PooledNullUnpooledAlt",
        "continuity_correction": false,
        "correlation_method": "FisherZ"
    });
    assert!(serde_json::from_value::<PowerAnalysisSpec>(invalid_alpha.clone()).is_err());

    let bad_anova_tail = PowerAnalysisSpec {
        family: PowerTestFamily::OneWayAnova,
        tail_mode: PowerTailMode::OneSided(PowerOneSidedDirection::Greater),
        effect: PowerEffectSpec::Standardized(0.4),
        ..PowerAnalysisSpec::default()
    };
    let bad_anova_val = serde_json::to_value(&bad_anova_tail)?;
    assert!(serde_json::from_value::<PowerAnalysisSpec>(bad_anova_val).is_err());
    assert!(AnalysisSpec::new("p1", "d1", AnalysisKind::PowerAnalysis(bad_anova_tail.clone())).is_err());

    let mut spec = AnalysisSpec::new("p-ok", "d1", AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default()))?;
    assert!(spec.try_update_kind(AnalysisKind::PowerAnalysis(bad_anova_tail)).is_err());

    let mut analysis_json = serde_json::to_value(&spec)?;
    analysis_json["kind"] = serde_json::json!({ "PowerAnalysis": invalid_alpha });
    assert!(serde_json::from_value::<AnalysisSpec>(analysis_json).is_err());
    Ok(())
}

#[test]
fn fa_sec02_07_sheet_id_validation_and_project_validate() -> TestRes {
    assert!(SheetId::new("valid_sheet-1").validate().is_ok());
    for bad in ["", "   ", "bad:id", "bad id", "bad\tid", "bad{{hook}}", "bad}}hook", &"x".repeat(MAX_SHEET_ID_BYTES + 1)] {
        assert!(SheetId::new(bad).validate().is_err(), "expected invalid SheetId `{bad}`");
    }
    assert!(serde_json::from_value::<SheetId>(serde_json::json!("")).is_err());
    assert!(serde_json::from_value::<SheetId>(serde_json::json!("bad\0id")).is_err());
    assert!(serde_json::from_value::<SheetId>(serde_json::json!("x".repeat(MAX_TITLE_BYTES + 1))).is_err());

    let mut doc = ProjectDocument::new("IdValidation")?;
    for bad_insert in ["bad:id", "bad id", "bad{{1}}"] {
        let ds = DataSheet::new(bad_insert, "Title", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?))?;
        assert!(doc.add_data_sheet(ds).is_err(), "add_data_sheet must reject `{bad_insert}`");
    }

    let ds_long = DataSheet::new(
        "z".repeat(MAX_SHEET_ID_BYTES + 10),
        "Long",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?),
    )?;
    doc.data_sheets.push(ds_long);
    assert!(doc.validate().is_err(), "ProjectDocument::validate must reject >256 byte SheetId");

    let mut retarget = AnalysisSpec::new("res-rt", "data-1", AnalysisKind::DescriptiveStats(Default::default()))?;
    assert!(retarget.retarget_source("bad\0id", curvebook_doc::TableKind::Column, vec![]).is_err());
    assert!(retarget.retarget_source("x".repeat(MAX_TITLE_BYTES + 1), curvebook_doc::TableKind::Column, vec![]).is_err());
    Ok(())
}

#[test]
fn fa_sec02_08_lookup_scalar_requires_up_to_date_status() -> TestRes {
    let spec = AnalysisSpec::new("res-1", "data-1", AnalysisKind::LinearRegression(Default::default()))?;
    let mut res = ResultSheet::new(spec)?;
    res.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 2.5, units: None });
    assert_eq!(res.status, ResultStatus::Dirty);
    assert_eq!(res.lookup_scalar("Slope", Some(0)), None, "Dirty ResultSheet must return None");

    res.status = ResultStatus::UpToDate;
    assert_eq!(res.lookup_scalar("Slope", Some(0)), Some(2.5));

    res.status = ResultStatus::Failed { code: curvebook_doc::ErrorCode::SolverFailed, message: String::from("failed") };
    assert_eq!(res.lookup_scalar("Slope", Some(0)), None);
    Ok(())
}

#[test]
fn fa_sec02_09_nested_clone_preserves_custom_subcolumn_titles() -> TestRes {
    let mut tbl = NestedTable::new(2, 2, 3)?;
    tbl.groups[0].title = String::from("Control");
    tbl.groups[0].subcolumn_titles = vec![String::from("Rat-1"), String::from("Rat-2")];
    tbl.groups[1].title = String::from("Drug");
    tbl.groups[1].subcolumn_titles = vec![String::from("Rat-3"), String::from("Rat-4")];
    let ds = DataSheet::new("nest-1", "Nested", TableArchetype::Nested(tbl))?;

    let cloned_preserved = ds.clone_structure(
        "nest-2",
        &CloneDataSheetOptions {
            title: String::from("Cloned"),
            keep_x_values: false,
            clear_y_values: false,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: None,
        },
    )?;
    if let TableArchetype::Nested(nt) = &cloned_preserved.archetype {
        assert_eq!(nt.groups[0].subcolumn_titles, vec!["Rat-1", "Rat-2"]);
        assert_eq!(nt.groups[1].subcolumn_titles, vec!["Rat-3", "Rat-4"]);
    } else {
        return Err(String::from("expected Nested archetype").into());
    }

    let cloned_reset = ds.clone_structure(
        "nest-3",
        &CloneDataSheetOptions {
            title: String::from("Reset"),
            keep_x_values: false,
            clear_y_values: false,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: Some(3),
        },
    )?;
    if let TableArchetype::Nested(nt) = &cloned_reset.archetype {
        assert_eq!(nt.groups[0].subcolumn_titles, vec!["Control:1", "Control:2", "Control:3"]);
    } else {
        return Err(String::from("expected Nested archetype").into());
    }
    Ok(())
}

#[test]
fn fa_sec02_10_generated_sequence_validation_and_exclusion_plus_grouped_spacing() -> TestRes {
    let bad_mode = XColumnMode::GeneratedSequence { start: f64::NAN, step: 1.0, progression: SequenceProgression::Arithmetic };
    assert!(bad_mode.validate().is_err());

    let mut grp = GroupedTable::new(curvebook_doc::SubcolumnMode::RawReplicates(2), 2, 2)?;
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(f64::INFINITY)]);
    assert!(grp.validate_finite().is_err());

    let mut xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4)?;
    xy.x_mode = XColumnMode::GeneratedSequence { start: 10.0, step: 5.0, progression: SequenceProgression::Arithmetic };
    xy.x_cells = vec![Vec::new(); 4];
    let mut ds = DataSheet::new("xy-seq", "Seq", TableArchetype::Xy(xy))?;
    ds.set_x_or_time_excluded(2, 0, true)?;
    ds.validate()?;
    let x2 = ds.archetype.x_or_time_cell(2, 0).ok_or("expected materialized generated X cell")?;
    assert!(x2.excluded);
    assert_eq!(x2.value, CellValue::Number(20.0));
    Ok(())
}

#[test]
fn fa_sec02_11_deserialize_enforces_domain_bounds_on_specs_and_sheets() -> TestRes {
    assert!(InfoConstant::new("bad", ConstantValue::Number(f64::INFINITY), None).is_err());

    // LayoutSpec invalid geometry rejected on Deserialize.
    let mut bad_layout = serde_json::to_value(LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm))?;
    bad_layout["width"] = serde_json::json!(0.0);
    assert!(serde_json::from_value::<LayoutSpec>(bad_layout).is_err());

    // GraphSpec invalid bounds rejected on Deserialize.
    let ds = make_col_sheet("d-g", 1, 2)?;
    let mut gspec = GraphSpec::default_for_data_sheet(&ds);
    gspec.kind = GraphKind::Column;
    let base_gjson = serde_json::to_value(&gspec)?;

    let mut zero_canvas = base_gjson.clone();
    zero_canvas["canvas_size_pt"]["width"] = serde_json::json!(0.0);
    assert!(serde_json::from_value::<GraphSpec>(zero_canvas).is_err());

    let mut neg_axis = base_gjson.clone();
    neg_axis["x_axis"]["stroke_width_pt"] = serde_json::json!(-1.0);
    assert!(serde_json::from_value::<GraphSpec>(neg_axis).is_err());

    let mut empty_segs = base_gjson.clone();
    empty_segs["y_axis"]["segments"] = serde_json::json!([]);
    assert!(serde_json::from_value::<GraphSpec>(empty_segs).is_err());

    let mut zero_share = base_gjson.clone();
    zero_share["x_axis"]["segments"] = serde_json::json!([serde_json::to_value(AxisSegment { length_share: 0.0, ..AxisSegment::default() })?]);
    assert!(serde_json::from_value::<GraphSpec>(zero_share).is_err());

    let mut bad_gap = base_gjson.clone();
    bad_gap["heatmap_labels"]["cell_gap_pt"] = serde_json::json!(20.0);
    assert!(serde_json::from_value::<GraphSpec>(bad_gap).is_err());

    // Negative GDO stroke / border widths & invalid markers rejected on GraphSpec::Deserialize.
    let mut bad_bar = base_gjson.clone();
    bad_bar["series"][0]["gdo"] = serde_json::json!({
        "BarWithReplicates": {
            "summary": "Mean",
            "fill_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "border_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "border_width_pt": -1.0,
            "replicate_overlay": null
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(bad_bar).is_err());

    let mut bad_line = base_gjson.clone();
    bad_line["series"][0]["gdo"] = serde_json::json!({
        "ScatterAndLine": {
            "marker": null,
            "line": {
                "kind": "PointToPoint",
                "dash_style": "Solid",
                "stroke_width_pt": -0.5,
                "color": { "r": 0, "g": 0, "b": 0, "a": 255 }
            },
            "area_fill": null,
            "replicate_lines": "MeanTrajectory"
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(bad_line).is_err());

    let mut bad_fbar = base_gjson.clone();
    bad_fbar["series"][0]["gdo"] = serde_json::json!({
        "FloatingBar": {
            "summary": null,
            "fill_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "border_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "border_width_pt": -2.0
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(bad_fbar).is_err());

    let mut bad_ba = base_gjson.clone();
    bad_ba["series"][0]["gdo"] = serde_json::json!({
        "BeforeAfterLines": {
            "marker": null,
            "line_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "stroke_width_pt": -1.0
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(bad_ba).is_err());

    let mut bad_km = base_gjson;
    bad_km["series"][0]["gdo"] = serde_json::json!({
        "KaplanMeierStep": {
            "line_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "stroke_width_pt": -1.0,
            "show_censored_ticks": true,
            "show_greenwood_ci": false,
            "ci_dashed": false
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(bad_km).is_err());

    let mv = MultipleVariablesTable::new(2, 2)?;
    let mv_ds = DataSheet::new("mv-g", "MV", TableArchetype::MultipleVariables(mv))?;
    let mut mv_gspec = serde_json::to_value(GraphSpec::default_for_data_sheet(&mv_ds))?;
    mv_gspec["multivariate"]["plot_mode"] = serde_json::json!({
        "CategoricalSummary": {
            "gdo": {
                "BarWithReplicates": {
                    "summary": "Mean",
                    "fill_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "border_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "border_width_pt": -1.0,
                    "replicate_overlay": null
                }
            },
            "y_error_bars": null
        }
    });
    assert!(serde_json::from_value::<GraphSpec>(mv_gspec).is_err());

    // FloatingNote negative width rejected on InfoSheet/GraphSheet/LayoutSheet Deserialize.
    let mut gs = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    gs.floating_notes.push(FloatingNote::new("n-1", RectPt { x: 0.0, y: 0.0, width: -5.0, height: 10.0 }, "note"));
    assert!(serde_json::from_value::<GraphSheet>(serde_json::to_value(&gs)?).is_err());

    let mut ls = LayoutSheet::new("l-1", "Layout", LayoutSpec::default())?;
    ls.floating_notes.push(FloatingNote::new("n-1", RectPt { x: 0.0, y: 0.0, width: 10.0, height: -2.0 }, "note"));
    assert!(serde_json::from_value::<LayoutSheet>(serde_json::to_value(&ls)?).is_err());

    let mut isheet = InfoSheet::new_preseeded("i-1", "Info")?;
    isheet.floating_notes.push(FloatingNote::new("", RectPt::new(0.0, 0.0, 10.0, 10.0), "note"));
    assert!(serde_json::from_value::<InfoSheet>(serde_json::to_value(&isheet)?).is_err());
    Ok(())
}

#[test]
fn fa_sec02_12_schema_tightens_sheet_id_caps_and_eliminates_wildcards() -> TestRes {
    let val = serde_json::to_value(project_document_schema())?;
    assert_eq!(val["$defs"]["SheetId"]["maxLength"], MAX_SHEET_ID_BYTES);
    let pat = val["$defs"]["SheetId"]["pattern"].as_str().ok_or("missing SheetId pattern")?;
    assert!(pat.contains(r"\{\{") && pat.contains(':'), "pattern must exclude delimiters: {pat}");
    assert_eq!(val["$defs"]["NamedCheckpoint"]["properties"]["id"]["maxLength"], MAX_SHEET_ID_BYTES);
    assert_eq!(val["properties"]["image_assets"]["maxItems"], MAX_PROJECT_IMAGE_ASSETS);

    for sheet_def in ["DataSheet", "InfoSheet", "ResultSheet", "GraphSheet", "LayoutSheet"] {
        let props = val["$defs"][sheet_def]["properties"].as_object().ok_or("missing sheet properties")?;
        for (k, prop_val) in props {
            assert_ne!(*prop_val, serde_json::json!({}), "{sheet_def}.{k} must not be empty wildcard schema");
        }
    }
    Ok(())
}

#[test]
fn fa_sec02_13_summary_point_non_integer_n_message() -> TestRes {
    let mut xy = XyTable::new(curvebook_doc::SubcolumnMode::MeanSdN, 1, 1)?;
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(1.0), Cell::number(2.5)];
    let err = xy.summarized_y_point(0, 0).err().ok_or("expected error for non-integer N=2.5")?;
    assert!(matches!(err, SummaryPointError::SampleSizeBelowOne { n } if (n - 2.5).abs() < 1e-12));
    let msg = err.to_string();
    assert!(msg.contains("integer >= 1"), "unexpected SummaryPointError message: {msg}");
    Ok(())
}

#[test]
fn fa_sec02_14_project_too_many_cells_display_mentions_project() {
    let proj_err = DocError::TooManyCells { count: MAX_PROJECT_CELLS + 1, max: MAX_PROJECT_CELLS };
    assert!(proj_err.to_string().starts_with("project cell count "), "got: {proj_err}");
    let sheet_err = DocError::TooManyCells { count: MAX_SHEET_CELLS + 1, max: MAX_SHEET_CELLS };
    assert!(sheet_err.to_string().starts_with("sheet cell count "), "got: {sheet_err}");
}
