//! Native `egui` `26 pt` Status & Telemetry Footer (`docs/planning/gui-layout-spec.md` §7, `R15.3`).
//!
//! Renders navigation history (`< >`), clickable section & active-sheet switcher controls,
//! [`LinkedDataRegion`] upstream provenance indicators, `< 0.5 ms` cached selection Quick-Stats
//! (`N`, `Mean`, `SD`, `SEM`, `Median`, `Range`) or live canvas cursor `(X, Y)` coordinates,
//! CVD/Grayscale preview filter selector, `ChartScene`/`LayoutScene`-only Rulers toggle, and
//! strictly `Graph`/`Layout`/`Gallery`-only canvas Zoom controls.

use std::cmp::Ordering;

use curvebook_doc::{
    Cell, CellCoord, CellValue, DataSheet, LinkBorderColor, LinkedDataRegion, ProjectDocument, ResultStatus, SheetId, SheetSection, TableArchetype,
};
use curvebook_palette::CanvasPreviewFilter;
use curvebook_tools::GridCellCoord;
use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::canvas_view::CanvasHoverReadout;
use crate::egui_shell::{
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, doc_sheet_title,
};
use crate::grid_view::{resolve_flat_coord, total_flat_data_subcolumns};
use crate::i18n::{is_rtl_locale, tr_or_fallback};
use crate::inspect::ZLayer;

/// Hard selection cell cap (`50,000`) preventing pathological UI stalls on 1M-row sheets (`§7.2`).
pub const QUICK_STATS_CELL_CAP: usize = 50_000;

const FOOTER_CTRL_H: f32 = 24.0;
const POPUP_ROW_H: f32 = 24.0;

/// Cached summary statistics over the active sheet selection (`N`, `Mean`, `SD`, `SEM`, `Median`, `Range`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickStatsSummary {
    pub n: usize,
    pub excluded_count: usize,
    pub selected_cells: usize,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
    pub sem: Option<f64>,
    pub median: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub range: Option<f64>,
    pub sum: Option<f64>,
    pub truncated_at_cap: bool,
}

impl QuickStatsSummary {
    /// Format compact full-telemetry status line (`N`, `Mean`, `SD`, `SEM`, `Median`, `Range`).
    #[must_use]
    pub fn format_telemetry(&self, locale: &str) -> String {
        if self.n == 0 {
            return if self.excluded_count > 0 {
                format!("N = 0 (Excl {})", self.excluded_count)
            } else if self.selected_cells > 0 {
                String::from("N = 0")
            } else {
                String::from(tr_or_fallback(locale, "w6.view.status.ready", "Ready"))
            };
        }
        let mut parts = Vec::with_capacity(7);
        parts.push(format!("N={}", self.n));
        if let Some(mean) = self.mean {
            parts.push(format!("Mean={}", format_compact_stat(mean)));
        }
        if let Some(sd) = self.sd {
            parts.push(format!("SD={}", format_compact_stat(sd)));
        }
        if let Some(sem) = self.sem {
            parts.push(format!("SEM={}", format_compact_stat(sem)));
        }
        if let Some(med) = self.median {
            parts.push(format!("Med={}", format_compact_stat(med)));
        }
        if let Some(range) = self.range {
            parts.push(format!("Rng={}", format_compact_stat(range)));
        }
        if self.excluded_count > 0 {
            parts.push(format!("Excl={}", self.excluded_count));
        }
        if self.truncated_at_cap {
            parts.push(String::from("[50k cap]"));
        }
        parts.join("  ")
    }
}

#[derive(Debug, Clone, Default)]
struct ScratchFloats(Vec<f64>);

impl PartialEq for ScratchFloats {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Memoization cache avoiding per-frame scans over active grid selections (`O(1)` steady frame).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickStatsCache {
    sheet_id: Option<SheetId>,
    mutation_seq: u64,
    selection_bounds_key: Option<(usize, usize, usize, usize, usize, usize)>,
    discontig_len: usize,
    discontig_hash: u64,
    pub summary: QuickStatsSummary,
    /// Number of cold recomputations performed since initialization.
    pub recompute_count: u64,
    scratch_values: ScratchFloats,
}

#[inline]
fn hash_discontiguous_coords(coords: &[GridCellCoord]) -> u64 {
    let len = coords.len();
    if len == 0 {
        return 0;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |acc: &mut u64, c: GridCellCoord| {
        for v in [c.row as u64, c.col as u64, c.subcol as u64] {
            *acc ^= v;
            *acc = acc.wrapping_mul(0x1000_0000_01b3);
        }
    };
    if len <= 16 {
        for &c in coords {
            mix(&mut h, c);
        }
    } else {
        let step = (len / 16).max(1);
        let mut idx = 0usize;
        while idx < len {
            mix(&mut h, coords[idx]);
            idx = idx.saturating_add(step);
        }
        if let Some(&last) = coords.last() {
            mix(&mut h, last);
        }
    }
    h
}

#[inline]
fn flat_index_for_coord_fast(arch: &TableArchetype, col: usize, subcol: usize) -> usize {
    match arch {
        TableArchetype::Xy(t) => {
            let subs = t.y_subcolumn_mode.subcolumn_count().max(1);
            col.saturating_mul(subs).saturating_add(subcol.min(subs - 1))
        }
        TableArchetype::Column(t) => {
            let subs = t.subcolumn_mode.subcolumn_count().max(1);
            col.saturating_mul(subs).saturating_add(subcol.min(subs - 1))
        }
        TableArchetype::Grouped(t) => {
            let subs = t.subcolumn_mode.subcolumn_count().max(1);
            col.saturating_mul(subs).saturating_add(subcol.min(subs - 1))
        }
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => col,
        TableArchetype::Nested(t) => {
            let mut offset = 0usize;
            for (idx, grp) in t.groups.iter().enumerate() {
                let units = grp.biological_unit_count().max(1);
                if idx == col {
                    return offset.saturating_add(subcol.min(units - 1));
                }
                offset = offset.saturating_add(units);
            }
            offset
        }
    }
}

#[inline]
fn finite_f64_cmp(a: &f64, b: &f64) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

struct StatsAccum<'a> {
    values: &'a mut Vec<f64>,
    sum: f64,
    anchor: f64,
    has_anchor: bool,
    sum_d: f64,
    sum_dd: f64,
    min: f64,
    max: f64,
    excluded_count: usize,
}

impl<'a> StatsAccum<'a> {
    #[inline]
    fn new(values: &'a mut Vec<f64>, cap: usize) -> Self {
        values.clear();
        if values.capacity() < cap {
            values.reserve(cap - values.capacity());
        }
        Self {
            values,
            sum: 0.0,
            anchor: 0.0,
            has_anchor: false,
            sum_d: 0.0,
            sum_dd: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            excluded_count: 0,
        }
    }

    #[inline(always)]
    fn push_cell(&mut self, cell: &Cell) {
        if cell.excluded {
            self.excluded_count = self.excluded_count.saturating_add(1);
            return;
        }
        if let CellValue::Number(v) = cell.value
            && v.is_finite()
        {
            self.values.push(v);
            self.sum += v;
            if !self.has_anchor {
                self.anchor = v;
                self.has_anchor = true;
            }
            let d = v - self.anchor;
            self.sum_d += d;
            self.sum_dd += d * d;
            if v < self.min {
                self.min = v;
            }
            if v > self.max {
                self.max = v;
            }
        }
    }

