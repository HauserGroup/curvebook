//! Standalone 4-stage Power & Sample Size Wizard backed by `curvebook-stats` (`R8.7`, `WF-15`).
//!
//! Provides complete state, validation, Stage 4 curve hover inspection, sensitivity table
//! projection, live sidebar recalculation, and GUI `Create` command emission producing both a
//! Power & Sample Size [`curvebook_doc::ResultSheet`] and linked [`curvebook_doc::GraphSheet`].

use curvebook_doc::SheetId;
#[cfg(any(test, feature = "test-support"))]
use curvebook_engine::Session;
use curvebook_engine::{EngineError, ErrorCode};
#[cfg(any(test, feature = "test-support"))]
use curvebook_stats::power::paired_sd_from_correlation;
use curvebook_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerAnalysisResult, PowerCurvePoint, PowerEffectSpec, PowerSolveTarget,
    PowerTestFamily, TailMode, TwoProportionsMethod, cohens_d_from_means, cohens_dz_from_correlation, cohens_f_from_means, solve_power,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Wizard step across the 4-stage Power & Sample Size flow (`docs/gui-workflows.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PowerWizardStage {
    #[default]
    ExperimentDesign,
    EffectSize,
    SignificanceAndSolve,
    Results,
}

impl PowerWizardStage {
    /// All four stages in sequential wizard order.
    pub const ALL: [Self; 4] = [Self::ExperimentDesign, Self::EffectSize, Self::SignificanceAndSolve, Self::Results];

    /// Zero-based stage index (`0..=3`).
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::ExperimentDesign => 0,
            Self::EffectSize => 1,
            Self::SignificanceAndSolve => 2,
            Self::Results => 3,
        }
    }

    /// Construct a stage from a zero-based index (`0..=3`).
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::ExperimentDesign),
            1 => Some(Self::EffectSize),
            2 => Some(Self::SignificanceAndSolve),
            3 => Some(Self::Results),
            _ => None,
        }
    }

    /// Next sequential stage, if any.
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::ExperimentDesign => Some(Self::EffectSize),
            Self::EffectSize => Some(Self::SignificanceAndSolve),
            Self::SignificanceAndSolve => Some(Self::Results),
            Self::Results => None,
        }
    }

    /// Previous sequential stage, if any.
    #[must_use]
    pub const fn prev(self) -> Option<Self> {
        match self {
            Self::ExperimentDesign => None,
            Self::EffectSize => Some(Self::ExperimentDesign),
            Self::SignificanceAndSolve => Some(Self::EffectSize),
            Self::Results => Some(Self::SignificanceAndSolve),
        }
    }

    /// Canonical title for stage breadcrumbs and snapshots.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ExperimentDesign => "1. Experiment Design",
            Self::EffectSize => "2. Effect Size",
            Self::SignificanceAndSolve => "3. Significance & Solve",
            Self::Results => "4. Results & Sensitivity",
        }
    }
}

/// How the user parameterizes the effect size in Stage 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PowerEffectInputMode {
    /// Direct standardized effect size (`d`, `d_z`, `f`, `h`, `r`, `w`, or `f^2`).
    #[default]
    Standardized,
    /// Compute from raw group means and standard deviations (with optional correlation `r` for paired *t*).
    MeansAndSd,
    /// Direct paired differences `(mean_diff, sigma_d)`.
    PairedDiffDirect,
    /// Two proportions `(p1, p2)` or baseline `p1`.
    Proportions,
    /// Regression coefficient of determination `R^2`.
    RegressionRSquared,
    /// Regression slope `(beta1, sigma_x, sigma_eps)`.
    RegressionSlope,
}

/// Sheet identifiers produced when committing Stage 4 `Create` into the active project document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerWizardCreatedSheets {
    pub data_sheet_id: SheetId,
    pub info_sheet_id: Option<SheetId>,
    pub graph_sheet_id: Option<SheetId>,
    pub result_sheet_id: SheetId,
}

/// Full interactive state for the standalone 4-stage Power & Sample Size Wizard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerWizardState {
    pub stage: PowerWizardStage,
    pub family: PowerTestFamily,
    pub solve_target: PowerSolveTarget,
    pub effect_mode: PowerEffectInputMode,

    // Standardized & raw effect parameters (Stage 2)
    pub standardized_effect: f64,
    pub mean_diff: f64,
    pub sd: f64,
    pub sd_group2: Option<f64>,
    pub within_pair_correlation: f64,
    pub paired_sd_diff: f64,
    pub group_means: Vec<f64>,
    pub anova_groups: usize,
    pub p1: f64,
    pub p2: f64,
    pub r_squared: f64,
    pub beta1: f64,
    pub sigma_x: f64,
    pub sigma_eps: f64,

    // Design & Significance parameters (Stage 1 / 2 / 3 & Stage 4 sidebar)
    pub two_tailed: bool,
    pub one_sided_greater: bool,
    pub alpha: f64,
    pub target_power: f64,
    pub sample_size: f64,
    pub sample_size_group2: Option<f64>,
    pub allocation_ratio: f64,
    pub chi_square_df: f64,
    pub two_proportions_mode: TwoProportionsMethod,
    pub continuity_correction: bool,
    pub correlation_method: CorrelationPowerMethod,

    // Live solve output + Stage 4 hover cursor state
    pub result: Option<PowerAnalysisResult>,
    pub validation_error: Option<String>,
    pub hover_n: Option<f64>,
    pub hover_power: Option<f64>,
    pub hover_fraction_x: Option<f64>,
    pub created_sheets: Option<PowerWizardCreatedSheets>,

    // Pilot / survival-log-rank input drafts
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
}

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

impl Default for PowerWizardState {
    fn default() -> Self {
        let mut state = Self {
            stage: PowerWizardStage::ExperimentDesign,
            family: PowerTestFamily::TwoSampleUnpairedT,
            solve_target: PowerSolveTarget::SampleSize,
            effect_mode: PowerEffectInputMode::Standardized,
            standardized_effect: 0.80,
            mean_diff: 0.80,
            sd: 1.0,
            sd_group2: None,
            within_pair_correlation: 0.50,
            paired_sd_diff: 1.0,
            group_means: vec![0.0, 0.4, 0.8],
            anova_groups: 3,
            p1: 0.50,
            p2: 0.70,
            r_squared: 0.13,
            beta1: 0.50,
            sigma_x: 1.0,
            sigma_eps: 1.25,
            two_tailed: true,
            one_sided_greater: true,
            alpha: 0.05,
            target_power: 0.80,
            sample_size: 26.0,
            sample_size_group2: None,
            allocation_ratio: 1.0,
            chi_square_df: 1.0,
            two_proportions_mode: TwoProportionsMethod::PooledNullUnpooledAlt,
            continuity_correction: false,
            correlation_method: CorrelationPowerMethod::FisherZ,
            result: None,
            validation_error: None,
            hover_n: None,
            hover_power: None,
            hover_fraction_x: None,
            created_sheets: None,
            pilot_mean1: default_pilot_mean1(),
            pilot_mean2: default_pilot_mean2(),
            survival_log_rank: false,
            survival_hazard_ratio: default_survival_hr(),
            survival_event_prob: default_survival_event_prob(),
        };
        let _ = state.recalculate();
        state
    }
}

impl PowerWizardState {
    /// Synchronize survival log-rank proportions (`p1`, `p2`, and Cohen's `h`) from hazard ratio and event probability.
    pub fn sync_survival_proportions(&mut self) {
        let p1 = self.survival_event_prob.clamp(0.05, 0.95);
        let s1 = (1.0 - p1).clamp(0.01, 0.99);
        let s2 = s1.powf(self.survival_hazard_ratio.clamp(0.10, 5.0)).clamp(0.01, 0.99);
        let p2 = (1.0 - s2).clamp(0.01, 0.99);
        self.p1 = p1;
        self.p2 = p2;
        self.effect_mode = PowerEffectInputMode::Proportions;
        if let Ok(h) = curvebook_stats::power::cohens_h(p1, p2) {
            self.standardized_effect = h.abs().max(0.05);
        }
    }

    /// Set the active test family and normalize incompatible tail / effect input defaults.
    pub fn set_family(&mut self, family: PowerTestFamily) {
        self.family = family;
        if !family.supports_one_sided() {
            self.two_tailed = true;
        }
        self.effect_mode = match family {
            PowerTestFamily::TwoProportions => PowerEffectInputMode::Proportions,
            PowerTestFamily::SimpleLinearRegression => {
                if matches!(self.effect_mode, PowerEffectInputMode::RegressionRSquared | PowerEffectInputMode::RegressionSlope) {
                    self.effect_mode
                } else {
                    PowerEffectInputMode::Standardized
                }
            }
            PowerTestFamily::TwoSamplePairedT => {
                if matches!(
                    self.effect_mode,
                    PowerEffectInputMode::Proportions | PowerEffectInputMode::RegressionRSquared | PowerEffectInputMode::RegressionSlope
                ) {
                    PowerEffectInputMode::Standardized
                } else {
                    self.effect_mode
                }
            }
            _ => {
                if matches!(
                    self.effect_mode,
                    PowerEffectInputMode::Proportions
                        | PowerEffectInputMode::PairedDiffDirect
                        | PowerEffectInputMode::RegressionRSquared
                        | PowerEffectInputMode::RegressionSlope
                ) {
                    PowerEffectInputMode::Standardized
                } else {
                    self.effect_mode
                }
            }
        };
        self.default_effect_for_family(family);
        let _ = self.recalculate();
    }

    fn default_effect_for_family(&mut self, family: PowerTestFamily) {
        if self.standardized_effect == 0.0 || !self.standardized_effect.is_finite() {
            self.standardized_effect = match family {
                PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => 0.80,
                PowerTestFamily::OneWayAnova => 0.40,
                PowerTestFamily::TwoProportions => 0.4115,
                PowerTestFamily::Correlation | PowerTestFamily::ChiSquare => 0.30,
                PowerTestFamily::SimpleLinearRegression => 0.15,
                PowerTestFamily::LogRankSurvival => 0.50,
            };
        }
        if family == PowerTestFamily::Correlation && self.standardized_effect.abs() >= 1.0 {
            self.standardized_effect = 0.30;
        }
    }

    /// Effective tail mode derived from `family`, `two_tailed`, and `one_sided_greater`.
    #[must_use]
    pub const fn effective_tail_mode(&self) -> TailMode {
        if self.two_tailed || !self.family.supports_one_sided() {
            TailMode::TwoSided
        } else if self.one_sided_greater {
            TailMode::OneSided(OneSidedDirection::Greater)
        } else {
            TailMode::OneSided(OneSidedDirection::Less)
        }
    }

    /// Derived paired difference standard deviation `sigma_d = sigma * sqrt(2 * (1 - r))`.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn computed_paired_sigma_d(&self) -> Option<f64> {
        paired_sd_from_correlation(self.sd, self.within_pair_correlation).ok()
    }

    /// Live standardized effect size implied by the current Stage 2 parameterization.
    #[must_use]
    pub fn derived_standardized_effect(&self) -> Option<f64> {
        match (self.family, self.effect_mode) {
            (_, PowerEffectInputMode::Standardized) => self.standardized_effect.is_finite().then_some(self.standardized_effect),
            (PowerTestFamily::TwoSamplePairedT, PowerEffectInputMode::MeansAndSd) => {
                cohens_dz_from_correlation(self.mean_diff, self.sd, self.within_pair_correlation).ok()
            }
            (PowerTestFamily::TwoSamplePairedT, PowerEffectInputMode::PairedDiffDirect) => {
                (self.paired_sd_diff.is_finite() && self.paired_sd_diff > 0.0 && self.mean_diff.is_finite())
                    .then_some(self.mean_diff / self.paired_sd_diff)
            }
            (PowerTestFamily::OneWayAnova, PowerEffectInputMode::MeansAndSd) => cohens_f_from_means(&self.group_means, self.sd).ok(),
            (_, PowerEffectInputMode::MeansAndSd) => cohens_d_from_means(self.mean_diff, 0.0, self.sd).ok(),
            (PowerTestFamily::TwoProportions, PowerEffectInputMode::Proportions) => curvebook_stats::power::cohens_h(self.p1, self.p2).ok(),
            (PowerTestFamily::SimpleLinearRegression, PowerEffectInputMode::RegressionRSquared) => {
                (self.r_squared.is_finite() && self.r_squared >= 0.0 && self.r_squared < 1.0).then_some(self.r_squared / (1.0 - self.r_squared))
            }
            (PowerTestFamily::SimpleLinearRegression, PowerEffectInputMode::RegressionSlope) => {
                if self.beta1.is_finite() && self.sigma_x > 0.0 && self.sigma_eps > 0.0 {
                    let ratio = (self.beta1 * self.sigma_x) / self.sigma_eps;
                    Some(ratio * ratio)
                } else {
                    None
                }
            }
            _ => self.standardized_effect.is_finite().then_some(self.standardized_effect),
        }
    }

    /// Build a [`PowerAnalysisRequest`] from the current wizard parameters.
    #[must_use]
    pub fn to_request(&self) -> PowerAnalysisRequest {
        let mut req = PowerAnalysisRequest::new(self.family)
            .with_target(self.solve_target)
            .with_tail(self.effective_tail_mode())
            .with_alpha(self.alpha)
            .with_allocation_ratio(self.allocation_ratio)
            .with_chi_square_df(self.chi_square_df)
            .with_two_proportions_mode(self.two_proportions_mode)
            .with_continuity_correction(self.continuity_correction)
            .with_correlation_method(self.correlation_method);

        if self.solve_target != PowerSolveTarget::Power {
            req.target_power = Some(self.target_power);
        } else {
            req.target_power = None;
        }
        if matches!(self.solve_target, PowerSolveTarget::Power | PowerSolveTarget::DetectableEffect) {
            req.sample_size = Some(self.sample_size);
            if matches!(self.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions) {
                req.sample_size_group2 = self.sample_size_group2;
            }
        }
        if self.family == PowerTestFamily::OneWayAnova {
            let k = if self.effect_mode == PowerEffectInputMode::MeansAndSd { self.group_means.len().max(2) } else { self.anova_groups.max(2) };
            req.num_groups = Some(k);
        }
        req.effect = self.build_effect_spec();
        req
    }

    /// Build a persistent document [`curvebook_doc::PowerAnalysisSpec`] capturing all wizard parameters.
    #[must_use]
    pub fn to_doc_spec(&self, info_sheet_id: Option<SheetId>) -> curvebook_doc::PowerAnalysisSpec {
        let req = self.to_request();
        let family = match req.family {
            PowerTestFamily::TwoSampleUnpairedT => curvebook_doc::PowerTestFamily::TwoSampleUnpairedT,
            PowerTestFamily::TwoSamplePairedT => curvebook_doc::PowerTestFamily::TwoSamplePairedT,
            PowerTestFamily::OneSampleT => curvebook_doc::PowerTestFamily::OneSampleT,
            PowerTestFamily::OneWayAnova => curvebook_doc::PowerTestFamily::OneWayAnova,
            PowerTestFamily::TwoProportions => curvebook_doc::PowerTestFamily::TwoProportions,
            PowerTestFamily::Correlation => curvebook_doc::PowerTestFamily::Correlation,
            PowerTestFamily::ChiSquare => curvebook_doc::PowerTestFamily::ChiSquare,
            PowerTestFamily::SimpleLinearRegression => curvebook_doc::PowerTestFamily::SimpleLinearRegression,
            PowerTestFamily::LogRankSurvival => curvebook_doc::PowerTestFamily::LogRankSurvival,
        };
        let solve_target = match req.solve_target {
            PowerSolveTarget::SampleSize => curvebook_doc::PowerSolveTarget::SampleSize,
            PowerSolveTarget::Power => curvebook_doc::PowerSolveTarget::Power,
            PowerSolveTarget::DetectableEffect => curvebook_doc::PowerSolveTarget::DetectableEffect,
        };
        let tail_mode = match req.tail_mode {
            TailMode::TwoSided => curvebook_doc::PowerTailMode::TwoSided,
            TailMode::OneSided(OneSidedDirection::Greater) => curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater),
            TailMode::OneSided(OneSidedDirection::Less) => curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Less),
        };
        let two_proportions_mode = match req.two_proportions_mode {
            TwoProportionsMethod::PooledNullUnpooledAlt => curvebook_doc::TwoProportionsMethod::PooledNullUnpooledAlt,
            TwoProportionsMethod::Unpooled => curvebook_doc::TwoProportionsMethod::Unpooled,
            TwoProportionsMethod::CohensH => curvebook_doc::TwoProportionsMethod::CohensH,
        };
        let correlation_method = match req.correlation_method {
            CorrelationPowerMethod::FisherZ => curvebook_doc::CorrelationPowerMethod::FisherZ,
            CorrelationPowerMethod::FisherZWithTCrit => curvebook_doc::CorrelationPowerMethod::FisherZWithTCrit,
            CorrelationPowerMethod::ExactTTransform => curvebook_doc::CorrelationPowerMethod::ExactTTransform,
            CorrelationPowerMethod::ConditionalSlopeT => curvebook_doc::CorrelationPowerMethod::ConditionalSlopeT,
        };
        let effect = match req.effect {
            PowerEffectSpec::Unspecified => curvebook_doc::PowerEffectSpec::Unspecified,
            PowerEffectSpec::Standardized(d) => curvebook_doc::PowerEffectSpec::Standardized(d),
            PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => {
                curvebook_doc::PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation }
            }
            PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => curvebook_doc::PowerEffectSpec::PairedDiff { mean_diff, sd_diff },
            PowerEffectSpec::GroupMeansAndSd { means, within_sd } => curvebook_doc::PowerEffectSpec::GroupMeansAndSd { means, within_sd },
            PowerEffectSpec::TwoProportions { p1, p2 } => curvebook_doc::PowerEffectSpec::TwoProportions { p1, p2 },
            PowerEffectSpec::BaselineProportion { p1 } => curvebook_doc::PowerEffectSpec::BaselineProportion { p1 },
            PowerEffectSpec::RSquared(r2) => curvebook_doc::PowerEffectSpec::RSquared(r2),
            PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => {
                curvebook_doc::PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps }
            }
            PowerEffectSpec::HazardRatio { hazard_ratio, event_prob_group1, event_prob_group2 } => {
                curvebook_doc::PowerEffectSpec::HazardRatio { hazard_ratio, event_prob_group1, event_prob_group2 }
            }
        };
        curvebook_doc::PowerAnalysisSpec {
            family,
            solve_target,
            tail_mode,
            alpha: req.alpha,
            target_power: req.target_power,
            sample_size: req.sample_size,
            sample_size_group2: req.sample_size_group2,
            allocation_ratio: req.allocation_ratio,
            effect,
            num_groups: req.num_groups,
            chi_square_df: req.chi_square_df,
            two_proportions_mode,
            continuity_correction: req.continuity_correction,
            correlation_method,
            info_sheet_id,
            ..curvebook_doc::PowerAnalysisSpec::default()
        }
    }

    /// Populate wizard fields from an existing L1 [`curvebook_doc::PowerAnalysisSpec`] (`Cmd+T` re-entry).
    pub fn sync_from_doc_spec(&mut self, spec: &curvebook_doc::PowerAnalysisSpec) {
        self.stage = PowerWizardStage::ExperimentDesign;
        self.family = match spec.family {
            curvebook_doc::PowerTestFamily::TwoSampleUnpairedT => PowerTestFamily::TwoSampleUnpairedT,
            curvebook_doc::PowerTestFamily::TwoSamplePairedT => PowerTestFamily::TwoSamplePairedT,
            curvebook_doc::PowerTestFamily::OneSampleT => PowerTestFamily::OneSampleT,
            curvebook_doc::PowerTestFamily::OneWayAnova => PowerTestFamily::OneWayAnova,
            curvebook_doc::PowerTestFamily::TwoProportions => PowerTestFamily::TwoProportions,
            curvebook_doc::PowerTestFamily::Correlation => PowerTestFamily::Correlation,
            curvebook_doc::PowerTestFamily::ChiSquare => PowerTestFamily::ChiSquare,
            curvebook_doc::PowerTestFamily::SimpleLinearRegression => PowerTestFamily::SimpleLinearRegression,
            curvebook_doc::PowerTestFamily::LogRankSurvival => PowerTestFamily::LogRankSurvival,
        };
        self.solve_target = match spec.solve_target {
            curvebook_doc::PowerSolveTarget::SampleSize => PowerSolveTarget::SampleSize,
            curvebook_doc::PowerSolveTarget::Power => PowerSolveTarget::Power,
            curvebook_doc::PowerSolveTarget::DetectableEffect => PowerSolveTarget::DetectableEffect,
        };
        match spec.tail_mode {
            curvebook_doc::PowerTailMode::TwoSided => self.two_tailed = true,
            curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater) => {
                self.two_tailed = false;
                self.one_sided_greater = true;
            }
            curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Less) => {
                self.two_tailed = false;
                self.one_sided_greater = false;
            }
        }
        self.alpha = spec.alpha;
        if let Some(tp) = spec.target_power {
            self.target_power = tp;
        }
        if let Some(n) = spec.sample_size {
            self.sample_size = n;
        }
        self.sample_size_group2 = spec.sample_size_group2;
        self.allocation_ratio = spec.allocation_ratio;
        self.chi_square_df = spec.chi_square_df;
        self.two_proportions_mode = match spec.two_proportions_mode {
            curvebook_doc::TwoProportionsMethod::PooledNullUnpooledAlt => TwoProportionsMethod::PooledNullUnpooledAlt,
            curvebook_doc::TwoProportionsMethod::Unpooled => TwoProportionsMethod::Unpooled,
            curvebook_doc::TwoProportionsMethod::CohensH => TwoProportionsMethod::CohensH,
        };
        self.continuity_correction = spec.continuity_correction;
        self.correlation_method = match spec.correlation_method {
            curvebook_doc::CorrelationPowerMethod::FisherZ => CorrelationPowerMethod::FisherZ,
            curvebook_doc::CorrelationPowerMethod::FisherZWithTCrit => CorrelationPowerMethod::FisherZWithTCrit,
            curvebook_doc::CorrelationPowerMethod::ExactTTransform => CorrelationPowerMethod::ExactTTransform,
            curvebook_doc::CorrelationPowerMethod::ConditionalSlopeT => CorrelationPowerMethod::ConditionalSlopeT,
        };
        if let Some(k) = spec.num_groups {
            self.anova_groups = k.max(2);
        }
        match &spec.effect {
            curvebook_doc::PowerEffectSpec::Unspecified => {}
            curvebook_doc::PowerEffectSpec::Standardized(d) => {
                self.effect_mode = PowerEffectInputMode::Standardized;
                self.standardized_effect = *d;
            }
            curvebook_doc::PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => {
                self.effect_mode = PowerEffectInputMode::MeansAndSd;
                self.mean_diff = *mean_diff;
                self.sd = *sd;
                self.sd_group2 = *sd_group2;
                if let Some(rho) = within_pair_correlation {
                    self.within_pair_correlation = *rho;
                }
            }
            curvebook_doc::PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => {
                self.effect_mode = PowerEffectInputMode::PairedDiffDirect;
                self.mean_diff = *mean_diff;
                self.paired_sd_diff = *sd_diff;
            }
            curvebook_doc::PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
                self.effect_mode = PowerEffectInputMode::MeansAndSd;
                if !means.is_empty() {
                    self.group_means = means.clone();
                    self.anova_groups = spec.num_groups.unwrap_or(means.len()).max(2);
                }
                self.sd = *within_sd;
            }
            curvebook_doc::PowerEffectSpec::TwoProportions { p1, p2 } => {
                self.effect_mode = PowerEffectInputMode::Proportions;
                self.p1 = *p1;
                self.p2 = *p2;
            }
            curvebook_doc::PowerEffectSpec::BaselineProportion { p1 } => {
                self.effect_mode = PowerEffectInputMode::Proportions;
                self.p1 = *p1;
            }
            curvebook_doc::PowerEffectSpec::RSquared(r2) => {
                self.effect_mode = PowerEffectInputMode::RegressionRSquared;
                self.r_squared = *r2;
            }
            curvebook_doc::PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => {
                self.effect_mode = PowerEffectInputMode::RegressionSlope;
                self.beta1 = *beta1;
                self.sigma_x = *sigma_x;
                self.sigma_eps = *sigma_eps;
            }
            curvebook_doc::PowerEffectSpec::HazardRatio { hazard_ratio, event_prob_group1, .. } => {
                self.survival_hazard_ratio = *hazard_ratio;
                if let Some(p1) = event_prob_group1 {
                    self.survival_event_prob = *p1;
                }
            }
        }
        self.created_sheets = None;
        self.clear_hover();
        let _ = self.recalculate();
    }

    fn build_effect_spec(&self) -> PowerEffectSpec {
        if self.solve_target == PowerSolveTarget::DetectableEffect {
            return match self.family {
                PowerTestFamily::TwoProportions if self.effect_mode == PowerEffectInputMode::Proportions => {
                    PowerEffectSpec::BaselineProportion { p1: self.p1 }
                }
                _ => PowerEffectSpec::Unspecified,
            };
        }

        match (self.family, self.effect_mode) {
            (_, PowerEffectInputMode::Standardized) => PowerEffectSpec::Standardized(self.standardized_effect),
            (PowerTestFamily::TwoSamplePairedT, PowerEffectInputMode::MeansAndSd) => PowerEffectSpec::MeanDiffAndSd {
                mean_diff: self.mean_diff,
                sd: self.sd,
                sd_group2: self.sd_group2,
                within_pair_correlation: Some(self.within_pair_correlation),
            },
            (PowerTestFamily::TwoSamplePairedT, PowerEffectInputMode::PairedDiffDirect) => {
                PowerEffectSpec::PairedDiff { mean_diff: self.mean_diff, sd_diff: self.paired_sd_diff }
            }
            (PowerTestFamily::OneWayAnova, PowerEffectInputMode::MeansAndSd) => {
                PowerEffectSpec::GroupMeansAndSd { means: self.group_means.clone(), within_sd: self.sd }
            }
            (_, PowerEffectInputMode::MeansAndSd) => {
                PowerEffectSpec::MeanDiffAndSd { mean_diff: self.mean_diff, sd: self.sd, sd_group2: self.sd_group2, within_pair_correlation: None }
            }
            (PowerTestFamily::TwoProportions, PowerEffectInputMode::Proportions) => PowerEffectSpec::TwoProportions { p1: self.p1, p2: self.p2 },
            (PowerTestFamily::SimpleLinearRegression, PowerEffectInputMode::RegressionRSquared) => PowerEffectSpec::RSquared(self.r_squared),
            (PowerTestFamily::SimpleLinearRegression, PowerEffectInputMode::RegressionSlope) => {
                PowerEffectSpec::RegressionSlope { beta1: self.beta1, sigma_x: self.sigma_x, sigma_eps: self.sigma_eps }
            }
            _ => PowerEffectSpec::Standardized(self.standardized_effect),
        }
    }

    /// Recompute the power/sample-size/MDE result, updating `result` or `validation_error` live.
    pub fn recalculate(&mut self) -> Result<&PowerAnalysisResult, String> {
        let req = self.to_request();
        match solve_power(&req) {
            Ok(res) => {
                self.validation_error = None;
                let hover_frac = self.hover_fraction_x;
                self.result = Some(res);
                if let Some(frac) = hover_frac {
                    self.hover_at_fraction(frac);
                } else {
                    self.clear_hover();
                }
                match self.result.as_ref() {
                    Some(stored) => Ok(stored),
                    None => Err(String::from("power result missing after calculation")),
                }
            }
            Err(err) => {
                let msg = err.to_string();
                self.validation_error = Some(msg.clone());
                self.result = None;
                self.clear_hover();
                Err(msg)
            }
        }
    }

    /// Validate whether the wizard can advance from `self.stage` toward `Results`.
    pub fn can_advance(&self) -> Result<(), String> {
        if !self.alpha.is_finite() || !(0.0..1.0).contains(&self.alpha) || self.alpha <= 0.0 {
            return Err(String::from("Significance alpha must lie in (0, 1)."));
        }
        if self.solve_target != PowerSolveTarget::Power
            && (!self.target_power.is_finite() || self.target_power <= self.alpha || self.target_power >= 1.0)
        {
            return Err(String::from("Target power (1 - beta) must exceed alpha and be strictly less than 1.0."));
        }
        if self.stage == PowerWizardStage::EffectSize
            && self.solve_target != PowerSolveTarget::DetectableEffect
            && self.derived_standardized_effect().is_none_or(|v| v == 0.0)
        {
            return Err(String::from("Enter a non-zero finite effect size or raw group parameters."));
        }
        if matches!(self.stage, PowerWizardStage::SignificanceAndSolve | PowerWizardStage::Results)
            && let Some(ref err) = self.validation_error
        {
            return Err(err.clone());
        }
        Ok(())
    }

    /// Advance to the next stage if validation succeeds (running `recalculate` entering Stage 4).
    pub fn advance(&mut self) -> Result<PowerWizardStage, String> {
        self.can_advance()?;
        let next = self.stage.next().ok_or_else(|| String::from("Already on the final Power Wizard stage."))?;
        if matches!(next, PowerWizardStage::SignificanceAndSolve | PowerWizardStage::Results) {
            self.recalculate()?;
        }
        self.stage = next;
        Ok(next)
    }

    /// Step back to the previous stage.
    pub fn go_back(&mut self) -> Result<PowerWizardStage, String> {
        let prev = self.stage.prev().ok_or_else(|| String::from("Already on the first Power Wizard stage."))?;
        self.stage = prev;
        Ok(prev)
    }

    /// Move sequentially between wizard stages (`advance` validates each intermediate stage).
    pub fn step_to_stage(&mut self, target: PowerWizardStage) -> Result<(), String> {
        while self.stage.index() < target.index() {
            let _ = self.advance()?;
        }
        while self.stage.index() > target.index() {
            let _ = self.go_back()?;
        }
        Ok(())
    }

    /// Clear any Stage 4 curve hover readout.
    pub fn clear_hover(&mut self) {
        self.hover_n = None;
        self.hover_power = None;
        self.hover_fraction_x = None;
    }

    /// Update the Stage 4 interactive power-curve hover readout from normalized plot X in `[0.0, 1.0]`.
    pub fn hover_at_fraction(&mut self, fraction_x: f64) -> Option<(f64, f64)> {
        let curve = &self.result.as_ref()?.power_curve;
        if curve.is_empty() {
            return None;
        }
        let f = fraction_x.clamp(0.0, 1.0);
        let n_min = curve.first().map_or(2.0, |p| p.n);
        let n_max = curve.last().map_or(n_min, |p| p.n);
        let target_n = n_min + f * (n_max - n_min);
        self.hover_fraction_x = Some(f);
        self.hover_at_n(target_n)
    }

    /// Interpolate/evaluate `(hover_n, hover_power)` on the Stage 4 curve at sample size `n`.
    pub fn hover_at_n(&mut self, n: f64) -> Option<(f64, f64)> {
        let curve = &self.result.as_ref()?.power_curve;
        if curve.is_empty() || !n.is_finite() {
            return None;
        }
        let clamped_n = n.clamp(curve.first().map_or(n, |p| p.n), curve.last().map_or(n, |p| p.n));
        let power = interpolate_curve_power(curve, clamped_n)?;
        self.hover_n = Some(clamped_n);
        self.hover_power = Some(power);
        Some((clamped_n, power))
    }

    /// Execute Stage 4 `Create` through `Session::execute_as_ui` (`Caller::Ui`), creating both
    /// Build the Power Curve DataSheet, ResultSheet, InfoSheet, and GraphSheet using a caller-provided
    /// command executor so batched UI transactions can record each engine step.
    pub fn execute_create_with(
        &mut self,
        exec: &mut dyn FnMut(&str, Value) -> Result<curvebook_engine::CommandOutcome, EngineError>,
    ) -> Result<PowerWizardCreatedSheets, EngineError> {
        self.recalculate().map_err(|msg| EngineError::with_code(ErrorCode::BadInput, msg))?;
        let res = self.result.clone().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "power calculation produced no result"))?;

        let mut undoable_steps = 0_usize;
        let run_result = self.execute_create_steps(&res, &mut |cmd_id, params| {
            let outcome = exec(cmd_id, params)?;
            if outcome.mutated {
                undoable_steps = undoable_steps.saturating_add(1);
            }
            Ok(outcome)
        });
        match run_result {
            Ok(created) => {
                self.created_sheets = Some(created.clone());
                Ok(created)
            }
            Err(err) => {
                for _ in 0..undoable_steps {
                    let _ = exec("edit.undo", json!({}));
                }
                self.created_sheets = None;
                Err(err)
            }
        }
    }

    fn execute_create_steps(
        &self,
        res: &PowerAnalysisResult,
        exec: &mut dyn FnMut(&str, Value) -> Result<curvebook_engine::CommandOutcome, EngineError>,
    ) -> Result<PowerWizardCreatedSheets, EngineError> {
        let curve_rows = build_power_curve_table_rows(res);
        let create_data_outcome = exec(
            "sheet.createData",
            json!({
                "title": format!("{} — Power Curve Data", res.family.label()),
                "kind": "xy",
                "x_title": "Sample Size (N)",
                "cols": 2,
                "rows": curve_rows.len().max(2),
                "auto_info": true,
                "auto_graph": true,
            }),
        )?;

        let data_id_str = extract_str_field(&create_data_outcome.payload, "sheet_id", "data sheet")?;
        let info_id_str = create_data_outcome.payload.get("info_sheet_id").and_then(Value::as_str).map(ToOwned::to_owned);
        let graph_id_str = create_data_outcome.payload.get("graph_sheet_id").and_then(Value::as_str).map(ToOwned::to_owned);

        let x_values: Vec<f64> = res.power_curve.iter().map(|pt| pt.n).collect();
        let y_rows: Vec<Vec<f64>> = res.power_curve.iter().map(|pt| vec![pt.power, pt.total_n]).collect();

        exec(
            "table.setTitles",
            json!({
                "sheet_id": &data_id_str,
                "x_title": "Sample Size (N)",
                "titles": ["Statistical Power (1 - beta)", "Total N"],
            }),
        )?;
        exec("table.setXCells", json!({ "sheet_id": &data_id_str, "values": x_values }))?;
        exec("table.setCells", json!({ "sheet_id": &data_id_str, "rows": y_rows }))?;

        if let Some(ref info_id) = info_id_str {
            populate_power_info_sheet(exec, info_id, res)?;
        }
        if let Some(ref graph_id) = graph_id_str {
            configure_power_graph_sheet(exec, graph_id, res)?;
        }

        let info_sid =
            info_id_str.as_deref().map(SheetId::try_new).transpose().map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
        let power_doc_spec = self.to_doc_spec(info_sid.clone());
        let res_outcome = exec(
            "analysis.run",
            json!({
                "source_sheet_id": &data_id_str,
                "title": format!("Power & Sample Size — {}", res.family.label()),
                "kind": {
                    "PowerAnalysis": power_doc_spec
                }
            }),
        )?;
        let result_id_str = extract_str_field(&res_outcome.payload, "sheet_id", "result sheet")?;
        let _ = exec("sheet.select", json!({ "sheet_id": &result_id_str }));

        Ok(PowerWizardCreatedSheets {
            data_sheet_id: SheetId::try_new(&data_id_str).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
            info_sheet_id: info_sid,
            graph_sheet_id: graph_id_str
                .map(|s| SheetId::try_new(&s))
                .transpose()
                .map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
            result_sheet_id: SheetId::try_new(&result_id_str).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
        })
    }

    /// Build the Power Curve DataSheet, ResultSheet, InfoSheet, and GraphSheet into `session`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn execute_create(&mut self, session: &mut Session) -> Result<PowerWizardCreatedSheets, EngineError> {
        let res = session.execute_atomic_group(|s| self.execute_create_with(&mut |command_id, params| s.execute_as_ui(command_id, params)));
        if res.is_err() {
            self.created_sheets = None;
        }
        res
    }
}

fn populate_power_info_sheet(
    exec: &mut dyn FnMut(&str, Value) -> Result<curvebook_engine::CommandOutcome, EngineError>,
    info_id: &str,
    res: &PowerAnalysisResult,
) -> Result<(), EngineError> {
    let entries = [
        ("Test Family", json!(res.family.label()), "text"),
        ("Solve Target", json!(solve_target_label(res.solve_target)), "text"),
        ("Alpha", json!(res.alpha), "number"),
        ("Effect Size", json!(res.effect_size), "number"),
        ("N per group", json!(res.n_per_group as f64), "number"),
        ("Total N", json!(res.total_n as f64), "number"),
        ("Achieved Power", json!(res.achieved_power), "number"),
        ("Critical Value", json!(res.critical_value), "number"),
    ];
    for (name, value, kind) in entries {
        exec("info.setConstant", json!({ "sheet_id": info_id, "name": name, "value": value, "kind": kind }))?;
    }
    if let Some(n2) = res.n_group2 {
        exec("info.setConstant", json!({ "sheet_id": info_id, "name": "N group 2", "value": n2 as f64, "kind": "number" }))?;
    }
    let sensitivity_summary: Vec<String> = res
        .sensitivity_table
        .iter()
        .map(|cell| format!("alpha={:.3}, power={:.2} -> n={}, N={}", cell.alpha, cell.target_power, cell.sample_size, cell.total_n))
        .collect();
    let note = format!("{}\n\nSensitivity Table:\n{}", res.notes.join("\n"), sensitivity_summary.join("\n"));
    exec("info.setNote", json!({ "sheet_id": info_id, "text": note }))?;
    Ok(())
}

fn configure_power_graph_sheet(
    exec: &mut dyn FnMut(&str, Value) -> Result<curvebook_engine::CommandOutcome, EngineError>,
    graph_id: &str,
    res: &PowerAnalysisResult,
) -> Result<(), EngineError> {
    exec("graph.setKind", json!({ "sheet_id": graph_id, "kind": "xy", "reset_series_style": false, "option_id": "xy_points_connecting" }))?;
    exec(
        "graph.setSpec",
        json!({
            "sheet_id": graph_id,
            "title": format!("Power Curve — {}", res.family.label()),
            "initialized": true
        }),
    )?;
    exec(
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "x_axis": { "title": "Sample Size (N)" },
            "y_axis": { "title": "Statistical Power (1 - beta)", "min": 0.0, "max": 1.0 }
        }),
    )?;
    if let Some(tp) = res.target_power {
        exec(
            "graph.setAdditionalTicks",
            json!({
                "sheet_id": graph_id,
                "axis": "y",
                "ticks": [{ "value": tp, "label": format!("Target {:.0}%", tp * 100.0), "draw_gridline": true }]
            }),
        )?;
    }
    Ok(())
}

fn build_power_curve_table_rows(res: &PowerAnalysisResult) -> Vec<[f64; 2]> {
    res.power_curve.iter().map(|pt| [pt.power, pt.total_n]).collect()
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

fn extract_str_field(val: &Value, field: &str, label: &str) -> Result<String, EngineError> {
    val.get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, format!("missing `{field}` in {label} response")))
}

const fn solve_target_label(target: PowerSolveTarget) -> &'static str {
    match target {
        PowerSolveTarget::SampleSize => "Sample Size",
        PowerSolveTarget::Power => "Power",
        PowerSolveTarget::DetectableEffect => "Detectable Effect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::ProjectDocument;

    #[test]
    fn all_eight_families_solve_and_produce_curve_and_sensitivity() {
        let mut wizard = PowerWizardState::default();
        for family in PowerTestFamily::ALL {
            wizard.set_family(family);
            wizard.alpha = 0.05;
            wizard.target_power = 0.80;
            wizard.two_tailed = true;
            match family {
                PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => {
                    wizard.effect_mode = PowerEffectInputMode::Standardized;
                    wizard.standardized_effect = 0.65;
                }
                PowerTestFamily::OneWayAnova => {
                    wizard.effect_mode = PowerEffectInputMode::Standardized;
                    wizard.standardized_effect = 0.40;
                    wizard.anova_groups = 3;
                }
                PowerTestFamily::TwoProportions => {
                    wizard.effect_mode = PowerEffectInputMode::Proportions;
                    wizard.p1 = 0.25;
                    wizard.p2 = 0.55;
                }
                PowerTestFamily::Correlation | PowerTestFamily::ChiSquare => {
                    wizard.effect_mode = PowerEffectInputMode::Standardized;
                    wizard.standardized_effect = 0.35;
                }
                PowerTestFamily::SimpleLinearRegression => {
                    wizard.effect_mode = PowerEffectInputMode::Standardized;
                    wizard.standardized_effect = 0.15;
                }
                PowerTestFamily::LogRankSurvival => continue,
            }
            let res = wizard.recalculate().unwrap_or_else(|e| panic!("{family:?} failed: {e}"));
            assert!(res.total_n >= res.n_per_group);
            assert!(res.achieved_power >= 0.80);
            assert!(!res.power_curve.is_empty());
            assert!(!res.sensitivity_table.is_empty());
        }
    }

    #[test]
    fn paired_correlation_converts_sigma_to_sigma_d() {
        let mut wizard = PowerWizardState::default();
        wizard.set_family(PowerTestFamily::TwoSamplePairedT);
        wizard.effect_mode = PowerEffectInputMode::MeansAndSd;
        wizard.mean_diff = 1.0;
        wizard.sd = 2.0;
        wizard.within_pair_correlation = 0.50;
        // sigma_d = 2 * sqrt(2 * (1 - 0.5)) = 2.0 -> d_z = 1.0 / 2.0 = 0.50
        let sigma_d = wizard.computed_paired_sigma_d().expect("finite sigma_d");
        assert!((sigma_d - 2.0).abs() < 1e-12);
        let dz = wizard.derived_standardized_effect().expect("finite dz");
        assert!((dz - 0.5).abs() < 1e-12);
        let res = wizard.recalculate().expect("paired t solves cleanly");
        assert_eq!(res.n_per_group, 34);
    }

    #[test]
    fn stage4_hover_and_live_recalc_work_consistently() {
        let mut wizard = PowerWizardState { standardized_effect: 0.80, ..Default::default() };
        let _ = wizard.advance().expect("stage 1 -> 2");
        let _ = wizard.advance().expect("stage 2 -> 3");
        let _ = wizard.advance().expect("stage 3 -> 4");
        assert_eq!(wizard.stage, PowerWizardStage::Results);

        let n_at_80 = wizard.result.as_ref().expect("result").n_per_group;
        let (hn, hp) = wizard.hover_at_fraction(0.5).expect("hover");
        assert!(hn > 0.0);
        assert!((0.0..=1.0).contains(&hp));

        wizard.alpha = 0.01;
        let res_01 = wizard.recalculate().expect("recalc at alpha=0.01");
        assert!(res_01.n_per_group > n_at_80);
        assert!(wizard.hover_power.is_some());
    }

    #[test]
    fn create_emits_linked_result_and_graph() {
        let mut doc = ProjectDocument::new("Power Plan").expect("doc");
        let ds = curvebook_doc::DataSheet::new(
            SheetId::try_new("data-base").expect("id"),
            "Base",
            curvebook_doc::TableArchetype::Column(
                curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 2, 4).expect("col"),
            ),
        )
        .expect("ds");
        doc.add_data_sheet(ds).expect("add");
        let mut session = Session::from_document(doc);

        let mut wizard = PowerWizardState::default();
        let created = wizard.execute_create(&mut session).expect("gui create succeeds");
        assert!(session.document().result_sheet(&created.result_sheet_id).is_some());
        let graph_id = created.graph_sheet_id.expect("linked graph created");
        let graph = session.document().graph_sheet(&graph_id).expect("graph present");
        assert!(graph.initialized);
        assert!(graph.title.contains("Power Curve"));
    }
}
