# D1 Reference Oracle Script for R18.5:
# Simple Linear Fixed-Point, Standard Addition (Fieller 1940),
# Deming (Model II) Unequal-Error Regression (Deming 1943; Linnet 1990, 1993),
# and Duplicate-Pair Repeatability SD (Dahlberg 1940; Bland & Altman 1996).
#
# Run with: Rscript crates/fit/tests/reference/r18_5_linear_deming.R

options(digits = 17)
cat(sprintf("R version: %s\n\n", R.version.string))

# ==============================================================================
# 1. Arbitrary Fixed-Point Linear Regression: Y - Y0 = b * (X - X0), df = N - 1
# ==============================================================================
x_fp <- c(1.0, 2.0, 3.0, 4.0, 5.0, 6.0)
y_fp <- c(2.4, 4.9, 6.8, 9.3, 11.2, 13.7)
x0 <- 2.0
y0 <- 5.0

u <- x_fp - x0
v <- y_fp - y0
fit_fp <- lm(v ~ 0 + u)
s_fp <- summary(fit_fp)
slope_fp <- unname(coef(fit_fp)[1])
se_slope_fp <- s_fp$coefficients[1, 2]
ci_slope_fp <- unname(confint(fit_fp, level = 0.95)[1, ])
df_fp <- df.residual(fit_fp)
t_fp <- qt(0.975, df_fp)
intercept_fp <- y0 - slope_fp * x0
se_int_fp <- abs(x0) * se_slope_fp
ci_int_fp <- intercept_fp + c(-1, 1) * t_fp * se_int_fp
rss_fp <- sum(resid(fit_fp)^2)
syx_fp <- s_fp$sigma
r2_fp <- s_fp$r.squared
f_fp <- unname(s_fp$fstatistic["value"])
p_fp <- pf(f_fp, 1, df_fp, lower.tail = FALSE)
x_int_fp <- x0 - y0 / slope_fp
# Exact Fieller (1940) CI for X-intercept under fixed point (x0, y0):
# (0 - (y0 + b*(x - x0)))^2 <= t^2 * s^2 * (x - x0)^2 / S_uu
# => slope bounds b_lo = b - t*SE(b), b_hi = b + t*SE(b) when g < 1
x_int_fp_ci <- sort(c(x0 - y0 / ci_slope_fp[1], x0 - y0 / ci_slope_fp[2]))
# Pointwise CI/PI at x_eval = 4.5
pred_ci_fp <- predict(fit_fp, newdata = data.frame(u = 4.5 - x0), interval = "confidence", level = 0.95) + y0
pred_pi_fp <- predict(fit_fp, newdata = data.frame(u = 4.5 - x0), interval = "prediction", level = 0.95) + y0

cat("=== 1. Fixed-Point Linear Regression (x0=2, y0=5) ===\n")
cat(sprintf("slope        = %.15f\n", slope_fp))
cat(sprintf("intercept    = %.15f\n", intercept_fp))
cat(sprintf("se_slope     = %.15f\n", se_slope_fp))
cat(sprintf("se_intercept = %.15f\n", se_int_fp))
cat(sprintf("ci_slope     = [%.15f, %.15f]\n", ci_slope_fp[1], ci_slope_fp[2]))
cat(sprintf("ci_intercept = [%.15f, %.15f]\n", ci_int_fp[1], ci_int_fp[2]))
cat(sprintf("rss          = %.15f\n", rss_fp))
cat(sprintf("sy_x         = %.15f\n", syx_fp))
cat(sprintf("r_squared    = %.15f\n", r2_fp))
cat(sprintf("f_stat       = %.15f\n", f_fp))
cat(sprintf("p_value      = %.15e\n", p_fp))
cat(sprintf("x_intercept  = %.15f\n", x_int_fp))
cat(sprintf("x_int_ci     = [%.15f, %.15f]\n", x_int_fp_ci[1], x_int_fp_ci[2]))
cat(sprintf("band(4.5) y=%.15f ci=[%.15f, %.15f] pi=[%.15f, %.15f]\n\n",
            pred_ci_fp[1], pred_ci_fp[2], pred_ci_fp[3], pred_pi_fp[2], pred_pi_fp[3]))

# ==============================================================================
# 2. Standard Addition & Exact Fieller (1940) X-Intercept CI
# ==============================================================================
x_sa <- c(0.0, 5.0, 10.0, 15.0, 20.0, 25.0)
y_sa <- c(0.182, 0.348, 0.505, 0.678, 0.835, 1.008)
fit_sa <- lm(y_sa ~ x_sa)
s_sa <- summary(fit_sa)
a_sa <- unname(coef(fit_sa)[1])
b_sa <- unname(coef(fit_sa)[2])
syx_sa <- s_sa$sigma
df_sa <- df.residual(fit_sa)
t_sa <- qt(0.975, df_sa)
n_sa <- length(x_sa)
xbar_sa <- mean(x_sa)
ybar_sa <- mean(y_sa)
sxx_sa <- sum((x_sa - xbar_sa)^2)
x_int_sa <- -a_sa / b_sa
g_sa <- (t_sa * syx_sa)^2 / (b_sa^2 * sxx_sa)
d_sa <- -ybar_sa / b_sa
x_mid_sa <- xbar_sa + d_sa / (1 - g_sa)
half_sa <- (t_sa * syx_sa / (abs(b_sa) * (1 - g_sa))) * sqrt((1 - g_sa) / n_sa + d_sa^2 / sxx_sa)
x_int_sa_ci <- c(x_mid_sa - half_sa, x_mid_sa + half_sa)
conc_sa <- -x_int_sa
conc_sa_ci <- c(-x_int_sa_ci[2], -x_int_sa_ci[1])

