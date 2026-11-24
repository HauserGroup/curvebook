//! D1 result-sheet oracle checks for Poisson GLM and Cox PH regression.
//!
//! Reference values were generated independently with R 4.5.2 (`stats::glm` and
//! `survival` 3.8-3) via `reference/poisson_cox_result_sheet.R`. The Rust tests run
//! offline against the full-precision stored R outputs.
//!
//! Model settings and case-design provenance:
//! - Poisson GLM (design modeled on UCLA OARC Poisson regression walkthrough,
//!   <https://stats.oarc.ucla.edu/r/dae/poisson-regression/>, and R `stats::summary.glm`,
//!   <https://search.r-project.org/R/refmans/stats/help/summary.glm.html>):
//!   `stats::glm(..., family = poisson(link = "log"), control = glm.control(epsilon = 1e-14, maxit = 100))`,
//!   inference via `summary(..., dispersion = 1)` (model-based SEs, Wald z and p), and
//!   95% Wald rate-ratio intervals `exp(confint.default(..., level = 0.95))`.
//! - Cox PH (design modeled on UCLA OARC survival walkthrough,
//!   <https://stats.oarc.ucla.edu/wp-content/uploads/2025/02/survival_r_full.html>, and R `survival`,
//!   <https://cran.r-project.org/web/packages/survival/refman/survival.html>):
//!   `survival::coxph(Surv(time, status) ~ regimen + biomarker, ties = ...)` across `efron`,
//!   `breslow`, and `exact`, 95% Wald HR intervals, likelihood-ratio chi-square, and Efron
//!   baseline `H_0(t)` / `S_0(t)` from `survfit` evaluated at retained design-column means.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, CoxTieMethod, DataSheet, GlmFamily, GlmSpec, MultipleVariablesTable, ResultCell,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SurvivalAnalysisSpec, TableArchetype, VariableRole,
};
use curvebook_stats::run_analysis;
use curvebook_testkit::assert_approx_eq;

// Poisson IRLS terminates once log-likelihood change < 1e-9 and step < 1e-7
// (`crates/stats/src/regression.rs`), while R uses deviance epsilon = 1e-14.
// Observed differences across estimates, model-based SEs (dispersion = 1), Wald stats,
// rate ratios, Wald CIs, residual deviance, and row residuals stay within 1e-9.
const POISSON_ABS_TOL: f64 = 1e-9;
const POISSON_REL_TOL: f64 = 1e-9;
// Cox PH Newton-Raphson terminates once partial log-likelihood change < 1e-10 and
// linear-predictor step < 1e-8 (`crates/stats/src/survival.rs`), matching R's `coxph`
// (default eps = 1e-9) and `survfit` at retained design-column means within 1e-10.
const COX_ABS_TOL: f64 = 1e-10;
const COX_REL_TOL: f64 = 1e-10;
const COX_BASELINE_ABS_TOL: f64 = 1e-10;
const COX_BASELINE_REL_TOL: f64 = 1e-10;

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> Option<&'a ResultTab> {
    sheet.tabs.iter().find(|t| t.id == id)
}

fn scalar_val(sheet: &ResultSheet, key: &str) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == key).map(|s| s.value)
}

fn cell_num(cell: &ResultCell, label: &str) -> f64 {
    let ResultCell::Number { value, .. } = cell else {
        assert!(matches!(cell, ResultCell::Number { .. }), "{label}: expected Number cell, got {cell:?}");
        return f64::NAN;
    };
    *value
}

fn cell_p_val(cell: &ResultCell, label: &str) -> f64 {
    let ResultCell::PValue { raw, .. } = cell else {
        assert!(matches!(cell, ResultCell::PValue { .. }), "{label}: expected PValue cell, got {cell:?}");
        return f64::NAN;
    };
    *raw
}

