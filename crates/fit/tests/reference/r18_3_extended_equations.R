# Reference generator for `d1_r18_3_extended_equations.rs`.
#
# Run under R 4.5.2 (2025-10-31):
#   Rscript crates/fit/tests/reference/r18_3_extended_equations.R
#
# Verifies all 34 R18.3 built-in equation library additions against independent
# R `stats::nls` / `stats::lm` estimates, asymptotic standard errors, and RSS
# on deterministic perturbed synthetic benchmarks (identical points in Rust).

options(digits = 15)
ctl <- nls.control(maxiter = 5000, tol = 1e-10, minFactor = 1e-12, scaleOffset = 1)

eps_vec <- function(n, amp) {
  i <- seq_len(n)
  amp * sin(2.3 * i + 0.7)
}

run_nls <- function(...) {
  r <- try(stats::nls(..., control = ctl), silent = TRUE)
  if (inherits(r, "try-error")) stats::nls(..., algorithm = "port", control = ctl) else r
}

emit_fit <- function(name, fit) {
  co <- summary(fit)$coefficients
  rss <- deviance(fit)
  cat(sprintf("=== %s | RSS=%.15e ===\n", name, rss))
  for (rn in rownames(co)) {
    cat(sprintf("  %-16s est=%.15e se=%.15e\n", rn, co[rn, 1], co[rn, 2]))
  }
}

# --- Group 1: Linear-concentration normalized 3PL/4PL (4 models) ---
x_c <- 10^seq(-3, 3, length.out = 15)

# 1. agonist_vs_response_normalized
y <- 100 * x_c / (2.5 + x_c) + eps_vec(length(x_c), 0.6)
emit_fit("agonist_vs_response_normalized",
         run_nls(y ~ 100 * x_c / (EC50 + x_c), start = list(EC50 = 1.5)))

# 2. agonist_vs_response_normalized_var
y <- 100 / (1 + (2.5 / x_c)^1.6) + eps_vec(length(x_c), 0.6)
emit_fit("agonist_vs_response_normalized_var",
         run_nls(y ~ 100 / (1 + (EC50 / x_c)^HillSlope), start = list(EC50 = 1.5, HillSlope = 1.2)))

# 3. inhibitor_vs_response_normalized
y <- 100 / (1 + x_c / 3.2) + eps_vec(length(x_c), 0.6)
emit_fit("inhibitor_vs_response_normalized",
         run_nls(y ~ 100 / (1 + x_c / IC50), start = list(IC50 = 2.0)))

# 4. inhibitor_vs_response_normalized_var (negative HillSlope convention)
y <- 100 / (1 + (3.2 / x_c)^(-1.4)) + eps_vec(length(x_c), 0.6)
emit_fit("inhibitor_vs_response_normalized_var",
         run_nls(y ~ 100 / (1 + (IC50 / x_c)^HillSlope), start = list(IC50 = 2.0, HillSlope = -1.0)))

# --- Group 2: Linear-concentration Special Dose-Response (8 models) ---
# 5. agonist_vs_response_5pl
x5 <- 10^seq(-3, 3, length.out = 19)
y <- 5 + (95 - 5) / (1 + (2^(1 / 1.8) - 1) * (1.5 / x5)^1.3)^1.8 + eps_vec(length(x5), 0.25)
emit_fit("agonist_vs_response_5pl",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (2^(1 / S) - 1) * (EC50 / x5)^HillSlope)^S,
                 start = list(Bottom = 4, Top = 96, EC50 = 1.2, HillSlope = 1.1, S = 1.5)))

# 6. biphasic_dose_response_conc
xb <- 10^seq(-4, 4, length.out = 25)
y <- 10 + 90 * (0.4 / (1 + (0.05 / xb)^1.2) + 0.6 / (1 + (50 / xb)^1.1)) + eps_vec(length(xb), 0.3)
emit_fit("biphasic_dose_response_conc",
         run_nls(y ~ Bottom + (Top - Bottom) * (Frac / (1 + (EC50_1 / xb)^nH1) + (1 - Frac) / (1 + (EC50_2 / xb)^nH2)),
                 start = list(Bottom = 8, Top = 102, Frac = 0.45, EC50_1 = 0.04, EC50_2 = 40, nH1 = 1.0, nH2 = 1.0)))

