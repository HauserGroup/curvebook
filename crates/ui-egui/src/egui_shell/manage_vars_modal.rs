//! `Manage Variables...` master-detail modal in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.3).
//!
//! Synchronizes against `app.grid.variables_dialog` (`VariablesDialogState`), exposing filter chips,
//! role overrides, title/unit metadata editing, categorical level inline rename/merge,
//! explicit/frequency/alpha/drag ordering, reference level pinning, and single-commit
//! `PendingUiIntents` settlement.

use curvebook_doc::{
    CategoricalLevelSummary, DataSheet, LevelOrderKind, LevelOrderSpec, MultipleVariablesTable, SheetId, TableArchetype, VariableColumn,
    VariableRole, VariableSource,
};
use egui::{Align, Context, Layout, Rect, RichText, Stroke, Ui, vec2};
use serde_json::json;

use crate::UiApp;
use crate::i18n::tr_or_fallback;
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, widgets};

/// Filter tabs exposed in the `Manage Variables...` upper master list (`gui-layout-spec.md` §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ManageVarsFilter {
    #[default]
    All,
    Categorical,
    Continuous,
    Calculated,
    Labels,
}

impl ManageVarsFilter {
    const ALL_FILTERS: [(Self, &str, &str); 5] = [
        (Self::All, "all", "All"),
        (Self::Categorical, "cat", "Categorical"),
        (Self::Continuous, "num", "Continuous"),
        (Self::Calculated, "calc", "Calculated"),
        (Self::Labels, "lbl", "Labels"),
    ];

    fn matches(self, var: &VariableColumn) -> bool {
        match self {
            Self::All => true,
            Self::Categorical => var.role == VariableRole::Categorical,
            Self::Continuous => var.role == VariableRole::Continuous,
            Self::Calculated => var.role == VariableRole::Calculated || matches!(var.source, VariableSource::Calculated { .. }),
            Self::Labels => var.role == VariableRole::Label,
        }
    }
}

/// Persistent transient UI state for `render_manage_vars_modal`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManageVarsModalEguiState {
    pub filter: ManageVarsFilter,
    pub selected_level: Option<String>,
    pub rename_draft: String,
    pub search_query: String,
    pub recode_from_draft: String,
    pub recode_to_draft: String,
    pub title_draft: String,
    pub unit_draft: String,
    pub dragging_level: Option<usize>,
    pub synced_sheet_col: Option<(SheetId, usize)>,
    pub synced_var_title: Option<String>,
    pub synced_level_key: Option<String>,
    pub level_merge_target_draft: String,
    pub compact_detail_tab: usize,
}

/// Frame geometry and modal capture report emitted by `render_manage_vars_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManageVarsModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

#[derive(Clone, Copy)]
struct ManageVarsSheetCtx<'a> {
    ds: &'a DataSheet,
    mv: &'a MultipleVariablesTable,
    col: usize,
    tokens: &'a Tokens,
}

fn split_title_and_unit(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let Some(open) = trimmed.rfind(" (")
        && trimmed.ends_with(')')
        && open > 0
    {
        let name = trimmed[..open].trim().to_owned();
        let unit = trimmed[open + 2..trimmed.len() - 1].trim().to_owned();
        return (name, unit);
    }
    (trimmed.to_owned(), String::new())
}

fn combine_title_and_unit(name: &str, unit: &str) -> String {
    let n = name.trim();
    let u = unit.trim();
    if u.is_empty() { n.to_owned() } else { format!("{n} ({u})") }
}

fn active_mv_sheet(app: &UiApp) -> Option<(&DataSheet, &MultipleVariablesTable)> {
    let doc = app.document();
    let active_id = doc.active_sheet_id.as_ref();
    let ds = active_id
        .and_then(|id| doc.data_sheet(id))
        .or_else(|| doc.data_sheets.iter().find(|s| matches!(s.archetype, TableArchetype::MultipleVariables(_))))?;
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => Some((ds, mv)),
        _ => None,
    }
}