fn cell_ci(cell: &ResultCell, label: &str) -> (f64, f64) {
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = cell else {
        assert!(
            matches!(cell, ResultCell::ConfidenceInterval { lower: CiBound::Finite(_), upper: CiBound::Finite(_) }),
            "{label}: expected finite ConfidenceInterval cell, got {cell:?}"
        );
        return (f64::NAN, f64::NAN);
    };
    (*lo, *hi)
}

#[test]
fn poisson_glm_result_sheet_matches_r_with_explicit_reference_and_zeros() {
    let events = [0.0, 1.0, 6.0, 3.0, 0.0, 2.0, 0.0, 4.0, 2.0, 3.0, 1.0, 12.0, 0.0, 2.0, 7.0, 0.0, 7.0, 15.0, 3.0, 4.0, 8.0];
    let programs = [
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
        "Control",
        "Focused",
        "Intensive",
    ];
    let scores = [0.8, 1.5, 1.2, 2.4, 0.9, 2.1, 1.6, 2.8, 1.9, 3.2, 3.6, 3.4, 2.9, 1.4, 3.1, 4.1, 4.3, 4.5, 3.7, 4.8, 4.0];

    let n = events.len();
    let mut mv = MultipleVariablesTable::new(3, n).unwrap();
    mv.variables[0].title = String::from("Events");
    mv.variables[1].title = String::from("Program");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Score");

    for r in 0..n {
        mv.variables[0].cells[r] = Cell::number(events[r]);
        mv.variables[1].cells[r] = Cell::text(programs[r]);
        mv.variables[2].cells[r] = Cell::number(scores[r]);
    }
    assert_eq!(mv.variables[1].ordered_levels().first().map(|s| s.level.as_str()), Some("Control"), "Focused must be a non-default reference level");

    let sheet = DataSheet::new(SheetId::new("pois-oracle-data"), "Poisson oracle data", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("pois-oracle-res"),
        SheetId::new("pois-oracle-data"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Poisson,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: vec![(1, String::from("Focused"))],
            compute_vif: false,
        }),
    )
    .unwrap();

    let result = run_analysis(&sheet, &spec).unwrap();
    assert_eq!(result.status, ResultStatus::UpToDate);
    let eq_chk =
        result.checklist.iter().find(|c| c.assumption == "Equidispersion assumption (Poisson)").expect("missing equidispersion checklist item");
    assert_eq!(eq_chk.status, ChecklistStatus::Pass);

    // Parameter estimates, model-based SEs (dispersion = 1), Wald z/p, RRs, and 95% Wald CIs.
    let params_tab = tab_by_id(&result, "glm-poisson-params").unwrap();
    assert_eq!(params_tab.kind, ResultTabKind::ParametersAndConfidenceIntervals);
    assert_eq!(params_tab.column_headers, vec!["beta", "SE", "Wald z", "P value", "Rate Ratio", "95% CI (RR)"]);
    assert_eq!(
        params_tab.row_labels,
        vec![
            String::from("Intercept"),
            String::from("Program[Control vs Focused]"),
            String::from("Program[Intensive vs Focused]"),
            String::from("Score"),
        ]
    );

    let expected_params = [
        (
            "Intercept",
            -0.435_243_997_400_681_65,
            0.441_453_100_217_549_2,
            -0.985_934_852_844_373_1,
            0.324_165_082_648_330_44,
            0.647_106_755_447_511_6,
            0.272_402_051_184_122_25,
            1.537_239_353_101_514_3,
        ),
        (
            "Program[Control vs Focused]",
            -0.619_363_020_260_221,
            0.407_691_339_821_340_4,
            -1.519_195_920_452_075_5,
            0.128_713_187_891_596_3,
            0.538_287_206_459_370_6,
            0.242_095_412_713_097_62,
            1.196_855_047_316_462_5,
        ),
        (
            "Program[Intensive vs Focused]",
            1.026_899_479_357_497,
            0.268_887_226_910_024_37,
            3.819_071_255_850_767_3,
            0.000_133_955_067_224_212_53,
            2.792_394_521_996_209,
            1.648_540_248_917_798_3,
            4.729_922_227_615_108,
        ),
        (
            "Score",
            0.448_555_892_660_485_3,
            0.104_706_533_019_100_81,
            4.283_934_151_259_298,
            1.836_172_767_687_327_2e-5,
            1.566_049_008_880_739_3,
            1.275_495_663_814_082,
            1.922_789_365_573_121_4,
        ),
    ];

    assert_eq!(params_tab.rows.len(), expected_params.len());
    for (i, (label, beta, se, z, p, rr, ci_lo, ci_hi)) in expected_params.into_iter().enumerate() {
        let row = &params_tab.rows[i];
        assert_approx_eq(cell_num(&row[0], label), beta, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} beta"));
        assert_approx_eq(cell_num(&row[1], label), se, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} se"));
        assert_approx_eq(cell_num(&row[2], label), z, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} wald z"));
        assert_approx_eq(cell_p_val(&row[3], label), p, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} p"));
        assert_approx_eq(cell_num(&row[4], label), rr, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} RR"));
        let (actual_lo, actual_hi) = cell_ci(&row[5], label);
        assert_approx_eq(actual_lo, ci_lo, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} RR CI lo"));
        assert_approx_eq(actual_hi, ci_hi, POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} RR CI hi"));
    }

    assert_approx_eq(
        scalar_val(&result, "Residual deviance").unwrap(),
        29.040_380_050_132_644,
        POISSON_ABS_TOL,
        POISSON_REL_TOL,
        "residual deviance",
    );
    assert_approx_eq(
        scalar_val(&result, "Pearson dispersion").unwrap(),
        1.478_894_596_761_119_6,
        POISSON_ABS_TOL,
        POISSON_REL_TOL,
        "pearson dispersion",
    );
    assert_approx_eq(scalar_val(&result, "AIC").unwrap(), 88.492_935_463_225_68, POISSON_ABS_TOL, POISSON_REL_TOL, "poisson AIC");

    // Fitted counts, Pearson residuals, deviance residuals, and row order preservation.
    let resid_tab = tab_by_id(&result, "glm-poisson-residuals").unwrap();
    assert_eq!(resid_tab.kind, ResultTabKind::Residuals);
    assert_eq!(resid_tab.column_headers, vec!["Observed Y", "Predicted count", "Pearson residual", "Deviance residual"]);
    let expected_row_labels: Vec<String> = (1..=n).map(|r| format!("Row {r}")).collect();
    assert_eq!(resid_tab.row_labels, expected_row_labels);

    let expected_fitted = [
        0.498_694_146_741_356_2,
        1.268_188_927_782_523_1,
        3.095_416_760_561_887_4,
        1.022_169_290_887_176_5,
        0.968_948_618_281_806_1,
        4.634_938_158_045_648,
        0.713_967_675_979_940_8,
        2.272_118_149_775_1,
        4.237_238_153_071_111,
        1.463_413_673_173_277_4,
        3.252_933_537_614_032_5,
        8.304_068_014_845_994,
        1.279_161_865_719_237_2,
        1.212_560_515_559_684_6,
        7.258_540_308_630_907,
        2.191_249_967_117_619_8,
        4.452_858_907_593_676,
        13.601_186_055_858_053,
        1.831_343_380_381_391_7,
        5.572_391_343_393_183,
        10.868_612_548_986_418,
    ];
    let expected_pearson = [
        -0.706_182_799_805_656_7,
        -0.238_149_056_574_863_32,
        1.650_913_284_016_011,
        1.956_265_072_905_265,
        -0.984_351_877_268_391_2,
        -1.223_907_101_831_862,
        -0.844_966_079_780_686,
        1.146_300_778_531_307_7,
        -1.086_852_928_647_049_6,
        1.270_203_807_136_961_5,
        -1.249_139_049_264_071,
        1.282_562_452_551_426,
        -1.131_000_382_722_851_6,
        0.715_097_857_117_062_1,
        -0.095_962_955_808_006_51,
        -1.480_287_123_201_988_7,
        1.207_072_998_374_325_8,
        0.379_290_148_047_939_3,
        0.863_578_918_508_614_1,
        -0.666_100_622_858_481_8,
        -0.870_131_398_631_653,
    ];
    let expected_deviance = [
        -0.998_693_292_999_764,
        -0.247_382_643_661_502,
        1.460_437_096_922_582_8,
        1.582_545_280_691_026_5,
        -1.392_083_774_980_375_4,
        -1.381_294_180_164_290_2,
        -1.194_962_489_771_073,
        1.033_871_932_165_637_3,
        -1.213_020_328_079_894_1,
        1.110_797_031_522_551_3,
        -1.465_180_071_825_280_3,
        1.201_666_849_232_695_6,
        -1.599_476_080_295_818_1,
        0.653_278_458_288_010_3,
        -0.096_541_261_519_212_1,
        -2.093_442_125_838_505,
        1.113_018_925_000_859,
        0.373_052_512_271_986_2,
        0.789_975_810_815_802,
        -0.701_814_275_138_594,
        -0.913_359_142_544_763_9,
    ];

    assert_eq!(resid_tab.rows.len(), n);
    for i in 0..n {
        let row = &resid_tab.rows[i];
        let label = format!("Row {}", i + 1);
        assert_approx_eq(cell_num(&row[0], &label), events[i], 0.0, 0.0, &format!("{label} observed count"));
        assert_approx_eq(cell_num(&row[1], &label), expected_fitted[i], POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} fitted count"));
        assert_approx_eq(cell_num(&row[2], &label), expected_pearson[i], POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} pearson residual"));
        assert_approx_eq(cell_num(&row[3], &label), expected_deviance[i], POISSON_ABS_TOL, POISSON_REL_TOL, &format!("{label} deviance residual"));
    }
}

