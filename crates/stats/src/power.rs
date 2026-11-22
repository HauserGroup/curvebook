//! Exact statistical power, sample-size, and minimum-detectable-effect calculation engine (`R8.7`).
//!
//! Backed by tail-safe noncentral *t*, noncentral *F*, noncentral chi-square, and normal
//! distributions from [`curvebook_math::dist`], plus Brent-Dekker root finding from
//! [`curvebook_math::roots`].
//!
//! Supported test families ([`PowerTestFamily`]):
//! 1. `TwoSampleUnpairedT`: equal or unequal allocation ratio `k = n2 / n1`, Cohen's `d` or raw group means & pooled SD.
//! 2. `TwoSamplePairedT`: direct paired `d_z` / `(mean_diff, sd_d)` or group SD + within-pair correlation `r` via `sigma_d = sigma * sqrt(2 * (1 - r))`.
//! 3. `OneSampleT`: single-sample Student *t*-test against `mu_0`.
//! 4. `OneWayAnova`: balanced `k >= 2` groups, Cohen's `f` or group means & within-group SD (`lambda = k * n * f^2`).
//! 5. `TwoProportions`: normal approximation (`PooledNullUnpooledAlt` Fleiss/G*Power, `Unpooled` Wald, or `CohensH` arcsine) with optional continuity correction.
//! 6. `Correlation`: Pearson `r` vs `H0: r_0 = 0` (Fisher *z*, R `pwr.r.test` hybrid, or conditional *t*-transform).
//! 7. `ChiSquare`: goodness-of-fit / contingency test parameterized by Cohen's `w` and `df`.
//! 8. `SimpleLinearRegression`: slope *t*-test / `u = 1` *F*-test parameterized by Cohen's `f^2 = R^2 / (1 - R^2)`, `R^2`, or `(beta1, sigma_x, sigma_eps)` (`lambda = n * f^2`, `df = n - 2`).
//!
//! Effect-size conventions (`d`, `d_z`, `f`, `h`, `r`, `w`, `f^2`) and the noncentrality parameters
//! follow Cohen (1988); *t*-test power via the noncentral *t* follows Owen (1965). Per-family sources
//! are cited on the evaluators below in short `Author (Year)` form.
//!
//! References:
//! - Chow, S.-C., Shao, J., & Wang, H. (2008). *Sample Size Calculations in Clinical Research*
//!   (2nd ed.). Chapman & Hall/CRC.
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//!   Lawrence Erlbaum Associates.
//! - Fisher, R. A. (1921). On the "probable error" of a coefficient of correlation deduced from a
//!   small sample. *Metron*, 1, 3-32.
//! - Fleiss, J. L., Levin, B., & Paik, M. C. (2003). *Statistical Methods for Rates and Proportions*
//!   (3rd ed.). Wiley.
//! - Fleiss, J. L., Tytun, A., & Ury, H. K. (1980). A simple approximation for calculating sample
//!   sizes for comparing independent proportions. *Biometrics*, 36(2), 343-346. doi:10.2307/2529990.
//! - Owen, D. B. (1965). The power of Student's t-test. *Journal of the American Statistical
//!   Association*, 60(309), 320-333. doi:10.2307/2283156.

use curvebook_doc::{
    AnalysisSpec, Cell, ChecklistStatus, DataSheet, ResultCell, ResultSheet, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, XColumnMode,
    XyTable,
};
use curvebook_math::dist::{
    chi2_isf, f_isf, ncchi2_sf, ncf_sf, nct_cdf, nct_sf, norm_cdf, norm_isf, norm_ppf, norm_sf, norm_two_sided_isf, student_t_isf,
    student_t_two_sided_isf,
};
use curvebook_math::quad::gauss_legendre_32;
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::special::ln_gamma;
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::common::{build_tab, checklist_item, init_result_sheet, num_cell, opt_num_cell, push_scalar, sanitize_result_sheet_floats};
use crate::error::StatsError;

const MAX_CONTINUOUS_N: f64 = 1e12;
const MIN_ALLOCATION_RATIO: f64 = 1e-6;
const MAX_ALLOCATION_RATIO: f64 = 1e6;
const MAX_ANOVA_GROUPS: usize = 10_000;
const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0; // 2^53 - 1
const MAX_DISCRETE_VERIFY_STEPS: usize = 100_000;
const ROOT_TOL: RootOptions = RootOptions { rel_tol: 1e-12, abs_tol: 1e-12, max_iters: 128 };
const DEFAULT_SENSITIVITY_ALPHAS: &[f64] = &[0.10, 0.05, 0.01, 0.001];
const DEFAULT_SENSITIVITY_POWERS: &[f64] = &[0.70, 0.80, 0.90, 0.95];

// ============================================================================
// Domain Model & Configuration Types
// ============================================================================

/// Supported hypothesis test families for power and sample-size planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PowerTestFamily {
    /// Two-group independent Student *t* (or Welch–Satterthwaite *t* when `sd_group2` differs from `sd`).
    #[default]
    TwoSampleUnpairedT,
    /// Paired matched-pairs Student *t* (`n - 1` df) parameterized by `Cohen's d_z` or `(mean_diff, sd_diff / r)`.
    TwoSamplePairedT,
    /// Single-sample Student *t* against known null mean $\mu_0$ (`n - 1` df).
    OneSampleT,
    /// Omnibus *k*-group one-way fixed-effects ANOVA *F*-test (`df1 = k - 1`, `df2 = N - k`) parameterized by Cohen's *f*.
    OneWayAnova,
    /// Two independent binomial proportions test (`PooledNullUnpooledAlt`, `Unpooled`, or arcsine `CohensH`).
    TwoProportions,
    /// Single-sample Pearson correlation *r* test against $H_0: \rho = 0$ (`FisherZ`, `ExactTTransform`, etc.).
    Correlation,
    /// Pearson chi-square goodness-of-fit or independence test parameterized by Cohen's *w* and `chi_square_df`.
    ChiSquare,
    /// Conditional fixed-*x* simple linear regression slope *t*-test (`df = n - 2`, Cohen's $f^2 = R^2 / (1 - R^2)$).
    SimpleLinearRegression,
}

impl PowerTestFamily {
    /// All eight supported power-analysis test families in canonical wizard order.
    pub const ALL: [Self; 8] = [
        Self::TwoSampleUnpairedT,
        Self::TwoSamplePairedT,
        Self::OneSampleT,
        Self::OneWayAnova,
        Self::TwoProportions,
        Self::Correlation,
        Self::ChiSquare,
        Self::SimpleLinearRegression,
    ];

    /// Neutral display label for UI pickers and result summaries.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TwoSampleUnpairedT => "Unpaired t test",
            Self::TwoSamplePairedT => "Paired t test",
            Self::OneSampleT => "One-sample t test",
            Self::OneWayAnova => "One-way ANOVA",
            Self::TwoProportions => "Two proportions",
            Self::Correlation => "Pearson correlation",
            Self::ChiSquare => "Chi-square test",
            Self::SimpleLinearRegression => "Simple linear regression",
        }
    }

    /// Name of the canonical standardized effect size for this test family.
    #[must_use]
    pub const fn default_effect_label(self) -> &'static str {
        match self {
            Self::TwoSampleUnpairedT | Self::OneSampleT => "Cohen's d",
            Self::TwoSamplePairedT => "Cohen's dz",
            Self::OneWayAnova => "Cohen's f",
            Self::TwoProportions => "Cohen's h",
            Self::Correlation => "Pearson r",
            Self::ChiSquare => "Cohen's w",
            Self::SimpleLinearRegression => "Cohen's f^2",
        }
    }

    /// Whether one-sided alternatives (`Greater` / `Less`) are meaningful for this family.
    #[must_use]
    pub const fn supports_one_sided(self) -> bool {
        !matches!(self, Self::OneWayAnova | Self::ChiSquare)
    }

    /// Minimum valid integer primary sample size for this test family.
    #[must_use]
    pub const fn min_integer_n(self) -> usize {
        match self {
            Self::TwoSampleUnpairedT | Self::TwoSamplePairedT | Self::OneSampleT | Self::OneWayAnova => 2,
            Self::SimpleLinearRegression => 3,
            Self::Correlation => 4,
            Self::TwoProportions | Self::ChiSquare => 1,
        }
    }
}

/// Direction for one-sided hypothesis tests (`H1: theta > theta_0` vs `H1: theta < theta_0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OneSidedDirection {
    /// Upper-tail alternative (`effect > 0`).
    #[default]
    Greater,
    /// Lower-tail alternative (`effect < 0`).
    Less,
}

/// Hypothesis tail mode for power calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TailMode {
    /// Two-sided alternative hypothesis (`H1: effect != 0`).
    #[default]
    TwoSided,
    /// Directional one-sided alternative specified by [`OneSidedDirection`].
    OneSided(OneSidedDirection),
}

impl TailMode {
    /// Convenience constant for upper-tail one-sided tests (`H1: effect > 0`).
    pub const ONE_SIDED_GREATER: Self = Self::OneSided(OneSidedDirection::Greater);
    /// Convenience constant for lower-tail one-sided tests (`H1: effect < 0`).
    pub const ONE_SIDED_LESS: Self = Self::OneSided(OneSidedDirection::Less);
}

/// Unknown quantity solved by [`solve_power`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PowerSolveTarget {
    /// Solve for required continuous and ceiling integer sample size (`n1`, `n2`, `total_n`) given `alpha`, `target_power`, and effect.
    #[default]
    SampleSize,
    /// Solve for exact statistical power `1 - beta` given `alpha`, `sample_size`, and effect.
    Power,
    /// Solve for minimum detectable standardized and raw effect size given `alpha`, `target_power`, and `sample_size`.
    DetectableEffect,
}

/// Variance / standard-error approximation used by [`PowerTestFamily::TwoProportions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TwoProportionsMethod {
    /// Pooled proportion `p_bar` under `H0` and unpooled SE under `H1` (Fleiss / G*Power standard):
    /// the normal-approximation power formula of Fleiss, Levin & Paik (2003).
    #[default]
    PooledNullUnpooledAlt,
    /// Unpooled Wald SE under both `H0` and `H1` (Chow, Shao & Wang 2008).
    Unpooled,
    /// Arcsine-transformed Cohen's `h = 2 asin(sqrt(p1)) - 2 asin(sqrt(p2))` (`pwr::pwr.2p.test`);
    /// Cohen (1988, ch. 6), with `z = h * sqrt(n1 n2 / (n1 + n2))` for unequal `n`.
    CohensH,
}

/// Power-curve convention used by [`PowerTestFamily::Correlation`] (`H0: rho = 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CorrelationPowerMethod {
    /// Standard Fisher *z*-transformation (`z = atanh(r)`, `SE = 1 / sqrt(n - 3)`; Fisher 1921):
    /// yields exact size `alpha` at `r = 0` and is strictly monotonic for all `n > 3`.
    #[default]
    FisherZ,
    /// R `pwr::pwr.r.test` convention: exact *t*-transform critical threshold `r_c` under `H0`
    /// paired with bias-corrected Fisher *z* (`atanh(r) + r / (2*(n-1))`, `SE = 1/sqrt(n-3)`).
    /// The bias term `rho / (2(n - 1))` is Fisher's (1921) approximation to the mean of `atanh(r)`.
    FisherZWithTCrit,
    /// Exact unconditional distribution of sample Pearson correlation *r* under bivariate normality
    /// (`rho != 0`, `n > 2`; Fisher 1915, *Biometrika* 10:507–521; Hotelling 1953, *JRSS-B*
    /// 15:193–232; Anderson 2003, Ch. 4). Integrates the exact hypergeometric density of *r* over
    /// the exact `t_{n - 2}` rejection region in Fisher `z = atanh(r)` space.
    ExactTTransform,
    /// Conditional fixed-*x* regression slope *t*-test (`df = n - 2`,
    /// `delta = (r / sqrt(1 - r^2)) * sqrt(n)`): uses the same `lambda = n * f^2` noncentrality
    /// convention (`f^2 = r^2 / (1 - r^2)`) as [`PowerTestFamily::SimpleLinearRegression`]
    /// (Cohen 1988, Ch. 9; Chow, Shao & Wang 2008, §10.2), so fixed-*x* correlation and simple
    /// linear regression with `R^2 = r^2` agree identically.
    ConditionalSlopeT,
}

/// Effect-size input parameterization for [`PowerAnalysisRequest`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum PowerEffectSpec {
    /// Omitted effect size (valid when `solve_target == PowerSolveTarget::DetectableEffect`).
    #[default]
    Unspecified,
    /// Direct standardized effect size (`d`, `d_z`, `f`, `h`, `r`, `w`, or `f^2`).
    Standardized(f64),
    /// Group mean difference (`mu_1 - mu_2` or `mu - mu_0`) and common/group SD `sigma`.
    /// For [`PowerTestFamily::TwoSamplePairedT`], `within_pair_correlation` converts `sigma` via
    /// `sigma_d = sigma * sqrt(2 * (1 - r))` (or `sqrt(sd1^2 + sd2^2 - 2 * r * sd1 * sd2)`).
    MeanDiffAndSd {
        /// Expected difference of means (`mu_1 - mu_2` for two-sample or `mu - mu_0` for one-sample) in raw response units.
        mean_diff: f64,
        /// Common or first-group population standard deviation `sigma > 0` in raw response units.
        sd: f64,
        /// Optional distinct second-group standard deviation `sigma_2 > 0` triggering Welch–Satterthwaite *t* calculations when unequal to `sd`.
        sd_group2: Option<f64>,
        /// Optional within-pair Pearson correlation `r` in `(-1, 1)` used to derive `sigma_d` for paired *t* designs (`None` treats `sd` directly or defaults per family).
        within_pair_correlation: Option<f64>,
    },
    /// Direct paired-differences mean difference and SD of paired differences (`sigma_d`).
    PairedDiff {
        /// Expected mean of paired differences `mu_d` in raw response units.
        mean_diff: f64,
        /// Standard deviation of paired differences `sigma_d > 0` in raw response units.
        sd_diff: f64,
    },
    /// Raw group means and common within-group SD (for `OneWayAnova` or `TwoSampleUnpairedT`).
    GroupMeansAndSd {
        /// Expected group population means `[mu_1, ..., mu_k]` across `k >= 2` groups.
        means: Vec<f64>,
        /// Pooled within-group standard deviation `sigma > 0` in raw response units.
        within_sd: f64,
    },
    /// Group proportions `(p1, p2)` in `[0, 1]` for [`PowerTestFamily::TwoProportions`].
    TwoProportions {
        /// Group 1 baseline / control proportion `p_1` in `(0, 1)` (or `[0, 1]`).
        p1: f64,
        /// Group 2 treatment / comparison proportion `p_2` in `(0, 1)` (or `[0, 1]`).
        p2: f64,
    },
    /// Fixed baseline proportion `p1` when solving `DetectableEffect` for `p2` in [`PowerTestFamily::TwoProportions`].
    BaselineProportion {
        /// Known baseline proportion `p_1` in `(0, 1)` used as anchor when solving minimum detectable `p_2`.
        p1: f64,
    },
    /// Coefficient of determination `R^2` in `[0, 1)` for [`PowerTestFamily::SimpleLinearRegression`].
    RSquared(f64),
    /// Regression slope `beta1`, predictor SD `sigma_x`, and residual SD `sigma_eps`.
    RegressionSlope {
        /// Hypothesized alternative slope `beta_1` in `Y / X` units.
        beta1: f64,
        /// Predictor population standard deviation `sigma_X > 0`.
        sigma_x: f64,
        /// Residual error standard deviation `sigma_epsilon > 0`.
        sigma_eps: f64,
    },
}

/// Typed input specification for [`solve_power`], [`compute_power_curve`], and [`compute_sensitivity_table`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerAnalysisRequest {
    /// Hypothesis test family (`TwoSampleUnpairedT`, `TwoProportions`, `OneWayAnova`, etc.).
    pub family: PowerTestFamily,
    /// Target unknown solved by `solve_power` (`SampleSize`, `Power`, or `DetectableEffect`).
    pub solve_target: PowerSolveTarget,
    /// Two-sided vs one-sided directional alternative test tail.
    pub tail_mode: TailMode,
    /// Significance level `alpha` in `(0, 1)` (default `0.05`).
    pub alpha: f64,
    /// Target statistical power `1 - beta` in `(0, 1)` (default `Some(0.80)`).
    pub target_power: Option<f64>,
    /// Primary sample size (`n1` per group for 2-group/ANOVA; total `N` for single-sample/correlation/chi2/regression).
    pub sample_size: Option<f64>,
    /// Optional explicit second-group sample size `n2` for two-sample designs.
    pub sample_size_group2: Option<f64>,
    /// Allocation ratio `k = n2 / n1 > 0` for two-sample designs when `sample_size_group2` is `None`.
    pub allocation_ratio: f64,
    /// Effect-size parameterization.
    pub effect: PowerEffectSpec,
    /// Group count `k >= 2` for `OneWayAnova` (inferred from `GroupMeansAndSd` when `None`).
    pub num_groups: Option<usize>,
    /// Degrees of freedom `df > 0` for `ChiSquare` (default `1.0`).
    pub chi_square_df: f64,
    /// Variance / SE convention for `TwoProportions` (default `PooledNullUnpooledAlt`).
    pub two_proportions_mode: TwoProportionsMethod,
    /// Yates / Fleiss continuity correction for `TwoProportions` normal approximations: shrinks
    /// `|p1 - p2|` by `(1/n1 + 1/n2) / 2` as in Fleiss, Tytun & Ury (1980).
    pub continuity_correction: bool,
    /// Power-curve convention for `Correlation` (default `FisherZ`).
    pub correlation_method: CorrelationPowerMethod,
}

