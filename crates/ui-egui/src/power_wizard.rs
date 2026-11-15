//! Standalone 4-stage Power & Sample Size Wizard backed by `lumencraft-stats` (`R8.7`, `WF-15`).
//!
//! Provides complete state, validation, Stage 4 curve hover inspection, sensitivity table
//! projection, live sidebar recalculation, and GUI `Create` command emission producing both a
//! Power & Sample Size [`lumencraft_doc::ResultSheet`] and linked [`lumencraft_doc::GraphSheet`].

use lumencraft_doc::SheetId;
use lumencraft_engine::{EngineError, ErrorCode, Session};
use lumencraft_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerAnalysisResult, PowerCurvePoint, PowerEffectSpec, PowerSolveTarget,
    PowerTestFamily, SensitivityCell, TailMode, TwoProportionsMethod, cohens_d_from_means, cohens_dz_from_correlation, cohens_f_from_means,
    paired_sd_from_correlation, solve_power,
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
        };
        let _ = state.recalculate();
        state
    }
}

impl PowerWizardState {
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
                if self.effect_mode == PowerEffectInputMode::Proportions || self.effect_mode == PowerEffectInputMode::RegressionRSquared {
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
            (PowerTestFamily::TwoProportions, PowerEffectInputMode::Proportions) => lumencraft_stats::power::cohens_h(self.p1, self.p2).ok(),
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

    /// Return sensitivity table cells grouped cleanly or directly from the solved result.
    #[must_use]
    pub fn sensitivity_cells(&self) -> &[SensitivityCell] {
        self.result.as_ref().map_or(&[], |r| r.sensitivity_table.as_slice())
    }

    /// Format compact Stage 4 headline metric lines for compositor rendering & `ui.inspect`.
    #[must_use]
    pub fn summary_lines(&self) -> Vec<String> {
        let Some(ref res) = self.result else {
            return vec![
                format!("Family: {}", self.family.label()),
                self.validation_error.as_deref().unwrap_or("Adjust parameters to solve.").to_owned(),
            ];
        };
        let mut lines = vec![
            format!("Family: {} ({})", res.family.label(), solve_target_label(res.solve_target)),
            format!("alpha = {:.4} | effect = {:.4} | power = {:.4}", res.alpha, res.effect_size, res.achieved_power),
            match res.n_group2 {
                Some(n2) => format!("Required N: n1 = {}, n2 = {}, total = {}", res.n_per_group, n2, res.total_n),
                None => format!("Required N: per group = {}, total = {}", res.n_per_group, res.total_n),
            },
            format!("Critical = {:.4} | NCP = {:.4}", res.critical_value, res.noncentrality_param),
        ];
        if let (Some(hn), Some(hp)) = (self.hover_n, self.hover_power) {
            lines.push(format!("Hover curve: N = {:.1}, Power = {:.4}", hn, hp));
        }
        lines
    }

    /// Apply a UI patch map (used by `ui.answer_dialog` and dialog form updates) and live-recalculate.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            self.apply_single_field(key, val)?;
        }
        let _ = self.recalculate();
        Ok(())
    }

    fn apply_single_field(&mut self, key: &str, val: &Value) -> Result<(), String> {
        match key {
            "stage" | "power.stage" => self.patch_stage(val)?,
            "family" | "power.family" => self.patch_family(val)?,
            "solve_target" | "power.solve_target" => self.patch_solve_target(val)?,
            "effect_mode" | "power.effect_mode" => self.patch_effect_mode(val)?,
            "standardized_effect" | "power.standardized_effect" | "effect_size" | "d" => {
                self.standardized_effect = parse_num(val, key)?;
            }
            "mean_diff" | "power.mean_diff" => self.mean_diff = parse_num(val, key)?,
            "sd" | "sigma" | "power.sd" => self.sd = parse_num(val, key)?,
            "sd_group2" | "power.sd_group2" => self.sd_group2 = val.as_f64(),
            "within_pair_correlation" | "r" | "power.within_pair_correlation" => {
                self.within_pair_correlation = parse_num(val, key)?;
            }
            "paired_sd_diff" | "sigma_d" | "power.paired_sd_diff" => self.paired_sd_diff = parse_num(val, key)?,
            "anova_groups" | "num_groups" | "k" | "power.anova_groups" => {
                let k = parse_positive_usize_arg(val, key)?;
                self.anova_groups = k.max(2);
            }
            "group_means" | "power.group_means" => self.patch_group_means(val)?,
            "p1" | "power.p1" => self.p1 = parse_num(val, key)?,
            "p2" | "power.p2" => self.p2 = parse_num(val, key)?,
            "r_squared" | "power.r_squared" => self.r_squared = parse_num(val, key)?,
            "beta1" | "power.beta1" => self.beta1 = parse_num(val, key)?,
            "sigma_x" | "power.sigma_x" => self.sigma_x = parse_num(val, key)?,
            "sigma_eps" | "power.sigma_eps" => self.sigma_eps = parse_num(val, key)?,
            "two_tailed" | "power.two_tailed" => {
                self.two_tailed = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
            }
            "one_sided_greater" | "power.one_sided_greater" => {
                self.one_sided_greater = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
            }
            "tail_mode" | "power.tail_mode" => self.patch_tail_mode(val)?,
            "alpha" | "power.alpha" => self.alpha = parse_num(val, key)?,
            "target_power" | "power" | "power.target_power" => self.target_power = parse_num(val, key)?,
            "sample_size" | "n" | "power.sample_size" => self.sample_size = parse_num(val, key)?,
            "sample_size_group2" | "n2" | "power.sample_size_group2" => self.sample_size_group2 = val.as_f64(),
            "allocation_ratio" | "power.allocation_ratio" => self.allocation_ratio = parse_num(val, key)?,
            "chi_square_df" | "df" | "power.chi_square_df" => self.chi_square_df = parse_num(val, key)?,
            "continuity_correction" | "power.continuity_correction" => {
                self.continuity_correction = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
            }
            "hover_n" | "power.hover_n" => {
                let hn = parse_num(val, key)?;
                if self.result.is_none() {
                    let _ = self.recalculate();
                }
                let _ = self.hover_at_n(hn);
            }
            "hover_fraction_x" | "power.hover_fraction_x" => {
                let f = parse_num(val, key)?;
                if self.result.is_none() {
                    let _ = self.recalculate();
                }
                let _ = self.hover_at_fraction(f);
            }
            _ => {}
        }
        Ok(())
    }

    fn patch_stage(&mut self, val: &Value) -> Result<(), String> {
        if let Some(idx) = val.as_u64() {
            if let Some(st) = PowerWizardStage::from_index(idx as usize) {
                self.stage = st;
                return Ok(());
            }
            return Err(format!("invalid power wizard stage index `{idx}`"));
        }
        if let Some(raw) = val.as_str() {
            let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
            self.stage = match norm.as_str() {
                "1" | "experimentdesign" | "design" => PowerWizardStage::ExperimentDesign,
                "2" | "effectsize" | "effect" => PowerWizardStage::EffectSize,
                "3" | "significanceandsolve" | "significance" => PowerWizardStage::SignificanceAndSolve,
                "4" | "results" | "resultssensitivity" => PowerWizardStage::Results,
                _ => return Err(format!("unknown power wizard stage `{raw}`")),
            };
            return Ok(());
        }
        Err(String::from("`stage` must be a string or stage index"))
    }

    fn patch_family(&mut self, val: &Value) -> Result<(), String> {
        let raw = val.as_str().ok_or_else(|| String::from("`family` must be a string"))?;
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        let family = match norm.as_str() {
            "unpairedt" | "twosampleunpairedt" | "independentt" => PowerTestFamily::TwoSampleUnpairedT,
            "pairedt" | "twosamplepairedt" | "dependentt" => PowerTestFamily::TwoSamplePairedT,
            "onesamplet" | "singlesamplet" => PowerTestFamily::OneSampleT,
            "onewayanova" | "anova" => PowerTestFamily::OneWayAnova,
            "twoproportions" | "proportions" => PowerTestFamily::TwoProportions,
            "correlation" | "pearsoncorrelation" => PowerTestFamily::Correlation,
            "chisquare" | "chi2" => PowerTestFamily::ChiSquare,
            "simplelinearregression" | "linearregression" | "regression" => PowerTestFamily::SimpleLinearRegression,
            _ => return Err(format!("unknown power test family `{raw}`")),
        };
        self.set_family(family);
        Ok(())
    }

    fn patch_solve_target(&mut self, val: &Value) -> Result<(), String> {
        let raw = val.as_str().ok_or_else(|| String::from("`solve_target` must be a string"))?;
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        self.solve_target = match norm.as_str() {
            "samplesize" | "n" => PowerSolveTarget::SampleSize,
            "power" => PowerSolveTarget::Power,
            "detectableeffect" | "effect" | "mde" => PowerSolveTarget::DetectableEffect,
            _ => return Err(format!("unknown power solve target `{raw}`")),
        };
        Ok(())
    }

    fn patch_effect_mode(&mut self, val: &Value) -> Result<(), String> {
        let raw = val.as_str().ok_or_else(|| String::from("`effect_mode` must be a string"))?;
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        self.effect_mode = match norm.as_str() {
            "standardized" | "direct" => PowerEffectInputMode::Standardized,
            "meansandsd" | "rawmeans" | "means" => PowerEffectInputMode::MeansAndSd,
            "paireddiffdirect" | "paireddiff" | "sigmad" => PowerEffectInputMode::PairedDiffDirect,
            "proportions" | "twoproportions" => PowerEffectInputMode::Proportions,
            "regressionrsquared" | "rsquared" | "r2" => PowerEffectInputMode::RegressionRSquared,
            "regressionslope" | "slope" => PowerEffectInputMode::RegressionSlope,
            _ => return Err(format!("unknown power effect mode `{raw}`")),
        };
        Ok(())
    }

    fn patch_group_means(&mut self, val: &Value) -> Result<(), String> {
        let arr = val.as_array().ok_or_else(|| String::from("`group_means` must be an array of numbers"))?;
        let mut parsed = Vec::with_capacity(arr.len());
        for v in arr {
            parsed.push(parse_num(v, "group_means")?);
        }
        if parsed.len() < 2 {
            return Err(String::from("`group_means` must contain at least 2 group means"));
        }
        self.anova_groups = parsed.len();
        self.group_means = parsed;
        Ok(())
    }

    fn patch_tail_mode(&mut self, val: &Value) -> Result<(), String> {
        let raw = val.as_str().ok_or_else(|| String::from("`tail_mode` must be a string"))?;
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        match norm.as_str() {
            "twosided" | "twotailed" | "two" => {
                self.two_tailed = true;
            }
            "onesidedgreater" | "onetailedgreater" | "onesided" | "onetailed" | "greater" => {
                if !self.family.supports_one_sided() {
                    return Err(format!("{} only supports two-tailed tests", self.family.label()));
                }
                self.two_tailed = false;
                self.one_sided_greater = true;
            }
            "onesidedless" | "onetailedless" | "less" => {
                if !self.family.supports_one_sided() {
                    return Err(format!("{} only supports two-tailed tests", self.family.label()));
                }
                self.two_tailed = false;
                self.one_sided_greater = false;
            }
            _ => return Err(format!("unknown tail mode `{raw}`")),
        }
        Ok(())
    }

    /// Execute Stage 4 `Create` through `Session::execute_as_ui` (`Caller::Ui`), creating both
    /// Build the Power Curve DataSheet, ResultSheet, InfoSheet, and GraphSheet using a caller-provided
    /// command executor so batched UI transactions can record each engine step.
    pub fn execute_create_with(
        &mut self,
        exec: &mut dyn FnMut(&str, Value) -> Result<lumencraft_engine::CommandOutcome, EngineError>,
    ) -> Result<PowerWizardCreatedSheets, EngineError> {
        self.recalculate().map_err(|msg| EngineError::with_code(ErrorCode::BadInput, msg))?;
        let res = self.result.clone().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "power calculation produced no result"))?;

        let curve_rows = build_power_curve_table_rows(&res);
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
            populate_power_info_sheet(exec, info_id, &res)?;
        }
        if let Some(ref graph_id) = graph_id_str {
            configure_power_graph_sheet(exec, graph_id, &res)?;
        }

        let res_outcome = exec(
            "analysis.run",
            json!({
                "source_sheet_id": &data_id_str,
                "title": format!("Power & Sample Size — {}", res.family.label()),
                "kind": {
                    "Transform": {
                        "PruneRows": {
                            "every_k_rows": 1,
                            "average_window": false
                        }
                    }
                }
            }),
        )?;
        let result_id_str = extract_str_field(&res_outcome.payload, "sheet_id", "result sheet")?;
        let _ = exec("sheet.select", json!({ "sheet_id": &result_id_str }));

        let created = PowerWizardCreatedSheets {
            data_sheet_id: SheetId::try_new(&data_id_str).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
            info_sheet_id: info_id_str
                .map(|s| SheetId::try_new(&s))
                .transpose()
                .map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
            graph_sheet_id: graph_id_str
                .map(|s| SheetId::try_new(&s))
                .transpose()
                .map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
            result_sheet_id: SheetId::try_new(&result_id_str).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?,
        };
        self.created_sheets = Some(created.clone());
        Ok(created)
    }

    /// Build the Power Curve DataSheet, ResultSheet, InfoSheet, and GraphSheet into `session`.
    pub fn execute_create(&mut self, session: &mut Session) -> Result<PowerWizardCreatedSheets, EngineError> {
        self.execute_create_with(&mut |command_id, params| session.execute_as_ui(command_id, params))
    }
}

