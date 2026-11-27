# Independent R 4.5.2 (2025-10-31) black-box reference generator for CurveBook
# R23.6 (`crates/fit`):
#   1. Relational inequality constraints (KFast > KSlow on two-phase decay;
#      KdLo > KdHi on two-site binding & LogKiLo > LogKiHi on two-site Ki)
#   2. Derived-scalar CIs:
#      - 1-parameter monotonic transforms (10^LogEC50; 10^(-pKB) & ln(2)/K
#        with exact bound reversal on negative derivative)
#      - Multi-parameter delta-method CIs (Span = Top - Bottom; Logistic &
#        Gompertz analytical inflection points)
#   3. All 8 fixed canonical built-in equations (no backwards compat on the
#      8 diverged forms):
#      - operational_model_agonist (<A> full logistic vs <~A> partial agonist)
#        & operational_model_agonist_conc
#      - log_agonist_vs_response_5pl parameterized directly in LogEC50
#      - two_sites_fit_ki with separate HotKdNMHi and HotKdNMLo
#      - association_kinetics_one_conc parameterized in Kon and Bmax
#      - allosteric_ec50_shift (log-X) parameterized in LogKB and Logalpha
#      - lorentzian parameterized in HWHM (Width = HWHM)
#      - sine_wave, damped_sine_wave, sinc_wave parameterized in Wavelength
#      - logistic_growth & gompertz_growth with analytical inflection points
#   4. All 7 new modern built-in equations:
#      - pade_1_1, horizontal_line, semilog_x_lin_y_log, sum_of_two_lorentzians,
#        cumulative_gaussian_percent, cumulative_gaussian_fraction,
#        cumulative_gaussian_counts
#   5. Key legacy reparameterizations (classic 1-/2-phase decay & association,
#      classic exponential growth, classic 1-/2-site binding hyperbola)
#   6. GOF metrics (Adjusted R^2, Willett & Singer 1988 Weighted R^2, RMSE &
#      Sy.x) and Poisson nonlinear regression (NLL, deviance, pseudo-R^2,
#      Pearson chi-square, dispersion)
#   7. §6.2 continuous (z, t, F, chi^2) tail-shading & discrete (Binomial,
#      Poisson) theoretical distribution plotting grids
#
# Run under R 4.5.2:
#   /usr/local/bin/Rscript crates/fit/tests/reference/r23_fit_oracles.R

if (getRversion() != "4.5.2") {
  stop(sprintf("Expected R 4.5.2, found %s", as.character(getRversion())))
}

options(digits = 16)
ctl <- stats::nls.control(maxiter = 5000, tol = 1e-11, minFactor = 1e-12, scaleOffset = 1)

# Minimal dependency-free deterministic JSON serializer for base R 4.5.2.
json_escape <- function(s) {
  s <- gsub("\\\\", "\\\\\\\\", s)
  s <- gsub("\"", "\\\\\"", s)
  s <- gsub("\n", "\\\\n", s)
  s <- gsub("\r", "\\\\r", s)
  s <- gsub("\t", "\\\\t", s)
  paste0("\"", s, "\"")
}

to_json <- function(x, indent = 0L) {
  pad <- paste(rep("  ", indent), collapse = "")
  pad_in <- paste(rep("  ", indent + 1L), collapse = "")
  if (is.null(x)) return("null")
  if (is.list(x)) {
    if (length(x) == 0L) {
      return(if (!is.null(names(x))) "{}" else "[]")
    }
    nms <- names(x)
    if (!is.null(nms) && all(nzchar(nms))) {
      entries <- vapply(seq_along(x), function(i) {
        paste0(pad_in, json_escape(nms[i]), ": ", to_json(x[[i]], indent + 1L))
      }, character(1L))
      return(paste0("{\n", paste(entries, collapse = ",\n"), "\n", pad, "}"))
    } else {
      entries <- vapply(seq_along(x), function(i) {
        paste0(pad_in, to_json(x[[i]], indent + 1L))
      }, character(1L))
      return(paste0("[\n", paste(entries, collapse = ",\n"), "\n", pad, "]"))
    }
  }
  if (length(x) > 1L || inherits(x, "AsIs")) {
    vals <- vapply(seq_along(x), function(i) to_json(x[[i]], indent + 1L), character(1L))
    return(paste0("[", paste(vals, collapse = ", "), "]"))
  }
  if (is.logical(x)) {
    if (is.na(x)) return("null")
    return(if (isTRUE(x)) "true" else "false")
  }
  if (is.integer(x)) {
    if (is.na(x)) return("null")
    return(as.character(x))
  }
  if (is.numeric(x)) {
    if (is.na(x) || is.nan(x) || !is.finite(x)) return("null")
    if (x == 0) return("0")
    return(sprintf("%.17g", x))
  }
  if (is.character(x)) {
    if (is.na(x)) return("null")
    return(json_escape(x))
  }
  stop(sprintf("Unsupported type for JSON serialization: %s", typeof(x)))
}

as_arr <- function(v) I(v)

eps_vec <- function(n, amp) {
  i <- seq_len(n)
  amp * sin(2.3 * i + 0.7)
}

run_nls <- function(...) {
  r <- try(stats::nls(..., control = ctl), silent = TRUE)
  if (inherits(r, "try-error")) stats::nls(..., algorithm = "port", control = ctl) else r
}

summarize_nls <- function(fit, x_data = NULL, y_data = NULL, extra = NULL) {
  co <- summary(fit)$coefficients
  rss <- stats::deviance(fit)
  n <- length(stats::residuals(fit))
  k <- nrow(co)
  df_res <- n - k
  t_crit <- qt(0.975, df = df_res)
  ests <- unname(co[, 1L])
  ses <- unname(co[, 2L])
  params <- setNames(lapply(seq_len(k), function(i) {
    list(
      est = ests[i],
      se = ses[i],
      ci95_sym_lo = ests[i] - t_crit * ses[i],
      ci95_sym_hi = ests[i] + t_crit * ses[i]
    )
  }), rownames(co))
  res <- list(
    n = as.integer(n),
    k = as.integer(k),
    df = as.integer(df_res),
    rss = rss,
    rmse = sqrt(rss / n),
    sy_x = sqrt(rss / df_res),
    params = params
  )
  if (!is.null(x_data)) res$x <- as_arr(x_data)
  if (!is.null(y_data)) {
    res$y <- as_arr(y_data)
    tss <- sum((y_data - mean(y_data))^2)
    r2 <- 1 - rss / tss
    adj_r2 <- 1 - (1 - r2) * (n - 1) / (n - k)
    res$r_squared <- r2
    res$adj_r_squared <- adj_r2
  }
  if (!is.null(extra)) res <- c(res, extra)
  res
}