impl Default for PowerAnalysisRequest {
    fn default() -> Self {
        Self {
            family: PowerTestFamily::TwoSampleUnpairedT,
            solve_target: PowerSolveTarget::SampleSize,
            tail_mode: TailMode::TwoSided,
            alpha: 0.05,
            target_power: Some(0.80),
            sample_size: None,
            sample_size_group2: None,
            allocation_ratio: 1.0,
            effect: PowerEffectSpec::Unspecified,
            num_groups: None,
            chi_square_df: 1.0,
            two_proportions_mode: TwoProportionsMethod::PooledNullUnpooledAlt,
            continuity_correction: false,
            correlation_method: CorrelationPowerMethod::FisherZ,
        }
    }
}

impl PowerAnalysisRequest {
    /// Construct a default `SampleSize` request for `family` at `alpha = 0.05`, `power = 0.80`, two-sided.
    #[must_use]
    pub fn new(family: PowerTestFamily) -> Self {
        Self { family, ..Self::default() }
    }

    /// Construct an unpaired *t*-test sample-size request from Cohen's `d`.
    #[must_use]
    pub fn unpaired_t(d: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::TwoSampleUnpairedT,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::Standardized(d),
            ..Self::default()
        }
    }

    /// Construct a paired *t*-test sample-size request from standardized paired effect `d_z`.
    #[must_use]
    pub fn paired_t(dz: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::TwoSamplePairedT,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::Standardized(dz),
            ..Self::default()
        }
    }

    /// Construct a paired *t*-test sample-size request from mean difference, common SD `sigma`,
    /// and within-pair correlation `r` (`sigma_d = sigma * sqrt(2 * (1 - r))`).
    #[must_use]
    pub fn paired_t_correlated(mean_diff: f64, sd: f64, correlation_r: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::TwoSamplePairedT,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2: None, within_pair_correlation: Some(correlation_r) },
            ..Self::default()
        }
    }

    /// Construct a one-sample *t*-test sample-size request from Cohen's `d`.
    #[must_use]
    pub fn one_sample_t(d: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::OneSampleT,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::Standardized(d),
            ..Self::default()
        }
    }

    /// Construct a balanced one-way ANOVA sample-size request for `num_groups >= 2` and Cohen's `f`.
    #[must_use]
    pub fn one_way_anova(num_groups: usize, f: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::OneWayAnova,
            alpha,
            target_power: Some(target_power),
            num_groups: Some(num_groups),
            effect: PowerEffectSpec::Standardized(f),
            ..Self::default()
        }
    }

    /// Construct a two-proportions sample-size request for `(p1, p2)`.
    #[must_use]
    pub fn two_proportions(p1: f64, p2: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::TwoProportions,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::TwoProportions { p1, p2 },
            ..Self::default()
        }
    }

    /// Construct a Pearson correlation sample-size request against `H0: r = 0`.
    #[must_use]
    pub fn correlation(r: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::Correlation,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::Standardized(r),
            ..Self::default()
        }
    }

    /// Construct a chi-square goodness-of-fit / contingency sample-size request from Cohen's `w` and `df`.
    #[must_use]
    pub fn chi_square(w: f64, df: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::ChiSquare,
            alpha,
            target_power: Some(target_power),
            chi_square_df: df,
            effect: PowerEffectSpec::Standardized(w),
            ..Self::default()
        }
    }

    /// Construct a simple linear regression (`u = 1`) sample-size request from Cohen's `f^2`.
    #[must_use]
    pub fn simple_linear_regression(f2: f64, alpha: f64, target_power: f64) -> Self {
        Self {
            family: PowerTestFamily::SimpleLinearRegression,
            alpha,
            target_power: Some(target_power),
            effect: PowerEffectSpec::Standardized(f2),
            ..Self::default()
        }
    }

    /// Set the unknown quantity solved by [`solve_power`] (`SampleSize`, `Power`, or `DetectableEffect`).
    #[must_use]
    pub const fn with_target(mut self, target: PowerSolveTarget) -> Self {
        self.solve_target = target;
        self
    }

    /// Set the alternative hypothesis tail mode (`TwoSided` or directional `OneSided`).
    #[must_use]
    pub const fn with_tail(mut self, tail_mode: TailMode) -> Self {
        self.tail_mode = tail_mode;
        self
    }

    /// Set the Type I error significance threshold `alpha` in `(0, 1)`.
    #[must_use]
    pub const fn with_alpha(mut self, alpha: f64) -> Self {
        self.alpha = alpha;
        self
    }

    /// Set the target statistical power `1 - beta` in `(0, 1)`.
    #[must_use]
    pub const fn with_target_power(mut self, target_power: f64) -> Self {
        self.target_power = Some(target_power);
        self
    }

    /// Set the primary sample size (`n1` per group or total `N`, continuous or integer).
    #[must_use]
    pub const fn with_sample_size(mut self, n: f64) -> Self {
        self.sample_size = Some(n);
        self
    }

    /// Set an explicit second-group sample size `n2` for two-sample designs.
    #[must_use]
    pub const fn with_sample_size_group2(mut self, n2: f64) -> Self {
        self.sample_size_group2 = Some(n2);
        self
    }

    /// Set the allocation ratio `k = n2 / n1 > 0` used when `sample_size_group2` is `None`.
    #[must_use]
    pub const fn with_allocation_ratio(mut self, ratio: f64) -> Self {
        self.allocation_ratio = ratio;
        self
    }

    /// Set the effect-size parameterization (`Standardized`, `MeanDiffAndSd`, `TwoProportions`, etc.).
    #[must_use]
    pub fn with_effect(mut self, effect: PowerEffectSpec) -> Self {
        self.effect = effect;
        self
    }

    /// Set the group count `k >= 2` for `OneWayAnova` designs.
    #[must_use]
    pub const fn with_num_groups(mut self, k: usize) -> Self {
        self.num_groups = Some(k);
        self
    }

    /// Set the chi-square test degrees of freedom `df > 0` for `ChiSquare` designs.
    #[must_use]
    pub const fn with_chi_square_df(mut self, df: f64) -> Self {
        self.chi_square_df = df;
        self
    }

    /// Set the variance / SE approximation method used for `TwoProportions`.
    #[must_use]
    pub const fn with_two_proportions_mode(mut self, mode: TwoProportionsMethod) -> Self {
        self.two_proportions_mode = mode;
        self
    }

    /// Enable or disable Yates/Fleiss continuity correction for normal-approx `TwoProportions`.
    #[must_use]
    pub const fn with_continuity_correction(mut self, enabled: bool) -> Self {
        self.continuity_correction = enabled;
        self
    }

    /// Set the power calculation method for `Correlation` (`FisherZ`, `ExactTTransform`, etc.).
    #[must_use]
    pub const fn with_correlation_method(mut self, method: CorrelationPowerMethod) -> Self {
        self.correlation_method = method;
        self
    }
}

/// Single point on a power-vs-sample-size curve (`Stage 4` interactive chart).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PowerCurvePoint {
    /// Primary sample size coordinate (`n1` per group or total `N`).
    pub n: f64,
    /// Exact achieved statistical power `1 - beta` in `[0, 1]` at `(n, effect_size)`.
    pub power: f64,
    /// Standardized effect size on the test family's canonical scale (`d`, `d_z`, `f`, `h`, `r`, `w`, or `f^2`).
    pub effect_size: f64,
    /// Total sample size across all groups (`total_n >= n`).
    pub total_n: f64,
}

/// Single cell in the multi-alpha by multi-power sensitivity table (`Stage 4`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SensitivityCell {
    /// Nominal significance level `alpha` in `(0, 1)` evaluated for this grid cell.
    pub alpha: f64,
    /// Requested target power `1 - beta` in `(0, 1)` evaluated for this grid cell.
    pub target_power: f64,
    /// Ceiling integer primary sample size `n1` required to attain at least `target_power` (or input `n1` when solving `DetectableEffect`).
    pub sample_size: usize,
    /// Standardized effect size used or solved in this cell (`d`, `d_z`, `f`, `h`, `r`, `w`, or `f^2`).
    pub effect_size: f64,
    /// Total integer sample size across all groups.
    pub total_n: usize,
    /// Exact continuous primary sample size solving the power equation before integer ceiling rounding.
    pub continuous_n: f64,
}

/// Complete numerical output returned by [`solve_power`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerAnalysisResult {
    /// Hypothesis test family evaluated.
    pub family: PowerTestFamily,
    /// Quantity solved for (`SampleSize`, `Power`, or `DetectableEffect`).
    pub solve_target: PowerSolveTarget,
    /// Two-sided vs one-sided directional alternative test tail.
    pub tail_mode: TailMode,
    /// Significance level `alpha` in `(0, 1)` applied to critical thresholds.
    pub alpha: f64,
    /// Target statistical power `1 - beta` (`Some(...)` when solving `SampleSize` or `DetectableEffect`, optional when solving `Power`).
    pub target_power: Option<f64>,
    /// Exact continuous primary sample size (`n1` or `n_per_group` or total `N`).
    pub continuous_n: f64,
    /// Ceiling integer primary sample size (`n1` or `n_per_group` or total `N`).
    pub n_per_group: usize,
    /// Ceiling integer second-group sample size for two-sample tests (`None` otherwise).
    pub n_group2: Option<usize>,
    /// Total integer sample size across all groups.
    pub total_n: usize,
    /// Exact power evaluated at `continuous_n` (matches `target_power` when solving `SampleSize`).
    pub power_at_continuous_n: f64,
    /// Exact achieved power evaluated at the ceiling integer sample size(s) (`n_per_group`, `n_group2`).
    pub achieved_power: f64,
    /// Standardized effect size on the family's canonical scale (`d`, `d_z`, `f`, `h`, `r`, `w`, or `f^2`).
    pub effect_size: f64,
    /// Raw-unit effect size when scale information was supplied (`mean_diff`, `p1 - p2`, `beta1`, or `f * within_sd`).
    pub raw_effect_size: Option<f64>,
    /// Implied or given second proportion `p2` for `TwoProportions`.
    pub implied_p2: Option<f64>,
    /// Implied Cohen's `h` for `TwoProportions`.
    pub cohens_h: Option<f64>,
    /// Implied `R^2 = f^2 / (1 + f^2)` for `SimpleLinearRegression`.
    pub implied_r_squared: Option<f64>,
    /// Denominator / test degrees of freedom evaluated at ceiling `n` (`None` for normal-approx two proportions).
    pub actual_df: Option<f64>,
    /// Numerator degrees of freedom (`Some(k - 1)` for ANOVA, `Some(1.0)` for SimpleLinearRegression).
    pub numerator_df: Option<f64>,
    /// Noncentrality parameter (`delta` for *t*/*z*, `lambda` for *F*/chi-square) at ceiling `n`.
    pub noncentrality_param: f64,
    /// Critical test-statistic threshold at ceiling `n` and `alpha`.
    pub critical_value: f64,
    /// Sampled power-vs-`N` curve around the computed design.
    pub power_curve: Vec<PowerCurvePoint>,
    /// Multi-`alpha` by multi-power sensitivity matrix for Stage 4 exploration.
    pub sensitivity_table: Vec<SensitivityCell>,
    /// Human-readable convention notes for UI / `ResultSheet` summaries.
    pub notes: Vec<String>,
}

// ============================================================================
// Standalone Effect-Size Conversion Helpers
// ============================================================================

/// Compute Cohen's `d = (mean_1 - mean_2) / pooled_sd` (Cohen 1988, ch. 2).
pub fn cohens_d_from_means(mean_1: f64, mean_2: f64, pooled_sd: f64) -> Result<f64, StatsError> {
    if !mean_1.is_finite() || !mean_2.is_finite() {
        return Err(StatsError::InvalidParameter { name: "mean", value: mean_1 + mean_2 });
    }
    if !pooled_sd.is_finite() || pooled_sd <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "pooled_sd", value: pooled_sd });
    }
    Ok((mean_1 - mean_2) / pooled_sd)
}

/// Compute paired-difference standard deviation `sigma_d = sigma * sqrt(2 * (1 - r))`.
pub fn paired_sd_from_correlation(sd: f64, correlation_r: f64) -> Result<f64, StatsError> {
    if !sd.is_finite() || sd <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sd", value: sd });
    }
    if !correlation_r.is_finite() || !(-1.0..1.0).contains(&correlation_r) {
        return Err(StatsError::InvalidParameter { name: "within_pair_correlation", value: correlation_r });
    }
    Ok(sd * (2.0 * (1.0 - correlation_r)).sqrt())
}

/// Compute paired Cohen's `d_z = mean_diff / (sd * sqrt(2 * (1 - r)))` (Cohen 1988, ch. 2).
pub fn cohens_dz_from_correlation(mean_diff: f64, sd: f64, correlation_r: f64) -> Result<f64, StatsError> {
    if !mean_diff.is_finite() {
        return Err(StatsError::InvalidParameter { name: "mean_diff", value: mean_diff });
    }
    let sd_d = paired_sd_from_correlation(sd, correlation_r)?;
    Ok(mean_diff / sd_d)
}

/// Compute balanced one-way ANOVA Cohen's `f = sqrt(sum((mu_i - mu_bar)^2) / k) / within_sd`
/// (Cohen 1988, ch. 8: `sigma_m` uses divisor `k`).
pub fn cohens_f_from_means(group_means: &[f64], within_sd: f64) -> Result<f64, StatsError> {
    if group_means.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "one-way ANOVA means", required: 2, actual: group_means.len() });
    }
    if !within_sd.is_finite() || within_sd <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "within_sd", value: within_sd });
    }
    let moments = SummaryMoments::from_slice(group_means)?;
    let sigma_m = (moments.css / (group_means.len() as f64)).sqrt();
    Ok(sigma_m / within_sd)
}

/// Compute Cohen's `h = 2 * asin(sqrt(p1)) - 2 * asin(sqrt(p2))` (Cohen 1988, ch. 6).
pub fn cohens_h(p1: f64, p2: f64) -> Result<f64, StatsError> {
    if !p1.is_finite() || !(0.0..=1.0).contains(&p1) {
        return Err(StatsError::InvalidParameter { name: "p1", value: p1 });
    }
    if !p2.is_finite() || !(0.0..=1.0).contains(&p2) {
        return Err(StatsError::InvalidParameter { name: "p2", value: p2 });
    }
    Ok(2.0 * p1.sqrt().asin() - 2.0 * p2.sqrt().asin())
}

/// Compute Cohen's `f^2 = R^2 / (1 - R^2)` from coefficient of determination `R^2` in `[0, 1)`
/// (Cohen 1988, ch. 9).
pub fn cohens_f2_from_r_squared(r_squared: f64) -> Result<f64, StatsError> {
    if !r_squared.is_finite() || !(0.0..1.0).contains(&r_squared) {
        return Err(StatsError::InvalidParameter { name: "r_squared", value: r_squared });
    }
    Ok(r_squared / (1.0 - r_squared))
}

/// Compute Cohen's `f^2 = (beta1 * sigma_x / sigma_eps)^2` for simple linear regression
/// (Cohen 1988, ch. 9, with `R^2 / (1 - R^2) = beta1^2 sigma_x^2 / sigma_eps^2`).
pub fn cohens_f2_from_slope(beta1: f64, sigma_x: f64, sigma_eps: f64) -> Result<f64, StatsError> {
    if !beta1.is_finite() {
        return Err(StatsError::InvalidParameter { name: "beta1", value: beta1 });
    }
    if !sigma_x.is_finite() || sigma_x <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sigma_x", value: sigma_x });
    }
    if !sigma_eps.is_finite() || sigma_eps <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sigma_eps", value: sigma_eps });
    }
    let ratio = beta1 * sigma_x / sigma_eps;
    Ok(ratio * ratio)
}

// ============================================================================
// Internal Resolved Representation & Validation
// ============================================================================

