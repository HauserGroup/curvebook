//! Integration tests for `R17.7` Verification Plan §1:
//! - Schema `v7 -> v8` migration & `v7` forward-version refusal (`UnsupportedSchemaVersion`)
//! - `.curvebook` ZIP archive persistence of `plate_well_origins` (direct & streaming `DataSheetVisitor`),
//!   `DataSheet::validate()` OOB/duplicate/empty checks, and `.cbtemplate` stripping
//! - Versioned `.cbplate` atomic save/load, `v0 -> v1` migration, forward refusal on load & save,
//!   `DESC_PLATE_MAP`, and non-collision with `.cbtemplate` & `.json`
//! - `.xlsx` `cell_range`, exact XML `fill_merged_cells`, `skip_rows` + shared `InfoSheet` preamble,
//!   `split_by_column` (stacked & interleaved, deterministic IDs), and `row_match`
//! - `.csv` trailing-comma padded preamble + European decimal comma, `ImportFilterSpec` windowing,
//!   `row_match`, and deterministic `split_by_column`
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use super::common;

use std::error::Error;
use std::path::Path;

use common::TempDirGuard;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AreaUnderCurveSpec, AucBaseline, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellExclusionNote, CellValue,
    ColumnDesignHint, ColumnEqualsFilter, ColumnTable, ConstantValue, DataSheet, DocError, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec,
    InfoSheet, LinkSource, LinkedDataRegion, PlateCellOrigin, PlateGeometry, PlateWellRole, ProjectDocument, ResultSheet, ResultStatus,
    SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TableKind, XyTable,
};
use curvebook_format::{
    CURRENT_PLATE_MAP_VERSION, CsvOptions, DESC_PLATE_MAP, FormatDescriptor, FormatError, FormatKind, FormatRegistry, ImportWarningKind,
    PLATE_MAP_EXTENSION, PlateMapFile, VersionStep, XlsxImportOptions, ZipWriter, filter_delimited_text, import_csv_sheets_with_filter,
    import_xlsx_bytes_with_options, load_cbplate_file, migrate_document_json, open_project_bytes, open_template_bytes, parse_cbplate_json,
    save_cbplate_file, save_project_bytes, save_template_bytes, serialize_cbplate_json, strip_project_data_for_template, upgrade,
};

type TestResult = Result<(), Box<dyn Error>>;

fn wrap_zip_archive(doc_json: &serde_json::Value, manifest_schema_version: u32, kind: &str) -> Result<Vec<u8>, FormatError> {
    let doc_bytes = serde_json::to_vec(doc_json)?;
    let title = doc_json.get("title").and_then(serde_json::Value::as_str).unwrap_or("Archive");
    let manifest = serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": manifest_schema_version,
        "kind": kind,
        "title": title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let mut writer = ZipWriter::new();
    writer.add_file("document.json", doc_bytes)?;
    writer.add_file("manifest.json", serde_json::to_vec(&manifest)?)?;
    writer.finish()
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Result<Vec<u8>, FormatError> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec())?;
    }
    zw.finish()
}

fn noop_version_step(_val: &mut serde_json::Value) -> Result<(), FormatError> {
    Ok(())
}

fn build_v8_plate_project() -> Result<ProjectDocument, FormatError> {
    let mut doc = ProjectDocument::new("Microplate Kinetic Study")?;

    let mut info = InfoSheet::new_preseeded("info-1", "Run Metadata")?;
    info.set_constant("Temperature", ConstantValue::Number(37.0), Some(String::from("C")))?;
    doc.add_info_sheet(info)?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    if let [ds0, ds1] = col.data_sets.as_mut_slice() {
        ds0.title = String::from("Compound 1");
        ds1.title = String::from("Buffer");
        if let [r0, r1, r2] = ds0.cells.as_mut_slice() {
            *r0 = vec![Cell::number(112.5)];
            *r1 = vec![Cell::excluded_number(999.0)];
            *r2 = vec![Cell::number(108.0)];
        }
        if let [r0, r1, r2] = ds1.cells.as_mut_slice() {
            *r0 = vec![Cell::number(10.0)];
            *r1 = vec![Cell::number(11.0)];
            *r2 = vec![Cell::number(9.5)];
        }
    }
    let mut col_sheet = DataSheet::new("plate-screen", "Screen AUC", TableArchetype::Column(col))?;
    col_sheet.info_sheet_id = Some(SheetId::new("info-1"));
    col_sheet.exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(1, 0, 0),
        reason_tag: String::from("pipetting_error"),
        note: String::from("Bubble in well G12"),
        timestamp_utc: String::from("2026-01-01T00:00:00Z"),
    });
    col_sheet.plate_well_origins = vec![
        PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("A01") },
        PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("G12") },
        PlateCellOrigin { coord: CellCoord::new(2, 0, 0), well: String::from("H12") },
        PlateCellOrigin { coord: CellCoord::new(0, 1, 0), well: String::from("A12") },
    ];
    col_sheet.add_linked_region(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("kinetic.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec {
                row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Em 460"), drop_column: true }),
                ..ImportFilterSpec::default()
            },
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0)),
        provenance_summary: String::from("Filtered channel Em 460"),
    })?;
    doc.add_data_sheet(col_sheet)?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 3)?;
    xy.x_title = String::from("Time (s)");
    for (r, t) in [0.0, 30.0, 60.0].into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(t))?;
    }
    if let [ds0, ds1] = xy.data_sets.as_mut_slice() {
        ds0.title = String::from("A01");
        ds1.title = String::from("A02");
        if let [r0, r1, r2] = ds0.cells.as_mut_slice() {
            *r0 = vec![Cell::number(1.0)];
            *r1 = vec![Cell::number(4.0)];
            *r2 = vec![Cell::number(2.0)];
        }
        if let [r0, r1, r2] = ds1.cells.as_mut_slice() {
            *r0 = vec![Cell::number(0.5)];
            *r1 = vec![Cell::number(1.5)];
            *r2 = vec![Cell::number(1.0)];
        }
    }
    let xy_sheet = DataSheet::new("kinetic-em460", "Kinetic Em 460", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(xy_sheet)?;

    let auc_spec = AreaUnderCurveSpec {
        baseline: AucBaseline::FirstPoint,
        x_min: Some(0.0),
        x_max: Some(60.0),
        independent_time_points: false,
        min_peak_height: 0.25,
        min_area_fraction: 0.05,
    };
    let analysis = AnalysisSpec::new("auc-1", "kinetic-em460", AnalysisKind::AreaUnderCurve(auc_spec))?;
    let mut res = ResultSheet::new(analysis)?;
    res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(res)?;
    Ok(doc)
}