# Verify exact Fieller quadratic boundary identity at both limits
for (lim in x_int_sa_ci) {
  lhs <- (a_sa + b_sa * lim)^2
  rhs <- (t_sa * syx_sa)^2 * (1 / n_sa + (lim - xbar_sa)^2 / sxx_sa)
  stopifnot(abs(lhs - rhs) < 1e-14)
}

cat("=== 2. Standard Addition & Fieller (1940) X-Intercept ===\n")
cat(sprintf("slope        = %.15f\n", b_sa))
cat(sprintf("intercept    = %.15f\n", a_sa))
cat(sprintf("sy_x         = %.15f\n", syx_sa))
cat(sprintf("fieller_g    = %.15f\n", g_sa))
cat(sprintf("x_intercept  = %.15f\n", x_int_sa))
cat(sprintf("x_int_ci     = [%.15f, %.15f]\n", x_int_sa_ci[1], x_int_sa_ci[2]))
cat(sprintf("unknown_conc = %.15f\n", conc_sa))
cat(sprintf("conc_ci      = [%.15f, %.15f]\n\n", conc_sa_ci[1], conc_sa_ci[2]))

# ==============================================================================
# 3. Deming (Model II) Regression with Unequal Errors (Deming 1943; Linnet 1990/1993)
# ==============================================================================
deming_core <- function(x, y, lambda) {
  n <- length(x)
  mx <- mean(x)
  my <- mean(y)
  sxx <- sum((x - mx)^2)
  syy <- sum((y - my)^2)
  sxy <- sum((x - mx) * (y - my))
  u <- lambda * syy - sxx
  slope <- (u + sqrt(u^2 + 4 * lambda * sxy^2)) / (2 * lambda * sxy)
  intercept <- my - slope * mx
  syx <- sqrt(sum((y - (intercept + slope * x))^2) / ((1 + lambda * slope^2) * (n - 2)))
  list(slope = slope, intercept = intercept, sy_x = syx, x_intercept = -intercept / slope)
}

deming_full <- function(x, y, lambda) {
  n <- length(x)
  core <- deming_core(x, y, lambda)
  js <- numeric(n)
  ji <- numeric(n)
  for (i in seq_len(n)) {
    sub <- deming_core(x[-i], y[-i], lambda)
    js[i] <- sub$slope
    ji[i] <- sub$intercept
  }
  se_s <- sqrt((n - 1) / n * sum((js - mean(js))^2))
  se_i <- sqrt((n - 1) / n * sum((ji - mean(ji))^2))
  t_c <- qt(0.975, n - 2)
  c(core, list(
    se_slope = se_s,
    se_intercept = se_i,
    ci_slope = core$slope + c(-1, 1) * t_c * se_s,
    ci_intercept = core$intercept + c(-1, 1) * t_c * se_i
  ))
}

x_dem <- c(10.2, 19.8, 30.5, 39.7, 51.0, 59.4, 70.6, 80.1)
y_dem <- c(11.5, 22.1, 31.8, 43.2, 54.0, 62.7, 74.9, 85.2)

cat("=== 3. Deming Regression (Unequal Errors) ===\n")
for (lam in c(1.0, 4.0, 0.25)) {
  d <- deming_full(x_dem, y_dem, lam)
  cat(sprintf("--- lambda = %.4f ---\n", lam))
  cat(sprintf("slope        = %.15f\n", d$slope))
  cat(sprintf("intercept    = %.15f\n", d$intercept))
  cat(sprintf("se_slope     = %.15f\n", d$se_slope))
  cat(sprintf("se_intercept = %.15f\n", d$se_intercept))
  cat(sprintf("ci_slope     = [%.15f, %.15f]\n", d$ci_slope[1], d$ci_slope[2]))
  cat(sprintf("ci_intercept = [%.15f, %.15f]\n", d$ci_intercept[1], d$ci_intercept[2]))
  cat(sprintf("sy_x         = %.15f\n", d$sy_x))
  cat(sprintf("x_intercept  = %.15f\n\n", d$x_intercept))
}

# ==============================================================================
# 4. Duplicate-Pair Repeatability SD (Dahlberg 1940; Bland & Altman 1996)
# ==============================================================================
dup_a <- c(10.1, 20.3, 30.0, 40.2, 50.8, 60.4)
dup_b <- c(10.4, 19.9, 30.5, 40.2, 50.1, 60.9)
sd_rep <- sqrt(sum((dup_a - dup_b)^2) / (2 * length(dup_a)))
cat("=== 4. Duplicate-Pair Repeatability SD ===\n")
cat(sprintf("sd_repeatability = %.15f\n", sd_rep))