#[derive(Debug, Clone)]
enum ResolvedEffect {
    UnpairedT {
        d: f64,
        raw_scale_sd: Option<f64>,
        /// `(mean_diff, sd1, sd2)` when heteroscedastic `sd_group2` is supplied (Welch–Satterthwaite *t*-test).
        welch_sds: Option<(f64, f64, f64)>,
    },
    SingleT {
        d: f64,
        raw_scale_sd: Option<f64>,
    },
    Anova {
        k: usize,
        f: f64,
        within_sd: Option<f64>,
    },
    TwoProportions {
        p1: Option<f64>,
        p2: Option<f64>,
        h: f64,
    },
    Correlation {
        r: f64,
    },
    ChiSquare {
        df: f64,
        w: f64,
    },
    LinearRegression {
        f2: f64,
        sign: f64,
        slope_scale: Option<f64>,
    },
}

#[derive(Debug, Clone)]
struct ResolvedContext {
    family: PowerTestFamily,
    solve_target: PowerSolveTarget,
    tail_mode: TailMode,
    alpha: f64,
    target_power: Option<f64>,
    allocation_ratio: f64,
    fixed_n2: Option<f64>,
    two_proportions_mode: TwoProportionsMethod,
    continuity_correction: bool,
    correlation_method: CorrelationPowerMethod,
    effect: ResolvedEffect,
}

#[derive(Debug, Clone, Copy)]
struct PowerEval {
    power: f64,
    actual_df: Option<f64>,
    numerator_df: Option<f64>,
    noncentrality_param: f64,
    critical_value: f64,
}

fn validate_prob_open(name: &'static str, v: f64) -> Result<f64, StatsError> {
    if !v.is_finite() || v <= 0.0 || v >= 1.0 {
        return Err(StatsError::InvalidParameter { name, value: v });
    }
    Ok(v)
}

fn validate_positive_finite(name: &'static str, v: f64) -> Result<f64, StatsError> {
    if !v.is_finite() || v <= 0.0 {
        return Err(StatsError::InvalidParameter { name, value: v });
    }
    Ok(v)
}

fn validate_tail_mode(family: PowerTestFamily, tail_mode: TailMode) -> Result<(), StatsError> {
    if !family.supports_one_sided() && matches!(tail_mode, TailMode::OneSided(_)) {
        return Err(StatsError::InvalidParameter { name: "tail_mode", value: 1.0 });
    }
    Ok(())
}

fn f64_to_checked_usize(name: &'static str, v: f64) -> Result<usize, StatsError> {
    let max_safe = (usize::MAX as f64).min(MAX_SAFE_INT_F64);
    if !v.is_finite() || v < 0.0 || v > max_safe {
        return Err(StatsError::InvalidParameter { name, value: v });
    }
    Ok(v as usize)
}

fn validate_allocation_ratio(v: f64) -> Result<f64, StatsError> {
    if !v.is_finite() || !(MIN_ALLOCATION_RATIO..=MAX_ALLOCATION_RATIO).contains(&v) {
        return Err(StatsError::InvalidParameter { name: "allocation_ratio", value: v });
    }
    Ok(v)
}

fn welch_satterthwaite_df(n1: f64, n2: f64, sd1: f64, sd2: f64) -> Result<f64, StatsError> {
    if !n1.is_finite() || !n2.is_finite() || n1 <= 1.0 || n2 <= 1.0 {
        return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
    }
    let v1 = (sd1 * sd1) / n1;
    let v2 = (sd2 * sd2) / n2;
    let v_sum = v1 + v2;
    let denom = (v1 * v1) / (n1 - 1.0) + (v2 * v2) / (n2 - 1.0);
    if !v_sum.is_finite() || !denom.is_finite() || denom <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "df", value: f64::NAN });
    }
    Ok((v_sum * v_sum) / denom)
}

/// Resolve the unpaired-*t* effect to Cohen's `d` (or carry `(mean_diff, sd1, sd2)` for Welch's *t*-test
/// when `sd_group2` is supplied).
fn resolve_unpaired_t_effect(req: &PowerAnalysisRequest, k: f64) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => Ok(ResolvedEffect::UnpairedT { d: 0.0, raw_scale_sd: None, welch_sds: None }),
        PowerEffectSpec::Standardized(d) => {
            if !d.is_finite() {
                return Err(StatsError::InvalidParameter { name: "cohens_d", value: *d });
            }
            Ok(ResolvedEffect::UnpairedT { d: *d, raw_scale_sd: None, welch_sds: None })
        }
        PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation: None } => {
            if !mean_diff.is_finite() {
                return Err(StatsError::InvalidParameter { name: "mean_diff", value: *mean_diff });
            }
            let sd1 = validate_positive_finite("sd", *sd)?;
            if let Some(sd2_raw) = sd_group2 {
                let sd2 = validate_positive_finite("sd_group2", *sd2_raw)?;
                let eff_sd = ((k * sd1 * sd1 + sd2 * sd2) / (k + 1.0)).sqrt();
                Ok(ResolvedEffect::UnpairedT { d: mean_diff / eff_sd, raw_scale_sd: Some(eff_sd), welch_sds: Some((*mean_diff, sd1, sd2)) })
            } else {
                Ok(ResolvedEffect::UnpairedT { d: mean_diff / sd1, raw_scale_sd: Some(sd1), welch_sds: None })
            }
        }
        PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
            if means.len() != 2 {
                return Err(StatsError::DimensionMismatch { expected: 2, actual: means.len() });
            }
            let sd = validate_positive_finite("within_sd", *within_sd)?;
            let d = cohens_d_from_means(means[0], means[1], sd)?;
            Ok(ResolvedEffect::UnpairedT { d, raw_scale_sd: Some(sd), welch_sds: None })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "TwoSampleUnpairedT effect specification" }),
    }
}

fn resolve_paired_t_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => Ok(ResolvedEffect::SingleT { d: 0.0, raw_scale_sd: None }),
        PowerEffectSpec::Standardized(dz) => {
            if !dz.is_finite() {
                return Err(StatsError::InvalidParameter { name: "cohens_dz", value: *dz });
            }
            Ok(ResolvedEffect::SingleT { d: *dz, raw_scale_sd: None })
        }
        PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => {
            if !mean_diff.is_finite() {
                return Err(StatsError::InvalidParameter { name: "mean_diff", value: *mean_diff });
            }
            let sdd = validate_positive_finite("sd_diff", *sd_diff)?;
            Ok(ResolvedEffect::SingleT { d: mean_diff / sdd, raw_scale_sd: Some(sdd) })
        }
        PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => {
            if !mean_diff.is_finite() {
                return Err(StatsError::InvalidParameter { name: "mean_diff", value: *mean_diff });
            }
            let sd1 = validate_positive_finite("sd", *sd)?;
            let r = within_pair_correlation.unwrap_or(0.0);
            if !r.is_finite() || !(-1.0..=1.0).contains(&r) {
                return Err(StatsError::InvalidParameter { name: "within_pair_correlation", value: r });
            }
            let sdd = if let Some(sd2_raw) = sd_group2 {
                let sd2 = validate_positive_finite("sd_group2", *sd2_raw)?;
                (sd1 * sd1 + sd2 * sd2 - 2.0 * r * sd1 * sd2).max(0.0).sqrt()
            } else {
                sd1 * (2.0 * (1.0 - r)).max(0.0).sqrt()
            };
            if !sdd.is_finite() || sdd <= 0.0 {
                return Err(StatsError::ZeroVariance { context: "paired difference SD" });
            }
            Ok(ResolvedEffect::SingleT { d: mean_diff / sdd, raw_scale_sd: Some(sdd) })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "TwoSamplePairedT effect specification" }),
    }
}

fn resolve_one_sample_t_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => Ok(ResolvedEffect::SingleT { d: 0.0, raw_scale_sd: None }),
        PowerEffectSpec::Standardized(d) => {
            if !d.is_finite() {
                return Err(StatsError::InvalidParameter { name: "cohens_d", value: *d });
            }
            Ok(ResolvedEffect::SingleT { d: *d, raw_scale_sd: None })
        }
        PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2: None, within_pair_correlation: None } => {
            if !mean_diff.is_finite() {
                return Err(StatsError::InvalidParameter { name: "mean_diff", value: *mean_diff });
            }
            let s = validate_positive_finite("sd", *sd)?;
            Ok(ResolvedEffect::SingleT { d: mean_diff / s, raw_scale_sd: Some(s) })
        }
        PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
            let s = validate_positive_finite("within_sd", *within_sd)?;
            let diff = match means.as_slice() {
                [mu] => *mu,
                [mu, mu0] => *mu - *mu0,
                other => return Err(StatsError::DimensionMismatch { expected: 2, actual: other.len() }),
            };
            if !diff.is_finite() {
                return Err(StatsError::InvalidParameter { name: "mean", value: diff });
            }
            Ok(ResolvedEffect::SingleT { d: diff / s, raw_scale_sd: Some(s) })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "OneSampleT effect specification" }),
    }
}

fn resolve_anova_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => {
            let k = req.num_groups.unwrap_or(0);
            if !(2..=MAX_ANOVA_GROUPS).contains(&k) {
                return Err(StatsError::InvalidParameter { name: "num_groups", value: k as f64 });
            }
            Ok(ResolvedEffect::Anova { k, f: 0.0, within_sd: None })
        }
        PowerEffectSpec::Standardized(f) => {
            let k = req.num_groups.unwrap_or(0);
            if !(2..=MAX_ANOVA_GROUPS).contains(&k) {
                return Err(StatsError::InvalidParameter { name: "num_groups", value: k as f64 });
            }
            if !f.is_finite() || *f < 0.0 {
                return Err(StatsError::InvalidParameter { name: "cohens_f", value: *f });
            }
            Ok(ResolvedEffect::Anova { k, f: *f, within_sd: None })
        }
        PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
            if means.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "one-way ANOVA groups", required: 2, actual: means.len() });
            }
            if means.len() > MAX_ANOVA_GROUPS {
                return Err(StatsError::InvalidParameter { name: "num_groups", value: means.len() as f64 });
            }
            if let Some(k) = req.num_groups
                && k != means.len()
            {
                return Err(StatsError::DimensionMismatch { expected: k, actual: means.len() });
            }
            let sd = validate_positive_finite("within_sd", *within_sd)?;
            let f = cohens_f_from_means(means, sd)?;
            Ok(ResolvedEffect::Anova { k: means.len(), f, within_sd: Some(sd) })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "OneWayAnova effect specification" }),
    }
}

fn resolve_two_proportions_effect(req: &PowerAnalysisRequest, k: f64) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => {
            if req.two_proportions_mode != TwoProportionsMethod::CohensH {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "TwoProportions DetectableEffect requires BaselineProportion { p1 } or CohensH mode",
                });
            }
            Ok(ResolvedEffect::TwoProportions { p1: None, p2: None, h: 0.0 })
        }
        PowerEffectSpec::Standardized(h) => {
            // Cohen's h = 2 asin(sqrt(p1)) - 2 asin(sqrt(p2)) is strictly bounded by |h| <= pi (Cohen 1988, ch. 6).
            if !h.is_finite() || h.abs() > std::f64::consts::PI {
                return Err(StatsError::InvalidParameter { name: "cohens_h", value: *h });
            }
            Ok(ResolvedEffect::TwoProportions { p1: None, p2: None, h: *h })
        }
        PowerEffectSpec::BaselineProportion { p1 } => {
            if !allow_unspecified {
                return Err(StatsError::IncompatibleAnalysisSpec { expected: "BaselineProportion is valid only for DetectableEffect" });
            }
            if !p1.is_finite() || *p1 <= 0.0 || *p1 >= 1.0 {
                return Err(StatsError::InvalidParameter { name: "p1", value: *p1 });
            }
            Ok(ResolvedEffect::TwoProportions { p1: Some(*p1), p2: None, h: 0.0 })
        }
        PowerEffectSpec::TwoProportions { p1, p2 } => {
            let h = cohens_h(*p1, *p2)?;
            if req.two_proportions_mode != TwoProportionsMethod::CohensH && !allow_unspecified {
                let p_bar = (*p1 + k * *p2) / (1.0 + k);
                let s0 = p_bar * (1.0 - p_bar);
                let s1 = *p1 * (1.0 - *p1) + (*p2 * (1.0 - *p2)) / k;
                if (req.two_proportions_mode == TwoProportionsMethod::PooledNullUnpooledAlt && s0 <= 0.0) || s1 <= 0.0 {
                    return Err(StatsError::ZeroVariance { context: "two-proportions normal approximation" });
                }
            }
            Ok(ResolvedEffect::TwoProportions { p1: Some(*p1), p2: Some(*p2), h })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "TwoProportions effect specification" }),
    }
}

fn resolve_correlation_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => Ok(ResolvedEffect::Correlation { r: 0.0 }),
        PowerEffectSpec::Standardized(r) => {
            if !r.is_finite() || r.abs() >= 1.0 {
                return Err(StatsError::InvalidParameter { name: "correlation_r", value: *r });
            }
            Ok(ResolvedEffect::Correlation { r: *r })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "Correlation effect specification (Standardized(r))" }),
    }
}

fn resolve_chi_square_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let df = validate_positive_finite("chi_square_df", req.chi_square_df)?;
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => Ok(ResolvedEffect::ChiSquare { df, w: 0.0 }),
        PowerEffectSpec::Standardized(w) => {
            if !w.is_finite() || *w < 0.0 {
                return Err(StatsError::InvalidParameter { name: "cohens_w", value: *w });
            }
            Ok(ResolvedEffect::ChiSquare { df, w: *w })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "ChiSquare effect specification (Standardized(w))" }),
    }
}

fn resolve_regression_effect(req: &PowerAnalysisRequest) -> Result<ResolvedEffect, StatsError> {
    let allow_unspecified = req.solve_target == PowerSolveTarget::DetectableEffect;
    match &req.effect {
        PowerEffectSpec::Unspecified if allow_unspecified => {
            let sign = if matches!(req.tail_mode, TailMode::OneSided(OneSidedDirection::Less)) { -1.0 } else { 1.0 };
            Ok(ResolvedEffect::LinearRegression { f2: 0.0, sign, slope_scale: None })
        }
        PowerEffectSpec::Standardized(f2) => {
            if !f2.is_finite() || *f2 < 0.0 {
                return Err(StatsError::InvalidParameter { name: "cohens_f2", value: *f2 });
            }
            if matches!(req.tail_mode, TailMode::OneSided(OneSidedDirection::Less)) && !allow_unspecified {
                return Err(StatsError::InvalidParameter { name: "tail_mode", value: -1.0 });
            }
            let sign = if matches!(req.tail_mode, TailMode::OneSided(OneSidedDirection::Less)) { -1.0 } else { 1.0 };
            Ok(ResolvedEffect::LinearRegression { f2: *f2, sign, slope_scale: None })
        }
        PowerEffectSpec::RSquared(r2) => {
            let f2 = cohens_f2_from_r_squared(*r2)?;
            if matches!(req.tail_mode, TailMode::OneSided(OneSidedDirection::Less)) && !allow_unspecified {
                return Err(StatsError::InvalidParameter { name: "tail_mode", value: -1.0 });
            }
            let sign = if matches!(req.tail_mode, TailMode::OneSided(OneSidedDirection::Less)) { -1.0 } else { 1.0 };
            Ok(ResolvedEffect::LinearRegression { f2, sign, slope_scale: None })
        }
        PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => {
            let f2 = cohens_f2_from_slope(*beta1, *sigma_x, *sigma_eps)?;
            let sign = if *beta1 < 0.0 { -1.0 } else { 1.0 };
            let slope_scale = Some(*sigma_eps / *sigma_x);
            Ok(ResolvedEffect::LinearRegression { f2, sign, slope_scale })
        }
        _ => Err(StatsError::IncompatibleAnalysisSpec { expected: "SimpleLinearRegression effect specification" }),
    }
}

