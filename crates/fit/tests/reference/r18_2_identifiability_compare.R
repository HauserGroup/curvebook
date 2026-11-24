# Independent R 4.5.2 oracle for R18.2 (§2.5 Identifiability, Model-Comparison,
# Output & Quality-Flag Extensions).
#
# Run with:
#   Rscript crates/fit/tests/reference/r18_2_identifiability_compare.R

options(digits = 15)

cat("=== 1. Greedy Hessian Identifiability (kappa(H) > 10 / sqrt(eps_mach)) ===\n")
eps_mach <- .Machine$double.eps
kappa_thresh <- 10.0 / sqrt(eps_mach)
cat(sprintf("eps_mach      = %.17e\n", eps_mach))
cat(sprintf("kappa_thresh  = %.17e\n", kappa_thresh))

# Construct a 3-parameter linear-in-parameters model Y = A*X + B + C*(1e-6 * X^2)
# on X = 0..11 so H = 2 * J^T J has kappa(H) > 6.71e8, while dropping C leaves
# well-conditioned (A, B) with kappa(H_{A,B}) << 1e3.
xs <- as.numeric(0:11)
noise <- c(0.02, -0.015, 0.01, -0.025, 0.015, -0.01, 0.02, -0.02, 0.005, -0.015, 0.025, -0.01)
ys <- 1.5 + 2.5 * xs + noise
J <- cbind(xs, rep(1.0, length(xs)), 1e-6 * xs^2)
H <- 2.0 * crossprod(J)
sv_full <- svd(H)$d
kappa_full <- max(sv_full) / min(sv_full)
cat(sprintf("kappa(H_full) = %.12e\n", kappa_full))
for (drop_idx in 1:3) {
  sub <- H[-drop_idx, -drop_idx, drop = FALSE]
  sv <- svd(sub)$d
  cat(sprintf("kappa(H_-%d)   = %.12e\n", drop_idx, max(sv) / min(sv)))
}
# Reduced-subspace OLS fit on (A, B) with df = N - K_full = 12 - 3 = 9 (matching solver df)
fit_full <- lm(ys ~ xs + I(1e-6 * xs^2))
rss_full <- sum(resid(fit_full)^2)
s2_full <- rss_full / (length(xs) - 3)
J_ab <- J[, 1:2]
cov_ab <- s2_full * solve(crossprod(J_ab))
se_ab <- sqrt(diag(cov_ab))
cat(sprintf("RSS_full      = %.15f\n", rss_full))
cat(sprintf("SE_A (sub)    = %.15f\n", se_ab[1]))
cat(sprintf("SE_B (sub)    = %.15f\n", se_ab[2]))

cat("\n=== 2. Model Comparison Questions (4PL / 3PL) ===\n")
x_dr <- c(-9, -8, -7.5, -7, -6.5, -6, -5.5, -5, -4)
e_a <- c(0.4, -0.3, 0.2, -0.5, 0.3, -0.2, 0.4, -0.3, 0.1)
e_b <- c(-0.3, 0.4, -0.2, 0.3, -0.4, 0.2, -0.1, 0.3, -0.2)
# Dataset A: Bottom=10, Top=100, LogEC50=-6.5, HillSlope=1.8
y_a <- 10 + 90 / (1 + 10^((-6.5 - x_dr) * 1.8)) + e_a
# Dataset B: Bottom=10, Top=100, LogEC50=-5.8, HillSlope=1.8 (same slope, shifted EC50)
y_b <- 10 + 90 / (1 + 10^((-5.8 - x_dr) * 1.8)) + e_b

# Q1: AlternativeEquation (4PL vs 3PL on Dataset A)
f_4pl_a <- nls(y_a ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - x_dr) * HillSlope)),
               start = list(Bottom = 10, Top = 100, LogEC50 = -6.5, HillSlope = 1.5))
f_3pl_a <- nls(y_a ~ Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - x_dr)),
               start = list(Bottom = 10, Top = 100, LogEC50 = -6.5))
ss1 <- sum(resid(f_3pl_a)^2); df1 <- df.residual(f_3pl_a)
ss2 <- sum(resid(f_4pl_a)^2); df2 <- df.residual(f_4pl_a)
F_q1 <- ((ss1 - ss2) / (df1 - df2)) / (ss2 / df2)
p_q1 <- pf(F_q1, df1 - df2, df2, lower.tail = FALSE)
n_a <- length(x_dr)
aicc <- function(ss, k_free, n) {
  K <- k_free + 1
  n * log(ss / n) + 2 * K + (2 * K * (K + 1)) / (n - K - 1)
}
aicc_4pl <- aicc(ss2, 4, n_a)
aicc_3pl <- aicc(ss1, 3, n_a)
cat(sprintf("Q1 (4PL vs 3PL): SS_3pl=%.12f df=%d, SS_4pl=%.12f df=%d, F=%.12f, p=%.12e\n",
            ss1, df1, ss2, df2, F_q1, p_q1))
cat(sprintf("Q1 AICc: 4PL=%.12f, 3PL=%.12f, delta=%.12f\n", aicc_4pl, aicc_3pl, aicc_3pl - aicc_4pl))

# Q4: ParameterEqualsTheoretical (HillSlope = 1.0 on Dataset A -> identical to 3PL!)
cat(sprintf("Q4 (HillSlope=1.0 vs free): F=%.12f, dfn=%d, dfd=%d, p=%.12e\n",
            F_q1, df1 - df2, df2, p_q1))

