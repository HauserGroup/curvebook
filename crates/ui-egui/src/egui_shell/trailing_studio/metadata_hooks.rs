//! Metadata & Hooks studio surface (`InfoSheet` typed constant editor + downstream `Referenced By` jump list).

use curvebook_doc::{AnalysisKind, ConstantValue, HookSource, InfoSheet, ParameterScope, ProjectDocument, SheetId, TableArchetype, VariableSource};
use curvebook_transform::{DependencyTarget, extract_dependencies, parse_formula};
use egui::Vec2;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, widgets};
use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;
use crate::inspect::ZLayer;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
use super::{InfoDownstreamRef, TrailingStudioContext, TrailingStudioEguiState};

/// Scan `ProjectDocument` for every downstream sheet referencing constants or notes from `info`.
#[must_use]
pub fn collect_info_downstream_references(doc: &ProjectDocument, info: &InfoSheet) -> Vec<InfoDownstreamRef> {
    let mut refs = Vec::new();
    let info_matches = |hook_sheet: Option<&SheetId>, paired_source: Option<&SheetId>| -> bool {
        if let Some(explicit) = hook_sheet {
            return explicit == &info.id;
        }
        paired_source.is_some_and(|src| info.linked_data_sheets.contains(src)) || doc.info_sheets.first().is_some_and(|p| p.id == info.id)
    };

    for ds_id in &info.linked_data_sheets {
        if let Some(ds) = doc.data_sheet(ds_id) {
            refs.push(InfoDownstreamRef {
                target_sheet_id: ds.id.clone(),
                target_title: ds.title.clone(),
                section_tag: "Data",
                binding_summary: String::from("Paired info table"),
            });
        }
    }

    for ds in &doc.data_sheets {
        // Engine resolves MV `Info(...)` constants strictly from `ds.info_sheet_id` (`AUD-09-20`).
        if ds.info_sheet_id.as_ref() != Some(&info.id) {
            continue;
        }
        if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
            for var in &mv.variables {
                if let VariableSource::Calculated { formula } = &var.source
                    && let Ok(expr) = parse_formula(formula)
                {
                    let info_keys: Vec<String> = extract_dependencies(&expr)
                        .into_iter()
                        .filter_map(|d| match d {
                            DependencyTarget::InfoConstant(k) if info.get_constant(&k).is_some() => Some(k),
                            _ => None,
                        })
                        .collect();
                    if !info_keys.is_empty() {
                        refs.push(InfoDownstreamRef {
                            target_sheet_id: ds.id.clone(),
                            target_title: ds.title.clone(),
                            section_tag: "Data",
                            binding_summary: format!("Formula `{}` uses Info({})", var.title, info_keys.join(", ")),
                        });
                    }
                }
            }
        }
    }

    for rs in &doc.result_sheets {
        if let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind {
            for rule in &nl.parameter_rules {
                if let ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id, constant_name }) = &rule.scope
                    && info_matches(info_sheet_id.as_ref(), rs.analysis.source_sheet_id.as_ref())
                {
                    refs.push(InfoDownstreamRef {
                        target_sheet_id: rs.id.clone(),
                        target_title: rs.title().to_owned(),
                        section_tag: "Result",
                        binding_summary: format!("Constraint `{}` <- {{{{info:{constant_name}}}}}", rule.name),
                    });
                }
            }
        }
    }

    for gs in &doc.graph_sheets {
        for hook in gs.spec.inline_hooks() {
            if let HookSource::InfoConstant { info_sheet_id, constant_name } = hook
                && info_matches(info_sheet_id.as_ref(), Some(&gs.spec.primary_source_sheet_id))
            {
                refs.push(InfoDownstreamRef {
                    target_sheet_id: gs.id.clone(),
                    target_title: gs.title.clone(),
                    section_tag: "Graph",
                    binding_summary: format!("Text/title hook {{{{info:{constant_name}}}}}"),
                });
            }
        }
    }

    for ls in &doc.layout_sheets {
        for hook in ls.spec.inline_hooks() {
            if let HookSource::InfoConstant { info_sheet_id, constant_name } = hook
                && info_matches(info_sheet_id.as_ref(), None)
            {
                refs.push(InfoDownstreamRef {
                    target_sheet_id: ls.id.clone(),
                    target_title: ls.title.clone(),
                    section_tag: "Layout",
                    binding_summary: format!("Callout hook {{{{info:{constant_name}}}}}"),
                });
            }
        }
    }

    refs
}