fn build_context(req: &PowerAnalysisRequest) -> Result<ResolvedContext, StatsError> {
    let alpha = validate_prob_open("alpha", req.alpha)?;
    validate_tail_mode(req.family, req.tail_mode)?;
    let mut allocation_ratio = validate_allocation_ratio(req.allocation_ratio)?;
    let mut fixed_n2 = None;
    if req.sample_size_group2.is_some() && !matches!(req.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions) {
        return Err(StatsError::IncompatibleAnalysisSpec { expected: "sample_size_group2 is only valid for two-sample designs" });
    }
    if matches!(req.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions) {
        let is_welch = matches!(&req.effect, PowerEffectSpec::MeanDiffAndSd { sd_group2: Some(_), .. });
        let min_n2 = if is_welch { 1.0 + 1e-6 } else { 1.0 };
        match (req.sample_size, req.sample_size_group2) {
            (Some(n1_raw), Some(n2_raw)) => {
                if !n1_raw.is_finite() || n1_raw < 1.0 || n1_raw > MAX_CONTINUOUS_N {
                    return Err(StatsError::InvalidParameter { name: "sample_size", value: n1_raw });
                }
                if !n2_raw.is_finite() || n2_raw < min_n2 || n2_raw > MAX_CONTINUOUS_N {
                    return Err(StatsError::InvalidParameter { name: "sample_size_group2", value: n2_raw });
                }
                allocation_ratio = validate_allocation_ratio(n2_raw / n1_raw)?;
            }
            (None, Some(n2_raw)) => {
                if !n2_raw.is_finite() || n2_raw < min_n2 || n2_raw > MAX_CONTINUOUS_N {
                    return Err(StatsError::InvalidParameter { name: "sample_size_group2", value: n2_raw });
                }
                fixed_n2 = Some(n2_raw);
            }
            _ => {}
        }
    }

    let target_power = match req.solve_target {
        PowerSolveTarget::SampleSize | PowerSolveTarget::DetectableEffect => {
            let tp = req.target_power.ok_or(StatsError::InvalidParameter { name: "target_power", value: f64::NAN })?;
            Some(validate_prob_open("target_power", tp)?)
        }
        PowerSolveTarget::Power => req.target_power.map(|tp| validate_prob_open("target_power", tp)).transpose()?,
    };

    let effect = match req.family {
        PowerTestFamily::TwoSampleUnpairedT => resolve_unpaired_t_effect(req, allocation_ratio)?,
        PowerTestFamily::TwoSamplePairedT => resolve_paired_t_effect(req)?,
        PowerTestFamily::OneSampleT => resolve_one_sample_t_effect(req)?,
        PowerTestFamily::OneWayAnova => resolve_anova_effect(req)?,
        PowerTestFamily::TwoProportions => resolve_two_proportions_effect(req, allocation_ratio)?,
        PowerTestFamily::Correlation => resolve_correlation_effect(req)?,
        PowerTestFamily::ChiSquare => resolve_chi_square_effect(req)?,
        PowerTestFamily::SimpleLinearRegression => resolve_regression_effect(req)?,
    };

    Ok(ResolvedContext {
        family: req.family,
        solve_target: req.solve_target,
        tail_mode: req.tail_mode,
        alpha,
        target_power,
        allocation_ratio,
        fixed_n2,
        two_proportions_mode: req.two_proportions_mode,
        continuity_correction: req.continuity_correction,
        correlation_method: req.correlation_method,
        effect,
    })
}

// ============================================================================
// Core Distribution & Power Evaluators
// ============================================================================

/// Power of a Student *t* test from the noncentral *t* distribution (Owen 1965): two-sided
/// `P(T' > t_c) + P(T' < -t_c)`, one-sided the corresponding single tail, with `T' ~ t'(df, delta)`.
/// Callers supply `delta = d * sqrt(n1 n2 / (n1 + n2))` (unpaired), `d * sqrt(n)` (paired /
/// one-sample) or `sqrt(n f^2)` (regression slope); see Cohen (1988, chs. 2 and 9).
fn t_power(df: f64, delta: f64, alpha: f64, tail_mode: TailMode, numerator_df: Option<f64>) -> Result<PowerEval, StatsError> {
    if !df.is_finite() || df <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "df", value: df });
    }
    let (t_crit, power) = match tail_mode {
        TailMode::TwoSided => {
            let tc = student_t_two_sided_isf(alpha, df)?;
            let p = (nct_sf(tc, df, delta)? + nct_cdf(-tc, df, delta)?).clamp(0.0, 1.0);
            (tc, p)
        }
        TailMode::OneSided(OneSidedDirection::Greater) => {
            let tc = student_t_isf(alpha, df)?;
            let p = nct_sf(tc, df, delta)?.clamp(0.0, 1.0);
            (tc, p)
        }
        TailMode::OneSided(OneSidedDirection::Less) => {
            let tc = student_t_isf(alpha, df)?;
            let p = nct_cdf(-tc, df, delta)?.clamp(0.0, 1.0);
            (-tc, p)
        }
    };
    Ok(PowerEval { power, actual_df: Some(df), numerator_df, noncentrality_param: delta, critical_value: t_crit })
}

fn z_critical_value(alpha: f64, tail_mode: TailMode) -> Result<f64, StatsError> {
    Ok(match tail_mode {
        TailMode::TwoSided => norm_two_sided_isf(alpha)?,
        TailMode::OneSided(_) => norm_isf(alpha)?,
    })
}

/// Normal-approximation power for Cohen's `h` (Cohen 1988, ch. 6): `delta = h * sqrt(n1 n2 / (n1 + n2))`.
fn two_proportions_h_power(n1: f64, n2: f64, h: f64, alpha: f64, tail_mode: TailMode) -> Result<PowerEval, StatsError> {
    let n_eff = ((n1 * n2) / (n1 + n2)).sqrt();
    let delta = h * n_eff;
    let z_crit = z_critical_value(alpha, tail_mode)?;
    let (crit_out, power) = match tail_mode {
        TailMode::TwoSided => (z_crit, (norm_sf(z_crit - delta) + norm_cdf(-z_crit - delta)).clamp(0.0, 1.0)),
        TailMode::OneSided(OneSidedDirection::Greater) => (z_crit, norm_sf(z_crit - delta).clamp(0.0, 1.0)),
        TailMode::OneSided(OneSidedDirection::Less) => (-z_crit, norm_cdf(-z_crit - delta).clamp(0.0, 1.0)),
    };
    Ok(PowerEval { power, actual_df: None, numerator_df: None, noncentrality_param: delta, critical_value: crit_out })
}

/// Normal-approximation power for `D = p1 - p2`: upper tail `P(Z > (z_a * SE0 + c - D) / SE1)`, lower
/// tail the same with `+D` (two-sided sums both). `SE0` is pooled (Fleiss, Levin & Paik 2003) or
/// unpooled `SE0 = SE1` (Chow, Shao & Wang 2008); the optional continuity correction is
/// `c = (1/n1 + 1/n2) / 2` (Fleiss, Tytun & Ury 1980).
fn two_proportions_p_power(ctx: &ResolvedContext, n1: f64, n2: f64, p1: f64, p2: f64) -> Result<PowerEval, StatsError> {
    if ctx.two_proportions_mode == TwoProportionsMethod::CohensH {
        let h = cohens_h(p1, p2)?;
        return two_proportions_h_power(n1, n2, h, ctx.alpha, ctx.tail_mode);
    }

    let delta = p1 - p2;
    let p_bar = (n1 * p1 + n2 * p2) / (n1 + n2);
    let inv_n = 1.0 / n1 + 1.0 / n2;
    let se1 = (p1 * (1.0 - p1) / n1 + p2 * (1.0 - p2) / n2).sqrt();
    let se0 = match ctx.two_proportions_mode {
        TwoProportionsMethod::PooledNullUnpooledAlt => (p_bar * (1.0 - p_bar) * inv_n).sqrt(),
        TwoProportionsMethod::Unpooled => se1,
        TwoProportionsMethod::CohensH => se1,
    };
    if !se0.is_finite() || se0 <= 0.0 || !se1.is_finite() || se1 <= 0.0 {
        return Err(StatsError::ZeroVariance { context: "two-proportions standard error" });
    }

    let c = if ctx.continuity_correction { 0.5 * inv_n } else { 0.0 };
    let z_crit = z_critical_value(ctx.alpha, ctx.tail_mode)?;
    let (crit_out, power) = match ctx.tail_mode {
        TailMode::TwoSided => {
            let upper = norm_sf((z_crit * se0 + c - delta) / se1);
            let lower = norm_sf((z_crit * se0 + c + delta) / se1);
            (z_crit, (upper + lower).clamp(0.0, 1.0))
        }
        TailMode::OneSided(OneSidedDirection::Greater) => (z_crit, norm_sf((z_crit * se0 + c - delta) / se1).clamp(0.0, 1.0)),
        TailMode::OneSided(OneSidedDirection::Less) => (-z_crit, norm_sf((z_crit * se0 + c + delta) / se1).clamp(0.0, 1.0)),
    };
    Ok(PowerEval { power, actual_df: None, numerator_df: None, noncentrality_param: delta / se0, critical_value: crit_out })
}

fn hyp2f1_half_half(c: f64, w: f64) -> Result<f64, StatsError> {
    if w <= 0.85 || c >= 16.0 {
        let mut sum = 1.0;
        let mut term = 1.0;
        for k in 0..2500 {
            let kf = k as f64;
            term *= ((0.5 + kf) * (0.5 + kf)) / ((c + kf) * (kf + 1.0)) * w;
            sum += term;
            if term.abs() <= 1e-16 * sum.abs() {
                return Ok(sum);
            }
        }
        return Ok(sum);
    }
    // Euler integral representation with t = u^2 and u = 1 - s^2 handles c < 16 as w -> 1 cleanly.
    let log_norm = 4.0_f64.ln() - (ln_gamma(0.5)? + ln_gamma(c - 0.5)? - ln_gamma(c)?);
    let norm = log_norm.exp();
    let one_minus_w = (1.0 - w).max(1e-15);
    let s_split = one_minus_w.powf(0.25).clamp(1e-5, 0.5);
    let integrand = |s: f64| -> f64 {
        if s <= 0.0 {
            return 0.0;
        }
        let s2 = s * s;
        let one_minus_u2 = s2 * (2.0 - s2);
        let denom = ((1.0 - w) + w * one_minus_u2).max(1e-300).sqrt();
        s * ((c - 1.5) * one_minus_u2.ln()).exp() / denom
    };
    let i1 = gauss_legendre_32(integrand, 0.0, s_split);
    let i2 = gauss_legendre_32(integrand, s_split, 1.0);
    Ok(norm * (i1 + i2))
}

fn hotelling_ln_prefactor(n: f64) -> Result<f64, StatsError> {
    let x = n - 1.0;
    let half_ln_2pi = 0.5 * (2.0 * std::f64::consts::PI).ln();
    if x < 100.0 {
        return Ok((n - 2.0).ln() + ln_gamma(x)? - ln_gamma(x + 0.5)? - half_ln_2pi);
    }
    // Tricomi & Erdelyi (1951) asymptotic difference for ln(Gamma(x) / Gamma(x + 1/2)).
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    Ok(0.5 * x.ln() + (-inv).ln_1p() + 0.125 * inv - (1.0 / 192.0) * (inv * inv2) + (1.0 / 640.0) * (inv * inv2 * inv2) - half_ln_2pi)
}

fn hotelling_ln_sech(delta: f64) -> f64 {
    let ad = delta.abs();
    if ad <= 1.0 {
        let s = (0.5 * ad).sinh();
        -(2.0 * s * s).ln_1p()
    } else {
        -ad - (-2.0 * ad).exp().ln_1p() + std::f64::consts::LN_2
    }
}

fn hotelling_pdf_z(z: f64, n: f64, rho: f64, ln_c: f64, zeta: f64) -> Result<f64, StatsError> {
    let delta = z - zeta;
    let ln_s = hotelling_ln_sech(delta);
    let prod = rho * delta.tanh();
    let one_plus_rt = if prod < -0.5 {
        let e2 = (-2.0 * delta.abs()).exp();
        ((1.0 - rho.abs()) + (1.0 + rho.abs()) * e2) / (1.0 + e2)
    } else {
        1.0 + prod
    };
    let exponent = ln_c + (n - 2.0) * ln_s + 0.5 * one_plus_rt.max(1e-300).ln();
    if exponent < -745.0 {
        return Ok(0.0);
    }
    let r = z.tanh();
    let w = if rho * r > 0.5 {
        let e2z = (-2.0 * z.abs()).exp();
        let one_m_rr = ((1.0 - rho.abs()) + (1.0 + rho.abs()) * e2z) / (1.0 + e2z);
        1.0 - 0.5 * one_m_rr
    } else {
        0.5 * (1.0 + rho * r)
    };
    Ok(exponent.exp() * hyp2f1_half_half(n - 0.5, w.min(1.0 - 1e-16))?)
}

fn integrate_hotelling_z(n: f64, rho: f64, a: f64, b: f64, mu: f64, sig: f64) -> Result<f64, StatsError> {
    if !(a.is_finite() && b.is_finite()) || b <= a {
        return Ok(0.0);
    }
    let ln_c = hotelling_ln_prefactor(n)?;
    let zeta = rho.atanh();
    let mut pts = Vec::with_capacity(28);
    pts.push(a);
    for &k in &[-160.0, -96.0, -48.0, -24.0, -14.0, -7.0, -3.5, -1.5, -0.5, 0.5, 1.5, 3.5, 7.0, 14.0, 24.0, 48.0, 96.0, 160.0] {
        let p = mu + k * sig;
        if p > a && p < b {
            pts.push(p);
        }
    }
    pts.push(b);
    let mut err: Option<StatsError> = None;
    let mut total = 0.0;
    for win in pts.windows(2) {
        let (u, v) = (win[0], win[1]);
        if v - u <= 1e-15 {
            continue;
        }
        total += gauss_legendre_32(
            |z| {
                if err.is_some() {
                    return 0.0;
                }
                match hotelling_pdf_z(z, n, rho, ln_c, zeta) {
                    Ok(val) => val,
                    Err(e) => {
                        err = Some(e);
                        0.0
                    }
                }
            },
            u,
            v,
        );
        if let Some(e) = err {
            return Err(e);
        }
    }
    Ok(total)
}

fn hotelling_correlation_power(n: f64, r: f64, alpha: f64, tail_mode: TailMode) -> Result<PowerEval, StatsError> {
    if !n.is_finite() || n <= 2.0 {
        return Err(StatsError::InvalidParameter { name: "sample_size", value: n });
    }
    let df = n - 2.0;
    if r == 0.0 {
        return t_power(df, 0.0, alpha, tail_mode, None);
    }
    let t_crit = match tail_mode {
        TailMode::TwoSided => student_t_two_sided_isf(alpha, df)?,
        TailMode::OneSided(_) => student_t_isf(alpha, df)?,
    };
    let r_crit = t_crit / t_crit.hypot(df.sqrt());
    let z_crit = r_crit.atanh();
    let sig = 1.0 / df.sqrt();
    let span = (14.0 * sig).max(52.0 / df);

    let (crit_out, power) = match tail_mode {
        TailMode::TwoSided => {
            let rho = r.abs();
            let mu = rho.atanh() + rho / (2.0 * (n - 1.0));
            let (z_lo, z_hi) = (mu - span, mu + span);
            let p_mid = integrate_hotelling_z(n, rho, (-z_crit).max(z_lo), z_crit.min(z_hi), mu, sig)?;
            let p = if p_mid <= 0.5 {
                1.0 - p_mid
            } else {
                let p_left = integrate_hotelling_z(n, rho, z_lo, (-z_crit).min(z_hi), mu, sig)?;
                let p_right = integrate_hotelling_z(n, rho, z_crit.max(z_lo), z_hi, mu, sig)?;
                p_left + p_right
            };
            (t_crit, p.clamp(0.0, 1.0))
        }
        TailMode::OneSided(dir) => {
            let (rho, crit) = match dir {
                OneSidedDirection::Greater => (r, t_crit),
                OneSidedDirection::Less => (-r, -t_crit),
            };
            let mu = rho.atanh() + rho / (2.0 * (n - 1.0));
            let (z_lo, z_hi) = (mu - span, mu + span);
            let p = if z_crit >= mu {
                integrate_hotelling_z(n, rho, z_crit.max(z_lo), z_hi, mu, sig)?
            } else {
                1.0 - integrate_hotelling_z(n, rho, z_lo, z_crit.min(z_hi), mu, sig)?
            };
            (crit, p.clamp(0.0, 1.0))
        }
    };
    let ncp_eff = (r / (1.0 - r * r).sqrt()) * (n - 1.0).sqrt();
    Ok(PowerEval { power, actual_df: Some(df), numerator_df: None, noncentrality_param: ncp_eff, critical_value: crit_out })
}

