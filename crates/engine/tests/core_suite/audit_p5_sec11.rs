#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::error::Error;
use std::fs;
use std::thread;
use std::time::Duration;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, CURRENT_SCHEMA_VERSION, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorCode,
    GraphAnnotation, GraphSheet, GraphicalDataObject, HookSource, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, LinearRegressionSpec,
    NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, RectPt, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SeriesSpec, SheetId, TableArchetype, TransformAnalysisSpec,
};
use curvebook_engine::cmd;
use curvebook_engine::cmd::helpers::{decode_base64, decode_base64_capped, encode_base64, write_output_file};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MemoryJournalBackend};
use serde_json::{Value, json};

use super::common::{TempDir, assert_bad_pointer, ds, gs, rs, session_with, try_cmd as ok_cmd, try_err_cmd as err_cmd, xy_sheet};

type TestResult = Result<(), Box<dyn Error>>;

fn build_xy_sheet(id: &str, x_title: &str, y_titles: &[&str]) -> DataSheet {
    let ys: Vec<(&str, Vec<f64>)> = y_titles
        .iter()
        .enumerate()
        .map(|(c, &t)| (t, (0..4).map(|r| (r + 1) as f64 + (c as f64 + 1.0) * 0.5 + (r % 2) as f64 * 0.1).collect()))
        .collect();
    let refs: Vec<(&str, &[f64])> = ys.iter().map(|(t, v)| (*t, v.as_slice())).collect();
    xy_sheet(id, "Method XY", x_title, &[1.0, 2.0, 3.0, 4.0], &refs)
}

#[test]
fn fa_sec11_01_bland_altman_xy_x_vs_y0_methods_report_labels() -> TestResult {
    let mut session = session_with(
        "BA XY Report",
        vec![
            build_xy_sheet("xy-1", "Reference", &["Assay"]),
            build_xy_sheet("xy-2", "Reference", &["AssayA", "AssayB"]),
            build_xy_sheet("xy-blank", "   ", &["  "]),
        ],
    );
    for (id, src, mode) in
        [("ba-1", "xy-1", None), ("ba-2-xy", "xy-2", Some(true)), ("ba-2-yy", "xy-2", Some(false)), ("ba-blank", "xy-blank", Some(true))]
    {
        let spec = AnalysisSpec::new(
            id,
            src,
            AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: mode }),
        )?;
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }))?;
    }
    assert_eq!(rs(&session, "ba-1").tabs[0].column_headers, vec!["Reference vs. Assay"]);
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let rep = generate_methods_report(session.document(), &SheetId::new("ba-1"), fmt)?;
        assert!(rep.contains("Reference") && rep.contains("Assay") && !rep.contains("Column 2"));
    }
    let rep2 = generate_methods_report(session.document(), &SheetId::new("ba-2-xy"), ReportFormat::PlainText)?;
    assert!(rep2.contains("Method agreement between Reference and AssayA") && !rep2.contains("Method agreement between AssayA"));
    assert!(
        generate_methods_report(session.document(), &SheetId::new("ba-2-yy"), ReportFormat::PlainText)?
            .contains("Method agreement between AssayA and AssayB")
    );
    assert!(
        generate_methods_report(session.document(), &SheetId::new("ba-blank"), ReportFormat::PlainText)?
            .contains("Method agreement between X and Column 1")
    );
    Ok(())
}

