//! Integration tests verifying `CommandSpec` registry completeness, `docs/command-ids.md` parity,
//! RFC-6901 parameter validation errors, exhaustive `EngineError` mappings, and deterministic replay.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

use curvebook_chart::ChartError;
use curvebook_doc::{DocError, SheetFolderKind, SheetId, SubcolumnMode, SummaryPointError, TableKind};
use curvebook_engine::cmd::{CommandGroup, Effects};
use curvebook_engine::{Caller, EngineError, ErrorCode, Session, cmd};
use curvebook_fit::FitError;
use curvebook_format::{FormatError, MemoryJournalBackend};
use curvebook_math::MathError;
use curvebook_pdf::PdfError;
use curvebook_platform::{PlatformError, TimeoutKind};
use curvebook_render::RenderError;
use curvebook_stats::StatsError;
use curvebook_svg::SvgError;
use curvebook_transform::TransformError;
use serde_json::json;

use super::common::{assert_bad_pointer, run_cmds};

fn is_valid_command_id_format(id: &str) -> bool {
    const AREAS: &[&str] = &[
        "project",
        "file",
        "edit",
        "sheet",
        "table",
        "info",
        "analyze",
        "transform",
        "stats",
        "fit",
        "analysis",
        "result",
        "graph",
        "layout",
        "export",
    ];
    let Some((area, action)) = id.split_once('.') else { return false };
    let mut chars = action.bytes();
    let Some(first) = chars.next() else { return false };
    !action.contains('.')
        && !area.is_empty()
        && area.bytes().all(|b| b.is_ascii_lowercase())
        && AREAS.contains(&area)
        && first.is_ascii_lowercase()
        && chars.all(|b| b.is_ascii_alphanumeric())
}

#[test]
fn registry_matches_command_id_category_effect_and_schema_spec() {
    let built = cmd::build_registry().expect("registry build must succeed with zero duplicates");
    let reg = cmd::registry();
    assert_eq!(built.len(), 110, "expected 110 registered commands");
    assert_eq!(reg.len(), 110);

    let built_ids: BTreeSet<&str> = built.specs().map(|s| s.id).collect();
    let reg_ids: BTreeSet<&str> = reg.specs().map(|s| s.id).collect();
    assert_eq!(built_ids, reg_ids, "cmd::build_registry() IDs must match cmd::registry() 1:1");
    assert_eq!(reg_ids.len(), 110, "command IDs must be unique across all 110 commands");

    let mut group_total = 0usize;
    for &group in CommandGroup::all() {
        assert!(!group.as_str().trim().is_empty() && !group.title().trim().is_empty());
        let group_specs = reg.specs_by_group(group);
        assert!(!group_specs.is_empty(), "group `{}` has zero commands", group.as_str());
        group_total += group_specs.len();
    }
    assert_eq!(group_total, 110, "CommandGroup::all() partition must sum to 110");

    let markdown = cmd::render_commands_markdown();
    assert!(markdown.starts_with("# CurveBook Command Reference\n"));

    for spec in reg.specs() {
        assert!(!spec.label.trim().is_empty() && !spec.menu_path.trim().is_empty() && !spec.summary.trim().is_empty(), "{}", spec.id);
        assert!(is_valid_command_id_format(spec.id), "command `{}` violates docs/command-ids.md canonical syntax", spec.id);
        assert_eq!(spec.effects.bits(), Effects::from_bits_truncate(spec.effects.bits()).bits(), "{}", spec.id);
        assert_eq!(spec.effects.flag_names().len() as u32, spec.effects.bits().count_ones(), "{}", spec.id);
        if spec.effects.contains(Effects::MAY_DISCARD_DATA) {
            assert!(spec.effects.intersects(Effects::MUTATES_DOCUMENT | Effects::WRITES_FILESYSTEM), "{}", spec.id);
        }

        let schema = (spec.input_schema)();
        let obj = schema.as_object().unwrap_or_else(|| panic!("schema for `{}` must be a JSON object", spec.id));
        let schema_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!schema_type.is_empty(), "schema for `{}` must declare non-empty string `type`", spec.id);
        if schema_type == "object" {
            assert!(obj.get("properties").is_none_or(|p| p.is_object()), "schema `properties` for `{}` must be object", spec.id);
        }
        assert!(markdown.contains(spec.id), "markdown documentation missing command `{}`", spec.id);
    }
}

