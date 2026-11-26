//! Targeted regression tests for AUD-01 findings in `curvebook-doc`
//! (`AUD-01-03`, `AUD-01-04`, `AUD-01-10`, `AUD-01-13`, `AUD-01-16`, `AUD-01-19`, `AUD-01-21`).

use curvebook_doc::analysis::{ALL_ANALYSIS_DESCRIPTORS, AnalysisKindId};
use curvebook_doc::schema::project_document_schema;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CloneDataSheetOptions, ClusterDistance, ClusterLinkage, CoxTieMethod, DataSheet, DemingErrorSpec,
    DocError, ErrorPropagationMode, LinearRegressionSpec, MAX_TITLE_BYTES, MathTransformPreset, MultiplicitySpec, MultivariateSpec,
    NestedGroupColumn, NestedTable, NormalizeAnchor, OutlierSpec, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SequenceProgression, SheetId, SimulationNoise, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec,
    XColumnMode, XyTable, infer_number_separators, parse_import_number, strip_thousands_grouping,
};

#[test]
fn aud_01_03_generated_sequence_x_exclusion_honored_by_value_summary_and_pairs() {
    for progression in [SequenceProgression::Arithmetic, SequenceProgression::Geometric] {
        let mut xy =
            XyTable::new_with_x_mode(XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression }, SubcolumnMode::Single, 1, 3).unwrap();
        xy.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)], vec![Cell::number(30.0)]];
        let mut ds = DataSheet::new("xy-gen", "Generated X", TableArchetype::Xy(xy)).unwrap();

        ds.set_x_or_time_excluded(0, 0, true).unwrap();
        let TableArchetype::Xy(ref xy) = ds.archetype else { panic!("expected Xy") };
        assert_eq!(xy.raw_x_value_at(0), Some(1.0));
        assert_eq!(xy.materialized_x_cell_at(0), Cell::excluded_number(1.0));
        assert_eq!(xy.x_value_at(0), None, "excluded generated X must not be active");
        assert_eq!(xy.x_summary_at(0), Ok(None));
        let expected_x1 = match progression {
            SequenceProgression::Arithmetic => 3.0,
            SequenceProgression::Geometric => 2.0,
        };
        let expected_x2 = match progression {
            SequenceProgression::Arithmetic => 5.0,
            SequenceProgression::Geometric => 4.0,
        };
        assert_eq!(xy.x_value_at(1), Some(expected_x1));
        assert_eq!(xy.active_xy_pairs(0), vec![(expected_x1, 20.0), (expected_x2, 30.0)]);

        // Un-excluding restores the point across all accessors.
        ds.set_x_or_time_excluded(0, 0, false).unwrap();
        let TableArchetype::Xy(ref xy) = ds.archetype else { panic!("expected Xy") };
        assert_eq!(xy.x_value_at(0), Some(1.0));
        assert_eq!(xy.active_xy_pairs(0), vec![(1.0, 10.0), (expected_x1, 20.0), (expected_x2, 30.0)]);
    }
}

