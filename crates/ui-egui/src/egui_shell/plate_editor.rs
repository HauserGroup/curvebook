//! Interactive color-coded `8×12` / `16×24` microplate editor, sheet/AUC value-source picker,
//! dilution-series filler, pasted/sheet grid loader, and `.cbplate` persistence controls (`R17.7`).

use curvebook_doc::{PlateGeometry, PlateWellRole, ResultStatus, TableArchetype, TableKind};
use egui::{Color32, CornerRadius, FontId, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::grid_view::{ImportReshapeMode, PlateMatchWellsKind, PlateSourceReducerKind, PlateValuesSourceMode, parse_localized_number_input};
use crate::i18n::{LocalePreset, tr};

use super::import_wizard::{ImportWizardEguiState, PlateAssignRoleKind, PlateDilutionAxis, PlateDilutionScale, PlateSubTab};
use super::{FrameInspectEntryInput, FrameInspectMeta, OKABE_ITO_SERIES_PALETTE, PendingUiIntents, Tokens, UiIntent, ui_kit, widgets};

/// Parse a JSON role entry stored in `ImportReshapeState::plate_roles` into a typed [`PlateWellRole`].
#[must_use]
pub(crate) fn parse_role_value(val: &Value) -> PlateWellRole {
    if let Some(tag) = val.as_str() {
        return match tag.trim().to_ascii_lowercase().as_str() {
            "blank" => PlateWellRole::Blank,
            "positivecontrol" | "positive_control" | "positive-control" | "positive" => PlateWellRole::PositiveControl,
            _ => PlateWellRole::Excluded,
        };
    }
    if let Ok(role) = serde_json::from_value::<PlateWellRole>(val.clone()) {
        return role;
    }
    if let Some(obj) = val.as_object() {
        if let Some(kind) = obj.get("role").and_then(Value::as_str) {
            match kind.trim().to_ascii_lowercase().as_str() {
                "blank" => return PlateWellRole::Blank,
                "positivecontrol" | "positive_control" | "positive-control" | "positive" => return PlateWellRole::PositiveControl,
                "excluded" | "unused" => return PlateWellRole::Excluded,
                _ => {}
            }
        }
        if let Some(group_name) = obj.get("group_name").and_then(Value::as_str)
            && !group_name.trim().is_empty()
        {
            let dilution_x = obj.get("dilution_x").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
            let replicate = obj.get("replicate").and_then(Value::as_u64).and_then(|u| usize::try_from(u).ok()).unwrap_or(0);
            return PlateWellRole::Treatment { group_name: group_name.to_owned(), dilution_x, replicate };
        }
    }
    PlateWellRole::Excluded
}

/// Convert a [`PlateWellRole`] into the canonical JSON representation accepted by `table.importPlateMap`.
#[must_use]
pub(crate) fn role_to_json(role: &PlateWellRole) -> Value {
    match role {
        PlateWellRole::Blank => Value::String(String::from("Blank")),
        PlateWellRole::PositiveControl => Value::String(String::from("PositiveControl")),
        PlateWellRole::Excluded => Value::String(String::from("Excluded")),
        PlateWellRole::Treatment { group_name, dilution_x, replicate } => json!({
            "role": "treatment",
            "group_name": group_name,
            "dilution_x": dilution_x,
            "replicate": replicate,
        }),
    }
}

/// Materialize a full `geometry.well_count()` vector of [`PlateWellRole`] from `raw_roles`.
#[must_use]
pub(crate) fn materialize_plate_roles(raw_roles: &[Value], geometry: PlateGeometry) -> Vec<PlateWellRole> {
    let expected = geometry.well_count();
    if raw_roles.len() == expected {
        return raw_roles.iter().map(parse_role_value).collect();
    }
    let mut out = vec![PlateWellRole::Excluded; expected];
    for (slot, val) in out.iter_mut().zip(raw_roles.iter()) {
        *slot = parse_role_value(val);
    }
    out
}

/// Re-number treatment replicate indices zero-based per `(group_name, dilution_x)` in row-major order.
fn normalize_treatment_replicates(roles: &mut [PlateWellRole]) {
    let mut counts: std::collections::BTreeMap<(String, u64), usize> = std::collections::BTreeMap::new();
    for role in roles.iter_mut() {
        if let PlateWellRole::Treatment { group_name, dilution_x, replicate } = role {
            let norm_x = if *dilution_x == 0.0 { 0.0 } else { *dilution_x };
            let key = (group_name.trim().to_owned(), norm_x.to_bits());
            let counter = counts.entry(key).or_insert(0);
            *replicate = *counter;
            *counter += 1;
        }
    }
}

fn patch_field(intents: &mut PendingUiIntents, key: &str, value: Value) {
    let mut fields = Map::new();
    fields.insert(key.to_owned(), value);
    intents.push(UiIntent::PatchActiveDialog { fields });
}

fn patch_roles(intents: &mut PendingUiIntents, roles: &[PlateWellRole]) {
    let mut fields = Map::new();
    fields.insert(String::from("plate_roles"), Value::Array(roles.iter().map(role_to_json).collect()));
    fields.insert(String::from("plate_layout_source"), Value::String(String::from("interactive")));
    intents.push(UiIntent::PatchActiveDialog { fields });
}

fn parse_label_csv(raw: &str) -> Vec<String> {
    raw.split([',', ';', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned).collect()
}

fn group_color_index(name: &str) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    let chromatic_len = OKABE_ITO_SERIES_PALETTE.len().saturating_sub(1).max(1);
    1 + usize::try_from(h % (chromatic_len as u64)).unwrap_or(0)
}

fn blend_tint(base: Color32, tint: Color32, alpha: f32) -> Color32 {
    base.lerp_to_gamma(tint, alpha.clamp(0.0, 1.0))
}

fn compact_well_tag(role: &PlateWellRole, well_label: &str) -> String {
    match role {
        PlateWellRole::Excluded => format!("{well_label}\n×"),
        PlateWellRole::Blank => format!("{well_label}\nBLK"),
        PlateWellRole::PositiveControl => format!("{well_label}\nPOS"),
        PlateWellRole::Treatment { group_name, dilution_x, .. } => {
            let short_g: String = group_name.chars().take(5).collect();
            if *dilution_x == 0.0 { format!("{well_label}\n{short_g}") } else { format!("{short_g}\n{dilution_x:.3}") }
        }
    }
}

fn role_tooltip(role: &PlateWellRole, well_label: &str) -> String {
    match role {
        PlateWellRole::Excluded => format!("{well_label}: Excluded"),
        PlateWellRole::Blank => format!("{well_label}: Blank"),
        PlateWellRole::PositiveControl => format!("{well_label}: Positive Control"),
        PlateWellRole::Treatment { group_name, dilution_x, replicate } => {
            format!("{well_label}: {group_name} (X = {dilution_x}, rep {})", replicate + 1)
        }
    }
}

/// Render the interactive `96`-well / `384`-well Plate Map section inside `ImportReshape`.
pub(super) fn render_plate_map_reshape_section(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    avail_w: f32,
) {
    let ir = &app.grid.import_reshape;
    let locale = tokens.locale;
    let is_384 = ir.mode == ImportReshapeMode::Plate384;
    let geom = if is_384 { PlateGeometry::Wells384 } else { PlateGeometry::Wells96 };
    let expected = geom.well_count();

    // Compact 3-tab switcher keeping visible non-gridcell controls well within modal control budgets.
    ui.horizontal_wrapped(|bar| {
        for (tab, wid, key) in [
            (PlateSubTab::Values, "import_plate.subtab.values", "plate.tab.values"),
            (PlateSubTab::Layout, "import_plate.subtab.layout", "plate.tab.layout"),
            (PlateSubTab::GridsIo, "import_plate.subtab.grids_io", "plate.tab.grids_io"),
        ] {
            let active = state.plate_subtab == tab;
            let lbl = tr(locale, key);
            let r = ui_kit::choice_pill(bar, tokens, lbl, active, true);
            meta.record_modal_response(bar, &r, FrameInspectEntryInput::modal(wid, "tab", lbl, Some(active.to_string()), true));
            if r.clicked() {
                state.plate_subtab = tab;
            }
        }
    });
    ui.add_space(4.0);
    widgets::divider(ui);
    ui.add_space(4.0);

    if let Some(msg) = &ir.plate_status_message
        && !msg.trim().is_empty()
    {
        ui.label(RichText::new(msg).small().color(tokens.accent));
        ui.add_space(2.0);
    }

    match state.plate_subtab {
        PlateSubTab::Values => render_plate_values_subtab(ui, app, state, tokens, meta, intents, avail_w, is_384, expected),
        PlateSubTab::Layout => render_plate_layout_subtab(ui, app, state, tokens, meta, intents, avail_w, geom),
        PlateSubTab::GridsIo => render_plate_grids_io_subtab(ui, app, state, tokens, meta, intents, avail_w),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_plate_values_subtab(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    avail_w: f32,
    is_384: bool,
    expected: usize,
) {
    let ir = &app.grid.import_reshape;
    let locale = tokens.locale;

    ui.horizontal_wrapped(|row| {
        row.label(RichText::new(tr(locale, "plate.values_source.label")).strong());
        for (mode, wire, wid, key) in [
            (PlateValuesSourceMode::PasteGrid, "paste", "import_plate.values_src.paste", "plate.values_source.paste"),
            (PlateValuesSourceMode::ExistingSheet, "sheet", "import_plate.values_src.sheet", "plate.values_source.sheet"),
        ] {
            let sel = ir.plate_values_source == mode;
            let lbl = tr(locale, key);
            let r = ui_kit::choice_pill(row, tokens, lbl, sel, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(sel.to_string()), true));
            if r.clicked() {
                let mut f = Map::new();
                f.insert(String::from("plate_values_source"), Value::String(String::from(wire)));
                if mode == PlateValuesSourceMode::ExistingSheet
                    && ir.plate_source_sheet_id.is_none()
                    && let Some((default_id, is_mv)) = first_eligible_plate_source(app)
                {
                    f.insert(String::from("plate_source_sheet_id"), Value::String(default_id));
                    f.insert(String::from("plate_source_reducer"), Value::String(String::from(if is_mv { "value_column" } else { "max" })));
                }
                intents.push(UiIntent::PatchActiveDialog { fields: f });
            }
        }
    });

    ui.add_space(4.0);
    if ir.plate_values_source == PlateValuesSourceMode::PasteGrid {
        ui.label(format!("Expected well count: {expected} (currently {} parsed values)", ir.plate_values.len()));
        let pr = ui.add(
            egui::TextEdit::multiline(&mut state.plate_values_draft)
                .hint_text("Paste well numbers…")
                .desired_rows(4)
                .desired_width((avail_w - 28.0).clamp(180.0, 620.0)),
        );
        meta.record_modal_response(
            ui,
            &pr,
            FrameInspectEntryInput::modal("import_paste.input.plate_values", "textbox", "Plate Values", Some(state.plate_values_draft.clone()), true),
        );
        if pr.changed() {
            let mut f = Map::new();
            f.insert(String::from("plate_values"), Value::String(state.plate_values_draft.clone()));
            if ir.plate_roles.len() != expected {
                f.insert(String::from("plate_roles"), Value::Array(super::import_wizard::default_plate_roles(is_384, ir.subtract_blank_mean)));
            }
            intents.push(UiIntent::PatchActiveDialog { fields: f });
        }
        return;
    }

    // Existing Sheet / AUC Result source controls
    let doc = app.document();
    let candidates = collect_plate_source_candidates(app);
    let selected_label = ir
        .plate_source_sheet_id
        .as_deref()
        .and_then(|sid| candidates.iter().find(|c| c.id == sid).map(|c| c.display.clone()))
        .unwrap_or_else(|| tr(locale, "plate.source_sheet.none").to_owned());

    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.source_sheet.label"));
        let cb_resp = widgets::dropdown("import_plate.source_sheet", &selected_label)
            .width(260.0)
            .show_ui(row, |menu| {
                for cand in &candidates {
                    let is_sel = ir.plate_source_sheet_id.as_deref() == Some(cand.id.as_str());
                    if menu.selectable_label(is_sel, &cand.display).clicked() {
                        let mut f = Map::new();
                        f.insert(String::from("plate_source_sheet_id"), Value::String(cand.id.clone()));
                        if cand.is_mv {
                            f.insert(String::from("plate_source_reducer"), Value::String(String::from("value_column")));
                            if let Some(first_val_col) = cand.mv_numeric_cols.first() {
                                f.insert(String::from("plate_source_value_column"), Value::String(first_val_col.clone()));
                                state.plate_source_val_col_draft = first_val_col.clone();
                            }
                        } else {
                            f.insert(String::from("plate_source_reducer"), Value::String(String::from("max")));
                        }
                        intents.push(UiIntent::PatchActiveDialog { fields: f });
                    }
                }
            })
            .response;
        meta.record_modal_response(
            row,
            &cb_resp,
            FrameInspectEntryInput::modal(
                "import_plate.source_sheet",
                "combobox",
                tr(locale, "plate.source_sheet.label"),
                ir.plate_source_sheet_id.clone(),
                !candidates.is_empty(),
            ),
        );
    });

    if let Some(sid) = ir.plate_source_sheet_id.as_deref()
        && let Some(rs) = doc.result_sheet(&curvebook_doc::SheetId::new(sid))
        && rs.status != ResultStatus::UpToDate
    {
        ui.label(RichText::new(tr(locale, "plate.source_sheet.stale_warn")).small().color(tokens.error));
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.reducer.label"));
        for (kind, wire, wid, key) in [
            (PlateSourceReducerKind::ValueColumn, "value_column", "import_plate.reducer.value_column", "plate.reducer.value_column"),
            (PlateSourceReducerKind::RowIndex, "row_index", "import_plate.reducer.row_index", "plate.reducer.row_index"),
            (PlateSourceReducerKind::XWindowMean, "x_window_mean", "import_plate.reducer.x_window_mean", "plate.reducer.x_window_mean"),
            (PlateSourceReducerKind::Min, "min", "import_plate.reducer.min", "plate.reducer.min"),
            (PlateSourceReducerKind::Max, "max", "import_plate.reducer.max", "plate.reducer.max"),
        ] {
            let sel = ir.plate_source_reducer == kind;
            let lbl = tr(locale, key);
            let r = ui_kit::choice_pill(row, tokens, lbl, sel, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(sel.to_string()), true));
            if r.clicked() {
                patch_field(intents, "plate_source_reducer", Value::String(String::from(wire)));
            }
        }
    });

    ui.add_space(4.0);
    match ir.plate_source_reducer {
        PlateSourceReducerKind::ValueColumn => {
            ui.horizontal_wrapped(|row| {
                row.label(tr(locale, "plate.value_column.label"));
                let ed = row.add(egui::TextEdit::singleline(&mut state.plate_source_val_col_draft).hint_text("Net Area").desired_width(160.0));
                meta.record_modal_response(
                    row,
                    &ed,
                    FrameInspectEntryInput::modal(
                        "import_plate.input.source_value_col",
                        "textbox",
                        tr(locale, "plate.value_column.label"),
                        Some(state.plate_source_val_col_draft.clone()),
                        true,
                    ),
                );
                if ed.changed() {
                    patch_field(intents, "plate_source_value_column", Value::String(state.plate_source_val_col_draft.clone()));
                }
            });
        }
        PlateSourceReducerKind::RowIndex => {
            ui.horizontal_wrapped(|row| {
                row.label(tr(locale, "plate.row_index.label"));
                let ed = row.add(egui::TextEdit::singleline(&mut state.plate_source_row_draft).hint_text("0").desired_width(72.0));
                meta.record_modal_response(
                    row,
                    &ed,
                    FrameInspectEntryInput::modal(
                        "import_plate.input.reducer_row_index",
                        "textbox",
                        tr(locale, "plate.row_index.label"),
                        Some(state.plate_source_row_draft.clone()),
                        true,
                    ),
                );
                if ed.changed()
                    && let Ok(n) = state.plate_source_row_draft.trim().parse::<usize>()
                {
                    patch_field(intents, "plate_source_row_index", json!(n));
                }
            });
        }
        PlateSourceReducerKind::XWindowMean => {
            ui.horizontal_wrapped(|row| {
                row.label(tr(locale, "auc.x_min"));
                let e0 = row.add(egui::TextEdit::singleline(&mut state.plate_source_xmin_draft).hint_text("0").desired_width(76.0));
                meta.record_modal_response(
                    row,
                    &e0,
                    FrameInspectEntryInput::modal(
                        "import_plate.input.reducer_x_min",
                        "textbox",
                        tr(locale, "auc.x_min"),
                        Some(state.plate_source_xmin_draft.clone()),
                        true,
                    ),
                );
                if e0.changed()
                    && let Some(v) = parse_localized_number_input(&state.plate_source_xmin_draft, &app.locale_preset)
                {
                    patch_field(intents, "plate_source_x_min", json!(v));
                }
                row.label(tr(locale, "auc.x_max"));
                let e1 = row.add(egui::TextEdit::singleline(&mut state.plate_source_xmax_draft).hint_text("60").desired_width(76.0));
                meta.record_modal_response(
                    row,
                    &e1,
                    FrameInspectEntryInput::modal(
                        "import_plate.input.reducer_x_max",
                        "textbox",
                        tr(locale, "auc.x_max"),
                        Some(state.plate_source_xmax_draft.clone()),
                        true,
                    ),
                );
                if e1.changed()
                    && let Some(v) = parse_localized_number_input(&state.plate_source_xmax_draft, &app.locale_preset)
                {
                    patch_field(intents, "plate_source_x_max", json!(v));
                }
            });
        }
        PlateSourceReducerKind::Min | PlateSourceReducerKind::Max => {}
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.match_wells.label"));
        for (mw, wire, wid, key) in [
            (PlateMatchWellsKind::ByWellId, "by_well_id", "import_plate.match.by_well_id", "plate.match_wells.by_id"),
            (PlateMatchWellsKind::Positional, "positional", "import_plate.match.positional", "plate.match_wells.positional"),
        ] {
            let sel = ir.plate_match_wells == mw;
            let lbl = tr(locale, key);
            let r = ui_kit::choice_pill(row, tokens, lbl, sel, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(sel.to_string()), true));
            if r.clicked() {
                patch_field(intents, "plate_match_wells", Value::String(String::from(wire)));
            }
        }
    });
}

