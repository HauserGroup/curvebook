#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! QOL-7 / R9.7 Diagnostics-Only Assumption Evaluation checks.
//!
//! Ensures every `ChecklistItem` emitted across all statistical analyses sets `remediation: None`
//! and reports purely diagnostic metrics, statistical limitations, and tab references without
//! recommending alternative tests or data transformations.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, ContingencySpec, ContingencyTable,
    CoxTieMethod, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec, MultipleVariablesTable, MultiplicitySpec, MultivariateSpec, OneWayAnovaSpec,
    OneWayMethod, OutlierSpec, RocSpec, SheetId, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use curvebook_stats::run_analysis;

const FORBIDDEN_RECOMMENDATION_PHRASES: &[&str] = &[
    "use mann-whitney",
    "use mann–whitney",
    "use wilcoxon",
    "use welch",
    "consider welch",
    "consider kruskal",
    "use kruskal",
    "use friedman",
    "consider friedman",
    "consider fisher",
    "prefer fisher",
    "try spearman",
    "apply log",
    "log transform",
    "remove outliers",
    "is recommended",
    "consider enabling",
    "consider brown-forsythe",
    "warrants larger",
];

fn assert_diagnostics_only_checklist(sheet: &DataSheet, spec: &AnalysisSpec, context: &str) {
    let res = run_analysis(sheet, spec).unwrap_or_else(|e| panic!("{context} failed: {e:?}"));
    for item in &res.checklist {
        assert!(item.remediation.is_none(), "{context}: expected remediation: None on {:?}, got {:?}", item.assumption, item.remediation);
        let lower = item.explanation.to_ascii_lowercase();
        for forbidden in FORBIDDEN_RECOMMENDATION_PHRASES {
            assert!(
                !lower.contains(forbidden),
                "{context}: checklist item {:?} contains forbidden recommendation phrase {:?} in {:?}",
                item.assumption,
                forbidden,
                item.explanation
            );
        }
    }
}

fn column_sheet(cols: &[(&str, &[f64])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("diag-col"), "Diagnostic Columns", TableArchetype::Column(tab)).unwrap()
}

