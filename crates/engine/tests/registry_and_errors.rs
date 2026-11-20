//! Integration tests verifying `CommandSpec` registry completeness, `docs/command-ids.md` parity,
//! RFC-6901 parameter validation errors, exhaustive `EngineError` mappings, and deterministic replay.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

use curvebook_chart::ChartError;
use curvebook_doc::{DocError, SheetFolderKind, SheetId, SubcolumnMode, SummaryPointError, TableKind};
use curvebook_engine::cmd::{CommandGroup, Effects};
use curvebook_engine::{Caller, ErrorCode, Session, cmd};
use curvebook_fit::FitError;
use curvebook_format::{FormatError, MemoryJournalBackend};
use curvebook_math::MathError;
use curvebook_pdf::PdfError;
use curvebook_platform::{PlatformError, TimeoutKind};
use curvebook_render::RenderError;
use curvebook_stats::StatsError;
use curvebook_svg::SvgError;
use curvebook_transform::TransformError;
use serde_json::{Value, json};

fn is_valid_command_id_format(id: &str) -> bool {
    let valid_areas = [
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
    let Some((area, action)) = id.split_once('.') else {
        return false;
    };
    if action.contains('.') || area.is_empty() || !area.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    if !valid_areas.contains(&area) {
        return false;
    }
    let mut chars = action.bytes();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase() && chars.all(|b| b.is_ascii_alphanumeric())
}

#[test]
fn registry_matches_command_id_category_effect_and_schema_spec() {
    let built = cmd::build_registry().expect("registry build must succeed with zero duplicates");
    let reg = cmd::registry();
    assert_eq!(built.len(), 105, "expected 105 registered commands");
    assert_eq!(reg.len(), 105);

    let built_ids: BTreeSet<&str> = built.specs().map(|s| s.id).collect();
    let reg_ids: BTreeSet<&str> = reg.specs().map(|s| s.id).collect();
    assert_eq!(built_ids, reg_ids, "cmd::build_registry() IDs must match cmd::registry() 1:1");
    assert_eq!(reg_ids.len(), 105, "command IDs must be unique across all 105 commands");

    let mut group_total = 0usize;
    for &group in CommandGroup::all() {
        assert!(!group.as_str().trim().is_empty(), "empty CommandGroup::as_str");
        assert!(!group.title().trim().is_empty(), "empty CommandGroup::title");
        let group_specs = reg.specs_by_group(group);
        assert!(!group_specs.is_empty(), "group `{}` has zero commands", group.as_str());
        group_total += group_specs.len();
    }
    assert_eq!(group_total, 105, "CommandGroup::all() partition must sum to 105");

    let markdown = cmd::render_commands_markdown();
    assert!(markdown.starts_with("# CurveBook Command Reference\n"), "commands markdown must use canonical `# CurveBook Command Reference` heading");

    for spec in reg.specs() {
        assert!(!spec.label.trim().is_empty(), "command `{}` must have non-empty label", spec.id);
        assert!(!spec.menu_path.trim().is_empty(), "command `{}` must have non-empty menu_path", spec.id);
        assert!(!spec.summary.trim().is_empty(), "command `{}` must have non-empty summary", spec.id);
        assert!(is_valid_command_id_format(spec.id), "command `{}` violates docs/command-ids.md canonical syntax", spec.id);

        assert_eq!(
            spec.effects.bits(),
            Effects::from_bits_truncate(spec.effects.bits()).bits(),
            "command `{}` declares unknown effect bits",
            spec.id
        );
        assert_eq!(
            spec.effects.flag_names().len() as u32,
            spec.effects.bits().count_ones(),
            "command `{}` flag_names() must match set effect bits",
            spec.id
        );
        if spec.effects.contains(Effects::MAY_DISCARD_DATA) {
            assert!(
                spec.effects.intersects(Effects::MUTATES_DOCUMENT | Effects::WRITES_FILESYSTEM),
                "command `{}` sets MAY_DISCARD_DATA without MUTATES_DOCUMENT or WRITES_FILESYSTEM",
                spec.id
            );
        }

        let schema = (spec.input_schema)();
        let obj = schema.as_object().unwrap_or_else(|| panic!("schema for `{}` must be a JSON object", spec.id));
        let schema_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!schema_type.is_empty(), "schema for `{}` must declare non-empty string `type`", spec.id);
        if schema_type == "object" {
            assert!(obj.get("properties").is_none_or(|p| p.is_object()), "schema `properties` for `{}` must be a JSON object", spec.id);
        }
        assert!(markdown.contains(spec.id), "markdown documentation missing command `{}`", spec.id);
    }
}