#[test]
fn fa_sec11_02_decode_base64_bounded_non_empty_and_raw_guard() -> TestResult {
    for empty in ["", "   \t\r\n  "] {
        assert_eq!(decode_base64(empty).expect_err("empty").code, ErrorCode::BadInput);
        let err = decode_base64_capped(empty, 1024, "/bytes_base64").expect_err("empty capped");
        assert_eq!((err.code, err.details.as_ref().and_then(|d| d.get("pointer"))), (ErrorCode::BadInput, Some(&json!("/bytes_base64"))));
    }
    assert_eq!(decode_base64_capped(&encode_base64(b"abcd"), 4, "/bytes_base64")?, b"abcd");
    let over = decode_base64_capped(&encode_base64(b"abcde"), 4, "/bytes_base64").expect_err("over cap");
    assert_eq!((over.code, over.details.as_ref().and_then(|d| d.get("pointer"))), (ErrorCode::ResourceLimit, Some(&json!("/bytes_base64"))));
    assert!(decode_base64_capped(&format!("{}\n{}", encode_base64(b"hello"), encode_base64(b"world")), 64, "/p").is_err());
    assert_eq!(decode_base64_capped("AA==  \n", 1, "/bytes_base64")?, vec![0]);
    assert_eq!(decode_base64_capped(&" ".repeat(100_000), 16, "/bytes_base64").expect_err("raw guard").code, ErrorCode::ResourceLimit);

    let mut session = Session::new();
    for cmd_id in ["project.open", "project.instantiateTemplate"] {
        for bad in ["", "   \n\t "] {
            assert_eq!(err_cmd(&mut session, cmd_id, json!({ "bytes_base64": bad }))?.code, ErrorCode::BadInput);
        }
    }
    let oversized_archive_b64 = "A".repeat(MAX_ARCHIVE_FILE_BYTES.div_ceil(3).saturating_mul(4) + 4);
    assert_eq!(err_cmd(&mut session, "project.open", json!({ "bytes_base64": oversized_archive_b64 }))?.code, ErrorCode::ResourceLimit);
    Ok(())
}

#[test]
fn fa_sec11_03_extract_result_tab_enforces_max_project_cells() -> TestResult {
    let mut doc = ProjectDocument::new("Cell Budget")?;
    for id in ["big-1", "big-2"] {
        doc.add_data_sheet(DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 25, 998_000)?))?)?;
    }
    let mut r = ResultSheet::new(AnalysisSpec::new("res-big", "big-1", AnalysisKind::DescriptiveStats(Default::default()))?)?;
    r.status = ResultStatus::UpToDate;
    for (id, rows) in [("big-tab", 100_000), ("fit-tab", 50_000)] {
        r.tabs.push(ResultTab {
            id: id.to_owned(),
            title: id.to_owned(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("A"), String::from("B")],
            row_labels: Vec::new(),
            rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }; 2]; rows],
            footnotes: Vec::new(),
        });
    }
    doc.add_result_sheet(r)?;

    let mut session = Session::from_document(doc);
    assert_eq!(
        err_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "res-big", "tab_id": "big-tab", "sheet_id": "too-big" }))?.code,
        ErrorCode::ResourceLimit
    );
    assert!(session.document().data_sheet(&SheetId::new("too-big")).is_none());

    let ok = ok_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "res-big", "tab_id": "fit-tab", "sheet_id": "exact-fit" }))?;
    assert_eq!((ok.payload["sheet_id"].as_str(), session.document().total_allocated_cells()?), (Some("exact-fit"), 50_000_000));
    Ok(())
}

#[test]
fn fa_sec11_04a_project_open_and_save_dirty_state_parity() -> TestResult {
    let dir = TempDir::try_new("dirty-parity")?;
    let (archive_path, unpacked_dir, future_path) = (dir.child("study.curvebook"), dir.child("study.curvebook.d"), dir.child("future.curvebook"));

    let mut w = curvebook_format::zip::ZipWriter::new();
    let fv = u64::from(CURRENT_SCHEMA_VERSION) + 1;
    w.add_file(
        "manifest.json",
        format!(r#"{{"format":"curvebook-archive","schema_version":{fv},"kind":"project","title":"Future","document_crc32":0,"sheets":[]}}"#)
            .into_bytes(),
    )?;
    w.add_file("document.json", format!(r#"{{"schema_version":{fv},"title":"Future"}}"#).into_bytes())?;
    fs::write(&future_path, w.finish()?)?;

    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d1", "title": "Assay", "kind": "xy", "rows": 3, "cols": 1 }))?;
    assert!(session.is_dirty() && session.file_path().is_none());

    let mem_save = ok_cmd(&mut session, "project.save", json!({ "return_bytes": true }))?;
    assert!(!mem_save.mutated && session.is_dirty() && session.file_path().is_none());
    let b64 = mem_save.payload["bytes_base64"].as_str().ok_or("b64")?.to_owned();

    assert_eq!(err_cmd(&mut session, "project.save", json!({ "path": future_path.to_string_lossy() }))?.code, ErrorCode::UnsupportedVersion);
    assert!(session.is_dirty() && session.file_path().is_none());

    let undo_before = session.undo_depth();
    let saved = ok_cmd(&mut session, "project.save", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(!saved.mutated && !session.is_dirty() && session.file_path() == Some(archive_path.as_path()) && session.undo_depth() == undo_before);

    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 9.5 }))?;
    assert!(session.is_dirty());
    ok_cmd(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy(), "unpacked": true }))?;
    assert!(!session.is_dirty() && session.file_path() == Some(unpacked_dir.as_path()));

    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "sheet.createData", json!({ "sheet_id": "scratch" }))?;
    let opened = ok_cmd(&mut reopened, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(opened.mutated && !reopened.is_dirty() && reopened.file_path() == Some(archive_path.as_path()));
    ok_cmd(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }))?;
    assert!(!reopened.is_dirty() && reopened.file_path() == Some(unpacked_dir.as_path()));
    ok_cmd(&mut reopened, "project.open", json!({ "bytes_base64": b64 }))?;
    assert!(reopened.is_dirty() && reopened.file_path().is_none());
    Ok(())
}

