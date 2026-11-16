use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphSheet, InfoSheet, JournalWidthPreset,
    LayoutItem, LayoutSheet, LayoutSpec, NamedCheckpoint, NonlinearFitSpec, ParameterRule, ParameterScope, ProjectDocument, RectPt,
    ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_format::prism::MAX_PROJECT_CELLS;
use curvebook_format::pzfx::MAX_IMPORT_CELLS;
use curvebook_format::zip::ZipWriter;
use curvebook_format::{
    ArchiveKind, ArchiveManifest, FormatError, import_prism_bytes, import_pzfx_str, is_unpacked_project_dir, open_project_bytes,
    open_project_unpacked_dir, save_project_bytes, save_project_unpacked_dir, save_template_bytes,
};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, FormatError> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-w5-format-{label}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build_rich_project_with_notes_and_checkpoints() -> Result<ProjectDocument, FormatError> {
    let mut doc = ProjectDocument::new("Checkpoint & Exclusion Assay")?;

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM")))?;
    info.notes_markdown = String::from("Incubation at 25 C for 45 min.");
    doc.add_info_sheet(info)?;

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    xy.x_title = String::from("Conc (nM)");
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    for (r, x) in [1.0, 10.0, 100.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(*x))?;
    }
    xy.data_sets[0].cells[0] = vec![Cell::number(11.2), Cell::number(12.4)];
    xy.data_sets[0].cells[1] = vec![Cell::number(45.8), Cell::number(999.0)];
    xy.data_sets[0].cells[2] = vec![Cell::number(88.1), Cell::number(89.5)];
    xy.data_sets[1].cells[0] = vec![Cell::number(10.0), Cell::number(10.6)];

    let mut ds1 = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy))?;
    ds1.info_sheet_id = Some(SheetId::new("info-1"));
    ds1.set_excluded_with_reason(CellCoord::new(1, 0, 1), "pipetting_error", "Bubble observed in well B2", "2026-11-14T08:00:00Z")?;
    doc.add_data_sheet(ds1.clone())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2)?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[0].cells[0] = vec![Cell::number(4.5)];
    col.data_sets[0].cells[1] = vec![Cell::number(4.8)];
    col.data_sets[1].cells[0] = vec![Cell::number(9.2)];
    col.data_sets[1].cells[1] = vec![Cell::number(9.7)];
    let mut ds2 = DataSheet::new("data-col", "Cohort Summary", TableArchetype::Column(col))?;
    ds2.set_excluded_with_reason(CellCoord::new(0, 1, 0), "hemolyzed", "Sample hemolyzed on visual QC", "2026-11-14T08:05:00Z")?;
    doc.add_data_sheet(ds2)?;

    let mut fit = NonlinearFitSpec::new("four_pl");
    fit.parameter_rules.push(ParameterRule {
        name: String::from("HotNM"),
        scope: ParameterScope::HookedConstant(curvebook_doc::HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("HotNM"),
        }),
        initial_override: None,
        lower_bound: Some(0.0),
        upper_bound: None,
    });
    let spec = AnalysisSpec::new("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 12.3, units: Some(String::from("nM")) });
    doc.add_result_sheet(res)?;

    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &ds1)?)?;

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(10.0, 10.0, 180.0, 120.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec)?)?;

    let mut snap_doc = doc.clone();
    snap_doc.checkpoints.clear();
    let snapshot_json = serde_json::to_string_pretty(&snap_doc)?;
    doc.checkpoints.push(NamedCheckpoint {
        id: String::from("cp-baseline"),
        name: String::from("Baseline Before QC"),
        note: String::from("Captured prior to secondary plate exclusion"),
        created_utc: String::from("2026-11-14T08:10:00Z"),
        snapshot_json,
    });

    Ok(doc)
}