    fn finalize(self, total_selected: usize, truncated_at_cap: bool) -> QuickStatsSummary {
        let n = self.values.len();
        if n == 0 {
            return QuickStatsSummary {
                n: 0,
                excluded_count: self.excluded_count,
                selected_cells: total_selected,
                mean: None,
                sd: None,
                sem: None,
                median: None,
                min: None,
                max: None,
                range: None,
                sum: None,
                truncated_at_cap,
            };
        }

        let nf = n as f64;
        let mean = self.anchor + (self.sum_d / nf);
        let (sd, sem) = if n >= 2 {
            let span = (self.max - self.min).max(1e-9);
            let max_abs = self.min.abs().max(self.max.abs());
            let m2 = if max_abs <= 1e5 * span {
                (self.sum_dd - (self.sum_d * self.sum_d) / nf).max(0.0)
            } else {
                let mut acc = 0.0_f64;
                for &x in self.values.iter() {
                    let d = x - mean;
                    acc += d * d;
                }
                acc
            };
            let var = (m2 / ((n - 1) as f64)).max(0.0);
            let s = var.sqrt();
            (Some(s), Some(s / nf.sqrt()))
        } else {
            (None, None)
        };

        let median = if n == 1 {
            Some(self.values[0])
        } else if n % 2 == 1 {
            let mid = n / 2;
            let (_, &mut med, _) = self.values.select_nth_unstable_by(mid, finite_f64_cmp);
            Some(med)
        } else {
            let mid = n / 2;
            let (left, &mut upper, _) = self.values.select_nth_unstable_by(mid, finite_f64_cmp);
            let mut lower = f64::NEG_INFINITY;
            for &v in left.iter() {
                if v > lower {
                    lower = v;
                }
            }
            Some(0.5 * (lower + upper))
        };

        let min = Some(self.min);
        let max = Some(self.max);
        let range = Some((self.max - self.min).max(0.0));
        QuickStatsSummary {
            n,
            excluded_count: self.excluded_count,
            selected_cells: total_selected,
            mean: Some(mean),
            sd,
            sem,
            median,
            min,
            max,
            range,
            sum: Some(self.sum),
            truncated_at_cap,
        }
    }
}

#[inline]
fn scan_dataset_col_range(rows: &[Vec<Cell>], r0: usize, r_end_incl: usize, subcol: usize, acc: &mut StatsAccum<'_>) {
    if rows.is_empty() {
        return;
    }
    let hi = r_end_incl.min(rows.len() - 1);
    let Some(slice) = rows.get(r0..=hi) else {
        return;
    };
    for row_cells in slice {
        if let Some(cell) = row_cells.get(subcol) {
            acc.push_cell(cell);
        }
    }
}

fn scan_rect_selection(
    ds: &DataSheet,
    r0: usize,
    r1: usize,
    col_subcols: &[(usize, usize)],
    scratch_values: &mut Vec<f64>,
) -> (QuickStatsSummary, usize) {
    let total_rows = r1.saturating_sub(r0).saturating_add(1);
    let total_selected = total_rows.saturating_mul(col_subcols.len());
    let truncated_at_cap = total_selected > QUICK_STATS_CELL_CAP;
    let cap_budget = total_selected.min(QUICK_STATS_CELL_CAP);
    let mut acc = StatsAccum::new(scratch_values, cap_budget);
    let mut remaining = cap_budget;

    for &(col, subcol) in col_subcols {
        if remaining == 0 {
            break;
        }
        let rows_for_col = remaining.min(total_rows);
        let r_end = r0.saturating_add(rows_for_col - 1).min(r1);
        match &ds.archetype {
            TableArchetype::Xy(t) => {
                if let Some(c) = t.data_sets.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::Column(t) => {
                if let Some(c) = t.data_sets.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::Grouped(t) => {
                if let Some(c) = t.data_sets.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::Contingency(t) => {
                if let Some(c) = t.data_sets.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::Survival(t) => {
                if let Some(c) = t.cohorts.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::PartsOfWhole(t) => {
                if let Some(c) = t.data_sets.get(col) {
                    scan_dataset_col_range(&c.cells, r0, r_end, subcol, &mut acc);
                }
            }
            TableArchetype::MultipleVariables(t) => {
                if subcol == 0
                    && let Some(var) = t.variables.get(col)
                    && !var.cells.is_empty()
                {
                    let hi = r_end.min(var.cells.len() - 1);
                    if let Some(slice) = var.cells.get(r0..=hi) {
                        for cell in slice {
                            acc.push_cell(cell);
                        }
                    }
                }
            }
            TableArchetype::Nested(t) => {
                if let Some(grp) = t.groups.get(col) {
                    scan_dataset_col_range(&grp.cells, r0, r_end, subcol, &mut acc);
                }
            }
        }
        remaining = remaining.saturating_sub(rows_for_col);
    }

    (acc.finalize(total_selected, truncated_at_cap), total_selected)
}

/// Recompute [`QuickStatsSummary`] only when `(sheet_id, mutation_seq, selection_key)` changed,
/// preserving `O(1)` steady-frame cost on unchanged frames and `< 0.5 ms` on 50k selected cells.
pub fn compute_cached_quick_stats<'a>(app: &UiApp, cache: &'a mut QuickStatsCache) -> &'a QuickStatsSummary {
    let doc = app.document();
    let sheet_id = doc.active_sheet_id.clone();
    let mutation_seq = app.mutation_seq();
    let bounds_key = app.grid.tool.selection_bounds().map(|(a, b)| (a.row, a.col, a.subcol, b.row, b.col, b.subcol));
    let discontig_len = app.grid.selected_coords.len();
    let discontig_hash = hash_discontiguous_coords(&app.grid.selected_coords);

    if cache.sheet_id == sheet_id
        && cache.mutation_seq == mutation_seq
        && cache.selection_bounds_key == bounds_key
        && cache.discontig_len == discontig_len
        && cache.discontig_hash == discontig_hash
    {
        return &cache.summary;
    }

    cache.sheet_id = sheet_id.clone();
    cache.mutation_seq = mutation_seq;
    cache.selection_bounds_key = bounds_key;
    cache.discontig_len = discontig_len;
    cache.discontig_hash = discontig_hash;
    cache.recompute_count = cache.recompute_count.saturating_add(1);

    let Some(ds) = sheet_id.as_ref().and_then(|id| doc.data_sheet(id).or_else(|| doc.result_sheet(id).and_then(|rs| rs.analyzable_data.as_ref())))
    else {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    };

    if discontig_len > 1 {
        let total_selected = discontig_len;
        let truncated_at_cap = total_selected > QUICK_STATS_CELL_CAP;
        let cap = total_selected.min(QUICK_STATS_CELL_CAP);
        let mut acc = StatsAccum::new(&mut cache.scratch_values.0, cap);
        for &coord in app.grid.selected_coords.iter().take(cap) {
            if let Some(cell) = ds.cell(CellCoord::from(coord)) {
                acc.push_cell(cell);
            }
        }
        cache.summary = acc.finalize(total_selected, truncated_at_cap);
        return &cache.summary;
    }

    let rect_bounds = bounds_key.or_else(|| app.grid.selected_coords.first().map(|c| (c.row, c.col, c.subcol, c.row, c.col, c.subcol)));
    let Some((ar, ac, asub, br, bc, bsub)) = rect_bounds else {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    };

    let row_count = ds.archetype.row_count();
    let total_flat = total_flat_data_subcolumns(&ds.archetype);
    if row_count == 0 || total_flat == 0 {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    }

    let r0 = ar.min(br);
    if r0 >= row_count {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    }
    let r1 = ar.max(br).min(row_count - 1);
    let fa = flat_index_for_coord_fast(&ds.archetype, ac, asub);
    let fb = flat_index_for_coord_fast(&ds.archetype, bc, bsub);
    let lo = fa.min(fb);
    if lo >= total_flat {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    }
    let hi = fa.max(fb).min(total_flat - 1);
    let col_subcols: Vec<(usize, usize)> = (lo..=hi).filter_map(|flat| resolve_flat_coord(&ds.archetype, flat)).collect();
    if col_subcols.is_empty() {
        cache.summary = QuickStatsSummary::default();
        return &cache.summary;
    }

    let (summary, _) = scan_rect_selection(ds, r0, r1, &col_subcols, &mut cache.scratch_values.0);
    cache.summary = summary;
    &cache.summary
}

#[must_use]
fn format_compact_stat(val: f64) -> String {
    if !val.is_finite() {
        return String::from("—");
    }
    let abs = val.abs();
    if abs != 0.0 && !(1e-3..1e6).contains(&abs) {
        format!("{val:.2e}")
    } else if (val - val.round()).abs() < 1e-9 && abs < 1e6 {
        format!("{:.0}", val.round())
    } else {
        format!("{val:.3}")
    }
}

/// Persistent UI state for Footer popups (`PreviewFilter` menu, sheet switcher, link details, stats details).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusFooterEguiState {
    pub filter_popup_open: bool,
    pub sheet_switcher_open: bool,
    pub link_popover_open: bool,
    pub stats_popover_open: bool,
}

/// Input context bundle for [`render_status_footer`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct StatusFooterContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub history_can_back: bool,
    pub history_can_forward: bool,
    pub hover_readout: Option<&'a CanvasHoverReadout>,
    pub enabled: bool,
}

/// Per-frame geometry & telemetry report emitted by [`render_status_footer`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusFooterFrameReport {
    pub footer_rect_pt: Option<egui::Rect>,
    pub section_pills_shown: bool,
    pub quick_stats_shown: bool,
    pub cursor_readout_shown: bool,
    pub zoom_controls_visible: bool,
    pub rulers_toggle_visible: bool,
    pub link_indicator_shown: bool,
    pub computed_quick_stats: Option<QuickStatsSummary>,
}

/// Inspect active sheet provenance / [`LinkedDataRegion`] health for footer telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkHealthSummary {
    badge_text: String,
    tooltip: String,
    accent: Color32,
    upstream_links: Vec<(String, String, Option<SheetId>, Color32)>,
}