#[test]
fn aud_01_04_linear_regression_non_finite_fixed_point_and_deming_error_rejected() {
    let cases = [
        (LinearRegressionSpec { fixed_point: Some((f64::NAN, 0.0)), ..LinearRegressionSpec::default() }, "analysis.fixed_point.x0"),
        (LinearRegressionSpec { fixed_point: Some((0.0, f64::INFINITY)), ..LinearRegressionSpec::default() }, "analysis.fixed_point.y0"),
        (
            LinearRegressionSpec {
                deming_model_ii: true,
                deming_error: DemingErrorSpec::StandardDeviations { sd_x: f64::NAN, sd_y: 1.0 },
                ..LinearRegressionSpec::default()
            },
            "analysis.deming_error.sd_x",
        ),
        (
            LinearRegressionSpec {
                deming_model_ii: true,
                deming_error: DemingErrorSpec::StandardDeviations { sd_x: 1.0, sd_y: f64::NEG_INFINITY },
                ..LinearRegressionSpec::default()
            },
            "analysis.deming_error.sd_y",
        ),
        (
            LinearRegressionSpec {
                deming_model_ii: true,
                deming_error: DemingErrorSpec::VarianceRatio { lambda: f64::INFINITY },
                ..LinearRegressionSpec::default()
            },
            "analysis.deming_error.lambda",
        ),
    ];

    for (spec, expected_path) in cases {
        let kind = AnalysisKind::LinearRegression(spec);
        assert!(AnalysisSpec::new("lin-bad", "data-1", kind.clone()).is_err(), "AnalysisSpec::new must reject {expected_path}");

        // Construct via finite spec then mutate to verify `first_non_finite_float` directly.
        let mut raw = AnalysisSpec::new("lin-raw", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
        raw.kind = kind;
        assert_eq!(raw.first_non_finite_float().as_deref(), Some(expected_path));
        assert!(raw.validate_structural().is_err());

        let mut doc = ProjectDocument::new("Finite Check").unwrap();
        let mut rs =
            ResultSheet::new(AnalysisSpec::new("lin-rs", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap())
                .unwrap();
        rs.analysis.kind = raw.kind;
        doc.result_sheets.push(rs);
        assert_eq!(doc.first_non_finite_float().as_deref(), Some(format!("lin-rs.{expected_path}").as_str()));
    }

    // Valid finite specs continue to pass.
    let valid = LinearRegressionSpec {
        deming_model_ii: true,
        fixed_point: Some((1.5, 2.5)),
        deming_error: DemingErrorSpec::StandardDeviations { sd_x: 0.5, sd_y: 0.25 },
        ..LinearRegressionSpec::default()
    };
    let ok = AnalysisSpec::new("lin-ok", "data-1", AnalysisKind::LinearRegression(valid)).unwrap();
    assert_eq!(ok.first_non_finite_float(), None);
}

#[test]
fn aud_01_10_project_document_schema_covers_all_analysis_kind_variants_including_r18() {
    let schema = project_document_schema();
    let analysis_def = schema.defs.get("AnalysisSpec").expect("AnalysisSpec $def");
    let kind_prop = analysis_def.properties.get("kind").expect("kind property");
    assert_eq!(kind_prop.any_of.len(), 2, "kind schema should be anyOf([PowerAnalysis, other_kind])");
    let power_props = &kind_prop.any_of[0].properties;
    let other_props = &kind_prop.any_of[1].properties;
    assert!(power_props.contains_key("PowerAnalysis"));
    assert!(other_props.contains_key("FitSplineLowess"), "missing FitSplineLowess in schema");
    assert!(other_props.contains_key("SimpleLogisticRegression"), "missing SimpleLogisticRegression in schema");
    assert!(other_props.contains_key("LeveneTest"), "missing LeveneTest in schema");

    for desc in &ALL_ANALYSIS_DESCRIPTORS {
        let present = power_props.contains_key(desc.serde_variant) || other_props.contains_key(desc.serde_variant);
        assert!(present, "AnalysisKind variant `{}` missing from project_document_schema()", desc.serde_variant);
    }
    assert_eq!(power_props.len() + other_props.len(), ALL_ANALYSIS_DESCRIPTORS.len());
}

#[test]
fn aud_01_13_nested_clone_preserves_y_cells_and_bounds_subcolumn_titles() {
    // Part 1: clone_structure with clear_y_values=false & override_replicates preserves measurements.
    let mut tbl = NestedTable::new(2, 2, 3).unwrap();
    tbl.groups[0].title = String::from("Vehicle");
    tbl.groups[0].cells[0] = vec![Cell::number(11.0), Cell::number(12.0)];
    tbl.groups[0].cells[1] = vec![Cell::number(13.0), Cell::excluded_number(14.0)];
    tbl.groups[1].cells[0] = vec![Cell::number(21.0), Cell::number(22.0)];
    let ds = DataSheet::new("nest-src", "Nested Source", TableArchetype::Nested(tbl)).unwrap();

    let widened = ds
        .clone_structure(
            "nest-wide",
            &CloneDataSheetOptions {
                title: String::from("Widened"),
                keep_x_values: true,
                clear_y_values: false,
                clear_column_titles: false,
                clear_row_titles: false,
                override_replicates: Some(4),
            },
        )
        .unwrap();
    let TableArchetype::Nested(ref wide_nt) = widened.archetype else { panic!("expected Nested") };
    assert_eq!(wide_nt.groups[0].biological_unit_count(), 4);
    assert_eq!(wide_nt.groups[0].cells[0], vec![Cell::number(11.0), Cell::number(12.0), Cell::empty(), Cell::empty()]);
    assert_eq!(wide_nt.groups[0].cells[1], vec![Cell::number(13.0), Cell::excluded_number(14.0), Cell::empty(), Cell::empty()]);
    assert_eq!(wide_nt.groups[1].cells[0], vec![Cell::number(21.0), Cell::number(22.0), Cell::empty(), Cell::empty()]);

    let cleared = ds
        .clone_structure(
            "nest-clear",
            &CloneDataSheetOptions {
                title: String::from("Cleared"),
                keep_x_values: true,
                clear_y_values: true,
                clear_column_titles: false,
                clear_row_titles: false,
                override_replicates: Some(4),
            },
        )
        .unwrap();
    assert_eq!(cleared.cell(CellCoord::new(0, 0, 0)), Some(&Cell::empty()));

    // Part 2: 4096-byte ASCII and multi-byte UTF-8 group titles produce valid <= MAX_TITLE_BYTES subcolumn titles.
    for long_title in ["A".repeat(MAX_TITLE_BYTES), "α".repeat(MAX_TITLE_BYTES / 2)] {
        assert_eq!(long_title.len(), MAX_TITLE_BYTES);
        let grp = NestedGroupColumn::new(long_title.clone(), 2, 2).unwrap();
        assert_eq!(grp.title.len(), MAX_TITLE_BYTES);
        for sub in &grp.subcolumn_titles {
            assert!(sub.len() <= MAX_TITLE_BYTES, "subcolumn title exceeded MAX_TITLE_BYTES: {}", sub.len());
        }
        let nested = NestedTable { row_titles: vec![String::new(); 2], row_titles_expanded: false, groups: vec![grp] };
        nested.validate().unwrap();
        let sheet = DataSheet::new("nest-long", "Long Group Title", TableArchetype::Nested(nested)).unwrap();
        let cloned = sheet
            .clone_structure(
                "nest-long-clone",
                &CloneDataSheetOptions {
                    title: String::from("Cloned Long"),
                    keep_x_values: true,
                    clear_y_values: false,
                    clear_column_titles: false,
                    clear_row_titles: false,
                    override_replicates: Some(3),
                },
            )
            .unwrap();
        cloned.validate().unwrap();
        let TableArchetype::Nested(ref cloned_nt) = cloned.archetype else { panic!("expected Nested") };
        assert_eq!(cloned_nt.groups[0].subcolumn_titles.len(), 3);
        for sub in &cloned_nt.groups[0].subcolumn_titles {
            assert!(sub.len() <= MAX_TITLE_BYTES);
        }
    }
}

#[test]
fn aud_01_16_strip_thousands_grouping_rejects_leading_zero_groups() {
    assert_eq!(strip_thousands_grouping("0,123", &[',']), None);
    assert_eq!(strip_thousands_grouping("00,123", &[',']), None);
    assert_eq!(strip_thousands_grouping("0,000", &[',']), None);
    assert_eq!(strip_thousands_grouping("0", &[',']), Some(String::from("0")));
    assert_eq!(strip_thousands_grouping("1,000", &[',']), Some(String::from("1000")));
    assert_eq!(strip_thousands_grouping("12,345,678", &[',']), Some(String::from("12345678")));

    assert_eq!(parse_import_number("0,123", '.', Some(',')), None);
    assert_eq!(parse_import_number("0,000.5", '.', Some(',')), None);
    assert_eq!(parse_import_number("0.123", '.', Some(',')), Some(0.123));
    assert_eq!(parse_import_number("1,234.56", '.', Some(',')), Some(1234.56));

    // Leading-zero comma fraction must not be misclassified as US thousands grouping.
    assert_eq!(infer_number_separators(["0,000.5", "0,123"].into_iter(), ';'), (',', None));
    assert_eq!(infer_number_separators(["1,234.56", "2,000.00"].into_iter(), ';'), ('.', Some(',')));
}

#[test]
fn aud_01_19_result_sheet_validate_rejects_non_finite_pvalue() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut rs = ResultSheet::new(AnalysisSpec::new("res-p", "data-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
        rs.tabs.push(ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("P value")],
            row_labels: vec![String::from("Row 1")],
            rows: vec![vec![ResultCell::PValue { raw: bad, formatted: String::new(), stars: String::new() }]],
            footnotes: Vec::new(),
        });
        assert!(
            matches!(rs.validate(), Err(DocError::InvalidCellValue { reason: "result.tabs.rows.p_value", .. })),
            "expected PValue finiteness rejection for {bad:?}"
        );
    }

    let mut good = ResultSheet::new(AnalysisSpec::new("res-ok", "data-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    good.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("P value")],
        row_labels: vec![String::from("Row 1")],
        rows: vec![vec![ResultCell::PValue { raw: 0.0123, formatted: String::from("0.0123"), stars: String::from("*") }]],
        footnotes: Vec::new(),
    });
    good.validate().unwrap();
}