#[test]
fn zip_and_unpacked_dir_preserve_exclusion_notes_and_checkpoints() {
    let doc = build_rich_project_with_notes_and_checkpoints().unwrap();

    // `.curvebook` ZIP round-trip
    let zip_bytes = save_project_bytes(&doc).unwrap();
    let decoded_zip = open_project_bytes(&zip_bytes).unwrap();
    assert_eq!(decoded_zip, doc);
    assert_eq!(decoded_zip.data_sheets[0].exclusion_notes.len(), 1);
    assert_eq!(decoded_zip.data_sheets[0].exclusion_note(CellCoord::new(1, 0, 1)).unwrap().reason_tag, "pipetting_error");
    assert_eq!(decoded_zip.checkpoints.len(), 1);
    assert_eq!(decoded_zip.checkpoints[0].id, "cp-baseline");

    // `.curvebook.d/` directory round-trip
    let root = TempDirGuard::new("roundtrip").unwrap();
    let proj_dir = root.path().join("assay.curvebook.d");
    assert!(!is_unpacked_project_dir(&proj_dir));

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(is_unpacked_project_dir(&proj_dir));

    let decoded_dir = open_project_unpacked_dir(&proj_dir).unwrap();
    assert_eq!(decoded_dir, doc);
    assert_eq!(decoded_dir.data_sheets[1].exclusion_note(CellCoord::new(0, 1, 0)).unwrap().reason_tag, "hemolyzed");
    assert_eq!(decoded_dir.checkpoints, doc.checkpoints);

    // Non-project paths must return false for `is_unpacked_project_dir`
    let zip_file = root.path().join("assay.curvebook");
    std::fs::write(&zip_file, &zip_bytes).unwrap();
    assert!(!is_unpacked_project_dir(&zip_file));

    let empty_dir = root.path().join("empty.curvebook.d");
    std::fs::create_dir_all(&empty_dir).unwrap();
    assert!(!is_unpacked_project_dir(&empty_dir));

    // Template manifest unpacked on disk is rejected by both `is_unpacked_project_dir` and `open_project_unpacked_dir`
    let tpl_dir = root.path().join("assay.cbtemplate.d");
    save_project_unpacked_dir(&doc, &tpl_dir).unwrap();
    let tpl_zip = save_template_bytes(&doc).unwrap();
    let mut tpl_manifest: ArchiveManifest = serde_json::from_slice(&std::fs::read(tpl_dir.join("manifest.json")).unwrap()).unwrap();
    tpl_manifest.kind = ArchiveKind::Template;
    std::fs::write(tpl_dir.join("manifest.json"), serde_json::to_vec_pretty(&tpl_manifest).unwrap()).unwrap();
    assert!(!tpl_zip.is_empty());
    assert!(!is_unpacked_project_dir(&tpl_dir));
    assert!(matches!(open_project_unpacked_dir(&tpl_dir), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));
}

#[test]
fn unpacked_dir_is_deterministic_git_friendly_and_preserves_repo_files() {
    let mut doc = build_rich_project_with_notes_and_checkpoints().unwrap();
    let root = TempDirGuard::new("git-diff").unwrap();
    let proj_dir = root.path().join("study.curvebook.d");

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    std::fs::create_dir_all(proj_dir.join(".git")).unwrap();
    std::fs::write(proj_dir.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::write(proj_dir.join(".gitignore"), b"*.tmp\n").unwrap();
    std::fs::write(proj_dir.join("README.md"), b"# Study Repository\n").unwrap();

    let manifest_v1 = std::fs::read_to_string(proj_dir.join("manifest.json")).unwrap();
    let document_v1 = std::fs::read_to_string(proj_dir.join("document.json")).unwrap();
    let sheet1_v1 = std::fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap();
    let sheet2_v1 = std::fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap();

    // Canonical 2-space pretty JSON (no tabs) + valid RFC-4180 CSV headers
    assert!(manifest_v1.contains("\n  \"format\": \"curvebook-archive\""));
    assert!(document_v1.contains("\n  \"schema_version\":"));
    assert!(!document_v1.contains('\t'));
    assert!(sheet2_v1.contains("Vehicle"));

    // Idempotent re-save produces byte-for-byte identical outputs
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert_eq!(std::fs::read_to_string(proj_dir.join("manifest.json")).unwrap(), manifest_v1);
    assert_eq!(std::fs::read_to_string(proj_dir.join("document.json")).unwrap(), document_v1);
    assert_eq!(std::fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap(), sheet1_v1);
    assert_eq!(std::fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap(), sheet2_v1);

    // Mutate only a single cell in `data-xy`
    if let TableArchetype::Xy(ref mut xy) = doc.data_sheets[0].archetype {
        xy.data_sets[0].cells[2][1] = Cell::number(91.25);
    }
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();

    // Unchanged sheet CSV stays byte-for-byte identical; `.git` and repo files remain untouched
    assert_eq!(std::fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap(), sheet2_v1);
    assert_ne!(std::fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap(), sheet1_v1);
    assert_eq!(std::fs::read(proj_dir.join(".git/HEAD")).unwrap(), b"ref: refs/heads/main\n");
    assert_eq!(std::fs::read(proj_dir.join(".gitignore")).unwrap(), b"*.tmp\n");
    assert_eq!(std::fs::read(proj_dir.join("README.md")).unwrap(), b"# Study Repository\n");

    // Verify surgical line-level JSON diff stability in `document.json`
    let document_v2 = std::fs::read_to_string(proj_dir.join("document.json")).unwrap();
    let lines1: Vec<&str> = document_v1.lines().collect();
    let lines2: Vec<&str> = document_v2.lines().collect();
    assert_eq!(lines1.len(), lines2.len());
    let changed_lines: Vec<usize> = lines1.iter().zip(&lines2).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
    assert_eq!(changed_lines.len(), 1, "expected exactly 1 line changed in document.json, got {changed_lines:?}");
}

#[test]
fn unpacked_dir_prunes_stale_sheet_files_on_resave() {
    let mut doc = build_rich_project_with_notes_and_checkpoints().unwrap();
    let root = TempDirGuard::new("prune").unwrap();
    let proj_dir = root.path().join("prune.curvebook.d");

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    std::fs::write(proj_dir.join("README.md"), b"keep me").unwrap();
    assert!(proj_dir.join("sheets/data-xy.csv").is_file());
    assert!(proj_dir.join("sheets/data-col.csv").is_file());

    // Remove `data-col` and re-save
    doc.data_sheets.retain(|ds| ds.id.as_str() != "data-col");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(proj_dir.join("sheets/data-xy.csv").is_file());
    assert!(!proj_dir.join("sheets/data-col.csv").exists());
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), doc);

    // Also remove `data-xy` (0 data sheets remaining); sheets/ should be cleaned up while README survives
    doc.result_sheets.clear();
    doc.graph_sheets.clear();
    doc.layout_sheets.clear();
    doc.data_sheets.clear();
    doc.active_sheet_id = None;
    doc.family_anchor_sheet_id = None;
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(!proj_dir.join("sheets/data-xy.csv").exists());
    assert!(!proj_dir.join("sheets").exists());
    assert_eq!(std::fs::read(proj_dir.join("README.md")).unwrap(), b"keep me");
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), doc);
}

