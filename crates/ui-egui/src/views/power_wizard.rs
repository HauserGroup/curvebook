//! Interactive 4-stage Power & Sample Size wizard modal body (`R10.3`).

#[path = "../i18n/w5_power_wizard.rs"]
mod w5_i18n;

use curvebook_math::dist::norm_ppf;
use curvebook_palette::Rgba8;
use curvebook_stats::power::{
    CorrelationPowerMethod, PowerAnalysisResult, PowerCurvePoint, PowerSolveTarget, PowerTestFamily, SensitivityCell, TwoProportionsMethod, cohens_h,
};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::paint::PaintOpKind;
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldEditState, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab,
    emit_text_field, emit_toggle, maybe_paint_focus_ring, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_widget};
use crate::power_wizard::{PowerEffectInputMode, PowerWizardStage, PowerWizardState};
use crate::text_bridge::UiTextStyle;

use super::a11y::{labeled_meta, selectable_meta};

const DOMAIN: &str = "power_wizard";

const fn default_pilot_mean1() -> f64 {
    10.0
}

const fn default_pilot_mean2() -> f64 {
    10.8
}

const fn default_survival_hr() -> f64 {
    0.65
}

const fn default_survival_event_prob() -> f64 {
    0.60
}

/// Transient view state owned by the Power & Sample Size wizard shard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerWizardViewState {
    pub hover_sample_size: Option<u32>,
    pub hover_power: Option<f64>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub dropout_rate: f64,
    #[serde(default = "default_pilot_mean1")]
    pub pilot_mean1: f64,
    #[serde(default = "default_pilot_mean2")]
    pub pilot_mean2: f64,
    #[serde(default)]
    pub survival_log_rank: bool,
    #[serde(default = "default_survival_hr")]
    pub survival_hazard_ratio: f64,
    #[serde(default = "default_survival_event_prob")]
    pub survival_event_prob: f64,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Default for PowerWizardViewState {
    fn default() -> Self {
        Self {
            hover_sample_size: None,
            hover_power: None,
            revision: 0,
            dropout_rate: 0.0,
            pilot_mean1: default_pilot_mean1(),
            pilot_mean2: default_pilot_mean2(),
            survival_log_rank: false,
            survival_hazard_ratio: default_survival_hr(),
            survival_event_prob: default_survival_event_prob(),
            last_error: None,
        }
    }
}

fn wiz_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    crate::i18n::try_tr(locale, key).or_else(|| crate::i18n::try_tr_in_slices(&[w5_i18n::ENTRIES], locale, key)).unwrap_or(fallback)
}

fn custom_action(payload: impl Into<String>) -> WidgetAction {
    WidgetAction::Custom { domain: String::from(DOMAIN), payload: payload.into() }
}

fn handled_outcome(action: impl Into<String>) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: action.into(), ..Default::default() }
}

#[derive(Clone, Copy)]
struct FamilyCardSpec {
    key: &'static str,
    title: &'static str,
    subtitle: &'static str,
    family: PowerTestFamily,
    is_survival: bool,
}

const FAMILY_CARDS: [FamilyCardSpec; 9] = [
    FamilyCardSpec {
        key: "unpaired_t",
        title: "Two-Sample t",
        subtitle: "Independent means (d)",
        family: PowerTestFamily::TwoSampleUnpairedT,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "paired_t",
        title: "Paired t",
        subtitle: "Matched pairs (dz)",
        family: PowerTestFamily::TwoSamplePairedT,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "one_sample_t",
        title: "One-Sample t",
        subtitle: "Mean vs null (d)",
        family: PowerTestFamily::OneSampleT,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "one_way_anova",
        title: "One-Way ANOVA",
        subtitle: "k >= 2 groups (f)",
        family: PowerTestFamily::OneWayAnova,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "survival_log_rank",
        title: "Survival Log-Rank",
        subtitle: "Time-to-event (HR)",
        family: PowerTestFamily::TwoProportions,
        is_survival: true,
    },
    FamilyCardSpec {
        key: "two_proportions",
        title: "Two Proportions",
        subtitle: "Rates (p1 vs p2)",
        family: PowerTestFamily::TwoProportions,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "chi_square",
        title: "Contingency / Chi2",
        subtitle: "Goodness-of-fit (w)",
        family: PowerTestFamily::ChiSquare,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "correlation",
        title: "Correlation",
        subtitle: "Pearson r vs H0=0",
        family: PowerTestFamily::Correlation,
        is_survival: false,
    },
    FamilyCardSpec {
        key: "regression",
        title: "Linear Regression",
        subtitle: "Slope test (f^2 / R^2)",
        family: PowerTestFamily::SimpleLinearRegression,
        is_survival: false,
    },
];

/// Dispatch custom `WidgetAction::Custom` events targeting the `power_wizard` domain.
///
/// # Errors
/// Returns an error if sequential stage validation fails or numeric inputs are out of range.
pub fn handle_custom_action(app: &mut UiApp, state: &mut PowerWizardViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(raw_stage) = payload.strip_prefix("stage:") {
        return handle_stage_action(app, state, raw_stage);
    }
    if let Some(key) = payload.strip_prefix("family:") {
        return handle_family_action(app, state, key);
    }
    if let Some(target) = payload.strip_prefix("solve:") {
        return handle_solve_action(app, state, target);
    }
    if let Some(mode) = payload.strip_prefix("effect_mode:") {
        return handle_effect_mode_action(app, state, mode);
    }
    if let Some(preset) = payload.strip_prefix("pilot_preset:") {
        return handle_pilot_preset_action(app, state, preset);
    }
    if payload == "pilot_apply" {
        return handle_pilot_apply_action(app, state);
    }
    if let Some(tail) = payload.strip_prefix("tail:") {
        return handle_tail_action(app, state, tail);
    }
    if payload == "toggle:continuity_correction" {
        app.canvas_dialogs.power_wizard.continuity_correction = !app.canvas_dialogs.power_wizard.continuity_correction;
        let _ = app.canvas_dialogs.power_wizard.recalculate();
        state.revision = state.revision.wrapping_add(1);
        return Ok(handled_outcome("power_wizard.continuity_correction"));
    }
    if let Some(method) = payload.strip_prefix("prop_method:") {
        return handle_prop_method_action(app, state, method);
    }
    if let Some(method) = payload.strip_prefix("corr_method:") {
        return handle_corr_method_action(app, state, method);
    }
    if let Some(rest) = payload.strip_prefix("spin:") {
        return handle_spinner_action(app, state, rest);
    }
    if let Some(field) = payload.strip_prefix("slider:") {
        return handle_slider_action(app, state, field);
    }
    if let Some(raw_frac) = payload.strip_prefix("hover_frac:") {
        return handle_hover_frac_action(app, state, raw_frac);
    }
    if let Some(raw_idx) = payload.strip_prefix("select_sens:") {
        return handle_select_sens_action(app, state, raw_idx);
    }
    if let Some(rest) = payload.strip_prefix("preset:") {
        return handle_quick_preset_action(app, state, rest);
    }
    Ok(UiEventOutcome::default())
}

fn handle_stage_action(app: &mut UiApp, state: &mut PowerWizardViewState, raw_stage: &str) -> Result<UiEventOutcome, String> {
    let idx: usize = raw_stage.trim().parse().map_err(|_| format!("invalid power stage `{raw_stage}`"))?;
    let target = PowerWizardStage::from_index(idx.saturating_sub(1)).ok_or_else(|| format!("unknown power stage `{idx}`"))?;
    match app.canvas_dialogs.power_wizard.step_to_stage(target) {
        Ok(()) => {
            state.last_error = None;
            state.revision = state.revision.wrapping_add(1);
            Ok(handled_outcome(format!("power_wizard.stage:{idx}")))
        }
        Err(msg) => {
            state.last_error = Some(msg.clone());
            state.revision = state.revision.wrapping_add(1);
            Err(msg)
        }
    }
}

fn handle_family_action(app: &mut UiApp, state: &mut PowerWizardViewState, key: &str) -> Result<UiEventOutcome, String> {
    let Some(card) = FAMILY_CARDS.iter().find(|c| c.key == key) else {
        return Err(format!("unknown power test family `{key}`"));
    };
    state.survival_log_rank = card.is_survival;
    app.canvas_dialogs.power_wizard.set_family(card.family);
    if card.is_survival {
        sync_survival_proportions(&mut app.canvas_dialogs.power_wizard, state);
        let _ = app.canvas_dialogs.power_wizard.recalculate();
    }
    clear_pilot_text_buffers(app);
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.family:{key}")))
}

fn handle_solve_action(app: &mut UiApp, state: &mut PowerWizardViewState, target: &str) -> Result<UiEventOutcome, String> {
    let solve_target = match target {
        "sample_size" => PowerSolveTarget::SampleSize,
        "power" => PowerSolveTarget::Power,
        "effect" => PowerSolveTarget::DetectableEffect,
        _ => return Err(format!("unknown solve target `{target}`")),
    };
    app.canvas_dialogs.power_wizard.solve_target = solve_target;
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.solve:{target}")))
}

fn handle_effect_mode_action(app: &mut UiApp, state: &mut PowerWizardViewState, mode: &str) -> Result<UiEventOutcome, String> {
    let next_mode = match mode {
        "standardized" => PowerEffectInputMode::Standardized,
        "means_and_sd" => PowerEffectInputMode::MeansAndSd,
        "paired_diff" => PowerEffectInputMode::PairedDiffDirect,
        "proportions" => PowerEffectInputMode::Proportions,
        "r_squared" => PowerEffectInputMode::RegressionRSquared,
        "slope" => PowerEffectInputMode::RegressionSlope,
        _ => return Err(format!("unknown effect mode `{mode}`")),
    };
    let pw = &mut app.canvas_dialogs.power_wizard;
    pw.effect_mode = next_mode;
    if next_mode == PowerEffectInputMode::MeansAndSd
        && matches!(pw.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT)
    {
        pw.mean_diff = state.pilot_mean2 - state.pilot_mean1;
    }
    let _ = pw.recalculate();
    clear_pilot_text_buffers(app);
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.effect_mode:{mode}")))
}

fn handle_pilot_preset_action(app: &mut UiApp, state: &mut PowerWizardViewState, preset: &str) -> Result<UiEventOutcome, String> {
    let pw = &mut app.canvas_dialogs.power_wizard;
    let scale = match preset {
        "small" => 0,
        "medium" => 1,
        "large" => 2,
        _ => return Err(format!("unknown pilot preset `{preset}`")),
    };
    let eff = match pw.family {
        PowerTestFamily::OneWayAnova => [0.10, 0.25, 0.40][scale],
        PowerTestFamily::Correlation | PowerTestFamily::ChiSquare => [0.10, 0.30, 0.50][scale],
        PowerTestFamily::SimpleLinearRegression => [0.02, 0.15, 0.35][scale],
        _ => [0.20, 0.50, 0.80][scale],
    };
    pw.standardized_effect = eff;
    pw.effect_mode = PowerEffectInputMode::Standardized;
    if matches!(pw.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT) {
        pw.mean_diff = eff * pw.sd.max(0.01);
        state.pilot_mean2 = state.pilot_mean1 + pw.mean_diff;
    } else if pw.family == PowerTestFamily::TwoProportions {
        if state.survival_log_rank {
            state.survival_hazard_ratio = [0.82, 0.65, 0.48][scale];
            sync_survival_proportions(pw, state);
        } else {
            pw.p1 = 0.30;
            pw.p2 = [0.40, 0.54, 0.68][scale];
            pw.effect_mode = PowerEffectInputMode::Proportions;
        }
    } else if pw.family == PowerTestFamily::SimpleLinearRegression {
        pw.r_squared = eff / (1.0 + eff);
    }
    let _ = pw.recalculate();
    clear_pilot_text_buffers(app);
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.pilot_preset:{preset}")))
}

