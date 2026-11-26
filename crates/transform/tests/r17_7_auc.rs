//! R17.7 / R19.2 Area Under the Curve (AUC) verification suite:
//! partial-window integration, exact Bailer (1988) SE knot weights, `FirstPoint` covariance
//! weight adjustment, sampling-design gating, `n = 1` knot reporting, `1e-12` boundary tolerance,
//! one-sided & unwindowed comparability warnings, clipped peak detection, interior missing/excluded
//! row bridging (Yeh & Kwan 1978), and unavailable row retention in `auc-summary` & `analyzable_data`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AreaUnderCurveSpec, AucBaseline, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, ResultCell,
    ResultStatus, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::curve::{AucWindowOptions, compute_windowed_auc, execute_area_under_curve};
use curvebook_transform::{
    AucOptions, TransformContext, TransformError, compute_auc, compute_auc_with_se, compute_sheet_auc, execute_analysis_transform,
};

const TOL: f64 = 1e-12;

fn make_xy_single(id: &str, xs: &[f64], series: &[(&str, &[Option<f64>])]) -> Result<DataSheet, curvebook_doc::DocError> {
    let mut xy = XyTable::new(SubcolumnMode::Single, series.len(), xs.len())?;
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
    }
    for (c, &(title, ys)) in series.iter().enumerate() {
        xy.data_sets[c].title = title.to_owned();
        for (r, &y_opt) in ys.iter().enumerate() {
            if let Some(y) = y_opt {
                xy.data_sets[c].cells[r][0] = Cell::number(y);
            }
        }
    }
    DataSheet::new(id, id, TableArchetype::Xy(xy))
}

type MeanSdNRow = (f64, f64, f64, f64);

fn make_xy_mean_sd_n(id: &str, series: &[(&str, &[MeanSdNRow])]) -> Result<DataSheet, curvebook_doc::DocError> {
    let rows = series.first().map_or(0, |(_, pts)| pts.len());
    let mut xy = XyTable::new(SubcolumnMode::MeanSdN, series.len(), rows)?;
    for (c, &(title, pts)) in series.iter().enumerate() {
        xy.data_sets[c].title = title.to_owned();
        for (r, &(x, mean, sd, n)) in pts.iter().enumerate() {
            if c == 0 {
                xy.set_x_cell(r, Cell::number(x))?;
            }
            xy.data_sets[c].cells[r] = vec![Cell::number(mean), Cell::number(sd), Cell::number(n)];
        }
    }
    DataSheet::new(id, id, TableArchetype::Xy(xy))
}

fn auc_spec(source_id: &str, spec: AreaUnderCurveSpec) -> Result<AnalysisSpec, curvebook_doc::DocError> {
    AnalysisSpec::new("auc-1", source_id, AnalysisKind::AreaUnderCurve(spec))
}

fn cell_number(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::Number { value, .. } => Some(*value),
        _ => None,
    }
}