# 7. bell_shaped_dose_response_conc
y <- 90 + (10 - 90) / (1 + (xb / 0.1)^1.3) + (15 - 90) * (1 - 1 / (1 + (xb / 20)^1.1)) + eps_vec(length(xb), 0.3)
emit_fit("bell_shaped_dose_response_conc",
         run_nls(y ~ Dip + (Plateau1 - Dip) / (1 + (xb / EC50_1)^nH1) + (Plateau2 - Dip) * (1 - 1 / (1 + (xb / EC50_2)^nH2)),
                 start = list(Plateau1 = 12, Plateau2 = 18, Dip = 88, EC50_1 = 0.08, EC50_2 = 18, nH1 = 1.1, nH2 = 1.0)))

# 8. operational_model_agonist_conc (Emax fixed at 100, single partial agonist curve)
xo <- 10^seq(-3, 2, length.out = 16)
y <- 5 + (100 - 5) / (1 + ((2.0 + xo) / (1.5 * xo))^1.2) + eps_vec(length(xo), 0.25)
emit_fit("operational_model_agonist_conc",
         run_nls(y ~ Basal + (100 - Basal) / (1 + ((KA + xo) / (Tau * xo))^n),
                 start = list(Basal = 4, KA = 1.8, Tau = 1.4, n = 1.1)))

# 9. ec50_shift_gaddum_schild_conc (3 datasets with AntagConc = 0, 1e-7, 1e-6)
x_g <- 10^seq(-9, -4, length.out = 12)
antags <- c(0, 1e-7, 1e-6)
df_g <- do.call(rbind, lapply(seq_along(antags), function(k) {
  b_conc <- antags[k]
  dr <- if (b_conc > 0) 1 + (b_conc * 10^7.5)^1.15 else 1
  ec_eff <- 1e-7 * dr
  yv <- 5 + 95 / (1 + (ec_eff / x_g)^1.2) + eps_vec(length(x_g), 0.35) * (if (k %% 2 == 0) -1 else 1)
  data.frame(x = x_g, y = yv, antag = b_conc)
}))
emit_fit("ec50_shift_gaddum_schild_conc",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (EC50 * (1 + ifelse(antag > 0, (antag * 10^pA2)^SchildSlope, 0)) / x)^HillSlope),
                 data = df_g,
                 start = list(Bottom = 4, Top = 98, EC50 = 1.2e-7, HillSlope = 1.1, pA2 = 7.3, SchildSlope = 1.0)))

# 10. allosteric_ec50_shift_conc (3 datasets with ModConc = 0, 1e-6, 1e-5)
mods <- c(0, 1e-6, 1e-5)
df_a <- do.call(rbind, lapply(seq_along(mods), function(k) {
  m_conc <- mods[k]
  ec_eff <- 1e-7 * (1 + m_conc / 2e-6) / (1 + 0.1 * m_conc / 2e-6)
  yv <- 4 + 92 / (1 + (ec_eff / x_g)^1.1) + eps_vec(length(x_g), 0.35) * (if (k %% 2 == 0) -1 else 1)
  data.frame(x = x_g, y = yv, mod = m_conc)
}))
emit_fit("allosteric_ec50_shift_conc",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (EC50 * (1 + mod / Kb) / (1 + alpha * mod / Kb) / x)^HillSlope),
                 data = df_a,
                 start = list(Bottom = 3, Top = 95, EC50 = 1.1e-7, HillSlope = 1.0, Kb = 1.8e-6, alpha = 0.12)))

# 11. agonist_vs_response_ecf (F = 80 fixed)
y <- 5 + 95 / (1 + ((4.0 * ((100 - 80) / 80)^(1 / 1.4)) / x_c)^1.4) + eps_vec(length(x_c), 0.35)
emit_fit("agonist_vs_response_ecf",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + ((ECF * ((100 - 80) / 80)^(1 / HillSlope)) / x_c)^HillSlope),
                 start = list(Bottom = 4, Top = 98, ECF = 3.5, HillSlope = 1.2)))

# 12. absolute_ic50_4pl_conc (Control0 = 0, Control100 = 100 fixed; Bottom = 15, Top = 95)
fifty <- 50
ic_rel <- 5.0 * ((fifty - 15) / (95 - fifty))^(1 / 1.3)
y <- 15 + (95 - 15) / (1 + (x_c / ic_rel)^1.3) + eps_vec(length(x_c), 0.35)
emit_fit("absolute_ic50_4pl_conc",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (x_c / (IC50 * ((50 - Bottom) / (Top - 50))^(1 / HillSlope)))^HillSlope),
                 start = list(Bottom = 14, Top = 96, IC50 = 4.5, HillSlope = 1.2)))

