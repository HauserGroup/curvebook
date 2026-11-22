//! Top-level `ProjectDocument` container: five-section sheet catalog (`Data`, `Info`, `Analyses`,
//! `Graphs`, `Layouts`), stable `family_anchor_sheet_id` navigation state, and canonical upstream
//! dependency extraction for `curvebook-engine`'s topological DAG.

use std::collections::{BTreeMap, HashMap};

use crate::SheetSection;
use crate::analysis::{AnalysisKind, ParameterScope};
use crate::archetypes::{DataSetColumn, TableArchetype, XColumnMode};
use crate::cell::{Cell, CellValue};
use crate::error::{CURRENT_SCHEMA_VERSION, DocError, MAX_SHEETS_PER_PROJECT, validate_title};
use crate::graph::{GraphAnnotation, GraphSheet};
use crate::ids::{FloatingNote, HookSource, SheetId};
use crate::image::{MAX_PROJECT_IMAGE_ASSETS, ProjectImageAsset};
use crate::info::{ConstantValue, InfoSheet};
use crate::layout::{LayoutItem, LayoutSheet};
use crate::links::DataSheet;
use crate::result::{CiBound, ResultCell, ResultSheet};

/// Navigator sheet folder targeted by sheet-ordering operations such as [`ProjectDocument::reorder_sheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SheetFolderKind {
    /// [`ProjectDocument::data_sheets`] folder.
    Data,
    /// [`ProjectDocument::info_sheets`] folder.
    Info,
    /// [`ProjectDocument::result_sheets`] (`Analyses`) folder.
    Results,
    /// [`ProjectDocument::graph_sheets`] folder.
    Graphs,
    /// [`ProjectDocument::layout_sheets`] folder.
    Layouts,
}

impl SheetFolderKind {
    /// Map this folder kind to the corresponding [`SheetSection`] navigator section.
    #[must_use]
    pub const fn section(self) -> SheetSection {
        match self {
            Self::Data => SheetSection::Data,
            Self::Info => SheetSection::Info,
            Self::Results => SheetSection::Analyses,
            Self::Graphs => SheetSection::Graphs,
            Self::Layouts => SheetSection::Layouts,
        }
    }
}

/// Immutable named project milestone snapshot stored alongside the project (`QOL-11`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamedCheckpoint {
    /// Unique checkpoint identifier within `ProjectDocument::checkpoints`.
    pub id: String,
    /// Short user-facing milestone label.
    pub name: String,
    /// Optional free-form rationale or audit comment recorded when the checkpoint was captured.
    pub note: String,
    /// Creation timestamp formatted as an UTC epoch or ISO-8601 string (caller-supplied for determinism).
    pub created_utc: String,
    /// Canonical JSON snapshot of `ProjectDocument` with `checkpoints` empty to prevent recursive nesting.
    pub snapshot_json: String,
}

/// Complete serializable CurveBook project model (`L1`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectDocument {
    /// Persisted `.curvebook` document schema version (defaults to [`CURRENT_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Human-readable project title (`<= MAX_TITLE_BYTES` UTF-8 bytes).
    pub title: String,
    /// Ordered `Data` folder tables.
    pub data_sheets: Vec<DataSheet>,
    /// Ordered `Info` folder laboratory metadata sheets.
    pub info_sheets: Vec<InfoSheet>,
    /// Ordered `Analyses` folder [`ResultSheet`] nodes.
    pub result_sheets: Vec<ResultSheet>,
    /// Ordered `Graphs` folder scientific figures.
    pub graph_sheets: Vec<GraphSheet>,
    /// Ordered `Layouts` folder multi-panel publication canvases.
    pub layout_sheets: Vec<LayoutSheet>,
    /// Currently focused sheet in the workspace; `None` when the project contains zero sheets or nothing is selected.
    pub active_sheet_id: Option<SheetId>,
    /// Anchored root sheet for the Family dependency pane (updated when navigating in the Project
    /// Navigator, kept stable when inspecting sibling sheets inside the Family pane; `docs/evidence/user-interface.md`).
    pub family_anchor_sheet_id: Option<SheetId>,
    /// Named snapshots created via `project.createCheckpoint` (`QOL-11`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<NamedCheckpoint>,
    /// Content-addressed PNG/JPEG/TIFF raster images referenced by `EmbeddedImage` items (`R13.7`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_assets: Vec<ProjectImageAsset>,
}

