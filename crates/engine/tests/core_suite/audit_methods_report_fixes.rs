//! Permanent Wave 3 audit regression tests for `crates/engine/src/methods_report.rs`
//! covering findings #81, #82, #83, #102, #105, #106, #107, #108, #109, #110, #111, #112, #113, #114.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, ColumnDesignHint, ConfidenceLevel, ContingencyMethod, ContingencySpec, DataSheet, FactorialAnovaSpec,
    InfoSheet, MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NestedTable, NonlinearFitSpec,
    OneWayAnovaSpec, OneWayMethod, PValueFormat, ParameterRule, ParameterScope, PassingBablokSpec, PowerAnalysisSpec, PowerEffectSpec,
    PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultScalarEntry,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SurvivalAnalysisSpec, TableArchetype, TwoGroupSpec,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{
    TempDir, col_sheet, col_sheet_hint, contingency_sheet, grouped_rep_sheet, ok_cmd, rs, run_cmds, session_with, survival_sheet, xy_sheet,
};

fn find_tool_on_path(bin: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH")
        && let Some(p) = std::env::split_paths(&path_var).map(|d| d.join(bin)).find(|c| c.is_file())
    {
        return Some(p);
    }
    let texbin = Path::new("/Library/TeX/texbin").join(bin);
    texbin.is_file().then_some(texbin)
}

fn assert_clean_room_prose(text: &str) {
    let lower = text.to_ascii_lowercase();
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!lower.contains(forbidden), "forbidden brand token `{forbidden}` in report:\n{text}");
    }
}

fn seed_three_group_column(id: &str) -> DataSheet {
    col_sheet(
        id,
        "ThreeGroup",
        &[("Ctrl", &[10.0, 11.0, 12.0, 9.5, 10.5]), ("Low", &[14.0, 15.0, 14.5, 16.0, 15.5]), ("High", &[22.0, 23.0, 21.5, 24.0, 22.5])],
    )
}

fn seed_rm_three_group_column(id: &str) -> DataSheet {
    col_sheet_hint(
        id,
        "RmCol",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("Baseline", &[10.0, 11.2, 9.8, 10.6, 12.1]), ("DoseA", &[13.0, 14.8, 12.6, 13.9, 15.4]), ("DoseB", &[19.0, 20.5, 18.4, 19.7, 21.8])],
    )
}

fn rep_txt(doc: &ProjectDocument, id: &str) -> String {
    generate_methods_report(doc, &SheetId::new(id), ReportFormat::PlainText).unwrap()
}