/// Power for `H0: rho = 0` via Fisher's (1921) *z* (plain or with the `pwr.r.test` *t*-critical
/// hybrid), the exact bivariate-normal distribution of *r* (`ExactTTransform`; Fisher 1915;
/// Hotelling 1953), or the fixed-*x* conditional slope *t*-test (`ConditionalSlopeT`); see
/// [`CorrelationPowerMethod`].
fn correlation_power(method: CorrelationPowerMethod, n: f64, r: f64, alpha: f64, tail_mode: TailMode) -> Result<PowerEval, StatsError> {
    let df = n - 2.0;
    match method {
        CorrelationPowerMethod::FisherZ => {
            if !n.is_finite() || n <= 3.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n });
            }
            let sqrt_n3 = (n - 3.0).sqrt();
            let delta_z = r.atanh() * sqrt_n3;
            let z_crit = z_critical_value(alpha, tail_mode)?;
            let (crit_out, power) = match tail_mode {
                TailMode::TwoSided => (z_crit, (norm_sf(z_crit - delta_z) + norm_cdf(-z_crit - delta_z)).clamp(0.0, 1.0)),
                TailMode::OneSided(OneSidedDirection::Greater) => (z_crit, norm_sf(z_crit - delta_z).clamp(0.0, 1.0)),
                TailMode::OneSided(OneSidedDirection::Less) => (-z_crit, norm_cdf(-z_crit - delta_z).clamp(0.0, 1.0)),
            };
            Ok(PowerEval { power, actual_df: Some(df), numerator_df: None, noncentrality_param: delta_z, critical_value: crit_out })
        }
        CorrelationPowerMethod::FisherZWithTCrit => {
            if !n.is_finite() || n <= 3.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n });
            }
            let t_crit = match tail_mode {
                TailMode::TwoSided => student_t_two_sided_isf(alpha, df)?,
                TailMode::OneSided(_) => student_t_isf(alpha, df)?,
            };
            let rc = (t_crit * t_crit / (t_crit * t_crit + df)).sqrt();
            let zrc = rc.atanh();
            let zr = r.atanh() + r / (2.0 * (n - 1.0));
            let sqrt_n3 = (n - 3.0).sqrt();
            let (crit_out, power) = match tail_mode {
                TailMode::TwoSided => {
                    let p = norm_sf((zrc - zr) * sqrt_n3) + norm_cdf((-zrc - zr) * sqrt_n3);
                    (t_crit, p.clamp(0.0, 1.0))
                }
                TailMode::OneSided(OneSidedDirection::Greater) => (t_crit, norm_sf((zrc - zr) * sqrt_n3).clamp(0.0, 1.0)),
                TailMode::OneSided(OneSidedDirection::Less) => (-t_crit, norm_cdf((-zrc - zr) * sqrt_n3).clamp(0.0, 1.0)),
            };
            Ok(PowerEval { power, actual_df: Some(df), numerator_df: None, noncentrality_param: zr * sqrt_n3, critical_value: crit_out })
        }
        CorrelationPowerMethod::ExactTTransform => hotelling_correlation_power(n, r, alpha, tail_mode),
        CorrelationPowerMethod::ConditionalSlopeT => {
            if !n.is_finite() || n <= 2.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n });
            }
            let delta = (r / (1.0 - r * r).sqrt()) * n.sqrt();
            t_power(df, delta, alpha, tail_mode, None)
        }
    }
}

/// Evaluate power at a given design. Besides [`t_power`] and the proportion / correlation helpers,
/// one-way ANOVA uses noncentral *F* with `lambda = k n f^2` on `(k - 1, k (n - 1))` df and the
/// chi-square family uses noncentral chi-square with `lambda = N w^2` (Cohen 1988, chs. 7 and 8).
fn eval_power_at(ctx: &ResolvedContext, n1: f64, n2_explicit: Option<f64>) -> Result<PowerEval, StatsError> {
    let eff_n2 = n2_explicit.or(ctx.fixed_n2);
    match &ctx.effect {
        ResolvedEffect::UnpairedT { d, welch_sds, .. } => {
            let n2 = eff_n2.unwrap_or(ctx.allocation_ratio * n1);
            if let Some((mean_diff, sd1, sd2)) = welch_sds {
                let df = welch_satterthwaite_df(n1, n2, *sd1, *sd2)?;
                let se = ((sd1 * sd1) / n1 + (sd2 * sd2) / n2).sqrt();
                let delta = *mean_diff / se;
                t_power(df, delta, ctx.alpha, ctx.tail_mode, None)
            } else {
                let df = n1 + n2 - 2.0;
                if n1 < 1.0 || n2 < 1.0 || df <= 0.0 {
                    return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
                }
                let delta = *d * ((n1 * n2) / (n1 + n2)).sqrt();
                t_power(df, delta, ctx.alpha, ctx.tail_mode, None)
            }
        }
        ResolvedEffect::SingleT { d, .. } => {
            let df = n1 - 1.0;
            if df <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let delta = *d * n1.sqrt();
            t_power(df, delta, ctx.alpha, ctx.tail_mode, None)
        }
        ResolvedEffect::Anova { k, f, .. } => {
            let df1 = (*k - 1) as f64;
            let df2 = (*k as f64) * (n1 - 1.0);
            if df2 <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let f_crit = f_isf(ctx.alpha, df1, df2)?;
            let lambda = (*k as f64) * n1 * (*f) * (*f);
            let power = ncf_sf(f_crit, df1, df2, lambda)?.clamp(0.0, 1.0);
            Ok(PowerEval { power, actual_df: Some(df2), numerator_df: Some(df1), noncentrality_param: lambda, critical_value: f_crit })
        }
        ResolvedEffect::TwoProportions { p1: Some(p1), p2: Some(p2), .. } => {
            let n2 = eff_n2.unwrap_or(ctx.allocation_ratio * n1);
            if n1 < 1.0 || n2 < 1.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            two_proportions_p_power(ctx, n1, n2, *p1, *p2)
        }
        ResolvedEffect::TwoProportions { h, .. } => {
            let n2 = eff_n2.unwrap_or(ctx.allocation_ratio * n1);
            if n1 < 1.0 || n2 < 1.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            two_proportions_h_power(n1, n2, *h, ctx.alpha, ctx.tail_mode)
        }
        ResolvedEffect::Correlation { r } => correlation_power(ctx.correlation_method, n1, *r, ctx.alpha, ctx.tail_mode),
        ResolvedEffect::ChiSquare { df, w } => {
            if !n1.is_finite() || n1 <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let chi2_crit = chi2_isf(ctx.alpha, *df)?;
            let lambda = n1 * (*w) * (*w);
            let power = ncchi2_sf(chi2_crit, *df, lambda)?.clamp(0.0, 1.0);
            Ok(PowerEval { power, actual_df: Some(*df), numerator_df: None, noncentrality_param: lambda, critical_value: chi2_crit })
        }
        ResolvedEffect::LinearRegression { f2, sign, .. } => {
            let df = n1 - 2.0;
            if df <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let delta = *sign * (n1 * (*f2)).sqrt();
            t_power(df, delta, ctx.alpha, ctx.tail_mode, Some(1.0))
        }
    }
}

// ============================================================================
// Root-Finding Solvers (SampleSize & DetectableEffect)
// ============================================================================

fn min_continuous_n(ctx: &ResolvedContext) -> f64 {
    match ctx.family {
        PowerTestFamily::TwoSampleUnpairedT => {
            if let Some(n2_fix) = ctx.fixed_n2 {
                return match &ctx.effect {
                    ResolvedEffect::UnpairedT { welch_sds: Some(_), .. } => 1.0 + 1e-6,
                    _ => (2.25 - n2_fix).max(1.0),
                };
            }
            let k = ctx.allocation_ratio;
            match &ctx.effect {
                ResolvedEffect::UnpairedT { welch_sds: Some(_), .. } => (1.0_f64 + 1e-6).max((1.0 + 1e-6) / k),
                _ => (2.25 / (1.0 + k)).max(1.0).max((1.0 + 1e-12) / k),
            }
        }
        PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => 1.25,
        PowerTestFamily::OneWayAnova => {
            let k = match &ctx.effect {
                ResolvedEffect::Anova { k, .. } => *k as f64,
                _ => 2.0,
            };
            1.0 + 0.5 / k
        }
        PowerTestFamily::TwoProportions => {
            if ctx.fixed_n2.is_some() {
                1.0
            } else {
                1.0_f64.max((1.0 + 1e-12) / ctx.allocation_ratio)
            }
        }
        PowerTestFamily::ChiSquare => 1.0,
        PowerTestFamily::Correlation => match ctx.correlation_method {
            CorrelationPowerMethod::ExactTTransform | CorrelationPowerMethod::ConditionalSlopeT => 2.25,
            CorrelationPowerMethod::FisherZ | CorrelationPowerMethod::FisherZWithTCrit => 3.005,
        },
        PowerTestFamily::SimpleLinearRegression => 2.25,
    }
}

fn min_curve_integer_n(ctx: &ResolvedContext) -> Result<usize, StatsError> {
    let base = ctx.family.min_integer_n();
    match ctx.family {
        PowerTestFamily::TwoSampleUnpairedT => {
            if let Some(n2_fix) = ctx.fixed_n2 {
                let need = match &ctx.effect {
                    ResolvedEffect::UnpairedT { welch_sds: Some(_), .. } => 2,
                    _ => {
                        let n2_int = f64_to_checked_usize("sample_size_group2", n2_fix.ceil())?.max(1);
                        3usize.saturating_sub(n2_int).max(1)
                    }
                };
                return Ok(base.max(need));
            }
            let need = match &ctx.effect {
                ResolvedEffect::UnpairedT { welch_sds: Some(_), .. } => {
                    let mut n = f64_to_checked_usize("sample_size", (2.0 / ctx.allocation_ratio).ceil().max(2.0))?;
                    while (n as f64) * ctx.allocation_ratio <= 1.0 {
                        n = n.checked_add(1).ok_or(StatsError::InvalidParameter { name: "sample_size", value: f64::INFINITY })?;
                    }
                    n
                }
                _ => {
                    let start = (1.0 / ctx.allocation_ratio).max(2.25 / (1.0 + ctx.allocation_ratio)).ceil();
                    let mut n = f64_to_checked_usize("sample_size", start)?;
                    while (n as f64) * ctx.allocation_ratio < 1.0 || (n as f64) + (n as f64) * ctx.allocation_ratio - 2.0 <= 0.0 {
                        n = n.checked_add(1).ok_or(StatsError::InvalidParameter { name: "sample_size", value: f64::INFINITY })?;
                    }
                    n
                }
            };
            Ok(base.max(need))
        }
        PowerTestFamily::TwoProportions => {
            if ctx.fixed_n2.is_some() {
                return Ok(base.max(1));
            }
            let mut n = f64_to_checked_usize("sample_size", (1.0 / ctx.allocation_ratio).ceil())?;
            while (n as f64) * ctx.allocation_ratio < 1.0 {
                n = n.checked_add(1).ok_or(StatsError::InvalidParameter { name: "sample_size", value: f64::INFINITY })?;
            }
            Ok(base.max(n))
        }
        _ => Ok(base),
    }
}

fn validate_directional_effect_for_sample_size(ctx: &ResolvedContext, target_power: f64) -> Result<(), StatsError> {
    let signed = match &ctx.effect {
        ResolvedEffect::UnpairedT { d, .. } | ResolvedEffect::SingleT { d, .. } => *d,
        ResolvedEffect::Anova { f, .. } => *f,
        ResolvedEffect::TwoProportions { p1: Some(p1), p2: Some(p2), .. } => *p1 - *p2,
        ResolvedEffect::TwoProportions { p1: Some(_), p2: None, .. } => {
            return Err(StatsError::InvalidParameter { name: "p2", value: f64::NAN });
        }
        ResolvedEffect::TwoProportions { h, .. } => *h,
        ResolvedEffect::Correlation { r } => *r,
        ResolvedEffect::ChiSquare { w, .. } => *w,
        ResolvedEffect::LinearRegression { f2, sign, .. } => *sign * f2.sqrt(),
    };

    if signed == 0.0 {
        return Err(StatsError::InvalidParameter { name: "effect_size", value: 0.0 });
    }
    if target_power <= ctx.alpha {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    match ctx.tail_mode {
        TailMode::OneSided(OneSidedDirection::Greater) if signed < 0.0 => {
            return Err(StatsError::InvalidParameter { name: "effect_size", value: signed });
        }
        TailMode::OneSided(OneSidedDirection::Less) if signed > 0.0 => {
            return Err(StatsError::InvalidParameter { name: "effect_size", value: signed });
        }
        _ => {}
    }
    Ok(())
}

fn asymptotic_n_guess(ctx: &ResolvedContext, target_power: f64) -> f64 {
    let z_alpha = z_critical_value(ctx.alpha, ctx.tail_mode).unwrap_or(1.96);
    let z_beta = norm_ppf(target_power).unwrap_or(0.84);
    let z_sum = (z_alpha + z_beta).abs().max(0.5);
    let k = ctx.allocation_ratio;

    match &ctx.effect {
        ResolvedEffect::UnpairedT { d, .. } => ((z_sum / d.abs().max(1e-6)).powi(2) * (1.0 + k) / k).max(4.0),
        ResolvedEffect::SingleT { d, .. } => ((z_sum / d.abs().max(1e-6)).powi(2) + 1.5).max(4.0),
        ResolvedEffect::Anova { f, k: groups, .. } => {
            let num = (z_sum + ((*groups - 1) as f64).sqrt()).powi(2);
            (num / ((*groups as f64) * f.abs().max(1e-6).powi(2))).max(4.0)
        }
        ResolvedEffect::TwoProportions { p1: Some(p1), p2: Some(p2), .. } if ctx.two_proportions_mode != TwoProportionsMethod::CohensH => {
            let delta = (p1 - p2).abs().max(1e-6);
            let p_bar = (*p1 + k * *p2) / (1.0 + k);
            let s0 = (p_bar * (1.0 - p_bar) * (1.0 + 1.0 / k)).sqrt();
            let s1 = (*p1 * (1.0 - *p1) + *p2 * (1.0 - *p2) / k).sqrt();
            ((z_alpha * s0 + z_beta * s1) / delta).powi(2).max(2.0)
        }
        ResolvedEffect::TwoProportions { p1: Some(_), p2: None, .. } => 10.0,
        ResolvedEffect::TwoProportions { h, .. } => ((z_sum / h.abs().max(1e-6)).powi(2) * (1.0 + k) / k).max(2.0),
        ResolvedEffect::Correlation { r } => ((z_sum / r.atanh().abs().max(1e-6)).powi(2) + 3.0).max(5.0),
        ResolvedEffect::ChiSquare { w, df } => ((z_sum + df.sqrt()).powi(2) / w.abs().max(1e-6).powi(2)).max(2.0),
        ResolvedEffect::LinearRegression { f2, .. } => ((z_sum * z_sum) / f2.abs().max(1e-8) + 2.0).max(4.0),
    }
}

fn solve_continuous_n(ctx: &ResolvedContext) -> Result<f64, StatsError> {
    let target_power = ctx.target_power.ok_or(StatsError::InvalidParameter { name: "target_power", value: f64::NAN })?;
    validate_directional_effect_for_sample_size(ctx, target_power)?;

    let n_min = min_continuous_n(ctx);
    if ctx.family == PowerTestFamily::ChiSquare
        && let ResolvedEffect::ChiSquare { df, w } = &ctx.effect
    {
        let chi2_crit = chi2_isf(ctx.alpha, *df)?;
        let lambda = solve_chi2_lambda(chi2_crit, *df, target_power)?;
        return Ok((lambda / (*w * *w)).max(n_min));
    }

    let guess = asymptotic_n_guess(ctx, target_power).max(n_min + 1.0);
    let mut lo = (0.25 * guess).max(n_min);
    let mut hi = (2.0 * guess).max(n_min + 4.0);

    let eval = |n: f64| -> f64 { eval_power_at(ctx, n, None).map_or(f64::NAN, |e| e.power - target_power) };

    let mut f_lo = eval(lo);
    if !f_lo.is_finite() {
        return Err(StatsError::NotConverged { algorithm: "solve_continuous_n" });
    }
    if f_lo > 0.0 {
        lo = n_min;
        f_lo = eval(lo);
        if f_lo >= 0.0 {
            return Ok(n_min);
        }
    }

    let mut f_hi = eval(hi);
    for _ in 0..48 {
        if !f_hi.is_finite() {
            return Err(StatsError::NotConverged { algorithm: "solve_continuous_n" });
        }
        if f_hi >= 0.0 {
            return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
        }
        lo = hi;
        hi *= 2.0;
        if hi > MAX_CONTINUOUS_N {
            return Err(StatsError::NotConverged { algorithm: "solve_continuous_n" });
        }
        f_hi = eval(hi);
    }
    Err(StatsError::NotConverged { algorithm: "solve_continuous_n" })
}

fn solve_chi2_lambda(chi2_crit: f64, df: f64, target_power: f64) -> Result<f64, StatsError> {
    let eval = |lam: f64| -> f64 { ncchi2_sf(chi2_crit, df, lam).map_or(f64::NAN, |p| p - target_power) };
    let mut lo = 0.0_f64;
    let mut hi = (df + 8.0).max(4.0);
    for _ in 0..48 {
        if eval(hi) >= 0.0 {
            return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
        }
        lo = hi;
        hi *= 2.5;
    }
    Err(StatsError::NotConverged { algorithm: "solve_chi2_lambda" })
}

fn solve_t_delta(df: f64, alpha: f64, tail_mode: TailMode, target_power: f64) -> Result<f64, StatsError> {
    let p_zero = t_power(df, 0.0, alpha, tail_mode, None)?.power;
    if (p_zero - target_power).abs() <= 1e-13 && matches!(tail_mode, TailMode::OneSided(_)) {
        return Ok(0.0);
    }
    if matches!(tail_mode, TailMode::TwoSided) && target_power <= p_zero {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }

    let search_positive = match tail_mode {
        TailMode::TwoSided | TailMode::OneSided(OneSidedDirection::Greater) => target_power >= p_zero,
        TailMode::OneSided(OneSidedDirection::Less) => target_power < p_zero,
    };

    let eval = |delta: f64| -> f64 { t_power(df, delta, alpha, tail_mode, None).map_or(f64::NAN, |e| e.power - target_power) };
    let f_zero = p_zero - target_power;
    if search_positive {
        let mut lo = 0.0_f64;
        let mut f_lo = f_zero;
        let mut hi = 2.0_f64;
        for _ in 0..48 {
            let f_hi = eval(hi);
            if !f_hi.is_finite() {
                break;
            }
            if f_lo * f_hi <= 0.0 {
                return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
            }
            lo = hi;
            f_lo = f_hi;
            hi *= 2.0;
        }
    } else {
        let mut hi = 0.0_f64;
        let mut f_hi = f_zero;
        let mut lo = -2.0_f64;
        for _ in 0..48 {
            let f_lo = eval(lo);
            if !f_lo.is_finite() {
                break;
            }
            if f_lo * f_hi <= 0.0 {
                return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
            }
            hi = lo;
            f_hi = f_lo;
            lo *= 2.0;
        }
    }
    Err(StatsError::NotConverged { algorithm: "solve_t_delta" })
}

fn solve_anova_f(k: usize, n_per_group: f64, alpha: f64, target_power: f64) -> Result<f64, StatsError> {
    let df1 = (k - 1) as f64;
    let df2 = (k as f64) * (n_per_group - 1.0);
    if df2 <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "sample_size", value: n_per_group });
    }
    if target_power <= alpha {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    let f_crit = f_isf(alpha, df1, df2)?;
    let eval = |f: f64| -> f64 {
        let lam = (k as f64) * n_per_group * f * f;
        ncf_sf(f_crit, df1, df2, lam).map_or(f64::NAN, |p| p - target_power)
    };
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..48 {
        if eval(hi) >= 0.0 {
            return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
        }
        lo = hi;
        hi *= 2.0;
    }
    Err(StatsError::NotConverged { algorithm: "solve_anova_f" })
}