fn populate_power_info_sheet(
    exec: &mut dyn FnMut(&str, Value) -> Result<lumencraft_engine::CommandOutcome, EngineError>,
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
    exec: &mut dyn FnMut(&str, Value) -> Result<lumencraft_engine::CommandOutcome, EngineError>,
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

fn parse_num(val: &Value, field: &str) -> Result<f64, String> {
    let parsed = val.as_f64().or_else(|| val.as_str().and_then(|s| s.trim().parse::<f64>().ok()));
    parsed.filter(|v| v.is_finite()).ok_or_else(|| format!("`{field}` must be a finite number"))
}

fn parse_positive_usize_arg(val: &Value, field: &str) -> Result<usize, String> {
    if let Some(u) = val.as_u64().filter(|&n| n >= 1) {
        return usize::try_from(u).map_err(|_| format!("`{field}` exceeds platform integer bounds"));
    }
    let f = parse_num(val, field).map_err(|_| format!("`{field}` must be a positive integer"))?;
    if f < 1.0 || (f - f.round()).abs() > 1e-9 || f > (usize::MAX as f64) {
        return Err(format!("`{field}` must be a positive integer"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(f.round() as usize)
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
    use lumencraft_doc::ProjectDocument;

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
    fn create_emits_linked_result_and_graph_under_gui_only() {
        let mut doc = ProjectDocument::new("Power Plan").expect("doc");
        let ds = lumencraft_doc::DataSheet::new(
            SheetId::try_new("data-base").expect("id"),
            "Base",
            lumencraft_doc::TableArchetype::Column(
                lumencraft_doc::ColumnTable::new(lumencraft_doc::ColumnDesignHint::UnpairedReplicates, 2, 4).expect("col"),
            ),
        )
        .expect("ds");
        doc.add_data_sheet(ds).expect("add");
        let mut session = Session::from_document(doc);
        session.set_gui_only(lumencraft_engine::GuiOnlyPolicy::Strict);

        let mut wizard = PowerWizardState::default();
        let created = wizard.execute_create(&mut session).expect("gui create succeeds even in gui_only");
        assert!(session.document().result_sheet(&created.result_sheet_id).is_some());
        let graph_id = created.graph_sheet_id.expect("linked graph created");
        let graph = session.document().graph_sheet(&graph_id).expect("graph present");
        assert!(graph.initialized);
        assert!(graph.title.contains("Power Curve"));
    }

    #[test]
    fn anova_groups_accepts_integers_floats_and_numeric_strings() {
        let mut wizard = PowerWizardState::default();
        let mut patch = serde_json::Map::new();
        patch.insert(String::from("family"), json!("one_way_anova"));
        patch.insert(String::from("power.anova_groups"), json!("4"));
        wizard.apply_field_patch(&patch).expect("numeric string anova_groups");
        assert_eq!(wizard.anova_groups, 4);

        patch.insert(String::from("power.anova_groups"), json!(5.0));
        wizard.apply_field_patch(&patch).expect("float anova_groups");
        assert_eq!(wizard.anova_groups, 5);

        patch.insert(String::from("power.anova_groups"), json!("6.0"));
        wizard.apply_field_patch(&patch).expect("float string anova_groups");
        assert_eq!(wizard.anova_groups, 6);

        for invalid in [json!(0), json!(-2), json!("abc"), json!(2.5)] {
            patch.insert(String::from("power.anova_groups"), invalid);
            assert!(wizard.apply_field_patch(&patch).is_err());
        }
    }
}