/// Render the master-detail `Manage Variables...` modal when `app.grid.variables_dialog.open` is true.
pub fn render_manage_vars_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut ManageVarsModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ManageVarsModalFrameReport {
    if !app.grid.variables_dialog.open {
        state.synced_sheet_col = None;
        state.synced_var_title = None;
        return ManageVarsModalFrameReport::default();
    }

    let title = tr_or_fallback(tokens.locale, "mv.vars.title", "Manage Variables & Levels");
    let viewport = ctx.content_rect();
    let max_w = (viewport.width() - 24.0).clamp(300.0, 780.0);
    let max_h = (viewport.height() - 24.0).clamp(240.0, 560.0);

    let mv_ctx = active_mv_sheet(app);
    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_manage_vars_dialog")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        let inner_w = (max_w - 24.0).max(260.0);
        ui.set_width(inner_w);
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);

        ui.horizontal(|hdr| {
            hdr.heading(RichText::new(title).color(tokens.text_primary));
            hdr.with_layout(Layout::right_to_left(Align::Center), |rhs| {
                let close_lbl = "×";
                let close_resp = rhs.button(close_lbl);
                meta.record_modal_response(rhs, &close_resp, FrameInspectEntryInput::modal_button("mv_formula.vars.close", close_lbl, true));
                if close_resp.clicked() {
                    intents.push_close_manage_variables();
                }
            });
        });
        widgets::divider(ui);

        let body_h = (max_h - 92.0).max(120.0);
        if let Some((ds, mv)) = mv_ctx {
            let sel_col = app.grid.variables_dialog.selected_column.unwrap_or(0).min(mv.variables.len().saturating_sub(1));
            let col_changed = state.synced_sheet_col.as_ref() != Some(&(ds.id.clone(), sel_col));
            let live_title = mv.variables.get(sel_col).map(|v| v.title.as_str());
            if col_changed {
                state.synced_sheet_col = Some((ds.id.clone(), sel_col));
                state.selected_level = None;
                state.synced_level_key = None;
                state.rename_draft.clear();
                state.dragging_level = None;
            }
            if col_changed || state.synced_var_title.as_deref() != live_title {
                state.synced_var_title = live_title.map(str::to_owned);
                if let Some(title) = live_title {
                    let (n, u) = split_title_and_unit(title);
                    state.title_draft = n;
                    state.unit_draft = u;
                }
            }

            let sheet = ManageVarsSheetCtx { ds, mv, col: sel_col, tokens };
            let compact = inner_w < 540.0;
            if compact {
                egui::ScrollArea::vertical()
                    .id_salt("mv_vars_compact_scroll")
                    .max_width(inner_w)
                    .max_height(body_h)
                    .auto_shrink([false, false])
                    .show(ui, |scroll_ui| {
                        let content_w = (inner_w - 18.0).max(200.0);
                        scroll_ui.set_max_width(content_w);
                        render_master_pane(scroll_ui, sheet, content_w, state, meta, intents);
                        scroll_ui.add_space(6.0);
                        widgets::divider(scroll_ui);
                        scroll_ui.add_space(6.0);
                        render_detail_pane(scroll_ui, app, sheet, body_h, state, meta, intents);
                    });
            } else {
                let left_w = (inner_w * 0.35).clamp(170.0, 240.0);
                let right_w = (inner_w - left_w - 20.0).max(220.0);
                ui.allocate_ui_with_layout(vec2(inner_w, body_h), Layout::left_to_right(Align::Min), |split_ui| {
                    split_ui.allocate_ui_with_layout(vec2(left_w, body_h), Layout::top_down(Align::Min), |left_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("mv_vars_left_scroll")
                            .max_width(left_w)
                            .max_height(body_h)
                            .auto_shrink([false, false])
                            .show(left_ui, |inner| {
                                let lw = (left_w - 16.0).max(140.0);
                                inner.set_max_width(lw);
                                render_master_pane(inner, sheet, lw, state, meta, intents);
                            });
                    });
                    split_ui.separator();
                    split_ui.allocate_ui_with_layout(vec2(right_w, body_h), Layout::top_down(Align::Min), |right_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("mv_vars_right_scroll")
                            .max_width(right_w)
                            .max_height(body_h)
                            .auto_shrink([false, false])
                            .show(right_ui, |inner| {
                                let rw = (right_w - 16.0).max(180.0);
                                inner.set_max_width(rw);
                                render_detail_pane(inner, app, sheet, body_h, state, meta, intents);
                            });
                    });
                });
            }
        } else {
            ui.add_space(12.0);
            ui.label(RichText::new("Select a Multiple Variables sheet to inspect variables and categorical levels.").color(tokens.text_muted));
        }

        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        ui.horizontal(|footer| {
            let cancel_lbl = tr_or_fallback(tokens.locale, "btn.cancel", "Cancel");
            let cancel_resp = widgets::secondary_button(footer, cancel_lbl);
            meta.record_modal_response(footer, &cancel_resp, FrameInspectEntryInput::modal_button("modal.cancel", cancel_lbl, true));
            if cancel_resp.clicked() {
                intents.push_close_manage_variables();
            }

            footer.with_layout(Layout::right_to_left(Align::Center), |rhs| {
                let ok_lbl = tr_or_fallback(tokens.locale, "btn.ok", "OK");
                let ok_resp = widgets::primary_button(rhs, ok_lbl);
                meta.record_modal_response(rhs, &ok_resp, FrameInspectEntryInput::modal_button("modal.ok", ok_lbl, true));
                if ok_resp.clicked() {
                    if let Some((ds, _)) = mv_ctx
                        && !app.grid.variables_dialog.recode_mappings.is_empty()
                    {
                        intents.push_apply_manage_var_recodes(ds.id.clone());
                    }
                    intents.push_close_manage_variables();
                }
            });
        });
    });

    let title_for_ak = title.to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.manage_variables", title, z_dialog: ZLayer::ModalDialog },
        meta,
    );
    ManageVarsModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_master_pane(
    ui: &mut Ui,
    sheet: ManageVarsSheetCtx<'_>,
    avail_w: f32,
    state: &mut ManageVarsModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ManageVarsSheetCtx { ds, mv, col: sel_col, tokens } = sheet;
    ui.label(RichText::new("Variables").strong().color(tokens.text_primary));
    let sr = ui.add(egui::TextEdit::singleline(&mut state.search_query).hint_text("Search variables…").desired_width(avail_w));
    meta.record_modal_response(
        ui,
        &sr,
        FrameInspectEntryInput::modal("mv_formula.vars.search", "searchbox", "Search variables", Some(state.search_query.clone()), true),
    );

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        for (filter_kind, slug, label) in ManageVarsFilter::ALL_FILTERS {
            let active = state.filter == filter_kind;
            let resp = choice_pill(row, tokens, label, active, true);
            let wid = format!("mv_formula.vars.filter.{slug}");
            meta.record_modal_response(row, &resp, FrameInspectEntryInput::modal_button(&wid, label, true));
            if resp.clicked() {
                state.filter = filter_kind;
            }
        }
    });
    ui.add_space(4.0);

    let q_low = state.search_query.trim().to_ascii_lowercase();
    for (idx, var) in mv.variables.iter().enumerate() {
        if !state.filter.matches(var) {
            continue;
        }
        if !q_low.is_empty() && !var.title.to_ascii_lowercase().contains(&q_low) {
            continue;
        }
        let selected = idx == sel_col;
        let role_tag = match (&var.source, var.role) {
            (VariableSource::Calculated { .. }, _) => "FX=",
            (_, VariableRole::Continuous) => "NUM",
            (_, VariableRole::Categorical) => "CAT",
            (_, VariableRole::Label) => "LBL",
            (_, VariableRole::Calculated) => "FX=",
        };
        let missing = var.cells.iter().filter(|c| c.is_empty() || c.excluded).count();
        let text = if var.role_inferred && !matches!(var.source, VariableSource::Calculated { .. }) {
            format!("[Auto·{role_tag}] {} ({missing} miss)", var.title)
        } else {
            format!("[{role_tag}] {} ({missing} miss)", var.title)
        };
        let resp = ui.add_sized(
            vec2(avail_w, 24.0),
            egui::Button::new(RichText::new(&text).color(if selected { tokens.accent } else { tokens.text_primary })).selected(selected),
        );
        let wid = format!("mv_formula.vars.col.{idx}");
        meta.record_modal_response(ui, &resp, FrameInspectEntryInput::modal_button(&wid, &text, true));
        if resp.clicked() {
            intents.push_select_manage_var_column(ds.id.clone(), idx);
        }
    }
}