fn cell_text(cell: &ResultCell) -> Option<&str> {
    match cell {
        ResultCell::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

#[test]
fn unit_width_constant_curve_knot_weights_sum_to_one() {
    // Multi-segment non-uniform constant curve clipped to unit width [0.2, 1.2].
    let pts = [(0.0, 7.0, Some(0.5)), (0.3, 7.0, Some(0.4)), (0.9, 7.0, Some(0.3)), (1.5, 7.0, Some(0.2))];
    let opts = AucWindowOptions {
        baseline: AucBaseline::Fixed { value: 0.0 },
        x_min: Some(0.2),
        x_max: Some(1.2),
        compute_bailer_se: true,
        min_peak_height: 0.0,
        min_area_fraction: 0.0,
    };
    let res = compute_windowed_auc(&pts, &opts).unwrap();
    assert_approx_eq(res.span, 1.0, TOL, 0.0, "unit span");
    let weight_sum: f64 = res.knot_weights.iter().map(|&(_, w)| w).sum();
    assert_approx_eq(weight_sum, 1.0, TOL, 0.0, "multi-segment unit-width weight sum must equal 1.0");
    assert_approx_eq(res.auc.net_area, 7.0, TOL, 0.0, "constant curve area on unit window");

    // Exact analytic weights:
    // seg [0.0, 0.3] clipped [0.2, 0.3]: L=0.1, t_a=2/3, t_b=1, m=5/6 => w0=1/60, w1+=5/60
    // seg [0.3, 0.9] clipped [0.3, 0.9]: L=0.6, m=1/2 => w1+=0.3, w2+=0.3
    // seg [0.9, 1.5] clipped [0.9, 1.2]: L=0.3, t_a=0, t_b=1/2, m=1/4 => w2+=0.225, w3=0.075
    let expected_w = [1.0 / 60.0, 5.0 / 60.0 + 0.3, 0.3 + 0.225, 0.075];
    assert_eq!(res.knot_weights.len(), expected_w.len());
    for ((_, got_w), exp_w) in res.knot_weights.iter().zip(expected_w) {
        assert_approx_eq(*got_w, exp_w, TOL, 0.0, "exact clipped knot weight");
    }
    let exp_se = expected_w.iter().zip([0.5_f64, 0.4, 0.3, 0.2]).map(|(w, s)| (w * s).powi(2)).sum::<f64>().sqrt();
    assert_approx_eq(res.auc.standard_error.unwrap(), exp_se, TOL, 0.0, "unit-width Bailer SE");

    // Single-segment constant curve clipped strictly inside [0, 10] -> [3, 4].
    let single_seg = [(0.0, 5.0, Some(0.4)), (10.0, 5.0, Some(0.6))];
    let res_single = compute_windowed_auc(
        &single_seg,
        &AucWindowOptions { x_min: Some(3.0), x_max: Some(4.0), compute_bailer_se: true, ..AucWindowOptions::default() },
    )
    .unwrap();
    let w_sum_single: f64 = res_single.knot_weights.iter().map(|&(_, w)| w).sum();
    assert_approx_eq(w_sum_single, 1.0, TOL, 0.0, "single-segment unit-width weight sum must equal 1.0");
    assert_approx_eq(res_single.knot_weights[0].1, 0.65, TOL, 0.0, "single-segment left knot weight");
    assert_approx_eq(res_single.knot_weights[1].1, 0.35, TOL, 0.0, "single-segment right knot weight");
    assert_approx_eq(res_single.auc.net_area, 5.0, TOL, 0.0, "single-segment constant area");
    let exp_single_se = ((0.65_f64 * 0.4).powi(2) + (0.35_f64 * 0.6).powi(2)).sqrt();
    assert_approx_eq(res_single.auc.standard_error.unwrap(), exp_single_se, TOL, 0.0, "single-segment Bailer SE");
}

#[test]
fn partial_auc_and_bailer_se_match_exact_rational_oracle() {
    // Non-uniform knots with interior interpolated bounds [1.0, 10.5] and Fixed baseline 2.0.
    // Cross-checked via exact rational arithmetic (Python fractions.Fraction).
    let pts = [(0.0, 1.0, Some(0.2)), (2.0, 7.0, Some(0.3)), (5.0, -2.0, Some(0.4)), (9.0, 6.0, Some(0.25)), (12.0, 2.0, Some(0.5))];
    let opts = AucWindowOptions {
        baseline: AucBaseline::Fixed { value: 2.0 },
        x_min: Some(1.0),
        x_max: Some(10.5),
        compute_bailer_se: true,
        min_peak_height: 0.0,
        min_area_fraction: 0.0,
    };
    let res = compute_windowed_auc(&pts, &opts).unwrap();
    assert_approx_eq(res.start_x, 1.0, TOL, 0.0, "start_x");
    assert_approx_eq(res.end_x, 10.5, TOL, 0.0, "end_x");
    assert_approx_eq(res.span, 9.5, TOL, 0.0, "span");
    assert_eq!(res.n_points, 5);
    assert_approx_eq(res.resolved_baseline, 2.0, TOL, 0.0, "fixed baseline");
    assert_approx_eq(res.auc.positive_area, 97.0 / 6.0, TOL, 0.0, "positive area");
    assert_approx_eq(res.auc.negative_area, 20.0 / 3.0, TOL, 0.0, "negative area");
    assert_approx_eq(res.auc.net_area, 9.5, TOL, 0.0, "net area");
    assert_approx_eq(res.auc.total_area, 137.0 / 6.0, TOL, 0.0, "total area");

    let expected_weights = [0.25, 2.25, 3.5, 25.0 / 8.0, 3.0 / 8.0];
    for ((_, got_w), exp_w) in res.knot_weights.iter().zip(expected_weights) {
        assert_approx_eq(*got_w, exp_w, TOL, 0.0, "partial-AUC knot weight");
    }
    let expected_se = (78_429.0_f64 / 25_600.0).sqrt();
    assert_approx_eq(res.auc.standard_error.unwrap(), expected_se, TOL, 0.0, "partial-AUC Bailer SE");

    // Verify end-to-end execution through execute_analysis_transform on a MeanSdN XyTable
    // (using N = 4 so SD = 2 * SE reproduces the exact knot SEs).
    let mean_sd_n_rows: Vec<(f64, f64, f64, f64)> = pts.iter().map(|&(x, y, se)| (x, y, se.unwrap() * 2.0, 4.0)).collect();
    let sheet = make_xy_mean_sd_n("kinetic-sd", &[("Well A01", &mean_sd_n_rows)]).unwrap();
    let analysis = auc_spec(
        "kinetic-sd",
        AreaUnderCurveSpec {
            baseline: AucBaseline::Fixed { value: 2.0 },
            x_min: Some(1.0),
            x_max: Some(10.5),
            independent_time_points: true,
            min_peak_height: 0.0,
            min_area_fraction: 0.0,
        },
    )
    .unwrap();
    let result = execute_analysis_transform(&sheet, &analysis, &TransformContext::default()).unwrap();
    result.validate().unwrap();
    assert_eq!(result.status, ResultStatus::UpToDate);
    assert_approx_eq(result.lookup_scalar("PositiveArea", Some(0)).unwrap(), 97.0 / 6.0, TOL, 0.0, "scalar PositiveArea");
    assert_approx_eq(result.lookup_scalar("NegativeArea", Some(0)).unwrap(), 20.0 / 3.0, TOL, 0.0, "scalar NegativeArea");
    assert_approx_eq(result.lookup_scalar("NetArea", Some(0)).unwrap(), 9.5, TOL, 0.0, "scalar NetArea");
    assert_approx_eq(result.lookup_scalar("TotalArea", Some(0)).unwrap(), 137.0 / 6.0, TOL, 0.0, "scalar TotalArea");
    assert_approx_eq(result.lookup_scalar("NetAreaSe", Some(0)).unwrap(), expected_se, TOL, 0.0, "scalar NetAreaSe");

    // Check auc-summary column headers and values.
    let summary = result.tabs.iter().find(|t| t.id == "auc-summary").unwrap();
    assert_eq!(
        summary.column_headers,
        vec![
            "Start X",
            "End X",
            "Span",
            "N Points",
            "Baseline",
            "Positive Area",
            "Negative Area",
            "Net Area",
            "Total Area",
            "Net Area SE (Bailer, indep.)",
            "Peak Count",
            "Peak X",
            "Peak Y",
            "Status",
        ]
    );
    assert_eq!(summary.row_labels, vec!["Well A01"]);
    assert_approx_eq(cell_number(&summary.rows[0][7]).unwrap(), 9.5, TOL, 0.0, "summary Net Area");
    assert_approx_eq(cell_number(&summary.rows[0][9]).unwrap(), expected_se, TOL, 0.0, "summary Net Area SE");
    assert_eq!(cell_text(&summary.rows[0][13]), Some("OK"));

    // Check analyzable_data MultipleVariables schema and values.
    let analyzable = result.analyzable_data.as_ref().unwrap();
    let TableArchetype::MultipleVariables(mv) = &analyzable.archetype else {
        panic!("expected MultipleVariables analyzable_data");
    };
    let col_titles: Vec<&str> = mv.variables.iter().map(|v| v.title.as_str()).collect();
    assert_eq!(
        col_titles,
        vec!["Series", "Start X", "End X", "Span", "N Points", "Baseline", "Positive Area", "Negative Area", "Net Area", "Total Area", "Net Area SE",]
    );
    assert_eq!(mv.variables[0].role, VariableRole::Label);
    assert_eq!(mv.variables[0].cells[0].active_text(), Some("Well A01"));
    assert_approx_eq(mv.variables[8].cells[0].active_number().unwrap(), 9.5, TOL, 0.0, "analyzable Net Area");
    assert_approx_eq(mv.variables[10].cells[0].active_number().unwrap(), expected_se, TOL, 0.0, "analyzable Net Area SE");
}

#[test]
fn first_point_baseline_weights_sum_to_zero_and_match_exact_se_oracle() {
    // Interpolated window start x_min = 1.0 shares knots (0, 2) with the integral!
    let pts = [(0.0, 1.0, Some(0.2)), (2.0, 7.0, Some(0.3)), (5.0, -2.0, Some(0.4)), (9.0, 6.0, Some(0.25)), (12.0, 2.0, Some(0.5))];
    let opts = AucWindowOptions {
        baseline: AucBaseline::FirstPoint,
        x_min: Some(1.0),
        x_max: Some(10.5),
        compute_bailer_se: true,
        min_peak_height: 0.0,
        min_area_fraction: 0.0,
    };
    let res = compute_windowed_auc(&pts, &opts).unwrap();
    assert_approx_eq(res.resolved_baseline, 4.0, TOL, 0.0, "interpolated FirstPoint baseline y(1.0)");
    let weight_sum: f64 = res.knot_weights.iter().map(|&(_, w)| w).sum();
    assert_approx_eq(weight_sum, 0.0, TOL, 0.0, "FirstPoint adjusted weights must sum to 0");

    let expected_weights = [-4.5, -2.5, 3.5, 25.0 / 8.0, 3.0 / 8.0];
    for ((_, got_w), exp_w) in res.knot_weights.iter().zip(expected_weights) {
        assert_approx_eq(*got_w, exp_w, TOL, 0.0, "FirstPoint adjusted knot weight");
    }
    let linear_functional_net: f64 = res.knot_weights.iter().zip(pts.iter()).map(|(&(_, w), &(_, y, _))| w * y).sum();
    assert_approx_eq(res.auc.net_area, -9.5, TOL, 0.0, "FirstPoint net area");
    assert_approx_eq(linear_functional_net, res.auc.net_area, TOL, 0.0, "FirstPoint linear functional identity");
    assert_approx_eq(res.auc.positive_area, 5.5, TOL, 0.0, "FirstPoint positive area");
    assert_approx_eq(res.auc.negative_area, 15.0, TOL, 0.0, "FirstPoint negative area");
    assert_approx_eq(res.auc.total_area, 20.5, TOL, 0.0, "FirstPoint total area");

    let expected_se = (101_837.0_f64 / 25_600.0).sqrt();
    assert_approx_eq(res.auc.standard_error.unwrap(), expected_se, TOL, 0.0, "FirstPoint Bailer SE");

    // Exact-knot start x_min = 2.0, x_max = 9.0: knot 0 (x=0) and knot 4 (x=12) must not contribute.
    let res_exact = compute_windowed_auc(
        &pts,
        &AucWindowOptions {
            baseline: AucBaseline::FirstPoint,
            x_min: Some(2.0),
            x_max: Some(9.0),
            compute_bailer_se: true,
            ..AucWindowOptions::default()
        },
    )
    .unwrap();
    assert_eq!(res_exact.n_points, 3);
    assert_approx_eq(res_exact.resolved_baseline, 7.0, TOL, 0.0, "exact knot FirstPoint baseline");
    let exact_w_sum: f64 = res_exact.knot_weights.iter().map(|&(_, w)| w).sum();
    assert_approx_eq(exact_w_sum, 0.0, TOL, 0.0, "exact-knot FirstPoint weight sum == 0");
    // Unadjusted weights on [2, 5, 9]: [1.5, 3.5, 2.0]; subtract span=7 from knot at x=2 => [-5.5, 3.5, 2.0].
    assert_approx_eq(res_exact.knot_weights[0].1, -5.5, TOL, 0.0, "exact-knot FirstPoint w0");
    assert_approx_eq(res_exact.knot_weights[1].1, 3.5, TOL, 0.0, "exact-knot FirstPoint w1");
    assert_approx_eq(res_exact.knot_weights[2].1, 2.0, TOL, 0.0, "exact-knot FirstPoint w2");
    let exp_exact_se = ((-5.5_f64 * 0.3).powi(2) + (3.5_f64 * 0.4).powi(2) + (2.0_f64 * 0.25).powi(2)).sqrt();
    assert_approx_eq(res_exact.auc.standard_error.unwrap(), exp_exact_se, TOL, 0.0, "exact-knot FirstPoint SE");
}

#[test]
fn sampling_design_gating_and_n_equals_one_knot_diagnostics() {
    // 2 series with 3 replicates per row; Series B loses 2 replicates at X = 4.0 (n = 1).
    let xs = [0.0, 2.0, 4.0, 6.0];
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 2, xs.len()).unwrap();
    xy.data_sets[0].title = String::from("Series A");
    xy.data_sets[1].title = String::from("Series B");
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        let base = (r as f64 + 1.0) * 2.0;
        xy.data_sets[0].cells[r] = vec![Cell::number(base - 0.5), Cell::number(base), Cell::number(base + 0.5)];
        if r == 2 {
            // Only 1 active replicate at X = 4.0 (1 empty, 1 excluded).
            xy.data_sets[1].cells[r] = vec![Cell::number(base), Cell::empty(), Cell::excluded_number(base + 10.0)];
        } else {
            xy.data_sets[1].cells[r] = vec![Cell::number(base - 0.5), Cell::number(base), Cell::number(base + 0.5)];
        }
    }
    let sheet = DataSheet::new("rep-sheet", "Replicates", TableArchetype::Xy(xy)).unwrap();

    // 1. Default independent_time_points = false suppresses NetAreaSe for all series + emits Info note.
    let spec_dep = AreaUnderCurveSpec::default();
    assert!(!spec_dep.independent_time_points);
    let res_dep = execute_area_under_curve(&sheet, &auc_spec("rep-sheet", spec_dep.clone()).unwrap(), &spec_dep).unwrap();
    res_dep.validate().unwrap();
    assert_eq!(res_dep.lookup_scalar("NetAreaSe", Some(0)), None);
    assert_eq!(res_dep.lookup_scalar("NetAreaSe", Some(1)), None);
    let summary_dep = res_dep.tabs.iter().find(|t| t.id == "auc-summary").unwrap();
    assert_eq!(summary_dep.rows[0][9], ResultCell::Empty);
    assert_eq!(summary_dep.rows[1][9], ResultCell::Empty);
    let dep_info = res_dep
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Info && item.assumption.contains("Bailer SE"))
        .expect("expected Bailer SE Info checklist item");
    assert_eq!(dep_info.remediation, None);
    assert!(dep_info.explanation.contains("either direction"), "explanation: {}", dep_info.explanation);
    assert!(dep_info.explanation.contains("FirstPoint"), "explanation: {}", dep_info.explanation);

    // 2. independent_time_points = true emits NetAreaSe for Series A (all knots n = 3),
    //    leaves Series B blank (n = 1 at X = 4), and emits QOL-7 Info note naming X = 4.
    let spec_indep = AreaUnderCurveSpec { independent_time_points: true, ..AreaUnderCurveSpec::default() };
    let res_indep = execute_area_under_curve(&sheet, &auc_spec("rep-sheet", spec_indep.clone()).unwrap(), &spec_indep).unwrap();
    res_indep.validate().unwrap();
    assert!(res_indep.lookup_scalar("NetAreaSe", Some(0)).is_some());
    assert_eq!(res_indep.lookup_scalar("NetAreaSe", Some(1)), None);
    let n1_info = res_indep
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Info && item.explanation.contains("Series B") && item.explanation.contains('4'))
        .expect("expected Info checklist item naming Series B and n = 1 knot X = 4");
    assert_eq!(n1_info.remediation, None);

    // 3. Windowing to [0.0, 2.0] excludes the n = 1 knot at X = 4.0, so Series B DOES get NetAreaSe!
    let spec_sub = AreaUnderCurveSpec { x_min: Some(0.0), x_max: Some(2.0), independent_time_points: true, ..AreaUnderCurveSpec::default() };
    let res_sub = execute_area_under_curve(&sheet, &auc_spec("rep-sheet", spec_sub.clone()).unwrap(), &spec_sub).unwrap();
    assert!(res_sub.lookup_scalar("NetAreaSe", Some(1)).is_some());
    assert!(!res_sub.checklist.iter().any(|item| item.assumption.contains("n = 1")));

    // 4. Single-replicate sheet with independent_time_points = true emits None + Info note naming knots.
    let single_sheet = make_xy_single("sgl", &[0.0, 5.0, 10.0], &[("SingleWell", &[Some(1.0), Some(3.0), Some(2.0)])]).unwrap();
    let res_sgl = execute_area_under_curve(&single_sheet, &auc_spec("sgl", spec_indep.clone()).unwrap(), &spec_indep).unwrap();
    assert_eq!(res_sgl.lookup_scalar("NetAreaSe", Some(0)), None);
    let sgl_info = res_sgl
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Info && item.explanation.contains("SingleWell"))
        .expect("expected Info checklist item for single-replicate series");
    assert_eq!(sgl_info.remediation, None);
    assert!(sgl_info.explanation.contains('0') && sgl_info.explanation.contains('5') && sgl_info.explanation.contains("10"));

    // 5. MeanSdN with N = 1 at one knot treats that knot as missing point SE.
    let msn_sheet = make_xy_mean_sd_n("msn-n1", &[("ParamN1", &[(0.0, 1.0, 0.2, 4.0), (5.0, 3.0, 0.0, 1.0), (10.0, 2.0, 0.2, 4.0)])]).unwrap();
    let res_msn = execute_area_under_curve(&msn_sheet, &auc_spec("msn-n1", spec_indep.clone()).unwrap(), &spec_indep).unwrap();
    assert_eq!(res_msn.lookup_scalar("NetAreaSe", Some(0)), None);
    assert!(
        res_msn
            .checklist
            .iter()
            .any(|item| item.status == ChecklistStatus::Info && item.explanation.contains("ParamN1") && item.explanation.contains('5'))
    );
}