fn sample_plate_map(geometry: PlateGeometry, name: &str) -> Result<PlateMapFile, FormatError> {
    let mut wells = vec![PlateWellRole::Excluded; geometry.well_count()];
    if let Some(slot) = wells.get_mut(0) {
        *slot = PlateWellRole::Treatment { group_name: String::from("Compound 1"), dilution_x: -6.0, replicate: 0 };
    }
    if let Some(slot) = wells.get_mut(1) {
        *slot = PlateWellRole::Treatment { group_name: String::from("Compound 1"), dilution_x: -6.0, replicate: 1 };
    }
    if let Some(slot) = wells.get_mut(2) {
        *slot = PlateWellRole::Blank;
    }
    if let Some(slot) = wells.get_mut(3) {
        *slot = PlateWellRole::PositiveControl;
    }
    PlateMapFile::new(name, geometry, "Log[Conc] (M)", wells)
}

#[test]
fn v7_to_v8_migration_and_v7_forward_cap_rejection() -> TestResult {
    assert_eq!(CURRENT_SCHEMA_VERSION, 9);

    // 1. Golden v7 project document (no `plate_well_origins`, no `row_match`) upgrades cleanly to current schema.
    let mut baseline_doc = ProjectDocument::new("Legacy v7 Study")?;
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    if let Some(ds0) = col.data_sets.first_mut() {
        ds0.title = String::from("Control");
        if let [r0, r1] = ds0.cells.as_mut_slice() {
            *r0 = vec![Cell::number(4.2)];
            *r1 = vec![Cell::number(5.8)];
        }
    }
    let mut ds = DataSheet::new("data-v7", "Legacy Table", TableArchetype::Column(col))?;
    ds.add_linked_region(LinkedDataRegion {
        id: String::from("legacy-link"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("legacy.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("Legacy CSV"),
    })?;
    baseline_doc.add_data_sheet(ds)?;

    let mut v7_val = serde_json::to_value(&baseline_doc)?;
    if let Some(root) = v7_val.as_object_mut() {
        root.insert(String::from("schema_version"), serde_json::json!(7_u32));
        if let Some(sheets) = root.get_mut("data_sheets").and_then(serde_json::Value::as_array_mut)
            && let Some(ds0) = sheets.first_mut().and_then(serde_json::Value::as_object_mut)
        {
            ds0.remove("plate_well_origins");
            if let Some(regions) = ds0.get_mut("linked_regions").and_then(serde_json::Value::as_array_mut)
                && let Some(r0) = regions.first_mut().and_then(serde_json::Value::as_object_mut)
                && let Some(spec) = r0.get_mut("spec").and_then(serde_json::Value::as_object_mut)
                && let Some(filter) = spec.get_mut("filter").and_then(serde_json::Value::as_object_mut)
            {
                filter.remove("row_match");
            }
        }
    }

    let migrated_val = migrate_document_json(v7_val.clone(), 7)?;
    assert_eq!(migrated_val.get("schema_version").and_then(serde_json::Value::as_u64), Some(u64::from(CURRENT_SCHEMA_VERSION)));
    let migrated_ds0_origins = migrated_val
        .get("data_sheets")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|d| d.get("plate_well_origins"))
        .and_then(serde_json::Value::as_array);
    assert_eq!(migrated_ds0_origins.map(Vec::len), Some(0));

    let v7_zip = wrap_zip_archive(&v7_val, 7, "Project")?;
    let reopened_v7 = open_project_bytes(&v7_zip)?;
    assert_eq!(reopened_v7.schema_version, CURRENT_SCHEMA_VERSION);
    let reopened_ds = reopened_v7.data_sheets.first().ok_or("missing migrated data sheet")?;
    assert!(reopened_ds.plate_well_origins.is_empty());
    let reopened_region = reopened_ds.linked_regions.first().ok_or("missing migrated linked region")?;
    assert_eq!(reopened_region.spec.filter.row_match, None);

    // 2. Forward-safety: simulated v7 binary (cap = 7) refuses v8 documents using each v8 feature.
    let v7_steps: [VersionStep; 7] = [
        VersionStep { from_version: 0, apply: noop_version_step },
        VersionStep { from_version: 1, apply: noop_version_step },
        VersionStep { from_version: 2, apply: noop_version_step },
        VersionStep { from_version: 3, apply: noop_version_step },
        VersionStep { from_version: 4, apply: noop_version_step },
        VersionStep { from_version: 5, apply: noop_version_step },
        VersionStep { from_version: 6, apply: noop_version_step },
    ];

    let full_v8 = build_v8_plate_project()?;
    let full_v8_json = serde_json::to_value(&full_v8)?;
    assert_eq!(full_v8_json.get("schema_version").and_then(serde_json::Value::as_u64), Some(u64::from(CURRENT_SCHEMA_VERSION)));

    // Confirm all three v8 features are serialized in the test payload.
    let serialized_str = serde_json::to_string(&full_v8)?;
    assert!(serialized_str.contains("\"plate_well_origins\""));
    assert!(serialized_str.contains("\"row_match\""));
    assert!(serialized_str.contains("\"AreaUnderCurve\""));

    let v7_cap_err = upgrade(full_v8_json, 8, 0, 7, &v7_steps);
    assert_eq!(v7_cap_err, Err(FormatError::UnsupportedSchemaVersion { version: 8, max_supported: 7 }));

    // 3. Current v9 archive opener rejects future v10 archives cleanly.
    let next_ver = CURRENT_SCHEMA_VERSION + 1;
    let mut future_val = serde_json::to_value(&full_v8)?;
    if let Some(root) = future_val.as_object_mut() {
        root.insert(String::from("schema_version"), serde_json::json!(next_ver));
    }
    let future_zip = wrap_zip_archive(&future_val, next_ver, "Project")?;
    assert_eq!(
        open_project_bytes(&future_zip),
        Err(FormatError::UnsupportedSchemaVersion { version: next_ver, max_supported: CURRENT_SCHEMA_VERSION })
    );
    Ok(())
}

#[test]
fn curvebook_zip_roundtrip_preserves_plate_well_origins_streaming_validates_and_cbtemplate_strips() -> TestResult {
    let doc = build_v8_plate_project()?;

    // 1. Real `.curvebook` ZIP archive save & reopen preserves `plate_well_origins`, `row_match`, and `AreaUnderCurve`.
    let archive_bytes = save_project_bytes(&doc)?;
    let reopened = open_project_bytes(&archive_bytes)?;
    assert_eq!(reopened, doc);
    let reopened_screen = reopened.data_sheets.first().ok_or("missing screen sheet")?;
    assert_eq!(reopened_screen.plate_well_origins.len(), 4);
    assert_eq!(reopened_screen.plate_well_origin(CellCoord::new(1, 0, 0)), Some("G12"));
    assert_eq!(reopened_screen.plate_well_origin(CellCoord::new(2, 0, 0)), Some("H12"));

    // 2. Streaming `DataSheetVisitor` path (omitting `schema_version` in `document.json` so `open_project_bytes`
    //    routes through `parse_streamed_document_outcome` -> `DataSheetVisitor`) ALSO preserves `plate_well_origins`.
    let mut streamed_doc_val = serde_json::to_value(&doc)?;
    if let Some(root) = streamed_doc_val.as_object_mut() {
        root.remove("schema_version");
    }
    let streamed_zip = wrap_zip_archive(&streamed_doc_val, CURRENT_SCHEMA_VERSION, "Project")?;
    let streamed_reopened = open_project_bytes(&streamed_zip)?;
    assert_eq!(streamed_reopened, doc);
    let streamed_screen = streamed_reopened.data_sheets.first().ok_or("missing streamed screen sheet")?;
    assert_eq!(streamed_screen.plate_well_origins, reopened_screen.plate_well_origins);

    // 3. `DataSheet::validate()` rejects out-of-bounds, duplicate, and empty `plate_well_origins`.
    let mut bad_oob = reopened_screen.clone();
    bad_oob.plate_well_origins.push(PlateCellOrigin { coord: CellCoord::new(99, 0, 0), well: String::from("B02") });
    assert!(matches!(bad_oob.validate(), Err(DocError::CellOutOfBounds { row: 99, col: 0, subcol: 0 })));

    let mut bad_dup = reopened_screen.clone();
    bad_dup.plate_well_origins.push(PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("G12") });
    assert!(matches!(bad_dup.validate(), Err(DocError::InvalidIdentifier { .. })));

    let mut bad_empty = reopened_screen.clone();
    bad_empty.plate_well_origins.push(PlateCellOrigin { coord: CellCoord::new(1, 1, 0), well: String::from("   ") });
    assert!(matches!(bad_empty.validate(), Err(DocError::EmptyIdentifier)));

    // Forged ZIP archives with OOB or duplicate `plate_well_origins` fail on BOTH direct v8 and streaming visitor paths.
    for omit_schema_version in [false, true] {
        let mut forged_oob = serde_json::to_value(&doc)?;
        if let Some(root) = forged_oob.as_object_mut() {
            if omit_schema_version {
                root.remove("schema_version");
            }
            if let Some(sheets) = root.get_mut("data_sheets").and_then(serde_json::Value::as_array_mut)
                && let Some(ds0) = sheets.first_mut().and_then(serde_json::Value::as_object_mut)
            {
                ds0.insert(String::from("plate_well_origins"), serde_json::json!([{ "coord": { "row": 50, "col": 0, "subcol": 0 }, "well": "A01" }]));
            }
        }
        let oob_zip = wrap_zip_archive(&forged_oob, CURRENT_SCHEMA_VERSION, "Project")?;
        assert!(open_project_bytes(&oob_zip).is_err(), "forged OOB origin zip must fail (streamed={omit_schema_version})");

        let mut forged_dup = serde_json::to_value(&doc)?;
        if let Some(root) = forged_dup.as_object_mut() {
            if omit_schema_version {
                root.remove("schema_version");
            }
            if let Some(sheets) = root.get_mut("data_sheets").and_then(serde_json::Value::as_array_mut)
                && let Some(ds0) = sheets.first_mut().and_then(serde_json::Value::as_object_mut)
            {
                ds0.insert(
                    String::from("plate_well_origins"),
                    serde_json::json!([
                        { "coord": { "row": 0, "col": 0, "subcol": 0 }, "well": "A01" },
                        { "coord": { "row": 0, "col": 0, "subcol": 0 }, "well": "A02" }
                    ]),
                );
            }
        }
        let dup_zip = wrap_zip_archive(&forged_dup, CURRENT_SCHEMA_VERSION, "Project")?;
        assert!(open_project_bytes(&dup_zip).is_err(), "forged duplicate origin zip must fail (streamed={omit_schema_version})");
    }

    // 4. `strip_project_data_for_template` and `.cbtemplate` ZIP save/reopen clear `plate_well_origins` and `exclusion_notes`.
    let stripped = strip_project_data_for_template(&doc)?;
    let stripped_screen = stripped.data_sheets.first().ok_or("missing stripped screen sheet")?;
    assert!(stripped_screen.plate_well_origins.is_empty());
    assert!(stripped_screen.exclusion_notes.is_empty());

    let tmpl_bytes = save_template_bytes(&doc)?;
    let reopened_tmpl = open_template_bytes(&tmpl_bytes)?;
    let tmpl_screen = reopened_tmpl.data_sheets.first().ok_or("missing template screen sheet")?;
    assert!(tmpl_screen.plate_well_origins.is_empty());
    assert!(tmpl_screen.exclusion_notes.is_empty());
    let tmpl_auc = reopened_tmpl.result_sheets.first().ok_or("missing template AUC sheet")?;
    assert_eq!(tmpl_auc.status, ResultStatus::Dirty);
    assert!(matches!(tmpl_auc.analysis.kind, AnalysisKind::AreaUnderCurve(_)));
    Ok(())
}