# --- Group 3: Mechanistic pharmacology (6 models) ---
# 13 & 14. operational_model_depletion_log & _conc (2 datasets: control tau1=5, depleted tau2=0.8)
log_x <- seq(-8, -3, length.out = 14)
conc_x <- 10^log_x
taus <- c(5.0, 0.8)
df_dep <- do.call(rbind, lapply(seq_along(taus), function(k) {
  a <- conc_x
  yv <- 3 + (105 - 3) / (1 + ((1e-5 + a) / (taus[k] * a))^1.15) + eps_vec(length(a), 0.3) * (if (k == 2) -1 else 1)
  data.frame(lx = log_x, x = a, y = yv, ds = k)
}))
emit_fit("operational_model_depletion_log",
         run_nls(y ~ Basal + (EffectMax - Basal) / (1 + ((10^LogKA + 10^lx) / (ifelse(ds == 1, 10^LogTau1, 10^LogTau2) * 10^lx))^n),
                 data = df_dep,
                 start = list(Basal = 2, EffectMax = 102, LogKA = -5.1, LogTau1 = 0.65, LogTau2 = -0.12, n = 1.1)))
emit_fit("operational_model_depletion_conc",
         run_nls(y ~ Basal + (EffectMax - Basal) / (1 + ((KA + x) / (ifelse(ds == 1, Tau1, Tau2) * x))^n),
                 data = df_dep,
                 start = list(Basal = 2, EffectMax = 102, KA = 9e-6, Tau1 = 4.5, Tau2 = 0.75, n = 1.1)))

# 15 & 16. ec50_shift_gaddum_schild_unity_log & _conc (SchildSlope = 1)
df_su <- do.call(rbind, lapply(seq_along(antags), function(k) {
  b_conc <- antags[k]
  dr <- 1 + b_conc * 10^7.8
  ec_eff <- 1e-7 * dr
  yv <- 5 + 95 / (1 + (ec_eff / x_g)^1.2) + eps_vec(length(x_g), 0.35) * (if (k %% 2 == 0) -1 else 1)
  data.frame(lx = log10(x_g), x = x_g, y = yv, antag = b_conc)
}))
emit_fit("ec50_shift_gaddum_schild_unity_log",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + antag * 10^pKB) - lx) * HillSlope)),
                 data = df_su,
                 start = list(Bottom = 4, Top = 98, LogEC50 = -7.1, HillSlope = 1.1, pKB = 7.6)))
emit_fit("ec50_shift_gaddum_schild_unity_conc",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (EC50 * (1 + antag * 10^pKB) / x)^HillSlope),
                 data = df_su,
                 start = list(Bottom = 4, Top = 98, EC50 = 1.1e-7, HillSlope = 1.1, pKB = 7.6)))

# 17 & 18. ec50_shift_ratio_log & _conc (dataset 1 = ctrl, dataset 2 = 6.5x shifted)
df_sr <- do.call(rbind, lapply(1:2, function(k) {
  ec_eff <- if (k == 1) 1e-7 else 1e-7 * 6.5
  yv <- 4 + 94 / (1 + (ec_eff / x_g)^1.25) + eps_vec(length(x_g), 0.35) * (if (k == 2) -1 else 1)
  data.frame(lx = log10(x_g), x = x_g, y = yv, shifted = as.numeric(k == 2))
}))
emit_fit("ec50_shift_ratio_log",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50Control + shifted * log10(EC50Ratio) - lx) * HillSlope)),
                 data = df_sr,
                 start = list(Bottom = 3, Top = 96, LogEC50Control = -7.1, HillSlope = 1.1, EC50Ratio = 5.5)))
emit_fit("ec50_shift_ratio_conc",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + (EC50Control * ifelse(shifted == 1, EC50Ratio, 1) / x)^HillSlope),
                 data = df_sr,
                 start = list(Bottom = 3, Top = 96, EC50Control = 1.1e-7, HillSlope = 1.1, EC50Ratio = 5.5)))