#[test]
fn fa_sec11_04b_project_open_error_and_wal_rollback_preserve_before_dirty() -> TestResult {
    let dir = TempDir::try_new("dirty-rollback")?;
    let (archive_path, unpacked_dir) = (dir.child("study.curvebook"), dir.child("study.curvebook.d"));
    let mut seed = Session::new();
    ok_cmd(&mut seed, "sheet.createData", json!({ "sheet_id": "d1", "title": "Assay", "kind": "xy", "rows": 2, "cols": 1 }))?;
    ok_cmd(&mut seed, "project.save", json!({ "path": archive_path.to_string_lossy() }))?;
    ok_cmd(&mut seed, "project.save", json!({ "path": unpacked_dir.to_string_lossy(), "unpacked": true }))?;

    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    let _ = err_cmd(&mut reopened, "project.open", json!({ "path": dir.child("missing.curvebook").to_string_lossy() }))?;
    assert!(!reopened.is_dirty() && reopened.file_path() == Some(archive_path.as_path()));

    let (mut wal_sess, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "dirty-wal", 30_000, 1_000)?;
    wal_sess.execute_and_record(&mut journal, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    ok_cmd(&mut wal_sess, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 42.0 }))?;
    journal.backend_mut().set_lock_lease(None);
    assert_eq!(
        wal_sess.execute_and_record(&mut journal, "project.open", json!({ "path": unpacked_dir.to_string_lossy() })).expect_err("lost lease").code,
        ErrorCode::IoError
    );
    assert!(wal_sess.is_dirty() && wal_sess.file_path() == Some(archive_path.as_path()));
    Ok(())
}

#[test]
fn fa_sec11_05_create_checkpoint_wal_checkpoint_and_replay_parity() -> TestResult {
    assert!(Session::command_requires_wal_checkpoint("project.createCheckpoint"));
    let (mut live, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "cp-live", 30_000, 1_000)?;
    for (cmd_id, params) in [
        ("sheet.createData", json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "rows": 2, "cols": 1 })),
        ("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 10.0 })),
        ("project.createCheckpoint", json!({ "name": "Baseline" })),
        ("table.setCell", json!({ "sheet_id": "d1", "row": 1, "col": 0, "value": 20.0 })),
    ] {
        live.execute_and_record(&mut journal, cmd_id, params)?;
    }
    thread::sleep(Duration::from_millis(1_150));
    let (replayed, _, _) = Session::open_with_recovery(journal.abandon(), "cp-recover", 30_000, 40_000)?;
    assert_eq!(live.document(), replayed.document());
    Ok(())
}

