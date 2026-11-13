//! Native `.lumencraft` and `.lctemplate` ZIP archive serialization, deserialization,
//! schema migration, dimension/float validation, and circular dependency guards.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use lumencraft_doc::{
    CURRENT_SCHEMA_VERSION, Cell, CellValue, CloneDataSheetOptions, ConstantValue, DataSetColumn, DataSheet, GraphAnnotation, GraphSheet, HookSource,
    InfoSheet, LayoutItem, LayoutSheet, MAX_DATA_SETS, MAX_ROWS, MAX_SHEETS_PER_PROJECT, MAX_SUBCOLUMNS, ProjectDocument, ResultSheet, ResultStatus,
    SheetId, SheetSection, TableArchetype, validate_data_sets, validate_rows, validate_subcolumns, validate_title,
};

use crate::csv::{CsvOptions, export_datasheet_to_csv};
use crate::error::FormatError;
use crate::zip::{ZipArchive, ZipLimits, ZipWriter};

const MANIFEST_PATH: &str = "manifest.json";
const DOCUMENT_PATH: &str = "document.json";
const FORMAT_MAGIC: &str = "lumencraft-archive";

/// Kind of native LumenCraft ZIP archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ArchiveKind {
    Project,
    Template,
}

impl ArchiveKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Template => "template",
        }
    }
}

/// Index entry inside [`ArchiveManifest`] pointing to per-sheet human-inspectable payloads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SheetManifestEntry {
    pub id: SheetId,
    pub section: SheetSection,
    pub title: String,
    pub csv_path: Option<String>,
    pub crc32: Option<u32>,
}

/// Top-level `manifest.json` stored inside `.lumencraft` and `.lctemplate` archives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchiveManifest {
    pub format: String,
    pub schema_version: u32,
    pub kind: ArchiveKind,
    pub title: String,
    pub document_crc32: u32,
    pub sheets: Vec<SheetManifestEntry>,
}

/// Serialize a [`ProjectDocument`] into deterministic `.lumencraft` archive bytes.
pub fn save_project_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    save_archive_bytes(doc, ArchiveKind::Project)
}

/// Serialize a [`ProjectDocument`] into deterministic `.lctemplate` archive bytes, stripping
/// project-specific raw data cell values and cached results while preserving schemas, specs,
/// graphs, layouts, and info constants.
pub fn save_template_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    let templated = strip_project_data_for_template(doc)?;
    save_archive_bytes(&templated, ArchiveKind::Template)
}

/// Load and validate a `.lumencraft` project archive from raw bytes.
pub fn open_project_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Project {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Project.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load and validate a `.lctemplate` archive from raw bytes.
pub fn open_template_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Template {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Template.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load a `.lctemplate` (or `.lumencraft`) archive and instantiate a clean [`ProjectDocument`]
/// ready for new experimental data, optionally assigning a new project title.
pub fn instantiate_template_bytes(bytes: &[u8], new_title: Option<&str>) -> Result<ProjectDocument, FormatError> {
    let (_, raw_doc) = open_archive_bytes(bytes)?;
    let mut doc = strip_project_data_for_template(&raw_doc)?;
    if let Some(t) = new_title {
        validate_title(t)?;
        doc.title = t.to_owned();
    }
    Ok(doc)
}

/// Load any native LumenCraft archive (`.lumencraft` or `.lctemplate`), returning its [`ArchiveKind`]
/// and validated, schema-migrated [`ProjectDocument`].
pub fn open_archive_bytes(bytes: &[u8]) -> Result<(ArchiveKind, ProjectDocument), FormatError> {
    open_archive_bytes_with_limits(bytes, ZipLimits::default())
}

/// Load a native archive with explicit [`ZipLimits`] for security testing.
pub fn open_archive_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<(ArchiveKind, ProjectDocument), FormatError> {
    let archive = ZipArchive::from_bytes_with_limits(bytes, limits)?;
    let manifest_text = archive.get_utf8(MANIFEST_PATH)?.ok_or_else(|| FormatError::MissingArchiveEntry { path: String::from(MANIFEST_PATH) })?;
    let manifest: ArchiveManifest = serde_json::from_str(manifest_text)?;
    if manifest.format != FORMAT_MAGIC {
        return Err(FormatError::UnsupportedFormat { reason: format!("unexpected archive format `{}`", manifest.format) });
    }
    if manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: manifest.schema_version, max_supported: CURRENT_SCHEMA_VERSION });
    }

    let doc_bytes = archive.get(DOCUMENT_PATH).ok_or_else(|| FormatError::MissingArchiveEntry { path: String::from(DOCUMENT_PATH) })?;
    let actual_doc_crc = crc32fast::hash(doc_bytes);
    if actual_doc_crc != manifest.document_crc32 {
        return Err(FormatError::CorruptCrc { entry: String::from(DOCUMENT_PATH), expected: manifest.document_crc32, actual: actual_doc_crc });
    }

    for entry in &manifest.sheets {
        if let (Some(csv_path), Some(expected_crc)) = (&entry.csv_path, entry.crc32) {
            let csv_bytes = archive.get(csv_path).ok_or_else(|| FormatError::MissingArchiveEntry { path: csv_path.clone() })?;
            let actual_crc = crc32fast::hash(csv_bytes);
            if actual_crc != expected_crc {
                return Err(FormatError::CorruptCrc { entry: csv_path.clone(), expected: expected_crc, actual: actual_crc });
            }
        }
    }

    let raw_value: serde_json::Value = serde_json::from_slice(doc_bytes)?;
    let doc_version =
        raw_value.get("schema_version").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(manifest.schema_version);
    if doc_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: doc_version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    let migrated_value = migrate_document_json(raw_value, doc_version)?;
    let doc: ProjectDocument = serde_json::from_value(migrated_value)?;
    validate_project_document(&doc)?;
    Ok((manifest.kind, doc))
}