#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
fn render_plate_layout_subtab(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    avail_w: f32,
    geom: PlateGeometry,
) {
    let ir = &app.grid.import_reshape;
    let locale = tokens.locale;
    let rows = geom.rows();
    let cols = geom.cols();
    let mut roles = materialize_plate_roles(&ir.plate_roles, geom);

    // Clamp selection to active geometry
    if let Some((r, c)) = state.plate_sel_anchor
        && (r >= rows || c >= cols)
    {
        state.plate_sel_anchor = Some((0, 0));
        state.plate_sel_focus = Some((0, 0));
    }
    let sel_rect = selected_plate_rect(state, rows, cols);
    let sel_summary = format_plate_selection_summary(geom, sel_rect);

    // 1. Compact assignment & dilution toolbar
    ui.horizontal_wrapped(|row| {
        row.label(RichText::new(&sel_summary).strong().color(tokens.text_primary));
        let all_btn = widgets::secondary_button(row, tr(locale, "plate.sel.all"));
        meta.record_modal_response(
            row,
            &all_btn,
            FrameInspectEntryInput::modal("import_plate.sel.all", "button", tr(locale, "plate.sel.all"), None, true),
        );
        if all_btn.clicked() {
            state.plate_sel_anchor = Some((0, 0));
            state.plate_sel_focus = Some((rows - 1, cols - 1));
        }

        row.separator();
        for (kind, wid, key) in [
            (PlateAssignRoleKind::Treatment, "import_plate.assign.treatment", "plate.role.treatment"),
            (PlateAssignRoleKind::Blank, "import_plate.assign.blank", "plate.role.blank"),
            (PlateAssignRoleKind::PositiveControl, "import_plate.assign.pos_ctrl", "plate.role.positive_control"),
            (PlateAssignRoleKind::Excluded, "import_plate.assign.excluded", "plate.role.excluded"),
        ] {
            let active = state.plate_assign_role == kind;
            let lbl = tr(locale, key);
            let r = ui_kit::choice_pill(row, tokens, lbl, active, true);
            meta.record_modal_response(row, &r, FrameInspectEntryInput::modal(wid, "radio", lbl, Some(active.to_string()), true));
            if r.clicked() {
                state.plate_assign_role = kind;
                if kind != PlateAssignRoleKind::Treatment
                    && let Some((r0, r1, c0, c1)) = sel_rect
                {
                    let assigned = match kind {
                        PlateAssignRoleKind::Blank => PlateWellRole::Blank,
                        PlateAssignRoleKind::PositiveControl => PlateWellRole::PositiveControl,
                        PlateAssignRoleKind::Excluded | PlateAssignRoleKind::Treatment => PlateWellRole::Excluded,
                    };
                    for rr in r0..=r1 {
                        for cc in c0..=c1 {
                            roles[rr * cols + cc] = assigned.clone();
                        }
                    }
                    normalize_treatment_replicates(&mut roles);
                    patch_roles(intents, &roles);
                }
            }
        }
    });

    if state.plate_assign_role == PlateAssignRoleKind::Treatment {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|row| {
            row.label(tr(locale, "plate.group.label"));
            let g_ed = row.add(egui::TextEdit::singleline(&mut state.plate_group_draft).hint_text("Compound A").desired_width(104.0));
            meta.record_modal_response(
                row,
                &g_ed,
                FrameInspectEntryInput::modal(
                    "import_plate.input.group_name",
                    "textbox",
                    tr(locale, "plate.group.label"),
                    Some(state.plate_group_draft.clone()),
                    true,
                ),
            );
            row.label(tr(locale, "plate.const_x.label"));
            let x_ed = row.add(egui::TextEdit::singleline(&mut state.plate_const_x_draft).hint_text("0").desired_width(64.0));
            meta.record_modal_response(
                row,
                &x_ed,
                FrameInspectEntryInput::modal(
                    "import_plate.input.const_x",
                    "textbox",
                    tr(locale, "plate.const_x.label"),
                    Some(state.plate_const_x_draft.clone()),
                    true,
                ),
            );
            let apply_btn = widgets::secondary_button(row, tr(locale, "plate.btn.apply_role"));
            meta.record_modal_response(
                row,
                &apply_btn,
                FrameInspectEntryInput::modal(
                    "import_plate.btn.apply_treatment",
                    "button",
                    tr(locale, "plate.btn.apply_role"),
                    None,
                    sel_rect.is_some(),
                ),
            );
            if apply_btn.clicked()
                && let Some((r0, r1, c0, c1)) = sel_rect
            {
                let group = state.plate_group_draft.trim();
                let group_name = if group.is_empty() { String::from("Sample") } else { group.to_owned() };
                let x_val = parse_plate_f64(&state.plate_const_x_draft, &app.locale_preset).unwrap_or(0.0);
                for rr in r0..=r1 {
                    for cc in c0..=c1 {
                        roles[rr * cols + cc] = PlateWellRole::Treatment { group_name: group_name.clone(), dilution_x: x_val, replicate: 0 };
                    }
                }
                normalize_treatment_replicates(&mut roles);
                patch_roles(intents, &roles);
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|row| {
            row.label(RichText::new(tr(locale, "plate.dilution.label")).strong());
            for (axis, wid, key) in [
                (PlateDilutionAxis::AlongRows, "import_plate.dilution.dir.row", "plate.dilution.rows"),
                (PlateDilutionAxis::AlongCols, "import_plate.dilution.dir.col", "plate.dilution.cols"),
            ] {
                let lbl = tr(locale, key);
                let r = ui_kit::choice_pill(row, tokens, lbl, state.plate_dilution_axis == axis, true);
                meta.record_modal_response(
                    row,
                    &r,
                    FrameInspectEntryInput::modal(wid, "radio", lbl, Some((state.plate_dilution_axis == axis).to_string()), true),
                );
                if r.clicked() {
                    state.plate_dilution_axis = axis;
                }
            }
            for (scale, wid, key) in [
                (PlateDilutionScale::Linear, "import_plate.dilution.mode.linear", "plate.dilution.linear"),
                (PlateDilutionScale::Multiplicative, "import_plate.dilution.mode.mult", "plate.dilution.multiplicative"),
            ] {
                let lbl = tr(locale, key);
                let r = ui_kit::choice_pill(row, tokens, lbl, state.plate_dilution_scale == scale, true);
                meta.record_modal_response(
                    row,
                    &r,
                    FrameInspectEntryInput::modal(wid, "radio", lbl, Some((state.plate_dilution_scale == scale).to_string()), true),
                );
                if r.clicked() {
                    state.plate_dilution_scale = scale;
                }
            }
            row.label(tr(locale, "plate.dilution.start"));
            let s_ed = row.add(egui::TextEdit::singleline(&mut state.plate_dilution_start_draft).hint_text("-6").desired_width(58.0));
            meta.record_modal_response(
                row,
                &s_ed,
                FrameInspectEntryInput::modal(
                    "import_plate.input.dilution_start",
                    "textbox",
                    tr(locale, "plate.dilution.start"),
                    Some(state.plate_dilution_start_draft.clone()),
                    true,
                ),
            );
            row.label(tr(locale, "plate.dilution.step"));
            let st_ed = row.add(egui::TextEdit::singleline(&mut state.plate_dilution_step_draft).hint_text("-1").desired_width(58.0));
            meta.record_modal_response(
                row,
                &st_ed,
                FrameInspectEntryInput::modal(
                    "import_plate.input.dilution_step",
                    "textbox",
                    tr(locale, "plate.dilution.step"),
                    Some(state.plate_dilution_step_draft.clone()),
                    true,
                ),
            );
            let fill_btn = widgets::secondary_button(row, tr(locale, "plate.btn.fill_dilution"));
            meta.record_modal_response(
                row,
                &fill_btn,
                FrameInspectEntryInput::modal(
                    "import_plate.btn.fill_dilution",
                    "button",
                    tr(locale, "plate.btn.fill_dilution"),
                    None,
                    sel_rect.is_some(),
                ),
            );
            if fill_btn.clicked()
                && let Some((r0, r1, c0, c1)) = sel_rect
                && let (Some(start_x), Some(step_x)) = (
                    parse_plate_f64(&state.plate_dilution_start_draft, &app.locale_preset),
                    parse_plate_f64(&state.plate_dilution_step_draft, &app.locale_preset),
                )
            {
                let group = state.plate_group_draft.trim();
                let group_name = if group.is_empty() { String::from("Sample") } else { group.to_owned() };
                for rr in r0..=r1 {
                    for cc in c0..=c1 {
                        let k = match state.plate_dilution_axis {
                            PlateDilutionAxis::AlongRows => cc - c0,
                            PlateDilutionAxis::AlongCols => rr - r0,
                        };
                        #[allow(clippy::cast_precision_loss)]
                        let x_val = match state.plate_dilution_scale {
                            PlateDilutionScale::Linear => start_x + (k as f64) * step_x,
                            PlateDilutionScale::Multiplicative => {
                                let exp = i32::try_from(k).unwrap_or(i32::MAX);
                                start_x * step_x.powi(exp)
                            }
                        };
                        if x_val.is_finite() {
                            roles[rr * cols + cc] = PlateWellRole::Treatment { group_name: group_name.clone(), dilution_x: x_val, replicate: 0 };
                        }
                    }
                }
                normalize_treatment_replicates(&mut roles);
                patch_roles(intents, &roles);
            }
        });
    }

    ui.add_space(6.0);
    // 2. Interactive color-coded 8x12 / 16x24 plate grid inside a bounded scroll area.
    let cell_w: f32 = if geom == PlateGeometry::Wells384 { 38.0 } else { 54.0 };
    let cell_h: f32 = if geom == PlateGeometry::Wells384 { 24.0 } else { 26.0 };
    let hdr_w: f32 = 24.0;
    let hdr_h: f32 = 24.0;
    let gap: f32 = 2.0;

    let pointer_down = ui.input(|i| i.pointer.primary_down());
    let shift_down = ui.input(|i| i.modifiers.shift);
    let ptr_pos = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()));
    if !pointer_down {
        state.plate_dragging = false;
    }

    egui::ScrollArea::both()
        .id_salt(("plate_map_grid_scroll", rows, cols))
        .max_width((avail_w - 16.0).max(200.0))
        .max_height(260.0)
        .auto_shrink([false, false])
        .show(ui, |grid_ui| {
            let total_w = hdr_w + (cols as f32) * (cell_w + gap);
            let total_h = hdr_h + (rows as f32) * (cell_h + gap);
            let (origin_rect, _) = grid_ui.allocate_exact_size(vec2(total_w, total_h), Sense::hover());
            let ox = origin_rect.min.x;
            let oy = origin_rect.min.y;

            // Column headers (1..=12 / 1..=24) — click selects whole column
            for c in 0..cols {
                let x = ox + hdr_w + (c as f32) * (cell_w + gap);
                let rect = Rect::from_min_size(pos2(x, oy), vec2(cell_w, hdr_h));
                let resp = grid_ui.interact(rect, Id::unique(("plate_col_hdr", c)), Sense::click());
                if grid_ui.is_rect_visible(rect) {
                    let bg = if resp.hovered() { tokens.hover_bg } else { tokens.header_bg };
                    grid_ui.painter().rect_filled(rect, CornerRadius::same(2), bg);
                    grid_ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{:02}", c + 1),
                        FontId::proportional(10.5),
                        tokens.text_muted,
                    );
                }
                if resp.clicked() {
                    state.plate_sel_anchor = Some((0, c));
                    state.plate_sel_focus = Some((rows - 1, c));
                }
            }

            // Row headers (A..H / A..P) — click selects whole row
            for r in 0..rows {
                let y = oy + hdr_h + (r as f32) * (cell_h + gap);
                let rect = Rect::from_min_size(pos2(ox, y), vec2(hdr_w, cell_h));
                let resp = grid_ui.interact(rect, Id::unique(("plate_row_hdr", r)), Sense::click());
                let row_ch = char::from(b'A'.saturating_add(u8::try_from(r).unwrap_or(0)));
                if grid_ui.is_rect_visible(rect) {
                    let bg = if resp.hovered() { tokens.hover_bg } else { tokens.header_bg };
                    grid_ui.painter().rect_filled(rect, CornerRadius::same(2), bg);
                    grid_ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        row_ch.to_string(),
                        FontId::proportional(10.5),
                        tokens.text_muted,
                    );
                }
                if resp.clicked() {
                    state.plate_sel_anchor = Some((r, 0));
                    state.plate_sel_focus = Some((r, cols - 1));
                }
            }

            // Interactive color-coded wells (`role: "gridcell"` for WCAG/budget compliance)
            for r in 0..rows {
                let y = oy + hdr_h + (r as f32) * (cell_h + gap);
                for c in 0..cols {
                    let x = ox + hdr_w + (c as f32) * (cell_w + gap);
                    let rect = Rect::from_min_size(pos2(x, y), vec2(cell_w, cell_h));
                    let well_label = geom.well_label(r, c).unwrap_or_else(|_| format!("R{r}C{c}"));
                    let wid = format!("import_plate.well.{well_label}");
                    let resp = grid_ui.interact(rect, Id::unique(("plate_well", r, c)), Sense::click_and_drag());
                    let role = &roles[r * cols + c];
                    let is_sel = sel_rect.is_some_and(|(r0, r1, c0, c1)| (r0..=r1).contains(&r) && (c0..=c1).contains(&c));

                    if grid_ui.is_rect_visible(rect) {
                        let fill = match role {
                            PlateWellRole::Excluded => tokens.panel_bg,
                            PlateWellRole::Blank => blend_tint(tokens.surface_bg, tokens.text_muted, 0.20),
                            PlateWellRole::PositiveControl => blend_tint(tokens.surface_bg, tokens.link_green, 0.28),
                            PlateWellRole::Treatment { group_name, .. } => {
                                let base_hue = OKABE_ITO_SERIES_PALETTE[group_color_index(group_name)];
                                blend_tint(tokens.surface_bg, base_hue, 0.28)
                            }
                        };
                        let border = if is_sel {
                            Stroke::new(2.0, tokens.accent)
                        } else if resp.hovered() {
                            Stroke::new(1.5, tokens.text_primary)
                        } else {
                            Stroke::new(1.0, tokens.border)
                        };
                        grid_ui.painter().rect_filled(rect, CornerRadius::same(3), fill);
                        grid_ui.painter().rect_stroke(rect, CornerRadius::same(3), border, StrokeKind::Inside);
                        let tag = compact_well_tag(role, &well_label);
                        grid_ui.painter().with_clip_rect(rect.shrink(1.0)).text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            tag,
                            FontId::proportional(if geom == PlateGeometry::Wells384 { 8.5 } else { 9.5 }),
                            if matches!(role, PlateWellRole::Excluded) { tokens.text_muted } else { tokens.text_primary },
                        );
                    }

                    if resp.hovered() {
                        let _ = resp.clone().on_hover_text(role_tooltip(role, &well_label));
                    }
                    if resp.is_pointer_button_down_on() && grid_ui.input(|i| i.pointer.primary_pressed()) {
                        if shift_down && state.plate_sel_anchor.is_some() {
                            state.plate_sel_focus = Some((r, c));
                        } else {
                            state.plate_sel_anchor = Some((r, c));
                            state.plate_sel_focus = Some((r, c));
                            state.plate_dragging = true;
                        }
                    } else if state.plate_dragging
                        && pointer_down
                        && let Some(p) = ptr_pos
                        && rect.contains(p)
                    {
                        state.plate_sel_focus = Some((r, c));
                    } else if resp.clicked() {
                        if shift_down && state.plate_sel_anchor.is_some() {
                            state.plate_sel_focus = Some((r, c));
                        } else {
                            state.plate_sel_anchor = Some((r, c));
                            state.plate_sel_focus = Some((r, c));
                        }
                    }

                    let tip_val = role_tooltip(role, &well_label);
                    meta.record_modal_response(grid_ui, &resp, FrameInspectEntryInput::modal(&wid, "gridcell", &well_label, Some(tip_val), true));
                }
            }
        });
}