# ============================================================================
# 1. RELATIONAL CONSTRAINTS (KFast > KSlow; KdLo > KdHi)
# ============================================================================

# 1A. Two-phase exponential decay with KFast > KSlow (gap reparameterization)
t_dec <- seq(0, 12, length.out = 25)
y_dec <- 12 + 55 * exp(-1.8 * t_dec) + 33 * exp(-0.22 * t_dec) + eps_vec(length(t_dec), 0.30)
fit_dec_direct <- run_nls(
  y_dec ~ Plateau + SpanFast * exp(-KFast * t_dec) + SpanSlow * exp(-KSlow * t_dec),
  start = list(Plateau = 10, SpanFast = 50, SpanSlow = 35, KFast = 1.5, KSlow = 0.2)
)
fit_dec_reparam <- run_nls(
  y_dec ~ Plateau + SpanFast * exp(-(KSlow + exp(LogDeltaK)) * t_dec) + SpanSlow * exp(-KSlow * t_dec),
  start = list(Plateau = 10, SpanFast = 50, SpanSlow = 35, KSlow = 0.2, LogDeltaK = log(1.3))
)
# Active relational gap constraint KFast >= KSlow + 2.0 (where unconstrained gap is ~1.58 < 2.0)
fit_dec_active_gap <- stats::nls(
  y_dec ~ Plateau + SpanFast * exp(-(KSlow + DeltaK) * t_dec) + SpanSlow * exp(-KSlow * t_dec),
  start = list(Plateau = 10, SpanFast = 50, SpanSlow = 35, KSlow = 0.2, DeltaK = 2.5),
  algorithm = "port",
  lower = c(-Inf, 0, 0, 1e-6, 2.0),
  upper = c(Inf, Inf, Inf, Inf, Inf),
  control = ctl
)
# Verify reparameterized SE back-transformation matches direct fit via delta method:
# KFast = KSlow + exp(LogDeltaK) => grad = [0, 0, 0, 1, exp(LogDeltaK)]
V_rep <- vcov(fit_dec_reparam)
co_rep <- coef(fit_dec_reparam)
g_kfast <- c(0, 0, 0, 1, exp(co_rep["LogDeltaK"]))
se_kfast_reparam <- sqrt(as.numeric(t(g_kfast) %*% V_rep %*% g_kfast))
se_kfast_direct <- summary(fit_dec_direct)$coefficients["KFast", 2L]
if (abs(se_kfast_reparam - se_kfast_direct) > 1e-8) {
  stop("Chain-rule SE mismatch on relational reparameterization")
}

# 1B. Two-site binding hyperbola with KdLo > KdHi
x_bind <- c(0.05, 0.1, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512)
y_bind <- 45 * x_bind / (0.6 + x_bind) + 95 * x_bind / (38.0 + x_bind) + eps_vec(length(x_bind), 0.45)
fit_bind_rel <- run_nls(
  y_bind ~ BmaxHi * x_bind / (KdHi + x_bind) + BmaxLo * x_bind / ((KdHi + exp(LogGapKd)) + x_bind),
  start = list(BmaxHi = 40, BmaxLo = 90, KdHi = 0.5, LogGapKd = log(35.0))
)
fit_bind_direct <- run_nls(
  y_bind ~ BmaxHi * x_bind / (KdHi + x_bind) + BmaxLo * x_bind / (KdLo + x_bind),
  start = list(BmaxHi = 40, BmaxLo = 90, KdHi = 0.5, KdLo = 35.0)
)

relational_constraints_out <- list(
  two_phase_decay_kfast_gt_kslow = list(
    direct = summarize_nls(fit_dec_direct, t_dec, y_dec),
    reparam_gap_se_kfast = se_kfast_reparam,
    active_min_gap_2_0 = summarize_nls(fit_dec_active_gap, t_dec, y_dec,
                                       extra = list(kfast_constrained = unname(coef(fit_dec_active_gap)["KSlow"] + coef(fit_dec_active_gap)["DeltaK"])))
  ),
  two_site_binding_kdlo_gt_kdhi = list(
    direct = summarize_nls(fit_bind_direct, x_bind, y_bind),
    reparam_kdlo = unname(coef(fit_bind_rel)["KdHi"] + exp(coef(fit_bind_rel)["LogGapKd"])),
    reparam_rss = stats::deviance(fit_bind_rel)
  )
)

# ============================================================================
# 2. DERIVED-SCALAR CIs (Monotonic Bound Reversal + Multi-Param Delta Method)
# ============================================================================

# 2A. 4PL Log-Dose Response: EC50 = 10^LogEC50 (increasing) & Span = Top - Bottom (delta-method)
x_4pl <- seq(-9, -4, length.out = 16)
y_4pl <- 8.0 + (96.0 - 8.0) / (1 + 10^((-6.4 - x_4pl) * 1.35)) + eps_vec(length(x_4pl), 0.5)
fit_4pl <- run_nls(
  y_4pl ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - x_4pl) * HillSlope)),
  start = list(Bottom = 6, Top = 95, LogEC50 = -6.5, HillSlope = 1.2)
)
co_4pl <- summary(fit_4pl)$coefficients
V_4pl <- vcov(fit_4pl)
df_4pl <- length(x_4pl) - 4L
t_4pl <- qt(0.975, df = df_4pl)
log_ec50 <- unname(co_4pl["LogEC50", 1L])
se_log_ec50 <- unname(co_4pl["LogEC50", 2L])
ci_log_ec50 <- c(log_ec50 - t_4pl * se_log_ec50, log_ec50 + t_4pl * se_log_ec50)
ec50_est <- 10^log_ec50
ec50_se_delta <- ec50_est * log(10) * se_log_ec50

