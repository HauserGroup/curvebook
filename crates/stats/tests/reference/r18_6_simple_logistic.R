# Independent D1 oracle script for R18.6 Standalone XY Simple Logistic Regression.
# Verified with R 4.5.2 (stats::glm, stats::confint via MASS::confint.glm, stats::predict.glm).
#
# Run: Rscript crates/stats/tests/reference/r18_6_simple_logistic.R

options(digits = 15)

profile_x50_ci <- function(x, y, fit, level = 0.95) {
  b <- coef(fit)
  b0 <- unname(b[1])
  b1 <- unname(b[2])
  x50_hat <- -b0 / b1
  ll_hat <- as.numeric(logLik(fit))
  ll_null <- as.numeric(logLik(glm(y ~ 1, family = binomial())))
  crit <- ll_hat - 0.5 * qchisq(level, df = 1)
  if (ll_null >= crit) {
    return(c(lower = NA_real_, upper = NA_real_))
  }
  prof_ll <- function(x0) {
    obj <- function(slope) {
      eta <- slope * (x - x0)
      sum(ifelse(eta >= 0, (y - 1) * eta - log1p(exp(-eta)), y * eta - log1p(exp(eta))))
    }
    opt <- optimize(obj, interval = c(b1 - 50 * abs(b1) - 10, b1 + 50 * abs(b1) + 10), maximum = TRUE, tol = 1e-13)
    opt$objective - crit
  }
  lo_hi <- x50_hat
  step <- max(abs(x50_hat) * 0.25, sd(x) * 0.5, 0.5)
  lo_bound <- x50_hat - step
  while (prof_ll(lo_bound) > 0) {
    step <- step * 2
    lo_bound <- x50_hat - step
  }
  lower <- uniroot(prof_ll, c(lo_bound, lo_hi), tol = 1e-12)$root

  step <- max(abs(x50_hat) * 0.25, sd(x) * 0.5, 0.5)
  hi_bound <- x50_hat + step
  while (prof_ll(hi_bound) > 0) {
    step <- step * 2
    hi_bound <- x50_hat + step
  }
  upper <- uniroot(prof_ll, c(lo_hi, hi_bound), tol = 1e-12)$root
  c(lower = lower, upper = upper)
}

run_case <- function(label, x, y, x_new = numeric(0)) {
  cat("========================================\n")
  cat("CASE:", label, "\n")
  fit <- glm(y ~ x, family = binomial(link = "logit"), control = glm.control(epsilon = 1e-14, maxit = 100))
  s <- summary(fit)
  ci <- suppressMessages(confint(fit, level = 0.95))
  b0 <- unname(coef(fit)[1])
  b1 <- unname(coef(fit)[2])
  se0 <- unname(s$coefficients[1, 2])
  se1 <- unname(s$coefficients[2, 2])
  z0 <- unname(s$coefficients[1, 3])
  p0 <- unname(s$coefficients[1, 4])
  z1 <- unname(s$coefficients[2, 3])
  p1 <- unname(s$coefficients[2, 4])
  d0 <- fit$null.deviance
  d1 <- fit$deviance
  g2 <- d0 - d1
  lr_p <- pchisq(g2, df = 1, lower.tail = FALSE)
  n <- length(y)
  k <- 2
  aic <- fit$aic
  aicc <- aic + (2 * k * (k + 1)) / (n - k - 1)
  p_hat <- fitted(fit)
  tjur <- mean(p_hat[y == 1]) - mean(p_hat[y == 0])
  cox_snell <- 1 - exp(-g2 / n)
  x50 <- -b0 / b1
  x50_ci <- profile_x50_ci(x, y, fit, 0.95)

  # Exact empirical ROC AUC (Mann-Whitney concordance)
  p1_vals <- p_hat[y == 1]
  p0_vals <- p_hat[y == 0]
  conc <- outer(p1_vals, p0_vals, function(a, b) ifelse(a > b, 1, ifelse(a == b, 0.5, 0)))
  auc <- mean(conc)

  cat(sprintf("beta0 = %.15f, se0 = %.15f, z0 = %.15f, p0 = %.15e\n", b0, se0, z0, p0))
  cat(sprintf("beta1 = %.15f, se1 = %.15f, z1 = %.15f, p1 = %.15e\n", b1, se1, z1, p1))
  cat(sprintf("ci_beta0 = (%.15f, %.15f)\n", ci[1, 1], ci[1, 2]))
  cat(sprintf("ci_beta1 = (%.15f, %.15f)\n", ci[2, 1], ci[2, 2]))
  cat(sprintf("or0 = %.15f, ci_or0 = (%.15f, %.15f)\n", exp(b0), exp(ci[1, 1]), exp(ci[1, 2])))
  cat(sprintf("or1 = %.15f, ci_or1 = (%.15f, %.15f)\n", exp(b1), exp(ci[2, 1]), exp(ci[2, 2])))
  cat(sprintf("x50 = %.15f, ci_x50 = (%.15f, %.15f)\n", x50, x50_ci["lower"], x50_ci["upper"]))
  cat(sprintf("d0 = %.15f, d1 = %.15f, g2 = %.15f, lr_p = %.15e\n", d0, d1, g2, lr_p))
  cat(sprintf("aic = %.15f, aicc = %.15f\n", aic, aicc))
  cat(sprintf("tjur_r2 = %.15f, cox_snell_r2 = %.15f, auc = %.15f\n", tjur, cox_snell, auc))

  if (length(x_new) > 0) {
    pr <- predict(fit, newdata = data.frame(x = x_new), type = "link", se.fit = TRUE)
    zc <- qnorm(0.975)
    for (i in seq_along(x_new)) {
      eta <- pr$fit[i]
      se <- pr$se.fit[i]
      cat(sprintf("pred[x=%.4f]: p = %.15f, ci = (%.15f, %.15f)\n",
                  x_new[i], plogis(eta), plogis(eta - zc * se), plogis(eta + zc * se)))
    }
  }
}

# Case 1: Positive-slope dose-response (n = 24)
x1 <- c(0.5, 0.8, 1.0, 1.2, 1.5, 1.8, 2.0, 2.2, 2.5, 2.8, 3.0, 3.2,
        3.5, 3.8, 4.0, 4.2, 4.5, 4.8, 5.0, 5.2, 5.5, 5.8, 6.0, 6.5)
y1 <- c(0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1,
        1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1)
run_case("dose_response_pos", x1, y1, x_new = c(1.0, 3.0, 5.0))

# Case 2: Negative-slope biomarker survival (n = 18)
x2 <- c(10, 12, 14, 15, 17, 19, 21, 22, 24, 26, 28, 30, 32, 35, 37, 40, 42, 45)
y2 <- c(1, 1, 1, 1, 0, 1, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0)
run_case("biomarker_neg", x2, y2, x_new = c(25.0))

# Case 3: Weak / non-significant association (n = 16, G^2 < chi^2_{1,0.95} => unbounded X50 CI)
x3 <- c(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
y3 <- c(0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1)
run_case("weak_nonsig", x3, y3)