#[test]
fn bad_params_emit_structured_bad_input_with_rfc6901_pointer() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "d1", "title": "Dose", "archetype": "column", "cols": 2, "rows": 3, "auto_info": true, "info_sheet_id": "i1", "auto_graph": true, "graph_sheet_id": "g1" }),
            ),
            ("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }] })),
            ("layout.create", json!({ "sheet_id": "l1", "title": "Layout" })),
            ("stats.run", json!({ "sheet_id": "r1", "source_sheet_id": "d1", "kind": "descriptive" })),
        ],
    );

    for (cmd_id, params, ptr) in [
        ("sheet.createData", json!({ "title": 123 }), "/title"),
        ("sheet.rename", json!({ "new_title": "Renamed" }), "/sheet_id"),
        ("info.setNote", json!({ "sheet_id": "d1" }), "/text"),
        ("info.setConstant", json!({ "sheet_id": "d1", "value": 1 }), "/name"),
        ("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": "bad", "col": 0, "value": 1.0 }] }), "/edits/0/row"),
        ("table.setCell", json!({ "sheet_id": "d1", "row": -1, "col": 0, "value": 1.0 }), "/row"),
        ("analysis.updateParams", json!({}), "/sheet_id"),
        ("graph.setKind", json!({ "sheet_id": "g1" }), "/kind"),
        ("layout.addPanel", json!({ "sheet_id": "l1", "row": -2 }), "/row"),
        ("export.png", json!({ "sheet_id": "g1", "dpi": "300" }), "/dpi"),
    ] {
        assert_bad_pointer(&mut session, cmd_id, params, ptr);
    }
}

fn check_mapped<E: Into<EngineError>>(err: E, expected: ErrorCode) {
    let eng = err.into();
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
    assert!(!eng.message().is_empty());
}

fn expected_doc_code(err: &DocError) -> ErrorCode {
    match err {
        DocError::EmptyIdentifier
        | DocError::InvalidIdentifier { .. }
        | DocError::InvalidSubcolumnCount { .. }
        | DocError::IncompatibleSubcolumnMode { .. }
        | DocError::CellOutOfBounds { .. }
        | DocError::InvalidCellValue { .. }
        | DocError::SheetIndexOutOfBounds { .. }
        | DocError::InvalidConfidenceLevel { .. }
        | DocError::InvalidAxisSegments { .. }
        | DocError::IncompatibleRetargetTable { .. } => ErrorCode::BadInput,
        DocError::TooManyRows { .. }
        | DocError::TooManyDataSets { .. }
        | DocError::TooManySubcolumns { .. }
        | DocError::TooManyCells { .. }
        | DocError::TooManyLayoutItems { .. }
        | DocError::TooManySheets { .. }
        | DocError::TooManyImageAssets { .. }
        | DocError::TitleTooLong { .. } => ErrorCode::ResourceLimit,
        DocError::SheetFrozen { .. } => ErrorCode::FrozenSheet,
        DocError::SheetNotFound { .. } => ErrorCode::NotFound,
        DocError::DuplicateSheetId { .. } => ErrorCode::AlreadyExists,
    }
}

fn expected_summary_point_code(err: &SummaryPointError) -> ErrorCode {
    match err {
        SummaryPointError::NotParametricSummaryMode { .. } | SummaryPointError::NegativeSpread { .. } => ErrorCode::BadInput,
        SummaryPointError::MissingSpread | SummaryPointError::MissingSampleSize | SummaryPointError::SampleSizeBelowOne { .. } => {
            ErrorCode::InsufficientData
        }
    }
}

fn expected_math_code(err: &MathError) -> ErrorCode {
    match err {
        MathError::EmptyInput => ErrorCode::InsufficientData,
        MathError::DimensionMismatch { .. } | MathError::InvalidParameter { .. } | MathError::DomainError { .. } => ErrorCode::BadInput,
        MathError::SingularMatrix { .. } | MathError::NotPositiveDefinite { .. } | MathError::NotConverged { .. } => ErrorCode::SolverFailed,
    }
}