span_est <- unname(co_4pl["Top", 1L] - co_4pl["Bottom", 1L])
g_span <- c(-1, 1, 0, 0) # [Bottom, Top, LogEC50, HillSlope]
span_se <- sqrt(as.numeric(t(g_span) %*% V_4pl %*% g_span))
span_ci <- c(span_est - t_4pl * span_se, span_est + t_4pl * span_se)

# 2B. Gaddum/Schild Unity Slope: KB = 10^(-pKB) (decreasing -> BOUND REVERSAL!)
x_gs <- seq(-9, -4, length.out = 12)
antags <- c(0, 1e-7, 1e-6)
df_gs <- do.call(rbind, lapply(seq_along(antags), function(k) {
  b <- antags[k]
  yv <- 5 + 95 / (1 + 10^((-7.0 + log10(1 + b * 10^7.8) - x_gs) * 1.2)) +
    eps_vec(length(x_gs), 0.35) * (if (k %% 2 == 0) -1 else 1)
  data.frame(lx = x_gs, y = yv, antag = b)
}))
fit_gs <- run_nls(
  y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + antag * 10^pKB) - lx) * HillSlope)),
  data = df_gs,
  start = list(Bottom = 4, Top = 98, LogEC50 = -7.1, HillSlope = 1.1, pKB = 7.6)
)
co_gs <- summary(fit_gs)$coefficients
df_gs_res <- nrow(df_gs) - 5L
t_gs <- qt(0.975, df = df_gs_res)
pkb_est <- unname(co_gs["pKB", 1L]); pkb_se <- unname(co_gs["pKB", 2L])
pkb_ci <- c(pkb_est - t_gs * pkb_se, pkb_est + t_gs * pkb_se)
kb_est <- 10^(-pkb_est)
kb_ci_reversed <- c(10^(-pkb_ci[2L]), 10^(-pkb_ci[1L]))
kb_se_delta <- kb_est * log(10) * pkb_se

# 2C. One-Phase Decay: HalfLife = ln(2)/K (decreasing -> BOUND REVERSAL!) & Span = Y0 - Plateau
t_1p <- seq(0, 10, length.out = 16)
y_1p <- 15 + (95 - 15) * exp(-0.45 * t_1p) + eps_vec(length(t_1p), 0.4)
fit_1p <- run_nls(
  y_1p ~ Plateau + (Y0 - Plateau) * exp(-K * t_1p),
  start = list(Y0 = 92, Plateau = 14, K = 0.4)
)
co_1p <- summary(fit_1p)$coefficients
V_1p <- vcov(fit_1p)
df_1p <- length(t_1p) - 3L
t_1p_crit <- qt(0.975, df = df_1p)
k_est <- unname(co_1p["K", 1L]); k_se <- unname(co_1p["K", 2L])
k_ci <- c(k_est - t_1p_crit * k_se, k_est + t_1p_crit * k_se)
hl_est <- log(2) / k_est
hl_ci_reversed <- c(log(2) / k_ci[2L], log(2) / k_ci[1L])
hl_se_delta <- (log(2) / (k_est^2)) * k_se
span1p_est <- unname(co_1p["Y0", 1L] - co_1p["Plateau", 1L])
g_span1p <- c(1, -1, 0)
span1p_se <- sqrt(as.numeric(t(g_span1p) %*% V_1p %*% g_span1p))
span1p_ci <- c(span1p_est - t_1p_crit * span1p_se, span1p_est + t_1p_crit * span1p_se)

derived_scalar_cis_out <- list(
  four_pl_ec50_and_span = list(
    fit = summarize_nls(fit_4pl, x_4pl, y_4pl),
    ec50_est = ec50_est,
    ec50_monotonic_ci95 = as_arr(c(10^ci_log_ec50[1L], 10^ci_log_ec50[2L])),
    ec50_se_delta = ec50_se_delta,
    span_est = span_est,
    span_se_delta = span_se,
    span_ci95_delta = as_arr(span_ci)
  ),
  schild_unity_kb_bound_reversal = list(
    fit = summarize_nls(fit_gs),
    pkb_ci95 = as_arr(pkb_ci),
    kb_est = kb_est,
    kb_monotonic_ci95_reversed = as_arr(kb_ci_reversed),
    kb_se_delta = kb_se_delta
  ),
  one_phase_decay_halflife_and_span = list(
    fit = summarize_nls(fit_1p, t_1p, y_1p),
    k_ci95 = as_arr(k_ci),
    halflife_est = hl_est,
    halflife_monotonic_ci95_reversed = as_arr(hl_ci_reversed),
    halflife_se_delta = hl_se_delta,
    span_est = span1p_est,
    span_se_delta = span1p_se,
    span_ci95_delta = as_arr(span1p_ci)
  )
)

# ============================================================================
# 3. ALL 8 FIXED CANONICAL BUILT-IN EQUATIONS (NO BACKWARDS COMPAT!)
# ============================================================================

# 3.1a operational_model_agonist (log-X): <A> full agonist logistic vs <~A> partial agonist
lx_op <- seq(-9, -4, length.out = 15)
df_op_log <- rbind(
  data.frame(
    x = lx_op, ds = 1L,
    y = 4.0 + (102.0 - 4.0) / (1 + 10^((-7.2 - lx_op) * 1.15)) + eps_vec(length(lx_op), 0.35)
  ),
  data.frame(
    x = lx_op, ds = 2L,
    y = 4.0 + (102.0 - 4.0) / (1 + ((10^(-5.8) + 10^lx_op) / (10^(0.15 + lx_op)))^1.15) - eps_vec(length(lx_op), 0.35)
  )
)
fit_op_log <- run_nls(
  y ~ ifelse(
    ds == 1L,
    Basal + (EffectMax - Basal) / (1 + 10^((LogEC50 - x) * n)),
    Basal + (EffectMax - Basal) / (1 + ((10^LogKA + 10^x) / (10^(LogTau + x)))^n)
  ),
  data = df_op_log,
  start = list(Basal = 3, EffectMax = 100, LogEC50 = -7.1, LogKA = -5.7, LogTau = 0.1, n = 1.1)
)