#[test]
fn unpacked_dir_rejects_traversal_tampering_and_corrupt_entries() {
    let doc = build_rich_project_with_notes_and_checkpoints().unwrap();
    let root = TempDirGuard::new("security").unwrap();
    let proj_dir = root.path().join("sec.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();

    let orig_manifest_bytes = std::fs::read(proj_dir.join("manifest.json")).unwrap();
    let orig_manifest: ArchiveManifest = serde_json::from_slice(&orig_manifest_bytes).unwrap();

    // 1. Unsafe paths in manifest `csv_path` (`..`, leading `/`, backslash, drive letter)
    for bad_rel in ["../outside.csv", "/etc/passwd", "sheets\\data-xy.csv", "C:/evil.csv", ""] {
        let mut tampered = orig_manifest.clone();
        tampered.sheets[0].csv_path = Some(String::from(bad_rel));
        std::fs::write(proj_dir.join("manifest.json"), serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
        assert!(!is_unpacked_project_dir(&proj_dir));
        assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::UnsafePath { .. })));
    }
    std::fs::write(proj_dir.join("manifest.json"), &orig_manifest_bytes).unwrap();

    // 2. CRC mismatch in sheet CSV and in document.json
    let orig_csv = std::fs::read(proj_dir.join("sheets/data-xy.csv")).unwrap();
    std::fs::write(proj_dir.join("sheets/data-xy.csv"), b"tampered,csv\n").unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::CorruptCrc { .. })));
    std::fs::write(proj_dir.join("sheets/data-xy.csv"), &orig_csv).unwrap();

    // 3. Missing sheet CSV
    std::fs::remove_file(proj_dir.join("sheets/data-xy.csv")).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::MissingArchiveEntry { .. })));
    std::fs::write(proj_dir.join("sheets/data-xy.csv"), &orig_csv).unwrap();

    // 4. Non-UTF-8 manifest
    std::fs::write(proj_dir.join("manifest.json"), [0xff_u8, 0xfe, 0x00]).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::InvalidZip { .. })));
}

