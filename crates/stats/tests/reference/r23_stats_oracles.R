# Independent R 4.5.2 (2025-10-31) black-box reference generator for CurveBook
# R23.2, R23.4 & R23.5 (`crates/stats`):
#   1. Lognormal suite (R23.2):
#      - Normality + Lognormality 4 tests (Shapiro-Wilk, D'Agostino-Pearson K^2,
#        Anderson-Darling A^2*, Lilliefors KS D) + Normal vs Lognormal log-LR &
#        Relative Likelihood + Blom Q-Q expected quantiles
#      - One-sample ratio t-test
#      - Unpaired Lognormal Welch t-test (and equal-variance comparison)
#      - Lognormal 1-way Ordinary, Welch/Brown-Forsythe, and RM/REML ANOVA +
#        geometric-mean ratio post-hoc CIs
#      - Multiple Row Lognormal (equal-var & Welch) & Ratio Paired t-tests +
#        Holm-Sidak, BH, BY, and two-stage BKY adjustments
#   2. RM 2-way/3-way GG & HF sphericity, 2-way 1-factor-diff comparisons,
#      Spearman heteroscedasticity, K-means validity indices & Hartigan-Wong,
#      and Survival Power (Schoenfeld 1983 & Freedman 1982) (R23.4)
#   3. Multivariable GLM §5.4 (R23.5):
#      - Linear / WLS / Logistic / Poisson with 2-/3-way interactions, X^2 and
#        sqrt(X) transforms, no-intercept, nested F / deviance LRT / AICc,
#        quasi-Poisson dispersion, leverage / Cook's D / DFBETAs, out-of-sample
#        predictions, profile-likelihood CIs (MASS::confint), SE of OR/IRR,
#        and configurable logistic cutoffs
#   4. Cox PH & Kaplan-Meier extensions (R23.5):
#      - Cox PH with 2-way interaction & sqrt transform (Efron & Breslow),
#        SE of HR, Wald & profile-likelihood CIs, parameter correlation matrix,
#        nested Cox LRT, covariate-profile survfit(coxph), Cox-Snell residuals,
#        reverse-KM median follow-up, and ratio of median survival
#
# Run under R 4.5.2 (`stats` 4.5.2, `survival` 3.8-3, `nlme` 3.1-168, `MASS`, `cluster`):
#   /usr/local/bin/Rscript crates/stats/tests/reference/r23_stats_oracles.R

if (getRversion() != "4.5.2") {
  stop(sprintf("Expected R 4.5.2, found %s", as.character(getRversion())))
}
if (packageVersion("survival") != package_version("3.8-3")) {
  stop(sprintf("Expected survival 3.8-3, found %s", as.character(packageVersion("survival"))))
}
if (packageVersion("nlme") != package_version("3.1-168")) {
  stop(sprintf("Expected nlme 3.1-168, found %s", as.character(packageVersion("nlme"))))
}

suppressPackageStartupMessages({
  library(survival)
  library(nlme)
  library(MASS)
  library(cluster)
})

options(digits = 16)

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

# ============================================================================
# Shared Statistical Helpers
# ============================================================================

# D'Agostino, Belanger & D'Agostino (1990) omnibus K^2 normality test
dagostino_pearson_k2 <- function(x) {
  n <- length(x)
  m <- mean(x)
  d <- x - m
  m2 <- sum(d^2) / n
  m3 <- sum(d^3) / n
  m4 <- sum(d^4) / n
  g1 <- m3 / (m2^1.5)
  b2 <- m4 / (m2^2)

  # Skewness Z1
  y <- g1 * sqrt((n + 1) * (n + 3) / (6 * (n - 2)))
  beta2 <- 3 * (n^2 + 27 * n - 70) * (n + 1) * (n + 3) /
    ((n - 2) * (n + 5) * (n + 7) * (n + 9))
  w2 <- -1 + sqrt(2 * (beta2 - 1))
  delta <- 1 / sqrt(0.5 * log(w2))
  alpha <- sqrt(2 / (w2 - 1))
  z1 <- delta * log(y / alpha + sqrt((y / alpha)^2 + 1))

  # Kurtosis Z2 (Anscombe & Glynn 1983)
  e_b2 <- 3 * (n - 1) / (n + 1)
  var_b2 <- 24 * n * (n - 2) * (n - 3) / ((n + 1)^2 * (n + 3) * (n + 5))
  x_k <- (b2 - e_b2) / sqrt(var_b2)
  sqrt_beta1 <- (6 * (n^2 - 5 * n + 2) / ((n + 7) * (n + 9))) *
    sqrt(6 * (n + 3) * (n + 5) / (n * (n - 2) * (n - 3)))
  A <- 6 + (8 / sqrt_beta1) * (2 / sqrt_beta1 + sqrt(1 + 4 / sqrt_beta1^2))
  z2 <- ((1 - 2 / (9 * A)) - ((1 - 2 / A) / (1 + x_k * sqrt(2 / (A - 4))))^(1 / 3)) /
    sqrt(2 / (9 * A))

  k2 <- z1^2 + z2^2
  p <- pchisq(k2, df = 2, lower.tail = FALSE)
  list(k2 = k2, z_skew = z1, z_kurt = z2, p_value = p)
}

# Anderson-Darling A^2 and Stephens (1986) modified A^{*2} + p-value
anderson_darling_norm <- function(x) {
  n <- length(x)
  z <- sort((x - mean(x)) / sd(x))
  p <- pnorm(z)
  i <- seq_len(n)
  s <- sum((2 * i - 1) * (log(p) + log(1 - rev(p))))
  a2 <- -n - s / n
  a2_star <- a2 * (1 + 0.75 / n + 2.25 / (n^2))
  pv <- if (a2_star >= 0.60) {
    exp(1.2937 - 5.709 * a2_star + 0.0186 * a2_star^2)
  } else if (a2_star >= 0.34) {
    exp(0.9177 - 4.279 * a2_star - 1.38 * a2_star^2)
  } else if (a2_star >= 0.20) {
    1 - exp(-8.318 + 42.796 * a2_star - 59.938 * a2_star^2)
  } else {
    1 - exp(-13.436 + 101.14 * a2_star - 223.73 * a2_star^2)
  }
  pv <- max(0.0, min(1.0, pv))
  list(a2 = a2, a2_star = a2_star, p_value = pv)
}

# Lilliefors KS statistic D (unknown mean & sd estimated from sample)
lilliefors_d <- function(x) {
  n <- length(x)
  z <- sort((x - mean(x)) / sd(x))
  f <- pnorm(z)
  i <- seq_len(n)
  max(c(i / n - f, f - (i - 1) / n))
}

# Blom (1958) Q-Q expected quantiles
blom_qq <- function(x) {
  sx <- sort(x)
  n <- length(sx)
  p <- (seq_len(n) - 0.375) / (n + 0.25)
  list(observed = as_arr(sx), expected = as_arr(mean(sx) + sd(sx) * qnorm(p)))
}

# Orthonormal Helmert contrast matrix H of size (k-1) x k
helmert_orthonormal <- function(k) {
  H <- matrix(0, nrow = k - 1L, ncol = k)
  for (r in seq_len(k - 1L)) {
    H[r, seq_len(r)] <- 1 / sqrt(r * (r + 1))
    H[r, r + 1L] <- -r / sqrt(r * (r + 1))
  }
  H
}

# Sphericity from covariance matrix S_cov and orthonormal contrast matrix M
# Computes Mauchly W, Greenhouse-Geisser eps, Huynh-Feldt (1976) eps, and
# Lecoutre (1991) eps (identical to HF when g = 1).
sphericity_from_cov <- function(S_cov, M, df_e, n_total, n_groups) {
  C_mat <- M %*% S_cov %*% t(M)
  q <- nrow(C_mat)
  if (q <= 1L) {
    return(list(q = as.integer(q), mauchly_w = 1.0, gg_eps = 1.0, hf_eps_1976 = 1.0, hf_eps_lecoutre = 1.0))
  }
  ev <- eigen(0.5 * (C_mat + t(C_mat)), symmetric = TRUE, only.values = TRUE)$values
  ev <- pmax(ev, 0)
  tr1 <- sum(ev)
  tr2 <- sum(ev^2)
  gg <- (tr1^2) / (q * tr2)
  gg <- max(1 / q, min(1.0, gg))
  w_mauchly <- if (tr1 > 0) prod(ev) / ((tr1 / q)^q) else 0.0
  hf_1976 <- (n_total * q * gg - 2) / (q * (df_e - q * gg))
  hf_1976 <- max(1 / q, min(1.0, hf_1976))
  hf_lec <- ((df_e + 1) * q * gg - 2) / (q * (df_e - q * gg))
  hf_lec <- max(1 / q, min(1.0, hf_lec))
  list(
    q = as.integer(q),
    mauchly_w = w_mauchly,
    gg_eps = gg,
    hf_eps_1976 = hf_1976,
    hf_eps_lecoutre = hf_lec
  )
}

# Holm-Sidak step-down adjusted p-values
holm_sidak_adj <- function(p) {
  m <- length(p)
  ord <- order(p)
  adj <- numeric(m)
  running_max <- 0.0
  for (r in seq_len(m)) {
    k <- m - r + 1L
    val <- 1 - (1 - p[ord[r]])^k
    running_max <- max(running_max, val)
    adj[ord[r]] <- min(1.0, running_max)
  }
  adj
}

# Two-stage linear step-up Benjamini-Krieger-Yekutieli (2006) q-values at level Q
bky_two_stage_q <- function(p, Q = 0.05) {
  m <- length(p)
  ord <- order(p)
  ps <- p[ord]
  q_prime <- Q / (1 + Q)
  bh1 <- pmin(1.0, rev(cummin(rev(ps * m / seq_len(m)))))
  r1 <- sum(bh1 <= q_prime)
  if (r1 == 0L || r1 == m) {
    dec_q <- numeric(m)
    dec_q[ord] <- pmin(1.0, bh1 * (1 + Q))
    return(list(
      pi0_hat = if (r1 == 0L) 1.0 else 0.0,
      stage1_r = as.integer(r1),
      stage2_discoveries = as.integer(r1),
      bh_scaled_q = if (r1 == 0L) rep(1.0, m) else rep(0.0, m),
      bky_decision_q = dec_q
    ))
  }
  m0_hat <- m - r1
  pi0_hat <- m0_hat / m
  # Stage-2 step-up thresholds are i * q' / m0_hat where q' = Q / (1 + Q).
  # `bh_scaled_q` compares directly against q'; `bky_decision_q` scales by (1 + Q)
  # for direct comparison against target Q (matching `adjust_two_stage_bky`).
  bh2 <- pmin(1.0, rev(cummin(rev(ps * m0_hat / seq_len(m)))))
  bky_dec <- pmin(1.0, rev(cummin(rev(ps * m0_hat * (1 + Q) / seq_len(m)))))
  q_out <- numeric(m)
  q_out[ord] <- bh2
  dec_out <- numeric(m)
  dec_out[ord] <- bky_dec
  list(
    pi0_hat = pi0_hat,
    stage1_r = as.integer(r1),
    stage2_discoveries = as.integer(sum(bky_dec <= Q)),
    bh_scaled_q = q_out,
    bky_decision_q = dec_out
  )
}

# ============================================================================
# SECTION 1: LOGNORMAL SUITE (R23.2)
# ============================================================================

# 1A. Normality & Lognormality Battery + Normal vs Lognormal Relative Likelihood
ln_skewed <- c(
  1.12, 1.48, 1.85, 2.04, 2.31, 2.74, 3.15, 3.62, 4.18, 4.95,
  5.82, 6.90, 8.45, 10.20, 12.80, 16.50, 21.40, 29.80, 44.50, 72.00
)
ln_normal <- c(
  42.1, 45.3, 46.8, 47.5, 48.2, 48.9, 49.4, 49.8, 50.1, 50.4,
  50.7, 51.0, 51.5, 52.1, 52.8, 53.6, 54.4, 55.2, 56.5, 58.9
)
ln_nonpos <- c(-1.5, 0.0, 1.2, 2.4, 3.8, 5.1, 7.3, 9.5, 12.0, 15.2)

