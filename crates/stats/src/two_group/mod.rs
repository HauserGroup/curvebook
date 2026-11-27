//! Two-group parametric and nonparametric hypothesis tests, standardized effect sizes with
//! exact noncentral-*t* confidence intervals, variance homogeneity tests, and Bland–Altman agreement.
//!
//! # References
//!
//! Full bibliographic details for the methods implemented in this module; individual items cite
//! them in short `Author (Year)` form.
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 7 (one-sample),
//!   ch. 8 (two-sample, including the variance-ratio *F* test), ch. 9 (paired-sample hypotheses).
//! - Levene, H. (1960). Robust tests for equality of variances. In I. Olkin et al. (Eds.),
//!   *Contributions to Probability and Statistics: Essays in Honor of Harold Hotelling*, pp. 278–292.
//!   Stanford University Press.
//! - Brown, M. B., & Forsythe, A. B. (1974). Robust tests for the equality of variances.
//!   *Journal of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.). Lawrence Erlbaum.
//! - Hedges, L. V. (1981). Distribution theory for Glass's estimator of effect size and related
//!   estimators. *Journal of Educational Statistics* 6(2):107–128. doi:10.3102/10769986006002107
//! - Glass, G. V. (1976). Primary, secondary, and meta-analysis of research. *Educational Researcher*
//!   5(10):3–8. doi:10.3102/0013189X005010003
//! - Steiger, J. H., & Fouladi, R. T. (1997). Noncentrality interval estimation and the evaluation of
//!   statistical models. In L. L. Harlow, S. A. Mulaik, & J. H. Steiger (Eds.), *What If There Were No
//!   Significance Tests?*, pp. 221–257. Lawrence Erlbaum.
//! - Steiger, J. H. (2004). Beyond the F test: Effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods* 9(2):164–182.
//!   doi:10.1037/1082-989X.9.2.164
//! - Cumming, G., & Finch, S. (2001). A primer on the understanding, use, and calculation of confidence
//!   intervals that are based on central and noncentral distributions. *Educational and Psychological
//!   Measurement* 61(4):532–574.
//! - Rosenthal, R. (1991). *Meta-Analytic Procedures for Social Research* (rev. ed.). Sage.
//! - Bland, J. M., & Altman, D. G. (1996). Statistics notes: The use of transformation when comparing
//!   two means. *BMJ* 312(7039):1153. doi:10.1136/bmj.312.7039.1153
//! - Wilcoxon, F. (1945). Individual comparisons by ranking methods. *Biometrics Bulletin* 1(6):80–83.
//!   doi:10.2307/3001968
//! - Mann, H. B., & Whitney, D. R. (1947). On a test of whether one of two random variables is
//!   stochastically larger than the other. *Annals of Mathematical Statistics* 18(1):50–60.
//!   doi:10.1214/aoms/1177730491
//! - Pratt, J. W. (1959). Remarks on zeros and ties in the Wilcoxon signed rank procedures.
//!   *Journal of the American Statistical Association* 54(287):655–667. doi:10.1080/01621459.1959.10501526
//! - Hodges, J. L., & Lehmann, E. L. (1963). Estimates of location based on rank tests.
//!   *Annals of Mathematical Statistics* 34(2):598–611. doi:10.1214/aoms/1177704172
//! - Monahan, J. F. (1984). Algorithm 616: Fast computation of the Hodges–Lehmann location estimator.
//!   *ACM Transactions on Mathematical Software* 10(3):265–270. doi:10.1145/1271.319414
//! - Hollander, M., Wolfe, D. A., & Chicken, E. (2014). *Nonparametric Statistical Methods* (3rd ed.).
//!   Wiley. Ch. 3 (paired/one-sample location: signed-rank test, Walsh-average estimator), ch. 4
//!   (two-sample location: rank-sum test, Hodges–Lehmann shift estimator, Moses interval).
//! - Conover, W. J. (1999). *Practical Nonparametric Statistics* (3rd ed.). Wiley.
//! - Dixon, W. J., & Mood, A. M. (1946). The statistical sign test. *Journal of the American
//!   Statistical Association* 41:557–566.
//! - Cliff, N. (1993). Dominance statistics: Ordinal analyses to answer ordinal questions.
//!   *Psychological Bulletin* 114(3):494–509. doi:10.1037/0033-2909.114.3.494
//! - Wendt, H. W. (1972). Dealing with a common problem in social science: A simplified rank-biserial
//!   coefficient of correlation based on the U statistic. *European Journal of Social Psychology* 2.
//!   doi:10.1002/ejsp.2420020412
//! - Kerby, D. S. (2014). The simple difference formula: An approach to teaching nonparametric
//!   correlation. *Comprehensive Psychology* 3. doi:10.2466/11.IT.3.1
//! - Smirnov, N. (1948). Table for estimating the goodness of fit of empirical distributions.
//!   *Annals of Mathematical Statistics* 19(2):279–281. doi:10.1214/aoms/1177730256
//! - Hodges, J. L. (1958). The significance probability of the Smirnov two-sample test.
//!   *Arkiv för Matematik* 3(5):469–486. doi:10.1007/BF02589501
//! - Bland, J. M., & Altman, D. G. (1986). Statistical methods for assessing agreement between two
//!   methods of clinical measurement. *Lancet* 327(8476):307–310. doi:10.1016/S0140-6736(86)90837-8
//! - Bland, J. M., & Altman, D. G. (1999). Measuring agreement in method comparison studies.
//!   *Statistical Methods in Medical Research* 8(2):135–160. doi:10.1177/096228029900800204
//!
//! Module layout: variance-homogeneity tests (`variance`), parametric *t*-tests and standardized
//! effect sizes (`parametric`), nonparametric rank/sign/KS tests (`nonparametric`), Bland–Altman
//! method comparison (`bland_altman`), row-wise multiple testing (`multiple_row`), and the
//! `ResultSheet` builders (`sheet`). Every public item is re-exported here.

mod bland_altman;
mod multiple_row;
mod nonparametric;
mod parametric;
mod sheet;
mod variance;

#[cfg(test)]
mod tests;

pub use crate::lognormal::{OneSampleRatioTResult, lognormal_welch_t_test, one_sample_ratio_t_test};
pub use bland_altman::{BlandAltmanResult, bland_altman};
pub use multiple_row::{MultipleRowTestEntry, multiple_row_t_tests};
pub use nonparametric::{
    MannWhitneyResult, PairedSignTestResult, TwoSampleKsResult, WilcoxonMatchedResult, ks_two_sample, mann_whitney_u, paired_sign_test,
    wilcoxon_matched_pairs,
};
pub use parametric::{
    PairedTResult, TwoGroupEffectSizes, UnpairedTResult, lognormal_unpaired_t_test, one_sample_t_test_from_summary, paired_t_test,
    ratio_paired_t_test, unpaired_t_test, unpaired_t_test_from_summary, welch_t_test, welch_t_test_from_summary,
};
pub use sheet::{run_bland_altman_analysis, run_multiple_row_t_tests_analysis, run_two_group_analysis};
pub(crate) use variance::levene_variance_core;
pub use variance::{LeveneTestResult, VarianceFTestResult, f_test_variances, levene_variance_test};
