//! One-way, two-way, three-way, N-way factorial ANOVA, repeated-measures sphericity corrections
//! (Mauchly, Greenhouse–Geisser, Huynh–Feldt), mixed-effects REML (Satterthwaite & Kenward–Roger
//! degrees of freedom), nested hierarchical ANOVA / nested *t*-test, and effect sizes with exact
//! noncentral-*F* confidence intervals.
//!
//! Classical fixed-effects ANOVA partitions (one-way, factorial, repeated-measures, nested) follow
//! the textbook treatments in Zar (2010), Maxwell & Delaney (2004), and Searle (1987); ordinary
//! one-way results are checked against the NIST StRD ANOVA datasets.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - NIST Statistical Reference Datasets (StRD), Analysis of Variance:
//!   <https://www.itl.nist.gov/div898/strd/anova/anova.html>
//! - Bartlett, M. S. (1937). Properties of sufficiency and statistical tests. *Proceedings of the
//!   Royal Society of London A* 160(901):268–282. doi:10.1098/rspa.1937.0109
//! - Brown, M. B., & Forsythe, A. B. (1974a). Robust tests for the equality of variances. *Journal
//!   of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955
//! - Brown, M. B., & Forsythe, A. B. (1974b). The small sample behavior of some statistics which
//!   test the equality of several means. *Technometrics* 16(1):129–132.
//! - Welch, B. L. (1951). On the comparison of several mean values: an alternative approach.
//!   *Biometrika* 38(3/4):330–336.
//! - Mauchly, J. W. (1940). Significance test for sphericity of a normal *n*-variate distribution.
//!   *Annals of Mathematical Statistics* 11(2):204–209. doi:10.1214/aoms/1177731915
//! - Greenhouse, S. W., & Geisser, S. (1959). On methods in the analysis of profile data.
//!   *Psychometrika* 24(2):95–112. doi:10.1007/BF02289823
//! - Huynh, H., & Feldt, L. S. (1976). Estimation of the Box correction for degrees of freedom from
//!   sample data in randomized block and split-plot designs. *Journal of Educational Statistics*
//!   1(1):69–82. doi:10.3102/10769986001001069
//! - Lecoutre, B. (1991). A correction for the epsilon-tilde approximate test in repeated measures
//!   designs with two or more independent groups. *Journal of Educational Statistics*
//!   16(4):371–372. doi:10.3102/10769986016004371
//! - Copenhaver, M. D., & Holland, B. S. (1988). Computation of the distribution of the maximum
//!   studentized range statistic with application to multiple significance testing of simple
//!   effects. *Journal of Statistical Computation and Simulation* 30(1):1–15.
//! - Kruskal, W. H., & Wallis, W. A. (1952). Use of ranks in one-criterion variance analysis.
//!   *Journal of the American Statistical Association* 47(260):583–621.
//!   doi:10.1080/01621459.1952.10483441
//! - Friedman, M. (1937). The use of ranks to avoid the assumption of normality implicit in the
//!   analysis of variance. *Journal of the American Statistical Association* 32(200):675–701.
//!   doi:10.1080/01621459.1937.10503522
//! - Kendall, M. G., & Babington Smith, B. (1939). The problem of *m* rankings. *Annals of
//!   Mathematical Statistics* 10(3):275–287.
//! - Steiger, J. H. (2004). Beyond the F test: effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods* 9(2):164–182.
//!   doi:10.1037/1082-989X.9.2.164
//! - Olejnik, S., & Algina, J. (2003). Generalized eta and omega squared statistics: measures of
//!   effect size for some common research designs. *Psychological Methods* 8(4):434–447.
//!   doi:10.1037/1082-989X.8.4.434
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//!   Lawrence Erlbaum.
//! - Patterson, H. D., & Thompson, R. (1971). Recovery of inter-block information when block sizes
//!   are unequal. *Biometrika* 58(3):545–554. doi:10.1093/biomet/58.3.545
//! - Self, S. G., & Liang, K.-Y. (1987). Asymptotic properties of maximum likelihood estimators and
//!   likelihood ratio tests under nonstandard conditions. *Journal of the American Statistical
//!   Association* 82(398):605–610. doi:10.1080/01621459.1987.10478472
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Giesbrecht, F. G., & Burns, J. C. (1985). Two-stage analysis based on a mixed model:
//!   large-sample asymptotic theory and small-sample simulation results. *Biometrics*
//!   41(2):477–486. doi:10.2307/2530872
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558
//! - Fai, A. H.-T., & Cornelius, P. L. (1996). Approximate F-tests of multiple degree of freedom
//!   hypotheses in generalized least squares analyses of unbalanced split-plot experiments.
//!   *Journal of Statistical Computation and Simulation* 54(4):363–378. doi:10.1080/00949659608811740
//! - Gilmour, A. R., Thompson, R., & Cullis, B. R. (1995). Average Information REML: an efficient
//!   algorithm for variance parameter estimation in linear mixed models. *Biometrics*
//!   51(4):1440–1450. doi:10.2307/2533274
//! - Pinheiro, J. C., & Bates, D. M. (1996). Unconstrained parametrizations for variance-covariance
//!   matrices. *Statistics and Computing* 6(3):289–296. doi:10.1007/BF00140873
//! - Lindstrom, M. J., & Bates, D. M. (1988). Newton-Raphson and EM algorithms for linear
//!   mixed-effects models for repeated-measures data. *Journal of the American Statistical
//!   Association* 83(404):1014–1022. doi:10.1080/01621459.1988.10478693
//! - Verbeke, G., & Molenberghs, G. (2000). *Linear Mixed Models for Longitudinal Data*. Springer.
//! - Tomczak, M., & Tomczak, E. (2014). The need to report effect size estimates revisited. An
//!   overview of some recommended measures of effect size. *Trends in Sport Sciences* 1(21):19–25.
//!
//! Module layout: shared result types (`results`), effect sizes and variance-homogeneity tests
//! (`effect_sizes`), one-way and rank-based tests (`one_way`, `nonparametric`), the factorial engine
//! (`factorial`), repeated-measures two-way ANOVA (`rm_two_way`), mixed-model REML (`reml`,
//! `reml_unstructured`), nested designs (`nested`), post-hoc contrasts (`posthoc`), and the
//! `ResultSheet` builders (`sheet`). Every public item is re-exported here.