#[test]
fn cbplate_descriptor_non_collision_and_atomic_io() -> TestResult {
    // 1. Descriptor & non-collision with `.cbtemplate` (ZIP) and `.json` (`DESC_JSON`).
    assert_eq!(CURRENT_PLATE_MAP_VERSION, 1);
    assert_eq!(PLATE_MAP_EXTENSION, "cbplate");
    assert_eq!(
        DESC_PLATE_MAP,
        FormatDescriptor { id: "cbplate", label: "CurveBook Plate Map Layout", extensions: &["cbplate"], kind: FormatKind::Template }
    );

    let reg = FormatRegistry::default_registry();
    assert!(reg.importer_for_path(Path::new("assay_layout.cbplate")).is_none());
    assert!(reg.exporter_for_path(Path::new("assay_layout.cbplate")).is_none());
    assert_eq!(reg.importer_for_path(Path::new("project.cbtemplate")).map(|i| i.descriptor().id), Some("cbtemplate"));
    assert_eq!(reg.exporter_for_path(Path::new("project.cbtemplate")).map(|e| e.descriptor().id), Some("cbtemplate"));
    assert_eq!(reg.exporter_for_path(Path::new("results.json")).map(|e| e.descriptor().id), Some("json"));

    let plate96 = sample_plate_map(PlateGeometry::Wells96, "Standard 96-Well Screen")?;
    let plate384 = sample_plate_map(PlateGeometry::Wells384, "High-Density 384-Well Screen")?;
    let cbplate_json = serialize_cbplate_json(&plate96, true)?;

    // `.cbplate` UTF-8 JSON is rejected by `.cbtemplate` ZIP opener, and `.cbtemplate` ZIP is rejected by `.cbplate` parser.
    assert!(open_template_bytes(cbplate_json.as_bytes()).is_err());
    let sample_doc = build_v8_plate_project()?;
    let cbtemplate_zip = save_template_bytes(&sample_doc)?;
    assert!(parse_cbplate_json(&String::from_utf8_lossy(&cbtemplate_zip)).is_err());

    // 2. Atomic save & load round-trip on disk with zero lingering `.tmp` files.
    let dir = TempDirGuard::new("r17-7-cbplate")?;
    let path96 = dir.child("nested").join("screen96.cbplate");
    let path384 = dir.child("nested").join("screen384.cbplate");
    save_cbplate_file(&path96, &plate96, true)?;
    save_cbplate_file(&path384, &plate384, false)?;
    assert_eq!(load_cbplate_file(&path96)?, plate96);
    assert_eq!(load_cbplate_file(&path384)?, plate384);

    let nested_dir = dir.child("nested");
    for entry in std::fs::read_dir(&nested_dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        assert!(!name.contains(".tmp"), "unexpected leftover temp file: {name}");
    }
    Ok(())
}

