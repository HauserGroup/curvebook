# Regenerate the Poisson GLM and Cox PH result-sheet reference values in
# `d1_poisson_cox_result_sheets.rs` using R 4.5.2 and `survival` 3.8-3.
#
# Run with:
#   /usr/local/bin/Rscript crates/stats/tests/reference/poisson_cox_result_sheet.R
#
# Case designs follow the UCLA statistical modeling walkthroughs:
# - Poisson: https://stats.oarc.ucla.edu/r/dae/poisson-regression/
#   and R stats::summary.glm: https://search.r-project.org/R/refmans/stats/help/summary.glm.html
# - Cox PH: https://stats.oarc.ucla.edu/wp-content/uploads/2025/02/survival_r_full.html
#   and R survival refman: https://cran.r-project.org/web/packages/survival/refman/survival.html

if (getRversion() != "4.5.2") {
  stop(sprintf("Expected R 4.5.2, found %s", as.character(getRversion())))
}
if (packageVersion("survival") != package_version("3.8-3")) {
  stop(sprintf("Expected survival 3.8-3, found %s", as.character(packageVersion("survival"))))
}

suppressPackageStartupMessages(library(survival))

emit_vec <- function(label, values) {
  cat(label, ": ", paste(sprintf("%.17g", values), collapse = ", "), "\n", sep = "")
}

# ============================================================================
# 1. Poisson GLM (counts with zeros, 3-level factor, non-default reference)
# ============================================================================
# Row 1 begins with "Control", Row 2 with "Focused", Row 3 with "Intensive".
# Setting reference level = "Focused" overrides both EntryOrder and alphabetical
# defaults while retaining ["Control", "Intensive"] as non-reference contrasts.
pois_df <- data.frame(
  events = c(
    0L, 1L, 6L,
    3L, 0L, 2L,
    0L, 4L, 2L,
    3L, 1L, 12L,
    0L, 2L, 7L,
    0L, 7L, 15L,
    3L, 4L, 8L
  ),
  program = factor(
    rep(c("Control", "Focused", "Intensive"), times = 7),
    levels = c("Focused", "Control", "Intensive")
  ),
  score = c(
    0.8, 1.5, 1.2,
    2.4, 0.9, 2.1,
    1.6, 2.8, 1.9,
    3.2, 3.6, 3.4,
    2.9, 1.4, 3.1,
    4.1, 4.3, 4.5,
    3.7, 4.8, 4.0
  )
)

pois_fit <- stats::glm(
  events ~ program + score,
  data = pois_df,
  family = stats::poisson(link = "log"),
  control = stats::glm.control(epsilon = 1e-14, maxit = 100)
)
pois_sum <- summary(pois_fit, dispersion = 1)
pois_coef <- pois_sum$coefficients
pois_rr_ci <- exp(stats::confint.default(pois_fit, level = 0.95))

cat("=== POISSON GLM ===\n")
cat("terms:", paste(rownames(pois_coef), collapse = ", "), "\n")
emit_vec("beta", pois_coef[, "Estimate"])
emit_vec("se (dispersion=1)", pois_coef[, "Std. Error"])
emit_vec("wald_z", pois_coef[, "z value"])
emit_vec("p_value", pois_coef[, "Pr(>|z|)"])
emit_vec("rate_ratio", exp(stats::coef(pois_fit)))
emit_vec("rr_ci_lo", pois_rr_ci[, 1L])
emit_vec("rr_ci_hi", pois_rr_ci[, 2L])
emit_vec("residual_deviance", pois_fit$deviance)
emit_vec("pearson_dispersion", sum(stats::residuals(pois_fit, type = "pearson")^2) / pois_fit$df.residual)
emit_vec("aic", stats::AIC(pois_fit))
emit_vec("fitted_counts", stats::fitted(pois_fit))
emit_vec("pearson_residuals", stats::residuals(pois_fit, type = "pearson"))
emit_vec("deviance_residuals", stats::residuals(pois_fit, type = "deviance"))