#[test]
fn aud_01_21_all_sub_enum_variant_keys_registered_including_pcr_outcome_column() {
    let mv_desc = AnalysisKindId::Multivariate.descriptor();
    assert!(mv_desc.accepts_canonical_param_key("pcr_outcome_column"));

    let sub_enum_cases: Vec<(AnalysisKindId, AnalysisKind)> = vec![
        (
            AnalysisKindId::Multivariate,
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42), pcr_outcome_column: Some(0) }),
        ),
        (AnalysisKindId::Multivariate, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true })),
        (
            AnalysisKindId::Multivariate,
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: Some(3),
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Ward,
                k_sweep_max: Some(6),
            }),
        ),
        (
            AnalysisKindId::Multivariate,
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 5,
                max_iters: 10,
                seed: 7,
                response_column: Some(0),
                predictor_columns: vec![1, 2],
            }),
        ),
        (AnalysisKindId::OutlierDetection, AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: true })),
        (AnalysisKindId::OutlierDetection, AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        (
            AnalysisKindId::Survival,
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: Some(0),
                mv_status_column: Some(1),
                mv_group_column: Some(2),
            }),
        ),
        (
            AnalysisKindId::Survival,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: Vec::new(),
                tie_method: CoxTieMethod::Efron,
            }),
        ),
        (
            AnalysisKindId::Transform,
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::Log10),
                y_transform: None,
                error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            }),
        ),
        (
            AnalysisKindId::Transform,
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::ColumnMin,
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: true,
            }),
        ),
        (
            AnalysisKindId::Transform,
            AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
                seed: 1,
                row_count: 10,
                replicates: 2,
                x_min: 0.0,
                x_max: 10.0,
                equation_id: String::from("four_pl"),
                parameters: vec![(String::from("Top"), 100.0)],
                noise: SimulationNoise::None,
            }),
        ),
        (
            AnalysisKindId::Transform,
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                seed: 1,
                sample_size: 5,
                stratify_column: Some(0),
                proportional_allocation: true,
            }),
        ),
        (
            AnalysisKindId::Transform,
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                filter_formula: Some(String::from("[A] > 0")),
                target_archetype: TableKind::Column,
                derived_columns: vec![curvebook_doc::DerivedColumnFormula::new("D", "[A] * 2")],
            }),
        ),
    ];

    for (id, kind) in sub_enum_cases {
        let desc = id.descriptor();
        let val = serde_json::to_value(&kind).unwrap();
        let outer = val.as_object().unwrap();
        let inner = outer.get(desc.serde_variant).unwrap().as_object().unwrap();
        for sub_val in inner.values() {
            if let Some(sub_obj) = sub_val.as_object() {
                for key in sub_obj.keys() {
                    assert!(desc.accepts_canonical_param_key(key), "descriptor `{}` missing key `{key}`", desc.slug);
                }
            }
        }
    }
    let _ = SheetId::new("unused");
}
