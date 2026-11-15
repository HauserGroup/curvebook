//! Verification tests for structured L1 error codes, `RemediationKind`, and Draft 2020-12 JSON Schema helpers (`C2`).

use lumencraft_doc::layout::{LayoutItemLimitError, MAX_LAYOUT_ITEMS};
use lumencraft_doc::{
    Cell, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ContingencyTable, DRAFT_2020_12_SCHEMA_URI, DataSetColumn, DataSheet,
    DocError, ErrorCategory, ErrorCode, GroupedTable, JournalWidthPreset, JsonSchema, LayoutItem, LayoutSpec, MAX_SUBCOLUMNS, MultipleVariablesTable,
    NestedTable, PROJECT_DOCUMENT_SCHEMA_ID, PartsOfWholeTable, ProjectDocument, RectPt, RemediationKind, ResultStatus, SequenceProgression, SheetId,
    SubcolumnMode, SurvivalTable, TableArchetype, VariableColumn, XColumnMode, XyTable, array_schema, bool_schema, enum_schema, integer_schema,
    number_schema, object_schema, project_document_schema, string_schema, transpose_cell_grid,
};
use serde_json::json;

#[test]
fn error_code_canonical_roundtrip_and_categories() {
    for code in ErrorCode::ALL {
        let s = code.as_str();
        assert_eq!(code.to_string(), s);
        let json = serde_json::to_value(code).expect("serialize ErrorCode");
        assert_eq!(json, serde_json::Value::String(s.to_owned()));
        let decoded: ErrorCode = serde_json::from_value(json).expect("deserialize ErrorCode");
        assert_eq!(decoded, code);
        assert_eq!(ErrorCode::from_code_str(s), code);
    }

    assert_eq!(ErrorCode::BadInput.category(), ErrorCategory::Input);
    assert_eq!(ErrorCode::NotFound.category(), ErrorCategory::State);
    assert_eq!(ErrorCode::AlreadyExists.category(), ErrorCategory::State);
    assert_eq!(ErrorCode::Conflict.category(), ErrorCategory::State);
    assert_eq!(ErrorCode::FrozenSheet.category(), ErrorCategory::State);
    assert_eq!(ErrorCode::CycleDetected.category(), ErrorCategory::State);
    assert_eq!(ErrorCode::UnsupportedFormat.category(), ErrorCategory::Format);
    assert_eq!(ErrorCode::UnsupportedVersion.category(), ErrorCategory::Format);
    assert_eq!(ErrorCode::CorruptData.category(), ErrorCategory::Format);
    assert_eq!(ErrorCode::IoError.category(), ErrorCategory::Io);
    assert_eq!(ErrorCode::PathEscape.category(), ErrorCategory::Policy);
    assert_eq!(ErrorCode::ResourceLimit.category(), ErrorCategory::Policy);
    assert_eq!(ErrorCode::GuiOnlyViolation.category(), ErrorCategory::Policy);
    assert_eq!(ErrorCode::SolverFailed.category(), ErrorCategory::Compute);
    assert_eq!(ErrorCode::InsufficientData.category(), ErrorCategory::Compute);
    assert_eq!(ErrorCode::Cancelled.category(), ErrorCategory::Compute);
    assert_eq!(ErrorCode::InternalPanic.category(), ErrorCategory::Internal);
    assert_eq!(ErrorCode::InternalError.category(), ErrorCategory::Internal);
}

#[test]
fn legacy_freeform_result_status_codes_deserialize_cleanly() {
    let analysis_err: ResultStatus = serde_json::from_value(json!({
        "Failed": { "code": "analysis_error", "message": "did not converge" }
    }))
    .expect("legacy analysis_error");
    assert_eq!(analysis_err, ResultStatus::Failed { code: ErrorCode::SolverFailed, message: String::from("did not converge") });

    let missing_source: ResultStatus = serde_json::from_value(json!({
        "Failed": { "code": "missing_source", "message": "missing source table" }
    }))
    .expect("legacy missing_source");
    assert_eq!(missing_source, ResultStatus::Failed { code: ErrorCode::NotFound, message: String::from("missing source table") });

    let unknown_legacy: ResultStatus = serde_json::from_value(json!({
        "Failed": { "code": "E_TEST_LEGACY", "message": "historical snapshot error" }
    }))
    .expect("unknown legacy string maps to InternalError");
    assert_eq!(unknown_legacy, ResultStatus::Failed { code: ErrorCode::InternalError, message: String::from("historical snapshot error") });
}