fn handle_pilot_apply_action(app: &mut UiApp, state: &mut PowerWizardViewState) -> Result<UiEventOutcome, String> {
    let preset = app.locale_preset.clone();
    let parse_buf = |id: &str| -> Result<Option<f64>, String> {
        let Some(buf) = app.interaction.text_field_buffers.get(id) else {
            return Ok(None);
        };
        let raw = buf.text.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        parse_localized_number_input(raw, &preset)
            .filter(|v| v.is_finite())
            .map(Some)
            .ok_or_else(|| format!("Invalid numeric input `{raw}` in `{id}`"))
    };

    let m1_opt = parse_buf("power.pilot.mean1.input")?;
    let m2_opt = parse_buf("power.pilot.mean2.input")?;
    let sd_opt = parse_buf("power.pilot.sd.input")?;
    let corr_opt = parse_buf("power.pilot.corr.input")?;
    let p1_opt = parse_buf("power.pilot.p1.input")?;
    let p2_opt = parse_buf("power.pilot.p2.input")?;
    let hr_opt = parse_buf("power.pilot.hr.input")?;
    let ev_opt = parse_buf("power.pilot.evprob.input")?;
    let r2_opt = parse_buf("power.pilot.r2.input")?;
    let std_opt = parse_buf("power.pilot.std.input")?;

    let pw = &mut app.canvas_dialogs.power_wizard;
    if let Some(m1) = m1_opt {
        state.pilot_mean1 = m1;
    }
    if let Some(m2) = m2_opt {
        state.pilot_mean2 = m2;
    }
    if let Some(sd) = sd_opt {
        if sd <= 0.0 {
            return Err(String::from("Standard deviation must be > 0."));
        }
        pw.sd = sd;
    }
    if let Some(r) = corr_opt {
        if !(-0.99..1.0).contains(&r) {
            return Err(String::from("Within-pair correlation r must lie in (-1, 1)."));
        }
        pw.within_pair_correlation = r;
    }
    if let Some(p1) = p1_opt {
        if !(0.01..0.99).contains(&p1) {
            return Err(String::from("Proportion p1 must lie in (0.01, 0.99)."));
        }
        pw.p1 = p1;
    }
    if let Some(p2) = p2_opt {
        if !(0.01..0.99).contains(&p2) {
            return Err(String::from("Proportion p2 must lie in (0.01, 0.99)."));
        }
        pw.p2 = p2;
    }
    if let Some(hr) = hr_opt {
        if hr <= 0.0 || (hr - 1.0).abs() < 1e-4 {
            return Err(String::from("Hazard Ratio must be > 0 and != 1.0."));
        }
        state.survival_hazard_ratio = hr;
    }
    if let Some(ev) = ev_opt {
        if !(0.02..0.98).contains(&ev) {
            return Err(String::from("Event probability must lie in (0.02, 0.98)."));
        }
        state.survival_event_prob = ev;
    }
    if let Some(r2) = r2_opt {
        if !(0.005..0.99).contains(&r2) {
            return Err(String::from("R-squared must lie in (0.005, 0.99)."));
        }
        pw.r_squared = r2;
        pw.effect_mode = PowerEffectInputMode::RegressionRSquared;
    }
    if let Some(eff) = std_opt {
        if eff == 0.0 {
            return Err(String::from("Standardized effect size must be non-zero."));
        }
        pw.standardized_effect = eff;
    }

    if state.survival_log_rank && pw.family == PowerTestFamily::TwoProportions {
        sync_survival_proportions(pw, state);
    } else if matches!(pw.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT)
        && (m1_opt.is_some() || m2_opt.is_some() || sd_opt.is_some() || corr_opt.is_some())
    {
        pw.mean_diff = state.pilot_mean2 - state.pilot_mean1;
        pw.effect_mode = PowerEffectInputMode::MeansAndSd;
        if let Some(derived) = pw.derived_standardized_effect() {
            pw.standardized_effect = derived;
        }
    } else if pw.family == PowerTestFamily::TwoProportions && (p1_opt.is_some() || p2_opt.is_some()) {
        pw.effect_mode = PowerEffectInputMode::Proportions;
        if let Some(derived) = pw.derived_standardized_effect() {
            pw.standardized_effect = derived;
        }
    } else if let Some(derived) = pw.derived_standardized_effect() {
        pw.standardized_effect = derived;
    }

    match pw.recalculate() {
        Ok(_) => {
            state.last_error = None;
            state.revision = state.revision.wrapping_add(1);
            Ok(handled_outcome("power_wizard.pilot_apply"))
        }
        Err(err) => {
            state.last_error = Some(err.clone());
            state.revision = state.revision.wrapping_add(1);
            Err(err)
        }
    }
}

fn handle_tail_action(app: &mut UiApp, state: &mut PowerWizardViewState, tail: &str) -> Result<UiEventOutcome, String> {
    let pw = &mut app.canvas_dialogs.power_wizard;
    match tail {
        "two" => pw.two_tailed = true,
        "greater" => {
            if !pw.family.supports_one_sided() {
                return Err(format!("{} supports two-tailed tests only.", pw.family.label()));
            }
            pw.two_tailed = false;
            pw.one_sided_greater = true;
        }
        "less" => {
            if !pw.family.supports_one_sided() {
                return Err(format!("{} supports two-tailed tests only.", pw.family.label()));
            }
            pw.two_tailed = false;
            pw.one_sided_greater = false;
        }
        _ => return Err(format!("unknown tail option `{tail}`")),
    }
    let _ = pw.recalculate();
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.tail:{tail}")))
}

fn handle_prop_method_action(app: &mut UiApp, state: &mut PowerWizardViewState, method: &str) -> Result<UiEventOutcome, String> {
    app.canvas_dialogs.power_wizard.two_proportions_mode = match method {
        "pooled" => TwoProportionsMethod::PooledNullUnpooledAlt,
        "unpooled" => TwoProportionsMethod::Unpooled,
        "cohens_h" => TwoProportionsMethod::CohensH,
        _ => return Err(format!("unknown proportions method `{method}`")),
    };
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.prop_method:{method}")))
}

fn handle_corr_method_action(app: &mut UiApp, state: &mut PowerWizardViewState, method: &str) -> Result<UiEventOutcome, String> {
    app.canvas_dialogs.power_wizard.correlation_method = match method {
        "fisher_z" => CorrelationPowerMethod::FisherZ,
        "fisher_z_tcrit" => CorrelationPowerMethod::FisherZWithTCrit,
        "exact_t" => CorrelationPowerMethod::ExactTTransform,
        "slope_t" => CorrelationPowerMethod::ConditionalSlopeT,
        _ => return Err(format!("unknown correlation method `{method}`")),
    };
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.corr_method:{method}")))
}

fn handle_spinner_action(app: &mut UiApp, state: &mut PowerWizardViewState, rest: &str) -> Result<UiEventOutcome, String> {
    let Some((field, raw_delta)) = rest.split_once(':') else {
        return Err(format!("invalid spinner payload `{rest}`"));
    };
    let delta: f64 = raw_delta.parse().map_err(|_| format!("invalid spinner delta `{raw_delta}`"))?;
    apply_field_delta(app, state, field, delta)?;
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.spin:{field}")))
}

fn apply_field_delta(app: &mut UiApp, state: &mut PowerWizardViewState, field: &str, delta: f64) -> Result<(), String> {
    let mut buf_sync: Option<(&'static str, String)> = None;
    {
        let pw = &mut app.canvas_dialogs.power_wizard;
        match field {
            "alpha" => {
                pw.alpha = round_dec((pw.alpha + delta).clamp(0.001, 0.25), 4);
                if pw.target_power <= pw.alpha + 0.02 {
                    pw.target_power = (pw.alpha + 0.05).min(0.95);
                }
            }
            "target_power" => {
                let min_p = (pw.alpha + 0.02).max(0.50);
                pw.target_power = round_dec((pw.target_power + delta).clamp(min_p, 0.99), 3);
            }
            "effect_size" => {
                let cur = effective_effect_display(pw, state);
                let (lo, hi) = effect_range_for_family(pw.family);
                let next = round_dec((cur + delta).clamp(lo, hi), 3);
                apply_standardized_effect_update(pw, state, next);
            }
            "allocation_ratio" => {
                pw.allocation_ratio = round_dec((pw.allocation_ratio + delta).clamp(0.25, 4.0), 2);
            }
            "dropout" => {
                state.dropout_rate = round_dec((state.dropout_rate + delta).clamp(0.0, 0.75), 3);
            }
            "sample_size" => {
                let min_n = pw.family.min_integer_n() as f64;
                pw.sample_size = (pw.sample_size + delta).round().clamp(min_n, 5000.0);
            }
            "anova_groups" => {
                let next_k = (pw.anova_groups as isize + delta.round() as isize).clamp(2, 12) as usize;
                resize_anova_groups(pw, next_k);
            }
            "chi2_df" => {
                pw.chi_square_df = (pw.chi_square_df + delta).round().clamp(1.0, 50.0);
            }
            "pilot_mean1" => {
                state.pilot_mean1 = round_dec(state.pilot_mean1 + delta, 3);
                pw.mean_diff = round_dec(state.pilot_mean2 - state.pilot_mean1, 4);
                pw.effect_mode = PowerEffectInputMode::MeansAndSd;
                buf_sync = Some(("power.pilot.mean1.input", format!("{:.2}", state.pilot_mean1)));
            }
            "pilot_mean2" => {
                state.pilot_mean2 = round_dec(state.pilot_mean2 + delta, 3);
                pw.mean_diff = round_dec(state.pilot_mean2 - state.pilot_mean1, 4);
                pw.effect_mode = PowerEffectInputMode::MeansAndSd;
                buf_sync = Some(("power.pilot.mean2.input", format!("{:.2}", state.pilot_mean2)));
            }
            "pilot_sd" => {
                pw.sd = round_dec((pw.sd + delta).clamp(0.05, 1000.0), 3);
                pw.effect_mode = PowerEffectInputMode::MeansAndSd;
                buf_sync = Some(("power.pilot.sd.input", format!("{:.2}", pw.sd)));
            }
            "pilot_corr" => {
                pw.within_pair_correlation = round_dec((pw.within_pair_correlation + delta).clamp(-0.90, 0.95), 2);
                pw.effect_mode = PowerEffectInputMode::MeansAndSd;
                buf_sync = Some(("power.pilot.corr.input", format!("{:.2}", pw.within_pair_correlation)));
            }
            "pilot_p1" => {
                pw.p1 = round_dec((pw.p1 + delta).clamp(0.02, 0.98), 3);
                if (pw.p1 - pw.p2).abs() < 0.01 {
                    pw.p2 = if pw.p1 < 0.85 { round_dec(pw.p1 + 0.05, 3) } else { round_dec(pw.p1 - 0.05, 3) };
                }
                pw.effect_mode = PowerEffectInputMode::Proportions;
                buf_sync = Some(("power.pilot.p1.input", format!("{:.2}", pw.p1)));
            }
            "pilot_p2" => {
                pw.p2 = round_dec((pw.p2 + delta).clamp(0.02, 0.98), 3);
                if (pw.p1 - pw.p2).abs() < 0.01 {
                    pw.p1 = if pw.p2 > 0.15 { round_dec(pw.p2 - 0.05, 3) } else { round_dec(pw.p2 + 0.05, 3) };
                }
                pw.effect_mode = PowerEffectInputMode::Proportions;
                buf_sync = Some(("power.pilot.p2.input", format!("{:.2}", pw.p2)));
            }
            "pilot_hr" => {
                let mut next = round_dec((state.survival_hazard_ratio + delta).clamp(0.10, 5.0), 2);
                if (next - 1.0).abs() < 0.03 {
                    next = if delta >= 0.0 { 1.05 } else { 0.95 };
                }
                state.survival_hazard_ratio = next;
                sync_survival_proportions(pw, state);
                buf_sync = Some(("power.pilot.hr.input", format!("{:.2}", state.survival_hazard_ratio)));
            }
            "pilot_evprob" => {
                state.survival_event_prob = round_dec((state.survival_event_prob + delta).clamp(0.05, 0.95), 2);
                sync_survival_proportions(pw, state);
                buf_sync = Some(("power.pilot.evprob.input", format!("{:.2}", state.survival_event_prob)));
            }
            "pilot_r2" => {
                pw.r_squared = round_dec((pw.r_squared + delta).clamp(0.01, 0.95), 3);
                pw.effect_mode = PowerEffectInputMode::RegressionRSquared;
                buf_sync = Some(("power.pilot.r2.input", format!("{:.2}", pw.r_squared)));
            }
            "pilot_std" => {
                let (lo, hi) = effect_range_for_family(pw.family);
                pw.standardized_effect = round_dec((pw.standardized_effect + delta).clamp(lo, hi), 3);
                pw.effect_mode = PowerEffectInputMode::Standardized;
                buf_sync = Some(("power.pilot.std.input", format!("{:.2}", pw.standardized_effect)));
            }
            _ if field.starts_with("pilot_group_") => {
                let idx: usize = field.strip_prefix("pilot_group_").and_then(|s| s.parse().ok()).unwrap_or(0);
                if let Some(slot) = pw.group_means.get_mut(idx) {
                    *slot = round_dec(*slot + delta, 2);
                }
                pw.effect_mode = PowerEffectInputMode::MeansAndSd;
            }
            _ => return Err(format!("unknown spinner field `{field}`")),
        }
    }
    if let Some((buf_id, buf_val)) = buf_sync {
        sync_text_buffer(app, buf_id, &buf_val);
    }
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    Ok(())
}

fn handle_slider_action(app: &mut UiApp, state: &mut PowerWizardViewState, field: &str) -> Result<UiEventOutcome, String> {
    let widget_id = format!("power.sidebar.{field}.slider");
    let t = resolve_slider_normalized(app, &widget_id);
    let pw = &mut app.canvas_dialogs.power_wizard;
    match field {
        "alpha" => {
            pw.alpha = round_dec(0.001 + t * (0.20 - 0.001), 4);
            if pw.target_power <= pw.alpha + 0.02 {
                pw.target_power = (pw.alpha + 0.05).min(0.95);
            }
        }
        "target_power" => {
            let min_p = (pw.alpha + 0.02).max(0.50);
            pw.target_power = round_dec(min_p + t * (0.99 - min_p), 3);
        }
        "effect_size" => {
            let (lo, hi) = effect_range_for_family(pw.family);
            let next = round_dec(lo + t * (hi - lo), 3);
            apply_standardized_effect_update(pw, state, next);
        }
        "allocation_ratio" => {
            pw.allocation_ratio = round_dec(0.25 + t * (4.0 - 0.25), 2);
        }
        "dropout" => {
            state.dropout_rate = round_dec(t * 0.50, 3);
        }
        "sample_size" => {
            let min_n = pw.family.min_integer_n() as f64;
            pw.sample_size = (min_n + t * (300.0 - min_n)).round().clamp(min_n, 5000.0);
        }
        _ => return Err(format!("unknown slider field `{field}`")),
    }
    let _ = pw.recalculate();
    state.last_error = None;
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.slider:{field}")))
}

fn handle_hover_frac_action(app: &mut UiApp, state: &mut PowerWizardViewState, raw_frac: &str) -> Result<UiEventOutcome, String> {
    let frac: f64 = raw_frac.trim().parse().map_err(|_| format!("invalid hover fraction `{raw_frac}`"))?;
    if let Some((hn, hp)) = app.canvas_dialogs.power_wizard.hover_at_fraction(frac.clamp(0.0, 1.0)) {
        state.hover_sample_size = Some(hn.round().max(1.0) as u32);
        state.hover_power = Some(hp);
    }
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome("power_wizard.hover"))
}

