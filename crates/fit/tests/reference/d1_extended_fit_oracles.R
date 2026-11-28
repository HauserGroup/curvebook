# Independent R 4.5.2 (2025-10-31) black-box reference generator for
# extended CurveBook fit D1 oracles:
#   - 4PL inverse-interpolation `uniroot` edge cases (extending `r13_1_d1_oracles.rs`)
#   - `exponential_growth`, `one_site_specific_hill`, `mixed_model_inhibition`,
#     `linear_quadratic_survival`
#   - `compare_linear_lines` ANCOVA (`anova(lm(y ~ x * g))`, plus through-origin)
#   - Model comparisons: noisy `competition_ki` (1-site vs 2-site), `extra_ss_f`
#     on NIST `Rat43` (4PL vs 5PL), noisy `global_shared` 3-curve decay, and
#     noisy 4-curve Gaddum/Schild (`SchildSlope = 1` vs free).
#
# Run under R 4.5.2 (2025-10-31):
#   Rscript crates/fit/tests/reference/d1_extended_fit_oracles.R

options(digits = 16)

print_nls <- function(tag, fit) {
  sm <- summary(fit)
  co <- sm$coefficients
  rss <- sum(residuals(fit)^2)
  df <- df.residual(fit)
  cat(sprintf("--- %s: n=%d df=%d rss=%.12f syx=%.12f ---\n",
              tag, length(residuals(fit)), df, rss, sqrt(rss / df)))
  for (nm in rownames(co)) {
    cat(sprintf("  %-14s est=%.12f se=%.12f\n", nm, co[nm, "Estimate"], co[nm, "Std. Error"]))
  }
}

cat("=== 1. 4PL Inverse-Interpolation uniroot Edge Cases ===\n")
xs_4pl <- c(-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0)
noise_mat <- matrix(c(
  -0.6,  0.2,  0.4,
   0.5, -0.4, -0.1,
  -0.3,  0.5, -0.2,
   0.4, 120.0, -0.3,
  -0.5,  0.3,  0.2,
   0.3, -0.5,  0.2,
  -0.2,  0.4, -0.2,
   0.4, -0.3, -0.1
), nrow = 8, byrow = TRUE)
x_clean <- c(); y_clean <- c()
for (r in 1:8) {
  y0 <- 10 + 90 / (1 + 10^((-6.2 - xs_4pl[r]) * 1.1))
  for (s in 1:3) {
    if (r == 4 && s == 2) next
    x_clean <- c(x_clean, xs_4pl[r])
    y_clean <- c(y_clean, y0 + noise_mat[r, s])
  }
}
fit_4pl <- nls(
  y_clean ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - x_clean) * HillSlope)),
  start = list(Bottom = 10, Top = 100, LogEC50 = -6.2, HillSlope = 1.1),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("4PL agonist clean23", fit_4pl)
p_ag <- coef(fit_4pl)
V_ag <- vcov(fit_4pl) # sigma^2 * (J^T J)^{-1}
sigma_ag <- summary(fit_4pl)$sigma
df_ag <- df.residual(fit_4pl)
tcrit_ag <- qt(0.975, df_ag)