# 3.1b operational_model_agonist_conc (linear-X): <A> full agonist EC50 vs <~A> partial agonist KA, Tau
cx_op <- 10^lx_op
df_op_conc <- data.frame(x = rep(cx_op, 2L), ds = df_op_log$ds, y = df_op_log$y)
fit_op_conc <- run_nls(
  y ~ ifelse(
    ds == 1L,
    Basal + (EffectMax - Basal) / (1 + (EC50 / x)^n),
    Basal + (EffectMax - Basal) / (1 + ((KA + x) / (Tau * x))^n)
  ),
  data = df_op_conc,
  start = list(Basal = 3, EffectMax = 100, EC50 = 10^(-7.1), KA = 10^(-5.7), Tau = 10^(0.1), n = 1.1)
)

# 3.2 log_agonist_vs_response_5pl parameterized directly in LogEC50
x_5pl <- seq(-9, -3, length.out = 21)
eval_5pl_logec50 <- function(x, Bottom, Top, LogEC50, HillSlope, S) {
  log_xb <- LogEC50 + (1 / HillSlope) * log10(2^(1 / S) - 1)
  Bottom + (Top - Bottom) / (1 + 10^((log_xb - x) * HillSlope))^S
}
y_5pl <- eval_5pl_logec50(x_5pl, 5.0, 95.0, -6.2, 1.3, 1.8) + eps_vec(length(x_5pl), 0.25)
fit_5pl_logec50 <- run_nls(
  y_5pl ~ eval_5pl_logec50(x_5pl, Bottom, Top, LogEC50, HillSlope, S),
  start = list(Bottom = 4, Top = 96, LogEC50 = -6.0, HillSlope = 1.1, S = 1.5)
)

# 3.3 two_sites_fit_ki with separate HotKdNMHi and HotKdNMLo
x_2ki <- seq(-11, -3, length.out = 21)
hot_nm <- 2.0; hot_kd_hi <- 1.5; hot_kd_lo <- 12.0
eval_2ki <- function(x, Top, Bottom, FractionHi, LogKiHi, LogKiLo) {
  log_ic50_hi <- log10((10^LogKiHi) * (1 + hot_nm / hot_kd_hi))
  log_ic50_lo <- log10((10^LogKiLo) * (1 + hot_nm / hot_kd_lo))
  Bottom + (Top - Bottom) * (FractionHi / (1 + 10^(x - log_ic50_hi)) +
                               (1 - FractionHi) / (1 + 10^(x - log_ic50_lo)))
}
y_2ki <- eval_2ki(x_2ki, 100.0, 5.0, 0.38, -8.8, -5.8) + eps_vec(length(x_2ki), 0.3)
fit_2ki <- run_nls(
  y_2ki ~ eval_2ki(x_2ki, Top, Bottom, FractionHi, LogKiHi, LogKiLo),
  start = list(Top = 98, Bottom = 4, FractionHi = 0.4, LogKiHi = -8.6, LogKiLo = -5.9)
)

# 3.4 association_kinetics_one_conc parameterized in Kon and Bmax (fixed Koff, Hotnm)
t_ak <- seq(0, 60, length.out = 16)
koff_fix <- 0.05; hotnm_fix <- 10.0; L_fix <- hotnm_fix * 1e-9
eval_ak1 <- function(t, Kon, Bmax) {
  kd <- koff_fix / Kon
  kob <- Kon * L_fix + koff_fix
  occ <- L_fix / (L_fix + kd)
  (occ * Bmax) * (1 - exp(-kob * t))
}
y_ak <- eval_ak1(t_ak, 8e6, 240.0) + eps_vec(length(t_ak), 0.6)
fit_ak1 <- run_nls(
  y_ak ~ eval_ak1(t_ak, Kon, Bmax),
  start = list(Kon = 6e6, Bmax = 220.0)
)
kon_hat <- unname(coef(fit_ak1)["Kon"])

# 3.5 allosteric_ec50_shift (log-X) parameterized in LogKB and Logalpha
x_allo <- seq(-9, -4, length.out = 12)
b_concs <- c(0, 1e-6, 1e-5)
df_allo_log <- do.call(rbind, lapply(seq_along(b_concs), function(k) {
  b <- b_concs[k]
  antag <- (1 + b / 10^(-5.7)) / (1 + 10^(-1.0) * b / 10^(-5.7))
  log_ec <- log10(10^(-7.0) * antag)
  yv <- 4 + 92 / (1 + 10^((log_ec - x_allo) * 1.15)) +
    eps_vec(length(x_allo), 0.35) * (if (k %% 2 == 0) -1 else 1)
  data.frame(x = x_allo, y = yv, B = b)
}))
fit_allo_log <- run_nls(
  y ~ Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10((1 + B / 10^LogKB) / (1 + 10^Logalpha * B / 10^LogKB)) - x) * HillSlope)),
  data = df_allo_log,
  start = list(Bottom = 3, Top = 95, LogEC50 = -7.1, HillSlope = 1.0, LogKB = -5.6, Logalpha = -0.9)
)

# 3.6 lorentzian parameterized in HWHM (Width = HWHM, NOT FWHM!)
x_lor <- seq(-6, 14, length.out = 25)
y_lor <- 42.0 / (1 + ((x_lor - 4.2) / 1.8)^2) + eps_vec(length(x_lor), 0.18)
fit_lor_hwhm <- run_nls(
  y_lor ~ Amplitude / (1 + ((x_lor - Center) / Width)^2),
  start = list(Amplitude = 40, Center = 4.0, Width = 1.6)
)

# 3.7a sine_wave (nonzero baseline) parameterized in Wavelength
x_wav <- seq(0, 12, length.out = 25)
y_sine <- 4.5 * sin(2 * pi * x_wav / 5.2 + 0.6) + 12.0 + eps_vec(length(x_wav), 0.12)
fit_sine_wl <- run_nls(
  y_sine ~ Amplitude * sin(2 * pi * x_wav / Wavelength + PhaseShift) + Baseline,
  start = list(Amplitude = 4.2, Wavelength = 5.0, PhaseShift = 0.5, Baseline = 11.5)
)