fn handle_select_sens_action(app: &mut UiApp, state: &mut PowerWizardViewState, raw_idx: &str) -> Result<UiEventOutcome, String> {
    let idx: usize = raw_idx.trim().parse().map_err(|_| format!("invalid sensitivity index `{raw_idx}`"))?;
    let Some(cell) = app.canvas_dialogs.power_wizard.sensitivity_cells().get(idx).copied() else {
        return Err(format!("sensitivity cell index `{idx}` out of bounds"));
    };
    app.canvas_dialogs.power_wizard.alpha = cell.alpha;
    app.canvas_dialogs.power_wizard.target_power = cell.target_power;
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.select_sens:{idx}")))
}

fn handle_quick_preset_action(app: &mut UiApp, state: &mut PowerWizardViewState, rest: &str) -> Result<UiEventOutcome, String> {
    let Some((kind, raw_val)) = rest.split_once(':') else {
        return Err(format!("invalid preset `{rest}`"));
    };
    let val: f64 = raw_val.parse().map_err(|_| format!("invalid preset value `{raw_val}`"))?;
    let pw = &mut app.canvas_dialogs.power_wizard;
    match kind {
        "alpha" => pw.alpha = val.clamp(0.001, 0.25),
        "power" => pw.target_power = val.clamp((pw.alpha + 0.02).max(0.50), 0.99),
        "alloc" => pw.allocation_ratio = val.clamp(0.25, 4.0),
        "dropout" => state.dropout_rate = val.clamp(0.0, 0.75),
        _ => return Err(format!("unknown preset kind `{kind}`")),
    }
    let _ = pw.recalculate();
    state.revision = state.revision.wrapping_add(1);
    Ok(handled_outcome(format!("power_wizard.preset:{kind}")))
}

fn resolve_slider_normalized(app: &UiApp, widget_id: &str) -> f64 {
    let Some(frame) = &app.retained_layout.0 else {
        return 0.5;
    };
    let Some(node) = frame.tree.node(widget_id) else {
        return 0.5;
    };
    let Some([lx, _]) = app.interaction.last_logical_pos else {
        return 0.5;
    };
    let r = node.bounds.rect_logical;
    let track_x = r[0] + 6.0;
    let track_w = (r[2] - 12.0).max(1.0);
    let raw = ((lx - track_x) / track_w).clamp(0.0, 1.0);
    if frame.tree.is_rtl { 1.0 - raw } else { raw }
}

fn sync_text_buffer(app: &mut UiApp, id: &str, value: &str) {
    app.interaction.text_field_buffers.insert(id.to_owned(), TextFieldEditState::new(value, false));
}

fn clear_pilot_text_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.retain(|k, _| !k.starts_with("power.pilot."));
}

fn sync_survival_proportions(pw: &mut PowerWizardState, state: &PowerWizardViewState) {
    let p1 = state.survival_event_prob.clamp(0.05, 0.95);
    let s1 = (1.0 - p1).clamp(0.01, 0.99);
    let s2 = s1.powf(state.survival_hazard_ratio.clamp(0.10, 5.0)).clamp(0.01, 0.99);
    let p2 = (1.0 - s2).clamp(0.01, 0.99);
    pw.p1 = p1;
    pw.p2 = p2;
    pw.effect_mode = PowerEffectInputMode::Proportions;
    if let Ok(h) = cohens_h(p1, p2) {
        pw.standardized_effect = h.abs().max(0.05);
    }
}

fn resize_anova_groups(pw: &mut PowerWizardState, next_k: usize) {
    pw.anova_groups = next_k;
    if pw.group_means.len() < next_k {
        let base = pw.group_means.first().copied().unwrap_or(0.0);
        while pw.group_means.len() < next_k {
            let idx = pw.group_means.len() as f64;
            pw.group_means.push(round_dec(base + idx * 0.4, 2));
        }
    } else if pw.group_means.len() > next_k {
        pw.group_means.truncate(next_k);
    }
}

fn apply_standardized_effect_update(pw: &mut PowerWizardState, state: &mut PowerWizardViewState, next: f64) {
    pw.standardized_effect = next;
    match pw.effect_mode {
        PowerEffectInputMode::Standardized => {}
        PowerEffectInputMode::MeansAndSd => {
            if matches!(pw.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT) {
                let sd_eff = if pw.family == PowerTestFamily::TwoSamplePairedT { pw.computed_paired_sigma_d().unwrap_or(pw.sd) } else { pw.sd };
                pw.mean_diff = round_dec(next * sd_eff, 4);
                state.pilot_mean2 = round_dec(state.pilot_mean1 + pw.mean_diff, 3);
            } else {
                pw.effect_mode = PowerEffectInputMode::Standardized;
            }
        }
        PowerEffectInputMode::Proportions => {
            let angle1 = pw.p1.clamp(0.02, 0.98).sqrt().asin();
            let dir = if pw.p2 >= pw.p1 { 1.0 } else { -1.0 };
            let angle2 = (angle1 + dir * 0.5 * next).clamp(0.08, std::f64::consts::FRAC_PI_2 - 0.08);
            let s = angle2.sin();
            pw.p2 = round_dec((s * s).clamp(0.02, 0.98), 3);
        }
        PowerEffectInputMode::RegressionRSquared => {
            pw.r_squared = round_dec((next / (1.0 + next)).clamp(0.01, 0.95), 3);
        }
        _ => {
            pw.effect_mode = PowerEffectInputMode::Standardized;
        }
    }
}

fn round_dec(val: f64, places: i32) -> f64 {
    let factor = 10.0_f64.powi(places);
    (val * factor).round() / factor
}

const fn effect_range_for_family(family: PowerTestFamily) -> (f64, f64) {
    match family {
        PowerTestFamily::Correlation => (0.05, 0.92),
        PowerTestFamily::OneWayAnova => (0.05, 1.25),
        PowerTestFamily::SimpleLinearRegression => (0.02, 1.20),
        _ => (0.05, 2.00),
    }
}

fn effective_effect_display(pw: &PowerWizardState, state: &PowerWizardViewState) -> f64 {
    let _ = state;
    pw.derived_standardized_effect().map(f64::abs).unwrap_or(pw.standardized_effect.abs())
}

fn slider_norm(val: f64, lo: f64, hi: f64) -> f64 {
    if (hi - lo).abs() < 1e-12 || !val.is_finite() {
        return 0.5;
    }
    ((val - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn dropout_adjusted_n(n: usize, dropout_rate: f64) -> usize {
    let retain = (1.0 - dropout_rate.clamp(0.0, 0.75)).max(0.25);
    ((n as f64) / retain).ceil() as usize
}

fn schoenfeld_events(alpha: f64, power: f64, two_tailed: bool, alloc_ratio: f64, hr: f64) -> Option<usize> {
    let ln_hr = hr.ln();
    if !ln_hr.is_finite() || ln_hr.abs() < 1e-6 {
        return None;
    }
    let a_tail = if two_tailed { alpha * 0.5 } else { alpha };
    let z_a = norm_ppf(1.0 - a_tail.clamp(1e-6, 0.49)).ok()?;
    let z_b = norm_ppf(power.clamp(0.05, 0.9999)).ok()?;
    let k = alloc_ratio.max(0.05);
    let factor = (1.0 + k).powi(2) / k;
    let events = ((z_a + z_b).powi(2) * factor) / (ln_hr * ln_hr);
    events.is_finite().then_some(events.ceil().max(1.0) as usize)
}

fn scaled(logical: f64, scale: f64) -> u32 {
    (logical * scale).round().max(1.0) as u32
}

fn check_stage_gate(pw: &PowerWizardState, from_stage: PowerWizardStage) -> Result<(), String> {
    if !pw.alpha.is_finite() || !(0.0..1.0).contains(&pw.alpha) || pw.alpha <= 0.0 {
        return Err(String::from("Significance alpha must lie in (0, 1)."));
    }
    if pw.solve_target != PowerSolveTarget::Power && (!pw.target_power.is_finite() || pw.target_power <= pw.alpha || pw.target_power >= 1.0) {
        return Err(String::from("Target power (1 - beta) must exceed alpha and be strictly less than 1.0."));
    }
    if from_stage == PowerWizardStage::EffectSize
        && pw.solve_target != PowerSolveTarget::DetectableEffect
        && pw.derived_standardized_effect().is_none_or(|v| v == 0.0)
    {
        return Err(String::from("Enter a non-zero finite effect size or raw group parameters."));
    }
    if matches!(from_stage, PowerWizardStage::SignificanceAndSolve | PowerWizardStage::Results)
        && let Some(ref err) = pw.validation_error
    {
        return Err(err.clone());
    }
    Ok(())
}

fn compute_stage_enablements(pw: &PowerWizardState) -> [ControlEnablement; 4] {
    let cur = pw.stage.index();
    let stages = PowerWizardStage::ALL;
    let mut out = [ControlEnablement::enabled(), ControlEnablement::enabled(), ControlEnablement::enabled(), ControlEnablement::enabled()];
    let mut blocked_reason: Option<String> = None;
    for target_idx in (cur + 1)..stages.len() {
        if blocked_reason.is_none()
            && let Err(reason) = check_stage_gate(pw, stages[target_idx - 1])
        {
            blocked_reason = Some(reason);
        }
        if let Some(ref reason) = blocked_reason {
            out[target_idx] = ControlEnablement::disabled(reason.clone());
        }
    }
    out
}

/// Build the complete 4-stage Power & Sample Size wizard body inside `body_rect`.
pub fn build_power_wizard_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let s = builder.scale();
    let stepper_h = scaled(26.0, s).min(body_rect.height / 4);
    let gap = scaled(6.0, s);
    let stepper_rect = LayoutRectPx::new(body_rect.x, body_rect.y, body_rect.width, stepper_h);
    build_stage_stepper(builder, &parent, stepper_rect);

    let content_y = body_rect.y + i32::try_from(stepper_h + gap).unwrap_or(32);
    let content_h = body_rect.height.saturating_sub(stepper_h + gap);
    if content_h < scaled(48.0, s) || body_rect.width < scaled(180.0, s) {
        return;
    }

    let sidebar_w = if body_rect.width >= scaled(520.0, s) {
        (body_rect.width / 3).clamp(scaled(200.0, s), scaled(280.0, s))
    } else {
        (body_rect.width * 2 / 5).max(scaled(120.0, s))
    };
    let main_w = body_rect.width.saturating_sub(sidebar_w + gap);
    let is_rtl = builder.tree.is_rtl;
    let (main_x, side_x) = if is_rtl {
        (body_rect.x + i32::try_from(sidebar_w + gap).unwrap_or(0), body_rect.x)
    } else {
        (body_rect.x, body_rect.x + i32::try_from(main_w + gap).unwrap_or(0))
    };
    let main_rect = LayoutRectPx::new(main_x, content_y, main_w, content_h);
    let side_rect = LayoutRectPx::new(side_x, content_y, sidebar_w, content_h);

    build_main_stage_panel(builder, &parent, main_rect);
    build_live_sidebar(builder, &parent, side_rect);
}

fn build_stage_stepper(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let locale = builder.tree.locale.clone();
    let stages = PowerWizardStage::ALL;
    let enablements = compute_stage_enablements(pw);
    let count = stages.len() as u32;
    let gap = 4u32;
    let tab_w = ((rect.width.saturating_sub(gap * (count - 1))) / count).max(48);
    let keys = ["pw.stage.1", "pw.stage.2", "pw.stage.3", "pw.stage.4"];

    for (i, st) in stages.iter().enumerate() {
        let local_x = (i as u32) * (tab_w + gap);
        let draw_x = if builder.tree.is_rtl {
            rect.right() - i32::try_from(local_x + tab_w).unwrap_or(0)
        } else {
            rect.x + i32::try_from(local_x).unwrap_or(0)
        };
        let label = wiz_tr(&locale, keys[i], st.label());
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("power.stage.{}", i + 1),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(draw_x, rect.y, tab_w, rect.height),
                label,
                keyboard_path: &format!("power.stage.{}", i + 1),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + i as u32),
                primary: false,
                selected: pw.stage == *st,
                shortcut: None,
                enablement: enablements[i].clone(),
                action: custom_action(format!("stage:{}", i + 1)),
            },
        );
    }
}

