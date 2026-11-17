//! Regression tests for `RV-53`: Unicode-safe, traversal-proof, and Windows-reserved-name-guarded
//! download filename sanitization in `curvebook-web`.

use curvebook_ui_egui::serde_json::json;
use curvebook_web::host_adapters::sanitize_download_stem;
use curvebook_web::{FileSystemAccessBridge, WebExportFormat, WebWorkspaceHost};

#[test]
fn rv53_web_download_filenames_preserve_cjk_cyrillic_arabic_hebrew_and_guard_reserved_stems() {
    // 1. CJK sheet titles (`数据`, `图表`) must stay distinct and never collapse to `curvebook-project`.
    assert_eq!(sanitize_download_stem("数据"), "数据");
    assert_eq!(sanitize_download_stem("图表"), "图表");
    assert_ne!(sanitize_download_stem("数据"), sanitize_download_stem("图表"));

    // 2. Cyrillic, Arabic, and Hebrew titles preserve Unicode alphanumerics.
    assert_eq!(sanitize_download_stem("График доза-ответ"), "График_доза-ответ");
    assert_eq!(sanitize_download_stem("منحنى الاستجابة"), "منحنى_الاستجابة");
    assert_eq!(sanitize_download_stem("עקומת מינון"), "עקומת_מינון");

    // 3. Path traversal, control bytes, and forbidden characters are stripped cleanly.
    assert_eq!(sanitize_download_stem("../foo"), "foo");
    assert_eq!(sanitize_download_stem("..\\..\\dir/foo:bar*baz?\"<qux>|\0"), "dir_foo_bar_baz_qux");
    assert_eq!(sanitize_download_stem("...///\\\\\\"), "curvebook-project");

    // 4. Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..COM9`, `LPT1..LPT9`) are guarded.
    for reserved in ["CON", "con", "../CON", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9"] {
        let safe = sanitize_download_stem(reserved);
        assert!(safe.starts_with('_'), "expected reserved stem `{reserved}` to be prefixed safely, got `{safe}`");
        assert!(!safe.eq_ignore_ascii_case(reserved.trim_start_matches("../")));
    }

    // 5. Length capping cuts cleanly on UTF-8 char boundaries without panicking.
    let long_unicode = "数据图表График".repeat(30);
    let capped = sanitize_download_stem(&long_unicode);
    assert!(!capped.is_empty());
    assert!(capped.chars().count() <= 64);
    assert!(capped.len() <= 120);

    // 6. End-to-end `FileSystemAccessBridge::export_bytes` CSV exports for `数据` and `图表` produce distinct `.csv` file names.
    let mut host = WebWorkspaceHost::new();
    let s1 = host.import_delimited_bytes(b"A,B\n1,2\n3,4\n", false, Some("数据"), Some("xy")).expect("import 数据 sheet");
    assert_eq!(s1.data_sheet_count, 1);
    let id1 = host.app().document().data_sheets[0].id.as_str().to_owned();

    let s2 = host.import_delimited_bytes(b"A,B\n5,6\n7,8\n", false, Some("图表"), Some("xy")).expect("import 图表 sheet");
    assert_eq!(s2.data_sheet_count, 2);
    let id2 = host.app().document().data_sheets[1].id.as_str().to_owned();

    let mut fs = FileSystemAccessBridge::new();
    let csv1 = fs.export_bytes(&mut host, WebExportFormat::Csv, Some(&id1), 300).expect("export 数据.csv");
    let csv2 = fs.export_bytes(&mut host, WebExportFormat::Csv, Some(&id2), 300).expect("export 图表.csv");
    assert_eq!(csv1.file_name, "数据.csv");
    assert_eq!(csv2.file_name, "图表.csv");
    assert_ne!(csv1.file_name, csv2.file_name);

    // Rename project to `CON` and verify archive export guards the reserved stem.
    let _ = host.execute_command("project.setTitle", json!({ "title": "CON" }));
    if host.app().document().title == "CON" {
        let archive = fs.export_bytes(&mut host, WebExportFormat::CurveBookArchive, None, 300).expect("export archive");
        assert_eq!(archive.file_name, "_CON.curvebook");
    }
}