normality_lognormality_eval <- function(x) {
  n <- length(x)
  n_imp <- sum(x <= 0)
  sw_n <- stats::shapiro.test(x)
  dp_n <- dagostino_pearson_k2(x)
  ad_n <- anderson_darling_norm(x)
  ks_n <- lilliefors_d(x)

  res <- list(
    values = as_arr(x),
    n = as.integer(n),
    n_impossible_nonpositive = as.integer(n_imp),
    normal = list(
      shapiro_wilk = list(w = unname(sw_n$statistic), p = sw_n$p.value),
      dagostino_pearson = dp_n,
      anderson_darling = ad_n,
      lilliefors_d = ks_n,
      qq = blom_qq(x)
    )
  )
  if (n_imp == 0L) {
    lx <- log(x)
    sw_l <- stats::shapiro.test(lx)
    dp_l <- dagostino_pearson_k2(lx)
    ad_l <- anderson_darling_norm(lx)
    ks_l <- lilliefors_d(lx)
    var_x <- sum((x - mean(x))^2) / n
    var_l <- sum((lx - mean(lx))^2) / n
    ll_norm <- -0.5 * n * (log(2 * pi * var_x) + 1)
    ll_lognorm <- -0.5 * n * (log(2 * pi * var_l) + 1) - sum(lx)
    log_lr <- ll_norm - ll_lognorm
    p_norm <- 1 / (1 + exp(-log_lr))
    p_lognorm <- 1 / (1 + exp(log_lr))
    res$lognormal <- list(
      shapiro_wilk = list(w = unname(sw_l$statistic), p = sw_l$p.value),
      dagostino_pearson = dp_l,
      anderson_darling = ad_l,
      lilliefors_d = ks_l,
      qq_log_space = blom_qq(lx)
    )
    res$comparison <- list(
      ll_normal = ll_norm,
      ll_lognormal = ll_lognorm,
      log_lr_norm_minus_lognorm = log_lr,
      lr_norm_over_lognorm = exp(log_lr),
      lr_lognorm_over_norm = exp(-log_lr),
      prob_normal = p_norm,
      prob_lognormal = p_lognorm,
      preferred = if (log_lr >= 0) "Normal" else "Lognormal"
    )
  } else {
    res$lognormal <- NULL
    res$comparison <- NULL
  }
  res
}

# 1B. One-Sample Ratio t-test
os_ratio_vals <- c(4.2, 5.8, 6.1, 7.5, 8.9, 5.1, 6.8, 9.4, 7.2, 5.5, 8.1, 6.4)
os_mu0 <- 5.0
os_tt <- stats::t.test(log(os_ratio_vals), mu = log(os_mu0), conf.level = 0.95)
one_sample_ratio_t <- list(
  values = as_arr(os_ratio_vals),
  hypothetical_geo_mean = os_mu0,
  t = unname(os_tt$statistic),
  df = unname(os_tt$parameter),
  p_value = os_tt$p.value,
  geo_mean = exp(mean(log(os_ratio_vals))),
  geo_sd_factor = exp(sd(log(os_ratio_vals))),
  geo_mean_ci95 = as_arr(exp(unname(os_tt$conf.int))),
  ratio_gm_over_mu0 = exp(mean(log(os_ratio_vals)) - log(os_mu0)),
  ratio_ci95 = as_arr(exp(unname(os_tt$conf.int) - log(os_mu0)))
)

# 1C. Unpaired Lognormal Welch t-test (unequal GSDs) + Equal-var Lognormal t-test
ln_grp_a <- c(10.5, 11.2, 12.1, 10.8, 11.7, 12.4, 11.0, 11.9)
ln_grp_b <- c(14.0, 28.5, 19.2, 42.0, 16.5, 35.8, 22.4, 51.0, 18.1, 31.2)
wt_ba <- stats::t.test(log(ln_grp_b), log(ln_grp_a), var.equal = FALSE, conf.level = 0.95)
et_ba <- stats::t.test(log(ln_grp_b), log(ln_grp_a), var.equal = TRUE, conf.level = 0.95)
lognormal_two_group <- list(
  group_a = as_arr(ln_grp_a),
  group_b = as_arr(ln_grp_b),
  gm_a = exp(mean(log(ln_grp_a))),
  gsd_a = exp(sd(log(ln_grp_a))),
  gm_b = exp(mean(log(ln_grp_b))),
  gsd_b = exp(sd(log(ln_grp_b))),
  welch_b_over_a = list(
    t = unname(wt_ba$statistic),
    df = unname(wt_ba$parameter),
    p_value = wt_ba$p.value,
    log_diff = mean(log(ln_grp_b)) - mean(log(ln_grp_a)),
    se_log_diff = unname(wt_ba$stderr),
    gm_ratio_b_over_a = exp(mean(log(ln_grp_b)) - mean(log(ln_grp_a))),
    gm_ratio_ci95 = as_arr(exp(unname(wt_ba$conf.int)))
  ),
  equal_var_b_over_a = list(
    t = unname(et_ba$statistic),
    df = unname(et_ba$parameter),
    p_value = et_ba$p.value,
    log_diff = mean(log(ln_grp_b)) - mean(log(ln_grp_a)),
    se_log_diff = unname(et_ba$stderr),
    gm_ratio_b_over_a = exp(mean(log(ln_grp_b)) - mean(log(ln_grp_a))),
    gm_ratio_ci95 = as_arr(exp(unname(et_ba$conf.int)))
  )
)

# 1D. Lognormal 1-Way ANOVA (Ordinary, Welch/BF, RM complete, Mixed-Effects REML)
ln_ow_groups <- list(
  G1 = c(4.2, 4.8, 5.1, 4.5, 5.3, 4.9),
  G2 = c(6.5, 9.2, 7.1, 11.4, 8.0, 10.5, 7.6),
  G3 = c(12.0, 18.5, 14.2, 22.8, 16.0),
  G4 = c(24.5, 27.0, 25.8, 28.4, 26.2, 29.1)
)
ln_ow_df <- data.frame(
  y = unlist(ln_ow_groups),
  log_y = log(unlist(ln_ow_groups)),
  g = factor(rep(names(ln_ow_groups), lengths(ln_ow_groups)), levels = names(ln_ow_groups))
)
ln_ord_aov <- stats::aov(log_y ~ g, data = ln_ow_df)
ln_ord_sum <- summary(ln_ord_aov)[[1]]
ln_tukey <- stats::TukeyHSD(ln_ord_aov, conf.level = 0.95)$g
ln_tukey_list <- lapply(seq_len(nrow(ln_tukey)), function(i) {
  list(
    pair = rownames(ln_tukey)[i],
    log_diff = unname(ln_tukey[i, "diff"]),
    gm_ratio = exp(unname(ln_tukey[i, "diff"])),
    gm_ratio_ci95 = as_arr(c(exp(unname(ln_tukey[i, "lwr"])), exp(unname(ln_tukey[i, "upr"])))),
    p_adj = unname(ln_tukey[i, "p adj"])
  )
})

# Welch & Brown-Forsythe 1-way ANOVA on log(y)
ln_welch <- stats::oneway.test(log_y ~ g, data = ln_ow_df, var.equal = FALSE)
ns_ln <- sapply(ln_ow_groups, length)
ms_ln <- sapply(ln_ow_groups, function(v) mean(log(v)))
vs_ln <- sapply(ln_ow_groups, function(v) var(log(v)))
N_ln <- sum(ns_ln)
k_ln <- length(ns_ln)
gm_log <- sum(ns_ln * ms_ln) / N_ln
bf_num <- sum(ns_ln * (ms_ln - gm_log)^2)
bf_den <- sum((1 - ns_ln / N_ln) * vs_ln)
bf_f <- bf_num / bf_den
c_bf <- ((1 - ns_ln / N_ln) * vs_ln) / bf_den
bf_df2 <- 1 / sum((c_bf^2) / (ns_ln - 1))
bf_p <- pf(bf_f, k_ln - 1, bf_df2, lower.tail = FALSE)

# Games-Howell post-hoc ratios on log(y)
gh_pairs <- list()
for (i in 1:(k_ln - 1L)) {
  for (j in (i + 1L):k_ln) {
    wt <- stats::t.test(log(ln_ow_groups[[j]]), log(ln_ow_groups[[i]]), var.equal = FALSE)
    diff_ij <- ms_ln[j] - ms_ln[i]
    se_ij <- unname(wt$stderr)
    df_ij <- unname(wt$parameter)
    q_ij <- abs(diff_ij) / (se_ij / sqrt(2))
    p_gh <- ptukey(q_ij, nmeans = k_ln, df = df_ij, lower.tail = FALSE)
    qc <- qtukey(0.95, nmeans = k_ln, df = df_ij)
    ci_log <- c(diff_ij - qc * se_ij / sqrt(2), diff_ij + qc * se_ij / sqrt(2))
    gh_pairs[[sprintf("%s-%s", names(ln_ow_groups)[j], names(ln_ow_groups)[i])]] <- list(
      log_diff = diff_ij,
      df = df_ij,
      q = q_ij,
      p_adj = p_gh,
      gm_ratio = exp(diff_ij),
      gm_ratio_ci95 = as_arr(exp(ci_log))
    )
  }
}

# RM 1-way ANOVA on log(y): complete 6 subjects x 4 treatments + missing-value REML
rm_ln_mat <- matrix(
  c(10.2, 14.5, 21.0, 18.2,
    12.8, 17.2, 26.4, 22.0,
     8.5, 11.0, 18.5, 14.8,
    15.1, 21.4, 34.0, 28.5,
     9.4, 13.8, 22.1, 17.6,
    11.6, 16.5, 28.0, 23.4),
  nrow = 6, ncol = 4, byrow = TRUE
)
log_rm_mat <- log(rm_ln_mat)
rm_ln_df <- data.frame(
  subj = factor(rep(paste0("S", 1:6), times = 4)),
  trt = factor(rep(paste0("T", 1:4), each = 6), levels = paste0("T", 1:4)),
  y = as.vector(rm_ln_mat),
  log_y = as.vector(log_rm_mat)
)
rm_ln_aov <- summary(stats::aov(log_y ~ trt + Error(subj / trt), data = rm_ln_df))
rm_trt_tab <- rm_ln_aov[["Error: subj:trt"]][[1]]
rm_f <- rm_trt_tab["trt", "F value"]
rm_df1 <- rm_trt_tab["trt", "Df"]
rm_df2 <- rm_trt_tab["Residuals", "Df"]
rm_p_unadj <- rm_trt_tab["trt", "Pr(>F)"]
sph_rm_ln <- sphericity_from_cov(cov(log_rm_mat), helmert_orthonormal(4L), 5, 6, 1)
rm_p_gg <- pf(rm_f, rm_df1 * sph_rm_ln$gg_eps, rm_df2 * sph_rm_ln$gg_eps, lower.tail = FALSE)
rm_p_hf <- pf(rm_f, rm_df1 * sph_rm_ln$hf_eps_1976, rm_df2 * sph_rm_ln$hf_eps_1976, lower.tail = FALSE)

# Unbalanced REML (drop 2 cells: S2-T3 and S5-T4)
rm_ln_unbal <- rm_ln_df[!(rm_ln_df$subj == "S2" & rm_ln_df$trt == "T3") &
                          !(rm_ln_df$subj == "S5" & rm_ln_df$trt == "T4"), ]
lme_ln <- nlme::lme(log_y ~ trt, random = ~ 1 | subj, data = rm_ln_unbal, method = "REML")
lme_ln_aov <- stats::anova(lme_ln)
vc_ln <- as.numeric(nlme::VarCorr(lme_ln)[, "Variance"])
sigma2_u <- vc_ln[1]; sigma2_e <- vc_ln[2]
fe_ln <- nlme::fixef(lme_ln)
se_ln <- sqrt(diag(vcov(lme_ln)))
# Contrasts T2/T1, T3/T1, T4/T1 (Wald t on lme denDF)
den_df_lme <- lme_ln_aov["trt", "denDF"]
t_crit_lme <- qt(0.975, df = den_df_lme)
reml_vs_t1 <- lapply(2:4, function(idx) {
  est <- unname(fe_ln[idx])
  se_i <- unname(se_ln[idx])
  list(
    contrast = paste0("T", idx, "/T1"),
    log_diff = est,
    se = se_i,
    gm_ratio = exp(est),
    gm_ratio_ci95 = as_arr(exp(c(est - t_crit_lme * se_i, est + t_crit_lme * se_i)))
  )
})