fn build_main_stage_panel(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    builder.paint_fill(ZLayer::ModalDialog, Some(parent), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(parent), rect, builder.palette.border, 1);

    let inner = rect.inset(6);
    if inner.width < 80 || inner.height < 40 {
        return;
    }

    match builder.app.canvas_dialogs.power_wizard.stage {
        PowerWizardStage::ExperimentDesign => build_stage1_design(builder, parent, inner),
        PowerWizardStage::EffectSize => build_stage2_effect(builder, parent, inner),
        PowerWizardStage::SignificanceAndSolve => build_stage3_significance(builder, parent, inner),
        PowerWizardStage::Results => build_stage4_results(builder, parent, inner),
    }
}

fn paint_section_heading(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, text: &str) {
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect,
            pad_x: 2,
            text,
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
}

fn build_stage1_design(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let s = builder.scale();
    let locale = builder.tree.locale.clone();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let survival_active = builder.app.view_shards.power_wizard.survival_log_rank && pw.family == PowerTestFamily::TwoProportions;

    let hdr_h = scaled(18.0, s);
    paint_section_heading(
        builder,
        parent,
        LayoutRectPx::new(rect.x, rect.y, rect.width, hdr_h),
        wiz_tr(&locale, "pw.section.families", "Statistical Test Family"),
    );

    let grid_y = rect.y + i32::try_from(hdr_h + 4).unwrap_or(22);
    let cols = 3u32;
    let rows = 3u32;
    let gap = scaled(6.0, s);
    let card_w = ((rect.width.saturating_sub(gap * (cols - 1))) / cols).max(60);
    let avail_grid_h = rect.height.saturating_sub(hdr_h + scaled(108.0, s)).max(scaled(120.0, s));
    let card_h = ((avail_grid_h.saturating_sub(gap * (rows - 1))) / rows).clamp(scaled(38.0, s), scaled(68.0, s));

    let title_sz = (11.5 * s).clamp(8.5, 20.0);
    let sub_sz = (10.0 * s).clamp(8.0, 18.0);

    for (idx, card) in FAMILY_CARDS.iter().enumerate() {
        let r = (idx as u32) / cols;
        let c = (idx as u32) % cols;
        let cx = rect.x + i32::try_from(c * (card_w + gap)).unwrap_or(0);
        let cy = grid_y + i32::try_from(r * (card_h + gap)).unwrap_or(0);
        let card_rect = LayoutRectPx::new(cx, cy, card_w, card_h);
        if card_rect.bottom() > rect.bottom() {
            continue;
        }
        let selected = if card.is_survival {
            survival_active
        } else if card.family == PowerTestFamily::TwoProportions {
            pw.family == PowerTestFamily::TwoProportions && !survival_active
        } else {
            pw.family == card.family
        };

        let id = format!("power.family.{}", card.key);
        let st = widget_state(builder, &id, ControlEnablement::enabled());
        let mut a11y = selectable_meta(WidgetRole::RadioButton, card.title, FocusScope::ModalDialog, &id, selected);
        a11y.checked = Some(selected);
        a11y.tab_order = Some(20 + idx as u32);

        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(id),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: card_rect,
            a11y,
            state: st.clone(),
            action: custom_action(format!("family:{}", card.key)),
        });

        let bg = if selected {
            builder.palette.surface
        } else if st.hovered {
            builder.palette.header
        } else {
            builder.palette.surface
        };
        let border_col = if selected || st.focused { builder.palette.accent } else { builder.palette.border };
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), card_rect, bg);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), card_rect, border_col, if selected { 2 } else { 1 });
        if selected {
            let strip = LayoutRectPx::new(card_rect.x + 2, card_rect.y + 2, 4, card_rect.height.saturating_sub(4));
            builder.paint_fill(ZLayer::ModalDialog, Some(&wid), strip, builder.palette.accent);
        }
        maybe_paint_focus_ring(builder, ZLayer::ModalDialog, &wid, card_rect, st.focused);

        let title_h = (card_h / 2).max(12);
        let title_rect = LayoutRectPx::new(card_rect.x + 8, card_rect.y + 3, card_rect.width.saturating_sub(12), title_h);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: title_rect,
                pad_x: 2,
                text: card.title,
                style: &if selected { UiTextStyle::ui_bold(title_sz) } else { UiTextStyle::ui_sans(title_sz) },
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        let sub_h = card_h.saturating_sub(title_h + 6);
        if sub_h >= 10 {
            let sub_rect = LayoutRectPx::new(card_rect.x + 8, title_rect.bottom() + 1, card_rect.width.saturating_sub(12), sub_h);
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: sub_rect,
                    pad_x: 2,
                    text: card.subtitle,
                    style: &UiTextStyle::ui_sans(sub_sz),
                    color: builder.palette.muted,
                    scroll_container: None,
                },
            );
        }
    }

    let mut next_y = grid_y + i32::try_from(rows * card_h + (rows - 1) * gap + scaled(10.0, s)).unwrap_or(180);
    if next_y + i32::try_from(scaled(46.0, s)).unwrap_or(46) <= rect.bottom() {
        paint_section_heading(
            builder,
            parent,
            LayoutRectPx::new(rect.x, next_y, rect.width, hdr_h),
            wiz_tr(&locale, "pw.section.solve_target", "Solve For"),
        );
        next_y += i32::try_from(hdr_h + 4).unwrap_or(22);

        let targets = [
            ("sample_size", "Sample Size (N)", PowerSolveTarget::SampleSize),
            ("power", "Power (1 - beta)", PowerSolveTarget::Power),
            ("effect", "Detectable Effect", PowerSolveTarget::DetectableEffect),
        ];
        let btn_w = ((rect.width.saturating_sub(gap * 2)) / 3).max(60);
        let btn_h = scaled(24.0, s);
        for (idx, (key, lbl, val)) in targets.iter().enumerate() {
            let bx = rect.x + i32::try_from((idx as u32) * (btn_w + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.solve.{key}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, next_y, btn_w, btn_h),
                    label: lbl,
                    keyboard_path: &format!("power.solve.{key}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(35 + idx as u32),
                    primary: false,
                    selected: pw.solve_target == *val,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("solve:{key}")),
                },
            );
        }
        next_y += i32::try_from(btn_h + scaled(8.0, s)).unwrap_or(32);
    }

    let row_h = scaled(22.0, s);
    if next_y + i32::try_from(row_h).unwrap_or(22) <= rect.bottom() {
        if pw.family == PowerTestFamily::OneWayAnova {
            emit_labeled_spinner_row(
                builder,
                parent,
                LayoutRectPx::new(rect.x, next_y, rect.width, row_h),
                LabeledSpinnerSpec {
                    id_prefix: "power.design.anova_groups",
                    field_key: "anova_groups",
                    label: "ANOVA Groups (k)",
                    value_text: &pw.anova_groups.to_string(),
                    step: 1.0,
                    tab_order: 42,
                    enablement: ControlEnablement::enabled(),
                },
            );
        } else if pw.family == PowerTestFamily::ChiSquare {
            emit_labeled_spinner_row(
                builder,
                parent,
                LayoutRectPx::new(rect.x, next_y, rect.width, row_h),
                LabeledSpinnerSpec {
                    id_prefix: "power.design.chi2_df",
                    field_key: "chi2_df",
                    label: "Degrees of Freedom (df)",
                    value_text: &format!("{:.0}", pw.chi_square_df),
                    step: 1.0,
                    tab_order: 42,
                    enablement: ControlEnablement::enabled(),
                },
            );
        }
    }
}

fn build_stage2_effect(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let s = builder.scale();
    let locale = builder.tree.locale.clone();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let vs = &builder.app.view_shards.power_wizard;
    let survival_active = vs.survival_log_rank && pw.family == PowerTestFamily::TwoProportions;

    let mut y = rect.y;
    let hdr_h = scaled(18.0, s);
    paint_section_heading(
        builder,
        parent,
        LayoutRectPx::new(rect.x, y, rect.width, hdr_h),
        wiz_tr(&locale, "pw.section.pilot_calc", "Pilot-Data Effect-Size Calculator"),
    );
    y += i32::try_from(hdr_h + 4).unwrap_or(22);

    // Input Mode strip
    let modes = effect_modes_for_family(pw.family, survival_active);
    let btn_h = scaled(22.0, s);
    if !modes.is_empty() && y + i32::try_from(btn_h).unwrap_or(22) <= rect.bottom() {
        let gap = 4u32;
        let mw = ((rect.width.saturating_sub(gap * (modes.len() as u32 - 1))) / (modes.len() as u32)).max(72);
        for (i, (key, lbl, mode_val)) in modes.iter().enumerate() {
            let mx = rect.x + i32::try_from((i as u32) * (mw + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.effect_mode.{key}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(mx, y, mw, btn_h),
                    label: lbl,
                    keyboard_path: &format!("power.effect_mode.{key}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(20 + i as u32),
                    primary: false,
                    selected: pw.effect_mode == *mode_val,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("effect_mode:{key}")),
                },
            );
        }
        y += i32::try_from(btn_h + 6).unwrap_or(28);
    }

    // Pilot Calculator card
    let calc_h = (rect.bottom() - y - i32::try_from(scaled(74.0, s)).unwrap_or(74)).max(scaled(110.0, s) as i32) as u32;
    let calc_rect = LayoutRectPx::new(rect.x, y, rect.width, calc_h.min(rect.bottom().saturating_sub(y) as u32));
    builder.paint_fill(ZLayer::ModalDialog, Some(parent), calc_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(parent), calc_rect, builder.palette.border, 1);

    let calc_inner = calc_rect.inset(6);
    let row_h = scaled(23.0, s);
    let row_gap = scaled(4.0, s);
    let mut cy = calc_inner.y;

    match pw.family {
        PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => {
            let m2_lbl = if pw.family == PowerTestFamily::OneSampleT { "Null Mean (mu0)" } else { "Group 2 Mean" };
            let m1_lbl = if pw.family == PowerTestFamily::OneSampleT { "Sample Mean" } else { "Group 1 Mean" };
            let rows_data: [(&str, &str, &str, String, f64); 3] = [
                ("power.pilot.mean1", "pilot_mean1", m1_lbl, format!("{:.2}", vs.pilot_mean1), 0.2),
                ("power.pilot.mean2", "pilot_mean2", m2_lbl, format!("{:.2}", vs.pilot_mean2), 0.2),
                ("power.pilot.sd", "pilot_sd", "Common SD (sigma)", format!("{:.2}", pw.sd), 0.1),
            ];
            for (idx, (prefix, fkey, lbl, val, step)) in rows_data.iter().enumerate() {
                if cy + i32::try_from(row_h).unwrap_or(23) > calc_inner.bottom() {
                    break;
                }
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec { id_prefix: prefix, field_key: fkey, label: lbl, value_text: val, step: *step, tab_order: 30 + (idx as u32) * 3 },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
            if pw.family == PowerTestFamily::TwoSamplePairedT && cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.corr",
                        field_key: "pilot_corr",
                        label: "Within-Pair r",
                        value_text: &format!("{:.2}", pw.within_pair_correlation),
                        step: 0.05,
                        tab_order: 40,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
        PowerTestFamily::OneWayAnova => {
            for g in 0..pw.anova_groups.min(4) {
                if cy + i32::try_from(row_h).unwrap_or(23) > calc_inner.bottom() {
                    break;
                }
                let val = pw.group_means.get(g).copied().unwrap_or(0.0);
                emit_labeled_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    LabeledSpinnerSpec {
                        id_prefix: &format!("power.pilot.group_{g}"),
                        field_key: &format!("pilot_group_{g}"),
                        label: &format!("Group {} Mean", g + 1),
                        value_text: &format!("{val:.2}"),
                        step: 0.2,
                        tab_order: 30 + (g as u32) * 2,
                        enablement: ControlEnablement::enabled(),
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.sd",
                        field_key: "pilot_sd",
                        label: "Within-Group SD",
                        value_text: &format!("{:.2}", pw.sd),
                        step: 0.1,
                        tab_order: 42,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
        PowerTestFamily::TwoProportions if survival_active => {
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.hr",
                        field_key: "pilot_hr",
                        label: "Hazard Ratio (HR)",
                        value_text: &format!("{:.2}", vs.survival_hazard_ratio),
                        step: 0.05,
                        tab_order: 30,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.evprob",
                        field_key: "pilot_evprob",
                        label: "Control Event Rate",
                        value_text: &format!("{:.2}", vs.survival_event_prob),
                        step: 0.05,
                        tab_order: 33,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
        PowerTestFamily::TwoProportions => {
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.p1",
                        field_key: "pilot_p1",
                        label: "Control Rate (p1)",
                        value_text: &format!("{:.2}", pw.p1),
                        step: 0.05,
                        tab_order: 30,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.p2",
                        field_key: "pilot_p2",
                        label: "Treatment Rate (p2)",
                        value_text: &format!("{:.2}", pw.p2),
                        step: 0.05,
                        tab_order: 33,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
        PowerTestFamily::SimpleLinearRegression => {
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.r2",
                        field_key: "pilot_r2",
                        label: "Model R-Squared",
                        value_text: &format!("{:.2}", pw.r_squared),
                        step: 0.05,
                        tab_order: 30,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
        PowerTestFamily::Correlation | PowerTestFamily::ChiSquare => {
            if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
                emit_pilot_input_spinner_row(
                    builder,
                    parent,
                    LayoutRectPx::new(calc_inner.x, cy, calc_inner.width, row_h),
                    PilotRowSpec {
                        id_prefix: "power.pilot.std",
                        field_key: "pilot_std",
                        label: pw.family.default_effect_label(),
                        value_text: &format!("{:.2}", pw.standardized_effect),
                        step: 0.05,
                        tab_order: 30,
                    },
                );
                cy += i32::try_from(row_h + row_gap).unwrap_or(27);
            }
        }
    }

    // Apply typed pilot input button + Derived Effect Size banner inside calculator
    if cy + i32::try_from(row_h).unwrap_or(23) <= calc_inner.bottom() {
        let apply_w = scaled(140.0, s).min(calc_inner.width / 2);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "power.pilot.apply",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(calc_inner.x, cy, apply_w, row_h),
                label: wiz_tr(&locale, "pw.pilot.apply", "Apply Pilot Inputs"),
                keyboard_path: "power.pilot.apply",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(48),
                primary: true,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action("pilot_apply"),
            },
        );
        let derived_rect =
            LayoutRectPx::new(calc_inner.x + i32::try_from(apply_w + 8).unwrap_or(148), cy, calc_inner.width.saturating_sub(apply_w + 8), row_h);
        let derived_txt = format_derived_effect_summary(pw, vs);
        let sz = (11.5 * s).clamp(8.5, 20.0);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: derived_rect,
                pad_x: 4,
                text: &derived_txt,
                style: &UiTextStyle::ui_bold(sz),
                color: builder.palette.accent,
                scroll_container: None,
            },
        );
    }

    // Benchmarks strip below calculator card
    let bench_y = calc_rect.bottom() + i32::try_from(scaled(8.0, s)).unwrap_or(8);
    if bench_y + i32::try_from(scaled(44.0, s)).unwrap_or(44) <= rect.bottom() {
        paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, bench_y, rect.width, hdr_h), "Standardized Benchmarks (Cohen)");
        let py = bench_y + i32::try_from(hdr_h + 4).unwrap_or(22);
        let gap = 6u32;
        let pw_btn = ((rect.width.saturating_sub(gap * 2)) / 3).max(60);
        for (i, (key, lbl)) in [("small", "Small Effect"), ("medium", "Medium Effect"), ("large", "Large Effect")].iter().enumerate() {
            let bx = rect.x + i32::try_from((i as u32) * (pw_btn + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.pilot.preset.{key}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, py, pw_btn, btn_h),
                    label: lbl,
                    keyboard_path: &format!("power.pilot.preset.{key}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(50 + i as u32),
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("pilot_preset:{key}")),
                },
            );
        }
    }
}