fn render_plate_grids_io_subtab(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ImportWizardEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    avail_w: f32,
) {
    let locale = tokens.locale;
    let doc = app.document();

    // Section A: Save / Load .cbplate
    ui.label(RichText::new(tr(locale, "plate.cbplate.section")).strong().color(tokens.text_primary));
    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.cbplate.path"));
        let path_ed = row.add(
            egui::TextEdit::singleline(&mut state.plate_cbplate_path_draft)
                .hint_text("layout.cbplate")
                .desired_width((avail_w - 280.0).clamp(140.0, 340.0)),
        );
        meta.record_modal_response(
            row,
            &path_ed,
            FrameInspectEntryInput::modal(
                "import_plate.input.cbplate_path",
                "textbox",
                tr(locale, "plate.cbplate.path"),
                Some(state.plate_cbplate_path_draft.clone()),
                true,
            ),
        );
        if path_ed.changed() {
            patch_field(intents, "plate_template_path", Value::String(state.plate_cbplate_path_draft.clone()));
        }
        let save_btn = widgets::secondary_button(row, tr(locale, "plate.btn.save_cbplate"));
        meta.record_modal_response(
            row,
            &save_btn,
            FrameInspectEntryInput::modal("import_plate.btn.save_cbplate", "button", tr(locale, "plate.btn.save_cbplate"), None, true),
        );
        if save_btn.clicked() {
            intents.push(UiIntent::SavePlateMapFile { path: state.plate_cbplate_path_draft.clone() });
        }
        let load_btn = widgets::secondary_button(row, tr(locale, "plate.btn.load_cbplate"));
        meta.record_modal_response(
            row,
            &load_btn,
            FrameInspectEntryInput::modal("import_plate.btn.load_cbplate", "button", tr(locale, "plate.btn.load_cbplate"), None, true),
        );
        if load_btn.clicked() {
            intents.push(UiIntent::LoadPlateMapFile { path: state.plate_cbplate_path_draft.clone() });
        }
    });

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);

    // Section B: Control label rules (shared by Load Sheet and Paste Grid)
    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.blank_labels.label"));
        let b_ed = row.add(egui::TextEdit::singleline(&mut state.plate_blank_labels_draft).hint_text("Buffer, Blank").desired_width(140.0));
        meta.record_modal_response(
            row,
            &b_ed,
            FrameInspectEntryInput::modal(
                "import_plate.input.blank_labels",
                "textbox",
                tr(locale, "plate.blank_labels.label"),
                Some(state.plate_blank_labels_draft.clone()),
                true,
            ),
        );
        if b_ed.changed() {
            patch_field(intents, "plate_blank_labels", json!(parse_label_csv(&state.plate_blank_labels_draft)));
        }
        row.label(tr(locale, "plate.pos_ctrl_labels.label"));
        let p_ed = row.add(egui::TextEdit::singleline(&mut state.plate_pos_ctrl_labels_draft).hint_text("PosCtrl").desired_width(140.0));
        meta.record_modal_response(
            row,
            &p_ed,
            FrameInspectEntryInput::modal(
                "import_plate.input.pos_ctrl_labels",
                "textbox",
                tr(locale, "plate.pos_ctrl_labels.label"),
                Some(state.plate_pos_ctrl_labels_draft.clone()),
                true,
            ),
        );
        if p_ed.changed() {
            patch_field(intents, "plate_positive_control_labels", json!(parse_label_csv(&state.plate_pos_ctrl_labels_draft)));
        }
    });

    ui.add_space(6.0);
    // Section C: Load Layout / X Sheet into interactive grid
    ui.label(RichText::new(tr(locale, "plate.load_sheet.section")).strong().color(tokens.text_primary));
    let layout_sel_title = state
        .plate_layout_sheet_draft
        .as_deref()
        .and_then(|sid| doc.data_sheet(&curvebook_doc::SheetId::new(sid)).map(|d| d.title.clone()))
        .unwrap_or_else(|| tr(locale, "plate.source_sheet.none").to_owned());
    let xgrid_sel_title = state
        .plate_x_grid_sheet_draft
        .as_deref()
        .and_then(|sid| doc.data_sheet(&curvebook_doc::SheetId::new(sid)).map(|d| d.title.clone()))
        .unwrap_or_else(|| tr(locale, "plate.x_sheet.none").to_owned());

    ui.horizontal_wrapped(|row| {
        row.label(tr(locale, "plate.layout_sheet.label"));
        let l_cb = widgets::dropdown("import_plate.layout_sheet", &layout_sel_title)
            .width(150.0)
            .show_ui(row, |menu| {
                for ds in &doc.data_sheets {
                    let sel = state.plate_layout_sheet_draft.as_deref() == Some(ds.id.as_str());
                    if menu.selectable_label(sel, &ds.title).clicked() {
                        state.plate_layout_sheet_draft = Some(ds.id.as_str().to_owned());
                        patch_field(intents, "plate_layout_sheet_id", Value::String(ds.id.as_str().to_owned()));
                    }
                }
            })
            .response;
        meta.record_modal_response(
            row,
            &l_cb,
            FrameInspectEntryInput::modal(
                "import_plate.layout_sheet",
                "combobox",
                tr(locale, "plate.layout_sheet.label"),
                state.plate_layout_sheet_draft.clone(),
                !doc.data_sheets.is_empty(),
            ),
        );

        row.label(tr(locale, "plate.x_sheet.label"));
        let x_cb = widgets::dropdown("import_plate.x_grid_sheet", &xgrid_sel_title)
            .width(140.0)
            .show_ui(row, |menu| {
                if menu.selectable_label(state.plate_x_grid_sheet_draft.is_none(), tr(locale, "plate.x_sheet.none")).clicked() {
                    state.plate_x_grid_sheet_draft = None;
                    patch_field(intents, "plate_x_grid_sheet_id", Value::Null);
                }
                for ds in &doc.data_sheets {
                    let sel = state.plate_x_grid_sheet_draft.as_deref() == Some(ds.id.as_str());
                    if menu.selectable_label(sel, &ds.title).clicked() {
                        state.plate_x_grid_sheet_draft = Some(ds.id.as_str().to_owned());
                        patch_field(intents, "plate_x_grid_sheet_id", Value::String(ds.id.as_str().to_owned()));
                    }
                }
            })
            .response;
        meta.record_modal_response(
            row,
            &x_cb,
            FrameInspectEntryInput::modal(
                "import_plate.x_grid_sheet",
                "combobox",
                tr(locale, "plate.x_sheet.label"),
                state.plate_x_grid_sheet_draft.clone(),
                !doc.data_sheets.is_empty(),
            ),
        );

        let load_sh_btn = widgets::secondary_button(row, tr(locale, "plate.btn.load_sheet"));
        let has_layout = state.plate_layout_sheet_draft.is_some();
        meta.record_modal_response(
            row,
            &load_sh_btn,
            FrameInspectEntryInput::modal("import_plate.btn.load_layout_sheet", "button", tr(locale, "plate.btn.load_sheet"), None, has_layout),
        );
        if load_sh_btn.clicked()
            && let Some(lsid) = &state.plate_layout_sheet_draft
        {
            intents.push(UiIntent::LoadPlateLayoutFromSheet {
                layout_sheet_id: curvebook_doc::SheetId::new(lsid.as_str()),
                x_grid_sheet_id: state.plate_x_grid_sheet_draft.as_deref().map(curvebook_doc::SheetId::new),
                blank_labels: parse_label_csv(&state.plate_blank_labels_draft),
                positive_control_labels: parse_label_csv(&state.plate_pos_ctrl_labels_draft),
            });
        }
    });

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);

    // Section D: Paste 8x12 / 16x24 Label Grid & optional X Grid
    ui.label(RichText::new(tr(locale, "plate.paste_grids.section")).strong().color(tokens.text_primary));
    let half_w = ((avail_w - 40.0) * 0.5).clamp(140.0, 340.0);
    ui.horizontal_top(|cols_ui| {
        cols_ui.vertical(|v| {
            v.label(tr(locale, "plate.label_grid.label"));
            let lg = v.add(
                egui::TextEdit::multiline(&mut state.plate_label_grid_draft).hint_text("8×12 or 16×24 labels…").desired_rows(3).desired_width(half_w),
            );
            meta.record_modal_response(
                v,
                &lg,
                FrameInspectEntryInput::modal(
                    "import_plate.input.label_grid",
                    "textbox",
                    tr(locale, "plate.label_grid.label"),
                    Some(state.plate_label_grid_draft.clone()),
                    true,
                ),
            );
        });
        cols_ui.vertical(|v| {
            v.label(tr(locale, "plate.x_grid.label"));
            let xg = v.add(
                egui::TextEdit::multiline(&mut state.plate_x_grid_draft)
                    .hint_text("Optional 8×12 / 16×24 X values…")
                    .desired_rows(3)
                    .desired_width(half_w),
            );
            meta.record_modal_response(
                v,
                &xg,
                FrameInspectEntryInput::modal(
                    "import_plate.input.x_grid",
                    "textbox",
                    tr(locale, "plate.x_grid.label"),
                    Some(state.plate_x_grid_draft.clone()),
                    true,
                ),
            );
        });
    });

    ui.add_space(4.0);
    let apply_grids_btn = widgets::secondary_button(ui, tr(locale, "plate.btn.apply_grids"));
    meta.record_modal_response(
        ui,
        &apply_grids_btn,
        FrameInspectEntryInput::modal(
            "import_plate.btn.apply_grids",
            "button",
            tr(locale, "plate.btn.apply_grids"),
            None,
            !state.plate_label_grid_draft.trim().is_empty(),
        ),
    );
    if apply_grids_btn.clicked() && !state.plate_label_grid_draft.trim().is_empty() {
        intents.push(UiIntent::ApplyPlateGridsText {
            label_grid_text: state.plate_label_grid_draft.clone(),
            x_grid_text: state.plate_x_grid_draft.clone(),
            blank_labels: parse_label_csv(&state.plate_blank_labels_draft),
            positive_control_labels: parse_label_csv(&state.plate_pos_ctrl_labels_draft),
        });
    }
}

