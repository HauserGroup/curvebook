#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet,
    DescriptiveSpec, FloatingNote, GraphSheet, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, JournalWidthPreset,
    LayoutItem, LayoutSheet, LayoutSpec, LinkSource, NamedCheckpoint, NonlinearFitSpec, OneWayAnovaSpec, PairwiseBracketSpec, ParameterRule,
    ParameterScope, PlacementMode, ProjectDocument, RectPt, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SheetCellRange, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_format::recovery::{CrashRecoveryJournal, DirectoryJournalBackend, JournalLockLease, LOCKFILE_NAME, RecoveryReport};
use curvebook_format::{FormatError, ImportBaselineAdjustment, ZipWriter, apply_csv_import_link};
use flate2::Compression;
use flate2::write::DeflateEncoder;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::{Value, json};

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct TempDirGuard {
    path: PathBuf,
    #[cfg(unix)]
    restore_writable: Vec<PathBuf>,
}

impl TempDirGuard {
    pub fn new(label: &str) -> std::io::Result<Self> {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let clean: String = label.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
        let pid = std::process::id();
        let dir_name = if clean.is_empty() { format!("cb-format-{pid}-{seq}") } else { format!("cb-format-{clean}-{pid}-{seq}") };
        let path = std::env::temp_dir().join(dir_name);
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            #[cfg(unix)]
            restore_writable: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    #[cfg(unix)]
    pub fn track_chmod_restore(&mut self, dir: PathBuf) {
        self.restore_writable.push(dir);
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for dir in &self.restore_writable {
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
            }
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn create_sparse_file(path: &Path, byte_len: u64) -> std::io::Result<()> {
    File::create(path)?.set_len(byte_len)
}

pub fn assert_no_lock_temp_or_evict_leftovers(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.ends_with(".tmp") && !name.contains(".tmp.") && !name.ends_with(".evict"), "leftover `{name}` in `{}`", dir.display());
    }
}

#[cfg(unix)]
pub fn run_with_fifo_watchdog<F>(fifo_path: &Path, label: &str, op: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(op());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(res) => res,
        Err(err) => {
            if let Ok(writer) = fs::OpenOptions::new().write(true).open(fifo_path) {
                drop(writer);
            }
            Err(format!("hung on FIFO `{label}`: {err}"))
        }
    }
}

pub fn write_stale_lock(dir: &Path, owner: &str, acquired_ms: u64, ttl_ms: u64) {
    let lease = JournalLockLease::new(owner, acquired_ms, ttl_ms).unwrap();
    fs::write(dir.join(LOCKFILE_NAME), serde_json::to_vec(&lease).unwrap()).unwrap();
}

pub fn open_dir_journal(
    dir: &Path,
    owner: &str,
    ttl_ms: u64,
    now_ms: u64,
) -> Result<(CrashRecoveryJournal<DirectoryJournalBackend>, RecoveryReport), FormatError> {
    CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(dir)?, owner, ttl_ms, now_ms)
}

// ============================================================================
// ZIP, Archive, JSON Bomb, XML & XLSX Builders
// ============================================================================

pub fn build_zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new();
    for (name, bytes) in entries {
        writer.add_file(name, bytes.to_vec()).unwrap();
    }
    writer.finish().unwrap()
}

pub fn build_zip_str_entries(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = ZipWriter::new();
    for (name, text) in entries {
        writer.add_file(name, text.as_bytes().to_vec()).unwrap();
    }
    writer.finish().unwrap()
}

pub fn build_archive_zip(doc_bytes: &[u8], manifest_schema_version: u32, kind: &str, title: &str) -> Vec<u8> {
    let manifest = json!({
        "format": "curvebook-archive", "schema_version": manifest_schema_version, "kind": kind,
        "title": title, "document_crc32": crc32fast::hash(doc_bytes), "sheets": []
    });
    build_zip_entries(&[("document.json", doc_bytes), ("manifest.json", &serde_json::to_vec(&manifest).unwrap())])
}

pub fn build_archive_zip_from_value(doc_json: &Value, schema_version: u32) -> Vec<u8> {
    let title = doc_json.get("title").and_then(Value::as_str).unwrap_or("Test");
    build_archive_zip(&serde_json::to_vec(doc_json).unwrap(), schema_version, "Project", title)
}

pub struct RawZipEntry<'a> {
    pub cd_name: &'a [u8],
    pub local_name: Option<&'a [u8]>,
    pub flags: u16,
    pub method: u16,
    pub crc: Option<u32>,
    pub declared_compressed: Option<u32>,
    pub declared_uncompressed: Option<u32>,
    pub payload: &'a [u8],
    pub compressed_override: Option<Vec<u8>>,
}