fn effect_modes_for_family(family: PowerTestFamily, survival_active: bool) -> Vec<(&'static str, &'static str, PowerEffectInputMode)> {
    if survival_active {
        return vec![
            ("proportions", "Hazard Ratio & Events", PowerEffectInputMode::Proportions),
            ("standardized", "Standardized (h)", PowerEffectInputMode::Standardized),
        ];
    }
    match family {
        PowerTestFamily::TwoSamplePairedT => vec![
            ("standardized", "Standardized (dz)", PowerEffectInputMode::Standardized),
            ("means_and_sd", "Pilot Means, SD & r", PowerEffectInputMode::MeansAndSd),
        ],
        PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::OneSampleT | PowerTestFamily::OneWayAnova => vec![
            ("standardized", "Standardized", PowerEffectInputMode::Standardized),
            ("means_and_sd", "Pilot Means & SD", PowerEffectInputMode::MeansAndSd),
        ],
        PowerTestFamily::TwoProportions => vec![
            ("proportions", "Proportions (p1, p2)", PowerEffectInputMode::Proportions),
            ("standardized", "Cohen's h", PowerEffectInputMode::Standardized),
        ],
        PowerTestFamily::SimpleLinearRegression => vec![
            ("standardized", "Cohen's f^2", PowerEffectInputMode::Standardized),
            ("r_squared", "R-Squared (R^2)", PowerEffectInputMode::RegressionRSquared),
        ],
        PowerTestFamily::Correlation | PowerTestFamily::ChiSquare => {
            vec![("standardized", family.default_effect_label(), PowerEffectInputMode::Standardized)]
        }
    }
}

fn format_derived_effect_summary(pw: &PowerWizardState, vs: &PowerWizardViewState) -> String {
    let eff = pw.derived_standardized_effect().unwrap_or(pw.standardized_effect);
    if vs.survival_log_rank && pw.family == PowerTestFamily::TwoProportions {
        let ev = schoenfeld_events(pw.alpha, pw.target_power, pw.two_tailed, pw.allocation_ratio, vs.survival_hazard_ratio);
        return match ev {
            Some(e) => format!("HR = {:.2} -> h = {:.3} (~{} events)", vs.survival_hazard_ratio, eff.abs(), e),
            None => format!("HR = {:.2} -> h = {:.3}", vs.survival_hazard_ratio, eff.abs()),
        };
    }
    if pw.family == PowerTestFamily::TwoSamplePairedT
        && pw.effect_mode == PowerEffectInputMode::MeansAndSd
        && let Some(sd_d) = pw.computed_paired_sigma_d()
    {
        return format!("sigma_d = {sd_d:.3} | dz = {eff:.3}");
    }
    format!("Derived {}: {eff:.3}", pw.family.default_effect_label())
}

fn build_stage3_significance(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let s = builder.scale();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let vs = &builder.app.view_shards.power_wizard;
    let hdr_h = scaled(18.0, s);
    let row_h = scaled(24.0, s);
    let gap = scaled(6.0, s);
    let mut y = rect.y;

    paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, y, rect.width, hdr_h), "Hypothesis Tail Mode");
    y += i32::try_from(hdr_h + 4).unwrap_or(22);

    let half_w = (rect.width.saturating_sub(gap)) / 2;
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "power.two_tailed",
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x, y, half_w, row_h),
            label: "Two-Tailed Test",
            keyboard_path: "power.two_tailed",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(20),
            checked: pw.two_tailed,
            is_radio: true,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            action: custom_action("tail:two"),
        },
    );
    let one_en = evaluate_dialog_widget(&builder.app.canvas_dialogs, builder.app.document(), "power.one_sided");
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "power.one_sided",
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x + i32::try_from(half_w + gap).unwrap_or(100), y, half_w, row_h),
            label: "One-Sided Test",
            keyboard_path: "power.one_sided",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(21),
            checked: !pw.two_tailed,
            is_radio: true,
            enablement: one_en.clone(),
            scroll_container: None,
            action: custom_action(if pw.one_sided_greater { "tail:greater" } else { "tail:less" }),
        },
    );
    y += i32::try_from(row_h + gap).unwrap_or(30);

    if !pw.two_tailed && one_en.enabled && y + i32::try_from(row_h).unwrap_or(24) <= rect.bottom() {
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "power.tail.greater",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x, y, half_w, row_h),
                label: "Direction: Greater (H1 > 0)",
                keyboard_path: "power.tail.greater",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(22),
                primary: false,
                selected: pw.one_sided_greater,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action("tail:greater"),
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "power.tail.less",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x + i32::try_from(half_w + gap).unwrap_or(100), y, half_w, row_h),
                label: "Direction: Less (H1 < 0)",
                keyboard_path: "power.tail.less",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(23),
                primary: false,
                selected: !pw.one_sided_greater,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action("tail:less"),
            },
        );
        y += i32::try_from(row_h + gap).unwrap_or(30);
    }

    // Alpha presets
    if y + i32::try_from(hdr_h + row_h + 6).unwrap_or(48) <= rect.bottom() {
        paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, y, rect.width, hdr_h), "Significance Presets (Alpha)");
        y += i32::try_from(hdr_h + 2).unwrap_or(20);
        let alphas = [("010", "0.10", 0.10), ("005", "0.05", 0.05), ("001", "0.01", 0.01), ("0001", "0.001", 0.001)];
        let bw = ((rect.width.saturating_sub(gap * 3)) / 4).max(48);
        for (i, (k, lbl, v)) in alphas.iter().enumerate() {
            let bx = rect.x + i32::try_from((i as u32) * (bw + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.stage3.alpha.{k}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, y, bw, row_h),
                    label: lbl,
                    keyboard_path: &format!("power.stage3.alpha.{k}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(25 + i as u32),
                    primary: false,
                    selected: (pw.alpha - *v).abs() < 1e-4,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("preset:alpha:{v}")),
                },
            );
        }
        y += i32::try_from(row_h + gap).unwrap_or(30);
    }

    // Target power presets
    if y + i32::try_from(hdr_h + row_h + 6).unwrap_or(48) <= rect.bottom() {
        paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, y, rect.width, hdr_h), "Target Power Presets (1 - beta)");
        y += i32::try_from(hdr_h + 2).unwrap_or(20);
        let powers = [("70", "70%", 0.70), ("80", "80%", 0.80), ("90", "90%", 0.90), ("95", "95%", 0.95)];
        let bw = ((rect.width.saturating_sub(gap * 3)) / 4).max(48);
        let p_en = if pw.solve_target == PowerSolveTarget::Power {
            ControlEnablement::disabled("Target power is solved automatically when Solve For = Power.")
        } else {
            ControlEnablement::enabled()
        };
        for (i, (k, lbl, v)) in powers.iter().enumerate() {
            let bx = rect.x + i32::try_from((i as u32) * (bw + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.stage3.power.{k}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, y, bw, row_h),
                    label: lbl,
                    keyboard_path: &format!("power.stage3.power.{k}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(30 + i as u32),
                    primary: false,
                    selected: (pw.target_power - *v).abs() < 1e-3,
                    shortcut: None,
                    enablement: p_en.clone(),
                    action: custom_action(format!("preset:power:{v}")),
                },
            );
        }
        y += i32::try_from(row_h + gap).unwrap_or(30);
    }

    // Method-specific controls
    if pw.family == PowerTestFamily::TwoProportions
        && !vs.survival_log_rank
        && y + i32::try_from(hdr_h + row_h * 2 + 12).unwrap_or(68) <= rect.bottom()
    {
        paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, y, rect.width, hdr_h), "Proportions Variance Method");
        y += i32::try_from(hdr_h + 2).unwrap_or(20);
        let methods = [
            ("pooled", "Fleiss Pooled", TwoProportionsMethod::PooledNullUnpooledAlt),
            ("unpooled", "Wald Unpooled", TwoProportionsMethod::Unpooled),
            ("cohens_h", "Cohen's h", TwoProportionsMethod::CohensH),
        ];
        let bw = ((rect.width.saturating_sub(gap * 2)) / 3).max(60);
        for (i, (k, lbl, m)) in methods.iter().enumerate() {
            let bx = rect.x + i32::try_from((i as u32) * (bw + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.prop_method.{k}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, y, bw, row_h),
                    label: lbl,
                    keyboard_path: &format!("power.prop_method.{k}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(40 + i as u32),
                    primary: false,
                    selected: pw.two_proportions_mode == *m,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("prop_method:{k}")),
                },
            );
        }
        y += i32::try_from(row_h + gap).unwrap_or(30);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "power.continuity_correction",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x, y, rect.width, row_h),
                label: "Apply Yates / Fleiss Continuity Correction",
                keyboard_path: "power.continuity_correction",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(44),
                checked: pw.continuity_correction,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: custom_action("toggle:continuity_correction"),
            },
        );
        y += i32::try_from(row_h + gap).unwrap_or(30);
    } else if pw.family == PowerTestFamily::Correlation && y + i32::try_from(hdr_h + row_h + 6).unwrap_or(48) <= rect.bottom() {
        paint_section_heading(builder, parent, LayoutRectPx::new(rect.x, y, rect.width, hdr_h), "Correlation Method");
        y += i32::try_from(hdr_h + 2).unwrap_or(20);
        let methods = [
            ("fisher_z", "Fisher z", CorrelationPowerMethod::FisherZ),
            ("fisher_z_tcrit", "Fisher z + t", CorrelationPowerMethod::FisherZWithTCrit),
            ("exact_t", "Exact t", CorrelationPowerMethod::ExactTTransform),
        ];
        let bw = ((rect.width.saturating_sub(gap * 2)) / 3).max(60);
        for (i, (k, lbl, m)) in methods.iter().enumerate() {
            let bx = rect.x + i32::try_from((i as u32) * (bw + gap)).unwrap_or(0);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("power.corr_method.{k}"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(bx, y, bw, row_h),
                    label: lbl,
                    keyboard_path: &format!("power.corr_method.{k}"),
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(40 + i as u32),
                    primary: false,
                    selected: pw.correlation_method == *m,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("corr_method:{k}")),
                },
            );
        }
        y += i32::try_from(row_h + gap).unwrap_or(30);
    }

    // Validation status banner
    if y + i32::try_from(row_h).unwrap_or(24) <= rect.bottom() {
        let err_msg = pw.can_advance().err().or_else(|| pw.validation_error.clone()).or_else(|| vs.last_error.clone());
        let banner_rect = LayoutRectPx::new(rect.x, y, rect.width, row_h);
        let (msg, col) = match err_msg {
            Some(e) => (format!("Check inputs: {e}"), Rgba8::rgb(185, 28, 28)),
            None => (String::from("Parameters valid. Proceed to Stage 4 Results & Sensitivity."), builder.palette.muted),
        };
        let sz = (11.0 * s).clamp(8.5, 19.0);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: banner_rect,
                pad_x: 4,
                text: &msg,
                style: &UiTextStyle::ui_sans(sz),
                color: col,
                scroll_container: None,
            },
        );
    }
}