fn render_detail_pane(
    ui: &mut Ui,
    app: &UiApp,
    sheet: ManageVarsSheetCtx<'_>,
    body_h: f32,
    state: &mut ManageVarsModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ManageVarsSheetCtx { ds, mv, col, tokens } = sheet;
    let Some(var) = mv.variables.get(col) else {
        ui.label("No variable selected.");
        return;
    };
    let is_calc = matches!(var.source, VariableSource::Calculated { .. });
    let vd = &app.grid.variables_dialog;
    let tight_h = body_h < 380.0;

    ui.label(RichText::new(format!("Variable Detail · {}", var.title)).strong().color(tokens.text_primary));
    if tight_h {
        ui.horizontal_wrapped(|trow| {
            for (idx, slug, lbl) in [(0usize, "levels", "Role & Levels"), (1, "recode", "Bulk Recode")] {
                let r = choice_pill(trow, tokens, lbl, state.compact_detail_tab == idx, true);
                meta.record_modal_response(trow, &r, FrameInspectEntryInput::modal_button(&format!("mv_formula.vars.detail_tab.{slug}"), lbl, true));
                if r.clicked() {
                    state.compact_detail_tab = idx;
                }
            }
        });
    }
    ui.add_space(if tight_h { tokens.metrics.space_xxs } else { tokens.metrics.space_xs });

    let show_primary = !tight_h || state.compact_detail_tab == 0;
    let show_recode = !tight_h || state.compact_detail_tab == 1;

    if show_primary {
        // Title & Unit metadata editing.
        let name_w = if tight_h { 92.0 } else { 120.0 };
        let unit_w = if tight_h { 64.0 } else { 82.0 };
        ui.horizontal_wrapped(|row| {
            row.label("Name:");
            let nr = row.add(egui::TextEdit::singleline(&mut state.title_draft).hint_text("Variable name").desired_width(name_w));
            meta.record_modal_response(
                row,
                &nr,
                FrameInspectEntryInput::modal("mv_formula.vars.title_input", "textbox", "Variable Name", Some(state.title_draft.clone()), !ds.frozen),
            );
            row.label("Unit:");
            let ur = row.add(egui::TextEdit::singleline(&mut state.unit_draft).hint_text("e.g. uM, mg/kg").desired_width(unit_w));
            meta.record_modal_response(
                row,
                &ur,
                FrameInspectEntryInput::modal("mv_formula.vars.unit_input", "textbox", "Variable Unit", Some(state.unit_draft.clone()), !ds.frozen),
            );
            let apply_lbl = "Save";
            let can_save = !ds.frozen && !state.title_draft.trim().is_empty();
            let ar = row.add_enabled(can_save, egui::Button::new(apply_lbl));
            meta.record_modal_response(row, &ar, FrameInspectEntryInput::modal_button("mv_formula.vars.meta.apply", apply_lbl, can_save));
            if ar.clicked() {
                let next_title = combine_title_and_unit(&state.title_draft, &state.unit_draft);
                if !next_title.is_empty() {
                    intents.push_engine("table.setTitles", json!({ "sheet_id": ds.id.as_str(), "col": col, "title": next_title }));
                }
            }
        });

        ui.add_space(if tight_h { tokens.metrics.space_xxs } else { tokens.metrics.space_sm });
        // Role + Auto checkbox (`table.setVariableRole`).
        ui.horizontal_wrapped(|row| {
            row.label("Role:");
            let mut auto_checked = var.role_inferred && !is_calc;
            let auto_resp = row.add_enabled(!ds.frozen && !is_calc, egui::Checkbox::new(&mut auto_checked, "Auto"));
            meta.record_modal_response(
                row,
                &auto_resp,
                FrameInspectEntryInput::modal(
                    "mv_formula.vars.lock_toggle",
                    "checkbox",
                    "Auto Role",
                    Some(auto_checked.to_string()),
                    !ds.frozen && !is_calc,
                ),
            );
            if auto_resp.changed() {
                let target_role = if auto_checked {
                    "auto"
                } else {
                    match var.role {
                        VariableRole::Categorical => "categorical",
                        VariableRole::Label => "label",
                        VariableRole::Continuous | VariableRole::Calculated => "continuous",
                    }
                };
                intents.push_engine("table.setVariableRole", json!({ "sheet_id": ds.id.as_str(), "col": col, "role": target_role }));
            }
            for (role_kind, slug, label) in [
                (VariableRole::Continuous, "continuous", "Continuous (NUM)"),
                (VariableRole::Categorical, "categorical", "Categorical (CAT)"),
                (VariableRole::Label, "label", "Label (LBL)"),
            ] {
                let active = var.role == role_kind;
                let rr = choice_pill(row, tokens, label, active, !ds.frozen && !is_calc);
                let wid = format!("mv_formula.vars.role.{slug}");
                meta.record_modal_response(row, &rr, FrameInspectEntryInput::modal_button(&wid, label, !ds.frozen && !is_calc));
                if rr.clicked() {
                    intents.push_engine("table.setVariableRole", json!({ "sheet_id": ds.id.as_str(), "col": col, "role": slug }));
                }
            }
        });

        if let VariableSource::Calculated { formula, .. } = &var.source {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|row| {
                row.label(RichText::new(format!("FX= {formula}")).monospace().color(tokens.role_fx));
                let fr = widgets::secondary_button(row, "Edit Formula…");
                meta.record_modal_response(row, &fr, FrameInspectEntryInput::modal_button("mv_formula.vars.edit_formula", "Edit Formula", true));
                if fr.clicked() {
                    intents.push_close_manage_variables();
                    intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": col }));
                }
            });
        }

        ui.add_space(if tight_h { tokens.metrics.space_xs } else { tokens.metrics.space_md });
        widgets::divider(ui);
        ui.add_space(if tight_h { tokens.metrics.space_xxs } else { tokens.metrics.space_sm });

        // Level Ordering & Reference Level Controls.
        ui.label(RichText::new("Categorical Level Ordering & Reference Pin").strong().color(tokens.text_primary));
        ui.horizontal_wrapped(|row| {
            for (slug, kind, label) in [
                ("entry", LevelOrderKind::EntryOrder, "Entry Order"),
                ("alpha", LevelOrderKind::Alphabetical, "A–Z"),
                ("freq_desc", LevelOrderKind::FrequencyDescending, "Freq ↓"),
                ("freq_asc", LevelOrderKind::FrequencyAscending, "Freq ↑"),
            ] {
                let active = std::mem::discriminant(&vd.level_order.order) == std::mem::discriminant(&kind);
                let r = choice_pill(row, tokens, label, active, !ds.frozen);
                let wid = format!("mv_formula.vars.order.{slug}");
                meta.record_modal_response(row, &r, FrameInspectEntryInput::modal_button(&wid, label, !ds.frozen));
                if r.clicked() {
                    intents.push_apply_manage_var_order(ds.id.clone(), col, LevelOrderSpec { order: kind, reversed: vd.level_order.reversed }, None);
                }
            }
            let mut rev = vd.level_order.reversed;
            let rr = row.checkbox(&mut rev, "Reversed");
            meta.record_modal_response(
                row,
                &rr,
                FrameInspectEntryInput::modal("mv_formula.vars.order.reversed", "checkbox", "Reversed", Some(rev.to_string()), !ds.frozen),
            );
            if rr.changed() {
                intents.push_apply_manage_var_order(ds.id.clone(), col, LevelOrderSpec { order: vd.level_order.order.clone(), reversed: rev }, None);
            }
            let apply_ord = row.add_enabled(!ds.frozen, egui::Button::new("Apply Order"));
            meta.record_modal_response(
                row,
                &apply_ord,
                FrameInspectEntryInput::modal_button("mv_formula.vars.order.apply", "Apply Order", !ds.frozen),
            );
            if apply_ord.clicked() {
                intents.push_apply_manage_var_order(ds.id.clone(), col, vd.level_order.clone(), None);
            }
        });

        ui.add_space(if tight_h { tokens.metrics.space_xxs } else { tokens.metrics.space_xs });
        let live_levels = var.ordered_levels();
        render_levels_inspector(ui, sheet, &live_levels, state, meta, intents);

        if !tight_h {
            ui.add_space(8.0);
            widgets::divider(ui);
            ui.add_space(6.0);
        }
    }

    if show_recode {
        // Bulk Recode & Merge staging.
        ui.label(RichText::new("Stage Level Rename / Merge").strong().color(tokens.text_primary));
        ui.horizontal_wrapped(|row| {
            row.label("From:");
            let rf = row.add(egui::TextEdit::singleline(&mut state.recode_from_draft).hint_text("Old level").desired_width(92.0));
            meta.record_modal_response(
                row,
                &rf,
                FrameInspectEntryInput::modal(
                    "mv_formula.vars.recode_from.input",
                    "textbox",
                    "Recode From",
                    Some(state.recode_from_draft.clone()),
                    !ds.frozen,
                ),
            );
            row.label("To:");
            let rt = row.add(egui::TextEdit::singleline(&mut state.recode_to_draft).hint_text("New level").desired_width(92.0));
            meta.record_modal_response(
                row,
                &rt,
                FrameInspectEntryInput::modal(
                    "mv_formula.vars.recode_to.input",
                    "textbox",
                    "Recode To",
                    Some(state.recode_to_draft.clone()),
                    !ds.frozen,
                ),
            );
            let can_stage = !ds.frozen && !state.recode_from_draft.trim().is_empty();
            let stage_r = row.add_enabled(can_stage, egui::Button::new("Stage"));
            meta.record_modal_response(row, &stage_r, FrameInspectEntryInput::modal_button("mv_formula.vars.recode_stage", "Stage", can_stage));
            if stage_r.clicked() {
                intents.push_stage_manage_var_recode(state.recode_from_draft.trim(), state.recode_to_draft.trim());
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|row| {
            let can_apply = !ds.frozen && !vd.recode_mappings.is_empty();
            let clear_r = row.add_enabled(can_apply, egui::Button::new("Clear"));
            meta.record_modal_response(row, &clear_r, FrameInspectEntryInput::modal_button("mv_formula.vars.recode_clear", "Clear", can_apply));
            if clear_r.clicked() {
                intents.push_clear_manage_var_recodes();
            }
            let apply_r = widgets::secondary_button_enabled(row, "Apply Recodes", can_apply);
            meta.record_modal_response(
                row,
                &apply_r,
                FrameInspectEntryInput::modal_button("mv_formula.vars.recode_apply", "Apply Recodes", can_apply),
            );
            if apply_r.clicked() {
                intents.push_apply_manage_var_recodes(ds.id.clone());
            }
            let preview_txt = if vd.recode_mappings.is_empty() {
                String::from("No staged recodes")
            } else {
                let summary = vd.recode_mappings.iter().map(|(f, t)| format!("{f} -> {t}")).collect::<Vec<_>>().join(", ");
                match vd.preview_recode(ds) {
                    Some((n, lvls)) => format!("{summary} ({n} cells, {} levels)", lvls.len()),
                    None => summary,
                }
            };
            let pr = row.label(RichText::new(&preview_txt).size(11.5).color(tokens.text_muted));
            meta.record_modal_response(
                row,
                &pr,
                FrameInspectEntryInput::modal("mv_formula.vars.recode_preview", "label", &preview_txt, Some(preview_txt.clone()), true),
            );
        });
    }
}

fn render_levels_inspector(
    ui: &mut Ui,
    sheet: ManageVarsSheetCtx<'_>,
    levels: &[CategoricalLevelSummary],
    state: &mut ManageVarsModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ManageVarsSheetCtx { ds, col, tokens, .. } = sheet;
    let mut explicit_names: Vec<String> = levels.iter().map(|l| l.level.clone()).collect();
    let sel_idx = state.selected_level.as_ref().and_then(|s| explicit_names.iter().position(|x| x == s));

    // Toolbar Move Top / Up / Down / End.
    ui.horizontal_wrapped(|bar| {
        let can_up = !ds.frozen && sel_idx.is_some_and(|i| i > 0);
        let can_dn = !ds.frozen && sel_idx.is_some_and(|i| i + 1 < explicit_names.len());
        let top_lbl = tr_or_fallback(tokens.locale, "w7.mv.move_to_top", "Top");
        let up_lbl = tr_or_fallback(tokens.locale, "w7.mv.move_up", "Up");
        let dn_lbl = tr_or_fallback(tokens.locale, "w7.mv.move_down", "Down");
        let end_lbl = tr_or_fallback(tokens.locale, "w7.mv.move_to_end", "End");

        for (wid, lbl, en, target_pos) in [
            ("mv_formula.vars.move_top", top_lbl, can_up, 0usize),
            ("mv_formula.vars.move_up", up_lbl, can_up, sel_idx.unwrap_or(0).saturating_sub(1)),
            ("mv_formula.vars.move_down", dn_lbl, can_dn, (sel_idx.unwrap_or(0) + 1).min(explicit_names.len().saturating_sub(1))),
            ("mv_formula.vars.move_end", end_lbl, can_dn, explicit_names.len().saturating_sub(1)),
        ] {
            let r = choice_pill(bar, tokens, lbl, false, en);
            meta.record_modal_response(bar, &r, FrameInspectEntryInput::modal_button(wid, lbl, en));
            if r.clicked()
                && let Some(i) = sel_idx
                && i < explicit_names.len()
            {
                let item = explicit_names.remove(i);
                explicit_names.insert(target_pos.min(explicit_names.len()), item);
                intents.push_apply_manage_var_order(
                    ds.id.clone(),
                    col,
                    LevelOrderSpec { order: LevelOrderKind::Explicit(explicit_names.clone()), reversed: false },
                    None,
                );
                break;
            }
        }
    });

    if levels.is_empty() {
        ui.label(RichText::new("No categorical levels in this variable.").color(tokens.text_muted));
        return;
    }

    let mut row_rects: Vec<Rect> = Vec::with_capacity(levels.len());
    let mut commit_reorder: Option<(usize, usize)> = None;

    for (idx, item) in levels.iter().enumerate() {
        if ui.cursor().top() > ui.clip_rect().bottom() + 1.0 {
            ui.allocate_space(vec2(ui.available_width().max(1.0), 30.0));
            continue;
        }
        let is_selected = state.selected_level.as_deref() == Some(item.level.as_str());
        if is_selected && state.synced_level_key.as_deref() != Some(item.level.as_str()) {
            state.synced_level_key = Some(item.level.clone());
            state.rename_draft = item.level.clone();
            state.level_merge_target_draft.clear();
        }
        let frame_fill = if is_selected { tokens.accent.linear_multiply(0.14) } else { tokens.panel_bg };
        let resp_frame = egui::Frame::new()
            .fill(frame_fill)
            .stroke(Stroke::new(1.0, if idx == 0 { tokens.accent } else { tokens.border }))
            .corner_radius(4.0)
            .inner_margin(vec2(6.0, 3.0))
            .show(ui, |row| {
                row.horizontal_wrapped(|h| {
                    let drag_lbl = "::";
                    let dr = h.add_enabled(!ds.frozen, egui::Label::new(RichText::new(drag_lbl).color(tokens.text_muted)).sense(egui::Sense::drag()));
                    if dr.drag_started() {
                        state.dragging_level = Some(idx);
                        state.selected_level = Some(item.level.clone());
                    }

                    let title_lbl = if idx == 0 { format!("{} [Ref]", item.level) } else { item.level.clone() };
                    let lvl_r = h.selectable_label(is_selected, &title_lbl);
                    meta.record_modal_response(
                        h,
                        &lvl_r,
                        FrameInspectEntryInput::modal_button(&format!("mv_formula.vars.level.{idx}"), &title_lbl, true),
                    );
                    if lvl_r.clicked() {
                        state.selected_level = Some(item.level.clone());
                        state.synced_level_key = Some(item.level.clone());
                        state.rename_draft = item.level.clone();
                    }

                    h.label(
                        RichText::new(format!("{} incl / {} excl", item.included_count, item.excluded_count))
                            .size(tokens.metrics.font_ui_small_pt)
                            .color(tokens.text_muted),
                    );

                    let up_r = choice_pill(h, tokens, "Up", false, !ds.frozen && idx > 0);
                    meta.record_modal_response(
                        h,
                        &up_r,
                        FrameInspectEntryInput::modal_button(&format!("mv_formula.vars.level_up.{idx}"), "Move Level Up", !ds.frozen && idx > 0),
                    );
                    if up_r.clicked() {
                        commit_reorder = Some((idx, idx.saturating_sub(1)));
                    }

                    let dn_r = choice_pill(h, tokens, "Dn", false, !ds.frozen && idx + 1 < levels.len());
                    meta.record_modal_response(
                        h,
                        &dn_r,
                        FrameInspectEntryInput::modal_button(
                            &format!("mv_formula.vars.level_dn.{idx}"),
                            "Move Level Down",
                            !ds.frozen && idx + 1 < levels.len(),
                        ),
                    );
                    if dn_r.clicked() {
                        commit_reorder = Some((idx, idx + 1));
                    }

                    let ref_r = choice_pill(h, tokens, "Pin Ref", idx == 0, !ds.frozen);
                    meta.record_modal_response(
                        h,
                        &ref_r,
                        FrameInspectEntryInput::modal_button(&format!("mv_formula.vars.level_ref.{idx}"), "Pin Ref", !ds.frozen),
                    );
                    if ref_r.clicked() {
                        intents.push_apply_manage_var_order(
                            ds.id.clone(),
                            col,
                            LevelOrderSpec { order: LevelOrderKind::Explicit(explicit_names.clone()), reversed: false },
                            Some(item.level.clone()),
                        );
                    }
                });

                if is_selected {
                    row.horizontal_wrapped(|edit_row| {
                        edit_row.label("Rename/Merge to:");
                        let er = edit_row.add(egui::TextEdit::singleline(&mut state.rename_draft).hint_text("New level name").desired_width(105.0));
                        meta.record_modal_response(
                            edit_row,
                            &er,
                            FrameInspectEntryInput::modal(
                                &format!("mv_formula.vars.level_rename.{idx}"),
                                "textbox",
                                "Inline Level Rename",
                                Some(state.rename_draft.clone()),
                                !ds.frozen,
                            ),
                        );
                        let trimmed = state.rename_draft.trim();
                        let can_recode = !ds.frozen && !trimmed.is_empty() && trimmed != item.level;
                        let mr = edit_row.add_enabled(can_recode, egui::Button::new("Apply Rename/Merge"));
                        meta.record_modal_response(
                            edit_row,
                            &mr,
                            FrameInspectEntryInput::modal_button(&format!("mv_formula.vars.level_merge.{idx}"), "Apply Rename/Merge", can_recode),
                        );
                        if mr.clicked() || (er.lost_focus() && er.ctx.input(|i| i.key_pressed(egui::Key::Enter)) && can_recode) {
                            intents.push_engine(
                                "table.recodeLevels",
                                json!({ "sheet_id": ds.id.as_str(), "col": col, "mappings": [{ "from": item.level, "to": trimmed }] }),
                            );
                            state.selected_level = Some(trimmed.to_owned());
                            state.synced_level_key = None;
                        }
                    });
                }
            });
        row_rects.push(resp_frame.response.rect);
    }

    // Complete pointer drag-to-reorder on pointer release.
    if let Some(drag_src) = state.dragging_level
        && ui.input(|i| i.pointer.any_released())
    {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            && let Some(dst) = row_rects.iter().position(|r| r.contains(pos))
            && dst != drag_src
        {
            commit_reorder = Some((drag_src, dst));
        }
        state.dragging_level = None;
    }

    if let Some((src, dst)) = commit_reorder
        && src < explicit_names.len()
        && dst < explicit_names.len()
        && src != dst
    {
        let moved = explicit_names.remove(src);
        explicit_names.insert(dst.min(explicit_names.len()), moved);
        intents.push_apply_manage_var_order(
            ds.id.clone(),
            col,
            LevelOrderSpec { order: LevelOrderKind::Explicit(explicit_names), reversed: false },
            None,
        );
    }
}

fn choice_pill(ui: &mut Ui, tokens: &Tokens, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let mut text = RichText::new(label);
    if selected {
        text = text.color(tokens.accent).strong();
    }
    let btn = egui::Button::new(text).selected(selected);
    ui.add_enabled(enabled, btn)
}