/// Write a `.lumencraft` archive to disk.
pub fn save_project_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_project_bytes(doc)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a `.lumencraft` archive from disk.
pub fn open_project_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = std::fs::read(path)?;
    open_project_bytes(&bytes)
}

/// Write a `.lctemplate` archive to disk.
pub fn save_template_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_template_bytes(doc)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a `.lctemplate` archive from disk.
pub fn open_template_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = std::fs::read(path)?;
    open_template_bytes(&bytes)
}

fn save_archive_bytes(doc: &ProjectDocument, kind: ArchiveKind) -> Result<Vec<u8>, FormatError> {
    validate_project_document(doc)?;

    let mut normalized = doc.clone();
    normalized.schema_version = CURRENT_SCHEMA_VERSION;
    let doc_json = serde_json::to_vec_pretty(&normalized)?;
    let doc_crc = crc32fast::hash(&doc_json);

    let mut writer = ZipWriter::new();
    writer.add_file(DOCUMENT_PATH, doc_json)?;

    let mut manifest_sheets = Vec::with_capacity(normalized.total_sheet_count());
    let csv_opts = CsvOptions::csv();
    let mut used_paths = HashSet::new();

    for (idx, ds) in normalized.data_sheets.iter().enumerate() {
        let csv_path = unique_sheet_csv_path(idx, &ds.id, &mut used_paths);
        let csv_bytes = export_datasheet_to_csv(ds, &csv_opts)?.into_bytes();
        let crc = crc32fast::hash(&csv_bytes);
        writer.add_file(&csv_path, csv_bytes)?;
        manifest_sheets.push(SheetManifestEntry {
            id: ds.id.clone(),
            section: SheetSection::Data,
            title: ds.title.clone(),
            csv_path: Some(csv_path),
            crc32: Some(crc),
        });
    }

    push_metadata_entries(&mut manifest_sheets, &normalized);

    let manifest = ArchiveManifest {
        format: String::from(FORMAT_MAGIC),
        schema_version: CURRENT_SCHEMA_VERSION,
        kind,
        title: normalized.title.clone(),
        document_crc32: doc_crc,
        sheets: manifest_sheets,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    writer.add_file(MANIFEST_PATH, manifest_json)?;
    writer.finish()
}

fn push_metadata_entries(entries: &mut Vec<SheetManifestEntry>, doc: &ProjectDocument) {
    for info in &doc.info_sheets {
        entries.push(SheetManifestEntry { id: info.id.clone(), section: SheetSection::Info, title: info.title.clone(), csv_path: None, crc32: None });
    }
    for res in &doc.result_sheets {
        entries.push(SheetManifestEntry {
            id: res.id.clone(),
            section: SheetSection::Analyses,
            title: res.title().to_owned(),
            csv_path: None,
            crc32: None,
        });
    }
    for graph in &doc.graph_sheets {
        entries.push(SheetManifestEntry {
            id: graph.id.clone(),
            section: SheetSection::Graphs,
            title: graph.spec.title.clone(),
            csv_path: None,
            crc32: None,
        });
    }
    for layout in &doc.layout_sheets {
        entries.push(SheetManifestEntry {
            id: layout.id.clone(),
            section: SheetSection::Layouts,
            title: layout.title.clone(),
            csv_path: None,
            crc32: None,
        });
    }
}

fn unique_sheet_csv_path(idx: usize, id: &SheetId, used: &mut HashSet<String>) -> String {
    let sanitized: String = id.as_str().chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
    let stem = if sanitized.is_empty() { format!("sheet_{idx}") } else { sanitized };
    let mut candidate = format!("sheets/{stem}.csv");
    let mut suffix = 1_usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("sheets/{stem}_{suffix}.csv");
        suffix += 1;
    }
    candidate
}