struct PlateSourceCandidate {
    id: String,
    display: String,
    is_mv: bool,
    mv_numeric_cols: Vec<String>,
}

fn collect_plate_source_candidates(app: &UiApp) -> Vec<PlateSourceCandidate> {
    let doc = app.document();
    let mut out = Vec::new();
    for rs in &doc.result_sheets {
        if let Some(ds) = rs.analyzable_data.as_ref() {
            let (is_mv, mv_cols) = mv_numeric_column_titles(ds);
            let tag = if rs.status == ResultStatus::UpToDate { "Result" } else { "Stale Result" };
            out.push(PlateSourceCandidate {
                id: rs.id.as_str().to_owned(),
                display: format!("[{tag}] {}", rs.title()),
                is_mv,
                mv_numeric_cols: mv_cols,
            });
        }
    }
    for ds in &doc.data_sheets {
        let (is_mv, mv_cols) = mv_numeric_column_titles(ds);
        out.push(PlateSourceCandidate { id: ds.id.as_str().to_owned(), display: format!("[Data] {}", ds.title), is_mv, mv_numeric_cols: mv_cols });
    }
    out
}

fn first_eligible_plate_source(app: &UiApp) -> Option<(String, bool)> {
    collect_plate_source_candidates(app).into_iter().next().map(|c| (c.id, c.is_mv))
}