#[test]
fn boundary_tolerance_snaps_jitter_and_retains_unavailable_truncated_rows() {
    // Jitter within 1e-12 * max(1, |bound|) snaps cleanly; genuine truncation marks series unavailable.
    let xs = [30.000_000_000_000_004, 60.0, 65.0, 89.999_999_999_999_99];
    let sheet = make_xy_single(
        "jitter-trunc",
        &xs,
        &[
            ("JitterOK", &[Some(2.0), Some(4.0), Some(4.0), Some(2.0)]),
            ("TruncatedStart", &[None, None, Some(3.0), Some(5.0)]),
            ("TruncatedEnd", &[Some(2.0), Some(4.0), None, None]),
            ("SinglePoint", &[Some(5.0), None, None, None]),
        ],
    )
    .unwrap();

    let spec = AreaUnderCurveSpec {
        baseline: AucBaseline::Fixed { value: 0.0 },
        x_min: Some(30.0),
        x_max: Some(90.0),
        independent_time_points: false,
        min_peak_height: 0.0,
        min_area_fraction: 0.0,
    };
    let res = execute_area_under_curve(&sheet, &auc_spec("jitter-trunc", spec.clone()).unwrap(), &spec).unwrap();
    res.validate().unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);

    let summary = res.tabs.iter().find(|t| t.id == "auc-summary").unwrap();
    assert_eq!(summary.row_labels, vec!["JitterOK", "TruncatedStart", "TruncatedEnd", "SinglePoint"]);
    assert_approx_eq(cell_number(&summary.rows[0][0]).unwrap(), 30.0, TOL, 0.0, "snapped Start X");
    assert_approx_eq(cell_number(&summary.rows[0][1]).unwrap(), 90.0, TOL, 0.0, "snapped End X");
    assert_approx_eq(cell_number(&summary.rows[0][2]).unwrap(), 60.0, TOL, 0.0, "snapped Span");
    // Trapezoid over (30,2)-(60,4)-(65,4)-(90,2): 30*3 + 5*4 + 25*3 = 90 + 20 + 75 = 185.
    assert_approx_eq(cell_number(&summary.rows[0][7]).unwrap(), 185.0, TOL, 0.0, "snapped Net Area");
    assert_eq!(cell_text(&summary.rows[0][13]), Some("OK"));

    // Unavailable rows retain their Series row with empty numeric cells + explicit reason in Status.
    for row_idx in [1_usize, 2, 3] {
        for col_idx in 0..13 {
            assert_eq!(summary.rows[row_idx][col_idx], ResultCell::Empty, "row {row_idx} col {col_idx} must be empty");
        }
        let status = cell_text(&summary.rows[row_idx][13]).unwrap();
        assert!(status.starts_with("Unavailable:"), "expected Unavailable status, got: {status}");
    }

    // analyzable_data retains all 4 rows with empty numeric cells on rows 1..=3.
    let TableArchetype::MultipleVariables(mv) = &res.analyzable_data.as_ref().unwrap().archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.row_count(), 4);
    for (r, expected_name) in ["JitterOK", "TruncatedStart", "TruncatedEnd", "SinglePoint"].into_iter().enumerate() {
        assert_eq!(mv.variables[0].cells[r].active_text(), Some(expected_name));
    }
    assert_approx_eq(mv.variables[8].cells[0].active_number().unwrap(), 185.0, TOL, 0.0, "analyzable JitterOK Net Area");
    for row_idx in [1_usize, 2, 3] {
        for var_idx in 1..mv.variables.len() {
            assert!(mv.variables[var_idx].cells[row_idx].is_empty(), "analyzable row {row_idx} col {var_idx} must be empty");
        }
    }

    // Scalars emitted only for JitterOK (ds_idx 0).
    assert!(res.lookup_scalar("NetArea", Some(0)).is_some());
    assert_eq!(res.lookup_scalar("NetArea", Some(1)), None);
    assert_eq!(res.lookup_scalar("NetArea", Some(2)), None);
    assert_eq!(res.lookup_scalar("NetArea", Some(3)), None);

    // Zero selected series succeeding must fail the entire ResultSheet.
    let mut only_trunc = auc_spec("jitter-trunc", spec.clone()).unwrap();
    only_trunc.selected_data_sets = vec![1, 2, 3];
    let err = execute_area_under_curve(&sheet, &only_trunc, &spec).unwrap_err();
    assert!(matches!(err, TransformError::InvalidInput { .. }));

    // Every checklist item has remediation: None.
    assert!(res.checklist.iter().all(|item| item.remediation.is_none()));
}