impl Default for ProjectDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            title: String::from("Untitled Project"),
            data_sheets: Vec::new(),
            info_sheets: Vec::new(),
            result_sheets: Vec::new(),
            graph_sheets: Vec::new(),
            layout_sheets: Vec::new(),
            active_sheet_id: None,
            family_anchor_sheet_id: None,
            checkpoints: Vec::new(),
            image_assets: Vec::new(),
        }
    }
}

impl ProjectDocument {
    /// Construct an empty `ProjectDocument` at [`CURRENT_SCHEMA_VERSION`] with a validated `title`.
    pub fn new(title: impl Into<String>) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        Ok(Self { title, ..Self::default() })
    }

    /// Total sheet count across all five sections.
    #[must_use]
    pub fn total_sheet_count(&self) -> usize {
        self.data_sheets.len() + self.info_sheets.len() + self.result_sheets.len() + self.graph_sheets.len() + self.layout_sheets.len()
    }

    /// Determine which navigator section owns `id`, if present.
    #[must_use]
    pub fn sheet_section(&self, id: &SheetId) -> Option<SheetSection> {
        if self.data_sheets.iter().any(|s| &s.id == id) {
            Some(SheetSection::Data)
        } else if self.info_sheets.iter().any(|s| &s.id == id) {
            Some(SheetSection::Info)
        } else if self.result_sheets.iter().any(|s| &s.id == id) {
            Some(SheetSection::Analyses)
        } else if self.graph_sheets.iter().any(|s| &s.id == id) {
            Some(SheetSection::Graphs)
        } else if self.layout_sheets.iter().any(|s| &s.id == id) {
            Some(SheetSection::Layouts)
        } else {
            None
        }
    }

    /// Total allocated cell slots summed across all data sheets (and cached analyzable result tables),
    /// enforced against [`crate::MAX_PROJECT_CELLS`].
    pub fn total_allocated_cells(&self) -> Result<usize, DocError> {
        let mut total = 0usize;
        for ds in &self.data_sheets {
            total = crate::error::validate_project_cells(total, ds.archetype.allocated_cell_slots())?;
        }
        for rs in &self.result_sheets {
            if let Some(ds) = &rs.analyzable_data {
                total = crate::error::validate_project_cells(total, ds.archetype.allocated_cell_slots())?;
            }
        }
        Ok(total)
    }

    /// Validate structural project limits during deserialization while permitting legacy `.prism`
    /// sheet IDs awaiting format-level sanitization before full [`Self::validate`].
    pub fn validate_structural_limits(&self) -> Result<(), DocError> {
        validate_title(&self.title)?;
        let total_sheets = self.total_sheet_count();
        if total_sheets > MAX_SHEETS_PER_PROJECT {
            return Err(DocError::TooManySheets { count: total_sheets, max: MAX_SHEETS_PER_PROJECT });
        }
        self.total_allocated_cells()?;
        if self.image_assets.len() > crate::image::MAX_PROJECT_IMAGE_ASSETS {
            return Err(DocError::TooManyImageAssets { count: self.image_assets.len(), max: crate::image::MAX_PROJECT_IMAGE_ASSETS });
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in self
            .data_sheets
            .iter()
            .map(|s| &s.id)
            .chain(self.info_sheets.iter().map(|s| &s.id))
            .chain(self.result_sheets.iter().map(|s| &s.id))
            .chain(self.graph_sheets.iter().map(|s| &s.id))
            .chain(self.layout_sheets.iter().map(|s| &s.id))
        {
            id.validate_non_empty_bounded()?;
            if !seen.insert(id.clone()) {
                return Err(DocError::DuplicateSheetId { sheet_id: id.clone() });
            }
        }
        Ok(())
    }

    /// Perform full strict validation across sheet IDs, folder limits, project cell budget, image assets,
    /// and every child sheet specification.
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_structural_limits()?;
        if let Some(active) = &self.active_sheet_id {
            active.validate()?;
        }
        if let Some(anchor) = &self.family_anchor_sheet_id {
            anchor.validate()?;
        }
        for ds in &self.data_sheets {
            ds.id.validate()?;
            ds.validate_strict()?;
            for region in &ds.linked_regions {
                if let Some(src_id) = region.spec.source.upstream_sheet_id() {
                    src_id.validate()?;
                }
            }
        }
        for info in &self.info_sheets {
            info.id.validate()?;
            for ds_id in &info.linked_data_sheets {
                ds_id.validate()?;
            }
            info.validate()?;
        }
        for rs in &self.result_sheets {
            rs.id.validate()?;
            rs.analysis.source_sheet_id.validate()?;
            for up in rs.analysis.kind.upstream_sheet_ids() {
                up.validate()?;
            }
            rs.validate()?;
        }
        for gs in &self.graph_sheets {
            gs.id.validate()?;
            gs.spec.primary_source_sheet_id.validate()?;
            gs.validate()?;
        }
        for ls in &self.layout_sheets {
            ls.id.validate()?;
            ls.validate()?;
        }
        for asset in &self.image_assets {
            asset.validate()?;
        }
        Ok(())
    }

    fn ensure_can_insert(&self, id: &SheetId) -> Result<(), DocError> {
        id.validate_hook_safe_bounded()?;
        if self.sheet_section(id).is_some() {
            return Err(DocError::DuplicateSheetId { sheet_id: id.clone() });
        }
        let next = self.total_sheet_count() + 1;
        if next > MAX_SHEETS_PER_PROJECT {
            return Err(DocError::TooManySheets { count: next, max: MAX_SHEETS_PER_PROJECT });
        }
        Ok(())
    }

    /// Append a [`DataSheet`] to the `Data` folder, selecting it as active/family-anchor if none is set yet.
    pub fn add_data_sheet(&mut self, sheet: DataSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        let existing_cells = self.total_allocated_cells()?;
        crate::error::validate_project_cells(existing_cells, sheet.archetype.allocated_cell_slots())?;
        if self.active_sheet_id.is_none() {
            self.active_sheet_id = Some(sheet.id.clone());
            self.family_anchor_sheet_id = Some(sheet.id.clone());
        }
        self.data_sheets.push(sheet);
        Ok(())
    }

    /// Append an [`InfoSheet`] to the `Info` folder after checking ID uniqueness and project sheet limits.
    pub fn add_info_sheet(&mut self, sheet: InfoSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.info_sheets.push(sheet);
        Ok(())
    }

    /// Append a [`ResultSheet`] to the `Analyses` folder after checking ID uniqueness, project sheet limits,
    /// and cumulative cell budget when `analyzable_data` is attached.
    pub fn add_result_sheet(&mut self, sheet: ResultSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        if let Some(ds) = &sheet.analyzable_data {
            let existing_cells = self.total_allocated_cells()?;
            crate::error::validate_project_cells(existing_cells, ds.archetype.allocated_cell_slots())?;
        }
        self.result_sheets.push(sheet);
        Ok(())
    }

    /// Append a [`GraphSheet`] to the `Graphs` folder after checking ID uniqueness and project sheet limits.
    pub fn add_graph_sheet(&mut self, sheet: GraphSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.graph_sheets.push(sheet);
        Ok(())
    }

    /// Append a [`LayoutSheet`] to the `Layouts` folder after checking ID uniqueness and project sheet limits.
    pub fn add_layout_sheet(&mut self, sheet: LayoutSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.layout_sheets.push(sheet);
        Ok(())
    }

    /// Move a sheet inside `folder` from `from_index` to `to_index`, preserving relative order of
    /// all other sheets in that folder.
    ///
    /// # Errors
    /// Returns [`DocError::SheetIndexOutOfBounds`] when either `from_index` or `to_index` is outside
    /// `0..len`.
    pub fn reorder_sheet(&mut self, folder: SheetFolderKind, from_index: usize, to_index: usize) -> Result<(), DocError> {
        match folder {
            SheetFolderKind::Data => reorder_vec(&mut self.data_sheets, folder, from_index, to_index),
            SheetFolderKind::Info => reorder_vec(&mut self.info_sheets, folder, from_index, to_index),
            SheetFolderKind::Results => reorder_vec(&mut self.result_sheets, folder, from_index, to_index),
            SheetFolderKind::Graphs => reorder_vec(&mut self.graph_sheets, folder, from_index, to_index),
            SheetFolderKind::Layouts => reorder_vec(&mut self.layout_sheets, folder, from_index, to_index),
        }
    }

    /// Select a sheet from the main Project Navigator, re-anchoring `family_anchor_sheet_id`.
    pub fn select_in_navigator(&mut self, id: &SheetId) -> Result<(), DocError> {
        if self.sheet_section(id).is_none() {
            return Err(DocError::SheetNotFound { sheet_id: id.clone() });
        }
        self.active_sheet_id = Some(id.clone());
        self.family_anchor_sheet_id = Some(id.clone());
        Ok(())
    }

    /// Select a sheet inside the Family pane without mutating `family_anchor_sheet_id`.
    pub fn select_in_family_pane(&mut self, id: &SheetId) -> Result<(), DocError> {
        if self.sheet_section(id).is_none() {
            return Err(DocError::SheetNotFound { sheet_id: id.clone() });
        }
        self.active_sheet_id = Some(id.clone());
        Ok(())
    }

    /// Look up an immutable [`DataSheet`] by [`SheetId`].
    #[must_use]
    pub fn data_sheet(&self, id: &SheetId) -> Option<&DataSheet> {
        self.data_sheets.iter().find(|s| &s.id == id)
    }

    /// Look up a mutable [`DataSheet`] by [`SheetId`].
    pub fn data_sheet_mut(&mut self, id: &SheetId) -> Option<&mut DataSheet> {
        self.data_sheets.iter_mut().find(|s| &s.id == id)
    }

    /// Look up an immutable [`InfoSheet`] by [`SheetId`].
    #[must_use]
    pub fn info_sheet(&self, id: &SheetId) -> Option<&InfoSheet> {
        self.info_sheets.iter().find(|s| &s.id == id)
    }

    /// Look up a mutable [`InfoSheet`] by [`SheetId`].
    pub fn info_sheet_mut(&mut self, id: &SheetId) -> Option<&mut InfoSheet> {
        self.info_sheets.iter_mut().find(|s| &s.id == id)
    }

    /// Look up an immutable [`ResultSheet`] by [`SheetId`].
    #[must_use]
    pub fn result_sheet(&self, id: &SheetId) -> Option<&ResultSheet> {
        self.result_sheets.iter().find(|s| &s.id == id)
    }

    /// Look up a mutable [`ResultSheet`] by [`SheetId`].
    pub fn result_sheet_mut(&mut self, id: &SheetId) -> Option<&mut ResultSheet> {
        self.result_sheets.iter_mut().find(|s| &s.id == id)
    }

    /// Look up an immutable [`GraphSheet`] by [`SheetId`].
    #[must_use]
    pub fn graph_sheet(&self, id: &SheetId) -> Option<&GraphSheet> {
        self.graph_sheets.iter().find(|s| &s.id == id)
    }

    /// Look up a mutable [`GraphSheet`] by [`SheetId`].
    pub fn graph_sheet_mut(&mut self, id: &SheetId) -> Option<&mut GraphSheet> {
        self.graph_sheets.iter_mut().find(|s| &s.id == id)
    }

    /// Look up an immutable [`LayoutSheet`] by [`SheetId`].
    #[must_use]
    pub fn layout_sheet(&self, id: &SheetId) -> Option<&LayoutSheet> {
        self.layout_sheets.iter().find(|s| &s.id == id)
    }

    /// Look up a mutable [`LayoutSheet`] by [`SheetId`].
    pub fn layout_sheet_mut(&mut self, id: &SheetId) -> Option<&mut LayoutSheet> {
        self.layout_sheets.iter_mut().find(|s| &s.id == id)
    }

    /// Look up a stored [`ProjectImageAsset`] by its content-addressed `asset_id`.
    #[must_use]
    pub fn image_asset(&self, asset_id: &str) -> Option<&ProjectImageAsset> {
        self.image_assets.iter().find(|a| a.asset_id == asset_id)
    }

    /// Insert `asset` if its content-addressed `asset_id` is not already registered, enforcing
    /// [`MAX_PROJECT_IMAGE_ASSETS`]. Returns the canonical `asset_id` on success.
    pub fn upsert_image_asset(&mut self, asset: ProjectImageAsset) -> Result<String, DocError> {
        if let Some(existing) = self.image_asset(&asset.asset_id) {
            if existing.media_type != asset.media_type || existing.pixel_size != asset.pixel_size || existing.bytes != asset.bytes {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "image asset id collision with mismatched payload",
                });
            }
            return Ok(existing.asset_id.clone());
        }
        let next = self.image_assets.len().saturating_add(1);
        if next > MAX_PROJECT_IMAGE_ASSETS {
            return Err(DocError::TooManyImageAssets { count: next, max: MAX_PROJECT_IMAGE_ASSETS });
        }
        let id = asset.asset_id.clone();
        self.image_assets.push(asset);
        Ok(id)
    }

    /// Resolve the paired [`InfoSheet`] ID for a `DataSheet` or (chained) `ResultSheet` source,
    /// without falling back to the project's first info sheet.
    #[must_use]
    pub fn paired_info_sheet_id(&self, source_id: &SheetId) -> Option<&SheetId> {
        if let Some(ds) = self.data_sheet(source_id) {
            return ds.info_sheet_id.as_ref();
        }
        let mut cursor = source_id;
        let mut seen = Vec::new();
        while !seen.contains(&cursor) {
            seen.push(cursor);
            if let Some(ds) = self.data_sheet(cursor) {
                return ds.info_sheet_id.as_ref();
            }
            if let Some(res) = self.result_sheet(cursor) {
                if let Some(info_id) = res.analyzable_data.as_ref().and_then(|d| d.info_sheet_id.as_ref()) {
                    return Some(info_id);
                }
                cursor = &res.analysis.source_sheet_id;
            } else {
                break;
            }
        }
        None
    }

    /// Return sorted, deduplicated direct upstream `SheetId` dependencies of `id` across data links,
    /// analyses, constant hooks, graph series/overlays/brackets, and layout placements.
    #[must_use]
    pub fn upstream_dependencies(&self, id: &SheetId) -> Vec<SheetId> {
        let mut deps = Vec::new();
        let default_info = self.info_sheets.first().map(|s| s.id.clone());

        if let Some(ds) = self.data_sheet(id) {
            if let Some(info_id) = &ds.info_sheet_id {
                deps.push(info_id.clone());
            }
            for region in &ds.linked_regions {
                if let Some(up) = region.spec.source.upstream_sheet_id() {
                    deps.push(up.clone());
                }
            }
        } else if let Some(rs) = self.result_sheet(id) {
            deps.push(rs.analysis.source_sheet_id.clone());
            let paired_info = self.paired_info_sheet_id(&rs.analysis.source_sheet_id).cloned().or_else(|| default_info.clone());
            for hook in rs.analysis.kind.hooked_sources() {
                push_hook_dep(&mut deps, &hook, paired_info.as_ref());
            }
            if let crate::AnalysisKind::PowerAnalysis(p) = &rs.analysis.kind
                && let Some(info_id) = &p.info_sheet_id
            {
                deps.push(info_id.clone());
            }
        } else if let Some(gs) = self.graph_sheet(id) {
            deps.push(gs.spec.primary_source_sheet_id.clone());
            for series in &gs.spec.series {
                deps.push(series.source_sheet_id.clone());
                if let Some(ov) = &series.regression_overlay {
                    deps.push(ov.result_sheet_id.clone());
                }
            }
            for cmp in &gs.spec.comparisons {
                deps.push(cmp.result_sheet_id.clone());
            }
            if let Some(cld) = &gs.spec.compact_letter_display {
                deps.push(cld.result_sheet_id.clone());
            }
            for ann in &gs.spec.annotations {
                match ann {
                    GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                        deps.push(result_sheet_id.clone())
                    }
                    GraphAnnotation::TextBox { .. }
                    | GraphAnnotation::LineOrArrow { .. }
                    | GraphAnnotation::Shape { .. }
                    | GraphAnnotation::EmbeddedImage { .. } => {}
                }
            }
            let paired_info = self.paired_info_sheet_id(&gs.spec.primary_source_sheet_id).cloned().or_else(|| default_info.clone());
            for hook in gs.spec.inline_hooks() {
                push_hook_dep(&mut deps, &hook, paired_info.as_ref());
            }
        } else if let Some(ls) = self.layout_sheet(id) {
            for item in &ls.spec.items {
                match item {
                    LayoutItem::LiveGraph { graph_sheet_id, .. } => deps.push(graph_sheet_id.clone()),
                    LayoutItem::LiveTable { source_sheet_id, .. } => deps.push(source_sheet_id.clone()),
                    LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {}
                }
            }
            for hook in ls.spec.inline_hooks() {
                push_hook_dep(&mut deps, &hook, default_info.as_ref());
            }
        }

        deps.sort();
        deps.dedup();
        deps
    }

    /// Detect circular `HookSource`, `ImportLinkSpec`, and analysis/graph/layout DAG dependencies
    /// across all sheets in this project (including self-referential loops).
    ///
    /// Returns `Some(cycle)` containing the closed cycle path (`[start, ..., start]`) on the first
    /// cycle encountered in deterministic `SheetId` order, or `None` when the project graph is acyclic.
    #[must_use]
    pub fn find_dependency_cycle(&self) -> Option<Vec<SheetId>> {
        let mut adj: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
        for id in self
            .data_sheets
            .iter()
            .map(|s| &s.id)
            .chain(self.info_sheets.iter().map(|s| &s.id))
            .chain(self.result_sheets.iter().map(|s| &s.id))
            .chain(self.graph_sheets.iter().map(|s| &s.id))
            .chain(self.layout_sheets.iter().map(|s| &s.id))
        {
            adj.insert(id.clone(), self.upstream_dependencies(id));
        }

        let mut state: HashMap<SheetId, CycleVisitState> = HashMap::with_capacity(adj.len());
        let mut stack: Vec<SheetId> = Vec::new();
        for node in adj.keys() {
            if !state.contains_key(node)
                && let Some(cycle) = dfs_find_cycle(node, &adj, &mut state, &mut stack)
            {
                return Some(cycle);
            }
        }
        None
    }

    /// Inspect a single [`DataSheet`] for any non-finite (`NaN` or `±Inf`) numeric cell, sequence
    /// parameter, numeric row spacing, or floating-note coordinate, returning its field path.
    #[must_use]
    pub fn first_non_finite_data_sheet_float(ds: &DataSheet) -> Option<String> {
        let ctx = ds.id.as_str();
        first_non_finite_archetype_float(&ds.archetype, ctx).or_else(|| first_non_finite_notes_float(&ds.floating_notes, ctx))
    }

    /// Inspect every sheet in this project for any non-finite (`NaN` or `±Inf`) floating-point
    /// value, returning the first offending field path if present.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<String> {
        for ds in &self.data_sheets {
            if let Some(loc) = Self::first_non_finite_data_sheet_float(ds) {
                return Some(loc);
            }
        }
        for info in &self.info_sheets {
            if let Some(loc) = first_non_finite_info_float(info) {
                return Some(loc);
            }
        }
        for res in &self.result_sheets {
            if let Some(loc) = first_non_finite_result_float(res) {
                return Some(loc);
            }
        }
        for graph in &self.graph_sheets {
            if let Some(field) = graph.first_non_finite_float() {
                return Some(format!("{}.{field}", graph.id));
            }
        }
        for layout in &self.layout_sheets {
            if let Some(field) = layout.first_non_finite_float() {
                return Some(format!("{}.{field}", layout.id));
            }
        }
        None
    }
}

