//! Contingency table, categorical proportion, agreement, and goodness-of-fit analyses.
//!
//! Includes Pearson chi-square (with Yates continuity option), likelihood-ratio `G^2`,
//! Fisher's exact test (`2x2` + `RxC` Freeman–Halton / deterministic Monte Carlo), Cochran–Armitage
//! trend test, McNemar paired test, Wilson & Clopper–Pearson binomial CIs, Baptista–Pike OR CI,
//! Koopman score RR CI, Newcombe risk-difference CI, Cohen's kappa (unweighted/weighted),
//! Fleiss' kappa, and phi / Cramér's `V` with exact noncentral chi-square CIs.
//!
//! References (D1 citation pass 2026-11-18; helpers cite their own sources inline):
//! - Pearson, K. (1900). On the criterion that a given system of deviations from the probable in the
//!   case of a correlated system of variables is such that it can be reasonably supposed to have
//!   arisen from random sampling. *Philosophical Magazine*, 50(302), 157-175.
//!   `doi:10.1080/14786440009463897`.
//! - Yates, F. (1934). Contingency tables involving small numbers and the chi-squared test.
//!   *Supplement to the Journal of the Royal Statistical Society*, 1(2), 217-235. `doi:10.2307/2983604`.
//! - Wilks, S. S. (1938). The large-sample distribution of the likelihood ratio for testing composite
//!   hypotheses. *Annals of Mathematical Statistics*, 9(1), 60-62. `doi:10.1214/aoms/1177732360`.
//! - Cochran, W. G. (1954). Some methods for strengthening the common chi-squared tests.
//!   *Biometrics*, 10(4), 417-451. `doi:10.2307/3001616`.
//! - Armitage, P. (1955). Tests for linear trends in proportions and frequencies. *Biometrics*,
//!   11(3), 375-386. `doi:10.2307/3001775`.
//! - McNemar, Q. (1947). Note on the sampling error of the difference between correlated proportions
//!   or percentages. *Psychometrika*, 12(2), 153-157. `doi:10.1007/bf02295996`.
//! - Edwards, A. L. (1948). Note on the "correction for continuity" in testing the significance of the
//!   difference between correlated proportions. *Psychometrika*, 13(3), 185-187.
//!   `doi:10.1007/bf02289261`.
//! - Sterne, T. E. (1954). Some remarks on confidence or fiducial limits. *Biometrika*, 41(1-2),
//!   275-278. `doi:10.1093/biomet/41.1-2.275`.
//! - Cohen, J. (1960). A coefficient of agreement for nominal scales. *Educational and Psychological
//!   Measurement*, 20(1), 37-46. `doi:10.1177/001316446002000104`.
//! - Cohen, J. (1968). Weighted kappa: Nominal scale agreement with provision for scaled disagreement
//!   or partial credit. *Psychological Bulletin*, 70(4), 213-220. `doi:10.1037/h0026256`.
//! - Fleiss, J. L., Cohen, J., & Everitt, B. S. (1969). Large sample standard errors of kappa and
//!   weighted kappa. *Psychological Bulletin*, 72(5), 323-327. `doi:10.1037/h0028106`.
//! - Fleiss, J. L. (1971). Measuring nominal scale agreement among many raters. *Psychological
//!   Bulletin*, 76(5), 378-382. `doi:10.1037/h0031619`.
//! - Fleiss, J. L., Nee, J. C. M., & Landis, J. R. (1979). Large sample variance of kappa in the case
//!   of different sets of raters. *Psychological Bulletin*, 86(5), 974-977.
//!   `doi:10.1037/0033-2909.86.5.974`.
//! - Fleiss, J. L., Levin, B., & Paik, M. C. (2003). *Statistical Methods for Rates and Proportions*
//!   (3rd ed.). Wiley. (Non-null asymptotic standard errors for Fleiss's multi-rater kappa, ch. 18).
//! - Gwet, K. L. (2008). Computing inter-rater reliability and its variance in the presence of high
//!   agreement. *British Journal of Mathematical and Statistical Psychology*, 61(1), 29-48.
//!   `doi:10.1348/000711006X126600`.
//!
//! Module layout: typed result/ordering structures (`types`), exact binomial tests and proportion
//! CIs (`binomial`), contingency/Fisher/Barnard/McNemar/trend tests (`contingency`), `2 x 2`
//! effect ratios and NNT (`ratios_nnt`), inter-rater kappa agreement (`kappa`), and `ResultSheet`
//! builders (`sheet`). Every public item is re-exported here.

mod binomial;
mod contingency;
mod kappa;
mod ratios_nnt;
mod sheet;
mod types;

pub use binomial::{agresti_coull_ci, clopper_pearson_ci, exact_binomial_test, wilson_score_ci};
pub use contingency::{
    analyze_contingency, barnard_exact_2x2, barnard_exact_2x2_with_ordering, cochran_armitage_trend, fisher_exact_2x2, fisher_exact_rxc,
    mcnemar_detailed, mcnemar_test,
};
pub use curvebook_doc::KappaWeighting;
pub use kappa::{cohens_kappa, fleiss_kappa};
pub use ratios_nnt::{altman_nnt_from_rd, analyze_2x2_ratios, baptista_pike_or_ci, koopman_rr_ci};
pub use sheet::run_contingency_analysis;
pub use types::{
    BarnardExactResult, BarnardOrdering, ContingencyResult, ExactBinomialResult, KappaResult, McNemarResult, NntKind, TwoByTwoRatiosResult,
};