fn build_stage4_results(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let s = builder.scale();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let vs = &builder.app.view_shards.power_wizard;

    let Some(ref res) = pw.result else {
        let msg = pw.validation_error.as_deref().unwrap_or("Adjust parameters in the sidebar to compute power.");
        let sz = (12.0 * s).clamp(9.0, 22.0);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect,
                pad_x: 6,
                text: msg,
                style: &UiTextStyle::ui_sans(sz),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        return;
    };

    // 1. Headline solved metrics banner
    let banner_h = scaled(44.0, s).min(rect.height / 4);
    let banner_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, banner_h);
    build_results_headline_banner(builder, parent, banner_rect, res, vs);

    let rem_y = banner_rect.bottom() + i32::try_from(scaled(6.0, s)).unwrap_or(6);
    let rem_h = rect.bottom().saturating_sub(rem_y) as u32;
    if rem_h < scaled(72.0, s) {
        return;
    }

    // 2. Power vs Sample Size vector plot (~54% of remaining height)
    let gap = scaled(6.0, s);
    let plot_h = ((rem_h * 54) / 100).max(scaled(105.0, s)).min(rem_h.saturating_sub(scaled(64.0, s)));
    let plot_rect = LayoutRectPx::new(rect.x, rem_y, rect.width, plot_h);
    build_power_vector_plot(builder, parent, plot_rect, res);

    // 3. Scrollable sensitivity table
    let tbl_y = plot_rect.bottom() + i32::try_from(gap).unwrap_or(6);
    let tbl_h = rect.bottom().saturating_sub(tbl_y) as u32;
    if tbl_h >= scaled(48.0, s) {
        build_sensitivity_table(builder, parent, LayoutRectPx::new(rect.x, tbl_y, rect.width, tbl_h), res);
    }
}

fn build_results_headline_banner(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    res: &PowerAnalysisResult,
    vs: &PowerWizardViewState,
) {
    let s = builder.scale();
    builder.paint_fill(ZLayer::ModalDialog, Some(parent), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(parent), rect, builder.palette.border, 1);

    let line1_h = (rect.height / 2).max(12);
    let line2_h = rect.height.saturating_sub(line1_h + 2);
    let line1_rect = LayoutRectPx::new(rect.x + 6, rect.y + 1, rect.width.saturating_sub(12), line1_h);
    let line2_rect = LayoutRectPx::new(rect.x + 6, line1_rect.bottom() + 1, rect.width.saturating_sub(12), line2_h);

    let adj_total = dropout_adjusted_n(res.total_n, vs.dropout_rate);
    let line1 = match res.n_group2 {
        Some(n2) if vs.dropout_rate > 0.001 => format!(
            "{}: n1={}, n2={}, N={} (Adj +{:.0}%: {})",
            res.family.label(),
            res.n_per_group,
            n2,
            res.total_n,
            vs.dropout_rate * 100.0,
            adj_total
        ),
        Some(n2) => format!("{}: n1 = {}, n2 = {}, Total N = {}", res.family.label(), res.n_per_group, n2, res.total_n),
        None if vs.dropout_rate > 0.001 => format!(
            "{}: n = {}, Total N = {} (Adj +{:.0}%: {})",
            res.family.label(),
            res.n_per_group,
            res.total_n,
            vs.dropout_rate * 100.0,
            adj_total
        ),
        None => format!("{}: n/group = {}, Total N = {}", res.family.label(), res.n_per_group, res.total_n),
    };
    let line2 = if vs.survival_log_rank {
        let ev = schoenfeld_events(res.alpha, res.achieved_power, true, 1.0, vs.survival_hazard_ratio);
        format!(
            "Power = {:.3} | Alpha = {:.3} | HR = {:.2} | Events ~ {}",
            res.achieved_power,
            res.alpha,
            vs.survival_hazard_ratio,
            ev.map_or_else(|| String::from("-"), |e| e.to_string())
        )
    } else {
        format!(
            "Power = {:.3} | Alpha = {:.3} | Effect = {:.3} | Crit = {:.3} | NCP = {:.3}",
            res.achieved_power, res.alpha, res.effect_size, res.critical_value, res.noncentrality_param
        )
    };

    let sz1 = (11.5 * s).clamp(8.5, 20.0);
    let sz2 = (10.5 * s).clamp(8.0, 18.0);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: line1_rect,
            pad_x: 2,
            text: &line1,
            style: &UiTextStyle::ui_bold(sz1),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    if line2_h >= 10 {
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: line2_rect,
                pad_x: 2,
                text: &line2,
                style: &UiTextStyle::grid_tabular(sz2),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
}

fn build_power_vector_plot(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, res: &PowerAnalysisResult) {
    let s = builder.scale();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let locale = builder.tree.locale.clone();

    let curve_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("power.curve_hover"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Canvas, "Power vs N Curve", FocusScope::ModalDialog, "ArrowLeft/Right").with_tab_order(25),
        state: widget_state(builder, "power.curve_hover", ControlEnablement::enabled()),
        action: custom_action("hover_frac:0.5"),
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&curve_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&curve_wid), rect, builder.palette.border, 1);

    let top_pad = scaled(20.0, s);
    let bot_pad = scaled(28.0, s);
    let left_pad = scaled(44.0, s);
    let right_pad = scaled(12.0, s);

    let plot_w = rect.width.saturating_sub(left_pad + right_pad);
    let plot_h = rect.height.saturating_sub(top_pad + bot_pad);
    if plot_w < 60 || plot_h < 40 {
        return;
    }
    let plot_box = LayoutRectPx::new(rect.x + i32::try_from(left_pad).unwrap_or(44), rect.y + i32::try_from(top_pad).unwrap_or(20), plot_w, plot_h);
    builder.paint_fill(ZLayer::ModalDialog, Some(&curve_wid), plot_box, builder.palette.panel);

    let curve = &res.power_curve;
    let n_min = curve.first().map_or(2.0, |p| p.n);
    let n_max = curve.last().map_or(n_min + 10.0, |p| p.n.max(n_min + 1.0));
    let n_span = (n_max - n_min).max(1.0);

    let is_rtl = builder.tree.is_rtl;
    let map_x = |n: f64| -> i32 {
        let f = ((n - n_min) / n_span).clamp(0.0, 1.0);
        let fx = if is_rtl { 1.0 - f } else { f };
        plot_box.x + (fx * f64::from(plot_box.width.saturating_sub(1))).round() as i32
    };
    let map_y = |power: f64| -> i32 {
        let p = power.clamp(0.0, 1.0);
        plot_box.bottom() - 1 - (p * f64::from(plot_box.height.saturating_sub(1))).round() as i32
    };

    // Gridlines
    for p_grid in [0.25, 0.50, 0.75] {
        let gy = map_y(p_grid);
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [plot_box.x, gy], [plot_box.right() - 1, gy], builder.palette.border, 1);
    }
    for f_grid in [0.25, 0.50, 0.75] {
        let gx = plot_box.x + (f_grid * f64::from(plot_box.width.saturating_sub(1))).round() as i32;
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [gx, plot_box.y], [gx, plot_box.bottom() - 1], builder.palette.border, 1);
    }

    // Target-power dashed horizontal reference line
    if let Some(tp) = res.target_power {
        let ty = map_y(tp);
        let ref_col = Rgba8::rgb(217, 119, 6);
        let mut sx = plot_box.x;
        let dash = 6i32;
        while sx < plot_box.right() - 2 {
            let ex = (sx + dash).min(plot_box.right() - 1);
            builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [sx, ty], [ex, ty], ref_col, 1);
            sx += dash * 2;
        }
    }

    // Main axes lines
    builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [plot_box.x, plot_box.y], [plot_box.x, plot_box.bottom() - 1], builder.palette.text, 1);
    builder.paint_line(
        ZLayer::ModalDialog,
        Some(&curve_wid),
        [plot_box.x, plot_box.bottom() - 1],
        [plot_box.right() - 1, plot_box.bottom() - 1],
        builder.palette.text,
        1,
    );

    // Curve segments
    for pair in curve.windows(2) {
        let p0 = [map_x(pair[0].n), map_y(pair[0].power)];
        let p1 = [map_x(pair[1].n), map_y(pair[1].power)];
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), p0, p1, builder.palette.accent, 2);
    }

    // Solved design marker dot + drop line
    let solved_n = res.n_per_group as f64;
    if solved_n >= n_min && solved_n <= n_max {
        let sx = map_x(solved_n);
        let sy = map_y(res.achieved_power);
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [sx, sy], [sx, plot_box.bottom() - 1], builder.palette.muted, 1);
        emit_circle_op(builder, &curve_wid, [sx, sy], 4, builder.palette.accent, Some(Rgba8::WHITE));
    }

    // Hover bins inside plot_box for passive pointer hover + click pinning
    let num_bins = 20usize.min((plot_box.width / 6).max(1) as usize);
    let bin_w_f = f64::from(plot_box.width) / (num_bins as f64);
    let mut active_hover: Option<(f64, f64)> = None;

    let hovered_id = builder.app.interaction.hovered_widget_id.as_deref();
    let hovering_plot = hovered_id.is_some_and(|id| id == "power.curve_hover" || id.starts_with("power.curve_hover.bin."));
    if hovering_plot && let Some([lx, ly]) = builder.app.interaction.last_logical_pos {
        let [px, py] = builder.tree.metrics.logical_to_physical_px([lx, ly]);
        if plot_box.contains_px(px.round() as i32, py.round() as i32) {
            let raw_f = ((px - f64::from(plot_box.x)) / f64::from(plot_box.width.max(1))).clamp(0.0, 1.0);
            let frac = if builder.tree.is_rtl { 1.0 - raw_f } else { raw_f };
            let target_n = n_min + frac * n_span;
            if let Some(hp) = interpolate_curve_power(curve, target_n) {
                active_hover = Some((target_n, hp));
            }
        }
    }
    if active_hover.is_none()
        && let (Some(hn), Some(hp)) = (pw.hover_n, pw.hover_power)
    {
        active_hover = Some((hn, hp));
    }

    for i in 0..num_bins {
        let bx0 = plot_box.x + (i as f64 * bin_w_f).round() as i32;
        let bx1 = if i + 1 == num_bins { plot_box.right() } else { plot_box.x + (((i + 1) as f64) * bin_w_f).round() as i32 };
        let bw = u32::try_from((bx1 - bx0).max(1)).unwrap_or(1);
        let bin_rect = LayoutRectPx::new(bx0, plot_box.y, bw, plot_box.height);
        let frac = ((i as f64) + 0.5) / (num_bins as f64);
        let logical_frac = if builder.tree.is_rtl { 1.0 - frac } else { frac };
        let bin_n = n_min + logical_frac * n_span;
        let bin_id = format!("power.curve_hover.bin.{i}");
        let is_hov = hovered_id == Some(bin_id.as_str());

        let mut a11y = labeled_meta(WidgetRole::DataPoint, format!("Curve N={bin_n:.0}"), FocusScope::ModalDialog, "ArrowLeft/Right");
        a11y.value_text = Some(format!("N={bin_n:.0}"));
        let bin_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(bin_id),
            parent: Some(curve_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: bin_rect,
            a11y,
            state: WidgetVisualState { hovered: is_hov, ..Default::default() },
            action: custom_action(format!("hover_frac:{logical_frac:.4}")),
        });
        let tint = if is_hov { Rgba8::rgba(59, 130, 246, 26) } else { Rgba8::rgba(59, 130, 246, 2) };
        builder.paint_fill(ZLayer::ModalDialog, Some(&bin_wid), bin_rect, tint);
    }

    // Active hover crosshair + indicator dot
    if let Some((hn, hp)) = active_hover {
        let hx = map_x(hn);
        let hy = map_y(hp);
        let cross_col = Rgba8::rgb(220, 38, 38);
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [hx, plot_box.y], [hx, plot_box.bottom() - 1], cross_col, 1);
        builder.paint_line(ZLayer::ModalDialog, Some(&curve_wid), [plot_box.x, hy], [plot_box.right() - 1, hy], cross_col, 1);
        emit_circle_op(builder, &curve_wid, [hx, hy], 4, cross_col, Some(Rgba8::WHITE));
    }

    // Disjoint header strip: Y-axis title | Plot title | Hover callout readout
    paint_plot_labels(builder, &curve_wid, rect, plot_box, res, active_hover, &locale);
}