#[test]
fn cbplate_v0_migration_forward_refusal_and_validation() -> TestResult {
    let plate96 = sample_plate_map(PlateGeometry::Wells96, "Standard 96-Well Screen")?;
    let dir = TempDirGuard::new("r17-7-cbplate-ver")?;

    // 1. `v0 -> v1` migration: both omitted `format_version` and explicit `format_version: 0` upgrade to `1`.
    let mut v0_omitted = serde_json::to_value(&plate96)?;
    if let Some(obj) = v0_omitted.as_object_mut() {
        obj.remove("format_version");
    }
    let parsed_omitted = parse_cbplate_json(&serde_json::to_string(&v0_omitted)?)?;
    assert_eq!(parsed_omitted.format_version, CURRENT_PLATE_MAP_VERSION);
    assert_eq!(parsed_omitted, plate96);

    let mut v0_zero = serde_json::to_value(&plate96)?;
    if let Some(obj) = v0_zero.as_object_mut() {
        obj.insert(String::from("format_version"), serde_json::json!(0_u32));
    }
    let parsed_zero = parse_cbplate_json(&serde_json::to_string(&v0_zero)?)?;
    assert_eq!(parsed_zero.format_version, CURRENT_PLATE_MAP_VERSION);
    assert_eq!(parsed_zero, plate96);

    // 2. Forward refusal (`format_version > 1`) on load AND save (both existing newer disk file and in-memory future struct).
    let mut v2_val = serde_json::to_value(&plate96)?;
    if let Some(obj) = v2_val.as_object_mut() {
        obj.insert(String::from("format_version"), serde_json::json!(2_u32));
    }
    let v2_raw = serde_json::to_string_pretty(&v2_val)?;
    assert_eq!(parse_cbplate_json(&v2_raw), Err(FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }));

    let mut huge_ver_val = serde_json::to_value(&plate96)?;
    if let Some(obj) = huge_ver_val.as_object_mut() {
        obj.insert(String::from("format_version"), serde_json::json!(9_999_999_999_999_u64));
    }
    assert_eq!(
        parse_cbplate_json(&serde_json::to_string(&huge_ver_val)?),
        Err(FormatError::UnsupportedSchemaVersion { version: u32::MAX, max_supported: 1 })
    );

    let newer_disk_path = dir.child("future.cbplate");
    std::fs::write(&newer_disk_path, &v2_raw)?;
    assert_eq!(load_cbplate_file(&newer_disk_path), Err(FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }));
    assert_eq!(save_cbplate_file(&newer_disk_path, &plate96, true), Err(FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }));
    assert_eq!(std::fs::read_to_string(&newer_disk_path)?, v2_raw);

    let mut in_mem_future = plate96.clone();
    in_mem_future.format_version = 2;
    assert_eq!(
        save_cbplate_file(&dir.child("should_not_exist.cbplate"), &in_mem_future, true),
        Err(FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 })
    );
    assert!(!dir.child("should_not_exist.cbplate").exists());

    // 3. Validation & bounded well-array deserialization.
    let short_wells = vec![PlateWellRole::Blank; 95];
    assert!(matches!(PlateMapFile::new("Short", PlateGeometry::Wells96, "X", short_wells), Err(FormatError::InvalidPipeline { .. })));

    let oversized_json = serde_json::json!({
        "format_version": 1,
        "name": "Oversized",
        "geometry": "Wells384",
        "x_title": "X",
        "wells": vec!["Blank"; 400]
    })
    .to_string();
    assert!(matches!(parse_cbplate_json(&oversized_json), Err(FormatError::Json(_))));

    let mut nan_plate = plate96.clone();
    if let Some(slot) = nan_plate.wells.first_mut() {
        *slot = PlateWellRole::Treatment { group_name: String::from("Compound 1"), dilution_x: f64::NAN, replicate: 0 };
    }
    assert!(matches!(nan_plate.validate(), Err(FormatError::NonFiniteFloat { .. })));
    Ok(())
}