#[test]
fn one_sided_and_unwindowed_unequal_spans_emit_comparability_warning() {
    // Well Short ends at 8, Well Long ends at 10.
    let sheet = make_xy_single(
        "spans",
        &[0.0, 2.0, 8.0, 10.0],
        &[("Well Short", &[Some(1.0), Some(2.0), Some(4.0), None]), ("Well Long", &[Some(1.0), Some(2.0), Some(4.0), Some(6.0)])],
    )
    .unwrap();

    // 1. One-sided bound: x_min = Some(2.0), x_max = None -> effective spans 6 vs 8 -> Warn!
    let spec_one_sided = AreaUnderCurveSpec { x_min: Some(2.0), x_max: None, ..AreaUnderCurveSpec::default() };
    let res_one = execute_area_under_curve(&sheet, &auc_spec("spans", spec_one_sided.clone()).unwrap(), &spec_one_sided).unwrap();
    let summary_one = res_one.tabs.iter().find(|t| t.id == "auc-summary").unwrap();
    assert_approx_eq(cell_number(&summary_one.rows[0][0]).unwrap(), 2.0, TOL, 0.0, "short Start X");
    assert_approx_eq(cell_number(&summary_one.rows[0][1]).unwrap(), 8.0, TOL, 0.0, "short End X");
    assert_approx_eq(cell_number(&summary_one.rows[0][2]).unwrap(), 6.0, TOL, 0.0, "short Span");
    assert_approx_eq(cell_number(&summary_one.rows[1][0]).unwrap(), 2.0, TOL, 0.0, "long Start X");
    assert_approx_eq(cell_number(&summary_one.rows[1][1]).unwrap(), 10.0, TOL, 0.0, "long End X");
    assert_approx_eq(cell_number(&summary_one.rows[1][2]).unwrap(), 8.0, TOL, 0.0, "long Span");

    let warn_one = res_one
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Warn && item.assumption.contains("Comparable integration windows"))
        .expect("expected unequal effective-window Warn for one-sided x_min=2, x_max=None");
    assert_eq!(warn_one.remediation, None);
    assert!(warn_one.explanation.contains("Well Short") && warn_one.explanation.contains("Well Long"));
    assert!(warn_one.explanation.contains("span 6") && warn_one.explanation.contains("span 8"));

    // 2. Unwindowed: x_min = None, x_max = None -> spans 8 vs 10 -> Warn!
    let spec_unwin = AreaUnderCurveSpec::default();
    let res_unwin = execute_area_under_curve(&sheet, &auc_spec("spans", spec_unwin.clone()).unwrap(), &spec_unwin).unwrap();
    let warn_unwin = res_unwin
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Warn && item.assumption.contains("Comparable integration windows"))
        .expect("expected unequal effective-window Warn for unwindowed uneven spans");
    assert_eq!(warn_unwin.remediation, None);
    assert!(warn_unwin.explanation.contains("span 8") && warn_unwin.explanation.contains("span 10"));

    // 3. One-sided bound where effective windows ARE equal (x_min = None, x_max = Some(8.0) -> both [0, 8]) -> NO Warn!
    let spec_equal = AreaUnderCurveSpec { x_min: None, x_max: Some(8.0), ..AreaUnderCurveSpec::default() };
    let res_equal = execute_area_under_curve(&sheet, &auc_spec("spans", spec_equal.clone()).unwrap(), &spec_equal).unwrap();
    assert!(!res_equal.checklist.iter().any(|item| item.status == ChecklistStatus::Warn));
}