fn constant_kind_code(val: &ConstantValue) -> u8 {
    match val {
        ConstantValue::Number(_) | ConstantValue::Empty => 0,
        ConstantValue::Text(_) => 1,
        ConstantValue::Date(_) => 2,
    }
}

fn format_localized_studio_constant(val: &ConstantValue, preset: &LocalePreset) -> String {
    match val {
        ConstantValue::Number(v) if v.is_finite() => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            let text = format!("{normalized}");
            if preset.decimal_separator != '.' { text.replace('.', &preset.decimal_separator.to_string()) } else { text }
        }
        _ => val.display_text(),
    }
}

pub(super) fn render_metadata_hooks_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    info: &InfoSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> usize {
    let tokens = ctx_in.tokens;
    let doc = app.document();

    widgets::subheader(ui, "Constants & Hooks");
    if state.info_selected_constant.as_ref().is_none_or(|name| info.get_constant(name).is_none()) {
        state.info_selected_constant = info.constants.first().map(|c| c.name.clone());
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let max_w = ui.available_width();
        for (idx, c) in info.constants.iter().enumerate() {
            let selected = state.info_selected_constant.as_deref().is_some_and(|sel| sel.eq_ignore_ascii_case(&c.name));
            let lbl = format!("{} = {}", c.name, c.value.display_text());
            let wid = format!("inspector.info.const.{idx}");
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: &lbl,
                    tooltip: "",
                    max_w,
                    selected,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.info_selected_constant = Some(c.name.clone());
                state.info_synced_key = None;
            }
        }
    });

    let sel_key = state.info_selected_constant.clone().unwrap_or_default();
    if state.info_synced_key.as_ref().is_none_or(|(sid, nm)| sid != &info.id || !nm.eq_ignore_ascii_case(&sel_key)) {
        state.info_synced_key = Some((info.id.clone(), sel_key.clone()));
        if let Some(c) = info.get_constant(&sel_key) {
            state.info_name_draft = c.name.clone();
            state.info_value_draft = format_localized_studio_constant(&c.value, &app.locale_preset);
            state.info_units_draft = c.units.clone().unwrap_or_default();
            state.info_kind_draft = constant_kind_code(&c.value);
        } else {
            state.info_name_draft.clear();
            state.info_value_draft.clear();
            state.info_units_draft.clear();
            state.info_kind_draft = 0;
        }
    }

    ui.add_space(4.0);
    let name_resp = ui.add_enabled(
        ctx_in.enabled,
        egui::TextEdit::singleline(&mut state.info_name_draft).desired_width(ui.available_width()).hint_text("Constant name"),
    );
    meta.record_response(
        ui,
        &name_resp,
        FrameInspectEntryInput {
            widget_id: "inspector.info.name_input",
            role: "textbox",
            label: "Constant name",
            command_id: None,
            value: Some(json!(state.info_name_draft)),
            enabled: ctx_in.enabled,
            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    let val_resp = ui.add_enabled(
        ctx_in.enabled,
        egui::TextEdit::singleline(&mut state.info_value_draft).desired_width(ui.available_width()).hint_text("Value (number / text / YYYY-MM-DD)"),
    );
    meta.record_response(
        ui,
        &val_resp,
        FrameInspectEntryInput {
            widget_id: "inspector.info.value_input",
            role: "textbox",
            label: "Constant value",
            command_id: None,
            value: Some(json!(state.info_value_draft)),
            enabled: ctx_in.enabled,
            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    let units_resp = ui.add_enabled(
        ctx_in.enabled,
        egui::TextEdit::singleline(&mut state.info_units_draft).desired_width(ui.available_width()).hint_text("Units (optional)"),
    );
    meta.record_response(
        ui,
        &units_resp,
        FrameInspectEntryInput {
            widget_id: "inspector.info.units_input",
            role: "textbox",
            label: "Units",
            command_id: None,
            value: Some(json!(state.info_units_draft)),
            enabled: ctx_in.enabled,
            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (idx, kind_lbl) in [(0u8, "Num"), (1, "Text"), (2, "Date")] {
            let wid = format!("inspector.info.kind.{idx}");
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: kind_lbl,
                    tooltip: "Constant value kind",
                    max_w: 52.0,
                    selected: state.info_kind_draft == idx,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.info_kind_draft = idx;
            }
        }
    });

    let name_ok = !state.info_name_draft.trim().is_empty();
    let raw_val = state.info_value_draft.trim();
    let parsed_num = (state.info_kind_draft == 0 && !raw_val.is_empty()).then(|| parse_localized_number_input(raw_val, &app.locale_preset)).flatten();
    let num_ok = state.info_kind_draft != 0 || raw_val.is_empty() || parsed_num.is_some();
    let date_ok = state.info_kind_draft != 2 || raw_val.is_empty() || crate::egui_shell::info_view::is_valid_iso_date(raw_val);
    let value_ok = num_ok && date_ok;
    if !value_ok {
        let err_lbl = if !date_ok { "Expected ISO date YYYY-MM-DD" } else { "Expected finite number" };
        let err_resp = ui.colored_label(tokens.link_red, err_lbl);
        meta.record_response(
            ui,
            &err_resp,
            FrameInspectEntryInput {
                widget_id: "inspector.info.value_error",
                role: "text",
                label: err_lbl,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    let save_disabled_reason = if !name_ok {
        Some("constant name cannot be empty")
    } else if !date_ok {
        Some("constant date must match YYYY-MM-DD")
    } else if !value_ok {
        Some("constant numeric value must be a finite number")
    } else {
        None
    };
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Save Constant",
            tooltip: "Store or update constant on InfoSheet (info.setConstant)",
            selected: false,
            enabled: ctx_in.enabled && name_ok && value_ok,
            primary: true,
            widget_id: "inspector.info.save_const",
            command_id: Some("info.setConstant"),
            disabled_reason: save_disabled_reason,
        },
    ) {
        let kind_str = match state.info_kind_draft {
            1 => "text",
            2 => "date",
            _ => "number",
        };
        let val_json = match state.info_kind_draft {
            0 | 2 if raw_val.is_empty() => serde_json::Value::Null,
            0 => parsed_num.map_or_else(|| json!(raw_val), |n| json!(n)),
            _ => json!(raw_val),
        };
        let units_val = if state.info_units_draft.trim().is_empty() { serde_json::Value::Null } else { json!(state.info_units_draft.trim()) };
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("sheet_id"), json!(info.id.as_str()));
        payload.insert(String::from("name"), json!(state.info_name_draft.trim()));
        payload.insert(String::from("value"), val_json);
        payload.insert(String::from("units"), units_val);
        if !(state.info_kind_draft == 2 && raw_val.is_empty()) {
            payload.insert(String::from("kind"), json!(kind_str));
        }
        intents.push_engine("info.setConstant", serde_json::Value::Object(payload));
    }

    if let Some(existing_name) = state.info_selected_constant.clone()
        && info.get_constant(&existing_name).is_some()
        && state.info_name_draft.trim().eq_ignore_ascii_case(&existing_name)
    {
        let remove_lbl = format!("Remove \"{existing_name}\"");
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: &remove_lbl,
                tooltip: "Delete selected constant (info.removeConstant)",
                selected: false,
                enabled: ctx_in.enabled,
                primary: false,
                widget_id: "inspector.info.remove_const",
                command_id: Some("info.removeConstant"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("info.removeConstant", json!({ "sheet_id": info.id.as_str(), "name": existing_name }));
        }
    }

    ui.add_space(6.0);
    widgets::subheader(ui, "Referenced By");
    let refs = collect_info_downstream_references(doc, info);
    let ref_count = refs.len();
    if refs.is_empty() {
        widgets::dim_label(ui, "No downstream sheets hook this InfoSheet yet.");
    } else {
        for (idx, item) in refs.iter().enumerate() {
            let lbl = format!("[{}] {} — {}", item.section_tag, item.target_title, item.binding_summary);
            let wid = format!("inspector.info.ref.{idx}");
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: &lbl,
                    tooltip: "Jump to downstream referencing sheet",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: &wid,
                    command_id: Some("sheet.select"),
                    disabled_reason: None,
                },
            ) {
                intents.push_engine("sheet.select", json!({ "sheet_id": item.target_sheet_id.as_str() }));
            }
        }
    }

    if let Some(first_graph) = doc.graph_sheets.first() {
        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: "Insert Model / Hook Box",
                tooltip: "Insert hooked equation or callout onto active graph",
                selected: false,
                enabled: ctx_in.enabled,
                primary: false,
                widget_id: "inspector.info.open_hook_browser",
                command_id: Some("ui.graph.insertModelEquation"),
                disabled_reason: None,
            },
        ) {
            intents.push_ui("ui.graph.insertModelEquation", json!({ "sheet_id": first_graph.id.as_str() }));
        }
    }

    ref_count
}