#[test]
fn checklist_item_remediation_is_backwards_compatible_when_omitted() {
    let legacy_json = json!({
        "assumption": "Equal variances",
        "status": "Violated",
        "diagnostic_metric": "F = 9.1, p = 0.003",
        "explanation": "Variance differs significantly."
    });
    let decoded: ChecklistItem = serde_json::from_value(legacy_json).expect("legacy ChecklistItem without remediation");
    assert_eq!(decoded.remediation, None);

    let reserialized = serde_json::to_value(&decoded).expect("serialize ChecklistItem");
    assert!(reserialized.get("remediation").is_none(), "remediation: None should skip_serializing_if");

    let with_remediation = ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: None,
        explanation: String::from("Non-normal"),
        remediation: Some(RemediationKind::MannWhitneyU),
    };
    let roundtrip: ChecklistItem = serde_json::from_value(serde_json::to_value(&with_remediation).unwrap()).unwrap();
    assert_eq!(roundtrip.remediation, Some(RemediationKind::MannWhitneyU));
}

#[test]
fn schema_helpers_produce_valid_draft_2020_12_json() {
    let node = object_schema(
        vec![
            ("name", string_schema()),
            ("enabled", bool_schema()),
            ("weight", number_schema()),
            ("count", integer_schema()),
            ("tags", array_schema(string_schema())),
            ("mode", enum_schema(&["fast", "accurate"])),
        ],
        &["name", "mode"],
        false,
    );
    let val = serde_json::to_value(&node).expect("serialize SchemaNode");
    assert_eq!(val["type"], "object");
    assert_eq!(val["additionalProperties"], false);
    assert_eq!(val["required"], json!(["name", "mode"]));
    assert_eq!(val["properties"]["enabled"]["type"], "boolean");
    assert_eq!(val["properties"]["weight"]["type"], "number");
    assert_eq!(val["properties"]["count"]["type"], "integer");
    assert_eq!(val["properties"]["tags"]["type"], "array");
    assert_eq!(val["properties"]["tags"]["items"]["type"], "string");
    assert_eq!(val["properties"]["mode"]["enum"], json!(["fast", "accurate"]));

    let doc_schema = project_document_schema();
    assert_eq!(ProjectDocument::json_schema(), doc_schema);
    let doc_val = serde_json::to_value(&doc_schema).expect("serialize ProjectDocument schema");
    assert_eq!(doc_val["$schema"], DRAFT_2020_12_SCHEMA_URI);
    assert_eq!(doc_val["$id"], PROJECT_DOCUMENT_SCHEMA_ID);
    assert_eq!(doc_val["title"], "ProjectDocument");
    assert_eq!(doc_val["type"], "object");
    assert!(doc_val["$defs"]["ErrorCode"]["enum"].as_array().expect("ErrorCode enum").len() == ErrorCode::ALL.len());
    assert!(doc_val["$defs"]["RemediationKind"]["enum"].as_array().expect("RemediationKind enum").len() == RemediationKind::ALL.len());
    assert!(doc_val["$defs"]["ChecklistItem"]["properties"]["remediation"].is_object());
    assert!(doc_val["$defs"]["ResultStatus"]["oneOf"].is_array());
}

#[test]
fn layout_spec_enforces_max_layout_items_in_methods_and_serde() {
    let mut spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    let make_callout = |i: usize| LayoutItem::TextCallout {
        id: format!("c-{i}"),
        markdown: format!("Note {i}"),
        bounds_pt: RectPt::new(10.0, 10.0, 40.0, 20.0),
        font_size_pt: 9.0,
    };

    for i in 0..MAX_LAYOUT_ITEMS {
        spec.add_item(make_callout(i)).expect("items up to MAX_LAYOUT_ITEMS should be accepted");
    }
    assert!(spec.validate_item_count().is_ok());

    let add_err = spec.add_item(make_callout(MAX_LAYOUT_ITEMS)).unwrap_err();
    assert_eq!(add_err, LayoutItemLimitError { count: MAX_LAYOUT_ITEMS + 1, max: MAX_LAYOUT_ITEMS });
    assert_eq!(spec.items.len(), MAX_LAYOUT_ITEMS);

    let at_limit_json = serde_json::to_value(&spec).expect("serialize LayoutSpec at limit");
    let decoded_ok: LayoutSpec = serde_json::from_value(at_limit_json).expect("deserialize LayoutSpec at limit");
    assert_eq!(decoded_ok.items.len(), MAX_LAYOUT_ITEMS);

    let extra_item = LayoutItem::LiveGraph {
        id: String::from("overflow-panel"),
        graph_sheet_id: SheetId::new("g-1"),
        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
        scale_percent: 100.0,
        badge_override: None,
    };
    spec.items.push(extra_item);
    assert_eq!(spec.validate_item_count(), Err(LayoutItemLimitError { count: MAX_LAYOUT_ITEMS + 1, max: MAX_LAYOUT_ITEMS }));

    let over_limit_json = serde_json::to_value(&spec).expect("serialize over-limit LayoutSpec");
    let serde_err = serde_json::from_value::<LayoutSpec>(over_limit_json).unwrap_err();
    assert!(serde_err.to_string().contains(&MAX_LAYOUT_ITEMS.to_string()), "unexpected serde error: {serde_err}");

    for i in (MAX_LAYOUT_ITEMS + 1)..500 {
        spec.items.push(make_callout(i));
    }
    let burst_json = serde_json::to_value(&spec).expect("serialize 500-item LayoutSpec");
    assert!(serde_json::from_value::<LayoutSpec>(burst_json).is_err());
}