impl<'de> serde::Deserialize<'de> for ProjectDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawProjectDocument {
            schema_version: u32,
            title: String,
            data_sheets: Vec<DataSheet>,
            info_sheets: Vec<InfoSheet>,
            result_sheets: Vec<ResultSheet>,
            graph_sheets: Vec<GraphSheet>,
            layout_sheets: Vec<LayoutSheet>,
            active_sheet_id: Option<SheetId>,
            family_anchor_sheet_id: Option<SheetId>,
            #[serde(default)]
            checkpoints: Vec<NamedCheckpoint>,
            #[serde(default)]
            image_assets: Vec<ProjectImageAsset>,
        }

        let raw = RawProjectDocument::deserialize(deserializer)?;
        let doc = Self {
            schema_version: raw.schema_version,
            title: raw.title,
            data_sheets: raw.data_sheets,
            info_sheets: raw.info_sheets,
            result_sheets: raw.result_sheets,
            graph_sheets: raw.graph_sheets,
            layout_sheets: raw.layout_sheets,
            active_sheet_id: raw.active_sheet_id,
            family_anchor_sheet_id: raw.family_anchor_sheet_id,
            checkpoints: raw.checkpoints,
            image_assets: raw.image_assets,
        };
        doc.validate_structural_limits().map_err(serde::de::Error::custom)?;
        Ok(doc)
    }
}

