//! Permanent Wave 3 audit regression tests for `crates/engine/src/methods_report.rs`
//! covering findings #81, #82, #83, #102, #105, #106, #107, #108, #109, #110, #111, #112, #113, #114.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, ColumnDesignHint, ColumnTable, ConfidenceLevel, ContingencyMethod, ContingencySpec, ContingencyTable,
    DataSheet, FactorialAnovaSpec, GroupedTable, InfoSheet, MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec,
    NestedAnalysisSpec, NestedTable, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, ParameterRule, ParameterScope, PassingBablokSpec,
    PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument, RepeatedMeasuresDesign,
    ResidualCovariance, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, SurvivalTable, TableArchetype, TwoGroupSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

struct ScratchDir {
    root: PathBuf,
}

impl ScratchDir {
    fn new(label: &str) -> Self {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-methods-audit-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> curvebook_engine::CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn find_tool_on_path(bin: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(bin);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let texbin = Path::new("/Library/TeX/texbin").join(bin);
    if texbin.is_file() { Some(texbin) } else { None }
}

fn assert_clean_room_prose(text: &str) {
    let lower = text.to_ascii_lowercase();
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!lower.contains(forbidden), "forbidden brand token `{forbidden}` in report:\n{text}");
    }
}

fn seed_three_group_column(id: &str) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap();
    t.data_sets[0].title = String::from("Ctrl");
    t.data_sets[1].title = String::from("Low");
    t.data_sets[2].title = String::from("High");
    let cols = [[10.0, 11.0, 12.0, 9.5, 10.5], [14.0, 15.0, 14.5, 16.0, 15.5], [22.0, 23.0, 21.5, 24.0, 22.5]];
    for (c, vals) in cols.iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(id, "ThreeGroup", TableArchetype::Column(t)).unwrap()
}

fn seed_rm_three_group_column(id: &str) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).unwrap();
    t.data_sets[0].title = String::from("Baseline");
    t.data_sets[1].title = String::from("DoseA");
    t.data_sets[2].title = String::from("DoseB");
    let rows = [[10.0, 13.0, 19.0], [11.2, 14.8, 20.5], [9.8, 12.6, 18.4], [10.6, 13.9, 19.7], [12.1, 15.4, 21.8]];
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(id, "RmCol", TableArchetype::Column(t)).unwrap()
}

#[test]
fn audit_105_108_latex_structural_and_unicode_infinity_compile() {
    let mut doc = ProjectDocument::new("TeX Compile Audit").unwrap();
    doc.add_data_sheet(seed_three_group_column("col-3")).unwrap();

    // 2x2 contingency table whose risk-difference CI crosses zero -> produces "∞" in NNT text (#108).
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.row_titles = vec![String::from("Event"), String::from("NoEvent")];
    ct.data_sets[0].title = String::from("Arm A");
    ct.data_sets[1].title = String::from("Arm B");
    ct.data_sets[0].cells[0][0] = Cell::number(10.0);
    ct.data_sets[1].cells[0][0] = Cell::number(12.0);
    ct.data_sets[0].cells[1][0] = Cell::number(11.0);
    ct.data_sets[1].cells[1][0] = Cell::number(10.0);
    doc.add_data_sheet(DataSheet::new("ct-nnt", "NNT Infinity", TableArchetype::Contingency(ct)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-3", "sheet_id": "res-aov", "kind": "one_way_anova", "multiplicity": "tukey" }));
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": AnalysisSpec::new(
                "res-nnt",
                "ct-nnt",
                AnalysisKind::Contingency(ContingencySpec {
                    method: ContingencyMethod::FisherExact,
                    compute_odds_and_risk_ratios: true,
                    compute_sensitivity_specificity: true,
                    compute_phi_and_cramers_v: true,
                    ..ContingencySpec::default()
                }),
            ).unwrap()
        }),
    );

    // Build a synthetic ResultSheet exercising leading `[` rows, open CIs, and Unicode symbols (#105, #108).
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
        assert!(!tex.contains("\\\\\\relax"), "must never emit \\\\\\relax before \\hline:\n{tex}");
        assert!(tex.contains(" \\\\\n\\hline"), "tabular rows must terminate with plain `\\\\` before `\\hline`:\n{tex}");
        assert!(!tex.contains('∞'), "raw Unicode infinity U+221E must not appear in LaTeX:\n{tex}");
        assert!(!tex.contains('²'), "raw superscript ² must be escaped in LaTeX:\n{tex}");
        assert!(!tex.contains('°'), "raw degree ° must be escaped in LaTeX:\n{tex}");
        assert_eq!(tex.matches("\\begin{tabular}").count(), tex.matches("\\end{tabular}").count());
        assert_eq!(tex.matches("\\begin{itemize}").count(), tex.matches("\\end{itemize}").count());
        assert_clean_room_prose(tex);
    }

    assert!(tex_nnt.contains("$\\infty$"), "2x2 NNT crossing zero must map ∞ to $\\infty$:\n{tex_nnt}");
    assert!(tex_synth.contains("{[}Agonist]"), "leading `[` cell must be guarded with `{{[}}`:\n{tex_synth}");
    assert!(tex_synth.contains(" {[}-inf, 2.5]"), "leading whitespace + `[` must be guarded with `{{[}}`:\n{tex_synth}");
    assert!(tex_synth.contains("\\textsuperscript{2}"), "superscript ² should map cleanly:\n{tex_synth}");
    assert!(tex_synth.contains("$\\infty$"), "infinity symbol should map to $\\infty$:\n{tex_synth}");

    // Compile combined fragments with external TeX engines if installed on PATH.
    let combined_doc = format!("\\documentclass{{article}}\n\\begin{{document}}\n{tex_aov}\n{tex_nnt}\n{tex_synth}\n\\end{{document}}\n");
    let scratch = ScratchDir::new("tex-compile");
    let tex_path = scratch.root.join("report.tex");
    fs::write(&tex_path, combined_doc).unwrap();

    for engine in ["pdflatex", "lualatex"] {
        let Some(bin) = find_tool_on_path(engine) else {
            continue;
        };
        let mut cmd = Command::new(&bin);
        cmd.arg("-interaction=nonstopmode").arg("-halt-on-error").arg(format!("-output-directory={}", scratch.root.display())).arg(&tex_path);
        if engine == "lualatex" {
            let cache_dir = scratch.root.join("texmf-var");
            fs::create_dir_all(&cache_dir).unwrap();
            cmd.env("TEXMFVAR", &cache_dir).env("TEXMFCACHE", &cache_dir).env("HOME", &scratch.root);
        }
        let out = cmd.output().unwrap_or_else(|e| panic!("failed to spawn {engine}: {e}"));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{engine} failed on generated LaTeX:\n{stdout}");
        let pdf_path = scratch.root.join("report.pdf");
        assert!(fs::metadata(&pdf_path).is_ok_and(|m| m.len() > 0), "{engine} produced empty/missing PDF");
        let _ = fs::remove_file(&pdf_path);
    }
}