f_ag <- function(x) p_ag["Bottom"] + (p_ag["Top"] - p_ag["Bottom"]) / (1 + 10^((p_ag["LogEC50"] - x) * p_ag["HillSlope"]))
grad_ag <- function(x) {
  u <- (p_ag["LogEC50"] - x) * p_ag["HillSlope"]
  e <- 10^u
  sig <- 1 / (1 + e)
  span <- p_ag["Top"] - p_ag["Bottom"]
  ds_du <- -sig * (1 - sig) * log(10)
  c(
    Bottom = 1 - sig,
    Top = sig,
    LogEC50 = span * ds_du * p_ag["HillSlope"],
    HillSlope = span * ds_du * (p_ag["LogEC50"] - x)
  )
}
inv_4pl_edge <- function(y_target, m = 1, scan_lo = -11.5, scan_hi = -1.5) {
  if (y_target <= p_ag["Bottom"] || y_target >= p_ag["Top"]) {
    cat(sprintf("  y=%.4f -> OUTSIDE PLATEAU [%.6f, %.6f]\n", y_target, p_ag["Bottom"], p_ag["Top"]))
    return(invisible(NULL))
  }
  x_hat <- uniroot(function(x) f_ag(x) - y_target, c(scan_lo, scan_hi), tol = 1e-13)$root
  band_lo <- min(scan_lo, x_hat - 5)
  band_hi <- max(scan_hi, x_hat + 5)
  se_mean <- function(x) { g <- grad_ag(x); sqrt(as.numeric(t(g) %*% V_ag %*% g)) }
  se_pred <- function(x) { g <- grad_ag(x); sqrt(sigma_ag^2 / m + as.numeric(t(g) %*% V_ag %*% g)) }
  find_root_opt <- function(fn, lo, hi) {
    flo <- fn(lo); fhi <- fn(hi)
    if (!is.finite(flo) || !is.finite(fhi) || flo * fhi > 0) return(NA_real_)
    uniroot(fn, c(lo, hi), tol = 1e-13)$root
  }
  ci_lo <- find_root_opt(function(x) f_ag(x) + tcrit_ag * se_mean(x) - y_target, band_lo, x_hat)
  ci_hi <- find_root_opt(function(x) f_ag(x) - tcrit_ag * se_mean(x) - y_target, x_hat, band_hi)
  pi_lo <- find_root_opt(function(x) f_ag(x) + tcrit_ag * se_pred(x) - y_target, band_lo, x_hat)
  pi_hi <- find_root_opt(function(x) f_ag(x) - tcrit_ag * se_pred(x) - y_target, x_hat, band_hi)
  cat(sprintf("  y=%.4f (m=%d): x_hat=%.12f CI=[%.12f, %.12f] PI=[%.12f, %.12f]\n",
              y_target, m, x_hat, ci_lo, ci_hi, pi_lo, pi_hi))
}
# Near top plateau (finite CI, open PI hi; and open CI hi), near bottom plateau (open lo), extrapolated outside [-9,-4]
for (yt in c(99.2, 99.65, 99.90, 10.30, 10.05, 105.0, 5.0)) {
  inv_4pl_edge(yt, m = 1)
}

# Decreasing 4PL inhibitor (`log_inhibitor_vs_response_4pl`)
x_inh <- c(-10.0, -9.5, -9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0)
e_inh <- c(0.4, -0.5, 0.3, -0.4, 0.5, -0.6, 0.4, -0.5, 0.6, -0.3, 0.4, -0.3)
y_inh <- 12.0 + (98.0 - 12.0) / (1 + 10^((x_inh - (-7.1)) * 1.25)) + e_inh
fit_inh <- nls(
  y_inh ~ Bottom + (Top - Bottom) / (1 + 10^((x_inh - LogIC50) * HillSlope)),
  start = list(Bottom = 12, Top = 98, LogIC50 = -7.1, HillSlope = 1.25),
  algorithm = "port",
  control = nls.control(tol = 1e-9, maxiter = 500)
)
print_nls("4PL inhibitor", fit_inh)
p_in <- coef(fit_inh); V_in <- vcov(fit_inh); sig_in <- summary(fit_inh)$sigma; tc_in <- qt(0.975, df.residual(fit_inh))
f_in <- function(x) p_in["Bottom"] + (p_in["Top"] - p_in["Bottom"]) / (1 + 10^((x - p_in["LogIC50"]) * p_in["HillSlope"]))
grad_in <- function(x) {
  u <- (x - p_in["LogIC50"]) * p_in["HillSlope"]
  e <- 10^u; sig <- 1 / (1 + e); span <- p_in["Top"] - p_in["Bottom"]
  ds_du <- -sig * (1 - sig) * log(10)
  c(Bottom = 1 - sig, Top = sig, LogIC50 = -span * ds_du * p_in["HillSlope"], HillSlope = span * ds_du * (x - p_in["LogIC50"]))
}
for (yt in c(55.0, 97.5)) {
  xh <- uniroot(function(x) f_in(x) - yt, c(-13, -1), tol = 1e-13)$root
  se_m <- function(x) { g <- grad_in(x); sqrt(as.numeric(t(g) %*% V_in %*% g)) }
  se_p <- function(x) { g <- grad_in(x); sqrt(sig_in^2 + as.numeric(t(g) %*% V_in %*% g)) }
  fopt <- function(fn, lo, hi) {
    flo <- fn(lo); fhi <- fn(hi)
    if (!is.finite(flo) || !is.finite(fhi) || flo * fhi > 0) NA_real_ else uniroot(fn, c(lo, hi), tol = 1e-13)$root
  }
  # Decreasing curve: lower band crosses at smaller X (ci_lo), upper band crosses at larger X (ci_hi)
  ci_lo <- fopt(function(x) f_in(x) - tc_in * se_m(x) - yt, -16, xh)
  ci_hi <- fopt(function(x) f_in(x) + tc_in * se_m(x) - yt, xh, 2)
  pi_lo <- fopt(function(x) f_in(x) - tc_in * se_p(x) - yt, -16, xh)
  pi_hi <- fopt(function(x) f_in(x) + tc_in * se_p(x) - yt, xh, 2)
  cat(sprintf("  Inhibitor y=%.2f: x_hat=%.12f CI=[%.12f, %.12f] PI=[%.12f, %.12f]\n", yt, xh, ci_lo, ci_hi, pi_lo, pi_hi))
}