#[cfg(unix)]
#[test]
fn unpacked_dir_rejects_symlinks_escaping_root_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let doc = build_rich_project_with_notes_and_checkpoints().unwrap();
    let root = TempDirGuard::new("symlinks").unwrap();

    let outside_dir = root.path().join("outside");
    std::fs::create_dir_all(&outside_dir).unwrap();
    let outside_file = outside_dir.join("secret.csv");
    std::fs::write(&outside_file, b"top,secret\n").unwrap();

    // Case A: sheet file symlinked outside directory
    let proj_a = root.path().join("proj_a.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_a).unwrap();
    std::fs::remove_file(proj_a.join("sheets/data-xy.csv")).unwrap();
    symlink(&outside_file, proj_a.join("sheets/data-xy.csv")).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_a), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(save_project_unpacked_dir(&doc, &proj_a), Err(FormatError::UnsafePath { .. })));
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"top,secret\n");

    // Case B: `sheets` directory symlinked outside directory
    let proj_b = root.path().join("proj_b.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_b).unwrap();
    std::fs::remove_dir_all(proj_b.join("sheets")).unwrap();
    symlink(&outside_dir, proj_b.join("sheets")).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_b), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(save_project_unpacked_dir(&doc, &proj_b), Err(FormatError::UnsafePath { .. })));
    let remaining_outside: BTreeSet<String> =
        std::fs::read_dir(&outside_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(remaining_outside, BTreeSet::from([String::from("secret.csv")]));

    // Case C: `document.json` symlinked outside directory
    let proj_c = root.path().join("proj_c.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_c).unwrap();
    std::fs::remove_file(proj_c.join("document.json")).unwrap();
    symlink(&outside_file, proj_c.join("document.json")).unwrap();
    assert!(!is_unpacked_project_dir(&proj_c));
    assert!(matches!(open_project_unpacked_dir(&proj_c), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(save_project_unpacked_dir(&doc, &proj_c), Err(FormatError::UnsafePath { .. })));
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"top,secret\n");

    // Case D: dangling symlink inside `sheets/`
    let proj_d = root.path().join("proj_d.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_d).unwrap();
    std::fs::remove_file(proj_d.join("sheets/data-xy.csv")).unwrap();
    symlink(outside_dir.join("does_not_exist.csv"), proj_d.join("sheets/data-xy.csv")).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_d), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(save_project_unpacked_dir(&doc, &proj_d), Err(FormatError::UnsafePath { .. })));
}

fn build_large_xy_table_xml(id: &str, rows: usize, y_cols: usize, replicates: usize) -> String {
    let mut xml = format!("<Table ID=\"{id}\" TableType=\"XY\" YFormat=\"replicates\" Replicates=\"{replicates}\"><XColumn><Subcolumn>");
    for r in 0..rows {
        xml.push_str("<d>");
        xml.push_str(&r.to_string());
        xml.push_str("</d>");
    }
    xml.push_str("</Subcolumn></XColumn>");
    for c in 0..y_cols {
        xml.push_str(&format!("<YColumn><Title>Y{c}</Title></YColumn>"));
    }
    xml.push_str("</Table>");
    xml
}

fn build_large_survival_table_xml(id: &str, rows: usize, cohorts: usize) -> String {
    let mut xml = format!("<Table ID=\"{id}\" TableType=\"Survival\"><XColumn><Subcolumn>");
    for r in 0..rows {
        xml.push_str("<d>");
        xml.push_str(&(r + 1).to_string());
        xml.push_str("</d>");
    }
    xml.push_str("</Subcolumn></XColumn>");
    for c in 0..cohorts {
        xml.push_str(&format!("<YColumn><Title>Cohort{c}</Title></YColumn>"));
    }
    xml.push_str("</Table>");
    xml
}