#[test]
fn r1_s05_01_datasheet_and_serde_enforce_rectangular_and_subcolumn_invariants() {
    // 1. Ragged MultipleVariables rejected on DataSheet::new and serde deserialization.
    let mut mv = MultipleVariablesTable::new(2, 3).expect("valid mv");
    mv.variables[1].cells.pop();
    let mv_json = serde_json::to_value(&mv).expect("serialize ragged mv");
    assert!(serde_json::from_value::<MultipleVariablesTable>(mv_json.clone()).is_err());
    assert!(matches!(DataSheet::new("mv-rag", "Ragged MV", TableArchetype::MultipleVariables(mv)), Err(DocError::InvalidCellValue { .. })));

    // 2. Ragged XyTable (mismatched x_cells rows, subcolumn width, and non-empty row_titles).
    let mut xy_rows = XyTable::new(SubcolumnMode::Single, 1, 2).expect("valid xy");
    xy_rows.x_cells.pop();
    assert!(serde_json::from_value::<XyTable>(serde_json::to_value(&xy_rows).expect("serialize xy")).is_err());
    assert!(DataSheet::new("xy-rows", "Ragged XY", TableArchetype::Xy(xy_rows)).is_err());

    let mut xy_subs = XyTable::new(SubcolumnMode::Single, 1, 2).expect("valid xy");
    xy_subs.data_sets[0].cells[0].push(Cell::number(9.0));
    assert!(serde_json::from_value::<XyTable>(serde_json::to_value(&xy_subs).expect("serialize xy subs")).is_err());
    assert!(DataSheet::new("xy-subs", "Bad Subcols", TableArchetype::Xy(xy_subs)).is_err());

    let mut xy_titles = XyTable::new(SubcolumnMode::Single, 1, 2).expect("valid xy");
    xy_titles.row_titles = vec![String::from("only-one")];
    assert!(DataSheet::new("xy-titles", "Bad RowTitles", TableArchetype::Xy(xy_titles)).is_err());

    // Empty row_titles (legacy omitted) remains valid, and GeneratedSequence with 0-wide or 1-wide x_cells remains valid.
    let mut xy_seq = XyTable::new(SubcolumnMode::Single, 1, 2).expect("valid xy");
    xy_seq.row_titles.clear();
    xy_seq.x_mode = XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: SequenceProgression::Arithmetic };
    let seq_sheet = DataSheet::new("xy-seq", "Generated XY", TableArchetype::Xy(xy_seq)).expect("1-wide GeneratedSequence");
    let seq_round: DataSheet = serde_json::from_value(serde_json::to_value(&seq_sheet).expect("ser")).expect("de");
    assert_eq!(seq_round.archetype.row_count(), 2);

    // 3. Disallowed SubcolumnMode on Column / Grouped rejected on DataSheet::new and serde.
    let col_bad_json = json!({
        "design_hint": "UnpairedReplicates",
        "subcolumn_mode": { "RawReplicates": 3 },
        "row_titles": ["", ""],
        "row_titles_expanded": false,
        "data_sets": [{ "title": "A", "cells": [[], []] }]
    });
    assert!(serde_json::from_value::<ColumnTable>(col_bad_json).is_err());

    let mut col_bad = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("valid col");
    col_bad.subcolumn_mode = SubcolumnMode::RawReplicates(3);
    assert!(matches!(DataSheet::new("col-bad", "Bad Column Mode", TableArchetype::Column(col_bad)), Err(DocError::IncompatibleSubcolumnMode { .. })));

    let mut grp_bad = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).expect("valid grp");
    grp_bad.subcolumn_mode = SubcolumnMode::Single;
    assert!(serde_json::from_value::<GroupedTable>(serde_json::to_value(&grp_bad).expect("ser grp")).is_err());
    assert!(matches!(
        DataSheet::new("grp-bad", "Bad Grouped Mode", TableArchetype::Grouped(grp_bad)),
        Err(DocError::IncompatibleSubcolumnMode { .. })
    ));

    // 4. Ragged Grouped row_numeric_spacing, Contingency, Survival, PartsOfWhole, Nested rejected.
    let mut grp_sp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 1, 2).expect("valid grp");
    grp_sp.row_numeric_spacing = Some(vec![Some(1.0)]);
    assert!(DataSheet::new("grp-sp", "Bad Spacing", TableArchetype::Grouped(grp_sp)).is_err());

    let mut cont = ContingencyTable::new(2, 2).expect("valid contingency");
    cont.data_sets[1].cells.push(vec![Cell::number(1.0)]);
    assert!(DataSheet::new("cont-rag", "Ragged Cont", TableArchetype::Contingency(cont)).is_err());

    let mut surv = SurvivalTable::new(1, 2).expect("valid surv");
    surv.times.pop();
    assert!(DataSheet::new("surv-rag", "Ragged Surv", TableArchetype::Survival(surv)).is_err());

    let mut pow = PartsOfWholeTable::new(1, 2).expect("valid pow");
    pow.slice_labels = vec![String::from("OnlyOne")];
    assert!(DataSheet::new("pow-rag", "Bad Labels", TableArchetype::PartsOfWhole(pow)).is_err());

    let mut nested = NestedTable::new(1, 2, 2).expect("valid nested");
    nested.groups[0].cells[1].pop();
    assert!(DataSheet::new("nest-rag", "Ragged Nested", TableArchetype::Nested(nested)).is_err());

    // 5. DataSheet JSON payload with ragged dataset rejects cleanly during serde::Deserialize.
    let forged_sheet = json!({
        "id": "forged",
        "title": "Forged",
        "archetype": {
            "MultipleVariables": {
                "row_titles": ["r1", "r2"],
                "row_titles_expanded": false,
                "variables": [
                    { "title": "A", "role": "Continuous", "source": "Entered", "cells": [{"value":"Empty","excluded":false}, {"value":"Empty","excluded":false}], "category_order": null },
                    { "title": "B", "role": "Continuous", "source": "Entered", "cells": [], "category_order": null }
                ]
            }
        }
    });
    assert!(serde_json::from_value::<DataSheet>(forged_sheet).is_err());
    let _ = (DataSetColumn::new("T", 0, 1).expect("empty col"), VariableColumn::new("V", 0).expect("empty var"));
}