#[test]
fn fa_sec11_06_clone_downstream_graphs_covers_annotations_and_inline_hooks() -> TestResult {
    let mut doc = ProjectDocument::new("Graph Hooks Clone")?;
    let mut info1 = InfoSheet::new_preseeded("info-1", "Main Info")?;
    info1.set_constant("Alpha", ConstantValue::Number(0.05), None)?;
    info1.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info1)?;
    let mut xy1 = build_xy_sheet("xy-1", "Dose", &["Response"]);
    xy1.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(xy1)?;
    doc.add_data_sheet(build_xy_sheet("xy-other", "Other X", &["Other Y"]))?;

    let mut session = Session::from_document(doc);
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "spec": AnalysisSpec::new("fit-1", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))? }),
    )?;
    let mut working = session.document().clone();
    let other_ds = working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?.clone();

    let mut add_g = |id: &str, f: fn(&mut GraphSheet)| -> TestResult {
        let mut g = GraphSheet::new_uninitialized_for_data(id, &other_ds)?;
        f(&mut g);
        working.add_graph_sheet(g)?;
        Ok(())
    };
    add_g("g-table", |g| {
        g.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("fit-1"),
            tab_id: None,
            bounds_pt: RectPt::new(10.0, 10.0, 80.0, 40.0),
            selected_rows: None,
            selected_cols: None,
            style: Default::default(),
        })
    })?;
    add_g("g-eq", |g| {
        g.spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("fit-1"),
            data_set_index: 0,
            mode: Default::default(),
            bounds_pt: RectPt::new(10.0, 10.0, 80.0, 30.0),
            font_size_pt: 10.0,
            bordered: false,
        })
    })?;
    add_g("g-box", |g| {
        g.spec.annotations.push(GraphAnnotation::TextBox {
            bounds_pt: RectPt::new(10.0, 10.0, 80.0, 30.0),
            markdown: String::from("Slope={{result:fit-1:Slope}}"),
            bordered: false,
        })
    })?;
    add_g("g-title", |g| {
        g.spec.title = String::from("Alpha={{info:info-1:Alpha}}");
        g.spec.x_axis.title = String::from("X ({{info:info-1:Alpha}})");
    })?;
    add_g("g-legend", |g| {
        g.spec.series = vec![SeriesSpec {
            source_sheet_id: SheetId::new("xy-other"),
            data_set_index: 0,
            legend_label: Some(String::from("Fit={{result:fit-1:Slope}}")),
            bind_to_right_y2: false,
            gdo: GraphicalDataObject::ScatterAndLine { marker: None, line: None, area_fill: None, replicate_lines: Default::default() },
            y_error_bars: None,
            x_error_bars: None,
            regression_overlay: None,
            offset: Default::default(),
            bar_width_override_pt: None,
            show_raw_data: true,
        }]
    })?;
    add_g("g-unrelated", |g| g.spec.title = String::from("Unrelated {{info:Alpha}}"))?;

    let mut lay_spec = LayoutSpec::default();
    lay_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-1"),
        graph_sheet_id: SheetId::new("g-title"),
        bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    working.add_layout_sheet(LayoutSheet::new("lay-1", "Figure", lay_spec)?)?;

    let mut clone_sess = Session::from_document(working);
    let dup = ok_cmd(&mut clone_sess, "sheet.duplicateFamily", json!({ "sheet_id": "xy-1", "prefix": "dup" }))?;
    let (new_info, new_res) = (dup.payload["info_sheet_id"].as_str().ok_or("info")?, dup.payload["result_sheet_ids"][0].as_str().ok_or("res")?);
    let graph_ids: Vec<&str> = dup.payload["graph_sheet_ids"].as_array().ok_or("graphs")?.iter().filter_map(Value::as_str).collect();
    assert_eq!((graph_ids.len(), dup.payload["layout_sheet_ids"].as_array().ok_or("layouts")?.len()), (5, 1));

    let (mut saw_t, mut saw_e, mut saw_b, mut saw_ti, mut saw_l) = (false, false, false, false, false);
    for gid in &graph_ids {
        let g = gs(&clone_sess, gid);
        for ann in &g.spec.annotations {
            match ann {
                GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } => {
                    assert_eq!(result_sheet_id.as_str(), new_res);
                    saw_t = true;
                }
                GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                    assert_eq!(result_sheet_id.as_str(), new_res);
                    saw_e = true;
                }
                GraphAnnotation::TextBox { markdown, .. } => {
                    assert_eq!(markdown, &format!("Slope={{{{result:{new_res}:Slope}}}}"));
                    saw_b = true;
                }
                _ => {}
            }
        }
        if g.spec.title.starts_with("Alpha=") {
            assert_eq!(
                (&g.spec.title, &g.spec.x_axis.title),
                (&format!("Alpha={{{{info:{new_info}:Alpha}}}}"), &format!("X ({{{{info:{new_info}:Alpha}}}})"))
            );
            saw_ti = true;
        }
        if let Some(lbl) = g.spec.series.first().and_then(|s| s.legend_label.as_deref()) {
            assert_eq!(lbl, format!("Fit={{{{result:{new_res}:Slope}}}}"));
            saw_l = true;
        }
    }
    assert!(saw_t && saw_e && saw_b && saw_ti && saw_l);
    assert_eq!(
        ok_cmd(&mut clone_sess, "sheet.clonePipeline", json!({ "sheet_id": "xy-1", "prefix": "pipe" }))?.payload["graph_sheet_ids"]
            .as_array()
            .ok_or("pipe")?
            .len(),
        5
    );
    Ok(())
}