fn reorder_vec<T>(items: &mut [T], folder: SheetFolderKind, from_index: usize, to_index: usize) -> Result<(), DocError> {
    let len = items.len();
    if from_index >= len {
        return Err(DocError::SheetIndexOutOfBounds { folder, index: from_index, len });
    }
    if to_index >= len {
        return Err(DocError::SheetIndexOutOfBounds { folder, index: to_index, len });
    }
    if from_index < to_index {
        if let Some(window) = items.get_mut(from_index..=to_index) {
            window.rotate_left(1);
        }
    } else if from_index > to_index
        && let Some(window) = items.get_mut(to_index..=from_index)
    {
        window.rotate_right(1);
    }
    Ok(())
}

fn push_hook_dep(deps: &mut Vec<SheetId>, hook: &HookSource, fallback_info: Option<&SheetId>) {
    match hook {
        HookSource::InfoConstant { info_sheet_id: Some(id), .. } => deps.push(id.clone()),
        HookSource::InfoConstant { info_sheet_id: None, .. } => {
            if let Some(id) = fallback_info {
                deps.push(id.clone());
            }
        }
        HookSource::ResultScalar { result_sheet_id, .. } => deps.push(result_sheet_id.clone()),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => deps.push(data_sheet_id.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleVisitState {
    Visiting,
    Done,
}

fn dfs_find_cycle(
    node: &SheetId,
    adj: &BTreeMap<SheetId, Vec<SheetId>>,
    state: &mut HashMap<SheetId, CycleVisitState>,
    stack: &mut Vec<SheetId>,
) -> Option<Vec<SheetId>> {
    state.insert(node.clone(), CycleVisitState::Visiting);
    stack.push(node.clone());
    if let Some(neighbors) = adj.get(node) {
        for next in neighbors {
            match state.get(next).copied() {
                Some(CycleVisitState::Visiting) => {
                    let start = stack.iter().position(|id| id == next).unwrap_or(0);
                    let mut cycle = stack.get(start..).unwrap_or(&[]).to_vec();
                    cycle.push(next.clone());
                    return Some(cycle);
                }
                Some(CycleVisitState::Done) => {}
                None => {
                    if adj.contains_key(next)
                        && let Some(cycle) = dfs_find_cycle(next, adj, state, stack)
                    {
                        return Some(cycle);
                    }
                }
            }
        }
    }
    let _ = stack.pop();
    state.insert(node.clone(), CycleVisitState::Done);
    None
}

fn first_non_finite_notes_float(notes: &[FloatingNote], sheet_id: &str) -> Option<String> {
    for (idx, note) in notes.iter().enumerate() {
        let b = note.bounds_pt;
        if !b.x.is_finite() || !b.y.is_finite() || !b.width.is_finite() || !b.height.is_finite() {
            return Some(format!("{sheet_id}.floating_notes[{idx}].bounds_pt"));
        }
    }
    None
}

fn first_non_finite_cell_rows(rows: &[Vec<Cell>], ctx: &str) -> Option<String> {
    for (r, row) in rows.iter().enumerate() {
        for (s, cell) in row.iter().enumerate() {
            if let CellValue::Number(v) = cell.value
                && !v.is_finite()
            {
                return Some(format!("{ctx}[{r}][{s}]"));
            }
        }
    }
    None
}

fn first_non_finite_dataset_columns(cols: &[DataSetColumn], ctx: &str) -> Option<String> {
    for (c, col) in cols.iter().enumerate() {
        if let Some(loc) = first_non_finite_cell_rows(&col.cells, &format!("{ctx}.col[{c}]")) {
            return Some(loc);
        }
    }
    None
}

fn first_non_finite_archetype_float(arch: &TableArchetype, ctx: &str) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => {
            if let XColumnMode::GeneratedSequence { start, step, .. } = t.x_mode
                && (!start.is_finite() || !step.is_finite())
            {
                return Some(format!("{ctx}.x_mode"));
            }
            first_non_finite_cell_rows(&t.x_cells, &format!("{ctx}.x_cells")).or_else(|| first_non_finite_dataset_columns(&t.data_sets, ctx))
        }
        TableArchetype::Column(t) => first_non_finite_dataset_columns(&t.data_sets, ctx),
        TableArchetype::Grouped(t) => {
            if let Some(loc) = first_non_finite_dataset_columns(&t.data_sets, ctx) {
                return Some(loc);
            }
            if let Some(spacing) = &t.row_numeric_spacing {
                for (row, maybe_v) in spacing.iter().enumerate() {
                    if maybe_v.is_some_and(|v| !v.is_finite()) {
                        return Some(format!("{ctx}.row_numeric_spacing[{row}]"));
                    }
                }
            }
            None
        }
        TableArchetype::Contingency(t) => first_non_finite_dataset_columns(&t.data_sets, ctx),
        TableArchetype::Survival(t) => {
            for (r, cell) in t.times.iter().enumerate() {
                if let CellValue::Number(v) = cell.value
                    && !v.is_finite()
                {
                    return Some(format!("{ctx}.times[{r}]"));
                }
            }
            first_non_finite_dataset_columns(&t.cohorts, ctx)
        }
        TableArchetype::PartsOfWhole(t) => first_non_finite_dataset_columns(&t.data_sets, ctx),
        TableArchetype::MultipleVariables(t) => {
            for (c, var) in t.variables.iter().enumerate() {
                for (r, cell) in var.cells.iter().enumerate() {
                    if let CellValue::Number(v) = cell.value
                        && !v.is_finite()
                    {
                        return Some(format!("{ctx}.var[{c}][{r}]"));
                    }
                }
            }
            None
        }
        TableArchetype::Nested(t) => {
            for (g_idx, grp) in t.groups.iter().enumerate() {
                if let Some(loc) = first_non_finite_cell_rows(&grp.cells, &format!("{ctx}.group[{g_idx}]")) {
                    return Some(loc);
                }
            }
            None
        }
    }
}

fn first_non_finite_info_float(info: &InfoSheet) -> Option<String> {
    for (idx, c) in info.constants.iter().enumerate() {
        if let ConstantValue::Number(v) = c.value
            && !v.is_finite()
        {
            return Some(format!("{}.constants[{idx}]", info.id));
        }
    }
    first_non_finite_notes_float(&info.floating_notes, info.id.as_str())
}

fn result_cell_is_finite(cell: &ResultCell) -> bool {
    match cell {
        ResultCell::Number { value, .. } => value.is_finite(),
        ResultCell::PValue { raw, .. } => raw.is_finite(),
        ResultCell::ConfidenceInterval { lower, upper } => {
            matches!(*lower, CiBound::Open).then_some(true).unwrap_or_else(|| matches!(*lower, CiBound::Finite(v) if v.is_finite()))
                && matches!(*upper, CiBound::Open).then_some(true).unwrap_or_else(|| matches!(*upper, CiBound::Finite(v) if v.is_finite()))
        }
        ResultCell::Empty | ResultCell::Text(_) | ResultCell::Boolean(_) => true,
    }
}

fn first_non_finite_result_tabs_and_curves(res: &ResultSheet) -> Option<String> {
    for (tab_idx, tab) in res.tabs.iter().enumerate() {
        for (r, row) in tab.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if !result_cell_is_finite(cell) {
                    return Some(format!("{}.tabs[{tab_idx}].rows[{r}][{c}]", res.id));
                }
            }
        }
    }
    for (idx, cmp) in res.pairwise_comparisons.iter().enumerate() {
        if cmp.estimate_diff.is_some_and(|v| !v.is_finite()) || !cmp.raw_p_or_q.is_finite() {
            return Some(format!("{}.pairwise_comparisons[{idx}]", res.id));
        }
    }
    for (c_idx, curve) in res.fitted_curves.iter().enumerate() {
        for (p_idx, pt) in curve.points.iter().enumerate() {
            if [Some(pt.x), Some(pt.y), pt.ci_lower, pt.ci_upper, pt.pred_lower, pt.pred_upper].into_iter().flatten().any(|v| !v.is_finite()) {
                return Some(format!("{}.fitted_curves[{c_idx}].points[{p_idx}]", res.id));
            }
        }
    }
    None
}