#[test]
fn pzfx_multi_table_cell_budget_enforcement() {
    // Table 1: 2,500 rows * (1 X + 48 Y * 50 replicates) = 2,500 * 2,401 = 6,002,500 cells (<= 10,000,000)
    // Table 2: same 6,002,500 cells -> cumulative 12,005,000 > MAX_IMPORT_CELLS (10,000,000)
    let t1 = build_large_xy_table_xml("t1", 2_500, 48, 50);
    let within_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{t1}</TableSequence></PrismFile>");
    let imported_one = import_pzfx_str(&within_xml).unwrap();
    assert_eq!(imported_one.document.data_sheets[0].archetype.allocated_cell_slots(), 6_002_500);
    assert!(imported_one.document.data_sheets[0].archetype.allocated_cell_slots() <= MAX_IMPORT_CELLS);

    let t2 = build_large_xy_table_xml("t2", 2_500, 48, 50);
    let multi_over_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{t1}{t2}</TableSequence></PrismFile>");
    let err = import_pzfx_str(&multi_over_xml).unwrap_err();
    assert!(
        matches!(err, FormatError::XmlParse { ref reason, .. } if reason.contains("budget") || reason.contains("cap")),
        "unexpected error for multi-table XY over budget: {err:?}"
    );

    // Multi-table Survival + XY cumulative check:
    // Each Survival table with 60,000 rows * (1 time + 99 cohorts) = 6,000,000 allocated_cell_slots().
    // Note: keep node counts compact using 4,000 rows * (1 + 499 cohorts) = 2,000,000 cells, repeated across tables:
    // t_xy = 6,002,500; s1 = 4,000 * (1 + 1_000) = 4,004,000 -> total 10,006,500 > 10,000,000!
    // Wait: let's keep cohorts within MAX_DATA_SETS (check 250 cohorts * 16,100 rows = 4,041,100? Let's use 16,100 rows * (1 + 250) = 4,041,100!)
    let s_within = build_large_survival_table_xml("s1", 16_000, 250);
    let surv_single_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{s_within}</TableSequence></PrismFile>");
    let imported_surv = import_pzfx_str(&surv_single_xml).unwrap();
    assert_eq!(imported_surv.document.data_sheets[0].archetype.allocated_cell_slots(), 16_000 * 251);

    let combined_over_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{t1}{s_within}</TableSequence></PrismFile>");
    // 6,002,500 + 4,016,000 = 10,018,500 > MAX_IMPORT_CELLS (10,000,000)
    let err_surv = import_pzfx_str(&combined_over_xml).unwrap_err();
    assert!(
        matches!(err_surv, FormatError::XmlParse { ref reason, .. } if reason.contains("budget") || reason.contains("cap")),
        "unexpected error for combined XY + Survival over budget: {err_surv:?}"
    );

    // XFormat="err" allocates 3 X subcolumns (`x_cells` has 3 cells/row instead of 1):
    // 2,500 * (3 + 48 * 50) = 6,007,500 cells per table -> two tables exceed MAX_IMPORT_CELLS.
    let mut xerr = String::from("<Table ID=\"xerr1\" TableType=\"XY\" XFormat=\"err\" YFormat=\"replicates\" Replicates=\"50\"><XColumn>");
    for _sub in 0..3 {
        xerr.push_str("<Subcolumn>");
        for r in 0..2_500 {
            xerr.push_str(&format!("<d>{r}</d>"));
        }
        xerr.push_str("</Subcolumn>");
    }
    xerr.push_str("</XColumn>");
    for c in 0..48 {
        xerr.push_str(&format!("<YColumn><Title>Y{c}</Title></YColumn>"));
    }
    xerr.push_str("</Table>");

    let xerr_single = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{xerr}</TableSequence></PrismFile>");
    assert_eq!(import_pzfx_str(&xerr_single).unwrap().document.data_sheets[0].archetype.allocated_cell_slots(), 6_007_500);
    let xerr_double = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{t1}{xerr}</TableSequence></PrismFile>");
    assert!(matches!(import_pzfx_str(&xerr_double), Err(FormatError::XmlParse { .. })));
}