mod effect_sizes;
mod factorial;
mod nested;
mod nonparametric;
mod one_way;
mod posthoc;
mod reml;
mod reml_unstructured;
mod results;
mod rm_two_way;
mod sheet;

pub use effect_sizes::{compute_anova_effect_sizes, compute_anova_effect_sizes_for_n, variance_homogeneity_tests};
pub use factorial::{FactorialObservations, fit_factorial_anova};
pub use nested::{nested_one_way_anova, nested_one_way_anova_with_df_method};
pub use nonparametric::{friedman_test, kruskal_wallis_test};
pub use one_way::{ordinary_one_way_anova, ordinary_one_way_from_summary, rm_one_way_anova, unequal_variance_one_way_anova};
pub use reml::{
    fit_multi_vc_reml, fit_multi_vc_reml_with_df_method, fit_multi_vc_reml_with_policy, fit_random_intercept_reml,
    fit_random_intercept_reml_with_df_method, fit_random_intercept_reml_with_policy,
};
pub use reml_unstructured::{fit_unstructured_rm_reml, fit_unstructured_rm_reml_with_df_method};
pub use results::{
    AnovaEffectSizes, FactorialAnovaResult, FactorialAnovaTerm, FriedmanResult, KruskalWallisResult, MixedRemlResult, MultiVcRemlResult,
    NestedAnovaResult, OneWayAnovaResult, RmOneWayAnovaResult, RmSphericityCorrection, RmThreeWayAnovaResult, RmTwoWayAnovaResult,
    RmTwoWaySphericities, SphericityResult, UnequalVarianceAnovaResult, UnstructuredFallbackReason, UnstructuredRemlResult,
    VarianceHomogeneityResult,
};
pub use rm_two_way::{fit_rm_three_way_anova, fit_rm_two_way_anova, fit_rm_two_way_anova_with_sphericity};
pub use sheet::factorial::run_factorial_anova;
pub use sheet::nested::run_nested_analysis;
pub use sheet::one_way::run_one_way_anova;