fn mv_numeric_column_titles(ds: &curvebook_doc::DataSheet) -> (bool, Vec<String>) {
    if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
        let preferred = ["Net Area", "Positive Area", "Total Area", "Negative Area"];
        let mut cols: Vec<String> = mv
            .variables
            .iter()
            .filter(|v| !v.title.trim().eq_ignore_ascii_case("Series") && !v.title.trim().eq_ignore_ascii_case("Status"))
            .map(|v| v.title.clone())
            .collect();
        if let Some(pos) = cols.iter().position(|c| preferred.iter().any(|p| p.eq_ignore_ascii_case(c.trim()))) {
            let chosen = cols.remove(pos);
            cols.insert(0, chosen);
        }
        (true, cols)
    } else {
        (false, Vec::new())
    }
}

fn selected_plate_rect(state: &ImportWizardEguiState, rows: usize, cols: usize) -> Option<(usize, usize, usize, usize)> {
    let a = state.plate_sel_anchor?;
    let f = state.plate_sel_focus.unwrap_or(a);
    let r0 = a.0.min(f.0).min(rows.saturating_sub(1));
    let r1 = a.0.max(f.0).min(rows.saturating_sub(1));
    let c0 = a.1.min(f.1).min(cols.saturating_sub(1));
    let c1 = a.1.max(f.1).min(cols.saturating_sub(1));
    Some((r0, r1, c0, c1))
}