#[test]
fn xlsx_cell_range_single_sheet_exact_dims_and_exact_merge_fill() -> TestResult {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Layout" sheetId="1" r:id="rId1"/>
    <sheet name="Kinetic" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    // Plate layout in B4:M11 (8 rows x 12 cols):
    // - Row 4: B4:C4 merged ("Compound 1"), D4:D5 merged ("Buffer"), E4 unmerged empty, L4 = "Ref", M4 empty
    // - Row 6: A6:B6 merged ("BorderComp") anchored outside B4:M11 so expanding merges BEFORE cropping fills B6
    // - Row 11 (H1..H12) and Column M (A12..H12) intentionally empty to verify exact 8x12 preservation
    let layout_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="3"><c r="B3" t="inlineStr"><is><t>Col 1</t></is></c></row>
    <row r="4">
      <c r="A4" t="inlineStr"><is><t>A</t></is></c>
      <c r="B4" t="inlineStr"><is><t>Compound 1</t></is></c>
      <c r="D4" t="inlineStr"><is><t>Buffer</t></is></c>
      <c r="L4" t="inlineStr"><is><t>Ref</t></is></c>
    </row>
    <row r="6">
      <c r="A6" t="inlineStr"><is><t>BorderComp</t></is></c>
    </row>
  </sheetData>
  <mergeCells count="3">
    <mergeCell ref="B4:C4"/>
    <mergeCell ref="D4:D5"/>
    <mergeCell ref="A6:B6"/>
  </mergeCells>
</worksheet>"#;
    let kinetic_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Time</t></is></c><c r="B1" t="inlineStr"><is><t>A01</t></is></c></row>
    <row r="2"><c r="A2"><v>0</v></c><c r="B2"><v>10</v></c></row>
  </sheetData>
</worksheet>"#;

    let bytes = build_xlsx_zip(&[
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", layout_sheet),
        ("xl/worksheets/sheet2.xml", kinetic_sheet),
    ])?;

    // Multi-sheet default selection with `cell_range` must fail.
    let multi_default_err = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions { cell_range: Some(String::from("B4:M11")), has_header: false, ..XlsxImportOptions::default() },
    );
    assert!(matches!(multi_default_err, Err(FormatError::InvalidPipeline { .. })));

    // Explicit multi-sheet selection with `cell_range` must also fail.
    let multi_explicit_err = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            sheet_names: Some(vec![String::from("Layout"), String::from("Kinetic")]),
            cell_range: Some(String::from("B4:M11")),
            has_header: false,
            ..XlsxImportOptions::default()
        },
    );
    assert!(matches!(multi_explicit_err, Err(FormatError::InvalidPipeline { .. })));

    // Single-sheet selection with `fill_merged_cells: false` keeps exact 8x12 dims and does NOT expand merges.
    let no_fill = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            sheet_names: Some(vec![String::from("Layout")]),
            cell_range: Some(String::from("B4:M11")),
            has_header: false,
            fill_merged_cells: false,
            ..XlsxImportOptions::default()
        },
    )?;
    assert_eq!(no_fill.document.data_sheets.len(), 1);
    assert!(no_fill.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection && w.message.contains("merged range")));
    let ds_no_fill = no_fill.document.data_sheets.first().ok_or("missing no-fill sheet")?;
    assert_eq!(ds_no_fill.archetype.row_count(), 8);
    assert_eq!(ds_no_fill.archetype.column_count(), 12);
    assert_eq!(ds_no_fill.cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("Compound 1")));
    assert_eq!(ds_no_fill.cell(CellCoord::new(0, 1, 0)), Some(&Cell::empty()));
    assert_eq!(ds_no_fill.cell(CellCoord::new(1, 2, 0)), Some(&Cell::empty()));
    assert_eq!(ds_no_fill.cell(CellCoord::new(2, 0, 0)), Some(&Cell::empty()));
    assert_eq!(ds_no_fill.cell(CellCoord::new(7, 11, 0)), Some(&Cell::empty()));

    // Single-sheet selection with `fill_merged_cells: true` expands exact XML merges before cropping and keeps unmerged empty cells empty.
    let filled = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            sheet_indices: Some(vec![0]),
            cell_range: Some(String::from("B4:M11")),
            has_header: false,
            fill_merged_cells: true,
            ..XlsxImportOptions::default()
        },
    )?;
    assert_eq!(filled.document.data_sheets.len(), 1);
    assert!(filled.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.message.contains("expanded")));
    let ds_filled = filled.document.data_sheets.first().ok_or("missing filled sheet")?;
    assert_eq!(ds_filled.archetype.row_count(), 8);
    assert_eq!(ds_filled.archetype.column_count(), 12);
    assert_eq!(ds_filled.cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("Compound 1")));
    assert_eq!(ds_filled.cell(CellCoord::new(0, 1, 0)), Some(&Cell::text("Compound 1")));
    assert_eq!(ds_filled.cell(CellCoord::new(0, 2, 0)), Some(&Cell::text("Buffer")));
    assert_eq!(ds_filled.cell(CellCoord::new(1, 2, 0)), Some(&Cell::text("Buffer")));
    assert_eq!(ds_filled.cell(CellCoord::new(2, 0, 0)), Some(&Cell::text("BorderComp")));
    assert_eq!(ds_filled.cell(CellCoord::new(0, 3, 0)), Some(&Cell::empty())); // unmerged E4 stays empty
    assert_eq!(ds_filled.cell(CellCoord::new(0, 10, 0)), Some(&Cell::text("Ref"))); // L4
    assert_eq!(ds_filled.cell(CellCoord::new(0, 11, 0)), Some(&Cell::empty())); // trailing empty M4
    assert_eq!(ds_filled.cell(CellCoord::new(7, 11, 0)), Some(&Cell::empty())); // trailing empty M11
    Ok(())
}