impl<'a> RawZipEntry<'a> {
    pub fn stored(name: &'a [u8], payload: &'a [u8]) -> Self {
        Self {
            cd_name: name,
            local_name: None,
            flags: 0x0800,
            method: 0,
            crc: None,
            declared_compressed: None,
            declared_uncompressed: None,
            payload,
            compressed_override: None,
        }
    }

    pub fn deflated(name: &'a [u8], payload: &'a [u8]) -> Self {
        Self { method: 8, ..Self::stored(name, payload) }
    }
}

pub fn deflate_raw(data: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn push_u16s(out: &mut Vec<u8>, vals: &[u16]) {
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u32s(out: &mut Vec<u8>, vals: &[u32]) {
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

pub fn build_raw_zip(entries: &[RawZipEntry<'_>], eocd_comment: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::with_capacity(entries.len());

    for e in entries {
        let local_off = u32::try_from(out.len()).unwrap();
        let comp = match &e.compressed_override {
            Some(buf) => buf.clone(),
            None if e.method == 8 => deflate_raw(e.payload),
            None => e.payload.to_vec(),
        };
        let crc = e.crc.unwrap_or_else(|| crc32fast::hash(e.payload));
        let comp_len = e.declared_compressed.unwrap_or_else(|| u32::try_from(comp.len()).unwrap());
        let uncomp_len = e.declared_uncompressed.unwrap_or_else(|| u32::try_from(e.payload.len()).unwrap());
        let local_name = e.local_name.unwrap_or(e.cd_name);

        push_u32s(&mut out, &[0x0403_4b50]);
        push_u16s(&mut out, &[20, e.flags, e.method, 0, 0x0021]);
        push_u32s(&mut out, &[crc, comp_len, uncomp_len]);
        push_u16s(&mut out, &[u16::try_from(local_name.len()).unwrap(), 0]);
        out.extend_from_slice(local_name);
        out.extend_from_slice(&comp);
        cd.push((e.cd_name, e.flags, e.method, crc, comp_len, uncomp_len, local_off));
    }

    let cd_offset = u32::try_from(out.len()).unwrap();
    for &(name, flags, method, crc, comp_len, uncomp_len, local_off) in &cd {
        push_u32s(&mut out, &[0x0201_4b50]);
        push_u16s(&mut out, &[0x0314, 20, flags, method, 0, 0x0021]);
        push_u32s(&mut out, &[crc, comp_len, uncomp_len]);
        push_u16s(&mut out, &[u16::try_from(name.len()).unwrap()]);
        out.extend_from_slice(&[0_u8; 12]);
        push_u32s(&mut out, &[local_off]);
        out.extend_from_slice(name);
    }
    let cd_size = u32::try_from(out.len()).unwrap() - cd_offset;
    let count = u16::try_from(cd.len()).unwrap();
    push_u32s(&mut out, &[0x0605_4b50, 0]);
    push_u16s(&mut out, &[count, count]);
    push_u32s(&mut out, &[cd_size, cd_offset]);
    push_u16s(&mut out, &[u16::try_from(eocd_comment.len()).unwrap()]);
    out.extend_from_slice(eocd_comment);
    out
}

pub fn make_nested_empty_array_json(prefix: &str, count: usize, suffix: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + count * 3 + suffix.len() + 2);
    out.extend_from_slice(prefix.as_bytes());
    for i in 0..count {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"[]");
    }
    out.extend_from_slice(suffix.as_bytes());
    out
}

pub fn make_array_bomb_json(count: usize) -> Vec<u8> {
    make_nested_empty_array_json("[", count, "]")
}
pub fn pzfx_doc(inner: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PrismFile>{inner}</PrismFile>")
}
pub fn pzfx_tables(tables: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PrismFile><TableSequence>{tables}</TableSequence></PrismFile>")
}

pub fn xlsx_single_sheet_zip(sheet_name: &str, rows_xml: &str) -> Vec<u8> {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let wb = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{sheet_name}" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows_xml}</sheetData></worksheet>"#
    );
    build_zip_str_entries(&[
        ("_rels/.rels", rels),
        ("xl/workbook.xml", &wb),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

// ============================================================================
// Image & Document/Sheet/Import Fixtures
// ============================================================================

fn encode_img(img: DynamicImage, fmt: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), fmt).unwrap();
    out
}

pub fn encode_rgba_png(w: u32, h: u32) -> Vec<u8> {
    encode_img(
        DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 40) as u8, (y * 55) as u8, 200, if (x + y).is_multiple_of(2) { 255 } else { 128 }])
        })),
        ImageFormat::Png,
    )
}