#[test]
fn prism_multi_entry_cumulative_project_cell_budget_enforcement() {
    assert_eq!(MAX_PROJECT_CELLS, MAX_IMPORT_CELLS);

    // Compact XY CSV with 1 header row (`X` + 2,400 Y columns -> 48 data sets * 50 replicates) and 2,500 data rows:
    // 2,500 * (1 + 48 * 50) = 6,002,500 dense cells (~10 KB CSV payload).
    let mut xy_csv = format!("X{}\n", ",Y".repeat(2_400));
    xy_csv.push_str(&"0\n".repeat(2_500));

    let make_manifest = |entries: &[(&str, &str, &str, usize)]| {
        let sheets_json: Vec<String> = entries
            .iter()
            .map(|(csv_path, kind, mode, reps)| {
                format!("{{\"csv_path\":\"{csv_path}\",\"kind\":\"{kind}\",\"subcolumn_mode\":\"{mode}\",\"replicates\":{reps}}}")
            })
            .collect();
        format!("{{\"title\":\"Budget Test\",\"sheets\":[{}]}}", sheets_json.join(","))
    };

    // Single XY CSV (6,002,500 <= MAX_PROJECT_CELLS) succeeds.
    let mut single_zip = ZipWriter::new();
    single_zip.add_file("manifest.json", make_manifest(&[("tables/t1.csv", "xy", "replicates", 50)]).into_bytes()).unwrap();
    single_zip.add_file("tables/t1.csv", xy_csv.as_bytes().to_vec()).unwrap();
    let single_report = import_prism_bytes(&single_zip.finish().unwrap()).unwrap();
    assert_eq!(single_report.document.data_sheets[0].archetype.allocated_cell_slots(), 6_002_500);

    // Two XY CSVs totaling 12,005,000 > MAX_PROJECT_CELLS must fail before allocating the second sheet.
    let mut multi_xy_zip = ZipWriter::new();
    multi_xy_zip
        .add_file(
            "manifest.json",
            make_manifest(&[("tables/t1.csv", "xy", "replicates", 50), ("tables/t2.csv", "xy", "replicates", 50)]).into_bytes(),
        )
        .unwrap();
    multi_xy_zip.add_file("tables/t1.csv", xy_csv.as_bytes().to_vec()).unwrap();
    multi_xy_zip.add_file("tables/t2.csv", xy_csv.as_bytes().to_vec()).unwrap();
    let multi_xy_err = import_prism_bytes(&multi_xy_zip.finish().unwrap()).unwrap_err();
    assert!(
        matches!(multi_xy_err, FormatError::CsvParse { ref reason, .. } if reason.contains("budget")),
        "unexpected error for multi-CSV .prism over MAX_PROJECT_CELLS: {multi_xy_err:?}"
    );

    // Five smaller Column CSVs (1,000 cols * 2,500 rows = 2,500,000 cells each -> 12,500,000 total) must also fail cleanly.
    let mut col_csv = format!("C0{}\n", ",C".repeat(999));
    col_csv.push_str(&"1\n".repeat(2_500));
    let mut five_col_zip = ZipWriter::new();
    for i in 1..=5 {
        five_col_zip.add_file(&format!("tables/col_{i}.csv"), col_csv.as_bytes().to_vec()).unwrap();
    }
    let five_col_err = import_prism_bytes(&five_col_zip.finish().unwrap()).unwrap_err();
    assert!(
        matches!(five_col_err, FormatError::CsvParse { ref reason, .. } if reason.contains("budget")),
        "unexpected error for 5-CSV .prism over MAX_PROJECT_CELLS: {five_col_err:?}"
    );

    // Combined CSV (6,002,500) + embedded .pzfx (16,000 rows * 251 = 4,016,000) -> 10,018,500 > MAX_PROJECT_CELLS.
    let surv_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><TableSequence>{}</TableSequence></PrismFile>",
        build_large_survival_table_xml("s1", 16_000, 250)
    );
    let mut mixed_over_zip = ZipWriter::new();
    mixed_over_zip.add_file("manifest.json", make_manifest(&[("tables/t1.csv", "xy", "replicates", 50)]).into_bytes()).unwrap();
    mixed_over_zip.add_file("tables/t1.csv", xy_csv.as_bytes().to_vec()).unwrap();
    mixed_over_zip.add_file("embedded/extra.pzfx", surv_xml.into_bytes()).unwrap();
    let mixed_over_err = import_prism_bytes(&mixed_over_zip.finish().unwrap()).unwrap_err();
    assert!(
        matches!(mixed_over_err, FormatError::XmlParse { ref reason, .. } if reason.contains("budget") || reason.contains("cap")),
        "unexpected error for CSV + PZFX .prism over MAX_PROJECT_CELLS: {mixed_over_err:?}"
    );

    // Combined CSV (6,002,500) + small embedded .pzfx (2 cells) succeeds within budget.
    let small_pzfx = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile><Table ID=\"p1\" TableType=\"Column\"><Column><Subcolumn><d>1</d><d>2</d></Subcolumn></Column></Table></PrismFile>";
    let mut mixed_ok_zip = ZipWriter::new();
    mixed_ok_zip.add_file("manifest.json", make_manifest(&[("tables/t1.csv", "xy", "replicates", 50)]).into_bytes()).unwrap();
    mixed_ok_zip.add_file("tables/t1.csv", xy_csv.into_bytes()).unwrap();
    mixed_ok_zip.add_file("embedded/small.pzfx", small_pzfx.as_bytes().to_vec()).unwrap();
    let mixed_ok = import_prism_bytes(&mixed_ok_zip.finish().unwrap()).unwrap();
    assert_eq!(mixed_ok.document.data_sheets.len(), 2);
}