#[test]
fn xlsx_skip_rows_shared_preamble_split_by_column_and_row_match() -> TestResult {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Kinetic Plate" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    // Rows 1..=4: preamble (KV single-cell, numeric KV, 2-cell KV, plain-text note)
    // Row 5: header (Channel, Time, A01, A02)
    // Rows 6..=17: stacked + interleaved blocks, leading-space block label, slug collision labels,
    // blank separator row, whitespace-only separator row, non-blank row with empty discriminator, and mixed-text block.
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Assay: Dual Emission</t></is></c></row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>Temperature: 37.5</t></is></c></row>
    <row r="3"><c r="A3" t="inlineStr"><is><t>Note:</t></is></c><c r="B3" t="inlineStr"><is><t>Kinetic plate read</t></is></c></row>
    <row r="4"><c r="A4" t="inlineStr"><is><t>Unstructured preamble summary line</t></is></c></row>
    <row r="5">
      <c r="A5" t="inlineStr"><is><t>Channel</t></is></c>
      <c r="B5" t="inlineStr"><is><t>Time</t></is></c>
      <c r="C5" t="inlineStr"><is><t>A01</t></is></c>
      <c r="D5" t="inlineStr"><is><t>A02</t></is></c>
    </row>
    <row r="6">
      <c r="A6" t="inlineStr"><is><t>  Em 460  </t></is></c>
      <c r="B6"><v>0</v></c><c r="C6"><v>100</v></c><c r="D6"><v>200</v></c>
    </row>
    <row r="7">
      <c r="A7" t="inlineStr"><is><t>Em 530</t></is></c>
      <c r="B7"><v>0</v></c><c r="C7"><v>50</v></c><c r="D7"><v>80</v></c>
    </row>
    <row r="8">
      <c r="A8" t="inlineStr"><is><t>Em 460</t></is></c>
      <c r="B8"><v>10</v></c><c r="C8"><v>140</v></c><c r="D8"><v>260</v></c>
    </row>
    <row r="9">
      <c r="A9" t="inlineStr"><is><t>   </t></is></c>
    </row>
    <row r="11">
      <c r="B11"><v>99</v></c><c r="C11"><v>1</v></c>
    </row>
    <row r="12">
      <c r="A12" t="inlineStr"><is><t>Em 530</t></is></c>
      <c r="B12"><v>10</v></c><c r="C12"><v>60</v></c><c r="D12"><v>95</v></c>
    </row>
    <row r="13">
      <c r="A13" t="inlineStr"><is><t>Em-460</t></is></c>
      <c r="B13"><v>0</v></c><c r="C13"><v>300</v></c><c r="D13"><v>400</v></c>
    </row>
    <row r="14">
      <c r="A14" t="inlineStr"><is><t>Em/460</t></is></c>
      <c r="B14"><v>0</v></c><c r="C14" t="inlineStr"><is><t>Flagged</t></is></c><c r="D14"><v>500</v></c>
    </row>
  </sheetData>
</worksheet>"#;

    let bytes = build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", sheet1)])?;

    // Mutual exclusivity between `split_by_column` and `row_match`.
    let conflict_err = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            skip_rows: 4,
            split_by_column: Some(0),
            row_match: Some(ColumnEqualsFilter::new(0, "Em 460")),
            ..XlsxImportOptions::default()
        },
    );
    assert!(matches!(conflict_err, Err(FormatError::InvalidPipeline { .. })));

    // Split + shared InfoSheet preamble extraction.
    let split_rep = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions { skip_rows: 4, extract_preamble_info: true, split_by_column: Some(0), ..XlsxImportOptions::default() },
    )?;

    // Warning emitted for the 1 non-empty row (row 11) with an empty discriminator, while blank/whitespace rows 9 & 10 are skipped silently.
    assert!(split_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection && w.message.contains("skipped 1 non-empty row(s)")));

    // Exactly ONE shared InfoSheet linked to all 4 emitted DataSheets.
    assert_eq!(split_rep.document.info_sheets.len(), 1);
    let info = split_rep.document.info_sheets.first().ok_or("missing shared InfoSheet")?;
    assert_eq!(info.get_constant("Assay").map(|c| &c.value), Some(&ConstantValue::Text(String::from("Dual Emission"))));
    assert_eq!(info.get_constant("Temperature").and_then(|c| c.value.as_finite_f64()), Some(37.5));
    assert_eq!(info.get_constant("Note").map(|c| &c.value), Some(&ConstantValue::Text(String::from("Kinetic plate read"))));
    assert!(info.notes_markdown.contains("Unstructured preamble summary line"));

    assert_eq!(split_rep.document.data_sheets.len(), 4);
    let ids: Vec<&str> = split_rep.document.data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["kinetic-plate_em_460", "kinetic-plate_em_530", "kinetic-plate_em_460_2", "kinetic-plate_em_460_3"]);
    let titles: Vec<&str> = split_rep.document.data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["Kinetic Plate — Em 460", "Kinetic Plate — Em 530", "Kinetic Plate — Em-460", "Kinetic Plate — Em/460"]);

    let expected_linked: Vec<SheetId> = split_rep.document.data_sheets.iter().map(|s| s.id.clone()).collect();
    assert_eq!(info.linked_data_sheets, expected_linked);
    for ds in &split_rep.document.data_sheets {
        assert_eq!(ds.info_sheet_id.as_ref(), Some(&info.id));
    }

    // Block 0 (`Em 460`): interleaved rows 6 and 8 merged in order, discriminator dropped, inferred as Xy.
    let b0 = split_rep.document.data_sheets.first().ok_or("missing block 0")?;
    let TableArchetype::Xy(xy0) = &b0.archetype else {
        return Err("expected block 0 to infer Xy".into());
    };
    assert_eq!(xy0.x_title, "Time");
    assert_eq!(xy0.row_count(), 2);
    assert_eq!(xy0.x_value_at(0), Some(0.0));
    assert_eq!(xy0.x_value_at(1), Some(10.0));
    assert_eq!(xy0.data_sets.first().and_then(|d| d.cell(0, 0)), Some(&Cell::number(100.0)));
    assert_eq!(xy0.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(140.0)));

    // Block 3 (`Em/460`): contains text cell `"Flagged"`, inferred independently as MultipleVariables.
    let b3 = split_rep.document.data_sheets.get(3).ok_or("missing block 3")?;
    assert!(matches!(b3.archetype, TableArchetype::MultipleVariables(_)));

    // `row_match` with `drop_column: true` vs `drop_column: false`.
    let matched_drop = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            skip_rows: 4,
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Em 530"), drop_column: true }),
            ..XlsxImportOptions::default()
        },
    )?;
    let md0 = matched_drop.document.data_sheets.first().ok_or("missing row_match drop sheet")?;
    let TableArchetype::Xy(xy_md) = &md0.archetype else {
        return Err("expected row_match drop_column=true to infer Xy".into());
    };
    assert_eq!(xy_md.row_count(), 2);
    assert_eq!(xy_md.data_sets.len(), 2);
    assert_eq!(xy_md.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(60.0)));

    let matched_keep = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            skip_rows: 4,
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Em 530"), drop_column: false }),
            ..XlsxImportOptions::default()
        },
    )?;
    let mk0 = matched_keep.document.data_sheets.first().ok_or("missing row_match keep sheet")?;
    let TableArchetype::MultipleVariables(mv_mk) = &mk0.archetype else {
        return Err("expected row_match drop_column=false to retain text discriminator column".into());
    };
    assert_eq!(mv_mk.variables.len(), 4);
    assert_eq!(mv_mk.row_count(), 2);
    assert_eq!(mv_mk.variables.first().and_then(|v| v.cells.first()), Some(&Cell::text("Em 530")));
    Ok(())
}