#[test]
fn peaks_under_clipped_window_and_first_point_stay_within_effective_bounds() {
    // Three peaks at X=1 (outside), X=4 (clipped on left at x_min=3), and X=9 (clipped on right at x_max=10).
    let pts = [(0.0, 0.0), (1.0, 10.0), (2.0, 0.0), (4.0, 6.0), (6.0, 0.0), (7.0, -2.0), (9.0, 8.0), (11.0, 0.0)];
    let xs: Vec<f64> = pts.iter().map(|&(x, _)| x).collect();
    let ys: Vec<Option<f64>> = pts.iter().map(|&(_, y)| Some(y)).collect();
    let sheet = make_xy_single("peaks-clip", &xs, &[("Trace", &ys)]).unwrap();

    let spec = AreaUnderCurveSpec {
        baseline: AucBaseline::Fixed { value: 1.0 },
        x_min: Some(3.0),
        x_max: Some(10.0),
        independent_time_points: false,
        min_peak_height: 0.0,
        min_area_fraction: 0.0,
    };
    let res = execute_area_under_curve(&sheet, &auc_spec("peaks-clip", spec.clone()).unwrap(), &spec).unwrap();
    let peaks_tab = res.tabs.iter().find(|t| t.id == "auc-peaks").expect("expected auc-peaks tab");
    assert_eq!(peaks_tab.rows.len(), 2, "outside peak at X=1 must not appear");

    // Check every peak start_x and end_x is bounded inside [3.0, 10.0].
    let win_range = (3.0 - TOL)..=(10.0 + TOL);
    for row in &peaks_tab.rows {
        let peak_x = cell_number(&row[2]).unwrap();
        let start_x = cell_number(&row[5]).unwrap();
        let end_x = cell_number(&row[6]).unwrap();
        assert!(win_range.contains(&start_x), "start_x={start_x} out of [3, 10]");
        assert!(win_range.contains(&peak_x), "peak_x={peak_x} out of [3, 10]");
        assert!(win_range.contains(&end_x), "end_x={end_x} out of [3, 10]");
        assert!(start_x <= peak_x && peak_x <= end_x);
    }
    // Peak 1 starts at x_min=3.0 (y(3)=3.0 > 1.0), apex at (4, 6), crosses y=1 at x = 4 + 5/6 * 2 = 17/3.
    assert_approx_eq(cell_number(&peaks_tab.rows[0][5]).unwrap(), 3.0, TOL, 0.0, "clipped peak 1 start_x");
    assert_approx_eq(cell_number(&peaks_tab.rows[0][6]).unwrap(), 17.0 / 3.0, TOL, 0.0, "clipped peak 1 end_x");
    // Peak 2 ends at x_max=10.0 (y(10)=4.0 > 1.0), apex at (9, 8).
    assert_approx_eq(cell_number(&peaks_tab.rows[1][6]).unwrap(), 10.0, TOL, 0.0, "clipped peak 2 end_x");

    // Dominant peak in summary should be Peak 2 (Peak X = 9, Peak Y = 8).
    let summary = res.tabs.iter().find(|t| t.id == "auc-summary").unwrap();
    assert_approx_eq(cell_number(&summary.rows[0][10]).unwrap(), 2.0, TOL, 0.0, "Peak Count");
    assert_approx_eq(cell_number(&summary.rows[0][11]).unwrap(), 9.0, TOL, 0.0, "Dominant Peak X");
    assert_approx_eq(cell_number(&summary.rows[0][12]).unwrap(), 8.0, TOL, 0.0, "Dominant Peak Y");

    // FirstPoint baseline on [3.0, 10.0]: y(3.0) = 3.0, so Peak 1 starts at 3.0 (d0=0, rises to 6 at X=4)
    // and ends at X=5.0; Peak 2 rises above 3.0 at X=8.0, apex (9, 8), ends at X=10.0 (y=4 > 3).
    let spec_fp = AreaUnderCurveSpec { baseline: AucBaseline::FirstPoint, x_min: Some(3.0), x_max: Some(10.0), ..AreaUnderCurveSpec::default() };
    let res_fp = execute_area_under_curve(&sheet, &auc_spec("peaks-clip", spec_fp.clone()).unwrap(), &spec_fp).unwrap();
    let fp_peaks = res_fp.tabs.iter().find(|t| t.id == "auc-peaks").unwrap();
    assert_eq!(fp_peaks.rows.len(), 2);
    assert_approx_eq(cell_number(&fp_peaks.rows[0][5]).unwrap(), 3.0, TOL, 0.0, "FP peak 1 start_x");
    assert_approx_eq(cell_number(&fp_peaks.rows[0][6]).unwrap(), 5.0, TOL, 0.0, "FP peak 1 end_x");
    assert_approx_eq(cell_number(&fp_peaks.rows[1][5]).unwrap(), 8.0, TOL, 0.0, "FP peak 2 start_x");
    assert_approx_eq(cell_number(&fp_peaks.rows[1][6]).unwrap(), 10.0, TOL, 0.0, "FP peak 2 end_x");

    // When min_peak_height filters out all peaks, auc-peaks tab is omitted.
    let spec_high = AreaUnderCurveSpec { min_peak_height: 100.0, ..spec };
    let res_high = execute_area_under_curve(&sheet, &auc_spec("peaks-clip", spec_high.clone()).unwrap(), &spec_high).unwrap();
    assert!(!res_high.tabs.iter().any(|t| t.id == "auc-peaks"));
}