# --- Group 4: Receptor binding (3 models) ---
# 19. one_site_total_ligand_depletion (SpecAct=2000 cpm/fmol, Vol=0.5 mL fixed)
x_cpm <- seq(500, 15000, length.out = 16)
swillens_y <- function(x, Bmax, Kd, NS, SpecAct = 2000, Vol = 0.5) {
  kd_cpm <- Kd * Vol * 1000 * SpecAct
  a <- -(1 + NS)
  b <- kd_cpm * (1 + NS) + x * (1 + 2 * NS) + Bmax
  c <- -x * (NS * kd_cpm + NS * x + Bmax)
  (-b + sqrt(b^2 - 4 * a * c)) / (2 * a)
}
y <- swillens_y(x_cpm, 3200, 0.0025, 0.04) + eps_vec(length(x_cpm), 8.0)
emit_fit("one_site_total_ligand_depletion",
         run_nls(y ~ swillens_y(x_cpm, Bmax, Kd, NS),
                 start = list(Bmax = 3000, Kd = 0.002, NS = 0.03)))

# 20. two_sites_fit_logic50
x_comp <- seq(-11, -3, length.out = 21)
y <- 5 + 95 * (0.35 / (1 + 10^(x_comp - (-8.5))) + 0.65 / (1 + 10^(x_comp - (-5.5)))) + eps_vec(length(x_comp), 0.3)
emit_fit("two_sites_fit_logic50",
         run_nls(y ~ Bottom + (Top - Bottom) * (FractionHi / (1 + 10^(x_comp - LogIC50_Hi)) + (1 - FractionHi) / (1 + 10^(x_comp - LogIC50_Lo))),
                 start = list(Bottom = 4, Top = 98, FractionHi = 0.4, LogIC50_Hi = -8.3, LogIC50_Lo = -5.7)))

# 21. allosteric_modulator_competition (RadioligandNM=1.0, HotKdNM=2.0 fixed)
x_mod <- seq(-9, -3, length.out = 16)
allo_y <- function(x, Y0, LogKB, LogAlpha, hot = 1.0, kd = 2.0) {
  u <- 10^(x - LogKB)
  k_app <- kd * (1 + u) / (1 + 10^LogAlpha * u)
  Y0 * (hot + kd) / (hot + k_app)
}
y <- allo_y(x_mod, 100, -6.2, -0.8) + eps_vec(length(x_mod), 0.35)
emit_fit("allosteric_modulator_competition",
         run_nls(y ~ allo_y(x_mod, Y0, LogKB, LogAlpha),
                 start = list(Y0 = 98, LogKB = -6.0, LogAlpha = -0.6)))

# --- Group 5: Curves & radiobiology (13 models) ---
# 22. two_phase_association
t_a <- seq(0, 12, length.out = 25)
y <- 10 + 90 * (0.4 * (1 - exp(-2.5 * t_a)) + 0.6 * (1 - exp(-0.35 * t_a))) + eps_vec(length(t_a), 0.25)
emit_fit("two_phase_association",
         run_nls(y ~ Y0 + (Plateau - Y0) * (PercentFast / 100 * (1 - exp(-KFast * t_a)) + (1 - PercentFast / 100) * (1 - exp(-KSlow * t_a))),
                 start = list(Y0 = 9, Plateau = 98, PercentFast = 45, KFast = 2.2, KSlow = 0.3)))

# 23. two_lines_intersection (2 datasets crossing at XCross=4.2, YCross=11.5, Slope1=1.8, Slope2=-0.9)
x_l <- seq(0, 10, length.out = 11)
df_l <- rbind(
  data.frame(x = x_l, y = 11.5 + 1.8 * (x_l - 4.2) + eps_vec(length(x_l), 0.2), ds = 1),
  data.frame(x = x_l, y = 11.5 - 0.9 * (x_l - 4.2) - eps_vec(length(x_l), 0.2), ds = 2)
)
emit_fit("two_lines_intersection",
         run_nls(y ~ YCross + ifelse(ds == 1, Slope1, Slope2) * (x - XCross),
                 data = df_l,
                 start = list(XCross = 4.0, YCross = 11.0, Slope1 = 1.5, Slope2 = -1.0)))

