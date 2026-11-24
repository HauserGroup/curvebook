//! Transactional `serde_json`-based programmatic dialog patching (`F5`).
//!
//! Replaces per-dialog manual field-patch ladders with a 4-stage pipeline:
//! (1) key-alias & unknown-key validation + locale/enum normalization,
//! (2) `serde_json::to_value` -> recursive/atomic merge -> `serde_json::from_value`,
//! (3) priority/contextual variant routing, and
//! (4) per-dialog post-merge invariant hooks.
//!
//! Strictly `wasm32`-clean and `egui`-free.

mod analyze_edit;
mod engine;
mod graph_layout;
mod r9_misc;
mod value_parsers;

use curvebook_doc::{ErrorCode, ProjectDocument, SheetId};
use curvebook_engine::EngineError;
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::modal_predicates::{r1_paired_or_matched_allowed, r2_assume_gaussian_allowed};

pub use analyze_edit::{patch_analyze, patch_edit_params};
pub use graph_layout::{
    patch_apply_style, patch_bubble_plot, patch_change_graph_type, patch_create_graph, patch_equalize_scaling, patch_export_figure,
    patch_format_axes, patch_format_embedded_table, patch_format_graph, patch_format_image, patch_format_selected_points, patch_page_layout,
    patch_pairwise,
};
pub use r9_misc::{
    patch_checkpoints, patch_command_palette, patch_convert_archetype, patch_exclude_with_reason, patch_hook_picker, patch_import_reshape,
    patch_insert_model_equation, patch_power_wizard, patch_print, patch_random_sample_rows, patch_randomize_groups, patch_simulate_distributions,
};

fn bad_input(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, msg)
}

fn disabled_err(reason: impl Into<String>) -> EngineError {
    let msg = reason.into();
    EngineError::with_code(ErrorCode::Conflict, msg.clone())
        .with_remediation("Satisfy the control precondition or select a compatible mode/sheet first.")
        .with_details(json!({ "disabled": true, "disabled_reason": msg }))
}

fn map_opt_bool(fields: &Map<String, Value>, keys: &[&str]) -> Result<Option<bool>, EngineError> {
    for &k in keys {
        if let Some(v) = fields.get(k) {
            let b = v.as_bool().ok_or_else(|| bad_input(format!("`{k}` must be a boolean")))?;
            return Ok(Some(b));
        }
    }
    Ok(None)
}