#[test]
fn fa_sec11_07_methods_report_formats_hook_source_without_debug_prose() -> TestResult {
    let mut doc = ProjectDocument::new("Hook Formatting")?;
    let mut info1 = InfoSheet::new_preseeded("info-1", "Assay Info")?;
    info1.set_constant("IC50", ConstantValue::Number(5.0), None)?;
    info1.set_constant("Top", ConstantValue::Number(100.0), None)?;
    info1.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info1)?;
    let mut xy = build_xy_sheet("xy-1", "Dose", &["10.0"]);
    xy.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(xy)?;

    let mut lin_res = ResultSheet::new(AnalysisSpec::new("lin-1", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?)?;
    lin_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(lin_res)?;

    let hook_rule = |name: &str, h: HookSource| ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::HookedConstant(h),
        lower_bound: None,
        upper_bound: None,
        initial_override: None,
    };
    let mut nl_fit = NonlinearFitSpec::new("four_pl");
    nl_fit.parameter_rules = vec![
        hook_rule("IC50", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-1")), constant_name: String::from("IC50") }),
        hook_rule("Top", HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Top") }),
        hook_rule("Bottom", HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("xy-1"), data_set_index: 0 }),
    ];
    let mut nl_res = ResultSheet::new(AnalysisSpec::new("fit-hook", "xy-1", AnalysisKind::NonlinearRegression(nl_fit))?)?;
    nl_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(nl_res)?;

    let mut norm_res = ResultSheet::new(AnalysisSpec::new(
        "norm-hook",
        "xy-1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("lin-1"),
                parameter_key: String::from("Slope"),
                data_set_index: Some(0),
            }),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("lin-1"),
                parameter_key: String::from("YIntercept"),
                data_set_index: None,
            }),
            output_percent: true,
            per_data_set: false,
        }),
    )?)?;
    norm_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(norm_res)?;

    for res_id in ["fit-hook", "norm-hook"] {
        for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
            let report = generate_methods_report(&doc, &SheetId::new(res_id), fmt)?;
            assert!(["InfoConstant", "ResultScalar", "ColumnTitleValue", "SheetId(", "Some("].iter().all(|t| !report.contains(t)));
        }
    }
    let nl_plain = generate_methods_report(&doc, &SheetId::new("fit-hook"), ReportFormat::PlainText)?;
    assert!(
        nl_plain.contains("info constant 'IC50' on sheet 'info-1'")
            && nl_plain.contains("info constant 'Top' on paired info sheet")
            && nl_plain.contains("column 1 title on sheet 'xy-1'")
    );
    let norm_plain = generate_methods_report(&doc, &SheetId::new("norm-hook"), ReportFormat::PlainText)?;
    assert!(norm_plain.contains("fitted scalar 'Slope' (dataset 1) from 'lin-1'") && norm_plain.contains("fitted scalar 'YIntercept' from 'lin-1'"));
    Ok(())
}

