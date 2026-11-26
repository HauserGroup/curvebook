use std::collections::BTreeSet;
use std::fs;

use crate::common::{TempDirGuard, build_zip_entries, pzfx_tables, sample_checkpoint_project, sample_column_project};
use curvebook_doc::{Cell, CellCoord, TableArchetype};
use curvebook_format::prism::MAX_PROJECT_CELLS;
use curvebook_format::pzfx::MAX_IMPORT_CELLS;
use curvebook_format::{
    ArchiveKind, ArchiveManifest, FormatError, import_prism_bytes, import_pzfx_str, is_unpacked_project_dir, open_project_bytes,
    open_project_unpacked_dir, save_project_bytes, save_project_unpacked_dir, save_template_bytes,
};
use serde_json::json;

#[test]
fn zip_and_unpacked_dir_preserve_exclusion_notes_and_checkpoints() {
    let doc = sample_checkpoint_project();

    let zip_bytes = save_project_bytes(&doc).unwrap();
    let decoded_zip = open_project_bytes(&zip_bytes).unwrap();
    assert_eq!(decoded_zip, doc);
    assert_eq!(decoded_zip.data_sheets[0].exclusion_note(CellCoord::new(1, 0, 1)).unwrap().reason_tag, "pipetting_error");
    assert_eq!(
        (decoded_zip.data_sheets[0].exclusion_notes.len(), decoded_zip.checkpoints.len(), decoded_zip.checkpoints[0].id.as_str()),
        (1, 1, "cp-baseline")
    );

    let root = TempDirGuard::new("roundtrip").unwrap();
    let proj_dir = root.child("assay.curvebook.d");
    assert!(!is_unpacked_project_dir(&proj_dir));
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(is_unpacked_project_dir(&proj_dir));

    let decoded_dir = open_project_unpacked_dir(&proj_dir).unwrap();
    assert_eq!(decoded_dir, doc);
    assert_eq!(decoded_dir.data_sheets[1].exclusion_note(CellCoord::new(0, 1, 0)).unwrap().reason_tag, "hemolyzed");
    assert_eq!(decoded_dir.checkpoints, doc.checkpoints);

    let zip_file = root.child("assay.curvebook");
    fs::write(&zip_file, &zip_bytes).unwrap();
    let empty_dir = root.child("empty.curvebook.d");
    fs::create_dir_all(&empty_dir).unwrap();
    assert!(!is_unpacked_project_dir(&zip_file) && !is_unpacked_project_dir(&empty_dir));

    let tpl_dir = root.child("assay.cbtemplate.d");
    save_project_unpacked_dir(&doc, &tpl_dir).unwrap();
    assert!(!save_template_bytes(&doc).unwrap().is_empty());
    let mut tpl_manifest: ArchiveManifest = serde_json::from_slice(&fs::read(tpl_dir.join("manifest.json")).unwrap()).unwrap();
    tpl_manifest.kind = ArchiveKind::Template;
    fs::write(tpl_dir.join("manifest.json"), serde_json::to_vec_pretty(&tpl_manifest).unwrap()).unwrap();
    assert!(!is_unpacked_project_dir(&tpl_dir));
    assert!(matches!(open_project_unpacked_dir(&tpl_dir), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));
}