lognormal_one_way <- list(
  groups = lapply(ln_ow_groups, as_arr),
  ordinary = list(
    f = unname(ln_ord_sum["g", "F value"]),
    df1 = unname(ln_ord_sum["g", "Df"]),
    df2 = unname(ln_ord_sum["Residuals", "Df"]),
    p_value = unname(ln_ord_sum["g", "Pr(>F)"]),
    r_squared = ln_ord_sum["g", "Sum Sq"] / sum(ln_ord_sum[, "Sum Sq"]),
    tukey_gm_ratios = ln_tukey_list
  ),
  welch = list(
    f = unname(ln_welch$statistic),
    df1 = unname(ln_welch$parameter[1]),
    df2 = unname(ln_welch$parameter[2]),
    p_value = ln_welch$p.value
  ),
  brown_forsythe = list(
    f_star = bf_f,
    df1 = as.numeric(k_ln - 1L),
    df2 = bf_df2,
    p_value = bf_p
  ),
  games_howell_gm_ratios = gh_pairs,
  rm_complete = list(
    matrix_subjects_x_trt = lapply(seq_len(nrow(rm_ln_mat)), function(r) as_arr(rm_ln_mat[r, ])),
    f = rm_f,
    df1 = rm_df1,
    df2 = rm_df2,
    p_unadjusted = rm_p_unadj,
    sphericity = sph_rm_ln,
    p_gg = rm_p_gg,
    p_hf = rm_p_hf
  ),
  reml_missing_2_cells = list(
    dropped = as_arr(c("S2:T3", "S5:T4")),
    f = unname(lme_ln_aov["trt", "F-value"]),
    df1 = unname(lme_ln_aov["trt", "numDF"]),
    df2 = unname(den_df_lme),
    p_value = unname(lme_ln_aov["trt", "p-value"]),
    log_reml = as.numeric(logLik(lme_ln, REML = TRUE)),
    var_subject = sigma2_u,
    var_residual = sigma2_e,
    icc = sigma2_u / (sigma2_u + sigma2_e),
    contrasts_vs_t1 = reml_vs_t1
  )
)

# 1E. Multiple Row Lognormal & Ratio Paired t-tests (5 rows x 2 groups x 5 reps)
mr_a <- list(
  c(10.2, 11.4, 9.8, 12.1, 10.6),
  c(20.5, 22.0, 19.2, 24.1, 21.3),
  c(5.1,  6.2,  4.8,  5.7,  5.4),
  c(50.0, 48.2, 53.5, 51.0, 49.4),
  c(8.2,  9.5,  7.9,  8.8,  9.1)
)
mr_b <- list(
  c(18.5, 22.1, 16.4, 24.0, 19.8),
  c(21.0, 23.5, 20.1, 25.0, 22.2),
  c(11.2, 15.8, 9.6,  14.4, 12.5),
  c(49.1, 47.8, 52.0, 50.2, 48.9),
  c(14.5, 19.2, 12.8, 17.1, 16.0)
)
mr_eval <- function(mode) {
  rows_res <- lapply(seq_along(mr_a), function(i) {
    la <- log(mr_a[[i]]); lb <- log(mr_b[[i]])
    tt <- if (mode == "unpaired_equal") {
      stats::t.test(lb, la, var.equal = TRUE)
    } else if (mode == "unpaired_welch") {
      stats::t.test(lb, la, var.equal = FALSE)
    } else {
      stats::t.test(lb, la, paired = TRUE)
    }
    diff_log <- mean(lb) - mean(la)
    list(
      row = as.integer(i - 1L),
      t = unname(tt$statistic),
      df = unname(tt$parameter),
      p_raw = tt$p.value,
      gm_ratio_b_over_a = exp(diff_log),
      gm_ratio_ci95 = as_arr(exp(unname(tt$conf.int)))
    )
  })
  p_raw <- vapply(rows_res, function(r) r$p_raw, numeric(1L))
  list(
    rows = rows_res,
    p_holm_sidak = as_arr(holm_sidak_adj(p_raw)),
    q_bh = as_arr(stats::p.adjust(p_raw, method = "BH")),
    q_by = as_arr(stats::p.adjust(p_raw, method = "BY")),
    bky_q05 = bky_two_stage_q(p_raw, Q = 0.05)
  )
}
multiple_row_lognormal <- list(
  group_a_rows = lapply(mr_a, as_arr),
  group_b_rows = lapply(mr_b, as_arr),
  unpaired_lognormal_equal_var = mr_eval("unpaired_equal"),
  unpaired_lognormal_welch = mr_eval("unpaired_welch"),
  ratio_paired = mr_eval("ratio_paired")
)

# ============================================================================
# SECTION 2: RM 2/3-WAY SPHERICITY, 1-FACTOR-DIFF, SPEARMAN HETERO, K-MEANS,
#            SURVIVAL POWER (R23.4)
# ============================================================================

# 2A-1. Split-Plot 2-Way RM ANOVA (ColumnFactorMatched: 2 between x 4 within x 5 subj/grp)
sp_g1 <- matrix(
  c(10.2, 12.1, 18.5, 19.2,
    11.5, 13.0, 21.0, 22.1,
     9.8, 10.5, 14.2, 15.0,
    12.4, 15.8, 24.5, 25.2,
    10.0, 11.2, 16.0, 17.5),
  nrow = 5, ncol = 4, byrow = TRUE
)
sp_g2 <- matrix(
  c(11.0, 15.2, 26.0, 28.5,
    12.2, 17.0, 31.5, 34.0,
    10.4, 13.5, 21.8, 24.2,
    13.1, 18.4, 34.2, 36.8,
    11.6, 15.9, 28.0, 30.1),
  nrow = 5, ncol = 4, byrow = TRUE
)
sp_mat <- rbind(sp_g1, sp_g2)
sp_df <- data.frame(
  subj = factor(rep(paste0("S", 1:10), times = 4)),
  grp = factor(rep(rep(c("Ctrl", "Drug"), each = 5), times = 4), levels = c("Ctrl", "Drug")),
  time = factor(rep(paste0("T", 1:4), each = 10), levels = paste0("T", 1:4)),
  y = as.vector(sp_mat)
)
sp_aov <- summary(stats::aov(y ~ grp * time + Error(subj / time), data = sp_df))
sp_bt <- sp_aov[["Error: subj"]][[1]]
sp_wt <- sp_aov[["Error: subj:time"]][[1]]
# Pooled within-group covariance matrix across 4 time levels (df_e = 10 - 2 = 8)
S_pool_sp <- (4 * cov(sp_g1) + 4 * cov(sp_g2)) / 8
sph_sp <- sphericity_from_cov(S_pool_sp, helmert_orthonormal(4L), df_e = 8, n_total = 10, n_groups = 2)

f_sp_time <- sp_wt["time", "F value"]; df1_t <- sp_wt["time", "Df"]; df2_t <- sp_wt["Residuals", "Df"]
f_sp_int <- sp_wt["grp:time", "F value"]; df1_i <- sp_wt["grp:time", "Df"]

rm_2way_splitplot <- list(
  group1_matrix = lapply(1:5, function(r) as_arr(sp_g1[r, ])),
  group2_matrix = lapply(1:5, function(r) as_arr(sp_g2[r, ])),
  between_grp = list(
    f = unname(sp_bt["grp", "F value"]),
    df1 = unname(sp_bt["grp", "Df"]),
    df2 = unname(sp_bt["Residuals", "Df"]),
    p_value = unname(sp_bt["grp", "Pr(>F)"])
  ),
  within_time = list(
    f = unname(f_sp_time), df1 = unname(df1_t), df2 = unname(df2_t),
    p_unadjusted = unname(sp_wt["time", "Pr(>F)"]),
    p_gg = pf(f_sp_time, df1_t * sph_sp$gg_eps, df2_t * sph_sp$gg_eps, lower.tail = FALSE),
    p_hf_1976 = pf(f_sp_time, df1_t * sph_sp$hf_eps_1976, df2_t * sph_sp$hf_eps_1976, lower.tail = FALSE),
    p_hf_lecoutre = pf(f_sp_time, df1_t * sph_sp$hf_eps_lecoutre, df2_t * sph_sp$hf_eps_lecoutre, lower.tail = FALSE)
  ),
  interaction_grp_time = list(
    f = unname(f_sp_int), df1 = unname(df1_i), df2 = unname(df2_t),
    p_unadjusted = unname(sp_wt["grp:time", "Pr(>F)"]),
    p_gg = pf(f_sp_int, df1_i * sph_sp$gg_eps, df2_t * sph_sp$gg_eps, lower.tail = FALSE),
    p_hf_1976 = pf(f_sp_int, df1_i * sph_sp$hf_eps_1976, df2_t * sph_sp$hf_eps_1976, lower.tail = FALSE),
    p_hf_lecoutre = pf(f_sp_int, df1_i * sph_sp$hf_eps_lecoutre, df2_t * sph_sp$hf_eps_lecoutre, lower.tail = FALSE)
  ),
  sphericity = sph_sp
)

# 2A-2. Both-Factors-Matched 2-Way RM ANOVA (3 rows x 4 cols x 6 subjects)
bf_flat <- matrix(
  c(
    10.0, 12.2, 16.5, 17.8,  14.1, 17.0, 23.2, 25.0,  18.5, 22.8, 32.0, 35.1,
    11.8, 13.5, 19.0, 21.2,  16.0, 18.8, 27.5, 29.8,  21.0, 25.5, 37.2, 41.0,
     9.2, 10.8, 13.4, 14.1,  12.5, 14.9, 19.0, 20.2,  15.8, 19.2, 26.0, 28.4,
    13.0, 15.6, 22.1, 24.8,  18.2, 21.5, 31.0, 34.2,  24.0, 29.1, 42.5, 46.8,
    10.6, 11.9, 15.2, 16.0,  14.8, 16.4, 22.0, 23.5,  19.2, 22.0, 30.8, 33.0,
    12.4, 14.8, 20.5, 22.9,  17.1, 20.2, 28.8, 31.6,  22.5, 27.4, 39.0, 43.2
  ),
  nrow = 6, ncol = 12, byrow = TRUE
)
bf_df <- expand.grid(col = paste0("C", 1:4), row = paste0("R", 1:3), subj = paste0("S", 1:6))
bf_df$y <- as.vector(t(bf_flat))
bf_df$row <- factor(bf_df$row, levels = paste0("R", 1:3))
bf_df$col <- factor(bf_df$col, levels = paste0("C", 1:4))
bf_df$subj <- factor(bf_df$subj, levels = paste0("S", 1:6))
bf_aov <- summary(stats::aov(y ~ row * col + Error(subj / (row * col)), data = bf_df))

S_bf <- cov(bf_flat)
H_r <- helmert_orthonormal(3L); u_r <- matrix(1 / sqrt(3), nrow = 1L, ncol = 3L)
H_c <- helmert_orthonormal(4L); u_c <- matrix(1 / sqrt(4), nrow = 1L, ncol = 4L)
sph_bf_row <- sphericity_from_cov(S_bf, kronecker(H_r, u_c), df_e = 5, n_total = 6, n_groups = 1)
sph_bf_col <- sphericity_from_cov(S_bf, kronecker(u_r, H_c), df_e = 5, n_total = 6, n_groups = 1)
sph_bf_int <- sphericity_from_cov(S_bf, kronecker(H_r, H_c), df_e = 5, n_total = 6, n_groups = 1)

extract_bf_term <- function(tab, term, sph) {
  fv <- unname(tab[term, "F value"])
  d1 <- unname(tab[term, "Df"])
  d2 <- unname(tab["Residuals", "Df"])
  list(
    f = fv, df1 = d1, df2 = d2,
    p_unadjusted = unname(tab[term, "Pr(>F)"]),
    sphericity = sph,
    p_gg = pf(fv, d1 * sph$gg_eps, d2 * sph$gg_eps, lower.tail = FALSE),
    p_hf = pf(fv, d1 * sph$hf_eps_1976, d2 * sph$hf_eps_1976, lower.tail = FALSE)
  )
}

rm_2way_both <- list(
  flat_subjects_x_rowmajor_3x4 = lapply(1:6, function(r) as_arr(bf_flat[r, ])),
  row = extract_bf_term(bf_aov[["Error: subj:row"]][[1]], "row", sph_bf_row),
  col = extract_bf_term(bf_aov[["Error: subj:col"]][[1]], "col", sph_bf_col),
  interaction = extract_bf_term(bf_aov[["Error: subj:row:col"]][[1]], "row:col", sph_bf_int)
)