fn solve_correlation_r(ctx: &ResolvedContext, n: f64, target_power: f64) -> Result<f64, StatsError> {
    let p_zero = correlation_power(ctx.correlation_method, n, 0.0, ctx.alpha, ctx.tail_mode)?.power;
    if (target_power - p_zero).abs() <= 1e-13 && matches!(ctx.tail_mode, TailMode::OneSided(_)) {
        return Ok(0.0);
    }
    if matches!(ctx.tail_mode, TailMode::TwoSided) && target_power <= p_zero {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    let search_positive = match ctx.tail_mode {
        TailMode::TwoSided | TailMode::OneSided(OneSidedDirection::Greater) => target_power >= p_zero,
        TailMode::OneSided(OneSidedDirection::Less) => target_power < p_zero,
    };
    let eval =
        |r: f64| -> f64 { correlation_power(ctx.correlation_method, n, r, ctx.alpha, ctx.tail_mode).map_or(f64::NAN, |e| e.power - target_power) };
    let max_r = 1.0 - 1e-12;
    let (lo, hi) = if search_positive { (0.0, max_r) } else { (-max_r, 0.0) };
    let f_lo = eval(lo);
    let f_hi = eval(hi);
    if !f_lo.is_finite() || !f_hi.is_finite() || f_lo * f_hi > 0.0 {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    Ok(brent_root(eval, lo, hi, ROOT_TOL)?)
}

fn solve_two_proportions_p2(
    ctx: &ResolvedContext,
    n1: f64,
    n2: f64,
    p1: f64,
    hint_p2: Option<f64>,
    target_power: f64,
) -> Result<(f64, f64), StatsError> {
    let eps = 1e-9;
    let p_zero = two_proportions_p_power(ctx, n1, n2, p1, p1)?.power;
    if (target_power - p_zero).abs() <= 1e-13 && matches!(ctx.tail_mode, TailMode::OneSided(_)) {
        return Ok((p1, 0.0));
    }
    if matches!(ctx.tail_mode, TailMode::TwoSided) && target_power <= p_zero {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    let eval_p2 = |p2: f64| -> f64 { two_proportions_p_power(ctx, n1, n2, p1, p2).map_or(f64::NAN, |e| e.power - target_power) };

    let solve_bracket = |lo: f64, hi: f64| -> Option<f64> {
        let f_lo = eval_p2(lo);
        let f_hi = eval_p2(hi);
        if f_lo.is_finite() && f_hi.is_finite() && f_lo * f_hi <= 0.0 { brent_root(eval_p2, lo, hi, ROOT_TOL).ok() } else { None }
    };
    let solve_lower = || solve_bracket(eps, p1);
    let solve_upper = || solve_bracket(p1, 1.0 - eps);

    let p2 = match ctx.tail_mode {
        TailMode::OneSided(OneSidedDirection::Greater) => {
            if target_power >= p_zero {
                solve_lower()
            } else {
                solve_upper()
            }
        }
        TailMode::OneSided(OneSidedDirection::Less) => {
            if target_power >= p_zero {
                solve_upper()
            } else {
                solve_lower()
            }
        }
        TailMode::TwoSided => match hint_p2 {
            Some(h2) if h2 < p1 => solve_lower().or_else(solve_upper),
            Some(h2) if h2 > p1 => solve_upper().or_else(solve_lower),
            _ if p1 <= 0.5 => solve_upper().or_else(solve_lower),
            _ => solve_lower().or_else(solve_upper),
        },
    };
    let p2_val = p2.ok_or(StatsError::InvalidParameter { name: "target_power", value: target_power })?;
    Ok((p2_val, cohens_h(p1, p2_val)?))
}

/// Solve minimum detectable Cohen's `h` in `[-pi, pi]` (Cohen 1988, ch. 6), returning
/// [`StatsError::InvalidParameter`] when `target_power` cannot be achieved within `|h| <= pi`.
fn solve_two_proportions_h(ctx: &ResolvedContext, n1: f64, n2: f64, target_power: f64) -> Result<f64, StatsError> {
    let p_zero = two_proportions_h_power(n1, n2, 0.0, ctx.alpha, ctx.tail_mode)?.power;
    if (target_power - p_zero).abs() <= 1e-13 && matches!(ctx.tail_mode, TailMode::OneSided(_)) {
        return Ok(0.0);
    }
    if matches!(ctx.tail_mode, TailMode::TwoSided) && target_power <= p_zero {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    let search_positive = match ctx.tail_mode {
        TailMode::TwoSided | TailMode::OneSided(OneSidedDirection::Greater) => target_power >= p_zero,
        TailMode::OneSided(OneSidedDirection::Less) => target_power < p_zero,
    };
    let eval = |h: f64| -> f64 { two_proportions_h_power(n1, n2, h, ctx.alpha, ctx.tail_mode).map_or(f64::NAN, |e| e.power - target_power) };
    let max_h = std::f64::consts::PI;
    let (lo, hi) = if search_positive { (0.0, max_h) } else { (-max_h, 0.0) };
    let f_lo = eval(lo);
    let f_hi = eval(hi);
    if !f_lo.is_finite() || !f_hi.is_finite() || f_lo * f_hi > 0.0 {
        return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
    }
    Ok(brent_root(eval, lo, hi, ROOT_TOL)?)
}

fn solve_detectable_effect(ctx: &mut ResolvedContext, n1: f64, n2_explicit: Option<f64>) -> Result<(), StatsError> {
    let target_power = ctx.target_power.ok_or(StatsError::InvalidParameter { name: "target_power", value: f64::NAN })?;
    let n2 = n2_explicit.unwrap_or(ctx.allocation_ratio * n1);

    if let ResolvedEffect::TwoProportions { p1: Some(p1), p2: hint_p2, .. } = &ctx.effect {
        if n1 < 1.0 || n2 < 1.0 {
            return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
        }
        let (solved_p2, solved_h) = solve_two_proportions_p2(ctx, n1, n2, *p1, *hint_p2, target_power)?;
        if let ResolvedEffect::TwoProportions { p2, h, .. } = &mut ctx.effect {
            *p2 = Some(solved_p2);
            *h = solved_h;
        }
        return Ok(());
    }
    if matches!(&ctx.effect, ResolvedEffect::TwoProportions { p1: None, .. }) {
        if n1 < 1.0 || n2 < 1.0 {
            return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
        }
        let solved_h = solve_two_proportions_h(ctx, n1, n2, target_power)?;
        if let ResolvedEffect::TwoProportions { h, .. } = &mut ctx.effect {
            *h = solved_h;
        }
        return Ok(());
    }
    if matches!(&ctx.effect, ResolvedEffect::Correlation { .. }) {
        let solved_r = solve_correlation_r(ctx, n1, target_power)?;
        if let ResolvedEffect::Correlation { r } = &mut ctx.effect {
            *r = solved_r;
        }
        return Ok(());
    }

    match &mut ctx.effect {
        ResolvedEffect::UnpairedT { d, welch_sds, .. } => {
            if let Some((mean_diff, sd1, sd2)) = welch_sds {
                let df = welch_satterthwaite_df(n1, n2, *sd1, *sd2)?;
                let delta = solve_t_delta(df, ctx.alpha, ctx.tail_mode, target_power)?;
                let se = ((*sd1 * *sd1) / n1 + (*sd2 * *sd2) / n2).sqrt();
                *mean_diff = delta * se;
                let eff_sd = ((ctx.allocation_ratio * *sd1 * *sd1 + *sd2 * *sd2) / (ctx.allocation_ratio + 1.0)).sqrt();
                *d = *mean_diff / eff_sd;
            } else {
                let df = n1 + n2 - 2.0;
                if n1 < 1.0 || n2 < 1.0 || df <= 0.0 {
                    return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
                }
                let delta = solve_t_delta(df, ctx.alpha, ctx.tail_mode, target_power)?;
                *d = delta / ((n1 * n2) / (n1 + n2)).sqrt();
            }
        }
        ResolvedEffect::SingleT { d, .. } => {
            let df = n1 - 1.0;
            if df <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let delta = solve_t_delta(df, ctx.alpha, ctx.tail_mode, target_power)?;
            *d = delta / n1.sqrt();
        }
        ResolvedEffect::Anova { k, f, .. } => {
            *f = solve_anova_f(*k, n1, ctx.alpha, target_power)?;
        }
        ResolvedEffect::TwoProportions { .. } | ResolvedEffect::Correlation { .. } => {}
        ResolvedEffect::ChiSquare { df, w } => {
            if !n1.is_finite() || n1 <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            if target_power <= ctx.alpha {
                return Err(StatsError::InvalidParameter { name: "target_power", value: target_power });
            }
            let chi2_crit = chi2_isf(ctx.alpha, *df)?;
            let lambda = solve_chi2_lambda(chi2_crit, *df, target_power)?;
            *w = (lambda / n1).sqrt();
        }
        ResolvedEffect::LinearRegression { f2, sign, .. } => {
            let df = n1 - 2.0;
            if df <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
            }
            let delta = solve_t_delta(df, ctx.alpha, ctx.tail_mode, target_power)?;
            *f2 = (delta * delta) / n1;
            *sign = if delta < 0.0 { -1.0 } else { 1.0 };
        }
    }
    Ok(())
}

// ============================================================================
// Integer Ceiling, Metadata Extraction & Public API
// ============================================================================

fn extract_input_n(req: &PowerAnalysisRequest, ctx: &ResolvedContext) -> Result<(f64, Option<f64>), StatsError> {
    let n1 = req.sample_size.ok_or(StatsError::InvalidParameter { name: "sample_size", value: f64::NAN })?;
    let n_min = min_continuous_n(ctx);
    if !n1.is_finite() || n1 < n_min || n1 > MAX_CONTINUOUS_N {
        return Err(StatsError::InvalidParameter { name: "sample_size", value: n1 });
    }
    let is_welch = matches!(&ctx.effect, ResolvedEffect::UnpairedT { welch_sds: Some(_), .. });
    let n2 = match req.sample_size_group2 {
        Some(v) => {
            if !matches!(req.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions) {
                return Err(StatsError::IncompatibleAnalysisSpec { expected: "sample_size_group2 is only valid for two-sample designs" });
            }
            let min_n2 = if is_welch { 1.0 + 1e-6 } else { 1.0 };
            if !v.is_finite() || v < min_n2 || v > MAX_CONTINUOUS_N {
                return Err(StatsError::InvalidParameter { name: "sample_size_group2", value: v });
            }
            Some(v)
        }
        None => None,
    };
    Ok((n1, n2))
}

fn min_group2_integer_n(ctx: &ResolvedContext) -> usize {
    match &ctx.effect {
        ResolvedEffect::UnpairedT { welch_sds: Some(_), .. } => 2,
        _ => 1,
    }
}

fn checked_two_sample_or_total_counts(ctx: &ResolvedContext, n1: usize, n2_explicit: Option<f64>) -> Result<(Option<usize>, usize), StatsError> {
    match ctx.family {
        PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions => {
            let n2_min = min_group2_integer_n(ctx);
            let mut n2 = match n2_explicit.or(ctx.fixed_n2) {
                Some(n2v) => f64_to_checked_usize("sample_size_group2", n2v.ceil())?,
                None => f64_to_checked_usize("sample_size", ((n1 as f64) * ctx.allocation_ratio).ceil())?,
            };
            n2 = n2.max(n2_min);
            if matches!(ctx.family, PowerTestFamily::TwoSampleUnpairedT) && n1.saturating_add(n2) <= 2 {
                n2 = n2.max(3usize.saturating_sub(n1));
            }
            let tot = n1.checked_add(n2).ok_or(StatsError::InvalidParameter { name: "total_n", value: f64::INFINITY })?;
            Ok((Some(n2), tot))
        }
        PowerTestFamily::OneWayAnova => {
            let k = match &ctx.effect {
                ResolvedEffect::Anova { k, .. } => *k,
                _ => 2,
            };
            let tot = k.checked_mul(n1).ok_or(StatsError::InvalidParameter { name: "total_n", value: f64::INFINITY })?;
            Ok((None, tot))
        }
        _ => Ok((None, n1)),
    }
}

fn ceiling_counts(ctx: &ResolvedContext, n1_cont: f64, n2_explicit: Option<f64>) -> Result<(usize, Option<usize>, usize), StatsError> {
    let min_int = ctx.family.min_integer_n();
    let n1_ceil = f64_to_checked_usize("sample_size", n1_cont.ceil())?.max(min_int);
    let (n_group2, total_n) = checked_two_sample_or_total_counts(ctx, n1_ceil, n2_explicit)?;
    Ok((n1_ceil, n_group2, total_n))
}

struct DerivedEffectSummary {
    canonical: f64,
    raw: Option<f64>,
    implied_p2: Option<f64>,
    cohens_h: Option<f64>,
    implied_r_squared: Option<f64>,
}

fn summarize_effect(effect: &ResolvedEffect) -> DerivedEffectSummary {
    match effect {
        ResolvedEffect::UnpairedT { d, raw_scale_sd, .. } | ResolvedEffect::SingleT { d, raw_scale_sd } => {
            DerivedEffectSummary { canonical: *d, raw: raw_scale_sd.map(|sd| d * sd), implied_p2: None, cohens_h: None, implied_r_squared: None }
        }
        ResolvedEffect::Anova { f, within_sd, .. } => {
            DerivedEffectSummary { canonical: *f, raw: within_sd.map(|sd| f * sd), implied_p2: None, cohens_h: None, implied_r_squared: None }
        }
        ResolvedEffect::TwoProportions { p1, p2, h } => DerivedEffectSummary {
            canonical: *h,
            raw: match (p1, p2) {
                (Some(a), Some(b)) => Some(a - b),
                _ => None,
            },
            implied_p2: *p2,
            cohens_h: Some(*h),
            implied_r_squared: None,
        },
        ResolvedEffect::Correlation { r } => {
            DerivedEffectSummary { canonical: *r, raw: Some(*r), implied_p2: None, cohens_h: None, implied_r_squared: Some(r * r) }
        }
        ResolvedEffect::ChiSquare { w, .. } => {
            DerivedEffectSummary { canonical: *w, raw: None, implied_p2: None, cohens_h: None, implied_r_squared: None }
        }
        ResolvedEffect::LinearRegression { f2, sign, slope_scale } => DerivedEffectSummary {
            canonical: *f2,
            raw: slope_scale.map(|sc| *sign * f2.sqrt() * sc),
            implied_p2: None,
            cohens_h: None,
            implied_r_squared: Some(*f2 / (1.0 + *f2)),
        },
    }
}

fn build_convention_notes(ctx: &ResolvedContext) -> Vec<String> {
    let mut notes = Vec::new();
    match ctx.family {
        PowerTestFamily::TwoSampleUnpairedT => {
            let alloc_desc = match ctx.fixed_n2 {
                Some(n2_fix) => format!("fixed n2 = {n2_fix:.1}"),
                None => format!("n2/n1 = {:.3}", ctx.allocation_ratio),
            };
            if matches!(&ctx.effect, ResolvedEffect::UnpairedT { welch_sds: Some(_), .. }) {
                notes.push(format!("Unpaired Welch t-test using Welch-Satterthwaite df and exact noncentral-t distribution ({alloc_desc})."));
            } else {
                notes.push(format!("Unpaired Student t-test using exact noncentral-t distribution ({alloc_desc})."));
            }
        }
        PowerTestFamily::TwoSamplePairedT => {
            notes.push(String::from("Paired t-test using exact noncentral-t on paired differences (sigma_d = sigma * sqrt(2*(1 - r)))."));
        }
        PowerTestFamily::OneSampleT => {
            notes.push(String::from("One-sample t-test using exact noncentral-t distribution (df = n - 1)."));
        }
        PowerTestFamily::OneWayAnova => {
            notes.push(String::from("Balanced one-way ANOVA using exact noncentral-F distribution (lambda = k * n * f^2)."));
        }
        PowerTestFamily::TwoProportions => {
            let mode_str = match ctx.two_proportions_mode {
                TwoProportionsMethod::PooledNullUnpooledAlt => "pooled null SE / unpooled alternative SE",
                TwoProportionsMethod::Unpooled => "unpooled Wald SE",
                TwoProportionsMethod::CohensH => "arcsine Cohen's h approximation",
            };
            let cc = if ctx.continuity_correction { " with Yates/Fleiss continuity correction" } else { "" };
            notes.push(format!("Two-proportions normal approximation ({mode_str}{cc})."));
        }
        PowerTestFamily::Correlation => {
            let conv = match ctx.correlation_method {
                CorrelationPowerMethod::FisherZ => "Fisher z-transformation (z = atanh(r), SE = 1/sqrt(n - 3)) vs H0: r0 = 0",
                CorrelationPowerMethod::FisherZWithTCrit => "R pwr.r.test convention (exact t-transform r_c + bias-corrected Fisher z)",
                CorrelationPowerMethod::ExactTTransform => {
                    "Exact unconditional bivariate-normal r distribution (Fisher 1915 / Hotelling 1953 / Anderson Ch. 4, df = n - 2)"
                }
                CorrelationPowerMethod::ConditionalSlopeT => {
                    "Conditional fixed-x slope t-test (df = n - 2, lambda = n * f^2, aligned with SimpleLinearRegression)"
                }
            };
            notes.push(format!("Pearson correlation power via {conv}."));
        }
        PowerTestFamily::ChiSquare => {
            notes.push(String::from("Chi-square goodness-of-fit/contingency power via exact noncentral chi-square (lambda = N * w^2)."));
        }
        PowerTestFamily::SimpleLinearRegression => {
            notes.push(String::from("Simple linear regression slope test (u = 1, df = n - 2, lambda = n * f^2)."));
        }
    }
    notes
}

fn verify_discrete_ceiling(
    ctx: &ResolvedContext,
    mut n_per_group: usize,
    mut n_group2: Option<usize>,
    mut total_n: usize,
    mut ceil_eval: PowerEval,
) -> Result<(usize, Option<usize>, usize, PowerEval), StatsError> {
    if ctx.solve_target != PowerSolveTarget::SampleSize {
        return Ok((n_per_group, n_group2, total_n, ceil_eval));
    }
    let Some(tp) = ctx.target_power else {
        return Ok((n_per_group, n_group2, total_n, ceil_eval));
    };
    let mut steps = 0usize;
    while ceil_eval.power + 1e-11 < tp {
        if steps >= MAX_DISCRETE_VERIFY_STEPS {
            return Err(StatsError::NotConverged { algorithm: "solve_power discrete ceiling verification" });
        }
        steps += 1;
        n_per_group = n_per_group.checked_add(1).ok_or(StatsError::InvalidParameter { name: "sample_size", value: f64::INFINITY })?;
        if (n_per_group as f64) > MAX_CONTINUOUS_N {
            return Err(StatsError::NotConverged { algorithm: "solve_power discrete ceiling verification" });
        }
        let (next_n2, next_total) = checked_two_sample_or_total_counts(ctx, n_per_group, None)?;
        n_group2 = next_n2;
        total_n = next_total;
        ceil_eval = eval_power_at(ctx, n_per_group as f64, n_group2.map(|v| v as f64))?;
    }
    Ok((n_per_group, n_group2, total_n, ceil_eval))
}

fn solve_power_core_with_ctx(req: &PowerAnalysisRequest) -> Result<(PowerAnalysisResult, ResolvedContext), StatsError> {
    let mut ctx = build_context(req)?;
    let (continuous_n, n2_explicit, power_at_continuous_n) = match ctx.solve_target {
        PowerSolveTarget::SampleSize => {
            let n_cont = solve_continuous_n(&ctx)?;
            let ev_cont = eval_power_at(&ctx, n_cont, None)?;
            (n_cont, None, ev_cont.power)
        }
        PowerSolveTarget::Power => {
            let (n1, n2) = extract_input_n(req, &ctx)?;
            let ev_cont = eval_power_at(&ctx, n1, n2)?;
            (n1, n2, ev_cont.power)
        }
        PowerSolveTarget::DetectableEffect => {
            let (n1, n2) = extract_input_n(req, &ctx)?;
            solve_detectable_effect(&mut ctx, n1, n2)?;
            let ev_cont = eval_power_at(&ctx, n1, n2)?;
            (n1, n2, ev_cont.power)
        }
    };

    let (n1_ceil, n2_ceil, total_ceil) = ceiling_counts(&ctx, continuous_n, n2_explicit)?;
    let ceil_eval_init = eval_power_at(&ctx, n1_ceil as f64, n2_ceil.map(|v| v as f64))?;
    let (n_per_group, n_group2, total_n, ceil_eval) = verify_discrete_ceiling(&ctx, n1_ceil, n2_ceil, total_ceil, ceil_eval_init)?;
    let eff = summarize_effect(&ctx.effect);

    let result = PowerAnalysisResult {
        family: ctx.family,
        solve_target: ctx.solve_target,
        tail_mode: ctx.tail_mode,
        alpha: ctx.alpha,
        target_power: ctx.target_power,
        continuous_n,
        n_per_group,
        n_group2,
        total_n,
        power_at_continuous_n,
        achieved_power: ceil_eval.power,
        effect_size: eff.canonical,
        raw_effect_size: eff.raw,
        implied_p2: eff.implied_p2,
        cohens_h: eff.cohens_h,
        implied_r_squared: eff.implied_r_squared,
        actual_df: ceil_eval.actual_df,
        numerator_df: ceil_eval.numerator_df,
        noncentrality_param: ceil_eval.noncentrality_param,
        critical_value: ceil_eval.critical_value,
        power_curve: Vec::new(),
        sensitivity_table: Vec::new(),
        notes: build_convention_notes(&ctx),
    };
    Ok((result, ctx))
}

fn solve_power_core(req: &PowerAnalysisRequest) -> Result<PowerAnalysisResult, StatsError> {
    solve_power_core_with_ctx(req).map(|(res, _)| res)
}

/// Solve a power, sample-size, or minimum-detectable-effect request across any of the 8 test
/// families, populating Stage 4 power curves and sensitivity tables.
///
/// # Errors
/// Returns [`StatsError`] if `alpha`, `target_power`, `sample_size`, `tail_mode`, or effect-size
/// parameters are invalid, incompatible, or cannot achieve the requested power.
pub fn solve_power(req: &PowerAnalysisRequest) -> Result<PowerAnalysisResult, StatsError> {
    let (mut res, ctx) = solve_power_core_with_ctx(req)?;

    let mut curve_req = req.clone();
    if req.solve_target == PowerSolveTarget::DetectableEffect {
        curve_req.effect = resolved_effect_to_spec(&res, &ctx.effect);
    }
    if let ResolvedEffect::Anova { k, .. } = &ctx.effect {
        curve_req.num_groups = Some(*k);
    }
    if req.sample_size.is_some() && req.sample_size_group2.is_some() {
        curve_req.allocation_ratio = ctx.allocation_ratio;
        if req.solve_target == PowerSolveTarget::SampleSize {
            curve_req.sample_size = None;
            curve_req.sample_size_group2 = None;
        }
    }

    let min_curve_n = min_curve_integer_n(&ctx)?;
    let center_n = res.n_per_group.max(min_curve_n);
    let max_n = center_n.saturating_mul(2).max(min_curve_n.saturating_add(20)).min(1_000_000).max(center_n);
    let mut curve = compute_power_curve(&curve_req, min_curve_n, max_n, 41)?;
    if !curve.iter().any(|pt| (pt.n - (center_n as f64)).abs() < 0.5) {
        let mut center_ctx = build_context(&curve_req)?;
        center_ctx.solve_target = PowerSolveTarget::Power;
        if let Ok(ev) = eval_power_at(&center_ctx, center_n as f64, res.n_group2.map(|v| v as f64)) {
            curve.push(PowerCurvePoint { n: center_n as f64, power: ev.power, effect_size: res.effect_size, total_n: res.total_n as f64 });
            curve.sort_by(|a, b| a.n.total_cmp(&b.n));
        }
    }
    res.power_curve = curve;

    let can_build_sensitivity = match req.solve_target {
        PowerSolveTarget::DetectableEffect => true,
        _ => res.effect_size != 0.0,
    };
    if can_build_sensitivity {
        let valid_alphas: Vec<f64> = DEFAULT_SENSITIVITY_ALPHAS.iter().copied().filter(|&a| a < 0.70).collect();
        if let Ok(table) = compute_sensitivity_table(&curve_req, &valid_alphas, DEFAULT_SENSITIVITY_POWERS) {
            res.sensitivity_table = table;
        }
    }

    Ok(res)
}

fn resolved_effect_to_spec(res: &PowerAnalysisResult, effect: &ResolvedEffect) -> PowerEffectSpec {
    match effect {
        ResolvedEffect::UnpairedT { welch_sds: Some((mean_diff, sd1, sd2)), .. } => {
            PowerEffectSpec::MeanDiffAndSd { mean_diff: *mean_diff, sd: *sd1, sd_group2: Some(*sd2), within_pair_correlation: None }
        }
        ResolvedEffect::TwoProportions { .. } => match res.implied_p2 {
            Some(p2) => PowerEffectSpec::TwoProportions { p1: derive_p1_from_p2_and_h(p2, res.effect_size), p2 },
            None => PowerEffectSpec::Standardized(res.effect_size),
        },
        ResolvedEffect::LinearRegression { f2, sign, slope_scale } => {
            let sigma_eps = slope_scale.unwrap_or(1.0);
            PowerEffectSpec::RegressionSlope { beta1: *sign * f2.sqrt() * sigma_eps, sigma_x: 1.0, sigma_eps }
        }
        _ => PowerEffectSpec::Standardized(res.effect_size),
    }
}

fn derive_p1_from_p2_and_h(p2: f64, h: f64) -> f64 {
    let angle = p2.sqrt().asin() + 0.5 * h;
    let s = angle.clamp(0.0, std::f64::consts::FRAC_PI_2).sin();
    (s * s).clamp(0.0, 1.0)
}

/// Evaluate the power-vs-sample-size curve across `[n_min, n_max]` using up to `num_points`
/// deduplicated integer sample sizes.
pub fn compute_power_curve(req: &PowerAnalysisRequest, n_min: usize, n_max: usize, num_points: usize) -> Result<Vec<PowerCurvePoint>, StatsError> {
    let mut curve_req = req.clone();
    curve_req.solve_target = PowerSolveTarget::Power;
    let ctx = build_context(&curve_req)?;
    let eff = summarize_effect(&ctx.effect);

    let min_allowed = min_curve_integer_n(&ctx)?;
    let lo = n_min.max(min_allowed);
    if n_max < lo {
        return Err(StatsError::InvalidParameter { name: "n_max", value: n_max as f64 });
    }
    if num_points == 0 {
        return Ok(Vec::new());
    }

    let design_pair = if matches!(ctx.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions)
        && let (Some(n1_raw), Some(n2_raw)) = (req.sample_size, req.sample_size_group2)
    {
        Some((f64_to_checked_usize("sample_size", n1_raw.ceil())?, n2_raw))
    } else {
        None
    };

    let n_values = sample_integer_grid(lo, n_max, num_points);
    let mut points = Vec::with_capacity(n_values.len());
    for n in n_values {
        let nf = n as f64;
        let explicit_n2 = match design_pair {
            Some((n1_ceil, n2_raw)) if n == n1_ceil => Some(n2_raw),
            _ => ctx.fixed_n2,
        };
        let (n2_int, total_int) = checked_two_sample_or_total_counts(&ctx, n, explicit_n2)?;
        let ev = eval_power_at(&ctx, nf, n2_int.map(|v| v as f64))?;
        points.push(PowerCurvePoint { n: nf, power: ev.power, effect_size: eff.canonical, total_n: total_int as f64 });
    }
    Ok(points)
}

fn sample_integer_grid(lo: usize, hi: usize, num_points: usize) -> Vec<usize> {
    if lo >= hi || num_points <= 1 {
        return vec![lo];
    }
    let span = hi - lo;
    if span < num_points {
        return (lo..=hi).collect();
    }
    let mut out = Vec::with_capacity(num_points);
    let denom = (num_points - 1) as f64;
    for i in 0..num_points {
        let frac = (i as f64) / denom;
        let delta = (frac * (span as f64)).round() as usize;
        let val = lo.saturating_add(delta).min(hi);
        if out.last().copied() != Some(val) {
            out.push(val);
        }
    }
    if out.last().copied() != Some(hi) {
        out.push(hi);
    }
    out
}

/// Compute a Stage 4 sensitivity matrix across every `(alpha, target_power)` combination.
///
/// When `req.solve_target == PowerSolveTarget::DetectableEffect`, each cell reports the
/// minimum detectable effect size at `req.sample_size`; otherwise each cell reports the required
/// sample size (`sample_size`, `total_n`, `continuous_n`) at the configured effect size.
pub fn compute_sensitivity_table(req: &PowerAnalysisRequest, alphas: &[f64], target_powers: &[f64]) -> Result<Vec<SensitivityCell>, StatsError> {
    let mut out = Vec::with_capacity(alphas.len().saturating_mul(target_powers.len()));
    let mode = if req.solve_target == PowerSolveTarget::DetectableEffect { PowerSolveTarget::DetectableEffect } else { PowerSolveTarget::SampleSize };
    let mut base_req = req.clone();
    if matches!(req.family, PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoProportions)
        && let (Some(n1_raw), Some(n2_raw)) = (req.sample_size, req.sample_size_group2)
    {
        let is_welch = matches!(&req.effect, PowerEffectSpec::MeanDiffAndSd { sd_group2: Some(_), .. });
        let min_n2 = if is_welch { 1.0 + 1e-6 } else { 1.0 };
        if !n1_raw.is_finite() || n1_raw < 1.0 || n1_raw > MAX_CONTINUOUS_N {
            return Err(StatsError::InvalidParameter { name: "sample_size", value: n1_raw });
        }
        if !n2_raw.is_finite() || n2_raw < min_n2 || n2_raw > MAX_CONTINUOUS_N {
            return Err(StatsError::InvalidParameter { name: "sample_size_group2", value: n2_raw });
        }
        base_req.allocation_ratio = validate_allocation_ratio(n2_raw / n1_raw)?;
        if mode == PowerSolveTarget::SampleSize {
            base_req.sample_size = None;
            base_req.sample_size_group2 = None;
        }
    }

    for &alpha in alphas {
        let _ = validate_prob_open("alpha", alpha)?;
        for &tp in target_powers {
            let _ = validate_prob_open("target_power", tp)?;
            let mut cell_req = base_req.clone();
            cell_req.solve_target = mode;
            cell_req.alpha = alpha;
            cell_req.target_power = Some(tp);
            let solved = solve_power_core(&cell_req)?;
            out.push(SensitivityCell {
                alpha,
                target_power: tp,
                sample_size: solved.n_per_group,
                effect_size: solved.effect_size,
                total_n: solved.total_n,
                continuous_n: solved.continuous_n,
            });
        }
    }
    Ok(out)
}

/// Convert a serialized document [`curvebook_doc::PowerAnalysisSpec`] into a solver
/// [`PowerAnalysisRequest`].
#[must_use]
pub fn power_request_from_doc_spec(spec: &curvebook_doc::PowerAnalysisSpec) -> PowerAnalysisRequest {
    let family = match spec.family {
        curvebook_doc::PowerTestFamily::TwoSampleUnpairedT => PowerTestFamily::TwoSampleUnpairedT,
        curvebook_doc::PowerTestFamily::TwoSamplePairedT => PowerTestFamily::TwoSamplePairedT,
        curvebook_doc::PowerTestFamily::OneSampleT => PowerTestFamily::OneSampleT,
        curvebook_doc::PowerTestFamily::OneWayAnova => PowerTestFamily::OneWayAnova,
        curvebook_doc::PowerTestFamily::TwoProportions => PowerTestFamily::TwoProportions,
        curvebook_doc::PowerTestFamily::Correlation => PowerTestFamily::Correlation,
        curvebook_doc::PowerTestFamily::ChiSquare => PowerTestFamily::ChiSquare,
        curvebook_doc::PowerTestFamily::SimpleLinearRegression => PowerTestFamily::SimpleLinearRegression,
    };
    let solve_target = match spec.solve_target {
        curvebook_doc::PowerSolveTarget::SampleSize => PowerSolveTarget::SampleSize,
        curvebook_doc::PowerSolveTarget::Power => PowerSolveTarget::Power,
        curvebook_doc::PowerSolveTarget::DetectableEffect => PowerSolveTarget::DetectableEffect,
    };
    let tail_mode = match spec.tail_mode {
        curvebook_doc::PowerTailMode::TwoSided => TailMode::TwoSided,
        curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater) => TailMode::OneSided(OneSidedDirection::Greater),
        curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Less) => TailMode::OneSided(OneSidedDirection::Less),
    };
    let two_proportions_mode = match spec.two_proportions_mode {
        curvebook_doc::TwoProportionsMethod::PooledNullUnpooledAlt => TwoProportionsMethod::PooledNullUnpooledAlt,
        curvebook_doc::TwoProportionsMethod::Unpooled => TwoProportionsMethod::Unpooled,
        curvebook_doc::TwoProportionsMethod::CohensH => TwoProportionsMethod::CohensH,
    };
    let correlation_method = match spec.correlation_method {
        curvebook_doc::CorrelationPowerMethod::FisherZ => CorrelationPowerMethod::FisherZ,
        curvebook_doc::CorrelationPowerMethod::FisherZWithTCrit => CorrelationPowerMethod::FisherZWithTCrit,
        curvebook_doc::CorrelationPowerMethod::ExactTTransform => CorrelationPowerMethod::ExactTTransform,
        curvebook_doc::CorrelationPowerMethod::ConditionalSlopeT => CorrelationPowerMethod::ConditionalSlopeT,
    };
    let effect = match &spec.effect {
        curvebook_doc::PowerEffectSpec::Unspecified => PowerEffectSpec::Unspecified,
        curvebook_doc::PowerEffectSpec::Standardized(d) => PowerEffectSpec::Standardized(*d),
        curvebook_doc::PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => PowerEffectSpec::MeanDiffAndSd {
            mean_diff: *mean_diff,
            sd: *sd,
            sd_group2: *sd_group2,
            within_pair_correlation: *within_pair_correlation,
        },
        curvebook_doc::PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => PowerEffectSpec::PairedDiff { mean_diff: *mean_diff, sd_diff: *sd_diff },
        curvebook_doc::PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
            PowerEffectSpec::GroupMeansAndSd { means: means.clone(), within_sd: *within_sd }
        }
        curvebook_doc::PowerEffectSpec::TwoProportions { p1, p2 } => PowerEffectSpec::TwoProportions { p1: *p1, p2: *p2 },
        curvebook_doc::PowerEffectSpec::BaselineProportion { p1 } => PowerEffectSpec::BaselineProportion { p1: *p1 },
        curvebook_doc::PowerEffectSpec::RSquared(r2) => PowerEffectSpec::RSquared(*r2),
        curvebook_doc::PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => {
            PowerEffectSpec::RegressionSlope { beta1: *beta1, sigma_x: *sigma_x, sigma_eps: *sigma_eps }
        }
    };
    PowerAnalysisRequest {
        family,
        solve_target,
        tail_mode,
        alpha: spec.alpha,
        target_power: spec.target_power,
        sample_size: spec.sample_size,
        sample_size_group2: spec.sample_size_group2,
        allocation_ratio: spec.allocation_ratio,
        effect,
        num_groups: spec.num_groups,
        chi_square_df: spec.chi_square_df,
        two_proportions_mode,
        continuity_correction: spec.continuity_correction,
        correlation_method,
    }
}