#[test]
fn audit_105_108_latex_structural_and_unicode_infinity_compile() {
    let ct = contingency_sheet("ct-nnt", "NNT Infinity", &["Event", "NoEvent"], &["Arm A", "Arm B"], &[&[10.0, 12.0], &[11.0, 10.0]]);
    let mut session = session_with("TeX Compile Audit", vec![seed_three_group_column("col-3"), ct]);

    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-3", "sheet_id": "res-aov", "kind": "one_way_anova", "multiplicity": "tukey" }));
    let nnt_spec = AnalysisSpec::new(
        "res-nnt",
        "ct-nnt",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            ..ContingencySpec::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": nnt_spec }));

    let mut synth_doc = session.document().clone();
    let mut synth_res =
        ResultSheet::new(AnalysisSpec::new("res-synth", "col-3", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    synth_res.status = ResultStatus::UpToDate;
    synth_res.tabs.push(ResultTab {
        id: String::from("sym-tab"),
        title: String::from("Unicode & Bracket Safety"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Estimate"), String::from("Interval")],
        row_labels: vec![String::from("[Agonist]"), String::from(" [-inf, 2.5]"), String::from("Stats Symbols")],
        rows: vec![
            vec![
                ResultCell::Number { value: 1.25, prefix: None, units: None },
                ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.5) },
            ],
            vec![
                ResultCell::Number { value: 3.5, prefix: None, units: None },
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.0), upper: CiBound::Open },
            ],
            vec![
                ResultCell::Text(String::from("R² = 0.99; θ = 90°; κ = 0.8; ν = 4; Σ, Ω")),
                ResultCell::Text(String::from("x ≠ y; a ≈ b; u − v; c · d; e ÷ f; ∞")),
            ],
        ],
        footnotes: vec![String::from("Leading [bracket] row and ∞ symbol check.")],
    });
    synth_doc.add_result_sheet(synth_res).unwrap();

    let tex_aov = generate_methods_report(&synth_doc, &SheetId::new("res-aov"), ReportFormat::Latex).unwrap();
    let tex_nnt = generate_methods_report(&synth_doc, &SheetId::new("res-nnt"), ReportFormat::Latex).unwrap();
    let tex_synth = generate_methods_report(&synth_doc, &SheetId::new("res-synth"), ReportFormat::Latex).unwrap();

    for tex in [&tex_aov, &tex_nnt, &tex_synth] {
        assert!(!tex.contains("\\\\\\relax") && tex.contains(" \\\\\n\\hline") && !tex.contains('∞') && !tex.contains('²') && !tex.contains('°'));
        assert_eq!(tex.matches("\\begin{tabular}").count(), tex.matches("\\end{tabular}").count());
        assert_eq!(tex.matches("\\begin{itemize}").count(), tex.matches("\\end{itemize}").count());
        assert_clean_room_prose(tex);
    }
    assert!(
        tex_nnt.contains("$\\infty$")
            && tex_synth.contains("{[}Agonist]")
            && tex_synth.contains(" {[}-inf, 2.5]")
            && tex_synth.contains("\\textsuperscript{2}")
            && tex_synth.contains("$\\infty$")
    );

    let scratch = TempDir::new("tex-compile");
    let tex_path = scratch.child("report.tex");
    fs::write(&tex_path, format!("\\documentclass{{article}}\n\\begin{{document}}\n{tex_aov}\n{tex_nnt}\n{tex_synth}\n\\end{{document}}\n")).unwrap();
    for engine in ["pdflatex", "lualatex"] {
        let Some(bin) = find_tool_on_path(engine) else { continue };
        let mut cmd = Command::new(&bin);
        cmd.arg("-interaction=nonstopmode").arg("-halt-on-error").arg(format!("-output-directory={}", scratch.path.display())).arg(&tex_path);
        if engine == "lualatex" {
            let cache_dir = scratch.child("texmf-var");
            fs::create_dir_all(&cache_dir).unwrap();
            cmd.env("TEXMFVAR", &cache_dir).env("TEXMFCACHE", &cache_dir).env("HOME", &scratch.path);
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{engine} failed:\n{}", String::from_utf8_lossy(&out.stdout));
        let pdf_path = scratch.child("report.pdf");
        assert!(fs::metadata(&pdf_path).is_ok_and(|m| m.len() > 0));
        let _ = fs::remove_file(&pdf_path);
    }
}

#[test]
fn audit_106_107_markdown_latex_escaping_newlines_and_bold_labels() {
    let ds = col_sheet(
        "col-xss",
        "Data `ticks` ~tilde~ [link](url)\nSecondLine",
        &[("Dose | [uM] <b\n> & *10*", &[10.0, 11.0, 12.0, 13.0]), ("Resp `tick` ~wave~ #tag", &[15.0, 16.0, 17.0, 18.0])],
    );
    let mut session = session_with("Proj *star* _em_ <img src=x onerror=alert(1)> & #1\n\n# FakeProj", vec![ds]);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-xss", "sheet_id": "res-xss", "kind": "unpaired_t" }));

    let mut dirty_doc = session.document().clone();
    dirty_doc.result_sheet_mut(&SheetId::new("res-xss")).unwrap().analysis.custom_title =
        Some(String::from("Res\n\n<script>alert(1)</script>\n# Injected heading\n---"));
    let md = generate_methods_report(&dirty_doc, &SheetId::new("res-xss"), ReportFormat::Markdown).unwrap();
    let tex = generate_methods_report(&dirty_doc, &SheetId::new("res-xss"), ReportFormat::Latex).unwrap();

    assert!(md.contains("\\<script\\>") && !md.contains(" <script>") && md.contains("\\<img") && !md.contains(" <img"));
    assert!(!md.lines().any(|l| l.starts_with("# Injected") || l.trim() == "---") && md.contains("\\*star\\*") && md.contains("\\_em\\_"));
    assert!(!md.contains(" :**") && !md.contains(": **"));
    for expected in [
        "**Project:** ",
        "**Analysis sheet:** ",
        "**Procedure:** ",
        "**Recalculation status:** ",
        "**Source table:** ",
        "**Observation exclusions:** ",
        "**Key scalar outputs:**",
    ] {
        assert!(md.contains(expected), "missing `{expected}`");
    }
    for line in tex.lines().filter(|l| l.starts_with("\\section*{") || l.starts_with("\\subsection*{") || l.starts_with("\\paragraph*{")) {
        assert!(line.ends_with('}'), "multi-line LaTeX heading: {line}");
    }

    if let Some(pandoc) = find_tool_on_path("pandoc") {
        use std::io::Write as _;
        for fmt in ["commonmark", "gfm"] {
            let mut child = Command::new(&pandoc)
                .args(["-f", fmt, "-t", "html"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.as_mut().unwrap().write_all(md.as_bytes()).unwrap();
            let rendered = child.wait_with_output().unwrap();
            let html = String::from_utf8_lossy(&rendered.stdout);
            assert!(
                rendered.status.success()
                    && !html.contains("<script")
                    && !html.contains("<img")
                    && !html.contains("<em>star</em>")
                    && html.contains("<strong>Project:</strong>")
                    && html.contains("<strong>Analysis sheet:</strong>")
            );
        }
    }
    if let Some(pdflatex) = find_tool_on_path("pdflatex") {
        let scratch = TempDir::new("tex-xss");
        let tex_path = scratch.child("xss.tex");
        fs::write(&tex_path, format!("\\documentclass{{article}}\n\\begin{{document}}\n{tex}\n\\end{{document}}\n")).unwrap();
        let out = Command::new(&pdflatex)
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg(format!("-output-directory={}", scratch.path.display()))
            .arg(&tex_path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
}

#[test]
fn audit_83_survival_multiplicity_plans() {
    let srv3 = survival_sheet(
        "srv-3",
        "Surv3",
        &["Ctrl", "LowDose", "HighDose"],
        &[
            (5.0, &[Some(1.0), Some(1.0), Some(0.0)]),
            (10.0, &[Some(1.0), Some(0.0), Some(0.0)]),
            (15.0, &[Some(1.0), Some(1.0), Some(0.0)]),
            (20.0, &[Some(1.0), Some(0.0), Some(1.0)]),
        ],
    );
    let srv2 = survival_sheet(
        "srv-2",
        "Surv2",
        &["Ctrl", "Drug"],
        &[(4.0, &[Some(1.0), Some(0.0)]), (8.0, &[Some(1.0), Some(1.0)]), (12.0, &[Some(1.0), Some(1.0)]), (16.0, &[Some(1.0), Some(1.0)])],
    );
    let mut session = session_with("Survival Plans", vec![srv3, srv2]);

    for (sid, src, mult) in [
        ("km-tuk", "srv-3", MultiplicityMethod::TukeyHsd),
        ("km-dun", "srv-3", MultiplicityMethod::Dunnett { control_column: 0 }),
        ("km-lsd", "srv-3", MultiplicityMethod::FisherLsd),
        ("km-2", "srv-2", MultiplicityMethod::TukeyHsd),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            src,
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec { method: mult, ..Default::default() },
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
    }
    let methods_of = |id: &str| rep_txt(session.document(), id).split("Stored Results Summary").next().unwrap().to_owned();
    assert!(methods_of("km-tuk").contains("Bonferroni FWER adjustment") && !methods_of("km-tuk").contains("Tukey"));
    assert!(
        methods_of("km-dun").contains("against control cohort Ctrl")
            && methods_of("km-dun").contains("Bonferroni FWER adjustment")
            && !methods_of("km-dun").contains("Dunnett")
    );
    assert!(rep_txt(session.document(), "km-lsd").contains("unadjusted Fisher's LSD P values"));
    assert!(methods_of("km-2").contains("pairwise log-rank post-hoc comparisons were not computed") && !methods_of("km-2").contains("Tukey"));
}

#[test]
fn audit_81_83_nested_welch_friedman_and_row_t_plans() {
    let mut nest = NestedTable::new(3, 3, 3).unwrap();
    for g in 0..3 {
        nest.groups[g].title = format!("Grp{}", g + 1);
        for u in 0..3 {
            for r in 0..3 {
                nest.groups[g].cells[u][r] = Cell::number(10.0 + (g as f64) * 4.0 + (u as f64) * 0.5 + (r as f64) * 0.1);
            }
        }
    }
    let grp_mr = grouped_rep_sheet(
        "grp-mr",
        "MultiRow",
        &["Gene_1", "Gene_2", "Gene_3", "Gene_4"],
        &[
            ("Ctrl", &[&[10.0, 10.2, 10.4], &[10.0, 10.2, 10.4], &[10.0, 10.2, 10.4], &[10.0, 10.2, 10.4]]),
            ("Treated", &[&[13.0, 13.2, 13.4], &[14.0, 14.2, 14.4], &[15.0, 15.2, 15.4], &[16.0, 16.2, 16.4]]),
        ],
    );
    let mut session = session_with(
        "Effective Plans",
        vec![
            DataSheet::new("nest-3", "Nested3", TableArchetype::Nested(nest)).unwrap(),
            seed_three_group_column("col-3"),
            seed_rm_three_group_column("rm-3"),
            grp_mr,
        ],
    );

    for (sid, two_group, expected) in
        [("nest-t", true, "Post-hoc pairwise comparisons were not computed"), ("nest-a2", false, "fewer than three groups were analyzed")]
    {
        let mut spec = AnalysisSpec::new(
            sid,
            "nest-3",
            AnalysisKind::NestedComparison(NestedAnalysisSpec {
                two_group_t_test: two_group,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
                ..Default::default()
            }),
        )
        .unwrap();
        spec.selected_data_sets = vec![0, 1];
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
        let r = rep_txt(session.document(), sid);
        assert!(r.contains(expected) && !r.contains("Tukey"), "{sid}:\n{r}");
    }

    for (sid, mult, expected, forbidden) in [
        ("bf-none", MultiplicityMethod::None, "not requested (none computed)", "Games-Howell"),
        ("bf-bonf", MultiplicityMethod::Bonferroni, "Welch unequal-variance t tests", "Games-Howell"),
        ("bf-dun", MultiplicityMethod::Dunnett { control_column: 0 }, "heteroscedastic Dunnett loadings", "Games-Howell"),
        ("bf-trend", MultiplicityMethod::LinearTrend, "heteroscedastic polynomial test", "Tukey"),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            "col-3",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::BrownForsytheAndWelch,
                multiplicity: MultiplicitySpec { method: mult, ..Default::default() },
                ..Default::default()
            }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
        let r = rep_txt(session.document(), sid);
        assert!(r.contains(expected) && !r.contains(forbidden), "{sid}:\n{r}");
    }

    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "rm-3", "sheet_id": "fr-dunn", "kind": "friedman", "multiplicity": "dunn" }));
    let fr = rep_txt(session.document(), "fr-dunn");
    assert!(fr.contains("within-block mean rank pairwise z-tests") && fr.contains("Bonferroni"));

    for (sid, mult, expected) in [
        ("mr-bh", MultiplicityMethod::BenjaminiHochbergFdr, "Benjamini-Hochberg"),
        ("mr-lsd", MultiplicityMethod::FisherLsd, "Fisher's unprotected LSD; no multiplicity correction"),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            "grp-mr",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                multiplicity: MultiplicitySpec { method: mult, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
        let r = rep_txt(session.document(), sid);
        assert!(r.contains(expected) && !r.contains("multiplicity-adjusted confidence intervals"));
    }

    let mut legacy_doc = session.document().clone();
    let mut legacy_mr = ResultSheet::new(
        AnalysisSpec::new(
            "mr-legacy-tuk",
            "grp-mr",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            }),
        )
        .unwrap(),
    )
    .unwrap();
    legacy_mr.status = ResultStatus::UpToDate;
    legacy_doc.add_result_sheet(legacy_mr).unwrap();
    let lrep = rep_txt(&legacy_doc, "mr-legacy-tuk");
    assert!(lrep.contains("Bonferroni FWER correction") && !lrep.contains("Tukey"));
}