# 2B. RM 3-Way ANOVA GG & HF Sphericity (2 between A x 2 within B x 3 within C; 5 subj/A)
# Columns ordered: (B1,C1),(B1,C2),(B1,C3),(B2,C1),(B2,C2),(B2,C3)
rm3_a1 <- matrix(
  c(10.1, 13.5, 19.0, 12.0, 16.2, 24.5,
    11.4, 15.2, 22.8, 13.5, 18.0, 28.4,
     9.5, 11.8, 15.4, 10.8, 14.0, 19.6,
    12.8, 17.4, 26.0, 15.1, 21.0, 32.5,
    10.6, 14.0, 20.2, 12.4, 17.1, 25.8),
  nrow = 5, ncol = 6, byrow = TRUE
)
rm3_a2 <- matrix(
  c(11.2, 16.8, 27.5, 14.5, 22.4, 38.0,
    12.5, 19.0, 32.0, 16.2, 26.0, 44.5,
    10.4, 14.5, 22.4, 13.0, 19.2, 31.2,
    13.8, 21.2, 36.4, 18.0, 29.5, 50.2,
    11.9, 17.6, 29.1, 15.1, 24.0, 41.0),
  nrow = 5, ncol = 6, byrow = TRUE
)
rm3_all <- rbind(rm3_a1, rm3_a2)
rm3_df <- expand.grid(C = paste0("C", 1:3), B = paste0("B", 1:2), subj = paste0("S", 1:10))
rm3_df$A <- factor(ifelse(as.integer(rm3_df$subj) <= 5L, "A1", "A2"), levels = c("A1", "A2"))
rm3_df$y <- as.vector(t(rm3_all))
rm3_aov <- summary(stats::aov(y ~ A * B * C + Error(subj / (B * C)), data = rm3_df))
S_pool_rm3 <- (4 * cov(rm3_a1) + 4 * cov(rm3_a2)) / 8
H_b <- helmert_orthonormal(2L); u_b <- matrix(1 / sqrt(2), nrow = 1L, ncol = 2L)
H_c3 <- helmert_orthonormal(3L); u_c3 <- matrix(1 / sqrt(3), nrow = 1L, ncol = 3L)
sph_rm3_c <- sphericity_from_cov(S_pool_rm3, kronecker(u_b, H_c3), df_e = 8, n_total = 10, n_groups = 2)
sph_rm3_bc <- sphericity_from_cov(S_pool_rm3, kronecker(H_b, H_c3), df_e = 8, n_total = 10, n_groups = 2)

tab_c <- rm3_aov[["Error: subj:C"]][[1]]
tab_bc <- rm3_aov[["Error: subj:B:C"]][[1]]
tab_b <- rm3_aov[["Error: subj:B"]][[1]]
tab_a <- rm3_aov[["Error: subj"]][[1]]

rm_3way_out <- list(
  a1_matrix = lapply(1:5, function(r) as_arr(rm3_a1[r, ])),
  a2_matrix = lapply(1:5, function(r) as_arr(rm3_a2[r, ])),
  A = list(f = unname(tab_a["A", "F value"]), df1 = unname(tab_a["A", "Df"]), df2 = unname(tab_a["Residuals", "Df"]), p = unname(tab_a["A", "Pr(>F)"])),
  B = list(f = unname(tab_b["B", "F value"]), df1 = unname(tab_b["B", "Df"]), df2 = unname(tab_b["Residuals", "Df"]), p = unname(tab_b["B", "Pr(>F)"])),
  AB = list(f = unname(tab_b["A:B", "F value"]), df1 = unname(tab_b["A:B", "Df"]), df2 = unname(tab_b["Residuals", "Df"]), p = unname(tab_b["A:B", "Pr(>F)"])),
  C = list(
    f = unname(tab_c["C", "F value"]), df1 = unname(tab_c["C", "Df"]), df2 = unname(tab_c["Residuals", "Df"]),
    p_unadjusted = unname(tab_c["C", "Pr(>F)"]),
    sphericity = sph_rm3_c,
    p_gg = pf(tab_c["C", "F value"], tab_c["C", "Df"] * sph_rm3_c$gg_eps, tab_c["Residuals", "Df"] * sph_rm3_c$gg_eps, lower.tail = FALSE),
    p_hf_1976 = pf(tab_c["C", "F value"], tab_c["C", "Df"] * sph_rm3_c$hf_eps_1976, tab_c["Residuals", "Df"] * sph_rm3_c$hf_eps_1976, lower.tail = FALSE),
    p_hf_lecoutre = pf(tab_c["C", "F value"], tab_c["C", "Df"] * sph_rm3_c$hf_eps_lecoutre, tab_c["Residuals", "Df"] * sph_rm3_c$hf_eps_lecoutre, lower.tail = FALSE)
  ),
  AC = list(
    f = unname(tab_c["A:C", "F value"]), df1 = unname(tab_c["A:C", "Df"]), df2 = unname(tab_c["Residuals", "Df"]),
    p_unadjusted = unname(tab_c["A:C", "Pr(>F)"]),
    p_gg = pf(tab_c["A:C", "F value"], tab_c["A:C", "Df"] * sph_rm3_c$gg_eps, tab_c["Residuals", "Df"] * sph_rm3_c$gg_eps, lower.tail = FALSE),
    p_hf_lecoutre = pf(tab_c["A:C", "F value"], tab_c["A:C", "Df"] * sph_rm3_c$hf_eps_lecoutre, tab_c["Residuals", "Df"] * sph_rm3_c$hf_eps_lecoutre, lower.tail = FALSE)
  ),
  BC = list(
    f = unname(tab_bc["B:C", "F value"]), df1 = unname(tab_bc["B:C", "Df"]), df2 = unname(tab_bc["Residuals", "Df"]),
    p_unadjusted = unname(tab_bc["B:C", "Pr(>F)"]),
    sphericity = sph_rm3_bc,
    p_gg = pf(tab_bc["B:C", "F value"], tab_bc["B:C", "Df"] * sph_rm3_bc$gg_eps, tab_bc["Residuals", "Df"] * sph_rm3_bc$gg_eps, lower.tail = FALSE),
    p_hf_1976 = pf(tab_bc["B:C", "F value"], tab_bc["B:C", "Df"] * sph_rm3_bc$hf_eps_1976, tab_bc["Residuals", "Df"] * sph_rm3_bc$hf_eps_1976, lower.tail = FALSE),
    p_hf_lecoutre = pf(tab_bc["B:C", "F value"], tab_bc["B:C", "Df"] * sph_rm3_bc$hf_eps_lecoutre, tab_bc["Residuals", "Df"] * sph_rm3_bc$hf_eps_lecoutre, lower.tail = FALSE)
  ),
  ABC = list(
    f = unname(tab_bc["A:B:C", "F value"]), df1 = unname(tab_bc["A:B:C", "Df"]), df2 = unname(tab_bc["Residuals", "Df"]),
    p_unadjusted = unname(tab_bc["A:B:C", "Pr(>F)"]),
    p_gg = pf(tab_bc["A:B:C", "F value"], tab_bc["A:B:C", "Df"] * sph_rm3_bc$gg_eps, tab_bc["Residuals", "Df"] * sph_rm3_bc$gg_eps, lower.tail = FALSE),
    p_hf_lecoutre = pf(tab_bc["A:B:C", "F value"], tab_bc["A:B:C", "Df"] * sph_rm3_bc$hf_eps_lecoutre, tab_bc["Residuals", "Df"] * sph_rm3_bc$hf_eps_lecoutre, lower.tail = FALSE)
  )
)

# 2C. 2-Way "Cells Differing by Only One Factor" (3 x 3 balanced, n = 4/cell)
f2_cells <- list(
  "R1:C1" = c(10.1, 10.8, 9.6, 10.5),
  "R1:C2" = c(12.2, 13.0, 11.8, 12.6),
  "R1:C3" = c(15.4, 16.1, 14.9, 15.8),
  "R2:C1" = c(14.0, 14.7, 13.5, 14.2),
  "R2:C2" = c(18.5, 19.2, 17.9, 18.8),
  "R2:C3" = c(24.1, 25.0, 23.6, 24.5),
  "R3:C1" = c(11.5, 12.1, 10.9, 11.8),
  "R3:C2" = c(15.0, 15.8, 14.4, 15.2),
  "R3:C3" = c(28.0, 29.2, 27.4, 28.6)
)
f2_df <- data.frame(
  row = factor(rep(rep(paste0("R", 1:3), each = 3), each = 4), levels = paste0("R", 1:3)),
  col = factor(rep(rep(paste0("C", 1:3), times = 3), each = 4), levels = paste0("C", 1:3)),
  y = unlist(f2_cells)
)
f2_fit <- stats::aov(y ~ row * col, data = f2_df)
f2_tab <- summary(f2_fit)[[1]]
mse_f2 <- f2_tab["Residuals", "Mean Sq"]
df_err_f2 <- f2_tab["Residuals", "Df"]
se_cell_diff <- sqrt(2 * mse_f2 / 4)
cell_means <- sapply(f2_cells, mean)
# Generate all 18 one-factor-diff pairs (9 within-row + 9 within-col)
one_fac_pairs <- list()
for (r in 1:3) {
  for (c1 in 1:2) for (c2 in (c1 + 1L):3) {
    k1 <- sprintf("R%d:C%d", r, c1); k2 <- sprintf("R%d:C%d", r, c2)
    diff_v <- cell_means[k1] - cell_means[k2]
    t_v <- abs(diff_v) / se_cell_diff
    p_u <- 2 * pt(t_v, df = df_err_f2, lower.tail = FALSE)
    one_fac_pairs[[paste0(k1, "_vs_", k2)]] <- list(
      family_type = "within_row", family_id = sprintf("R%d", r),
      diff = unname(diff_v), se = se_cell_diff, t = unname(t_v), p_unadj = unname(p_u),
      p_sidak_m18 = 1 - (1 - p_u)^18,
      p_bonf_m18 = min(1.0, 18 * p_u),
      p_sidak_m9 = 1 - (1 - p_u)^9,
      p_sidak_m3 = 1 - (1 - p_u)^3,
      p_tukey_k3 = ptukey(t_v * sqrt(2), nmeans = 3, df = df_err_f2, lower.tail = FALSE)
    )
  }
}
for (c_i in 1:3) {
  for (r1 in 1:2) for (r2 in (r1 + 1L):3) {
    k1 <- sprintf("R%d:C%d", r1, c_i); k2 <- sprintf("R%d:C%d", r2, c_i)
    diff_v <- cell_means[k1] - cell_means[k2]
    t_v <- abs(diff_v) / se_cell_diff
    p_u <- 2 * pt(t_v, df = df_err_f2, lower.tail = FALSE)
    one_fac_pairs[[paste0(k1, "_vs_", k2)]] <- list(
      family_type = "within_col", family_id = sprintf("C%d", c_i),
      diff = unname(diff_v), se = se_cell_diff, t = unname(t_v), p_unadj = unname(p_u),
      p_sidak_m18 = 1 - (1 - p_u)^18,
      p_bonf_m18 = min(1.0, 18 * p_u),
      p_sidak_m9 = 1 - (1 - p_u)^9,
      p_sidak_m3 = 1 - (1 - p_u)^3,
      p_tukey_k3 = ptukey(t_v * sqrt(2), nmeans = 3, df = df_err_f2, lower.tail = FALSE)
    )
  }
}

two_way_one_factor_diff <- list(
  cells = lapply(f2_cells, as_arr),
  mse = mse_f2,
  df_error = df_err_f2,
  n_one_factor_pairs = as.integer(length(one_fac_pairs)),
  t_crit_sidak_m18 = qt(1 - (1 - (1 - 0.05)^(1 / 18)) / 2, df = df_err_f2),
  t_crit_bonf_m18 = qt(1 - 0.05 / (2 * 18), df = df_err_f2),
  pairs = one_fac_pairs
)

