//! Trust-region Levenberg–Marquardt nonlinear regression engine, global sharing, constraints,
//! Nelder–Mead simplex warm-start, weighting, Poisson deviance IRLS, Cauchy robust IRLS,
//! ROUT outlier elimination, curve bands, and Brent inverse unknown interpolation.
//!
//! # Methods and references
//!
//! - Damped least squares: Levenberg (1944) and Marquardt (1963); the trust-region form with
//!   adaptive column scaling, step-bound update, and `ftol`/`xtol`/`gtol` tests follows
//!   Moré (1978) and the MINPACK-1 implementation (Moré, Garbow & Hillstrom 1980). See
//!   `run_trust_region_lm` for the documented deviations.
//! - Simplex rescue: Nelder & Mead (1965) with the outside/inside contraction rules of
//!   Lagarias et al. (1998).
//! - Finite-difference Jacobian step sizes (`~eps^(1/3)` central, `~eps^(1/2)` forward):
//!   Nocedal & Wright (2006), Ch. 8.
//! - Weighting, asymptotic standard errors, Wald and profile intervals, confidence/prediction
//!   bands, and model comparison: Bates & Watts (1988), Seber & Wild (1989), and
//!   Motulsky & Christopoulos (2004).
//! - Poisson regression by iteratively reweighted least squares: Green (1984);
//!   McCullagh & Nelder (1989).
//! - Robust (Cauchy / Lorentzian) fitting and ROUT outlier removal: Motulsky & Brown (2006),
//!   solved here by IRLS (Holland & Welsch 1977).
//! - Root finding for interpolation: Brent (1973), Ch. 4.
//!
//! Reference: Levenberg, K. (1944). A method for the solution of certain non-linear problems in least squares. *Quarterly of Applied Mathematics* 2(2), 164–168. doi:10.1090/qam/10666
//!
//! Reference: Marquardt, D. W. (1963). An algorithm for least-squares estimation of nonlinear parameters. *Journal of the Society for Industrial and Applied Mathematics* 11(2), 431–441. doi:10.1137/0111030
//!
//! Reference: Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory. In G. A. Watson (ed.), *Numerical Analysis*, Lecture Notes in Mathematics 630, 105–116. Springer. doi:10.1007/BFb0067700
//!
//! Reference: Moré, J. J., Garbow, B. S. & Hillstrom, K. E. (1980). *User Guide for MINPACK-1*. Report ANL-80-74, Argonne National Laboratory.
//!
//! Reference: Nelder, J. A. & Mead, R. (1965). A simplex method for function minimization. *The Computer Journal* 7(4), 308–313. doi:10.1093/comjnl/7.4.308
//!
//! Reference: Lagarias, J. C., Reeds, J. A., Wright, M. H. & Wright, P. E. (1998). Convergence properties of the Nelder–Mead simplex method in low dimensions. *SIAM Journal on Optimization* 9(1), 112–147. doi:10.1137/S1052623496303470
//!
//! Reference: Nocedal, J. & Wright, S. J. (2006). *Numerical Optimization*, 2nd ed. Springer.
//!
//! Reference: Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//!
//! Reference: Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//!
//! Reference: Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//!
//! Reference: Green, P. J. (1984). Iteratively reweighted least squares for maximum likelihood estimation, and some robust and resistant alternatives. *Journal of the Royal Statistical Society, Series B* 46(2), 149–192.
//!
//! Reference: McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//!
//! Reference: Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
//!
//! Reference: Holland, P. W. & Welsch, R. E. (1977). Robust regression using iteratively reweighted least-squares. *Communications in Statistics – Theory and Methods* 6(9), 813–827. doi:10.1080/03610927708827533
//!
//! Reference: Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.
//!
//! Module layout: configuration and result types (`results`), parameter layout and observations
//! (`layout`), the Levenberg–Marquardt core (`lm`), inference and diagnostics (`inference`), the
//! `fit_nonlinear` entry point (`fit`), global and per-dataset fits (`datasets`), result assembly
//! (`assembly`), model comparison (`comparison`), IRLS fitting (`irls`), profile intervals
//! (`profile`), curve bands and interpolation (`curves`), and identifiability checks
//! (`identifiability`). Every public item is re-exported here.

mod assembly;
mod comparison;
mod curves;
mod datasets;
mod fit;
mod identifiability;
mod inference;
mod irls;
mod layout;
mod lm;
mod profile;
mod results;

pub use comparison::{
    HorizontalLineClassification, HorizontalLineComparisonReport, HorizontalLineDatasetDiagnostic, HorizontalLineModel,
    ModelComparisonQuestionResult, SynchronizedComparisonRequest, compare_models, compare_models_checked, compare_models_question,
    compare_models_synchronized, compare_vs_horizontal_line, ordered_nested_roles,
};
pub use fit::fit_nonlinear;
pub use identifiability::{
    FitFlagOutcome, GreedyHessianReduction, assemble_objective_hessian, evaluate_fit_flags, greedy_hessian_reduction, hessian_condition_number,
    hessian_kappa_threshold, is_fit_well_defined,
};
pub use results::{
    FittedDatasetResult, FittedParameterSummary, InferenceAvailability, InterpolatedUnknown, ModelComparisonOutcome, ModelComparisonResult,
    NonlinearFitResult, PoissonNonlinearGof, SolverConfig,
};