# 3.7b damped_sine_wave parameterized in Wavelength (zero baseline canonical form)
y_dsine <- 6.0 * exp(-0.18 * x_wav) * sin(2 * pi * x_wav / 4.4 + 0.35) + eps_vec(length(x_wav), 0.08)
fit_dsine_wl <- run_nls(
  y_dsine ~ Amplitude * exp(-K * x_wav) * sin(2 * pi * x_wav / Wavelength + PhaseShift),
  start = list(Amplitude = 5.5, K = 0.15, Wavelength = 4.2, PhaseShift = 0.3)
)

# 3.7c sinc_wave parameterized in Wavelength
x_sinc <- seq(-6, 6, length.out = 25)
eval_sinc_wl <- function(x, Amplitude, Wavelength) {
  u <- 2 * pi * x / Wavelength
  ifelse(abs(u) < 1e-12, Amplitude, Amplitude * sin(u) / u)
}
y_sinc <- eval_sinc_wl(x_sinc, 15.0, 4.8) + eps_vec(length(x_sinc), 0.12)
fit_sinc_wl <- run_nls(
  y_sinc ~ eval_sinc_wl(x_sinc, Amplitude, Wavelength),
  start = list(Amplitude = 14.0, Wavelength = 4.5)
)

# 3.8a logistic_growth + analytical inflection point X_inflect = ln((YM - Y0)/Y0) / k
t_gr <- seq(0, 12, length.out = 19)
y_logg <- 120.0 * 4.0 / ((120.0 - 4.0) * exp(-0.65 * t_gr) + 4.0) + eps_vec(length(t_gr), 0.45)
fit_logg <- run_nls(
  y_logg ~ YM * Y0 / ((YM - Y0) * exp(-k * t_gr) + Y0),
  start = list(YM = 115, Y0 = 5, k = 0.6)
)
co_lg <- coef(fit_logg); V_lg <- vcov(fit_logg)
ym_l <- unname(co_lg["YM"]); y0_l <- unname(co_lg["Y0"]); k_l <- unname(co_lg["k"])
x_inf_lg <- log((ym_l - y0_l) / y0_l) / k_l
# Gradient of x_inf_lg wrt (YM, Y0, k):
# d/dYM = 1/(k*(YM-Y0)), d/dY0 = -YM/(k*Y0*(YM-Y0)), d/dk = -x_inf_lg/k
g_inf_lg <- c(1 / (k_l * (ym_l - y0_l)), -ym_l / (k_l * y0_l * (ym_l - y0_l)), -x_inf_lg / k_l)
se_inf_lg <- sqrt(as.numeric(t(g_inf_lg) %*% V_lg %*% g_inf_lg))
t_lg <- qt(0.975, df = length(t_gr) - 3L)

# 3.8b gompertz_growth + analytical inflection point X_inflect = ln(ln(YM/Y0)) / k
y_gomp <- 120.0 * (4.0 / 120.0)^(exp(-0.45 * t_gr)) + eps_vec(length(t_gr), 0.45)
fit_gomp <- run_nls(
  y_gomp ~ YM * (Y0 / YM)^(exp(-k * t_gr)),
  start = list(YM = 115, Y0 = 5, k = 0.4)
)
co_gp <- coef(fit_gomp); V_gp <- vcov(fit_gomp)
ym_g <- unname(co_gp["YM"]); y0_g <- unname(co_gp["Y0"]); k_g <- unname(co_gp["k"])
L_g <- log(ym_g / y0_g)
x_inf_gp <- log(L_g) / k_g
# Gradient of x_inf_gp = ln(ln(YM/Y0))/k wrt (YM, Y0, k):
g_inf_gp <- c(1 / (k_g * ym_g * L_g), -1 / (k_g * y0_g * L_g), -x_inf_gp / k_g)
se_inf_gp <- sqrt(as.numeric(t(g_inf_gp) %*% V_gp %*% g_inf_gp))
t_gp <- qt(0.975, df = length(t_gr) - 3L)

fixed_8_builtins_out <- list(
  operational_model_agonist_log = summarize_nls(fit_op_log, df_op_log$x, df_op_log$y,
                                                extra = list(dataset_idx = as_arr(df_op_log$ds - 1L))),
  operational_model_agonist_conc = summarize_nls(fit_op_conc, df_op_conc$x, df_op_conc$y,
                                                 extra = list(dataset_idx = as_arr(df_op_conc$ds - 1L))),
  log_agonist_vs_response_5pl_logec50 = summarize_nls(fit_5pl_logec50, x_5pl, y_5pl),
  two_sites_fit_ki_dual_kd = summarize_nls(fit_2ki, x_2ki, y_2ki,
                                           extra = list(HotNM = hot_nm, HotKdNMHi = hot_kd_hi, HotKdNMLo = hot_kd_lo)),
  association_kinetics_one_conc_kon_bmax = summarize_nls(
    fit_ak1, t_ak, y_ak,
    extra = list(Koff = koff_fix, Hotnm = hotnm_fix, derived_Kd = koff_fix / kon_hat, derived_Kobs = kon_hat * L_fix + koff_fix)
  ),
  allosteric_ec50_shift_logkb_logalpha = summarize_nls(fit_allo_log, df_allo_log$x, df_allo_log$y,
                                                       extra = list(mod_conc = as_arr(df_allo_log$B))),
  lorentzian_hwhm = summarize_nls(fit_lor_hwhm, x_lor, y_lor,
                                  extra = list(derived_fwhm = 2 * unname(coef(fit_lor_hwhm)["Width"]))),
  sine_wave_wavelength = summarize_nls(fit_sine_wl, x_wav, y_sine,
                                       extra = list(derived_frequency = 1 / unname(coef(fit_sine_wl)["Wavelength"]))),
  damped_sine_wave_wavelength = summarize_nls(fit_dsine_wl, x_wav, y_dsine,
                                              extra = list(derived_frequency = 1 / unname(coef(fit_dsine_wl)["Wavelength"]))),
  sinc_wave_wavelength = summarize_nls(fit_sinc_wl, x_sinc, y_sinc,
                                       extra = list(derived_frequency = 1 / unname(coef(fit_sinc_wl)["Wavelength"]))),
  logistic_growth_inflection = summarize_nls(
    fit_logg, t_gr, y_logg,
    extra = list(
      x_inflect_analytical = x_inf_lg,
      y_inflect_analytical = ym_l / 2,
      x_inflect_se_delta = se_inf_lg,
      x_inflect_ci95_delta = as_arr(c(x_inf_lg - t_lg * se_inf_lg, x_inf_lg + t_lg * se_inf_lg)),
      inv_k = 1 / k_l
    )
  ),
  gompertz_growth_inflection = summarize_nls(
    fit_gomp, t_gr, y_gomp,
    extra = list(
      x_inflect_analytical = x_inf_gp,
      y_inflect_analytical = ym_g / exp(1),
      x_inflect_se_delta = se_inf_gp,
      x_inflect_ci95_delta = as_arr(c(x_inf_gp - t_gp * se_inf_gp, x_inf_gp + t_gp * se_inf_gp)),
      inv_k = 1 / k_g
    )
  )
)