#[test]
fn r1_s05_03_x_column_mode_validated_on_ctor_and_serde() {
    let zero_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(0) };
    assert!(matches!(zero_mode.validate(), Err(DocError::InvalidSubcolumnCount { count: 0 })));

    let over_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(MAX_SUBCOLUMNS + 1) };
    assert!(matches!(over_mode.validate(), Err(DocError::TooManySubcolumns { .. })));

    let forged_huge = json!({ "Numbers": { "subcolumn_mode": { "RawReplicates": usize::MAX } } });
    assert!(serde_json::from_value::<XColumnMode>(forged_huge).is_err());

    let forged_zero = json!({ "Numbers": { "subcolumn_mode": { "RawReplicates": 0 } } });
    assert!(serde_json::from_value::<XColumnMode>(forged_zero).is_err());

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).expect("valid xy");
    xy.x_mode = over_mode;
    assert!(matches!(DataSheet::new("xy-over-x", "Over X", TableArchetype::Xy(xy)), Err(DocError::TooManySubcolumns { .. })));
}

#[test]
fn r1_s05_02_transpose_ragged_cell_grid_is_bounded() {
    let mut grid = Vec::with_capacity(1_001);
    grid.push((0..1_000).map(|i| Cell::number(i as f64)).collect::<Vec<_>>());
    for r in 0..1_000 {
        grid.push(vec![Cell::number((1_000 + r) as f64)]);
    }
    let input_cells: usize = grid.iter().map(Vec::len).sum();
    assert_eq!(input_cells, 2_000);

    let transposed = transpose_cell_grid(&grid);
    let out_cells: usize = transposed.iter().map(Vec::len).sum();
    assert!(out_cells <= 2 * input_cells, "expected bounded output <= {}, got {out_cells}", 2 * input_cells);
    assert_eq!(transposed.len(), 1_000);
    assert_eq!(transposed[0].len(), 1_001);
    assert_eq!(transposed[1].len(), 1);
    assert_eq!(transposed[999].len(), 1);
    assert_eq!(transposed[999][0].active_number(), Some(999.0));
    assert_eq!(transposed[0][1_000].active_number(), Some(1_999.0));

    // Empty input stays empty; rectangular involution still holds.
    assert!(transpose_cell_grid(&[]).is_empty());
    let rect = vec![vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)], vec![Cell::number(4.0), Cell::number(5.0), Cell::number(6.0)]];
    assert_eq!(transpose_cell_grid(&transpose_cell_grid(&rect)), rect);
}