pub fn encode_rgb_jpeg(w: u32, h: u32) -> Vec<u8> {
    encode_img(DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| Rgb([((x * 60) % 255) as u8, 90, ((y * 70) % 255) as u8]))), ImageFormat::Jpeg)
}

pub fn encode_rgba_tiff(w: u32, h: u32) -> Vec<u8> {
    encode_img(DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, y| Rgba([30, (x * 50) as u8, (y * 60) as u8, 210]))), ImageFormat::Tiff)
}

pub fn rnum(value: f64) -> ResultCell {
    ResultCell::Number { value, prefix: None, units: None }
}

pub fn col_sheet(id: &str, cols: usize, rows: usize) -> DataSheet {
    DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).unwrap())).unwrap()
}

pub fn xy_sheet(id: &str, mode: SubcolumnMode, cols: usize, rows: usize) -> DataSheet {
    DataSheet::new(id, id, TableArchetype::Xy(XyTable::new(mode, cols, rows).unwrap())).unwrap()
}

pub fn result_sheet_with_kind(id: &str, src: &str, kind: AnalysisKind) -> ResultSheet {
    ResultSheet::new(AnalysisSpec::new(id, src, kind).unwrap()).unwrap()
}

pub fn param_rule(name: &str, scope: ParameterScope) -> ParameterRule {
    ParameterRule { name: String::from(name), scope, initial_override: None, lower_bound: None, upper_bound: None }
}

pub fn live_graph_item(id: &str, graph_id: &str) -> LayoutItem {
    LayoutItem::LiveGraph {
        id: String::from(id),
        graph_sheet_id: SheetId::new(graph_id),
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 90.0),
        scale_percent: 100.0,
        badge_override: None,
    }
}

pub fn callout_item(id: &str, markdown: &str) -> LayoutItem {
    LayoutItem::TextCallout {
        id: String::from(id),
        markdown: String::from(markdown),
        bounds_pt: RectPt::new(10.0, 110.0, 120.0, 40.0),
        font_size_pt: 10.0,
    }
}

pub fn bracket_spec(result_id: &str) -> PairwiseBracketSpec {
    PairwiseBracketSpec {
        result_sheet_id: SheetId::new(result_id),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 10.0,
        bracket_step_pt: 12.0,
    }
}

pub fn make_result_sheet(id: &str, source_id: &str, status: ResultStatus) -> ResultSheet {
    let mut rs = result_sheet_with_kind(id, source_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    rs.status = status;
    rs.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("Mean")],
        rows: vec![vec![ResultCell::Text(String::from("Alpha")), rnum(3.25)]],
        footnotes: Vec::new(),
    });
    rs
}

pub fn raw_datasheet(id: &str, archetype: TableArchetype) -> DataSheet {
    DataSheet {
        id: SheetId::new(id),
        title: format!("Sheet {id}"),
        archetype,
        frozen: false,
        info_sheet_id: None,
        linked_regions: Vec::new(),
        floating_notes: Vec::new(),
        color_tag: None,
        exclusion_notes: Vec::new(),
        plate_well_origins: Vec::new(),
        origin: None,
    }
}

pub fn make_import_link_spec(mode: PlacementMode, anchor: CellCoord, propagate_title_header: bool) -> ImportLinkSpec {
    ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("test.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode, anchor, propagate_title_header, skip_contiguous_blanks: false, transpose: false },
    }
}

pub fn apply_clip(sheet: &mut DataSheet, csv: &str, mode: PlacementMode, anchor: CellCoord, has_header: bool) -> Result<SheetCellRange, FormatError> {
    let spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode, anchor, propagate_title_header: has_header, ..ImportPlacementSpec::default() },
    };
    apply_csv_import_link(sheet, csv, &spec, has_header, ImportBaselineAdjustment::None, "l_clip", "clip")
}

pub fn sample_column_project(title: &str) -> ProjectDocument {
    let mut doc = ProjectDocument::new(title).unwrap();
    let mut sheet = col_sheet("data-1", 2, 3);
    sheet.title = String::from("Cohort Table");
    sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(1.5)).unwrap();
    sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(2.5)).unwrap();
    doc.add_data_sheet(sheet).unwrap();
    doc
}