cat("\n=== 2. exponential_growth ===\n")
t_eg <- c(0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0)
e_eg <- c(0.15, -0.22, 0.18, -0.14, 0.30, -0.25, 0.42, -0.35, 0.50, -0.45, 0.38)
y_eg <- 2.5 * exp(0.45 * t_eg) + e_eg
fit_eg <- nls(y_eg ~ Y0 * exp(k * t_eg), start = list(Y0 = 2.5, k = 0.45), algorithm = "port", control = nls.control(tol = 1e-8, maxiter = 1000))
print_nls("exponential_growth", fit_eg)
cat(sprintf("  DoublingTime=%.12f\n", log(2) / coef(fit_eg)["k"]))

cat("\n=== 3. one_site_specific_hill ===\n")
c_hill <- c(0.1, 0.25, 0.5, 1.0, 2.0, 3.5, 5.0, 8.0, 12.0, 20.0, 35.0, 60.0)
e_hill <- c(0.3, -0.4, 0.5, -0.6, 0.8, -0.9, 0.7, -0.8, 0.6, -0.5, 0.4, -0.3)
y_hill <- 140.0 * (c_hill^1.65) / (4.2^1.65 + c_hill^1.65) + e_hill
fit_hill <- nls(y_hill ~ Bmax * (c_hill^h) / (Kd^h + c_hill^h),
                start = list(Bmax = 140, Kd = 4.2, h = 1.6), algorithm = "port", control = nls.control(tol = 1e-8, maxiter = 1000))
print_nls("one_site_specific_hill", fit_hill)

cat("\n=== 4. mixed_model_inhibition (global 4 inhibitor concentrations) ===\n")
s_mix <- c(0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0)
i_concs <- c(0.0, 2.0, 6.0, 15.0)
mix_s <- rep(s_mix, times = length(i_concs))
mix_i <- rep(i_concs, each = length(s_mix))
mix_err <- rep(c(0.45, -0.35, 0.50, -0.40, 0.30, -0.45, 0.35, -0.30), times = 4) *
           rep(c(1.0, -1.0, 0.8, -0.9), each = 8)