fn first_non_finite_nonlinear_bounds(kind: &AnalysisKind, sheet_id: &SheetId) -> Option<String> {
    let AnalysisKind::NonlinearRegression(fit) = kind else {
        return None;
    };
    for (idx, rule) in fit.parameter_rules.iter().enumerate() {
        if let ParameterScope::PerDatasetConstant(pairs) = &rule.scope {
            for (ds_idx, val) in pairs {
                if !val.is_finite() {
                    return Some(format!("{sheet_id}.parameter_rules[{idx}].per_dataset[{ds_idx}]"));
                }
            }
        }
    }
    let bounds = [fit.plot_x_min, fit.plot_x_max]
        .into_iter()
        .chain(fit.x_range_filter.into_iter().flat_map(|(a, b)| [a, b]))
        .chain(fit.y_range_filter.into_iter().flat_map(|(a, b)| [a, b]))
        .flatten();
    bounds.into_iter().any(|v| !v.is_finite()).then(|| format!("{sheet_id}.nonlinear_fit_bounds"))
}

fn first_non_finite_result_float(res: &ResultSheet) -> Option<String> {
    for (idx, s) in res.scalars.iter().enumerate() {
        if !s.value.is_finite() {
            return Some(format!("{}.scalars[{idx}]", res.id));
        }
    }
    if let Some(loc) = first_non_finite_result_tabs_and_curves(res) {
        return Some(loc);
    }
    if let Some(data) = &res.analyzable_data
        && let Some(loc) = first_non_finite_archetype_float(&data.archetype, res.id.as_str())
            .or_else(|| first_non_finite_notes_float(&data.floating_notes, res.id.as_str()))
    {
        return Some(loc);
    }
    if let Some(field) = res.analysis.first_non_finite_float() {
        return Some(format!("{}.{field}", res.id.as_str()));
    }
    first_non_finite_nonlinear_bounds(&res.analysis.kind, &res.id).or_else(|| first_non_finite_notes_float(&res.floating_notes, res.id.as_str()))
}