#[test]
fn cox_ph_result_sheets_match_r_across_tie_methods_and_efron_baseline() {
    let times = [15.0, 5.0, 22.0, 7.0, 10.0, 3.0, 10.0, 11.0, 5.0, 28.0, 12.0, 14.0, 10.0, 24.0, 15.0, 30.0, 8.0, 17.0, 26.0];
    let statuses = [1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0];
    let regimens = [
        "Control", "ArmA", "ArmB", "Control", "ArmB", "Control", "ArmA", "ArmA", "ArmB", "ArmB", "ArmB", "Control", "Control", "Control", "ArmA",
        "Control", "ArmA", "Control", "ArmA",
    ];
    let biomarkers = [
        Some(3.6),
        Some(2.8),
        Some(1.9),
        Some(2.2),
        Some(4.4),
        Some(4.1),
        Some(3.3),
        None,
        Some(3.9),
        Some(1.4),
        Some(3.1),
        Some(1.8),
        Some(2.9),
        Some(2.5),
        Some(2.7),
        Some(1.6),
        Some(3.5),
        Some(2.1),
        Some(2.0),
    ];

    let n = times.len();
    let mut mv = MultipleVariablesTable::new(4, n).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Regimen");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Biomarker");

    for r in 0..n {
        mv.variables[0].cells[r] = Cell::number(times[r]);
        mv.variables[1].cells[r] = Cell::number(statuses[r]);
        mv.variables[2].cells[r] = Cell::text(regimens[r]);
        mv.variables[3].cells[r] = biomarkers[r].map_or_else(Cell::empty, Cell::number);
    }
    assert_eq!(mv.variables[2].ordered_levels().first().map(|s| s.level.as_str()), Some("Control"), "ArmA must be a non-default reference level");

    // Retained complete-case design-column means must match R's colMeans(model.matrix(fit_efron)).
    let retained_indices: Vec<usize> = (0..n).filter(|&r| biomarkers[r].is_some()).collect();
    assert_eq!(retained_indices.len(), 18);
    let retained_n = retained_indices.len() as f64;
    let mean_ctrl = retained_indices.iter().filter(|&&r| regimens[r] == "Control").count() as f64 / retained_n;
    let mean_arm_b = retained_indices.iter().filter(|&&r| regimens[r] == "ArmB").count() as f64 / retained_n;
    let mean_bio = retained_indices.iter().filter_map(|&r| biomarkers[r]).sum::<f64>() / retained_n;
    assert_approx_eq(mean_ctrl, 0.444_444_444_444_444_4, 1e-15, 0.0, "retained mean Control dummy");
    assert_approx_eq(mean_arm_b, 0.277_777_777_777_777_8, 1e-15, 0.0, "retained mean ArmB dummy");
    assert_approx_eq(mean_bio, 2.766_666_666_666_667, 1e-15, 0.0, "retained mean Biomarker");

    let sheet = DataSheet::new(SheetId::new("cox-oracle-data"), "Cox oracle data", TableArchetype::MultipleVariables(mv)).unwrap();

    let expected_cases = [
        (
            CoxTieMethod::Efron,
            14.505_499_947_552_174,
            0.002_291_923_640_368_326_5,
            [
                (
                    "Regimen[Control vs ArmA]",
                    -0.497_164_735_815_274_4,
                    0.787_498_902_340_960_9,
                    -0.631_321_179_416_728_3,
                    0.527_830_541_411_133_7,
                    0.608_252_774_540_668_1,
                    0.129_946_269_920_166_97,
                    2.847_110_871_006_257_6,
                ),
                (
                    "Regimen[ArmB vs ArmA]",
                    -0.166_291_546_117_151_6,
                    0.814_105_468_506_362,
                    -0.204_262_902_719_774_68,
                    0.838_148_058_113_851_3,
                    0.846_799_317_144_375_7,
                    0.171_716_753_083_520_22,
                    4.175_883_078_614_992_5,
                ),
                (
                    "Biomarker",
                    1.586_711_720_680_589_4,
                    0.489_403_233_103_871_35,
                    3.242_135_755_044_806_4,
                    0.001_186_374_948_244_553_2,
                    4.887_650_514_684_068,
                    1.872_920_561_044_802_8,
                    12.755_013_773_977_126,
                ),
            ],
        ),
        (
            CoxTieMethod::Breslow,
            13.302_397_787_014_883,
            0.004_026_263_641_592_193,
            [
                (
                    "Regimen[Control vs ArmA]",
                    -0.469_597_602_741_750_2,
                    0.784_533_028_496_279_3,
                    -0.598_569_576_658_654_7,
                    0.549_459_948_880_378_9,
                    0.625_253_818_089_939_7,
                    0.134_357_102_015_234_2,
                    2.909_725_881_045_872,
                ),
                (
                    "Regimen[ArmB vs ArmA]",
                    -0.165_238_733_387_394_9,
                    0.817_047_607_691_310_5,
                    -0.202_238_806_933_541_5,
                    0.839_730_036_337_582_4,
                    0.847_691_307_712_135_6,
                    0.170_909_240_939_914_3,
                    4.204_456_992_605_44,
                ),
                (
                    "Biomarker",
                    1.499_268_353_373_223_6,
                    0.483_856_293_211_412_1,
                    3.098_581_902_123_046,
                    0.001_944_492_157_225_825_4,
                    4.478_411_256_894_286,
                    1.734_861_260_350_881_5,
                    11.560_675_106_562_14,
                ),
            ],
        ),
        (
            CoxTieMethod::Exact,
            14.904_698_929_139_194,
            0.001_899_928_165_906_084_8,
            [
                (
                    "Regimen[Control vs ArmA]",
                    -0.468_062_038_840_436_84,
                    0.817_440_249_486_102,
                    -0.572_594_803_271_177_5,
                    0.566_919_079_164_947_8,
                    0.626_214_672_820_271_7,
                    0.126_158_593_195_251_5,
                    3.108_348_044_500_546_7,
                ),
                (
                    "Regimen[ArmB vs ArmA]",
                    -0.071_578_722_374_342_53,
                    0.845_216_193_623_312_7,
                    -0.084_686_880_012_905_9,
                    0.932_510_326_727_111_5,
                    0.930_922_990_242_100_6,
                    0.177_608_778_522_832_74,
                    4.879_362_500_935_644,
                ),
                (
                    "Biomarker",
                    1.775_766_486_858_686_4,
                    0.573_054_165_754_912_1,
                    3.098_775_984_848_453_3,
                    0.001_943_218_888_299_500_8,
                    5.904_805_357_929_266,
                    1.920_529_560_533_902_6,
                    18.154_745_978_154_75,
                ),
            ],
        ),
    ];

    let mut efron_result = None;
    for (tie_method, lr_chi2, lr_p, rows) in expected_cases {
        let spec = AnalysisSpec::new(
            SheetId::new(format!("cox-oracle-{tie_method:?}")),
            SheetId::new("cox-oracle-data"),
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2, 3],
                reference_levels: vec![(2, String::from("ArmA"))],
                tie_method,
            }),
        )
        .unwrap();
        let result = run_analysis(&sheet, &spec).unwrap();
        assert_eq!(result.status, ResultStatus::UpToDate);
        assert_approx_eq(scalar_val(&result, "Converged").unwrap(), 1.0, 0.0, 0.0, &format!("{tie_method:?} converged"));

        // Verify listwise exclusion of Row 8 with missing Biomarker (19 rows -> 18 retained).
        let alignment_chk = result
            .checklist
            .iter()
            .find(|c| c.assumption == "Complete-case row alignment")
            .unwrap_or_else(|| panic!("{tie_method:?}: missing listwise row alignment checklist entry"));
        assert_eq!(alignment_chk.status, ChecklistStatus::Info);
        assert_eq!(alignment_chk.diagnostic_metric.as_deref(), Some("1 row(s) omitted; 18 retained"));
        assert!(alignment_chk.explanation.contains("omitted 1 incomplete or excluded row(s) (18 complete row(s) retained)"));
        assert!(!result.checklist.iter().any(|c| c.assumption == "Numeric input validation"));

        let ph_tab = tab_by_id(&result, "cox-ph").unwrap();
        assert_eq!(ph_tab.kind, ResultTabKind::ParametersAndConfidenceIntervals);
        assert!(
            ph_tab.footnotes.iter().any(|f| f.contains("omitted 1 incomplete or excluded row(s) (18 complete row(s) retained)")),
            "{tie_method:?}: missing listwise footnote in {:?}",
            ph_tab.footnotes
        );
        assert_eq!(ph_tab.column_headers, vec!["beta", "SE", "Wald z", "P value", "Hazard Ratio", "95% CI (HR)", "Schoenfeld P"]);
        assert_eq!(ph_tab.row_labels, vec!["Regimen[Control vs ArmA]", "Regimen[ArmB vs ArmA]", "Biomarker"]);

        assert_approx_eq(
            scalar_val(&result, "Likelihood ratio chi2").unwrap(),
            lr_chi2,
            COX_ABS_TOL,
            COX_REL_TOL,
            &format!("{tie_method:?} LR chi2"),
        );
        assert_approx_eq(scalar_val(&result, "Likelihood ratio P").unwrap(), lr_p, COX_ABS_TOL, COX_REL_TOL, &format!("{tie_method:?} LR P"));

        assert_eq!(ph_tab.rows.len(), rows.len());
        for (i, (label, beta, se, z, p, hr, ci_lo, ci_hi)) in rows.into_iter().enumerate() {
            let row = &ph_tab.rows[i];
            let ctx = format!("{tie_method:?} {label}");
            assert_approx_eq(cell_num(&row[0], &ctx), beta, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} beta"));
            assert_approx_eq(cell_num(&row[1], &ctx), se, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} se"));
            assert_approx_eq(cell_num(&row[2], &ctx), z, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} wald z"));
            assert_approx_eq(cell_p_val(&row[3], &ctx), p, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} p"));
            assert_approx_eq(cell_num(&row[4], &ctx), hr, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} HR"));
            let (actual_lo, actual_hi) = cell_ci(&row[5], &ctx);
            assert_approx_eq(actual_lo, ci_lo, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} HR CI lo"));
            assert_approx_eq(actual_hi, ci_hi, COX_ABS_TOL, COX_REL_TOL, &format!("{ctx} HR CI hi"));
        }

        if tie_method == CoxTieMethod::Efron {
            efron_result = Some(result);
        }
    }

    // Efron baseline cumulative hazard H_0(t) and survival S_0(t) at retained design-column means.
    let efron = efron_result.unwrap();
    let base_tab = tab_by_id(&efron, "cox-baseline-survival").unwrap();
    assert_eq!(base_tab.kind, ResultTabKind::BaselineSurvival);
    assert_eq!(base_tab.column_headers, vec!["Time", "Cumulative Hazard H0(t)", "Baseline Survival S0(t)"]);
    assert!(base_tab.footnotes.iter().any(|f| f.contains("sample covariate means")));

    let expected_baseline = [
        (3.0, 0.021_802_430_556_094_803, 0.978_433_524_523_872_9),
        (5.0, 0.075_634_050_288_996_72, 0.927_155_436_749_134_9),
        (8.0, 0.108_001_237_253_408_24, 0.897_626_485_838_318_8),
        (10.0, 0.198_517_717_164_567_04, 0.819_945_243_506_899_4),
        (12.0, 0.324_798_867_401_418_67, 0.722_672_692_061_913_3),
        (15.0, 0.758_845_505_220_401_5, 0.468_206_657_244_974_5),
        (17.0, 1.342_213_487_435_395_3, 0.261_266_717_469_797_35),
        (22.0, 2.037_472_973_220_324, 0.130_357_712_435_735_5),
        (24.0, 2.900_522_706_482_091_5, 0.054_994_466_578_087_61),
        (28.0, 6.880_351_176_384_446, 0.001_027_783_048_656_692),
    ];

    assert_eq!(base_tab.rows.len(), expected_baseline.len() + 1);
    assert_approx_eq(cell_num(&base_tab.rows[0][0], "t0"), 0.0, 0.0, 0.0, "baseline t=0");
    assert_approx_eq(cell_num(&base_tab.rows[0][1], "H0(0)"), 0.0, 0.0, 0.0, "baseline H0(0)");
    assert_approx_eq(cell_num(&base_tab.rows[0][2], "S0(0)"), 1.0, 0.0, 0.0, "baseline S0(0)");

    let base_curve =
        efron.fitted_curves.iter().find(|c| c.equation_id == "cox_baseline" && c.data_set_index == 0).expect("missing cox_baseline overlay");
    assert_eq!(base_curve.points.len(), expected_baseline.len() + 1);
    assert_approx_eq(base_curve.points[0].x, 0.0, 0.0, 0.0, "baseline curve t=0");
    assert_approx_eq(base_curve.points[0].y, 1.0, 0.0, 0.0, "baseline curve S0(0)");

    for (idx, &(t, h0, s0)) in expected_baseline.iter().enumerate() {
        let row = &base_tab.rows[idx + 1];
        assert_approx_eq(cell_num(&row[0], "time"), t, 0.0, 0.0, &format!("baseline time[{idx}]"));
        assert_approx_eq(cell_num(&row[1], "H0"), h0, COX_BASELINE_ABS_TOL, COX_BASELINE_REL_TOL, &format!("H0(t={t})"));
        assert_approx_eq(cell_num(&row[2], "S0"), s0, COX_BASELINE_ABS_TOL, COX_BASELINE_REL_TOL, &format!("S0(t={t})"));
        assert_approx_eq(base_curve.points[idx + 1].x, t, 0.0, 0.0, &format!("baseline curve time[{idx}]"));
        assert_approx_eq(base_curve.points[idx + 1].y, s0, COX_BASELINE_ABS_TOL, COX_BASELINE_REL_TOL, &format!("baseline curve S0(t={t})"));
    }
}