fn assert_cmd_bad_pointer(session: &mut Session, command_id: &str, params: Value, expected_pointer: &str) {
    let err = session.execute(command_id, params).expect_err(&format!("`{command_id}` should reject invalid parameters"));
    assert_eq!(err.code, ErrorCode::BadInput, "`{command_id}` expected BadInput, got {:?}", err.code);
    assert_eq!(err.category, ErrorCode::BadInput.category());
    let ptr = err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str);
    assert_eq!(ptr, Some(expected_pointer), "`{command_id}` details: {:?}", err.details);
}

#[test]
fn bad_params_emit_structured_bad_input_with_rfc6901_pointer() {
    let mut session = Session::new();
    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "d1",
                "title": "Dose",
                "archetype": "column",
                "cols": 2,
                "rows": 3,
                "auto_info": true,
                "info_sheet_id": "i1",
                "auto_graph": true,
                "graph_sheet_id": "g1"
            }),
        )
        .expect("seed data+info+graph");
    session
        .execute(
            "table.setCells",
            json!({ "sheet_id": "d1", "cells": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }] }),
        )
        .expect("seed data cells");
    session.execute("layout.create", json!({ "sheet_id": "l1", "title": "Layout" })).expect("seed layout");
    session.execute("stats.run", json!({ "sheet_id": "r1", "source_sheet": "d1", "kind": "descriptive" })).expect("seed result");

    assert_cmd_bad_pointer(&mut session, "sheet.createData", json!({ "title": 123 }), "/title");
    assert_cmd_bad_pointer(&mut session, "sheet.rename", json!({ "new_title": "Renamed" }), "/sheet_id");
    assert_cmd_bad_pointer(&mut session, "info.setNote", json!({ "sheet_id": "d1" }), "/text");
    assert_cmd_bad_pointer(&mut session, "info.setConstant", json!({ "sheet_id": "d1", "value": 1 }), "/name");
    assert_cmd_bad_pointer(
        &mut session,
        "table.setCells",
        json!({ "sheet_id": "d1", "cells": [{ "row": "bad", "col": 0, "value": 1.0 }] }),
        "/edits/0/row",
    );
    assert_cmd_bad_pointer(&mut session, "table.setCell", json!({ "sheet_id": "d1", "row": -1, "col": 0, "value": 1.0 }), "/row");
    assert_cmd_bad_pointer(&mut session, "analysis.updateParams", json!({}), "/sheet_id");
    assert_cmd_bad_pointer(&mut session, "graph.setKind", json!({ "sheet_id": "g1" }), "/kind");
    assert_cmd_bad_pointer(&mut session, "layout.addPanel", json!({ "sheet_id": "l1", "row": -2 }), "/row");
    assert_cmd_bad_pointer(&mut session, "export.png", json!({ "sheet_id": "g1", "dpi": "300" }), "/dpi");
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

fn check_doc_error(err: DocError) {
    let expected = expected_doc_code(&err);
    let eng = curvebook_engine::EngineError::from(err);
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
    assert!(!eng.message().is_empty());
}

