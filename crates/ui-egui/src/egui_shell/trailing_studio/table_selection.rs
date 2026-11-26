//! Structured Table & Selection studio surface.

use curvebook_doc::{DataSheet, ProjectDocument, SheetId};
use egui::Vec2;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, widgets};
use crate::i18n::tr;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
use super::{TrailingStudioContext, TrailingStudioEguiState};

fn find_downstream_graph_for_data(doc: &ProjectDocument, ds_id: &SheetId) -> Option<SheetId> {
    doc.graph_sheets.iter().find(|g| &g.spec.primary_source_sheet_id == ds_id).map(|g| g.id.clone())
}

pub(super) fn render_table_selection_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let doc = app.document();
    let Some(ds) = ds else {
        widgets::dim_label(ui, tr(tokens.locale, "inspector.no_active_sheet"));
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: "Open Data Table…",
                tooltip: "Create or select a sheet from Navigator",
                selected: false,
                enabled: ctx_in.enabled && !doc.data_sheets.is_empty(),
                primary: false,
                widget_id: "inspector.table.select_first",
                command_id: Some("sheet.select"),
                disabled_reason: doc.data_sheets.is_empty().then_some("no data sheets available"),
            },
        ) && let Some(first) = doc.data_sheets.first()
        {
            intents.push_engine("sheet.select", json!({ "sheet_id": first.id.as_str() }));
        }
        return;
    };

    let arch_label = format!("{:?}", ds.kind());
    ui.label(
        egui::RichText::new(format!("{}  •  {}×{}", arch_label, ds.archetype.row_count(), ds.archetype.column_count()))
            .size(11.5)
            .strong()
            .color(tokens.text_strong),
    );
    if ds.frozen {
        ui.label(egui::RichText::new("Frozen (read-only)").size(tokens.metrics.font_ui_small_pt).color(tokens.accent));
    }

    ui.add_space(4.0);
    widgets::subheader(ui, "Display Decimals");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (opt, lbl) in [(None, "Auto"), (Some(0), "0"), (Some(1), "1"), (Some(2), "2"), (Some(3), "3"), (Some(4), "4")] {
            let wid = match opt {
                None => String::from("inspector.table.decimals.auto"),
                Some(d) => format!("inspector.table.decimals.{d}"),
            };
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Display decimal places",
                    max_w: 46.0,
                    selected: state.table_decimals == opt,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.table_decimals = opt;
                super::set_studio_table_decimals(ui.ctx(), opt);
            }
        }
    });

    ui.add_space(6.0);
    widgets::subheader(ui, "Selection & Graph Subset");
    let resolved_sel = crate::egui_shell::data_grid::resolve_grid_selection(app, ds);
    let selection_in_other_sheet = app.grid.selection_sheet_id.as_ref().is_some_and(|sid| sid != &ds.id);
    let sel_text = resolved_sel.as_ref().map_or_else(
        || String::from(if selection_in_other_sheet { "No cells selected in this table" } else { "No cells selected" }),
        |s| s.summary_label(),
    );
    state.selected_subset_rows_summary = sel_text.clone();
    widgets::dim_label(ui, &sel_text);

    let has_selection = resolved_sel.is_some() && !ds.frozen;
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Exclude Selected Subset",
            tooltip: "Exclude selected rows/columns from graphs and analyses (table.excludeCells)",
            selected: false,
            enabled: ctx_in.enabled && has_selection,
            primary: false,
            widget_id: "inspector.table.exclude_subset",
            command_id: Some("table.excludeCells"),
            disabled_reason: ds.frozen.then_some("sheet is frozen").or_else(|| (!has_selection).then_some("select cells first")),
        },
    ) && let Some(sel) = resolved_sel.as_ref()
    {
        crate::egui_shell::data_grid::push_exclude_selection_intent(ds, sel, true, intents);
    }

    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Include Selected Subset",
            tooltip: "Clear exclusion mask on selected cells (table.excludeCells)",
            selected: false,
            enabled: ctx_in.enabled && has_selection,
            primary: false,
            widget_id: "inspector.table.include_subset",
            command_id: Some("table.excludeCells"),
            disabled_reason: ds.frozen.then_some("sheet is frozen").or_else(|| (!has_selection).then_some("select cells first")),
        },
    ) && let Some(sel) = resolved_sel.as_ref()
    {
        crate::egui_shell::data_grid::push_exclude_selection_intent(ds, sel, false, intents);
    }

    let downstream_graph = find_downstream_graph_for_data(doc, &ds.id);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Style Subset in Graph…",
            tooltip: "Override marker color/size for selected rows on downstream graph",
            selected: false,
            enabled: ctx_in.enabled && downstream_graph.is_some(),
            primary: false,
            widget_id: "inspector.table.style_subset",
            command_id: Some("ui.formatPoints.open"),
            disabled_reason: downstream_graph.is_none().then_some("no downstream graph linked to this table"),
        },
    ) && let Some(gid) = downstream_graph
    {
        intents.push_ui("ui.formatPoints.open", json!({ "sheet_id": gid.as_str() }));
    }

    if !ds.linked_regions.is_empty() {
        ui.add_space(6.0);
        widgets::subheader(ui, "Linked Data Regions");
        for (idx, reg) in ds.linked_regions.iter().enumerate() {
            let up_id = reg.spec.source.upstream_sheet_id().cloned();
            let broken = crate::egui_shell::data_grid::is_linked_region_broken(doc, reg);
            let upstream_exists = up_id.as_ref().is_some_and(|sid| doc.sheet_section(sid).is_some());
            let label = if broken { format!("Broken: {}", reg.provenance_summary) } else { reg.provenance_summary.clone() };
            let tooltip = if broken { "Upstream linked sheet is missing or failed" } else { "Jump to upstream linked source sheet" };
            let disabled_reason = if !upstream_exists && up_id.is_some() {
                Some("upstream linked sheet is missing")
            } else if up_id.is_none() {
                Some("external or clipboard provenance")
            } else {
                None
            };
            let wid = format!("inspector.table.link.{idx}");
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: &label,
                    tooltip,
                    selected: false,
                    enabled: ctx_in.enabled && upstream_exists,
                    primary: false,
                    widget_id: &wid,
                    command_id: up_id.as_ref().map(|_| "sheet.select"),
                    disabled_reason,
                },
            ) && let Some(sid) = up_id
            {
                intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str() }));
            }
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Convert Table Archetype…",
            tooltip: "Open Table Archetype Converter",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.table.format_table",
            command_id: Some("ui.table.convertArchetype.open"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": ds.id.as_str() }));
    }
}