/// Execute [`curvebook_doc::PowerAnalysisSpec`] and emit a complete [`ResultSheet`] containing
/// summary metrics, sensitivity table, power-curve tab, scalars, checklist, and `analyzable_data`.
pub fn run_power_analysis(_sheet: &DataSheet, spec: &AnalysisSpec, doc_power: &curvebook_doc::PowerAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let req = power_request_from_doc_spec(doc_power);
    let solved = solve_power(&req)?;
    let mut res = init_result_sheet(spec)?;

    let solve_label = match solved.solve_target {
        PowerSolveTarget::SampleSize => "Required sample size",
        PowerSolveTarget::Power => "Achievable power (1 - beta)",
        PowerSolveTarget::DetectableEffect => "Minimum detectable effect",
    };
    let tail_label = match solved.tail_mode {
        TailMode::TwoSided => "Two-sided",
        TailMode::OneSided(OneSidedDirection::Greater) => "One-sided (greater)",
        TailMode::OneSided(OneSidedDirection::Less) => "One-sided (less)",
    };

    let sum_headers = vec![String::from("Estimate")];
    let sum_row_labels = vec![
        String::from("Test family"),
        String::from("Solve target"),
        String::from("Tail mode"),
        String::from("Significance level (alpha)"),
        String::from("Target power (1 - beta)"),
        String::from("Standardized effect size"),
        String::from("Raw effect size"),
        String::from("Sample size (n1 / per group)"),
        String::from("Sample size (group 2)"),
        String::from("Total N"),
        String::from("Achieved power"),
        String::from("Power at continuous N"),
        String::from("Continuous N"),
        String::from("Denominator df"),
        String::from("Critical threshold"),
        String::from("Noncentrality parameter"),
        String::from("Implied proportion p2"),
        String::from("Cohen's h"),
        String::from("Implied R^2"),
    ];
    let sum_rows = vec![
        vec![ResultCell::Text(String::from(solved.family.label()))],
        vec![ResultCell::Text(String::from(solve_label))],
        vec![ResultCell::Text(String::from(tail_label))],
        vec![num_cell(solved.alpha)],
        vec![opt_num_cell(solved.target_power)],
        vec![num_cell(solved.effect_size)],
        vec![opt_num_cell(solved.raw_effect_size)],
        vec![num_cell(solved.n_per_group as f64)],
        vec![opt_num_cell(solved.n_group2.map(|v| v as f64))],
        vec![num_cell(solved.total_n as f64)],
        vec![num_cell(solved.achieved_power)],
        vec![num_cell(solved.power_at_continuous_n)],
        vec![num_cell(solved.continuous_n)],
        vec![opt_num_cell(solved.actual_df)],
        vec![num_cell(solved.critical_value)],
        vec![num_cell(solved.noncentrality_param)],
        vec![opt_num_cell(solved.implied_p2)],
        vec![opt_num_cell(solved.cohens_h)],
        vec![opt_num_cell(solved.implied_r_squared)],
    ];
    res.tabs.push(build_tab("power-summary", "Power & Sample Size — Summary", ResultTabKind::MainSummary, sum_headers, sum_row_labels, sum_rows));

    if !solved.sensitivity_table.is_empty() {
        let sens_headers = vec![
            String::from("Alpha"),
            String::from("Target power"),
            String::from("Effect size"),
            String::from("Sample size (n1)"),
            String::from("Total N"),
        ];
        let mut sens_labels = Vec::with_capacity(solved.sensitivity_table.len());
        let mut sens_rows = Vec::with_capacity(solved.sensitivity_table.len());
        for (idx, cell) in solved.sensitivity_table.iter().enumerate() {
            sens_labels.push(format!("Scenario {}", idx + 1));
            sens_rows.push(vec![
                num_cell(cell.alpha),
                num_cell(cell.target_power),
                num_cell(cell.effect_size),
                num_cell(cell.sample_size as f64),
                num_cell(cell.total_n as f64),
            ]);
        }
        res.tabs.push(build_tab("power-sensitivity", "Sensitivity Matrix", ResultTabKind::MultipleComparisons, sens_headers, sens_labels, sens_rows));
    }

    if !solved.power_curve.is_empty() {
        let curve_headers =
            vec![String::from("Sample size (N)"), String::from("Power (1 - beta)"), String::from("Effect size"), String::from("Total N")];
        let mut curve_labels = Vec::with_capacity(solved.power_curve.len());
        let mut curve_rows = Vec::with_capacity(solved.power_curve.len());
        let mut xy = XyTable::new_with_x_mode(
            XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
            SubcolumnMode::Single,
            1,
            solved.power_curve.len(),
        )?;
        xy.x_title = String::from("Sample Size (N)");
        xy.data_sets[0].title = String::from("Power (1 - beta)");
        for (r, pt) in solved.power_curve.iter().enumerate() {
            curve_labels.push(format!("Point {}", r + 1));
            curve_rows.push(vec![num_cell(pt.n), num_cell(pt.power), num_cell(pt.effect_size), num_cell(pt.total_n)]);
            if r < xy.x_cells.len() {
                xy.x_cells[r][0] = Cell::number(pt.n);
            }
            if r < xy.data_sets[0].cells.len() {
                xy.data_sets[0].cells[r][0] = Cell::number(pt.power);
            }
        }
        res.tabs.push(build_tab("power-curve", "Power Curve", ResultTabKind::CurveCoordinates, curve_headers, curve_labels, curve_rows));
        let data_id = SheetId::try_new(format!("{}-curve-data", spec.id.as_str())).unwrap_or_else(|_| SheetId::new("power-curve-data"));
        let analyzable = DataSheet::new(data_id, "Power Curve Data", TableArchetype::Xy(xy))?;
        res.analyzable_data = Some(analyzable);
    }

    push_scalar(&mut res, "Alpha", None, solved.alpha);
    if let Some(tp) = solved.target_power {
        push_scalar(&mut res, "TargetPower", None, tp);
    }
    push_scalar(&mut res, "EffectSize", None, solved.effect_size);
    if let Some(raw_eff) = solved.raw_effect_size {
        push_scalar(&mut res, "RawEffectSize", None, raw_eff);
    }
    push_scalar(&mut res, "NPerGroup", None, solved.n_per_group as f64);
    if let Some(n2) = solved.n_group2 {
        push_scalar(&mut res, "NGroup2", None, n2 as f64);
    }
    push_scalar(&mut res, "TotalN", None, solved.total_n as f64);
    push_scalar(&mut res, "AchievedPower", None, solved.achieved_power);
    push_scalar(&mut res, "PowerAtContinuousN", None, solved.power_at_continuous_n);
    push_scalar(&mut res, "ContinuousN", None, solved.continuous_n);
    push_scalar(&mut res, "CriticalValue", None, solved.critical_value);
    push_scalar(&mut res, "NoncentralityParam", None, solved.noncentrality_param);
    if let Some(df) = solved.actual_df {
        push_scalar(&mut res, "ActualDf", None, df);
    }
    if let Some(p2) = solved.implied_p2 {
        push_scalar(&mut res, "ImpliedP2", None, p2);
    }
    if let Some(h) = solved.cohens_h {
        push_scalar(&mut res, "CohensH", None, h);
    }
    if let Some(r2) = solved.implied_r_squared {
        push_scalar(&mut res, "ImpliedRSquared", None, r2);
    }

    res.checklist.push(checklist_item(
        "Power & sample-size calculation",
        ChecklistStatus::Pass,
        Some(format!("n1 = {}, total N = {}, power = {:.4}", solved.n_per_group, solved.total_n, solved.achieved_power)),
        solved.notes.join(" "),
    ));
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::{
        CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode,
        TwoProportionsMethod, cohens_d_from_means, cohens_dz_from_correlation, cohens_f_from_means, cohens_f2_from_r_squared, cohens_f2_from_slope,
        cohens_h, compute_power_curve, compute_sensitivity_table, paired_sd_from_correlation, solve_power,
    };

    #[test]
    fn effect_size_conversions_match_analytical_identities() {
        assert!((cohens_d_from_means(12.0, 10.0, 4.0).unwrap() - 0.5).abs() < 1e-14);
        let sd_d = paired_sd_from_correlation(4.0, 0.7).unwrap();
        assert!((sd_d - 4.0 * (2.0 * 0.3_f64).sqrt()).abs() < 1e-14);
        let dz = cohens_dz_from_correlation(2.0, 4.0, 0.7).unwrap();
        assert!((dz - 2.0 / sd_d).abs() < 1e-14);

        let f = cohens_f_from_means(&[10.0, 10.0, 12.0, 12.0], 4.0).unwrap();
        assert!((f - 0.25).abs() < 1e-14);

        let h = cohens_h(0.5, 0.3).unwrap();
        assert!((h - (2.0 * 0.5_f64.sqrt().asin() - 2.0 * 0.3_f64.sqrt().asin())).abs() < 1e-14);

        assert!((cohens_f2_from_r_squared(0.2).unwrap() - 0.25).abs() < 1e-14);
        assert!((cohens_f2_from_slope(0.5, 2.0, 2.0).unwrap() - 0.25).abs() < 1e-14);
    }

    #[test]
    fn family_catalog_and_invalid_inputs_handled_cleanly() {
        assert_eq!(PowerTestFamily::ALL.len(), 8);
        assert!(!PowerTestFamily::OneWayAnova.supports_one_sided());
        assert!(!PowerTestFamily::ChiSquare.supports_one_sided());
        assert!(PowerTestFamily::TwoSampleUnpairedT.supports_one_sided());

        let bad_tail = PowerAnalysisRequest::one_way_anova(3, 0.25, 0.05, 0.80).with_tail(TailMode::ONE_SIDED_GREATER);
        assert!(solve_power(&bad_tail).is_err());

        let bad_alpha = PowerAnalysisRequest::unpaired_t(0.5, 1.2, 0.80);
        assert!(solve_power(&bad_alpha).is_err());

        let zero_eff = PowerAnalysisRequest::unpaired_t(0.0, 0.05, 0.80);
        assert!(solve_power(&zero_eff).is_err());

        let wrong_dir = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_tail(TailMode::OneSided(OneSidedDirection::Less));
        assert!(solve_power(&wrong_dir).is_err());

        let zero_power = PowerAnalysisRequest::unpaired_t(0.0, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(25.0);
        let res_null = solve_power(&zero_power).unwrap();
        assert!((res_null.achieved_power - 0.05).abs() < 1e-12);
    }

    #[test]
    fn power_curve_and_sensitivity_table_work_across_targets() {
        let req = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80);
        let res = solve_power(&req).unwrap();
        assert!(!res.power_curve.is_empty());
        assert_eq!(res.sensitivity_table.len(), 16);

        let curve = compute_power_curve(&req, 5, 100, 20).unwrap();
        assert_eq!(curve.first().unwrap().n, 5.0);
        assert_eq!(curve.last().unwrap().n, 100.0);
        for w in curve.windows(2) {
            assert!(w[1].n > w[0].n);
            assert!(w[1].power > w[0].power);
        }

        let mde_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
            .with_target(PowerSolveTarget::DetectableEffect)
            .with_sample_size(64.0)
            .with_alpha(0.05)
            .with_target_power(0.80);
        let table = compute_sensitivity_table(&mde_req, &[0.05, 0.01], &[0.80, 0.90]).unwrap();
        assert_eq!(table.len(), 4);
        assert!(table[0].effect_size < table[1].effect_size); // higher power needs larger detectable d
        assert!(table[0].effect_size < table[2].effect_size); // stricter alpha needs larger detectable d

        let p2_req = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
            .with_target(PowerSolveTarget::DetectableEffect)
            .with_sample_size(100.0)
            .with_effect(PowerEffectSpec::BaselineProportion { p1: 0.30 })
            .with_two_proportions_mode(TwoProportionsMethod::PooledNullUnpooledAlt);
        let p2_res = solve_power(&p2_req).unwrap();
        assert!(p2_res.implied_p2.is_some());

        let r_pwr = PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::FisherZWithTCrit);
        let r_res = solve_power(&r_pwr).unwrap();
        assert_eq!(r_res.n_per_group, 85);
    }
}