fn check_format_error(err: FormatError) {
    let expected = match &err {
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
    let eng = curvebook_engine::EngineError::from(err);
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
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

fn check_transform_error(err: TransformError) {
    let expected = match &err {
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
        | TransformError::InvalidInput { .. } => ErrorCode::BadInput,
        TransformError::EvalError { .. } => ErrorCode::SolverFailed,
        TransformError::MissingHook { .. } | TransformError::UnknownEquation { .. } => ErrorCode::NotFound,
    };
    let eng = curvebook_engine::EngineError::from(err);
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
}

fn check_stats_error(err: StatsError) {
    let expected = match &err {
        StatsError::EmptyInput | StatsError::InsufficientSampleSize { .. } | StatsError::ZeroVariance { .. } => ErrorCode::InsufficientData,
        StatsError::InvalidParameter { .. }
        | StatsError::DimensionMismatch { .. }
        | StatsError::RequiresRawReplicates
        | StatsError::IncompatibleTable { .. }
        | StatsError::IncompatibleAnalysisSpec { .. }
        | StatsError::MissingColumn { .. } => ErrorCode::BadInput,
        StatsError::NotConverged { .. } => ErrorCode::SolverFailed,
        StatsError::SummaryPoint(e) => expected_summary_point_code(e),
        StatsError::Math(e) => expected_math_code(e),
        StatsError::Doc(e) => expected_doc_code(e),
    };
    let eng = curvebook_engine::EngineError::from(err);
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
}

fn check_fit_error(err: FitError) {
    let expected = match &err {
        FitError::EmptyInput | FitError::InsufficientData { .. } => ErrorCode::InsufficientData,
        FitError::UnknownEquation { .. } | FitError::HookResolutionError { .. } => ErrorCode::NotFound,
        FitError::InvalidParameter { .. }
        | FitError::IncompatibleTable { .. }
        | FitError::ImpossibleWeights { .. }
        | FitError::RequiresRawReplicates
        | FitError::BadInitialValues { .. }
        | FitError::DslCompileError { .. } => ErrorCode::BadInput,
        FitError::DslEvalError { .. } | FitError::OdeIntegrationFailed { .. } | FitError::ImplicitSolveFailed { .. } => ErrorCode::SolverFailed,
        FitError::Math(e) => expected_math_code(e),
        FitError::Doc(e) => expected_doc_code(e),
    };
    let eng = curvebook_engine::EngineError::from(err);
    assert_eq!(eng.code, expected);
    assert_eq!(eng.category, expected.category());
}

fn all_doc_errors() -> Vec<DocError> {
    let s = SheetId::new("d1");
    vec![
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
    ]
}

fn all_format_errors() -> Vec<FormatError> {
    vec![
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
    ]
}

fn all_transform_errors() -> Vec<TransformError> {
    vec![
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
        TransformError::MissingHook { description: String::from("c1") },
        TransformError::UnknownEquation { equation_id: String::from("eq") },
        TransformError::InvalidInput { message: String::from("nan") },
    ]
}

fn all_stats_and_fit_errors() -> (Vec<StatsError>, Vec<FitError>) {
    let stats = vec![
        StatsError::EmptyInput,
        StatsError::InsufficientSampleSize { context: "anova", required: 3, actual: 1 },
        StatsError::InvalidParameter { name: "alpha", value: 2.0 },
        StatsError::DimensionMismatch { expected: 4, actual: 2 },
        StatsError::RequiresRawReplicates,
        StatsError::IncompatibleTable { analysis: "one_way_anova", actual: TableKind::Xy },
        StatsError::IncompatibleAnalysisSpec { expected: "t_test" },
        StatsError::ZeroVariance { context: "col" },
        StatsError::MissingColumn { index: 5 },
        StatsError::NotConverged { algorithm: "reml" },
        StatsError::SummaryPoint(SummaryPointError::MissingSampleSize),
        StatsError::Math(MathError::DomainError { name: "x", value: -1.0 }),
        StatsError::Doc(DocError::SheetNotFound { sheet_id: SheetId::new("d1") }),
    ];
    let fits = vec![
        FitError::EmptyInput,
        FitError::InsufficientData { required: 4, actual: 2 },
        FitError::UnknownEquation { id: String::from("bad_eq") },
        FitError::InvalidParameter { name: "top", reason: String::from("nan") },
        FitError::IncompatibleTable { kind: TableKind::Column },
        FitError::ImpossibleWeights { reason: String::from("y=0") },
        FitError::RequiresRawReplicates,
        FitError::BadInitialValues { reason: String::from("neg") },
        FitError::DslCompileError { message: String::from("tok"), line: 1, column: 3 },
        FitError::DslEvalError { message: String::from("domain") },
        FitError::OdeIntegrationFailed { reason: String::from("stiff") },
        FitError::ImplicitSolveFailed { reason: String::from("bracket") },
        FitError::HookResolutionError { message: String::from("missing") },
        FitError::Math(MathError::SingularMatrix { rank: 1, expected_rank: 2 }),
        FitError::Doc(DocError::SheetFrozen { sheet_id: SheetId::new("d1") }),
    ];
    (stats, fits)
}

#[test]
fn exhaustive_error_mappings_cover_all_nine_domain_enums() {
    for err in all_doc_errors() {
        check_doc_error(err);
    }
    for err in all_format_errors() {
        check_format_error(err);
    }
    for err in all_transform_errors() {
        check_transform_error(err);
    }
    let (stats, fits) = all_stats_and_fit_errors();
    for err in stats {
        check_stats_error(err);
    }
    for err in fits {
        check_fit_error(err);
    }

    for err in [
        ChartError::SheetNotFound { sheet_id: String::from("g1") },
        ChartError::InvalidSpec { message: String::from("neg") },
        ChartError::ResourceLimit { message: String::from("budget") },
        ChartError::CompileFailed { message: String::from("fail") },
    ] {
        let code = match &err {
            ChartError::SheetNotFound { .. } => ErrorCode::NotFound,
            ChartError::InvalidSpec { .. } => ErrorCode::BadInput,
            ChartError::ResourceLimit { .. } => ErrorCode::ResourceLimit,
            ChartError::CompileFailed { .. } => ErrorCode::InternalError,
        };
        let eng = curvebook_engine::EngineError::from(err);
        assert_eq!(eng.code, code);
        assert_eq!(eng.category, code.category());
    }

    for err in [SvgError::InvalidScene { message: String::from("bad") }, SvgError::ValidationFailed { message: String::from("xml") }] {
        let code = match &err {
            SvgError::InvalidScene { .. } => ErrorCode::BadInput,
            SvgError::ValidationFailed { .. } => ErrorCode::InternalError,
        };
        let eng = curvebook_engine::EngineError::from(err);
        assert_eq!(eng.code, code);
        assert_eq!(eng.category, code.category());
    }

    for err in [
        PdfError::EmptyDocument,
        PdfError::InvalidScene { message: String::from("bad") },
        PdfError::ResourceLimit { message: String::from("cap") },
        PdfError::CompressionFailed { message: String::from("zlib") },
    ] {
        let code = match &err {
            PdfError::EmptyDocument | PdfError::InvalidScene { .. } => ErrorCode::BadInput,
            PdfError::ResourceLimit { .. } => ErrorCode::ResourceLimit,
            PdfError::CompressionFailed { .. } => ErrorCode::InternalError,
        };
        let eng = curvebook_engine::EngineError::from(err);
        assert_eq!(eng.code, code);
        assert_eq!(eng.category, code.category());
    }

    for err in [
        RenderError::InvalidTarget { message: String::from("0x0") },
        RenderError::BufferSizeMismatch { expected: 16, actual: 4 },
        RenderError::EncodingFailed { message: String::from("png") },
    ] {
        let code = match &err {
            RenderError::InvalidTarget { .. } | RenderError::BufferSizeMismatch { .. } => ErrorCode::BadInput,
            RenderError::EncodingFailed { .. } => ErrorCode::InternalError,
        };
        let eng = curvebook_engine::EngineError::from(err);
        assert_eq!(eng.code, code);
        assert_eq!(eng.category, code.category());
    }

    assert_eq!(
        curvebook_engine::EngineError::from(PlatformError::TimedOut { program: String::from("rscript"), kind: TimeoutKind::Idle, limit_ms: 250 })
            .code,
        ErrorCode::IoError
    );
}

#[test]
fn deterministic_replay_parity_between_live_wal_and_batch_v1() {
    let batch_manifest = json!({
        "schema": "curvebook.batch.v1",
        "commands": [
            { "cmd": "project.new", "params": { "title": "Replay Parity Study" } },
            {
                "cmd": "sheet.createData",
                "params": {
                    "sheet_id": "d1",
                    "title": "Dose",
                    "archetype": "column",
                    "cols": 2,
                    "rows": 3,
                    "auto_info": true,
                    "info_sheet_id": "i1"
                }
            },
            { "cmd": "info.setConstant", "params": { "sheet_id": "d1", "name": "Unit", "value": "uM" } },
            {
                "cmd": "table.setCells",
                "params": {
                    "sheet_id": "d1",
                    "cells": [
                        { "row": 0, "col": 0, "value": 10.0 },
                        { "row": 1, "col": 0, "value": 20.0 },
                        { "row": 2, "col": 0, "value": 30.0 },
                        { "row": 0, "col": 1, "value": 25.0 },
                        { "row": 1, "col": 1, "value": 35.0 },
                        { "row": 2, "col": 1, "value": 45.0 }
                    ]
                }
            },
            { "cmd": "table.setGroupTitles", "params": { "sheet_id": "d1", "titles": ["Control", "Treated"] } },
            { "cmd": "stats.run", "params": { "sheet_id": "r1", "source_sheet": "d1", "kind": "unpaired_t" } },
            { "cmd": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" } },
            {
                "cmd": "graph.setAdditionalTicks",
                "params": {
                    "sheet_id": "g1",
                    "axis": "y",
                    "ticks": [{ "value": 25.0, "label": "Cutoff25", "draw_gridline": true }]
                }
            },
            { "cmd": "layout.create", "params": { "sheet_id": "l1", "title": "Figure 1" } }
        ]
    });
    let commands = batch_manifest["commands"].as_array().expect("batch commands array");

    // 1. Live execution recording every mutating step to CrashRecoveryJournal WAL.
    let backend = MemoryJournalBackend::new();
    let (mut live, mut journal, init_report) = Session::open_with_recovery(backend, "live-client", 5_000, 1_000).expect("open WAL session");
    assert!(init_report.recovered_commands.is_empty());
    for entry in commands {
        let cmd_id = entry["cmd"].as_str().expect("cmd id");
        let params = entry["params"].clone();
        live.execute_and_record(&mut journal, cmd_id, params).unwrap_or_else(|err| panic!("live `{cmd_id}` failed: {err:?}"));
    }

    // 2. WAL replay onto a clean session from abandoned journal state.
    let abandoned = journal.abandon();
    let web_json = abandoned.to_web_storage_json().expect("serialize WAL storage");
    let restored_backend = MemoryJournalBackend::from_web_storage_json(&web_json).expect("restore WAL storage");
    let (wal_replayed, j2, rec_report) = Session::open_with_recovery(restored_backend, "replay-client", 5_000, 7_000).expect("WAL replay");
    assert_eq!(rec_report.recovered_commands.len(), commands.len());
    drop(j2);

    // 3. Sequential execution of the curvebook.batch.v1 manifest via Caller::Cli.
    let mut batch_session = Session::new();
    let cancelled = AtomicBool::new(false);
    for entry in commands {
        let cmd_id = entry["cmd"].as_str().expect("cmd id");
        let params = entry["params"].clone();
        batch_session.execute_with_caller(cmd_id, params, Caller::Cli, &cancelled).unwrap_or_else(|err| panic!("batch `{cmd_id}` failed: {err:?}"));
    }

    let live_json = serde_json::to_value(live.document()).expect("serialize live doc");
    let wal_json = serde_json::to_value(wal_replayed.document()).expect("serialize WAL doc");
    let batch_json = serde_json::to_value(batch_session.document()).expect("serialize batch doc");

    assert_eq!(wal_json, live_json, "WAL replay document diverged from live session");
    assert_eq!(batch_json, live_json, "batch replay document diverged from live session");
}