#[test]
fn audit_82_alpha_aware_stars_and_legend() {
    let col2 = col_sheet("col-2", "TwoGroup", &[("Ctrl", &[10.0, 10.5, 11.0, 9.5, 10.2]), ("Treated", &[11.0, 11.5, 12.0, 10.5, 11.2])]);
    let col3p = col_sheet(
        "col-3p",
        "ThreeGroupP",
        &[("A", &[10.0, 11.0, 12.0, 9.0, 10.5]), ("B", &[10.9, 12.0, 12.8, 10.2, 11.5]), ("C", &[11.9, 13.0, 13.7, 11.3, 12.5])],
    );
    let mut session = session_with("Alpha Stars", vec![col2, col3p]);

    let mut spec_t = AnalysisSpec::new("res-t99", "col-2", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    spec_t.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    let mut spec_aov = AnalysisSpec::new(
        "res-aov99",
        "col-3p",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::None, threshold: 0.01, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    spec_aov.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    run_cmds(&mut session, &[("analysis.run", json!({ "spec": spec_t })), ("analysis.run", json!({ "spec": spec_aov }))]);

    for id in ["res-t99", "res-aov99"] {
        let rep = rep_txt(session.document(), id);
        assert!(
            rep.contains("alpha = 0.01")
                && rep.contains("p <= alpha")
                && rep.contains("ns for p > 0.01, * for p <= 0.01, ** for p <= 0.002, *** for p <= 0.0002, **** for p <= 0.00002")
        );
    }
    let t_p = rs(&session, "res-t99").lookup_scalar("PValue", None).unwrap();
    let t_rep = rep_txt(session.document(), "res-t99");
    assert!(t_p > 0.01 && t_p < 0.05 && t_rep.contains("(ns)") && !t_rep.contains("(*)"));

    let p_val = rs(&session, "res-aov99").lookup_scalar("P value", None).unwrap();
    let aov_rep = rep_txt(session.document(), "res-aov99");
    assert!(
        p_val > 0.01
            && p_val < 0.05
            && aov_rep.contains("0.0309 (ns)")
            && !aov_rep.contains("0.0309 (*)")
            && aov_rep.contains("0.0097 (*)")
            && !aov_rep.contains("0.0097 (**)")
    );

    let mut boundary_doc = session.document().clone();
    let mut b_spec = AnalysisSpec::new("res-apa99", "col-2", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    b_spec.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    b_spec.p_value_format = PValueFormat::Apa;
    let mut b_res = ResultSheet::new(b_spec).unwrap();
    b_res.status = ResultStatus::UpToDate;
    b_res.tabs.push(ResultTab {
        id: String::from("b-tab"),
        title: String::from("Boundary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("P value")],
        row_labels: vec![String::from("Exact boundary"), String::from("Above alpha")],
        rows: vec![
            vec![ResultCell::PValue { raw: 0.01, formatted: String::from(".010"), stars: String::from("ns") }],
            vec![ResultCell::PValue { raw: 0.0369, formatted: String::from(".037"), stars: String::from("*") }],
        ],
        footnotes: Vec::new(),
    });
    boundary_doc.add_result_sheet(b_res).unwrap();
    let b_rep = rep_txt(&boundary_doc, "res-apa99");
    assert!(!b_rep.contains("****") && b_rep.contains(".010 (*)") && b_rep.contains(".037 (ns)"));
}

#[test]
fn audit_109_110_factorial_rm_estimator_and_effect_sizes() {
    let grp = grouped_rep_sheet(
        "grp-rm",
        "Rm2Way",
        &["Pre", "Post"],
        &[
            ("T0", &[&[10.0, 10.5, 11.0], &[13.0, 13.5, 14.0]]),
            ("T1", &[&[12.0, 12.5, 13.0], &[15.0, 15.5, 16.0]]),
            ("T2", &[&[14.0, 14.5, 15.0], &[17.0, 17.5, 18.0]]),
        ],
    );
    let mut session = session_with("Factorial RM Estimator", vec![grp]);

    let rm_spec = AnalysisSpec::new(
        "res-rm-complete",
        "grp-rm",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            fit_reml_on_missing: true,
            compute_effect_sizes: true,
            ..Default::default()
        }),
    )
    .unwrap();
    let un_spec = AnalysisSpec::new(
        "res-rm-un",
        "grp-rm",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            residual_covariance: ResidualCovariance::Unstructured,
            compute_effect_sizes: true,
            ..Default::default()
        }),
    )
    .unwrap();
    run_cmds(&mut session, &[("analysis.run", json!({ "spec": rm_spec })), ("analysis.run", json!({ "spec": un_spec }))]);

    let rep_c = rep_txt(session.document(), "res-rm-complete");
    assert!(
        rep_c.contains("classical balanced repeated-measures ANOVA (OLS)")
            && !rep_c.contains("REML")
            && rep_c.contains("Partial eta-squared and eta-squared effect sizes were calculated")
            && !rep_c.contains("omega-squared")
    );

    let rep_u = rep_txt(session.document(), "res-rm-un");
    assert!(
        rep_u.contains("unstructured residual covariance matrix")
            && rep_u.contains("Partial eta-squared effect sizes were calculated")
            && !rep_u.contains("Partial eta-squared and eta-squared")
            && !rep_u.contains("omega-squared")
    );

    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "grp-rm", "cells": [{ "row": 0, "col": 1, "subcol": 2 }] }));
    let rep_r = rep_txt(session.document(), "res-rm-complete");
    assert!(
        rep_r.contains("compound-symmetry")
            && rep_r.contains("REML")
            && !rep_r.contains("classical balanced")
            && rep_r.contains("Partial eta-squared effect sizes were calculated")
            && !rep_r.contains("Partial eta-squared and eta-squared")
            && !rep_r.contains("omega-squared")
    );
}