# 2D. Spearman Heteroscedasticity Test (predicted cell mean vs |residual|)
het_cells <- list(
  "R1:C1" = c(10.0, 10.2, 9.8, 10.1, 9.9),
  "R1:C2" = c(20.0, 21.1, 18.9, 20.6, 19.4),
  "R2:C1" = c(40.0, 44.5, 35.5, 42.8, 37.2),
  "R2:C2" = c(80.0, 94.0, 66.0, 88.5, 71.5)
)
het_df <- data.frame(
  row = factor(rep(c("R1", "R1", "R2", "R2"), each = 5)),
  col = factor(rep(c("C1", "C2", "C1", "C2"), each = 5)),
  y = unlist(het_cells)
)
het_fit <- stats::lm(y ~ row * col, data = het_df)
sp_homo <- suppressWarnings(stats::cor.test(fitted(f2_fit), abs(residuals(f2_fit)), method = "spearman", exact = FALSE))
sp_hetero <- suppressWarnings(stats::cor.test(fitted(het_fit), abs(residuals(het_fit)), method = "spearman", exact = FALSE))
spearman_heteroscedasticity <- list(
  homoscedastic_3x3 = list(
    rho = unname(sp_homo$estimate),
    p_value_asymp = sp_homo$p.value
  ),
  heteroscedastic_2x2 = list(
    cells = lapply(het_cells, as_arr),
    predicted = as_arr(unname(fitted(het_fit))),
    abs_residuals = as_arr(unname(abs(residuals(het_fit)))),
    rho = unname(sp_hetero$estimate),
    p_value_asymp = sp_hetero$p.value
  )
)

# 2E. K-Means Validity Indices & Hartigan-Wong (n = 30, 3 clean 2D clusters)
km_x <- rbind(
  # Cluster 1 around (0, 0)
  cbind(c(-0.42, 0.18, 0.31, -0.25, 0.05, 0.48, -0.15, -0.35, 0.22, 0.11),
        c( 0.12,-0.30, 0.40, -0.10, 0.28,-0.45, -0.08,  0.33,-0.20, 0.02)),
  # Cluster 2 around (8, 0)
  cbind(c( 7.65, 8.20, 7.90,  8.45, 7.75, 8.10,  8.35,  7.55, 8.05, 8.25),
        c(-0.25, 0.30,-0.12,  0.42, 0.15,-0.38,  0.08, -0.18, 0.25,-0.05)),
  # Cluster 3 around (4, 7)
  cbind(c( 3.70, 4.25, 3.95,  4.40, 3.80, 4.15,  4.30,  3.60, 4.05, 4.20),
        c( 6.75, 7.30, 6.88,  7.45, 7.10, 6.62,  7.18,  6.85, 7.22, 6.95))
)
km_dist <- as.matrix(dist(km_x, method = "euclidean"))

# Deterministic initial center row indices for k = 1..5
init_idx_list <- list(
  "1" = c(1L),
  "2" = c(1L, 11L),
  "3" = c(1L, 11L, 21L),
  "4" = c(1L, 11L, 21L, 6L),
  "5" = c(1L, 11L, 21L, 6L, 16L)
)
km_fits <- list()
wcss_vec <- numeric(5)
for (k in 1:5) {
  if (k == 1L) {
    cen <- matrix(colMeans(km_x), nrow = 1L)
    cl <- rep(1L, nrow(km_x))
    w <- sum(rowSums((km_x - matrix(cen, nrow = nrow(km_x), ncol = ncol(km_x), byrow = TRUE))^2))
    km_fits[[k]] <- list(cluster = cl, centers = cen, tot.withinss = w, totss = w, betweenss = 0)
  } else {
    km_fits[[k]] <- stats::kmeans(km_x, centers = km_x[init_idx_list[[as.character(k)]], , drop = FALSE],
                                  iter.max = 100L, algorithm = "Hartigan-Wong")
  }
  wcss_vec[k] <- km_fits[[k]]$tot.withinss
}

# Compute validity indices for k = 2..5 (plus TraceW/BallHall for k = 1..5)
compute_km_indices <- function(X, Dmat, cl, k, wcss_all) {
  n <- nrow(X); p <- ncol(X)
  tot_cen <- colMeans(X)
  tss <- sum(rowSums((X - matrix(tot_cen, n, p, byrow = TRUE))^2))
  W_mat <- matrix(0, p, p)
  cents <- matrix(0, k, p)
  nk <- integer(k)
  w_k <- 0.0
  scatters <- numeric(k)
  diams <- numeric(k)
  for (c_i in seq_len(k)) {
    idx <- which(cl == c_i)
    nk[c_i] <- length(idx)
    Xi <- X[idx, , drop = FALSE]
    ci <- colMeans(Xi)
    cents[c_i, ] <- ci
    diff_i <- Xi - matrix(ci, nrow(Xi), p, byrow = TRUE)
    W_mat <- W_mat + t(diff_i) %*% diff_i
    dists_c <- sqrt(rowSums(diff_i^2))
    w_k <- w_k + sum(dists_c^2)
    scatters[c_i] <- mean(dists_c)
    diams[c_i] <- if (length(idx) > 1L) max(Dmat[idx, idx]) else 0.0
  }
  B_mat <- t(X - matrix(tot_cen, n, p, byrow = TRUE)) %*% (X - matrix(tot_cen, n, p, byrow = TRUE)) - W_mat
  b_k <- tss - w_k

  ch <- (b_k / (k - 1)) / (w_k / (n - k))
  # Davies-Bouldin
  db_r <- numeric(k)
  for (i in seq_len(k)) {
    vals <- numeric(0)
    for (j in seq_len(k)) if (i != j) {
      dij <- sqrt(sum((cents[i, ] - cents[j, ])^2))
      vals <- c(vals, (scatters[i] + scatters[j]) / dij)
    }
    db_r[i] <- max(vals)
  }
  db <- mean(db_r)
  # Dunn
  min_inter <- Inf
  for (i in 1:(k - 1L)) for (j in (i + 1L):k) {
    ii <- which(cl == i); jj <- which(cl == j)
    min_inter <- min(min_inter, min(Dmat[ii, jj]))
  }
  dunn <- min_inter / max(diams)
  # Silhouette via cluster::silhouette
  sil <- mean(cluster::silhouette(cl, as.dist(Dmat))[, 3])
  # Hartigan H(k) = (W_k / W_{k+1} - 1) * (n - k - 1)
  hartigan <- if (k < length(wcss_all)) (wcss_all[k] / wcss_all[k + 1L] - 1) * (n - k - 1) else NA_real_
  # Krzanowski-Lai KL(k) for k in 2..(K_max-1)
  kl <- if (k >= 2L && k < length(wcss_all)) {
    diff_k <- ((k - 1)^(2 / p)) * wcss_all[k - 1L] - (k^(2 / p)) * wcss_all[k]
    diff_k1 <- (k^(2 / p)) * wcss_all[k] - ((k + 1)^(2 / p)) * wcss_all[k + 1L]
    abs(diff_k / diff_k1)
  } else NA_real_
  # C-index & McClain-Rao & Ptbiserial
  up <- upper.tri(Dmat)
  all_d <- Dmat[up]
  same_cl <- outer(cl, cl, "==")[up]
  nw <- sum(same_cl); nb <- sum(!same_cl)
  sw <- sum(all_d[same_cl]); sb <- sum(all_d[!same_cl])
  s_sorted <- sort(all_d)
  s_min <- sum(s_sorted[seq_len(nw)])
  s_max <- sum(tail(s_sorted, nw))
  c_index <- (sw - s_min) / (s_max - s_min)
  mcclain <- (sw / nw) / (sb / nb)
  # Point-biserial correlation between distance and (1 - same_cl)
  ptbiserial <- cor(all_d, as.numeric(!same_cl))
  # Ratkowsky-Lance: sqrt(mean(BGSS_j / TSS_j)) / sqrt(k)
  rat_j <- numeric(p)
  for (j in seq_len(p)) {
    tss_j <- sum((X[, j] - tot_cen[j])^2)
    bg_j <- sum(nk * (cents[, j] - tot_cen[j])^2)
    rat_j[j] <- bg_j / tss_j
  }
  ratkowsky <- sqrt(mean(rat_j)) / sqrt(k)
  # Rubin (Friedman & Rubin 1967; Charrad et al. 2014): det(T_mat) / det(W_mat)
  T_mat <- W_mat + B_mat
  rubin <- det(T_mat) / det(W_mat)
  friedman <- sum(diag(solve(W_mat, B_mat)))
  marriot <- (k^2) * det(W_mat)
  ball_hall <- w_k / k

  list(
    k = as.integer(k),
    assignments_0based = as_arr(as.integer(cl - 1L)),
    centroids = lapply(seq_len(k), function(r) as_arr(as.numeric(cents[r, ]))),
    mean_within_dist = sw / nw,
    mean_between_dist = sb / nb,
    trace_w = w_k,
    ball_hall = ball_hall,
    calinski_harabasz = ch,
    davies_bouldin = db,
    dunn = dunn,
    silhouette = sil,
    hartigan = hartigan,
    krzanowski_lai = kl,
    c_index = c_index,
    mcclain_rao = mcclain,
    ptbiserial = ptbiserial,
    ratkowsky_lance = ratkowsky,
    rubin = rubin,
    friedman = friedman,
    marriot = marriot,
    frey = NA_real_
  )
}

km_indices_by_k <- list()
for (k in 2:5) {
  km_indices_by_k[[paste0("k", k)]] <- compute_km_indices(km_x, km_dist, km_fits[[k]]$cluster, k, wcss_vec)
}
# Frey & van Groenewoud (1972): (s_bar_b[k+1] - s_bar_b[k]) / (s_bar_w[k+1] - s_bar_w[k])
for (k in 2:4) {
  cur <- km_indices_by_k[[paste0("k", k)]]
  nxt <- km_indices_by_k[[paste0("k", k + 1L)]]
  km_indices_by_k[[paste0("k", k)]]$frey <-
    (nxt$mean_between_dist - cur$mean_between_dist) / (nxt$mean_within_dist - cur$mean_within_dist)
}

# Gap statistic (Tibshirani et al. 2001) via cluster::clusGap with fixed seed
set.seed(2026L)
gap_res <- cluster::clusGap(km_x, FUNcluster = stats::kmeans, K.max = 5L, B = 50L, nstart = 10L, verbose = FALSE)
gap_tab <- gap_res$Tab
gap_k_opt <- cluster::maxSE(gap_tab[, "gap"], gap_tab[, "SE.sim"], method = "firstSEmax")

kmeans_out <- list(
  points_30x2 = lapply(seq_len(nrow(km_x)), function(r) as_arr(as.numeric(km_x[r, ]))),
  initial_center_indices_0based = lapply(init_idx_list, function(v) as_arr(as.integer(v - 1L))),
  wcss_k1_to_k5 = as_arr(wcss_vec),
  by_k = km_indices_by_k,
  gap_clusGap_seed2026_B50 = list(
    logW = as_arr(unname(gap_tab[, "logW"])),
    E_logW = as_arr(unname(gap_tab[, "E.logW"])),
    gap = as_arr(unname(gap_tab[, "gap"])),
    se_sim = as_arr(unname(gap_tab[, "SE.sim"])),
    optimal_k_firstSEmax = as.integer(gap_k_opt)
  )
)

# 2F. Survival Power Analysis (Schoenfeld 1983 & Freedman 1982)
surv_power_case <- function(hr, alpha, power, alloc_ratio_n2_over_n1, two_sided = TRUE, s1 = 0.60) {
  z_alpha <- qnorm(if (two_sided) 1 - alpha / 2 else 1 - alpha)
  z_beta <- qnorm(power)
  p1 <- 1 / (1 + alloc_ratio_n2_over_n1)
  p2 <- alloc_ratio_n2_over_n1 / (1 + alloc_ratio_n2_over_n1)
  # Schoenfeld (1983), Biometrics 39:499-503
  e_schoenfeld <- (z_alpha + z_beta)^2 / (p1 * p2 * (log(hr))^2)
  # Freedman (1982), Statistics in Medicine 1:121-129
  # E = (z_alpha + z_beta)^2 * (p1 + p2 * hr)^2 / (p1 * p2 * (1 - hr)^2)
  # Note: for r = n2/n1, (p1 + p2*hr)^2/(p1*p2) = (1 + r*hr)^2 / r; at r=1 gives ((1+hr)/(1-hr))^2.
  e_freedman <- (z_alpha + z_beta)^2 * (p1 + p2 * hr)^2 / (p1 * p2 * (1 - hr)^2)
  # Arm 2 survival probability under proportional hazards S2 = S1^HR
  s2 <- s1^hr
  p_event <- 1 - (p1 * s1 + p2 * s2)
  n_schoenfeld <- e_schoenfeld / p_event
  n_freedman <- e_freedman / p_event
  # Power achieved at fixed E = 100 events
  e_fixed <- 100.0
  pwr_schoenfeld_e100 <- pnorm(sqrt(e_fixed * p1 * p2) * abs(log(hr)) - z_alpha)
  pwr_freedman_e100 <- pnorm(sqrt(e_fixed * p1 * p2) * abs(1 - hr) / (p1 + p2 * hr) - z_alpha)
  # Detectable HR (< 1) at fixed E = 100, given alpha & power
  det_hr_schoenfeld_e100 <- exp(-(z_alpha + z_beta) / sqrt(e_fixed * p1 * p2))
  # Solve (p1 + p2*hr) / (1 - hr) = sqrt(E*p1*p2) / (z_a + z_b) = M => hr = (M - p1)/(M + p2)
  M_f <- sqrt(e_fixed * p1 * p2) / (z_alpha + z_beta)
  det_hr_freedman_e100 <- if (M_f > p1) (M_f - p1) / (M_f + p2) else NA_real_

  list(
    hr = hr, alpha = alpha, power = power, alloc_ratio = alloc_ratio_n2_over_n1,
    two_sided = two_sided, s1 = s1, s2 = s2, p_event = p_event,
    schoenfeld_events = e_schoenfeld,
    freedman_events = e_freedman,
    schoenfeld_total_n = n_schoenfeld,
    freedman_total_n = n_freedman,
    power_at_100_events_schoenfeld = pwr_schoenfeld_e100,
    power_at_100_events_freedman = pwr_freedman_e100,
    detectable_hr_lt1_at_100_events_schoenfeld = det_hr_schoenfeld_e100,
    detectable_hr_lt1_at_100_events_freedman = det_hr_freedman_e100
  )
}

