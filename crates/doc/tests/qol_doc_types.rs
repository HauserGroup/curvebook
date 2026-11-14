//! Integration tests for Step 1 QOL document types: `FigureCanvasPreset`, `CanvasResizeMode`,
//! `DataSheet::set_excluded_with_reason`, `TableArchetype::allocated_cell_slots`, and
//! `NamedCheckpoint` snapshots on `ProjectDocument`.

use lumencraft_doc::{
    CanvasResizeMode, CellCoord, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, FigureCanvasPreset, GroupedTable, JournalWidthPreset,
    LayoutSpec, MultipleVariablesTable, NamedCheckpoint, NestedTable, PartsOfWholeTable, PhysicalUnit, ProjectDocument, SheetId, SubcolumnMode,
    SurvivalTable, TableArchetype, XyTable,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn figure_canvas_preset_dimensions_and_aliases() {
    let presets = [
        (FigureCanvasPreset::SingleColumn88mm, (88.0, 66.0, PhysicalUnit::Millimeters), "single_column_88mm"),
        (FigureCanvasPreset::ColumnAndHalf130mm, (130.0, 97.5, PhysicalUnit::Millimeters), "130mm"),
        (FigureCanvasPreset::DoubleColumn180mm, (180.0, 120.0, PhysicalUnit::Millimeters), "double_column_180mm"),
        (FigureCanvasPreset::FullPagePortrait, (180.0, 240.0, PhysicalUnit::Millimeters), "full_page_portrait"),
        (FigureCanvasPreset::Slide16x9, (720.0, 405.0, PhysicalUnit::Points), "slide16x9"),
        (FigureCanvasPreset::PosterPanel, (864.0, 648.0, PhysicalUnit::Points), "poster"),
    ];

    for (preset, expected_dims, alias) in presets {
        assert_eq!(preset.dimensions(), expected_dims);
        let pt = preset.dimensions_pt();
        assert!((pt.width - expected_dims.2.to_points(expected_dims.0)).abs() < 1e-9);
        assert!((pt.height - expected_dims.2.to_points(expected_dims.1)).abs() < 1e-9);

        let (w_mm, h_mm) = preset.dimensions_mm();
        let expected_w_mm = pt.width * 25.4 / 72.0;
        let expected_h_mm = pt.height * 25.4 / 72.0;
        assert!((w_mm - expected_w_mm).abs() < 1e-6);
        assert!((h_mm - expected_h_mm).abs() < 1e-6);
        assert_eq!(FigureCanvasPreset::from_name(alias), Some(preset));
    }
    assert_eq!(FigureCanvasPreset::from_name("180mm"), Some(FigureCanvasPreset::DoubleColumn180mm));
    assert_eq!(FigureCanvasPreset::from_name("unknown-canvas"), None);
}

#[test]
fn canvas_resize_mode_default_aliases_and_layout_round_trip() -> TestResult {
    assert_eq!(CanvasResizeMode::default(), CanvasResizeMode::KeepTypographyPt);
    assert_eq!(CanvasResizeMode::from_name("keep_typography_pt"), Some(CanvasResizeMode::KeepTypographyPt));
    assert_eq!(CanvasResizeMode::from_name("lock_typography"), Some(CanvasResizeMode::KeepTypographyPt));
    assert_eq!(CanvasResizeMode::from_name("proportional_scale"), Some(CanvasResizeMode::ProportionalScale));
    assert_eq!(CanvasResizeMode::from_name("scale"), Some(CanvasResizeMode::ProportionalScale));
    assert_eq!(CanvasResizeMode::from_name("other"), None);

    let mode = CanvasResizeMode::ProportionalScale;
    let json = serde_json::to_string(&mode)?;
    let decoded: CanvasResizeMode = serde_json::from_str(&json)?;
    assert_eq!(decoded, mode);

    let spec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    let spec_json = serde_json::to_string(&spec)?;
    let spec_back: LayoutSpec = serde_json::from_str(&spec_json)?;
    assert_eq!(spec_back, spec);
    Ok(())
}

#[test]
fn exclusion_notes_set_sorted_updated_and_auto_cleared() -> TestResult {
    let table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    let mut sheet = DataSheet::new(SheetId::new("col-notes"), "Observations", TableArchetype::Column(table))?;

    let c2 = CellCoord::new(2, 1, 0);
    let c0 = CellCoord::new(0, 0, 0);

    sheet.set_excluded_with_reason(c2, "outlier", "Hemolyzed sample", "2026-11-14T08:00:00Z")?;
    sheet.set_excluded_with_reason(c0, "qc_fail", "Bubble detected", "2026-11-14T08:01:00Z")?;
    assert_eq!(sheet.exclusion_notes.len(), 2);
    // Notes are maintained sorted by `CellCoord: Ord` (`(0, 0, 0)` before `(2, 1, 0)`).
    assert_eq!(sheet.exclusion_notes[0].coord, c0);
    assert_eq!(sheet.exclusion_notes[1].coord, c2);
    assert!(sheet.archetype.data_cell(c2).ok_or("missing cell c2")?.excluded);

    // Updating the same coordinate updates its note in place.
    sheet.set_excluded_with_reason(c2, "pipette_error", "Volume short", "2026-11-14T08:05:00Z")?;
    assert_eq!(sheet.exclusion_notes.len(), 2);
    let note_c2 = sheet.exclusion_note(c2).ok_or("expected note for c2")?;
    assert_eq!(note_c2.reason_tag, "pipette_error");
    assert_eq!(note_c2.note, "Volume short");

    // Clearing exclusion via `set_excluded(..., false)` automatically removes the note.
    sheet.set_excluded(c0, false)?;
    assert!(!sheet.archetype.data_cell(c0).ok_or("missing cell c0")?.excluded);
    assert_eq!(sheet.exclusion_note(c0), None);
    assert_eq!(sheet.exclusion_notes.len(), 1);
    assert_eq!(sheet.exclusion_notes[0].coord, c2);
    Ok(())
}

#[test]
fn allocated_cell_slots_across_all_archetypes() -> TestResult {
    // XY: 3 rows * (1 X cell + 2 data sets * 2 subcols) = 15
    let xy = TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?);
    assert_eq!(xy.allocated_cell_slots(), 15);

    // Column: 3 data sets * 4 rows * 1 subcol = 12
    let col = TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?);
    assert_eq!(col.allocated_cell_slots(), 12);

    // Grouped: 2 data sets * 3 rows * 2 subcols = 12
    let grp = TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?);
    assert_eq!(grp.allocated_cell_slots(), 12);

    // Contingency: 2 cols * 3 rows = 6
    let cont = TableArchetype::Contingency(ContingencyTable::new(2, 3)?);
    assert_eq!(cont.allocated_cell_slots(), 6);

    // Survival: 5 time cells + 2 cohorts * 5 rows = 15
    let surv = TableArchetype::Survival(SurvivalTable::new(2, 5)?);
    assert_eq!(surv.allocated_cell_slots(), 15);

    // PartsOfWhole: 1 col * 4 rows = 4
    let pow = TableArchetype::PartsOfWhole(PartsOfWholeTable::new(1, 4)?);
    assert_eq!(pow.allocated_cell_slots(), 4);

    // MultipleVariables: 4 variables * 5 rows = 20
    let mv = TableArchetype::MultipleVariables(MultipleVariablesTable::new(4, 5)?);
    assert_eq!(mv.allocated_cell_slots(), 20);

    // Nested: 2 groups * 3 bio units * 4 rows = 24
    let nested = TableArchetype::Nested(NestedTable::new(2, 3, 4)?);
    assert_eq!(nested.allocated_cell_slots(), 24);
    Ok(())
}