#[test]
fn exhaustive_error_mappings_cover_all_nine_domain_enums() {
    let s = SheetId::new("d1");
    for err in [
        DocError::EmptyIdentifier,
        DocError::TooManyRows { count: 10, max: 5 },
        DocError::TooManyDataSets { count: 10, max: 5 },
        DocError::TooManySubcolumns { count: 10, max: 5 },
        DocError::TooManyCells { count: 50_000_000, max: 30_000_000 },
        DocError::TooManyLayoutItems { count: 65, max: 64 },
        DocError::TooManySheets { count: 10, max: 5 },
        DocError::TooManyImageAssets { count: 257, max: 256 },
        DocError::TitleTooLong { byte_len: 300, max: 128 },
        DocError::InvalidSubcolumnCount { count: 0 },
        DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, mode: SubcolumnMode::Single },
        DocError::SheetFrozen { sheet_id: s.clone() },
        DocError::CellOutOfBounds { row: 1, col: 2, subcol: 0 },
        DocError::InvalidCellValue { archetype: TableKind::Column, row: 0, col: 0, reason: "bad" },
        DocError::SheetNotFound { sheet_id: s.clone() },
        DocError::SheetIndexOutOfBounds { folder: SheetFolderKind::Data, index: 3, len: 1 },
        DocError::DuplicateSheetId { sheet_id: s },
        DocError::InvalidConfidenceLevel { percent: 101.0 },
        DocError::InvalidAxisSegments { count: 4 },
        DocError::IncompatibleRetargetTable { analysis_title: String::from("Fit"), target_kind: TableKind::Column },
    ] {
        let code = expected_doc_code(&err);
        check_mapped(err, code);
    }

    for err in [
        FormatError::Io(String::from("io")),
        FormatError::Json(String::from("json")),
        FormatError::Doc(DocError::EmptyIdentifier),
        FormatError::InvalidZip { reason: String::from("zip") },
        FormatError::CorruptCrc { entry: String::from("doc.json"), expected: 1, actual: 2 },
        FormatError::UnsupportedCompression { entry: String::from("doc.json"), method: 99 },
        FormatError::ZipBomb { reason: String::from("ratio") },
        FormatError::FileTooLarge { path: String::from("big.csv"), actual_bytes: Some(2048), limit_bytes: 1024 },
        FormatError::UnsafePath { path: String::from("../x") },
        FormatError::DuplicateEntry { path: String::from("doc.json") },
        FormatError::MissingArchiveEntry { path: String::from("doc.json") },
        FormatError::UnsupportedSchemaVersion { version: 99, max_supported: 3 },
        FormatError::InvalidArchiveKind { expected: "project", found: "template" },
        FormatError::DependencyCycle { cycle: vec![SheetId::new("a"), SheetId::new("b")] },
        FormatError::NonFiniteFloat { location: String::from("y[0]") },
        FormatError::CsvParse { line: 3, reason: String::from("quote") },
        FormatError::XmlParse { offset: 12, reason: String::from("eof") },
        FormatError::XmlDepthExceeded { depth: 200, max: 128 },
        FormatError::UnsupportedFormat { reason: String::from("unknown") },
        FormatError::InvalidPipeline { reason: String::from("bad") },
    ] {
        let code = match &err {
            FormatError::Io(_) => ErrorCode::IoError,
            FormatError::Json(_)
            | FormatError::InvalidZip { .. }
            | FormatError::CorruptCrc { .. }
            | FormatError::MissingArchiveEntry { .. }
            | FormatError::NonFiniteFloat { .. }
            | FormatError::CsvParse { .. }
            | FormatError::XmlParse { .. } => ErrorCode::CorruptData,
            FormatError::Doc(inner) => expected_doc_code(inner),
            FormatError::UnsupportedCompression { .. } | FormatError::InvalidArchiveKind { .. } | FormatError::UnsupportedFormat { .. } => {
                ErrorCode::UnsupportedFormat
            }
            FormatError::ZipBomb { .. } | FormatError::FileTooLarge { .. } | FormatError::XmlDepthExceeded { .. } => ErrorCode::ResourceLimit,
            FormatError::UnsafePath { .. } => ErrorCode::PathEscape,
            FormatError::DuplicateEntry { .. } => ErrorCode::AlreadyExists,
            FormatError::UnsupportedSchemaVersion { .. } => ErrorCode::UnsupportedVersion,
            FormatError::DependencyCycle { .. } => ErrorCode::CycleDetected,
            FormatError::InvalidPipeline { .. } => ErrorCode::BadInput,
        };
        check_mapped(err, code);
    }

    for err in [
        TransformError::Doc(DocError::EmptyIdentifier),
        TransformError::Math(MathError::EmptyInput),
        TransformError::SummaryPoint(SummaryPointError::MissingSpread),
        TransformError::CircularDependency { cycle: vec![String::from("a"), String::from("b")] },
        TransformError::ParseError { position: 2, message: String::from("syntax") },
        TransformError::EvalError { message: String::from("div0") },
        TransformError::UnknownVariable { name: String::from("x") },
        TransformError::UnknownFunction { name: String::from("foo") },
        TransformError::ArityMismatch { function: String::from("log"), expected: String::from("1"), actual: 2 },
        TransformError::InvalidSpec { message: String::from("bad") },
        TransformError::IncompatibleTable { operation: "row_means", kind: TableKind::Xy },
        TransformError::UnsupportedSubcolumnMode {
            operation: "Transpose",
            table_kind: TableKind::Column,
            mode: SubcolumnMode::MeanSdN,
            reason: "pre-summarized column transpose",
        },
        TransformError::MissingHook { description: String::from("c1") },
        TransformError::UnknownEquation { equation_id: String::from("eq") },
        TransformError::InvalidInput { message: String::from("nan") },
    ] {
        let code = match &err {
            TransformError::Doc(e) => expected_doc_code(e),
            TransformError::Math(e) => expected_math_code(e),
            TransformError::SummaryPoint(e) => expected_summary_point_code(e),
            TransformError::CircularDependency { .. } => ErrorCode::CycleDetected,
            TransformError::ParseError { .. }
            | TransformError::UnknownVariable { .. }
            | TransformError::UnknownFunction { .. }
            | TransformError::ArityMismatch { .. }
            | TransformError::InvalidSpec { .. }
            | TransformError::IncompatibleTable { .. }
            | TransformError::UnsupportedSubcolumnMode { .. }
            | TransformError::InvalidInput { .. } => ErrorCode::BadInput,
            TransformError::EvalError { .. } => ErrorCode::SolverFailed,
            TransformError::MissingHook { .. } | TransformError::UnknownEquation { .. } => ErrorCode::NotFound,
        };
        check_mapped(err, code);
    }

    for err in [
        StatsError::EmptyInput,
        StatsError::InsufficientSampleSize { context: "anova", required: 3, actual: 1 },
        StatsError::InvalidParameter { name: "alpha", value: 2.0 },
        StatsError::DimensionMismatch { expected: 4, actual: 2 },
        StatsError::RequiresRawReplicates,
        StatsError::UnsupportedSubcolumnMode {
            analysis: "Two-group t-test",
            table_kind: TableKind::Column,
            mode: SubcolumnMode::MeanSd,
            reason: "missing N",
        },
        StatsError::IncompatibleTable { analysis: "one_way_anova", actual: TableKind::Xy },
        StatsError::IncompatibleAnalysisSpec { expected: "t_test" },
        StatsError::ZeroVariance { context: "col" },
        StatsError::MissingColumn { index: 5 },
        StatsError::NotConverged { algorithm: "reml" },
        StatsError::SummaryPoint(SummaryPointError::MissingSampleSize),
        StatsError::Math(MathError::DomainError { name: "x", value: -1.0 }),
        StatsError::Doc(DocError::SheetNotFound { sheet_id: SheetId::new("d1") }),
    ] {
        let code = match &err {
            StatsError::EmptyInput | StatsError::InsufficientSampleSize { .. } | StatsError::ZeroVariance { .. } => ErrorCode::InsufficientData,
            StatsError::InvalidParameter { .. }
            | StatsError::DimensionMismatch { .. }
            | StatsError::RequiresRawReplicates
            | StatsError::UnsupportedSubcolumnMode { .. }
            | StatsError::IncompatibleTable { .. }
            | StatsError::IncompatibleAnalysisSpec { .. }
            | StatsError::MissingColumn { .. } => ErrorCode::BadInput,
            StatsError::NotConverged { .. } => ErrorCode::SolverFailed,
            StatsError::SummaryPoint(e) => expected_summary_point_code(e),
            StatsError::Math(e) => expected_math_code(e),
            StatsError::Doc(e) => expected_doc_code(e),
        };
        check_mapped(err, code);
    }

    for err in [
        FitError::EmptyInput,
        FitError::InsufficientData { required: 4, actual: 2 },
        FitError::UnknownEquation { id: String::from("bad_eq") },
        FitError::InvalidParameter { name: "top", reason: String::from("nan") },
        FitError::IncompatibleTable { kind: TableKind::Column },
        FitError::ImpossibleWeights { reason: String::from("y=0") },
        FitError::RequiresRawReplicates,
        FitError::UnsupportedSubcolumnMode {
            analysis: "Nonlinear regression",
            table_kind: TableKind::Xy,
            mode: SubcolumnMode::MeanSd,
            reason: "missing N",
        },
        FitError::BadInitialValues { reason: String::from("neg") },
        FitError::DslCompileError { message: String::from("tok"), line: 1, column: 3 },
        FitError::DslEvalError { message: String::from("domain") },
        FitError::OdeIntegrationFailed { reason: String::from("stiff") },
        FitError::ImplicitSolveFailed { reason: String::from("bracket") },
        FitError::HookResolutionError { message: String::from("missing") },
        FitError::Math(MathError::SingularMatrix { rank: 1, expected_rank: 2 }),
        FitError::Doc(DocError::SheetFrozen { sheet_id: SheetId::new("d1") }),
    ] {
        let code = match &err {
            FitError::EmptyInput | FitError::InsufficientData { .. } => ErrorCode::InsufficientData,
            FitError::UnknownEquation { .. } | FitError::HookResolutionError { .. } => ErrorCode::NotFound,
            FitError::InvalidParameter { .. }
            | FitError::IncompatibleTable { .. }
            | FitError::ImpossibleWeights { .. }
            | FitError::RequiresRawReplicates
            | FitError::UnsupportedSubcolumnMode { .. }
            | FitError::BadInitialValues { .. }
            | FitError::DslCompileError { .. } => ErrorCode::BadInput,
            FitError::DslEvalError { .. } | FitError::OdeIntegrationFailed { .. } | FitError::ImplicitSolveFailed { .. } => ErrorCode::SolverFailed,
            FitError::Math(e) => expected_math_code(e),
            FitError::Doc(e) => expected_doc_code(e),
        };
        check_mapped(err, code);
    }

    for (err, code) in [
        (ChartError::SheetNotFound { sheet_id: String::from("g1") }, ErrorCode::NotFound),
        (ChartError::InvalidSpec { message: String::from("neg") }, ErrorCode::BadInput),
        (ChartError::ResourceLimit { message: String::from("budget") }, ErrorCode::ResourceLimit),
        (ChartError::CompileFailed { message: String::from("fail") }, ErrorCode::InternalError),
    ] {
        check_mapped(err, code);
    }
    for (err, code) in [
        (SvgError::InvalidScene { message: String::from("bad") }, ErrorCode::BadInput),
        (SvgError::ValidationFailed { message: String::from("xml") }, ErrorCode::InternalError),
    ] {
        check_mapped(err, code);
    }
    for (err, code) in [
        (PdfError::EmptyDocument, ErrorCode::BadInput),
        (PdfError::InvalidScene { message: String::from("bad") }, ErrorCode::BadInput),
        (PdfError::ResourceLimit { message: String::from("cap") }, ErrorCode::ResourceLimit),
        (PdfError::CompressionFailed { message: String::from("zlib") }, ErrorCode::InternalError),
    ] {
        check_mapped(err, code);
    }
    for (err, code) in [
        (RenderError::InvalidTarget { message: String::from("0x0") }, ErrorCode::BadInput),
        (RenderError::BufferSizeMismatch { expected: 16, actual: 4 }, ErrorCode::BadInput),
        (RenderError::EncodingFailed { message: String::from("png") }, ErrorCode::InternalError),
    ] {
        check_mapped(err, code);
    }
    check_mapped(PlatformError::TimedOut { program: String::from("rscript"), kind: TimeoutKind::Idle, limit_ms: 250 }, ErrorCode::IoError);
}