survival_power_out <- list(
  hr065_a005_p80_r1_2s = surv_power_case(0.65, 0.05, 0.80, 1.0, TRUE, 0.60),
  hr050_a005_p90_r1_2s = surv_power_case(0.50, 0.05, 0.90, 1.0, TRUE, 0.50),
  hr075_a001_p80_r2_2s = surv_power_case(0.75, 0.01, 0.80, 2.0, TRUE, 0.65),
  hr065_a005_p80_r05_1s = surv_power_case(0.65, 0.05, 0.80, 0.5, FALSE, 0.60),
  hr150_a005_p80_r1_2s = surv_power_case(1.50, 0.05, 0.80, 1.0, TRUE, 0.70)
)

# ============================================================================
# SECTION 3: MULTIVARIABLE GLM §5.4 (R23.5)
# ============================================================================

# Helper for AICc: Gaussian lm uses K = p + 1 (includes sigma^2); GLM binomial/poisson uses K = p.
glm_aicc <- function(fit, is_gaussian = FALSE) {
  n <- stats::nobs(fit)
  k <- length(stats::coef(fit)) + (if (is_gaussian) 1L else 0L)
  aic <- stats::AIC(fit)
  aic + (2 * k * (k + 1)) / (n - k - 1)
}

# 3A. Multiple Linear GLM (2-/3-way interactions, X^2, sqrt(X), no-intercept, nested F, WLS, diagnostics, OOS)
lin_df <- data.frame(
  x1 = c(1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5,
         1.2, 1.8, 2.4, 3.0, 3.6, 4.2, 4.8, 5.4, 6.0, 6.6, 7.2, 7.8, 8.4, 9.0),
  x2 = c(-1.5, 0.8, -0.4, 1.2, -1.0, 0.5, 1.8, -0.6, 0.2, 1.5, -1.2, 0.9, -0.2, 1.1,
          0.6, -1.3, 1.4, -0.5, 0.9, -1.1, 0.3, 1.7, -0.8, 0.4, 1.2, -1.4, 0.7, -0.1),
  grp = factor(rep(c("A", "B"), each = 14), levels = c("A", "B")),
  w_custom = c(1.0, 1.2, 0.8, 1.5, 1.1, 0.9, 1.3, 1.0, 1.4, 0.7, 1.2, 1.0, 1.6, 0.9,
               1.1, 0.8, 1.3, 1.0, 1.5, 1.2, 0.9, 1.4, 1.0, 1.1, 0.8, 1.3, 1.2, 1.0)
)
# Deterministic realistic outcome with nonlinear & interaction signal + small perturbation
eps_lin <- 0.45 * sin(2.7 * seq_len(nrow(lin_df)) + 0.4)
lin_df$y <- 4.0 + 1.8 * lin_df$x1 - 0.9 * lin_df$x2 + 2.2 * (lin_df$grp == "B") +
  0.35 * lin_df$x1 * lin_df$x2 + 0.6 * lin_df$x1 * (lin_df$grp == "B") -
  0.4 * lin_df$x2 * (lin_df$grp == "B") + 0.12 * lin_df$x1 * lin_df$x2 * (lin_df$grp == "B") +
  0.15 * (lin_df$x1^2) + 0.9 * sqrt(lin_df$x1) + eps_lin

lin_new <- data.frame(
  x1 = c(3.2, 6.4),
  x2 = c(0.5, -0.7),
  grp = factor(c("A", "B"), levels = c("A", "B"))
)

lm_red <- stats::lm(y ~ x1 + x2 + grp, data = lin_df)
lm_full <- stats::lm(y ~ x1 + x2 + grp + x1:x2 + x1:grp + x2:grp + x1:x2:grp + I(x1^2) + I(sqrt(x1)), data = lin_df)
lm_noint <- stats::lm(y ~ 0 + x1 + x2 + grp, data = lin_df)
lm_nest <- stats::anova(lm_red, lm_full)
lm_wls_custom <- stats::lm(y ~ x1 + x2 + grp, data = lin_df, weights = w_custom)
lm_wls_inv_y2 <- stats::lm(y ~ x1 + x2 + grp, data = lin_df, weights = 1 / (y^2))

summarize_lm <- function(fit) {
  s <- summary(fit)
  co <- s$coefficients
  ci <- stats::confint(fit, level = 0.95)
  list(
    terms = as_arr(rownames(co)),
    beta = as_arr(unname(co[, "Estimate"])),
    se = as_arr(unname(co[, "Std. Error"])),
    t = as_arr(unname(co[, "t value"])),
    p = as_arr(unname(co[, "Pr(>|t|)"])),
    ci95_lo = as_arr(unname(ci[, 1L])),
    ci95_hi = as_arr(unname(ci[, 2L])),
    r_squared = s$r.squared,
    adj_r_squared = s$adj.r.squared,
    sigma = s$sigma,
    df_residual = as.integer(fit$df.residual),
    rss = sum(stats::residuals(fit)^2 * (if (is.null(fit$weights)) 1 else fit$weights)),
    aic = stats::AIC(fit),
    aicc = glm_aicc(fit, is_gaussian = TRUE)
  )
}

pred_ci_red <- stats::predict(lm_red, newdata = lin_new, interval = "confidence", level = 0.95)
pred_pi_red <- stats::predict(lm_red, newdata = lin_new, interval = "prediction", level = 0.95)
pred_ci_wls <- stats::predict(lm_wls_custom, newdata = lin_new, interval = "confidence", level = 0.95)
pred_pi_wls <- suppressWarnings(stats::predict(lm_wls_custom, newdata = lin_new, interval = "prediction", weights = c(1, 1), level = 0.95))

dfb_red <- stats::dfbetas(lm_red)

glm_linear_out <- list(
  data = list(
    x1 = as_arr(lin_df$x1), x2 = as_arr(lin_df$x2),
    grp = as_arr(as.character(lin_df$grp)), w_custom = as_arr(lin_df$w_custom),
    y = as_arr(lin_df$y)
  ),
  new_data = list(x1 = as_arr(lin_new$x1), x2 = as_arr(lin_new$x2), grp = as_arr(as.character(lin_new$grp))),
  reduced = summarize_lm(lm_red),
  full_interactions_poly_sqrt = summarize_lm(lm_full),
  no_intercept = summarize_lm(lm_noint),
  nested_f_test = list(
    f = unname(lm_nest$F[2]),
    df1 = unname(lm_nest$Df[2]),
    df2 = unname(lm_nest$Res.Df[2]),
    p_value = unname(lm_nest$`Pr(>F)`[2]),
    delta_aicc_full_minus_red = glm_aicc(lm_full, TRUE) - glm_aicc(lm_red, TRUE)
  ),
  diagnostics_reduced = list(
    leverage = as_arr(unname(stats::hatvalues(lm_red))),
    raw_residuals = as_arr(unname(stats::residuals(lm_red))),
    standardized_residuals = as_arr(unname(stats::rstandard(lm_red))),
    studentized_residuals = as_arr(unname(stats::rstudent(lm_red))),
    cooks_d = as_arr(unname(stats::cooks.distance(lm_red))),
    dfbetas_row0 = as_arr(unname(dfb_red[1L, ])),
    dfbetas_row13 = as_arr(unname(dfb_red[14L, ])),
    dfbetas_row27 = as_arr(unname(dfb_red[28L, ]))
  ),
  oos_reduced = list(
    fit = as_arr(unname(pred_ci_red[, "fit"])),
    ci95_lo = as_arr(unname(pred_ci_red[, "lwr"])),
    ci95_hi = as_arr(unname(pred_ci_red[, "upr"])),
    pi95_lo = as_arr(unname(pred_pi_red[, "lwr"])),
    pi95_hi = as_arr(unname(pred_pi_red[, "upr"]))
  ),
  wls_custom = c(
    summarize_lm(lm_wls_custom),
    list(
      leverage = as_arr(unname(stats::hatvalues(lm_wls_custom))),
      cooks_d = as_arr(unname(stats::cooks.distance(lm_wls_custom))),
      oos_fit = as_arr(unname(pred_ci_wls[, "fit"])),
      oos_ci95_lo = as_arr(unname(pred_ci_wls[, "lwr"])),
      oos_ci95_hi = as_arr(unname(pred_ci_wls[, "upr"])),
      oos_pi95_lo = as_arr(unname(pred_pi_wls[, "lwr"])),
      oos_pi95_hi = as_arr(unname(pred_pi_wls[, "upr"]))
    )
  ),
  wls_inv_y2 = summarize_lm(lm_wls_inv_y2)
)

# 3B. Multiple Logistic GLM (2-way interaction, X^2, sqrt(X), no-intercept, nested LRT, profile CI, SE of OR, cutoffs)
# Well-conditioned overlapping binary outcome (n = 48, zero separation warnings)
log_df <- data.frame(
  x1 = c(
    0.5, 0.7, 0.9, 1.1, 1.3, 1.5, 1.7, 1.9, 2.1, 2.3, 2.5, 2.7,
    2.9, 3.1, 3.3, 3.5, 3.7, 3.9, 4.1, 4.3, 4.5, 4.7, 4.9, 5.1,
    0.6, 0.8, 1.0, 1.2, 1.4, 1.6, 1.8, 2.0, 2.2, 2.4, 2.6, 2.8,
    3.0, 3.2, 3.4, 3.6, 3.8, 4.0, 4.2, 4.4, 4.6, 4.8, 5.0, 5.2
  ),
  x2 = c(
    -1.4,  0.8, -0.5,  1.2, -1.1,  0.4,  1.6, -0.9,  0.2,  1.4, -1.3,  0.7,
    -0.3,  1.1, -1.5,  0.5, -0.1,  1.3, -0.7,  0.9, -1.2,  0.3, -0.4,  1.0,
     0.9, -1.2,  1.3, -0.6,  0.7, -1.4,  0.3,  1.5, -0.8,  0.5,  1.1, -1.1,
     0.4, -0.7,  1.2, -0.2,  0.8, -1.3,  0.1,  1.4, -0.9,  0.6, -0.5,  1.0
  ),
  grp = factor(rep(c("Ctrl", "Exp"), each = 24), levels = c("Ctrl", "Exp")),
  y = c(
    0L, 1L, 0L, 0L, 0L, 1L, 1L, 0L, 0L, 1L, 0L, 1L,
    1L, 0L, 0L, 1L, 1L, 1L, 0L, 1L, 1L, 1L, 0L, 1L,
    0L, 0L, 1L, 0L, 1L, 0L, 1L, 1L, 0L, 1L, 1L, 0L,
    1L, 1L, 1L, 0L, 1L, 1L, 1L, 1L, 0L, 1L, 1L, 1L
  )
)
log_new <- data.frame(x1 = c(1.6, 4.0), x2 = c(-0.4, 0.8), grp = factor(c("Ctrl", "Exp"), levels = c("Ctrl", "Exp")))

logit_red <- stats::glm(y ~ x1 + x2 + grp, data = log_df, family = stats::binomial(),
                        control = stats::glm.control(epsilon = 1e-14, maxit = 100))