#[test]
fn unpacked_dir_is_deterministic_git_friendly_and_preserves_repo_files() {
    let mut doc = sample_checkpoint_project();
    let root = TempDirGuard::new("git-diff").unwrap();
    let proj_dir = root.child("study.curvebook.d");

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    fs::create_dir_all(proj_dir.join(".git")).unwrap();
    fs::write(proj_dir.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(proj_dir.join(".gitignore"), b"*.tmp\n").unwrap();
    fs::write(proj_dir.join("README.md"), b"# Study Repository\n").unwrap();

    let manifest_v1 = fs::read_to_string(proj_dir.join("manifest.json")).unwrap();
    let document_v1 = fs::read_to_string(proj_dir.join("document.json")).unwrap();
    let sheet1_v1 = fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap();
    let sheet2_v1 = fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap();
    assert!(
        manifest_v1.contains("\n  \"format\": \"curvebook-archive\"")
            && document_v1.contains("\n  \"schema_version\":")
            && !document_v1.contains('\t')
            && sheet2_v1.contains("Vehicle")
    );

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert_eq!(fs::read_to_string(proj_dir.join("manifest.json")).unwrap(), manifest_v1);
    assert_eq!(fs::read_to_string(proj_dir.join("document.json")).unwrap(), document_v1);
    assert_eq!(fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap(), sheet1_v1);
    assert_eq!(fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap(), sheet2_v1);

    if let TableArchetype::Xy(ref mut xy) = doc.data_sheets[0].archetype {
        xy.data_sets[0].cells[2][1] = Cell::number(91.25);
    }
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert_eq!(fs::read_to_string(proj_dir.join("sheets/data-col.csv")).unwrap(), sheet2_v1);
    assert_ne!(fs::read_to_string(proj_dir.join("sheets/data-xy.csv")).unwrap(), sheet1_v1);
    assert_eq!(fs::read(proj_dir.join(".git/HEAD")).unwrap(), b"ref: refs/heads/main\n");
    assert_eq!(fs::read(proj_dir.join(".gitignore")).unwrap(), b"*.tmp\n");
    assert_eq!(fs::read(proj_dir.join("README.md")).unwrap(), b"# Study Repository\n");

    let document_v2 = fs::read_to_string(proj_dir.join("document.json")).unwrap();
    let (lines1, lines2): (Vec<_>, Vec<_>) = (document_v1.lines().collect(), document_v2.lines().collect());
    assert_eq!(lines1.len(), lines2.len());
    assert_eq!(lines1.iter().zip(&lines2).filter(|(a, b)| a != b).count(), 1);
}

#[test]
fn unpacked_dir_prunes_stale_sheet_files_on_resave() {
    let mut doc = sample_checkpoint_project();
    let root = TempDirGuard::new("prune").unwrap();
    let proj_dir = root.child("prune.curvebook.d");

    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    fs::write(proj_dir.join("README.md"), b"keep me").unwrap();
    assert!(proj_dir.join("sheets/data-xy.csv").is_file() && proj_dir.join("sheets/data-col.csv").is_file());

    doc.data_sheets.retain(|ds| ds.id.as_str() != "data-col");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(proj_dir.join("sheets/data-xy.csv").is_file() && !proj_dir.join("sheets/data-col.csv").exists());
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), doc);

    doc.result_sheets.clear();
    doc.graph_sheets.clear();
    doc.layout_sheets.clear();
    doc.data_sheets.clear();
    doc.active_sheet_id = None;
    doc.family_anchor_sheet_id = None;
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    assert!(!proj_dir.join("sheets/data-xy.csv").exists() && !proj_dir.join("sheets").exists());
    assert_eq!(
        (fs::read(proj_dir.join("README.md")).unwrap().as_slice(), open_project_unpacked_dir(&proj_dir).unwrap()),
        (b"keep me".as_slice(), doc)
    );
}