fn summarize_sheet_links(app: &UiApp, tokens: &Tokens) -> Option<LinkHealthSummary> {
    let doc = app.document();
    let active_id = doc.active_sheet_id.as_ref()?;
    if let Some(ds) = doc.data_sheet(active_id)
        && !ds.linked_regions.is_empty()
    {
        return Some(build_data_links_summary(doc, ds, tokens));
    }
    if let Some(rs) = doc.result_sheet(active_id) {
        let src_id = &rs.analysis.source_sheet_id;
        let src_title = doc_sheet_title(doc, src_id).unwrap_or_else(|| src_id.as_str()).to_owned();
        let (accent, status_tag) = match rs.status {
            ResultStatus::UpToDate => (tokens.link_green, "Synced"),
            ResultStatus::Dirty => (tokens.accent, "Dirty"),
            ResultStatus::Failed { .. } => (tokens.link_red, "Error"),
        };
        return Some(LinkHealthSummary {
            badge_text: format!("↗ {src_title}"),
            tooltip: format!("Upstream source: {src_title} ({status_tag})"),
            accent,
            upstream_links: vec![(src_title, String::from(status_tag), Some(src_id.clone()), accent)],
        });
    }
    if let Some(gs) = doc.graph_sheet(active_id) {
        let src_id = &gs.spec.primary_source_sheet_id;
        let src_title = doc_sheet_title(doc, src_id).unwrap_or_else(|| src_id.as_str()).to_owned();
        let missing = doc.sheet_section(src_id).is_none();
        let accent = if missing { tokens.link_red } else { tokens.link_blue };
        return Some(LinkHealthSummary {
            badge_text: if missing { format!("Broken: {src_title}") } else { format!("↗ {src_title}") },
            tooltip: format!("Primary source: {src_title}{}", if missing { " (missing)" } else { "" }),
            accent,
            upstream_links: vec![(src_title, String::from(if missing { "Missing" } else { "Linked" }), (!missing).then_some(src_id.clone()), accent)],
        });
    }
    None
}

fn link_border_color(region: &LinkedDataRegion, doc: &ProjectDocument, tokens: &Tokens) -> (Color32, &'static str) {
    if let Some(up_id) = region.spec.source.upstream_sheet_id() {
        if let Some(rs) = doc.result_sheet(up_id)
            && matches!(rs.status, ResultStatus::Failed { .. })
        {
            return (tokens.link_red, "Upstream failed");
        }
        if doc.sheet_section(up_id).is_none() {
            return (tokens.link_red, "Upstream missing");
        }
    }
    match region.border_color() {
        Some(LinkBorderColor::Green) => (tokens.link_green, "Transform result"),
        Some(LinkBorderColor::Red) => (tokens.link_red, "Terminal result"),
        Some(LinkBorderColor::Blue) => (tokens.link_blue, "Linked sheet"),
        Some(LinkBorderColor::Charcoal) | None => (tokens.text_muted, "External link"),
    }
}

fn build_data_links_summary(doc: &ProjectDocument, ds: &DataSheet, tokens: &Tokens) -> LinkHealthSummary {
    let mut has_broken = false;
    let mut has_green = false;
    let mut has_blue = false;
    let mut has_red = false;
    let mut broken_count = 0usize;
    let mut upstream_links = Vec::with_capacity(ds.linked_regions.len());

    for reg in &ds.linked_regions {
        let (clr, status_lbl) = link_border_color(reg, doc, tokens);
        let broken = crate::egui_shell::data_grid::is_linked_region_broken(doc, reg);
        if broken {
            has_broken = true;
            broken_count += 1;
        } else if clr == tokens.link_green {
            has_green = true;
        } else if clr == tokens.link_blue {
            has_blue = true;
        } else if clr == tokens.link_red {
            has_red = true;
        }
        let up_id = reg.spec.source.upstream_sheet_id().cloned();
        let up_label = up_id.as_ref().and_then(|sid| doc_sheet_title(doc, sid)).map_or_else(|| reg.provenance_summary.clone(), str::to_owned);
        upstream_links.push((up_label, String::from(status_lbl), up_id, clr));
    }

    let accent = if has_broken || has_red {
        tokens.link_red
    } else if has_green {
        tokens.link_green
    } else if has_blue {
        tokens.link_blue
    } else {
        tokens.text_muted
    };
    let n = ds.linked_regions.len();
    let badge_text = if n == 1 {
        let first = upstream_links.first().map_or("Link", |(t, _, _, _)| t.as_str());
        if has_broken { format!("Broken: {first}") } else { format!("🔗 {first}") }
    } else if has_broken {
        format!("Broken: {broken_count}/{n} links")
    } else {
        format!("🔗 {n} links")
    };
    let tooltip = upstream_links.iter().map(|(name, st, _, _)| format!("{name} ({st})")).collect::<Vec<_>>().join(" • ");
    LinkHealthSummary { badge_text, tooltip, accent, upstream_links }
}

fn first_sheet_in_section(doc: &ProjectDocument, section: SheetSection) -> Option<&SheetId> {
    match section {
        SheetSection::Data => doc.data_sheets.first().map(|s| &s.id),
        SheetSection::Info => doc.info_sheets.first().map(|s| &s.id),
        SheetSection::Analyses => doc.result_sheets.first().map(|s| &s.id),
        SheetSection::Graphs => doc.graph_sheets.first().map(|s| &s.id),
        SheetSection::Layouts => doc.layout_sheets.first().map(|s| &s.id),
    }
}