# ============================================================================
# 4. ALL 7 NEW MODERN BUILT-IN EQUATIONS
# ============================================================================

# 4.1 pade_1_1: Y = (A0 + A1 * X) / (1 + B1 * X)
x_pad <- seq(0, 10, length.out = 16)
y_pad <- (2.5 + 8.4 * x_pad) / (1 + 0.42 * x_pad) + eps_vec(length(x_pad), 0.12)
fit_pad <- run_nls(y_pad ~ (A0 + A1 * x_pad) / (1 + B1 * x_pad),
                   start = list(A0 = 2.0, A1 = 8.0, B1 = 0.4))

# 4.2 horizontal_line: Y = Mean
x_hor <- seq(1, 12, by = 1)
y_hor <- 18.5 + eps_vec(length(x_hor), 0.9)
fit_hor <- stats::lm(y_hor ~ 1)
co_hor <- summary(fit_hor)$coefficients

# 4.3 semilog_x_lin_y_log: Y = 10^(Slope * X + YIntercept)
x_sl <- seq(0, 6, length.out = 15)
y_sl <- 10^(0.28 * x_sl + 0.45) + eps_vec(length(x_sl), 0.35)
fit_sl <- run_nls(y_sl ~ 10^(Slope * x_sl + YIntercept),
                  start = list(Slope = 0.25, YIntercept = 0.4))

# 4.4 sum_of_two_lorentzians (HWHM Width1, Width2)
x_2lor <- seq(0, 20, length.out = 31)
y_2lor <- 30.0 / (1 + ((x_2lor - 6.0) / 1.4)^2) + 22.0 / (1 + ((x_2lor - 13.5) / 2.1)^2) +
  eps_vec(length(x_2lor), 0.18)
fit_2lor <- run_nls(
  y_2lor ~ Amplitude1 / (1 + ((x_2lor - Center1) / Width1)^2) +
    Amplitude2 / (1 + ((x_2lor - Center2) / Width2)^2),
  start = list(Amplitude1 = 28, Center1 = 5.8, Width1 = 1.3, Amplitude2 = 20, Center2 = 13.2, Width2 = 2.0)
)

# 4.5-4.7 3 fixed-scale Cumulative Gaussian built-ins
x_cg <- seq(-2, 14, length.out = 19)
y_cg_pct <- 100 * pnorm((x_cg - 6.2) / 2.3) + eps_vec(length(x_cg), 0.4)
fit_cg_pct <- run_nls(y_cg_pct ~ 100 * pnorm((x_cg - Mean) / SD), start = list(Mean = 6.0, SD = 2.0))

y_cg_frac <- pnorm((x_cg - 6.2) / 2.3) + eps_vec(length(x_cg), 0.004)
fit_cg_frac <- run_nls(y_cg_frac ~ pnorm((x_cg - Mean) / SD), start = list(Mean = 6.0, SD = 2.0))

y_cg_cnt <- 250 * pnorm((x_cg - 6.2) / 2.3) + eps_vec(length(x_cg), 1.0)
fit_cg_cnt <- run_nls(y_cg_cnt ~ N * pnorm((x_cg - Mean) / SD), start = list(N = 240, Mean = 6.0, SD = 2.0))

new_7_builtins_out <- list(
  pade_1_1 = summarize_nls(fit_pad, x_pad, y_pad),
  horizontal_line = list(
    x = as_arr(x_hor), y = as_arr(y_hor),
    n = as.integer(length(y_hor)), k = 1L, df = as.integer(length(y_hor) - 1L),
    mean = unname(co_hor[1L, 1L]), se = unname(co_hor[1L, 2L]),
    rss = sum((y_hor - mean(y_hor))^2),
    rmse = sqrt(sum((y_hor - mean(y_hor))^2) / length(y_hor)),
    sy_x = sd(y_hor)
  ),
  semilog_x_lin_y_log = summarize_nls(fit_sl, x_sl, y_sl),
  sum_of_two_lorentzians = summarize_nls(fit_2lor, x_2lor, y_2lor),
  cumulative_gaussian_percent = summarize_nls(fit_cg_pct, x_cg, y_cg_pct),
  cumulative_gaussian_fraction = summarize_nls(fit_cg_frac, x_cg, y_cg_frac),
  cumulative_gaussian_counts = summarize_nls(fit_cg_cnt, x_cg, y_cg_cnt)
)

# ============================================================================
# 5. KEY LEGACY REPARAMETERIZATIONS
# ============================================================================

# Classic 1-phase decay: Span * exp(-K * X) + Plateau
fit_leg_1p_dec <- run_nls(y_1p ~ Span * exp(-K * t_1p) + Plateau,
                          start = list(Span = 78, K = 0.4, Plateau = 14))
# Classic 2-phase decay: Span1 * exp(-K1 * X) + Span2 * exp(-K2 * X) + Plateau
fit_leg_2p_dec <- run_nls(y_dec ~ Span1 * exp(-K1 * t_dec) + Span2 * exp(-K2 * t_dec) + Plateau,
                          start = list(Span1 = 50, K1 = 1.5, Span2 = 35, K2 = 0.2, Plateau = 10))
# Classic 1-phase association: Ymax * (1 - exp(-K * X))
y_1pa <- 85.0 * (1 - exp(-0.55 * t_1p)) + eps_vec(length(t_1p), 0.35)
fit_leg_1p_assoc <- run_nls(y_1pa ~ Ymax * (1 - exp(-K * t_1p)),
                            start = list(Ymax = 82, K = 0.5))
