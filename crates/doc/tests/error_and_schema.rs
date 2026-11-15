//! Verification tests for structured L1 error codes, `RemediationKind`, and Draft 2020-12 JSON Schema helpers (`C2`).

use lumencraft_doc::layout::{LayoutItemLimitError, MAX_LAYOUT_ITEMS};
use lumencraft_doc::{
    ChecklistItem, ChecklistStatus, DRAFT_2020_12_SCHEMA_URI, ErrorCategory, ErrorCode, JournalWidthPreset, JsonSchema, LayoutItem, LayoutSpec,
    PROJECT_DOCUMENT_SCHEMA_ID, ProjectDocument, RectPt, RemediationKind, ResultStatus, SheetId, array_schema, bool_schema, enum_schema,
    integer_schema, number_schema, object_schema, project_document_schema, string_schema,
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
