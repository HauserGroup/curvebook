//! Top-level `ProjectDocument` container: five-section sheet catalog (`Data`, `Info`, `Analyses`,
//! `Graphs`, `Layouts`), stable `family_anchor_sheet_id` navigation state, and canonical upstream
//! dependency extraction for `lumencraft-engine`'s topological DAG.

use crate::SheetSection;
use crate::error::{CURRENT_SCHEMA_VERSION, DocError, MAX_SHEETS_PER_PROJECT, validate_title};
use crate::graph::{GraphAnnotation, GraphSheet};
use crate::ids::{HookSource, SheetId};
use crate::info::InfoSheet;
use crate::layout::{LayoutItem, LayoutSheet};
use crate::links::DataSheet;
use crate::result::ResultSheet;

/// Navigator sheet folder targeted by sheet-ordering operations such as [`ProjectDocument::reorder_sheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SheetFolderKind {
    Data,
    Info,
    Results,
    Graphs,
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

/// Complete serializable LumenCraft project model (`L1`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProjectDocument {
    pub schema_version: u32,
    pub title: String,
    pub data_sheets: Vec<DataSheet>,
    pub info_sheets: Vec<InfoSheet>,
    pub result_sheets: Vec<ResultSheet>,
    pub graph_sheets: Vec<GraphSheet>,
    pub layout_sheets: Vec<LayoutSheet>,
    pub active_sheet_id: Option<SheetId>,
    /// Anchored root sheet for the Family dependency pane (updated when navigating in the Project
    /// Navigator, kept stable when inspecting sibling sheets inside the Family pane; `docs/ui-parity.md`).
    pub family_anchor_sheet_id: Option<SheetId>,
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
        }
    }
}

impl ProjectDocument {
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

    fn ensure_can_insert(&self, id: &SheetId) -> Result<(), DocError> {
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        if self.sheet_section(id).is_some() {
            return Err(DocError::DuplicateSheetId { sheet_id: id.clone() });
        }
        let next = self.total_sheet_count() + 1;
        if next > MAX_SHEETS_PER_PROJECT {
            return Err(DocError::TooManySheets { count: next, max: MAX_SHEETS_PER_PROJECT });
        }
        Ok(())
    }

    pub fn add_data_sheet(&mut self, sheet: DataSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        if self.active_sheet_id.is_none() {
            self.active_sheet_id = Some(sheet.id.clone());
            self.family_anchor_sheet_id = Some(sheet.id.clone());
        }
        self.data_sheets.push(sheet);
        Ok(())
    }

    pub fn add_info_sheet(&mut self, sheet: InfoSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.info_sheets.push(sheet);
        Ok(())
    }

    pub fn add_result_sheet(&mut self, sheet: ResultSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.result_sheets.push(sheet);
        Ok(())
    }

    pub fn add_graph_sheet(&mut self, sheet: GraphSheet) -> Result<(), DocError> {
        self.ensure_can_insert(&sheet.id)?;
        self.graph_sheets.push(sheet);
        Ok(())
    }

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

    #[must_use]
    pub fn data_sheet(&self, id: &SheetId) -> Option<&DataSheet> {
        self.data_sheets.iter().find(|s| &s.id == id)
    }

    pub fn data_sheet_mut(&mut self, id: &SheetId) -> Option<&mut DataSheet> {
        self.data_sheets.iter_mut().find(|s| &s.id == id)
    }

    #[must_use]
    pub fn info_sheet(&self, id: &SheetId) -> Option<&InfoSheet> {
        self.info_sheets.iter().find(|s| &s.id == id)
    }

    pub fn info_sheet_mut(&mut self, id: &SheetId) -> Option<&mut InfoSheet> {
        self.info_sheets.iter_mut().find(|s| &s.id == id)
    }

    #[must_use]
    pub fn result_sheet(&self, id: &SheetId) -> Option<&ResultSheet> {
        self.result_sheets.iter().find(|s| &s.id == id)
    }

    pub fn result_sheet_mut(&mut self, id: &SheetId) -> Option<&mut ResultSheet> {
        self.result_sheets.iter_mut().find(|s| &s.id == id)
    }

    #[must_use]
    pub fn graph_sheet(&self, id: &SheetId) -> Option<&GraphSheet> {
        self.graph_sheets.iter().find(|s| &s.id == id)
    }

    pub fn graph_sheet_mut(&mut self, id: &SheetId) -> Option<&mut GraphSheet> {
        self.graph_sheets.iter_mut().find(|s| &s.id == id)
    }

    #[must_use]
    pub fn layout_sheet(&self, id: &SheetId) -> Option<&LayoutSheet> {
        self.layout_sheets.iter().find(|s| &s.id == id)
    }

    pub fn layout_sheet_mut(&mut self, id: &SheetId) -> Option<&mut LayoutSheet> {
        self.layout_sheets.iter_mut().find(|s| &s.id == id)
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
            let paired_info = self.data_sheet(&rs.analysis.source_sheet_id).and_then(|d| d.info_sheet_id.clone()).or_else(|| default_info.clone());
            for hook in rs.analysis.kind.hooked_sources() {
                push_hook_dep(&mut deps, &hook, paired_info.as_ref());
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
            for ann in &gs.spec.annotations {
                if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } = ann {
                    deps.push(result_sheet_id.clone());
                }
            }
            for hook in gs.spec.inline_hooks() {
                push_hook_dep(&mut deps, &hook, default_info.as_ref());
            }
        } else if let Some(ls) = self.layout_sheet(id) {
            for item in &ls.spec.items {
                match item {
                    LayoutItem::LiveGraph { graph_sheet_id, .. } => deps.push(graph_sheet_id.clone()),
                    LayoutItem::LiveTable { source_sheet_id, .. } => deps.push(source_sheet_id.clone()),
                    LayoutItem::TextCallout { .. } => {}
                }
            }
            for hook in ls.spec.inline_hooks() {
                push_hook_dep(&mut deps, &hook, default_info.as_ref());
            }
        }

        deps.retain(|d| d != id);
        deps.sort();
        deps.dedup();
        deps
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