#[test]
fn interior_missing_and_excluded_rows_bridge_linearly_and_report_in_checklist() {
    // X = [0, 1, 2, 3, 4, 5, 6]:
    // - row 2 (X=2): Y empty (missing interior)
    // - row 4 (X=4): Y = 99* excluded (excluded interior)
    // - row 6 (X=6): Y empty (trailing missing — must NOT count as bridged!)
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 7).unwrap();
    xy.data_sets[0].title = String::from("BridgedWell");
    for (r, x) in [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    xy.data_sets[0].cells[0][0] = Cell::number(0.0);
    xy.data_sets[0].cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[2][0] = Cell::empty();
    xy.data_sets[0].cells[3][0] = Cell::number(6.0);
    xy.data_sets[0].cells[4][0] = Cell::excluded_number(99.0);
    xy.data_sets[0].cells[5][0] = Cell::number(10.0);
    xy.data_sets[0].cells[6][0] = Cell::empty();
    let sheet = DataSheet::new("bridge-xy", "Bridge", TableArchetype::Xy(xy)).unwrap();

    // Linear y = 2*x across [0, 5] => bridged integral = 0.5 * 5 * 10 = 25.0.
    let spec = AreaUnderCurveSpec::default();
    let res = execute_area_under_curve(&sheet, &auc_spec("bridge-xy", spec.clone()).unwrap(), &spec).unwrap();
    assert_approx_eq(res.lookup_scalar("NetArea", Some(0)).unwrap(), 25.0, TOL, 0.0, "bridged linear area");

    let bridge_item = res
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Info && item.assumption.contains("Interior missing or excluded"))
        .expect("expected Info checklist item counting bridged rows");
    assert_eq!(bridge_item.remediation, None);
    assert!(
        bridge_item.explanation.contains("BridgedWell: 2 bridged rows"),
        "expected exactly 2 bridged rows (excluding trailing row 6), got: {}",
        bridge_item.explanation
    );

    // Partial window [0, 3] only bridges row 2 (X=2), NOT row 4 (X=4).
    let spec_sub = AreaUnderCurveSpec { x_min: Some(0.0), x_max: Some(3.0), ..AreaUnderCurveSpec::default() };
    let res_sub = execute_area_under_curve(&sheet, &auc_spec("bridge-xy", spec_sub.clone()).unwrap(), &spec_sub).unwrap();
    assert_approx_eq(res_sub.lookup_scalar("NetArea", Some(0)).unwrap(), 9.0, TOL, 0.0, "partial bridged area [0, 3]");
    let sub_item = res_sub
        .checklist
        .iter()
        .find(|item| item.assumption.contains("Interior missing or excluded"))
        .expect("expected Info checklist item for [0, 3]");
    assert!(sub_item.explanation.contains("BridgedWell: 1 bridged row"), "expected exactly 1 bridged row in [0, 3], got: {}", sub_item.explanation);

    // Excluded X cell at an interior coordinate (X = 2* with active Y) is also bridged and counted.
    let mut xy_ex_x = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    xy_ex_x.data_sets[0].title = String::from("ExcludedX");
    xy_ex_x.set_x_cell(0, Cell::number(0.0)).unwrap();
    xy_ex_x.set_x_cell(1, Cell::excluded_number(2.0)).unwrap();
    xy_ex_x.set_x_cell(2, Cell::number(4.0)).unwrap();
    xy_ex_x.set_x_cell(3, Cell::number(6.0)).unwrap();
    for (r, y) in [0.0, 999.0, 4.0, 6.0].into_iter().enumerate() {
        xy_ex_x.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet_ex_x = DataSheet::new("bridge-ex-x", "Bridge ExX", TableArchetype::Xy(xy_ex_x)).unwrap();
    let res_ex_x = execute_area_under_curve(&sheet_ex_x, &auc_spec("bridge-ex-x", spec.clone()).unwrap(), &spec).unwrap();
    assert_approx_eq(res_ex_x.lookup_scalar("NetArea", Some(0)).unwrap(), 18.0, TOL, 0.0, "excluded-X bridged area");
    assert!(res_ex_x.checklist.iter().any(|item| item.explanation.contains("ExcludedX: 1 bridged row")));
}