/// Strip raw cell values and cached result tables to produce a clean reusable `.lctemplate`.
pub fn strip_project_data_for_template(doc: &ProjectDocument) -> Result<ProjectDocument, FormatError> {
    validate_project_document(doc)?;
    let mut out = doc.clone();
    out.schema_version = CURRENT_SCHEMA_VERSION;
    for ds in &mut out.data_sheets {
        let was_frozen = ds.frozen;
        let notes = ds.floating_notes.clone();
        let linked = ds.linked_regions.clone();
        let opts = CloneDataSheetOptions {
            title: ds.title.clone(),
            keep_x_values: false,
            clear_y_values: true,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: None,
        };
        let mut clean = ds.clone_structure(ds.id.clone(), &opts)?;
        clean.frozen = was_frozen;
        clean.floating_notes = notes;
        clean.linked_regions = linked;
        *ds = clean;
    }
    for res in &mut out.result_sheets {
        res.status = ResultStatus::Dirty;
        res.analyzable_data = None;
        res.tabs.clear();
        res.scalars.clear();
        res.pairwise_comparisons.clear();
        res.fitted_curves.clear();
        res.checklist.clear();
    }
    Ok(out)
}

/// Migrate a deserialized JSON [`ProjectDocument`] from `from_version` to [`CURRENT_SCHEMA_VERSION`].
pub fn migrate_document_json(mut value: serde_json::Value, from_version: u32) -> Result<serde_json::Value, FormatError> {
    if from_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: from_version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    if from_version == CURRENT_SCHEMA_VERSION {
        return Ok(value);
    }
    // Schema v0 -> v1 migration: normalize missing optional v1 fields and bump schema_version.
    if from_version == 0
        && let Some(obj) = value.as_object_mut()
    {
        obj.insert(String::from("schema_version"), serde_json::Value::from(CURRENT_SCHEMA_VERSION));
        obj.entry(String::from("info_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        obj.entry(String::from("result_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        obj.entry(String::from("graph_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        obj.entry(String::from("layout_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        obj.entry(String::from("active_sheet_id")).or_insert(serde_json::Value::Null);
        obj.entry(String::from("family_anchor_sheet_id")).or_insert(serde_json::Value::Null);
    }
    Ok(value)
}

/// Enforce all `lumencraft-doc` structural bounds, finite-float invariants, and DAG cycle absence.
pub fn validate_project_document(doc: &ProjectDocument) -> Result<(), FormatError> {
    validate_title(&doc.title)?;
    let total = doc.total_sheet_count();
    if total > MAX_SHEETS_PER_PROJECT {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManySheets { count: total, max: MAX_SHEETS_PER_PROJECT }));
    }

    let mut seen_ids = HashSet::with_capacity(total);
    for ds in &doc.data_sheets {
        register_unique_sheet_id(&mut seen_ids, &ds.id)?;
        validate_data_sheet(ds)?;
    }
    for info in &doc.info_sheets {
        register_unique_sheet_id(&mut seen_ids, &info.id)?;
        validate_info_sheet(info)?;
    }
    for res in &doc.result_sheets {
        register_unique_sheet_id(&mut seen_ids, &res.id)?;
        validate_result_sheet(res)?;
    }
    for graph in &doc.graph_sheets {
        register_unique_sheet_id(&mut seen_ids, &graph.id)?;
        graph.spec.validate()?;
    }
    for layout in &doc.layout_sheets {
        register_unique_sheet_id(&mut seen_ids, &layout.id)?;
        validate_title(&layout.title)?;
    }

    detect_dependency_cycles(doc)
}

fn register_unique_sheet_id(seen: &mut HashSet<SheetId>, id: &SheetId) -> Result<(), FormatError> {
    if id.as_str().trim().is_empty() {
        return Err(FormatError::Doc(lumencraft_doc::DocError::EmptyIdentifier));
    }
    if !seen.insert(id.clone()) {
        return Err(FormatError::Doc(lumencraft_doc::DocError::DuplicateSheetId { sheet_id: id.clone() }));
    }
    Ok(())
}

fn validate_data_sheet(ds: &DataSheet) -> Result<(), FormatError> {
    validate_title(&ds.title)?;
    let ctx = ds.id.as_str();
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            validate_title(&t.x_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.y_subcolumn_mode.subcolumn_count())?;
            validate_cell_rows(&t.x_cells, MAX_SUBCOLUMNS, &format!("{ctx}.x_cells"))?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Column(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.subcolumn_mode.subcolumn_count())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Grouped(t) => {
            validate_title(&t.row_factor_title)?;
            validate_title(&t.column_factor_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.subcolumn_mode.subcolumn_count())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Contingency(t) => {
            validate_title(&t.row_factor_title)?;
            validate_title(&t.column_factor_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Survival(t) => {
            validate_title(&t.time_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.cohorts.len())?;
            for (r, c) in t.times.iter().enumerate() {
                validate_cell(c, &format!("{ctx}.times[{r}]"))?;
            }
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.cohorts, ctx)?;
        }
        TableArchetype::PartsOfWhole(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_titles(&t.slice_labels)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::MultipleVariables(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.variables.len())?;
            validate_titles(&t.row_titles)?;
            for (c, var) in t.variables.iter().enumerate() {
                validate_title(&var.title)?;
                validate_rows(var.cells.len())?;
                for (r, cell) in var.cells.iter().enumerate() {
                    validate_cell(cell, &format!("{ctx}.var[{c}][{r}]"))?;
                }
            }
        }
        TableArchetype::Nested(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.groups.len())?;
            validate_titles(&t.row_titles)?;
            for (g_idx, grp) in t.groups.iter().enumerate() {
                validate_title(&grp.title)?;
                validate_subcolumns(grp.biological_unit_count())?;
                validate_titles(&grp.subcolumn_titles)?;
                validate_cell_rows(&grp.cells, MAX_SUBCOLUMNS, &format!("{ctx}.group[{g_idx}]"))?;
            }
        }
    }
    Ok(())
}

fn validate_titles(titles: &[String]) -> Result<(), FormatError> {
    if titles.len() > MAX_ROWS {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManyRows { count: titles.len(), max: MAX_ROWS }));
    }
    for title in titles {
        validate_title(title)?;
    }
    Ok(())
}