#[test]
fn named_checkpoint_snapshots_and_serde_round_trip() -> TestResult {
    let mut doc = ProjectDocument::new("Checkpoint Study")?;
    let ds = DataSheet::new(SheetId::new("d1"), "Data 1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?))?;
    doc.add_data_sheet(ds)?;

    let empty_checkpoints_json = serde_json::to_string(&doc)?;
    assert!(!empty_checkpoints_json.contains("checkpoints"));

    let mut snap_doc = doc.clone();
    snap_doc.checkpoints.clear();
    let checkpoint = NamedCheckpoint {
        id: String::from("cp-1"),
        name: String::from("Baseline Lock"),
        note: String::from("Before normalization"),
        created_utc: String::from("2026-11-14T09:00:00Z"),
        snapshot_json: serde_json::to_string(&snap_doc)?,
    };
    doc.checkpoints.push(checkpoint);

    let full_json = serde_json::to_string(&doc)?;
    let decoded: ProjectDocument = serde_json::from_str(&full_json)?;
    assert_eq!(decoded.checkpoints.len(), 1);
    assert_eq!(decoded.checkpoints[0].name, "Baseline Lock");

    let restored: ProjectDocument = serde_json::from_str(&decoded.checkpoints[0].snapshot_json)?;
    assert!(restored.checkpoints.is_empty());
    assert_eq!(restored.data_sheets.len(), 1);
    assert_eq!(restored.title, "Checkpoint Study");
    Ok(())
}