#[test]
fn validation_non_numeric_x_selection_and_backwards_compat_apis() {
    // 1. Incompatible table archetype fails cleanly.
    let col_table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    let col_sheet = DataSheet::new("col-1", "Column", TableArchetype::Column(col_table)).unwrap();
    let err =
        execute_area_under_curve(&col_sheet, &auc_spec("col-1", AreaUnderCurveSpec::default()).unwrap(), &AreaUnderCurveSpec::default()).unwrap_err();
    assert!(matches!(err, TransformError::IncompatibleTable { .. }));

    // 2. Invalid spec parameters reject with InvalidSpec (and AnalysisSpec::new rejects non-finite floats).
    let valid_sheet =
        make_xy_single("ok", &[0.0, 2.0, 4.0], &[("A", &[Some(1.0), Some(3.0), Some(2.0)]), ("B", &[Some(2.0), Some(4.0), Some(1.0)])]).unwrap();
    let base_analysis = auc_spec("ok", AreaUnderCurveSpec::default()).unwrap();
    for bad_spec in [
        AreaUnderCurveSpec { baseline: AucBaseline::Fixed { value: f64::NAN }, ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { x_min: Some(f64::INFINITY), ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { x_max: Some(f64::NAN), ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { x_min: Some(4.0), x_max: Some(2.0), ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { x_min: Some(2.0), x_max: Some(2.0), ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { min_peak_height: -0.1, ..AreaUnderCurveSpec::default() },
        AreaUnderCurveSpec { min_area_fraction: 1.1, ..AreaUnderCurveSpec::default() },
    ] {
        let err = execute_area_under_curve(&valid_sheet, &base_analysis, &bad_spec).unwrap_err();
        assert!(matches!(err, TransformError::InvalidSpec { .. }), "expected InvalidSpec for {bad_spec:?}, got {err:?}");
    }
    assert!(
        AnalysisSpec::new(
            "bad-nan",
            "ok",
            AnalysisKind::AreaUnderCurve(AreaUnderCurveSpec { x_min: Some(f64::NAN), ..AreaUnderCurveSpec::default() })
        )
        .is_err()
    );

    // 3. selected_data_sets preserves upstream data_set_index on scalars.
    let mut sub_analysis = auc_spec("ok", AreaUnderCurveSpec::default()).unwrap();
    sub_analysis.selected_data_sets = vec![1];
    let sub_res = execute_analysis_transform(&valid_sheet, &sub_analysis, &TransformContext::default()).unwrap();
    assert_eq!(sub_res.lookup_scalar("NetArea", Some(0)), None);
    // Series B: (0,2)-(2,4)-(4,1) => 2*3 + 2*2.5 = 11.0.
    assert_approx_eq(sub_res.lookup_scalar("NetArea", Some(1)).unwrap(), 11.0, TOL, 0.0, "selected dataset index 1 NetArea");

    // 4. Non-numeric hh:mm:ss text X cells are rejected cleanly rather than integrated as garbage.
    let mut txt_xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    txt_xy.data_sets[0].title = String::from("TimeText");
    for (r, ts) in ["00:00:00", "00:01:00", "00:02:00"].into_iter().enumerate() {
        txt_xy.set_x_cell(r, Cell::text(ts)).unwrap();
        txt_xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let txt_sheet = DataSheet::new("txt-x", "Text X", TableArchetype::Xy(txt_xy)).unwrap();
    let txt_err =
        execute_area_under_curve(&txt_sheet, &auc_spec("txt-x", AreaUnderCurveSpec::default()).unwrap(), &AreaUnderCurveSpec::default()).unwrap_err();
    let msg = txt_err.to_string();
    assert!(msg.contains("numeric X"), "expected numeric-X guidance, got: {msg}");

    // 4b. Mixed numeric and non-numeric text X rows ignore text rows and emit a QOL-7 Warn.
    let mut mixed_xy = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    mixed_xy.data_sets[0].title = String::from("MixedX");
    mixed_xy.set_x_cell(0, Cell::number(0.0)).unwrap();
    mixed_xy.set_x_cell(1, Cell::text("00:01:00")).unwrap();
    mixed_xy.set_x_cell(2, Cell::number(2.0)).unwrap();
    mixed_xy.set_x_cell(3, Cell::number(4.0)).unwrap();
    for (r, y) in [0.0, 999.0, 2.0, 4.0].into_iter().enumerate() {
        mixed_xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mixed_sheet = DataSheet::new("mixed-x", "Mixed X", TableArchetype::Xy(mixed_xy)).unwrap();
    let mixed_res =
        execute_area_under_curve(&mixed_sheet, &auc_spec("mixed-x", AreaUnderCurveSpec::default()).unwrap(), &AreaUnderCurveSpec::default()).unwrap();
    assert_approx_eq(mixed_res.lookup_scalar("NetArea", Some(0)).unwrap(), 8.0, TOL, 0.0, "mixed-X numeric area");
    let mixed_warn = mixed_res
        .checklist
        .iter()
        .find(|item| item.status == ChecklistStatus::Warn && item.assumption.contains("Numeric X"))
        .expect("expected Numeric X Warn checklist item");
    assert_eq!(mixed_warn.remediation, None);

    // 5. Legacy AucOptions / compute_auc / compute_auc_with_se / compute_sheet_auc remain intact.
    let legacy_opts = AucOptions { baseline: 0.0, min_peak_height: 0.0, min_area_fraction: 0.0 };
    let legacy_auc = compute_auc(&[(0.0, 1.0), (2.0, 3.0), (4.0, 2.0)], &legacy_opts).unwrap();
    assert_approx_eq(legacy_auc.net_area, 9.0, TOL, 0.0, "legacy compute_auc");
    let legacy_se = compute_auc_with_se(&[(0.0, 1.0, 0.1), (2.0, 3.0, 0.2), (4.0, 2.0, 0.1)], &legacy_opts).unwrap();
    assert!(legacy_se.standard_error.is_some());
    let legacy_sheet = compute_sheet_auc(&valid_sheet, 0, &legacy_opts).unwrap();
    assert_approx_eq(legacy_sheet.net_area, 9.0, TOL, 0.0, "legacy compute_sheet_auc");
    assert!(matches!(compute_sheet_auc(&valid_sheet, 0, &AucOptions { baseline: f64::NAN, ..legacy_opts }), Err(TransformError::InvalidSpec { .. })));
}