#[must_use]
fn localized_preview_filter_label(locale: &str, filter: CanvasPreviewFilter) -> &'static str {
    let is_cjk_zh = locale.starts_with("zh");
    match filter {
        CanvasPreviewFilter::Normal => {
            if is_cjk_zh {
                "正常视觉"
            } else {
                "Normal"
            }
        }
        CanvasPreviewFilter::Protanopia => "Protanopia",
        CanvasPreviewFilter::Deuteranopia => "Deuteranopia",
        CanvasPreviewFilter::Tritanopia => "Tritanopia",
        CanvasPreviewFilter::GrayscalePrint => "Grayscale",
    }
}

#[must_use]
fn short_preview_filter_badge(filter: CanvasPreviewFilter) -> &'static str {
    match filter {
        CanvasPreviewFilter::Normal => "CVD",
        CanvasPreviewFilter::Protanopia => "CVD: Prot",
        CanvasPreviewFilter::Deuteranopia => "CVD: Deut",
        CanvasPreviewFilter::Tritanopia => "CVD: Trit",
        CanvasPreviewFilter::GrayscalePrint => "Gray",
    }
}

#[must_use]
fn center_stage_cursor_coords(ui: &egui::Ui, app: &UiApp, views: &EguiShellViewState, layout: ResponsiveShellLayout) -> Option<(f64, f64)> {
    let pos = ui.ctx().pointer_latest_pos()?;
    let doc = app.document();
    if let Some(active_id) = doc.active_sheet_id.as_ref()
        && views.canvas_view.last_active_paper_sheet_id.as_ref() == Some(active_id)
        && (doc.graph_sheet(active_id).is_some() || doc.layout_sheet(active_id).is_some())
        && let (Some(paper), Some([scene_w, scene_h])) = (views.canvas_view.last_paper_rect_pt, views.canvas_view.last_scene_size_pt)
    {
        if !paper.contains(pos) {
            return None;
        }
        let rel_x = f64::from(pos.x - paper.left()) / f64::from(paper.width().max(1.0)) * scene_w;
        let rel_y = f64::from(pos.y - paper.top()) / f64::from(paper.height().max(1.0)) * scene_h;
        return Some((rel_x, rel_y));
    }

    let stage_x = if layout.rtl_mirrored { layout.trailing_dock_w_pt } else { layout.leading_dock_w_pt };
    let stage_rect = Rect::from_min_size(
        Pos2::new(stage_x, layout.top_chrome_h_pt),
        Vec2::new(layout.center_stage_w_pt.max(0.0), layout.center_stage_h_pt.max(0.0)),
    );
    if !stage_rect.contains(pos) {
        return None;
    }
    let zoom = app.canvas_dialogs.canvas.zoom.clamp(0.1, 8.0);
    let pan = views.canvas_view.pan_pt;
    let rel_x = f64::from(pos.x - stage_rect.min.x - pan[0]) / zoom;
    let rel_y = f64::from(pos.y - stage_rect.min.y - pan[1]) / zoom;
    Some((rel_x, rel_y))
}

struct FooterChipSpec<'a> {
    label: &'a str,
    tooltip: &'a str,
    max_w: f32,
    min_w: f32,
    selected: bool,
    enabled: bool,
    accent_override: Option<Color32>,
    widget_id: &'a str,
    role: &'a str,
    command_id: Option<&'a str>,
    disabled_reason: Option<&'a str>,
}

fn draw_footer_chip(ui: &mut egui::Ui, tokens: &Tokens, spec: FooterChipSpec<'_>, meta: &mut FrameInspectMeta) -> Response {
    let FooterChipSpec { label, tooltip, max_w, min_w, selected, enabled, accent_override, widget_id, role, command_id, disabled_reason } = spec;
    let font_id = egui::FontId::proportional(tokens.metrics.font_ui_small_pt);
    let text_color = if !enabled {
        tokens.text_disabled
    } else if let Some(ov) = accent_override {
        ov
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let max_text_w = (max_w - 12.0).max(8.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, text_color);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = max_text_w;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let ellipsized = galley.elided;
    let w = (galley.size().x + 12.0).clamp(min_w.min(max_w), max_w.max(12.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, FOOTER_CTRL_H), if enabled { Sense::click() } else { Sense::hover() });

    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let border_c = if !enabled {
        tokens.divider
    } else if selected {
        accent_override.unwrap_or(tokens.accent)
    } else if resp.hovered() {
        tokens.accent
    } else {
        tokens.button_border
    };

    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, 4.0, bg, Stroke::new(1.0, border_c), StrokeKind::Inside);
        let text_pos = Pos2::new(rect.min.x + ((rect.width() - galley.size().x) * 0.5).max(5.0), rect.center().y - galley.size().y * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(3.0, 1.0)));
        painter.galley(text_pos, galley, text_color);
    }

    let acc_name = label.to_owned();
    let ak_role = match role {
        "menuitem" => egui::Role::MenuItem,
        "tab" => egui::Role::Tab,
        "text" => egui::Role::Label,
        _ => egui::Role::Button,
    };
    resp.widget_info(move || egui::WidgetInfo::labeled(ak_role, enabled, acc_name.clone()));

    let resp = if !enabled {
        if let Some(reason) = disabled_reason { resp.on_disabled_hover_text(reason) } else { resp }
    } else if ellipsized {
        resp.on_hover_text(label)
    } else if !tooltip.is_empty() && tooltip != label {
        resp.on_hover_text(tooltip)
    } else {
        resp
    };

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role,
                label,
                command_id,
                value: None,
                enabled,
                disabled_reason: (!enabled).then_some(disabled_reason).flatten(),
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(label),
            },
        );
    }
    resp
}

struct FooterSummarySpec<'a> {
    text: &'a str,
    tooltip: &'a str,
    max_w: f32,
    clickable: bool,
    widget_id: &'a str,
}

fn draw_footer_summary_label(ui: &mut egui::Ui, tokens: &Tokens, spec: FooterSummarySpec<'_>, meta: &mut FrameInspectMeta) -> Response {
    let font_id = egui::FontId::proportional(tokens.metrics.font_ui_small_pt);
    let mut job = egui::text::LayoutJob::simple_singleline(spec.text.to_owned(), font_id, tokens.text_primary);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = (spec.max_w - 8.0).max(8.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let ellipsized = galley.elided;
    let w = (galley.size().x + 8.0).clamp(24.0_f32.min(spec.max_w.max(8.0)), spec.max_w.max(8.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, FOOTER_CTRL_H), if spec.clickable { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        if spec.clickable && resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, tokens.hover_bg);
        }
        let text_pos = Pos2::new(rect.min.x + 4.0, rect.center().y - galley.size().y * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(2.0, 1.0)));
        painter.galley(text_pos, galley, tokens.text_primary);
    }
    let acc_sum = spec.text.to_owned();
    let sum_role = if spec.clickable { egui::Role::Button } else { egui::Role::Label };
    resp.widget_info(move || egui::WidgetInfo::labeled(sum_role, true, acc_sum.clone()));

    let resp = if ellipsized {
        resp.on_hover_text(spec.text)
    } else if !spec.tooltip.is_empty() && spec.tooltip != spec.text {
        resp.on_hover_text(spec.tooltip)
    } else {
        resp
    };
    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: spec.widget_id,
                role: if spec.clickable { "button" } else { "text" },
                label: spec.text,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(spec.text),
            },
        );
    }
    resp
}