# Classic 2-phase association: Ymax1*(1 - exp(-K1*X)) + Ymax2*(1 - exp(-K2*X))
y_2pa <- 45.0 * (1 - exp(-2.2 * t_dec)) + 55.0 * (1 - exp(-0.3 * t_dec)) + eps_vec(length(t_dec), 0.3)
fit_leg_2p_assoc <- run_nls(y_2pa ~ Ymax1 * (1 - exp(-K1 * t_dec)) + Ymax2 * (1 - exp(-K2 * t_dec)),
                            start = list(Ymax1 = 42, K1 = 2.0, Ymax2 = 52, K2 = 0.28))
# Classic exponential growth: Start * exp(K * X)
x_eg <- seq(0, 6, length.out = 15)
y_eg <- 3.2 * exp(0.42 * x_eg) + eps_vec(length(x_eg), 0.25)
fit_leg_eg <- run_nls(y_eg ~ Start * exp(K * x_eg), start = list(Start = 3.0, K = 0.4))
# Classic 1-site binding hyperbola: Bmax * X / (Kd + X)
x_1b <- c(0.1, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64)
y_1b <- 120.0 * x_1b / (3.5 + x_1b) + eps_vec(length(x_1b), 0.5)
fit_leg_1b <- run_nls(y_1b ~ Bmax * x_1b / (Kd + x_1b), start = list(Bmax = 115, Kd = 3.0))

legacy_reparams_out <- list(
  classic_one_phase_decay = summarize_nls(fit_leg_1p_dec, t_1p, y_1p),
  classic_two_phase_decay = summarize_nls(fit_leg_2p_dec, t_dec, y_dec),
  classic_one_phase_association = summarize_nls(fit_leg_1p_assoc, t_1p, y_1pa),
  classic_two_phase_association = summarize_nls(fit_leg_2p_assoc, t_dec, y_2pa),
  classic_exponential_growth = summarize_nls(fit_leg_eg, x_eg, y_eg),
  classic_one_site_binding = summarize_nls(fit_leg_1b, x_1b, y_1b),
  classic_two_site_binding = summarize_nls(fit_bind_direct, x_bind, y_bind)
)

# ============================================================================
# 6. ADJUSTED R^2, WEIGHTED R^2 (WILLETT & SINGER 1988), RMSE, POISSON NONLINEAR
# ============================================================================

# Weighted nonlinear fit on 1-site hyperbola with 1/Y^2 and 1/|Y| weights
df_1b_w <- data.frame(x = x_1b, y = y_1b, w_y2 = 1 / (y_1b^2), w_y1 = 1 / abs(y_1b))
w_y2 <- df_1b_w$w_y2
w_y1 <- df_1b_w$w_y1
ctl_w <- stats::nls.control(maxiter = 5000, tol = 1e-10, minFactor = 1e-12)
fit_w_y2 <- stats::nls(y ~ Bmax * x / (Kd + x), data = df_1b_w, start = list(Bmax = 115, Kd = 3.0),
                       weights = w_y2, algorithm = "port", control = ctl_w)
fit_w_y1 <- stats::nls(y ~ Bmax * x / (Kd + x), data = df_1b_w, start = list(Bmax = 115, Kd = 3.0),
                       weights = w_y1, algorithm = "port", control = ctl_w)

weighted_gof <- function(fit, y, w) {
  yhat <- stats::fitted(fit)
  n <- length(y); k <- length(coef(fit))
  wrss <- sum(w * (y - yhat)^2)
  ybar_w <- sum(w * y) / sum(w)
  wtss <- sum(w * (y - ybar_w)^2)
  wr2 <- 1 - wrss / wtss
  wadj_r2 <- 1 - (1 - wr2) * (n - 1) / (n - k)
  urss <- sum((y - yhat)^2)
  list(
    params = as.list(coef(fit)),
    weighted_rss = wrss,
    weighted_tss = wtss,
    weighted_r2 = wr2,
    weighted_adj_r2 = wadj_r2,
    weighted_rmse = sqrt(wrss / n),
    weighted_sy_x = sqrt(wrss / (n - k)),
    unweighted_rmse = sqrt(urss / n)
  )
}

# Poisson nonlinear regression: Michaelis-Menten count data Y ~ Poisson(Vmax * X / (Km + X))
x_pois_nl <- c(0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0)
y_pois_nl <- c(4L, 9L, 11L, 17L, 21L, 28L, 33L, 41L, 44L, 50L, 53L, 58L)
pois_dev <- function(par) {
  vmax <- par[1]; km <- par[2]
  if (vmax <= 0 || km <= 0) return(1e12)
  mu <- vmax * x_pois_nl / (km + x_pois_nl)
  if (any(mu <= 0)) return(1e12)
  term <- ifelse(y_pois_nl == 0, 0, y_pois_nl * log(y_pois_nl / mu))
  2 * sum(term - (y_pois_nl - mu))
}
pois_nll <- function(par) {
  vmax <- par[1]; km <- par[2]
  mu <- vmax * x_pois_nl / (km + x_pois_nl)
  -sum(stats::dpois(y_pois_nl, lambda = mu, log = TRUE))
}
opt_pois <- stats::optim(c(Vmax = 60.0, Km = 5.0), pois_nll, method = "BFGS",
                         hessian = TRUE, control = list(reltol = 1e-15, abstol = 1e-15, maxit = 1000))