#[allow(clippy::too_many_arguments)]
fn paint_plot_labels(
    builder: &mut LayoutBuilder<'_>,
    curve_wid: &WidgetId,
    rect: LayoutRectPx,
    plot_box: LayoutRectPx,
    res: &PowerAnalysisResult,
    active_hover: Option<(f64, f64)>,
    locale: &str,
) {
    let s = builder.scale();
    let sz_hdr = (10.5 * s).clamp(8.0, 18.0);
    let sz_tick = (9.5 * s).clamp(7.5, 16.0);
    let top_h = scaled(16.0, s);

    // Top row: 3 strictly disjoint boxes
    let y_title_w = scaled(68.0, s).min(rect.width / 4);
    let mid_w = (rect.width / 2).saturating_sub(y_title_w + 8).max(40);
    let right_x = rect.x + i32::try_from(y_title_w + mid_w + 8).unwrap_or(180);
    let right_w = u32::try_from((rect.right() - right_x - 4).max(40)).unwrap_or(40);

    paint_text_fitted(
        builder,
        curve_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x + 4, rect.y + 2, y_title_w, top_h),
            pad_x: 1,
            text: "Power",
            style: &UiTextStyle::ui_bold(sz_tick),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    paint_text_fitted(
        builder,
        curve_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x + i32::try_from(y_title_w + 6).unwrap_or(74), rect.y + 2, mid_w, top_h),
            pad_x: 2,
            text: wiz_tr(locale, "pw.plot.title", "Power vs. Sample Size"),
            style: &UiTextStyle::ui_bold(sz_hdr),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let callout = match active_hover {
        Some((hn, hp)) => format!("Hover: N={hn:.0}, Power={hp:.3}"),
        None => match res.target_power {
            Some(tp) => format!("Target {:.0}% -> n={}", tp * 100.0, res.n_per_group),
            None => format!("Solved Power={:.3} at N={}", res.achieved_power, res.n_per_group),
        },
    };
    let callout_col = if active_hover.is_some() { Rgba8::rgb(220, 38, 38) } else { builder.palette.accent };
    paint_text_fitted(
        builder,
        curve_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(right_x, rect.y + 2, right_w, top_h),
            pad_x: 2,
            text: &callout,
            style: &UiTextStyle::grid_tabular(sz_tick),
            color: callout_col,
            scroll_container: None,
        },
    );

    // Y ticks in left margin (strictly separated vertically)
    let tick_w = u32::try_from((plot_box.x - rect.x - 4).max(20)).unwrap_or(32);
    let tick_h = scaled(13.0, s);
    let y_positions = [
        ("1.0", plot_box.y),
        ("0.5", plot_box.center()[1] - i32::try_from(tick_h / 2).unwrap_or(6)),
        ("0.0", plot_box.bottom() - i32::try_from(tick_h).unwrap_or(13)),
    ];
    for (lbl, ty) in y_positions {
        paint_text_fitted(
            builder,
            curve_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x + 2, ty, tick_w, tick_h),
                pad_x: 1,
                text: lbl,
                style: &UiTextStyle::grid_tabular(sz_tick),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }

    // X ticks + X axis title in bottom margin (2 distinct vertical rows)
    let n_min = res.power_curve.first().map_or(2.0, |p| p.n);
    let n_max = res.power_curve.last().map_or(n_min + 10.0, |p| p.n);
    let n_mid = 0.5 * (n_min + n_max);
    let xtick_y = plot_box.bottom() + 2;
    let xtick_w = scaled(48.0, s).min(plot_box.width / 4);
    let xticks = [
        (format!("{n_min:.0}"), plot_box.x),
        (format!("{n_mid:.0}"), plot_box.center()[0] - i32::try_from(xtick_w / 2).unwrap_or(24)),
        (format!("{n_max:.0}"), plot_box.right() - i32::try_from(xtick_w).unwrap_or(48)),
    ];
    for (lbl, tx) in xticks {
        paint_text_fitted(
            builder,
            curve_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(tx, xtick_y, xtick_w, tick_h),
                pad_x: 1,
                text: &lbl,
                style: &UiTextStyle::grid_tabular(sz_tick),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }

    let x_title_y = xtick_y + i32::try_from(tick_h + 1).unwrap_or(14);
    let x_title_h = u32::try_from((rect.bottom() - x_title_y).max(0)).unwrap_or(0);
    if x_title_h >= 10 {
        paint_text_fitted(
            builder,
            curve_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(plot_box.x, x_title_y, plot_box.width, x_title_h),
                pad_x: 4,
                text: "Sample Size (N per group)",
                style: &UiTextStyle::ui_sans(sz_tick),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
}

fn emit_circle_op(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, center: [i32; 2], radius: u32, fill: Rgba8, stroke: Option<Rgba8>) {
    let order = builder.next_order();
    builder.tree.paint_ops.push(crate::layout::PaintOp {
        z_layer: ZLayer::ModalDialog,
        paint_order: order,
        clip: Some(builder.active_clip_id()),
        owner_widget_id: Some(owner.clone()),
        kind: PaintOpKind::CircleIndicator { center, radius, fill, stroke },
    });
}

fn build_sensitivity_table(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, res: &PowerAnalysisResult) {
    let s = builder.scale();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let vs = &builder.app.view_shards.power_wizard;
    let cells = pw.sensitivity_cells();

    let hdr_h = scaled(20.0, s);
    let hdr_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, hdr_h);
    builder.paint_fill(ZLayer::ModalDialog, Some(parent), hdr_rect, builder.palette.header);
    builder.paint_stroke(ZLayer::ModalDialog, Some(parent), hdr_rect, builder.palette.border, 1);

    let show_adj = vs.dropout_rate > 0.001 && res.solve_target != PowerSolveTarget::DetectableEffect;
    let col_count = if show_adj { 5u32 } else { 4u32 };
    let col_w = (rect.width.saturating_sub(14) / col_count).max(40);
    let col_headers: [&str; 5] = if res.solve_target == PowerSolveTarget::DetectableEffect {
        ["Alpha", "Power", "N", "Min Effect", ""]
    } else if show_adj {
        ["Alpha", "Power", "n/grp", "Total N", "Adj N"]
    } else {
        ["Alpha", "Power", "n/grp", "Total N", ""]
    };
    let sz_hdr = (10.5 * s).clamp(8.0, 18.0);
    for c_idx in 0..(col_count as usize) {
        let cx = rect.x + 2 + i32::try_from((c_idx as u32) * col_w).unwrap_or(0);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(cx, hdr_rect.y + 1, col_w.saturating_sub(4), hdr_h.saturating_sub(2)),
                pad_x: 2,
                text: col_headers[c_idx],
                style: &UiTextStyle::ui_bold(sz_hdr),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let body_rect = LayoutRectPx::new(rect.x, hdr_rect.bottom(), rect.width, rect.height.saturating_sub(hdr_h));
    if body_rect.height < 16 {
        return;
    }

    let row_h = scaled(20.0, s);
    let content_h = (cells.len() as u32).saturating_mul(row_h) + 6;
    let scroll_id = String::from("scroll.power.sensitivity");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), body_rect, [body_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let tbl_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("power.sensitivity_table"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: body_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Sensitivity Table", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&tbl_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&tbl_wid), body_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&tbl_wid), body_rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(body_rect.x + 2, body_rect.y + 2, body_rect.width.saturating_sub(14), body_rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let sz_cell = (10.5 * s).clamp(8.0, 18.0);
    let cell_style = UiTextStyle::grid_tabular(sz_cell);

    for (idx, cell) in cells.iter().enumerate() {
        let ry = clip.y + i32::try_from((idx as u32) * row_h).unwrap_or(0) - y_scroll;
        let row_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h - 1);
        if row_rect.y < clip.y || row_rect.bottom() > clip.bottom() {
            continue;
        }
        let row_id = format!("power.sensitivity.row.{idx}");
        let selected = (cell.alpha - pw.alpha).abs() < 1e-4 && (cell.target_power - pw.target_power).abs() < 1e-3;
        let st = widget_state(builder, &row_id, ControlEnablement::enabled());

        let mut a11y = selectable_meta(
            WidgetRole::Button,
            format!("Sensitivity alpha={:.3} power={:.2}", cell.alpha, cell.target_power),
            FocusScope::ModalDialog,
            &row_id,
            selected,
        );
        a11y.tab_order = Some(100 + idx as u32);
        let row_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(row_id),
            parent: Some(tbl_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: row_rect,
            a11y,
            state: st.clone(),
            action: custom_action(format!("select_sens:{idx}")),
        });

        let bg = if selected {
            builder.palette.accent
        } else if st.hovered {
            builder.palette.header
        } else if idx % 2 == 1 {
            builder.palette.panel
        } else {
            builder.palette.surface
        };
        let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
        builder.paint_fill(ZLayer::ModalDialog, Some(&row_wid), row_rect, bg);

        let vals = format_sensitivity_row_values(cell, res.solve_target, vs.dropout_rate);
        for (c_idx, txt) in vals.iter().take(col_count as usize).enumerate() {
            let cx = row_rect.x + i32::try_from((c_idx as u32) * col_w).unwrap_or(0);
            paint_text_fitted(
                builder,
                &row_wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(cx, row_rect.y, col_w.saturating_sub(4), row_rect.height),
                    pad_x: 2,
                    text: txt,
                    style: &cell_style,
                    color: fg,
                    scroll_container: Some(&scroll_id),
                },
            );
        }
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(tbl_wid));
}

fn format_sensitivity_row_values(cell: &SensitivityCell, target: PowerSolveTarget, dropout_rate: f64) -> [String; 5] {
    let a = format!("{:.3}", cell.alpha);
    let p = format!("{:.0}%", cell.target_power * 100.0);
    if target == PowerSolveTarget::DetectableEffect {
        [a, p, cell.sample_size.to_string(), format!("{:.3}", cell.effect_size), String::new()]
    } else {
        let adj = dropout_adjusted_n(cell.total_n, dropout_rate);
        [a, p, cell.sample_size.to_string(), cell.total_n.to_string(), adj.to_string()]
    }
}

fn build_live_sidebar(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx) {
    let s = builder.scale();
    let locale = builder.tree.locale.clone();
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let vs = &builder.app.view_shards.power_wizard;

    let hdr_h = scaled(22.0, s);
    let scroll_rect = LayoutRectPx::new(rect.x, rect.y + i32::try_from(hdr_h).unwrap_or(22), rect.width, rect.height.saturating_sub(hdr_h));

    let sidebar_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("power.sidebar"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Live Parameters", FocusScope::ModalDialog, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&sidebar_wid), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&sidebar_wid), rect, builder.palette.border, 1);

    let hdr_rect = LayoutRectPx::new(rect.x + 6, rect.y + 2, rect.width.saturating_sub(12), hdr_h.saturating_sub(4));
    paint_section_heading(builder, &sidebar_wid, hdr_rect, wiz_tr(&locale, "pw.section.sidebar", "Live Parameters"));

    if scroll_rect.height < scaled(40.0, s) {
        return;
    }

    let block_h = scaled(40.0, s);
    let block_gap = scaled(4.0, s);
    let summary_h = scaled(68.0, s);
    let num_blocks = 6u32;
    let content_h = num_blocks * (block_h + block_gap) + summary_h + 10;

    let scroll_id = String::from("scroll.power.sidebar");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), scroll_rect, [scroll_rect.width, content_h], scroll_off, ScrollAxes::Vertical);
    builder.attach_scroll(&sidebar_wid, scroll_meta.clone());

    let clip = LayoutRectPx::new(
        scroll_rect.x + 4,
        scroll_rect.y + 2,
        scroll_rect.width.saturating_sub(if scroll_meta.has_vertical_overflow() { 16 } else { 8 }),
        scroll_rect.height.saturating_sub(4),
    );
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;

    let (eff_lo, eff_hi) = effect_range_for_family(pw.family);
    let eff_val = effective_effect_display(pw, vs);
    let min_p = (pw.alpha + 0.02).max(0.50);
    let min_n = pw.family.min_integer_n() as f64;

    let power_en = if pw.solve_target == PowerSolveTarget::Power {
        ControlEnablement::disabled("Target power is solved output.")
    } else {
        ControlEnablement::enabled()
    };
    let eff_en = if pw.solve_target == PowerSolveTarget::DetectableEffect {
        ControlEnablement::disabled("Effect size is solved output.")
    } else {
        ControlEnablement::enabled()
    };
    let alloc_en = if matches!(pw.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Allocation ratio applies to two-group independent designs.")
    };
    let n_en = if pw.solve_target == PowerSolveTarget::SampleSize {
        ControlEnablement::disabled("Sample size N is solved output.")
    } else {
        ControlEnablement::enabled()
    };

    let controls: [SidebarControlSpec<'_>; 6] = [
        SidebarControlSpec {
            field_key: "alpha",
            label: "Alpha",
            value_text: &format!("{:.3}", pw.alpha),
            normalized: slider_norm(pw.alpha, 0.001, 0.20),
            step: 0.01,
            tab_order: 300,
            enablement: ControlEnablement::enabled(),
        },
        SidebarControlSpec {
            field_key: "target_power",
            label: "Target Power",
            value_text: &format!("{:.2}", pw.target_power),
            normalized: slider_norm(pw.target_power, min_p, 0.99),
            step: 0.05,
            tab_order: 310,
            enablement: power_en,
        },
        SidebarControlSpec {
            field_key: "effect_size",
            label: "Effect Size",
            value_text: &format!("{eff_val:.2}"),
            normalized: slider_norm(eff_val, eff_lo, eff_hi),
            step: 0.05,
            tab_order: 320,
            enablement: eff_en,
        },
        SidebarControlSpec {
            field_key: "allocation_ratio",
            label: "Ratio (n2/n1)",
            value_text: &format!("{:.2}", pw.allocation_ratio),
            normalized: slider_norm(pw.allocation_ratio, 0.25, 4.0),
            step: 0.25,
            tab_order: 330,
            enablement: alloc_en,
        },
        SidebarControlSpec {
            field_key: "dropout",
            label: "Dropout (%)",
            value_text: &format!("{:.0}%", vs.dropout_rate * 100.0),
            normalized: slider_norm(vs.dropout_rate, 0.0, 0.50),
            step: 0.05,
            tab_order: 340,
            enablement: ControlEnablement::enabled(),
        },
        SidebarControlSpec {
            field_key: "sample_size",
            label: "Sample N",
            value_text: &format!("{:.0}", pw.sample_size),
            normalized: slider_norm(pw.sample_size, min_n, 300.0),
            step: 2.0,
            tab_order: 350,
            enablement: n_en,
        },
    ];

    let mut cy = clip.y - y_scroll;
    for ctrl in &controls {
        let block_rect = LayoutRectPx::new(clip.x, cy, clip.width, block_h);
        if block_rect.y >= clip.y && block_rect.bottom() <= clip.bottom() {
            emit_sidebar_control_block(builder, &sidebar_wid, block_rect, ctrl, &scroll_id);
        }
        cy += i32::try_from(block_h + block_gap).unwrap_or(44);
    }

    // Live solved readout summary box
    let sum_rect = LayoutRectPx::new(clip.x, cy + 2, clip.width, summary_h);
    if sum_rect.y >= clip.y && sum_rect.bottom() <= clip.bottom() {
        emit_sidebar_live_summary(builder, &sidebar_wid, sum_rect, pw, vs, &scroll_id);
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(sidebar_wid));
}