fn map_opt_str<'a>(fields: &'a Map<String, Value>, keys: &[&str]) -> Result<Option<&'a str>, EngineError> {
    for &k in keys {
        if let Some(v) = fields.get(k) {
            let s = v.as_str().ok_or_else(|| bad_input(format!("`{k}` must be a string")))?;
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// Validate pre-summarized table restrictions on `AnalyzeSheet` / `EditAnalysisParameters` patches.
pub(crate) fn validate_pre_summarized_patch(doc: &ProjectDocument, src: Option<&SheetId>, fields: &Map<String, Value>) -> Result<(), EngineError> {
    let wants_paired = map_opt_bool(fields, &["paired_or_matched", "edit.paired_or_matched", "matched"])? == Some(true);
    let two_method_norm = map_opt_str(fields, &["two_group_method", "edit.two_group.method", "analyze.two_group.method"])?
        .map(|s| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect::<String>());
    let wants_paired_method = two_method_norm
        .as_deref()
        .is_some_and(|m| matches!(m, "pairedt" | "ratiopairedt" | "wilcoxon" | "wilcoxonsignedrank" | "pairedsigntest" | "signtest"));
    if wants_paired || wants_paired_method {
        let gate = r1_paired_or_matched_allowed(doc, src);
        if !gate.enabled {
            return Err(disabled_err(gate.disabled_reason.unwrap_or_else(|| String::from("Matched / Repeated Measures requires equal N >= 2"))));
        }
    }
    let wants_nonparam = map_opt_bool(fields, &["assume_gaussian", "edit.assume_gaussian"])? == Some(false)
        || two_method_norm.as_deref().is_some_and(|m| {
            matches!(
                m,
                "mannwhitney" | "mannwhitneyu" | "wilcoxon" | "wilcoxonsignedrank" | "pairedsigntest" | "signtest" | "ks" | "kolmogorovsmirnov"
            )
        });
    if wants_nonparam {
        let gate = r2_assume_gaussian_allowed(doc, src);
        if !gate.enabled {
            return Err(disabled_err(
                gate.disabled_reason.unwrap_or_else(|| String::from("Nonparametric rank tests are disabled for pre-summarized Column tables.")),
            ));
        }
    }
    Ok(())
}

fn is_bad_input_patch_err(msg: &str) -> bool {
    msg.starts_with("unknown ")
        || msg.starts_with("unsupported ")
        || msg.starts_with("conflicting ")
        || msg.starts_with("invalid parameter ")
        || msg.starts_with("invalid power wizard stage index")
        || msg.starts_with("invalid dialog patch")
        || (msg.starts_with('`') && msg.contains(" must be "))
        || msg.contains("must specify")
        || msg.contains("cannot be empty")
}

fn patch_str_err(msg: String) -> EngineError {
    if is_bad_input_patch_err(&msg) { bad_input(msg) } else { disabled_err(msg) }
}

/// Apply a field patch map to whichever modal dialog is currently active on `app`.
pub(crate) fn patch_active_dialog(app: &mut UiApp, fields: &Map<String, Value>) -> Result<(), EngineError> {
    let preset = app.locale_preset.clone();
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => patch_power_wizard(&mut app.canvas_dialogs.power_wizard, fields, &preset).map_err(patch_str_err)?,
        ActiveDialog::FormatSelectedPoints { .. } => {
            patch_format_selected_points(&mut app.canvas_dialogs.format_points, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::AnalyzeSheet { .. } => {
            let src = app.canvas_dialogs.analyze.source_sheet_id.as_ref();
            validate_pre_summarized_patch(app.document(), src, fields)?;
            patch_analyze(&mut app.canvas_dialogs.analyze, fields, &preset).map_err(patch_str_err)?;
        }
        ActiveDialog::EditAnalysisParameters { .. } => {
            let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
            validate_pre_summarized_patch(app.document(), src, fields)?;
            patch_edit_params(&mut app.canvas_dialogs.edit_params, fields, &preset).map_err(patch_str_err)?;
            if let Some(curvebook_doc::AnalysisSpec { kind: curvebook_doc::AnalysisKind::PowerAnalysis(pa), .. }) =
                &app.canvas_dialogs.edit_params.draft
            {
                app.canvas_dialogs.power_wizard.sync_from_doc_spec(pa);
            }
        }
        ActiveDialog::ChangeDataAnalyzed { .. } => {
            let mut working = app.canvas_dialogs.change_data.clone();
            for (key, val) in fields {
                match key.as_str() {
                    "target_sheet_id" | "source_sheet_id" | "change_data.target_sheet_id" => {
                        if !val.is_null() {
                            let sid = val.as_str().ok_or_else(|| bad_input(format!("`{key}` must be a string")))?;
                            let parsed = SheetId::try_new(sid).map_err(|e| bad_input(e.to_string()))?;
                            working.select_target(&parsed).map_err(disabled_err)?;
                        }
                    }
                    "selected_data_sets" | "change_data.selected_data_sets" => {
                        let arr = val.as_array().ok_or_else(|| bad_input(format!("`{key}` must be an array of indices")))?;
                        let mut indices = Vec::with_capacity(arr.len());
                        for (i, v) in arr.iter().enumerate() {
                            let raw = v.as_u64().ok_or_else(|| bad_input(format!("`{key}[{i}]` must be a non-negative integer")))?;
                            let idx = usize::try_from(raw).map_err(|_| bad_input(format!("`{key}[{i}]` exceeds usize")))?;
                            indices.push(idx);
                        }
                        working.selected_data_sets = indices;
                    }
                    other => {
                        if let Some(col_idx) = other.strip_prefix("change_data.col.").and_then(|s| s.parse::<usize>().ok()) {
                            let enabled = val.as_bool().ok_or_else(|| bad_input(format!("`{key}` must be a boolean")))?;
                            if working.selected_data_sets.is_empty()
                                && let Some(target_id) = working.selected_target_id.as_ref()
                                && let Some(ds) = app.document().data_sheet(target_id)
                            {
                                let total = match &ds.archetype {
                                    curvebook_doc::TableArchetype::Column(t) => t.data_sets.len(),
                                    curvebook_doc::TableArchetype::Grouped(t) => t.data_sets.len(),
                                    curvebook_doc::TableArchetype::Xy(t) => t.data_sets.len(),
                                    curvebook_doc::TableArchetype::Contingency(t) => t.data_sets.len(),
                                    curvebook_doc::TableArchetype::Survival(t) => t.cohorts.len(),
                                    curvebook_doc::TableArchetype::MultipleVariables(t) => t.variables.len(),
                                    _ => 0,
                                };
                                working.selected_data_sets = (0..total).collect();
                            }
                            if enabled {
                                if !working.selected_data_sets.contains(&col_idx) {
                                    working.selected_data_sets.push(col_idx);
                                    working.selected_data_sets.sort_unstable();
                                }
                            } else {
                                working.selected_data_sets.retain(|&c| c != col_idx);
                            }
                        } else {
                            return Err(bad_input(format!("unknown ChangeDataAnalyzed field `{other}`")));
                        }
                    }
                }
            }
            app.canvas_dialogs.change_data = working;
        }
        ActiveDialog::ChangeGraphType { .. } => {
            patch_change_graph_type(&mut app.canvas_dialogs.change_graph_type, fields).map_err(bad_input)?;
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            patch_exclude_with_reason(&mut app.canvas_dialogs.exclude_with_reason, fields).map_err(bad_input)?;
        }
        ActiveDialog::ImportReshape => {
            patch_import_reshape(&mut app.grid.import_reshape, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::ConvertArchetype { .. } => {
            let mut working = app.canvas_dialogs.convert_archetype.clone();
            let prev_target = working.target_kind;
            patch_convert_archetype(&mut working, fields).map_err(bad_input)?;
            let target_changed = working.target_kind != prev_target;
            if (target_changed || !working.preview_ready)
                && let Some(intent) = working.preview_intent()
            {
                let out = app.execute_session_ui_cmd(&intent.command_id, intent.params)?;
                working.apply_preview_payload(&out.payload);
            }
            app.canvas_dialogs.convert_archetype = working;
        }
        ActiveDialog::CommandPalette => {
            patch_command_palette(&mut app.canvas_dialogs.command_palette, fields).map_err(bad_input)?;
            #[cfg(not(target_arch = "wasm32"))]
            {
                app.egui_views.command_palette.query.clone_from(&app.canvas_dialogs.command_palette.query);
                app.egui_views.command_palette.selected_index = app.canvas_dialogs.command_palette.selected_index;
            }
        }
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => {
            let first_existing = existing_result_ids.first().map(|id| id.as_str().to_owned());
            let mut choice = app.canvas_dialogs.analyze.duplicate_choice;
            let mut target_id = app.canvas_dialogs.analyze.duplicate_target_id.clone();
            for (key, val) in fields {
                match key.as_str() {
                    "choice" | "duplicate.choice" => {
                        let s = val.as_str().ok_or_else(|| bad_input(format!("`{key}` must be a string")))?;
                        if s.eq_ignore_ascii_case("create_new") {
                            choice = crate::dialogs_canvas::DuplicateChoice::CreateNew;
                        } else if s.eq_ignore_ascii_case("modify_existing") {
                            choice = crate::dialogs_canvas::DuplicateChoice::ModifyExisting;
                            if target_id.is_none() {
                                target_id.clone_from(&first_existing);
                            }
                        } else {
                            return Err(bad_input(format!("unsupported duplicate analysis choice `{s}`; expected create_new or modify_existing")));
                        }
                    }
                    other => return Err(bad_input(format!("unknown DuplicateAnalysisPrompt field `{other}`"))),
                }
            }
            app.canvas_dialogs.analyze.duplicate_choice = choice;
            app.canvas_dialogs.analyze.duplicate_target_id = target_id;
        }
        ActiveDialog::ConstantHookPicker { .. } => {
            patch_hook_picker(&mut app.canvas_dialogs.hook_picker, fields).map_err(bad_input)?;
        }
        ActiveDialog::FormatGraph { .. } => {
            patch_format_graph(&mut app.canvas_dialogs.format_graph, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::FormatAxes { .. } => {
            let doc = app.document().clone();
            patch_format_axes(&mut app.canvas_dialogs.format_axes, fields, Some(&doc), &preset).map_err(bad_input)?;
        }
        ActiveDialog::PairwiseComparisons { .. } => {
            let doc = app.document().clone();
            patch_pairwise(&mut app.canvas_dialogs.pairwise, fields, Some(&doc), &preset).map_err(bad_input)?;
        }
        ActiveDialog::BubblePlotInspector { .. } => {
            patch_bubble_plot(&mut app.canvas_dialogs.bubble_plot, fields).map_err(bad_input)?;
        }
        ActiveDialog::ApplyStyleFrom { .. } => {
            patch_apply_style(&mut app.canvas_dialogs.apply_style, fields).map_err(bad_input)?;
        }
        ActiveDialog::PageLayoutComposer { .. } => {
            let doc = app.document().clone();
            patch_page_layout(&mut app.canvas_dialogs.page_layout, fields, Some(&doc)).map_err(bad_input)?;
        }
        ActiveDialog::ExportFigure { .. } => {
            patch_export_figure(&mut app.canvas_dialogs.export_figure, fields).map_err(bad_input)?;
        }
        ActiveDialog::RandomizeGroups => {
            patch_randomize_groups(&mut app.canvas_dialogs.randomize_groups, fields).map_err(bad_input)?;
        }
        ActiveDialog::RandomSampleRows { .. } => {
            patch_random_sample_rows(&mut app.canvas_dialogs.random_sample_rows, fields).map_err(bad_input)?;
        }
        ActiveDialog::SimulateDistributions => {
            patch_simulate_distributions(&mut app.canvas_dialogs.simulate_distributions, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::InsertModelEquation { .. } => {
            patch_insert_model_equation(&mut app.canvas_dialogs.insert_model_equation, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::CreateGraph { .. } => {
            patch_create_graph(&mut app.canvas_dialogs.create_graph, fields).map_err(bad_input)?;
        }
        ActiveDialog::EqualizeScaling { .. } => {
            patch_equalize_scaling(&mut app.canvas_dialogs.equalize_scaling, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::FormatEmbeddedTable { .. } => {
            patch_format_embedded_table(&mut app.canvas_dialogs.format_embedded_table, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::FormatImage { .. } => {
            patch_format_image(&mut app.canvas_dialogs.format_image, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::Print { .. } => {
            if map_opt_bool(fields, &["submit_to_os", "print.submit_to_os"])? == Some(true) && app.is_gui_only() {
                return Err(EngineError::with_code(
                    ErrorCode::GuiOnlyViolation,
                    "submitting print jobs directly to the OS queue is blocked under --gui-only",
                )
                .with_remediation("Keep `submit_to_os` false under --gui-only to spool in-memory PDF safely."));
            }
            patch_print(&mut app.canvas_dialogs.print, fields, &preset).map_err(bad_input)?;
        }
        ActiveDialog::Checkpoints => {
            patch_checkpoints(&mut app.canvas_dialogs.checkpoints, fields).map_err(bad_input)?;
        }
        ActiveDialog::ResultViewer { .. }
        | ActiveDialog::About
        | ActiveDialog::Feedback
        | ActiveDialog::ImportWarnings
        | ActiveDialog::ImportProjectMode { .. }
        | ActiveDialog::CsvExportOptions(_)
        | ActiveDialog::None => {
            if !fields.is_empty() {
                return Err(bad_input("active dialog does not accept field patches"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, EquationDisplayMode, MultipleVariablesTable, MultivariatePlotMode, MultivariateSpec, OneWayAnovaSpec,
        OneWayMethod, PValueFormat, PairwiseDisplayMode, SheetId, TableArchetype, TableKind, TwoGroupMethod, TwoGroupSpec,
    };
    use curvebook_stats::power::PowerTestFamily;
    use serde_json::{Map, Value, json};

    use super::{
        patch_active_dialog, patch_analyze, patch_bubble_plot, patch_edit_params, patch_equalize_scaling, patch_format_axes,
        patch_format_embedded_table, patch_format_graph, patch_format_image, patch_page_layout, patch_pairwise, patch_power_wizard,
        patch_randomize_groups, patch_simulate_distributions,
    };
    use crate::UiApp;
    use crate::dialogs_canvas::{
        ActiveDialog, AnalyzeDialogState, AxisTarget, BubblePlotInspectorState, DuplicateChoice, EditAnalysisParametersState,
        EqualizeScalingDialogState, FormatAxesState, FormatEmbeddedTableDialogState, FormatGraphState, FormatImageDialogState,
        PageLayoutComposerState, PairwiseComparisonsState,
    };
    use crate::dialogs_r9::{RandomizeGroupsDialogState, SimulateDistributionsDialogState};
    use crate::i18n::LocalePreset;
    use crate::power_wizard::PowerWizardState;

    fn map_from(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn edit_state_with_kind(kind: AnalysisKind) -> Result<EditAnalysisParametersState, String> {
        let mut ep = EditAnalysisParametersState {
            draft: Some(AnalysisSpec::new("res-1", "ds-1", kind).map_err(|e| e.to_string())?),
            ..EditAnalysisParametersState::default()
        };
        ep.sync_rule_flags_from_kind();
        Ok(ep)
    }

    #[test]
    fn reject_unknown_keys_and_roll_back_partial_updates() {
        let preset = LocalePreset::default();
        let mut fg = FormatGraphState::default();
        let orig_legend = fg.show_legend;
        let orig_marker = fg.marker_size_pt;

        let mut bad_patch = Map::new();
        bad_patch.insert(String::from("show_legend"), json!(!orig_legend));
        bad_patch.insert(String::from("nonexistent_field"), json!(true));
        let err = patch_format_graph(&mut fg, &bad_patch, &preset);
        assert!(err.is_err());
        assert_eq!(fg.show_legend, orig_legend);
        assert!((fg.marker_size_pt - orig_marker).abs() < 1e-12);
    }

    #[test]
    fn power_wizard_rolls_back_family_change_on_later_validation_error() {
        let preset = LocalePreset::default();
        let mut wizard = PowerWizardState::default();
        let orig_family = wizard.family;
        assert_ne!(orig_family, PowerTestFamily::OneWayAnova);

        let mut bad_patch = Map::new();
        bad_patch.insert(String::from("family"), json!("one_way_anova"));
        bad_patch.insert(String::from("anova_groups"), json!("not_a_number"));
        assert!(patch_power_wizard(&mut wizard, &bad_patch, &preset).is_err());
        assert_eq!(wizard.family, orig_family);
    }

    #[test]
    fn analyze_patch_preserves_skipped_explicit_response_column() -> Result<(), String> {
        let preset = LocalePreset::default();
        let mv_table = MultipleVariablesTable::new(3, 4).map_err(|e| e.to_string())?;
        let ds = curvebook_doc::DataSheet::new("mv-1", "MV", TableArchetype::MultipleVariables(mv_table)).map_err(|e| e.to_string())?;
        let mut analyze = AnalyzeDialogState::default();
        analyze.open_for_data_sheet(&ds);
        analyze.set_response_column(2);
        assert_eq!(analyze.explicit_response_column, Some(2));

        let mut patch = Map::new();
        patch.insert(String::from("search"), json!("regression"));
        patch_analyze(&mut analyze, &patch, &preset)?;
        assert_eq!(analyze.search_query, "regression");
        assert_eq!(analyze.explicit_response_column, Some(2));
        Ok(())
    }

    #[test]
    fn challenge1_hoists_p_value_format_two_group_method_and_reml_in_edit_params() -> Result<(), String> {
        let preset = LocalePreset::default();

        // p_value_format before p_value_decimals / edit.pvalue.digits
        let mut ep_pval = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        patch_edit_params(&mut ep_pval, &map_from(json!({ "p_value_format": "custom", "p_value_decimals": 2 })), &preset)?;
        assert_eq!(ep_pval.draft.as_ref().map(|d| d.p_value_format), Some(PValueFormat::new_custom(2, None)));

        let mut ep_pval_alias = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        patch_edit_params(&mut ep_pval_alias, &map_from(json!({ "edit.p_value_format": "custom", "edit.pvalue.digits": 3 })), &preset)?;
        assert_eq!(ep_pval_alias.draft.as_ref().map(|d| d.p_value_format), Some(PValueFormat::new_custom(3, None)));

        // two_group_method before edit.two_group.welch
        let mut ep_tg =
            edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }))?;
        patch_edit_params(&mut ep_tg, &map_from(json!({ "two_group_method": "unpaired_t", "edit.two_group.welch": true })), &preset)?;
        let Some(AnalysisKind::TwoGroupComparison(tg)) = ep_tg.draft.as_ref().map(|d| &d.kind) else {
            return Err(String::from("expected TwoGroupComparison"));
        };
        assert_eq!(tg.method, TwoGroupMethod::WelchT);

        // mixed_effects_reml: false before geisser_greenhouse: true
        let mut ep_reml = edit_state_with_kind(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
            mixed_effects_reml: true,
            ..OneWayAnovaSpec::default()
        }))?;
        patch_edit_params(&mut ep_reml, &map_from(json!({ "mixed_effects_reml": false, "geisser_greenhouse": true })), &preset)?;
        let Some(AnalysisKind::OneWayAnova(anova)) = ep_reml.draft.as_ref().map(|d| &d.kind) else {
            return Err(String::from("expected OneWayAnova"));
        };
        assert!(!anova.mixed_effects_reml);
        assert_eq!(anova.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: true });
        Ok(())
    }

    #[test]
    fn challenge1_correlation_heatmap_order_independent_error_and_enable() -> Result<(), String> {
        let preset = LocalePreset::default();

        // AnalyzeDialogState: reject heatmap=true + all_pairs=false in both key orders
        let mut az = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.corr_matrix")), ..AnalyzeDialogState::default() };
        let err_widget_order =
            patch_analyze(&mut az, &map_from(json!({ "analyze.correlation.heatmap": true, "analyze.correlation_all_pairs": false })), &preset)
                .expect_err("must reject heatmap when all_pairs is false");
        assert!(err_widget_order.contains("Create correlation heatmap is disabled in X vs every Y mode."));
        assert!(az.correlation_all_pairs);
        assert!(!az.correlation_heatmap);

        let err_plain_order = patch_analyze(&mut az, &map_from(json!({ "correlation_all_pairs": false, "correlation_heatmap": true })), &preset)
            .expect_err("must reject heatmap when all_pairs is false");
        assert!(err_plain_order.contains("Create correlation heatmap is disabled in X vs every Y mode."));

        az.correlation_all_pairs = false;
        patch_analyze(&mut az, &map_from(json!({ "analyze.correlation.heatmap": true, "analyze.correlation_all_pairs": true })), &preset)?;
        assert!(az.correlation_all_pairs);
        assert!(az.correlation_heatmap);

        // EditAnalysisParametersState: reject heatmap=true + all_pairs=false in both key orders
        let mut ep = edit_state_with_kind(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }))?;
        let err_edit_widget =
            patch_edit_params(&mut ep, &map_from(json!({ "edit.correlation.heatmap": true, "edit.correlation_all_pairs": false })), &preset)
                .expect_err("must reject edit heatmap when all_pairs is false");
        assert!(err_edit_widget.contains("Create correlation heatmap is disabled in X vs every Y mode."));
        assert!(ep.correlation_all_pairs);
        assert!(!ep.correlation_heatmap);

        let err_edit_plain = patch_edit_params(&mut ep, &map_from(json!({ "correlation_all_pairs": false, "correlation_heatmap": true })), &preset)
            .expect_err("must reject edit heatmap when all_pairs is false");
        assert!(err_edit_plain.contains("Create correlation heatmap is disabled in X vs every Y mode."));

        ep.correlation_all_pairs = false;
        patch_edit_params(&mut ep, &map_from(json!({ "edit.correlation.heatmap": true, "edit.correlation_all_pairs": true })), &preset)?;
        assert!(ep.correlation_all_pairs);
        assert!(ep.correlation_heatmap);
        Ok(())
    }

    #[test]
    fn challenge1_pcr_disjoint_swap_succeeds_and_overlap_errors() -> Result<(), String> {
        let preset = LocalePreset::default();

        // EditAnalysisParametersState
        let mut ep = edit_state_with_kind(AnalysisKind::Multivariate(MultivariateSpec::Pca {
            standardize: true,
            parallel_analysis_seed: None,
            pcr_outcome_column: Some(1),
        }))?;
        if let Some(d) = &mut ep.draft {
            d.selected_data_sets = vec![0, 2];
        }
        ep.pca_predictors = vec![0, 2];

        patch_edit_params(&mut ep, &map_from(json!({ "selected_data_sets": [0, 1], "pcr_outcome_column": 2 })), &preset)?;
        assert_eq!(ep.pca_predictors, vec![0, 1]);
        assert_eq!(ep.pcr_outcome_column, Some(2));

        let err_leak = patch_edit_params(&mut ep, &map_from(json!({ "selected_data_sets": [0, 1, 2], "pcr_outcome_column": 2 })), &preset)
            .expect_err("must reject overlapping pcr_outcome_column");
        assert!(err_leak.contains("target leakage"));
        assert_eq!(ep.pca_predictors, vec![0, 1]);
        assert_eq!(ep.pcr_outcome_column, Some(2));

        // AnalyzeDialogState
        let mut az = AnalyzeDialogState {
            selected_entry_id: Some(String::from("stats.pcr")),
            selected_data_sets: vec![0, 2],
            pca_predictors: vec![0, 2],
            pcr_outcome_column: Some(1),
            ..AnalyzeDialogState::default()
        };
        patch_analyze(&mut az, &map_from(json!({ "selected_data_sets": [0, 1], "pcr_outcome_column": 2 })), &preset)?;
        assert_eq!(az.pca_predictors, vec![0, 1]);
        assert_eq!(az.pcr_outcome_column, Some(2));

        let err_az_leak = patch_analyze(&mut az, &map_from(json!({ "selected_data_sets": [0, 1, 2], "pcr_outcome_column": 2 })), &preset)
            .expect_err("must reject overlapping pcr_outcome_column in Analyze");
        assert!(err_az_leak.contains("target leakage"));
        assert_eq!(az.pca_predictors, vec![0, 1]);
        assert_eq!(az.pcr_outcome_column, Some(2));
        Ok(())
    }

    #[test]
    fn challenge2_strict_transactional_rollback_across_dialogs() {
        let preset = LocalePreset::default();

        // FormatAxes: target_axis + invalid bounds / unknown key
        let mut fa = FormatAxesState::default();
        let orig_fa = fa.clone();
        assert!(
            patch_format_axes(&mut fa, &map_from(json!({ "target_axis": "X", "title": "Corrupted X", "min": 100.0, "max": 10.0 })), None, &preset)
                .is_err()
        );
        assert_eq!(fa, orig_fa);
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "target_axis": "X", "zzz_unknown": 1 })), None, &preset).is_err());
        assert_eq!(fa.target_axis, AxisTarget::Y);
        assert_eq!(fa, orig_fa);

        // FormatGraph: show_legend + failing move_up
        let mut fg = FormatGraphState::default();
        let orig_fg = fg.clone();
        assert!(patch_format_graph(&mut fg, &map_from(json!({ "show_legend": false, "move_up": true })), &preset).is_err());
        assert_eq!(fg, orig_fg);

        // SimulateDistributions: empty columns + unknown key
        let mut sim = SimulateDistributionsDialogState::default();
        sim.columns.clear();
        let orig_sim = sim.clone();
        assert!(patch_simulate_distributions(&mut sim, &map_from(json!({ "n_rows": 50, "zzz_unknown": 1 })), &preset).is_err());
        assert!(sim.columns.is_empty());
        assert_eq!(sim, orig_sim);

        // ActiveDialog::ChangeDataAnalyzed
        let mut app = UiApp::new();
        app.canvas_dialogs.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id: SheetId::new("res-1") };
        let orig_cd = app.canvas_dialogs.change_data.clone();
        assert!(patch_active_dialog(&mut app, &map_from(json!({ "selected_data_sets": [1, 2], "zzz_unknown": true }))).is_err());
        assert_eq!(app.canvas_dialogs.change_data, orig_cd);

        // ActiveDialog::DuplicateAnalysisPrompt
        app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: SheetId::new("ds-1"),
            pending_entry_id: String::from("stats.one_way_anova"),
            existing_result_ids: vec![SheetId::new("res-1")],
        };
        app.canvas_dialogs.analyze.duplicate_choice = DuplicateChoice::CreateNew;
        app.canvas_dialogs.analyze.duplicate_target_id = None;
        assert!(patch_active_dialog(&mut app, &map_from(json!({ "choice": "modify_existing", "zzz_unknown": true }))).is_err());
        assert_eq!(app.canvas_dialogs.analyze.duplicate_choice, DuplicateChoice::CreateNew);
        assert_eq!(app.canvas_dialogs.analyze.duplicate_target_id, None);

        // ActiveDialog::ConvertArchetype preview command failure rollback
        let missing_sid = SheetId::new("missing-sheet-id");
        app.canvas_dialogs.convert_archetype.sheet_id = Some(missing_sid.clone());
        app.canvas_dialogs.convert_archetype.target_kind = TableKind::Column;
        app.canvas_dialogs.convert_archetype.allow_lossy = false;
        app.canvas_dialogs.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: missing_sid };
        let orig_conv = app.canvas_dialogs.convert_archetype.clone();
        assert!(patch_active_dialog(&mut app, &map_from(json!({ "target_kind": "grouped", "allow_lossy": true }))).is_err());
        assert_eq!(app.canvas_dialogs.convert_archetype, orig_conv);
    }

    #[test]
    fn challenge3_power_wizard_preserves_hover_n_and_hover_power() -> Result<(), String> {
        let preset = LocalePreset::default();
        let mut wizard = PowerWizardState::default();
        patch_power_wizard(&mut wizard, &map_from(json!({ "hover_n": 20.0 })), &preset)?;
        assert!(wizard.hover_n.is_some());
        assert!(wizard.hover_power.is_some());

        let mut wizard_nested = PowerWizardState::default();
        patch_power_wizard(&mut wizard_nested, &map_from(json!({ "power.curve_hover": { "hover_n": 18.0 } })), &preset)?;
        assert!(wizard_nested.hover_n.is_some());
        assert!(wizard_nested.hover_power.is_some());
        Ok(())
    }

    #[test]
    fn challenge4_simulate_distributions_nested_params_and_validation() -> Result<(), String> {
        let de = LocalePreset::for_locale("de-DE");
        let mut sim = SimulateDistributionsDialogState::default();
        patch_simulate_distributions(
            &mut sim,
            &map_from(json!({ "columns": [{ "name": "C1", "distribution": "gaussian", "params": { "mean": "1,5", "sd": "0,25" } }] })),
            &de,
        )?;
        assert_eq!(sim.columns.len(), 1);
        let Some(col0) = sim.columns.first() else {
            return Err(String::from("expected 1 simulated column"));
        };
        assert_eq!(col0.name, "C1");
        assert!((col0.mean - 1.5).abs() < 1e-12);
        assert!((col0.sd - 0.25).abs() < 1e-12);

        // Unknown sub-keys inside params must still fail
        assert!(
            patch_simulate_distributions(
                &mut sim,
                &map_from(json!({ "columns": [{ "name": "C1", "distribution": "gaussian", "params": { "bogus": 1 } }] })),
                &de,
            )
            .is_err()
        );
        // Non-object params must fail
        assert!(
            patch_simulate_distributions(
                &mut sim,
                &map_from(json!({ "columns": [{ "name": "C1", "distribution": "gaussian", "params": 123 }] })),
                &de,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn challenge5_de_locale_r13_dialogs_and_invalid_value_rejections() -> Result<(), String> {
        let de = LocalePreset::for_locale("de-DE");

        let mut img = FormatImageDialogState::default();
        patch_format_image(&mut img, &map_from(json!({ "width_pt": "120,5", "crop_left_pt": "2,5" })), &de)?;
        assert!((img.bounds_pt.width - 120.5).abs() < 1e-12);
        assert!((img.crop.left_pt - 2.5).abs() < 1e-12);

        let mut tbl = FormatEmbeddedTableDialogState::default();
        patch_format_embedded_table(&mut tbl, &map_from(json!({ "font_size_pt": "10,5", "outer_border_width_pt": "1,5" })), &de)?;
        assert!((tbl.style.font_size_pt - 10.5).abs() < 1e-12);
        assert!((tbl.style.outer_border_width_pt - 1.5).abs() < 1e-12);

        let mut eq = EqualizeScalingDialogState::default();
        patch_equalize_scaling(&mut eq, &map_from(json!({ "percent": "85,5" })), &de)?;
        assert!((eq.explicit_percent - 85.5).abs() < 1e-12);

        let mut az = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.outliers")), ..AnalyzeDialogState::default() };
        patch_analyze(&mut az, &map_from(json!({ "outlier_method": "rout", "outlier_fdr_q_percent": "2,5" })), &de)?;
        assert!((az.outlier_fdr_q_percent - 2.5).abs() < 1e-12);
        patch_analyze(&mut az, &map_from(json!({ "outlier_method": "grubbs", "outlier_alpha": "0,05" })), &de)?;
        assert!((az.outlier_alpha - 0.05).abs() < 1e-12);

        // Invalid color_mode
        let mut bp = BubblePlotInspectorState::default();
        assert!(patch_bubble_plot(&mut bp, &map_from(json!({ "color_mode": "bogus_color_mode" }))).is_err());
        assert!(patch_bubble_plot(&mut bp, &map_from(json!({ "color_mode": 123 }))).is_err());

        // Wrong-variant keys on TwoGroupComparison
        let mut ep = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        assert!(patch_edit_params(&mut ep, &map_from(json!({ "fisher": true })), &de).is_err());
        assert!(patch_edit_params(&mut ep, &map_from(json!({ "anova_method": "welch" })), &de).is_err());

        // Non-string target_axis / entry_id
        let mut fa = FormatAxesState::default();
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "target_axis": 123 })), None, &de).is_err());
        assert!(patch_analyze(&mut az, &map_from(json!({ "entry_id": 123 })), &de).is_err());

        Ok(())
    }

    #[test]
    fn iter3_sequential_correlation_all_pairs_and_paired_sign_test_hoisting() -> Result<(), String> {
        let preset = LocalePreset::default();

        // Sequential patch_edit_params: `spearman_rank: false` must preserve `correlation_all_pairs: false`
        let mut ep_corr = edit_state_with_kind(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true }))?;
        patch_edit_params(&mut ep_corr, &map_from(json!({ "correlation_all_pairs": false })), &preset)?;
        assert!(!ep_corr.correlation_all_pairs);

        patch_edit_params(&mut ep_corr, &map_from(json!({ "spearman_rank": false })), &preset)?;
        assert!(!ep_corr.correlation_all_pairs);
        assert_eq!(
            ep_corr.draft.as_ref().map(|d| &d.kind),
            Some(&AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }))
        );

        let heatmap_err =
            patch_edit_params(&mut ep_corr, &map_from(json!({ "edit.corr.spearman_rank": true, "edit.correlation.heatmap": true })), &preset)
                .expect_err("heatmap must remain blocked across sequential patches when all_pairs is false");
        assert!(heatmap_err.contains("Create correlation heatmap is disabled in X vs every Y mode."));
        assert!(!ep_corr.correlation_all_pairs);

        // `paired_sign_test` hoisted ahead of `graph_ranks` and `hodges_lehmann_ci`
        let mut ep_ranks = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        patch_edit_params(&mut ep_ranks, &map_from(json!({ "paired_sign_test": true, "graph_ranks": true })), &preset)?;
        assert!(ep_ranks.graph_ranks);
        assert!(matches!(
            ep_ranks.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, .. }))
        ));

        let mut ep_hl = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        patch_edit_params(&mut ep_hl, &map_from(json!({ "edit.two_group.paired_sign_test": true, "hodges_lehmann_ci": true })), &preset)?;
        assert!(ep_hl.hodges_lehmann_ci);
        assert!(matches!(
            ep_hl.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, .. }))
        ));
        Ok(())
    }

    #[test]
    fn iter3_format_graph_reorder_false_noop_and_pca_pred_pcr_outcome_ordering() -> Result<(), String> {
        let preset = LocalePreset::default();

        // `move_up: false` must be a no-op; non-boolean must error
        let mut fg = FormatGraphState {
            mv_levels: vec![String::from("A"), String::from("B")],
            mv_selected_level: Some(String::from("B")),
            ..FormatGraphState::default()
        };
        patch_format_graph(&mut fg, &map_from(json!({ "move_up": false })), &preset)?;
        assert_eq!(fg.mv_levels, vec![String::from("A"), String::from("B")]);
        assert!(patch_format_graph(&mut fg, &map_from(json!({ "move_up": "yes" })), &preset).is_err());

        // `analyze.pca.pred.<col>: false` processed before `pcr_outcome_column` leakage check
        let mut az = AnalyzeDialogState {
            selected_entry_id: Some(String::from("stats.pcr")),
            selected_data_sets: vec![0, 1, 2],
            pca_predictors: vec![0, 1, 2],
            ..AnalyzeDialogState::default()
        };
        patch_analyze(&mut az, &map_from(json!({ "analyze.pca.pred.2": false, "pcr_outcome_column": 2 })), &preset)?;
        assert_eq!(az.pca_predictors, vec![0, 1]);
        assert_eq!(az.selected_data_sets, vec![0, 1]);
        assert_eq!(az.pcr_outcome_column, Some(2));

        let leak_err = patch_analyze(&mut az, &map_from(json!({ "analyze.pca.pred.2": true, "pcr_outcome_column": 2 })), &preset)
            .expect_err("must still reject leakage when predictor remains active");
        assert!(leak_err.contains("target leakage"));
        assert_eq!(az.pca_predictors, vec![0, 1]);
        assert_eq!(az.pcr_outcome_column, Some(2));

        // `analyze.pcr.outcome.<col>` preserved across disjoint `selected_data_sets` updates
        patch_analyze(&mut az, &map_from(json!({ "selected_data_sets": [0, 1], "analyze.pcr.outcome.2": true })), &preset)?;
        assert_eq!(az.pcr_outcome_column, Some(2));
        Ok(())
    }

    #[test]
    fn iter3_power_wizard_slugs_and_strict_field_validation() -> Result<(), String> {
        let preset = LocalePreset::default();

        // PowerWizardState accepts `two_sample_t` and `simple_linear`
        let mut pw = PowerWizardState::default();
        patch_power_wizard(&mut pw, &map_from(json!({ "family": "simple_linear" })), &preset)?;
        assert_eq!(pw.family, PowerTestFamily::SimpleLinearRegression);
        patch_power_wizard(&mut pw, &map_from(json!({ "family": "two_sample_t", "hover_n": 20.0 })), &preset)?;
        assert_eq!(pw.family, PowerTestFamily::TwoSampleUnpairedT);
        assert!(pw.hover_n.is_some());

        let mut ep_pwr = edit_state_with_kind(AnalysisKind::PowerAnalysis(curvebook_doc::PowerAnalysisSpec::default()))?;
        patch_edit_params(&mut ep_pwr, &map_from(json!({ "edit.power.family": "simple_linear" })), &preset)?;
        assert!(matches!(
            ep_pwr.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::PowerAnalysis(curvebook_doc::PowerAnalysisSpec {
                family: curvebook_doc::PowerTestFamily::SimpleLinearRegression,
                ..
            }))
        ));

        // Strict validation on `edit.post_hoc.*` and `p_value_format`
        let mut ep_anova = edit_state_with_kind(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?;
        assert!(patch_edit_params(&mut ep_anova, &map_from(json!({ "edit.post_hoc.tukey": "yes" })), &preset).is_err());
        patch_edit_params(&mut ep_anova, &map_from(json!({ "edit.post_hoc.tukey": false })), &preset)?;
        assert!(patch_edit_params(&mut ep_anova, &map_from(json!({ "p_value_format": "bogus_format" })), &preset).is_err());
        patch_edit_params(&mut ep_anova, &map_from(json!({ "p_value_format": "four_decimal" })), &preset)?;
        assert_eq!(ep_anova.draft.as_ref().map(|d| d.p_value_format), Some(PValueFormat::FourDecimal));

        // Strict validation on `add_segment`, `remove_segment`, `clear_thresholds`, and dual `target_axis`/`axes.target_axis`
        let mut fa = FormatAxesState::default();
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "add_segment": "yes" })), None, &preset).is_err());
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "remove_segment": "zero" })), None, &preset).is_err());
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "clear_thresholds": 1 })), None, &preset).is_err());
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "target_axis": "X", "axes.target_axis": 123 })), None, &preset).is_err());
        assert!(patch_format_axes(&mut fa, &map_from(json!({ "target_axis": "X", "axes.target_axis": "Y" })), None, &preset).is_err());
        patch_format_axes(&mut fa, &map_from(json!({ "add_segment": false, "clear_thresholds": false })), None, &preset)?;
        patch_format_axes(&mut fa, &map_from(json!({ "target_axis": "X", "axes.target_axis": "x" })), None, &preset)?;
        assert_eq!(fa.target_axis, AxisTarget::X);
        Ok(())
    }

    #[test]
    fn iter4_format_graph_toggle_eq_mode_and_analyze_two_group_alias_order() -> Result<(), String> {
        let preset = LocalePreset::default();

        // Challenge 1: `toggle_model_equation_mode: false` is a strict no-op, non-bool returns Err, `true` toggles
        let mut fg = FormatGraphState { model_equation_mode: Some(EquationDisplayMode::Substituted), ..FormatGraphState::default() };
        patch_format_graph(&mut fg, &map_from(json!({ "toggle_model_equation_mode": false })), &preset)?;
        assert_eq!(fg.model_equation_mode, Some(EquationDisplayMode::Substituted));
        patch_format_graph(&mut fg, &map_from(json!({ "format_graph.toggle_model_equation_mode": false })), &preset)?;
        assert_eq!(fg.model_equation_mode, Some(EquationDisplayMode::Substituted));
        assert!(patch_format_graph(&mut fg, &map_from(json!({ "toggle_model_equation_mode": "bogus" })), &preset).is_err());
        assert_eq!(fg.model_equation_mode, Some(EquationDisplayMode::Substituted));
        patch_format_graph(&mut fg, &map_from(json!({ "toggle_model_equation_mode": true })), &preset)?;
        assert_eq!(fg.model_equation_mode, Some(EquationDisplayMode::Symbolic));

        // Challenge 2: `patch_analyze` `TwoGroupMethod` invariant to alias vs BTreeMap alphabetical order
        let mut az_widget = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.two_group")), ..AnalyzeDialogState::default() };
        patch_analyze(&mut az_widget, &map_from(json!({ "analyze.two_group.method": "mann_whitney", "assume_gaussian": true })), &preset)?;
        assert_eq!(az_widget.two_group_method, Some(TwoGroupMethod::MannWhitneyU));

        let mut az_plain = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.two_group")), ..AnalyzeDialogState::default() };
        patch_analyze(&mut az_plain, &map_from(json!({ "two_group_method": "mann_whitney", "assume_gaussian": true })), &preset)?;
        assert_eq!(az_plain.two_group_method, Some(TwoGroupMethod::MannWhitneyU));

        let mut az_paired_widget = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.two_group")), ..AnalyzeDialogState::default() };
        patch_analyze(&mut az_paired_widget, &map_from(json!({ "paired_or_matched": true, "analyze.two_group.method": "unpaired_t" })), &preset)?;
        assert_eq!(az_paired_widget.two_group_method, Some(TwoGroupMethod::UnpairedT));

        let mut az_paired_plain = AnalyzeDialogState { selected_entry_id: Some(String::from("stats.two_group")), ..AnalyzeDialogState::default() };
        patch_analyze(&mut az_paired_plain, &map_from(json!({ "paired_or_matched": true, "two_group_method": "unpaired_t" })), &preset)?;
        assert_eq!(az_paired_plain.two_group_method, Some(TwoGroupMethod::UnpairedT));

        // Reviewer 6 Minor Note: `analyze.pcr.outcome.<col>: "false"` rejects non-bool, and `false` does not skip overlap clearing
        let mut az_pcr = AnalyzeDialogState {
            selected_entry_id: Some(String::from("stats.pcr")),
            selected_data_sets: vec![1, 2],
            pca_predictors: vec![1, 2],
            pcr_outcome_column: Some(0),
            ..AnalyzeDialogState::default()
        };
        assert!(patch_analyze(&mut az_pcr, &map_from(json!({ "selected_data_sets": [0, 1], "analyze.pcr.outcome.2": "false" })), &preset).is_err());
        assert_eq!(az_pcr.pcr_outcome_column, Some(0));
        patch_analyze(&mut az_pcr, &map_from(json!({ "selected_data_sets": [0, 1], "analyze.pcr.outcome.2": false })), &preset)?;
        assert_eq!(az_pcr.pcr_outcome_column, None);
        Ok(())
    }

    #[test]
    fn iter4_edit_params_welch_false_preserves_nonparametric_and_paired_methods() -> Result<(), String> {
        let preset = LocalePreset::default();

        // MannWhitneyU + welch_correction: false preserves MannWhitneyU; true returns Err
        let mut ep_mw =
            edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() }))?;
        assert!(patch_edit_params(&mut ep_mw, &map_from(json!({ "welch_correction": true })), &preset).is_err());
        patch_edit_params(&mut ep_mw, &map_from(json!({ "welch_correction": false })), &preset)?;
        assert!(matches!(
            ep_mw.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, .. }))
        ));
        assert!(!ep_mw.welch_correction);

        // PairedT + edit.two_group.welch: false preserves PairedT
        let mut ep_paired =
            edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }))?;
        patch_edit_params(&mut ep_paired, &map_from(json!({ "edit.two_group.welch": false })), &preset)?;
        assert!(matches!(
            ep_paired.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, .. }))
        ));

        // Combined {"paired_sign_test": true, "welch_correction": false} preserves PairedSignTest
        let mut ep_sign = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?;
        patch_edit_params(&mut ep_sign, &map_from(json!({ "paired_sign_test": true, "welch_correction": false })), &preset)?;
        assert!(matches!(
            ep_sign.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, .. }))
        ));

        // LognormalUnpairedT + welch_correction: false preserves LognormalUnpairedT
        let mut ep_log = edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::LognormalUnpairedT,
            ..TwoGroupSpec::default()
        }))?;
        patch_edit_params(&mut ep_log, &map_from(json!({ "welch_correction": false })), &preset)?;
        assert!(matches!(
            ep_log.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::LognormalUnpairedT, .. }))
        ));

        // WelchT -> UnpairedT on welch_correction: false, and UnpairedT -> WelchT on true
        let mut ep_welch =
            edit_state_with_kind(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }))?;
        patch_edit_params(&mut ep_welch, &map_from(json!({ "welch_correction": false })), &preset)?;
        assert!(matches!(
            ep_welch.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, .. }))
        ));
        assert!(!ep_welch.welch_correction);
        patch_edit_params(&mut ep_welch, &map_from(json!({ "edit.two_group.welch": true })), &preset)?;
        assert!(matches!(
            ep_welch.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, .. }))
        ));
        assert!(ep_welch.welch_correction);
        Ok(())
    }

    #[test]
    fn iter4_pairwise_bubble_layout_and_randomize_strict_validation() -> Result<(), String> {
        let preset = LocalePreset::default();

        // Pairwise: reject non-bool `auto_add` and contradictory `cld_enabled` + `display_mode`
        let mut pw = PairwiseComparisonsState::default();
        assert!(patch_pairwise(&mut pw, &map_from(json!({ "auto_add": "yes" })), None, &preset).is_err());
        assert!(patch_pairwise(&mut pw, &map_from(json!({ "cld_enabled": false, "display_mode": "both" })), None, &preset).is_err());
        assert_eq!(pw.display_mode, PairwiseDisplayMode::BracketsOnly);
        assert!(!pw.cld_enabled);
        assert!(patch_pairwise(&mut pw, &map_from(json!({ "cld_enabled": true, "display_mode": "brackets_only" })), None, &preset).is_err());
        patch_pairwise(&mut pw, &map_from(json!({ "auto_add": false, "cld_enabled": true, "display_mode": "both" })), None, &preset)?;
        assert!(pw.cld_enabled);
        assert_eq!(pw.display_mode, PairwiseDisplayMode::BracketsAndCompactLetterDisplay);

        // BubblePlot: reject unknown `plot_mode` strings and non-strings
        let mut bp = BubblePlotInspectorState::default();
        patch_bubble_plot(&mut bp, &map_from(json!({ "plot_mode": "categorical_summary" })))?;
        assert!(matches!(bp.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        assert!(patch_bubble_plot(&mut bp, &map_from(json!({ "plot_mode": "bogus_mode" }))).is_err());
        assert!(patch_bubble_plot(&mut bp, &map_from(json!({ "plot_mode": 123 }))).is_err());
        assert!(matches!(bp.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        patch_bubble_plot(&mut bp, &map_from(json!({ "plot_mode": "bubble_scatter" })))?;
        assert_eq!(bp.plot_mode, MultivariatePlotMode::BubbleScatter);

        // PageLayout: reject non-string `new_callout_markdown`
        let mut pl = PageLayoutComposerState::default();
        assert!(patch_page_layout(&mut pl, &map_from(json!({ "new_callout_markdown": 123 })), None).is_err());
        patch_page_layout(&mut pl, &map_from(json!({ "new_callout_markdown": "Note **A**" })), None)?;
        assert_eq!(pl.new_callout_markdown, "Note **A**");

        // RandomizeGroups: reject non-string elements in `subject_ids` array
        let mut rg = RandomizeGroupsDialogState { subject_ids: vec![String::from("Orig")], ..RandomizeGroupsDialogState::default() };
        assert!(patch_randomize_groups(&mut rg, &map_from(json!({ "subject_ids": [1, 2, 3] }))).is_err());
        assert_eq!(rg.subject_ids, vec![String::from("Orig")]);
        patch_randomize_groups(&mut rg, &map_from(json!({ "subject_ids": ["S1", "S2"] })))?;
        assert_eq!(rg.subject_ids, vec![String::from("S1"), String::from("S2")]);
        Ok(())
    }

    #[test]
    fn m2_iter3_invalid_parameter_stage_index_and_must_specify_return_bad_input() -> Result<(), String> {
        let mut app = UiApp::new();
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 2, "rows": 3, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setCells", json!({ "sheet_id": "ds-1", "rows": [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]] }))
            .map_err(|e| e.to_string())?;

        // 1. PowerAndSampleSize invalid parameter (`alpha: 1.5`), invalid stage index (`stage: 99`), and `must specify` -> BadInput.
        app.canvas_dialogs.open_power_wizard();
        app.canvas_dialogs.power_wizard.stage = crate::power_wizard::PowerWizardStage::SignificanceAndSolve;
        let alpha_err =
            patch_active_dialog(&mut app, &map_from(json!({ "alpha": 1.5 }))).err().ok_or_else(|| String::from("expected BadInput for alpha=1.5"))?;
        assert_eq!(alpha_err.code, curvebook_doc::ErrorCode::BadInput, "got {alpha_err:?}");

        let stage_err =
            patch_active_dialog(&mut app, &map_from(json!({ "stage": 99 }))).err().ok_or_else(|| String::from("expected BadInput for stage=99"))?;
        assert_eq!(stage_err.code, curvebook_doc::ErrorCode::BadInput, "got {stage_err:?}");

        let hover_err = patch_active_dialog(&mut app, &map_from(json!({ "power.curve_hover": {} })))
            .err()
            .ok_or_else(|| String::from("expected BadInput for empty power.curve_hover"))?;
        assert_eq!(hover_err.code, curvebook_doc::ErrorCode::BadInput, "got {hover_err:?}");
        app.canvas_dialogs.close_dialog();

        // 2. EditAnalysisParameters InvalidConfidenceLevel (`confidence_percent: 150.0`) and
        //    disabled precondition (`welch_correction` on PairedT) remain Conflict (`disabled: true`).
        let spec = AnalysisSpec::new(
            "res-1",
            "ds-1",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
        )
        .map_err(|e| e.to_string())?;
        app.canvas_dialogs.edit_params.draft = Some(spec);
        app.canvas_dialogs.edit_params.paired_or_matched = true;
        app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
        app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::from("res-1") };

        let conf_err = patch_active_dialog(&mut app, &map_from(json!({ "confidence_percent": 150.0 })))
            .err()
            .ok_or_else(|| String::from("expected Conflict for confidence_percent=150"))?;
        assert_eq!(conf_err.code, curvebook_doc::ErrorCode::Conflict, "got {conf_err:?}");

        let welch_err = patch_active_dialog(&mut app, &map_from(json!({ "welch_correction": true })))
            .err()
            .ok_or_else(|| String::from("expected Conflict for welch_correction on PairedT"))?;
        assert_eq!(welch_err.code, curvebook_doc::ErrorCode::Conflict, "got {welch_err:?}");
        Ok(())
    }
}