fn format_plate_selection_summary(geom: PlateGeometry, rect: Option<(usize, usize, usize, usize)>) -> String {
    let Some((r0, r1, c0, c1)) = rect else {
        return String::from("No wells selected");
    };
    let start = geom.well_label(r0, c0).unwrap_or_default();
    let end = geom.well_label(r1, c1).unwrap_or_default();
    let count = (r1 - r0 + 1) * (c1 - c0 + 1);
    if start == end { format!("Selected: {start} (1 well)") } else { format!("Selected: {start}:{end} ({count} wells)") }
}

fn parse_plate_f64(raw: &str, preset: &LocalePreset) -> Option<f64> {
    parse_localized_number_input(raw.trim(), preset).or_else(|| raw.trim().replace(',', ".").parse::<f64>().ok()).filter(|v| v.is_finite())
}

/// Convert a [`TableKind`] into the transform crate's [`curvebook_transform::TargetArchetypeKind`].
#[must_use]
pub(crate) const fn to_target_archetype_kind(kind: TableKind) -> curvebook_transform::TargetArchetypeKind {
    match kind {
        TableKind::Xy => curvebook_transform::TargetArchetypeKind::Xy,
        TableKind::Column => curvebook_transform::TargetArchetypeKind::Column,
        TableKind::MultipleVariables => curvebook_transform::TargetArchetypeKind::MultipleVariables,
        TableKind::Contingency => curvebook_transform::TargetArchetypeKind::Contingency,
        _ => curvebook_transform::TargetArchetypeKind::Grouped,
    }
}