#[test]
fn audit_111_post_test_confidence_intervals() {
    let mut session = session_with("PostTest CIs", vec![seed_three_group_column("col-3")]);
    for (sid, mult, need, ban) in [
        ("aov-hs", MultiplicityMethod::HolmSidak, None, Some("confidence intervals were")),
        ("aov-bh", MultiplicityMethod::BenjaminiHochbergFdr, None, Some("confidence intervals")),
        ("aov-nk", MultiplicityMethod::NewmanKeuls, None, Some("confidence intervals")),
        ("aov-lsd", MultiplicityMethod::FisherLsd, Some("with unadjusted confidence intervals"), Some("multiplicity-adjusted confidence intervals")),
        ("aov-tuk", MultiplicityMethod::TukeyHsd, Some("with multiplicity-adjusted confidence intervals"), None),
        ("aov-bonf", MultiplicityMethod::Bonferroni, Some("with multiplicity-adjusted confidence intervals"), None),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            "col-3",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::OrdinaryAnova,
                multiplicity: MultiplicitySpec { method: mult, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
        let rep = rep_txt(session.document(), sid);
        let mc_line = rep.lines().find(|l| l.contains("Multiple comparisons")).unwrap();
        if let Some(n) = need {
            assert!(mc_line.contains(n), "{sid}: {mc_line}");
        }
        if let Some(b) = ban {
            assert!(!mc_line.contains(b), "{sid}: {mc_line}");
        }
    }
}

#[test]
fn audit_112_power_analysis_effective_info_values() {
    let mut doc = ProjectDocument::new("Power Info Effective").unwrap();
    doc.add_data_sheet(seed_three_group_column("col-src")).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-pwr", "Power Constants").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    let p_spec = PowerAnalysisSpec {
        family: PowerTestFamily::TwoSampleUnpairedT,
        solve_target: PowerSolveTarget::SampleSize,
        tail_mode: PowerTailMode::TwoSided,
        alpha: 0.05,
        target_power: Some(0.80),
        effect: PowerEffectSpec::Standardized(0.5),
        info_sheet_id: Some(SheetId::new("info-pwr")),
        ..PowerAnalysisSpec::default()
    };
    run_cmds(
        &mut session,
        &[
            ("info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Alpha", "value": 0.01, "kind": "number" })),
            ("info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Target Power", "value": 0.95, "kind": "number" })),
            ("info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Effect Size", "value": 0.5, "kind": "number" })),
            ("analysis.run", json!({ "source_sheet_id": "col-src", "sheet_id": "res-pwr-info", "kind": { "PowerAnalysis": p_spec } })),
        ],
    );
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let rep = generate_methods_report(session.document(), &SheetId::new("res-pwr-info"), fmt).unwrap();
        assert!(
            rep.contains("0.01")
                && rep.contains("99")
                && rep.contains("target power = 0.95")
                && !rep.contains("alpha = 0.05")
                && !rep.contains("target power = 0.8")
        );
    }
}