par_p <- opt_pois$par
mu_hat <- par_p["Vmax"] * x_pois_nl / (par_p["Km"] + x_pois_nl)
mu_null <- rep(mean(y_pois_nl), length(y_pois_nl))
nll_mod <- -sum(stats::dpois(y_pois_nl, lambda = mu_hat, log = TRUE))
nll_sat <- -sum(stats::dpois(y_pois_nl, lambda = pmax(y_pois_nl, 1e-300), log = TRUE))
nll_null <- -sum(stats::dpois(y_pois_nl, lambda = mu_null, log = TRUE))
dev_mod <- 2 * (nll_mod - nll_sat)
dev_null <- 2 * (nll_null - nll_sat)
pseudo_r2_dev <- 1 - dev_mod / dev_null
pearson_chi2_nl <- sum((y_pois_nl - mu_hat)^2 / mu_hat)
df_pois_nl <- length(y_pois_nl) - 2L
disp_pois_nl <- pearson_chi2_nl / df_pois_nl
# Fisher information covariance at MLE: (J^T diag(1/mu) J)^{-1}
J_p <- cbind(
  x_pois_nl / (par_p["Km"] + x_pois_nl),
  -par_p["Vmax"] * x_pois_nl / (par_p["Km"] + x_pois_nl)^2
)
fisher_cov <- solve(t(J_p) %*% diag(1 / mu_hat) %*% J_p)
se_fisher <- sqrt(diag(fisher_cov))
se_hess <- sqrt(diag(solve(opt_pois$hessian)))

gof_and_poisson_out <- list(
  weighted_1_over_y2 = weighted_gof(fit_w_y2, y_1b, w_y2),
  weighted_1_over_abs_y = weighted_gof(fit_w_y1, y_1b, w_y1),
  poisson_nonlinear_mm = list(
    x = as_arr(x_pois_nl),
    y = as_arr(y_pois_nl),
    vmax = unname(par_p["Vmax"]),
    km = unname(par_p["Km"]),
    se_fisher = as_arr(unname(se_fisher)),
    se_observed_hessian = as_arr(unname(se_hess)),
    nll = nll_mod,
    nll_saturated = nll_sat,
    nll_null = nll_null,
    deviance = dev_mod,
    null_deviance = dev_null,
    pseudo_r2_deviance = pseudo_r2_dev,
    pearson_chi2 = pearson_chi2_nl,
    df = as.integer(df_pois_nl),
    dispersion = disp_pois_nl
  )
)

# ============================================================================
# 7. §6.2 CONTINUOUS & DISCRETE THEORETICAL DISTRIBUTION GRIDS
# ============================================================================

# 7A. Continuous tail-shaded PDF grids
z_grid <- seq(-4, 4, by = 0.5)
z_cut <- qnorm(0.975)
z_pdf <- dnorm(z_grid)

t_grid <- seq(-4, 4, by = 0.5)
t_df <- 12
t_cut <- qt(0.975, df = t_df)
t_pdf <- dt(t_grid, df = t_df)

f_grid <- seq(0, 6, by = 0.5)
f_df1 <- 4; f_df2 <- 20
f_cut <- qf(0.95, df1 = f_df1, df2 = f_df2)
f_pdf <- df(f_grid, df1 = f_df1, df2 = f_df2)

chi_grid <- seq(0, 18, by = 1.5)
chi_df <- 5
chi_cut <- qchisq(0.95, df = chi_df)
chi_pdf <- dchisq(chi_grid, df = chi_df)

# 7B. Discrete integer-step PMF grids
binom_k <- 0:12
binom_n <- 12L; binom_p <- 0.35
binom_pmf <- dbinom(binom_k, size = binom_n, prob = binom_p)
binom_cdf <- pbinom(binom_k, size = binom_n, prob = binom_p)
binom_upper <- pbinom(binom_k - 1L, size = binom_n, prob = binom_p, lower.tail = FALSE)

pois_k <- 0:14
pois_lam <- 4.2
pois_pmf <- dpois(pois_k, lambda = pois_lam)
pois_cdf <- ppois(pois_k, lambda = pois_lam)
pois_upper <- ppois(pois_k - 1L, lambda = pois_lam, lower.tail = FALSE)

plot_function_grids_out <- list(
  normal_z_two_tailed_005 = list(
    cutoff = z_cut,
    x = as_arr(z_grid),
    pdf = as_arr(z_pdf),
    left_tail_y = as_arr(ifelse(z_grid <= -z_cut, z_pdf, 0)),
    right_tail_y = as_arr(ifelse(z_grid >= z_cut, z_pdf, 0))
  ),
  student_t_df12_two_tailed_005 = list(
    df = t_df,
    cutoff = t_cut,
    x = as_arr(t_grid),
    pdf = as_arr(t_pdf),
    left_tail_y = as_arr(ifelse(t_grid <= -t_cut, t_pdf, 0)),
    right_tail_y = as_arr(ifelse(t_grid >= t_cut, t_pdf, 0))
  ),
  snedecor_f_4_20_upper_005 = list(
    df1 = f_df1, df2 = f_df2,
    cutoff = f_cut,
    x = as_arr(f_grid),
    pdf = as_arr(f_pdf),
    upper_tail_y = as_arr(ifelse(f_grid >= f_cut, f_pdf, 0))
  ),
  chi_square_df5_upper_005 = list(
    df = chi_df,
    cutoff = chi_cut,
    x = as_arr(chi_grid),
    pdf = as_arr(chi_pdf),
    upper_tail_y = as_arr(ifelse(chi_grid >= chi_cut, chi_pdf, 0))
  ),
  binomial_n12_p035 = list(
    n = binom_n, p = binom_p,
    k = as_arr(binom_k),
    pmf = as_arr(binom_pmf),
    cdf_le = as_arr(binom_cdf),
    sf_ge = as_arr(binom_upper)
  ),
  poisson_lambda4_2 = list(
    lambda = pois_lam,
    k = as_arr(pois_k),
    pmf = as_arr(pois_pmf),
    cdf_le = as_arr(pois_cdf),
    sf_ge = as_arr(pois_upper)
  )
)

payload <- list(
  provenance = list(
    script = "crates/fit/tests/reference/r23_fit_oracles.R",
    r_version = as.character(getRversion())
  ),
  relational_constraints = relational_constraints_out,
  derived_scalar_cis = derived_scalar_cis_out,
  fixed_8_canonical_builtins = fixed_8_builtins_out,
  new_7_modern_builtins = new_7_builtins_out,
  legacy_reparameterizations = legacy_reparams_out,
  gof_and_poisson_nonlinear = gof_and_poisson_out,
  section_6_2_distribution_grids = plot_function_grids_out
)

out_path <- "crates/fit/tests/reference/r23_fit_oracles.json"
writeLines(to_json(payload), con = out_path)
cat(sprintf("Wrote %s\n", out_path))