fn doc_has_any_sheet(doc: &ProjectDocument) -> bool {
    !doc.data_sheets.is_empty()
        || !doc.info_sheets.is_empty()
        || !doc.result_sheets.is_empty()
        || !doc.graph_sheets.is_empty()
        || !doc.layout_sheets.is_empty()
}

struct FooterPopupSpec<'a> {
    viewport: Rect,
    anchor: Rect,
    popup_id: &'a str,
    min_width: f32,
    max_width: f32,
}

fn show_footer_popup<F>(ctx: &egui::Context, tokens: &Tokens, spec: FooterPopupSpec<'_>, add_contents: F) -> Rect
where
    F: FnOnce(&mut egui::Ui),
{
    let est_x = spec.anchor.min.x.clamp(spec.viewport.min.x + 4.0, (spec.viewport.max.x - spec.min_width - 4.0).max(spec.viewport.min.x + 4.0));
    let area_resp = egui::Area::new(egui::Id::unique(spec.popup_id))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(est_x, (spec.anchor.min.y - 4.0).max(spec.viewport.min.y + 4.0)))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            let frame = egui::Frame::NONE
                .fill(tokens.surface_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(6));
            frame.show(ui, |ui| {
                ui.set_min_width(spec.min_width);
                ui.set_max_width(spec.max_width);
                add_contents(ui);
            });
        });
    area_resp.response.rect
}

struct PopupMenuItemSpec<'a> {
    label: &'a str,
    selected: bool,
    accent: Option<Color32>,
    widget_id: &'a str,
    command_id: Option<&'a str>,
}

fn popup_menu_item(ui: &mut egui::Ui, tokens: &Tokens, spec: PopupMenuItemSpec<'_>, meta: &mut FrameInspectMeta) -> bool {
    let PopupMenuItemSpec { label, selected, accent, widget_id, command_id } = spec;
    let w = ui.available_width().max(80.0);
    let font_id = egui::FontId::proportional(11.5);
    let tc = accent.unwrap_or(if selected { tokens.text_strong } else { tokens.text_primary });
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, tc);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = (w - 14.0).max(16.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let ellipsized = galley.elided;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, POPUP_ROW_H), Sense::click());
    let bg = if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    if ui.is_rect_visible(rect) {
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 4.0, bg);
        }
        let tp = Pos2::new(rect.min.x + 6.0, rect.center().y - galley.size().y * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(4.0, 1.0)));
        painter.galley(tp, galley, tc);
    }
    let acc_item = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::MenuItem, true, acc_item.clone()));
    let resp = if ellipsized { resp.on_hover_text(label) } else { resp };
    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role: "menuitem",
                label,
                command_id,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::DropdownMenu,
                ellipsized,
                full_label: ellipsized.then_some(label),
            },
        );
    }
    resp.clicked()
}

#[derive(Default)]
struct FooterAnchors {
    preview: Option<Rect>,
    sheet: Option<Rect>,
    link: Option<Rect>,
    stats: Option<Rect>,
}

#[derive(Clone, Copy)]
struct FooterTrailingFlags {
    compact: bool,
    show_zoom: bool,
    show_rulers: bool,
}

struct FooterPopoverData<'a> {
    tokens: &'a Tokens,
    anchors: &'a FooterAnchors,
    link_summary: Option<&'a LinkHealthSummary>,
    active_stats: Option<&'a QuickStatsSummary>,
}

fn render_footer_history_and_sections(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: StatusFooterContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut StatusFooterFrameReport,
) -> (Option<Rect>, Option<LinkHealthSummary>) {
    let doc = app.document();
    let locale = &ctx_in.tokens.locale;
    let is_rtl = is_rtl_locale(locale);
    let gallery_open = app.nav.gallery.open;
    let active_section = doc.active_sheet_id.as_ref().and_then(|sid| doc.sheet_section(sid));
    let is_welcome = doc.active_sheet_id.is_none() && !gallery_open;
    let compact = !report.section_pills_shown && ui.available_width() < 760.0;

    let back_resp = draw_footer_chip(
        ui,
        ctx_in.tokens,
        FooterChipSpec {
            label: if is_rtl { ">" } else { "<" },
            tooltip: "Previous sheet in history",
            max_w: 24.0,
            min_w: 24.0,
            selected: false,
            enabled: ctx_in.enabled && ctx_in.history_can_back,
            accent_override: None,
            widget_id: "status.history.back",
            role: "button",
            command_id: None,
            disabled_reason: (!ctx_in.history_can_back).then_some("no back history"),
        },
        meta,
    );
    if back_resp.clicked() {
        intents.push_history_back();
    }

    let fwd_resp = draw_footer_chip(
        ui,
        ctx_in.tokens,
        FooterChipSpec {
            label: if is_rtl { "<" } else { ">" },
            tooltip: "Next sheet in history",
            max_w: 24.0,
            min_w: 24.0,
            selected: false,
            enabled: ctx_in.enabled && ctx_in.history_can_forward,
            accent_override: None,
            widget_id: "status.history.forward",
            role: "button",
            command_id: None,
            disabled_reason: (!ctx_in.history_can_forward).then_some("no forward history"),
        },
        meta,
    );
    if fwd_resp.clicked() {
        intents.push_history_forward();
    }

    if report.section_pills_shown {
        let sections: [(SheetSection, &str, &str, &str); 5] = [
            (SheetSection::Data, "status.section.data", "nav.data_tables", "Data"),
            (SheetSection::Info, "status.section.info", "nav.info", "Info"),
            (SheetSection::Analyses, "status.section.results", "nav.results", "Results"),
            (SheetSection::Graphs, "status.section.graphs", "nav.graphs", "Graphs"),
            (SheetSection::Layouts, "status.section.layouts", "nav.layouts", "Layouts"),
        ];
        for (sec, wid, i18n_key, fallback) in sections {
            let first_id = first_sheet_in_section(doc, sec).cloned();
            let label = tr_or_fallback(locale, i18n_key, fallback);
            let is_selected = !gallery_open && active_section == Some(sec);
            let has_sheets = first_id.is_some();
            let pill_resp = draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label,
                    tooltip: label,
                    max_w: 96.0,
                    min_w: 36.0,
                    selected: is_selected,
                    enabled: ctx_in.enabled && has_sheets,
                    accent_override: None,
                    widget_id: wid,
                    role: "button",
                    command_id: first_id.as_ref().map(|_| "sheet.select"),
                    disabled_reason: (!has_sheets).then_some("no sheets in section"),
                },
                meta,
            );
            if pill_resp.clicked()
                && let Some(target_id) = first_id
            {
                if gallery_open {
                    intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
                }
                intents.push_engine("sheet.select", json!({ "sheet_id": target_id.as_str() }));
            }
        }

        let gal_lbl = tr_or_fallback(locale, "cmd.ui.gallery.toggle", "Gallery");
        let gal_resp = draw_footer_chip(
            ui,
            ctx_in.tokens,
            FooterChipSpec {
                label: gal_lbl,
                tooltip: "Portfolio Gallery",
                max_w: 88.0,
                min_w: 40.0,
                selected: gallery_open,
                enabled: ctx_in.enabled,
                accent_override: None,
                widget_id: "status.section.gallery",
                role: "button",
                command_id: Some("ui.gallery.toggle"),
                disabled_reason: None,
            },
            meta,
        );
        if gal_resp.clicked() {
            intents.push_ui("ui.gallery.toggle", json!({}));
        }
    }

    let active_label = if gallery_open {
        String::from("Portfolio Gallery")
    } else if is_welcome {
        String::from("Welcome")
    } else if let Some(sid) = doc.active_sheet_id.as_ref() {
        let title = doc_sheet_title(doc, sid).unwrap_or_else(|| sid.as_str());
        format!("{title} ▾")
    } else {
        String::from("Sheets ▾")
    };
    let switcher_max_w = if compact { 108.0 } else { 150.0 };
    let any_sheet = doc_has_any_sheet(doc);
    let sw_resp = draw_footer_chip(
        ui,
        ctx_in.tokens,
        FooterChipSpec {
            label: &active_label,
            tooltip: "Switch active sheet",
            max_w: switcher_max_w,
            min_w: 48.0,
            selected: views.status_footer.sheet_switcher_open,
            enabled: ctx_in.enabled && any_sheet,
            accent_override: None,
            widget_id: "status.sheet_switcher",
            role: "button",
            command_id: Some("sheet.select"),
            disabled_reason: (!any_sheet).then_some("project has no sheets"),
        },
        meta,
    );
    let sheet_anchor = Some(sw_resp.rect);
    if sw_resp.clicked() {
        views.status_footer.sheet_switcher_open = !views.status_footer.sheet_switcher_open;
        views.status_footer.filter_popup_open = false;
        views.status_footer.link_popover_open = false;
        views.status_footer.stats_popover_open = false;
    }

    let link_summary = if !gallery_open { summarize_sheet_links(app, ctx_in.tokens) } else { None };
    (sheet_anchor, link_summary)
}