#[test]
fn audit_113_114_mi_settings_and_nonlinear_rules_and_102_pb() {
    let xy = xy_sheet(
        "xy-pb",
        "PB",
        "X",
        &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0],
        &[("Method A", &[13.0, 24.0, 35.0, 46.0, 57.0, 68.0]), ("Method B", &[8.5, 18.0, 27.5, 37.0, 46.5, 56.0])],
    );
    let mut session = session_with("MI Rules PB", vec![xy]);
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "spec": AnalysisSpec::new("res-pb", "xy-pb", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 })).unwrap() }),
    );
    let pb_rep = rep_txt(session.document(), "res-pb");
    assert!(pb_rep.contains("against the fitted regression line") && !pb_rep.contains("against the identity line"));

    let mut synth_doc = session.document().clone();
    let mut mi_res = ResultSheet::new(
        AnalysisSpec::new(
            "res-mi-clamp",
            "xy-pb",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 1,
                max_iters: 0,
                seed: 7,
                response_column: None,
                predictor_columns: Vec::new(),
            }),
        )
        .unwrap(),
    )
    .unwrap();
    mi_res.status = ResultStatus::UpToDate;
    mi_res.scalars.push(ResultScalarEntry { key: String::from("Imputations (m)"), data_set_index: None, value: 2.0, units: None });
    mi_res.scalars.push(ResultScalarEntry { key: String::from("Max iterations"), data_set_index: None, value: 1.0, units: None });
    synth_doc.add_result_sheet(mi_res).unwrap();
    let mi_rep = rep_txt(&synth_doc, "res-mi-clamp");
    assert!(mi_rep.contains("m = 2 imputed datasets, max_iters = 1") && mi_rep.contains("clamped from requested m = 1, max_iters = 0"));

    let const_rule = |name: &str, v: f64| ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::Constant(v),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    };
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.parameter_rules = vec![const_rule("EC50", 1e-7), const_rule("hillslope", 1.0), const_rule("HillSlope", 2.0)];
    let mut nl_res = ResultSheet::new(AnalysisSpec::new("res-nl-legacy", "xy-pb", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap();
    nl_res.status = ResultStatus::UpToDate;
    synth_doc.add_result_sheet(nl_res).unwrap();
    let nl_rep = rep_txt(&synth_doc, "res-nl-legacy");
    assert!(
        nl_rep.contains("HillSlope: constrained to fixed constant 1")
            && !nl_rep.contains("fixed constant 2")
            && !nl_rep.contains("1e-7")
            && nl_rep.contains("Ignored 1 unrecognized and 1 superseded duplicate parameter rule(s)")
    );

    let mut dsl_nl = NonlinearFitSpec::new("custom_hill");
    dsl_nl.custom_dsl = Some(String::from("Y = Top * X / (Kd + X)"));
    dsl_nl.parameter_rules = vec![const_rule("kd", 5.0), const_rule("BogusParam", 9.0)];
    let mut dsl_res = ResultSheet::new(AnalysisSpec::new("res-dsl-rules", "xy-pb", AnalysisKind::NonlinearRegression(dsl_nl)).unwrap()).unwrap();
    dsl_res.status = ResultStatus::UpToDate;
    synth_doc.add_result_sheet(dsl_res).unwrap();
    let dsl_rep = rep_txt(&synth_doc, "res-dsl-rules");
    assert!(
        dsl_rep.contains("Kd: constrained to fixed constant 5")
            && !dsl_rep.contains("BogusParam")
            && dsl_rep.contains("Ignored 1 unrecognized and 0 superseded duplicate")
    );

    // AUD-02-27: missing result sheet returns ErrorCode::NotFound.
    assert_eq!(
        generate_methods_report(&synth_doc, &SheetId::new("missing-res"), ReportFormat::PlainText).unwrap_err().code,
        curvebook_doc::ErrorCode::NotFound
    );

    // AUD-02-23: Power info hooks resolve `N per group` on dirty fallback and fail result sheet on OOB Alpha = 1.5.
    let mut s_pwr = Session::new();
    ok_cmd(&mut s_pwr, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 2, "rows": 2, "auto_info": true }));
    let info_id = s_pwr.document().data_sheet(&SheetId::new("col-1")).unwrap().info_sheet_id.clone().unwrap();
    run_cmds(
        &mut s_pwr,
        &[
            ("info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "Alpha", "value": 0.05 })),
            ("info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "N per group", "value": 25.0 })),
            (
                "stats.run",
                json!({ "sheet_id": "pwr-1", "source_sheet_id": "col-1", "kind": "power", "solve_target": "Power", "info_sheet_id": info_id.as_str(), "sample_size": 10, "effect_size": 0.8 }),
            ),
        ],
    );
    let mut dirty_doc = s_pwr.document().clone();
    dirty_doc.result_sheet_mut(&SheetId::new("pwr-1")).unwrap().status = ResultStatus::Dirty;
    assert!(rep_txt(&dirty_doc, "pwr-1").contains("25"));
    ok_cmd(&mut s_pwr, "info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "Alpha", "value": 1.5 }));
    assert!(matches!(rs(&s_pwr, "pwr-1").status, ResultStatus::Failed { .. }));

    // AUD-13-25: Deming methods report includes X-intercept extension and custom plot X bounds.
    let mut s_dem = session_with("Dem", vec![xy_sheet("xy-1", "D", "X", &[2.0, 4.0, 6.0, 8.0, 10.0], &[("A", &[5.1, 8.9, 13.2, 16.8, 21.1])])]);
    ok_cmd(
        &mut s_dem,
        "fit.run",
        json!({ "sheet_id": "dem-1", "source_sheet_id": "xy-1", "kind": "deming", "extend_line_to_x_intercept": true, "plot_x_min": 0.0, "plot_x_max": 25.0 }),
    );
    let dem_rep = rep_txt(s_dem.document(), "dem-1");
    assert!(dem_rep.contains("X-intercept") && dem_rep.contains("25"), "{dem_rep}");
}