fn validate_dataset_columns(cols: &[DataSetColumn], ctx: &str) -> Result<(), FormatError> {
    if cols.len() > MAX_DATA_SETS {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManyDataSets { count: cols.len(), max: MAX_DATA_SETS }));
    }
    for (c, col) in cols.iter().enumerate() {
        validate_title(&col.title)?;
        validate_cell_rows(&col.cells, MAX_SUBCOLUMNS, &format!("{ctx}.col[{c}]"))?;
    }
    Ok(())
}

fn validate_cell_rows(rows: &[Vec<Cell>], max_subs: usize, ctx: &str) -> Result<(), FormatError> {
    validate_rows(rows.len())?;
    for (r, row) in rows.iter().enumerate() {
        if row.len() > max_subs {
            return Err(FormatError::Doc(lumencraft_doc::DocError::TooManySubcolumns { count: row.len(), max: max_subs }));
        }
        for (s, cell) in row.iter().enumerate() {
            validate_cell(cell, &format!("{ctx}[{r}][{s}]"))?;
        }
    }
    Ok(())
}

fn validate_cell(cell: &Cell, loc: &str) -> Result<(), FormatError> {
    if let CellValue::Number(v) = cell.value
        && !v.is_finite()
    {
        return Err(FormatError::NonFiniteFloat { location: loc.to_owned() });
    }
    Ok(())
}

fn validate_info_sheet(info: &InfoSheet) -> Result<(), FormatError> {
    validate_title(&info.title)?;
    for (idx, c) in info.constants.iter().enumerate() {
        validate_title(&c.name)?;
        if let Some(u) = &c.units {
            validate_title(u)?;
        }
        if let ConstantValue::Number(v) = c.value
            && !v.is_finite()
        {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.constants[{idx}]", info.id) });
        }
    }
    Ok(())
}

fn validate_result_sheet(res: &ResultSheet) -> Result<(), FormatError> {
    validate_title(res.title())?;
    for (idx, s) in res.scalars.iter().enumerate() {
        if !s.value.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.scalars[{idx}]", res.id) });
        }
    }
    if let Some(data) = &res.analyzable_data {
        validate_data_sheet(data)?;
    }
    Ok(())
}

