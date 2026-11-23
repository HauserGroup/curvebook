//! Interactive 4-stage Power & Sample Size wizard modal shard state and live actions (`R10.3`).

use curvebook_stats::power::{PowerTestFamily, cohens_h};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
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
    if let Some(mode) = payload.strip_prefix("effect_mode:") {
        return handle_effect_mode_action(app, state, mode);
    }
    if let Some(preset) = payload.strip_prefix("pilot_preset:") {
        return handle_pilot_preset_action(app, state, preset);
    }
    Ok(UiEventOutcome::default())
}

fn handle_stage_action(app: &mut UiApp, state: &mut PowerWizardViewState, raw_stage: &str) -> Result<UiEventOutcome, String> {
    let idx: usize = raw_stage.trim().parse().map_err(|_| format!("invalid power stage `{raw_stage}`"))?;
    let target = idx.checked_sub(1).and_then(PowerWizardStage::from_index).ok_or_else(|| format!("unknown power stage `{idx}`"))?;
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