#[test]
fn deterministic_replay_parity_between_live_wal_and_batch_v1() {
    let batch_manifest = json!({
        "schema": "curvebook.batch.v1",
        "commands": [
            { "cmd": "project.new", "params": { "title": "Replay Parity Study" } },
            { "cmd": "sheet.createData", "params": { "sheet_id": "d1", "title": "Dose", "archetype": "column", "cols": 2, "rows": 3, "auto_info": true, "info_sheet_id": "i1" } },
            { "cmd": "info.setConstant", "params": { "sheet_id": "d1", "name": "Unit", "value": "uM" } },
            { "cmd": "table.setCells", "params": { "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }, { "row": 2, "col": 0, "value": 30.0 }, { "row": 0, "col": 1, "value": 25.0 }, { "row": 1, "col": 1, "value": 35.0 }, { "row": 2, "col": 1, "value": 45.0 }] } },
            { "cmd": "table.setGroupTitles", "params": { "sheet_id": "d1", "titles": ["Control", "Treated"] } },
            { "cmd": "stats.run", "params": { "sheet_id": "r1", "source_sheet_id": "d1", "kind": "unpaired_t" } },
            { "cmd": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" } },
            { "cmd": "graph.setAdditionalTicks", "params": { "sheet_id": "g1", "axis": "y", "ticks": [{ "value": 25.0, "label": "Cutoff25", "draw_gridline": true }] } },
            { "cmd": "layout.create", "params": { "sheet_id": "l1", "title": "Figure 1" } }
        ]
    });
    let commands = batch_manifest["commands"].as_array().expect("batch commands array");

    // 1. Live execution recording every mutating step to CrashRecoveryJournal WAL.
    let (mut live, mut journal, init_report) =
        Session::open_with_recovery(MemoryJournalBackend::new(), "live-client", 5_000, 1_000).expect("open WAL");
    assert!(init_report.recovered_commands.is_empty());
    for entry in commands {
        let cmd_id = entry["cmd"].as_str().expect("cmd id");
        live.execute_and_record(&mut journal, cmd_id, entry["params"].clone()).unwrap_or_else(|e| panic!("live `{cmd_id}` failed: {e:?}"));
    }

    // 2. WAL replay onto a clean session from abandoned journal state.
    let web_json = journal.abandon().to_web_storage_json().expect("serialize WAL storage");
    let restored = MemoryJournalBackend::from_web_storage_json(&web_json).expect("restore WAL storage");
    let (wal_replayed, j2, rec_report) = Session::open_with_recovery(restored, "replay-client", 5_000, 7_000).expect("WAL replay");
    assert_eq!(rec_report.recovered_commands.len(), commands.len());
    drop(j2);

    // 3. Sequential execution of the curvebook.batch.v1 manifest via Caller::Cli.
    let mut batch_session = Session::new();
    let cancelled = AtomicBool::new(false);
    for entry in commands {
        let cmd_id = entry["cmd"].as_str().expect("cmd id");
        batch_session
            .execute_with_caller(cmd_id, entry["params"].clone(), Caller::Cli, &cancelled)
            .unwrap_or_else(|e| panic!("batch `{cmd_id}` failed: {e:?}"));
    }

    let live_json = serde_json::to_value(live.document()).expect("serialize live doc");
    assert_eq!(serde_json::to_value(wal_replayed.document()).expect("WAL doc"), live_json, "WAL replay diverged from live session");
    assert_eq!(serde_json::to_value(batch_session.document()).expect("batch doc"), live_json, "batch replay diverged from live session");
}