mix_y <- (115.0 * mix_s) / (5.5 * (1 + mix_i / 4.0) + mix_s * (1 + mix_i / (3.2 * 4.0))) + mix_err
fit_mix <- nls(
  mix_y ~ (Vmax * mix_s) / (Km * (1 + mix_i / Ki) + mix_s * (1 + mix_i / (Alpha * Ki))),
  start = list(Vmax = 110, Km = 5.0, Ki = 4.0, Alpha = 3.0),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("mixed_model_inhibition", fit_mix)

cat("\n=== 5. linear_quadratic_survival ===\n")
d_lq <- c(0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0)
e_lq <- c(0.005, -0.012, 0.010, -0.008, 0.006, -0.004, 0.003, -0.0015, 0.0005)
y_lq <- exp(-(0.28 * d_lq + 0.035 * d_lq^2)) + e_lq
fit_lq <- nls(y_lq ~ exp(-(Alpha * d_lq + Beta * d_lq^2)),
              start = list(Alpha = 0.25, Beta = 0.03), algorithm = "port", control = nls.control(tol = 1e-8, maxiter = 1000))
print_nls("linear_quadratic_survival", fit_lq)
cat(sprintf("  Alpha/Beta=%.12f\n", coef(fit_lq)["Alpha"] / coef(fit_lq)["Beta"]))

cat("\n=== 6. compare_linear_lines ANCOVA (anova(lm(y ~ x * g))) ===\n")
# Case A: 3 groups with different slopes
x1 <- c(1, 2, 3, 4, 5, 6);    y1 <- c(3.1, 4.9, 7.2, 9.0, 11.3, 12.9)
x2 <- c(1, 2, 3, 4, 5, 6, 7); y2 <- c(5.2, 8.1, 10.8, 14.1, 17.0, 19.8, 23.2)
x3 <- c(2, 3, 4, 5, 6);       y3 <- c(4.5, 5.9, 7.4, 8.8, 10.4)
df_a <- data.frame(x = c(x1, x2, x3), y = c(y1, y2, y3), g = factor(rep(1:3, c(length(x1), length(x2), length(x3)))))
an_a <- anova(lm(y ~ x + g, df_a), lm(y ~ x * g, df_a))
cat(sprintf("ANCOVA diff slopes: F=%.12f df=(%d,%d) p=%.12e\n", an_a$F[2], an_a$Df[2], an_a$Res.Df[2], an_a$`Pr(>F)`[2]))

# Case B: 3 parallel groups with different elevations
yp1 <- c(2.2, 4.0, 6.1, 7.9, 10.2, 11.9)
yp2 <- c(5.1, 7.2, 9.0, 11.3, 13.0, 15.1, 17.2)
yp3 <- c(8.8, 10.9, 12.7, 14.9, 16.8)
df_b <- data.frame(x = c(x1, x2, x3), y = c(yp1, yp2, yp3), g = factor(rep(1:3, c(length(x1), length(x2), length(x3)))))
m_b0 <- lm(y ~ x, df_b); m_b1 <- lm(y ~ x + g, df_b); m_b2 <- lm(y ~ x * g, df_b)
an_bs <- anova(m_b1, m_b2); an_bi <- anova(m_b0, m_b1)
cat(sprintf("ANCOVA parallel slopes: F_s=%.12f df=(%d,%d) p_s=%.12e\n", an_bs$F[2], an_bs$Df[2], an_bs$Res.Df[2], an_bs$`Pr(>F)`[2]))
cat(sprintf("ANCOVA parallel elev:   F_i=%.12f df=(%d,%d) p_i=%.12e pooled_slope=%.12f\n",
            an_bi$F[2], an_bi$Df[2], an_bi$Res.Df[2], an_bi$`Pr(>F)`[2], coef(m_b1)["x"]))

# Case C: Force through origin (3 groups)
yo1 <- c(2.1, 3.9, 6.2, 8.0, 10.1, 11.8)
yo2 <- c(2.8, 5.7, 8.4, 11.3, 14.1, 16.8, 19.7)
yo3 <- c(3.0, 4.6, 6.1, 7.4, 9.1)
df_c <- data.frame(x = c(x1, x2, x3), y = c(yo1, yo2, yo3), g = factor(rep(1:3, c(length(x1), length(x2), length(x3)))))
m_c0 <- lm(y ~ 0 + x, df_c); m_c1 <- lm(y ~ 0 + x:g, df_c)
an_c <- anova(m_c0, m_c1)
cat(sprintf("ANCOVA origin slopes: F=%.12f df=(%d,%d) p=%.12e pooled_slope=%.12f\n",
            an_c$F[2], an_c$Df[2], an_c$Res.Df[2], an_c$`Pr(>F)`[2], coef(m_c0)["x"]))

cat("\n=== 7. Noisy competition_ki (1-site vs 2-site) ===\n")
hot_c <- 2.4; hot_kd <- 0.8; sh <- log10(1 + hot_c / hot_kd)
log_comp <- seq(-12.0, -3.0, by = 0.5)
err_ki <- c(11, -14, 9, -8, 12, -10, 7, -13, 10, -9, 11, -12, 8, -7, 13, -11, 9, -10, 6)
y_ki2 <- 150 + 1050 * (0.42 / (1 + 10^(log_comp - (log10(5e-10) + sh))) +
                       0.58 / (1 + 10^(log_comp - (log10(2e-7) + sh)))) + err_ki
fit_ki1 <- nls(
  y_ki2 ~ Bottom + (Top - Bottom) / (1 + 10^(log_comp - (LogKi + sh))),
  start = list(Bottom = 150, Top = 1200, LogKi = -8.0),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
fit_ki2 <- nls(
  y_ki2 ~ Bottom + (Top - Bottom) * (FractionHi / (1 + 10^(log_comp - (LogKiHi + sh))) +
                                     (1 - FractionHi) / (1 + 10^(log_comp - (LogKiLo + sh)))),
  start = list(Bottom = 150, Top = 1200, FractionHi = 0.4, LogKiHi = -9.3, LogKiLo = -6.7),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("competition_ki 1-site", fit_ki1)
print_nls("competition_ki 2-site", fit_ki2)
an_ki <- anova(fit_ki1, fit_ki2)
cat(sprintf("competition_ki extra-SS F=%.12f df=(%d,%d) p=%.12e\n",
            an_ki$F[2], an_ki$Df[2], an_ki$Res.Df[2], an_ki$`Pr(>F)`[2]))

cat("\n=== 8. extra_ss_f on NIST Rat43 (4PL vs 5PL, Bottom = 0) ===\n")
rat_x <- 1:15
rat_y <- c(16.08, 33.83, 65.80, 97.20, 191.55, 326.20, 386.87, 520.53,
           590.03, 651.92, 724.93, 699.56, 689.96, 637.56, 717.41)
fit_r4pl <- nls(
  rat_y ~ Top / (1 + 10^((LogEC50 - rat_x) * HillSlope)),
  start = list(Top = 700, LogEC50 = 6.5, HillSlope = 0.3),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
fit_r5pl <- nls(
  rat_y ~ Top / (1 + 10^(((LogEC50 + log10(2^(1 / S) - 1) / HillSlope) - rat_x) * HillSlope))^S,
  start = list(Top = 700, LogEC50 = 6.5, HillSlope = 0.4, S = 0.5),
  algorithm = "port", control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("NIST Rat43 4PL (Bottom=0)", fit_r4pl)
print_nls("NIST Rat43 5PL (Bottom=0)", fit_r5pl)
an_rat <- anova(fit_r4pl, fit_r5pl)
n_rat <- length(rat_y)
rss4 <- sum(residuals(fit_r4pl)^2); rss5 <- sum(residuals(fit_r5pl)^2)
aicc4 <- n_rat * log(rss4 / n_rat) + 2 * 4 + 2 * 4 * 5 / (n_rat - 4 - 1)
aicc5 <- n_rat * log(rss5 / n_rat) + 2 * 5 + 2 * 5 * 6 / (n_rat - 5 - 1)
cat(sprintf("Rat43 4PL vs 5PL: F=%.12f df=(%d,%d) p=%.12e AICc4=%.12f AICc5=%.12f\n",
            an_rat$F[2], an_rat$Df[2], an_rat$Res.Df[2], an_rat$`Pr(>F)`[2], aicc4, aicc5))

cat("\n=== 9. Noisy global_shared 3-curve one_phase_decay vs independent ===\n")
t_gs <- seq(0, 15, by = 1.5)
n_gs <- c(0.4, -0.5, 0.3, -0.2, 0.4, -0.3, 0.2, -0.4, 0.3, -0.2, 0.0)
gs_y <- c(
  7.5 + (130 - 7.5) * exp(-0.32 * t_gs) + n_gs,
  7.5 + (85  - 7.5) * exp(-0.32 * t_gs) - n_gs,
  7.5 + (48  - 7.5) * exp(-0.32 * t_gs) + n_gs
)
gs_t <- rep(t_gs, 3)
gs_g <- factor(rep(1:3, each = length(t_gs)))
fit_gs_sh <- nls(
  gs_y ~ Plateau + (Y0[gs_g] - Plateau) * exp(-K * gs_t),
  start = list(Y0 = c(130, 85, 48), Plateau = 7.5, K = 0.32),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
fit_gs_in <- nls(
  gs_y ~ Plateau[gs_g] + (Y0[gs_g] - Plateau[gs_g]) * exp(-K[gs_g] * gs_t),
  start = list(Y0 = c(130, 85, 48), Plateau = c(7.5, 7.5, 7.5), K = c(0.32, 0.32, 0.32)),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("global_shared 3-curve decay", fit_gs_sh)
print_nls("independent 3-curve decay", fit_gs_in)
an_gs <- anova(fit_gs_sh, fit_gs_in)
cat(sprintf("global_shared vs indep: F=%.12f df=(%d,%d) p=%.12e\n",
            an_gs$F[2], an_gs$Df[2], an_gs$Res.Df[2], an_gs$`Pr(>F)`[2]))

cat("\n=== 10. Noisy 4-curve Gaddum/Schild (SchildSlope = 1 vs free) ===\n")
log_ag <- seq(-10.0, -3.5, by = 0.5) # 14 points per curve -> 56 total
ant_c <- c(0.0, 1e-8, 1e-7, 1e-6)
sh_x <- rep(log_ag, times = 4)
sh_b <- rep(ant_c, each = length(log_ag))
sh_err <- rep(c(0.7, -0.6, 0.5, -0.8, 0.6, -0.5, 0.8, -0.7, 0.4, -0.6, 0.5, -0.4, 0.6, -0.5), times = 4) *
          rep(c(1.0, -1.0, 0.9, -0.8), each = length(log_ag))
# Generate with non-unity SchildSlope = 1.35 so extra-SS F detects departure from competitive unity!
dr_true <- ifelse(sh_b > 0, 1 + (sh_b * 10^8.2)^1.35, 1.0)
sh_y <- 8.0 + (102.0 - 8.0) / (1 + 10^(((-7.8 + log10(dr_true)) - sh_x) * 1.15)) + sh_err
fit_sh_u <- nls(
  sh_y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + ifelse(sh_b > 0, log10(1 + sh_b * 10^pA2), 0) - sh_x) * HillSlope)),
  start = list(Bottom = 8, Top = 102, LogEC50 = -7.8, HillSlope = 1.15, pA2 = 8.2),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
fit_sh_f <- nls(
  sh_y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + ifelse(sh_b > 0, log10(1 + (sh_b * 10^pA2)^SchildSlope), 0) - sh_x) * HillSlope)),
  start = list(Bottom = 8, Top = 102, LogEC50 = -7.8, HillSlope = 1.15, pA2 = 8.2, SchildSlope = 1.3),
  algorithm = "port",
  control = nls.control(tol = 1e-8, maxiter = 1000)
)
print_nls("Schild unity (SchildSlope=1)", fit_sh_u)
print_nls("Schild free (SchildSlope free)", fit_sh_f)
an_sh <- anova(fit_sh_u, fit_sh_f)
cat(sprintf("Schild unity vs free: F=%.12f df=(%d,%d) p=%.12e\n",
            an_sh$F[2], an_sh$Df[2], an_sh$Res.Df[2], an_sh$`Pr(>F)`[2]))