# ============================================================================
# 2. Cox PH (tied events, right censoring, factor + covariate, 1 missing row)
# ============================================================================
# Rows are stored in unsorted follow-up order; Row 8 (status = 1) has missing `biomarker`
# so failing listwise deletion changes both sample size and event count.
# Retained rows have tied events at t = 5 (2 events), t = 10 (2 events + 1 censored),
# and t = 15 (2 events). Explicit non-default reference level = "ArmA".
cox_df <- data.frame(
  time = c(
    15.0,  5.0, 22.0,  7.0, 10.0,
     3.0, 10.0, 11.0,  5.0, 28.0,
    12.0, 14.0, 10.0, 24.0, 15.0,
    30.0,  8.0, 17.0, 26.0
  ),
  status = c(
    1L, 1L, 1L, 0L, 1L,
    1L, 0L, 1L, 1L, 1L,
    1L, 0L, 1L, 1L, 1L,
    0L, 1L, 1L, 0L
  ),
  regimen = factor(
    c(
      "Control", "ArmA", "ArmB", "Control", "ArmB",
      "Control", "ArmA", "ArmA", "ArmB", "ArmB",
      "ArmB", "Control", "Control", "Control", "ArmA",
      "Control", "ArmA", "Control", "ArmA"
    ),
    levels = c("ArmA", "Control", "ArmB")
  ),
  biomarker = c(
    3.6, 2.8, 1.9, 2.2, 4.4,
    4.1, 3.3, NA_real_, 3.9, 1.4,
    3.1, 1.8, 2.9, 2.5, 2.7,
    1.6, 3.5, 2.1, 2.0
  )
)

cat("\n=== COX PH ===\n")
for (tie_method in c("efron", "breslow", "exact")) {
  fit <- survival::coxph(
    survival::Surv(time, status) ~ regimen + biomarker,
    data = cox_df,
    ties = tie_method
  )
  s <- summary(fit)
  cat(sprintf("--- ties = %s (n = %d, omitted = %d [row %s], events = %d) ---\n",
              tie_method, fit$n, length(fit$na.action),
              paste(as.integer(fit$na.action), collapse = ","), fit$nevent))
  cat("terms:", paste(rownames(s$coefficients), collapse = ", "), "\n")
  emit_vec("beta", s$coefficients[, "coef"])
  emit_vec("se", s$coefficients[, "se(coef)"])
  emit_vec("wald_z", s$coefficients[, "z"])
  emit_vec("p_value", s$coefficients[, "Pr(>|z|)"])
  emit_vec("hazard_ratio", s$coefficients[, "exp(coef)"])
  emit_vec("hr_ci_lo", s$conf.int[, "lower .95"])
  emit_vec("hr_ci_hi", s$conf.int[, "upper .95"])
  emit_vec("lr_chi2", s$logtest[["test"]])
  emit_vec("lr_p", s$logtest[["pvalue"]])
}

# Efron baseline cumulative hazard H_0(t) and survival S_0(t) at retained design-column means.
# Notice survival::coxph sets binary/factor means to 0 by default unless numeric design columns
# are explicitly evaluated at colMeans(model.matrix(fit_efron)) via newdata.
fit_efron <- survival::coxph(
  survival::Surv(time, status) ~ regimen + biomarker,
  data = cox_df,
  ties = "efron"
)
cc <- cox_df[stats::complete.cases(cox_df), ]
mm <- stats::model.matrix(fit_efron)
design_means <- colMeans(mm)
mm_df <- data.frame(
  time = cc$time,
  status = cc$status,
  x1 = mm[, 1L],
  x2 = mm[, 2L],
  x3 = mm[, 3L]
)
fit_design <- survival::coxph(
  survival::Surv(time, status) ~ x1 + x2 + x3,
  data = mm_df,
  ties = "efron"
)
if (max(abs(stats::coef(fit_efron) - stats::coef(fit_design))) > 1e-14) {
  stop("Design-matrix refit deviated from factor coxph fit")
}
means_df <- data.frame(
  x1 = unname(design_means[1L]),
  x2 = unname(design_means[2L]),
  x3 = unname(design_means[3L])
)
sf_efron <- survival::survfit(fit_design, newdata = means_df)
event_idx <- sf_efron$n.event > 0

cat("\n--- Efron baseline at retained design-column means ---\n")
emit_vec("design_means", unname(design_means))
emit_vec("event_times", sf_efron$time[event_idx])
emit_vec("baseline_cumhaz", sf_efron$cumhaz[event_idx])
emit_vec("baseline_surv", sf_efron$surv[event_idx])