struct SidebarControlSpec<'a> {
    field_key: &'a str,
    label: &'a str,
    value_text: &'a str,
    normalized: f64,
    step: f64,
    tab_order: u32,
    enablement: ControlEnablement,
}

fn emit_sidebar_control_block(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    spec: &SidebarControlSpec<'_>,
    scroll_id: &str,
) {
    let s = builder.scale();
    let top_h = (rect.height / 2).max(14);
    let bot_h = rect.height.saturating_sub(top_h + 2).max(12);

    let label_w = (rect.width * 44 / 100).max(52);
    let spin_w = rect.width.saturating_sub(label_w + 4);
    let label_rect = LayoutRectPx::new(rect.x, rect.y, label_w, top_h);
    let spin_rect = LayoutRectPx::new(rect.x + i32::try_from(label_w + 4).unwrap_or(60), rect.y, spin_w, top_h);
    let slider_rect = LayoutRectPx::new(rect.x, rect.y + i32::try_from(top_h + 2).unwrap_or(20), rect.width, bot_h);

    let sz = (10.5 * s).clamp(8.0, 18.0);
    let fg = if spec.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: label_rect,
            pad_x: 2,
            text: spec.label,
            style: &UiTextStyle::ui_sans(sz),
            color: fg,
            scroll_container: Some(scroll_id),
        },
    );

    emit_power_spinner(
        builder,
        parent,
        spin_rect,
        PowerSpinnerSpec {
            id_prefix: &format!("power.sidebar.{}", spec.field_key),
            field_key: spec.field_key,
            label: spec.label,
            value_text: spec.value_text,
            step: spec.step,
            tab_order: spec.tab_order,
            enablement: spec.enablement.clone(),
            scroll_id: Some(scroll_id),
        },
    );

    emit_power_slider(
        builder,
        parent,
        slider_rect,
        &format!("power.sidebar.{}.slider", spec.field_key),
        spec.field_key,
        spec.label,
        spec.normalized,
        spec.tab_order + 3,
        spec.enablement.clone(),
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_power_slider(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    id: &str,
    field_key: &str,
    label: &str,
    normalized: f64,
    tab_order: u32,
    enablement: ControlEnablement,
) {
    let t = if normalized.is_finite() { normalized.clamp(0.0, 1.0) } else { 0.0 };
    let st = widget_state(builder, id, enablement);
    let mut a11y = labeled_meta(WidgetRole::Slider, label, FocusScope::ModalDialog, id).with_tab_order(tab_order);
    a11y.value_text = Some(format!("{:.0}%", t * 100.0));

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y,
        state: st.clone(),
        action: if st.enablement.enabled { custom_action(format!("slider:{field_key}")) } else { WidgetAction::NoOp },
    });

    let cy = rect.center()[1];
    let track = LayoutRectPx::new(rect.x + 6, cy - 2, rect.width.saturating_sub(12).max(4), 4);
    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), track, builder.palette.border);
    let fill_col = if st.enablement.enabled { builder.palette.accent } else { builder.palette.muted };
    let fill_w = (f64::from(track.width) * t).round() as u32;
    if fill_w > 0 {
        let fill_x = if builder.tree.is_rtl { track.right() - i32::try_from(fill_w).unwrap_or(0) } else { track.x };
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), LayoutRectPx::new(fill_x, track.y, fill_w, track.height), fill_col);
    }
    let thumb_cx =
        if builder.tree.is_rtl { track.right() - i32::try_from(fill_w).unwrap_or(0) } else { track.x + i32::try_from(fill_w).unwrap_or(0) };
    let thumb_rect = LayoutRectPx::new((thumb_cx - 4).clamp(rect.x, rect.right() - 8), (cy - 5).max(rect.y), 8, 10.min(rect.height));
    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), thumb_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), thumb_rect, fill_col, 1);
    maybe_paint_focus_ring(builder, ZLayer::ModalDialog, &wid, rect, st.focused);
}

struct PowerSpinnerSpec<'a> {
    id_prefix: &'a str,
    field_key: &'a str,
    label: &'a str,
    value_text: &'a str,
    step: f64,
    tab_order: u32,
    enablement: ControlEnablement,
    scroll_id: Option<&'a str>,
}

fn emit_power_spinner(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: PowerSpinnerSpec<'_>) {
    let btn_w = rect.height.clamp(16, 24);
    let val_w = rect.width.saturating_sub(btn_w.saturating_mul(2)).max(24);
    let dec_rect = LayoutRectPx::new(rect.x, rect.y, btn_w, rect.height);
    let val_rect = LayoutRectPx::new(dec_rect.right(), rect.y, val_w, rect.height);
    let inc_rect = LayoutRectPx::new(val_rect.right(), rect.y, btn_w, rect.height);

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.dec", spec.id_prefix),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: dec_rect,
            label: "-",
            keyboard_path: &format!("{}.dec", spec.id_prefix),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(spec.tab_order),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement.clone(),
            action: custom_action(format!("spin:{}:{}", spec.field_key, -spec.step)),
        },
    );

    let val_id = format!("{}.val", spec.id_prefix);
    let st = widget_state(builder, &val_id, spec.enablement.clone());
    let mut a11y = labeled_meta(WidgetRole::SpinButton, spec.label, FocusScope::ModalDialog, &val_id);
    a11y.value_text = Some(spec.value_text.to_owned());
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(val_id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: val_rect,
        a11y,
        state: st.clone(),
        action: if st.enablement.enabled { WidgetAction::FocusWidget(format!("{}.val", spec.id_prefix)) } else { WidgetAction::NoOp },
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), val_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), val_rect, builder.palette.border, 1);
    let sz = (10.5 * builder.scale()).clamp(8.0, 18.0);
    let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: val_rect,
            pad_x: 3,
            text: spec.value_text,
            style: &UiTextStyle::grid_tabular(sz),
            color: fg,
            scroll_container: spec.scroll_id,
        },
    );

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.inc", spec.id_prefix),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: inc_rect,
            label: "+",
            keyboard_path: &format!("{}.inc", spec.id_prefix),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(spec.tab_order + 1),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement,
            action: custom_action(format!("spin:{}:{}", spec.field_key, spec.step)),
        },
    );
}

struct LabeledSpinnerSpec<'a> {
    id_prefix: &'a str,
    field_key: &'a str,
    label: &'a str,
    value_text: &'a str,
    step: f64,
    tab_order: u32,
    enablement: ControlEnablement,
}

fn emit_labeled_spinner_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: LabeledSpinnerSpec<'_>) {
    let lbl_w = (rect.width * 55 / 100).max(80);
    let sp_w = rect.width.saturating_sub(lbl_w + 6);
    let sz = (11.0 * builder.scale()).clamp(8.5, 19.0);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x, rect.y, lbl_w, rect.height),
            pad_x: 2,
            text: spec.label,
            style: &UiTextStyle::ui_sans(sz),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_power_spinner(
        builder,
        parent,
        LayoutRectPx::new(rect.x + i32::try_from(lbl_w + 6).unwrap_or(86), rect.y, sp_w, rect.height),
        PowerSpinnerSpec {
            id_prefix: spec.id_prefix,
            field_key: spec.field_key,
            label: spec.label,
            value_text: spec.value_text,
            step: spec.step,
            tab_order: spec.tab_order,
            enablement: spec.enablement,
            scroll_id: None,
        },
    );
}

struct PilotRowSpec<'a> {
    id_prefix: &'a str,
    field_key: &'a str,
    label: &'a str,
    value_text: &'a str,
    step: f64,
    tab_order: u32,
}

fn emit_pilot_input_spinner_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: PilotRowSpec<'_>) {
    let s = builder.scale();
    let lbl_w = (rect.width * 38 / 100).max(76);
    let input_w = (rect.width * 28 / 100).max(52);
    let spin_w = rect.width.saturating_sub(lbl_w + input_w + 10);
    let sz = (11.0 * s).clamp(8.5, 19.0);

    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(rect.x, rect.y, lbl_w, rect.height),
            pad_x: 2,
            text: spec.label,
            style: &UiTextStyle::ui_sans(sz),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let input_rect = LayoutRectPx::new(rect.x + i32::try_from(lbl_w + 4).unwrap_or(80), rect.y, input_w, rect.height);
    emit_text_field(
        builder,
        TextFieldSpec {
            id: &format!("{}.input", spec.id_prefix),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: input_rect,
            label: spec.label,
            fallback_text: spec.value_text,
            keyboard_path: &format!("{}.input", spec.id_prefix),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(spec.tab_order),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    emit_power_spinner(
        builder,
        parent,
        LayoutRectPx::new(input_rect.right() + 6, rect.y, spin_w, rect.height),
        PowerSpinnerSpec {
            id_prefix: spec.id_prefix,
            field_key: spec.field_key,
            label: spec.label,
            value_text: spec.value_text,
            step: spec.step,
            tab_order: spec.tab_order + 1,
            enablement: ControlEnablement::enabled(),
            scroll_id: None,
        },
    );
}

fn emit_sidebar_live_summary(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    pw: &PowerWizardState,
    vs: &PowerWizardViewState,
    scroll_id: &str,
) {
    let s = builder.scale();
    builder.paint_fill(ZLayer::ModalDialog, Some(parent), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(parent), rect, builder.palette.border, 1);

    let line_h = (rect.height.saturating_sub(6) / 3).max(12);
    let sz = (10.5 * s).clamp(8.0, 18.0);
    let lines: [String; 3] = match &pw.result {
        Some(res) => {
            let adj = dropout_adjusted_n(res.total_n, vs.dropout_rate);
            let l1 = if vs.dropout_rate > 0.001 {
                format!("Solved N = {} (Adj: {})", res.total_n, adj)
            } else {
                format!("n/grp = {} | Total N = {}", res.n_per_group, res.total_n)
            };
            let l2 = format!("Power = {:.3} | Effect = {:.3}", res.achieved_power, res.effect_size);
            let l3 = format!("Alpha = {:.3} | Ratio = {:.2}", res.alpha, pw.allocation_ratio);
            [l1, l2, l3]
        }
        None => [String::from("No solved result"), pw.validation_error.clone().unwrap_or_else(|| String::from("Adjust parameters")), String::new()],
    };
    for (idx, txt) in lines.iter().enumerate() {
        if txt.is_empty() {
            continue;
        }
        let ly = rect.y + 3 + i32::try_from((idx as u32) * line_h).unwrap_or(0);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x + 4, ly, rect.width.saturating_sub(8), line_h),
                pad_x: 2,
                text: txt,
                style: &if idx == 0 { UiTextStyle::ui_bold(sz) } else { UiTextStyle::grid_tabular(sz) },
                color: if idx == 0 { builder.palette.accent } else { builder.palette.text },
                scroll_container: Some(scroll_id),
            },
        );
    }
}

fn interpolate_curve_power(curve: &[PowerCurvePoint], target_n: f64) -> Option<f64> {
    if curve.is_empty() {
        return None;
    }
    if target_n <= curve[0].n {
        return Some(curve[0].power);
    }
    for pair in curve.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if target_n >= a.n && target_n <= b.n {
            let span = (b.n - a.n).abs();
            if span < 1e-12 {
                return Some(a.power);
            }
            let t = (target_n - a.n) / (b.n - a.n);
            return Some((a.power + t * (b.power - a.power)).clamp(0.0, 1.0));
        }
    }
    curve.last().map(|p| p.power)
}