logit_full <- stats::glm(y ~ x1 + x2 + grp + x1:x2 + I(x1^2) + I(sqrt(x1)), data = log_df, family = stats::binomial(),
                         control = stats::glm.control(epsilon = 1e-14, maxit = 100))
logit_noint <- stats::glm(y ~ 0 + x1 + x2 + grp, data = log_df, family = stats::binomial(),
                          control = stats::glm.control(epsilon = 1e-14, maxit = 100))

summarize_logistic <- function(fit, do_profile = TRUE) {
  co <- summary(fit)$coefficients
  beta <- unname(co[, "Estimate"])
  se <- unname(co[, "Std. Error"])
  or <- exp(beta)
  se_or <- or * se
  ci_wald <- stats::confint.default(fit, level = 0.95)
  ci_prof <- if (do_profile) suppressMessages(stats::confint(fit, level = 0.95)) else NULL
  p_hat <- stats::fitted(fit)
  y_obs <- fit$y
  n <- length(y_obs)
  tjur_r2 <- mean(p_hat[y_obs == 1]) - mean(p_hat[y_obs == 0])
  ll_mod <- as.numeric(stats::logLik(fit))
  ll_null <- -0.5 * fit$null.deviance
  mcfadden_r2 <- 1 - fit$deviance / fit$null.deviance
  cs_r2 <- 1 - exp((fit$deviance - fit$null.deviance) / n)
  nagelkerke_r2 <- cs_r2 / (1 - exp(-fit$null.deviance / n))
  out <- list(
    terms = as_arr(rownames(co)),
    beta = as_arr(beta),
    se = as_arr(se),
    wald_z = as_arr(unname(co[, "z value"])),
    p = as_arr(unname(co[, "Pr(>|z|)"])),
    or = as_arr(or),
    se_or_delta = as_arr(se_or),
    beta_wald_ci95_lo = as_arr(unname(ci_wald[, 1L])),
    beta_wald_ci95_hi = as_arr(unname(ci_wald[, 2L])),
    or_wald_ci95_lo = as_arr(exp(unname(ci_wald[, 1L]))),
    or_wald_ci95_hi = as_arr(exp(unname(ci_wald[, 2L]))),
    deviance = fit$deviance,
    null_deviance = fit$null.deviance,
    df_residual = as.integer(fit$df.residual),
    log_lik = ll_mod,
    aic = stats::AIC(fit),
    aicc = glm_aicc(fit, FALSE),
    tjur_r2 = tjur_r2,
    mcfadden_r2 = mcfadden_r2,
    cox_snell_r2 = cs_r2,
    nagelkerke_r2 = nagelkerke_r2
  )
  if (!is.null(ci_prof)) {
    out$beta_profile_ci95_lo <- as_arr(unname(ci_prof[, 1L]))
    out$beta_profile_ci95_hi <- as_arr(unname(ci_prof[, 2L]))
    out$or_profile_ci95_lo <- as_arr(exp(unname(ci_prof[, 1L])))
    out$or_profile_ci95_hi <- as_arr(exp(unname(ci_prof[, 2L])))
  }
  out
}

confusion_at <- function(y, p_hat, cut) {
  pred <- as.integer(p_hat >= cut)
  tp <- sum(pred == 1L & y == 1L)
  tn <- sum(pred == 0L & y == 0L)
  fp <- sum(pred == 1L & y == 0L)
  fn <- sum(pred == 0L & y == 1L)
  list(
    cutoff = cut, tp = as.integer(tp), tn = as.integer(tn), fp = as.integer(fp), fn = as.integer(fn),
    sensitivity = tp / (tp + fn), specificity = tn / (tn + fp),
    ppv = if (tp + fp > 0) tp / (tp + fp) else NA_real_,
    npv = if (tn + fn > 0) tn / (tn + fn) else NA_real_,
    accuracy = (tp + tn) / length(y)
  )
}

logit_oos <- stats::predict(logit_red, newdata = log_new, type = "link", se.fit = TRUE)
z95 <- qnorm(0.975)
logit_lrt_chi2 <- logit_red$deviance - logit_full$deviance
logit_lrt_df <- logit_red$df.residual - logit_full$df.residual
dfb_logit <- stats::dfbetas(logit_red)

glm_logistic_out <- list(
  data = list(x1 = as_arr(log_df$x1), x2 = as_arr(log_df$x2), grp = as_arr(as.character(log_df$grp)), y = as_arr(log_df$y)),
  new_data = list(x1 = as_arr(log_new$x1), x2 = as_arr(log_new$x2), grp = as_arr(as.character(log_new$grp))),
  reduced = summarize_logistic(logit_red, TRUE),
  full_interaction_poly_sqrt = summarize_logistic(logit_full, FALSE),
  no_intercept = summarize_logistic(logit_noint, FALSE),
  nested_lrt = list(
    chi2 = logit_lrt_chi2,
    df = as.integer(logit_lrt_df),
    p_value = pchisq(logit_lrt_chi2, df = logit_lrt_df, lower.tail = FALSE),
    delta_aicc_full_minus_red = glm_aicc(logit_full, FALSE) - glm_aicc(logit_red, FALSE)
  ),
  diagnostics_reduced = list(
    leverage = as_arr(unname(stats::hatvalues(logit_red))),
    pearson_residuals = as_arr(unname(stats::residuals(logit_red, type = "pearson"))),
    deviance_residuals = as_arr(unname(stats::residuals(logit_red, type = "deviance"))),
    cooks_d = as_arr(unname(stats::cooks.distance(logit_red))),
    dfbetas_row0 = as_arr(unname(dfb_logit[1L, ])),
    dfbetas_last = as_arr(unname(dfb_logit[nrow(dfb_logit), ]))
  ),
  oos_reduced = list(
    prob = as_arr(plogis(unname(logit_oos$fit))),
    ci95_lo = as_arr(plogis(unname(logit_oos$fit - z95 * logit_oos$se.fit))),
    ci95_hi = as_arr(plogis(unname(logit_oos$fit + z95 * logit_oos$se.fit)))
  ),
  classification_reduced = list(
    cut_0_3 = confusion_at(log_df$y, stats::fitted(logit_red), 0.3),
    cut_0_5 = confusion_at(log_df$y, stats::fitted(logit_red), 0.5),
    cut_0_7 = confusion_at(log_df$y, stats::fitted(logit_red), 0.7)
  )
)

# 3C. Multiple Poisson GLM (2-way interaction, sqrt(X), nested LRT, quasi-Poisson dispersion, profile CI, SE of IRR, OOS)
pois_df <- data.frame(
  x1 = c(0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5,
         0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5),
  x2 = c(-1.0, 0.5, -0.4, 0.8, -0.7, 0.3, 1.1, -0.5, 0.2, 0.9, -0.8, 0.6, -0.2, 0.7, -0.9,
          0.6, -0.8, 0.9, -0.3, 0.4, -0.9, 0.2, 1.0, -0.6, 0.3, 0.8, -0.7, 0.5, -0.1, 0.7),
  grp = factor(rep(c("Ctrl", "Treat"), each = 15), levels = c("Ctrl", "Treat")),
  y = c(0L, 4L, 0L, 6L, 1L, 8L, 14L, 2L, 4L, 21L, 5L, 22L, 8L, 30L, 10L,
        1L, 6L, 2L, 9L, 4L, 13L, 7L, 27L, 8L, 29L, 41L, 14L, 48L, 20L, 62L)
)
pois_new <- data.frame(x1 = c(2.2, 5.8), x2 = c(0.1, -0.5), grp = factor(c("Ctrl", "Treat"), levels = c("Ctrl", "Treat")))

pois_red <- stats::glm(y ~ x1 + x2 + grp, data = pois_df, family = stats::poisson(),
                       control = stats::glm.control(epsilon = 1e-14, maxit = 100))
pois_full <- stats::glm(y ~ x1 + x2 + grp + x1:grp + I(sqrt(x1)), data = pois_df, family = stats::poisson(),
                        control = stats::glm.control(epsilon = 1e-14, maxit = 100))
pois_q_red <- stats::glm(y ~ x1 + x2 + grp, data = pois_df, family = stats::quasipoisson(),
                         control = stats::glm.control(epsilon = 1e-14, maxit = 100))

summarize_poisson <- function(fit, do_profile = TRUE) {
  co <- summary(fit, dispersion = 1)$coefficients
  beta <- unname(co[, "Estimate"])
  se <- unname(co[, "Std. Error"])
  irr <- exp(beta)
  se_irr <- irr * se
  ci_wald <- stats::confint.default(fit, level = 0.95)
  ci_prof <- if (do_profile) suppressMessages(stats::confint(fit, level = 0.95)) else NULL
  rp <- stats::residuals(fit, type = "pearson")
  phi <- sum(rp^2) / fit$df.residual
  out <- list(
    terms = as_arr(rownames(co)),
    beta = as_arr(beta),
    se = as_arr(se),
    wald_z = as_arr(unname(co[, "z value"])),
    p = as_arr(unname(co[, "Pr(>|z|)"])),
    irr = as_arr(irr),
    se_irr_delta = as_arr(se_irr),
    beta_wald_ci95_lo = as_arr(unname(ci_wald[, 1L])),
    beta_wald_ci95_hi = as_arr(unname(ci_wald[, 2L])),
    irr_wald_ci95_lo = as_arr(exp(unname(ci_wald[, 1L]))),
    irr_wald_ci95_hi = as_arr(exp(unname(ci_wald[, 2L]))),
    deviance = fit$deviance,
    null_deviance = fit$null.deviance,
    df_residual = as.integer(fit$df.residual),
    pearson_chi2 = sum(rp^2),
    pearson_dispersion = phi,
    aic = stats::AIC(fit),
    aicc = glm_aicc(fit, FALSE)
  )
  if (!is.null(ci_prof)) {
    out$beta_profile_ci95_lo <- as_arr(unname(ci_prof[, 1L]))
    out$beta_profile_ci95_hi <- as_arr(unname(ci_prof[, 2L]))
    out$irr_profile_ci95_lo <- as_arr(exp(unname(ci_prof[, 1L])))
    out$irr_profile_ci95_hi <- as_arr(exp(unname(ci_prof[, 2L])))
  }
  out
}

q_sum <- summary(pois_q_red)
pois_oos <- stats::predict(pois_red, newdata = pois_new, type = "link", se.fit = TRUE)
pois_lrt_chi2 <- pois_red$deviance - pois_full$deviance
pois_lrt_df <- pois_red$df.residual - pois_full$df.residual

glm_poisson_out <- list(
  data = list(x1 = as_arr(pois_df$x1), x2 = as_arr(pois_df$x2), grp = as_arr(as.character(pois_df$grp)), y = as_arr(pois_df$y)),
  new_data = list(x1 = as_arr(pois_new$x1), x2 = as_arr(pois_new$x2), grp = as_arr(as.character(pois_new$grp))),
  reduced = summarize_poisson(pois_red, TRUE),
  full_interaction_sqrt = summarize_poisson(pois_full, TRUE),
  quasi_poisson_reduced = list(
    dispersion = q_sum$dispersion,
    se_scaled = as_arr(unname(q_sum$coefficients[, "Std. Error"])),
    t_value = as_arr(unname(q_sum$coefficients[, "t value"])),
    p_value = as_arr(unname(q_sum$coefficients[, "Pr(>|t|)"]))
  ),
  nested_lrt = list(
    chi2 = pois_lrt_chi2,
    df = as.integer(pois_lrt_df),
    p_value = pchisq(pois_lrt_chi2, df = pois_lrt_df, lower.tail = FALSE),
    delta_aicc_full_minus_red = glm_aicc(pois_full, FALSE) - glm_aicc(pois_red, FALSE)
  ),
  diagnostics_reduced = list(
    leverage = as_arr(unname(stats::hatvalues(pois_red))),
    pearson_residuals = as_arr(unname(stats::residuals(pois_red, type = "pearson"))),
    deviance_residuals = as_arr(unname(stats::residuals(pois_red, type = "deviance"))),
    cooks_d = as_arr(unname(stats::cooks.distance(pois_red)))
  ),
  oos_reduced = list(
    rate = as_arr(exp(unname(pois_oos$fit))),
    ci95_lo = as_arr(exp(unname(pois_oos$fit - z95 * pois_oos$se.fit))),
    ci95_hi = as_arr(exp(unname(pois_oos$fit + z95 * pois_oos$se.fit)))
  )
)