# Q2: ParameterInvarianceAcrossDatasets (shared HillSlope across A & B vs separate)
df_ab <- data.frame(
  x = c(x_dr, x_dr),
  y = c(y_a, y_b),
  g = factor(c(rep("A", length(x_dr)), rep("B", length(x_dr))))
)
f_shared_h <- nls(
  y ~ Bottom[g] + (Top[g] - Bottom[g]) / (1 + 10^((LogEC50[g] - x) * HillSlope)),
  data = df_ab,
  start = list(Bottom = c(10, 10), Top = c(100, 100), LogEC50 = c(-6.5, -5.8), HillSlope = 1.8)
)
f_sep_ab <- nls(
  y ~ Bottom[g] + (Top[g] - Bottom[g]) / (1 + 10^((LogEC50[g] - x) * HillSlope[g])),
  data = df_ab,
  start = list(Bottom = c(10, 10), Top = c(100, 100), LogEC50 = c(-6.5, -5.8), HillSlope = c(1.8, 1.8))
)
ss_sh <- sum(resid(f_shared_h)^2); df_sh <- df.residual(f_shared_h)
ss_sep <- sum(resid(f_sep_ab)^2); df_sep <- df.residual(f_sep_ab)
F_q2 <- ((ss_sh - ss_sep) / (df_sh - df_sep)) / (ss_sep / df_sep)
p_q2 <- pf(F_q2, df_sh - df_sep, df_sep, lower.tail = FALSE)
cat(sprintf("Q2 (Shared HillSlope): SS_null=%.12f df=%d, SS_alt=%.12f df=%d, F=%.12f, p=%.12e\n",
            ss_sh, df_sh, ss_sep, df_sep, F_q2, p_q2))

# Q3: SingleGlobalCurveVsSeparate (one global 4PL curve vs separate curves on A & B)
f_global <- nls(
  y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - x) * HillSlope)),
  data = df_ab,
  start = list(Bottom = 10, Top = 100, LogEC50 = -6.1, HillSlope = 1.5)
)
ss_gl <- sum(resid(f_global)^2); df_gl <- df.residual(f_global)
F_q3 <- ((ss_gl - ss_sep) / (df_gl - df_sep)) / (ss_sep / df_sep)
p_q3 <- pf(F_q3, df_gl - df_sep, df_sep, lower.tail = FALSE)
cat(sprintf("Q3 (Global vs Separate): SS_null=%.12f df=%d, SS_alt=%.12f df=%d, F=%.12f, p=%.12e\n",
            ss_gl, df_gl, ss_sep, df_sep, F_q3, p_q3))

cat("\n=== 3. Parameter Summary (One-Phase Association k_obs vs [L]) ===\n")
t_kin <- c(0, 1, 2, 4, 8, 12, 20)
e_kin <- c(0.1, -0.15, 0.08, -0.12, 0.09, -0.05, 0.05)
for (conc in c(1, 2, 4)) {
  k_true <- 0.10 + 0.08 * conc
  y_k <- 5 + (100 - 5) * (1 - exp(-k_true * t_kin)) + e_kin
  fk <- nls(y_k ~ Y0 + (Plateau - Y0) * (1 - exp(-K * t_kin)),
            start = list(Y0 = 5, Plateau = 100, K = k_true))
  cat(sprintf("[L]=%d: K=%.15f\n", conc, coef(fk)[["K"]]))
}

cat("\n=== 4. Dual Interpolated Mean X / Replicates + 10^X Antilog Chaining ===\n")
# Linear log-X standard curve Y = 3 + 2 * X, X = 0..4, 2 replicates (+/-0.25)
# Unknown row: Y_rep1 = 6.0, Y_rep2 = 8.0 -> mean Y = 7.0 -> X_mean = 2.0, X_rep1 = 1.5, X_rep2 = 2.5
x_std <- rep(0:4, each = 2)
y_std <- 3 + 2 * x_std + rep(c(-0.25, 0.25), times = 5)
fit_lin <- lm(y_std ~ x_std)
s2 <- sum(resid(fit_lin)^2) / df.residual(fit_lin)
b <- coef(fit_lin)[["x_std"]]
xbar <- mean(x_std)
sxx <- sum((x_std - xbar)^2)
tcrit <- qt(0.975, df = df.residual(fit_lin))
# Exact mean-response band inversion (Fieller / Seber & Lee 2003 §6.1.3):
# solve a + b*x +/- t * s * sqrt(1/n + (x - xbar)^2 / Sxx) = y0
for (entry in list(list(name = "MeanX", y0 = 7.0), list(name = "Rep1", y0 = 6.0), list(name = "Rep2", y0 = 8.0))) {
  xh <- (entry$y0 - coef(fit_lin)[["(Intercept)"]]) / b
  a_coef <- coef(fit_lin)[["(Intercept)"]]
  lo <- uniroot(function(x) a_coef + b * x + tcrit * sqrt(s2 * (1 / length(x_std) + (x - xbar)^2 / sxx)) - entry$y0,
                interval = c(xh - 2, xh))$root
  hi <- uniroot(function(x) a_coef + b * x - tcrit * sqrt(s2 * (1 / length(x_std) + (x - xbar)^2 / sxx)) - entry$y0,
                interval = c(xh, xh + 2))$root
  cat(sprintf("%s: x_hat=%.12f, ci=[%.12f, %.12f], 10^x=%.12f, 10^ci=[%.12f, %.12f]\n",
              entry$name, xh, lo, hi, 10^xh, 10^lo, 10^hi))
}