#[test]
fn csv_preamble_european_decimal_windowing_row_match_and_split() -> TestResult {
    // 1. Comma-delimited CSV with trailing-comma padded preamble lines + stacked & interleaved blocks + slug collisions.
    let raw_csv = concat!(
        "Operator: Analyst 1,,,,\n",
        "Gain: 1250,,,,\n",
        "Free-form run note,,,,\n",
        "Channel,Time,A01,A02,Extra\n",
        "  Ch 1 ,0,10,20,999\n",
        "Ch 2,0,5,6,999\n",
        ",,,,\n",
        "Ch 1,10,15,25,999\n",
        ",15,1,2,999\n",
        "Ch-1,0,30,40,999\n",
        "Ch/1,0,50,60,999\n",
        "Ch 2,10,7,8,999\n",
    );

    let filter = ImportFilterSpec { row_window: Some((3, 50)), col_window: Some((0, 3)), ..ImportFilterSpec::default() };

    // Mutually exclusive `split_by_column` and `filter.row_match`.
    let bad_filter = ImportFilterSpec { row_match: Some(ColumnEqualsFilter::new(0, "Ch 1")), ..filter.clone() };
    let conflict = import_csv_sheets_with_filter(
        "run1",
        "Kinetic CSV",
        raw_csv,
        TableKind::Xy,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
        &bad_filter,
        Some(0),
        true,
    );
    assert!(matches!(conflict, Err(FormatError::InvalidPipeline { .. })));

    let res = import_csv_sheets_with_filter(
        "run1",
        "Kinetic CSV",
        raw_csv,
        TableKind::Xy,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
        &filter,
        Some(0),
        true,
    )?;

    assert!(res.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection && w.message.contains("skipped 1 non-empty row(s)")));
    let info = res.info_sheet.as_ref().ok_or("missing shared CSV InfoSheet")?;
    assert_eq!(info.get_constant("Operator").map(|c| &c.value), Some(&ConstantValue::Text(String::from("Analyst 1"))));
    assert_eq!(info.get_constant("Gain").and_then(|c| c.value.as_finite_f64()), Some(1250.0));
    assert!(info.notes_markdown.contains("Free-form run note"));

    let split_ids: Vec<&str> = res.sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(split_ids, vec!["run1_ch_1", "run1_ch_2", "run1_ch_1_2", "run1_ch_1_3"]);
    let expected_linked: Vec<SheetId> = res.sheets.iter().map(|s| s.id.clone()).collect();
    assert_eq!(info.linked_data_sheets, expected_linked);
    for ds in &res.sheets {
        assert_eq!(ds.info_sheet_id.as_ref(), Some(&info.id));
    }

    let ch1 = res.sheets.first().ok_or("missing Ch 1 sheet")?;
    let TableArchetype::Xy(xy_ch1) = &ch1.archetype else {
        return Err("expected Xy sheet for Ch 1".into());
    };
    assert_eq!(xy_ch1.x_title, "Time");
    assert_eq!(xy_ch1.data_sets.len(), 2); // `Extra` excluded by `col_window: (0, 3)` and `Channel` dropped
    assert_eq!(xy_ch1.row_count(), 2);
    assert_eq!(xy_ch1.x_value_at(1), Some(10.0));
    assert_eq!(xy_ch1.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(15.0)));

    // 2. European CSV (`;` delimiter, `,` decimal separator) locale-aware preamble + decimation + `row_match`.
    let eu_csv = concat!(
        "Temperature: 37,5;;;\n",
        "Offset:;1,25;;\n",
        "12:30;;;\n",
        "Channel;Time;A01;A02\n",
        "Ch1;0,0;10,5;20,5*\n",
        "Ch1;5,0;11,5;21,5\n",
        "Ch1;10,0;12,5;22,5\n",
        "Ch2;0,0;30,5;40,5\n",
    );
    let eu_filter = ImportFilterSpec {
        row_window: Some((3, 20)),
        decimate_every_k: Some(2),
        row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Ch1"), drop_column: true }),
        ..ImportFilterSpec::default()
    };
    let eu_res = import_csv_sheets_with_filter(
        "eu1",
        "EU Run",
        eu_csv,
        TableKind::Xy,
        SubcolumnMode::Single,
        &CsvOptions::european_csv(),
        &eu_filter,
        None,
        true,
    )?;
    let eu_info = eu_res.info_sheet.as_ref().ok_or("missing EU InfoSheet")?;
    assert_eq!(eu_info.get_constant("Temperature").and_then(|c| c.value.as_finite_f64()), Some(37.5));
    assert_eq!(eu_info.get_constant("Offset").and_then(|c| c.value.as_finite_f64()), Some(1.25));
    assert!(eu_info.get_constant("12").is_none());
    assert!(eu_info.notes_markdown.contains("12:30"));

    let eu_sheet = eu_res.sheets.first().ok_or("missing EU sheet")?;
    let TableArchetype::Xy(xy_eu) = &eu_sheet.archetype else {
        return Err("expected Xy sheet for EU run".into());
    };
    // Decimation keeps indices 0 and 2 (Time 0.0 and 10.0) of Ch1 rows.
    assert_eq!(xy_eu.row_count(), 2);
    assert_eq!(xy_eu.x_value_at(0), Some(0.0));
    assert_eq!(xy_eu.x_value_at(1), Some(10.0));
    assert_eq!(xy_eu.data_sets.first().and_then(|d| d.cell(0, 0)), Some(&Cell::number(10.5)));
    assert_eq!(xy_eu.data_sets.get(1).and_then(|d| d.cell(0, 0)), Some(&Cell { value: CellValue::Number(20.5), excluded: true }));

    // 3. `filter_delimited_text` honors `row_match` with `col_window` and `drop_column`.
    let filtered = filter_delimited_text(
        "Drop,Chan,Time,Val\nX,ChA,0,1.5\nX,ChB,0,2.5\nX,ChA,5,3.5\n",
        ',',
        '.',
        true,
        &ImportFilterSpec {
            col_window: Some((1, 3)),
            row_match: Some(ColumnEqualsFilter { column: 1, equals: String::from("ChA"), drop_column: true }),
            ..ImportFilterSpec::default()
        },
    )?;
    assert_eq!(filtered.headers, vec!["Time", "Val"]);
    assert_eq!(filtered.cells.len(), 2);
    assert_eq!(filtered.cells.get(1).and_then(|r| r.get(1)), Some(&Cell::number(3.5)));
    Ok(())
}