# ============================================================================
# SECTION 4: COX PH & KAPLAN-MEIER EXTENSIONS (R23.5)
# ============================================================================

cox_df <- data.frame(
  time = c(
     4,  6,  8,  8, 10, 12, 14, 15, 16, 18, 20, 21, 22, 25, 28, 30, 32, 36,
     5,  7,  9, 11, 13, 15, 17, 19, 22, 24, 26, 29, 31, 34, 37, 40, 42, 46
  ),
  status = c(
    1L, 1L, 1L, 0L, 1L, 1L, 1L, 0L, 1L, 1L, 1L, 0L, 1L, 1L, 0L, 1L, 0L, 0L,
    1L, 0L, 1L, 1L, 0L, 1L, 1L, 0L, 1L, 1L, 0L, 1L, 1L, 0L, 1L, 1L, 0L, 0L
  ),
  arm = factor(rep(c("Ctrl", "Drug"), each = 18), levels = c("Ctrl", "Drug")),
  biomarker = c(
    3.6, 2.2, 3.1, 1.8, 2.9, 1.5, 2.7, 2.4, 3.3, 1.9, 2.1, 1.4, 2.5, 1.7, 2.8, 1.2, 2.0, 1.1,
    3.8, 2.5, 3.4, 1.9, 2.6, 3.0, 1.6, 2.2, 2.8, 1.7, 2.4, 1.5, 2.1, 1.3, 2.3, 1.4, 1.9, 1.0
  ),
  age = c(
    62, 71, 48, 55, 67, 52, 45, 64, 58, 70, 49, 61, 53, 66, 44, 59, 47, 56,
    65, 50, 72, 57, 46, 63, 68, 51, 54, 69, 48, 60, 43, 58, 64, 52, 49, 45
  )
)

# Exact Cox PH profile-likelihood 95% CI by profiling partial log-likelihood
cox_profile_ci <- function(df, Xmat, ties = "efron", level = 0.95) {
  p <- ncol(Xmat)
  fit0 <- survival::coxph(survival::Surv(df$time, df$status) ~ Xmat, ties = ties)
  ll_max <- fit0$loglik[2]
  beta_hat <- unname(coef(fit0))
  se_hat <- sqrt(diag(vcov(fit0)))
  thresh <- ll_max - 0.5 * qchisq(level, df = 1)
  lo <- numeric(p); hi <- numeric(p)
  for (j in seq_len(p)) {
    prof_diff <- function(b_j) {
      off <- b_j * Xmat[, j]
      if (p == 1L) {
        f <- survival::coxph(survival::Surv(df$time, df$status) ~ offset(off), ties = ties)
        return(f$loglik[1] - thresh)
      } else {
        X_rest <- Xmat[, -j, drop = FALSE]
        f <- survival::coxph(survival::Surv(df$time, df$status) ~ X_rest + offset(off), ties = ties)
        return(f$loglik[2] - thresh)
      }
    }
    step <- max(se_hat[j], 0.1)
    lb <- beta_hat[j] - 2 * step
    while (prof_diff(lb) > 0) lb <- lb - 2 * step
    ub <- beta_hat[j] + 2 * step
    while (prof_diff(ub) > 0) ub <- ub + 2 * step
    lo[j] <- uniroot(prof_diff, c(lb, beta_hat[j]), tol = 1e-11)$root
    hi[j] <- uniroot(prof_diff, c(beta_hat[j], ub), tol = 1e-11)$root
  }
  cbind(lo = lo, hi = hi)
}

cox_red_efron <- survival::coxph(survival::Surv(time, status) ~ arm + biomarker, data = cox_df, ties = "efron")
cox_full_efron <- survival::coxph(survival::Surv(time, status) ~ arm + biomarker + I(sqrt(age)) + arm:biomarker, data = cox_df, ties = "efron")
cox_full_breslow <- survival::coxph(survival::Surv(time, status) ~ arm + biomarker + I(sqrt(age)) + arm:biomarker, data = cox_df, ties = "breslow")

X_cox_full <- stats::model.matrix(cox_full_efron)
prof_cox_efron <- cox_profile_ci(cox_df, X_cox_full, ties = "efron", level = 0.95)

summarize_cox <- function(fit, prof_ci = NULL) {
  s <- summary(fit)
  co <- s$coefficients
  beta <- unname(co[, "coef"])
  se <- unname(co[, "se(coef)"])
  hr <- exp(beta)
  se_hr <- hr * se
  cor_mat <- stats::cov2cor(vcov(fit))
  n_ev <- fit$nevent
  k <- length(beta)
  aic_cox <- -2 * fit$loglik[2] + 2 * k
  aicc_events <- aic_cox + (2 * k * (k + 1)) / (n_ev - k - 1)
  out <- list(
    terms = as_arr(rownames(co)),
    beta = as_arr(beta),
    se = as_arr(se),
    wald_z = as_arr(unname(co[, "z"])),
    p = as_arr(unname(co[, "Pr(>|z|)"])),
    hr = as_arr(hr),
    se_hr_delta = as_arr(se_hr),
    hr_wald_ci95_lo = as_arr(unname(s$conf.int[, "lower .95"])),
    hr_wald_ci95_hi = as_arr(unname(s$conf.int[, "upper .95"])),
    loglik_null = fit$loglik[1],
    loglik_model = fit$loglik[2],
    lr_chi2 = unname(s$logtest["test"]),
    lr_p = unname(s$logtest["pvalue"]),
    n = as.integer(fit$n),
    events = as.integer(n_ev),
    aic = aic_cox,
    aicc_by_events = aicc_events,
    param_corr_matrix = lapply(seq_len(nrow(cor_mat)), function(r) as_arr(unname(cor_mat[r, ])))
  )
  if (!is.null(prof_ci)) {
    out$beta_profile_ci95_lo <- as_arr(unname(prof_ci[, "lo"]))
    out$beta_profile_ci95_hi <- as_arr(unname(prof_ci[, "hi"]))
    out$hr_profile_ci95_lo <- as_arr(exp(unname(prof_ci[, "lo"])))
    out$hr_profile_ci95_hi <- as_arr(exp(unname(prof_ci[, "hi"])))
  }
  out
}

# Covariate-profile predicted survival curves from cox_full_efron
cox_profiles <- data.frame(
  arm = factor(c("Ctrl", "Drug"), levels = c("Ctrl", "Drug")),
  biomarker = c(1.5, 3.0),
  age = c(49, 64)
)
sf_prof <- survival::survfit(cox_full_efron, newdata = cox_profiles)
ev_mask <- sf_prof$n.event > 0

# Cox-Snell residuals: r_CS = delta_i - r_Martingale_i = H_0(t_i) * exp(eta_i)
r_mart <- unname(stats::residuals(cox_full_efron, type = "martingale"))
r_cs <- cox_df$status - r_mart

# Nested Cox LRT (reduced vs full on identical 36 subjects)
cox_nest_chi2 <- 2 * (cox_full_efron$loglik[2] - cox_red_efron$loglik[2])
cox_nest_df <- length(coef(cox_full_efron)) - length(coef(cox_red_efron))

# Reverse-KM median follow-up (Schemper & Smith 1996) & Ratio of Median Survival
km_arms <- survival::survfit(survival::Surv(time, status) ~ arm, data = cox_df, conf.type = "log-log")
km_tab <- summary(km_arms)$table
med_ctrl <- unname(km_tab["arm=Ctrl", "median"])
med_drug <- unname(km_tab["arm=Drug", "median"])

rev_km_all <- survival::survfit(survival::Surv(time, 1L - status) ~ 1, data = cox_df, conf.type = "log-log")
rev_tab_all <- summary(rev_km_all)$table
rev_km_arms <- survival::survfit(survival::Surv(time, 1L - status) ~ arm, data = cox_df, conf.type = "log-log")
rev_tab_arms <- summary(rev_km_arms)$table

cox_km_out <- list(
  data = list(
    time = as_arr(cox_df$time), status = as_arr(cox_df$status),
    arm = as_arr(as.character(cox_df$arm)),
    biomarker = as_arr(cox_df$biomarker), age = as_arr(cox_df$age)
  ),
  cox_reduced_efron = summarize_cox(cox_red_efron, NULL),
  cox_full_efron = summarize_cox(cox_full_efron, prof_cox_efron),
  cox_full_breslow = summarize_cox(cox_full_breslow, NULL),
  nested_lrt_efron = list(
    chi2 = cox_nest_chi2,
    df = as.integer(cox_nest_df),
    p_value = pchisq(cox_nest_chi2, df = cox_nest_df, lower.tail = FALSE)
  ),
  covariate_profile_survfit_efron = list(
    profiles = list(
      list(arm = "Ctrl", biomarker = 1.5, age = 49),
      list(arm = "Drug", biomarker = 3.0, age = 64)
    ),
    event_times = as_arr(sf_prof$time[ev_mask]),
    surv_profile0 = as_arr(sf_prof$surv[ev_mask, 1L]),
    surv_profile1 = as_arr(sf_prof$surv[ev_mask, 2L]),
    cumhaz_profile0 = as_arr(sf_prof$cumhaz[ev_mask, 1L]),
    cumhaz_profile1 = as_arr(sf_prof$cumhaz[ev_mask, 2L])
  ),
  cox_snell_residuals_full_efron = list(
    martingale = as_arr(r_mart),
    cox_snell = as_arr(r_cs)
  ),
  km_median_and_ratio = list(
    ctrl_median = med_ctrl,
    ctrl_ci95 = as_arr(unname(c(km_tab["arm=Ctrl", "0.95LCL"], km_tab["arm=Ctrl", "0.95UCL"]))),
    drug_median = med_drug,
    drug_ci95 = as_arr(unname(c(km_tab["arm=Drug", "0.95LCL"], km_tab["arm=Drug", "0.95UCL"]))),
    ratio_ctrl_over_drug = med_ctrl / med_drug,
    ratio_drug_over_ctrl = med_drug / med_ctrl
  ),
  reverse_km_median_followup = list(
    overall_median = unname(rev_tab_all["median"]),
    overall_ci95 = as_arr(unname(c(rev_tab_all["0.95LCL"], rev_tab_all["0.95UCL"]))),
    ctrl_median = unname(rev_tab_arms["arm=Ctrl", "median"]),
    ctrl_ci95 = as_arr(unname(c(rev_tab_arms["arm=Ctrl", "0.95LCL"], rev_tab_arms["arm=Ctrl", "0.95UCL"]))),
    drug_median = unname(rev_tab_arms["arm=Drug", "median"]),
    drug_ci95 = as_arr(unname(c(rev_tab_arms["arm=Drug", "0.95LCL"], rev_tab_arms["arm=Drug", "0.95UCL"])))
  )
)

payload <- list(
  provenance = list(
    script = "crates/stats/tests/reference/r23_stats_oracles.R",
    r_version = as.character(getRversion()),
    survival_version = as.character(packageVersion("survival")),
    nlme_version = as.character(packageVersion("nlme")),
    mass_version = as.character(packageVersion("MASS")),
    cluster_version = as.character(packageVersion("cluster"))
  ),
  r23_2_lognormal_suite = list(
    normality_skewed_pos = normality_lognormality_eval(ln_skewed),
    normality_gaussian_pos = normality_lognormality_eval(ln_normal),
    normality_with_nonpositive = normality_lognormality_eval(ln_nonpos),
    one_sample_ratio_t = one_sample_ratio_t,
    unpaired_lognormal_t = lognormal_two_group,
    lognormal_one_way_anova = lognormal_one_way,
    multiple_row_lognormal_and_ratio = multiple_row_lognormal
  ),
  r23_4_anova_kmeans_power = list(
    rm_2way_splitplot = rm_2way_splitplot,
    rm_2way_both_matched = rm_2way_both,
    rm_3way_mixed_sphericity = rm_3way_out,
    two_way_one_factor_diff = two_way_one_factor_diff,
    spearman_heteroscedasticity = spearman_heteroscedasticity,
    kmeans_validity_and_hw = kmeans_out,
    survival_power = survival_power_out
  ),
  r23_5_glm_5_4 = list(
    linear_and_wls = glm_linear_out,
    logistic = glm_logistic_out,
    poisson_and_quasi = glm_poisson_out
  ),
  r23_5_cox_and_km = cox_km_out
)

out_path <- "crates/stats/tests/reference/r23_stats_oracles.json"
writeLines(to_json(payload), con = out_path)
cat(sprintf("Wrote %s\n", out_path))