#[test]
fn audit_106_107_markdown_latex_escaping_newlines_and_bold_labels() {
    let mut doc = ProjectDocument::new("Proj *star* _em_ <img src=x onerror=alert(1)> & #1\n\n# FakeProj").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    col.data_sets[0].title = String::from("Dose | [uM] <b\n> & *10*");
    col.data_sets[1].title = String::from("Resp `tick` ~wave~ #tag");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(15.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("col-xss", "Data `ticks` ~tilde~ [link](url)\nSecondLine", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-xss", "sheet_id": "res-xss", "kind": "unpaired_t" }));

    // Rename the ResultSheet title to inject newlines, HTML, and Markdown heading/setext syntax (#106).
    let mut dirty_doc = session.document().clone();
    dirty_doc.result_sheet_mut(&SheetId::new("res-xss")).unwrap().analysis.custom_title =
        Some(String::from("Res\n\n<script>alert(1)</script>\n# Injected heading\n---"));

    let md = generate_methods_report(&dirty_doc, &SheetId::new("res-xss"), ReportFormat::Markdown).unwrap();
    let tex = generate_methods_report(&dirty_doc, &SheetId::new("res-xss"), ReportFormat::Latex).unwrap();

    // #106 Markdown safety: no raw HTML tags, no injected lines/headings/horizontal rules, no raw unescaped *star*.
    assert!(md.contains("\\<script\\>") && !md.contains(" <script>"), "raw <script> must be escaped in Markdown:\n{md}");
    assert!(md.contains("\\<img") && !md.contains(" <img"), "raw <img must be escaped in Markdown:\n{md}");
    assert!(!md.lines().any(|l| l.starts_with("# Injected") || l.trim() == "---"), "newlines must be collapsed outside tables:\n{md}");
    assert!(md.contains("\\*star\\*"), "literal *star* must be escaped in Markdown:\n{md}");
    assert!(md.contains("\\_em\\_"), "literal _em_ must be escaped in Markdown:\n{md}");

    // #107 Bold labels must put trailing whitespace OUTSIDE `**...**`.
    assert!(!md.contains(" :**") && !md.contains(": **"), "trailing space must sit outside bold markers:\n{md}");
    for expected in [
        "**Project:** ",
        "**Analysis sheet:** ",
        "**Procedure:** ",
        "**Recalculation status:** ",
        "**Source table:** ",
        "**Observation exclusions:** ",
        "**Key scalar outputs:**",
    ] {
        assert!(md.contains(expected), "missing properly closed bold marker `{expected}` in Markdown:\n{md}");
    }

    // #106 LaTeX safety: `\section*{...}` and `\subsection*{...}` and `\paragraph*{...}` are single-line.
    for line in tex.lines() {
        if line.starts_with("\\section*{") || line.starts_with("\\subsection*{") || line.starts_with("\\paragraph*{") {
            assert!(line.ends_with('}'), "LaTeX heading/caption must remain on one line:\n{line}");
        }
    }

    if let Some(pandoc) = find_tool_on_path("pandoc") {
        for fmt in ["commonmark", "gfm"] {
            let mut child = Command::new(&pandoc)
                .args(["-f", fmt, "-t", "html"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(md.as_bytes()).unwrap();
            let rendered = child.wait_with_output().unwrap();
            assert!(rendered.status.success(), "pandoc {fmt} failed");
            let html = String::from_utf8_lossy(&rendered.stdout);
            assert!(!html.contains("<script"), "pandoc {fmt} emitted raw <script>:\n{html}");
            assert!(!html.contains("<img"), "pandoc {fmt} emitted raw <img>:\n{html}");
            assert!(!html.contains("<em>star</em>"), "pandoc {fmt} parsed *star* as emphasis:\n{html}");
            assert!(html.contains("<strong>Project:</strong>"), "pandoc {fmt} failed to render **Project:** bold:\n{html}");
            assert!(html.contains("<strong>Analysis sheet:</strong>"), "pandoc {fmt} failed to render **Analysis sheet:** bold:\n{html}");
        }
    }

    if let Some(pdflatex) = find_tool_on_path("pdflatex") {
        let scratch = ScratchDir::new("tex-xss");
        let tex_path = scratch.root.join("xss.tex");
        fs::write(&tex_path, format!("\\documentclass{{article}}\n\\begin{{document}}\n{tex}\n\\end{{document}}\n")).unwrap();
        let out = Command::new(&pdflatex)
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg(format!("-output-directory={}", scratch.root.display()))
            .arg(&tex_path)
            .output()
            .unwrap();
        assert!(out.status.success(), "pdflatex failed on adversarial titles:\n{}", String::from_utf8_lossy(&out.stdout));
    }
}

#[test]
fn audit_83_survival_multiplicity_plans() {
    // 1. Survival KM 3-cohort with TukeyHsd -> Bonferroni; 3-cohort with Dunnett -> Bonferroni vs control;
    //    3-cohort with FisherLsd -> unadjusted; 2-cohort -> no pairwise comparisons (#83).
    let mut doc = ProjectDocument::new("Survival Plans").unwrap();
    let mut srv3 = SurvivalTable::new(3, 4).unwrap();
    srv3.cohorts[0].title = String::from("Ctrl");
    srv3.cohorts[1].title = String::from("LowDose");
    srv3.cohorts[2].title = String::from("HighDose");
    for r in 0..4 {
        srv3.times[r] = Cell::number((r + 1) as f64 * 5.0);
        srv3.cohorts[0].cells[r][0] = Cell::number(1.0);
        srv3.cohorts[1].cells[r][0] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
        srv3.cohorts[2].cells[r][0] = Cell::number(if r == 3 { 1.0 } else { 0.0 });
    }
    doc.add_data_sheet(DataSheet::new("srv-3", "Surv3", TableArchetype::Survival(srv3)).unwrap()).unwrap();

    let mut srv2 = SurvivalTable::new(2, 4).unwrap();
    srv2.cohorts[0].title = String::from("Ctrl");
    srv2.cohorts[1].title = String::from("Drug");
    for r in 0..4 {
        srv2.times[r] = Cell::number((r + 1) as f64 * 4.0);
        srv2.cohorts[0].cells[r][0] = Cell::number(1.0);
        srv2.cohorts[1].cells[r][0] = Cell::number(if r > 0 { 1.0 } else { 0.0 });
    }
    doc.add_data_sheet(DataSheet::new("srv-2", "Surv2", TableArchetype::Survival(srv2)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    for (sid, mult) in [
        ("km-tuk", MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() }),
        ("km-dun", MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, ..Default::default() }),
        ("km-lsd", MultiplicitySpec { method: MultiplicityMethod::FisherLsd, ..Default::default() }),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            "srv-3",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: mult,
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
    }
    let km2_spec = AnalysisSpec::new(
        "km-2",
        "srv-2",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": km2_spec }));

    let km_tuk_rep = generate_methods_report(session.document(), &SheetId::new("km-tuk"), ReportFormat::PlainText).unwrap();
    let km_tuk_methods = km_tuk_rep.split("Stored Results Summary").next().unwrap();
    assert!(km_tuk_methods.contains("Bonferroni FWER adjustment"), "{km_tuk_methods}");
    assert!(!km_tuk_methods.contains("Tukey"), "{km_tuk_methods}");

    let km_dun_rep = generate_methods_report(session.document(), &SheetId::new("km-dun"), ReportFormat::PlainText).unwrap();
    let km_dun_methods = km_dun_rep.split("Stored Results Summary").next().unwrap();
    assert!(km_dun_methods.contains("against control cohort Ctrl") && km_dun_methods.contains("Bonferroni FWER adjustment"), "{km_dun_methods}");
    assert!(!km_dun_methods.contains("Dunnett"), "{km_dun_methods}");

    let km_lsd_rep = generate_methods_report(session.document(), &SheetId::new("km-lsd"), ReportFormat::PlainText).unwrap();
    assert!(km_lsd_rep.contains("unadjusted Fisher's LSD P values"), "{km_lsd_rep}");

    let km2_rep = generate_methods_report(session.document(), &SheetId::new("km-2"), ReportFormat::PlainText).unwrap();
    let km2_methods = km2_rep.split("Stored Results Summary").next().unwrap();
    assert!(km2_methods.contains("pairwise log-rank post-hoc comparisons were not computed"), "{km2_methods}");
    assert!(!km2_methods.contains("Tukey"), "{km2_methods}");
}

#[test]
fn audit_81_83_nested_welch_friedman_and_row_t_plans() {
    let mut doc = ProjectDocument::new("Effective Plans").unwrap();
    // Nested 2-group vs 3-group table (#83).
    let mut nest = NestedTable::new(3, 3, 3).unwrap();
    for g in 0..3 {
        nest.groups[g].title = format!("Grp{}", g + 1);
        for u in 0..3 {
            for r in 0..3 {
                nest.groups[g].cells[u][r] = Cell::number(10.0 + (g as f64) * 4.0 + (u as f64) * 0.5 + (r as f64) * 0.1);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("nest-3", "Nested3", TableArchetype::Nested(nest)).unwrap()).unwrap();
    doc.add_data_sheet(seed_three_group_column("col-3")).unwrap();
    doc.add_data_sheet(seed_rm_three_group_column("rm-3")).unwrap();

    // Multiple row t-tests grouped table (#81).
    let mut grp_mr = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap();
    grp_mr.data_sets[0].title = String::from("Ctrl");
    grp_mr.data_sets[1].title = String::from("Treated");
    for r in 0..4 {
        grp_mr.row_titles[r] = format!("Gene_{}", r + 1);
        for s in 0..3 {
            grp_mr.data_sets[0].cells[r][s] = Cell::number(10.0 + s as f64 * 0.2);
            grp_mr.data_sets[1].cells[r][s] = Cell::number(13.0 + r as f64 + s as f64 * 0.2);
        }
    }
    doc.add_data_sheet(DataSheet::new("grp-mr", "MultiRow", TableArchetype::Grouped(grp_mr)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    // 2. Nested t-test (2 groups, even with TukeyHsd stored in spec) & Nested ANOVA 2-group vs 3-group (#83).
    let mut nest_t_spec = AnalysisSpec::new(
        "nest-t",
        "nest-3",
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            two_group_t_test: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    nest_t_spec.selected_data_sets = vec![0, 1];
    ok_cmd(&mut session, "analysis.run", json!({ "spec": nest_t_spec }));
    let nest_t_rep = generate_methods_report(session.document(), &SheetId::new("nest-t"), ReportFormat::PlainText).unwrap();
    assert!(nest_t_rep.contains("Post-hoc pairwise comparisons were not computed"), "{nest_t_rep}");
    assert!(!nest_t_rep.contains("Tukey"), "{nest_t_rep}");

    let mut nest_2_spec = AnalysisSpec::new(
        "nest-a2",
        "nest-3",
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            two_group_t_test: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    nest_2_spec.selected_data_sets = vec![0, 1];
    ok_cmd(&mut session, "analysis.run", json!({ "spec": nest_2_spec }));
    let nest_a2_rep = generate_methods_report(session.document(), &SheetId::new("nest-a2"), ReportFormat::PlainText).unwrap();
    assert!(nest_a2_rep.contains("fewer than three groups were analyzed"), "{nest_a2_rep}");
    assert!(!nest_a2_rep.contains("Tukey"), "{nest_a2_rep}");

    // 3. Welch/BF ANOVA effective plans (#83 / Wave 2 alignment).
    for (sid, mult, expected, forbidden) in [
        ("bf-none", MultiplicitySpec { method: MultiplicityMethod::None, ..Default::default() }, "not requested (none computed)", "Games-Howell"),
        (
            "bf-bonf",
            MultiplicitySpec { method: MultiplicityMethod::Bonferroni, ..Default::default() },
            "Welch unequal-variance t tests",
            "Games-Howell",
        ),
        (
            "bf-dun",
            MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, ..Default::default() },
            "heteroscedastic Dunnett loadings",
            "Games-Howell",
        ),
        ("bf-trend", MultiplicitySpec { method: MultiplicityMethod::LinearTrend, ..Default::default() }, "heteroscedastic polynomial test", "Tukey"),
    ] {
        let spec = AnalysisSpec::new(
            sid,
            "col-3",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, multiplicity: mult, ..Default::default() }),
        )
        .unwrap();
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
        let rep = generate_methods_report(session.document(), &SheetId::new(sid), ReportFormat::PlainText).unwrap();
        assert!(rep.contains(expected), "{sid}: expected `{expected}` in:\n{rep}");
        assert!(!rep.contains(forbidden), "{sid}: forbidden `{forbidden}` in:\n{rep}");
    }

    // 4. Friedman with DunnRankSum executes Dunn within-block rank tests (#83 / Wave 2 alignment).
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "rm-3", "sheet_id": "fr-dunn", "kind": "friedman", "multiplicity": "dunn" }));
    let fr_dunn_rep = generate_methods_report(session.document(), &SheetId::new("fr-dunn"), ReportFormat::PlainText).unwrap();
    assert!(fr_dunn_rep.contains("within-block mean rank pairwise z-tests") && fr_dunn_rep.contains("Bonferroni"), "{fr_dunn_rep}");

    // 5. Multiple row t-tests: valid BH & FisherLsd omit fake CIs, and legacy synthetic TukeyHsd maps to Bonferroni (#81).
    let mr_bh_spec = AnalysisSpec::new(
        "mr-bh",
        "grp-mr",
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": mr_bh_spec }));
    let mr_bh_rep = generate_methods_report(session.document(), &SheetId::new("mr-bh"), ReportFormat::PlainText).unwrap();
    assert!(mr_bh_rep.contains("Benjamini-Hochberg"), "{mr_bh_rep}");
    assert!(!mr_bh_rep.contains("multiplicity-adjusted confidence intervals"), "{mr_bh_rep}");

    let mr_lsd_spec = AnalysisSpec::new(
        "mr-lsd",
        "grp-mr",
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::FisherLsd, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": mr_lsd_spec }));
    let mr_lsd_rep = generate_methods_report(session.document(), &SheetId::new("mr-lsd"), ReportFormat::PlainText).unwrap();
    assert!(mr_lsd_rep.contains("Fisher's unprotected LSD; no multiplicity correction"), "{mr_lsd_rep}");
    assert!(!mr_lsd_rep.contains("multiplicity-adjusted confidence intervals"), "{mr_lsd_rep}");

    // Legacy UpToDate MultipleRowTTests ResultSheet with TukeyHsd must report Bonferroni (never Tukey's HSD).
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
    let legacy_mr_rep = generate_methods_report(&legacy_doc, &SheetId::new("mr-legacy-tuk"), ReportFormat::PlainText).unwrap();
    assert!(legacy_mr_rep.contains("Bonferroni FWER correction"), "{legacy_mr_rep}");
    assert!(!legacy_mr_rep.contains("Tukey"), "{legacy_mr_rep}");
}

#[test]
fn audit_82_alpha_aware_stars_and_legend() {
    let mut doc = ProjectDocument::new("Alpha Stars").unwrap();
    // Create two groups with p ~ 0.025 (significant at alpha=0.05, NOT significant at alpha=0.01).
    let mut t2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    t2.data_sets[0].title = String::from("Ctrl");
    t2.data_sets[1].title = String::from("Treated");
    for (r, (&a, &b)) in [10.0, 10.5, 11.0, 9.5, 10.2].iter().zip([11.0, 11.5, 12.0, 10.5, 11.2].iter()).enumerate() {
        t2.data_sets[0].cells[r][0] = Cell::number(a);
        t2.data_sets[1].cells[r][0] = Cell::number(b);
    }
    doc.add_data_sheet(DataSheet::new("col-2", "TwoGroup", TableArchetype::Column(t2)).unwrap()).unwrap();

    // Create 3-group table with ANOVA p in (0.01, 0.05).
    let mut t3 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap();
    t3.data_sets[0].title = String::from("A");
    t3.data_sets[1].title = String::from("B");
    t3.data_sets[2].title = String::from("C");
    for (r, ((&a, &b), &c)) in
        [10.0, 11.0, 12.0, 9.0, 10.5].iter().zip([10.9, 12.0, 12.8, 10.2, 11.5].iter()).zip([11.9, 13.0, 13.7, 11.3, 12.5].iter()).enumerate()
    {
        t3.data_sets[0].cells[r][0] = Cell::number(a);
        t3.data_sets[1].cells[r][0] = Cell::number(b);
        t3.data_sets[2].cells[r][0] = Cell::number(c);
    }
    doc.add_data_sheet(DataSheet::new("col-3p", "ThreeGroupP", TableArchetype::Column(t3)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    let mut spec_t = AnalysisSpec::new("res-t99", "col-2", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    spec_t.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": spec_t }));

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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": spec_aov }));

    for id in ["res-t99", "res-aov99"] {
        let rep = generate_methods_report(session.document(), &SheetId::new(id), ReportFormat::PlainText).unwrap();
        assert!(rep.contains("alpha = 0.01"), "{id}: missing alpha=0.01:\n{rep}");
        assert!(rep.contains("p <= alpha"), "{id}: missing inclusive boundary:\n{rep}");
        assert!(
            rep.contains("ns for p > 0.01, * for p <= 0.01, ** for p <= 0.002, *** for p <= 0.0002, **** for p <= 0.00002"),
            "{id}: star legend must match 99% confidence alpha=0.01:\n{rep}"
        );
    }

    // Confirm the two-group report renders (ns) rather than (*) when P falls in (0.01, 0.05),
    // and the ANOVA omnibus row renders 0.0309 (ns) while Adjusted P at threshold=0.01 uses (*) not (**).
    let t_res = session.document().result_sheet(&SheetId::new("res-t99")).unwrap();
    let t_p = t_res.lookup_scalar("PValue", None).unwrap();
    assert!(t_p > 0.01 && t_p < 0.05, "expected 0.01 < t_p < 0.05, got {t_p}");
    let t_rep = generate_methods_report(session.document(), &SheetId::new("res-t99"), ReportFormat::PlainText).unwrap();
    assert!(t_rep.contains("(ns)"), "99% two-group report must show (ns) for p={t_p}:\n{t_rep}");
    assert!(!t_rep.contains("(*)"), "99% two-group report must NOT show (*) for p={t_p}:\n{t_rep}");

    let aov_res = session.document().result_sheet(&SheetId::new("res-aov99")).unwrap();
    let p_val = aov_res.lookup_scalar("P value", None).unwrap();
    assert!(p_val > 0.01 && p_val < 0.05, "expected 0.01 < p < 0.05, got {p_val}");
    let aov_rep = generate_methods_report(session.document(), &SheetId::new("res-aov99"), ReportFormat::PlainText).unwrap();
    assert!(aov_rep.contains("0.0309 (ns)"), "99% confidence report must show 0.0309 (ns) for p={p_val}:\n{aov_rep}");
    assert!(!aov_rep.contains("0.0309 (*)"), "99% confidence report must NOT show 0.0309 (*) for p={p_val}:\n{aov_rep}");
    assert!(aov_rep.contains("0.0097 (*)"), "0.0097 at threshold=0.01 should receive (*):\n{aov_rep}");
    assert!(!aov_rep.contains("0.0097 (**)"), "0.0097 at threshold=0.01 must NOT receive (**):\n{aov_rep}");

    // APA preset omits 4th star tier (star_ceiling == 3), and exact p == 0.01 at alpha == 0.01 gets (*).
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
    let b_rep = generate_methods_report(&boundary_doc, &SheetId::new("res-apa99"), ReportFormat::PlainText).unwrap();
    assert!(!b_rep.contains("****"), "APA preset has 3-star ceiling:\n{b_rep}");
    assert!(b_rep.contains(".010 (*)"), "inclusive boundary p == alpha must earn `*`:\n{b_rep}");
    assert!(b_rep.contains(".037 (ns)"), "p=0.0369 > 0.01 must render `(ns)`:\n{b_rep}");
}

#[test]
fn audit_109_110_factorial_rm_estimator_and_effect_sizes() {
    let mut doc = ProjectDocument::new("Factorial RM Estimator").unwrap();
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 2).unwrap();
    grp.row_titles = vec![String::from("Pre"), String::from("Post")];
    for c in 0..3 {
        grp.data_sets[c].title = format!("T{c}");
        for r in 0..2 {
            for s in 0..3 {
                grp.data_sets[c].cells[r][s] = Cell::number(10.0 + (r as f64) * 3.0 + (c as f64) * 2.0 + (s as f64) * 0.5);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("grp-rm", "Rm2Way", TableArchetype::Grouped(grp)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // 1. Complete data with default fit_reml_on_missing: true -> runs Classical Balanced RM ANOVA (OLS).
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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": rm_spec }));
    let rep_complete = generate_methods_report(session.document(), &SheetId::new("res-rm-complete"), ReportFormat::PlainText).unwrap();
    assert!(rep_complete.contains("classical balanced repeated-measures ANOVA (OLS)"), "{rep_complete}");
    assert!(!rep_complete.contains("REML"), "complete classical RM must not claim REML:\n{rep_complete}");
    assert!(rep_complete.contains("Partial eta-squared and eta-squared effect sizes were calculated"), "{rep_complete}");
    assert!(!rep_complete.contains("omega-squared"), "factorial report must never claim omega-squared (#110):\n{rep_complete}");

    // 2. Unstructured REML explicit -> unstructured wording, partial eta-squared only (no eta-squared/omega-squared).
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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": un_spec }));
    let rep_un = generate_methods_report(session.document(), &SheetId::new("res-rm-un"), ReportFormat::PlainText).unwrap();
    assert!(rep_un.contains("unstructured residual covariance matrix"), "{rep_un}");
    assert!(rep_un.contains("Partial eta-squared effect sizes were calculated"), "{rep_un}");
    assert!(!rep_un.contains("Partial eta-squared and eta-squared"), "{rep_un}");
    assert!(!rep_un.contains("omega-squared"), "{rep_un}");

    // 3. Introduce missing cell -> res-rm-complete upgrades via DAG to Compound Symmetry REML!
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "grp-rm", "cells": [{ "row": 0, "col": 1, "subcol": 2 }] }));
    let rep_reml = generate_methods_report(session.document(), &SheetId::new("res-rm-complete"), ReportFormat::PlainText).unwrap();
    assert!(rep_reml.contains("compound-symmetry"), "{rep_reml}");
    assert!(rep_reml.contains("REML"), "{rep_reml}");
    assert!(!rep_reml.contains("classical balanced"), "{rep_reml}");
    assert!(rep_reml.contains("Partial eta-squared effect sizes were calculated"), "{rep_reml}");
    assert!(!rep_reml.contains("Partial eta-squared and eta-squared"), "{rep_reml}");
    assert!(!rep_reml.contains("omega-squared"), "{rep_reml}");
}

#[test]
fn audit_111_post_test_confidence_intervals() {
    let mut doc = ProjectDocument::new("PostTest CIs").unwrap();
    doc.add_data_sheet(seed_three_group_column("col-3")).unwrap();
    let mut session = Session::from_document(doc);

    for (sid, mult, must_contain, must_not_contain) in [
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
        let rep = generate_methods_report(session.document(), &SheetId::new(sid), ReportFormat::PlainText).unwrap();
        // Inspect only the methods paragraph describing Multiple comparisons (header mentions generic confidence intervals).
        let mc_line = rep.lines().find(|l| l.contains("Multiple comparisons")).unwrap_or_else(|| panic!("{sid} missing MC line:\n{rep}"));
        if let Some(need) = must_contain {
            assert!(mc_line.contains(need), "{sid}: expected `{need}` in `{mc_line}`");
        }
        if let Some(ban) = must_not_contain {
            assert!(!mc_line.contains(ban), "{sid}: must not contain `{ban}` in `{mc_line}`");
        }
    }
}

#[test]
fn audit_112_power_analysis_effective_info_values() {
    let mut doc = ProjectDocument::new("Power Info Effective").unwrap();
    doc.add_data_sheet(seed_three_group_column("col-src")).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-pwr", "Power Constants").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Alpha", "value": 0.01, "kind": "number" }));
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Target Power", "value": 0.95, "kind": "number" }));
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-pwr", "name": "Effect Size", "value": 0.5, "kind": "number" }));

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
    ok_cmd(&mut session, "analysis.run", json!({ "source_sheet_id": "col-src", "sheet_id": "res-pwr-info", "kind": { "PowerAnalysis": p_spec } }));

    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let rep = generate_methods_report(session.document(), &SheetId::new("res-pwr-info"), fmt).unwrap();
        assert!(rep.contains("0.01"), "{fmt:?}: must reflect Info alpha=0.01:\n{rep}");
        assert!(rep.contains("99"), "{fmt:?}: header must reflect 99% confidence corresponding to alpha=0.01:\n{rep}");
        assert!(rep.contains("target power = 0.95"), "{fmt:?}: must reflect Info target_power=0.95:\n{rep}");
        assert!(!rep.contains("alpha = 0.05"), "{fmt:?}: must NOT print stale stored alpha=0.05:\n{rep}");
        assert!(!rep.contains("target power = 0.8"), "{fmt:?}: must NOT print stale stored power=0.8:\n{rep}");
    }
}

#[test]
fn audit_113_114_mi_settings_and_nonlinear_rules_and_102_pb() {
    let mut doc = ProjectDocument::new("MI Rules PB").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).unwrap();
    xy.data_sets[0].title = String::from("Method A");
    xy.data_sets[1].title = String::from("Method B");
    for r in 0..6 {
        let x = (r + 1) as f64 * 10.0;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(x * 1.1 + 2.0);
        xy.data_sets[1].cells[r][0] = Cell::number(x * 0.95 - 1.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-pb", "PB", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // #102 Passing-Bablok CUSUM tests against the fitted regression line, not the identity line.
    let pb_spec = AnalysisSpec::new("res-pb", "xy-pb", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 })).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": pb_spec }));
    let pb_rep = generate_methods_report(session.document(), &SheetId::new("res-pb"), ReportFormat::PlainText).unwrap();
    assert!(pb_rep.contains("against the fitted regression line"), "{pb_rep}");
    assert!(!pb_rep.contains("against the identity line"), "{pb_rep}");

    // #113 Legacy clamped MI ResultSheet (requested m=1, max_iters=0 -> executed m=2, max_iters=1).
    let mut synth_doc = session.document().clone();
    let mi_spec = AnalysisSpec::new(
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
    .unwrap();
    let mut mi_res = ResultSheet::new(mi_spec).unwrap();
    mi_res.status = ResultStatus::UpToDate;
    mi_res.scalars.push(ResultScalarEntry { key: String::from("Imputations (m)"), data_set_index: None, value: 2.0, units: None });
    mi_res.scalars.push(ResultScalarEntry { key: String::from("Max iterations"), data_set_index: None, value: 1.0, units: None });
    synth_doc.add_result_sheet(mi_res).unwrap();
    let mi_rep = generate_methods_report(&synth_doc, &SheetId::new("res-mi-clamp"), ReportFormat::PlainText).unwrap();
    assert!(mi_rep.contains("m = 2 imputed datasets, max_iters = 1"), "{mi_rep}");
    assert!(mi_rep.contains("clamped from requested m = 1, max_iters = 0"), "{mi_rep}");

    // #114 Legacy nonlinear fit with unknown (`EC50`) and duplicate (`hillslope`, `HillSlope`) rules.
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.parameter_rules = vec![
        ParameterRule {
            name: String::from("EC50"),
            scope: ParameterScope::Constant(1e-7),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("hillslope"),
            scope: ParameterScope::Constant(1.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("HillSlope"),
            scope: ParameterScope::Constant(2.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let mut nl_res = ResultSheet::new(AnalysisSpec::new("res-nl-legacy", "xy-pb", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap();
    nl_res.status = ResultStatus::UpToDate;
    synth_doc.add_result_sheet(nl_res).unwrap();
    let nl_rep = generate_methods_report(&synth_doc, &SheetId::new("res-nl-legacy"), ReportFormat::PlainText).unwrap();
    assert!(nl_rep.contains("HillSlope: constrained to fixed constant 1"), "{nl_rep}");
    assert!(!nl_rep.contains("fixed constant 2"), "duplicate superseded rule must not be listed:\n{nl_rep}");
    assert!(!nl_rep.contains("1e-7"), "unknown rule value must not be listed as active:\n{nl_rep}");
    assert!(nl_rep.contains("Ignored 1 unrecognized and 1 superseded duplicate parameter rule(s)"), "{nl_rep}");

    // #114 custom_dsl parameter rules resolve compiled DSL parameter names cleanly too.
    let mut dsl_nl = NonlinearFitSpec::new("custom_hill");
    dsl_nl.custom_dsl = Some(String::from("Y = Top * X / (Kd + X)"));
    dsl_nl.parameter_rules = vec![
        ParameterRule {
            name: String::from("kd"),
            scope: ParameterScope::Constant(5.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("BogusParam"),
            scope: ParameterScope::Constant(9.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let mut dsl_res = ResultSheet::new(AnalysisSpec::new("res-dsl-rules", "xy-pb", AnalysisKind::NonlinearRegression(dsl_nl)).unwrap()).unwrap();
    dsl_res.status = ResultStatus::UpToDate;
    synth_doc.add_result_sheet(dsl_res).unwrap();
    let dsl_rep = generate_methods_report(&synth_doc, &SheetId::new("res-dsl-rules"), ReportFormat::PlainText).unwrap();
    assert!(dsl_rep.contains("Kd: constrained to fixed constant 5"), "{dsl_rep}");
    assert!(!dsl_rep.contains("BogusParam"), "{dsl_rep}");
    assert!(dsl_rep.contains("Ignored 1 unrecognized and 0 superseded duplicate"), "{dsl_rep}");
}