# 24. gaussian_area
x_g2 <- seq(-4, 10, length.out = 29)
y <- (25 / (1.8 * sqrt(2 * pi))) * exp(-0.5 * ((x_g2 - 3.2) / 1.8)^2) + eps_vec(length(x_g2), 0.08)
emit_fit("gaussian_area",
         run_nls(y ~ (Area / (SD * sqrt(2 * pi))) * exp(-0.5 * ((x_g2 - Mean) / SD)^2),
                 start = list(Area = 22, Mean = 3.0, SD = 1.6)))

# 25. lognormal_area
x_ln <- seq(0.5, 16, length.out = 24)
y <- (40 / x_ln) * exp(-0.5 * (log(x_ln / 4.5) / log(1.85))^2) + eps_vec(length(x_ln), 0.1)
emit_fit("lognormal_area",
         run_nls(y ~ (A / x_ln) * exp(-0.5 * (log(x_ln / GeoMean) / log(GeoSD))^2),
                 start = list(A = 38, GeoMean = 4.2, GeoSD = 1.75)))

# 26. standard_sine_wave
x_sw <- seq(0, 12, length.out = 25)
y <- 4.5 * sin(2 * pi * x_sw / 5.2 + 0.6) + eps_vec(length(x_sw), 0.12)
emit_fit("standard_sine_wave",
         run_nls(y ~ Amplitude * sin(2 * pi * x_sw / Wavelength + PhaseShift),
                 start = list(Amplitude = 4.2, Wavelength = 5.0, PhaseShift = 0.5)))

# 27. log_exponential_growth
x_eg <- seq(0, 10, length.out = 15)
y <- 1.4 + 0.32 * x_eg + eps_vec(length(x_eg), 0.08)
emit_fit("log_exponential_growth", lm(y ~ x_eg))

# 28-32. 5 Linear-Quadratic variants
d_rad <- seq(0, 8, length.out = 17)
s_true <- exp(-(0.25 * d_rad + 0.04 * d_rad^2))

y <- 100 * s_true + eps_vec(length(d_rad), 0.4)
emit_fit("linear_quadratic_percent_surviving",
         run_nls(y ~ 100 * exp(-(Alpha * d_rad + Beta * d_rad^2)), start = list(Alpha = 0.2, Beta = 0.03)))

y <- 250 * s_true + eps_vec(length(d_rad), 1.2)
emit_fit("linear_quadratic_number_surviving",
         run_nls(y ~ Y0 * exp(-(Alpha * d_rad + Beta * d_rad^2)), start = list(Y0 = 240, Alpha = 0.2, Beta = 0.03)))

y <- (1 - s_true) + eps_vec(length(d_rad), 0.004)
emit_fit("linear_quadratic_fraction_dead",
         run_nls(y ~ 1 - exp(-(Alpha * d_rad + Beta * d_rad^2)), start = list(Alpha = 0.2, Beta = 0.03)))

y <- 100 * (1 - s_true) + eps_vec(length(d_rad), 0.4)
emit_fit("linear_quadratic_percent_dead",
         run_nls(y ~ 100 * (1 - exp(-(Alpha * d_rad + Beta * d_rad^2))), start = list(Alpha = 0.2, Beta = 0.03)))

y <- 250 * (1 - s_true) + eps_vec(length(d_rad), 1.2)
emit_fit("linear_quadratic_number_dead",
         run_nls(y ~ Y0 * (1 - exp(-(Alpha * d_rad + Beta * d_rad^2))), start = list(Y0 = 240, Alpha = 0.2, Beta = 0.03)))

# 33. boltzmann_sigmoid
x_v <- seq(-80, 40, length.out = 25)
y <- 2.0 + (48.0 - 2.0) / (1 + exp((-22.5 - x_v) / 8.4)) + eps_vec(length(x_v), 0.25)
emit_fit("boltzmann_sigmoid",
         run_nls(y ~ Bottom + (Top - Bottom) / (1 + exp((V50 - x_v) / Slope)),
                 start = list(Bottom = 1.5, Top = 47, V50 = -20, Slope = 8)))

# 34. power_series_two_term
x_ps <- seq(0.5, 6.0, length.out = 20)
y <- 2.4 * x_ps^0.7 + 0.35 * x_ps^2.2 + eps_vec(length(x_ps), 0.08)
emit_fit("power_series_two_term",
         run_nls(y ~ A * x_ps^B + C * x_ps^D,
                 start = list(A = 2.2, B = 0.65, C = 0.3, D = 2.1)))