pub fn sample_audit_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("R2 Audit Study").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap();
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[0].cells = vec![vec![Cell::number(1.25)], vec![Cell::number(2.5)]];
    col.data_sets[1].cells = vec![vec![Cell::number(9.75)], vec![Cell::number(10.5)]];
    let mut ds = DataSheet::new("sheet-a", "Cohort A", TableArchetype::Column(col)).unwrap();
    ds.set_excluded_with_reason(CellCoord::new(1, 1, 0), "hemolyzed", "Patient sample #4 hemolyzed", "2026-11-15T10:00:00Z").unwrap();
    doc.add_data_sheet(ds).unwrap();

    let mut rs = result_sheet_with_kind("res-1", "sheet-a", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()));
    rs.status = ResultStatus::UpToDate;
    rs.scalars.push(ResultScalarEntry { key: String::from("Mean"), data_set_index: Some(0), value: 1.875, units: Some(String::from("nM")) });
    doc.add_result_sheet(rs).unwrap();

    let mut snap = doc.clone();
    snap.checkpoints.clear();
    doc.checkpoints.push(NamedCheckpoint {
        id: String::from("cp-1"),
        name: String::from("Pre-QC"),
        note: String::from("Private cohort snapshot"),
        created_utc: String::from("2026-11-15T10:05:00Z"),
        snapshot_json: serde_json::to_string(&snap).unwrap(),
    });
    doc
}

pub fn sample_dose_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Dose Response Assay").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata").unwrap();
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM"))).unwrap();
    info.notes_markdown = String::from("Room temperature incubation 30 min.");
    doc.add_info_sheet(info).unwrap();

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    xy.x_title = String::from("Log[Agonist]");
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    xy.data_sets[0].cells[0] = vec![Cell::number(10.5), Cell::excluded_number(99.9)];
    xy.data_sets[0].cells[1] = vec![Cell::number(48.2), Cell::number(51.0)];
    xy.data_sets[1].cells[0] = vec![Cell::number(12.0), Cell::number(11.4)];
    for (r, x) in [-9.0, -8.0, -7.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(*x)).unwrap();
    }
    let mut ds = DataSheet::new("data-xy", "Binding Curve", TableArchetype::Xy(xy)).unwrap();
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    ds.floating_notes.push(FloatingNote::new("note-1", RectPt::new(10.0, 10.0, 120.0, 50.0), "Outlier flagged"));
    doc.add_data_sheet(ds.clone()).unwrap();

    let mut fit = NonlinearFitSpec::new("four_pl");
    let mut rule = param_rule(
        "HotNM",
        ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("HotNM"),
        }),
    );
    rule.lower_bound = Some(0.0);
    fit.parameter_rules.push(rule);
    let mut res = result_sheet_with_kind("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit));
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 1.25e-8, units: Some(String::from("M")) });
    doc.add_result_sheet(res).unwrap();

    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &ds).unwrap()).unwrap();
    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    lspec.items.push(live_graph_item("panel-a", "graph-1"));
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).unwrap()).unwrap();
    doc
}

pub fn sample_checkpoint_project() -> ProjectDocument {
    let mut doc = sample_dose_project();
    doc.title = String::from("Checkpoint & Exclusion Assay");
    if let TableArchetype::Xy(ref mut xy) = doc.data_sheets[0].archetype {
        xy.data_sets[0].cells[2] = vec![Cell::number(88.1), Cell::number(89.5)];
    }
    doc.data_sheets[0]
        .set_excluded_with_reason(CellCoord::new(1, 0, 1), "pipetting_error", "Bubble observed in well B2", "2026-11-14T08:00:00Z")
        .unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap();
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[0].cells = vec![vec![Cell::number(4.5)], vec![Cell::number(4.8)]];
    col.data_sets[1].cells = vec![vec![Cell::number(9.2)], vec![Cell::number(9.7)]];
    let mut ds2 = DataSheet::new("data-col", "Cohort Summary", TableArchetype::Column(col)).unwrap();
    ds2.set_excluded_with_reason(CellCoord::new(0, 1, 0), "hemolyzed", "Sample hemolyzed on visual QC", "2026-11-14T08:05:00Z").unwrap();
    doc.add_data_sheet(ds2).unwrap();

    let mut snap_doc = doc.clone();
    snap_doc.checkpoints.clear();
    doc.checkpoints.push(NamedCheckpoint {
        id: String::from("cp-baseline"),
        name: String::from("Baseline Before QC"),
        note: String::from("Captured prior to secondary plate exclusion"),
        created_utc: String::from("2026-11-14T08:10:00Z"),
        snapshot_json: serde_json::to_string_pretty(&snap_doc).unwrap(),
    });
    doc
}