#[test]
fn fa_sec11_08_all_sheet_schemas_publish_additional_properties_false_and_reject_unknown_keys() -> TestResult {
    let sheet_specs: Vec<_> = cmd::registry().specs().filter(|s| s.id.starts_with("sheet.")).collect();
    assert_eq!(sheet_specs.len(), 15);
    for spec in &sheet_specs {
        assert_eq!((spec.input_schema)().get("additionalProperties"), Some(&json!(false)), "`{}`", spec.id);
    }

    let mut session = Session::new();
    for (c, p) in [
        ("sheet.createData", json!({ "sheet_id": "s1", "title": "Base", "kind": "xy", "rows": 2, "cols": 1 })),
        ("table.setCell", json!({ "sheet_id": "s1", "row": 0, "col": 0, "value": 1.0 })),
        ("table.setCell", json!({ "sheet_id": "s1", "row": 1, "col": 0, "value": 2.0 })),
        ("sheet.createData", json!({ "sheet_id": "s2", "title": "Second", "kind": "xy", "rows": 2, "cols": 1 })),
        ("stats.run", json!({ "source_sheet_id": "s1", "sheet_id": "r1", "kind": "descriptive_stats" })),
    ] {
        ok_cmd(&mut session, c, p)?;
    }

    let payloads = [
        ("sheet.createData", json!({ "sheet_id": "s-new", "bogus": 1 })),
        ("sheet.rename", json!({ "sheet_id": "s1", "title": "Renamed", "bogus": 1 })),
        ("sheet.delete", json!({ "sheet_id": "s2", "bogus": 1 })),
        ("sheet.duplicate", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.duplicateFamily", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.clonePipeline", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.freeze", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.unfreeze", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.reorder", json!({ "sheet_id": "s1", "index": 0, "bogus": 1 })),
        ("sheet.select", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.extractResultTab", json!({ "result_sheet_id": "r1", "bogus": 1 })),
        ("sheet.clearOrigin", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.addFloatingNote", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.updateFloatingNote", json!({ "sheet_id": "s1", "note_id": "note-1", "minimized": true, "bogus": 1 })),
        ("sheet.deleteFloatingNote", json!({ "sheet_id": "s1", "note_id": "note-1", "bogus": 1 })),
    ];
    assert_eq!(payloads.len(), sheet_specs.len());
    for (cmd_id, params) in payloads {
        assert_bad_pointer(&mut session, cmd_id, params, "/bogus");
    }

    let renamed = ok_cmd(&mut session, "sheet.rename", json!({ "id": "s1", "new_title": "Aliased" }))?;
    assert!(!renamed.warnings.is_empty() && ds(&session, "s1").title == "Aliased");
    Ok(())
}

#[test]
fn fa_sec11_09_write_output_file_enforces_policy_and_atomic_writes() -> TestResult {
    let (dir, outside_dir) = (TempDir::try_new("write-policy")?, TempDir::try_new("write-outside")?);
    let outside_target = outside_dir.child("escape.txt");

    assert_eq!(
        write_output_file(&ExternalPathPolicy::RejectExternal, &outside_target.to_string_lossy(), b"nope").expect_err("RejectExternal").code,
        ErrorCode::PathEscape
    );
    assert!(!outside_target.exists());

    let wd_policy = ExternalPathPolicy::workdir(&dir.path)?;
    assert_eq!(write_output_file(&wd_policy, "../escape.txt", b"nope").expect_err("traversal").code, ErrorCode::PathEscape);
    write_output_file(&wd_policy, "nested/export.bin", b"atomic-bytes")?;
    assert_eq!(fs::read(dir.child("nested/export.bin"))?, b"atomic-bytes");

    assert_eq!(write_output_file(&ExternalPathPolicy::Unconstrained, "bad\0name.txt", b"x").expect_err("NUL").code, ErrorCode::BadInput);
    assert_eq!(write_output_file(&ExternalPathPolicy::Unconstrained, "   ", b"x").expect_err("empty").code, ErrorCode::BadInput);

    let mut sess = Session::new().with_path_policy(ExternalPathPolicy::RejectExternal);
    ok_cmd(&mut sess, "sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "graph_sheet_id": "g1" }))?;
    assert_eq!(
        err_cmd(&mut sess, "export.svg", json!({ "sheet_id": "g1", "path": dir.child("g1.svg").to_string_lossy() }))?.code,
        ErrorCode::PathEscape
    );
    assert!(!dir.child("g1.svg").exists());
    Ok(())
}

#[test]
fn fa_sec11_10_engine_manifest_excludes_unused_tools_crate() {
    assert!(!include_str!("../../Cargo.toml").contains("curvebook-tools"));
}