#[test]
fn csv_interleaved_decimate_every_k_runs_within_split_and_row_match() -> TestResult {
    let interleaved = concat!(
        "Channel,Time,A01\n",
        "Ch1,0,10\n",
        "Ch2,0,100\n",
        "Ch1,5,15\n",
        "Ch2,5,150\n",
        "Ch1,10,20\n",
        "Ch2,10,200\n",
        "Ch1,15,25\n",
        "Ch2,15,250\n",
    );

    // 1. `split_by_column` + `decimate_every_k: Some(2)` keeps BOTH channels and decimates within each block.
    let split_dec = import_csv_sheets_with_filter(
        "dec-split",
        "Interleaved Split",
        interleaved,
        TableKind::Xy,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
        &ImportFilterSpec { decimate_every_k: Some(2), ..ImportFilterSpec::default() },
        Some(0),
        false,
    )?;
    assert_eq!(split_dec.sheets.len(), 2);
    let ch1 = split_dec.sheets.first().ok_or("missing Ch1")?;
    let ch2 = split_dec.sheets.get(1).ok_or("missing Ch2")?;
    let (TableArchetype::Xy(xy1), TableArchetype::Xy(xy2)) = (&ch1.archetype, &ch2.archetype) else {
        return Err("expected Xy sheets for interleaved split".into());
    };
    assert_eq!(xy1.row_count(), 2);
    assert_eq!(xy1.x_value_at(0), Some(0.0));
    assert_eq!(xy1.x_value_at(1), Some(10.0));
    assert_eq!(xy1.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(20.0)));
    assert_eq!(xy2.row_count(), 2);
    assert_eq!(xy2.x_value_at(0), Some(0.0));
    assert_eq!(xy2.x_value_at(1), Some(10.0));
    assert_eq!(xy2.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(200.0)));

    // 2. `row_match` on odd-indexed channel (`Ch2`) + `decimate_every_k: Some(2)` decimates matched rows rather than wiping Ch2.
    let match_dec = import_csv_sheets_with_filter(
        "dec-match",
        "Interleaved Match",
        interleaved,
        TableKind::Xy,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
        &ImportFilterSpec {
            decimate_every_k: Some(2),
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Ch2"), drop_column: true }),
            ..ImportFilterSpec::default()
        },
        None,
        false,
    )?;
    let matched_sheet = match_dec.sheets.first().ok_or("missing matched Ch2 sheet")?;
    let TableArchetype::Xy(xy_m) = &matched_sheet.archetype else {
        return Err("expected Xy sheet for matched Ch2".into());
    };
    assert_eq!(xy_m.row_count(), 2);
    assert_eq!(xy_m.x_value_at(0), Some(0.0));
    assert_eq!(xy_m.x_value_at(1), Some(10.0));
    assert_eq!(xy_m.data_sets.first().and_then(|d| d.cell(0, 0)), Some(&Cell::number(100.0)));
    assert_eq!(xy_m.data_sets.first().and_then(|d| d.cell(1, 0)), Some(&Cell::number(200.0)));

    // 3. `filter_delimited_text` applies `row_match` BEFORE `decimate_every_k`, and normalizes `-0.0` to `"0"`.
    let filtered = filter_delimited_text(
        interleaved,
        ',',
        '.',
        true,
        &ImportFilterSpec {
            decimate_every_k: Some(2),
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("Ch2"), drop_column: true }),
            ..ImportFilterSpec::default()
        },
    )?;
    assert_eq!(filtered.headers, vec!["Time", "A01"]);
    assert_eq!(filtered.cells.len(), 2);
    assert_eq!(filtered.cells.first().and_then(|r| r.get(1)), Some(&Cell::number(100.0)));
    assert_eq!(filtered.cells.get(1).and_then(|r| r.get(1)), Some(&Cell::number(200.0)));

    let neg_zero = filter_delimited_text(
        "Tag,Val\n-0.0,42\n1,99\n",
        ',',
        '.',
        true,
        &ImportFilterSpec {
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("0"), drop_column: true }),
            ..ImportFilterSpec::default()
        },
    )?;
    assert_eq!(neg_zero.cells.len(), 1);
    assert_eq!(neg_zero.cells.first().and_then(|r| r.first()), Some(&Cell::number(42.0)));
    Ok(())
}