fn render_footer_center_readout(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: StatusFooterContext<'_>,
    center_budget: f32,
    meta: &mut FrameInspectMeta,
    report: &mut StatusFooterFrameReport,
) -> (Option<Rect>, Option<QuickStatsSummary>) {
    let doc = app.document();
    let locale = &ctx_in.tokens.locale;
    let gallery_open = app.nav.gallery.open;
    let active_section = doc.active_sheet_id.as_ref().and_then(|sid| doc.sheet_section(sid));
    let is_welcome = doc.active_sheet_id.is_none() && !gallery_open;
    let is_graph_or_layout = !gallery_open && matches!(active_section, Some(SheetSection::Graphs | SheetSection::Layouts));

    if is_graph_or_layout {
        let cursor_pt = center_stage_cursor_coords(ui, app, views, ctx_in.layout);
        let mut readout_parts = Vec::with_capacity(2);
        if let Some(hr) = ctx_in.hover_readout {
            let base = if hr.value_summary.is_empty() { hr.label.clone() } else { format!("{}: {}", hr.label, hr.value_summary) };
            readout_parts.push(base);
        }
        if let Some((cx, cy)) = cursor_pt {
            readout_parts.push(format!("X: {cx:.1}, Y: {cy:.1} pt"));
        }
        let summary_text = if readout_parts.is_empty() {
            let unit_hint = if active_section == Some(SheetSection::Layouts) { "Layout" } else { "Graph" };
            format!("{unit_hint} ready")
        } else {
            report.cursor_readout_shown = true;
            readout_parts.join("  •  ")
        };
        draw_footer_summary_label(
            ui,
            ctx_in.tokens,
            FooterSummarySpec { text: &summary_text, tooltip: &summary_text, max_w: center_budget, clickable: false, widget_id: "status.summary" },
            meta,
        );
        return (None, None);
    }

    if !gallery_open
        && doc
            .active_sheet_id
            .as_ref()
            .is_some_and(|sid| doc.data_sheet(sid).is_some() || doc.result_sheet(sid).is_some_and(|r| r.analyzable_data.is_some()))
    {
        let stats = compute_cached_quick_stats(app, &mut views.quick_stats_cache).clone();
        report.quick_stats_shown = stats.n > 0 || stats.selected_cells > 0;
        let text = stats.format_telemetry(locale);
        report.computed_quick_stats = Some(stats.clone());
        let resp = draw_footer_summary_label(
            ui,
            ctx_in.tokens,
            FooterSummarySpec { text: &text, tooltip: &text, max_w: center_budget, clickable: true, widget_id: "status.summary" },
            meta,
        );
        if resp.clicked() {
            views.status_footer.stats_popover_open = !views.status_footer.stats_popover_open;
            views.status_footer.filter_popup_open = false;
            views.status_footer.sheet_switcher_open = false;
            views.status_footer.link_popover_open = false;
        }
        return (Some(resp.rect), Some(stats));
    }

    let status_text = if gallery_open {
        format!("Gallery ({} items)", app.nav.gallery.cards.len())
    } else if is_welcome {
        String::from("Welcome Hub")
    } else if let Some(sid) = doc.active_sheet_id.as_ref() {
        let title = doc_sheet_title(doc, sid).unwrap_or_else(|| sid.as_str());
        format!("{} — {title}", tr_or_fallback(locale, "w6.view.status.ready", "Ready"))
    } else {
        String::from(tr_or_fallback(locale, "w6.view.status.ready", "Ready"))
    };
    draw_footer_summary_label(
        ui,
        ctx_in.tokens,
        FooterSummarySpec { text: &status_text, tooltip: &status_text, max_w: center_budget, clickable: false, widget_id: "status.summary" },
        meta,
    );
    (None, None)
}

fn render_footer_trailing_controls(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: StatusFooterContext<'_>,
    flags: FooterTrailingFlags,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> Option<Rect> {
    let locale = &ctx_in.tokens.locale;
    let mut preview_anchor: Option<Rect> = None;

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        if flags.show_zoom {
            let zoom = app.canvas_dialogs.canvas.zoom;
            if draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label: "Fit",
                    tooltip: "Fit canvas to viewport (ui.zoom.fitSheet)",
                    max_w: 34.0,
                    min_w: 26.0,
                    selected: false,
                    enabled: ctx_in.enabled,
                    accent_override: None,
                    widget_id: "status.zoom.fit",
                    role: "button",
                    command_id: Some("ui.zoom.fitSheet"),
                    disabled_reason: None,
                },
                meta,
            )
            .clicked()
            {
                intents.push_ui("ui.zoom.fitSheet", json!({}));
            }

            if draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label: "+",
                    tooltip: "Zoom in",
                    max_w: 24.0,
                    min_w: 24.0,
                    selected: false,
                    enabled: ctx_in.enabled && zoom < 7.99,
                    accent_override: None,
                    widget_id: "status.zoom.in",
                    role: "button",
                    command_id: None,
                    disabled_reason: (zoom >= 7.99).then_some("at maximum zoom"),
                },
                meta,
            )
            .clicked()
            {
                intents.push_set_canvas_zoom((zoom * 1.25).clamp(0.1, 8.0));
            }

            let zpct = format!("{:.0}%", zoom * 100.0);
            if draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label: &zpct,
                    tooltip: "Reset zoom to 100%",
                    max_w: 44.0,
                    min_w: 36.0,
                    selected: false,
                    enabled: ctx_in.enabled,
                    accent_override: None,
                    widget_id: "status.zoom.reset",
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
                meta,
            )
            .clicked()
            {
                intents.push_set_canvas_zoom(1.0);
            }

            if draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label: "-",
                    tooltip: "Zoom out",
                    max_w: 24.0,
                    min_w: 24.0,
                    selected: false,
                    enabled: ctx_in.enabled && zoom > 0.11,
                    accent_override: None,
                    widget_id: "status.zoom.out",
                    role: "button",
                    command_id: None,
                    disabled_reason: (zoom <= 0.11).then_some("at minimum zoom"),
                },
                meta,
            )
            .clicked()
            {
                intents.push_set_canvas_zoom((zoom * 0.8).clamp(0.1, 8.0));
            }
        }

        if flags.show_rulers {
            let rulers_on = app.canvas_dialogs.rulers_visible;
            let lbl = if flags.compact { "⌗" } else { "Rulers" };
            if draw_footer_chip(
                ui,
                ctx_in.tokens,
                FooterChipSpec {
                    label: lbl,
                    tooltip: "Toggle page rulers & margin guides",
                    max_w: if flags.compact { 28.0 } else { 54.0 },
                    min_w: 26.0,
                    selected: rulers_on,
                    enabled: ctx_in.enabled,
                    accent_override: None,
                    widget_id: "status.rulers",
                    role: "button",
                    command_id: Some("ui.rulers.toggle"),
                    disabled_reason: None,
                },
                meta,
            )
            .clicked()
            {
                intents.push_ui("ui.rulers.toggle", json!({}));
            }
        }

        let filter = app.preview_filter;
        let filter_active = filter != CanvasPreviewFilter::Normal;
        let cvd_label = if flags.compact {
            short_preview_filter_badge(filter).to_owned()
        } else {
            format!("CVD: {}", localized_preview_filter_label(locale, filter))
        };
        let cvd_resp = draw_footer_chip(
            ui,
            ctx_in.tokens,
            FooterChipSpec {
                label: &cvd_label,
                tooltip: "Color-Vision Deficiency & Grayscale Preview",
                max_w: if flags.compact { 64.0 } else { 94.0 },
                min_w: 44.0,
                selected: filter_active || views.status_footer.filter_popup_open,
                enabled: ctx_in.enabled,
                accent_override: filter_active.then_some(ctx_in.tokens.selected_accent_text()),
                widget_id: "status.preview_filter",
                role: "button",
                command_id: Some("ui.previewFilter.set"),
                disabled_reason: None,
            },
            meta,
        );
        preview_anchor = Some(cvd_resp.rect);
        if cvd_resp.clicked() {
            views.status_footer.filter_popup_open = !views.status_footer.filter_popup_open;
            views.status_footer.sheet_switcher_open = false;
            views.status_footer.link_popover_open = false;
            views.status_footer.stats_popover_open = false;
        }
    });

    preview_anchor
}