#[test]
fn unpacked_dir_rejects_traversal_tampering_and_corrupt_entries() {
    let doc = sample_checkpoint_project();
    let root = TempDirGuard::new("security").unwrap();
    let proj_dir = root.child("sec.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();

    let orig_manifest_bytes = fs::read(proj_dir.join("manifest.json")).unwrap();
    let orig_manifest: ArchiveManifest = serde_json::from_slice(&orig_manifest_bytes).unwrap();
    for bad_rel in ["../outside.csv", "/etc/passwd", "sheets\\data-xy.csv", "C:/evil.csv", ""] {
        let mut tampered = orig_manifest.clone();
        tampered.sheets[0].csv_path = Some(String::from(bad_rel));
        fs::write(proj_dir.join("manifest.json"), serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
        assert!(!is_unpacked_project_dir(&proj_dir));
        assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::UnsafePath { .. })), "{bad_rel:?}");
    }
    fs::write(proj_dir.join("manifest.json"), &orig_manifest_bytes).unwrap();

    let orig_csv = fs::read(proj_dir.join("sheets/data-xy.csv")).unwrap();
    fs::write(proj_dir.join("sheets/data-xy.csv"), b"tampered,csv\n").unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::CorruptCrc { .. })));

    fs::remove_file(proj_dir.join("sheets/data-xy.csv")).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::MissingArchiveEntry { .. })));
    fs::write(proj_dir.join("sheets/data-xy.csv"), &orig_csv).unwrap();

    fs::write(proj_dir.join("manifest.json"), [0xff_u8, 0xfe, 0x00]).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::InvalidZip { .. })));

    // Malicious .curvebook.tx/COMMITTING listing arbitrary non-stage paths must fail without deleting user/.git files,
    // and valid stage names must not be cleaned up before snapshot validation succeeds.
    let tx_root = root.child("tx_sec.curvebook.d");
    let tx_doc = sample_column_project("Unpacked Security");
    save_project_unpacked_dir(&tx_doc, &tx_root).unwrap();
    fs::create_dir_all(tx_root.join(".git")).unwrap();
    fs::write(tx_root.join(".git/config"), b"[core]\n").unwrap();
    fs::write(tx_root.join("README.md"), b"# Notes\n").unwrap();
    fs::write(tx_root.join("analysis.py"), b"print(1)\n").unwrap();
    let tx_dir = tx_root.join(".curvebook.tx");
    fs::create_dir_all(&tx_dir).unwrap();
    fs::write(
        tx_dir.join("COMMITTING"),
        serde_json::to_vec(&json!({
            "format": "curvebook-unpacked-tx", "version": 1,
            "stage_paths": [".git/config", "README.md", "analysis.py"], "prior_csv_paths": [], "new_csv_paths": []
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(save_project_unpacked_dir(&tx_doc, &tx_root), Err(FormatError::UnsafePath { .. })));
    assert!(tx_root.join(".git/config").exists() && tx_root.join("README.md").exists() && tx_root.join("analysis.py").exists());

    let dummy_stage = tx_root.join(".document.json.stage.1234.9");
    fs::write(&dummy_stage, b"staged").unwrap();
    fs::write(
        tx_dir.join("COMMITTING"),
        serde_json::to_vec(&json!({
            "format": "curvebook-unpacked-tx", "version": 1,
            "stage_paths": [".document.json.stage.1234.9"], "prior_csv_paths": [], "new_csv_paths": []
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(save_project_unpacked_dir(&tx_doc, &tx_root), Err(FormatError::MissingArchiveEntry { .. })));
    assert!(dummy_stage.exists());
}

#[cfg(unix)]
#[test]
fn unpacked_dir_rejects_symlinks_escaping_root_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let doc = sample_checkpoint_project();
    let root = TempDirGuard::new("symlinks").unwrap();
    let outside_dir = root.child("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    let outside_file = outside_dir.join("secret.csv");
    fs::write(&outside_file, b"top,secret\n").unwrap();

    let cases = [
        ("proj_a.curvebook.d", "sheets/data-xy.csv", false, outside_file.clone(), false),
        ("proj_b.curvebook.d", "sheets", true, outside_dir.clone(), false),
        ("proj_c.curvebook.d", "document.json", false, outside_file.clone(), true),
        ("proj_d.curvebook.d", "sheets/data-xy.csv", false, outside_dir.join("does_not_exist.csv"), false),
    ];
    for (dir_name, rel, is_dir, target, check_is_unpacked_false) in cases {
        let proj = root.child(dir_name);
        save_project_unpacked_dir(&doc, &proj).unwrap();
        if is_dir {
            fs::remove_dir_all(proj.join(rel)).unwrap();
        } else {
            fs::remove_file(proj.join(rel)).unwrap();
        }
        symlink(&target, proj.join(rel)).unwrap();
        if check_is_unpacked_false {
            assert!(!is_unpacked_project_dir(&proj), "{dir_name}");
        }
        assert!(matches!(open_project_unpacked_dir(&proj), Err(FormatError::UnsafePath { .. })), "{dir_name}: open");
        assert!(matches!(save_project_unpacked_dir(&doc, &proj), Err(FormatError::UnsafePath { .. })), "{dir_name}: save");
    }
    assert_eq!(fs::read(&outside_file).unwrap(), b"top,secret\n");
    let remaining: BTreeSet<String> = fs::read_dir(&outside_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(remaining, BTreeSet::from([String::from("secret.csv")]));
}

fn build_large_table_xml(id: &str, attrs: &str, x_subcols: usize, rows: usize, y_prefix: &str, y_cols: usize) -> String {
    let mut xml = format!("<Table ID=\"{id}\" {attrs}><XColumn>");
    let sub_body: String = (0..rows).map(|r| format!("<d>{}</d>", r + 1)).collect();
    for _ in 0..x_subcols {
        xml.push_str("<Subcolumn>");
        xml.push_str(&sub_body);
        xml.push_str("</Subcolumn>");
    }
    xml.push_str("</XColumn>");
    for c in 0..y_cols {
        xml.push_str(&format!("<YColumn><Title>{y_prefix}{c}</Title></YColumn>"));
    }
    xml.push_str("</Table>");
    xml
}

#[test]
fn pzfx_multi_table_cell_budget_enforcement() {
    let t1 = build_large_table_xml("t1", "TableType=\"XY\" YFormat=\"replicates\" Replicates=\"50\"", 1, 2_500, "Y", 48);
    let t2 = build_large_table_xml("t2", "TableType=\"XY\" YFormat=\"replicates\" Replicates=\"50\"", 1, 2_500, "Y", 48);
    let s1 = build_large_table_xml("s1", "TableType=\"Survival\"", 1, 16_000, "Cohort", 250);
    let xerr = build_large_table_xml("xerr1", "TableType=\"XY\" XFormat=\"err\" YFormat=\"replicates\" Replicates=\"50\"", 3, 2_500, "Y", 48);

    for (label, table, expected_slots) in [("xy", &t1, 6_002_500), ("survival", &s1, 16_000 * 251), ("xerr", &xerr, 6_007_500)] {
        let slots = import_pzfx_str(&pzfx_tables(table)).unwrap().document.data_sheets[0].archetype.allocated_cell_slots();
        assert_eq!(slots, expected_slots, "{label}");
        assert!(slots <= MAX_IMPORT_CELLS);
    }

    for (label, second) in [("xy+xy", &t2), ("xy+survival", &s1), ("xy+xerr", &xerr)] {
        let err = import_pzfx_str(&pzfx_tables(&format!("{t1}{second}"))).unwrap_err();
        assert!(matches!(err, FormatError::XmlParse { ref reason, .. } if reason.contains("budget") || reason.contains("cap")), "{label}: {err:?}");
    }
}

#[test]
fn prism_multi_entry_cumulative_project_cell_budget_enforcement() {
    assert_eq!(MAX_PROJECT_CELLS, MAX_IMPORT_CELLS);

    let xy_csv = format!("X{}\n{}", ",Y".repeat(2_400), "0\n".repeat(2_500));
    let col_csv = format!("C0{}\n{}", ",C".repeat(999), "1\n".repeat(2_500));
    let make_manifest = |paths: &[&str]| -> Vec<u8> {
        let sheets: Vec<String> =
            paths.iter().map(|p| format!("{{\"csv_path\":\"{p}\",\"kind\":\"xy\",\"subcolumn_mode\":\"replicates\",\"replicates\":50}}")).collect();
        format!("{{\"title\":\"Budget Test\",\"sheets\":[{}]}}", sheets.join(",")).into_bytes()
    };

    // Single XY CSV (6,002,500 <= MAX_PROJECT_CELLS) succeeds.
    let single_zip = build_zip_entries(&[("manifest.json", &make_manifest(&["tables/t1.csv"])), ("tables/t1.csv", xy_csv.as_bytes())]);
    assert_eq!(import_prism_bytes(&single_zip).unwrap().document.data_sheets[0].archetype.allocated_cell_slots(), 6_002_500);

    // Two XY CSVs (12,005,000) & Five Column CSVs (12,500,000) exceed MAX_PROJECT_CELLS.
    let two_xy = build_zip_entries(&[
        ("manifest.json", &make_manifest(&["tables/t1.csv", "tables/t2.csv"])),
        ("tables/t1.csv", xy_csv.as_bytes()),
        ("tables/t2.csv", xy_csv.as_bytes()),
    ]);
    assert!(matches!(import_prism_bytes(&two_xy), Err(FormatError::CsvParse { ref reason, .. }) if reason.contains("budget")));

    let five_col = build_zip_entries(&[
        ("tables/col_1.csv", col_csv.as_bytes()),
        ("tables/col_2.csv", col_csv.as_bytes()),
        ("tables/col_3.csv", col_csv.as_bytes()),
        ("tables/col_4.csv", col_csv.as_bytes()),
        ("tables/col_5.csv", col_csv.as_bytes()),
    ]);
    assert!(matches!(import_prism_bytes(&five_col), Err(FormatError::CsvParse { ref reason, .. }) if reason.contains("budget")));

    // Combined CSV (6,002,500) + embedded .pzfx Survival (4,016,000) > MAX_PROJECT_CELLS.
    let surv_xml = pzfx_tables(&build_large_table_xml("s1", "TableType=\"Survival\"", 1, 16_000, "Cohort", 250));
    let mixed_over = build_zip_entries(&[
        ("manifest.json", &make_manifest(&["tables/t1.csv"])),
        ("tables/t1.csv", xy_csv.as_bytes()),
        ("embedded/extra.pzfx", surv_xml.as_bytes()),
    ]);
    assert!(
        matches!(import_prism_bytes(&mixed_over), Err(FormatError::XmlParse { ref reason, .. }) if reason.contains("budget") || reason.contains("cap"))
    );

    // Combined CSV (6,002,500) + small embedded .pzfx (2 cells) succeeds.
    let small_pzfx = pzfx_tables("<Table ID=\"p1\" TableType=\"Column\"><Column><Subcolumn><d>1</d><d>2</d></Subcolumn></Column></Table>");
    let mixed_ok = build_zip_entries(&[
        ("manifest.json", &make_manifest(&["tables/t1.csv"])),
        ("tables/t1.csv", xy_csv.as_bytes()),
        ("embedded/small.pzfx", small_pzfx.as_bytes()),
    ]);
    assert_eq!(import_prism_bytes(&mixed_ok).unwrap().document.data_sheets.len(), 2);
}