/// Detect circular `HookSource`, `ImportLinkSpec`, and transform/analysis DAG dependencies across
/// all sheets in `doc` (including self-referential loops).
pub fn detect_dependency_cycles(doc: &ProjectDocument) -> Result<(), FormatError> {
    let mut adj: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    let default_info = doc.info_sheets.first().map(|s| s.id.clone());

    for ds in &doc.data_sheets {
        let mut deps = Vec::new();
        if let Some(info_id) = &ds.info_sheet_id {
            deps.push(info_id.clone());
        }
        for region in &ds.linked_regions {
            if let Some(up) = region.spec.source.upstream_sheet_id() {
                deps.push(up.clone());
            }
        }
        adj.insert(ds.id.clone(), deps);
    }
    for info in &doc.info_sheets {
        adj.insert(info.id.clone(), Vec::new());
    }
    for res in &doc.result_sheets {
        let mut deps = vec![res.analysis.source_sheet_id.clone()];
        let paired_info = doc.data_sheet(&res.analysis.source_sheet_id).and_then(|d| d.info_sheet_id.clone()).or_else(|| default_info.clone());
        for hook in res.analysis.kind.hooked_sources() {
            if let Some(up) = resolve_hook_target(&hook, paired_info.as_ref()) {
                deps.push(up);
            }
        }
        adj.insert(res.id.clone(), deps);
    }
    for gs in &doc.graph_sheets {
        adj.insert(gs.id.clone(), collect_graph_deps(gs, default_info.as_ref()));
    }
    for ls in &doc.layout_sheets {
        adj.insert(ls.id.clone(), collect_layout_deps(ls, default_info.as_ref()));
    }

    run_cycle_dfs(&adj)
}

fn collect_graph_deps(gs: &GraphSheet, default_info: Option<&SheetId>) -> Vec<SheetId> {
    let mut deps = vec![gs.spec.primary_source_sheet_id.clone()];
    for series in &gs.spec.series {
        deps.push(series.source_sheet_id.clone());
        if let Some(ov) = &series.regression_overlay {
            deps.push(ov.result_sheet_id.clone());
        }
    }
    for cmp in &gs.spec.comparisons {
        deps.push(cmp.result_sheet_id.clone());
    }
    for ann in &gs.spec.annotations {
        if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } = ann {
            deps.push(result_sheet_id.clone());
        }
    }
    for hook in gs.spec.inline_hooks() {
        if let Some(up) = resolve_hook_target(&hook, default_info) {
            deps.push(up);
        }
    }
    deps
}

fn collect_layout_deps(ls: &LayoutSheet, default_info: Option<&SheetId>) -> Vec<SheetId> {
    let mut deps = Vec::new();
    for item in &ls.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => deps.push(graph_sheet_id.clone()),
            LayoutItem::LiveTable { source_sheet_id, .. } => deps.push(source_sheet_id.clone()),
            LayoutItem::TextCallout { .. } => {}
        }
    }
    for hook in ls.spec.inline_hooks() {
        if let Some(up) = resolve_hook_target(&hook, default_info) {
            deps.push(up);
        }
    }
    deps
}

fn resolve_hook_target(hook: &HookSource, fallback_info: Option<&SheetId>) -> Option<SheetId> {
    match hook {
        HookSource::InfoConstant { info_sheet_id: Some(id), .. } => Some(id.clone()),
        HookSource::InfoConstant { info_sheet_id: None, .. } => fallback_info.cloned(),
        HookSource::ResultScalar { result_sheet_id, .. } => Some(result_sheet_id.clone()),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => Some(data_sheet_id.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn run_cycle_dfs(adj: &BTreeMap<SheetId, Vec<SheetId>>) -> Result<(), FormatError> {
    let mut state: HashMap<SheetId, VisitState> = HashMap::with_capacity(adj.len());
    let mut stack: Vec<SheetId> = Vec::new();
    for node in adj.keys() {
        if !state.contains_key(node) {
            dfs_visit(node, adj, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

fn dfs_visit(
    node: &SheetId,
    adj: &BTreeMap<SheetId, Vec<SheetId>>,
    state: &mut HashMap<SheetId, VisitState>,
    stack: &mut Vec<SheetId>,
) -> Result<(), FormatError> {
    state.insert(node.clone(), VisitState::Visiting);
    stack.push(node.clone());
    if let Some(neighbors) = adj.get(node) {
        for next in neighbors {
            match state.get(next).copied() {
                Some(VisitState::Visiting) => {
                    let start = stack.iter().position(|id| id == next).unwrap_or(0);
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(next.clone());
                    return Err(FormatError::DependencyCycle { cycle });
                }
                Some(VisitState::Done) => {}
                None => {
                    if adj.contains_key(next) {
                        dfs_visit(next, adj, state, stack)?;
                    }
                }
            }
        }
    }
    let _ = stack.pop();
    state.insert(node.clone(), VisitState::Done);
    Ok(())
}