fn close_all_footer_popovers(state: &mut StatusFooterEguiState) {
    state.filter_popup_open = false;
    state.sheet_switcher_open = false;
    state.link_popover_open = false;
    state.stats_popover_open = false;
}

fn render_footer_popovers(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    data: FooterPopoverData<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = data.tokens;
    let anchors = data.anchors;
    let doc = app.document();
    let gallery_open = app.nav.gallery.open;
    let locale = &tokens.locale;
    let mut popup_rects: Vec<Rect> = Vec::new();

    if views.status_footer.filter_popup_open
        && let Some(anchor) = anchors.preview
    {
        let mut close_all = false;
        let options = [
            (CanvasPreviewFilter::Normal, "status.preview_filter.normal"),
            (CanvasPreviewFilter::Protanopia, "status.preview_filter.protanopia"),
            (CanvasPreviewFilter::Deuteranopia, "status.preview_filter.deuteranopia"),
            (CanvasPreviewFilter::Tritanopia, "status.preview_filter.tritanopia"),
            (CanvasPreviewFilter::GrayscalePrint, "status.preview_filter.grayscale"),
        ];
        let popup_rect = show_footer_popup(
            ui.ctx(),
            tokens,
            FooterPopupSpec { viewport: meta.viewport_rect, anchor, popup_id: "egui_shell.footer.cvd_popup", min_width: 148.0, max_width: 200.0 },
            |ui| {
                for (flt, wid) in options {
                    let lbl = localized_preview_filter_label(locale, flt);
                    if popup_menu_item(
                        ui,
                        tokens,
                        PopupMenuItemSpec {
                            label: lbl,
                            selected: app.preview_filter == flt,
                            accent: None,
                            widget_id: wid,
                            command_id: Some("ui.previewFilter.set"),
                        },
                        meta,
                    ) {
                        intents.push_set_preview_filter(flt);
                        close_all = true;
                    }
                }
            },
        );
        popup_rects.push(popup_rect);
        if close_all {
            views.status_footer.filter_popup_open = false;
        }
    } else {
        views.status_footer.filter_popup_open = false;
    }

    if views.status_footer.sheet_switcher_open
        && let Some(anchor) = anchors.sheet
    {
        let mut chosen_sheet: Option<SheetId> = None;
        let popup_rect = show_footer_popup(
            ui.ctx(),
            tokens,
            FooterPopupSpec {
                viewport: meta.viewport_rect,
                anchor,
                popup_id: "egui_shell.footer.sheet_switcher",
                min_width: 180.0,
                max_width: 260.0,
            },
            |ui| {
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    let entries: Vec<(SheetId, String, &str)> = doc
                        .data_sheets
                        .iter()
                        .map(|s| (s.id.clone(), s.title.clone(), "Data"))
                        .chain(doc.info_sheets.iter().map(|s| (s.id.clone(), s.title.clone(), "Info")))
                        .chain(doc.result_sheets.iter().map(|s| (s.id.clone(), s.title().to_owned(), "Result")))
                        .chain(doc.graph_sheets.iter().map(|s| (s.id.clone(), s.title.clone(), "Graph")))
                        .chain(doc.layout_sheets.iter().map(|s| (s.id.clone(), s.title.clone(), "Layout")))
                        .collect();
                    for (sid, title, kind_tag) in entries {
                        let selected = !gallery_open && doc.active_sheet_id.as_ref() == Some(&sid);
                        let lbl = format!("[{kind_tag}] {title}");
                        let wid = format!("status.sheet_switcher.item.{}", sid.as_str());
                        if popup_menu_item(
                            ui,
                            tokens,
                            PopupMenuItemSpec { label: &lbl, selected, accent: None, widget_id: &wid, command_id: Some("sheet.select") },
                            meta,
                        ) {
                            chosen_sheet = Some(sid);
                        }
                    }
                });
            },
        );
        popup_rects.push(popup_rect);
        if let Some(sid) = chosen_sheet {
            if gallery_open {
                intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
            }
            intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str() }));
            views.status_footer.sheet_switcher_open = false;
        }
    } else {
        views.status_footer.sheet_switcher_open = false;
    }

    if views.status_footer.link_popover_open
        && let Some(anchor) = anchors.link
        && let Some(link_info) = data.link_summary
    {
        let mut jump_target: Option<SheetId> = None;
        let mut close_link = false;
        let popup_rect = show_footer_popup(
            ui.ctx(),
            tokens,
            FooterPopupSpec { viewport: meta.viewport_rect, anchor, popup_id: "egui_shell.footer.link_popover", min_width: 210.0, max_width: 290.0 },
            |ui| {
                for (idx, (name, st, up_id, clr)) in link_info.upstream_links.iter().enumerate() {
                    let lbl = format!("{name} ({st})");
                    let wid = format!("status.link_indicator.item.{idx}");
                    if popup_menu_item(
                        ui,
                        tokens,
                        PopupMenuItemSpec {
                            label: &lbl,
                            selected: false,
                            accent: Some(*clr),
                            widget_id: &wid,
                            command_id: up_id.as_ref().map(|_| "sheet.select"),
                        },
                        meta,
                    ) {
                        jump_target = up_id.clone();
                        close_link = true;
                    }
                }
            },
        );
        popup_rects.push(popup_rect);
        if let Some(sid) = jump_target {
            intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str() }));
        }
        if close_link {
            views.status_footer.link_popover_open = false;
        }
    } else {
        views.status_footer.link_popover_open = false;
    }

    if views.status_footer.stats_popover_open
        && let Some(anchor) = anchors.stats
        && let Some(stats) = data.active_stats
    {
        let popup_rect = show_footer_popup(
            ui.ctx(),
            tokens,
            FooterPopupSpec { viewport: meta.viewport_rect, anchor, popup_id: "egui_shell.footer.stats_popover", min_width: 200.0, max_width: 260.0 },
            |ui| {
                let rows = [
                    ("N", stats.n.to_string()),
                    ("Selected", stats.selected_cells.to_string()),
                    ("Excluded", stats.excluded_count.to_string()),
                    ("Mean", stats.mean.map_or(String::from("—"), format_compact_stat)),
                    ("SD", stats.sd.map_or(String::from("—"), format_compact_stat)),
                    ("SEM", stats.sem.map_or(String::from("—"), format_compact_stat)),
                    ("Median", stats.median.map_or(String::from("—"), format_compact_stat)),
                    ("Range", stats.range.map_or(String::from("—"), format_compact_stat)),
                    ("Min", stats.min.map_or(String::from("—"), format_compact_stat)),
                    ("Max", stats.max.map_or(String::from("—"), format_compact_stat)),
                    ("Sum", stats.sum.map_or(String::from("—"), format_compact_stat)),
                ];
                for (k, v) in rows {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(k).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(v).size(tokens.metrics.font_ui_small_pt).strong().color(tokens.text_primary));
                        });
                    });
                }
            },
        );
        popup_rects.push(popup_rect);
    } else {
        views.status_footer.stats_popover_open = false;
    }

    if !popup_rects.is_empty() {
        let esc_dismiss = ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let click_outside = ui.ctx().input(|i| {
            i.pointer.primary_clicked()
                && i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()).is_some_and(|p| {
                    let in_popup = popup_rects.iter().any(|r| r.contains(p));
                    let in_anchor = [anchors.preview, anchors.sheet, anchors.link, anchors.stats].into_iter().flatten().any(|r| r.contains(p));
                    !in_popup && !in_anchor
                })
        });
        if esc_dismiss || click_outside {
            close_all_footer_popovers(&mut views.status_footer);
        }
    }
}

