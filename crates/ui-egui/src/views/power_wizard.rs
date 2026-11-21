//! Interactive 4-stage Power & Sample Size wizard modal body (`R10.3`).

use curvebook_stats::power::{CorrelationPowerMethod, PowerSolveTarget, PowerTestFamily, TwoProportionsMethod, cohens_h};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::interaction::{TextFieldEditState, UiEventOutcome};
use crate::power_wizard::{PowerEffectInputMode, PowerWizardStage, PowerWizardState};

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

fn handled_outcome(action: impl Into<String>) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: action.into(), ..Default::default() }
}

#[derive(Clone, Copy)]
struct FamilyCardSpec {
    key: &'static str,
    family: PowerTestFamily,
    is_survival: bool,
}

const FAMILY_CARDS: [FamilyCardSpec; 9] = [
    FamilyCardSpec { key: "unpaired_t", family: PowerTestFamily::TwoSampleUnpairedT, is_survival: false },
    FamilyCardSpec { key: "paired_t", family: PowerTestFamily::TwoSamplePairedT, is_survival: false },
    FamilyCardSpec { key: "one_sample_t", family: PowerTestFamily::OneSampleT, is_survival: false },
    FamilyCardSpec { key: "one_way_anova", family: PowerTestFamily::OneWayAnova, is_survival: false },
    FamilyCardSpec { key: "survival_log_rank", family: PowerTestFamily::TwoProportions, is_survival: true },
    FamilyCardSpec { key: "two_proportions", family: PowerTestFamily::TwoProportions, is_survival: false },
    FamilyCardSpec { key: "chi_square", family: PowerTestFamily::ChiSquare, is_survival: false },
    FamilyCardSpec { key: "correlation", family: PowerTestFamily::Correlation, is_survival: false },
    FamilyCardSpec { key: "regression", family: PowerTestFamily::SimpleLinearRegression, is_survival: false },
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
    let normalized_key = match key {
        "two_sample_t" => "unpaired_t",
        "simple_linear" => "regression",
        other => other,
    };
    let Some(card) = FAMILY_CARDS.iter().find(|c| c.key == normalized_key) else {
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
    let family = app.canvas_dialogs.power_wizard.family;
    let next_mode = match mode {
        "standardized" | "direct" => PowerEffectInputMode::Standardized,
        "pilot" => match family {
            PowerTestFamily::TwoProportions => PowerEffectInputMode::Proportions,
            PowerTestFamily::SimpleLinearRegression => PowerEffectInputMode::RegressionSlope,
            _ => PowerEffectInputMode::MeansAndSd,
        },
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
    let Some([lx, _]) = app.interaction.last_logical_pos else {
        return 0.5;
    };
    let snap = crate::inspect::inspect_ui_tree(app, Some("dialog"), true);
    let bounds = snap.dialog.as_ref().and_then(|d| d.controls.iter().find(|c| c.widget_id == widget_id)).and_then(|c| c.bounds);
    let Some(r) = bounds else {
        return 0.5;
    };
    let track_x = r[0] + 6.0;
    let track_w = (r[2] - 12.0).max(1.0);
    let raw = ((lx - track_x) / track_w).clamp(0.0, 1.0);
    if crate::i18n::is_rtl_locale(&app.locale) { 1.0 - raw } else { raw }
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