#[test]
fn all_checklists_emit_no_remediation_and_no_test_recommendations() {
    // 1. Two-group strongly skewed & unequal-variance raw samples (hits normality & F-test warns + small sample)
    let skewed_col =
        column_sheet(&[("Ctrl", &[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 50.0]), ("Treat", &[10.0, 10.2, 10.5, 10.8, 11.0, 11.2, 11.4, 250.0])]);
    let small_col = column_sheet(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);

    for (name, sheet, method) in [
        ("UnpairedT skewed", &skewed_col, TwoGroupMethod::UnpairedT),
        ("WelchT skewed", &skewed_col, TwoGroupMethod::WelchT),
        ("PairedT skewed", &skewed_col, TwoGroupMethod::PairedT),
        ("OneSampleT skewed", &skewed_col, TwoGroupMethod::OneSampleT { hypothetical: 0.0 }),
        ("PairedSignTest", &skewed_col, TwoGroupMethod::PairedSignTest),
        ("UnpairedT small n=3", &small_col, TwoGroupMethod::UnpairedT),
    ] {
        let spec =
            AnalysisSpec::new(SheetId::new("r"), sheet.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }))
                .unwrap();
        assert_diagnostics_only_checklist(sheet, &spec, name);
    }

    // 2. Column descriptive & outlier checklists
    let desc_spec =
        AnalysisSpec::new(SheetId::new("r-desc"), skewed_col.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    assert_diagnostics_only_checklist(&skewed_col, &desc_spec, "DescriptiveStats");

    let out_spec =
        AnalysisSpec::new(SheetId::new("r-out"), skewed_col.id.clone(), AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }))
            .unwrap();
    assert_diagnostics_only_checklist(&skewed_col, &out_spec, "OutlierDetection");

    // 3. One-way ANOVA (Ordinary, Welch, RepeatedMeasures with non-spherical data)
    let anova_sheet = column_sheet(&[
        ("G1", &[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 12.0]),
        ("G2", &[2.0, 5.0, 9.0, 14.0, 22.0, 35.0, 80.0]),
        ("G3", &[3.0, 3.2, 3.3, 3.5, 3.6, 3.9, 4.0]),
    ]);
    for (name, method) in [
        ("OneWay Ordinary", OneWayMethod::OrdinaryAnova),
        ("OneWay Welch", OneWayMethod::BrownForsytheAndWelch),
        ("OneWay RM", OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }),
    ] {
        let spec = AnalysisSpec::new(
            SheetId::new("r-aov"),
            anova_sheet.id.clone(),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method,
                multiplicity: MultiplicitySpec::default(),
                compute_effect_sizes: true,
                ..OneWayAnovaSpec::default()
            }),
        )
        .unwrap();
        assert_diagnostics_only_checklist(&anova_sheet, &spec, name);
    }

    // 4. Contingency with sparse expected counts (< 5)
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].cells[0][0] = Cell::number(1.0);
    ct.data_sets[1].cells[0][0] = Cell::number(2.0);
    ct.data_sets[0].cells[1][0] = Cell::number(3.0);
    ct.data_sets[1].cells[1][0] = Cell::number(4.0);
    let ct_sheet = DataSheet::new(SheetId::new("ct-small"), "Sparse 2x2", TableArchetype::Contingency(ct)).unwrap();
    let ct_spec = AnalysisSpec::new(SheetId::new("r-ct"), SheetId::new("ct-small"), AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    assert_diagnostics_only_checklist(&ct_sheet, &ct_spec, "Contingency sparse");

    // 5. Logistic & Poisson GLM
    let mut mv = MultipleVariablesTable::new(2, 8).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Y");
    for (r, (&x, &y)) in [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0].iter().zip([0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0].iter()).enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
    }
    let sep_sheet = DataSheet::new(SheetId::new("mv-sep"), "Sep", TableArchetype::MultipleVariables(mv)).unwrap();
    let log_spec = AnalysisSpec::new(
        SheetId::new("r-log"),
        SheetId::new("mv-sep"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 1,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    assert_diagnostics_only_checklist(&sep_sheet, &log_spec, "GLM Logistic separated");

    let mut mv_pois = MultipleVariablesTable::new(2, 8).unwrap();
    mv_pois.variables[0].title = String::from("X");
    mv_pois.variables[1].title = String::from("Y");
    for (r, &y) in [0.0, 1.0, 0.0, 12.0, 1.0, 25.0, 2.0, 40.0].iter().enumerate() {
        mv_pois.variables[0].cells[r] = Cell::number(r as f64);
        mv_pois.variables[1].cells[r] = Cell::number(y);
    }
    let pois_sheet = DataSheet::new(SheetId::new("mv-pois"), "Pois", TableArchetype::MultipleVariables(mv_pois)).unwrap();
    let pois_spec = AnalysisSpec::new(
        SheetId::new("r-pois"),
        SheetId::new("mv-pois"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Poisson,
            response_column: 1,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    assert_diagnostics_only_checklist(&pois_sheet, &pois_spec, "GLM Poisson overdispersed");

    // 6. Cox PH Survival
    let mut mv_cox = MultipleVariablesTable::new(3, 8).unwrap();
    mv_cox.variables[0].title = String::from("Time");
    mv_cox.variables[1].title = String::from("Status");
    mv_cox.variables[2].title = String::from("Covariate");
    for (r, ((&t, &e), &x)) in [2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0]
        .iter()
        .zip([1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0].iter())
        .zip([0.2, 0.4, 0.3, 0.8, 0.9, 0.5, 1.2, 1.4].iter())
        .enumerate()
    {
        mv_cox.variables[0].cells[r] = Cell::number(t);
        mv_cox.variables[1].cells[r] = Cell::number(e);
        mv_cox.variables[2].cells[r] = Cell::number(x);
    }
    let cox_sheet = DataSheet::new(SheetId::new("mv-cox"), "Cox", TableArchetype::MultipleVariables(mv_cox)).unwrap();
    let cox_spec = AnalysisSpec::new(
        SheetId::new("r-cox"),
        SheetId::new("mv-cox"),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert_diagnostics_only_checklist(&cox_sheet, &cox_spec, "Cox PH");

    // 7. Multivariate Clustering + ROC (AUC < 0.5)
    let mut mv_clust = MultipleVariablesTable::new(2, 5).unwrap();
    for r in 0..5 {
        mv_clust.variables[0].cells[r] = Cell::number(r as f64);
        mv_clust.variables[1].cells[r] = Cell::number((r * r) as f64);
    }
    let clust_sheet = DataSheet::new(SheetId::new("mv-cl"), "Cluster", TableArchetype::MultipleVariables(mv_clust)).unwrap();
    let cl_spec = AnalysisSpec::new(
        SheetId::new("r-cl"),
        SheetId::new("mv-cl"),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(3),
        }),
    )
    .unwrap();
    assert_diagnostics_only_checklist(&clust_sheet, &cl_spec, "Clustering");

    let roc_sheet = column_sheet(&[("Ctrl", &[10.0, 11.0, 12.0, 13.0]), ("Case", &[1.0, 2.0, 3.0, 4.0])]);
    let roc_spec = AnalysisSpec::new(SheetId::new("r-roc"), SheetId::new("diag-col"), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
    assert_diagnostics_only_checklist(&roc_sheet, &roc_spec, "ROC AUC < 0.5");
}