/// Render the `26 pt` bottom Status & Telemetry Footer.
pub fn render_status_footer(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: StatusFooterContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatusFooterFrameReport {
    let mut report = StatusFooterFrameReport::default();
    if ctx_in.layout.status_bar_h_pt <= 0.5 {
        return report;
    }

    let doc = app.document();
    let gallery_open = app.nav.gallery.open;
    let active_section = doc.active_sheet_id.as_ref().and_then(|sid| doc.sheet_section(sid));
    let is_graph_or_layout = !gallery_open && matches!(active_section, Some(SheetSection::Graphs | SheetSection::Layouts));
    let show_zoom = gallery_open || is_graph_or_layout;
    let show_rulers = is_graph_or_layout;
    report.zoom_controls_visible = show_zoom;
    report.rulers_toggle_visible = show_rulers;

    let footer_frame =
        egui::Frame::NONE.fill(ctx_in.tokens.app_bar).stroke(Stroke::new(1.0, ctx_in.tokens.border)).inner_margin(egui::Margin::symmetric(6, 1));

    let resp = egui::Panel::bottom("egui_shell.status_footer")
        .exact_size(ctx_in.layout.status_bar_h_pt.max(26.0))
        .frame(footer_frame)
        .show(ui, |ui| {
            let total_w = ui.available_width().max(80.0);
            report.section_pills_shown = total_w >= 860.0;

            let compact = total_w < 760.0;
            let cvd_w = if compact { 66.0 } else { 96.0 };
            let rulers_w = if show_rulers { if compact { 32.0 } else { 58.0 } } else { 0.0 };
            let zoom_w = if show_zoom { 136.0 } else { 0.0 };
            let right_budget = cvd_w + rulers_w + zoom_w + 18.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut anchors = FooterAnchors::default();

                let (sheet_anchor, link_summary) = render_footer_history_and_sections(ui, app, views, ctx_in, meta, intents, &mut report);
                anchors.sheet = sheet_anchor;

                if let Some(ref link_info) = link_summary
                    && ui.available_width() > right_budget + 80.0
                {
                    report.link_indicator_shown = true;
                    let link_resp = draw_footer_chip(
                        ui,
                        ctx_in.tokens,
                        FooterChipSpec {
                            label: &link_info.badge_text,
                            tooltip: &link_info.tooltip,
                            max_w: if compact { 76.0 } else { 126.0 },
                            min_w: 36.0,
                            selected: views.status_footer.link_popover_open,
                            enabled: ctx_in.enabled,
                            accent_override: Some(link_info.accent),
                            widget_id: "status.link_indicator",
                            role: "button",
                            command_id: None,
                            disabled_reason: None,
                        },
                        meta,
                    );
                    anchors.link = Some(link_resp.rect);
                    if link_resp.clicked() {
                        views.status_footer.link_popover_open = !views.status_footer.link_popover_open;
                        views.status_footer.filter_popup_open = false;
                        views.status_footer.sheet_switcher_open = false;
                        views.status_footer.stats_popover_open = false;
                    }
                }

                let center_budget = (ui.available_width() - right_budget).max(24.0);
                let (stats_anchor, active_stats) = render_footer_center_readout(ui, app, views, ctx_in, center_budget, meta, &mut report);
                anchors.stats = stats_anchor;

                anchors.preview =
                    render_footer_trailing_controls(ui, app, views, ctx_in, FooterTrailingFlags { compact, show_zoom, show_rulers }, meta, intents);

                if !ctx_in.enabled {
                    close_all_footer_popovers(&mut views.status_footer);
                } else {
                    render_footer_popovers(
                        ui,
                        app,
                        views,
                        FooterPopoverData {
                            tokens: ctx_in.tokens,
                            anchors: &anchors,
                            link_summary: link_summary.as_ref(),
                            active_stats: active_stats.as_ref(),
                        },
                        meta,
                        intents,
                    );
                }
            });
        })
        .response
        .rect;

    report.footer_rect_pt = Some(resp);
    report
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, MultipleVariablesTable, TableArchetype};

    use super::{StatsAccum, scan_dataset_col_range, scan_rect_selection};

    #[test]
    fn quick_stats_empty_column_vec_at_r0_zero_does_not_panic() {
        let mut scratch = Vec::new();
        let mut acc = StatsAccum::new(&mut scratch, 16);
        scan_dataset_col_range(&[], 0, 5, 0, &mut acc);
        let summary = acc.finalize(6, false);
        assert_eq!(summary.n, 0);
        assert_eq!(summary.mean, None);

        // Also exercise `scan_rect_selection` on ragged empty column vectors (Column and MultipleVariables).
        let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).expect("column table");
        let mut ds_col = DataSheet::new("ds-empty-col", "Empty Col", TableArchetype::Column(col_tbl)).expect("ds");
        if let TableArchetype::Column(ref mut c) = ds_col.archetype {
            c.data_sets[0].cells.clear();
        }
        let (col_summary, _) = scan_rect_selection(&ds_col, 0, 3, &[(0, 0)], &mut scratch);
        assert_eq!(col_summary.n, 0);
        assert_eq!(col_summary.mean, None);

        let mv_tbl = MultipleVariablesTable::new(1, 1).expect("mv table");
        let mut ds_mv = DataSheet::new("ds-empty-mv", "Empty MV", TableArchetype::MultipleVariables(mv_tbl)).expect("ds");
        if let TableArchetype::MultipleVariables(ref mut mv) = ds_mv.archetype {
            mv.variables[0].cells.clear();
        }
        let (mv_summary, _) = scan_rect_selection(&ds_mv, 0, 3, &[(0, 0)], &mut scratch);
        assert_eq!(mv_summary.n, 0);
        assert_eq!(mv_summary.mean, None);
    }
}
